//! The everyday file chores a net can have as parts without a script:
//! delete old files, zip, copy with robocopy, empty a folder, send and fetch
//! files over SFTP or to and from a shared folder, call a web API, run a
//! command on another PC over SSH or on a Windows PC over WinRM, send a mail,
//! run SQL on a database.
//! Each is written in the file as its own `kind` and runs as a PowerShell script
//! the engine writes when the run starts, so it has the job's slot, log,
//! `timeout` and `retry`. Over SFTP the script runs `koyomi sftp run`; an `ssh`
//! runs `koyomi ssh run` itself, without PowerShell, a `winrm` `koyomi winrm
//! run`, a `mail` `koyomi mail run` and a `sql` `koyomi sql run`. Each reads
//! what to do from the environment.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::jobnet::{Jobnet, Judge, Part, Range};

/// One chore and what it works on. Paths may have `{date}`, `{param:…}` and `{var:…}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Chore {
    /// 古いファイルを消す: the files in `path` (not below it) matching `pattern`
    /// that were last written more than `keep_days` days ago.
    Delete {
        path: String,
        #[serde(default)]
        pattern: Option<String>,
        keep_days: u32,
    },
    /// ZIP にする: `from` (a file, a folder or a wildcard) into the `.zip` file `into`, replaced.
    Zip { from: String, into: String },
    /// robocopy で写す: the folder `from` and the folders below it into `into`; only
    /// the files matching `pattern` when it is there. Nothing is deleted in `into`.
    Copy {
        from: String,
        into: String,
        #[serde(default)]
        pattern: Option<String>,
    },
    /// フォルダーを空ける: everything in `path` goes; the folder stays.
    Empty { path: String },
    /// ファイルを送る: the files `from` (a folder or a wildcard here) into the folder
    /// `into`, either `sftp://[user@]host[:port]/folder` or a folder or share.
    Send {
        from: String,
        into: String,
        /// The SSH private key for `sftp://`; without it and `password`, the account's own
        /// key in `~/.ssh`.
        #[serde(default)]
        key: Option<String>,
        /// The server's host key for `sftp://`, as `SHA256:…` (`koyomi sftp hostkey`).
        #[serde(default)]
        host_key: Option<String>,
        /// `secret:<name>`: log in with the password kept under that name.
        #[serde(default)]
        password: Option<String>,
    },
    /// ファイルを受け取る: the files `from`, either `sftp://[user@]host[:port]/folder/*.csv`
    /// or a folder or share (a wildcard allowed), into the folder `into` here.
    Fetch {
        from: String,
        into: String,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        host_key: Option<String>,
        #[serde(default)]
        password: Option<String>,
    },
    /// HTTP で呼ぶ: `method` to `url` with `body`, judged by the status code: one of
    /// `expect` (empty: 200–299) is 正常, another is 異常.
    Http {
        url: String,
        method: String,
        #[serde(default)]
        body: Option<String>,
        /// The body's type; without it, JSON (`application/json; charset=utf-8`).
        #[serde(default)]
        content_type: Option<String>,
        /// Header values may be `secret:<name>`; they reach the script as
        /// environment variables, so a secret is never written into it.
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        expect: Vec<u16>,
    },
    /// 別の PC で動かす: `command`, as it is written, on `on`
    /// (`ssh://[user@]host[:port]`) over SSH. Its output reaches the job's log as it
    /// comes, and its exit code is judged as a job's (`judge`). `on` may name a
    /// group of PCs instead (`group:web`, koyomi.toml's `[groups]`): the command
    /// runs on each, `max` at a time, and `pass` says whether every one must end
    /// with 0 or one is enough.
    Ssh {
        on: String,
        command: String,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        host_key: Option<String>,
        #[serde(default)]
        password: Option<String>,
        /// On a group: how many of its PCs run at once; one at a time when not written.
        #[serde(default)]
        max: Option<u32>,
        #[serde(default)]
        pass: Pass,
    },
    /// 別の Windows で動かす（WinRM）: `command`, a `cmd.exe` line as it is written,
    /// on `on` (`winrm://user@host[:port]`) over WinRM on HTTPS (5986), logged in
    /// as the user with `password` the way `auth` says. `cert` pins the server's
    /// certificate (its SHA-256, `koyomi winrm cert`). Its output and exit code go
    /// as an `ssh`'s do, and `on` may name a group of PCs as an `ssh`'s may.
    Winrm {
        on: String,
        command: String,
        #[serde(default)]
        cert: Option<String>,
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        auth: WinrmAuth,
        #[serde(default)]
        max: Option<u32>,
        #[serde(default)]
        pass: Pass,
    },
    /// メールを送る: `subject` and `body` (plain text) to `to` and `cc`, with the
    /// files `files` (each a path, or a folder and a name with `*` / `?`) attached,
    /// through the `[[notify.mails]]` place named `via` (the first when none is
    /// written). koyomi sends it itself (`koyomi mail run`).
    Mail {
        to: Vec<String>,
        #[serde(default)]
        cc: Vec<String>,
        subject: String,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        files: Vec<String>,
        #[serde(default)]
        via: Option<String>,
    },
    /// SQL を流す: `sql` (or the SQL in the file `file`) on the database `db`:
    /// `sqlserver://[user@]host[:port]/database`, `postgres://user@host[:port]/database`
    /// or `sqlite://C:\data\app.db`. A user logs in with `password` (a `secret:`);
    /// SQL Server without a user logs in as the Windows account the job runs as.
    /// The rows of a result go to the CSV file `out`, and `rows` says how many
    /// rows (a result's, else the changed ones) are 正常. koyomi runs it itself
    /// (`koyomi sql run`).
    Sql {
        db: String,
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        sql: Option<String>,
        #[serde(default)]
        file: Option<String>,
        #[serde(default)]
        out: Option<String>,
        #[serde(default)]
        rows: Option<String>,
        #[serde(default)]
        tls: SqlTls,
    },
}

/// How a `sql` talks to its server (`tls`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SqlTls {
    /// TLS, with a certificate the PC trusts for the host's name.
    #[default]
    Verify,
    /// TLS, with whatever certificate the server shows (its own self-signed one).
    Trust,
    /// No TLS: PostgreSQL only (SQL Server always encrypts).
    Off,
}

impl SqlTls {
    /// Each, in the order of the editor's list.
    pub const ALL: [SqlTls; 3] = [SqlTls::Verify, SqlTls::Trust, SqlTls::Off];

    /// As written in `tls`.
    pub fn key(self) -> &'static str {
        match self {
            SqlTls::Verify => "verify",
            SqlTls::Trust => "trust",
            SqlTls::Off => "off",
        }
    }

    /// The one written as `key`.
    pub fn of(key: &str) -> Option<SqlTls> {
        SqlTls::ALL.into_iter().find(|t| t.key() == key)
    }
}

/// How a `winrm` logs in (`auth`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WinrmAuth {
    /// The user and password as they are, inside the TLS: a local account of that
    /// PC, with Basic turned on in its WinRM.
    #[default]
    Basic,
    /// NTLM: a domain account (`CORP\name`) or a local one, without the password
    /// itself going over the wire.
    Ntlm,
}

impl WinrmAuth {
    /// Each, in the order of the editor's list.
    pub const ALL: [WinrmAuth; 2] = [WinrmAuth::Basic, WinrmAuth::Ntlm];

    /// As written in `auth`.
    pub fn key(self) -> &'static str {
        match self {
            WinrmAuth::Basic => "basic",
            WinrmAuth::Ntlm => "ntlm",
        }
    }

    /// As people call it, in the editor and the confirm box.
    pub fn name(self) -> &'static str {
        match self {
            WinrmAuth::Basic => "Basic",
            WinrmAuth::Ntlm => "NTLM",
        }
    }

    /// The one written as `s`.
    pub fn of(s: &str) -> Option<WinrmAuth> {
        WinrmAuth::ALL.into_iter().find(|a| a.key() == s)
    }
}

/// When an `ssh` or a `winrm` on a group of PCs (`on = "group:web"`) ends 正常:
/// `pass`. Either way the command runs on every PC of the group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pass {
    /// Every PC's command ended with 0.
    #[default]
    All,
    /// One PC's did.
    Any,
}

impl Pass {
    /// Each, in the order of the editor's choice.
    pub const ALL: [Pass; 2] = [Pass::All, Pass::Any];

    /// As written in `pass`.
    pub fn key(self) -> &'static str {
        match self {
            Pass::All => "all",
            Pass::Any => "any",
        }
    }

    /// The one written as `s`.
    pub fn of(s: &str) -> Option<Pass> {
        Pass::ALL.into_iter().find(|p| p.key() == s)
    }
}

/// What `on` starts with when it names a group of PCs in koyomi.toml's `[groups]`.
pub const GROUP: &str = "group:";

/// The most PCs in one group, and so the most an `ssh` or a `winrm` runs at once.
pub const GROUP_MAX: usize = 64;

/// The group `on` names (`group:web` → `web`), or `None` when it is one PC.
pub fn group(on: &str) -> Option<&str> {
    let t = on.trim();
    t.get(..GROUP.len()).filter(|p| p.eq_ignore_ascii_case(GROUP)).map(|_| t[GROUP.len()..].trim())
}

/// Whether `on` names a group (its name checked); `max` and `pass` go only with
/// one. An `on` that starts with `{…}` is told once it is filled in.
fn fan(on: &str, max: Option<u32>, pass: Pass) -> Result<bool, String> {
    let group = match group(on) {
        Some("") => return Err("on の group: のあとに組の名前を書く（koyomi.toml の [groups] の名前。例: on = \"group:web\"）".into()),
        Some(name) if crate::jobnet::plain_name(name) => true,
        Some(name) => return Err(format!("組の名前は英数字と _ だけ: {name}")),
        None => false,
    };
    if max.is_some_and(|m| !(1..=GROUP_MAX as u32).contains(&m)) {
        return Err(format!("max は 1 から {GROUP_MAX} までの数で書く（組の PC をいくつ同時に動かすか）"));
    }
    if !group && !on.trim().starts_with('{') {
        if max.is_some() {
            return Err("max は on が PC の組（group:名前）のときだけ書く".into());
        }
        if pass != Pass::All {
            return Err("pass は on が PC の組（group:名前）のときだけ書く".into());
        }
    }
    Ok(group)
}

/// The `kind` of each chore, in the order of the editor's shelf.
pub const KINDS: [&str; 11] = ["delete", "zip", "copy", "empty", "send", "fetch", "http", "ssh", "winrm", "mail", "sql"];

/// The methods `kind = "http"` takes.
pub const METHODS: [&str; 6] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];

/// The longest `body` (characters): the script goes on the command line, which
/// Windows cuts at 32767 characters.
pub const BODY_MAX: usize = 8000;

/// The exit code of an `http` that got no answer (the name, the connection, TLS,
/// the time): 異常, told apart from a status code that is not expected (1).
const NO_ANSWER: i32 = 2;

/// The exit code of a `send` or `fetch` that found no file: 警告.
pub const NOTHING: i32 = 3;

/// The exit code of an `ssh` or `winrm` that got no exit code from the far side
/// (no connection, no login, a signal, a lost connection); why is in the file
/// `KOYOMI_REASON` names.
pub const NO_EXIT: i32 = 255;

/// The longest `command` of an `ssh` or `winrm`, and `sql` of a `sql` (characters):
/// it reaches `koyomi ssh run`, `koyomi winrm run` or `koyomi sql run` in an
/// environment variable.
pub const COMMAND_MAX: usize = 8000;

/// The port of `winrm://` when none is written: WinRM over HTTPS.
pub const WINRM_PORT: u16 = 5986;

/// The most `files` a `mail` names, and the most files they may find.
pub const FILES_MAX: usize = 20;

/// Put before each text a `mail` or `sql` gives `koyomi mail run` or `koyomi sql
/// run` in the environment, so a text that starts with `secret:` is not read as a
/// secret.
pub const MAIL_TEXT: char = '|';

/// The port of `sqlserver://` when none is written.
pub const SQL_SERVER_PORT: u16 = 1433;

/// The port of `postgres://` when none is written.
pub const POSTGRES_PORT: u16 = 5432;

