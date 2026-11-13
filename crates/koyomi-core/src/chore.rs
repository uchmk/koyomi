//! The everyday file chores a net can have as parts without a script:
//! delete old files, zip, copy a folder, empty a folder, send and fetch
//! files over SFTP, to and from S3 or a shared folder, call a web API, run a
//! command on another PC over SSH or on a Windows PC over WinRM, send a mail,
//! run SQL on a database.
//! Each is written in the file as its own `kind` and runs as a job of the net, so it
//! has the job's slot, log, `timeout` and `retry`. A `zip` and a `send` or `fetch`
//! with a shared folder run as a PowerShell script the engine writes when the run
//! starts; over SFTP the script runs `koyomi sftp run`. An `ssh` runs `koyomi ssh
//! run` itself, without PowerShell, a `winrm` `koyomi winrm run`, a `mail` `koyomi
//! mail run`, a `sql` `koyomi sql run`, a `send` or `fetch` with S3 `koyomi cloud
//! run`, and a `delete`, `empty` or `copy` `koyomi file run`. Each reads what to do
//! from the environment.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::jobnet::{Jobnet, Judge, Part, Range};
use crate::msg;

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
    /// フォルダーを写す: the folder `from` and the folders below it into `into`; only
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
    /// `into`, either `sftp://[user@]host[:port]/folder`, `s3://bucket/folder/` or a
    /// folder or share.
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
        /// `secret:<name>`: log in with the password kept under that name; for `s3://`,
        /// the secret access key.
        #[serde(default)]
        password: Option<String>,
        /// For `s3://`: the access key ID, as it is or `secret:<name>`.
        #[serde(default)]
        key_id: Option<String>,
        /// For `s3://`: the bucket's region (`ap-northeast-1`); with `endpoint`,
        /// `us-east-1` when it is not written.
        #[serde(default)]
        region: Option<String>,
        /// For `s3://` on an S3-compatible store (MinIO, a NAS …): `http(s)://host[:port]`;
        /// the bucket then goes in the path.
        #[serde(default)]
        endpoint: Option<String>,
        /// A `[[host]]` of koyomi.toml (an ssh one): `into` is then only the path on it, and the
        /// engine joins them when the job starts. The login is the entry's.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<String>,
        /// A `[[bucket]]` of koyomi.toml: `into` is then only the path below its folder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bucket: Option<String>,
        /// Make sure each file arrived as it left, before it gets its name; none: as
        /// before, not checked.
        #[serde(default)]
        verify: Option<Verify>,
    },
    /// ファイルを受け取る: the files `from`, either `sftp://[user@]host[:port]/folder/*.csv`,
    /// `s3://bucket/folder/*.csv` or a folder or share (a wildcard allowed), into the
    /// folder `into` here.
    Fetch {
        from: String,
        into: String,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        host_key: Option<String>,
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        key_id: Option<String>,
        #[serde(default)]
        region: Option<String>,
        #[serde(default)]
        endpoint: Option<String>,
        /// A `[[host]]` of koyomi.toml: `from` is then only the path on it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<String>,
        /// A `[[bucket]]` of koyomi.toml: `from` is then only the path below its folder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bucket: Option<String>,
        #[serde(default)]
        verify: Option<Verify>,
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
    /// with 0 or one is enough. `on` may also be the name of a `[[host]]` of
    /// koyomi.toml ([`host_name`]), which owns the login.
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
    /// (`koyomi sql run`). Instead of `db` (with its `password` and `tls`), `database`
    /// names an `[[database]]` of koyomi.toml, which the engine reads when the job starts.
    Sql {
        #[serde(default)]
        db: String,
        #[serde(default)]
        database: Option<String>,
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

/// How a `send` or `fetch` makes sure what arrived is what left (`verify`). Either
/// way the file is checked under its `.part` name, before it gets its own, so a file
/// seen under its own name is whole. Over SFTP `sha256` hashes it on the far side
/// (`exec`) and `readback` reads it back; S3 checks either with the store's SHA-256
/// checksum, and a folder or share by hashing both files here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verify {
    /// Hashed where it landed: over SFTP with `exec` (`sha256sum`, or `Get-FileHash`
    /// on Windows), which a transfer-only account refuses.
    Sha256,
    /// Read back and hashed here: any SFTP server, twice the traffic.
    Readback,
}

impl Verify {
    /// Each, in the order of the editor's list.
    pub const ALL: [Verify; 2] = [Verify::Sha256, Verify::Readback];

    /// As written in `verify`.
    pub fn key(self) -> &'static str {
        match self {
            Verify::Sha256 => "sha256",
            Verify::Readback => "readback",
        }
    }

    /// The one written as `key`.
    pub fn of(key: &str) -> Option<Verify> {
        Verify::ALL.into_iter().find(|v| v.key() == key)
    }
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
        Some("") => return Err(msg::line(msg::CHORE_GROUP_NAME_EMPTY, &[])),
        Some(name) if crate::jobnet::plain_name(name) => true,
        Some(name) => return Err(msg::line(msg::CHORE_GROUP_NAME_BAD, &[name])),
        None => false,
    };
    if max.is_some_and(|m| !(1..=GROUP_MAX as u32).contains(&m)) {
        return Err(msg::line(msg::CHORE_MAX_RANGE, &[&GROUP_MAX.to_string()]));
    }
    if !group && !on.trim().starts_with('{') {
        if max.is_some() {
            return Err(msg::line(msg::CHORE_GROUP_ONLY, &["max"]));
        }
        if pass != Pass::All {
            return Err(msg::line(msg::CHORE_GROUP_ONLY, &["pass"]));
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
///
/// `koyomi cloud run` ends with it too.
pub const NOTHING: i32 = 3;

/// The exit code of an `ssh` or `winrm` that got no exit code from the far side
/// (no connection, no login, a signal, a lost connection); why is in the file
/// `KOYOMI_REASON` names.
pub const NO_EXIT: i32 = 255;

/// The longest `command` of an `ssh` or `winrm`, and `sql` of a `sql` (characters):
/// it reaches `koyomi ssh run`, `koyomi winrm run` or `koyomi sql run` in an
/// environment variable.
pub const COMMAND_MAX: usize = 8000;

/// The port of `ssh://` when none is written.
pub const SSH_PORT: u16 = 22;

/// The port of `winrm://` when none is written: WinRM over HTTPS.
pub const WINRM_PORT: u16 = 5986;

/// The most `files` a `mail` names, and the most files they may find.
pub const FILES_MAX: usize = 20;

/// Put before each text a `mail`, `sql`, `delete`, `empty` or `copy` gives
/// `koyomi mail run`, `koyomi sql run` or `koyomi file run` in the environment, so a
/// text that starts with `secret:` is not read as a secret.
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

    /// Whether it is a `send` into S3 or a `fetch` from it (`s3://`, or a `bucket` of
    /// koyomi.toml), which koyomi runs itself as `koyomi cloud run`.
    pub fn cloud(&self) -> bool {
        match self {
            Chore::Send { into, bucket, .. } => bucket.is_some() || s3(into).is_some(),
            Chore::Fetch { from, bucket, .. } => bucket.is_some() || s3(from).is_some(),
            _ => false,
        }
    }

    /// Whether it is a `delete`, an `empty` or a `copy`, which koyomi runs itself as
    /// `koyomi file run` (ito-fileops), without a shell.
    pub fn file(&self) -> bool {
        matches!(self, Chore::Delete { .. } | Chore::Empty { .. } | Chore::Copy { .. })
    }

    /// Whether koyomi runs it itself as `koyomi <op> run`, not as a script: an
    /// `ssh`, a `winrm`, a `mail`, a `sql`, a `delete`, an `empty`, a `copy`, and a
    /// `send` or `fetch` with S3.
    pub fn own(&self) -> bool {
        self.far() || self.cloud() || self.file() || matches!(self, Chore::Mail { .. } | Chore::Sql { .. })
    }

    /// The `<op>` of `koyomi <op> run` when koyomi runs it itself ([`Chore::own`]):
    /// its `kind`, `cloud` for S3, or `file` for a `delete`, `empty` or `copy`.
    pub fn op(&self) -> &'static str {
        if self.cloud() {
            "cloud"
        } else if self.file() {
            "file"
        } else {
            self.key()
        }
    }

    /// A new, empty one of `kind` (for the shelf).
    pub fn blank(kind: &str) -> Option<Chore> {
        Some(match kind {
            "delete" => Chore::Delete { path: String::new(), pattern: None, keep_days: 30 },
            "zip" => Chore::Zip { from: String::new(), into: String::new() },
            "copy" => Chore::Copy { from: String::new(), into: String::new(), pattern: None },
            "empty" => Chore::Empty { path: String::new() },
            "send" => Chore::Send {
                from: String::new(),
                into: String::new(),
                key: None,
                host_key: None,
                password: None,
                key_id: None,
                region: None,
                endpoint: None,
                host: None,
                bucket: None,
                verify: None,
            },
            "fetch" => Chore::Fetch {
                from: String::new(),
                into: String::new(),
                key: None,
                host_key: None,
                password: None,
                key_id: None,
                region: None,
                endpoint: None,
                host: None,
                bucket: None,
                verify: None,
            },
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
            "sql" => Chore::Sql {
                db: String::new(),
                database: None,
                password: None,
                sql: None,
                file: None,
                out: None,
                rows: None,
                tls: SqlTls::Verify,
            },
            _ => return None,
        })
    }

    /// Every text of it that is a template, to fill in `{date}`, `{param:…}` and `{var:…}`. (The
    /// name of a `[[host]]`, `[[bucket]]` or `[[database]]` is not one, and is not here; the far
    /// path of a `send` or `fetch` with a name is, and stays what is written until the job starts.)
    pub fn paths_mut(&mut self) -> Vec<(&'static str, &mut String)> {
        match self {
            Chore::Delete { path, .. } | Chore::Empty { path } => vec![("path", path)],
            Chore::Zip { from, into } | Chore::Copy { from, into, .. } => vec![("from", from), ("into", into)],
            Chore::Send { from, into, key, host_key, region, endpoint, .. }
            | Chore::Fetch { from, into, key, host_key, region, endpoint, .. } => {
                let mut v = vec![("from", from), ("into", into)];
                if let Some(k) = key {
                    v.push(("key", k));
                }
                if let Some(h) = host_key {
                    v.push(("host_key", h));
                }
                v.extend(region.iter_mut().map(|r| ("region", r)));
                v.extend(endpoint.iter_mut().map(|e| ("endpoint", e)));
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
            Chore::Sql { db, database, sql, file, out, .. } => {
                // A `database` is a name, not a template: only the address written here is filled in.
                let mut v = if database.is_some() { Vec::new() } else { vec![("db", db)] };
                v.extend(sql.iter_mut().map(|s| ("sql", s)));
                v.extend(file.iter_mut().map(|f| ("file", f)));
                v.extend(out.iter_mut().map(|o| ("out", o)));
                v
            }
        }
    }

    /// Every template text of it, as it is: what [`Chore::paths_mut`] fills in, and what the
    /// parser and the window check. None of them is looked up as a place on this PC.
    pub fn paths(&self) -> Vec<&String> {
        match self {
            Chore::Delete { path, .. } | Chore::Empty { path } => vec![path],
            Chore::Zip { from, into } | Chore::Copy { from, into, .. } => vec![from, into],
            Chore::Send { from, into, key, host_key, region, endpoint, .. }
            | Chore::Fetch { from, into, key, host_key, region, endpoint, .. } => {
                [from, into].into_iter().chain(key).chain(host_key).chain(region).chain(endpoint).collect()
            }
            Chore::Http { url, body, .. } => [url].into_iter().chain(body).collect(),
            Chore::Ssh { on, command, key, host_key, .. } => [on, command].into_iter().chain(key).chain(host_key).collect(),
            Chore::Winrm { on, command, cert, .. } => [on, command].into_iter().chain(cert).collect(),
            Chore::Mail { to, cc, subject, body, files, .. } => {
                [subject].into_iter().chain(to).chain(cc).chain(body).chain(files).collect()
            }
            Chore::Sql { db, database, sql, file, out, .. } => {
                [db].into_iter().filter(|_| database.is_none()).chain(sql).chain(file).chain(out).collect()
            }
        }
    }

    /// Why it may not run as it stands: a path that is empty, a drive's root
    /// for `delete` and `empty`, a zip that does not end in `.zip`, an `sftp://`
    /// that cannot be read or is on the wrong side, an `ssh` without its host,
    /// command or host key, a `winrm` without its user, host, command, certificate
    /// or password, a `mail` without a good address or a subject, a `sql` without
    /// a database it can read, a login that fits it, or its SQL.
    pub fn check(&self) -> Result<(), String> {
        let need = |name: &str, p: &str| if p.trim().is_empty() { Err(msg::line(msg::CHORE_KEY_EMPTY, &[name])) } else { Ok(()) };
        match self {
            Chore::Delete { path, keep_days, .. } => {
                need("path", path)?;
                not_root(path)?;
                if *keep_days == 0 {
                    return Err(msg::line(msg::CHORE_KEEP_DAYS, &[]));
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
                    return Err(msg::line(msg::CHORE_ZIP_INTO, &[&shown_address(into)]));
                }
            }
            Chore::Copy { from, into, .. } => {
                need("from", from)?;
                need("into", into)?;
            }
            Chore::Send { from, into, key, host_key, password, key_id, region, endpoint, host, bucket, .. } => {
                need("from", from)?;
                need("into", into)?;
                if sftp(from).is_some() || s3(from).is_some() {
                    return Err(msg::line(msg::CHORE_LOCAL_ONLY, &["send", "from", "into"]));
                }
                let own = [
                    ("key", key),
                    ("host_key", host_key),
                    ("password", password),
                    ("key_id", key_id),
                    ("region", region),
                    ("endpoint", endpoint),
                ];
                if named_far(true, host, bucket, into, &own)? {
                    // The entry owns the login; the path is all that is here.
                } else if s3(into).transpose()?.is_some() {
                    s3_login(key, host_key, password, key_id, region, endpoint)?;
                    far_path_ok(true, into, into)?;
                } else {
                    not_s3(key_id, region, endpoint)?;
                    let far = sftp(into).transpose()?;
                    login("sftp://", far.is_some(), true, key, host_key, password)?;
                }
            }
            Chore::Fetch { from, into, key, host_key, password, key_id, region, endpoint, host, bucket, .. } => {
                need("from", from)?;
                need("into", into)?;
                if sftp(into).is_some() || s3(into).is_some() {
                    return Err(msg::line(msg::CHORE_LOCAL_ONLY, &["fetch", "into", "from"]));
                }
                let own = [
                    ("key", key),
                    ("host_key", host_key),
                    ("password", password),
                    ("key_id", key_id),
                    ("region", region),
                    ("endpoint", endpoint),
                ];
                if named_far(false, host, bucket, from, &own)? {
                    // The entry owns the login; the path is all that is here.
                } else if s3(from).transpose()?.is_some() {
                    s3_login(key, host_key, password, key_id, region, endpoint)?;
                    far_path_ok(false, from, from)?;
                } else {
                    not_s3(key_id, region, endpoint)?;
                    let far = sftp(from).transpose()?;
                    far_path_ok(false, from, from)?;
                    login("sftp://", far.is_some(), true, key, host_key, password)?;
                }
            }
            Chore::Http { url, method, body, content_type, headers, expect } => {
                need("url", url)?;
                // A url that starts with `{param:…}` or `{var:…}` is told once it is filled in.
                let u = url.trim().to_ascii_lowercase();
                if !u.starts_with('{') && !u.starts_with("http://") && !u.starts_with("https://") {
                    return Err(msg::line(msg::CHORE_HTTP_URL, &["url", url]));
                }
                if !METHODS.contains(&method.as_str()) {
                    return Err(msg::line(msg::CHORE_HTTP_METHOD, &[&METHODS.join(", "), method]));
                }
                if body.is_some() && (method == "GET" || method == "HEAD") {
                    return Err(msg::line(msg::CHORE_HTTP_BODY_METHOD, &[method]));
                }
                if body.as_ref().is_some_and(|b| b.chars().count() > BODY_MAX) {
                    return Err(msg::line(msg::CHORE_TEXT_TOO_LONG, &["body", &BODY_MAX.to_string()]));
                }
                if content_type.is_some() && body.is_none() {
                    return Err(msg::line(msg::CHORE_CONTENT_TYPE_NO_BODY, &[]));
                }
                for k in headers.keys() {
                    header_name(k)?;
                }
                if let Some(c) = expect.iter().find(|c| !(100..=599).contains(*c)) {
                    return Err(msg::line(msg::CHORE_HTTP_EXPECT, &[&c.to_string()]));
                }
            }
            Chore::Ssh { on, command, key, host_key, password, max, pass } => {
                if on.trim().is_empty() {
                    return Err(msg::line(msg::CHORE_ON_EMPTY, &["ssh://[user@]host[:port]"]));
                }
                let group = fan(on, *max, *pass)?;
                if let Some(name) = host_name(on) {
                    // A `[[host]]` of koyomi.toml owns the address and the login.
                    named_on(name, &[("key", is_set(key)), ("host_key", is_set(host_key)), ("password", is_set(password))])?;
                    far_command(command)?;
                    return Ok(());
                }
                // An `on` that starts with `{param:…}` or `{var:…}` is told once it is filled in.
                if !group && ssh(on).transpose()?.is_none() && !on.trim().starts_with('{') {
                    return Err(msg::line(msg::CHORE_ON_BAD, &["ssh://[user@]host[:port]", &hide_password(on)]));
                }
                far_command(command)?;
                // A group's PCs may each have their own host key in `[groups]`.
                ssh_login(!group, key, host_key, password)?;
            }
            Chore::Winrm { on, command, cert, password, auth, max, pass } => {
                if on.trim().is_empty() {
                    return Err(msg::line(msg::CHORE_ON_EMPTY, &["winrm://user@host[:port]"]));
                }
                let group = fan(on, *max, *pass)?;
                if let Some(name) = host_name(on) {
                    named_on(name, &[("cert", is_set(cert)), ("password", is_set(password)), ("auth", *auth != WinrmAuth::default())])?;
                    far_command(command)?;
                    return Ok(());
                }
                if !group && winrm(on).transpose()?.is_none() && !on.trim().starts_with('{') {
                    return Err(msg::line(msg::CHORE_ON_BAD, &["winrm://user@host[:port]", &hide_password(on)]));
                }
                far_command(command)?;
                // Each PC of a group may have its own certificate in `[groups]`.
                winrm_login(group, cert, password)?;
            }
            Chore::Mail { to, cc, subject, body, files, .. } => {
                if to.is_empty() {
                    return Err(msg::line(msg::CHORE_MAIL_TO_EMPTY, &[]));
                }
                for a in to.iter().chain(cc) {
                    mail_address(a)?;
                }
                if subject.trim().is_empty() {
                    return Err(msg::line(msg::CHORE_MAIL_SUBJECT_EMPTY, &[]));
                }
                if subject.contains(['\r', '\n']) {
                    return Err(msg::line(msg::CHORE_MAIL_SUBJECT_NEWLINE, &[]));
                }
                if body.as_ref().is_some_and(|b| b.chars().count() > BODY_MAX) {
                    return Err(msg::line(msg::CHORE_TEXT_TOO_LONG, &["body", &BODY_MAX.to_string()]));
                }
                if files.len() > FILES_MAX {
                    return Err(msg::line(msg::CHORE_FILES_MAX, &[&FILES_MAX.to_string()]));
                }
                if files.iter().any(|f| f.trim().is_empty()) {
                    return Err(msg::line(msg::CHORE_FILES_EMPTY, &[]));
                }
            }
            Chore::Sql { db, database, password, sql, file, out, rows, tls } => {
                match database {
                    // A name of koyomi.toml's `[[database]]`: the entry holds the address, the password and the TLS.
                    Some(name) => {
                        if !db.trim().is_empty() {
                            return Err(msg::line(msg::DEF_SQL_DB_AND_DATABASE, &[]));
                        }
                        if !crate::registry::valid_name(name) {
                            return Err(msg::line(msg::DEF_REGISTERED_NAME_BAD, &["database", name]));
                        }
                        // (An explicit `tls = "verify"` is the parser's to refuse; here only what differs from it.)
                        if password.as_deref().is_some_and(|p| !p.trim().is_empty()) {
                            return Err(msg::line(msg::DEF_SQL_DATABASE_OWNS, &["password"]));
                        }
                        if *tls != SqlTls::Verify {
                            return Err(msg::line(msg::DEF_SQL_DATABASE_OWNS, &["tls"]));
                        }
                    }
                    // A `db` that starts with `{param:…}` or `{var:…}` is told once it is filled in.
                    None if db.trim().starts_with('{') => check_db_text(db, password.as_deref())?,
                    None => check_db(db, password.as_deref(), *tls)?,
                }
                match (sql, file) {
                    (None, None) | (Some(_), Some(_)) => return Err(msg::line(msg::CHORE_SQL_ONE_OF, &[])),
                    (Some(s), None) if s.trim().is_empty() => return Err(msg::line(msg::CHORE_KEY_EMPTY, &["sql"])),
                    (Some(s), None) if s.chars().count() > COMMAND_MAX => {
                        return Err(msg::line(msg::CHORE_SQL_TOO_LONG, &[&COMMAND_MAX.to_string()]));
                    }
                    (None, Some(f)) if f.trim().is_empty() => return Err(msg::line(msg::CHORE_KEY_EMPTY, &["file"])),
                    _ => {}
                }
                if out.as_ref().is_some_and(|o| o.trim().is_empty()) {
                    return Err(msg::line(msg::CHORE_KEY_EMPTY, &["out"]));
                }
                if let Some(r) = rows {
                    sql_rows(r)?;
                }
            }
        }
        Ok(())
    }

    /// The PowerShell script that does it, and how its exit code is judged.
    /// The paths are already filled in. An `ssh`, `winrm`, `mail`, `sql`, `delete`,
    /// `empty` or `copy` has none: [`fill`] runs `koyomi ssh run`, `koyomi winrm run`,
    /// `koyomi mail run`, `koyomi sql run` or `koyomi file run` straight (a `copy` has
    /// only its judge: 1 is a warning).
    pub fn script(&self) -> (String, Option<Judge>) {
        match self {
            // Not a script (fill()): koyomi runs it itself as `koyomi file run`.
            Chore::Delete { .. } | Chore::Empty { .. } => (String::new(), None),
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
            // Not a script (fill()): only its judge. 1 is the warning (something in `into` did not match), as
            // robocopy's 4-7 was.
            Chore::Copy { .. } => (String::new(), Some(Judge { warn: vec![Range { from: 1, to: 1 }], ..Default::default() })),
            Chore::Send { from, into, verify, host, .. } | Chore::Fetch { from, into, verify, host, .. } => {
                let warn = Some(Judge { warn: vec![Range { from: NOTHING, to: NOTHING }], ..Default::default() });
                let s = match (self, sftp(from).or_else(|| sftp(into))) {
                    // Not a script (fill()): only its judge.
                    _ if self.cloud() => String::new(),
                    // A `[[host]]`: the same, with the address built when the job starts.
                    _ if host.is_some() => SFTP_SCRIPT.to_owned(),
                    // What to do is in the environment (env()): no path or secret is written here.
                    (_, Some(Ok(_))) => SFTP_SCRIPT.to_owned(),
                    (Chore::Send { .. }, _) => shared(from, into, "send", "sent", verify.is_some()),
                    _ => shared(from, into, "fetch", "fetched", verify.is_some()),
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
    /// (the password as `secret:<name>`, read when the job starts); one with S3 as
    /// `KOYOMI_CLOUD_*` for `koyomi cloud run` (the same); either's `verify` as
    /// `KOYOMI_SFTP_VERIFY` or `KOYOMI_CLOUD_VERIFY`; an `ssh` as
    /// `KOYOMI_SSH_*`, which `koyomi ssh run` reads (fill() adds `KOYOMI_SSH_ENV`); a
    /// `winrm` as `KOYOMI_WINRM_*` for `koyomi winrm run` (and `KOYOMI_WINRM_ENV`),
    /// its `auth` always (`basic` when none is written); a `mail` as `KOYOMI_MAIL_*`
    /// for `koyomi mail run`, a list one per line, each after [`MAIL_TEXT`]; a `sql`
    /// as `KOYOMI_SQL_*` for `koyomi sql run`, each text after [`MAIL_TEXT`] but the
    /// password (a `secret:`) and `tls`; a `delete`, `empty` or `copy` as `KOYOMI_FILE_*` for
    /// `koyomi file run` (`OP` and `KEEP_DAYS` as they are, the folders and the `match` after
    /// [`MAIL_TEXT`]). With a `[[host]]`, `[[bucket]]` or `[[database]]` of koyomi.toml
    /// named, the login (and for a `host` or `bucket` the address) is not here: [`crate::registry::far_env`]
    /// adds it when the job starts, so the far side is as written (the path alone).
    pub fn env(&self) -> Vec<(String, String)> {
        match self {
            Chore::Http { headers, .. } => headers.values().enumerate().map(|(i, v)| (format!("KOYOMI_HEADER_{i}"), v.clone())).collect(),
            Chore::Send { from, into, password, key_id, region, endpoint, verify, bucket, .. }
            | Chore::Fetch { from, into, password, key_id, region, endpoint, verify, bucket, .. }
                if self.cloud() =>
            {
                let mut v = vec![
                    ("KOYOMI_CLOUD_OP".to_owned(), self.key().to_owned()),
                    ("KOYOMI_CLOUD_FROM".to_owned(), from.trim().to_owned()),
                    ("KOYOMI_CLOUD_INTO".to_owned(), into.trim().to_owned()),
                ];
                // A `bucket` owns the login: the engine adds it when the job starts.
                for (name, value) in [("KEY_ID", key_id), ("REGION", region), ("ENDPOINT", endpoint), ("PASSWORD", password)] {
                    if let Some(x) = value.as_deref().map(str::trim).filter(|x| !x.is_empty()).filter(|_| bucket.is_none()) {
                        v.push((format!("KOYOMI_CLOUD_{name}"), x.to_owned()));
                    }
                }
                if let Some(x) = verify {
                    v.push(("KOYOMI_CLOUD_VERIFY".to_owned(), x.key().to_owned()));
                }
                v
            }
            Chore::Send { from, into, key, host_key, password, verify, host, .. }
            | Chore::Fetch { from, into, key, host_key, password, verify, host, .. }
                if host.is_some() || sftp(from).or_else(|| sftp(into)).is_some() =>
            {
                let mut v = vec![
                    ("KOYOMI_SFTP_OP".to_owned(), self.key().to_owned()),
                    ("KOYOMI_SFTP_FROM".to_owned(), from.trim().to_owned()),
                    ("KOYOMI_SFTP_INTO".to_owned(), into.trim().to_owned()),
                ];
                // A `host` owns the login: the engine adds it when the job starts.
                for (name, value) in [("KEY", key), ("HOST_KEY", host_key), ("PASSWORD", password)] {
                    if let Some(x) = value.as_deref().map(str::trim).filter(|x| !x.is_empty()).filter(|_| host.is_none()) {
                        v.push((format!("KOYOMI_SFTP_{name}"), x.to_owned()));
                    }
                }
                if let Some(x) = verify {
                    v.push(("KOYOMI_SFTP_VERIFY".to_owned(), x.key().to_owned()));
                }
                v
            }
            Chore::Ssh { on, command, key, host_key, password, max, pass } => {
                let mut v = vec![("KOYOMI_SSH_ON".to_owned(), on.trim().to_owned()), ("KOYOMI_SSH_COMMAND".to_owned(), command.clone())];
                // With a `[[host]]` name `on` is the name, and the entry owns the login.
                let named = host_name(on).is_some();
                for (name, value) in [("KEY", key), ("HOST_KEY", host_key), ("PASSWORD", password)] {
                    if let Some(x) = value.as_deref().map(str::trim).filter(|x| !x.is_empty()).filter(|_| !named) {
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
                let named = host_name(on).is_some();
                for (name, value) in [("CERT", cert), ("PASSWORD", password)] {
                    if let Some(x) = value.as_deref().map(str::trim).filter(|x| !x.is_empty()).filter(|_| !named) {
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
            Chore::Sql { db, database, password, sql, file, out, rows, tls } => {
                let text = |s: &str| format!("{MAIL_TEXT}{s}");
                // With a `database` the engine adds DB, TLS and PASSWORD when the job starts.
                let mut v = if database.is_some() { Vec::new() } else { crate::registry::sql_target_env(db, password.as_deref(), *tls) };
                if let Some(s) = sql {
                    v.push(("KOYOMI_SQL_TEXT".to_owned(), text(s)));
                }
                for (name, value) in [("FILE", file), ("OUT", out), ("ROWS", rows)] {
                    if let Some(x) = value.as_deref().map(str::trim).filter(|x| !x.is_empty()) {
                        v.push((format!("KOYOMI_SQL_{name}"), text(x)));
                    }
                }
                v
            }
            Chore::Delete { .. } | Chore::Empty { .. } | Chore::Copy { .. } => {
                let text = |s: &str| format!("{MAIL_TEXT}{}", s.trim());
                let mut v = vec![("KOYOMI_FILE_OP".to_owned(), self.key().to_owned())];
                let pattern = |p: &Option<String>| {
                    p.as_deref().map(str::trim).filter(|x| !x.is_empty()).map(|x| ("KOYOMI_FILE_MATCH".to_owned(), text(x)))
                };
                match self {
                    Chore::Delete { path, pattern: p, keep_days } => {
                        v.push(("KOYOMI_FILE_PATH".to_owned(), text(path)));
                        v.extend(pattern(p));
                        v.push(("KOYOMI_FILE_KEEP_DAYS".to_owned(), keep_days.to_string()));
                    }
                    Chore::Empty { path } => v.push(("KOYOMI_FILE_PATH".to_owned(), text(path))),
                    Chore::Copy { from, into, pattern: p } => {
                        v.extend(pattern(p));
                        v.push(("KOYOMI_FILE_FROM".to_owned(), text(from)));
                        v.push(("KOYOMI_FILE_INTO".to_owned(), text(into)));
                    }
                    _ => {}
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
        Err(msg::line(msg::CHORE_MAIL_ADDRESS, &[a]))
    }
}

/// A header name as HTTP takes it; `Content-Type` is `content_type`.
pub fn header_name(k: &str) -> Result<(), String> {
    if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c)) {
        return Err(msg::line(msg::CHORE_HEADER_NAME, &[k]));
    }
    if k.eq_ignore_ascii_case("content-type") {
        return Err(msg::line(msg::CHORE_HEADER_CONTENT_TYPE, &[]));
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
/// runs as a PowerShell job (a `zip`, a `send` or `fetch` with a shared folder); a
/// root or an empty path after the filling stops the run.
/// An `ssh` runs `exe` (koyomi itself) as `exe ssh run`, not through PowerShell:
/// the engine's ask to stop reaches it straight, and PowerShell's debugger does
/// not take the ask and write into the log. It keeps its own `judge`, and the
/// names of its own `env` go to the far side as `KOYOMI_SSH_ENV`. A `winrm` the
/// same, as `exe winrm run` and `KOYOMI_WINRM_ENV`; a `mail` as `exe mail run`, a
/// `sql` as `exe sql run`, a `send` or `fetch` with S3 as `exe cloud run` (nothing
/// to send or fetch is a warning, as over SFTP), a `delete`, `empty` or `copy` as
/// `exe file run` (a `copy` keeps its judge: 1 is a warning).
pub fn fill(net: &mut Jobnet, exe: &str) -> Result<(), String> {
    for j in &mut net.jobs {
        if let Part::Chore(c) = &j.part {
            c.check().map_err(|e| format!("{}: {e}", j.id))?;
            if c.own() {
                let op = c.op();
                (j.shell, j.script) = (None, None);
                j.run = [exe, op, "run"].map(str::to_owned).to_vec();
                if c.cloud() || c.file() {
                    j.judge = c.script().1;
                }
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

/// A folder as `koyomi file run` opens it: trimmed and rebuilt from its parts, so a
/// separator or a `.` at the end is gone (`/x/link/` makes the system follow a link to its
/// target, and the link check must see the link itself); `..` stays. A bare drive or share
/// (`D:`, `\?\D:`, `\srv\share`) gets its root separator back (`D:` alone is the drive's
/// current folder); `/`, `D:\` and an empty path stay as they are.
pub fn folder_path(p: &str) -> PathBuf {
    let path = Path::new(p.trim());
    let mut out: PathBuf = path.components().collect();
    if matches!(path.components().next_back(), Some(Component::Prefix(_))) {
        out.push(std::path::MAIN_SEPARATOR_STR);
    }
    out
}

/// `delete` and `empty` never work on a drive or share root, however it is spelled. Only the
/// text is read (no file system is asked, and on any platform): `.` is dropped and `..`
/// takes the part before it, so `D:\x\..` is the root.
fn not_root(path: &str) -> Result<(), String> {
    let t = path.trim();
    let mut parts = t.split(['\\', '/']).filter(|s| !s.is_empty() && *s != ".").peekable();
    let share = t.starts_with("\\\\") || t.starts_with("//");
    let drive = !share && parts.peek().is_some_and(|f| f.len() == 2 && f.ends_with(':'));
    let anchored = share || drive || t.starts_with(['\\', '/']);
    // The drive, or the server and share, are what the rest hangs on: `..` does not take them.
    let anchor = if share { 2 } else { usize::from(drive) };
    let (mut seen, mut below) = (0, 0usize);
    for part in parts {
        if seen < anchor {
            seen += 1;
        } else if part == ".." {
            below = below.saturating_sub(1);
        } else {
            below += 1;
        }
    }
    if t.is_empty() || (anchored && below == 0) {
        return Err(msg::line(msg::CHORE_NOT_ROOT, &[path]));
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
    Some(guarded(t, |shown, masked| read_sftp(rest, "sftp://", shown, masked)))
}

/// Runs `read` on an address that has no password written in it, giving it the hidden form of the address and whether
/// hiding changed it; with a password written in it, the address is refused first.
fn guarded<T>(t: &str, read: impl FnOnce(&str, bool) -> Result<T, String>) -> Result<T, String> {
    let shown = hide_password(t);
    if has_password(t) {
        return Err(msg::line(msg::ADDR_PASSWORD, &[&shown]));
    }
    let masked = shown != t;
    read(&shown, masked)
}

/// `s` read as an `ssh://[user@]host[:port]` address (no path: `path` is `.`); `None`
/// when it does not start with `ssh://`.
pub fn ssh(s: &str) -> Option<Result<Sftp, String>> {
    let t = s.trim();
    let rest = t.get(..6).filter(|p| p.eq_ignore_ascii_case("ssh://")).map(|_| &t[6..])?;
    let read = guarded(t, |shown, masked| {
        read_sftp(rest, "ssh://", shown, masked).and_then(|far| match rest.split_once('/') {
            Some((_, p)) if !p.is_empty() => Err(msg::line(msg::CHORE_ADDR_PATH, &["ssh://", "ssh://[user@]host[:port]", shown])),
            _ => Ok(far),
        })
    });
    Some(read)
}

/// `s` read as a `winrm://user@host[:port]` address (no path: `path` is `.`, no
/// port: [`WINRM_PORT`]); `None` when it does not start with `winrm://`. The user
/// is needed: a password is for someone. It may be `DOMAIN\user` or `.\user`.
pub fn winrm(s: &str) -> Option<Result<Sftp, String>> {
    let t = s.trim();
    let rest = t.get(..8).filter(|p| p.eq_ignore_ascii_case("winrm://")).map(|_| &t[8..])?;
    let read = guarded(t, |shown, masked| {
        read_sftp(rest, "winrm://", shown, masked).and_then(|far| match rest.split_once('/') {
            Some((_, p)) if !p.is_empty() => Err(msg::line(msg::CHORE_ADDR_PATH, &["winrm://", "winrm://user@host[:port]", shown])),
            _ if far.user.is_none() => Err(msg::line(msg::CHORE_WINRM_USER, &[shown])),
            _ => Ok(far),
        })
    });
    Some(read)
}

/// Where a `sql`'s `db` points.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// Where an `s3://bucket/key` points.
#[derive(Debug, PartialEq, Eq)]
pub struct S3 {
    pub bucket: String,
    /// The key (or a pattern, or a folder ending in `/`) in the bucket, without a
    /// leading `/`; empty for the bucket's top.
    pub key: String,
}

/// `s` read as an `s3://bucket/key` address; `None` when it does not start with
/// `s3://`. A bucket is 3 to 63 letters, digits, `.`, `-` and `_` (told once a
/// `{param:…}` in it is filled in).
pub fn s3(s: &str) -> Option<Result<S3, String>> {
    let t = s.trim();
    let rest = after_scheme(t, "s3://")?;
    let (bucket, key) = rest.split_once('/').unwrap_or((rest, ""));
    let good = |b: &str| (3..=63).contains(&b.len()) && b.chars().all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c));
    let read = if bucket.is_empty() || (!bucket.contains('{') && !good(bucket)) {
        Err(msg::line(msg::CHORE_S3_BUCKET, &[&shown_address(t)]))
    } else if key.chars().any(char::is_control) {
        Err(msg::line(msg::CHORE_S3_KEY_CONTROL, &[&shown_address(t)]))
    } else {
        Ok(S3 { bucket: bucket.to_owned(), key: key.trim_start_matches('/').to_owned() })
    };
    Some(read)
}

/// How a `send` or `fetch` with S3 logs in: `key_id`, and `password` (the secret
/// access key) as a `secret:`; `region` unless `endpoint` is written. `key` and
/// `host_key` are for `sftp://`.
fn s3_login(
    key: &Option<String>,
    host_key: &Option<String>,
    password: &Option<String>,
    key_id: &Option<String>,
    region: &Option<String>,
    endpoint: &Option<String>,
) -> Result<(), String> {
    let val = |v: &Option<String>| v.as_deref().map(str::trim).filter(|x| !x.is_empty()).map(str::to_owned);
    if let Some((name, _)) = [("key", key), ("host_key", host_key)].into_iter().find(|(_, v)| val(v).is_some()) {
        return Err(msg::line(msg::CHORE_S3_NOT_SFTP_KEY, &[name]));
    }
    match val(key_id) {
        None => return Err(msg::line(msg::CHORE_S3_KEY_ID_MISSING, &[])),
        Some(k) if k.strip_prefix("secret:").is_some_and(|n| n.trim().is_empty()) => {
            return Err(msg::line(msg::CHORE_S3_KEY_ID_SECRET_NAME, &[]));
        }
        Some(k) if !k.starts_with("secret:") && k.chars().any(|c| c.is_whitespace() || c.is_control()) => {
            return Err(msg::line(msg::CHORE_S3_KEY_ID_SPACE, &[&k]));
        }
        Some(_) => {}
    }
    match val(password) {
        None => {
            return Err(msg::line(msg::CHORE_S3_SECRET_KEY_MISSING, &[]));
        }
        Some(p) if p.strip_prefix("secret:").is_none_or(|n| n.trim().is_empty()) => {
            return Err(msg::line(msg::CHORE_PASSWORD_SECRET, &[]));
        }
        Some(_) => {}
    }
    let (region, endpoint) = (val(region), val(endpoint));
    match &region {
        None if endpoint.is_none() => {
            return Err(msg::line(msg::CHORE_S3_REGION_MISSING, &[]));
        }
        // `{param:…}` or `{var:…}` is told once it is filled in.
        Some(r) if !r.starts_with('{') && !r.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') => {
            return Err(msg::line(msg::CHORE_S3_REGION_BAD, &[r]));
        }
        _ => {}
    }
    if let Some(e) = endpoint.filter(|e| !e.starts_with('{')) {
        let Some(rest) = after_scheme(&e, "https://").or_else(|| after_scheme(&e, "http://")) else {
            return Err(msg::line(msg::CHORE_HTTP_URL, &["endpoint", &hide_password(&e)]));
        };
        let host = rest.strip_suffix('/').unwrap_or(rest);
        if host.is_empty() || host.contains(['/', '?', '#', '@']) || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(msg::line(msg::CHORE_S3_ENDPOINT_HOST, &[&hide_password(&e)]));
        }
    }
    Ok(())
}

/// `key_id`, `region` and `endpoint` go only with `s3://`.
fn not_s3(key_id: &Option<String>, region: &Option<String>, endpoint: &Option<String>) -> Result<(), String> {
    let set = |v: &Option<String>| v.as_deref().is_some_and(|x| !x.trim().is_empty());
    match [("key_id", key_id), ("region", region), ("endpoint", endpoint)].into_iter().find(|(_, v)| set(v)) {
        Some((name, _)) => Err(msg::line(msg::CHORE_ONLY_FOR, &[name, "s3://"])),
        None => Ok(()),
    }
}

/// What can be told of a `sql`'s `db` and `password` without reading the address: `db` is given, and a
/// `password` is a `secret:<name>`. (A `db` that is a template is told once it is filled in.)
fn check_db_text(db: &str, password: Option<&str>) -> Result<(), String> {
    if db.trim().is_empty() {
        return Err(msg::line(msg::CHORE_SQL_DB_EMPTY, &[]));
    }
    let password = password.map(str::trim).filter(|p| !p.is_empty());
    if let Some(p) = password {
        if p.strip_prefix("secret:").is_none_or(|n| n.trim().is_empty()) {
            return Err(msg::line(msg::CHORE_PASSWORD_SECRET, &[]));
        }
    }
    Ok(())
}

/// The connection of a `sql`, whether written in the job (`db`, `password`, `tls`) or registered as a
/// `[[database]]`: `db` is given and reads (`sql_db`), `password` is a `secret:<name>`, and the user,
/// the password and the TLS fit the kind of server (a login needs its password; SQL Server without a
/// user takes none, and never `tls = "off"`; SQLite takes neither a password nor any TLS but `verify`).
pub fn check_db(db: &str, password: Option<&str>, tls: SqlTls) -> Result<(), String> {
    check_db_text(db, password)?;
    let password = password.map(str::trim).filter(|p| !p.is_empty());
    let parsed = sql_db(db)?;
    match &parsed {
        SqlDb::SqlServer { user: None, .. } if password.is_some() => Err(msg::line(msg::CHORE_SQL_WINDOWS_PASSWORD, &[])),
        SqlDb::SqlServer { user: Some(_), .. } | SqlDb::Postgres { .. } if password.is_none() => {
            let scheme = if matches!(parsed, SqlDb::Postgres { .. }) { "postgres://" } else { "sqlserver://" };
            Err(msg::line(msg::CHORE_PASSWORD_MISSING, &[scheme]))
        }
        SqlDb::SqlServer { .. } if tls == SqlTls::Off => Err(msg::line(msg::CHORE_SQL_TLS_OFF, &[])),
        SqlDb::Sqlite { .. } if password.is_some() => Err(msg::line(msg::CHORE_SQLITE_NO, &["password"])),
        SqlDb::Sqlite { .. } if tls != SqlTls::Verify => Err(msg::line(msg::CHORE_SQLITE_NO, &["tls"])),
        _ => Ok(()),
    }
}

/// Whether the text after `sqlite://` is a file path as [`sql_db`] takes it: a drive (`C:\` or `C:/`) or a start of
/// `\`, `/` or `{`.
fn sqlite_shape(path: &str) -> bool {
    let b = path.as_bytes();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
    drive || path.starts_with(['\\', '/', '{'])
}

/// `s` read as a `sql`'s `db`. A named instance (`host\name`) is not taken: its
/// port is written instead, as ito-sql connects by the port.
pub fn sql_db(s: &str) -> Result<SqlDb, String> {
    let t = s.trim();
    if let Some(path) = after_scheme(t, "sqlite://") {
        if path.contains(['\r', '\n']) || !sqlite_shape(path) {
            return Err(msg::line(msg::CHORE_SQLITE_PATH, &[&hide_password(t)]));
        }
        return Ok(SqlDb::Sqlite { path: path.to_owned() });
    }
    if has_password(t) {
        return Err(msg::line(msg::ADDR_PASSWORD, &[&hide_password(t)]));
    }
    let shown = hide_password(t);
    let masked = shown != t;
    let (rest, scheme, server) = if let Some(r) = after_scheme(t, "sqlserver://") {
        (r, "sqlserver://", true)
    } else if let Some(r) = after_scheme(t, "postgres://") {
        (r, "postgres://", false)
    } else if let Some(r) = after_scheme(t, "postgresql://") {
        (r, "postgresql://", false)
    } else {
        return Err(msg::line(msg::CHORE_DB_SCHEME, &[&shown]));
    };
    let form = if server { "sqlserver://[user@]host[:port]/database" } else { "postgres://user@host[:port]/database" };
    let Some((auth, database)) = rest.split_once('/').filter(|(_, d)| !d.trim().is_empty()) else {
        return Err(msg::line(msg::CHORE_DB_NAME_MISSING, &[scheme, form, &shown]));
    };
    if database.contains(['/', '?']) || database.chars().any(char::is_control) {
        return Err(msg::line(msg::CHORE_DB_NAME_BAD, &[&shown]));
    }
    let far = read_sftp(auth, scheme, &shown, masked)?;
    if far.host.contains('\\') {
        return Err(if server {
            msg::line(msg::CHORE_SQL_INSTANCE, &[&shown])
        } else {
            msg::line(msg::CHORE_DB_BACKSLASH, &["host", &shown])
        });
    }
    if far.user.as_deref().is_some_and(|u| u.contains('\\')) {
        return Err(if server {
            msg::line(msg::CHORE_SQL_USER_WINDOWS, &[&shown])
        } else {
            msg::line(msg::CHORE_DB_BACKSLASH, &["user", &shown])
        });
    }
    let database = database.to_owned();
    if server {
        return Ok(SqlDb::SqlServer { user: far.user, host: far.host, port: far.port.unwrap_or(SQL_SERVER_PORT), database });
    }
    let Some(user) = far.user else {
        return Err(msg::line(msg::CHORE_DB_USER_MISSING, &[scheme, form, &shown]));
    };
    Ok(SqlDb::Postgres { user, host: far.host, port: far.port.unwrap_or(POSTGRES_PORT), database })
}

/// Which field of a database form does not make a `db` that reads back as itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbField {
    Host,
    Port,
    Database,
    User,
    File,
}

/// The `db` URL of `db` (PostgreSQL or SQLite; SQL Server is not built here: `Err(DbField::Host)`), refused
/// unless `sql_db` reads it back as exactly the same parts.
pub fn db_url(db: &SqlDb) -> Result<String, DbField> {
    const HOST_BAD: &str = "/@\\{}?#\"'`;&|<>";
    const USER_BAD: &str = ":/\\{}\"'`;&|<>";
    let url = match db {
        SqlDb::SqlServer { .. } => return Err(DbField::Host),
        SqlDb::Postgres { user, host, port, database } => {
            let bracketed =
                host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).is_some_and(|h| h.parse::<std::net::Ipv6Addr>().is_ok());
            let colon_ok = bracketed || !host.contains([':', '[', ']']);
            if host.is_empty()
                || host.starts_with('-')
                || host.chars().any(|c| c.is_whitespace() || c.is_control() || HOST_BAD.contains(c))
                || !colon_ok
            {
                return Err(DbField::Host);
            }
            if *port == 0 {
                return Err(DbField::Port);
            }
            if database.trim().is_empty() || database.contains(['/', '?', '{', '}']) || database.chars().any(char::is_control) {
                return Err(DbField::Database);
            }
            if user.is_empty() || user.chars().any(|c| c.is_whitespace() || c.is_control() || USER_BAD.contains(c)) {
                return Err(DbField::User);
            }
            if *port == POSTGRES_PORT && !host.contains(':') {
                format!("postgres://{user}@{host}/{database}")
            } else {
                format!("postgres://{user}@{host}:{port}/{database}")
            }
        }
        SqlDb::Sqlite { path } => {
            let b = path.as_bytes();
            let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
            if path.is_empty() || path.contains(['{', '}', '\r', '\n']) || !(drive || path.starts_with(['\\', '/'])) {
                return Err(DbField::File);
            }
            format!("sqlite://{path}")
        }
    };
    match (sql_db(&url), db) {
        (Ok(back), _) if back == *db => Ok(url),
        (Ok(SqlDb::Postgres { host: h, port: p, database: d, .. }), SqlDb::Postgres { host, port, database, .. }) => Err(if h != *host {
            DbField::Host
        } else if p != *port {
            DbField::Port
        } else if d != *database {
            DbField::Database
        } else {
            DbField::User
        }),
        (_, SqlDb::Sqlite { .. }) => Err(DbField::File),
        _ => Err(DbField::Host),
    }
}

/// Which field of a 接続先 PC form does not make an address that reads back as itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostField {
    User,
    Host,
    Port,
}

/// `host` as it is written in an address and whether it is an IPv6 literal: `::1` and `[::1]` are written `[::1]`
/// (the readers take the last `:` as the port, so the brackets are kept); any other `:`, `[` or `]` is refused.
/// `None` for a host that is empty, starts with `-`, or holds a character no reader accepts (`/@\{}?#"'`;&|<>`,
/// white space, control characters). The text is otherwise kept as typed.
pub fn host_text(host: &str) -> Option<(String, bool)> {
    const HOST_BAD: &str = "/@\\{}?#\"'`;&|<>";
    if host.is_empty() || host.starts_with('-') || host.chars().any(|c| c.is_whitespace() || c.is_control() || HOST_BAD.contains(c)) {
        return None;
    }
    let inner = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
    if host.contains([':', '[', ']']) {
        return inner.parse::<std::net::Ipv6Addr>().ok().map(|_| (format!("[{inner}]"), true));
    }
    Some((host.to_owned(), false))
}

/// The `on` address of an `ssh` or `winrm` (`ssh://[user@]host[:port]`, `winrm://user@host[:port]`) from its parts,
/// refused (with the field at fault) unless [`ssh`] / [`winrm`] reads it back as exactly the same parts. The port is
/// left out when it is the kind's own (22, [`WINRM_PORT`]) and the host is not an IPv6 literal. `user` may be empty
/// for SSH only.
pub fn host_url(winrm_kind: bool, user: &str, host: &str, port: u16) -> Result<String, HostField> {
    const USER_BAD: &str = ":/{}\"'`;&|<>";
    if user.chars().any(|c| c.is_whitespace() || c.is_control() || USER_BAD.contains(c)) || (winrm_kind && user.is_empty()) {
        return Err(HostField::User);
    }
    let (host, v6) = host_text(host).ok_or(HostField::Host)?;
    if port == 0 {
        return Err(HostField::Port);
    }
    let default = if winrm_kind { WINRM_PORT } else { SSH_PORT };
    let scheme = if winrm_kind { "winrm://" } else { "ssh://" };
    let at = if user.is_empty() { String::new() } else { format!("{user}@") };
    let url = if port == default && !v6 { format!("{scheme}{at}{host}") } else { format!("{scheme}{at}{host}:{port}") };
    let far = if winrm_kind { winrm(&url) } else { ssh(&url) };
    match far {
        Some(Ok(far))
            if far.user.as_deref().unwrap_or("") == user && far.host == host && far.port.unwrap_or(default) == port && far.path == "." =>
        {
            Ok(url)
        }
        _ => Err(HostField::Host),
    }
}

/// Which field of an S3 form does not make a `url` that reads back as itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BucketField {
    Bucket,
    Folder,
}

/// The `url` of a `[[bucket]]` from its bucket name and folder (`reports/daily` is written `s3://b/reports/daily/`; the
/// slashes at the ends are trimmed), refused unless [`s3`] and `registry::bucket_prefix` read it back as the same parts.
pub fn bucket_url(bucket: &str, folder: &str) -> Result<String, BucketField> {
    if bucket.contains(['{', '}']) {
        return Err(BucketField::Bucket);
    }
    let folder = folder.trim().trim_matches('/');
    if (!folder.is_empty() && folder.split('/').any(str::is_empty))
        || folder.contains(['*', '?', '{', '}'])
        || folder.chars().any(char::is_control)
    {
        return Err(BucketField::Folder);
    }
    let url = if folder.is_empty() { format!("s3://{bucket}/") } else { format!("s3://{bucket}/{folder}/") };
    match s3(&url) {
        Some(Ok(at)) if at.bucket == bucket => {}
        _ => return Err(BucketField::Bucket),
    }
    let want = if folder.is_empty() { String::new() } else { format!("{folder}/") };
    match crate::registry::bucket_prefix(&url) {
        Ok(p) if p == want => Ok(url),
        _ => Err(BucketField::Folder),
    }
}

/// A `sql`'s `rows` read as the counts that are 正常, each `(least, most)`:
/// `0`, `1..` (one or more), `1..100`, `..5`, several with `,`.
pub fn sql_rows(s: &str) -> Result<Vec<(u64, u64)>, String> {
    let bad = || msg::line(msg::CHORE_ROWS, &[s]);
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

/// Whether an optional text is written (not empty).
fn is_set(v: &Option<String>) -> bool {
    v.as_deref().is_some_and(|x| !x.trim().is_empty())
}

/// The name of a `[[host]]` when `on` is one: not a group, not empty, not a template, and no `:` or
/// `@` (so not an address such as `ssh://web1`, nor `admin@web1` written without its scheme, which is
/// told as an address that does not read). Whether it is a name a registration can have is
/// [`crate::registry::valid_name`]'s to say.
pub fn host_name(on: &str) -> Option<&str> {
    let t = on.trim();
    if group(t).is_some() || t.is_empty() || t.starts_with('{') || t.contains([':', '@']) {
        None
    } else {
        Some(t)
    }
}

/// An `ssh` or `winrm` with a host name: the name is one, and nothing the entry owns is written
/// (`own` is each such key and whether the job has it).
fn named_on(name: &str, own: &[(&str, bool)]) -> Result<(), String> {
    if !crate::registry::valid_name(name) {
        return Err(msg::line(msg::DEF_REGISTERED_NAME_BAD, &["on", &hide_password(name)]));
    }
    match own.iter().find(|(_, set)| *set) {
        Some((key, _)) => Err(msg::line(msg::DEF_NAMED_OWNS, &[key, "on", "host"])),
        None => Ok(()),
    }
}

/// Whether a `send` or `fetch` names a `[[host]]` or a `[[bucket]]`; if it does, everything that
/// can be told without the entry: not both, a name that is one, a far side (`far`: `into` of a
/// send, `from` of a fetch) that is a path and not an address, none of `own` (the login keys the
/// entry owns), a bucket path that does not start with `/`, and the path's own rules ([`far_path_ok`])
/// as they would read after the join. A path with `{` is told once it is filled in.
fn named_far(
    send: bool,
    host: &Option<String>,
    bucket: &Option<String>,
    far: &str,
    own: &[(&str, &Option<String>)],
) -> Result<bool, String> {
    let (key, name) = match (host, bucket) {
        (Some(_), Some(_)) => return Err(msg::line(msg::DEF_HOST_AND_BUCKET, &[])),
        (Some(h), None) => ("host", h.as_str()),
        (None, Some(b)) => ("bucket", b.as_str()),
        (None, None) => return Ok(false),
    };
    if !crate::registry::valid_name(name) {
        return Err(msg::line(msg::DEF_REGISTERED_NAME_BAD, &[key, &hide_password(name)]));
    }
    if far.contains("://") {
        return Err(msg::line(msg::DEF_NAMED_FAR_URL, &[if send { "into" } else { "from" }, key]));
    }
    if let Some((k, _)) = own.iter().find(|(_, v)| is_set(v)) {
        return Err(msg::line(msg::DEF_NAMED_OWNS, &[k, key, key]));
    }
    if key == "bucket" && far.trim().starts_with('/') {
        return Err(msg::line(msg::DEF_BUCKET_PATH_SLASH, &[far.trim()]));
    }
    if !far.contains('{') {
        // A valid address of the same kind stands in for the entry's.
        let url = if key == "host" { sftp_join("ssh://placeholder", far)? } else { s3_join("s3://placeholder", far)? };
        far_path_ok(send, &url, far)?;
    }
    Ok(true)
}

/// The rules of the far side of a `send` or `fetch` that hold however its address comes about, written
/// whole (`url` is then what is written) or built from a registration ([`sftp_join`], [`s3_join`]).
/// `shown` is the text the lines show: the address as written, or the path alone. S3: a `send`'s key has
/// no `*` or `?`; a `fetch`'s key names a file (not empty, not a folder) and has a wildcard only in its
/// name. SFTP: the address reads, and a `fetch` names a file, not the home or a folder.
pub fn far_path_ok(send: bool, url: &str, shown: &str) -> Result<(), String> {
    if let Some(at) = s3(url).transpose()? {
        if send {
            if at.key.contains(['*', '?']) {
                return Err(msg::line(msg::CHORE_S3_KEY_WILDCARD, &[shown]));
            }
        } else {
            if at.key.is_empty() || at.key.ends_with('/') {
                return Err(msg::line(msg::CHORE_FETCH_FROM_NAME, &[shown]));
            }
            if at.key[..at.key.rfind('/').map_or(0, |i| i + 1)].contains(['*', '?']) {
                return Err(msg::line(msg::CHORE_S3_WILDCARD_FOLDER, &[shown]));
            }
        }
    } else if let Some(at) = sftp(url).transpose()? {
        if !send && (at.path == "." || at.path.ends_with('/')) {
            return Err(msg::line(msg::CHORE_FETCH_FROM_NAME, &[shown]));
        }
    }
    Ok(())
}

/// The `sftp://` address of `path` on the PC `host_on` (`ssh://[user@]host[:port]`, a `[[host]]`'s
/// `on`), built from what [`ssh`] reads of it: `/x` is `sftp://A/x`, nothing, `~` and `~/` the home
/// (`sftp://A/~`), `~/x` and `x` below the home (`sftp://A/~/x`). The result reads as [`sftp`] does.
pub fn sftp_join(host_on: &str, path: &str) -> Result<String, String> {
    let at = match ssh(host_on) {
        Some(r) => r?,
        None => return Err(msg::line(msg::CHORE_ON_BAD, &["ssh://[user@]host[:port]", &hide_password(host_on)])),
    };
    let mut auth = String::new();
    if let Some(u) = &at.user {
        auth.push_str(u);
        auth.push('@');
    }
    auth.push_str(&at.host);
    if let Some(p) = at.port {
        auth.push_str(&format!(":{p}"));
    }
    let p = path.trim();
    let tail = if p.starts_with('/') {
        p.to_owned()
    } else if p.is_empty() || p == "~" || p == "~/" {
        "/~".to_owned()
    } else if p.starts_with("~/") {
        format!("/{p}")
    } else {
        format!("/~/{p}")
    };
    let url = format!("sftp://{auth}{tail}");
    sftp(&url).transpose()?;
    Ok(url)
}

/// The `s3://` address of `path` in the `[[bucket]]` whose `url` is `bucket_url`: the bucket, its
/// folder ([`crate::registry::bucket_prefix`]) and the path. A path that starts with `/` is refused
/// (it is not stripped).
pub fn s3_join(bucket_url: &str, path: &str) -> Result<String, String> {
    let at = match s3(bucket_url) {
        Some(r) => r?,
        None => return Err(msg::line(msg::CHORE_S3_BUCKET, &[&shown_address(bucket_url)])),
    };
    let prefix = crate::registry::bucket_prefix(bucket_url)?;
    let p = path.trim();
    if p.starts_with('/') {
        return Err(msg::line(msg::DEF_BUCKET_PATH_SLASH, &[p]));
    }
    let url = format!("s3://{}/{prefix}{p}", at.bucket);
    s3(&url).transpose()?;
    Ok(url)
}

/// How an `ssh` logs in (also a `[[host]]`'s): `host_key` is needed unless `need_host_key` is false,
/// `password` is a `secret:`, and `key` and `password` go one at a time.
pub fn ssh_login(need_host_key: bool, key: &Option<String>, host_key: &Option<String>, password: &Option<String>) -> Result<(), String> {
    login("ssh://", true, need_host_key, key, host_key, password)
}

/// How a `winrm` logs in (also a `[[host]]`'s): `cert` is needed unless `group`, a SHA-256; `password`
/// is needed, as a `secret:`.
pub fn winrm_login(group: bool, cert: &Option<String>, password: &Option<String>) -> Result<(), String> {
    match cert.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        None if group => {}
        None => return Err(msg::line(msg::CHORE_WINRM_CERT_MISSING, &[])),
        Some(c) if c.starts_with('{') => {}
        Some(c) => cert_ok(c)?,
    }
    match password.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        None => Err(msg::line(msg::CHORE_PASSWORD_MISSING, &["winrm://"])),
        Some(p) if p.strip_prefix("secret:").is_none_or(|n| n.trim().is_empty()) => Err(msg::line(msg::CHORE_PASSWORD_SECRET, &[])),
        Some(_) => Ok(()),
    }
}

/// How an S3 place (also a `[[bucket]]`'s) logs in: `key_id`, `password` (the secret access key, a
/// `secret:`), and `region` unless `endpoint` is written.
pub fn s3_account(
    key_id: &Option<String>,
    region: &Option<String>,
    endpoint: &Option<String>,
    password: &Option<String>,
) -> Result<(), String> {
    s3_login(&None, &None, password, key_id, region, endpoint)
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
        return Err(msg::line(msg::CHORE_COMMAND_EMPTY, &[]));
    }
    if command.chars().count() > COMMAND_MAX {
        return Err(msg::line(msg::CHORE_TEXT_TOO_LONG, &["command", &COMMAND_MAX.to_string()]));
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
        Err(msg::line(msg::CHORE_CERT_BAD, &[c]))
    }
}

/// Where the password of an address is written, when it is written in the address: `scheme://[user[:password]@]host…`.
/// This is the **strict** rule, which decides that an address has a password in it (the readers refuse it). On
/// `addr.trim()`: the scheme ends at the first `://` (none: `None`; `sqlite://` is a file path: `None`); the authority
/// is the text after it up to the first `/`; `at` is the last `@` of the authority (none: `None`); the colon is the
/// first `:` before `at` that is not inside a `{param:…}` or `{var:…}` template (none: `None`). The span is what lies
/// between that colon and `at`, as a range into `addr` as given. `[` and `]` are not special: the user part never
/// holds an IPv6 host, the host is after `@`. The broad rule is [`hide_password`].
pub fn password_span(addr: &str) -> Option<std::ops::Range<usize>> {
    let t = addr.trim();
    let lead = addr.len() - addr.trim_start().len();
    let (scheme, rest) = t.split_once("://")?;
    if scheme.eq_ignore_ascii_case("sqlite") {
        return None;
    }
    let base = lead + scheme.len() + 3;
    let auth = rest.split_once('/').map_or(rest, |(a, _)| a);
    let at = auth.rfind('@')?;
    let colon = first_colon(&auth[..at])?;
    Some(base + colon + 1..base + at)
}

/// Whether [`password_span`] finds a password written in `addr` (the strict rule).
pub fn has_password(addr: &str) -> bool {
    password_span(addr).is_some()
}

/// `text` with a password written in an address hidden as `***`, for an error line that shows an address. This is the
/// **broad** rule. If the strict rule ([`password_span`]) finds a password, that span is replaced. Otherwise, for a text
/// that may be an address that was not accepted (`sftp://u:ab/cd@h/x`, a password that holds a `/`): in the text after
/// the first `://` (the whole text when there is none; a `sqlite://` address is left alone), what lies between the first
/// `:` (outside a template) and the last `@` is replaced. It hides more than it must rather than leak
/// (`sftp://h:22/in@box` shows as `sftp://h:***@box`), so use it only where an address is refused or reported as
/// unreadable; [`shown_address`] is for a stored value that is shown as is.
pub fn hide_password(text: &str) -> std::borrow::Cow<'_, str> {
    let cut = |r: std::ops::Range<usize>| std::borrow::Cow::Owned(format!("{}***{}", &text[..r.start], &text[r.end..]));
    if let Some(r) = password_span(text) {
        return cut(r);
    }
    let t = text.trim();
    let lead = text.len() - text.trim_start().len();
    let (base, rest) = match t.split_once("://") {
        // A `sqlite://` address with an accepted file path is left alone; one without it is read like any other text.
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("sqlite") && sqlite_shape(rest) => return std::borrow::Cow::Borrowed(text),
        Some((scheme, rest)) => (lead + scheme.len() + 3, rest),
        None => (lead, t),
    };
    let Some(at) = rest.rfind('@') else { return std::borrow::Cow::Borrowed(text) };
    match first_colon(&rest[..at]) {
        Some(colon) => cut(base + colon + 1..base + at),
        None => std::borrow::Cow::Borrowed(text),
    }
}

/// A stored address as it is shown in a list: unchanged, unless it has a password written in it (the strict rule), then
/// hidden. A row from an older engine may still hold one.
pub fn shown_address(addr: &str) -> std::borrow::Cow<'_, str> {
    if has_password(addr) {
        hide_password(addr)
    } else {
        std::borrow::Cow::Borrowed(addr)
    }
}

/// The index of the first `:` of `s` that is not inside a `{param:…}` or `{var:…}` template.
fn first_colon(s: &str) -> Option<usize> {
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if let Some(open) = ["{param:", "{var:"].into_iter().find(|o| rest.starts_with(o)) {
            if let Some(end) = rest[open.len()..].find('}') {
                i += open.len() + end + 1;
                continue;
            }
        }
        let c = rest.chars().next()?;
        if c == ':' {
            return Some(i);
        }
        i += c.len_utf8();
    }
    None
}

/// `rest` (the text after `scheme`) read as `[user@]host[:port][/path]`; `addr` is the address as an error line shows it (the
/// hidden form), `masked` that hiding changed it.
fn read_sftp(rest: &str, scheme: &str, addr: &str, masked: bool) -> Result<Sftp, String> {
    let (auth, path) = rest.split_once('/').unwrap_or((rest, ""));
    // A `:` inside `{param:…}` or `{var:…}` is not a port.
    let (dest, port) = match auth.rsplit_once(':') {
        Some((d, p)) if !p.contains('}') => match p.parse::<u16>() {
            Ok(n) if n > 0 => (d, Some(n)),
            _ => return Err(msg::line(msg::CHORE_PORT_BAD, &[if masked { "***" } else { p }, addr])),
        },
        _ => (auth, None),
    };
    let (user, host) = match dest.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, dest),
    };
    if host.is_empty() || user == Some("") {
        return Err(msg::line(msg::CHORE_ADDR_HOST_MISSING, &[scheme, addr]));
    }
    if host.starts_with('-') || dest.chars().any(|c| c.is_whitespace() || "\"'`;&|<>".contains(c)) {
        return Err(msg::line(msg::CHORE_ADDR_CHARS, &[addr]));
    }
    if path.contains(['\n', '\r']) {
        return Err(msg::line(msg::CHORE_ADDR_PATH_NEWLINE, &[addr]));
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
            return Err(msg::line(msg::CHORE_ONLY_FOR, &[name, scheme]));
        }
        return Ok(());
    }
    match host_key.as_deref().map(str::trim).filter(|h| !h.is_empty()) {
        None if !need_host_key => {}
        None => {
            return Err(msg::line(msg::CHORE_HOST_KEY_MISSING, &[scheme]));
        }
        // `{param:…}` or `{var:…}` is told once it is filled in.
        Some(h) if h.starts_with('{') => {}
        Some(h) => host_key_ok(h)?,
    }
    if let Some(p) = password.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        if p.strip_prefix("secret:").is_none_or(|n| n.trim().is_empty()) {
            return Err(msg::line(msg::CHORE_PASSWORD_SECRET, &[]));
        }
        if set(key) {
            return Err(msg::line(msg::CHORE_KEY_AND_PASSWORD, &[]));
        }
    }
    Ok(())
}

/// `SHA256:` and the digest in base64, as `ssh-keygen -l` and `koyomi sftp hostkey` print it.
pub fn host_key_ok(h: &str) -> Result<(), String> {
    let digest = h.get(..7).filter(|p| p.eq_ignore_ascii_case("sha256:")).map(|_| h[7..].trim_end_matches('='));
    match digest {
        Some(d) if d.len() == 43 && d.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/') => Ok(()),
        _ => Err(msg::line(msg::CHORE_HOST_KEY_BAD, &[h])),
    }
}

/// `send` and `fetch` between folders and shares: each file `from` matches is
/// copied into `into`, which is made when it is not there. With `verify`, each goes
/// to `<name>.part` first and gets its name only when both files hash the same; when
/// they do not, the `.part` goes and the job ends 異常 with why. Each file moved is
/// added to the list `KOYOMI_MOVED` names, as [`crate::run::Moved`].
fn shared(from: &str, into: &str, verb: &str, done: &str, verify: bool) -> String {
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         [Console]::OutputEncoding = [Text.Encoding]::UTF8\n\
         $from = {f}\n\
         $into = {i}\n\
         $verb = '{verb}'\n\
         $done = '{done}'\n\
         $verify = ${verify}\n\
         $nothing = {NOTHING}\n\
         $differs = {d}\n\
         {SHARED_SCRIPT}",
        f = quote(from),
        i = quote(into),
        d = quote(&msg::note(msg::N_CHORE_SHARED_DIFFERS, &["{0}", "{1}", "{2}"])),
    )
}

