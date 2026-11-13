//! `DatabaseTest`: the engine connects to the database a settings form describes and runs a probe that changes
//! nothing, for the form's 接続テスト. `prepare` runs on the engine loop (no network, no credential store); `run` runs
//! on a thread of its own (`slots`), where the saved password is read and the connection is made.
//!
//! The password never goes into a message or a log. A typed one is used literally (a value that starts with `secret:` is
//! not resolved). A saved one is used only for the server, port and user it was saved for, and for a TLS that is not
//! weaker. The server's own words reach the reply only when the password was typed in the same request (or there is none,
//! for SQLite); with the saved password the reply is a fixed line. The operations record keeps only the message ID.

use std::path::{Component, Path, Prefix};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use koyomi_core::chore::{check_db, sql_db, SqlDb, SqlTls};
use koyomi_core::msg::{self, line, note};
use koyomi_sql::{ConnectKind, Error, Target};

use super::typed_password::value_ok;
use crate::proto::Hidden;
use crate::remote::SecretStore;

/// How many tests may run at once.
pub const SLOTS: usize = 2;

/// How long a test may take from its start: the store read, the file check and the connection together.
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// The longest text of the server that is shown, in characters.
const SAID_MAX: usize = 200;

/// What the probe does with a target: connects (signs in to a server, or reads a SQLite file's header) and ends with
/// `Ok` or with koyomi-sql's error. Injected so tests never connect.
pub type RunFn = Arc<dyn Fn(&Target, &AtomicBool, Duration) -> Result<(), Error> + Send + Sync>;

/// The probe as it really connects.
pub fn real() -> RunFn {
    Arc::new(koyomi_sql::check)
}

/// What the operations record keeps of a result: `ok`, or only the ID of the error (never the server's words).
pub fn detail(result: &Result<String, String>) -> String {
    match result {
        Ok(_) => "ok".to_owned(),
        Err(e) if msg::is_coded(e) => e[7..12].to_owned(),
        Err(_) => "failed".to_owned(),
    }
}

/// What a request asks for, as the loop reads it.
pub struct Ask<'a> {
    pub db: &'a str,
    pub tls: Option<SqlTls>,
    pub entry: Option<&'a str>,
    pub value: Option<&'a Hidden>,
}

/// Which password the worker uses.
enum Pass {
    /// SQLite: none.
    None,
    /// Typed in this request.
    Typed(Hidden),
    /// The secret of the saved `[[database]]`, read by the worker.
    Saved(String),
}

/// A test that passed the checks of the loop. It holds the password by necessity; it has no `Debug`.
pub struct Probe {
    db: SqlDb,
    tls: SqlTls,
    pass: Pass,
    /// `host:port/database` or the file; no user.
    place: String,
}

impl Probe {
    /// The place without the user, for the operations record.
    pub fn place(&self) -> &str {
        &self.place
    }
}

/// How strong a TLS setting is: verify, then trust, then off.
fn strength(tls: SqlTls) -> u8 {
    match tls {
        SqlTls::Verify => 2,
        SqlTls::Trust => 1,
        SqlTls::Off => 0,
    }
}

/// Whether a path may be read by the engine on a window's ask: the SQLite file a test opens, and the folder a listing
/// reads (`dir_list`). On Windows only a path with a drive (`C:\...`, `C:/...`) is taken: a network path written as one
/// (`\\server\share`, `/\server\share`), `\\?\`, `\\.\` and a root without a drive are refused, lexically. A mapped
/// network drive or a link inside the path is not seen here (the check is lexical; a listing's own checks do the rest).
/// Elsewhere any absolute path is taken.
pub(super) fn path_ok(path: &str) -> bool {
    let path = Path::new(path);
    if !path.is_absolute() {
        return false;
    }
    if !cfg!(windows) {
        return true;
    }
    matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)))
}

