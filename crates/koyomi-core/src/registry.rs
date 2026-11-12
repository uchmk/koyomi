//! Accounts and databases registered once in koyomi.toml (`[[account]]`, `[[database]]`) and named
//! by a job (`run_as = "batch"`, `database = "sales"`), so the secret's name and the address are
//! not written job by job. The engine reads the entry when a job starts.
//!
//! This holds the entries, the one rule for a name, the connection's environment a `sql` job
//! gets, the names a net uses, and the edits of koyomi.toml that add, change and remove an entry
//! (comments and everything else are kept: `ito_common::set_tables`).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::chore::{Chore, SqlDb, SqlTls, MAIL_TEXT};
use crate::config::Config;
use crate::jobnet::{self, Account, Jobnet, RunAs};
use crate::msg;

/// Whether `name` may name an `[[account]]` or a `[[database]]`, and be written by a job as
/// `run_as = "…"` or `database = "…"`: as it stands (no space around it), a name `calendar::valid_name`
/// takes, and no `{` or `}` (a name is not a template). Case counts.
pub fn valid_name(name: &str) -> bool {
    name == name.trim() && crate::calendar::valid_name(name) && !name.contains(['{', '}'])
}

/// `[[account]]`: a logon registered once. `password` is `secret:<name>`, never the password.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountEntry {
    pub name: String,
    /// `DOMAIN\user`, `user@domain` or `user`.
    pub user: String,
    /// `secret:<name>`.
    pub password: String,
}

impl AccountEntry {
    /// The name of the secret, without `secret:`; `None` when `password` is not that form (or names nothing).
    pub fn secret(&self) -> Option<&str> {
        self.password.strip_prefix("secret:").filter(|n| !n.trim().is_empty())
    }

    /// The logon a job runs as. `None` when `password` is not a `secret:<name>`.
    pub fn account(&self) -> Option<Account> {
        Some(Account { user: self.user.clone(), secret: self.secret()?.to_owned() })
    }

    /// Why it may not be used: a name that is not one, a `user` a logon would not read, a `password` that
    /// is not `secret:<name>`. A coded line.
    pub fn check(&self) -> Result<(), String> {
        if !valid_name(&self.name) {
            return Err(msg::line(msg::CFG_ENTRY_NAME_BAD, &["[[account]]", &self.name]));
        }
        if !jobnet::valid_account(&self.user) {
            return Err(msg::line(msg::CFG_ACCOUNT_USER, &[&self.name]));
        }
        if self.secret().is_none() {
            return Err(msg::line(msg::CFG_ENTRY_PASSWORD, &["[[account]]", &self.name]));
        }
        Ok(())
    }
}

/// `[[database]]`: the address of a database (and its login) registered once. `password` is
/// `secret:<name>` and, like `tls`, follows the rules of a `sql` job's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseEntry {
    pub name: String,
    /// `sqlserver://[user@]host[:port]/database`, `postgres://user@host[:port]/database` or `sqlite://C:\data\app.db`.
    pub db: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<SqlTls>,
}

impl DatabaseEntry {
    /// The name of the secret, without `secret:`.
    pub fn secret(&self) -> Option<&str> {
        self.password.as_deref().map(str::trim).and_then(|p| p.strip_prefix("secret:")).filter(|n| !n.trim().is_empty())
    }

    /// The TLS as the entry says it, `verify` when it does not.
    pub fn tls(&self) -> SqlTls {
        self.tls.unwrap_or_default()
    }

    /// The kind of server `db` points to: `sqlserver`, `postgres` or `sqlite`; `None` if it does not read.
    pub fn kind(&self) -> Option<&'static str> {
        db_kind(&self.db)
    }

    /// The `KOYOMI_SQL_DB`, `_TLS` and `_PASSWORD` a job that names it gets.
    pub fn env(&self) -> Vec<(String, String)> {
        sql_target_env(&self.db, self.password.as_deref(), self.tls())
    }

    /// Why it may not be used: a name that is not one, a `{` in `db` (an entry is not a template), or
    /// whatever [`crate::chore::check_db`] says of the connection. A coded line.
    pub fn check(&self) -> Result<(), String> {
        if !valid_name(&self.name) {
            return Err(msg::line(msg::CFG_ENTRY_NAME_BAD, &["[[database]]", &self.name]));
        }
        if self.db.contains(['{', '}']) {
            return Err(msg::line(msg::CFG_DATABASE_BRACES, &[&self.name]));
        }
        crate::chore::check_db(&self.db, self.password.as_deref(), self.tls())
            .map_err(|e| msg::line(msg::CFG_DATABASE_BAD, &[&self.name, msg::uncoded(&e)]))
    }
}