impl Chore {
    /// The `kind` as written, and the key of its word.
    pub fn key(&self) -> &'static str {
        match self {
            Chore::Delete { .. } => "delete",
            Chore::Zip { .. } => "zip",
            Chore::Copy { .. } => "copy",
            Chore::Empty { .. } => "empty",
            Chore::Send { .. } => "send",
            Chore::Fetch { .. } => "fetch",
            Chore::Http { .. } => "http",
            Chore::Ssh { .. } => "ssh",
            Chore::Winrm { .. } => "winrm",
            Chore::Mail { .. } => "mail",
            Chore::Sql { .. } => "sql",
        }
    }

    /// Whether it runs a command on another PC (`ssh`, `winrm`): koyomi runs it
    /// itself, not a script, and it keeps a job's `judge` and `env`.
    pub fn far(&self) -> bool {
        matches!(self, Chore::Ssh { .. } | Chore::Winrm { .. })
    }

    /// Whether koyomi runs it itself as `koyomi <kind> run`, not as a script: an
    /// `ssh`, a `winrm`, a `mail`, a `sql`.
    pub fn own(&self) -> bool {
        self.far() || matches!(self, Chore::Mail { .. } | Chore::Sql { .. })
    }

    /// A new, empty one of `kind` (for the shelf).
    pub fn blank(kind: &str) -> Option<Chore> {
        Some(match kind {
            "delete" => Chore::Delete { path: String::new(), pattern: None, keep_days: 30 },
            "zip" => Chore::Zip { from: String::new(), into: String::new() },
            "copy" => Chore::Copy { from: String::new(), into: String::new(), pattern: None },
            "empty" => Chore::Empty { path: String::new() },
            "send" => Chore::Send { from: String::new(), into: String::new(), key: None, host_key: None, password: None },
            "fetch" => Chore::Fetch { from: String::new(), into: String::new(), key: None, host_key: None, password: None },
            "http" => Chore::Http {
                url: String::new(),
                method: "GET".into(),
                body: None,
                content_type: None,
                headers: BTreeMap::new(),
                expect: Vec::new(),
            },
            "ssh" => Chore::Ssh {
                on: String::new(),
                command: String::new(),
                key: None,
                host_key: None,
                password: None,
                max: None,
                pass: Pass::All,
            },
            "winrm" => Chore::Winrm {
                on: String::new(),
                command: String::new(),
                cert: None,
                password: None,
                auth: WinrmAuth::Basic,
                max: None,
                pass: Pass::All,
            },
            "mail" => Chore::Mail { to: Vec::new(), cc: Vec::new(), subject: String::new(), body: None, files: Vec::new(), via: None },
            "sql" => Chore::Sql { db: String::new(), password: None, sql: None, file: None, out: None, rows: None, tls: SqlTls::Verify },
            _ => return None,
        })
    }

    /// Every path of it, to fill in `{date}`, `{param:…}` and `{var:…}`.
    pub fn paths_mut(&mut self) -> Vec<(&'static str, &mut String)> {
        match self {
            Chore::Delete { path, .. } | Chore::Empty { path } => vec![("path", path)],
            Chore::Zip { from, into } | Chore::Copy { from, into, .. } => vec![("from", from), ("into", into)],
            Chore::Send { from, into, key, host_key, .. } | Chore::Fetch { from, into, key, host_key, .. } => {
                let mut v = vec![("from", from), ("into", into)];
                if let Some(k) = key {
                    v.push(("key", k));
                }
                if let Some(h) = host_key {
                    v.push(("host_key", h));
                }
                v
            }
            Chore::Http { url, body, .. } => {
                let mut v = vec![("url", url)];
                if let Some(b) = body {
                    v.push(("body", b));
                }
                v
            }
            Chore::Ssh { on, command, key, host_key, .. } => {
                let mut v = vec![("on", on), ("command", command)];
                if let Some(k) = key {
                    v.push(("key", k));
                }
                if let Some(h) = host_key {
                    v.push(("host_key", h));
                }
                v
            }
            Chore::Winrm { on, command, cert, .. } => {
                let mut v = vec![("on", on), ("command", command)];
                if let Some(c) = cert {
                    v.push(("cert", c));
                }
                v
            }
            Chore::Mail { to, cc, subject, body, files, .. } => {
                let mut v = vec![("subject", subject)];
                v.extend(to.iter_mut().map(|t| ("to", t)));
                v.extend(cc.iter_mut().map(|c| ("cc", c)));
                v.extend(body.iter_mut().map(|b| ("body", b)));
                v.extend(files.iter_mut().map(|f| ("files", f)));
                v
            }
            Chore::Sql { db, sql, file, out, .. } => {
                let mut v = vec![("db", db)];
                v.extend(sql.iter_mut().map(|s| ("sql", s)));
                v.extend(file.iter_mut().map(|f| ("file", f)));
                v.extend(out.iter_mut().map(|o| ("out", o)));
                v
            }
        }
    }

    /// Every path of it, as it is.
    pub fn paths(&self) -> Vec<&String> {
        match self {
            Chore::Delete { path, .. } | Chore::Empty { path } => vec![path],
            Chore::Zip { from, into } | Chore::Copy { from, into, .. } => vec![from, into],
            Chore::Send { from, into, key, host_key, .. } | Chore::Fetch { from, into, key, host_key, .. } => {
                [from, into].into_iter().chain(key).chain(host_key).collect()
            }
            Chore::Http { url, body, .. } => [url].into_iter().chain(body).collect(),
            Chore::Ssh { on, command, key, host_key, .. } => [on, command].into_iter().chain(key).chain(host_key).collect(),
            Chore::Winrm { on, command, cert, .. } => [on, command].into_iter().chain(cert).collect(),
            Chore::Mail { to, cc, subject, body, files, .. } => {
                [subject].into_iter().chain(to).chain(cc).chain(body).chain(files).collect()
            }
            Chore::Sql { db, sql, file, out, .. } => [db].into_iter().chain(sql).chain(file).chain(out).collect(),
        }
    }

    /// Why it may not run as it stands: a path that is empty, a drive's root
    /// for `delete` and `empty`, a zip that does not end in `.zip`, an `sftp://`
    /// that cannot be read or is on the wrong side, an `ssh` without its host,
    /// command or host key, a `winrm` without its user, host, command, certificate
    /// or password, a `mail` without a good address or a subject, a `sql` without
    /// a database it can read, a login that fits it, or its SQL.
    pub fn check(&self) -> Result<(), String> {
        let need = |name: &str, p: &str| if p.trim().is_empty() { Err(format!("{name} が空")) } else { Ok(()) };
        match self {
            Chore::Delete { path, keep_days, .. } => {
                need("path", path)?;
                not_root(path)?;
                if *keep_days == 0 {
                    return Err("keep_days は 1 以上（全部を消すなら kind = \"empty\"）".into());
                }
            }
            Chore::Empty { path } => {
                need("path", path)?;
                not_root(path)?;
            }
            Chore::Zip { from, into } => {
                need("from", from)?;
                need("into", into)?;
                if !into.trim().to_ascii_lowercase().ends_with(".zip") {
                    return Err(format!("into は .zip のファイル: {into}"));
                }
            }
            Chore::Copy { from, into, .. } => {
                need("from", from)?;
                need("into", into)?;
            }
            Chore::Send { from, into, key, host_key, password } => {
                need("from", from)?;
                need("into", into)?;
                if sftp(from).is_some() {
                    return Err("send の from はこのマシンのファイル（sftp:// は into に書く）".into());
                }
                let far = sftp(into).transpose()?;
                login("sftp://", far.is_some(), true, key, host_key, password)?;
            }
            Chore::Fetch { from, into, key, host_key, password } => {
                need("from", from)?;
                need("into", into)?;
                if sftp(into).is_some() {
                    return Err("fetch の into はこのマシンのフォルダー（sftp:// は from に書く）".into());
                }
                let far = sftp(from).transpose()?;
                if let Some(at) = &far {
                    if at.path == "." || at.path.ends_with('/') {
                        return Err(format!("fetch の from はファイルの名前か *.csv のような形まで書く: {from}"));
                    }
                }
                login("sftp://", far.is_some(), true, key, host_key, password)?;
            }
            Chore::Http { url, method, body, content_type, headers, expect } => {
                need("url", url)?;
                // A url that starts with `{param:…}` or `{var:…}` is told once it is filled in.
                let u = url.trim().to_ascii_lowercase();
                if !u.starts_with('{') && !u.starts_with("http://") && !u.starts_with("https://") {
                    return Err(format!("url は http:// か https:// で始める: {url}"));
                }
                if !METHODS.contains(&method.as_str()) {
                    return Err(format!("method は {} のどれか: {method}", METHODS.join("・")));
                }
                if body.is_some() && (method == "GET" || method == "HEAD") {
                    return Err(format!("method = \"{method}\" には body を書けない（送るなら method = \"POST\" など）"));
                }
                if body.as_ref().is_some_and(|b| b.chars().count() > BODY_MAX) {
                    return Err(format!("body は {BODY_MAX} 文字まで"));
                }
                if content_type.is_some() && body.is_none() {
                    return Err("content_type は body を書いたときだけ".into());
                }
                for k in headers.keys() {
                    header_name(k)?;
                }
                if let Some(c) = expect.iter().find(|c| !(100..=599).contains(*c)) {
                    return Err(format!("expect は 100〜599 の状態コード: {c}"));
                }
            }
            Chore::Ssh { on, command, key, host_key, password, max, pass } => {
                if on.trim().is_empty() {
                    return Err("on が空（動かす PC を ssh://[ユーザー@]ホスト[:ポート] か group:組の名前 で書く）".into());
                }
                let group = fan(on, *max, *pass)?;
                // An `on` that starts with `{param:…}` or `{var:…}` is told once it is filled in.
                if !group && ssh(on).transpose()?.is_none() && !on.trim().starts_with('{') {
                    return Err(format!("on は ssh://[ユーザー@]ホスト[:ポート] か group:組の名前 で書く: {on}"));
                }
                far_command(command)?;
                // A group's PCs may each have their own host key in `[groups]`.
                login("ssh://", true, !group, key, host_key, password)?;
            }
            Chore::Winrm { on, command, cert, password, max, pass, .. } => {
                if on.trim().is_empty() {
                    return Err("on が空（動かす PC を winrm://ユーザー@ホスト[:ポート] か group:組の名前 で書く）".into());
                }
                let group = fan(on, *max, *pass)?;
                if !group && winrm(on).transpose()?.is_none() && !on.trim().starts_with('{') {
                    return Err(format!("on は winrm://ユーザー@ホスト[:ポート] か group:組の名前 で書く: {on}"));
                }
                far_command(command)?;
                match cert.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                    // Each PC of a group may have its own in `[groups]`.
                    None if group => {}
                    None => {
                        return Err("winrm:// には cert が要る（相手の証明書の指紋 AB:CD:…。koyomi winrm cert <ホスト> で調べる）".into());
                    }
                    Some(c) if c.starts_with('{') => {}
                    Some(c) => cert_ok(c)?,
                }
                match password.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
                    None => {
                        return Err(
                            "winrm:// には password が要る（secret:<名前> で書き、値は koyomi secret set <名前> で OS の資格情報に置く）"
                                .into(),
                        );
                    }
                    Some(p) if p.strip_prefix("secret:").is_none_or(|n| n.trim().is_empty()) => {
                        return Err("password は secret:<名前> で書く（値は koyomi secret set <名前> で OS の資格情報に置く）".into());
                    }
                    Some(_) => {}
                }
            }
            Chore::Mail { to, cc, subject, body, files, .. } => {
                if to.is_empty() {
                    return Err("to が空（送り先のアドレスを書く。例: to = [\"ops@example.com\"]）".into());
                }
                for a in to.iter().chain(cc) {
                    mail_address(a)?;
                }
                if subject.trim().is_empty() {
                    return Err("subject が空（メールの件名を書く）".into());
                }
                if subject.contains(['\r', '\n']) {
                    return Err("subject に改行は書けない".into());
                }
                if body.as_ref().is_some_and(|b| b.chars().count() > BODY_MAX) {
                    return Err(format!("body は {BODY_MAX} 文字まで"));
                }
                if files.len() > FILES_MAX {
                    return Err(format!("files は {FILES_MAX} 個まで"));
                }
                if files.iter().any(|f| f.trim().is_empty()) {
                    return Err("files に空のものがある".into());
                }
            }
            Chore::Sql { db, password, sql, file, out, rows, tls } => {
                if db.trim().is_empty() {
                    return Err(
                        "db が空（流す先を sqlserver://ホスト/データベース・postgres://ユーザー@ホスト/データベース・sqlite://C:\\data\\app.db で書く）"
                            .into(),
                    );
                }
                let password = password.as_deref().map(str::trim).filter(|p| !p.is_empty());
                if let Some(p) = password {
                    if p.strip_prefix("secret:").is_none_or(|n| n.trim().is_empty()) {
                        return Err("password は secret:<名前> で書く（値は koyomi secret set <名前> で OS の資格情報に置く）".into());
                    }
                }
                // A `db` that starts with `{param:…}` or `{var:…}` is told once it is filled in.
                if !db.trim().starts_with('{') {
                    match sql_db(db)? {
                        SqlDb::SqlServer { user: None, .. } if password.is_some() => {
                            return Err("ユーザーを書かない sqlserver:// はジョブを動かす Windows のアカウントでログインする: password は書かない（SQL Server のログインなら sqlserver://ユーザー@ホスト/データベース）".into());
                        }
                        SqlDb::SqlServer { user: Some(_), .. } | SqlDb::Postgres { .. } if password.is_none() => {
                            return Err("このログインには password が要る（secret:<名前> で書き、値は koyomi secret set <名前> で OS の資格情報に置く）".into());
                        }
                        SqlDb::SqlServer { .. } if *tls == SqlTls::Off => {
                            return Err(
                                "SQL Server へはいつも暗号化してつなぐ: tls = \"off\" は使えない（証明書を確かめないなら tls = \"trust\"）"
                                    .into(),
                            );
                        }
                        SqlDb::Sqlite { .. } if password.is_some() => return Err("sqlite:// には password を書かない".into()),
                        SqlDb::Sqlite { .. } if *tls != SqlTls::Verify => return Err("sqlite:// には tls を書かない".into()),
                        _ => {}
                    }
                }
                match (sql, file) {
                    (None, None) => return Err("sql か file のどちらかを書く（流す SQL か、それを書いたファイル）".into()),
                    (Some(_), Some(_)) => return Err("sql と file はどちらか一方".into()),
                    (Some(s), None) if s.trim().is_empty() => return Err("sql が空".into()),
                    (Some(s), None) if s.chars().count() > COMMAND_MAX => {
                        return Err(format!("sql は {COMMAND_MAX} 文字まで（長いものは file に書く）"));
                    }
                    (None, Some(f)) if f.trim().is_empty() => return Err("file が空".into()),
                    _ => {}
                }
                if out.as_ref().is_some_and(|o| o.trim().is_empty()) {
                    return Err("out が空".into());
                }
                if let Some(r) = rows {
                    sql_rows(r)?;
                }
            }
        }
        Ok(())
    }

    /// The PowerShell script that does it, and how its exit code is judged.
    /// The paths are already filled in. An `ssh`, `winrm`, `mail` or `sql` has none:
    /// [`fill`] runs `koyomi ssh run`, `koyomi winrm run`, `koyomi mail run` or
    /// `koyomi sql run` straight.
    pub fn script(&self) -> (String, Option<Judge>) {
        match self {
            Chore::Delete { path, pattern, keep_days } => {
                let s = format!(
                    "$ErrorActionPreference = 'Stop'\n\
                     $limit = (Get-Date).AddDays(-{keep_days})\n\
                     $old = @(Get-ChildItem -LiteralPath {p} -Filter {f} -File | Where-Object {{ $_.LastWriteTime -lt $limit }})\n\
                     foreach ($x in $old) {{ Remove-Item -LiteralPath $x.FullName -Force; Write-Output \"deleted: $($x.FullName)\" }}\n\
                     Write-Output \"$($old.Count) file(s) deleted\"\n",
                    p = quote(path),
                    f = quote(pattern.as_deref().unwrap_or("*")),
                );
                (s, None)
            }
            Chore::Zip { from, into } => {
                let s = format!(
                    "$ErrorActionPreference = 'Stop'\n\
                     $into = {i}\n\
                     $dir = Split-Path -Parent $into\n\
                     if ($dir) {{ New-Item -ItemType Directory -Force -Path $dir | Out-Null }}\n\
                     Compress-Archive -Path {f} -DestinationPath $into -Force\n\
                     Write-Output \"zipped: $into\"\n",
                    i = quote(into),
                    f = quote(from),
                );
                (s, None)
            }
            Chore::Copy { from, into, pattern } => {
                let only = pattern.as_deref().map(|p| format!(" {}", quote(p))).unwrap_or_default();
                // robocopy: 0–3 copied or nothing to do, 4–7 something did not match, 8 and up failed.
                let s = format!(
                    "& robocopy {f} {i}{only} /E /R:1 /W:1 /NP\n\
                     $c = $LASTEXITCODE\n\
                     if ($c -ge 8) {{ exit 2 }}\n\
                     if ($c -ge 4) {{ exit 1 }}\n\
                     exit 0\n",
                    f = quote(&folder(from)),
                    i = quote(&folder(into)),
                );
                (s, Some(Judge { warn: vec![Range { from: 1, to: 1 }], ..Default::default() }))
            }
            Chore::Empty { path } => {
                let s = format!(
                    "$ErrorActionPreference = 'Stop'\n\
                     $all = @(Get-ChildItem -LiteralPath {p} -Force)\n\
                     foreach ($x in $all) {{ Remove-Item -LiteralPath $x.FullName -Recurse -Force }}\n\
                     Write-Output \"$($all.Count) item(s) removed\"\n",
                    p = quote(path),
                );
                (s, None)
            }
            Chore::Send { from, into, .. } | Chore::Fetch { from, into, .. } => {
                let warn = Some(Judge { warn: vec![Range { from: NOTHING, to: NOTHING }], ..Default::default() });
                let s = match (self, sftp(from).or_else(|| sftp(into))) {
                    // What to do is in the environment (env()): no path or secret is written here.
                    (_, Some(Ok(_))) => SFTP_SCRIPT.to_owned(),
                    (Chore::Send { .. }, _) => shared(from, into, "send", "sent"),
                    _ => shared(from, into, "fetch", "fetched"),
                };
                (s, warn)
            }
            Chore::Http { url, method, body, content_type, headers, expect } => {
                (http(url, method, body, content_type, headers, expect), None)
            }
            // Not a script (fill()); judged by the job's own `judge`, a `mail` and a `sql` as 0 or not.
            Chore::Ssh { .. } | Chore::Winrm { .. } | Chore::Mail { .. } | Chore::Sql { .. } => (String::new(), None),
        }
    }

    /// The environment the script reads, beside the job's own: an `http` header's
    /// value (maybe `secret:<name>`) as `KOYOMI_HEADER_<n>`, in the order of `headers`;
    /// an SFTP `send` or `fetch` as `KOYOMI_SFTP_*`, which `koyomi sftp run` reads
    /// (the password as `secret:<name>`, read when the job starts); an `ssh` as
    /// `KOYOMI_SSH_*`, which `koyomi ssh run` reads (fill() adds `KOYOMI_SSH_ENV`); a
    /// `winrm` as `KOYOMI_WINRM_*` for `koyomi winrm run` (and `KOYOMI_WINRM_ENV`),
    /// its `auth` always (`basic` when none is written); a `mail` as `KOYOMI_MAIL_*`
    /// for `koyomi mail run`, a list one per line, each after [`MAIL_TEXT`]; a `sql`
    /// as `KOYOMI_SQL_*` for `koyomi sql run`, each text after [`MAIL_TEXT`] but the
    /// password (a `secret:`) and `tls`.
    pub fn env(&self) -> Vec<(String, String)> {
        match self {
            Chore::Http { headers, .. } => headers.values().enumerate().map(|(i, v)| (format!("KOYOMI_HEADER_{i}"), v.clone())).collect(),
            Chore::Send { from, into, key, host_key, password } | Chore::Fetch { from, into, key, host_key, password }
                if sftp(from).or_else(|| sftp(into)).is_some() =>
            {
                let mut v = vec![
                    ("KOYOMI_SFTP_OP".to_owned(), self.key().to_owned()),
                    ("KOYOMI_SFTP_FROM".to_owned(), from.trim().to_owned()),
                    ("KOYOMI_SFTP_INTO".to_owned(), into.trim().to_owned()),
                ];
                for (name, value) in [("KEY", key), ("HOST_KEY", host_key), ("PASSWORD", password)] {
                    if let Some(x) = value.as_deref().map(str::trim).filter(|x| !x.is_empty()) {
                        v.push((format!("KOYOMI_SFTP_{name}"), x.to_owned()));
                    }
                }
                v
            }
            Chore::Ssh { on, command, key, host_key, password, max, pass } => {
                let mut v = vec![("KOYOMI_SSH_ON".to_owned(), on.trim().to_owned()), ("KOYOMI_SSH_COMMAND".to_owned(), command.clone())];
                for (name, value) in [("KEY", key), ("HOST_KEY", host_key), ("PASSWORD", password)] {
                    if let Some(x) = value.as_deref().map(str::trim).filter(|x| !x.is_empty()) {
                        v.push((format!("KOYOMI_SSH_{name}"), x.to_owned()));
                    }
                }
                v.extend(fan_env("KOYOMI_SSH_", *max, *pass));
                v
            }
            Chore::Winrm { on, command, cert, password, auth, max, pass } => {
                let mut v = vec![
                    ("KOYOMI_WINRM_ON".to_owned(), on.trim().to_owned()),
                    ("KOYOMI_WINRM_COMMAND".to_owned(), command.clone()),
                    ("KOYOMI_WINRM_AUTH".to_owned(), auth.key().to_owned()),
                ];
                for (name, value) in [("CERT", cert), ("PASSWORD", password)] {
                    if let Some(x) = value.as_deref().map(str::trim).filter(|x| !x.is_empty()) {
                        v.push((format!("KOYOMI_WINRM_{name}"), x.to_owned()));
                    }
                }
                v.extend(fan_env("KOYOMI_WINRM_", *max, *pass));
                v
            }
            Chore::Mail { to, cc, subject, body, files, via } => {
                let text = |s: &str| format!("{MAIL_TEXT}{s}");
                let lines = |l: &[String]| text(&l.iter().map(|x| x.trim()).collect::<Vec<_>>().join("\n"));
                let mut v = vec![("KOYOMI_MAIL_TO".to_owned(), lines(to)), ("KOYOMI_MAIL_SUBJECT".to_owned(), text(subject.trim()))];
                if !cc.is_empty() {
                    v.push(("KOYOMI_MAIL_CC".to_owned(), lines(cc)));
                }
                if let Some(b) = body {
                    v.push(("KOYOMI_MAIL_BODY".to_owned(), text(b)));
                }
                if !files.is_empty() {
                    v.push(("KOYOMI_MAIL_FILES".to_owned(), lines(files)));
                }
                if let Some(x) = via.as_deref().map(str::trim).filter(|x| !x.is_empty()) {
                    v.push(("KOYOMI_MAIL_VIA".to_owned(), text(x)));
                }
                v
            }
            Chore::Sql { db, password, sql, file, out, rows, tls } => {
                let text = |s: &str| format!("{MAIL_TEXT}{s}");
                let mut v = vec![("KOYOMI_SQL_DB".to_owned(), text(db.trim())), ("KOYOMI_SQL_TLS".to_owned(), tls.key().to_owned())];
                if let Some(s) = sql {
                    v.push(("KOYOMI_SQL_TEXT".to_owned(), text(s)));
                }
                for (name, value) in [("FILE", file), ("OUT", out), ("ROWS", rows)] {
                    if let Some(x) = value.as_deref().map(str::trim).filter(|x| !x.is_empty()) {
                        v.push((format!("KOYOMI_SQL_{name}"), text(x)));
                    }
                }
                if let Some(p) = password.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
                    v.push(("KOYOMI_SQL_PASSWORD".to_owned(), p.to_owned()));
                }
                v
            }
            _ => Vec::new(),
        }
    }
}