/// Checks `ask` and says what to run, or why not (a coded line). The order is fixed: it reads (`sql_db`, no `{` or `}`),
/// SQL Server is refused, the kind's own checks, then the password. `config` is the text of koyomi.toml, read only when
/// the saved password is asked for.
pub fn prepare(ask: &Ask, config: impl FnOnce() -> Result<String, String>) -> Result<Probe, String> {
    let text = ask.db.trim();
    let parsed = sql_db(text)?;
    if text.contains(['{', '}']) {
        return Err(line(msg::CFG_DATABASE_BRACES, &[ask.entry.map_or("-", str::trim)]));
    }
    let tls = ask.tls.unwrap_or_default();
    let (user, host, port, database) = match &parsed {
        SqlDb::SqlServer { .. } => return Err(line(msg::DB_TEST_KIND, &[])),
        SqlDb::Sqlite { path } => {
            check_db(text, None, tls)?;
            if !path_ok(path) {
                return Err(line(msg::DB_TEST_PATH_BAD, &[]));
            }
            if ask.value.is_some() {
                return Err(line(msg::DB_TEST_VALUE_BAD, &[]));
            }
            return Ok(Probe { db: parsed.clone(), tls, pass: Pass::None, place: path.clone() });
        }
        SqlDb::Postgres { user, host, port, database } => (user, host, *port, database),
    };
    // Only to see the rest of the checks (the user, the TLS): the placeholder is never used.
    check_db(text, Some("secret:x"), tls)?;
    let place = format!("{host}:{port}/{database}");
    let pass = if let Some(value) = ask.value {
        if !value_ok(value.expose()) {
            return Err(line(msg::DB_TEST_VALUE_BAD, &[]));
        }
        Pass::Typed(value.clone())
    } else if let Some(name) = ask.entry.map(str::trim) {
        let saved = koyomi_core::config::Config::parse(&config()?)?
            .database
            .into_iter()
            .find(|d| d.name == name)
            .ok_or_else(|| line(msg::REGISTRY_NAME_UNKNOWN, &["[[database]]", name]))?;
        let same = matches!(
            sql_db(&saved.db),
            Ok(SqlDb::Postgres { user: u, host: h, port: p, .. }) if u == *user && h.eq_ignore_ascii_case(host) && p == port
        );
        if !same || strength(tls) < strength(saved.tls()) {
            return Err(line(msg::DB_TEST_RETYPE, &[]));
        }
        match saved.secret() {
            Some(secret) => Pass::Saved(secret.trim().to_owned()),
            None => return Err(line(msg::DB_TEST_NO_PASSWORD, &[])),
        }
    } else {
        return Err(line(msg::DB_TEST_NO_PASSWORD, &[]));
    };
    Ok(Probe { db: parsed, tls, pass, place })
}

/// The server's words for the reply: the first line, control characters dropped, at most [`SAID_MAX`] characters. If the
/// password appears in it (before or after the cleaning), a fixed note takes its place.
fn said(s: &str, password: Option<&str>) -> String {
    let clean: String = s.lines().next().unwrap_or_default().chars().filter(|c| !c.is_control()).take(SAID_MAX).collect();
    match password.filter(|p| !p.is_empty()) {
        Some(p) if s.contains(p) || clean.contains(p) => note(msg::N_DB_TEST_WITHHELD, &[]),
        _ => clean,
    }
}

/// Runs the test on the worker's thread. `Ok` carries no text (the window says it in its own words). The deadline of
/// [`TIMEOUT`] (a ceiling) starts here and covers the store read and the file check too; `secrets.get` and the file check cannot be
/// cancelled, so a blocked one keeps the slot until it returns.
pub fn run(probe: Probe, secrets: &dyn SecretStore, probe_fn: &RunFn, cancel: &AtomicBool) -> Result<String, String> {
    run_within(probe, secrets, probe_fn, cancel, TIMEOUT)
}

