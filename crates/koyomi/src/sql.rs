//! `koyomi sql run`: what a `sql` job runs. It reads the database, the SQL (or
//! the file that has it), the CSV file for a result and the rows that are 正常
//! from the `KOYOMI_SQL_*` the engine sets (each text after [`MAIL_TEXT`]), and
//! runs the SQL with ito-sql on SQL Server, PostgreSQL or SQLite. The password
//! comes already read from the credential store, as any `secret:` of a job's
//! environment does; by hand, `secret:<name>` is read here.
//!
//! It ends with 0 when every batch ran and the rows fit `rows`; without, with
//! [`NO_EXIT`](crate::ssh) and why in the file `KOYOMI_REASON` names. Each batch
//! commits by itself (unless the SQL has its own `BEGIN` … `COMMIT`), so the
//! batches before the one that failed stay done.
//!
//! The last result set is written to `out`, or without one to a file of the
//! temp folder that is deleted after; either way its first lines go to the log.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use ito_sql::{ConnectKind, SqlAuth, SqliteOpen, Target, Tls};
use koyomi_core::chore::{sql_db, sql_rows, Chore, SqlDb, SqlTls};
use koyomi_core::msg::{self, line, note};

/// The most a run may take here: the job's own `timeout` ends it sooner.
const RUN_MAX: Duration = Duration::from_secs(30 * 24 * 3600);

/// How many lines of the result (its header too) go to the log.
const SHOWN: usize = 20;

pub fn cmd(args: &[String]) -> i32 {
    match args {
        [op] if op == "run" => crate::ssh::run(exec),
        _ => {
            crate::say("koyomi: sql takes run (what a sql job does)", true);
            2
        }
    }
}

/// `KOYOMI_SQL_<name>` as the engine gave it, its [`MAIL_TEXT`] taken off;
/// `None` when it is not there.
///
/// [`MAIL_TEXT`]: koyomi_core::chore::MAIL_TEXT
fn var(name: &str) -> Option<String> {
    std::env::var(format!("KOYOMI_SQL_{name}")).ok().map(|v| crate::mail::text(&v).to_owned())
}

/// Reads the job from the environment and runs it.
fn exec(stop: &AtomicBool) -> Result<i32, String> {
    let db = var("DB").ok_or_else(|| line(msg::JOB_ENV_MISSING, &["KOYOMI_SQL_DB", "kind = \"sql\"", "sql"]))?;
    let tls = std::env::var("KOYOMI_SQL_TLS").ok().and_then(|t| SqlTls::of(t.trim())).unwrap_or_default();
    // Its value, read from the credential store when the job started; by hand,
    // still `secret:<name>`.
    let password = std::env::var("KOYOMI_SQL_PASSWORD").ok().filter(|p| !p.is_empty());
    let job = Chore::Sql {
        db,
        database: None,
        // Checked as written: the value itself is not a `secret:`.
        password: password.as_ref().map(|_| "secret:password".to_owned()),
        sql: var("TEXT"),
        file: var("FILE"),
        out: var("OUT"),
        rows: var("ROWS"),
        tls,
    };
    go(&job, password, stop)
}

