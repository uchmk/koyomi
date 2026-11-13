//! Accounts and databases registered once in koyomi.toml (`[[account]]`, `[[database]]`) and named
//! by a job (`run_as = "batch"`, `database = "sales"`), so the secret's name and the address are
//! not written job by job. The engine reads the entry when a job starts.
//!
//! `[[host]]`, `[[bucket]]` and `[[mailbox]]` are the same for the places a job reaches: a PC to run
//! commands on or move files to (`on = "web1"`, `host = "web1"`), an S3 bucket (`bucket = "logs"`)
//! and a mailbox a `[[when]]` watches (`mailbox = "orders"`).
//!
//! This holds the entries, the one rule for a name, the connection's environment a `sql` job
//! gets, the names a net uses, and the edits of koyomi.toml that add, change and remove an entry
//! (comments and everything else are kept: `ito_common::set_tables`).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::chore::{Chore, SqlDb, SqlTls, WinrmAuth, MAIL_TEXT};
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

/// The two kinds of `[[host]]`: a PC reached over SSH (commands, SFTP) or over WinRM (commands).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKind {
    Ssh,
    Winrm,
}

impl HostKind {
    /// `ssh` or `winrm`.
    pub fn key(self) -> &'static str {
        match self {
            HostKind::Ssh => "ssh",
            HostKind::Winrm => "winrm",
        }
    }
}

/// `value` as a trimmed text when it is written (not empty).
fn written(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|x| !x.is_empty())
}

/// A `secret:<name>` password: `Some(true)` when it is one, `Some(false)` when it is written as anything else,
/// `None` when it is not written.
fn secret_form(v: &Option<String>) -> Option<bool> {
    written(v).map(|p| p.strip_prefix("secret:").is_some_and(|n| !n.trim().is_empty()))
}

/// The first of `fields` that has a `{` or `}`: an entry is not a template. A coded line.
fn braces(table: &str, name: &str, fields: &[(&str, Option<&str>)]) -> Result<(), String> {
    match fields.iter().find(|(_, v)| v.is_some_and(|v| v.contains(['{', '}']))) {
        Some((key, _)) => Err(msg::line(msg::CFG_ENTRY_BRACES, &[table, name, key])),
        None => Ok(()),
    }
}

/// `[[host]]`: a PC a job runs commands on (`ssh`, `winrm`) or moves files to (`send`, `fetch`), and how to
/// log in. `on` is `ssh://[user@]host[:port]` or `winrm://user@host[:port]`; the other keys follow the
/// rules of the same keys of an `ssh` or `winrm` job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostEntry {
    pub name: String,
    pub on: String,
    /// ssh only: `SHA256:…`, needed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_key: Option<String>,
    /// ssh only: the private key file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// `secret:<name>`: needed for winrm, optional for ssh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// winrm only: the SHA-256 of the server's certificate, needed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert: Option<String>,
    /// winrm only: `basic` when not written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<WinrmAuth>,
}

impl HostEntry {
    /// The kind `on` says; `None` when it is neither an ssh nor a winrm address.
    pub fn kind(&self) -> Option<HostKind> {
        let t = self.on.trim();
        if crate::chore::ssh(t).is_some() {
            Some(HostKind::Ssh)
        } else if crate::chore::winrm(t).is_some() {
            Some(HostKind::Winrm)
        } else {
            None
        }
    }

    /// The WinRM authentication, `basic` when not written.
    pub fn auth(&self) -> WinrmAuth {
        self.auth.unwrap_or_default()
    }

    /// Why it may not be used: a name that is not one, a `{` or `}` in a key, an `on` that is not a single
    /// PC's ssh or winrm address (a `group:` is refused), a key the kind of host cannot take, and whatever
    /// the same rules of an `ssh` or `winrm` job say. A coded line.
    pub fn check(&self) -> Result<(), String> {
        let table = "[[host]]";
        // An `on` with `@` is an address, never a name (`chore::host_name`), so no job could name it.
        if !valid_name(&self.name) || self.name.contains('@') {
            return Err(msg::line(msg::CFG_ENTRY_NAME_BAD, &[table, &self.name]));
        }
        braces(
            table,
            &self.name,
            &[
                ("on", Some(&self.on)),
                ("host_key", self.host_key.as_deref()),
                ("key", self.key.as_deref()),
                ("password", self.password.as_deref()),
                ("cert", self.cert.as_deref()),
            ],
        )?;
        let bad = |why: &str| msg::line(msg::CFG_HOST_BAD, &[&self.name, msg::uncoded(why)]);
        let read = match (crate::chore::ssh(&self.on), crate::chore::winrm(&self.on)) {
            (Some(r), _) | (_, Some(r)) => r,
            _ => Err(msg::line(
                msg::CHORE_ON_BAD,
                &["ssh://[user@]host[:port] / winrm://user@host[:port]", &crate::chore::hide_password(&self.on)],
            )),
        };
        read.map_err(|e| bad(&e))?;
        let wrong = |key: &str, kind: HostKind| msg::line(msg::CFG_HOST_FIELD_KIND, &[&self.name, key, kind.key()]);
        let password = secret_form(&self.password);
        match self.kind() {
            Some(HostKind::Ssh) => {
                if self.cert.is_some() {
                    return Err(wrong("cert", HostKind::Ssh));
                }
                if self.auth.is_some() {
                    return Err(wrong("auth", HostKind::Ssh));
                }
                if password == Some(false) {
                    return Err(msg::line(msg::CFG_ENTRY_PASSWORD, &[table, &self.name]));
                }
                crate::chore::ssh_login(true, &self.key, &self.host_key, &self.password).map_err(|e| bad(&e))
            }
            _ => {
                if self.key.is_some() {
                    return Err(wrong("key", HostKind::Winrm));
                }
                if self.host_key.is_some() {
                    return Err(wrong("host_key", HostKind::Winrm));
                }
                if password != Some(true) {
                    return Err(msg::line(msg::CFG_ENTRY_PASSWORD, &[table, &self.name]));
                }
                crate::chore::winrm_login(false, &self.cert, &self.password).map_err(|e| bad(&e))
            }
        }
    }
}