/// [`run`] with the whole time given; the probe gets exactly what is left of it.
fn run_within(probe: Probe, secrets: &dyn SecretStore, probe_fn: &RunFn, cancel: &AtomicBool, budget: Duration) -> Result<String, String> {
    let start = Instant::now();
    let Probe { db, tls, pass, place } = probe;
    let timed_out = || line(msg::DB_TEST_TIMEOUT, &[&place, &budget.as_secs().to_string()]);
    // The password, held where it prints as `***`.
    let (password, server_text) = match pass {
        Pass::None => (None, true),
        Pass::Typed(v) => (Some(v), true),
        Pass::Saved(name) => match secrets.get(&name) {
            Err(_) => return Err(line(msg::DB_TEST_SECRET, &[&name])),
            Ok(Some(v)) if !v.is_empty() => (Some(Hidden::new(v)), false),
            Ok(_) => return Err(line(msg::DB_TEST_NO_PASSWORD, &[])),
        },
    };
    // A missing file is refused first (cheap and clear). `koyomi_sql::check` opens a SQLite file read-only and never
    // creates or changes the database file (a WAL-mode one may get its `-wal` / `-shm` side files made or touched), so
    // a file removed after this check is `Connect { Missing }`, mapped the same way below.
    if let SqlDb::Sqlite { path } = &db {
        if !Path::new(path).is_file() {
            return Err(line(msg::DB_TEST_FILE_MISSING, &[path]));
        }
    }
    let target = crate::sql::target(db, tls, password.as_ref().map(|p| p.expose().to_owned())).map_err(|e| {
        // The target is checked again; nothing in `e` is the password.
        line(msg::DB_TEST_CONNECT, &[&place, &said(msg::uncoded(&e), None)])
    })?;
    let left = budget.checked_sub(start.elapsed()).filter(|d| !d.is_zero()).ok_or_else(timed_out)?;
    let secret = password.as_ref().map(Hidden::expose);
    match probe_fn(&target, cancel, left) {
        Ok(()) => Ok(String::new()),
        Err(Error::Cancelled) => Err(line(msg::DB_TEST_STOPPED, &[])),
        Err(Error::Timeout) => Err(timed_out()),
        Err(Error::Sql(s)) if server_text => Err(line(msg::DB_TEST_REFUSED, &[&place, &said(&s, secret)])),
        Err(Error::Sql(_)) => Err(line(msg::DB_TEST_REFUSED_SAVED, &[&place])),
        Err(Error::Connect { kind: ConnectKind::Missing, .. }) if matches!(target, Target::Sqlite { .. }) => {
            Err(line(msg::DB_TEST_FILE_MISSING, &[&place]))
        }
        Err(Error::Connect { text: s, .. } | Error::Unsupported(s) | Error::Io(s)) if server_text => {
            Err(line(msg::DB_TEST_CONNECT, &[&place, &said(&s, secret)]))
        }
        Err(Error::Connect { .. } | Error::Unsupported(_) | Error::Io(_)) => Err(line(msg::DB_TEST_CONNECT_SAVED, &[&place])),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use koyomi_sql::SqliteOpen;

    use super::*;

    const PW: &str = "pw-not-to-show";

    fn no_config() -> Result<String, String> {
        panic!("koyomi.toml should not be read")
    }

    fn pg(db: &str, tls: Option<SqlTls>, entry: Option<&str>, value: Option<&Hidden>, config: &str) -> Result<Probe, String> {
        prepare(&Ask { db, tls, entry, value }, || Ok(config.to_owned()))
    }

    const SAVED: &str =
        "[[database]]\nname = \"main\"\ndb = \"postgres://app@Db.Example:5432/sales\"\npassword = \"secret:koyomi-db-main\"\n";

    #[derive(Default)]
    struct Mem {
        items: Mutex<BTreeMap<String, String>>,
        calls: Mutex<Vec<String>>,
        fail: bool,
    }

    impl SecretStore for Mem {
        fn get(&self, name: &str) -> Result<Option<String>, String> {
            self.calls.lock().unwrap().push(format!("get {name}"));
            if self.fail {
                return Err("store down".into());
            }
            Ok(self.items.lock().unwrap().get(name).cloned())
        }
        fn set(&self, _: &str, _: &str) -> Result<(), String> {
            panic!("a test never writes the store")
        }
        fn delete(&self, _: &str) -> Result<(), String> {
            panic!("a test never deletes from the store")
        }
    }

    /// The passwords a fake probe was given.
    type Log = Arc<Mutex<Vec<String>>>;

    /// A probe fn that records the password it was given, and answers `answer`.
    fn recording(answer: Result<(), Error>) -> (RunFn, Log) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let f: RunFn = Arc::new(move |target, _, _| {
            let password = match target {
                Target::Postgres { password, .. } => password.clone(),
                _ => String::new(),
            };
            log.lock().unwrap().push(password);
            answer.clone()
        });
        (f, seen)
    }

    fn connect(text: &str) -> Error {
        Error::Connect { kind: ConnectKind::Other, text: text.into() }
    }

    fn typed() -> Hidden {
        Hidden::new(PW.into())
    }

    #[test]
    fn the_checks_run_in_a_fixed_order() {
        let t = typed();
        // Unreadable first, braces next, then SQL Server, then the kind's own checks, then the password.
        let e = pg("mysql://x", None, None, Some(&t), "").err().unwrap();
        assert!(msg::CHORE_DB_SCHEME.is_in(&e), "{e}");
        let e = pg("postgres://u@h/d{x}", None, Some("main"), Some(&t), "").err().unwrap();
        assert!(msg::CFG_DATABASE_BRACES.is_in(&e), "{e}");
        let e = pg("sqlserver://u@h/d", Some(SqlTls::Off), None, None, "").err().unwrap();
        assert!(msg::DB_TEST_KIND.is_in(&e), "{e}");
        // The kind's own check comes before the password: a SQLite file with tls off, and no password asked.
        let e = pg("sqlite:///x.db", Some(SqlTls::Off), None, Some(&t), "").err().unwrap();
        assert!(msg::CHORE_SQLITE_NO.is_in(&e), "{e}");
        // A bad typed value comes after the kind's checks.
        let e = pg("postgres://u@h/d", None, None, Some(&Hidden::new(String::new())), "").err().unwrap();
        assert!(msg::DB_TEST_VALUE_BAD.is_in(&e), "{e}");
        // No password at all.
        let e = pg("postgres://u@h/d", None, None, None, "").err().unwrap();
        assert!(msg::DB_TEST_NO_PASSWORD.is_in(&e), "{e}");
    }

    #[test]
    fn a_typed_value_is_taken_as_typed_and_bad_values_are_refused_without_quoting_them() {
        let secret_like = Hidden::new("secret:abc".into());
        let p = pg("postgres://u@h/d", None, None, Some(&secret_like), "").ok().unwrap();
        assert!(matches!(&p.pass, Pass::Typed(v) if v.expose() == "secret:abc"));
        assert_eq!(p.place(), "h:5432/d");
        for bad in [String::new(), "x".repeat(super::super::typed_password::VALUE_MAX + 1), "a\0b".to_owned()] {
            let e = pg("postgres://u@h/d", None, None, Some(&Hidden::new(bad.clone())), "").err().unwrap();
            assert!(msg::DB_TEST_VALUE_BAD.is_in(&e) && (bad.is_empty() || !e.contains(&bad)), "{e}");
        }
        // The longest value is taken.
        assert!(pg("postgres://u@h/d", None, None, Some(&Hidden::new("x".repeat(1024))), "").is_ok());
    }

    #[test]
    fn the_saved_password_is_used_only_for_the_same_server_port_user_and_a_tls_not_weaker() {
        let ok = |db: &str, tls: Option<SqlTls>| {
            pg(db, tls, Some("main"), None, SAVED).map(|p| matches!(p.pass, Pass::Saved(ref s) if s == "koyomi-db-main"))
        };
        // The host differs in case only.
        assert_eq!(ok("postgres://app@db.example/sales", None), Ok(true));
        assert_eq!(ok("postgres://app@DB.EXAMPLE:5432/other", Some(SqlTls::Verify)), Ok(true));
        for (db, tls) in [
            ("postgres://app@other.example/sales", None),
            ("postgres://app@db.example:5433/sales", None),
            ("postgres://App@db.example/sales", None),
            ("postgres://app@db.example/sales", Some(SqlTls::Trust)),
            ("postgres://app@db.example/sales", Some(SqlTls::Off)),
        ] {
            let e = ok(db, tls).err().unwrap();
            assert!(msg::DB_TEST_RETYPE.is_in(&e), "{db} {tls:?}: {e}");
        }
        // A saved entry with a weaker TLS allows a stronger or equal one.
        let weak = "[[database]]\nname = \"main\"\ndb = \"postgres://app@db.example/sales\"\npassword = \"secret:s\"\ntls = \"trust\"\n";
        assert!(pg("postgres://app@db.example/sales", Some(SqlTls::Trust), Some("main"), None, weak).is_ok());
        assert!(pg("postgres://app@db.example/sales", None, Some("main"), None, weak).is_ok());
        let e = pg("postgres://app@db.example/sales", Some(SqlTls::Off), Some("main"), None, weak).err().unwrap();
        assert!(msg::DB_TEST_RETYPE.is_in(&e), "{e}");
        // An unknown entry; an entry that is not PostgreSQL; an entry without a password.
        let e = pg("postgres://app@db.example/sales", None, Some("nope"), None, SAVED).err().unwrap();
        assert!(msg::REGISTRY_NAME_UNKNOWN.is_in(&e), "{e}");
        let lite = "[[database]]\nname = \"main\"\ndb = \"sqlite:///x.db\"\n";
        let e = pg("postgres://app@db.example/sales", None, Some("main"), None, lite).err().unwrap();
        assert!(msg::DB_TEST_RETYPE.is_in(&e), "{e}");
        let none = "[[database]]\nname = \"main\"\ndb = \"postgres://app@db.example/sales\"\n";
        let e = pg("postgres://app@db.example/sales", None, Some("main"), None, none).err().unwrap();
        assert!(msg::DB_TEST_NO_PASSWORD.is_in(&e) || msg::CFG_DATABASE_BAD.is_in(&e), "{e}");
    }

    #[test]
    fn a_typed_value_wins_over_the_entry_and_koyomi_toml_is_not_read_for_it() {
        let t = typed();
        let p = prepare(&Ask { db: "postgres://u@h/d", tls: None, entry: Some("main"), value: Some(&t) }, no_config).ok().unwrap();
        assert!(matches!(p.pass, Pass::Typed(_)));
    }

    #[test]
    fn a_sql_server_row_is_refused_and_sqlite_takes_no_password_and_only_an_absolute_local_path() {
        let e = pg("sqlserver://u@h/d", None, None, Some(&typed()), "").err().unwrap();
        assert!(msg::DB_TEST_KIND.is_in(&e), "{e}");
        let dir = std::env::temp_dir();
        let abs = format!("sqlite://{}", dir.join("x.db").display());
        let e = pg(&abs, None, None, Some(&typed()), "").err().unwrap();
        assert!(msg::DB_TEST_VALUE_BAD.is_in(&e), "{e}");
        let p = pg(&abs, None, None, None, "").ok().unwrap();
        assert!(matches!(p.pass, Pass::None));
        if cfg!(windows) {
            for bad in [
                r"sqlite://\\server\share\x.db",
                r"sqlite:///\server\share\x.db",
                r"sqlite://\/server\share\x.db",
                r"sqlite://\\?\C:\x.db",
                r"sqlite://\\.\C:\x.db",
                "sqlite:////?/C:/x.db",
                "sqlite:////./C:/x.db",
                "sqlite:////server/share/x.db",
                r"sqlite://\x.db",
            ] {
                let e = pg(bad, None, None, None, "").err().unwrap();
                assert!(msg::DB_TEST_PATH_BAD.is_in(&e), "{bad}: {e}");
            }
            for good in [r"sqlite://C:\data\x.db", "sqlite://C:/data/x.db"] {
                assert!(pg(good, None, None, None, "").is_ok(), "{good}");
            }
        } else {
            assert!(pg("sqlite:///srv/x.db", None, None, None, "").is_ok());
        }
    }

    #[test]
    fn what_the_record_keeps_is_the_id_alone() {
        assert_eq!(detail(&Ok(String::new())), "ok");
        assert_eq!(detail(&Err(line(msg::DB_TEST_CONNECT, &["h:5432/d", "password authentication failed for user bob"]))), "E2267");
        assert_eq!(detail(&Err(line(msg::DB_TEST_STOPPED, &[]))), "E2271");
        assert_eq!(detail(&Err("plain".into())), "failed");
    }

    #[test]
    fn said_takes_the_first_line_without_control_characters_cut_to_200_and_withholds_a_password() {
        assert_eq!(said("first\nsecond", None), "first");
        assert_eq!(said("a\u{7}b\tc", None), "abc");
        assert_eq!(said(&"x".repeat(500), None).chars().count(), SAID_MAX);
        let withheld = note(msg::N_DB_TEST_WITHHELD, &[]);
        assert_eq!(said("login failed for pw-not-to-show here", Some(PW)), withheld);
        // On the second line too: the whole text is looked at.
        assert_eq!(said("login failed\nthe password was pw-not-to-show", Some(PW)), withheld);
        // Split by a control character, it is still found once cleaned.
        assert_eq!(said("pw-not-\u{7}to-show is wrong", Some("pw-not-to-show")), withheld);
        assert_eq!(said("no secret here", Some(PW)), "no secret here");
        assert_eq!(said("anything", Some("")), "anything");
    }

    fn refused_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    fn probe(db: &str, value: Option<&Hidden>, entry: Option<&str>, config: &str) -> Probe {
        pg(db, None, entry, value, config).ok().unwrap()
    }

    #[test]
    fn the_literal_secret_like_password_reaches_the_probe_unresolved() {
        let (f, seen) = recording(Ok(()));
        let store = Mem::default();
        let t = Hidden::new("secret:abc".into());
        let r = run(probe("postgres://u@h/d", Some(&t), None, ""), &store, &f, &AtomicBool::new(false));
        assert_eq!(r, Ok(String::new()));
        assert_eq!(*seen.lock().unwrap(), vec!["secret:abc".to_owned()]);
        assert!(store.calls.lock().unwrap().is_empty(), "a typed password reads no secret");
    }

    #[test]
    fn the_saved_secret_is_read_by_the_worker_and_a_missing_or_failing_one_is_told_without_the_store_words() {
        let (f, seen) = recording(Ok(()));
        let store = Mem::default();
        store.items.lock().unwrap().insert("koyomi-db-main".into(), "saved-pw".into());
        let mk = || probe("postgres://app@db.example/sales", None, Some("main"), SAVED);
        assert_eq!(run(mk(), &store, &f, &AtomicBool::new(false)), Ok(String::new()));
        assert_eq!(seen.lock().unwrap()[0], "saved-pw");
        assert_eq!(*store.calls.lock().unwrap(), vec!["get koyomi-db-main".to_owned()]);
        // Missing, empty.
        for value in [None, Some("")] {
            let store = Mem::default();
            if let Some(v) = value {
                store.items.lock().unwrap().insert("koyomi-db-main".into(), v.into());
            }
            let e = run(mk(), &store, &f, &AtomicBool::new(false)).unwrap_err();
            assert!(msg::DB_TEST_NO_PASSWORD.is_in(&e), "{e}");
        }
        let store = Mem { fail: true, ..Mem::default() };
        let e = run(mk(), &store, &f, &AtomicBool::new(false)).unwrap_err();
        assert!(msg::DB_TEST_SECRET.is_in(&e) && e.contains("koyomi-db-main") && !e.contains("store down"), "{e}");
        assert_eq!(seen.lock().unwrap().len(), 1, "the probe ran only for the good read");
    }

    #[test]
    fn errors_are_told_with_the_servers_words_only_for_a_typed_password() {
        let store = Mem::default();
        store.items.lock().unwrap().insert("koyomi-db-main".into(), "saved-pw".into());
        let said_by_server = "password authentication failed for user app";
        let cases = [
            (connect(said_by_server), msg::DB_TEST_CONNECT, msg::DB_TEST_CONNECT_SAVED),
            (Error::Sql(said_by_server.into()), msg::DB_TEST_REFUSED, msg::DB_TEST_REFUSED_SAVED),
            (Error::Unsupported(said_by_server.into()), msg::DB_TEST_CONNECT, msg::DB_TEST_CONNECT_SAVED),
            (Error::Io(said_by_server.into()), msg::DB_TEST_CONNECT, msg::DB_TEST_CONNECT_SAVED),
        ];
        for (error, typed_id, saved_id) in cases {
            let (f, _) = recording(Err(error));
            let t = typed();
            let e = run(probe("postgres://app@db.example/sales", Some(&t), None, ""), &store, &f, &AtomicBool::new(false)).unwrap_err();
            assert!(typed_id.is_in(&e) && e.contains(said_by_server) && e.contains("db.example:5432/sales"), "{e}");
            assert!(!e.contains(PW));
            let e =
                run(probe("postgres://app@db.example/sales", None, Some("main"), SAVED), &store, &f, &AtomicBool::new(false)).unwrap_err();
            assert!(saved_id.is_in(&e) && !e.contains(said_by_server) && !e.contains("saved-pw"), "{e}");
        }
        // The place never holds the user.
        let (f, _) = recording(Err(connect("x")));
        let t = typed();
        let e = run(probe("postgres://app@db.example/sales", Some(&t), None, ""), &store, &f, &AtomicBool::new(false)).unwrap_err();
        assert!(!e.contains("app@"), "{e}");
        // A server that repeats the typed password.
        let (f, _) = recording(Err(connect(&format!("bad login {PW}"))));
        let e = run(probe("postgres://app@db.example/sales", Some(&t), None, ""), &store, &f, &AtomicBool::new(false)).unwrap_err();
        assert!(!e.contains(PW) && e.contains(&note(msg::N_DB_TEST_WITHHELD, &[])), "{e}");
        // A timeout and a cancel.
        let (f, _) = recording(Err(Error::Timeout));
        let e = run(probe("postgres://app@db.example/sales", Some(&t), None, ""), &store, &f, &AtomicBool::new(false)).unwrap_err();
        assert!(msg::DB_TEST_TIMEOUT.is_in(&e) && e.contains("15"), "{e}");
        let (f, _) = recording(Err(Error::Cancelled));
        let e = run(probe("postgres://app@db.example/sales", Some(&t), None, ""), &store, &f, &AtomicBool::new(false)).unwrap_err();
        assert!(msg::DB_TEST_STOPPED.is_in(&e), "{e}");
    }

    #[test]
    fn the_probe_gets_the_rest_of_the_time() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let f: RunFn = Arc::new(move |_, _, left| {
            log.lock().unwrap().push(left);
            Ok(())
        });
        let t = typed();
        run(probe("postgres://u@h/d", Some(&t), None, ""), &Mem::default(), &f, &AtomicBool::new(false)).unwrap();
        let left = seen.lock().unwrap()[0];
        assert!(left <= TIMEOUT && left > TIMEOUT - Duration::from_secs(2), "{left:?}");
    }

    #[test]
    fn a_saved_password_read_that_outlasts_the_budget_times_out_and_the_probe_is_never_called() {
        struct Slow;
        impl SecretStore for Slow {
            fn get(&self, _: &str) -> Result<Option<String>, String> {
                std::thread::sleep(Duration::from_millis(120));
                Ok(Some("saved-pw".into()))
            }
            fn set(&self, _: &str, _: &str) -> Result<(), String> {
                panic!("a test never writes the store")
            }
            fn delete(&self, _: &str) -> Result<(), String> {
                panic!("a test never deletes from the store")
            }
        }
        let (f, seen) = recording(Ok(()));
        let p = probe("postgres://app@db.example/sales", None, Some("main"), SAVED);
        let e = run_within(p, &Slow, &f, &AtomicBool::new(false), Duration::from_millis(50)).unwrap_err();
        assert!(msg::DB_TEST_TIMEOUT.is_in(&e), "{e}");
        assert!(seen.lock().unwrap().is_empty(), "the probe must not be called when no time is left");
    }

    #[test]
    fn a_sqlite_file_that_is_missing_is_refused_and_not_created() {
        let dir = std::env::temp_dir().join(format!("koyomi-dbtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("none.db");
        let (f, seen) = recording(Ok(()));
        let r = run(probe(&format!("sqlite://{}", file.display()), None, None, ""), &Mem::default(), &f, &AtomicBool::new(false));
        let e = r.unwrap_err();
        assert!(msg::DB_TEST_FILE_MISSING.is_in(&e), "{e}");
        assert!(!file.exists(), "a test must not create the file");
        assert!(seen.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_sqlite_file_gone_after_the_pre_check_is_the_missing_file_error() {
        let dir = std::env::temp_dir().join(format!("koyomi-dbtest-gone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gone.db");
        std::fs::write(&file, b"").unwrap();
        let gone = file.clone();
        // The file is removed after the pre-check passed, as the probe starts.
        let f: RunFn = Arc::new(move |_, _, _| {
            let _ = std::fs::remove_file(&gone);
            Err(Error::Connect { kind: ConnectKind::Missing, text: "unable to open database file".into() })
        });
        let r = run(probe(&format!("sqlite://{}", file.display()), None, None, ""), &Mem::default(), &f, &AtomicBool::new(false));
        let e = r.unwrap_err();
        assert!(msg::DB_TEST_FILE_MISSING.is_in(&e), "{e}");
        // A missing database on a server is not the file error.
        let (f, _) = recording(Err(Error::Connect { kind: ConnectKind::Missing, text: "database does not exist".into() }));
        let t = typed();
        let e =
            run(probe("postgres://app@db.example/sales", Some(&t), None, ""), &Mem::default(), &f, &AtomicBool::new(false)).unwrap_err();
        assert!(msg::DB_TEST_CONNECT.is_in(&e), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_real_check_never_creates_a_sqlite_file() {
        let dir = std::env::temp_dir().join(format!("koyomi-dbtest-nocreate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("none.db");
        let target = Target::Sqlite { path: file.clone(), open: SqliteOpen::Create };
        let e = real()(&target, &AtomicBool::new(false), Duration::from_secs(5)).unwrap_err();
        assert!(matches!(e, Error::Connect { kind: ConnectKind::Missing, .. }), "{e:?}");
        assert!(!file.exists(), "the check must not create the file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_real_sqlite_file_passes_an_empty_file_passes_and_garbage_does_not() {
        let dir = std::env::temp_dir().join(format!("koyomi-dbtest-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let url = |name: &str| format!("sqlite://{}", dir.join(name).display());
        let real = real();
        let never = AtomicBool::new(false);
        // A real database, made by koyomi-sql itself.
        let made = dir.join("ok.db");
        let target = Target::Sqlite { path: made.clone(), open: SqliteOpen::Create };
        koyomi_sql::run(&target, "CREATE TABLE t (a INTEGER)", None, &never, Duration::from_secs(5)).unwrap();
        assert_eq!(run(probe(&url("ok.db"), None, None, ""), &Mem::default(), &real, &never), Ok(String::new()));
        // An empty file is an empty database.
        std::fs::write(dir.join("empty.db"), b"").unwrap();
        assert_eq!(run(probe(&url("empty.db"), None, None, ""), &Mem::default(), &real, &never), Ok(String::new()));
        // Garbage that is not a database.
        std::fs::write(dir.join("junk.db"), b"this is not a database file at all, only text, long enough to be a header").unwrap();
        let e = run(probe(&url("junk.db"), None, None, ""), &Mem::default(), &real, &never).unwrap_err();
        assert!(msg::DB_TEST_CONNECT.is_in(&e), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn postgres_on_a_closed_loopback_port_cannot_connect() {
        let port = refused_port();
        let t = typed();
        let db = format!("postgres://u@127.0.0.1:{port}/d");
        let mut p = probe(&db, Some(&t), None, "");
        p.tls = SqlTls::Off;
        let e = run(p, &Mem::default(), &real(), &AtomicBool::new(false)).unwrap_err();
        assert!(msg::DB_TEST_CONNECT.is_in(&e) && !e.contains(PW), "{e}");
    }
}