/// Runs `job` (a `sql`, its `{…}` filled in) with the password `password`:
/// 0 when it ran and its rows fit `rows`, or why not.
fn go(job: &Chore, password: Option<String>, stop: &AtomicBool) -> Result<i32, String> {
    // Checked again now that each `{…}` is filled in.
    job.check()?;
    let Chore::Sql { db, sql, file, out, rows, tls, .. } = job else {
        return Err(line(msg::SQL_NOT_SQL, &[]));
    };
    let secret = || password.clone().map(crate::sftp::password).unwrap_or_else(|| Err(line(msg::CHORE_PASSWORD_MISSING, &[db])));
    let parsed = sql_db(db)?;
    // The password is read only for a login, as before.
    let resolved = match &parsed {
        SqlDb::SqlServer { user: Some(_), .. } | SqlDb::Postgres { .. } => Some(secret()?),
        SqlDb::SqlServer { user: None, .. } | SqlDb::Sqlite { .. } => None,
    };
    let target = target(parsed, *tls, resolved)?;
    let script = match (sql, file) {
        (Some(s), _) => s.clone(),
        (None, Some(f)) => {
            let path = Path::new(f.trim());
            let bytes =
                std::fs::read(path).map_err(|e| line(msg::SQL_FILE_UNREADABLE, &[&path.display().to_string(), &msg::io_words(&e)]))?;
            decode(&bytes)
        }
        (None, None) => return Err(line(msg::SQL_NO_SCRIPT, &[])),
    };
    let (csv, temp) = match out {
        Some(o) => {
            let path = PathBuf::from(o.trim());
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)
                    .map_err(|e| line(msg::SQL_OUT_DIR, &[&parent.display().to_string(), &msg::io_words(&e)]))?;
            }
            (path, false)
        }
        None => (std::env::temp_dir().join(format!("koyomi-sql-{}.csv", std::process::id())), true),
    };
    let place = place(&target);
    crate::say(&format!("sql: {place}"), false);
    let result = ito_sql::run(&target, &script, Some(&csv), stop, RUN_MAX);
    let shown = match &result {
        Ok(o) if o.result_rows.is_some() => preview(&csv),
        _ => Vec::new(),
    };
    if temp {
        let _ = std::fs::remove_file(&csv);
    }
    let o = result.map_err(|e| why(&e, &target, *tls, &place))?;
    crate::say(&format!("ran: {} batches, {} rows changed", o.batches, o.rows_affected), false);
    match o.result_rows {
        Some(n) if temp => crate::say(&format!("result: {n} rows"), false),
        Some(n) => crate::say(&format!("result: {n} rows -> {}", csv.display()), false),
        None if temp => {}
        None => crate::say_bad(&line(msg::SQL_NO_RESULT, &[&csv.display().to_string()])),
    }
    for row in shown {
        crate::say(&format!("  {row}"), false);
    }
    if let Some(r) = rows {
        let count = o.result_rows.unwrap_or(o.rows_affected);
        let what = note(if o.result_rows.is_some() { msg::N_SQL_RESULT_ROWS } else { msg::N_SQL_CHANGED_ROWS }, &[]);
        if !sql_rows(r)?.iter().any(|(least, most)| (*least..=*most).contains(&count)) {
            return Err(line(msg::SQL_ROWS_MISMATCH, &[&what, &count.to_string(), r]));
        }
    }
    Ok(0)
}

/// The ito-sql target of `db` with the password already read (`None` for SQL Server with Windows authentication and
/// for SQLite). A SQLite file that is not there is refused: ito-sql would make it.
pub(crate) fn target(db: SqlDb, tls: SqlTls, password: Option<String>) -> Result<Target, String> {
    let missing = |scheme: &str| line(msg::CHORE_PASSWORD_MISSING, &[scheme]);
    Ok(match db {
        SqlDb::SqlServer { user, host, port, database } => Target::SqlServer {
            host,
            port,
            database,
            auth: match user {
                None => SqlAuth::Windows,
                Some(user) => SqlAuth::Login { user, password: password.ok_or_else(|| missing("sqlserver://"))? },
            },
            trust_cert: tls == SqlTls::Trust,
        },
        SqlDb::Postgres { user, host, port, database } => Target::Postgres {
            host,
            port,
            database,
            user,
            password: password.ok_or_else(|| missing("postgres://"))?,
            tls: match tls {
                SqlTls::Verify => Tls::Verify,
                SqlTls::Trust => Tls::Require,
                SqlTls::Off => Tls::Disable,
            },
        },
        SqlDb::Sqlite { path } => {
            let path = PathBuf::from(path);
            // A file that is not there is refused here, with its own message; `Existing` also makes ito-sql refuse it
            // (and never create it), so a path written wrong never quietly gets an empty database.
            if !path.is_file() {
                return Err(line(msg::SQL_DB_FILE_MISSING, &[&path.display().to_string()]));
            }
            Target::Sqlite { path, open: SqliteOpen::Existing }
        }
    })
}

/// Where the SQL runs, for the log: `db01:1433/売上`, `C:\data\app.db`.
pub(crate) fn place(target: &Target) -> String {
    match target {
        Target::SqlServer { host, port, database, .. } | Target::Postgres { host, port, database, .. } => {
            format!("{host}:{port}/{database}")
        }
        Target::Sqlite { path, .. } => path.display().to_string(),
    }
}