/// `[[bucket]]`: an S3 bucket (and a folder in it) and its login. `password` is the secret access key as a
/// `secret:<name>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BucketEntry {
    pub name: String,
    /// `s3://bucket` or `s3://bucket/folder/`: a job's path is below it.
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

impl BucketEntry {
    /// Why it may not be used: a name that is not one, a `{` or `}` in a key, a `url` that is not an S3
    /// address with a plain folder, and whatever the same rules of a `send` or `fetch` with S3 say. A coded line.
    pub fn check(&self) -> Result<(), String> {
        let table = "[[bucket]]";
        if !valid_name(&self.name) {
            return Err(msg::line(msg::CFG_ENTRY_NAME_BAD, &[table, &self.name]));
        }
        braces(
            table,
            &self.name,
            &[
                ("url", Some(&self.url)),
                ("key_id", self.key_id.as_deref()),
                ("password", self.password.as_deref()),
                ("region", self.region.as_deref()),
                ("endpoint", self.endpoint.as_deref()),
            ],
        )?;
        let bad = |why: &str| msg::line(msg::CFG_BUCKET_BAD, &[&self.name, msg::uncoded(why)]);
        match crate::chore::s3(&self.url) {
            None => return Err(bad(&msg::line(msg::CHORE_S3_BUCKET, &[&crate::chore::shown_address(&self.url)]))),
            Some(Err(e)) => return Err(bad(&e)),
            Some(Ok(_)) => {}
        }
        bucket_prefix(&self.url).map_err(|e| bad(&e))?;
        if secret_form(&self.password) != Some(true) {
            return Err(msg::line(msg::CFG_ENTRY_PASSWORD, &[table, &self.name]));
        }
        crate::chore::s3_account(&self.key_id, &self.region, &self.endpoint, &self.password).map_err(|e| bad(&e))
    }
}

/// The folder part of a `[[bucket]]`'s `url`: `""` for the bucket's top, else `"folder/sub/"`; the slashes
/// at the ends of what is written are trimmed. Refused: an empty part (`a//b`), `*` and `?`. The text is not coded.
pub fn bucket_prefix(url: &str) -> Result<String, String> {
    let at = match crate::chore::s3(url) {
        Some(r) => r?,
        None => return Err(msg::line(msg::CHORE_S3_BUCKET, &[&crate::chore::shown_address(url)])),
    };
    let folder = at.key.trim_matches('/');
    if folder.is_empty() {
        return Ok(String::new());
    }
    if folder.split('/').any(str::is_empty) {
        return Err(msg::note(msg::N_BUCKET_PREFIX_SEGMENT, &[url.trim()]));
    }
    if folder.contains(['*', '?']) {
        return Err(msg::note(msg::N_BUCKET_PREFIX_WILDCARD, &[url.trim()]));
    }
    Ok(format!("{folder}/"))
}

/// `[[mailbox]]`: a mailbox a `[[when]]` watches (`mailbox = "orders"`), and its password as a `secret:<name>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailboxEntry {
    pub name: String,
    /// `imaps://user@host[:port][/folder]`.
    pub mail: String,
    /// `secret:<name>`.
    pub password: String,
}

impl MailboxEntry {
    /// The name of the secret, without `secret:`.
    pub fn secret(&self) -> Option<&str> {
        self.password.trim().strip_prefix("secret:").map(str::trim).filter(|n| !n.is_empty())
    }