/// `sqlserver`, `postgres` or `sqlite`, from a `db`'s scheme; `None` if it does not read.
pub fn db_kind(db: &str) -> Option<&'static str> {
    Some(match crate::chore::sql_db(db).ok()? {
        SqlDb::SqlServer { .. } => "sqlserver",
        SqlDb::Postgres { .. } => "postgres",
        SqlDb::Sqlite { .. } => "sqlite",
    })
}

/// The environment that tells `koyomi sql run` where to connect: `KOYOMI_SQL_DB` (after [`MAIL_TEXT`]),
/// `KOYOMI_SQL_TLS`, and `KOYOMI_SQL_PASSWORD` (the `secret:<name>` text, read when the job starts) when
/// there is a password. A `sql` job's own `db` and a `[[database]]` give the same pairs.
pub fn sql_target_env(db: &str, password: Option<&str>, tls: SqlTls) -> Vec<(String, String)> {
    let mut v =
        vec![("KOYOMI_SQL_DB".to_owned(), format!("{MAIL_TEXT}{}", db.trim())), ("KOYOMI_SQL_TLS".to_owned(), tls.key().to_owned())];
    if let Some(p) = password.map(str::trim).filter(|p| !p.is_empty()) {
        v.push(("KOYOMI_SQL_PASSWORD".to_owned(), p.to_owned()));
    }
    v
}

/// The names of the registered accounts and databases a net uses, as `(accounts, databases)`: a job's
/// `run_as = "…"` and a `sql`'s `database = "…"`. A job that only calls another net (`kind = "jobnet"`)
/// uses nothing itself.
pub fn names_used(net: &Jobnet) -> (BTreeSet<String>, BTreeSet<String>) {
    let (mut accounts, mut databases) = (BTreeSet::new(), BTreeSet::new());
    for j in &net.jobs {
        if let Some(RunAs::Named(n)) = &j.run_as {
            accounts.insert(n.clone());
        }
        if let crate::jobnet::Part::Chore(Chore::Sql { database: Some(n), .. }) = &j.part {
            databases.insert(n.clone());
        }
    }
    (accounts, databases)
}

/// The cautions (W3754, W3755) for the names `net` (called `id`) gives that `config` does not have: one
/// line per name, with the first job that gives it. On every OS, since the names are all it shows.
pub fn unknown_lines(config: &Config, id: &str, net: &Jobnet) -> Vec<String> {
    let mut out = Vec::new();
    let mut told: BTreeSet<(bool, String)> = BTreeSet::new();
    for j in &net.jobs {
        if let Some(RunAs::Named(n)) = &j.run_as {
            if !config.account.iter().any(|a| a.name == *n) && told.insert((true, n.clone())) {
                out.push(msg::line(msg::OS_ACCOUNT_UNKNOWN, &[id, &j.id, n]));
            }
        }
        if let crate::jobnet::Part::Chore(Chore::Sql { database: Some(n), .. }) = &j.part {
            if !config.database.iter().any(|d| d.name == *n) && told.insert((false, n.clone())) {
                out.push(msg::line(msg::OS_DATABASE_UNKNOWN, &[id, &j.id, n]));
            }
        }
    }
    out
}