/// Why the run did not end well, in the words of the log: an error with its ID
/// that names `place`, or (a stop, a timeout) a note led by it.
fn why(e: &ito_sql::Error, target: &Target, tls: SqlTls, place: &str) -> String {
    match e {
        ito_sql::Error::Connect { kind, text } => {
            let id = match (target, kind) {
                (Target::SqlServer { .. }, ConnectKind::Tls) if tls == SqlTls::Verify => msg::SQL_CONNECT_TRUST,
                (Target::Postgres { .. }, ConnectKind::Tls) if tls != SqlTls::Off => msg::SQL_CONNECT_TLS,
                (Target::Sqlite { path, .. }, ConnectKind::Missing) => {
                    return line(msg::SQL_DB_FILE_MISSING, &[&path.display().to_string()]);
                }
                _ => msg::SQL_CONNECT,
            };
            line(id, &[place, text])
        }
        ito_sql::Error::Sql(s) => line(msg::SQL_REFUSED, &[place, s]),
        ito_sql::Error::Cancelled => format!("{place}: {}", note(msg::N_STOPPED_SQL, &[])),
        ito_sql::Error::Timeout => format!("{place}: {}", note(msg::N_SQL_TIMEOUT, &[])),
        ito_sql::Error::Unsupported(s) => line(msg::SQL_UNSUPPORTED, &[place, s]),
        ito_sql::Error::Io(s) => line(msg::SQL_OUT_WRITE, &[place, s]),
    }
}

/// The text of a SQL file: UTF-8 (with or without a BOM), UTF-16 with a BOM
/// (SSMS's 「Unicode」), else Shift_JIS.
fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8_lossy(rest).into_owned();
    }
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| unit(*c)).collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        return utf16(rest, u16::from_be_bytes);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => ito_proc::Encoding::ShiftJis.decode(bytes),
    }
}