    /// Why it may not be used: a name that is not one, a `{` or `}` in a key, a `mail` a `[[when]]` would not
    /// read, a `password` that is not `secret:<name>`. A coded line.
    pub fn check(&self) -> Result<(), String> {
        let table = "[[mailbox]]";
        if !valid_name(&self.name) {
            return Err(msg::line(msg::CFG_ENTRY_NAME_BAD, &[table, &self.name]));
        }
        braces(table, &self.name, &[("mail", Some(&self.mail)), ("password", Some(&self.password))])?;
        crate::when::mailbox(&self.mail).map_err(|e| msg::line(msg::CFG_MAILBOX_BAD, &[&self.name, msg::uncoded(&e)]))?;
        if self.secret().is_none() {
            return Err(msg::line(msg::CFG_ENTRY_PASSWORD, &[table, &self.name]));
        }
        Ok(())
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

/// The registered names a net uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Names {
    /// A job's `run_as = "…"`.
    pub accounts: BTreeSet<String>,
    /// A `sql`'s `database = "…"`.
    pub databases: BTreeSet<String>,
    /// An `ssh` or `winrm`'s `on = "…"` that is a name, and a `send` or `fetch`'s `host = "…"`.
    pub hosts: BTreeSet<String>,
    /// A `send` or `fetch`'s `bucket = "…"`.
    pub buckets: BTreeSet<String>,
    /// A `[[when]]`'s `mailbox = "…"`.
    pub mailboxes: BTreeSet<String>,
}

/// The `[[host]]` a job names: its name, the kind of job (`send`, `fetch`, `ssh`, `winrm`) and the kind of host it needs.
fn job_host(c: &Chore) -> Option<(&str, &'static str, HostKind)> {
    match c {
        Chore::Ssh { on, .. } => crate::chore::host_name(on).map(|n| (n, "ssh", HostKind::Ssh)),
        Chore::Winrm { on, .. } => crate::chore::host_name(on).map(|n| (n, "winrm", HostKind::Winrm)),
        Chore::Send { host: Some(n), .. } => Some((n.as_str(), "send", HostKind::Ssh)),
        Chore::Fetch { host: Some(n), .. } => Some((n.as_str(), "fetch", HostKind::Ssh)),
        _ => None,
    }
}

/// The `[[bucket]]` a `send` or `fetch` names.
fn job_bucket(c: &Chore) -> Option<&str> {
    match c {
        Chore::Send { bucket: Some(n), .. } | Chore::Fetch { bucket: Some(n), .. } => Some(n.as_str()),
        _ => None,
    }
}

/// The names of the registered places a net uses: a job's `run_as = "…"`, a `sql`'s `database = "…"`,
/// the host of an `ssh`, `winrm`, `send` or `fetch`, the bucket of a `send` or `fetch`, and the mailbox of a
/// `[[when]]`. A job that only calls another net (`kind = "jobnet"`) uses nothing itself.
pub fn names_used(net: &Jobnet) -> Names {
    let mut names = Names::default();
    for j in &net.jobs {
        if let Some(RunAs::Named(n)) = &j.run_as {
            names.accounts.insert(n.clone());
        }
        if let crate::jobnet::Part::Chore(c) = &j.part {
            if let Chore::Sql { database: Some(n), .. } = c {
                names.databases.insert(n.clone());
            }
            if let Some((n, _, _)) = job_host(c) {
                names.hosts.insert(n.to_owned());
            }
            if let Some(n) = job_bucket(c) {
                names.buckets.insert(n.to_owned());
            }
        }
    }
    for w in &net.when {
        if let crate::when::Kind::Mail { mailbox: Some(n), .. } = &w.kind {
            names.mailboxes.insert(n.clone());
        }
    }
    names
}

/// The cautions (W3754, W3755, W3792 to W3795) for the names `net` (called `id`) gives that `config` does not
/// have, or has as a kind the job cannot use: one line per name, with the first job that gives it. On every
/// OS, since the names are all it shows.
pub fn unknown_lines(config: &Config, id: &str, net: &Jobnet) -> Vec<String> {
    let mut out = Vec::new();
    // What was told: which table (0 account, 1 database, 2 host, 3 bucket, 4 mailbox) and the name.
    let mut told: BTreeSet<(u8, String)> = BTreeSet::new();
    for j in &net.jobs {
        if let Some(RunAs::Named(n)) = &j.run_as {
            if !config.account.iter().any(|a| a.name == *n) && told.insert((0, n.clone())) {
                out.push(msg::line(msg::OS_ACCOUNT_UNKNOWN, &[id, &j.id, n]));
            }
        }
        let crate::jobnet::Part::Chore(c) = &j.part else { continue };
        if let Chore::Sql { database: Some(n), .. } = c {
            if !config.database.iter().any(|d| d.name == *n) && told.insert((1, n.clone())) {
                out.push(msg::line(msg::OS_DATABASE_UNKNOWN, &[id, &j.id, n]));
            }
        }
        if let Some((n, _, need)) = job_host(c) {
            match config.host.iter().find(|h| h.name == n) {
                None => {
                    if told.insert((2, n.to_owned())) {
                        out.push(msg::line(msg::OS_HOST_UNKNOWN, &[id, &j.id, n]));
                    }
                }
                Some(h) if h.kind() != Some(need) => {
                    if told.insert((2, n.to_owned())) {
                        out.push(msg::line(msg::OS_HOST_KIND, &[id, &j.id, n, h.kind().map_or("", HostKind::key)]));
                    }
                }
                Some(_) => {}
            }
        }
        if let Some(n) = job_bucket(c) {
            if !config.bucket.iter().any(|b| b.name == n) && told.insert((3, n.to_owned())) {
                out.push(msg::line(msg::OS_BUCKET_UNKNOWN, &[id, &j.id, n]));
            }
        }
    }
    for w in &net.when {
        if let crate::when::Kind::Mail { mailbox: Some(n), .. } = &w.kind {
            if !config.mailbox.iter().any(|m| m.name == *n) && told.insert((4, n.clone())) {
                out.push(msg::line(msg::OS_MAILBOX_UNKNOWN, &[id, n]));
            }
        }
    }
    out
}

/// The environment of a job that names a `[[host]]` or a `[[bucket]]`, with what the registration owns put in
/// place of what the job's own address and login would be (it is not wired into the engine here).
/// `env` is the job's [`Chore::env`] (after `fill`); `host` and `bucket` are the names the run records.
/// The kind is read from `env`: `KOYOMI_SSH_*`, `KOYOMI_WINRM_*`, `KOYOMI_SFTP_*` or `KOYOMI_CLOUD_*`. A
/// `secret:<name>` stays the text (it is read when the job starts); other keys are kept.
/// Errors: E3296 and E3297 (not in `config`), E3298 (the wrong kind of host), E3299 (the path cannot be joined).
pub fn far_env(
    config: &Config,
    host: Option<&str>,
    bucket: Option<&str>,
    env: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    let mut out = env.clone();
    let host_of = |name: &str, job: &str, need: HostKind| -> Result<&HostEntry, String> {
        let h = config.host.iter().find(|h| h.name == name).ok_or_else(|| msg::line(msg::RUN_HOST_UNKNOWN, &[name]))?;
        match h.kind() {
            Some(k) if k == need => Ok(h),
            other => Err(msg::line(msg::RUN_HOST_KIND, &[name, job, other.map_or("", HostKind::key)])),
        }
    };
    let set = |out: &mut BTreeMap<String, String>, key: &str, v: Option<&str>| {
        if let Some(v) = v.map(str::trim).filter(|v| !v.is_empty()) {
            out.insert(key.to_owned(), v.to_owned());
        }
    };
    if let Some(name) = host.filter(|_| env.contains_key("KOYOMI_SSH_COMMAND")) {
        let h = host_of(name, "ssh", HostKind::Ssh)?;
        for k in ["ON", "KEY", "HOST_KEY", "PASSWORD"] {
            out.remove(&format!("KOYOMI_SSH_{k}"));
        }
        out.insert("KOYOMI_SSH_ON".to_owned(), h.on.trim().to_owned());
        set(&mut out, "KOYOMI_SSH_KEY", h.key.as_deref());
        set(&mut out, "KOYOMI_SSH_HOST_KEY", h.host_key.as_deref());
        set(&mut out, "KOYOMI_SSH_PASSWORD", h.password.as_deref());
    } else if let Some(name) = host.filter(|_| env.contains_key("KOYOMI_WINRM_COMMAND")) {
        let h = host_of(name, "winrm", HostKind::Winrm)?;
        for k in ["ON", "AUTH", "CERT", "PASSWORD"] {
            out.remove(&format!("KOYOMI_WINRM_{k}"));
        }
        out.insert("KOYOMI_WINRM_ON".to_owned(), h.on.trim().to_owned());
        out.insert("KOYOMI_WINRM_AUTH".to_owned(), h.auth().key().to_owned());
        set(&mut out, "KOYOMI_WINRM_CERT", h.cert.as_deref());
        set(&mut out, "KOYOMI_WINRM_PASSWORD", h.password.as_deref());
    } else if let (Some(name), Some(op)) = (host, env.get("KOYOMI_SFTP_OP")) {
        let h = host_of(name, op, HostKind::Ssh)?;
        let far = if op == "send" { "KOYOMI_SFTP_INTO" } else { "KOYOMI_SFTP_FROM" };
        let path = env.get(far).map(String::as_str).unwrap_or_default();
        // The path was checked before it was filled in only when it had no `{`; the joined address is checked here.
        let url = crate::chore::sftp_join(&h.on, path)
            .and_then(|u| crate::chore::far_path_ok(op == "send", &u, path).map(|()| u))
            .map_err(|e| msg::line(msg::RUN_NAMED_PATH_BAD, &[name, path, msg::uncoded(&e)]))?;
        for k in ["KEY", "HOST_KEY", "PASSWORD"] {
            out.remove(&format!("KOYOMI_SFTP_{k}"));
        }
        out.insert(far.to_owned(), url);
        set(&mut out, "KOYOMI_SFTP_KEY", h.key.as_deref());
        set(&mut out, "KOYOMI_SFTP_HOST_KEY", h.host_key.as_deref());
        set(&mut out, "KOYOMI_SFTP_PASSWORD", h.password.as_deref());
    } else if let (Some(name), Some(op)) = (bucket, env.get("KOYOMI_CLOUD_OP")) {
        let b = config.bucket.iter().find(|b| b.name == name).ok_or_else(|| msg::line(msg::RUN_BUCKET_UNKNOWN, &[name]))?;
        let far = if op == "send" { "KOYOMI_CLOUD_INTO" } else { "KOYOMI_CLOUD_FROM" };
        let path = env.get(far).map(String::as_str).unwrap_or_default();
        let url = crate::chore::s3_join(&b.url, path)
            .and_then(|u| crate::chore::far_path_ok(op == "send", &u, path).map(|()| u))
            .map_err(|e| msg::line(msg::RUN_NAMED_PATH_BAD, &[name, path, msg::uncoded(&e)]))?;
        for k in ["KEY_ID", "REGION", "ENDPOINT", "PASSWORD"] {
            out.remove(&format!("KOYOMI_CLOUD_{k}"));
        }
        out.insert(far.to_owned(), url);
        set(&mut out, "KOYOMI_CLOUD_KEY_ID", b.key_id.as_deref());
        set(&mut out, "KOYOMI_CLOUD_REGION", b.region.as_deref());
        set(&mut out, "KOYOMI_CLOUD_ENDPOINT", b.endpoint.as_deref());
        set(&mut out, "KOYOMI_CLOUD_PASSWORD", b.password.as_deref());
    }
    Ok(out)
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

fn host_block(e: &HostEntry) -> Vec<(&'static str, String)> {
    let mut b = vec![("name", quote(&e.name)), ("on", quote(&e.on))];
    for (k, v) in [("host_key", &e.host_key), ("key", &e.key), ("password", &e.password), ("cert", &e.cert)] {
        if let Some(v) = v {
            b.push((k, quote(v)));
        }
    }
    if let Some(a) = e.auth {
        b.push(("auth", quote(a.key())));
    }
    b
}

fn bucket_block(e: &BucketEntry) -> Vec<(&'static str, String)> {
    let mut b = vec![("name", quote(&e.name)), ("url", quote(&e.url))];
    for (k, v) in [("key_id", &e.key_id), ("password", &e.password), ("region", &e.region), ("endpoint", &e.endpoint)] {
        if let Some(v) = v {
            b.push((k, quote(v)));
        }
    }
    b
}

fn mailbox_block(e: &MailboxEntry) -> Vec<(&'static str, String)> {
    vec![("name", quote(&e.name)), ("mail", quote(&e.mail)), ("password", quote(&e.password))]
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

/// `text` with the `[[host]]` `entry` added, or changed if one has its name.
pub fn put_host(text: &str, entry: &HostEntry) -> Result<String, String> {
    entry.check()?;
    let now = Config::parse(text)?.host;
    let all = with(&now, entry, |h| h.name == entry.name);
    let blocks: Vec<_> = all.iter().map(host_block).collect();
    ito_common::set_tables(text, "host", &blocks)
}

/// `text` without the `[[host]]` `name`. Refused: no such host.
pub fn remove_host(text: &str, name: &str) -> Result<String, String> {
    let now = Config::parse(text)?.host;
    if !now.iter().any(|h| h.name == name) {
        return Err(unknown("[[host]]", name));
    }
    let blocks: Vec<_> = now.iter().filter(|h| h.name != name).map(host_block).collect();
    ito_common::set_tables(text, "host", &blocks)
}

/// `text` with the `[[bucket]]` `entry` added, or changed if one has its name.
pub fn put_bucket(text: &str, entry: &BucketEntry) -> Result<String, String> {
    entry.check()?;
    let now = Config::parse(text)?.bucket;
    let all = with(&now, entry, |b| b.name == entry.name);
    let blocks: Vec<_> = all.iter().map(bucket_block).collect();
    ito_common::set_tables(text, "bucket", &blocks)
}

/// `text` without the `[[bucket]]` `name`. Refused: no such bucket.
pub fn remove_bucket(text: &str, name: &str) -> Result<String, String> {
    let now = Config::parse(text)?.bucket;
    if !now.iter().any(|b| b.name == name) {
        return Err(unknown("[[bucket]]", name));
    }
    let blocks: Vec<_> = now.iter().filter(|b| b.name != name).map(bucket_block).collect();
    ito_common::set_tables(text, "bucket", &blocks)
}

/// `text` with the `[[mailbox]]` `entry` added, or changed if one has its name.
pub fn put_mailbox(text: &str, entry: &MailboxEntry) -> Result<String, String> {
    entry.check()?;
    let now = Config::parse(text)?.mailbox;
    let all = with(&now, entry, |m| m.name == entry.name);
    let blocks: Vec<_> = all.iter().map(mailbox_block).collect();
    ito_common::set_tables(text, "mailbox", &blocks)
}

/// `text` without the `[[mailbox]]` `name`. Refused: no such mailbox.
pub fn remove_mailbox(text: &str, name: &str) -> Result<String, String> {
    let now = Config::parse(text)?.mailbox;
    if !now.iter().any(|m| m.name == name) {
        return Err(unknown("[[mailbox]]", name));
    }
    let blocks: Vec<_> = now.iter().filter(|m| m.name != name).map(mailbox_block).collect();
    ito_common::set_tables(text, "mailbox", &blocks)
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
        let n = names_used(&net);
        assert_eq!(
            (n.accounts.into_iter().collect::<Vec<_>>(), n.databases.into_iter().collect::<Vec<_>>()),
            (vec!["batch".to_owned()], vec!["sales".to_owned()])
        );
        assert!(n.hosts.is_empty() && n.buckets.is_empty() && n.mailboxes.is_empty());
    }

    const HK: &str = "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU";

    fn host(name: &str) -> HostEntry {
        HostEntry {
            name: name.into(),
            on: "ssh://ops@web1:2222".into(),
            host_key: Some(HK.into()),
            key: Some("C:/keys/id".into()),
            password: None,
            cert: None,
            auth: None,
        }
    }

    fn bucket(name: &str) -> BucketEntry {
        BucketEntry {
            name: name.into(),
            url: "s3://corp-logs/app/".into(),
            key_id: Some("AKIAEXAMPLE".into()),
            password: Some("secret:s3-logs".into()),
            region: Some("ap-northeast-1".into()),
            endpoint: None,
        }
    }

    fn mailbox(name: &str) -> MailboxEntry {
        MailboxEntry { name: name.into(), mail: "imaps://billing@mail.example.com/INBOX".into(), password: "secret:mail".into() }
    }

    #[test]
    fn host_bucket_and_mailbox_entries_parse_and_are_refused_with_their_reason() {
        assert_eq!(host("web1").check(), Ok(()));
        assert_eq!(host("web1").kind(), Some(HostKind::Ssh));
        assert_eq!(bucket("logs").check(), Ok(()));
        assert_eq!(mailbox("billing").check(), Ok(()));
        assert!(msg::CFG_ENTRY_NAME_BAD.is_in(&host("{x}").check().unwrap_err()));
        assert!(msg::CFG_ENTRY_NAME_BAD.is_in(&bucket("a/b").check().unwrap_err()));
        assert!(msg::CFG_ENTRY_NAME_BAD.is_in(&mailbox("").check().unwrap_err()));
        // A host's name with `@` reads as an address in a job's `on`, so it is not a name.
        assert!(msg::CFG_ENTRY_NAME_BAD.is_in(&host("ops@web1").check().unwrap_err()));
        // An address that is not ssh or winrm; a group; a key that is not the kind's; a braced value.
        assert!(msg::CFG_ENTRY_BRACES.is_in(&HostEntry { on: "{web}".into(), ..host("web1") }.check().unwrap_err()));
        assert!(msg::CFG_HOST_BAD.is_in(&HostEntry { on: "ftp://x".into(), ..host("web1") }.check().unwrap_err()));
        assert!(msg::CFG_HOST_FIELD_KIND.is_in(&HostEntry { cert: Some("x".into()), ..host("web1") }.check().unwrap_err()));
        assert!(msg::CFG_ENTRY_BRACES.is_in(&HostEntry { key: Some("C:/{k}".into()), ..host("web1") }.check().unwrap_err()));
        assert!(msg::CFG_BUCKET_BAD.is_in(&BucketEntry { url: "http://x".into(), ..bucket("logs") }.check().unwrap_err()));
        assert!(msg::CFG_BUCKET_BAD.is_in(&BucketEntry { url: "s3://bkt/a//c".into(), ..bucket("logs") }.check().unwrap_err()));
        assert!(msg::CFG_ENTRY_PASSWORD.is_in(&BucketEntry { password: Some("plain".into()), ..bucket("logs") }.check().unwrap_err()));
        assert!(msg::CFG_MAILBOX_BAD.is_in(&MailboxEntry { mail: "x".into(), ..mailbox("billing") }.check().unwrap_err()));
        assert!(msg::CFG_ENTRY_PASSWORD.is_in(&MailboxEntry { password: "plain".into(), ..mailbox("billing") }.check().unwrap_err()));
        // A table that repeats a name is refused in koyomi.toml.
        let one = format!("[[host]]\nname = \"a\"\non = \"ssh://u@h\"\nhost_key = \"{HK}\"\n");
        assert!(Config::parse(&format!("{one}{one}")).is_err());
    }

    #[test]
    fn put_and_remove_of_hosts_buckets_and_mailboxes_keep_the_comments() {
        let text = "# settings\nconcurrency = 2 # kept\n";
        let out = put_host(text, &host("web1")).unwrap();
        let out = put_bucket(&out, &bucket("logs")).unwrap();
        let out = put_mailbox(&out, &mailbox("billing")).unwrap();
        assert!(out.starts_with("# settings\nconcurrency = 2 # kept\n"), "{out}");
        let c = Config::parse(&out).unwrap();
        assert_eq!((c.host.len(), c.bucket.len(), c.mailbox.len()), (1, 1, 1));
        assert_eq!(c.bucket[0].url, "s3://corp-logs/app/");
        let out = put_host(&out, &HostEntry { on: "ssh://ops@web2".into(), ..host("web1") }).unwrap();
        assert_eq!(Config::parse(&out).unwrap().host[0].on, "ssh://ops@web2");
        let out = remove_mailbox(&remove_bucket(&remove_host(&out, "web1").unwrap(), "logs").unwrap(), "billing").unwrap();
        assert!(out.contains("# kept"), "{out}");
        let c = Config::parse(&out).unwrap();
        assert!(c.host.is_empty() && c.bucket.is_empty() && c.mailbox.is_empty());
        assert!(msg::REGISTRY_NAME_UNKNOWN.is_in(&remove_host(&out, "web1").unwrap_err()));
        assert!(msg::CFG_ENTRY_NAME_BAD.is_in(&put_bucket("", &bucket("{x}")).unwrap_err()));
    }

    #[test]
    fn the_prefix_of_a_bucket_is_its_folder() {
        assert_eq!(bucket_prefix("s3://bkt"), Ok(String::new()));
        assert_eq!(bucket_prefix("s3://bkt/"), Ok(String::new()));
        assert_eq!(bucket_prefix("s3://bkt/app"), Ok("app/".to_owned()));
        assert_eq!(bucket_prefix("s3://bkt/app/x/"), Ok("app/x/".to_owned()));
        assert!(bucket_prefix("s3://bkt/a//c").is_err());
        assert!(bucket_prefix("s3://bkt/a*").is_err());
    }

    fn named_net() -> Jobnet {
        jobnet::parse(
            "format = 1\n\
             [[when]]\nmailbox = \"billing\"\nsubject = \"Invoice\"\n\
             [[job]]\nid = \"s\"\nkind = \"send\"\nfrom = \"D:/o/*.csv\"\ninto = \"x\"\nhost = \"web1\"\n\
             [[job]]\nid = \"f\"\nkind = \"fetch\"\nfrom = \"in/*.csv\"\ninto = \"D:/in\"\nbucket = \"logs\"\n\
             [[job]]\nid = \"x\"\nkind = \"ssh\"\non = \"web9\"\ncommand = \"uptime\"\n\
             [[job]]\nid = \"g\"\nkind = \"ssh\"\non = \"{grp}\"\ncommand = \"uptime\"\nkey = \"C:/k\"\nhost_key = \"SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU\"\n",
        )
        .unwrap_or_else(|p| panic!("{p:?}"))
    }

    #[test]
    fn the_hosts_buckets_and_mailboxes_a_net_names_are_found_and_the_unknown_ones_told() {
        let net = named_net();
        let n = names_used(&net);
        assert_eq!(n.hosts.iter().map(String::as_str).collect::<Vec<_>>(), ["web1", "web9"]);
        assert_eq!(n.buckets.iter().map(String::as_str).collect::<Vec<_>>(), ["logs"]);
        assert_eq!(n.mailboxes.iter().map(String::as_str).collect::<Vec<_>>(), ["billing"]);
        let none = Config::default();
        let lines = unknown_lines(&none, "g/n", &net);
        for (id, count) in [(msg::OS_HOST_UNKNOWN, 2), (msg::OS_BUCKET_UNKNOWN, 1), (msg::OS_MAILBOX_UNKNOWN, 1)] {
            assert_eq!(lines.iter().filter(|l| id.is_in(l)).count(), count, "{lines:#?}");
        }
        // Registered, but web9 is a winrm PC and the ssh job cannot use it.
        let ok = Config::parse(&format!(
            "[[host]]\nname = \"web1\"\non = \"ssh://ops@web1\"\nhost_key = \"{HK}\"\n\
             [[host]]\nname = \"web9\"\non = \"winrm://ops@web9\"\ncert = \"{}\"\npassword = \"secret:w\"\n\
             [[bucket]]\nname = \"logs\"\nurl = \"s3://bkt/app/\"\nkey_id = \"AK\"\npassword = \"secret:s\"\nregion = \"ap-northeast-1\"\n\
             [[mailbox]]\nname = \"billing\"\nmail = \"imaps://u@m.example.com\"\npassword = \"secret:m\"\n",
            "A".repeat(64)
        ))
        .unwrap();
        let lines = unknown_lines(&ok, "g/n", &net);
        assert_eq!(lines.len(), 1, "{lines:#?}");
        assert!(msg::OS_HOST_KIND.is_in(&lines[0]), "{lines:#?}");
    }

    fn far(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn the_environment_of_a_named_host_or_bucket_is_the_registrations() {
        let config = Config::parse(&format!(
            "[[host]]\nname = \"web1\"\non = \"ssh://ops@web1:2222\"\nhost_key = \"{HK}\"\npassword = \"secret:web\"\n\
             [[bucket]]\nname = \"logs\"\nurl = \"s3://corp/app\"\nkey_id = \"AK\"\npassword = \"secret:s3\"\nregion = \"ap-northeast-1\"\n"
        ))
        .unwrap();
        // ssh: the job's own keys (stale) go, the entry's come, `secret:` stays text.
        let env = far(&[("KOYOMI_SSH_COMMAND", "uptime"), ("KOYOMI_SSH_ON", "web1"), ("KOYOMI_SSH_KEY", "stale")]);
        let out = far_env(&config, Some("web1"), None, &env).unwrap();
        assert_eq!(out["KOYOMI_SSH_ON"], "ssh://ops@web1:2222");
        assert_eq!(out["KOYOMI_SSH_PASSWORD"], "secret:web");
        assert_eq!(out["KOYOMI_SSH_HOST_KEY"], HK);
        assert!(!out.contains_key("KOYOMI_SSH_KEY"));
        // send: the far side is the sftp address built from the host and the path.
        let env = far(&[
            ("KOYOMI_SFTP_OP", "send"),
            ("KOYOMI_SFTP_FROM", "D:/o/a.csv"),
            ("KOYOMI_SFTP_INTO", "in/a.csv"),
            ("KOYOMI_SFTP_PASSWORD", "stale"),
        ]);
        let out = far_env(&config, Some("web1"), None, &env).unwrap();
        assert_eq!(out["KOYOMI_SFTP_INTO"], "sftp://ops@web1:2222/~/in/a.csv");
        assert_eq!(out["KOYOMI_SFTP_FROM"], "D:/o/a.csv");
        assert_eq!(out["KOYOMI_SFTP_PASSWORD"], "secret:web");
        // fetch from a bucket: the key is below the bucket's folder.
        let env = far(&[
            ("KOYOMI_CLOUD_OP", "fetch"),
            ("KOYOMI_CLOUD_FROM", "in/a.csv"),
            ("KOYOMI_CLOUD_INTO", "D:/in"),
            ("KOYOMI_CLOUD_REGION", "stale"),
        ]);
        let out = far_env(&config, None, Some("logs"), &env).unwrap();
        assert_eq!(out["KOYOMI_CLOUD_FROM"], "s3://corp/app/in/a.csv");
        assert_eq!(out["KOYOMI_CLOUD_REGION"], "ap-northeast-1");
        assert_eq!(out["KOYOMI_CLOUD_KEY_ID"], "AK");
        assert_eq!(out["KOYOMI_CLOUD_PASSWORD"], "secret:s3");
        // Errors.
        assert!(msg::RUN_HOST_UNKNOWN.is_in(&far_env(&config, Some("nope"), None, &far(&[("KOYOMI_SSH_COMMAND", "x")])).unwrap_err()));
        assert!(msg::RUN_BUCKET_UNKNOWN.is_in(&far_env(&config, None, Some("nope"), &far(&[("KOYOMI_CLOUD_OP", "send")])).unwrap_err()));
        assert!(msg::RUN_HOST_KIND.is_in(&far_env(&config, Some("web1"), None, &far(&[("KOYOMI_WINRM_COMMAND", "x")])).unwrap_err()));
        let env = far(&[("KOYOMI_CLOUD_OP", "send"), ("KOYOMI_CLOUD_INTO", "/abs")]);
        assert!(msg::RUN_NAMED_PATH_BAD.is_in(&far_env(&config, None, Some("logs"), &env).unwrap_err()));
        // A path filled in at run time keeps the path's rules: a fetch names a file, not the home or a folder.
        for (name, bucket, env) in [
            (Some("web1"), None, far(&[("KOYOMI_SFTP_OP", "fetch"), ("KOYOMI_SFTP_FROM", ""), ("KOYOMI_SFTP_INTO", "D:/in")])),
            (Some("web1"), None, far(&[("KOYOMI_SFTP_OP", "fetch"), ("KOYOMI_SFTP_FROM", "in/"), ("KOYOMI_SFTP_INTO", "D:/in")])),
            (None, Some("logs"), far(&[("KOYOMI_CLOUD_OP", "fetch"), ("KOYOMI_CLOUD_FROM", "in/"), ("KOYOMI_CLOUD_INTO", "D:/in")])),
            (None, Some("logs"), far(&[("KOYOMI_CLOUD_OP", "fetch"), ("KOYOMI_CLOUD_FROM", "a*/b.csv"), ("KOYOMI_CLOUD_INTO", "D:/in")])),
            (None, Some("logs"), far(&[("KOYOMI_CLOUD_OP", "send"), ("KOYOMI_CLOUD_FROM", "D:/o"), ("KOYOMI_CLOUD_INTO", "x*.csv")])),
        ] {
            let e = far_env(&config, name, bucket, &env).unwrap_err();
            assert!(msg::RUN_NAMED_PATH_BAD.is_in(&e), "{env:?}: {e}");
        }
    }

    #[test]
    fn an_entry_with_a_password_in_its_address_is_refused_without_showing_it() {
        let pw = "Hunter2zz";
        let e = database("s", "postgres://u:Hunter2zz@h:5432/x", None, None).check().unwrap_err();
        assert!(msg::CFG_DATABASE_BAD.is_in(&e) && !e.contains(pw), "{e}");
        let e = MailboxEntry { mail: "imaps://me:Hunter2zz@h".into(), ..mailbox("billing") }.check().unwrap_err();
        assert!(msg::CFG_MAILBOX_BAD.is_in(&e) && !e.contains(pw), "{e}");
        for on in ["ssh://u:Hunter2zz@h:22", "ssh://u:Hunter2zz/x@h", "ftp://u:Hunter2zz@h"] {
            let e = HostEntry { on: on.into(), ..host("web1") }.check().unwrap_err();
            assert!(!e.contains(pw), "{on}: {e}");
        }
        let e = BucketEntry { url: "sftp://u:Hunter2zz@h".into(), ..bucket("logs") }.check().unwrap_err();
        assert!(!e.contains(pw), "{e}");
        let e = BucketEntry { url: "s3://me:Hunter2zz@bkt".into(), ..bucket("logs") }.check().unwrap_err();
        assert!(e.contains("***") && !e.contains(pw), "{e}");
        let e = bucket_prefix("s3://me:Hunter2zz@bkt").unwrap_err();
        assert!(!e.contains(pw), "{e}");
    }
}