/// An address of a `mail`'s `to` or `cc`: `ops@example.com` or `名前 <ops@example.com>`,
/// as ito's mail reads it; one that starts with `{param:…}` or `{var:…}` is told once it is filled in.
pub fn mail_address(a: &str) -> Result<(), String> {
    let t = a.trim();
    if t.starts_with('{') {
        return Ok(());
    }
    let bare = match (t.rfind('<'), t.ends_with('>')) {
        (Some(i), true) => Some(t[i + 1..t.len() - 1].trim()),
        (None, false) => Some(t),
        _ => None,
    };
    let good = |b: &str| {
        b.split_once('@').is_some_and(|(l, d)| {
            !l.is_empty()
                && !d.is_empty()
                && !d.contains('@')
                && !b.chars().any(|c| c.is_whitespace() || c.is_control() || "<>\"".contains(c))
        })
    };
    if !t.contains(['\r', '\n', ',', ';']) && bare.is_some_and(good) {
        Ok(())
    } else {
        Err(format!("メールのアドレスとして読めない: {a}（例: ops@example.com か 名前 <ops@example.com>。1 つに 1 人）"))
    }
}

/// A header name as HTTP takes it; `Content-Type` is `content_type`.
pub fn header_name(k: &str) -> Result<(), String> {
    if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c)) {
        return Err(format!("headers の名前に使えない: {k}（英数字と - など）"));
    }
    if k.eq_ignore_ascii_case("content-type") {
        return Err("Content-Type は headers でなく content_type に書く".into());
    }
    Ok(())
}