/// The first [`SHOWN`] lines of the CSV at `path`, and `…` when it has more.
fn preview(path: &Path) -> Vec<String> {
    let Ok(f) = std::fs::File::open(path) else { return Vec::new() };
    let mut lines: Vec<String> = std::io::BufReader::new(f).lines().map_while(Result::ok).take(SHOWN + 1).collect();
    if let Some(first) = lines.first_mut() {
        if let Some(rest) = first.strip_prefix('\u{feff}') {
            *first = rest.to_owned();
        }
    }
    if lines.len() > SHOWN {
        lines.truncate(SHOWN);
        lines.push("…".into());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Dir {
            let dir = std::env::temp_dir().join(format!("koyomi-sql-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Dir(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn lite(db: &Path, sql: Option<&str>, file: Option<&Path>, out: Option<&Path>, rows: Option<&str>) -> Chore {
        Chore::Sql {
            database: None,
            db: format!("sqlite://{}", db.display()),
            password: None,
            sql: sql.map(Into::into),
            file: file.map(|f| f.display().to_string()),
            out: out.map(|o| o.display().to_string()),
            rows: rows.map(Into::into),
            tls: SqlTls::Verify,
        }
    }

    #[test]
    fn a_sql_runs_on_sqlite_writes_its_result_and_judges_its_rows() {
        let dir = Dir::new("run");
        let db = dir.0.join("app.db");
        let no = AtomicBool::new(false);
        // A file that is not there is not made.
        let e = go(&lite(&db, Some("select 1"), None, None, None), None, &no).unwrap_err();
        assert!(msg::SQL_DB_FILE_MISSING.is_in(&e), "{e}");
        std::fs::write(&db, b"").unwrap();
        let make = "create table log (id integer, msg text);\ninsert into log values (1, 'a'), (2, 'b'), (3, 'c');";
        assert_eq!(go(&lite(&db, Some(make), None, None, Some("3")), None, &no), Ok(0));
        // The rows changed are judged when there is no result.
        let e = go(&lite(&db, Some("delete from log where id > 5"), None, None, Some("1..")), None, &no).unwrap_err();
        assert!(msg::SQL_ROWS_MISMATCH.is_in(&e) && e.contains(&note(msg::N_SQL_CHANGED_ROWS, &[])) && e.contains("rows = 1.."), "{e}");
        // A result goes to out, its folder made; its rows are judged.
        let out = dir.0.join("deep").join("log.csv");
        let file = dir.0.join("q.sql");
        std::fs::write(&file, "\u{feff}select id, msg from log order by id;\n".as_bytes()).unwrap();
        assert_eq!(go(&lite(&db, None, Some(&file), Some(&out), Some("1..")), None, &no), Ok(0));
        let csv = std::fs::read_to_string(&out).unwrap();
        assert!(csv.contains("id,msg\r\n1,a\r\n2,b\r\n3,c"), "{csv:?}");
        assert_eq!(preview(&out), ["id,msg", "1,a", "2,b", "3,c"]);
        let e = go(&lite(&db, Some("select * from log"), None, None, Some("..2")), None, &no).unwrap_err();
        assert!(msg::SQL_ROWS_MISMATCH.is_in(&e) && e.contains(&note(msg::N_SQL_RESULT_ROWS, &[])) && e.contains("3 行"), "{e}");
        // The database's own words.
        let e = go(&lite(&db, Some("select * from nowhere"), None, None, None), None, &no).unwrap_err();
        assert!(msg::SQL_REFUSED.is_in(&e) && e.contains("nowhere"), "{e}");
        // A stop before it runs.
        let stop = AtomicBool::new(true);
        assert!(go(&lite(&db, Some("select 1"), None, None, None), None, &stop).unwrap_err().contains(&note(msg::N_STOPPED_SQL, &[])));
        // Checked again as filled in.
        assert!(go(&lite(&db, Some(" "), None, None, None), None, &no)
            .unwrap_err()
            .contains(&koyomi_core::msg::line(koyomi_core::msg::CHORE_KEY_EMPTY, &["sql"])));
    }

    #[test]
    fn a_sql_file_reads_as_utf8_utf16_or_shift_jis() {
        assert_eq!(decode("\u{feff}select '売上'".as_bytes()), "select '売上'");
        assert_eq!(decode("select '売上'".as_bytes()), "select '売上'");
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("select '売'".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(decode(&le), "select '売'");
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("select 1".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(decode(&be), "select 1");
        // 「売上」 in Shift_JIS.
        assert_eq!(decode(b"select '\x94\x84\x8f\xe3'"), "select '売上'");
    }

    #[test]
    fn a_long_result_shows_its_first_lines() {
        let dir = Dir::new("preview");
        let path = dir.0.join("a.csv");
        let text: String = (0..30).map(|i| format!("{i}\r\n")).collect();
        std::fs::write(&path, text).unwrap();
        let shown = preview(&path);
        assert_eq!(shown.len(), SHOWN + 1);
        assert_eq!((shown[0].as_str(), shown[SHOWN].as_str()), ("0", "…"));
        assert!(preview(&dir.0.join("none.csv")).is_empty());
    }

    #[test]
    fn a_failure_to_connect_says_what_tls_to_try() {
        let server = Target::SqlServer { host: "db01".into(), port: 1433, database: "x".into(), auth: SqlAuth::Windows, trust_cert: false };
        let e = ito_sql::Error::Connect { kind: ConnectKind::Tls, text: "certificate".into() };
        let said = why(&e, &server, SqlTls::Verify, "db01");
        assert!(msg::SQL_CONNECT_TRUST.is_in(&said) && said.contains("tls = \"trust\""), "{said}");
        let said = why(&e, &server, SqlTls::Trust, "db01");
        assert!(msg::SQL_CONNECT.is_in(&said) && !said.contains("tls ="), "{said}");
        let pg = Target::Postgres {
            host: "pg".into(),
            port: 5432,
            database: "app".into(),
            user: "app".into(),
            password: "pw".into(),
            tls: Tls::Verify,
        };
        let said = why(&e, &pg, SqlTls::Verify, "pg");
        assert!(msg::SQL_CONNECT_TLS.is_in(&said) && said.contains("tls = \"off\""), "{said}");
        assert_eq!(place(&pg), "pg:5432/app");
        // tls = "trust" on PostgreSQL does not fall back to plain text either.
        let said = why(&e, &pg, SqlTls::Trust, "pg");
        assert!(msg::SQL_CONNECT_TLS.is_in(&said), "{said}");
        let said = why(&e, &pg, SqlTls::Off, "pg");
        assert!(msg::SQL_CONNECT.is_in(&said), "{said}");
        // A sign-in refusal is not a TLS problem, whatever the TLS setting.
        let refused = ito_sql::Error::Connect { kind: ConnectKind::SignIn, text: "bad password".into() };
        let said = why(&refused, &pg, SqlTls::Verify, "pg");
        assert!(msg::SQL_CONNECT.is_in(&said) && !msg::SQL_CONNECT_TLS.is_in(&said), "{said}");
        let said = why(&refused, &server, SqlTls::Verify, "db01");
        assert!(msg::SQL_CONNECT.is_in(&said) && !msg::SQL_CONNECT_TRUST.is_in(&said), "{said}");
        // A SQLite file that went away after the pre-check.
        let lite = Target::Sqlite { path: "C:\\data\\gone.db".into(), open: SqliteOpen::Existing };
        let gone = ito_sql::Error::Connect { kind: ConnectKind::Missing, text: "unable to open".into() };
        let said = why(&gone, &lite, SqlTls::Off, "C:\\data\\gone.db");
        assert!(msg::SQL_DB_FILE_MISSING.is_in(&said) && said.contains("gone.db"), "{said}");
        assert_eq!(why(&ito_sql::Error::Timeout, &pg, SqlTls::Off, "pg"), format!("pg: {}", note(msg::N_SQL_TIMEOUT, &[])));
    }
}