/// The body of [`shared`], after its values. A record is TOML, a `[[file]]` each,
/// written as UTF-8 without a BOM; a name's `\` and `"` are escaped. The hash is
/// .NET's, not `Get-FileHash`: Windows PowerShell started from PowerShell 7 may not
/// load the module that has it (its `PSModulePath` points at 7's).
const SHARED_SCRIPT: &str = r#"$files = @(Get-ChildItem -Path $from -File)
if ($files.Count -eq 0) { Write-Output "no file to ${verb}: $from"; exit $nothing }
New-Item -ItemType Directory -Force -Path $into | Out-Null
$utf8 = New-Object System.Text.UTF8Encoding($false)
function Kept($name, $bytes, $sha, $matched) {
    if (-not $env:KOYOMI_MOVED) { return }
    $t = "[[file]]`nname = `"" + $name.Replace('\', '\\').Replace('"', '\"') + "`"`nbytes = $bytes`n"
    if ($sha) { $t += "sha256 = `"$sha`"`n" }
    if ($null -ne $matched) { $t += 'matched = ' + "$matched".ToLower() + "`n" }
    [System.IO.File]::AppendAllText($env:KOYOMI_MOVED, $t, $utf8)
}
function Sha($path) {
    $s = [System.IO.File]::OpenRead($path)
    try { -join ([System.Security.Cryptography.SHA256]::Create().ComputeHash($s) | ForEach-Object { $_.ToString('x2') }) }
    finally { $s.Dispose() }
}
if ($verify) { $dir = (Get-Item -LiteralPath $into).FullName }
foreach ($x in $files) {
    if (-not $verify) {
        Copy-Item -LiteralPath $x.FullName -Destination $into -Force
        Kept $x.Name $x.Length $null $null
        Write-Output "${done}: $($x.FullName)"
        continue
    }
    $to = Join-Path $dir $x.Name
    $part = $to + '.part'
    try { [System.IO.File]::Copy($x.FullName, $part, $true) } catch { Remove-Item -LiteralPath $part -Force -ErrorAction SilentlyContinue; throw }
    $ours = Sha $x.FullName
    $theirs = Sha $part
    if ($ours -ne $theirs) {
        Remove-Item -LiteralPath $part -Force
        Kept $x.Name $x.Length $ours $false
        $why = $differs -f $x.Name, $ours, $theirs
        if ($env:KOYOMI_REASON) { [System.IO.File]::WriteAllText($env:KOYOMI_REASON, $why, $utf8) }
        Write-Output "! $why"
        exit 2
    }
    if (Test-Path -LiteralPath $to) { Remove-Item -LiteralPath $to -Force }
    [System.IO.File]::Move($part, $to)
    Kept $x.Name $x.Length $ours $true
    Write-Output "${done}: $($x.FullName) (sha256 $ours)"
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scripts_quote_their_paths_and_a_copy_is_judged_as_a_file_job() {
        // No script for the file chores (koyomi file run); a copy keeps only its judge.
        let d = Chore::Delete { path: "D:\\logs\\it's".into(), pattern: Some("*.log".into()), keep_days: 30 };
        assert_eq!(d.script(), (String::new(), None));
        assert_eq!(Chore::Empty { path: "D:\\tmp".into() }.script(), (String::new(), None));
        let c = Chore::Copy { from: "D:\\out\\".into(), into: "E:\\".into(), pattern: None };
        let (s, j) = c.script();
        assert!(s.is_empty());
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
            Chore::Send {
                from: from.into(),
                into: into.into(),
                key: None,
                host_key: host_key.clone(),
                password: None,
                key_id: None,
                region: None,
                endpoint: None,
                verify: None,
                host: None,
                bucket: None,
            }
            .check()
        };
        let fetch = |from: &str, into: &str, host_key: &Option<String>| {
            Chore::Fetch {
                from: from.into(),
                into: into.into(),
                key: None,
                host_key: host_key.clone(),
                password: None,
                key_id: None,
                region: None,
                endpoint: None,
                verify: None,
                host: None,
                bucket: None,
            }
            .check()
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
                key_id: None,
                region: None,
                endpoint: None,
                verify: None,
                host: None,
                bucket: None,
            }
            .check()
        };
        let far = "sftp://u@h/in";
        assert!(msg::CHORE_HOST_KEY_MISSING.is_in(&send(None, None, None, far).unwrap_err()));
        assert!(msg::CHORE_HOST_KEY_BAD.is_in(&send(None, Some("47DEQpj8"), None, far).unwrap_err()));
        assert_eq!(send(None, Some(&HK.to_lowercase().replacen("sha256", "SHA256", 1)), None, far), Ok(()));
        assert_eq!(send(None, Some(&format!("{HK}=")), None, far), Ok(()));
        assert_eq!(send(None, Some("{param:hk}"), None, far), Ok(()));
        assert_eq!(send(None, Some(HK), Some("secret:partner"), far), Ok(()));
        assert!(msg::CHORE_PASSWORD_SECRET.is_in(&send(None, Some(HK), Some("hunter2"), far).unwrap_err()));
        assert!(msg::CHORE_KEY_AND_PASSWORD.is_in(&send(Some("C:\\keys\\id"), Some(HK), Some("secret:partner"), far).unwrap_err()));
        assert!(msg::CHORE_ONLY_FOR.is_in(&send(None, Some(HK), None, "\\\\nas\\share\\in").unwrap_err()));
        assert!(msg::CHORE_ONLY_FOR.is_in(&send(None, None, Some("secret:p"), "D:\\in").unwrap_err()));
    }

    #[test]
    fn the_sftp_script_leaves_the_work_to_koyomi_and_judges_no_file_as_a_warning() {
        let s = Chore::Send {
            from: "D:\\out\\*.csv".into(),
            into: "sftp://u@h:2222/in box".into(),
            key: None,
            host_key: Some(HK.into()),
            password: Some("secret:partner".into()),
            key_id: None,
            region: None,
            endpoint: None,
            verify: Some(Verify::Sha256),
            host: None,
            bucket: None,
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
        assert_eq!(get("KOYOMI_SFTP_VERIFY"), Some("sha256"));
        let f = Chore::Fetch {
            from: "sftp://u@h/~/out/*.csv".into(),
            into: "D:\\in".into(),
            key: Some("C:\\keys\\id".into()),
            host_key: Some(HK.into()),
            password: None,
            key_id: None,
            region: None,
            endpoint: None,
            verify: None,
            host: None,
            bucket: None,
        };
        assert!(f.env().contains(&("KOYOMI_SFTP_OP".to_owned(), "fetch".to_owned())));
        assert!(f.env().contains(&("KOYOMI_SFTP_KEY".to_owned(), "C:\\keys\\id".to_owned())));
        assert!(!f.env().iter().any(|(k, _)| k == "KOYOMI_SFTP_VERIFY"));
        let share = Chore::Send {
            from: "D:\\out".into(),
            into: "\\\\nas\\share\\in".into(),
            key: None,
            host_key: None,
            password: None,
            key_id: None,
            region: None,
            endpoint: None,
            verify: None,
            host: None,
            bucket: None,
        };
        let (script, judge) = share.script();
        assert!(script.contains("Copy-Item -LiteralPath $x.FullName -Destination $into -Force"), "{script}");
        assert!(script.contains("$verify = $false"), "{script}");
        assert!(!script.contains("sftp"), "{script}");
        assert!(judge.is_some());
        assert!(share.env().is_empty());
        assert!(!share.own() && !share.cloud());
        // With `verify`, it copies to `.part`, compares the hashes and renames only when they match.
        let mut sure = share.clone();
        if let Chore::Send { verify, .. } = &mut sure {
            *verify = Some(Verify::Readback);
        }
        let (script, _) = sure.script();
        assert!(script.contains("$verify = $true"), "{script}");
        assert!(script.contains("$part = $to + '.part'") && script.contains("SHA256]::Create()"), "{script}");
        assert!(script.contains("exit 2") && script.contains("KOYOMI_REASON"), "{script}");
        assert!(script.contains("$differs = '") && script.contains("$why = $differs -f $x.Name, $ours, $theirs"), "{script}");
        assert!(script.contains(&msg::note(msg::N_CHORE_SHARED_DIFFERS, &["{0}", "{1}", "{2}"])), "{script}");
        assert!(sure.env().is_empty(), "the shared folder needs no variable");
    }

    #[test]
    fn verify_has_two_keys() {
        for v in Verify::ALL {
            assert_eq!(Verify::of(v.key()), Some(v));
        }
        assert_eq!(Verify::of("md5"), None);
    }

    fn cloud_send(into: &str, key_id: Option<&str>, password: Option<&str>, region: Option<&str>, endpoint: Option<&str>) -> Chore {
        Chore::Send {
            from: "D:\\out\\*.csv".into(),
            into: into.into(),
            key: None,
            host_key: None,
            password: password.map(Into::into),
            key_id: key_id.map(Into::into),
            region: region.map(Into::into),
            endpoint: endpoint.map(Into::into),
            verify: None,
            host: None,
            bucket: None,
        }
    }

    #[test]
    fn an_s3_address_has_a_bucket_and_a_key() {
        assert_eq!(s3("s3://logs-2026/daily/"), Some(Ok(S3 { bucket: "logs-2026".into(), key: "daily/".into() })));
        assert_eq!(s3(" S3://logs//a.csv "), Some(Ok(S3 { bucket: "logs".into(), key: "a.csv".into() })));
        assert_eq!(s3("s3://logs"), Some(Ok(S3 { bucket: "logs".into(), key: String::new() })));
        assert_eq!(s3("s3://{param:b}/x").unwrap().unwrap().bucket, "{param:b}");
        assert!(s3("sftp://h/in").is_none());
        assert!(s3("D:\\in").is_none());
        assert!(msg::CHORE_S3_BUCKET.is_in(&s3("s3://").unwrap().unwrap_err()));
        assert!(msg::CHORE_S3_BUCKET.is_in(&s3("s3:///in").unwrap().unwrap_err()));
        for bad in ["s3://ab/x", "s3://a b/x", "s3://a:b/x", "s3://user@bucket/x"] {
            assert!(msg::CHORE_S3_BUCKET.is_in(&s3(bad).unwrap().unwrap_err()), "{bad}");
        }
        assert!(msg::CHORE_S3_KEY_CONTROL.is_in(&s3("s3://logs/a\nb").unwrap().unwrap_err()));
    }

    #[test]
    fn s3_needs_a_key_id_a_secret_and_a_region_or_an_endpoint() {
        let ok = |c: Chore| c.check();
        let to = "s3://logs/daily/";
        let pw = Some("secret:s3");
        assert_eq!(ok(cloud_send(to, Some("AKIAEXAMPLE"), pw, Some("ap-northeast-1"), None)), Ok(()));
        assert_eq!(ok(cloud_send(to, Some("secret:s3-id"), pw, None, Some("http://nas:9000"))), Ok(()));
        assert_eq!(ok(cloud_send(to, Some("minio"), pw, Some("{param:r}"), Some("https://s3.example.com/"))), Ok(()));
        assert_eq!(ok(cloud_send("s3://logs", Some("k"), pw, Some("us-east-1"), None)), Ok(()));
        assert!(msg::CHORE_S3_KEY_ID_MISSING.is_in(&ok(cloud_send(to, None, pw, Some("us-east-1"), None)).unwrap_err()));
        assert!(msg::CHORE_S3_KEY_ID_SECRET_NAME.is_in(&ok(cloud_send(to, Some("secret:"), pw, Some("us-east-1"), None)).unwrap_err()));
        assert!(msg::CHORE_S3_KEY_ID_SPACE.is_in(&ok(cloud_send(to, Some("AK IA"), pw, Some("us-east-1"), None)).unwrap_err()));
        assert!(msg::CHORE_S3_SECRET_KEY_MISSING.is_in(&ok(cloud_send(to, Some("k"), None, Some("us-east-1"), None)).unwrap_err()));
        assert!(msg::CHORE_PASSWORD_SECRET.is_in(&ok(cloud_send(to, Some("k"), Some("hunter2"), Some("us-east-1"), None)).unwrap_err()));
        assert!(msg::CHORE_S3_REGION_MISSING.is_in(&ok(cloud_send(to, Some("k"), pw, None, None)).unwrap_err()));
        assert!(msg::CHORE_S3_REGION_BAD.is_in(&ok(cloud_send(to, Some("k"), pw, Some("Tokyo"), None)).unwrap_err()));
        assert!(ok(cloud_send(to, Some("k"), pw, None, Some("nas:9000")))
            .unwrap_err()
            .contains(&msg::line(msg::CHORE_HTTP_URL, &["endpoint", "nas:9000"])));
        for bad in ["http://", "http://nas:9000/minio", "http://u@nas", "https://nas?x=1"] {
            assert!(msg::CHORE_S3_ENDPOINT_HOST.is_in(&ok(cloud_send(to, Some("k"), pw, None, Some(bad))).unwrap_err()), "{bad}");
        }
        assert!(msg::CHORE_S3_KEY_WILDCARD.is_in(&ok(cloud_send("s3://logs/*.csv", Some("k"), pw, Some("us-east-1"), None)).unwrap_err()));
        let mut with_key = cloud_send(to, Some("k"), pw, Some("us-east-1"), None);
        if let Chore::Send { host_key, .. } = &mut with_key {
            *host_key = Some(HK.into());
        }
        assert!(msg::CHORE_S3_NOT_SFTP_KEY.is_in(&ok(with_key).unwrap_err()));
        // `key_id`, `region` and `endpoint` go only with `s3://`.
        assert!(msg::CHORE_ONLY_FOR.is_in(&ok(cloud_send("\\\\nas\\in", Some("k"), None, None, None)).unwrap_err()));
        assert!(msg::CHORE_ONLY_FOR.is_in(&ok(cloud_send("D:\\in", None, None, Some("us-east-1"), None)).unwrap_err()));
        let mut sftp_with_endpoint = cloud_send("sftp://u@h/in", None, None, None, Some("http://nas:9000"));
        if let Chore::Send { host_key, .. } = &mut sftp_with_endpoint {
            *host_key = Some(HK.into());
        }
        assert!(msg::CHORE_ONLY_FOR.is_in(&ok(sftp_with_endpoint).unwrap_err()));
        // Not from S3 into here with a `send`, nor into S3 with a `fetch`.
        let back = Chore::Send {
            from: "s3://logs/a.csv".into(),
            into: "D:\\in".into(),
            key: None,
            host_key: None,
            password: None,
            key_id: None,
            region: None,
            endpoint: None,
            verify: None,
            host: None,
            bucket: None,
        };
        assert!(msg::CHORE_LOCAL_ONLY.is_in(&back.check().unwrap_err()));
    }

    #[test]
    fn a_fetch_from_s3_names_its_keys_and_has_a_wildcard_only_in_the_name() {
        let fetch = |from: &str, into: &str| Chore::Fetch {
            from: from.into(),
            into: into.into(),
            key: None,
            host_key: None,
            password: Some("secret:s3".into()),
            key_id: Some("k".into()),
            region: Some("ap-northeast-1".into()),
            endpoint: None,
            verify: None,
            host: None,
            bucket: None,
        };
        assert_eq!(fetch("s3://logs/daily/*.csv", "D:\\in").check(), Ok(()));
        assert_eq!(fetch("s3://logs/a.csv", "D:\\in").check(), Ok(()));
        assert!(msg::CHORE_FETCH_FROM_NAME.is_in(&fetch("s3://logs/daily/", "D:\\in").check().unwrap_err()));
        assert!(msg::CHORE_FETCH_FROM_NAME.is_in(&fetch("s3://logs", "D:\\in").check().unwrap_err()));
        assert!(msg::CHORE_S3_WILDCARD_FOLDER.is_in(&fetch("s3://logs/*/a.csv", "D:\\in").check().unwrap_err()));
        assert!(msg::CHORE_LOCAL_ONLY.is_in(&fetch("s3://logs/a.csv", "s3://logs/b/").check().unwrap_err()));
        let f = fetch("s3://logs/daily/*.csv", "D:\\in");
        assert!(f.cloud() && f.own() && !f.far());
        assert_eq!(f.op(), "cloud");
        let env = f.env();
        let get = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        assert_eq!(get("KOYOMI_CLOUD_OP"), Some("fetch"));
        assert_eq!(get("KOYOMI_CLOUD_FROM"), Some("s3://logs/daily/*.csv"));
        assert_eq!(get("KOYOMI_CLOUD_INTO"), Some("D:\\in"));
        assert_eq!(get("KOYOMI_CLOUD_KEY_ID"), Some("k"));
        assert_eq!(get("KOYOMI_CLOUD_REGION"), Some("ap-northeast-1"));
        assert_eq!(get("KOYOMI_CLOUD_PASSWORD"), Some("secret:s3"));
        assert_eq!(get("KOYOMI_CLOUD_ENDPOINT"), None);
        assert_eq!(get("KOYOMI_CLOUD_VERIFY"), None);
        assert!(!env.iter().any(|(k, _)| k.starts_with("KOYOMI_SFTP_")));
        let mut sure = f.clone();
        if let Chore::Fetch { verify, .. } = &mut sure {
            *verify = Some(Verify::Sha256);
        }
        assert!(sure.env().contains(&("KOYOMI_CLOUD_VERIFY".to_owned(), "sha256".to_owned())));
    }

    #[test]
    fn fill_runs_s3_as_koyomi_cloud_run_and_judges_no_file_as_a_warning() {
        let mut net = crate::jobnet::parse(
            "format = 1\n[[job]]\nid = \"up\"\nkind = \"send\"\nfrom = 'D:\\out\\*.csv'\ninto = \"s3://logs/daily/\"\nkey_id = \"AKIAEXAMPLE\"\npassword = \"secret:s3\"\nregion = \"ap-northeast-1\"\n",
        )
        .unwrap();
        fill(&mut net, "C:\\koyomi.exe").unwrap();
        let j = &net.jobs[0];
        assert_eq!(j.run, ["C:\\koyomi.exe", "cloud", "run"]);
        assert!(j.script.is_none() && j.shell.is_none());
        assert_eq!(j.judge.as_ref().unwrap().warn, vec![Range { from: NOTHING, to: NOTHING }]);
        assert_eq!(j.env.get("KOYOMI_CLOUD_INTO").map(String::as_str), Some("s3://logs/daily/"));
        assert!(!j.env.contains_key("KOYOMI_CLOUD_ENV"));
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
        assert!(msg::CHORE_ADDR_PATH.is_in(&ssh("ssh://h/tmp").unwrap().unwrap_err()));
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
        assert!(msg::CHORE_ON_EMPTY.is_in(&ok("", "df -h").unwrap_err()));
        assert!(msg::DEF_NAMED_OWNS.is_in(&ok("web1", "df -h").unwrap_err()));
        assert!(msg::CHORE_ON_BAD.is_in(&ok("u@h", "df -h").unwrap_err()));
        assert!(msg::CHORE_ON_BAD.is_in(&ok("sftp://u@h/in", "df -h").unwrap_err()));
        assert!(msg::CHORE_ADDR_PATH.is_in(&ok("ssh://h/x", "df -h").unwrap_err()));
        assert!(msg::CHORE_COMMAND_EMPTY.is_in(&ok("ssh://u@h", " \n").unwrap_err()));
        assert_eq!(ok("ssh://u@h", &"x".repeat(COMMAND_MAX)), Ok(()));
        assert!(msg::CHORE_TEXT_TOO_LONG.is_in(&ok("ssh://u@h", &"x".repeat(COMMAND_MAX + 1)).unwrap_err()));
        let login =
            |key: Option<&str>, host_key: Option<&str>, password: Option<&str>| remote("ssh://u@h", "x", key, host_key, password).check();
        assert!(msg::CHORE_HOST_KEY_MISSING.is_in(&login(None, None, None).unwrap_err()));
        assert!(msg::CHORE_HOST_KEY_BAD.is_in(&login(None, Some("47DEQpj8"), None).unwrap_err()));
        assert_eq!(login(None, Some(HK), Some("secret:ops")), Ok(()));
        assert!(msg::CHORE_PASSWORD_SECRET.is_in(&login(None, Some(HK), Some("hunter2")).unwrap_err()));
        assert!(msg::CHORE_KEY_AND_PASSWORD.is_in(&login(Some("C:\\keys\\id"), Some(HK), Some("secret:ops")).unwrap_err()));
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
        assert!(msg::CHORE_HOST_KEY_BAD.is_in(&fanned("group:web", Some("47DEQ"), None, Pass::All).check().unwrap_err()));
        assert!(msg::CHORE_GROUP_NAME_EMPTY.is_in(&fanned("group:", None, None, Pass::All).check().unwrap_err()));
        assert!(msg::CHORE_GROUP_NAME_BAD.is_in(&fanned("group:web-1", None, None, Pass::All).check().unwrap_err()));
        assert!(msg::CHORE_MAX_RANGE.is_in(&fanned("group:web", None, Some(0), Pass::All).check().unwrap_err()));
        assert!(msg::CHORE_MAX_RANGE.is_in(&fanned("group:web", None, Some(65), Pass::All).check().unwrap_err()));
        // max and pass go with a group only; a param is told once it is filled in.
        assert!(msg::CHORE_GROUP_ONLY.is_in(&fanned("ssh://h", Some(HK), Some(2), Pass::All).check().unwrap_err()));
        assert!(msg::CHORE_GROUP_ONLY.is_in(&fanned("ssh://h", Some(HK), None, Pass::Any).check().unwrap_err()));
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
        assert!(msg::CHORE_PASSWORD_MISSING.is_in(&w(None, None, None, Pass::All).check().unwrap_err()));
        assert!(msg::CHORE_CERT_BAD.is_in(&w(Some("3F:2A"), Some("secret:ops"), None, Pass::All).check().unwrap_err()));
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
        // A copy is the file child with no shell, and still has its warning.
        assert_eq!(net.jobs[1].judge.as_ref().unwrap().warn, vec![Range { from: 1, to: 1 }]);
        assert_eq!((net.jobs[1].shell.as_deref(), net.jobs[1].script.as_deref()), (None, None));
        assert_eq!(net.jobs[1].run, ["C:\\koyomi\\koyomi.exe", "file", "run"]);
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
        assert!(msg::CHORE_WINRM_USER.is_in(&winrm("winrm://h").unwrap().unwrap_err()));
        assert!(msg::CHORE_ADDR_PATH.is_in(&winrm("winrm://u@h/wsman").unwrap().unwrap_err()));
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
        assert!(msg::CHORE_ON_EMPTY.is_in(&ok("", "ipconfig").unwrap_err()));
        assert!(msg::CHORE_ON_BAD.is_in(&ok("ops@h", "ipconfig").unwrap_err()));
        assert!(msg::CHORE_ON_BAD.is_in(&ok("ssh://ops@h", "ipconfig").unwrap_err()));
        assert!(msg::CHORE_WINRM_USER.is_in(&ok("winrm://h", "ipconfig").unwrap_err()));
        assert!(msg::CHORE_COMMAND_EMPTY.is_in(&ok("winrm://ops@h", " \r\n").unwrap_err()));
        assert!(msg::CHORE_TEXT_TOO_LONG.is_in(&ok("winrm://ops@h", &"x".repeat(COMMAND_MAX + 1)).unwrap_err()));
        let login = |cert: Option<&str>, password: Option<&str>| windows("winrm://ops@h", "x", cert, password).check();
        assert!(msg::CHORE_WINRM_CERT_MISSING.is_in(&login(None, Some("secret:ops")).unwrap_err()));
        assert!(msg::CHORE_CERT_BAD.is_in(&login(Some("3F:2A"), Some("secret:ops")).unwrap_err()));
        assert!(msg::CHORE_CERT_BAD.is_in(&login(Some(&"zz".repeat(32)), Some("secret:ops")).unwrap_err()));
        // Colons, spaces and case as one likes, or a param.
        assert_eq!(login(Some(&CERT.replace(':', "").to_lowercase()), Some("secret:ops")), Ok(()));
        assert_eq!(login(Some(&CERT.replace(':', " ")), Some("secret:ops")), Ok(()));
        assert_eq!(login(Some("{param:cert}"), Some("secret:ops")), Ok(()));
        assert!(msg::CHORE_PASSWORD_MISSING.is_in(&login(Some(CERT), None).unwrap_err()));
        assert!(msg::CHORE_PASSWORD_MISSING.is_in(&login(Some(CERT), Some(" ")).unwrap_err()));
        assert!(msg::CHORE_PASSWORD_SECRET.is_in(&login(Some(CERT), Some("hunter2")).unwrap_err()));
        assert!(msg::CHORE_PASSWORD_SECRET.is_in(&login(Some(CERT), Some("secret: ")).unwrap_err()));
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
        assert!(url("ftp://x").unwrap_err().contains(&msg::line(msg::CHORE_HTTP_URL, &["url", "ftp://x"])));
        assert_eq!(url("HTTP://x"), Ok(()));
        assert_eq!(url("{param:api}/jobs"), Ok(()));
        assert_eq!(url("{var:api}/jobs"), Ok(()));
        assert!(msg::CHORE_HTTP_METHOD.is_in(&http("get", None, &[], &[]).check().unwrap_err()));
        assert!(msg::CHORE_HTTP_BODY_METHOD.is_in(&http("HEAD", Some("a"), &[], &[]).check().unwrap_err()));
        assert!(msg::CHORE_TEXT_TOO_LONG.is_in(&http("POST", Some(&"a".repeat(BODY_MAX + 1)), &[], &[]).check().unwrap_err()));
        assert_eq!(http("POST", Some(&"あ".repeat(BODY_MAX)), &[], &[]).check(), Ok(()));
        assert!(msg::CHORE_HEADER_CONTENT_TYPE.is_in(&http("GET", None, &[("content-type", "a")], &[]).check().unwrap_err()));
        assert!(msg::CHORE_HEADER_NAME.is_in(&http("GET", None, &[("X Day", "a")], &[]).check().unwrap_err()));
        assert!(msg::CHORE_HTTP_EXPECT.is_in(&http("GET", None, &[], &[600]).check().unwrap_err()));
        let mut typed = http("GET", None, &[], &[]);
        if let Chore::Http { content_type, .. } = &mut typed {
            *content_type = Some("text/plain".into());
        }
        assert!(msg::CHORE_CONTENT_TYPE_NO_BODY.is_in(&typed.check().unwrap_err()));
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
            assert!(msg::CHORE_MAIL_ADDRESS.is_in(&mail_address(bad).unwrap_err()), "{bad}");
        }
    }

    #[test]
    fn a_mail_that_cannot_be_sent_as_written_is_refused() {
        assert_eq!(mail(&["ops@x"], "報告 {date}", Some("本文"), &["D:\\out\\*.csv"]).check(), Ok(()));
        assert!(msg::CHORE_MAIL_TO_EMPTY.is_in(&mail(&[], "s", None, &[]).check().unwrap_err()));
        assert!(msg::CHORE_MAIL_ADDRESS.is_in(&mail(&["ops"], "s", None, &[]).check().unwrap_err()));
        assert!(msg::CHORE_MAIL_SUBJECT_EMPTY.is_in(&mail(&["ops@x"], " ", None, &[]).check().unwrap_err()));
        assert!(msg::CHORE_MAIL_SUBJECT_NEWLINE.is_in(&mail(&["ops@x"], "a\r\nBcc: c@x", None, &[]).check().unwrap_err()));
        assert!(msg::CHORE_TEXT_TOO_LONG.is_in(&mail(&["ops@x"], "s", Some(&"a".repeat(BODY_MAX + 1)), &[]).check().unwrap_err()));
        assert!(msg::CHORE_FILES_MAX.is_in(&mail(&["ops@x"], "s", None, &["a"; FILES_MAX + 1]).check().unwrap_err()));
        assert!(msg::CHORE_FILES_EMPTY.is_in(&mail(&["ops@x"], "s", None, &["a", " "]).check().unwrap_err()));
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

    fn env_of<'a>(j: &'a crate::jobnet::Job, k: &str) -> Option<&'a str> {
        j.env.get(k).map(String::as_str)
    }

    #[test]
    fn fill_runs_delete_empty_and_copy_as_koyomi_file_run() {
        let mut net = crate::jobnet::parse(
            "format = 1\n[[job]]\nid = \"d\"\nkind = \"delete\"\npath = ' secret:logs '\nmatch = '*.log'\nkeep_days = 7\n\
             [[job]]\nid = \"e\"\nkind = \"empty\"\npath = 'D:\\tmp'\n\
             [[job]]\nid = \"c\"\nkind = \"copy\"\nfrom = 'D:\\out'\ninto = 'E:\\in'\n\
             [[job]]\nid = \"m\"\nkind = \"copy\"\nfrom = 'D:\\out'\ninto = 'E:\\in'\nmatch = '*.csv'\n\
             [[job]]\nid = \"z\"\nkind = \"zip\"\nfrom = 'D:\\out'\ninto = 'D:\\a.zip'\n\
             [[job]]\nid = \"s\"\nkind = \"send\"\nfrom = 'D:\\out'\ninto = '\\\\srv\\share'\n",
        )
        .unwrap();
        fill(&mut net, "C:\\koyomi\\koyomi.exe").unwrap();
        for j in &net.jobs[..4] {
            assert_eq!((j.shell.as_deref(), j.script.as_deref()), (None, None), "{}", j.id);
            assert_eq!(j.run, ["C:\\koyomi\\koyomi.exe", "file", "run"], "{}", j.id);
        }
        let d = &net.jobs[0];
        assert_eq!(env_of(d, "KOYOMI_FILE_OP"), Some("delete"));
        // A folder that starts with `secret:` is not read as a secret.
        assert_eq!(env_of(d, "KOYOMI_FILE_PATH"), Some("|secret:logs"));
        assert_eq!(env_of(d, "KOYOMI_FILE_MATCH"), Some("|*.log"));
        assert_eq!(env_of(d, "KOYOMI_FILE_KEEP_DAYS"), Some("7"));
        assert!(d.judge.is_none() && net.jobs[1].judge.is_none());
        let e = &net.jobs[1];
        assert_eq!((env_of(e, "KOYOMI_FILE_OP"), env_of(e, "KOYOMI_FILE_PATH")), (Some("empty"), Some("|D:\\tmp")));
        assert!(!e.env.contains_key("KOYOMI_FILE_MATCH") && !e.env.contains_key("KOYOMI_FILE_KEEP_DAYS"));
        let c = &net.jobs[2];
        assert_eq!(env_of(c, "KOYOMI_FILE_OP"), Some("copy"));
        assert_eq!((env_of(c, "KOYOMI_FILE_FROM"), env_of(c, "KOYOMI_FILE_INTO")), (Some("|D:\\out"), Some("|E:\\in")));
        assert!(!c.env.contains_key("KOYOMI_FILE_MATCH"));
        assert_eq!(c.judge.as_ref().unwrap().warn, vec![Range { from: 1, to: 1 }]);
        assert_eq!(env_of(&net.jobs[3], "KOYOMI_FILE_MATCH"), Some("|*.csv"));
        // A zip and a send to a shared folder are still PowerShell.
        for j in &net.jobs[4..] {
            assert_eq!(j.shell.as_deref(), Some("powershell"), "{}", j.id);
        }
    }

    #[test]
    fn the_file_chores_are_koyomis_own() {
        for kind in ["delete", "empty", "copy"] {
            let c = Chore::blank(kind).unwrap();
            assert!(c.file() && c.own() && !c.cloud() && !c.far(), "{kind}");
            assert_eq!(c.op(), "file");
        }
        for kind in ["zip", "send", "fetch", "http"] {
            let c = Chore::blank(kind).unwrap();
            assert!(!c.file() && c.op() != "file", "{kind}");
        }
    }

    #[test]
    fn folder_path_drops_a_separator_at_the_end_and_gives_a_bare_drive_its_root() {
        assert_eq!(folder_path("/"), PathBuf::from("/"));
        assert_eq!(folder_path("/a/b/"), PathBuf::from("/a/b"));
        assert_eq!(folder_path("/a/b/."), PathBuf::from("/a/b"));
        assert_eq!(folder_path("a/../b/"), PathBuf::from("a/../b"));
        assert_eq!(folder_path(""), PathBuf::new());
        if cfg!(windows) {
            assert_eq!(folder_path("D:"), PathBuf::from("D:\\"));
            assert_eq!(folder_path("D:\\"), PathBuf::from("D:\\"));
            assert_eq!(folder_path(r" D:\out\ "), PathBuf::from(r"D:\out"));
            assert_eq!(folder_path(r"D:\out\"), PathBuf::from(r"D:\out"));
            assert_eq!(folder_path(r"D:\out\."), PathBuf::from(r"D:\out"));
            assert_eq!(folder_path(r"\\?\D:"), PathBuf::from(r"\\?\D:\"));
            assert_eq!(folder_path(r"\\srv\share\"), PathBuf::from(r"\\srv\share\"));
        }
    }

    #[test]
    fn a_root_is_refused_however_dotdot_reaches_it() {
        let refused = ["/", "/a/..", "/a/../", r"C:\", "C:", r"D:\x\..", r"D:\x\..\..", r"\\srv\share", r"\\srv\share\a\..", ""];
        for p in refused {
            assert!(not_root(p).is_err(), "{p:?}");
        }
        let allowed = [r"D:\x\..\y", "/a/../b", "/a", r"D:\logs", r"\\srv\share\tmp", "logs", "a/../b"];
        for p in allowed {
            assert!(not_root(p).is_ok(), "{p:?}");
        }
    }

    fn query(db: &str, password: Option<&str>, sql: Option<&str>, file: Option<&str>) -> Chore {
        Chore::Sql {
            db: db.into(),
            database: None,
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
        assert!(msg::CHORE_DB_SCHEME.is_in(&err("mysql://db01/x")));
        assert!(msg::CHORE_DB_NAME_MISSING.is_in(&err("sqlserver://db01")));
        assert!(msg::CHORE_DB_NAME_MISSING.is_in(&err("sqlserver://db01/ ")));
        assert!(msg::CHORE_DB_NAME_BAD.is_in(&err("sqlserver://db01/a/b")));
        assert!(msg::CHORE_DB_NAME_BAD.is_in(&err("postgres://app@pg/app?sslmode=require")));
        assert!(msg::CHORE_SQL_INSTANCE.is_in(&err("sqlserver://db01\\SQLEXPRESS/x")));
        assert!(msg::CHORE_SQL_USER_WINDOWS.is_in(&err("sqlserver://CORP\\ops@db01/x")));
        assert!(msg::CHORE_PORT_BAD.is_in(&err("sqlserver://db01:0/x")));
        assert!(msg::CHORE_DB_USER_MISSING.is_in(&err("postgres://pg/app")));
        assert!(msg::CHORE_SQLITE_PATH.is_in(&err("sqlite://app.db")));
    }

    #[test]
    fn a_db_url_is_built_from_fields_and_reads_back() {
        let pg = |user: &str, host: &str, port: u16, database: &str| SqlDb::Postgres {
            user: user.into(),
            host: host.into(),
            port,
            database: database.into(),
        };
        let lite = |p: &str| SqlDb::Sqlite { path: p.into() };
        assert_eq!(db_url(&pg("u", "db.example", 5432, "d")).as_deref(), Ok("postgres://u@db.example/d"));
        assert_eq!(db_url(&pg("u", "10.0.0.5", 5433, "d")).as_deref(), Ok("postgres://u@10.0.0.5:5433/d"));
        assert_eq!(db_url(&pg("u", "[::1]", 5432, "d")).as_deref(), Ok("postgres://u@[::1]:5432/d"));
        assert_eq!(db_url(&pg("a@b", "h", 5432, "d")).as_deref(), Ok("postgres://a@b@h/d"));
        assert_eq!(db_url(&pg("u", "h", 5432, "my db")).as_deref(), Ok("postgres://u@h/my db"));
        assert_eq!(db_url(&pg("a:b", "h", 5432, "d")), Err(DbField::User));
        assert_eq!(db_url(&pg("", "h", 5432, "d")), Err(DbField::User));
        for host in ["a b", "a/b", "-x", "a:b", "", "a@b", "[x]", "[::1", "[:::]", "[1:2]", "[]", "[1.2.3.4]"] {
            assert_eq!(db_url(&pg("u", host, 5432, "d")), Err(DbField::Host), "{host}");
        }
        for database in ["a/b", "a?b", "{x}", "", "  ", "a\nb"] {
            assert_eq!(db_url(&pg("u", "h", 5432, database)), Err(DbField::Database), "{database:?}");
        }
        assert_eq!(db_url(&pg("u", "h", 0, "d")), Err(DbField::Port));
        for path in ["C:\\data\\app.db", "C:/data/app.db", "\\\\server\\share\\a.db", "/var/lib/a.db"] {
            assert_eq!(db_url(&lite(path)), Ok(format!("sqlite://{path}")), "{path}");
        }
        for path in ["data\\app.db", "{x}.db", "", "C:\\a\nb.db", "C:\\a.db "] {
            assert_eq!(db_url(&lite(path)), Err(DbField::File), "{path:?}");
        }
        let server = SqlDb::SqlServer { user: None, host: "h".into(), port: 1433, database: "d".into() };
        assert_eq!(db_url(&server), Err(DbField::Host));
    }

    /// 143.41: a 接続先 PC address is built from its parts and read back as the same parts.
    #[test]
    fn a_host_url_is_built_from_parts_and_reads_back() {
        assert_eq!(host_url(false, "u", "web1", 22).as_deref(), Ok("ssh://u@web1"));
        assert_eq!(host_url(false, "", "web1", 22).as_deref(), Ok("ssh://web1"));
        assert_eq!(host_url(false, "u", "web1", 2222).as_deref(), Ok("ssh://u@web1:2222"));
        assert_eq!(host_url(true, "CORP\\ops", "win1", WINRM_PORT).as_deref(), Ok("winrm://CORP\\ops@win1"));
        assert_eq!(host_url(true, ".\\ops", "win1", 5985).as_deref(), Ok("winrm://.\\ops@win1:5985"));
        assert_eq!(host_url(true, "a@b", "win1", 5986).as_deref(), Ok("winrm://a@b@win1"));
        assert_eq!(host_url(false, "u", "::1", 22).as_deref(), Ok("ssh://u@[::1]:22"));
        assert_eq!(host_url(false, "u", "[::1]", 22).as_deref(), Ok("ssh://u@[::1]:22"));
        assert_eq!(host_url(false, "u", "[fe80::1]", 2200).as_deref(), Ok("ssh://u@[fe80::1]:2200"));
        for (winrm_kind, user) in [(false, "u"), (true, "CORP\\u")] {
            let url = host_url(winrm_kind, user, "h", 1234).unwrap();
            assert!(!url.contains(['{', '}']) && !has_password(&url), "{url}");
        }
        assert_eq!(host_url(true, "", "h", 5986), Err(HostField::User));
        for user in ["a:b", "a b", "a/b", "a{b", "a\nb"] {
            assert_eq!(host_url(false, user, "h", 22), Err(HostField::User), "{user:?}");
        }
        for host in ["a@b", "a b", "", "-x", "a:b", "a/b", "[x]", "[::1", "h\\x", "a{b"] {
            assert_eq!(host_url(false, "u", host, 22), Err(HostField::Host), "{host:?}");
        }
        assert_eq!(host_url(false, "u", "h", 0), Err(HostField::Port));
    }

    /// 143.42: an S3 address is built from the bucket and the folder, refusing the field at fault.
    #[test]
    fn a_bucket_url_is_built_from_parts_and_reads_back() {
        assert_eq!(bucket_url("my-bucket", "").as_deref(), Ok("s3://my-bucket/"));
        assert_eq!(bucket_url("my-bucket", "reports").as_deref(), Ok("s3://my-bucket/reports/"));
        assert_eq!(bucket_url("my-bucket", " /reports/daily/ ").as_deref(), Ok("s3://my-bucket/reports/daily/"));
        assert_eq!(bucket_url("my-bucket", "a b/c").as_deref(), Ok("s3://my-bucket/a b/c/"));
        assert_eq!(bucket_url("ab", ""), Err(BucketField::Bucket));
        assert_eq!(bucket_url("a{b}c", ""), Err(BucketField::Bucket));
        assert_eq!(bucket_url("a b c", ""), Err(BucketField::Bucket));
        for folder in ["a//b", "a*", "a?", "a\nb", "{x}"] {
            assert_eq!(bucket_url("my-bucket", folder), Err(BucketField::Folder), "{folder:?}");
        }
        let url = bucket_url("my-bucket", "a b").unwrap();
        assert!(!url.contains(['{', '}']) && !has_password(&url));
    }

    #[test]
    fn sql_rows_are_ranges_of_counts() {
        assert_eq!(sql_rows("0"), Ok(vec![(0, 0)]));
        assert_eq!(sql_rows(" 1.. "), Ok(vec![(1, u64::MAX)]));
        assert_eq!(sql_rows("1..100, ..0"), Ok(vec![(1, 100), (0, 0)]));
        for bad in ["", "x", "-1", "5..1", "1..=5", "1,,2"] {
            assert!(msg::CHORE_ROWS.is_in(&sql_rows(bad).unwrap_err()), "{bad}");
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
        assert!(msg::CHORE_SQL_DB_EMPTY.is_in(&err(query(" ", None, Some("select 1"), None))));
        assert!(msg::CHORE_DB_SCHEME.is_in(&err(query("mysql://db01/x", None, Some("select 1"), None))));
        assert!(msg::CHORE_SQL_WINDOWS_PASSWORD.is_in(&err(query("sqlserver://db01/x", Some("secret:db"), Some("select 1"), None))));
        assert!(msg::CHORE_PASSWORD_MISSING.is_in(&err(query("sqlserver://ops@db01/x", None, Some("select 1"), None))));
        assert!(msg::CHORE_PASSWORD_MISSING.is_in(&err(query("postgres://app@pg/app", None, Some("select 1"), None))));
        assert!(msg::CHORE_PASSWORD_SECRET.is_in(&err(query("postgres://app@pg/app", Some("hunter2"), Some("select 1"), None))));
        assert!(msg::CHORE_PASSWORD_SECRET.is_in(&err(query("postgres://app@pg/app", Some("secret: "), Some("select 1"), None))));
        assert!(msg::CHORE_SQLITE_NO.is_in(&err(query("sqlite://C:\\a.db", Some("secret:x"), Some("select 1"), None))));
        assert!(msg::CHORE_SQL_ONE_OF.is_in(&err(query("sqlite://C:\\a.db", None, None, None))));
        assert!(msg::CHORE_SQL_ONE_OF.is_in(&err(query("sqlite://C:\\a.db", None, Some("select 1"), Some("D:\\a.sql")))));
        assert!(err(query("sqlite://C:\\a.db", None, Some(" \n"), None)).contains(&msg::line(msg::CHORE_KEY_EMPTY, &["sql"])));
        assert!(err(query("sqlite://C:\\a.db", None, None, Some(" "))).contains(&msg::line(msg::CHORE_KEY_EMPTY, &["file"])));
        assert!(msg::CHORE_SQL_TOO_LONG.is_in(&err(query("sqlite://C:\\a.db", None, Some(&"x".repeat(COMMAND_MAX + 1)), None))));
        let with = |db: &str, password: Option<&str>, tls: SqlTls, out: Option<&str>, rows: Option<&str>| {
            let mut c = query(db, password, Some("select 1"), None);
            if let Chore::Sql { tls: t, out: o, rows: r, .. } = &mut c {
                (*t, *o, *r) = (tls, out.map(Into::into), rows.map(Into::into));
            }
            c
        };
        assert!(msg::CHORE_SQL_TLS_OFF.is_in(&err(with("sqlserver://db01/x", None, SqlTls::Off, None, None))));
        ok(with("sqlserver://db01/x", None, SqlTls::Trust, None, None));
        assert!(msg::CHORE_SQLITE_NO.is_in(&err(with("sqlite://C:\\a.db", None, SqlTls::Trust, None, None))));
        ok(with("postgres://app@pg/app", Some("secret:pg"), SqlTls::Off, Some("D:\\out\\a.csv"), Some("1..")));
        assert!(msg::CHORE_ROWS.is_in(&err(with("sqlite://C:\\a.db", None, SqlTls::Verify, None, Some("x")))));
        assert!(err(with("sqlite://C:\\a.db", None, SqlTls::Verify, Some(" "), None)).contains(&msg::line(msg::CHORE_KEY_EMPTY, &["out"])));
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

    fn send_to(host: Option<&str>, bucket: Option<&str>, into: &str) -> Chore {
        Chore::Send {
            from: "D:\\out\\*.csv".into(),
            into: into.into(),
            key: None,
            host_key: None,
            password: None,
            key_id: None,
            region: None,
            endpoint: None,
            verify: None,
            host: host.map(Into::into),
            bucket: bucket.map(Into::into),
        }
    }

    fn fetch_from(host: Option<&str>, bucket: Option<&str>, from: &str) -> Chore {
        Chore::Fetch {
            from: from.into(),
            into: "D:\\in".into(),
            key: None,
            host_key: None,
            password: None,
            key_id: None,
            region: None,
            endpoint: None,
            verify: None,
            host: host.map(Into::into),
            bucket: bucket.map(Into::into),
        }
    }

    #[test]
    fn a_bare_name_is_a_host_and_an_address_a_group_or_a_template_is_not() {
        assert_eq!(host_name("web1"), Some("web1"));
        assert_eq!(host_name("  web1 "), Some("web1"));
        assert_eq!(host_name("売上 サーバー"), Some("売上 サーバー"));
        for not in ["", "ssh://u@h", "winrm://u@h", "h:22", "{web}", "{date}", "group:web", "admin@web1"] {
            assert_eq!(host_name(not), None, "{not:?}");
        }
        // An address written without its scheme is told as one that does not read, not as an unknown host.
        let job = Chore::Ssh {
            on: "admin@web1".into(),
            command: "uptime".into(),
            key: None,
            host_key: Some("SHA256:x".into()),
            password: None,
            max: None,
            pass: Pass::All,
        };
        assert!(msg::CHORE_ON_BAD.is_in(&job.check().unwrap_err()));
    }

    #[test]
    fn the_sftp_address_of_a_host_and_a_path_reads_back_as_sftp() {
        for (on, path, want) in [
            ("ssh://web1", "/in/a.csv", "sftp://web1/in/a.csv"),
            ("ssh://ops@web1", "/in", "sftp://ops@web1/in"),
            ("ssh://ops@web1:2222", "in/a.csv", "sftp://ops@web1:2222/~/in/a.csv"),
            ("ssh://ops@web1", "~/in", "sftp://ops@web1/~/in"),
            ("ssh://ops@web1", "", "sftp://ops@web1/~"),
            ("ssh://ops@web1", "~", "sftp://ops@web1/~"),
            ("ssh://ops@[::1]:22", "x.csv", "sftp://ops@[::1]:22/~/x.csv"),
        ] {
            let url = sftp_join(on, path).unwrap();
            assert_eq!(url, want, "{on} {path:?}");
            // It reads as the address written whole would.
            assert!(sftp(&url).unwrap().is_ok(), "{url}");
        }
        let at = sftp(&sftp_join("ssh://ops@web1:2222", "in/a.csv").unwrap()).unwrap().unwrap();
        assert_eq!(at, Sftp { user: Some("ops".into()), host: "web1".into(), port: Some(2222), path: "in/a.csv".into() });
        assert!(sftp_join("winrm://ops@web1", "x").is_err());
    }

    #[test]
    fn the_s3_address_of_a_bucket_and_a_path_keeps_the_folder_and_refuses_a_slash() {
        assert_eq!(s3_join("s3://bkt", "a/b.csv").unwrap(), "s3://bkt/a/b.csv");
        assert_eq!(s3_join("s3://bkt/app/", "a.csv").unwrap(), "s3://bkt/app/a.csv");
        assert_eq!(s3_join("s3://bkt/app", "a.csv").unwrap(), "s3://bkt/app/a.csv");
        assert!(msg::DEF_BUCKET_PATH_SLASH.is_in(&s3_join("s3://bkt/app/", "/a.csv").unwrap_err()));
        assert!(s3_join("sftp://h/x", "a").is_err());
        let at = s3(&s3_join("s3://bkt/app/", "x/*.csv").unwrap()).unwrap().unwrap();
        assert_eq!(at.key, "app/x/*.csv");
    }

    #[test]
    fn a_send_or_fetch_with_a_host_or_bucket_is_checked_as_the_path_it_would_be() {
        assert!(send_to(Some("web1"), None, "/in").check().is_ok());
        assert!(send_to(None, Some("logs"), "out/a.csv").check().is_ok());
        assert!(fetch_from(Some("web1"), None, "in/*.csv").check().is_ok());
        assert!(fetch_from(None, Some("logs"), "in/a.csv").check().is_ok());
        let bad = |c: Chore| c.check().unwrap_err();
        assert!(msg::DEF_HOST_AND_BUCKET.is_in(&bad(send_to(Some("web1"), Some("logs"), "x"))));
        assert!(msg::DEF_NAMED_FAR_URL.is_in(&bad(send_to(Some("web1"), None, "sftp://h/x"))));
        assert!(msg::DEF_BUCKET_PATH_SLASH.is_in(&bad(send_to(None, Some("logs"), "/x"))));
        assert!(msg::DEF_REGISTERED_NAME_BAD.is_in(&bad(send_to(Some("a/b"), None, "x"))));
        // The far-path rules hold after the join: an S3 key to send has no wildcard, one to fetch names a file.
        assert!(msg::CHORE_S3_KEY_WILDCARD.is_in(&bad(send_to(None, Some("logs"), "out/*.csv"))));
        assert!(msg::CHORE_FETCH_FROM_NAME.is_in(&bad(fetch_from(None, Some("logs"), "in/"))));
        assert!(msg::CHORE_FETCH_FROM_NAME.is_in(&bad(fetch_from(Some("web1"), None, "in/"))));
        // The entry owns the login: a key written next to the name is refused.
        let mut c = send_to(Some("web1"), None, "/in");
        if let Chore::Send { key, .. } = &mut c {
            *key = Some("C:/k".into());
        }
        assert!(msg::DEF_NAMED_OWNS.is_in(&bad(c)));
        // A name for an ssh and a winrm, and what the entry owns.
        let ssh = |on: &str, key: Option<&str>| Chore::Ssh {
            on: on.into(),
            command: "uptime".into(),
            key: key.map(Into::into),
            host_key: None,
            password: None,
            max: None,
            pass: Pass::All,
        };
        assert!(ssh("web1", None).check().is_ok());
        assert!(msg::DEF_NAMED_OWNS.is_in(&bad(ssh("web1", Some("C:/k")))));
        assert!(msg::DEF_REGISTERED_NAME_BAD.is_in(&bad(ssh("a b/c", None))) || host_name("a b/c").is_none());
    }

    #[test]
    fn the_script_env_and_paths_of_a_named_send_or_fetch_carry_the_names_and_the_path_only() {
        let s = send_to(Some("web1"), None, "in/a.csv");
        let (script, _) = s.script();
        assert!(script.contains("sftp run"), "{script}");
        let env = s.env();
        let get = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        assert_eq!(get("KOYOMI_SFTP_OP"), Some("send"));
        // The far side is the path as written; the address is built when the job starts.
        assert_eq!(get("KOYOMI_SFTP_INTO"), Some("in/a.csv"));
        assert_eq!(get("KOYOMI_SFTP_PASSWORD"), None);
        let b = fetch_from(None, Some("logs"), "in/a.csv");
        let (script, _) = b.script();
        assert!(!script.contains("sftp run"), "{script}");
        let env = b.env();
        let get = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        assert_eq!(get("KOYOMI_CLOUD_OP"), Some("fetch"));
        assert_eq!(get("KOYOMI_CLOUD_FROM"), Some("in/a.csv"));
        // `paths()` holds the templates only: the far path is kept, the names are not.
        assert!(s.paths().iter().any(|p| p.as_str() == "in/a.csv"));
        assert!(!s.paths().iter().any(|p| p.as_str() == "web1"));
        assert!(!b.paths().iter().any(|p| p.as_str() == "logs"));
    }

    #[test]
    fn fill_runs_a_named_host_bucket_or_ssh_as_koyomi_with_the_names_recorded() {
        let mut net = crate::jobnet::parse(
            "format = 1\n[[job]]\nid = \"s\"\nkind = \"send\"\nfrom = 'D:/out/*.csv'\ninto = \"in/{date}\"\nhost = \"web1\"\n[[job]]\nid = \"f\"\nkind = \"fetch\"\nfrom = \"in/a.csv\"\ninto = 'D:/in'\nbucket = \"logs\"\n[[job]]\nid = \"x\"\nkind = \"ssh\"\non = \"web1\"\ncommand = \"uptime\"\n",
        )
        .unwrap();
        fill(&mut net, "C:/koyomi.exe").unwrap();
        assert!(net.jobs[0].script.as_deref().is_some_and(|s| s.contains("sftp run")), "{:?}", net.jobs[0]);
        assert_eq!(net.jobs[1].run, ["C:/koyomi.exe", "cloud", "run"]);
        assert_eq!(net.jobs[2].run, ["C:/koyomi.exe", "ssh", "run"]);
        // The environment holds the path as written (the address is built when the job starts), and no login.
        assert_eq!(net.jobs[0].env.get("KOYOMI_SFTP_INTO").map(String::as_str), Some("in/{date}"));
        assert_eq!(net.jobs[1].env.get("KOYOMI_CLOUD_FROM").map(String::as_str), Some("in/a.csv"));
        assert_eq!(net.jobs[2].env.get("KOYOMI_SSH_ON").map(String::as_str), Some("web1"));
        assert!(!net.jobs[0].env.contains_key("KOYOMI_SFTP_PASSWORD") && !net.jobs[1].env.contains_key("KOYOMI_CLOUD_PASSWORD"));
    }

    const PW: &str = "Hunter2zz";

    #[test]
    fn password_span_finds_what_is_written_after_the_user_and_colon() {
        let some = [
            ("sftp://u:pw@h/x", "pw"),
            ("sftp://u:pw@h:22/x", "pw"),
            ("postgres://u:p@ss@h/db", "p@ss"),
            ("sftp://:pw@h", "pw"),
            ("sftp://u:@h", ""),
            ("imaps://me@example.com:pw@imap.example.com/INBOX", "pw"),
            ("ssh://u[:pw]@h:22", "pw]"),
            ("imaps://u{:pw}@h", "pw}"),
            ("SFTP://u:pw@h", "pw"),
            ("  sftp://u:pw@h  ", "pw"),
            ("winrm://CORP\\u:pw@h", "pw"),
            ("sftp://{param:user}:{param:pw}@h", "{param:pw}"),
        ];
        for (addr, want) in some {
            let r = password_span(addr).unwrap_or_else(|| panic!("{addr}"));
            assert_eq!(&addr[r], want, "{addr}");
            assert!(has_password(addr), "{addr}");
        }
        let none = [
            "sftp://u@h:22/x",
            "sftp://h:22/in@box",
            "sftp://h:22/a@b:c",
            "sftp://h:{param:port}/a@b",
            "sftp://{param:host}:22/a@b:c",
            "imaps://me@h:993/INBOX/a@b",
            "sftp://h/a:b@c",
            "sftp://{param:user}@h",
            "sftp://u@[fe80::1]:22/x",
            "sftp://[::1]:22/a@b",
            "sqlite://C:\\data\\a@b.db",
            "C:\\data\\a:b@c",
            "imaps://me@host",
            "winrm://CORP\\ops@app01:5986",
            "winrm://.\\ops@app01",
            "s3://bkt/a:b@c",
        ];
        for addr in none {
            assert_eq!(password_span(addr), None, "{addr}");
            assert!(!has_password(addr), "{addr}");
        }
    }

    #[test]
    fn hide_password_and_shown_address_hide_exactly_the_password() {
        for (addr, want) in [
            ("sftp://u:pw@h/x", "sftp://u:***@h/x"),
            ("postgres://u:p@ss@h/db", "postgres://u:***@h/db"),
            ("  sftp://u:pw@h  ", "  sftp://u:***@h  "),
            ("sftp://{param:user}:{param:pw}@h", "sftp://{param:user}:***@h"),
            ("sftp://u:ab/cd@h/x", "sftp://u:***@h/x"),
            ("imaps://me:pw@h/a@b", "imaps://me:***@h/a@b"),
            ("u:pw@h", "u:***@h"),
            ("sftp://h:22/in@box", "sftp://h:***@box"),
        ] {
            assert_eq!(hide_password(addr), want, "{addr}");
        }
        for addr in [
            "sftp://u@h:22/x",
            "sftp://{param:user}@h",
            "sftp://u@[fe80::1]:22/x",
            "imaps://me@host",
            "winrm://CORP\\ops@app01:5986",
            "winrm://.\\ops@app01",
            "sqlite://C:\\data\\a:b@c.db",
            "sftp://h",
            "",
        ] {
            assert!(matches!(hide_password(addr), std::borrow::Cow::Borrowed(_)), "{addr}");
            assert_eq!(hide_password(addr), addr);
        }
        assert_eq!(shown_address("ssh://u@h:22"), "ssh://u@h:22");
        assert_eq!(shown_address("ssh://h:22/in@box"), "ssh://h:22/in@box");
        assert_eq!(shown_address("ssh://u:Hunter2zz@h:22"), "ssh://u:***@h:22");
    }

    /// 82.9: an address with a password written in it is refused and shown with `***`.
    #[test]
    fn an_address_with_a_password_is_refused_and_never_shown() {
        for a in [
            "postgres://u:Hunter2zz@h/db",
            "postgres://u:Hunter2zz@h:5432/db",
            "postgresql://u:Hunter2zz@h/db",
            "sqlserver://u:Hunter2zz@h/db",
            "sqlserver://u:Hunter2zz@h:1433/db",
            "mysql://u:Hunter2zz@h/db",
        ] {
            let e = sql_db(a).unwrap_err();
            assert!(msg::ADDR_PASSWORD.is_in(&e) && !e.contains(PW), "{a}: {e}");
        }
        for e in [
            sftp("sftp://u:Hunter2zz@h/x").unwrap().unwrap_err(),
            sftp("sftp://u:Hunter2zz@h:22/x").unwrap().unwrap_err(),
            ssh("ssh://u:Hunter2zz@h").unwrap().unwrap_err(),
            ssh("ssh://u:Hunter2zz@h:22").unwrap().unwrap_err(),
            winrm("winrm://CORP\\u:Hunter2zz@h").unwrap().unwrap_err(),
            crate::when::mailbox("imaps://me:Hunter2zz@h").unwrap_err(),
            crate::when::mailbox("imaps://me@example.com:Hunter2zz@imap.example.com/INBOX").unwrap_err(),
        ] {
            assert!(msg::ADDR_PASSWORD.is_in(&e) && !e.contains(PW), "{e}");
        }
        assert!(sftp("sftp://u:Hunter2zz@h/x").unwrap().unwrap_err().contains("sftp://u:***@h/x"));
    }

    #[test]
    fn an_unreadable_address_still_hides_a_password_it_could_not_split() {
        for e in [
            sftp("sftp://u:Hunter2zz/x@h/p").unwrap().unwrap_err(),
            sql_db("postgres://u:Hunter2zz/x@h/db").unwrap_err(),
            sql_db("sqlserver://u:Hunter2zz/x@h/db").unwrap_err(),
            sql_db("nosql://u:Hunter2zz/x@h/db").unwrap_err(),
            crate::when::mailbox("imaps://me:Hunter2zz/x@h").unwrap_err(),
            sftp_join("ssh://u:Hunter2zz/x@h", "a").unwrap_err(),
            ssh("ssh://u:Hunter2zz/x@h").unwrap().unwrap_err(),
        ] {
            assert!(!e.contains(PW), "{e}");
        }
    }

    #[test]
    fn a_password_in_a_malformed_authority_is_refused_by_the_strict_rule() {
        for e in [sftp("sftp://h:Hunter2zz@x/p").unwrap().unwrap_err(), sftp("sftp://u:Hunter2zz@h:x/p").unwrap().unwrap_err()] {
            assert!(msg::ADDR_PASSWORD.is_in(&e) && !e.contains(PW), "{e}");
        }
    }

    #[test]
    fn addresses_without_a_password_are_read_as_before() {
        assert_eq!(crate::when::mailbox("imaps://me@example.com@imap.example.com/INBOX").unwrap().user, "me@example.com");
        assert_eq!(winrm("winrm://CORP\\ops@app01:5986").unwrap().unwrap().user.as_deref(), Some("CORP\\ops"));
        assert_eq!(winrm("winrm://.\\ops@app01").unwrap().unwrap().user.as_deref(), Some(".\\ops"));
        assert_eq!(sftp(" SFTP://u@h:22/x ").unwrap().unwrap().port, Some(22));
        assert_eq!(sql_db("sqlite://C:\\data\\a:b@c.db").unwrap(), SqlDb::Sqlite { path: "C:\\data\\a:b@c.db".into() });
        assert!(sql_db("POSTGRES://u@h:5432/db").is_ok());
        let t = sftp("sftp://{param:user}@{param:host}:{param:port}/x").unwrap().unwrap();
        assert_eq!((t.user.as_deref(), t.host.as_str()), (Some("{param:user}"), "{param:host}:{param:port}"));
        assert_eq!(t.path, "/x");
        let v6 = sftp("sftp://u@[fe80::1]:22/x").unwrap().unwrap();
        assert_eq!((v6.host.as_str(), v6.port), ("[fe80::1]", Some(22)));
        assert!(s3("s3://bkt/a:b@c").unwrap().is_ok());
        assert_eq!(sql_db("sqlite:///var/a:b@c.db").unwrap(), SqlDb::Sqlite { path: "/var/a:b@c.db".into() });
        assert_eq!(sql_db("sqlite://C:/data/a@b.db").unwrap(), SqlDb::Sqlite { path: "C:/data/a@b.db".into() });
        for a in ["sqlite://C:/data/a@b.db", "sqlite:///var/a:b@c.db"] {
            assert!(matches!(hide_password(a), std::borrow::Cow::Borrowed(_)), "{a}");
        }
    }

    /// A `sqlite://` text that is not a file path, an `s3://` bucket and a zip's `into` never show a password.
    #[test]
    fn a_refused_sqlite_s3_or_zip_address_never_shows_a_password() {
        let e = sql_db("sqlite://u:Hunter2zz@h/db").unwrap_err();
        assert!(msg::CHORE_SQLITE_PATH.is_in(&e) && !e.contains(PW) && e.contains("***"), "{e}");
        let e = s3("s3://me:Hunter2zz@bkt/x").unwrap().unwrap_err();
        assert!(msg::CHORE_S3_BUCKET.is_in(&e) && !e.contains(PW) && e.contains("***"), "{e}");
        let e = s3_join("s3://me:Hunter2zz@bkt", "x").unwrap_err();
        assert!(!e.contains(PW), "{e}");
        assert_eq!(hide_password("sqlite://u:Hunter2zz@h/db"), "sqlite://u:***@h/db");
        let z = Chore::Zip { from: "C:\\a".into(), into: "sftp://u:Hunter2zz@h/x".into() };
        let e = z.check().unwrap_err();
        assert!(msg::CHORE_ZIP_INTO.is_in(&e) && !e.contains(PW) && e.contains("***"), "{e}");
    }
}