/// The script of an `http`: Windows PowerShell 5.1's `Invoke-WebRequest` (TLS 1.2
/// turned on, no progress bar: it slows the download and reaches stderr as CLIXML),
/// the status code and the start of the answer to the log, then the exit code: 0
/// when the code is expected, 1 when it is not, [`NO_ANSWER`] when there was no
/// answer.
fn http(
    url: &str,
    method: &str,
    body: &Option<String>,
    content_type: &Option<String>,
    headers: &BTreeMap<String, String>,
    expect: &[u16],
) -> String {
    let mut s = format!(
        "$ErrorActionPreference = 'Stop'\n\
         $ProgressPreference = 'SilentlyContinue'\n\
         [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12\n\
         $p = @{{ Uri = {u}; Method = '{method}'; UseBasicParsing = $true; Headers = @{{}} }}\n",
        u = quote(url),
    );
    for (i, k) in headers.keys().enumerate() {
        s += &format!("$p.Headers[{}] = $env:KOYOMI_HEADER_{i}\n", quote(k));
    }
    if let Some(b) = body {
        // As UTF-8 bytes: a string would go as the system's code page.
        s += &format!(
            "$p.Body = [Text.Encoding]::UTF8.GetBytes({})\n$p.ContentType = {}\n",
            quote_raw(b),
            quote(content_type.as_deref().unwrap_or("application/json; charset=utf-8")),
        );
    }
    let ok = if expect.is_empty() {
        "$code -ge 200 -and $code -le 299".to_owned()
    } else {
        format!("@({}) -contains $code", expect.iter().map(u16::to_string).collect::<Vec<_>>().join(","))
    };
    s += &format!(
        "try {{ $r = Invoke-WebRequest @p; $code = [int]$r.StatusCode; $text = $r.Content }}\n\
         catch {{\n\
         \x20 $res = $_.Exception.Response\n\
         \x20 if ($null -eq $res) {{ Write-Output \"no answer: $($_.Exception.Message)\"; exit {NO_ANSWER} }}\n\
         \x20 $code = [int]$res.StatusCode\n\
         \x20 $text = $_.ErrorDetails.Message\n\
         }}\n\
         Write-Output \"HTTP $code\"\n\
         if ($text -is [byte[]]) {{ $text = [Text.Encoding]::UTF8.GetString($text) }}\n\
         if ($text) {{ if ($text.Length -gt 4000) {{ $text = $text.Substring(0, 4000) + ' ...' }}; Write-Output $text }}\n\
         if ({ok}) {{ exit 0 }}\n\
         exit 1\n",
    );
    s
}

/// Gives each chore of `net` (its paths filled in) the script that does it, so it
/// runs as a PowerShell job; a root or an empty path after the filling stops the run.
/// An `ssh` runs `exe` (koyomi itself) as `exe ssh run`, not through PowerShell:
/// the engine's ask to stop reaches it straight, and PowerShell's debugger does
/// not take the ask and write into the log. It keeps its own `judge`, and the
/// names of its own `env` go to the far side as `KOYOMI_SSH_ENV`. A `winrm` the
/// same, as `exe winrm run` and `KOYOMI_WINRM_ENV`; a `mail` as `exe mail run`, a
/// `sql` as `exe sql run`.
pub fn fill(net: &mut Jobnet, exe: &str) -> Result<(), String> {
    for j in &mut net.jobs {
        if let Part::Chore(c) = &j.part {
            c.check().map_err(|e| format!("{}: {e}", j.id))?;
            if c.own() {
                let op = c.key();
                (j.shell, j.script) = (None, None);
                j.run = [exe, op, "run"].map(str::to_owned).to_vec();
                if c.far() {
                    // Not its own `KOYOMI_SSH_*`, should a net be filled twice.
                    let prefix = format!("KOYOMI_{}_", op.to_ascii_uppercase());
                    let names = j.env.keys().filter(|k| !k.starts_with(&prefix)).map(String::as_str).collect::<Vec<_>>().join(",");
                    j.env.insert(format!("{prefix}ENV"), names);
                }
            } else {
                let (script, judge) = c.script();
                (j.shell, j.script, j.judge) = (Some("powershell".to_owned()), Some(script), judge);
                j.run.clear();
            }
            j.env.extend(c.env());
        }
    }
    Ok(())
}

/// A PowerShell string that is `s` as it is: `'…'`, a `'` doubled.
fn quote(s: &str) -> String {
    quote_raw(s.trim())
}

/// [`quote`] without the trim, for a body. PowerShell also ends a `'…'` at the
/// typographic quotes, so they are doubled too.
fn quote_raw(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// A folder for robocopy: no `\` at the end (it would escape the closing quote),
/// but a drive's root keeps one as `D:\.`.
fn folder(p: &str) -> String {
    let t = p.trim().trim_end_matches(['\\', '/']);
    if t.len() == 2 && t.ends_with(':') {
        format!("{t}\\.")
    } else {
        t.to_owned()
    }
}

/// `delete` and `empty` never work on a drive or share root.
fn not_root(path: &str) -> Result<(), String> {
    let t = path.trim().trim_end_matches(['\\', '/']);
    let share_root = t.strip_prefix("\\\\").is_some_and(|rest| rest.split(['\\', '/']).filter(|s| !s.is_empty()).count() <= 2);
    if t.is_empty() || (t.len() == 2 && t.ends_with(':')) || share_root {
        return Err(format!("ドライブや共有の一番上は扱わない: {path}"));
    }
    Ok(())
}

/// What an SFTP `send` or `fetch` runs. Through a pipe, so PowerShell waits for koyomi
/// (a windowed program) and reads its UTF-8 lines.
const SFTP_SCRIPT: &str =
    "[Console]::OutputEncoding = [Text.Encoding]::UTF8\n& $env:KOYOMI_EXE sftp run | ForEach-Object { $_ }\nexit $LASTEXITCODE\n";

/// Where an `sftp://[user@]host[:port]/path` points.
#[derive(Debug, PartialEq, Eq)]
pub struct Sftp {
    /// The user to log in as; without one, the account's own name.
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
    /// The path on the server: `/in` from `…/in`, `in` (below the home) from `…/~/in`,
    /// `.` (the home) when there is none.
    pub path: String,
}

/// `s` read as an `sftp://` address; `None` when it is not one (a folder or share).
pub fn sftp(s: &str) -> Option<Result<Sftp, String>> {
    let t = s.trim();
    let rest = t.get(..7).filter(|p| p.eq_ignore_ascii_case("sftp://")).map(|_| &t[7..])?;
    Some(read_sftp(rest, "sftp://").map_err(|e| format!("{e}: {t}")))
}

/// `s` read as an `ssh://[user@]host[:port]` address (no path: `path` is `.`); `None`
/// when it does not start with `ssh://`.
pub fn ssh(s: &str) -> Option<Result<Sftp, String>> {
    let t = s.trim();
    let rest = t.get(..6).filter(|p| p.eq_ignore_ascii_case("ssh://")).map(|_| &t[6..])?;
    let read = read_sftp(rest, "ssh://").and_then(|far| match rest.split_once('/') {
        Some((_, p)) if !p.is_empty() => Err("ssh:// にパスは書かない（ssh://[ユーザー@]ホスト[:ポート]）".into()),
        _ => Ok(far),
    });
    Some(read.map_err(|e| format!("{e}: {t}")))
}

/// `s` read as a `winrm://user@host[:port]` address (no path: `path` is `.`, no
/// port: [`WINRM_PORT`]); `None` when it does not start with `winrm://`. The user
/// is needed: a password is for someone. It may be `DOMAIN\user` or `.\user`.
pub fn winrm(s: &str) -> Option<Result<Sftp, String>> {
    let t = s.trim();
    let rest = t.get(..8).filter(|p| p.eq_ignore_ascii_case("winrm://")).map(|_| &t[8..])?;
    let read = read_sftp(rest, "winrm://").and_then(|far| match rest.split_once('/') {
        Some((_, p)) if !p.is_empty() => Err("winrm:// にパスは書かない（winrm://ユーザー@ホスト[:ポート]）".into()),
        _ if far.user.is_none() => Err("winrm:// にはユーザーが要る（winrm://ユーザー@ホスト[:ポート]）".into()),
        _ => Ok(far),
    });
    Some(read.map_err(|e| format!("{e}: {t}")))
}

/// Where a `sql`'s `db` points.
#[derive(Debug, PartialEq, Eq)]
pub enum SqlDb {
    /// `sqlserver://[user@]host[:port]/database`: without a user, the Windows
    /// account the job runs as logs in.
    SqlServer { user: Option<String>, host: String, port: u16, database: String },
    /// `postgres://user@host[:port]/database` (or `postgresql://`).
    Postgres { user: String, host: String, port: u16, database: String },
    /// `sqlite://C:\data\app.db`: the file, by its whole path.
    Sqlite { path: String },
}

/// `t` after `scheme`, whatever its case; `None` when it does not start with it.
fn after_scheme<'a>(t: &'a str, scheme: &str) -> Option<&'a str> {
    t.get(..scheme.len()).filter(|p| p.eq_ignore_ascii_case(scheme)).map(|_| &t[scheme.len()..])
}

/// `s` read as a `sql`'s `db`. A named instance (`host\name`) is not taken: its
/// port is written instead, as ito-sql connects by the port.
pub fn sql_db(s: &str) -> Result<SqlDb, String> {
    let t = s.trim();
    if let Some(path) = after_scheme(t, "sqlite://") {
        let b = path.as_bytes();
        let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
        if path.contains(['\r', '\n']) || !(drive || path.starts_with(['\\', '/', '{'])) {
            return Err(format!("sqlite:// のあとはデータベースのファイルを C:\\data\\app.db のように全部書く: {t}"));
        }
        return Ok(SqlDb::Sqlite { path: path.to_owned() });
    }
    let (rest, scheme, server) = if let Some(r) = after_scheme(t, "sqlserver://") {
        (r, "sqlserver://", true)
    } else if let Some(r) = after_scheme(t, "postgres://") {
        (r, "postgres://", false)
    } else if let Some(r) = after_scheme(t, "postgresql://") {
        (r, "postgresql://", false)
    } else {
        return Err(format!("db は sqlserver://・postgres://・sqlite:// のどれかで書く: {t}"));
    };
    let form = if server {
        "sqlserver://[ユーザー@]ホスト[:ポート]/データベース"
    } else {
        "postgres://ユーザー@ホスト[:ポート]/データベース"
    };
    let Some((auth, database)) = rest.split_once('/').filter(|(_, d)| !d.trim().is_empty()) else {
        return Err(format!("{scheme} にはデータベースの名前が要る（{form}）: {t}"));
    };
    if database.contains(['/', '?']) || database.chars().any(char::is_control) {
        return Err(format!("データベースの名前に使えない文字がある: {t}"));
    }
    let far = read_sftp(auth, scheme).map_err(|e| format!("{e}: {t}"))?;
    if far.host.contains('\\') {
        let why = if server {
            "名前付きインスタンス（ホスト\\名前）は使えない: そのポートを書く（sqlserver://ホスト:ポート/データベース）"
        } else {
            "ホストに \\ は使えない"
        };
        return Err(format!("{why}: {t}"));
    }
    if far.user.as_deref().is_some_and(|u| u.contains('\\')) {
        let why = if server {
            "Windows のアカウントでログインするならユーザーを書かない（ジョブを動かすアカウントでログインする。ほかのアカウントは run_as）"
        } else {
            "ユーザーに \\ は使えない"
        };
        return Err(format!("{why}: {t}"));
    }
    let database = database.to_owned();
    if server {
        return Ok(SqlDb::SqlServer { user: far.user, host: far.host, port: far.port.unwrap_or(SQL_SERVER_PORT), database });
    }
    let Some(user) = far.user else {
        return Err(format!("{scheme} にはユーザーが要る（{form}）: {t}"));
    };
    Ok(SqlDb::Postgres { user, host: far.host, port: far.port.unwrap_or(POSTGRES_PORT), database })
}