fn quote(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

fn account_block(e: &AccountEntry) -> Vec<(&'static str, String)> {
    vec![("name", quote(&e.name)), ("user", quote(&e.user)), ("password", quote(&e.password))]
}

fn database_block(e: &DatabaseEntry) -> Vec<(&'static str, String)> {
    let mut b = vec![("name", quote(&e.name)), ("db", quote(&e.db))];
    if let Some(p) = &e.password {
        b.push(("password", quote(p)));
    }
    if let Some(t) = e.tls {
        b.push(("tls", quote(t.key())));
    }
    b
}

fn unknown(table: &str, name: &str) -> String {
    msg::line(msg::REGISTRY_NAME_UNKNOWN, &[table, name])
}

/// `list` with `entry` in it: replacing the one of its name, else after the last.
fn with<T: Clone>(list: &[T], entry: &T, same: impl Fn(&T) -> bool) -> Vec<T> {
    let mut out = list.to_vec();
    match out.iter().position(same) {
        Some(i) => out[i] = entry.clone(),
        None => out.push(entry.clone()),
    }
    out
}

/// `text` with the `[[account]]` `entry` added, or changed if one has its name. Everything else stays
/// as it was. Refused: an entry that does not check, and a `text` that does not read.
pub fn put_account(text: &str, entry: &AccountEntry) -> Result<String, String> {
    entry.check()?;
    let now = Config::parse(text)?.account;
    let all = with(&now, entry, |a| a.name == entry.name);
    let blocks: Vec<_> = all.iter().map(account_block).collect();
    ito_common::set_tables(text, "account", &blocks)
}

/// `text` without the `[[account]]` `name`. Refused: no such account.
pub fn remove_account(text: &str, name: &str) -> Result<String, String> {
    let now = Config::parse(text)?.account;
    if !now.iter().any(|a| a.name == name) {
        return Err(unknown("[[account]]", name));
    }
    let blocks: Vec<_> = now.iter().filter(|a| a.name != name).map(account_block).collect();
    ito_common::set_tables(text, "account", &blocks)
}

/// `text` with the `[[database]]` `entry` added, or changed if one has its name.
pub fn put_database(text: &str, entry: &DatabaseEntry) -> Result<String, String> {
    entry.check()?;
    let now = Config::parse(text)?.database;
    let all = with(&now, entry, |d| d.name == entry.name);
    let blocks: Vec<_> = all.iter().map(database_block).collect();
    ito_common::set_tables(text, "database", &blocks)
}

/// `text` without the `[[database]]` `name`. Refused: no such database.
pub fn remove_database(text: &str, name: &str) -> Result<String, String> {
    let now = Config::parse(text)?.database;
    if !now.iter().any(|d| d.name == name) {
        return Err(unknown("[[database]]", name));
    }
    let blocks: Vec<_> = now.iter().filter(|d| d.name != name).map(database_block).collect();
    ito_common::set_tables(text, "database", &blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(name: &str) -> AccountEntry {
        AccountEntry { name: name.into(), user: r"CORP\batch".into(), password: format!("secret:{name}") }
    }

    fn database(name: &str, db: &str, password: Option<&str>, tls: Option<SqlTls>) -> DatabaseEntry {
        DatabaseEntry { name: name.into(), db: db.into(), password: password.map(Into::into), tls }
    }

    #[test]
    fn a_name_is_plain_trimmed_and_not_a_template() {
        for ok in ["batch", "sales-db", "売上", "A b", "a.b"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", " ", " batch", "batch ", "{x}", "a{", "a}", "a/b", "a\\b", ".", "..", "_x", "a.", "con", "a:b"] {
            assert!(!valid_name(bad), "{bad:?}");
        }
        // Case counts: two names that differ in case are two names.
        assert!(valid_name("Batch") && valid_name("batch"));
    }

    #[test]
    fn the_connection_environment_is_one_function() {
        let v = sql_target_env(" sqlserver://ops@db01/x ", Some(" secret:db "), SqlTls::Trust);
        assert_eq!(
            v,
            [
                ("KOYOMI_SQL_DB".to_owned(), "|sqlserver://ops@db01/x".to_owned()),
                ("KOYOMI_SQL_TLS".to_owned(), "trust".to_owned()),
                ("KOYOMI_SQL_PASSWORD".to_owned(), "secret:db".to_owned()),
            ]
        );
        assert_eq!(sql_target_env("sqlite://C:\\a.db", Some(" "), SqlTls::Verify).len(), 2);
        let e = database("a", "postgres://app@pg/app", Some("secret:pg"), None);
        assert_eq!((e.kind(), e.secret(), e.tls()), (Some("postgres"), Some("pg"), SqlTls::Verify));
        assert_eq!(e.env(), sql_target_env("postgres://app@pg/app", Some("secret:pg"), SqlTls::Verify));
    }

    #[test]
    fn an_entry_that_cannot_be_used_is_refused_with_its_reason() {
        let bad = |e: &AccountEntry| e.check().unwrap_err();
        assert!(account("batch").check().is_ok());
        assert!(msg::CFG_ENTRY_NAME_BAD.is_in(&bad(&account("a{b}"))));
        assert!(msg::CFG_ACCOUNT_USER.is_in(&bad(&AccountEntry { user: "a b".into(), ..account("x") })));
        assert!(msg::CFG_ENTRY_PASSWORD.is_in(&bad(&AccountEntry { password: "hunter2".into(), ..account("x") })));
        assert!(msg::CFG_ENTRY_PASSWORD.is_in(&bad(&AccountEntry { password: "secret: ".into(), ..account("x") })));
        let dbad = |e: DatabaseEntry| e.check().unwrap_err();
        assert!(database("s", "sqlserver://db01/x", None, None).check().is_ok());
        assert!(msg::CFG_ENTRY_NAME_BAD.is_in(&dbad(database(" s", "sqlserver://db01/x", None, None))));
        assert!(msg::CFG_DATABASE_BRACES.is_in(&dbad(database("s", "sqlserver://{var:h}/x", None, None))));
        assert!(msg::CFG_DATABASE_BAD.is_in(&dbad(database("s", "sqlserver://ops@db01/x", None, None))));
        assert!(msg::CFG_DATABASE_BAD.is_in(&dbad(database("s", "sqlite://C:\\a.db", None, Some(SqlTls::Trust)))));
        assert!(msg::CFG_DATABASE_BAD.is_in(&dbad(database("s", "postgres://a@h/x", Some("hunter2"), None))));
    }

    #[test]
    fn put_and_remove_keep_the_comments_and_the_rest() {
        let text = "# the settings\nconcurrency = 2\n\n# batch logon\n[[account]]\nname = \"batch\"\nuser = 'CORP\\batch'\npassword = \"secret:batch\" # kept\n";
        let out = put_account(text, &account("ops")).unwrap();
        assert!(out.starts_with("# the settings\nconcurrency = 2\n"), "{out}");
        assert!(out.contains("# batch logon") && out.contains("# kept"), "{out}");
        let c = Config::parse(&out).unwrap();
        assert_eq!(c.account.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["batch", "ops"]);
        // Changing keeps the place and the comment.
        let changed = put_account(&out, &AccountEntry { user: "CORP\\other".into(), ..account("batch") }).unwrap();
        assert!(changed.contains("# kept"), "{changed}");
        let c = Config::parse(&changed).unwrap();
        assert_eq!(c.account[0].user, "CORP\\other");
        assert_eq!(c.account.len(), 2);
        let gone = remove_account(&changed, "ops").unwrap();
        assert_eq!(Config::parse(&gone).unwrap().account.len(), 1);
        assert!(gone.contains("# the settings") && gone.contains("# kept"));
        assert!(msg::REGISTRY_NAME_UNKNOWN.is_in(&remove_account(&gone, "ops").unwrap_err()));

        let d = put_database("", &database("sales", "sqlserver://ops@db01/sales", Some("secret:sales-db"), Some(SqlTls::Trust))).unwrap();
        let c = Config::parse(&d).unwrap();
        assert_eq!(c.database.len(), 1);
        assert_eq!((c.database[0].tls, c.database[0].password.as_deref()), (Some(SqlTls::Trust), Some("secret:sales-db")));
        let d = put_database(&d, &database("sales", "sqlserver://ops@db02/sales", Some("secret:sales-db"), None)).unwrap();
        let c = Config::parse(&d).unwrap();
        assert_eq!((c.database.len(), c.database[0].db.as_str(), c.database[0].tls), (1, "sqlserver://ops@db02/sales", None));
        assert!(msg::REGISTRY_NAME_UNKNOWN.is_in(&remove_database(&d, "nope").unwrap_err()));
        let d = remove_database(&d, "sales").unwrap();
        assert!(Config::parse(&d).unwrap().database.is_empty());
        // An entry that does not check writes nothing.
        assert!(msg::CFG_ENTRY_NAME_BAD.is_in(&put_account("", &account("{x}")).unwrap_err()));
    }

    #[test]
    fn the_names_a_net_uses_are_its_own_not_a_called_nets() {
        let net = jobnet::parse(
            "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\nrun_as = \"batch\"\n[[job]]\nid = \"b\"\nrun = [\"y\"]\nrun_as = { user = \"u\", password = \"secret:s\" }\n[[job]]\nid = \"c\"\nkind = \"sql\"\ndatabase = \"sales\"\nsql = \"select 1\"\n[[job]]\nid = \"d\"\nkind = \"jobnet\"\nuse = \"g/child\"\n",
        )
        .unwrap_or_else(|p| panic!("{p:?}"));
        let (a, d) = names_used(&net);
        assert_eq!(
            (a.into_iter().collect::<Vec<_>>(), d.into_iter().collect::<Vec<_>>()),
            (vec!["batch".to_owned()], vec!["sales".to_owned()])
        );
    }
}