/// A `sql`'s `rows` read as the counts that are 正常, each `(least, most)`:
/// `0`, `1..` (one or more), `1..100`, `..5`, several with `,`.
pub fn sql_rows(s: &str) -> Result<Vec<(u64, u64)>, String> {
    let bad = || format!("rows は 0・1..・1..100 のように書く（正常とする行の数。, で並べられる）: {s}");
    let num = |x: &str, none: u64| match x.trim() {
        "" => Some(none),
        n => n.parse::<u64>().ok(),
    };
    s.split(',')
        .map(|part| {
            let part = part.trim();
            let range = match part.split_once("..") {
                _ if part.is_empty() => None,
                Some((a, b)) => num(a, 0).zip(num(b, u64::MAX)),
                None => part.parse::<u64>().ok().map(|n| (n, n)),
            };
            range.filter(|(a, b)| a <= b).ok_or_else(bad)
        })
        .collect()
}

/// `max` and `pass` of an `ssh` or `winrm` for `koyomi ssh run` or `koyomi winrm
/// run`, as `<prefix>MAX` and `<prefix>PASS`, when they are written.
fn fan_env(prefix: &str, max: Option<u32>, pass: Pass) -> Vec<(String, String)> {
    let mut v = Vec::new();
    if let Some(m) = max {
        v.push((format!("{prefix}MAX"), m.to_string()));
    }
    if pass != Pass::All {
        v.push((format!("{prefix}PASS"), pass.key().to_owned()));
    }
    v
}

/// The command of an `ssh` or `winrm`: something, and not past [`COMMAND_MAX`].
fn far_command(command: &str) -> Result<(), String> {
    if command.trim().is_empty() {
        return Err("command が空（相手の PC で動かすコマンドを書く）".into());
    }
    if command.chars().count() > COMMAND_MAX {
        return Err(format!("command は {COMMAND_MAX} 文字まで"));
    }
    Ok(())
}

/// A certificate's SHA-256 as `koyomi winrm cert` prints it: 32 bytes in hex,
/// `AB:CD:…` (the colons and the case as you like).
pub fn cert_ok(c: &str) -> Result<(), String> {
    let hex: String = c.chars().filter(|x| *x != ':' && !x.is_whitespace()).collect();
    if hex.len() == 64 && hex.chars().all(|x| x.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!("cert は AB:CD:… の形（SHA-256 の 32 バイトを 16 進で）で書く（koyomi winrm cert <ホスト> が出すもの）: {c}"))
    }
}

fn read_sftp(rest: &str, scheme: &str) -> Result<Sftp, String> {
    let (auth, path) = rest.split_once('/').unwrap_or((rest, ""));
    // A `:` inside `{param:…}` or `{var:…}` is not a port.
    let (dest, port) = match auth.rsplit_once(':') {
        Some((d, p)) if !p.contains('}') => match p.parse::<u16>() {
            Ok(n) if n > 0 => (d, Some(n)),
            _ => return Err(format!("ポートは 1〜65535 の数で書く（{p}）")),
        },
        _ => (auth, None),
    };
    let (user, host) = match dest.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, dest),
    };
    if host.is_empty() || user == Some("") {
        return Err(format!("{scheme} には [ユーザー@]ホスト が要る"));
    }
    if host.starts_with('-') || dest.chars().any(|c| c.is_whitespace() || "\"'`;&|<>".contains(c)) {
        return Err("ユーザーとホストに使えない文字がある".into());
    }
    if path.contains(['\n', '\r']) {
        return Err("パスに改行は使えない".into());
    }
    let path = match path.strip_prefix('~') {
        _ if path.is_empty() || path == "~" => ".".to_owned(),
        Some(below) => below.trim_start_matches('/').to_owned(),
        None => format!("/{path}"),
    };
    Ok(Sftp { user: user.map(str::to_owned), host: host.to_owned(), port, path })
}

/// How an SFTP `send` or `fetch`, or an `ssh`, logs in when there is a `far` side
/// (`scheme`): `host_key` is needed, `password` is a `secret:`, and `key` and
/// `password` go one at a time. Without a far side, none of them is written.
fn login(
    scheme: &str,
    far: bool,
    need_host_key: bool,
    key: &Option<String>,
    host_key: &Option<String>,
    password: &Option<String>,
) -> Result<(), String> {
    let set = |v: &Option<String>| v.as_deref().is_some_and(|x| !x.trim().is_empty());
    if !far {
        if let Some((name, _)) = [("key", key), ("host_key", host_key), ("password", password)].into_iter().find(|(_, v)| set(v)) {
            return Err(format!("{name} は {scheme} の相手にだけ書く"));
        }
        return Ok(());
    }
    match host_key.as_deref().map(str::trim).filter(|h| !h.is_empty()) {
        None if !need_host_key => {}
        None => {
            return Err(format!("{scheme} には host_key が要る（相手のホスト鍵の指紋 SHA256:…。koyomi sftp hostkey <ホスト> で調べる）"));
        }
        // `{param:…}` or `{var:…}` is told once it is filled in.
        Some(h) if h.starts_with('{') => {}
        Some(h) => host_key_ok(h)?,
    }
    if let Some(p) = password.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        if p.strip_prefix("secret:").is_none_or(|n| n.trim().is_empty()) {
            return Err("password は secret:<名前> で書く（値は koyomi secret set <名前> で OS の資格情報に置く）".into());
        }
        if set(key) {
            return Err("key と password はどちらか一方".into());
        }
    }
    Ok(())
}

/// `SHA256:` and the digest in base64, as `ssh-keygen -l` and `koyomi sftp hostkey` print it.
pub fn host_key_ok(h: &str) -> Result<(), String> {
    let digest = h.get(..7).filter(|p| p.eq_ignore_ascii_case("sha256:")).map(|_| h[7..].trim_end_matches('='));
    match digest {
        Some(d) if d.len() == 43 && d.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/') => Ok(()),
        _ => Err(format!("host_key は SHA256:… の形で書く（koyomi sftp hostkey <ホスト> が出すもの）: {h}")),
    }
}

/// `send` and `fetch` between folders and shares: each file `from` matches is
/// copied into `into`, which is made when it is not there.
fn shared(from: &str, into: &str, verb: &str, done: &str) -> String {
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         $files = @(Get-ChildItem -Path {f} -File)\n\
         if ($files.Count -eq 0) {{ Write-Output ('no file to {verb}: ' + {f}); exit {NOTHING} }}\n\
         $into = {i}\n\
         New-Item -ItemType Directory -Force -Path $into | Out-Null\n\
         foreach ($x in $files) {{ Copy-Item -LiteralPath $x.FullName -Destination $into -Force; Write-Output \"{done}: $($x.FullName)\" }}\n",
        f = quote(from),
        i = quote(into),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scripts_quote_their_paths_and_copy_judges_robocopy() {
        let d = Chore::Delete { path: "D:\\logs\\it's".into(), pattern: Some("*.log".into()), keep_days: 30 };
        let (s, j) = d.script();
        assert!(s.contains("-LiteralPath 'D:\\logs\\it''s' -Filter '*.log'"), "{s}");
        assert!(s.contains("AddDays(-30)"), "{s}");
        assert!(j.is_none());
        let c = Chore::Copy { from: "D:\\out\\".into(), into: "E:\\".into(), pattern: None };
        let (s, j) = c.script();
        assert!(s.starts_with("& robocopy 'D:\\out' 'E:\\.' /E"), "{s}");
        assert_eq!(j.unwrap().warn, vec![Range { from: 1, to: 1 }]);
        let z = Chore::Zip { from: "D:\\out\\*.csv".into(), into: "D:\\arc\\a.zip".into() };
        assert!(z.script().0.contains("Compress-Archive -Path 'D:\\out\\*.csv'"));
    }

    #[test]
    fn roots_empty_paths_and_a_zip_without_zip_are_refused() {
        for p in ["C:\\", "C:", "\\\\nas\\share", "\\\\nas\\share\\", ""] {
            assert!(Chore::Empty { path: p.into() }.check().is_err(), "{p}");
        }
        assert!(Chore::Empty { path: "\\\\nas\\share\\tmp".into() }.check().is_ok());
        assert!(Chore::Delete { path: "D:\\logs".into(), pattern: None, keep_days: 0 }.check().is_err());
        assert!(Chore::Zip { from: "a".into(), into: "b.7z".into() }.check().is_err());
        assert!(Chore::Zip { from: "a".into(), into: "b.ZIP".into() }.check().is_ok());
    }

    const HK: &str = "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU";

    #[test]
    fn an_sftp_address_reads_and_a_strange_user_or_host_is_refused() {
        let a = sftp("sftp://partner@ftp.example.com:2222/in").unwrap().unwrap();
        assert_eq!(a, Sftp { user: Some("partner".into()), host: "ftp.example.com".into(), port: Some(2222), path: "/in".into() });
        assert_eq!(sftp("SFTP://h/~/out/*.csv").unwrap().unwrap().path, "out/*.csv");
        let home = sftp("sftp://h").unwrap().unwrap();
        assert_eq!((home.user, home.path), (None, ".".into()));
        assert_eq!(sftp("sftp://{param:user}@h/in").unwrap().unwrap().user.as_deref(), Some("{param:user}"));
        assert_eq!(sftp("sftp://h/a\"b").unwrap().unwrap().path, "/a\"b");
        assert!(sftp("\\\\nas\\share").is_none());
        for bad in
            ["sftp:///in", "sftp://@h/in", "sftp://h:0/in", "sftp://h:x/in", "sftp://-oProxyCommand=x/in", "sftp://a b/in", "sftp://h/a\nb"]
        {
            assert!(sftp(bad).unwrap().is_err(), "{bad}");
        }
    }

    #[test]
    fn send_and_fetch_take_sftp_on_the_far_side_only() {
        let hk = Some(HK.to_owned());
        let send = |from: &str, into: &str, host_key: &Option<String>| {
            Chore::Send { from: from.into(), into: into.into(), key: None, host_key: host_key.clone(), password: None }.check()
        };
        let fetch = |from: &str, into: &str, host_key: &Option<String>| {
            Chore::Fetch { from: from.into(), into: into.into(), key: None, host_key: host_key.clone(), password: None }.check()
        };
        assert!(send("D:\\out\\*.csv", "sftp://u@h/in", &hk).is_ok());
        assert!(send("D:\\out\\*.csv", "\\\\nas\\share\\in", &None).is_ok());
        assert!(send("sftp://u@h/out", "D:\\in", &hk).is_err());
        assert!(send("D:\\out", "sftp://-x/in", &hk).is_err());
        assert!(fetch("sftp://u@h/out/*.csv", "D:\\in", &hk).is_ok());
        assert!(fetch("sftp://u@h/out/", "D:\\in", &hk).is_err());
        assert!(fetch("sftp://u@h", "D:\\in", &hk).is_err());
        assert!(fetch("D:\\out\\*.csv", "sftp://u@h/in", &hk).is_err());
        assert!(fetch("\\\\nas\\share\\out\\*.csv", "D:\\in", &None).is_ok());
    }

    #[test]
    fn sftp_needs_the_host_key_and_a_password_only_as_a_secret() {
        let send = |key: Option<&str>, host_key: Option<&str>, password: Option<&str>, into: &str| {
            Chore::Send {
                from: "D:\\out".into(),
                into: into.into(),
                key: key.map(Into::into),
                host_key: host_key.map(Into::into),
                password: password.map(Into::into),
            }
            .check()
        };
        let far = "sftp://u@h/in";
        assert!(send(None, None, None, far).unwrap_err().contains("koyomi sftp hostkey"));
        assert!(send(None, Some("47DEQpj8"), None, far).unwrap_err().contains("SHA256:"));
        assert_eq!(send(None, Some(&HK.to_lowercase().replacen("sha256", "SHA256", 1)), None, far), Ok(()));
        assert_eq!(send(None, Some(&format!("{HK}=")), None, far), Ok(()));
        assert_eq!(send(None, Some("{param:hk}"), None, far), Ok(()));
        assert_eq!(send(None, Some(HK), Some("secret:partner"), far), Ok(()));
        assert!(send(None, Some(HK), Some("hunter2"), far).unwrap_err().contains("secret:<名前>"));
        assert!(send(Some("C:\\keys\\id"), Some(HK), Some("secret:partner"), far).unwrap_err().contains("どちらか一方"));
        assert!(send(None, Some(HK), None, "\\\\nas\\share\\in").unwrap_err().contains("host_key は sftp://"));
        assert!(send(None, None, Some("secret:p"), "D:\\in").unwrap_err().contains("password は sftp://"));
    }

    #[test]
    fn the_sftp_script_leaves_the_work_to_koyomi_and_judges_no_file_as_a_warning() {
        let s = Chore::Send {
            from: "D:\\out\\*.csv".into(),
            into: "sftp://u@h:2222/in box".into(),
            key: None,
            host_key: Some(HK.into()),
            password: Some("secret:partner".into()),
        };
        let (script, judge) = s.script();
        assert!(script.contains("& $env:KOYOMI_EXE sftp run | ForEach-Object"), "{script}");
        assert!(!script.contains("u@h") && !script.contains("partner"), "{script}");
        assert_eq!(judge.unwrap().warn, vec![Range { from: NOTHING, to: NOTHING }]);
        let env = s.env();
        let get = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        assert_eq!(get("KOYOMI_SFTP_OP"), Some("send"));
        assert_eq!(get("KOYOMI_SFTP_FROM"), Some("D:\\out\\*.csv"));
        assert_eq!(get("KOYOMI_SFTP_INTO"), Some("sftp://u@h:2222/in box"));
        assert_eq!(get("KOYOMI_SFTP_HOST_KEY"), Some(HK));
        assert_eq!(get("KOYOMI_SFTP_PASSWORD"), Some("secret:partner"));
        assert_eq!(get("KOYOMI_SFTP_KEY"), None);
        let f = Chore::Fetch {
            from: "sftp://u@h/~/out/*.csv".into(),
            into: "D:\\in".into(),
            key: Some("C:\\keys\\id".into()),
            host_key: Some(HK.into()),
            password: None,
        };
        assert!(f.env().contains(&("KOYOMI_SFTP_OP".to_owned(), "fetch".to_owned())));
        assert!(f.env().contains(&("KOYOMI_SFTP_KEY".to_owned(), "C:\\keys\\id".to_owned())));
        let share = Chore::Send { from: "D:\\out".into(), into: "\\\\nas\\share\\in".into(), key: None, host_key: None, password: None };
        let (script, judge) = share.script();
        assert!(script.contains("Copy-Item -LiteralPath $x.FullName -Destination $into -Force"), "{script}");
        assert!(!script.contains("sftp"), "{script}");
        assert!(judge.is_some());
        assert!(share.env().is_empty());
    }

    #[test]
    fn an_ssh_address_has_a_host_and_no_path() {
        let a = ssh("ssh://ops@db1.example.com:2222").unwrap().unwrap();
        assert_eq!((a.user.as_deref(), a.host.as_str(), a.port), (Some("ops"), "db1.example.com", Some(2222)));
        let b = ssh(" SSH://db1/ ").unwrap().unwrap();
        assert_eq!((b.user, b.host.as_str(), b.port), (None, "db1", None));
        assert_eq!(ssh("ssh://{param:who}@h").unwrap().unwrap().user.as_deref(), Some("{param:who}"));
        assert!(ssh("sftp://h").is_none());
        assert!(ssh("h").is_none());
        assert!(ssh("ssh://h/tmp").unwrap().unwrap_err().contains("パスは書かない"));
        for bad in ["ssh://", "ssh://@h", "ssh://h:0", "ssh://h:x", "ssh://-oProxyCommand=x", "ssh://a b", "ssh://h;calc"] {
            assert!(ssh(bad).unwrap().is_err(), "{bad}");
        }
    }

    fn remote(on: &str, command: &str, key: Option<&str>, host_key: Option<&str>, password: Option<&str>) -> Chore {
        Chore::Ssh {
            on: on.into(),
            command: command.into(),
            key: key.map(Into::into),
            host_key: host_key.map(Into::into),
            password: password.map(Into::into),
            max: None,
            pass: Pass::All,
        }
    }

    #[test]
    fn an_ssh_needs_its_host_command_and_host_key() {
        let ok = |on: &str, command: &str| remote(on, command, None, Some(HK), None).check();
        assert_eq!(ok("ssh://u@h", "df -h"), Ok(()));
        assert_eq!(ok("{param:pc}", "df -h"), Ok(()));
        assert_eq!(ok("{var:pc}", "df -h"), Ok(()));
        assert!(ok("", "df -h").unwrap_err().contains("on が空"));
        assert!(ok("u@h", "df -h").unwrap_err().contains("ssh://[ユーザー@]ホスト[:ポート] か group:組の名前 で書く"));
        assert!(ok("sftp://u@h/in", "df -h").unwrap_err().contains("ssh://"));
        assert!(ok("ssh://h/x", "df -h").unwrap_err().contains("パスは書かない"));
        assert!(ok("ssh://u@h", " \n").unwrap_err().contains("command が空"));
        assert_eq!(ok("ssh://u@h", &"x".repeat(COMMAND_MAX)), Ok(()));
        assert!(ok("ssh://u@h", &"x".repeat(COMMAND_MAX + 1)).unwrap_err().contains("8000 文字まで"));
        let login =
            |key: Option<&str>, host_key: Option<&str>, password: Option<&str>| remote("ssh://u@h", "x", key, host_key, password).check();
        assert!(login(None, None, None).unwrap_err().contains("ssh:// には host_key が要る"));
        assert!(login(None, Some("47DEQpj8"), None).unwrap_err().contains("SHA256:"));
        assert_eq!(login(None, Some(HK), Some("secret:ops")), Ok(()));
        assert!(login(None, Some(HK), Some("hunter2")).unwrap_err().contains("secret:<名前>"));
        assert!(login(Some("C:\\keys\\id"), Some(HK), Some("secret:ops")).unwrap_err().contains("どちらか一方"));
    }

    #[test]
    fn an_ssh_leaves_the_work_to_koyomi_and_keeps_the_command_in_the_environment() {
        let s = remote("ssh://ops@db1:2222 ", " backup.sh 'it''s' ; echo $HOME\n", Some("C:\\keys\\id"), Some(HK), None);
        assert_eq!(s.script(), (String::new(), None));
        let env = s.env();
        let get = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        assert_eq!(get("KOYOMI_SSH_ON"), Some("ssh://ops@db1:2222"));
        // The command as it is written, spaces and all.
        assert_eq!(get("KOYOMI_SSH_COMMAND"), Some(" backup.sh 'it''s' ; echo $HOME\n"));
        assert_eq!(get("KOYOMI_SSH_KEY"), Some("C:\\keys\\id"));
        assert_eq!(get("KOYOMI_SSH_HOST_KEY"), Some(HK));
        assert_eq!(get("KOYOMI_SSH_PASSWORD"), None);
        assert!(remote("ssh://h", "x", None, Some(HK), Some("secret:ops"))
            .env()
            .contains(&("KOYOMI_SSH_PASSWORD".to_owned(), "secret:ops".to_owned())));
    }

    #[test]
    fn an_ssh_or_winrm_on_a_group_says_how_many_at_once_and_when_it_passes() {
        assert_eq!(group(" group:web "), Some("web"));
        assert_eq!(group("GROUP: web"), Some("web"));
        assert_eq!(group("ssh://web"), None);
        assert_eq!(group("grou"), None);
        let fanned = |on: &str, host_key: Option<&str>, max: Option<u32>, pass: Pass| {
            let Chore::Ssh { on: o, command, key, password, .. } = remote(on, "uptime", None, host_key, None) else { unreachable!() };
            Chore::Ssh { on: o, command, key, host_key: host_key.map(Into::into), password, max, pass }
        };
        // Each PC's host key may be in [groups], so the job need not have one.
        assert_eq!(fanned("group:web", None, None, Pass::All).check(), Ok(()));
        assert_eq!(fanned("group:web", Some(HK), Some(4), Pass::Any).check(), Ok(()));
        assert!(fanned("group:web", Some("47DEQ"), None, Pass::All).check().unwrap_err().contains("SHA256:"));
        assert!(fanned("group:", None, None, Pass::All).check().unwrap_err().contains("組の名前を書く"));
        assert!(fanned("group:web-1", None, None, Pass::All).check().unwrap_err().contains("英数字と _ だけ"));
        assert!(fanned("group:web", None, Some(0), Pass::All).check().unwrap_err().contains("1 から 64 まで"));
        assert!(fanned("group:web", None, Some(65), Pass::All).check().unwrap_err().contains("1 から 64 まで"));
        // max and pass go with a group only; a param is told once it is filled in.
        assert!(fanned("ssh://h", Some(HK), Some(2), Pass::All).check().unwrap_err().contains("max は on が PC の組"));
        assert!(fanned("ssh://h", Some(HK), None, Pass::Any).check().unwrap_err().contains("pass は on が PC の組"));
        assert_eq!(fanned("{var:pcs}", Some(HK), Some(2), Pass::Any).check(), Ok(()));
        let env = fanned("group:web", None, Some(3), Pass::Any).env();
        assert!(env.contains(&("KOYOMI_SSH_ON".to_owned(), "group:web".to_owned())));
        assert!(env.contains(&("KOYOMI_SSH_MAX".to_owned(), "3".to_owned())));
        assert!(env.contains(&("KOYOMI_SSH_PASS".to_owned(), "any".to_owned())));
        let env = fanned("group:web", None, None, Pass::All).env();
        assert!(!env.iter().any(|(k, _)| k == "KOYOMI_SSH_MAX" || k == "KOYOMI_SSH_PASS"));
        // A winrm's PCs may each have their own certificate; a password is still needed.
        let w = |cert: Option<&str>, password: Option<&str>, max: Option<u32>, pass: Pass| {
            let Chore::Winrm { command, auth, .. } = windows("group:app", "ipconfig", cert, password) else { unreachable!() };
            Chore::Winrm {
                on: "group:app".into(),
                command,
                cert: cert.map(Into::into),
                password: password.map(Into::into),
                auth,
                max,
                pass,
            }
        };
        assert_eq!(w(None, Some("secret:ops"), Some(8), Pass::Any).check(), Ok(()));
        assert!(w(None, None, None, Pass::All).check().unwrap_err().contains("password が要る"));
        assert!(w(Some("3F:2A"), Some("secret:ops"), None, Pass::All).check().unwrap_err().contains("AB:CD:…"));
        let env = w(None, Some("secret:ops"), Some(8), Pass::Any).env();
        assert!(env.contains(&("KOYOMI_WINRM_MAX".to_owned(), "8".to_owned())));
        assert!(env.contains(&("KOYOMI_WINRM_PASS".to_owned(), "any".to_owned())));
        assert!(windows("winrm://ops@h", "x", Some(CERT), Some("secret:ops")).env().iter().all(|(k, _)| k != "KOYOMI_WINRM_MAX"));
        assert_eq!(Pass::ALL.map(|p| Pass::of(p.key())), [Some(Pass::All), Some(Pass::Any)]);
    }

    #[test]
    fn fill_keeps_an_ssh_judge_and_names_its_env_for_the_far_side() {
        let mut net = crate::jobnet::parse(&format!(
            "format = 1\n[[job]]\nid = \"r\"\nkind = \"ssh\"\non = \"ssh://ops@db1\"\ncommand = \"backup.sh\"\nhost_key = \"{HK}\"\njudge = {{ warn = \"1\" }}\nenv = {{ DAY = \"1\", WHO = \"me\" }}\n[[job]]\nid = \"c\"\nkind = \"copy\"\nfrom = 'D:\\out'\ninto = 'E:\\in'\nafter = [\"r\"]\n"
        ))
        .unwrap();
        fill(&mut net, "C:\\koyomi\\koyomi.exe").unwrap();
        let r = &net.jobs[0];
        // koyomi itself, not through PowerShell; the far command is not on the line.
        assert_eq!((r.shell.as_deref(), r.script.as_deref()), (None, None));
        assert_eq!(r.run, ["C:\\koyomi\\koyomi.exe", "ssh", "run"]);
        assert_eq!(r.judge.as_ref().unwrap().warn, vec![Range { from: 1, to: 1 }]);
        assert_eq!(r.env.get("KOYOMI_SSH_ENV").map(String::as_str), Some("DAY,WHO"));
        assert_eq!(r.env.get("KOYOMI_SSH_COMMAND").map(String::as_str), Some("backup.sh"));
        // A copy is still judged as robocopy is.
        assert!(net.jobs[1].judge.is_some());
        assert_eq!(net.jobs[1].shell.as_deref(), Some("powershell"));
        // Filled again, the far side is still asked for the job's own env only.
        fill(&mut net, "koyomi").unwrap();
        assert_eq!(net.jobs[0].env.get("KOYOMI_SSH_ENV").map(String::as_str), Some("DAY,WHO"));
    }

    const CERT: &str = "3F:2A:91:0C:5B:77:E4:18:A0:6D:C2:9F:41:B8:0E:D3:57:6A:FC:21:94:8B:3E:70:CD:12:65:A9:F0:4E:B7:83";

    #[test]
    fn a_winrm_address_has_a_user_and_a_host_and_no_path() {
        let a = winrm("winrm://ops@app01.example.com:5985").unwrap().unwrap();
        assert_eq!((a.user.as_deref(), a.host.as_str(), a.port), (Some("ops"), "app01.example.com", Some(5985)));
        let b = winrm(" WinRM://CORP\\ops@app01/ ").unwrap().unwrap();
        assert_eq!((b.user.as_deref(), b.host.as_str(), b.port), (Some("CORP\\ops"), "app01", None));
        assert_eq!(winrm("winrm://.\\ops@10.0.0.5").unwrap().unwrap().user.as_deref(), Some(".\\ops"));
        assert_eq!(winrm("winrm://{param:who}@h").unwrap().unwrap().user.as_deref(), Some("{param:who}"));
        assert!(winrm("ssh://u@h").is_none());
        assert!(winrm("h").is_none());
        assert!(winrm("winrm://h").unwrap().unwrap_err().contains("ユーザーが要る"));
        assert!(winrm("winrm://u@h/wsman").unwrap().unwrap_err().contains("パスは書かない"));
        for bad in ["winrm://", "winrm://@h", "winrm://u@h:0", "winrm://u@h:x", "winrm://u@-h", "winrm://u@a b", "winrm://u@h&calc"] {
            assert!(winrm(bad).unwrap().is_err(), "{bad}");
        }
    }

    fn windows(on: &str, command: &str, cert: Option<&str>, password: Option<&str>) -> Chore {
        Chore::Winrm {
            on: on.into(),
            command: command.into(),
            cert: cert.map(Into::into),
            password: password.map(Into::into),
            auth: WinrmAuth::Basic,
            max: None,
            pass: Pass::All,
        }
    }

    #[test]
    fn a_winrm_needs_its_user_host_command_certificate_and_password() {
        let ok = |on: &str, command: &str| windows(on, command, Some(CERT), Some("secret:ops")).check();
        assert_eq!(ok("winrm://ops@h", "ipconfig /all"), Ok(()));
        assert_eq!(ok("{param:pc}", "ipconfig"), Ok(()));
        assert!(ok("", "ipconfig").unwrap_err().contains("on が空"));
        assert!(ok("ops@h", "ipconfig").unwrap_err().contains("winrm://ユーザー@ホスト[:ポート] か group:組の名前 で書く"));
        assert!(ok("ssh://ops@h", "ipconfig").unwrap_err().contains("winrm://"));
        assert!(ok("winrm://h", "ipconfig").unwrap_err().contains("ユーザーが要る"));
        assert!(ok("winrm://ops@h", " \r\n").unwrap_err().contains("command が空"));
        assert!(ok("winrm://ops@h", &"x".repeat(COMMAND_MAX + 1)).unwrap_err().contains("8000 文字まで"));
        let login = |cert: Option<&str>, password: Option<&str>| windows("winrm://ops@h", "x", cert, password).check();
        assert!(login(None, Some("secret:ops")).unwrap_err().contains("koyomi winrm cert"));
        assert!(login(Some("3F:2A"), Some("secret:ops")).unwrap_err().contains("AB:CD:…"));
        assert!(login(Some(&"zz".repeat(32)), Some("secret:ops")).unwrap_err().contains("AB:CD:…"));
        // Colons, spaces and case as one likes, or a param.
        assert_eq!(login(Some(&CERT.replace(':', "").to_lowercase()), Some("secret:ops")), Ok(()));
        assert_eq!(login(Some(&CERT.replace(':', " ")), Some("secret:ops")), Ok(()));
        assert_eq!(login(Some("{param:cert}"), Some("secret:ops")), Ok(()));
        assert!(login(Some(CERT), None).unwrap_err().contains("password が要る"));
        assert!(login(Some(CERT), Some(" ")).unwrap_err().contains("password が要る"));
        assert!(login(Some(CERT), Some("hunter2")).unwrap_err().contains("secret:<名前>"));
        assert!(login(Some(CERT), Some("secret: ")).unwrap_err().contains("secret:<名前>"));
    }

    #[test]
    fn a_winrm_leaves_the_work_to_koyomi_and_keeps_its_judge_and_env() {
        let w = windows(" winrm://ops@app01 ", "  dir \"C:\\a b\" & exit /b 3\r\n", Some(CERT), Some(" secret:ops "));
        assert_eq!(w.script(), (String::new(), None));
        assert!(w.far() && !Chore::blank("http").unwrap().far());
        let env = w.env();
        let get = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        assert_eq!(get("KOYOMI_WINRM_ON"), Some("winrm://ops@app01"));
        // The command as it is written, spaces and all.
        assert_eq!(get("KOYOMI_WINRM_COMMAND"), Some("  dir \"C:\\a b\" & exit /b 3\r\n"));
        assert_eq!(get("KOYOMI_WINRM_CERT"), Some(CERT));
        assert_eq!(get("KOYOMI_WINRM_PASSWORD"), Some("secret:ops"));
        assert_eq!(get("KOYOMI_WINRM_AUTH"), Some("basic"));
        let Chore::Winrm { on, command, cert, password, max, pass, .. } = w.clone() else { unreachable!() };
        let ntlm = Chore::Winrm { on, command, cert, password, auth: WinrmAuth::Ntlm, max, pass };
        assert!(ntlm.env().contains(&("KOYOMI_WINRM_AUTH".to_owned(), "ntlm".to_owned())));
        assert_eq!(WinrmAuth::ALL.map(|a| WinrmAuth::of(a.key())), [Some(WinrmAuth::Basic), Some(WinrmAuth::Ntlm)]);
        assert_eq!(WinrmAuth::of("NTLM"), None);
        let mut net = crate::jobnet::parse(&format!(
            "format = 1\n[[job]]\nid = \"w\"\nkind = \"winrm\"\non = 'winrm://ops@app01'\ncommand = 'backup.cmd'\ncert = \"{CERT}\"\npassword = \"secret:ops\"\njudge = {{ warn = \"1\" }}\nenv = {{ DAY = \"1\" }}\n"
        ))
        .unwrap();
        fill(&mut net, "C:\\koyomi\\koyomi.exe").unwrap();
        let r = &net.jobs[0];
        assert_eq!((r.shell.as_deref(), r.script.as_deref()), (None, None));
        assert_eq!(r.run, ["C:\\koyomi\\koyomi.exe", "winrm", "run"]);
        assert_eq!(r.judge.as_ref().unwrap().warn, vec![Range { from: 1, to: 1 }]);
        assert_eq!(r.env.get("KOYOMI_WINRM_ENV").map(String::as_str), Some("DAY"));
        fill(&mut net, "koyomi").unwrap();
        assert_eq!(net.jobs[0].env.get("KOYOMI_WINRM_ENV").map(String::as_str), Some("DAY"));
    }

    fn http(method: &str, body: Option<&str>, headers: &[(&str, &str)], expect: &[u16]) -> Chore {
        Chore::Http {
            url: "https://api.example.com/jobs/{date}".into(),
            method: method.into(),
            body: body.map(Into::into),
            content_type: None,
            headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            expect: expect.to_vec(),
        }
    }

    #[test]
    fn an_http_script_reads_its_headers_from_the_environment_and_judges_by_the_code() {
        let h = http("POST", Some("{\"say\": \"it's\u{2019}\"}\n"), &[("X-Day", "1"), ("Authorization", "secret:api")], &[200, 202]);
        assert_eq!(h.check(), Ok(()));
        let (s, judge) = h.script();
        assert!(judge.is_none());
        assert!(s.contains("Uri = 'https://api.example.com/jobs/{date}'; Method = 'POST'"), "{s}");
        // In the order of the names, as env() gives the values.
        assert!(s.contains("$p.Headers['Authorization'] = $env:KOYOMI_HEADER_0\n$p.Headers['X-Day'] = $env:KOYOMI_HEADER_1"), "{s}");
        assert!(!s.contains("secret:api"), "{s}");
        assert_eq!(h.env(), vec![("KOYOMI_HEADER_0".to_owned(), "secret:api".to_owned()), ("KOYOMI_HEADER_1".to_owned(), "1".to_owned())]);
        // The body as it is, its quotes doubled, its last line kept.
        assert!(s.contains("UTF8.GetBytes('{\"say\": \"it''s\u{2019}\u{2019}\"}\n')"), "{s}");
        assert!(s.contains("$p.ContentType = 'application/json; charset=utf-8'"), "{s}");
        assert!(s.contains("if (@(200,202) -contains $code) { exit 0 }"), "{s}");
        assert!(s.contains("exit 2"), "{s}");
        let (s, _) = http("GET", None, &[], &[]).script();
        assert!(s.contains("if ($code -ge 200 -and $code -le 299) { exit 0 }"), "{s}");
        assert!(!s.contains("$p.Body"), "{s}");
        assert!(http("GET", None, &[], &[]).env().is_empty());
    }

    #[test]
    fn an_http_that_cannot_be_sent_as_written_is_refused() {
        let url = |u: &str| {
            let mut h = http("GET", None, &[], &[]);
            if let Chore::Http { url, .. } = &mut h {
                *url = u.into();
            }
            h.check()
        };
        assert!(url("").is_err());
        assert!(url("ftp://x").unwrap_err().contains("http:// か https://"));
        assert_eq!(url("HTTP://x"), Ok(()));
        assert_eq!(url("{param:api}/jobs"), Ok(()));
        assert_eq!(url("{var:api}/jobs"), Ok(()));
        assert!(http("get", None, &[], &[]).check().unwrap_err().contains("method は"));
        assert!(http("HEAD", Some("a"), &[], &[]).check().unwrap_err().contains("body を書けない"));
        assert!(http("POST", Some(&"a".repeat(BODY_MAX + 1)), &[], &[]).check().unwrap_err().contains("文字まで"));
        assert_eq!(http("POST", Some(&"あ".repeat(BODY_MAX)), &[], &[]).check(), Ok(()));
        assert!(http("GET", None, &[("content-type", "a")], &[]).check().unwrap_err().contains("content_type に書く"));
        assert!(http("GET", None, &[("X Day", "a")], &[]).check().unwrap_err().contains("名前に使えない"));
        assert!(http("GET", None, &[], &[600]).check().unwrap_err().contains("100〜599"));
        let mut typed = http("GET", None, &[], &[]);
        if let Chore::Http { content_type, .. } = &mut typed {
            *content_type = Some("text/plain".into());
        }
        assert!(typed.check().unwrap_err().contains("body を書いたときだけ"));
    }

    fn mail(to: &[&str], subject: &str, body: Option<&str>, files: &[&str]) -> Chore {
        Chore::Mail {
            to: to.iter().map(|t| t.to_string()).collect(),
            cc: Vec::new(),
            subject: subject.into(),
            body: body.map(Into::into),
            files: files.iter().map(|f| f.to_string()).collect(),
            via: None,
        }
    }

    #[test]
    fn a_mail_address_is_one_person_as_ito_reads_it() {
        for good in ["ops@example.com", " 経理 <keiri@example.co.jp> ", "\"Ops Team\" <ops@x>", "{param:who}"] {
            assert_eq!(mail_address(good), Ok(()), "{good}");
        }
        for bad in ["", "ops", "@x", "a@", "a@b@c", "a b@x", "a@x, b@x", "a@x;b@x", "名前 <a@x", "a@x>", "a@x\nBcc: c@x", "<a b@x>"] {
            assert!(mail_address(bad).unwrap_err().contains("アドレスとして読めない"), "{bad}");
        }
    }

    #[test]
    fn a_mail_that_cannot_be_sent_as_written_is_refused() {
        assert_eq!(mail(&["ops@x"], "報告 {date}", Some("本文"), &["D:\\out\\*.csv"]).check(), Ok(()));
        assert!(mail(&[], "s", None, &[]).check().unwrap_err().contains("to が空"));
        assert!(mail(&["ops"], "s", None, &[]).check().unwrap_err().contains("アドレス"));
        assert!(mail(&["ops@x"], " ", None, &[]).check().unwrap_err().contains("subject が空"));
        assert!(mail(&["ops@x"], "a\r\nBcc: c@x", None, &[]).check().unwrap_err().contains("改行"));
        assert!(mail(&["ops@x"], "s", Some(&"a".repeat(BODY_MAX + 1)), &[]).check().unwrap_err().contains("文字まで"));
        assert!(mail(&["ops@x"], "s", None, &["a"; FILES_MAX + 1]).check().unwrap_err().contains("個まで"));
        assert!(mail(&["ops@x"], "s", None, &["a", " "]).check().unwrap_err().contains("空のもの"));
        let mut cc = mail(&["ops@x"], "s", None, &[]);
        if let Chore::Mail { cc, .. } = &mut cc {
            cc.push("bad".into());
        }
        assert!(cc.check().unwrap_err().contains("bad"));
    }

    #[test]
    fn a_mail_leaves_the_work_to_koyomi_and_marks_its_texts() {
        let mut m = mail(&[" ops@x ", "名前 <b@x>"], " secret:x ", Some("本文\n2 行目"), &["D:\\a.csv", "D:\\b\\*.log"]);
        if let Chore::Mail { cc, via, .. } = &mut m {
            *cc = vec!["c@x".into()];
            *via = Some(" ops ".into());
        }
        assert_eq!(m.script(), (String::new(), None));
        assert!(m.own() && !m.far());
        let env = m.env();
        let get = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        assert_eq!(get("KOYOMI_MAIL_TO"), Some("|ops@x\n名前 <b@x>"));
        assert_eq!(get("KOYOMI_MAIL_CC"), Some("|c@x"));
        // Never read as a secret: it starts with `|`.
        assert_eq!(get("KOYOMI_MAIL_SUBJECT"), Some("|secret:x"));
        assert_eq!(get("KOYOMI_MAIL_BODY"), Some("|本文\n2 行目"));
        assert_eq!(get("KOYOMI_MAIL_FILES"), Some("|D:\\a.csv\nD:\\b\\*.log"));
        assert_eq!(get("KOYOMI_MAIL_VIA"), Some("|ops"));
        let bare = mail(&["ops@x"], "s", None, &[]).env();
        assert_eq!(bare.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["KOYOMI_MAIL_TO", "KOYOMI_MAIL_SUBJECT"]);
        // Every text is filled in: the subject, each address, the body, each file.
        assert_eq!(m.paths().len(), 1 + 2 + 1 + 1 + 2);
    }

    #[test]
    fn fill_runs_a_mail_as_koyomi_without_a_script() {
        let mut net = crate::jobnet::parse(
            "format = 1\n[[job]]\nid = \"m\"\nkind = \"mail\"\nto = [\"ops@x\"]\nsubject = \"done\"\nfiles = ['D:\\out\\a.csv']\n",
        )
        .unwrap();
        fill(&mut net, "C:\\koyomi\\koyomi.exe").unwrap();
        let m = &net.jobs[0];
        assert_eq!((m.shell.as_deref(), m.script.as_deref(), m.judge.as_ref()), (None, None, None));
        assert_eq!(m.run, ["C:\\koyomi\\koyomi.exe", "mail", "run"]);
        assert_eq!(m.env.get("KOYOMI_MAIL_FILES").map(String::as_str), Some("|D:\\out\\a.csv"));
        assert!(!m.env.contains_key("KOYOMI_MAIL_ENV"));
    }

    fn query(db: &str, password: Option<&str>, sql: Option<&str>, file: Option<&str>) -> Chore {
        Chore::Sql {
            db: db.into(),
            password: password.map(Into::into),
            sql: sql.map(Into::into),
            file: file.map(Into::into),
            out: None,
            rows: None,
            tls: SqlTls::Verify,
        }
    }

    #[test]
    fn a_sql_db_reads_as_sql_server_postgres_or_sqlite() {
        let server = |user: Option<&str>, host: &str, port: u16, database: &str| SqlDb::SqlServer {
            user: user.map(Into::into),
            host: host.into(),
            port,
            database: database.into(),
        };
        let pg = |user: &str, host: &str, port: u16, database: &str| SqlDb::Postgres {
            user: user.into(),
            host: host.into(),
            port,
            database: database.into(),
        };
        assert_eq!(sql_db(" sqlserver://db01/売上 "), Ok(server(None, "db01", 1433, "売上")));
        assert_eq!(sql_db("SQLSERVER://ops@db01:14330/sales"), Ok(server(Some("ops"), "db01", 14330, "sales")));
        assert_eq!(sql_db("postgresql://app@10.0.0.5/app"), Ok(pg("app", "10.0.0.5", 5432, "app")));
        assert_eq!(sql_db("postgres://app@pg:6543/app"), Ok(pg("app", "pg", 6543, "app")));
        assert_eq!(sql_db("sqlite://C:\\data\\app.db"), Ok(SqlDb::Sqlite { path: "C:\\data\\app.db".into() }));
        assert_eq!(sql_db("sqlite:///var/lib/app.db"), Ok(SqlDb::Sqlite { path: "/var/lib/app.db".into() }));
        let err = |s: &str| sql_db(s).unwrap_err();
        assert!(err("mysql://db01/x").contains("どれか"));
        assert!(err("sqlserver://db01").contains("データベースの名前が要る"));
        assert!(err("sqlserver://db01/ ").contains("データベースの名前が要る"));
        assert!(err("sqlserver://db01/a/b").contains("使えない文字"));
        assert!(err("postgres://app@pg/app?sslmode=require").contains("使えない文字"));
        assert!(err("sqlserver://db01\\SQLEXPRESS/x").contains("名前付きインスタンス"));
        assert!(err("sqlserver://CORP\\ops@db01/x").contains("ユーザーを書かない"));
        assert!(err("sqlserver://db01:0/x").contains("ポート"));
        assert!(err("postgres://pg/app").contains("ユーザーが要る"));
        assert!(err("sqlite://app.db").contains("全部書く"));
    }

    #[test]
    fn sql_rows_are_ranges_of_counts() {
        assert_eq!(sql_rows("0"), Ok(vec![(0, 0)]));
        assert_eq!(sql_rows(" 1.. "), Ok(vec![(1, u64::MAX)]));
        assert_eq!(sql_rows("1..100, ..0"), Ok(vec![(1, 100), (0, 0)]));
        for bad in ["", "x", "-1", "5..1", "1..=5", "1,,2"] {
            assert!(sql_rows(bad).unwrap_err().contains("rows は"), "{bad}");
        }
    }

    #[test]
    fn a_sql_that_cannot_run_as_written_is_refused() {
        let ok = |c: Chore| c.check().unwrap_or_else(|e| panic!("{e}"));
        let err = |c: Chore| c.check().unwrap_err();
        ok(query("sqlserver://db01/x", None, Some("select 1"), None));
        ok(query("sqlserver://ops@db01/x", Some("secret:db"), None, Some("D:\\sql\\a.sql")));
        ok(query("postgres://app@pg/app", Some("secret:pg"), Some("select 1"), None));
        ok(query("sqlite://C:\\data\\app.db", None, Some("select 1"), None));
        ok(query("{param:db}", Some("secret:db"), Some("select 1"), None));
        assert!(err(query(" ", None, Some("select 1"), None)).contains("db が空"));
        assert!(err(query("mysql://db01/x", None, Some("select 1"), None)).contains("どれか"));
        assert!(err(query("sqlserver://db01/x", Some("secret:db"), Some("select 1"), None)).contains("password は書かない"));
        assert!(err(query("sqlserver://ops@db01/x", None, Some("select 1"), None)).contains("password が要る"));
        assert!(err(query("postgres://app@pg/app", None, Some("select 1"), None)).contains("password が要る"));
        assert!(err(query("postgres://app@pg/app", Some("hunter2"), Some("select 1"), None)).contains("secret:<名前>"));
        assert!(err(query("postgres://app@pg/app", Some("secret: "), Some("select 1"), None)).contains("secret:<名前>"));
        assert!(err(query("sqlite://C:\\a.db", Some("secret:x"), Some("select 1"), None)).contains("password を書かない"));
        assert!(err(query("sqlite://C:\\a.db", None, None, None)).contains("sql か file"));
        assert!(err(query("sqlite://C:\\a.db", None, Some("select 1"), Some("D:\\a.sql"))).contains("どちらか一方"));
        assert!(err(query("sqlite://C:\\a.db", None, Some(" \n"), None)).contains("sql が空"));
        assert!(err(query("sqlite://C:\\a.db", None, None, Some(" "))).contains("file が空"));
        assert!(err(query("sqlite://C:\\a.db", None, Some(&"x".repeat(COMMAND_MAX + 1)), None)).contains("文字まで"));
        let with = |db: &str, password: Option<&str>, tls: SqlTls, out: Option<&str>, rows: Option<&str>| {
            let mut c = query(db, password, Some("select 1"), None);
            if let Chore::Sql { tls: t, out: o, rows: r, .. } = &mut c {
                (*t, *o, *r) = (tls, out.map(Into::into), rows.map(Into::into));
            }
            c
        };
        assert!(err(with("sqlserver://db01/x", None, SqlTls::Off, None, None)).contains("tls = \"off\" は使えない"));
        ok(with("sqlserver://db01/x", None, SqlTls::Trust, None, None));
        assert!(err(with("sqlite://C:\\a.db", None, SqlTls::Trust, None, None)).contains("tls を書かない"));
        ok(with("postgres://app@pg/app", Some("secret:pg"), SqlTls::Off, Some("D:\\out\\a.csv"), Some("1..")));
        assert!(err(with("sqlite://C:\\a.db", None, SqlTls::Verify, None, Some("x"))).contains("rows は"));
        assert!(err(with("sqlite://C:\\a.db", None, SqlTls::Verify, Some(" "), None)).contains("out が空"));
    }

    #[test]
    fn a_sql_leaves_the_work_to_koyomi_and_marks_its_texts() {
        let mut c = query(" sqlserver://ops@db01/x ", Some(" secret:db "), Some("select 1\n"), None);
        if let Chore::Sql { out, rows, tls, .. } = &mut c {
            (*out, *rows, *tls) = (Some(" D:\\out\\a.csv ".into()), Some("1..".into()), SqlTls::Trust);
        }
        assert_eq!(c.script(), (String::new(), None));
        assert!(c.own() && !c.far());
        let env = c.env();
        let get = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        assert_eq!(get("KOYOMI_SQL_DB"), Some("|sqlserver://ops@db01/x"));
        assert_eq!(get("KOYOMI_SQL_TEXT"), Some("|select 1\n"));
        assert_eq!(get("KOYOMI_SQL_OUT"), Some("|D:\\out\\a.csv"));
        assert_eq!(get("KOYOMI_SQL_ROWS"), Some("|1.."));
        assert_eq!(get("KOYOMI_SQL_TLS"), Some("trust"));
        // Read as a secret when the job starts.
        assert_eq!(get("KOYOMI_SQL_PASSWORD"), Some("secret:db"));
        assert_eq!(get("KOYOMI_SQL_FILE"), None);
        let bare = query("sqlite://C:\\a.db", None, None, Some("D:\\a.sql")).env();
        assert_eq!(bare.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["KOYOMI_SQL_DB", "KOYOMI_SQL_TLS", "KOYOMI_SQL_FILE"]);
        // Filled in: the database, the SQL, the out file.
        assert_eq!(c.paths().len(), 3);
    }

    #[test]
    fn fill_runs_a_sql_as_koyomi_without_a_script() {
        let mut net = crate::jobnet::parse(
            "format = 1\n[[job]]\nid = \"q\"\nkind = \"sql\"\ndb = 'sqlite://C:\\data\\app.db'\nsql = \"delete from log\"\n",
        )
        .unwrap();
        fill(&mut net, "C:\\koyomi\\koyomi.exe").unwrap();
        let q = &net.jobs[0];
        assert_eq!((q.shell.as_deref(), q.script.as_deref(), q.judge.as_ref()), (None, None, None));
        assert_eq!(q.run, ["C:\\koyomi\\koyomi.exe", "sql", "run"]);
        assert_eq!(q.env.get("KOYOMI_SQL_TEXT").map(String::as_str), Some("|delete from log"));
        assert!(!q.env.contains_key("KOYOMI_SQL_ENV"));
    }
}
