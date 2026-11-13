//! The アカウント, データベース, 接続先 PC, S3 and メールボックス pages of the settings: the engine's `[[account]]`,
//! `[[database]]`, `[[host]]`, `[[bucket]]` and `[[mailbox]]` of koyomi.toml, listed, added, changed and removed
//! through the door's `Registry` and the `…Put` and `…Remove` requests of each.
//!
//! A job names an entry (`run_as = "batch"`, `database = "sales"`, `on = "web1"`), so the secret's name and the
//! address are written once. The window never handles a stored secret's value; a password (an S3 bucket's secret
//! access key too) is typed in the form (a [`Hidden`] value, never shown or put in a message) and sent once for the
//! engine to store under a name it picks.
//!
//! The state is plain data, so the rules are tested without a window (as on the users page): what the page
//! may do ([`super::users::access`]), how a listing is taken, what an answer does ([`Registry::answered`])
//! and what a click asks for ([`Registry::act`]). The drawing is [`show`].

use koyomi_core::chore::{self, SqlDb, SqlTls, WinrmAuth};
use koyomi_core::registry::{BucketEntry, HostEntry, MailboxEntry};
use koyomi_core::{jobnet, msg, registry};

use super::users::{self, Access, Under};
use crate::proto::{peek_target_ok, AccountRow, BucketRow, DatabaseRow, Hidden, HostRow, MailboxRow, PeekKind, RegistryBody, Request};

/// The pages share one listing (koyomi.toml is read once).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Accounts,
    Databases,
    Hosts,
    Buckets,
    Mailboxes,
}

impl Page {
    /// How many pages there are (the size of [`Registry::forms`]).
    const COUNT: usize = 5;

    fn idx(self) -> usize {
        match self {
            Page::Accounts => 0,
            Page::Databases => 1,
            Page::Hosts => 2,
            Page::Buckets => 3,
            Page::Mailboxes => 4,
        }
    }

    /// Part of a test ID.
    fn tag(self) -> &'static str {
        match self {
            Page::Accounts => "accounts",
            Page::Databases => "databases",
            Page::Hosts => "hosts",
            Page::Buckets => "buckets",
            Page::Mailboxes => "mailboxes",
        }
    }

    /// The table of koyomi.toml, as the engine words it.
    fn table(self) -> &'static str {
        match self {
            Page::Accounts => "[[account]]",
            Page::Databases => "[[database]]",
            Page::Hosts => "[[host]]",
            Page::Buckets => "[[bucket]]",
            Page::Mailboxes => "[[mailbox]]",
        }
    }

    /// The keys of the words of this page: heading (this PC), heading (another PC), add, none, remove title, body,
    /// body when nets use it.
    fn words(self) -> [&'static str; 7] {
        match self {
            Page::Accounts => [
                "registry.accounts_this",
                "registry.accounts_far",
                "registry.add_account",
                "registry.none_accounts",
                "registry.remove_title_account",
                "registry.remove_body_account",
                "registry.remove_used_account",
            ],
            Page::Databases => [
                "registry.databases_this",
                "registry.databases_far",
                "registry.add_database",
                "registry.none_databases",
                "registry.remove_title_database",
                "registry.remove_body_database",
                "registry.remove_used_database",
            ],
            Page::Hosts => [
                "registry.hosts_this",
                "registry.hosts_far",
                "registry.add_host",
                "registry.none_hosts",
                "registry.remove_title_host",
                "registry.remove_body_host",
                "registry.remove_used_host",
            ],
            Page::Buckets => [
                "registry.buckets_this",
                "registry.buckets_far",
                "registry.add_bucket",
                "registry.none_buckets",
                "registry.remove_title_bucket",
                "registry.remove_body_bucket",
                "registry.remove_used_bucket",
            ],
            Page::Mailboxes => [
                "registry.mailboxes_this",
                "registry.mailboxes_far",
                "registry.add_mailbox",
                "registry.none_mailboxes",
                "registry.remove_title_mailbox",
                "registry.remove_body_mailbox",
                "registry.remove_used_mailbox",
            ],
        }
    }
}

/// The scheme of a host's address, which the kind seg supplies.
fn scheme(winrm: bool) -> &'static str {
    if winrm {
        "winrm://"
    } else {
        "ssh://"
    }
}

/// A stored `on` as the host form shows it: whether it is a WinRM address (`None` when it is neither), and the rest
/// after the scheme, VERBATIM. The scheme is found whatever its case; nothing is split further, so rebuilding is
/// the seg's scheme in front of the rest and nothing else.
pub(super) fn split_on(on: &str) -> (Option<bool>, String) {
    let t = on.trim();
    if chore::ssh(t).is_some() {
        (Some(false), t[scheme(false).len()..].to_owned())
    } else if chore::winrm(t).is_some() {
        (Some(true), t[scheme(true).len()..].to_owned())
    } else {
        (None, t.to_owned())
    }
}

/// A line under the list, kept as what it is so it is worded when it is drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Line {
    /// The engine's own words, or a coded line, as they came.
    Raw(String),
    /// koyomi.toml was changed since the listing was read.
    BaseChanged,
    /// The engine said yes, but its reply could not be read.
    ReplyBad,
    /// A request the thread did not take: why.
    Unsent(String),
    /// The name is already registered (adding).
    NameExists(String),
    /// The password field is empty where one is needed.
    PasswordEmpty,
    /// A refusal worded by a key of the window's own words (a database field that does not read).
    Key(&'static str),
}

impl Line {
    pub(super) fn text(&self, t: &ito_i18n::Table) -> String {
        match self {
            Line::Raw(s) | Line::Unsent(s) => s.clone(),
            Line::Key(k) => t.tr(k).to_owned(),
            Line::BaseChanged => t.tr("registry.base_changed").to_owned(),
            Line::ReplyBad => t.tr("registry.reply_bad").to_owned(),
            Line::NameExists(n) => t.trf("registry.name_exists", &[n]),
            Line::PasswordEmpty => t.tr("registry.password_empty").to_owned(),
        }
    }
}

/// The kind of database the form writes (SQL Server is not offered: its rows are fixed in koyomi.toml).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum DbKind {
    #[default]
    Postgres,
    Sqlite,
}

/// What the database form holds, field by field (its password is [`Form::password`], like every page's).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct DbFields {
    pub kind: DbKind,
    pub host: String,
    /// As typed: empty is the PostgreSQL default.
    pub port: String,
    pub database: String,
    pub user: String,
    /// SQLite: the file, by its whole path.
    pub file: String,
}

impl DbFields {
    /// A new PostgreSQL entry: the usual port is filled in.
    fn fresh() -> DbFields {
        DbFields { port: chore::POSTGRES_PORT.to_string(), ..DbFields::default() }
    }
}

/// The port as typed: empty is the PostgreSQL default; otherwise digits from 1 to 65535.
fn port_of(text: &str) -> Option<u16> {
    port_or(text, chore::POSTGRES_PORT)
}

/// The port as typed: empty is `default`; otherwise digits from 1 to 65535.
fn port_or(text: &str, default: u16) -> Option<u16> {
    let t = text.trim();
    if t.is_empty() {
        return Some(default);
    }
    if !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse::<u16>().ok().filter(|p| *p > 0)
}

/// What the 接続先 PC, メールボックス and S3 forms hold in parts (the address is built from them, as the database
/// form's `db` is). Each page uses the fields it shows: a host `user`, `host`, `port`; a mailbox those and `folder`; a
/// bucket `bucket` and `folder`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct AddrFields {
    pub user: String,
    pub host: String,
    /// As typed: empty is the kind's own port.
    pub port: String,
    /// A mailbox's folder (empty is `INBOX`), a bucket's folder (without the ending `/`).
    pub folder: String,
    pub bucket: String,
}

/// The port a host of this kind, or a mailbox, takes when none is written.
fn default_port(page: Page, winrm: bool) -> u16 {
    match (page, winrm) {
        (Page::Mailboxes, _) => 993,
        (_, true) => chore::WINRM_PORT,
        _ => chore::SSH_PORT,
    }
}

/// The address `text` read as the parts `page`'s form shows, when it reads and the builder takes those parts (else the
/// form shows the one line).
fn read_addr(page: Page, text: &str) -> Option<AddrFields> {
    let t = text.trim();
    let a = match page {
        Page::Hosts => {
            let (kind, _) = split_on(t);
            let winrm = kind?;
            let far = (if winrm { chore::winrm(t) } else { chore::ssh(t) })?.ok()?;
            let port = far.port.unwrap_or(default_port(page, winrm)).to_string();
            AddrFields { user: far.user.unwrap_or_default(), host: far.host, port, ..AddrFields::default() }
        }
        Page::Mailboxes => {
            let m = koyomi_core::when::mailbox(t).ok()?;
            AddrFields { user: m.user, host: m.host, port: m.port.to_string(), folder: m.folder, ..AddrFields::default() }
        }
        Page::Buckets => {
            let at = chore::s3(t)?.ok()?;
            let prefix = registry::bucket_prefix(t).ok()?;
            AddrFields { bucket: at.bucket, folder: prefix.trim_end_matches('/').to_owned(), ..AddrFields::default() }
        }
        Page::Accounts | Page::Databases => return None,
    };
    // A text the builder would refuse (a `{param}` in it, say), or would write with other parts (a folder that starts or
    // ends with a space, which the builder trims), is kept as one line, so it can still be changed as it is: the parts
    // compare trimmed, and a trimmed change must not read as no change.
    let same = match page {
        Page::Hosts => {
            chore::host_url(t.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("winrm://")), &a.user, &a.host, port_or(&a.port, 1)?).is_ok()
        }
        Page::Mailboxes => koyomi_core::when::mailbox_url(&a.user, &a.host, port_or(&a.port, 1)?, &a.folder)
            .ok()
            .and_then(|url| koyomi_core::when::mailbox(&url).ok())
            .is_some_and(|m| m.folder == a.folder),
        _ => chore::bucket_url(&a.bucket, &a.folder)
            .ok()
            .and_then(|url| registry::bucket_prefix(&url).ok())
            .is_some_and(|p| p.trim_end_matches('/') == a.folder),
    };
    same.then_some(a)
}

/// The key of the words for a field `host_url` refused (`winrm`: a WinRM host that has no user).
fn host_field_key(f: chore::HostField, winrm: bool, user: &str) -> &'static str {
    match f {
        chore::HostField::User if winrm && user.is_empty() => "registry.addr_winrm_user",
        chore::HostField::User => "registry.addr_bad_user",
        chore::HostField::Host => "registry.addr_bad_host",
        chore::HostField::Port => "registry.addr_bad_port",
    }
}

/// The key of the words for a field `mailbox_url` refused.
fn mail_field_key(f: koyomi_core::when::MailField) -> &'static str {
    match f {
        koyomi_core::when::MailField::User => "registry.addr_bad_user",
        koyomi_core::when::MailField::Host => "registry.addr_bad_host",
        koyomi_core::when::MailField::Port => "registry.addr_bad_port",
        koyomi_core::when::MailField::Folder => "registry.addr_bad_folder",
    }
}

/// The key of the words for a field `bucket_url` refused.
fn bucket_field_key(f: chore::BucketField) -> &'static str {
    match f {
        chore::BucketField::Bucket => "registry.addr_bad_bucket",
        chore::BucketField::Folder => "registry.addr_bad_folder",
    }
}

/// The key of the words for a field `db_url` refused.
fn db_field_key(f: chore::DbField) -> &'static str {
    match f {
        chore::DbField::Host => "registry.db_bad_host",
        chore::DbField::Port => "registry.db_bad_port",
        chore::DbField::Database => "registry.db_bad_database",
        chore::DbField::User => "registry.db_bad_user",
        chore::DbField::File => "registry.db_bad_file",
    }
}

/// The folder the SQLite chooser starts in: the folder of the path already in the field, when it exists.
fn start_dir(path: &str) -> Option<std::path::PathBuf> {
    let dir = std::path::Path::new(path.trim()).parent()?;
    (!dir.as_os_str().is_empty() && dir.is_dir()).then(|| dir.to_path_buf())
}

/// The add / edit form of one page. `secret` is the row's own secret NAME (never shown or edited); the value typed is
/// `password` (hidden), sent once for the engine to store.
#[derive(Clone, Debug)]
pub(super) struct Form {
    pub page: Page,
    /// `Some(name)`: editing that entry (its name is fixed); `None`: adding.
    pub editing: Option<String>,
    pub name: String,
    /// The `user` of an account, the `db` of a database.
    pub target: String,
    /// The secret's name the row already has (without `secret:`): sent back when the password is kept.
    pub secret: String,
    /// The password as typed (the S3 secret access key on the bucket page); its `Debug` is `***`.
    pub password: Hidden,
    /// An SSH host being edited: leave the password out (the secret stays in the store).
    pub drop_password: bool,
    pub tls: SqlTls,
    /// The rest of the host, bucket and mailbox forms.
    pub x: Extra,
    /// What the form opened with: the form is dirty when it differs.
    orig: (String, String, String, SqlTls),
    orig_x: Extra,
    /// The fields of a database form, and what they opened with (the password empty).
    pub db: DbFields,
    db_orig: DbFields,
    /// The parts of a host, bucket or mailbox address, and what they opened with.
    pub addr: AddrFields,
    addr_orig: AddrFields,
    /// The address is held as one line (`target`): the row's text did not read in parts.
    pub raw: bool,
    /// The row's address as stored, sent back as it is while its parts are not changed.
    orig_text: String,
    /// A host's stored `on` has neither scheme: it is shown whole and sent whole.
    scheme_less: bool,
    /// The `db` text of the row being edited, kept when its place is not changed.
    orig_db: String,
    /// The state of the row's password (`ok`, `missing`, `unreadable`).
    pw_state: String,
    /// The number given to this form when it was opened: a chooser's answer is for one form only.
    pub gen: u64,
    /// The role or the name at the engine changed under the page: 保存 waits until the person has looked at the new listing.
    pub stale: bool,
    pub error: Option<Line>,
    /// The host form's "connect and check": asked, and the answer when it came.
    pub peek: Option<Peek>,
    /// The database form's 接続テスト: asked, and the answer when it came.
    pub db_test: Option<DbTest>,
}

/// What a database test was asked with, as the form read at the click. The password is kept as its sha256 only, so
/// a change of it (or of anything else) is seen without the form holding a second copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DbAsk {
    kind: DbKind,
    host: String,
    port: String,
    database: String,
    user: String,
    file: String,
    tls: SqlTls,
    /// The sha256 of the typed password, as typed (not trimmed).
    pw: Option<String>,
    /// The password the entry already has was meant.
    saved: bool,
}

/// One 接続テスト. Only an answer with this `ticket` is taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DbTest {
    pub ticket: u64,
    pub asked: DbAsk,
    /// `None` while the engine has not answered.
    pub result: Option<Result<String, String>>,
}

/// One "connect and check" of a host form. Only an answer with this `ticket` is taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Peek {
    pub ticket: u64,
    pub kind: PeekKind,
    /// What was asked, as `peek_target` gave it at the click: the peek is dropped when the address no longer reads so.
    pub asked: (PeekKind, String, u16),
    /// `None` while the engine has not answered.
    pub result: Option<Result<String, String>>,
}

/// How a peeked value stands against what the field holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    /// The field is empty.
    Empty,
    Same,
    Differs,
}

/// Compares what a host showed with the field. SSH: the trimmed text, exactly. WinRM: both as fingerprints, so
/// case and colons do not matter.
pub(super) fn verdict(winrm: bool, field: &str, shown: &str) -> Verdict {
    let field = field.trim();
    if field.is_empty() {
        return Verdict::Empty;
    }
    let same = if winrm {
        matches!((ito_winrm::fingerprint::parse(field), ito_winrm::fingerprint::parse(shown)), (Some(a), Some(b)) if a == b)
    } else {
        field == shown.trim()
    };
    if same {
        Verdict::Same
    } else {
        Verdict::Differs
    }
}

/// What the host, bucket and mailbox forms hold besides the shared fields (`target` is a host's address WITHOUT the
/// scheme, a bucket's `url`, a mailbox's `mail`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Extra {
    /// A host's kind: WinRM (else SSH).
    pub winrm: bool,
    /// SSH: `SHA256:…`.
    pub host_key: String,
    /// SSH: the private key file.
    pub key: String,
    /// WinRM: the certificate's SHA-256.
    pub cert: String,
    pub auth: WinrmAuth,
    /// A bucket's `key_id` (plain, or `secret:<name>`).
    pub key_id: String,
    pub region: String,
    pub endpoint: String,
}

impl Form {
    fn add(page: Page) -> Form {
        let db = DbFields::fresh();
        // A new address starts in parts, with the usual port (and a mailbox's `INBOX`) filled in.
        let addr = match page {
            Page::Hosts => AddrFields { port: default_port(page, false).to_string(), ..AddrFields::default() },
            Page::Mailboxes => AddrFields { port: default_port(page, false).to_string(), folder: "INBOX".into(), ..AddrFields::default() },
            _ => AddrFields::default(),
        };
        Form {
            page,
            editing: None,
            name: String::new(),
            target: String::new(),
            secret: String::new(),
            password: Hidden::default(),
            drop_password: false,
            tls: SqlTls::Verify,
            x: Extra::default(),
            orig: (String::new(), String::new(), String::new(), SqlTls::Verify),
            orig_x: Extra::default(),
            db: db.clone(),
            db_orig: db,
            addr: addr.clone(),
            addr_orig: addr,
            raw: false,
            orig_text: String::new(),
            scheme_less: false,
            orig_db: String::new(),
            pw_state: String::new(),
            gen: 0,
            stale: false,
            error: None,
            peek: None,
            db_test: None,
        }
    }

    fn edit_host(row: &HostRow) -> Form {
        let (winrm, rest) = split_on(&row.on);
        let x = Extra {
            winrm: winrm.unwrap_or(row.kind == "winrm"),
            host_key: row.host_key.clone().unwrap_or_default(),
            key: row.key.clone().unwrap_or_default(),
            cert: row.cert.clone().unwrap_or_default(),
            auth: row.auth.as_deref().and_then(|a| WinrmAuth::ALL.into_iter().find(|w| w.key() == a)).unwrap_or_default(),
            ..Extra::default()
        };
        // An SSH host may have no password: its form does not make one up.
        let secret = row.secret.clone().unwrap_or_default();
        let mut f = Form::of(Page::Hosts, &row.name, &rest, &secret, SqlTls::Verify, &row.state);
        f.orig_x = x.clone();
        f.x = x;
        f.scheme_less = winrm.is_none();
        f.load_addr(&row.on);
        f
    }

    fn edit_bucket(row: &BucketRow) -> Form {
        let key_id = match (&row.key_id, &row.key_id_secret) {
            (Some(k), _) => k.clone(),
            (None, Some(s)) => format!("secret:{s}"),
            (None, None) => String::new(),
        };
        let x = Extra {
            key_id,
            region: row.region.clone().unwrap_or_default(),
            endpoint: row.endpoint.clone().unwrap_or_default(),
            ..Extra::default()
        };
        // A row without a reference keeps an empty `secret`: the form does not make one up.
        let secret = row.secret.clone().unwrap_or_default();
        let mut f = Form::of(Page::Buckets, &row.name, &row.url, &secret, SqlTls::Verify, &row.state);
        f.orig_x = x.clone();
        f.x = x;
        f.load_addr(&row.url);
        f
    }

    fn edit_mailbox(row: &MailboxRow) -> Form {
        let mut f = Form::of(Page::Mailboxes, &row.name, &row.mail, &row.secret, SqlTls::Verify, &row.state);
        f.load_addr(&row.mail);
        f
    }

    /// Splits the stored address `text` into the parts the form shows; a text that does not read that way is held as
    /// one line (`raw`), so nothing is lost and it can be fixed.
    fn load_addr(&mut self, text: &str) {
        self.orig_text = text.to_owned();
        match read_addr(self.page, text) {
            Some(a) => {
                self.addr = a.clone();
                self.addr_orig = a;
                self.raw = false;
            }
            None => self.raw = true,
        }
    }

    fn edit_account(row: &AccountRow) -> Form {
        Form::of(Page::Accounts, &row.name, &row.user, &row.secret, SqlTls::Verify, &row.state)
    }

    /// The form of a PostgreSQL or SQLite row; `None` for a SQL Server row (and one that does not read), which
    /// this screen does not change.
    fn edit_database(row: &DatabaseRow) -> Option<Form> {
        let db = match chore::sql_db(&row.db).ok()? {
            SqlDb::Postgres { user, host, port, database } => {
                DbFields { kind: DbKind::Postgres, host, port: port.to_string(), database, user, ..DbFields::default() }
            }
            SqlDb::Sqlite { path } => {
                DbFields { kind: DbKind::Sqlite, file: path, port: chore::POSTGRES_PORT.to_string(), ..DbFields::default() }
            }
            SqlDb::SqlServer { .. } => return None,
        };
        // The secret's name is sent back as it is; it is never shown.
        let secret = row.secret.clone().unwrap_or_default();
        let tls = row.tls.as_deref().and_then(SqlTls::of).unwrap_or_default();
        let mut f = Form::of(Page::Databases, &row.name, "", &secret, tls, &row.state);
        f.db_orig = db.clone();
        f.db = db;
        f.orig_db = row.db.clone();
        Some(f)
    }

    /// The form of a row being edited; `state` is the row's password state (`ok`, `missing`, `unreadable`,
    /// `not-needed`).
    fn of(page: Page, name: &str, target: &str, secret: &str, tls: SqlTls, state: &str) -> Form {
        Form {
            page,
            editing: Some(name.to_owned()),
            name: name.to_owned(),
            target: target.to_owned(),
            secret: secret.to_owned(),
            password: Hidden::default(),
            drop_password: false,
            tls,
            x: Extra::default(),
            orig: (name.to_owned(), target.to_owned(), secret.to_owned(), tls),
            orig_x: Extra::default(),
            db: DbFields::default(),
            db_orig: DbFields::default(),
            addr: AddrFields::default(),
            addr_orig: AddrFields::default(),
            raw: false,
            orig_text: String::new(),
            scheme_less: false,
            orig_db: String::new(),
            pw_state: state.to_owned(),
            gen: 0,
            stale: false,
            error: None,
            peek: None,
            db_test: None,
        }
    }

    /// The kind, host and port a "connect and check" would go to, when the address reads and passes the engine's rule.
    pub(super) fn peek_target(&self) -> Option<(PeekKind, String, u16)> {
        if self.page != Page::Hosts {
            return None;
        }
        let on = if self.raw {
            self.host_on()
        } else {
            let port = port_or(&self.addr.port, default_port(self.page, self.x.winrm))?;
            chore::host_url(self.x.winrm, self.addr.user.trim(), self.addr.host.trim(), port).ok()?
        };
        let (kind, read, default) = if self.x.winrm {
            (PeekKind::Winrm, chore::winrm(&on)?, chore::WINRM_PORT)
        } else {
            (PeekKind::Ssh, chore::ssh(&on)?, chore::SSH_PORT)
        };
        let far = read.ok()?;
        let port = far.port.unwrap_or(default);
        peek_target_ok(&far.host, port).then_some((kind, far.host, port))
    }

    /// Drops a peek whose address is no longer the one it was asked for.
    pub(super) fn drop_stale_peek(&mut self) {
        if self.peek.as_ref().is_some_and(|p| self.peek_target().as_ref() != Some(&p.asked)) {
            self.peek = None;
        }
    }

    /// Whether the "connect and check" button takes a click: the page is open, the address reads, and no ask is waiting.
    pub(super) fn peek_enabled(&self, open: bool) -> bool {
        open && self.peek_target().is_some() && !self.peek.as_ref().is_some_and(|p| p.result.is_none())
    }

    /// The database the fields describe, when the port reads and `db_url` takes it.
    fn db_test_url(&self) -> Option<(SqlDb, String)> {
        if self.page != Page::Databases {
            return None;
        }
        let d = &self.db;
        let sqldb = if self.is_sqlite() {
            SqlDb::Sqlite { path: d.file.trim().to_owned() }
        } else {
            let port = port_of(&d.port)?;
            SqlDb::Postgres { user: d.user.trim().to_owned(), host: d.host.trim().to_owned(), port, database: d.database.trim().to_owned() }
        };
        let url = chore::db_url(&sqldb).ok()?;
        Some((sqldb, url))
    }

    /// Whether the saved password may be meant: the row has one, and the server, port, user and TLS are what the form
    /// opened with (a TLS that is not weaker).
    fn db_test_saved_ok(&self) -> bool {
        let (d, o) = (&self.db, &self.db_orig);
        let rank = |t: SqlTls| match t {
            SqlTls::Verify => 2,
            SqlTls::Trust => 1,
            SqlTls::Off => 0,
        };
        self.editing.is_some()
            && self.pw_state == "ok"
            && o.kind == DbKind::Postgres
            && d.host.trim().eq_ignore_ascii_case(o.host.trim())
            && port_of(&d.port).is_some()
            && port_of(&d.port) == port_of(&o.port)
            && d.user.trim() == o.user.trim()
            && rank(self.tls) >= rank(self.orig.3)
    }

    /// What a test would be asked with now; `None` when the fields do not read or there is no password to test with.
    pub(super) fn db_test_ask(&self) -> Option<DbAsk> {
        self.db_test_url()?;
        let d = &self.db;
        let typed = !self.is_sqlite() && !self.password.is_empty();
        if !self.is_sqlite() && !typed && !self.db_test_saved_ok() {
            return None;
        }
        Some(DbAsk {
            kind: d.kind,
            host: d.host.trim().to_owned(),
            port: d.port.trim().to_owned(),
            database: d.database.trim().to_owned(),
            user: d.user.trim().to_owned(),
            file: d.file.trim().to_owned(),
            tls: self.tls,
            pw: typed.then(|| koyomi_core::users::sha256(self.password.expose())),
            saved: !self.is_sqlite() && !typed,
        })
    }

    /// Drops a test whose fields (the password included) are no longer the ones it was asked with.
    pub(super) fn drop_stale_db_test(&mut self) {
        if self.db_test.as_ref().is_some_and(|t| self.db_test_ask().as_ref() != Some(&t.asked)) {
            self.db_test = None;
        }
    }

    /// Whether 接続テスト takes a click: the page is open and may write, the fields read, a password can be used, and
    /// no test is waiting.
    pub(super) fn db_test_enabled(&self, can: bool) -> bool {
        can && self.db_test_ask().is_some() && !self.db_test.as_ref().is_some_and(|t| t.result.is_none())
    }

    /// The request of a test now, with `ticket`.
    fn db_test_request(&self, ticket: u64) -> Option<Request> {
        let (_, db) = self.db_test_url()?;
        let ask = self.db_test_ask()?;
        let typed = ask.pw.is_some();
        let sqlite = self.is_sqlite();
        Some(Request::DatabaseTest {
            db,
            tls: (!sqlite).then_some(self.tls),
            entry: (!sqlite && !typed).then(|| self.name.trim().to_owned()),
            password_value: typed.then(|| self.password.clone()),
            ticket,
        })
    }

    /// The value in the field the peek fills (the host key, or the certificate's fingerprint).
    fn peek_field(&self) -> &str {
        if self.x.winrm {
            &self.x.cert
        } else {
            &self.x.host_key
        }
    }

    pub(super) fn dirty(&self) -> bool {
        (&self.name, &self.target, &self.secret, self.tls) != (&self.orig.0, &self.orig.1, &self.orig.2, self.orig.3)
            || self.addr != self.addr_orig
            || self.x != self.orig_x
            || self.db != self.db_orig
            || !self.password.is_empty()
            || self.drop_password
    }

    /// The kind seg of a host was picked. WinRM needs a password, so "do not use a password" is cleared.
    pub(super) fn set_winrm(&mut self, winrm: bool) {
        if self.x.winrm != winrm {
            self.peek = None;
            // The kind's own port follows the kind; a port the person chose stays.
            let other = default_port(self.page, !winrm).to_string();
            if !self.raw && (self.addr.port.trim().is_empty() || self.addr.port.trim() == other) {
                self.addr.port = default_port(self.page, winrm).to_string();
            }
        }
        self.x.winrm = winrm;
        if winrm {
            self.drop_password = false;
        }
    }

    /// Whether the "do not use a password" box is offered: an SSH host being edited whose row has a password.
    pub(super) fn can_drop_password(&self) -> bool {
        self.page == Page::Hosts && !self.x.winrm && self.editing.is_some() && self.own_secret().is_some()
    }

    /// The "do not use a password" box was ticked or cleared: ticking empties the field.
    pub(super) fn set_drop_password(&mut self, drop: bool) {
        self.drop_password = drop;
        if drop {
            self.password = Hidden::default();
        }
    }

    /// The row's own secret, when it can be sent back: not empty and no space in it.
    fn own_secret(&self) -> Option<&str> {
        let own = self.secret.trim();
        (!own.is_empty() && !own.chars().any(char::is_whitespace)).then_some(own)
    }

    /// Whether the form is for a WinRM host.
    fn is_winrm(&self) -> bool {
        self.page == Page::Hosts && self.x.winrm
    }

    /// The `on` a host form held as one line (`raw`) writes: the seg's scheme in front of the address as typed (the
    /// text as it is when the row had neither scheme).
    pub(super) fn host_on(&self) -> String {
        if self.scheme_less {
            self.target.trim().to_owned()
        } else {
            format!("{}{}", scheme(self.x.winrm), self.target.trim())
        }
    }

    /// The parts of the address as they compare: user, host, port, folder, bucket, with the defaults filled in
    /// (an empty port, an empty mailbox folder) and a host written the way the builder writes it. `None` when the
    /// port does not read.
    fn addr_key(&self, a: &AddrFields, winrm: bool) -> Option<(String, String, u16, String, String)> {
        let host = a.host.trim();
        let host = chore::host_text(host).map_or_else(|| host.to_owned(), |(h, _)| h);
        let port = match self.page {
            Page::Buckets => 0,
            _ => port_or(&a.port, default_port(self.page, winrm))?,
        };
        let folder = match self.page {
            Page::Mailboxes if a.folder.trim().is_empty() => "INBOX".to_owned(),
            Page::Buckets => a.folder.trim().trim_matches('/').to_owned(),
            _ => a.folder.trim().to_owned(),
        };
        Some((a.user.trim().to_owned(), host, port, folder, a.bucket.trim().to_owned()))
    }

    /// The stored address text, while editing in parts with the kind and every part as they were stored (a hand-written
    /// `:22`, an upper-case `SSH://` stays).
    fn stored_address(&self) -> Option<String> {
        let same = self.editing.is_some()
            && !self.raw
            && self.x.winrm == self.orig_x.winrm
            && self.addr_key(&self.addr, self.x.winrm).is_some_and(|k| Some(k) == self.addr_key(&self.addr_orig, self.orig_x.winrm));
        same.then(|| self.orig_text.trim().to_owned())
    }

    /// The address a host, mailbox or bucket form sends: the stored text when its parts are not changed, else the
    /// text built from the parts (a refusal names the field), or the one line typed (`raw`).
    fn address(&self) -> Result<String, Line> {
        if self.raw {
            return Ok(match self.page {
                Page::Hosts => self.host_on(),
                _ => self.target.trim().to_owned(),
            });
        }
        if let Some(text) = self.stored_address() {
            return Ok(text);
        }
        let a = &self.addr;
        let (user, host) = (a.user.trim(), a.host.trim());
        let port = || port_or(&a.port, default_port(self.page, self.x.winrm)).ok_or(Line::Key("registry.addr_bad_port"));
        match self.page {
            Page::Hosts => chore::host_url(self.x.winrm, user, host, port()?).map_err(|f| Line::Key(host_field_key(f, self.x.winrm, user))),
            Page::Mailboxes => koyomi_core::when::mailbox_url(user, host, port()?, &a.folder).map_err(|f| Line::Key(mail_field_key(f))),
            Page::Buckets => chore::bucket_url(a.bucket.trim(), &a.folder).map_err(|f| Line::Key(bucket_field_key(f))),
            Page::Accounts | Page::Databases => Ok(self.target.trim().to_owned()),
        }
    }

    /// Whether the password may be left out: an SSH host's (it can log in with a key).
    pub(super) fn password_optional(&self) -> bool {
        self.page == Page::Hosts && !self.x.winrm
    }

    /// What the password field says for the form as it stands: the key of the note under it.
    pub(super) fn password_note(&self) -> &'static str {
        let optional = self.password_optional();
        if self.editing.is_none() {
            return if optional { "registry.pw_new_optional" } else { "registry.pw_new" };
        }
        // An SSH host with no password of its own signs in with its key.
        if optional && (self.own_secret().is_none() || self.pw_state == "not-needed") {
            return "registry.pw_none_optional";
        }
        match self.pw_state.as_str() {
            "ok" => "registry.pw_ok",
            "unreadable" => "registry.pw_unreadable",
            _ => "registry.pw_missing",
        }
    }

    /// Whether the database is a SQLite file: it takes no password and no TLS.
    pub(super) fn is_sqlite(&self) -> bool {
        self.db.kind == DbKind::Sqlite
    }

    /// The request of a database form. The place is built from the fields (or is the row's own text when it is
    /// not changed); the password is the typed one, else the one the row already has.
    fn database_request(&self, name: String, base: String) -> Result<Request, Line> {
        let d = &self.db;
        let sqlite = self.is_sqlite();
        let sqldb = if sqlite {
            SqlDb::Sqlite { path: d.file.trim().to_owned() }
        } else {
            let port = port_of(&d.port).ok_or(Line::Key("registry.db_bad_port"))?;
            SqlDb::Postgres { user: d.user.trim().to_owned(), host: d.host.trim().to_owned(), port, database: d.database.trim().to_owned() }
        };
        // An unchanged place keeps the row's text (an explicit `:5432` stays).
        let unchanged = self.editing.is_some() && chore::sql_db(&self.orig_db).ok().as_ref() == Some(&sqldb);
        let db = if unchanged { self.orig_db.clone() } else { chore::db_url(&sqldb).map_err(|f| Line::Key(db_field_key(f)))? };
        if db.contains(['{', '}']) {
            return Err(Line::Raw(msg::line(msg::CFG_DATABASE_BRACES, &[&name])));
        }
        let typed = !sqlite && !self.password.is_empty();
        let (password, password_value) = if sqlite {
            (None, None)
        } else if typed {
            (None, Some(self.password.clone()))
        } else if let Some(own) = self.own_secret().filter(|_| self.editing.is_some()) {
            (Some(format!("secret:{own}")), None)
        } else {
            return Err(Line::PasswordEmpty);
        };
        // `verify` is what no `tls` means; SQLite takes none.
        let tls = (!sqlite && self.tls != SqlTls::Verify).then_some(self.tls);
        // Only the shape is checked here: the engine names the secret it stores a typed password in.
        let shape = if typed { Some("secret:koyomi-db-x") } else { password.as_deref() };
        chore::check_db(&db, shape, tls.unwrap_or_default())
            .map_err(|e| Line::Raw(msg::line(msg::CFG_DATABASE_BAD, &[&name, msg::uncoded(&e)])))?;
        Ok(Request::DatabasePut { name, db, password, tls, base_sha256: base, password_value })
    }

    /// The request this form sends, or why it is refused (before anything is sent). `taken`: the names
    /// already registered on the page.
    pub(super) fn request(&self, base: &str, taken: &[String]) -> Result<Request, Line> {
        let name = self.name.trim().to_owned();
        if self.editing.is_none() {
            if !registry::valid_name(&name) {
                return Err(Line::Raw(msg::line(msg::CFG_ENTRY_NAME_BAD, &[self.page.table(), &name])));
            }
            if taken.contains(&name) {
                return Err(Line::NameExists(name));
            }
        }
        let base = base.to_owned();
        match self.page {
            Page::Accounts => {
                let user = self.target.trim().to_owned();
                if !jobnet::valid_account(&user) {
                    return Err(Line::Raw(msg::line(msg::CFG_ACCOUNT_USER, &[&name])));
                }
                let pw = self.password_choice(Line::PasswordEmpty)?;
                Ok(Request::AccountPut { name, user, password: pw.reference(), base_sha256: base, password_value: pw.value() })
            }
            Page::Databases => self.database_request(name, base),
            Page::Hosts => {
                let winrm = self.is_winrm();
                let opt = |s: &str| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
                let pw = self.password_choice(Line::PasswordEmpty)?;
                // Only the keys of the chosen kind go: the other kind's rows are hidden.
                let entry = HostEntry {
                    name: name.clone(),
                    on: self.address()?,
                    host_key: if winrm { None } else { opt(&self.x.host_key) },
                    key: if winrm { None } else { opt(&self.x.key) },
                    password: pw.check_reference(),
                    cert: if winrm { opt(&self.x.cert) } else { None },
                    auth: (winrm && self.x.auth != WinrmAuth::Basic).then_some(self.x.auth),
                };
                entry.check().map_err(Line::Raw)?;
                Ok(Request::HostPut {
                    name,
                    on: entry.on,
                    host_key: entry.host_key,
                    key: entry.key,
                    password: pw.reference(),
                    cert: entry.cert,
                    auth: entry.auth.map(|a| a.key().to_owned()),
                    base_sha256: base,
                    password_value: pw.value(),
                })
            }
            Page::Buckets => {
                let pw = self.password_choice(Line::Key("registry.s3_key_empty"))?;
                let opt = |s: &str| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
                let entry = BucketEntry {
                    name: name.clone(),
                    url: self.address()?,
                    key_id: opt(&self.x.key_id),
                    password: pw.check_reference(),
                    region: opt(&self.x.region),
                    endpoint: opt(&self.x.endpoint),
                };
                entry.check().map_err(Line::Raw)?;
                Ok(Request::BucketPut {
                    name,
                    url: entry.url,
                    key_id: entry.key_id,
                    password: pw.reference(),
                    region: entry.region,
                    endpoint: entry.endpoint,
                    base_sha256: base,
                    password_value: pw.value(),
                })
            }
            Page::Mailboxes => {
                let pw = self.password_choice(Line::PasswordEmpty)?;
                let entry = MailboxEntry { name: name.clone(), mail: self.address()?, password: pw.check_reference().unwrap_or_default() };
                entry.check().map_err(Line::Raw)?;
                Ok(Request::MailboxPut { name, mail: entry.mail, password: pw.reference(), base_sha256: base, password_value: pw.value() })
            }
        }
    }

    /// What the form sends for the password: the typed value (the field as typed, not trimmed), else the row's own
    /// secret when it is being edited and kept, else nothing (an SSH host only) or `missing`.
    fn password_choice(&self, missing: Line) -> Result<Pw, Line> {
        if !self.password.is_empty() {
            return Ok(Pw::Typed(self.password.clone()));
        }
        if self.editing.is_some() && !self.drop_password {
            if let Some(own) = self.own_secret() {
                return Ok(Pw::Kept(format!("secret:{own}")));
            }
        }
        if self.password_optional() {
            Ok(Pw::Nothing)
        } else {
            Err(missing)
        }
    }
}

/// What a request carries for the password.
enum Pw {
    /// A value typed in the form: sent once for the engine to store (it picks the secret's name).
    Typed(Hidden),
    /// The row's own `secret:<name>`, kept.
    Kept(String),
    /// No password (an SSH host that logs in with a key).
    Nothing,
}

impl Pw {
    /// The `password` field of the request: a reference only when the row's own is kept.
    fn reference(&self) -> Option<String> {
        match self {
            Pw::Kept(r) => Some(r.clone()),
            Pw::Typed(_) | Pw::Nothing => None,
        }
    }

    /// The `password_value` field of the request.
    fn value(&self) -> Option<Hidden> {
        match self {
            Pw::Typed(v) => Some(v.clone()),
            Pw::Kept(_) | Pw::Nothing => None,
        }
    }

    /// The reference the entry's own check reads: a typed value stands in as a placeholder (the engine names the
    /// real secret), and the value itself never goes into an entry.
    fn check_reference(&self) -> Option<String> {
        match self {
            Pw::Typed(_) => Some("secret:koyomi-x".to_owned()),
            Pw::Kept(r) => Some(r.clone()),
            Pw::Nothing => None,
        }
    }
}

/// The question before an entry is removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Confirm {
    pub page: Page,
    pub name: String,
    /// The nets that name it, as the owner saw them: `force` goes only if this is not empty.
    pub used_by: Vec<String>,
    /// 0: go on, 1: cancel. A fresh question starts on cancel.
    pub focus: usize,
}

/// What the page needs to know about the window each frame.
pub(super) struct View<'a> {
    /// The `[[remote]]` name of the PC shown; `None` for this PC.
    pub far: Option<&'a str>,
    /// The access is [`Access::Open`].
    pub open: bool,
}

/// What a click on the page asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Click {
    Reload,
    OpenAdd,
    OpenEdit(String),
    CloseForm,
    Save,
    /// 「新しい一覧を見ました」: the form may be sent again.
    Seen,
    AskRemove(String),
    Yes,
    No,
    /// Esc: the question, else the form.
    Esc,
    MoveFocus,
    /// 「接続して確かめる」 of the host form.
    Peek,
    /// 「接続テスト」 of the database form.
    DbTest,
    /// 「使う」: the value the host showed goes into the field.
    UsePeek,
}

/// What the browser does after a click.
// `Request` grew with the typed password fields; these are short-lived UI actions, one at a time, so boxing buys nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub(super) enum Do {
    Nothing,
    /// Ask `Registry` (a read).
    Ask,
    Send(Request),
}

/// What an answer did to the page, for the browser.
pub(super) struct Done {
    /// The listing is asked for again.
    pub reload: bool,
}

/// Which field a chosen file goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Chosen {
    /// A host's private key file.
    HostKey,
    /// A SQLite database file.
    SqliteFile,
}

/// A file chooser that is open, with what it was opened for: its answer goes into that form only.
pub(super) struct Browse {
    rx: std::sync::mpsc::Receiver<Vec<std::path::PathBuf>>,
    page: Page,
    /// The number of the form that was up at the click.
    gen: u64,
    /// The PC shown at the click.
    link: Option<String>,
    what: Chosen,
}

#[derive(Default)]
pub(super) struct Registry {
    /// The last frame a page was drawn in.
    seen: Option<u64>,
    /// The page drawn last: another one is a new opening.
    shown: Option<Page>,
    pub(super) listing: Option<RegistryBody>,
    /// The role and name last told, to see a change.
    ident: Option<Under>,
    /// `Registry` asked and not yet answered.
    asked: usize,
    /// The answers to throw away (asked before the role or the name changed), oldest first.
    discard: usize,
    /// The newest answer taken while others are still to come.
    got: Option<Result<RegistryBody, String>>,
    /// The listing is to be asked for when the page can.
    pub(super) need: bool,
    error: Option<String>,
    /// The form of each page ([`Page::idx`]).
    pub(super) forms: [Option<Form>; Page::COUNT],
    /// The file chooser, while it is open (only for an engine on this PC).
    browse: Option<Browse>,
    /// The last number given to an opened form.
    form_gen: u64,
    /// The `[[remote]]` name of the PC shown, `None` for this PC (set each frame the page is drawn).
    link: Option<String>,
    /// The engine is on this PC (set each frame the page is drawn).
    local: bool,
    /// The access is open (set each frame the page is drawn).
    open: bool,
    /// A write is out and not yet answered.
    write: bool,
    refusal: Option<Line>,
    /// The last ticket given to a "connect and check".
    peek_ticket: u64,
    pub(super) confirm: Option<Confirm>,
}

impl Registry {
    /// A page is drawn this frame. A frame that does not follow the last one, or another page, is a new
    /// opening: the listing is asked for again and forms that hold nothing typed are closed.
    pub(super) fn touch(&mut self, frame: u64, page: Page) {
        let fresh = self.seen.is_none_or(|s| frame > s + 1) || self.shown != Some(page);
        self.seen = Some(frame);
        self.shown = Some(page);
        if fresh {
            self.need = true;
            self.refusal = None;
            self.confirm = None;
            for f in &mut self.forms {
                if f.as_ref().is_some_and(|f| !f.dirty()) {
                    *f = None;
                }
            }
        }
    }

    /// Whether a page was drawn in the frame before `frame` (or in it).
    pub(super) fn is_open(&self, frame: u64) -> bool {
        self.seen.is_some_and(|s| frame <= s + 1)
    }

    /// `Registry` is being sent.
    pub(super) fn ask(&mut self) {
        self.need = false;
        self.asked += 1;
        self.error = None;
    }

    /// One `Registry` answer. Only when none is left to come is a listing taken: the newest wins.
    pub(super) fn listing_answer(&mut self, r: Result<String, String>) {
        self.asked = self.asked.saturating_sub(1);
        if self.discard > 0 {
            self.discard -= 1;
            return;
        }
        self.got = Some(r.and_then(|t| RegistryBody::parse(&t)));
        if self.asked == 0 {
            match self.got.take() {
                Some(Ok(l)) => {
                    self.listing = Some(l);
                    self.error = None;
                }
                Some(Err(e)) => self.error = Some(e),
                None => {}
            }
        }
    }

    /// The role or the name at the engine is told. A change from the last told one drops the listing.
    /// Returns whether it did.
    pub(super) fn role_told(&mut self, now: Under) -> bool {
        if self.ident.as_ref() == Some(&now) {
            return false;
        }
        let first = self.ident.is_none();
        self.ident = Some(now);
        if first {
            return false;
        }
        self.dropped();
        true
    }

    /// What was read under another role or name is not kept: the listing goes, forms with nothing typed close,
    /// and the others wait until the new listing is looked at.
    fn dropped(&mut self) {
        self.listing = None;
        self.got = None;
        self.error = None;
        self.refusal = None;
        self.confirm = None;
        self.discard = self.asked;
        self.need = true;
        for f in &mut self.forms {
            if let Some(form) = f {
                form.peek = None;
                form.db_test = None;
            }
            match f {
                Some(form) if form.dirty() => form.stale = true,
                _ => *f = None,
            }
        }
    }

    fn ready(&self, v: &View) -> bool {
        v.open && !self.write && self.asked == 0 && self.error.is_none() && self.listing.is_some()
    }

    fn can_write(&self, v: &View) -> bool {
        self.ready(v) && self.confirm.is_none()
    }

    fn base(&self) -> String {
        self.listing.as_ref().map(|l| l.sha256.clone()).unwrap_or_default()
    }

    /// The names registered on `page`.
    pub(super) fn names(&self, page: Page) -> Vec<String> {
        match (&self.listing, page) {
            (None, _) => Vec::new(),
            (Some(l), Page::Accounts) => l.accounts.iter().map(|a| a.name.clone()).collect(),
            (Some(l), Page::Databases) => l.databases.iter().map(|d| d.name.clone()).collect(),
            (Some(l), Page::Hosts) => l.hosts.iter().map(|d| d.name.clone()).collect(),
            (Some(l), Page::Buckets) => l.buckets.iter().map(|d| d.name.clone()).collect(),
            (Some(l), Page::Mailboxes) => l.mailboxes.iter().map(|d| d.name.clone()).collect(),
        }
    }

    /// The nets that name `name` on `page`, as the listing has it.
    fn used_by(&self, page: Page, name: &str) -> Option<Vec<String>> {
        let l = self.listing.as_ref()?;
        match page {
            Page::Accounts => l.accounts.iter().find(|a| a.name == name).map(|a| a.used_by.clone()),
            Page::Databases => l.databases.iter().find(|d| d.name == name).map(|d| d.used_by.clone()),
            Page::Hosts => l.hosts.iter().find(|d| d.name == name).map(|d| d.used_by.clone()),
            Page::Buckets => l.buckets.iter().find(|d| d.name == name).map(|d| d.used_by.clone()),
            Page::Mailboxes => l.mailboxes.iter().find(|d| d.name == name).map(|d| d.used_by.clone()),
        }
    }

    /// What a click on `page` asks for.
    pub(super) fn act(&mut self, click: Click, page: Page, v: &View) -> Do {
        let i = page.idx();
        match click {
            Click::Reload => {
                if v.open && self.asked == 0 && !self.write {
                    self.need = true;
                    return Do::Ask;
                }
            }
            Click::OpenAdd => {
                if self.forms[i].is_none() && v.open && !self.write {
                    let mut form = Form::add(page);
                    self.form_gen += 1;
                    form.gen = self.form_gen;
                    self.forms[i] = Some(form);
                }
            }
            Click::OpenEdit(name) => {
                if self.can_write(v) && !self.forms[i].as_ref().is_some_and(Form::dirty) {
                    let form = self.listing.as_ref().and_then(|l| match page {
                        Page::Accounts => l.accounts.iter().find(|a| a.name == name).map(Form::edit_account),
                        // A SQL Server row (or one that does not read) is not opened: koyomi.toml is where it is changed.
                        Page::Databases => l.databases.iter().find(|d| d.name == name).and_then(Form::edit_database),
                        Page::Hosts => l.hosts.iter().find(|d| d.name == name).map(Form::edit_host),
                        Page::Buckets => l.buckets.iter().find(|d| d.name == name).map(Form::edit_bucket),
                        Page::Mailboxes => l.mailboxes.iter().find(|d| d.name == name).map(Form::edit_mailbox),
                    });
                    if let Some(mut form) = form {
                        self.form_gen += 1;
                        form.gen = self.form_gen;
                        self.forms[i] = Some(form);
                        self.refusal = None;
                    }
                }
            }
            Click::CloseForm => self.forms[i] = None,
            Click::Save => {
                if self.can_write(v) && self.forms[i].as_ref().is_some_and(|f| !f.stale && f.dirty()) {
                    let (base, taken) = (self.base(), self.names(page));
                    if let Some(form) = self.forms[i].as_mut() {
                        match form.request(&base, &taken) {
                            Err(why) => form.error = Some(why),
                            Ok(req) => {
                                form.error = None;
                                self.refusal = None;
                                self.write = true;
                                return Do::Send(req);
                            }
                        }
                    }
                }
            }
            Click::Seen => {
                if self.ready(v) {
                    if let Some(f) = &mut self.forms[i] {
                        f.stale = false;
                    }
                }
            }
            Click::AskRemove(name) => {
                if self.can_write(v) {
                    if let Some(used_by) = self.used_by(page, &name) {
                        self.confirm = Some(Confirm { page, name, used_by, focus: 1 });
                    }
                }
            }
            Click::MoveFocus => {
                if let Some(c) = &mut self.confirm {
                    c.focus = 1 - c.focus.min(1);
                }
            }
            Click::No => self.confirm = None,
            Click::Yes => {
                let c = self.confirm.take();
                if let Some(c) = c.filter(|c| self.ready(v) && self.used_by(c.page, &c.name).is_some()) {
                    let (name, base) = (c.name, self.base());
                    // Only the ask the owner saw, naming the nets, forces it.
                    let force = !c.used_by.is_empty();
                    let req = match c.page {
                        Page::Accounts => Request::AccountRemove { name, base_sha256: base, force },
                        Page::Databases => Request::DatabaseRemove { name, base_sha256: base, force },
                        Page::Hosts => Request::HostRemove { name, base_sha256: base, force },
                        Page::Buckets => Request::BucketRemove { name, base_sha256: base, force },
                        Page::Mailboxes => Request::MailboxRemove { name, base_sha256: base, force },
                    };
                    self.refusal = None;
                    self.write = true;
                    return Do::Send(req);
                }
            }
            Click::Peek => {
                if v.open && page == Page::Hosts {
                    let ticket = self.peek_ticket + 1;
                    if let Some(f) = self.forms[i].as_mut().filter(|f| f.peek.as_ref().is_none_or(|p| p.result.is_some())) {
                        if let Some((kind, host, port)) = f.peek_target() {
                            self.peek_ticket = ticket;
                            f.peek = Some(Peek { ticket, kind, asked: (kind, host.clone(), port), result: None });
                            return Do::Send(Request::HostPeek { kind, host, port, ticket });
                        }
                    }
                }
            }
            Click::DbTest => {
                if v.open && page == Page::Databases && self.can_write(v) {
                    let ticket = self.peek_ticket + 1;
                    if let Some(f) = self.forms[i].as_mut().filter(|f| !f.stale && f.db_test_enabled(true)) {
                        if let (Some(req), Some(asked)) = (f.db_test_request(ticket), f.db_test_ask()) {
                            self.peek_ticket = ticket;
                            f.db_test = Some(DbTest { ticket, asked, result: None });
                            return Do::Send(req);
                        }
                    }
                }
            }
            Click::UsePeek => {
                if let Some(f) = self.forms[i].as_mut() {
                    f.drop_stale_peek();
                    if let Some(Peek { result: Some(Ok(value)), .. }) = f.peek.clone() {
                        if f.x.winrm {
                            f.x.cert = value;
                        } else {
                            f.x.host_key = value;
                        }
                    }
                }
            }
            Click::Esc => {
                // Only this page's question: one left on the other page is not on the screen.
                if self.confirm.as_ref().is_some_and(|q| q.page == page) {
                    self.confirm = None;
                } else {
                    self.forms[i] = None;
                }
            }
        }
        Do::Nothing
    }

    /// The answer to one of the page's writes. A reply is the new listing; an error is the page's line.
    pub(super) fn answered(&mut self, req: &Request, r: Result<String, String>) -> Done {
        if let Request::HostPeek { ticket, .. } = req {
            // Only the answer to the ask the open form is waiting for; any other is late and dropped.
            if let Some(f) = self.forms[Page::Hosts.idx()].as_mut() {
                f.drop_stale_peek();
                if let Some(p) = f.peek.as_mut().filter(|p| p.ticket == *ticket && p.result.is_none()) {
                    p.result = Some(r);
                }
            }
            return Done { reload: false };
        }
        if let Request::DatabaseTest { ticket, .. } = req {
            // Only the answer to the test the open form is waiting for; any other is late and dropped.
            if let Some(f) = self.forms[Page::Databases.idx()].as_mut() {
                f.drop_stale_db_test();
                if let Some(t) = f.db_test.as_mut().filter(|t| t.ticket == *ticket && t.result.is_none()) {
                    t.result = Some(r);
                }
            }
            return Done { reload: false };
        }
        let (page, name) = match req {
            Request::AccountPut { name, .. } | Request::AccountRemove { name, .. } => (Page::Accounts, name),
            Request::DatabasePut { name, .. } | Request::DatabaseRemove { name, .. } => (Page::Databases, name),
            Request::HostPut { name, .. } | Request::HostRemove { name, .. } => (Page::Hosts, name),
            Request::BucketPut { name, .. } | Request::BucketRemove { name, .. } => (Page::Buckets, name),
            Request::MailboxPut { name, .. } | Request::MailboxRemove { name, .. } => (Page::Mailboxes, name),
            _ => return Done { reload: false },
        };
        let name = name.trim().to_owned();
        self.write = false;
        match r {
            Ok(reply) => {
                if self.forms[page.idx()].as_ref().is_some_and(|f| f.name.trim() == name) {
                    self.forms[page.idx()] = None;
                }
                match RegistryBody::parse(&reply) {
                    Ok(body) => {
                        self.listing = Some(body);
                        self.error = None;
                        self.refusal = None;
                        Done { reload: false }
                    }
                    Err(_) => {
                        // The parser's text could quote the reply: it is dropped.
                        self.refusal = Some(Line::ReplyBad);
                        Done { reload: true }
                    }
                }
            }
            Err(e) => {
                self.refusal = Some(if msg::CONFIG_CHANGED.is_in(&e) { Line::BaseChanged } else { Line::Raw(e) });
                // The file changed meanwhile (E2247), or a net names it now (E2250): what is shown is read again.
                Done { reload: true }
            }
        }
    }

    /// A chosen file goes into the field it was asked for, only when the form of that page is still the one that was
    /// up at the click and the PC shown is the same; otherwise it is dropped. Returns whether it was put.
    pub(super) fn put_browsed(&mut self, page: Page, gen: u64, link: Option<&str>, what: Chosen, paths: &[std::path::PathBuf]) -> bool {
        if self.link.as_deref() != link {
            return false;
        }
        let (Some(f), Some(p)) = (self.forms[page.idx()].as_mut().filter(|f| f.gen == gen), paths.first()) else {
            return false;
        };
        let path = p.display().to_string();
        match (what, page) {
            (Chosen::HostKey, Page::Hosts) => f.x.key = path,
            (Chosen::SqliteFile, Page::Databases) => f.db.file = path,
            _ => return false,
        }
        true
    }

    /// Takes the answer of the open file chooser, when it came.
    fn poll_browse(&mut self, ctx: &egui::Context) {
        let Some(b) = self.browse.take() else { return };
        match b.rx.try_recv() {
            Ok(paths) => {
                self.put_browsed(b.page, b.gen, b.link.as_deref(), b.what, &paths);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                self.browse = Some(b);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
        }
    }

    /// A request the thread did not take: the page does not wait for an answer that will not come. What was
    /// typed stays.
    pub(super) fn unsent(&mut self, read: bool, why: String) {
        if read {
            self.asked = self.asked.saturating_sub(1);
            self.error = Some(why);
        } else {
            self.write = false;
            self.refusal = Some(Line::Unsent(why));
        }
    }

    /// A "connect and check" the thread did not take: the form shows why instead of waiting.
    pub(super) fn peek_unsent(&mut self, why: String) {
        if let Some(p) = self.forms[Page::Hosts.idx()].as_mut().and_then(|f| f.peek.as_mut()) {
            if p.result.is_none() {
                p.result = Some(Err(why));
            }
        }
    }

    /// A 接続テスト the thread did not take: the form shows why instead of waiting.
    pub(super) fn db_test_unsent(&mut self, why: String) {
        if let Some(t) = self.forms[Page::Databases.idx()].as_mut().and_then(|f| f.db_test.as_mut()) {
            if t.result.is_none() {
                t.result = Some(Err(why));
            }
        }
    }

    /// Whether a form holds typed changes.
    pub(super) fn forms_dirty(&self) -> bool {
        self.forms.iter().flatten().any(Form::dirty)
    }

    /// A question or a form is up on this page (Esc closes it before it closes the settings). The other
    /// page's form does not count: this page's Esc could not close it.
    pub(super) fn wants_esc(&self, page: Page) -> bool {
        self.confirm.as_ref().is_some_and(|q| q.page == page) || self.forms[page.idx()].is_some()
    }

    #[cfg(test)]
    pub(super) fn write(&self) -> bool {
        self.write
    }

    #[cfg(test)]
    pub(super) fn refusal(&self) -> Option<&Line> {
        self.refusal.as_ref()
    }

    #[cfg(test)]
    pub(super) fn asked(&self) -> usize {
        self.asked
    }

    #[cfg(test)]
    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// The command that puts a secret's value in (the value is typed there, never here). A name koyomi.toml was
/// hand-written with that a shell would split or read (a space, `&`, `;` …) is put in double quotes.
pub(super) fn secret_command(secret: &str) -> String {
    if !secret.is_empty() && secret.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        format!("koyomi secret set {secret}")
    } else {
        format!("koyomi secret set \"{secret}\"")
    }
}

/// The state of a secret in words.
fn state_word<'a>(t: &'a ito_i18n::Table, state: &str) -> &'a str {
    t.tr(match state {
        "ok" => "registry.state_ok",
        "missing" => "registry.state_missing",
        "unreadable" => "registry.state_unreadable",
        "not-needed" => "registry.state_not_needed",
        _ => "registry.state_unknown",
    })
}

/// The kind of a database in words.
pub(super) fn kind_word(kind: &str) -> &'static str {
    match kind {
        "sqlserver" => "SQL Server",
        "postgres" => "PostgreSQL",
        "sqlite" => "SQLite",
        _ => "",
    }
}

/// Where a database is: `host:port/database` for a server, the file for SQLite; the text itself if it does not read.
pub(super) fn where_word(db: &str) -> String {
    match chore::sql_db(db) {
        Ok(SqlDb::SqlServer { host, port, database, .. } | SqlDb::Postgres { host, port, database, .. }) => {
            format!("{host}:{port}/{database}")
        }
        Ok(SqlDb::Sqlite { path }) => path,
        Err(_) => chore::hide_password(db).into_owned(),
    }
}

/// The nets that name an entry, a few of them: `a, b, c` and, after that, 「ほか N 件」.
pub(super) fn nets_word(t: &ito_i18n::Table, used: &[String]) -> String {
    const SHOWN: usize = 3;
    let mut s = used.iter().take(SHOWN).cloned().collect::<Vec<_>>().join(", ");
    if used.len() > SHOWN {
        s.push(' ');
        s.push_str(&t.trf("registry.more", &[&(used.len() - SHOWN).to_string()]));
    }
    s
}

/// The question's title and body for removing `name`.
pub(super) fn remove_words(t: &ito_i18n::Table, c: &Confirm) -> (String, String) {
    let [.., title, plain, used] = c.page.words();
    let body = if c.used_by.is_empty() { t.trf(plain, &[&c.name]) } else { t.trf(used, &[&c.name, &nets_word(t, &c.used_by)]) };
    (t.tr(title).to_owned(), body)
}

/// Draws the page and returns the one click of this frame.
pub(super) fn show(r: &mut Registry, ui: &mut egui::Ui, t: &ito_i18n::Table, v: &View, access: &Access, page: Page) -> Option<Click> {
    let c = ito_theme::colors();
    let mut click: Option<Click> = None;
    r.local = v.far.is_none();
    r.link = v.far.map(str::to_owned);
    r.open = v.open;
    r.poll_browse(ui.ctx());
    let tag = page.tag();
    let (esc, enter, tab) = if r.wants_esc(page) {
        ui.ctx().input_mut(|i| {
            let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            let asking = r.confirm.as_ref().is_some_and(|q| q.page == page);
            let enter = asking && i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            let tab =
                asking && (i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) || i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab));
            (esc, enter, tab)
        })
    } else {
        (false, false, false)
    };
    if esc {
        click = Some(Click::Esc);
    } else if enter {
        click = r.confirm.as_ref().map(|q| if q.focus == 0 { Click::Yes } else { Click::No });
    } else if tab {
        click = Some(Click::MoveFocus);
    }
    let [this, far, add, none, ..] = page.words();
    let heading = match v.far {
        None => t.tr(this).to_owned(),
        Some(n) => t.trf(far, &[n]),
    };
    let body = ui.vertical(|ui| {
        ui.label(egui::RichText::new(&heading).strong());
        ui.add_space(4.0);
        match access {
            Access::Wait => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(t.tr("users.loading_far"));
                });
                return;
            }
            Access::NotAdmin => {
                users::say_line(ui, t.tr("registry.only_admin"), c.dim);
                return;
            }
            Access::Down(e) => users::say_line(ui, e, c.err),
            Access::Open => {}
        }
        let idle = v.open && !r.write && r.asked == 0;
        ui.horizontal(|ui| {
            if users::button(ui, idle, None, ito_prefs::Mark::Refresh, t.tr("users.reload"), ito_prefs::Tone::Plain, "registry.reload") {
                click.get_or_insert(Click::Reload);
            }
            let can_add = r.forms[page.idx()].is_none() && v.open && !r.write;
            let words = t.tr(add);
            if users::button(ui, can_add, None, ito_prefs::Mark::Plus, words, ito_prefs::Tone::Plain, &format!("registry.{tag}.add.open")) {
                click.get_or_insert(Click::OpenAdd);
            }
            if r.asked > 0 {
                ui.spinner();
                ui.label(t.tr("users.loading"));
            }
        });
        if let Some(e) = &r.error {
            users::say_line(ui, e, c.err);
        }
        let can = r.can_write(v);
        let editing = r.forms[page.idx()].as_ref().and_then(|f| f.editing.clone());
        if r.forms[page.idx()].as_ref().is_some_and(|f| f.editing.is_none()) {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                form_ui(r, ui, t, page, can, &mut click);
            });
        }
        let listing = r.listing.clone();
        let mut drawn = editing.is_none();
        if let Some(l) = &listing {
            let empty = match page {
                Page::Accounts => l.accounts.is_empty(),
                Page::Databases => l.databases.is_empty(),
                Page::Hosts => l.hosts.is_empty(),
                Page::Buckets => l.buckets.is_empty(),
                Page::Mailboxes => l.mailboxes.is_empty(),
            };
            if empty && r.error.is_none() && r.asked == 0 {
                users::say_line(ui, t.tr(none), c.dim);
            }
            match page {
                Page::Accounts => {
                    for row in &l.accounts {
                        let here = editing.as_deref() == Some(row.name.as_str());
                        drawn |= here;
                        card(ui, |ui| {
                            ui.label(egui::RichText::new(&row.name).strong());
                            let user = t.trf("registry.user_line", &[&row.user]);
                            line(ui, &user);
                            secret_lines(ui, t, page, (&row.name, &row.secret, &row.state));
                            used_line(ui, t, &row.used_by);
                            row_end(r, ui, t, (page, &row.name), (can, here), &mut click);
                        });
                    }
                }
                Page::Databases => {
                    for row in &l.databases {
                        let here = editing.as_deref() == Some(row.name.as_str());
                        drawn |= here;
                        card(ui, |ui| {
                            ui.label(egui::RichText::new(&row.name).strong());
                            line(ui, &t.trf("registry.kind_line", &[kind_word(&row.kind), &where_word(&row.db)]));
                            if row.kind != "sqlite" {
                                // No tls in koyomi.toml is the engine's default, verify.
                                let tls = row.tls.as_deref().and_then(SqlTls::of).unwrap_or_default();
                                line(ui, &t.trf("registry.tls_line", &[tls.key()]));
                            }
                            // A SQL Server row is written by hand: it keeps its secret lines and is not edited here.
                            let fixed_by_hand = !matches!(row.kind.as_str(), "postgres" | "sqlite");
                            if fixed_by_hand {
                                match &row.secret {
                                    Some(s) => secret_lines(ui, t, page, (&row.name, s, &row.state)),
                                    None => line(ui, &t.trf("registry.password_line", &[state_word(t, &row.state)])),
                                }
                            } else if row.kind != "sqlite" {
                                line(ui, &t.trf("registry.password_line", &[state_word(t, &row.state)]));
                            }
                            used_line(ui, t, &row.used_by);
                            if fixed_by_hand {
                                let why = t.tr("registry.db_sqlserver_note");
                                users::say_line(ui, why, c.dim);
                                row_end_with(r, ui, t, (page, &row.name), (can, here), Some(why), &mut click);
                            } else {
                                row_end(r, ui, t, (page, &row.name), (can, here), &mut click);
                            }
                        });
                    }
                }
                Page::Hosts => {
                    for row in &l.hosts {
                        let here = editing.as_deref() == Some(row.name.as_str());
                        drawn |= here;
                        card(ui, |ui| {
                            ui.label(egui::RichText::new(&row.name).strong());
                            let kind = if row.kind == "winrm" { "WinRM" } else { "SSH" };
                            line(ui, &t.trf("registry.kind_line", &[kind, &chore::shown_address(&row.on)]));
                            for (key, v) in [
                                ("registry.host_key_line", &row.host_key),
                                ("registry.key_line", &row.key),
                                ("registry.cert_line", &row.cert),
                                ("registry.auth_line", &row.auth),
                            ] {
                                if let Some(v) = v.as_deref().filter(|v| !v.is_empty()) {
                                    line(ui, &t.trf(key, &[v]));
                                }
                            }
                            match &row.secret {
                                Some(s) => secret_lines(ui, t, page, (&row.name, s, &row.state)),
                                None => line(ui, &t.trf("registry.password_line", &[state_word(t, &row.state)])),
                            }
                            used_line(ui, t, &row.used_by);
                            row_end(r, ui, t, (page, &row.name), (can, here), &mut click);
                        });
                    }
                }
                Page::Buckets => {
                    for row in &l.buckets {
                        let here = editing.as_deref() == Some(row.name.as_str());
                        drawn |= here;
                        card(ui, |ui| {
                            ui.label(egui::RichText::new(&row.name).strong());
                            line(ui, &t.trf("registry.kind_line", &["S3", &row.url]));
                            for (key, v) in [
                                ("registry.region_line", &row.region),
                                ("registry.endpoint_line", &row.endpoint),
                                ("registry.key_id_line", &row.key_id),
                            ] {
                                if let Some(v) = v.as_deref().filter(|v| !v.is_empty()) {
                                    line(ui, &t.trf(key, &[v]));
                                }
                            }
                            if let Some(s) = &row.key_id_secret {
                                let state = row.key_id_state.as_deref().unwrap_or("");
                                secret_lines_as(ui, t, page, (&row.name, s, state), ("registry.key_id_secret_line", ".keyid"));
                            }
                            match &row.secret {
                                Some(s) => secret_lines(ui, t, page, (&row.name, s, &row.state)),
                                None => line(ui, &t.trf("registry.password_line", &[state_word(t, &row.state)])),
                            }
                            used_line(ui, t, &row.used_by);
                            row_end(r, ui, t, (page, &row.name), (can, here), &mut click);
                        });
                    }
                }
                Page::Mailboxes => {
                    for row in &l.mailboxes {
                        let here = editing.as_deref() == Some(row.name.as_str());
                        drawn |= here;
                        card(ui, |ui| {
                            ui.label(egui::RichText::new(&row.name).strong());
                            line(ui, &t.trf("registry.mail_line", &[&chore::shown_address(&row.mail)]));
                            secret_lines(ui, t, page, (&row.name, &row.secret, &row.state));
                            used_line(ui, t, &row.used_by);
                            row_end(r, ui, t, (page, &row.name), (can, here), &mut click);
                        });
                    }
                }
            }
        }
        // An editor with typed changes whose card is not there (the listing changed): it still shows.
        if !drawn {
            card(ui, |ui| form_ui(r, ui, t, page, can, &mut click));
        }
        if let Some(l) = &r.refusal {
            ui.add_space(6.0);
            users::say_line(ui, &l.text(t), c.err);
        }
    });
    ito_a11y::test_id(&body.response, &format!("registry.{tag}.page"));
    if let Some(q) = r.confirm.as_ref().filter(|q| q.page == page) {
        let (title, body) = remove_words(t, q);
        let buttons = [t.tr("registry.remove_yes").to_owned(), t.tr("users.cancel").to_owned()];
        let body = [body];
        let question = ito_confirm::Question {
            title: &title,
            body: &body,
            buttons: &buttons,
            marks: &[Some(ito_prefs::Mark::Trash), Some(ito_prefs::Mark::Close)],
            focus: Some(q.focus),
            primary: Some(0),
            danger: true,
            ..Default::default()
        };
        if let Some(choice) = ito_confirm::show(ui.ctx(), &question, &super::Browser::style()) {
            click = Some(if choice == 0 { Click::Yes } else { Click::No });
        }
    }
    click
}

fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}

fn line(ui: &mut egui::Ui, text: &str) {
    let r = ui.add(egui::Label::new(text).wrap());
    ito_a11y::say(&r, ito_a11y::Role::Label, text);
}

/// 「使っているジョブネット N」 and, when there are some, which.
fn used_line(ui: &mut egui::Ui, t: &ito_i18n::Table, used: &[String]) {
    line(ui, &t.trf("registry.used_line", &[&used.len().to_string()]));
    if !used.is_empty() {
        let text = nets_word(t, used);
        let r = ui.add(egui::Label::new(egui::RichText::new(&text).color(ito_theme::colors().dim)).wrap());
        ito_a11y::say(&r, ito_a11y::Role::Label, &text);
    }
}

/// The secret's name and state; for a missing one, the command that sets it, with a copy button.
fn secret_lines(ui: &mut egui::Ui, t: &ito_i18n::Table, page: Page, who: (&str, &str, &str)) {
    secret_lines_as(ui, t, page, who, ("registry.secret_line", ""));
}

/// [`secret_lines`] with the key of its words and a tail for the copy button's ID (a second secret of one entry).
fn secret_lines_as(
    ui: &mut egui::Ui,
    t: &ito_i18n::Table,
    page: Page,
    (name, secret, state): (&str, &str, &str),
    (words, tail): (&str, &str),
) {
    line(ui, &t.trf(words, &[secret, state_word(t, state)]));
    if state == "missing" {
        let cmd = secret_command(secret);
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::Label::new(egui::RichText::new(&cmd).monospace()).selectable(true));
            let id = format!("registry.{}.copy.{}{tail}", page.tag(), users::tid(name));
            if users::button(ui, true, None, ito_prefs::Mark::Copy, t.tr("users.copy"), ito_prefs::Tone::Plain, &id) {
                ui.ctx().copy_text(cmd.clone());
            }
        });
    }
}

/// The buttons under a card, or the form when it is this card's.
fn row_end(r: &mut Registry, ui: &mut egui::Ui, t: &ito_i18n::Table, who: (Page, &str), state: (bool, bool), click: &mut Option<Click>) {
    row_end_with(r, ui, t, who, state, None, click);
}

/// [`row_end`]; with `no_edit` (the reason) 編集 is off and shows it as its hover.
fn row_end_with(
    r: &mut Registry,
    ui: &mut egui::Ui,
    t: &ito_i18n::Table,
    (page, name): (Page, &str),
    (can, editing): (bool, bool),
    no_edit: Option<&str>,
    click: &mut Option<Click>,
) {
    if editing {
        form_ui(r, ui, t, page, can, click);
        return;
    }
    let id = users::tid(name);
    let tag = page.tag();
    let on = can && no_edit.is_none() && !r.forms[page.idx()].as_ref().is_some_and(Form::dirty);
    ui.horizontal_wrapped(|ui| {
        if users::button(
            ui,
            on,
            no_edit,
            ito_prefs::Mark::Pencil,
            t.tr("registry.edit"),
            ito_prefs::Tone::Plain,
            &format!("registry.{tag}.edit.{id}"),
        ) {
            click.get_or_insert(Click::OpenEdit(name.to_owned()));
        }
        if users::button(
            ui,
            can,
            None,
            ito_prefs::Mark::Trash,
            t.tr("registry.remove"),
            ito_prefs::Tone::Danger,
            &format!("registry.{tag}.remove.{id}"),
        ) {
            click.get_or_insert(Click::AskRemove(name.to_owned()));
        }
    });
}

/// A typed line of the form: its label, the tail of its test ID, its hint.
fn typed_line(ui: &mut egui::Ui, t: &ito_i18n::Table, id: &str, (label, tail, hint): (&str, &str, &str), text: &mut String) {
    ui.horizontal(|ui| {
        ui.label(t.tr(label));
        let resp = ui.add(egui::TextEdit::singleline(text).hint_text(t.tr(hint)).desired_width(320.0));
        ito_a11y::say(&resp, ito_a11y::Role::Label, t.tr(label));
        ito_a11y::test_id(&resp, &format!("{id}.{tail}"));
    });
}

/// The region of a bucket: typed, or picked from the usual S3 regions.
fn region_line(ui: &mut egui::Ui, t: &ito_i18n::Table, id: &str, text: &mut String) {
    ui.horizontal(|ui| {
        let label = t.tr("registry.region_field");
        ui.label(label);
        let rows = crate::ui::pick::region_rows(t);
        let field_id = format!("{id}.region");
        let look = ito_prefs::Look { c: ito_theme::colors(), q: "" };
        ito_prefs::ComboField::new(&field_id, ito_prefs::FieldWords::for_lang(t.tr("app.lang")))
            .name(label)
            .mono()
            .empty_ok()
            .hint(t.tr("registry.region_hint"))
            .width(320.0)
            .suggestions(&rows)
            .show(ui, look, text);
    });
}

/// The "connect and check" button of a host form, and what the host showed.
fn peek_ui(ui: &mut egui::Ui, t: &ito_i18n::Table, id: &str, f: &Form, open: bool, click: &mut Option<Click>) {
    let c = ito_theme::colors();
    let winrm = f.x.winrm;
    let target = f.peek_target();
    let waiting = f.peek.as_ref().is_some_and(|p| p.result.is_none());
    let on = f.peek_enabled(open);
    let why = if target.is_none() {
        t.tr("registry.peek_bad_target")
    } else {
        t.tr(if winrm { "registry.peek_hover_winrm" } else { "registry.peek_hover_ssh" })
    };
    ui.horizontal_wrapped(|ui| {
        if users::button(
            ui,
            on,
            Some(why),
            ito_prefs::Mark::Search,
            t.tr("registry.peek_button"),
            ito_prefs::Tone::Plain,
            &format!("{id}.peek"),
        ) {
            click.get_or_insert(Click::Peek);
        }
        if waiting {
            ui.spinner();
            users::say_line(ui, t.tr("registry.peek_waiting"), c.dim);
        }
    });
    match f.peek.as_ref().and_then(|p| p.result.as_ref()) {
        Some(Ok(value)) => {
            users::say_line(ui, &t.trf(if winrm { "registry.peek_got_winrm" } else { "registry.peek_got_ssh" }, &[value]), c.fg);
            let verdict = verdict(winrm, f.peek_field(), value);
            match verdict {
                Verdict::Same => users::say_line(ui, t.tr("registry.peek_same"), c.dim),
                Verdict::Empty => users::say_line(ui, t.tr("registry.peek_first"), c.wait),
                Verdict::Differs => users::say_line(ui, t.tr("registry.peek_differs"), c.wait),
            }
            if verdict != Verdict::Same
                && users::button(
                    ui,
                    true,
                    None,
                    ito_prefs::Mark::Check,
                    t.tr("registry.peek_use"),
                    ito_prefs::Tone::Plain,
                    &format!("{id}.peek.use"),
                )
            {
                click.get_or_insert(Click::UsePeek);
            }
        }
        Some(Err(e)) => users::say_line(ui, e, c.err),
        None => {}
    }
}

/// The lang key of the button's hover: why a test cannot be asked yet, or what it does.
fn db_test_why(f: &Form) -> &'static str {
    if f.db_test_url().is_none() {
        "registry.db_test_fields"
    } else if f.db_test_ask().is_none() {
        "registry.db_test_need"
    } else {
        "registry.db_test_hover"
    }
}

/// The 接続テスト button of a database form, and what the test said.
fn db_test_ui(ui: &mut egui::Ui, t: &ito_i18n::Table, id: &str, f: &Form, can: bool, click: &mut Option<Click>) {
    let c = ito_theme::colors();
    let waiting = f.db_test.as_ref().is_some_and(|d| d.result.is_none());
    let on = f.db_test_enabled(can) && !f.stale;
    let why = t.tr(db_test_why(f));
    ui.horizontal_wrapped(|ui| {
        if users::button(
            ui,
            on,
            Some(why),
            ito_prefs::Mark::Search,
            t.tr("registry.db_test_button"),
            ito_prefs::Tone::Plain,
            &format!("{id}.db_test"),
        ) {
            click.get_or_insert(Click::DbTest);
        }
        if waiting {
            ui.spinner();
            users::say_line(ui, t.tr("registry.db_test_waiting"), c.dim);
        }
    });
    match f.db_test.as_ref().and_then(|d| d.result.as_ref()) {
        Some(Ok(_)) => users::say_line(ui, t.tr("registry.db_test_ok"), c.fg),
        Some(Err(e)) => users::say_line(ui, e, c.err),
        None => {}
    }
}

/// The password line of every page: the hidden field, the note under it, and (an SSH host being edited) the box that
/// leaves the password out.
// ito#231: `ito_prefs::secret_field` has no accessible name or test ID, so the line is drawn here until koyomi takes
// a rev that has them.
fn password_ui(ui: &mut egui::Ui, t: &ito_i18n::Table, id: &str, f: &mut Form) {
    let label = t.tr(if f.page == Page::Buckets { "registry.s3_key_field" } else { "registry.password_field" });
    ui.horizontal(|ui| {
        ui.label(label);
        let resp =
            ui.add_enabled(!f.drop_password, egui::TextEdit::singleline(f.password.as_mut_string()).password(true).desired_width(240.0));
        ito_a11y::say(&resp, ito_a11y::Role::Label, label);
        ito_a11y::test_id(&resp, &format!("{id}.password"));
    });
    if f.can_drop_password() {
        let mut drop = f.drop_password;
        let resp = ui.checkbox(&mut drop, t.tr("registry.drop_password"));
        ito_a11y::test_id(&resp, &format!("{id}.drop_password"));
        if resp.changed() {
            f.set_drop_password(drop);
        }
    }
    let note = egui::RichText::new(t.tr(f.password_note())).color(ito_theme::colors().dim);
    ui.add(egui::Label::new(note).wrap());
}

/// The fields of the database form: the kind, then the server, port, database name, user ID and password of
/// PostgreSQL (TLS under 詳しい設定), or the file of SQLite. `browsed` is set when 参照… is clicked.
fn database_ui(ui: &mut egui::Ui, t: &ito_i18n::Table, id: &str, f: &mut Form, local: bool, browsed: &mut Option<Chosen>) {
    let look = ito_prefs::Look { c: ito_theme::colors(), q: "" };
    ui.horizontal(|ui| {
        ui.label(t.tr("registry.db_kind_field"));
        let options = [(DbKind::Postgres, "PostgreSQL"), (DbKind::Sqlite, "SQLite")];
        let (to, resp) = ito_prefs::seg_named(ui, look, &format!("{id}.kind"), t.tr("registry.db_kind_field"), f.db.kind, &options);
        ito_a11y::test_id(&resp, &format!("{id}.kind"));
        if let Some(kind) = to {
            f.db.kind = kind;
            if kind == DbKind::Postgres && f.db.port.trim().is_empty() {
                f.db.port = chore::POSTGRES_PORT.to_string();
            }
        }
    });
    let text = |ui: &mut egui::Ui, label: &str, hint: &str, tail: &str, width: f32, value: &mut String| {
        ui.horizontal(|ui| {
            ui.label(t.tr(label));
            let mut edit = egui::TextEdit::singleline(value).desired_width(width);
            if !hint.is_empty() {
                edit = edit.hint_text(t.tr(hint));
            }
            let resp = ui.add(edit);
            ito_a11y::say(&resp, ito_a11y::Role::Label, t.tr(label));
            ito_a11y::test_id(&resp, &format!("{id}.{tail}"));
        });
    };
    if f.is_sqlite() {
        ui.horizontal(|ui| {
            ui.label(t.tr("registry.db_file_field"));
            let resp = ui.add(egui::TextEdit::singleline(&mut f.db.file).hint_text(t.tr("registry.db_file_hint")).desired_width(320.0));
            ito_a11y::say(&resp, ito_a11y::Role::Label, t.tr("registry.db_file_field"));
            ito_a11y::test_id(&resp, &format!("{id}.file"));
            // The file is on the engine's PC: it can be picked only when that is this PC.
            if local {
                let b = ito_prefs::button_with(ui, look, t.tr("pick.browse"), ito_prefs::Tone::Plain);
                ito_a11y::test_id(&b, &format!("{id}.file.browse"));
                if b.clicked() {
                    *browsed = Some(Chosen::SqliteFile);
                }
            }
        });
        return;
    }
    text(ui, "registry.db_host_field", "", "host", 240.0, &mut f.db.host);
    text(ui, "registry.db_port_field", "registry.db_port_hint", "port", 80.0, &mut f.db.port);
    text(ui, "registry.db_name_field", "", "database", 240.0, &mut f.db.database);
    text(ui, "registry.db_user_field", "", "user", 240.0, &mut f.db.user);
    password_ui(ui, t, id, f);
    // The TLS choice is open from the start when it is not the usual one.
    egui::CollapsingHeader::new(t.tr("registry.db_more")).id_salt(format!("{id}.more")).default_open(f.tls != SqlTls::Verify).show(
        ui,
        |ui| {
            ui.horizontal(|ui| {
                ui.label(t.tr("registry.tls_field"));
                for s in SqlTls::ALL {
                    let resp = ui.radio(f.tls == s, s.key());
                    ito_a11y::test_id(&resp, &format!("{id}.tls_{}", s.key()));
                    if resp.clicked() {
                        f.tls = s;
                    }
                }
            });
        },
    );
}

#[cfg(test)]
impl Form {
    /// Types the address `text` into the parts of this form. A host's text is split plainly as `[user@]host[:port]`
    /// (with or without its scheme, and with no check), so a test can type an address that is not good.
    pub(super) fn type_address(&mut self, text: &str) {
        if self.page == Page::Hosts {
            let t = text.split_once("://").map_or(text, |(_, r)| r);
            let (user, rest) = t.rsplit_once('@').unwrap_or(("", t));
            let (host, port) = match rest.rsplit_once(':') {
                Some((h, p)) if !rest.ends_with(']') && p.chars().all(|c| c.is_ascii_digit()) => (h, p),
                _ => (rest, ""),
            };
            let port = if port.is_empty() { default_port(self.page, self.x.winrm).to_string() } else { port.to_owned() };
            self.addr = AddrFields { user: user.to_owned(), host: host.to_owned(), port, ..AddrFields::default() };
        } else {
            self.addr = read_addr(self.page, text).unwrap_or_else(|| panic!("does not read in parts: {text}"));
        }
        self.raw = false;
    }
}

/// One typed line of an address in parts: its label key, its test ID tail, its hint (as shown) and its width. Returns
/// whether it changed.
fn part_line(
    ui: &mut egui::Ui,
    t: &ito_i18n::Table,
    id: &str,
    (label, tail, hint, width): (&str, &str, &str, f32),
    value: &mut String,
) -> bool {
    ui.horizontal(|ui| {
        ui.label(t.tr(label));
        let resp = ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(width));
        ito_a11y::say(&resp, ito_a11y::Role::Label, t.tr(label));
        ito_a11y::test_id(&resp, &format!("{id}.{tail}"));
        resp.changed()
    })
    .inner
}

/// The address lines of the account, host, bucket and mailbox forms: the parts (user, host and port; server, port,
/// user and folder; bucket and folder), or the one line a row that did not read in parts is held as.
fn address_ui(ui: &mut egui::Ui, t: &ito_i18n::Table, id: &str, f: &mut Form) {
    let page = f.page;
    if page != Page::Accounts && !f.raw {
        let winrm = f.x.winrm;
        let mut moved = false;
        // An empty port is the kind's own, so that is the hint.
        let port = default_port(page, winrm).to_string();
        match page {
            Page::Hosts => {
                let hint = t.tr(if winrm { "registry.addr_user_hint_winrm" } else { "registry.addr_user_hint_ssh" });
                moved |= part_line(ui, t, id, ("registry.addr_user", "user", hint, 240.0), &mut f.addr.user);
                moved |= part_line(ui, t, id, ("registry.addr_host", "host", t.tr("registry.addr_host_hint"), 240.0), &mut f.addr.host);
                moved |= part_line(ui, t, id, ("registry.addr_port", "port", &port, 80.0), &mut f.addr.port);
            }
            Page::Mailboxes => {
                moved |= part_line(ui, t, id, ("registry.addr_server", "host", t.tr("registry.addr_server_hint"), 240.0), &mut f.addr.host);
                moved |= part_line(ui, t, id, ("registry.addr_port", "port", &port, 80.0), &mut f.addr.port);
                moved |=
                    part_line(ui, t, id, ("registry.addr_user", "user", t.tr("registry.addr_mail_user_hint"), 240.0), &mut f.addr.user);
                let hint = t.tr("registry.addr_mail_folder_hint");
                moved |= part_line(ui, t, id, ("registry.addr_folder", "folder", hint, 240.0), &mut f.addr.folder);
            }
            _ => {
                moved |=
                    part_line(ui, t, id, ("registry.addr_bucket", "bucket", t.tr("registry.addr_bucket_hint"), 240.0), &mut f.addr.bucket);
                let hint = t.tr("registry.addr_s3_folder_hint");
                moved |= part_line(ui, t, id, ("registry.addr_folder", "folder", hint, 240.0), &mut f.addr.folder);
            }
        }
        if moved {
            // What a host showed is for the address it was asked of.
            f.peek = None;
        }
        return;
    }
    ui.horizontal(|ui| {
        let (label, hint) = match page {
            Page::Accounts => ("registry.user_field", "registry.user_hint"),
            Page::Databases => ("", ""),
            Page::Hosts if f.x.winrm => ("registry.host_on", "registry.host_on_hint_winrm"),
            Page::Hosts => ("registry.host_on", "registry.host_on_hint_ssh"),
            Page::Buckets => ("registry.bucket_url", "registry.bucket_url_hint"),
            Page::Mailboxes => ("registry.mailbox_mail", "registry.mailbox_mail_hint"),
        };
        ui.label(t.tr(label));
        if page == Page::Hosts && !f.scheme_less {
            ui.label(egui::RichText::new(scheme(f.x.winrm)).monospace());
        }
        let resp = ui.add(egui::TextEdit::singleline(&mut f.target).hint_text(t.tr(hint)).desired_width(320.0));
        ito_a11y::say(&resp, ito_a11y::Role::Label, t.tr(label));
        ito_a11y::test_id(&resp, &format!("{id}.target"));
        if resp.changed() {
            // What a host showed is for the address it was asked of.
            f.peek = None;
        }
    });
    if f.raw {
        ui.add(egui::Label::new(egui::RichText::new(t.tr("registry.addr_raw_note")).color(ito_theme::colors().dim)).wrap());
    }
}

/// The add / edit form of `page`.
fn form_ui(r: &mut Registry, ui: &mut egui::Ui, t: &ito_i18n::Table, page: Page, can: bool, click: &mut Option<Click>) {
    let busy = r.write;
    let local = r.local;
    // The page may ask the engine (a read; it needs no listing).
    let v_open = r.open;
    let Some(f) = r.forms[page.idx()].as_mut() else { return };
    let tag = page.tag();
    let id = format!("registry.{tag}.form");
    // A file the person picked lands in the form that was up at the click ([`Registry::poll_browse`]).
    let mut browsed: Option<Chosen> = None;
    ui.add_enabled_ui(!busy, |ui| {
        ui.horizontal(|ui| {
            ui.label(t.tr("registry.name_field"));
            let adding = f.editing.is_none();
            let resp = ui.add_enabled(adding, egui::TextEdit::singleline(&mut f.name).desired_width(200.0));
            ito_a11y::say(&resp, ito_a11y::Role::Label, t.tr("registry.name_field"));
            ito_a11y::test_id(&resp, &format!("{id}.name"));
        });
        if page == Page::Hosts {
            ui.horizontal(|ui| {
                ui.label(t.tr("registry.host_kind"));
                let look = ito_prefs::Look { c: ito_theme::colors(), q: "" };
                let options = [(false, "SSH"), (true, "WinRM")];
                let (to, resp) = ito_prefs::seg_named(ui, look, &format!("{id}.kind"), t.tr("registry.host_kind"), f.x.winrm, &options);
                ito_a11y::test_id(&resp, &format!("{id}.kind"));
                if let Some(winrm) = to {
                    f.set_winrm(winrm);
                }
            });
        }
        if page == Page::Databases {
            database_ui(ui, t, &id, f, local, &mut browsed);
            f.drop_stale_db_test();
            db_test_ui(ui, t, &id, f, can && v_open, click);
        } else {
            address_ui(ui, t, &id, f);
        }
        f.drop_stale_peek();
        match page {
            Page::Hosts if !f.x.winrm => {
                typed_line(ui, t, &id, ("registry.host_key_field", "host_key", "registry.host_key_hint"), &mut f.x.host_key);
                peek_ui(ui, t, &id, f, v_open, click);
                ui.horizontal(|ui| {
                    ui.label(t.tr("registry.key_file"));
                    let resp = ui.add(egui::TextEdit::singleline(&mut f.x.key).desired_width(320.0));
                    ito_a11y::say(&resp, ito_a11y::Role::Label, t.tr("registry.key_file"));
                    ito_a11y::test_id(&resp, &format!("{id}.key"));
                    // The key file is on the engine's PC: it can be picked only when that is this PC.
                    if local {
                        let b = ito_prefs::button_with(
                            ui,
                            ito_prefs::Look { c: ito_theme::colors(), q: "" },
                            t.tr("pick.browse"),
                            ito_prefs::Tone::Plain,
                        );
                        ito_a11y::test_id(&b, &format!("{id}.key.browse"));
                        if b.clicked() {
                            browsed = Some(Chosen::HostKey);
                        }
                    }
                });
            }
            Page::Hosts => {
                typed_line(ui, t, &id, ("registry.cert_field", "cert", "registry.cert_hint"), &mut f.x.cert);
                peek_ui(ui, t, &id, f, v_open, click);
                ui.horizontal(|ui| {
                    ui.label(t.tr("registry.auth_field"));
                    let look = ito_prefs::Look { c: ito_theme::colors(), q: "" };
                    // Shown by the protocol's own name, as the job form does (`Basic`, `NTLM`).
                    let options = WinrmAuth::ALL.map(|a| (a, a.name()));
                    let (to, resp) = ito_prefs::seg_named(ui, look, &format!("{id}.auth"), t.tr("registry.auth_field"), f.x.auth, &options);
                    ito_a11y::test_id(&resp, &format!("{id}.auth"));
                    if let Some(a) = to {
                        f.x.auth = a;
                    }
                });
            }
            Page::Buckets => {
                typed_line(ui, t, &id, ("registry.key_id_field", "key_id", "registry.key_id_hint"), &mut f.x.key_id);
            }
            _ => {}
        }
        if page != Page::Databases {
            password_ui(ui, t, &id, f);
        }
        if page == Page::Buckets {
            region_line(ui, t, &id, &mut f.x.region);
            typed_line(ui, t, &id, ("registry.endpoint_field", "endpoint", "registry.endpoint_hint"), &mut f.x.endpoint);
        }
    });
    if let (Some(what), None) = (browsed, &r.browse) {
        let (page, gen, link) = (page, f.gen, r.link.clone());
        let dialog = match what {
            Chosen::HostKey => ito_dialog::Dialog::new().title(t.tr("registry.key_file")),
            Chosen::SqliteFile => {
                let mut d = ito_dialog::Dialog::new()
                    .title(t.tr("registry.db_sqlite_title"))
                    .filter(ito_dialog::Filter::new(t.tr("registry.db_sqlite_filter"), &["db", "sqlite", "sqlite3", "db3"]))
                    .filter(ito_dialog::Filter::new(t.tr("registry.db_all_filter"), &["*"]));
                if let Some(dir) = start_dir(&f.db.file) {
                    d = d.directory(dir);
                }
                d
            }
        };
        let rx = ito_dialog::spawn(move || dialog.open_file().into_iter().collect());
        r.browse = Some(Browse { rx, page, gen, link, what });
    }
    if let Some(why) = &f.error {
        users::say_line(ui, &why.text(t), ito_theme::colors().err);
    }
    if f.stale {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(t.tr("registry.stale_note")).color(ito_theme::colors().err));
            if users::button(ui, can, None, ito_prefs::Mark::Check, t.tr("users.seen"), ito_prefs::Tone::Plain, &format!("{id}.seen")) {
                click.get_or_insert(Click::Seen);
            }
        });
    }
    let on = can && f.dirty() && !f.stale;
    ui.horizontal(|ui| {
        if users::button(ui, on, None, ito_prefs::Mark::Save, t.tr("users.save"), ito_prefs::Tone::Primary, &format!("{id}.save")) {
            click.get_or_insert(Click::Save);
        }
        if users::button(ui, true, None, ito_prefs::Mark::Close, t.tr("users.cancel"), ito_prefs::Tone::Plain, &format!("{id}.cancel")) {
            click.get_or_insert(Click::CloseForm);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use koyomi_core::role::Role;

    const OPEN: View = View { far: None, open: true };

    fn body() -> RegistryBody {
        RegistryBody {
            sha256: "base1".into(),
            accounts: vec![
                AccountRow {
                    name: "ops".into(),
                    user: "CORP\\ops".into(),
                    secret: "ops".into(),
                    state: "ok".into(),
                    used_by: ["a", "b", "c", "d", "e"].map(String::from).to_vec(),
                },
                AccountRow {
                    name: "free".into(),
                    user: "CORP\\free".into(),
                    secret: "free".into(),
                    state: "missing".into(),
                    used_by: vec![],
                },
            ],
            databases: vec![DatabaseRow {
                name: "main".into(),
                kind: "sqlserver".into(),
                db: "sqlserver://u@h/d".into(),
                secret: Some("main".into()),
                tls: None,
                state: "ok".into(),
                used_by: vec!["a".into()],
            }],
            ..RegistryBody::default()
        }
    }

    /// A page that has asked for and taken the listing.
    fn loaded() -> Registry {
        let mut r = Registry::default();
        r.touch(1, Page::Accounts);
        r.ask();
        r.listing_answer(Ok(body().to_toml()));
        r
    }

    #[test]
    fn the_pages_follow_the_users_page_access() {
        assert_eq!(users::access(false, None, None), users::Access::Open);
        assert_eq!(users::access(true, None, None), users::Access::Wait);
        assert_eq!(users::access(true, Some(Role::Edit), None), users::Access::NotAdmin);
        assert_eq!(users::access(true, Some(Role::Admin), None), users::Access::Open);
    }

    #[test]
    fn a_listing_is_taken_and_a_bad_or_old_engine_answer_is_the_pages_error_without_a_loop() {
        let r = loaded();
        assert_eq!(r.listing.as_ref(), Some(&body()));
        assert_eq!(
            (r.names(Page::Accounts), r.names(Page::Databases)),
            (vec!["ops".to_owned(), "free".to_owned()], vec!["main".to_owned()])
        );
        // An engine that does not know `Registry` answers an error: shown, and not asked again.
        let mut r = Registry::default();
        r.touch(1, Page::Accounts);
        assert!(r.need);
        r.ask();
        r.listing_answer(Err("E1000: unknown request".into()));
        assert_eq!(r.error(), Some("E1000: unknown request"));
        assert!(r.listing.is_none() && r.asked() == 0);
        r.touch(2, Page::Accounts);
        assert!(!r.need, "the next frame must not ask again");
        // A reply that is not a listing is an error too.
        let mut r = Registry::default();
        r.ask();
        r.listing_answer(Ok("sha256 = [".into()));
        assert!(r.error().is_some() && r.listing.is_none());
    }

    #[test]
    fn leaving_and_coming_back_asks_for_the_listing_again() {
        let mut r = loaded();
        r.touch(2, Page::Accounts);
        assert!(!r.need);
        r.touch(9, Page::Accounts);
        assert!(r.need);
    }

    #[test]
    fn removing_always_asks_and_only_the_ask_naming_nets_forces_it() {
        let mut r = loaded();
        // Without the ask, Yes sends nothing.
        assert!(matches!(r.act(Click::Yes, Page::Accounts, &OPEN), Do::Nothing));
        assert!(matches!(r.act(Click::AskRemove("ops".into()), Page::Accounts, &OPEN), Do::Nothing));
        let c = r.confirm.clone().unwrap();
        assert_eq!((c.name.as_str(), c.used_by.len()), ("ops", 5));
        // No closes the question and sends nothing.
        assert!(matches!(r.act(Click::No, Page::Accounts, &OPEN), Do::Nothing));
        assert!(r.confirm.is_none() && !r.write());
        r.act(Click::AskRemove("ops".into()), Page::Accounts, &OPEN);
        match r.act(Click::Yes, Page::Accounts, &OPEN) {
            Do::Send(Request::AccountRemove { name, base_sha256, force }) => {
                assert_eq!((name.as_str(), base_sha256.as_str(), force), ("ops", "base1", true));
            }
            other => panic!("{other:?}"),
        }
        assert!(r.write());
        // An entry no net names is asked about too, and goes without force.
        let mut r = loaded();
        r.act(Click::AskRemove("free".into()), Page::Accounts, &OPEN);
        assert!(r.confirm.is_some());
        match r.act(Click::Yes, Page::Accounts, &OPEN) {
            Do::Send(Request::AccountRemove { force, .. }) => assert!(!force),
            other => panic!("{other:?}"),
        }
        let mut r = loaded();
        r.act(Click::AskRemove("main".into()), Page::Databases, &OPEN);
        match r.act(Click::Yes, Page::Databases, &OPEN) {
            Do::Send(Request::DatabaseRemove { name, force, .. }) => assert_eq!((name.as_str(), force), ("main", true)),
            other => panic!("{other:?}"),
        }
        // Esc closes the question before anything else.
        let mut r = loaded();
        r.act(Click::AskRemove("free".into()), Page::Accounts, &OPEN);
        assert!(r.wants_esc(Page::Accounts) && !r.wants_esc(Page::Databases));
        r.act(Click::Esc, Page::Accounts, &OPEN);
        assert!(r.confirm.is_none());
        // A form left open on the other page does not take this page's Esc (it could not close it).
        r.act(Click::OpenAdd, Page::Accounts, &OPEN);
        assert!(r.wants_esc(Page::Accounts) && !r.wants_esc(Page::Databases));
        r.act(Click::Esc, Page::Accounts, &OPEN);
        assert!(!r.wants_esc(Page::Accounts));
    }

    fn conn_body() -> RegistryBody {
        RegistryBody {
            sha256: "base1".into(),
            hosts: vec![
                HostRow {
                    name: "web1".into(),
                    on: "ssh://ops@web1:2222/".into(),
                    kind: "ssh".into(),
                    host_key: Some("SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU".into()),
                    state: "ok".into(),
                    used_by: vec!["a".into()],
                    ..HostRow::default()
                },
                HostRow {
                    name: "pc".into(),
                    on: "winrm://.\\u@pc:5986".into(),
                    kind: "winrm".into(),
                    secret: Some("pc".into()),
                    state: "ok".into(),
                    ..HostRow::default()
                },
            ],
            buckets: vec![BucketRow {
                name: "logs".into(),
                url: "s3://logs/daily/".into(),
                key_id: Some("AKIAEXAMPLE".into()),
                region: Some("ap-northeast-1".into()),
                secret: Some("logs".into()),
                state: "ok".into(),
                ..BucketRow::default()
            }],
            mailboxes: vec![MailboxRow {
                name: "billing".into(),
                mail: "imaps://u@mail.example.com/INBOX".into(),
                secret: "billing".into(),
                state: "ok".into(),
                used_by: vec!["a".into()],
            }],
            ..RegistryBody::default()
        }
    }

    fn conn_loaded(page: Page) -> Registry {
        let mut r = Registry::default();
        r.touch(1, page);
        r.ask();
        r.listing_answer(Ok(conn_body().to_toml()));
        r
    }

    /// The scheme is the seg's; the rest is kept as it was typed, whatever its case.
    #[test]
    fn the_host_forms_scheme_is_stripped_and_rebuilt_without_touching_the_rest() {
        for (on, winrm, rest) in [
            ("SSH://web1", false, "web1"),
            ("ssh://u@web1:2222/", false, "u@web1:2222/"),
            ("winrm://DOMAIN\\u@pc", true, "DOMAIN\\u@pc"),
            ("winrm://.\\u@pc:5986", true, ".\\u@pc:5986"),
        ] {
            assert_eq!(split_on(on), (Some(winrm), rest.to_owned()), "{on}");
            let row = HostRow { name: "h".into(), on: on.into(), kind: if winrm { "winrm" } else { "ssh" }.into(), ..HostRow::default() };
            let f = Form::edit_host(&row);
            assert_eq!(f.is_winrm(), winrm);
            // Read in parts, the row is sent back as it was stored; held whole, the scheme is the seg's.
            if f.raw {
                assert_eq!(f.host_on(), format!("{}{rest}", scheme(winrm)), "{on}");
            } else {
                assert_eq!(f.address().unwrap(), on, "{on}");
            }
        }
        assert_eq!(split_on("web1"), (None, "web1".to_owned()));
    }

    #[test]
    fn the_host_bucket_and_mailbox_forms_build_their_requests() {
        let mut r = conn_loaded(Page::Hosts);
        assert!(matches!(r.act(Click::OpenEdit("web1".into()), Page::Hosts, &OPEN), Do::Nothing));
        let f = r.forms[Page::Hosts.idx()].as_ref().unwrap();
        assert!(!f.dirty());
        match f.request("base1", &[]) {
            Ok(Request::HostPut { name, on, password, .. }) => {
                assert_eq!((name.as_str(), on.as_str(), password), ("web1", "ssh://ops@web1:2222/", None));
            }
            other => panic!("{other:?}"),
        }
        // An empty form is refused, and a WinRM host needs its secret.
        let f = Form::add(Page::Hosts);
        assert!(f.request("b", &[]).is_err());
        let mut f = Form::edit_host(&conn_body().hosts[1]);
        f.secret.clear();
        assert_eq!(f.request("b", &[]).err(), Some(Line::PasswordEmpty));
        // A bucket and a mailbox.
        let f = Form::edit_bucket(&conn_body().buckets[0]);
        match f.request("b", &[]) {
            Ok(Request::BucketPut { name, url, password, password_value, .. }) => {
                assert_eq!((name.as_str(), url.as_str(), password.as_deref()), ("logs", "s3://logs/daily/", Some("secret:logs")));
                assert!(password_value.is_none());
            }
            other => panic!("{other:?}"),
        }
        let f = Form::edit_mailbox(&conn_body().mailboxes[0]);
        assert!(matches!(f.request("b", &[]), Ok(Request::MailboxPut { .. })));
        let mut f = Form::add(Page::Mailboxes);
        f.name = "billing".into();
        assert_eq!(f.request("b", &["billing".to_owned()]).err(), Some(Line::NameExists("billing".into())));
    }

    #[test]
    fn removing_a_host_bucket_or_mailbox_asks_and_forces_only_when_a_net_names_it() {
        for (page, name, used) in
            [(Page::Hosts, "web1", true), (Page::Hosts, "pc", false), (Page::Buckets, "logs", false), (Page::Mailboxes, "billing", true)]
        {
            let mut r = conn_loaded(page);
            assert!(matches!(r.act(Click::Yes, page, &OPEN), Do::Nothing));
            r.act(Click::AskRemove(name.into()), page, &OPEN);
            assert_eq!(r.confirm.as_ref().map(|c| c.used_by.is_empty()), Some(!used), "{name}");
            let Do::Send(req) = r.act(Click::Yes, page, &OPEN) else { panic!() };
            let (n, force) = match req {
                Request::HostRemove { name, force, .. }
                | Request::BucketRemove { name, force, .. }
                | Request::MailboxRemove { name, force, .. } => (name, force),
                other => panic!("{other:?}"),
            };
            assert_eq!((n.as_str(), force), (name, used));
        }
    }

    #[test]
    fn the_answers_of_the_six_requests_route_to_their_pages() {
        let mut r = conn_loaded(Page::Hosts);
        let reqs = [
            Request::HostPut {
                name: "web1".into(),
                on: "ssh://x".into(),
                host_key: None,
                key: None,
                password: None,
                cert: None,
                auth: None,
                base_sha256: "base1".into(),
                password_value: None,
            },
            Request::HostRemove { name: "web1".into(), base_sha256: "base1".into(), force: false },
            Request::BucketPut {
                name: "logs".into(),
                url: "s3://logs/".into(),
                key_id: None,
                password: Some("secret:logs".into()),
                region: None,
                endpoint: None,
                base_sha256: "base1".into(),
                password_value: None,
            },
            Request::BucketRemove { name: "logs".into(), base_sha256: "base1".into(), force: false },
            Request::MailboxPut {
                name: "billing".into(),
                mail: "imaps://bill@mail".into(),
                password: Some("secret:bill".into()),
                base_sha256: "base1".into(),
                password_value: None,
            },
            Request::MailboxRemove { name: "billing".into(), base_sha256: "base1".into(), force: false },
        ];
        for req in &reqs {
            r.write = true;
            let mut next = conn_body();
            next.sha256 = "base2".into();
            let done = r.answered(req, Ok(next.to_toml()));
            assert!(!done.reload && !r.write(), "{req:?}");
            assert_eq!(
                r.listing.as_ref().map(|l| (l.sha256.as_str(), l.hosts.len(), l.buckets.len(), l.mailboxes.len())),
                Some(("base2", 2, 1, 1))
            );
        }
        r.write = true;
        let done = r.answered(&reqs[1], Err(msg::line(msg::CONFIG_CHANGED, &[])));
        assert!(done.reload && r.refusal() == Some(&Line::BaseChanged));
    }

    #[test]
    fn the_ask_names_a_few_nets_then_the_rest_and_the_error_codes() {
        let (t, _) = crate::i18n::table("ja");
        let used: Vec<String> = ["a", "b", "c", "d", "e"].map(String::from).to_vec();
        assert_eq!(nets_word(&t, &used), "a, b, c ほか 2 件");
        assert_eq!(nets_word(&t, &used[..2]), "a, b");
        let c = Confirm { page: Page::Accounts, name: "ops".into(), used_by: used.clone(), focus: 1 };
        let (_, body) = remove_words(&t, &c);
        assert!(body.contains("ops") && body.contains("a, b, c ほか 2 件") && body.contains("E3294"), "{body}");
        let c = Confirm { page: Page::Databases, name: "main".into(), used_by: used, focus: 1 };
        assert!(remove_words(&t, &c).1.contains("E3295"));
        let c = Confirm { page: Page::Databases, name: "main".into(), used_by: vec![], focus: 1 };
        assert!(!remove_words(&t, &c).1.contains("E32"));
    }

    #[test]
    fn a_reply_is_the_new_listing_and_an_error_is_shown_and_reloads() {
        let mut r = loaded();
        let req = Request::AccountRemove { name: "free".into(), base_sha256: "base1".into(), force: false };
        r.write = true;
        let mut next = body();
        next.sha256 = "base2".into();
        next.accounts.pop();
        let done = r.answered(&req, Ok(next.to_toml()));
        assert!(!done.reload && !r.write());
        assert_eq!(r.listing.as_ref().map(|l| (l.sha256.as_str(), l.accounts.len())), Some(("base2", 1)));
        // A file changed under the page (E2247) and an entry a net names (E2250).
        r.write = true;
        let done = r.answered(&req, Err(msg::line(msg::CONFIG_CHANGED, &[])));
        assert!(done.reload && r.refusal() == Some(&Line::BaseChanged));
        r.write = true;
        let in_use = msg::line(msg::REGISTRY_IN_USE, &["free", "a"]);
        let done = r.answered(&req, Err(in_use.clone()));
        assert!(done.reload && r.refusal() == Some(&Line::Raw(in_use)));
        // A reply that cannot be read keeps the old listing and reloads.
        r.write = true;
        let done = r.answered(&req, Ok("sha256 = [".into()));
        assert!(done.reload && r.refusal() == Some(&Line::ReplyBad));
        assert!(r.listing.is_some());
    }

    #[test]
    fn an_account_form_is_checked_before_it_is_sent_and_sends_the_typed_password_once() {
        let taken = vec!["ops".to_owned()];
        let mut f = Form::add(Page::Accounts);
        f.name = "batch".into();
        f.target = "CORP\\batch".into();
        // Nothing typed: refused before anything is sent.
        assert_eq!(f.request("base1", &taken).unwrap_err(), Line::PasswordEmpty);
        f.password.as_mut_string().push_str(PW);
        match f.request("base1", &taken) {
            Ok(Request::AccountPut { name, user, password, base_sha256, password_value }) => {
                assert_eq!((name.as_str(), user.as_str(), base_sha256.as_str()), ("batch", "CORP\\batch", "base1"));
                assert!(password.is_none(), "a typed password names no secret: the engine picks it");
                assert_eq!(password_value.as_ref().map(Hidden::expose), Some(PW));
            }
            other => panic!("{other:?}"),
        }
        let mut bad = Form::add(Page::Accounts);
        bad.name = "a/b".into();
        bad.target = "CORP\\x".into();
        assert!(matches!(bad.request("b", &taken), Err(Line::Raw(_))));
        bad.name = "ops".into();
        assert_eq!(bad.request("b", &taken).unwrap_err(), Line::NameExists("ops".into()));
        bad.name = "new".into();
        bad.target = "bad user".into();
        bad.password.as_mut_string().push_str(PW);
        assert!(matches!(bad.request("b", &taken), Err(Line::Raw(_))));
        bad.target = "CORP\\x".into();
        bad.password = Hidden::default();
        assert_eq!(bad.request("b", &taken).unwrap_err(), Line::PasswordEmpty);
        // Editing keeps the name (it is not checked against the taken ones), and an empty field sends the row's own
        // secret back.
        let mut e = Form::edit_account(&body().accounts[0]);
        assert!(!e.dirty());
        match e.request("b", &taken) {
            Ok(Request::AccountPut { password, password_value, .. }) => {
                assert_eq!(password.as_deref(), Some("secret:ops"));
                assert!(password_value.is_none());
            }
            other => panic!("{other:?}"),
        }
        e.target = "CORP\\ops2".into();
        assert!(e.dirty());
    }

    const PW: &str = "pw-not-to-show";

    /// A form's value is hidden wherever the form is printed, and a request prints no value either.
    #[test]
    fn the_typed_password_is_not_printed_by_a_form_or_a_request() {
        let mut f = Form::edit_account(&body().accounts[0]);
        f.password.as_mut_string().push_str(PW);
        assert!(!format!("{f:?}").contains(PW));
        let req = f.request("b", &[]).unwrap();
        assert!(!format!("{req:?}").contains(PW));
        assert!(format!("{f:?}").contains("***"));
    }

    /// Typing a password makes the form changed, with nothing else touched.
    #[test]
    fn a_typed_password_alone_makes_the_form_dirty() {
        for page in [Page::Accounts, Page::Hosts, Page::Buckets, Page::Mailboxes] {
            let mut f = match page {
                Page::Accounts => Form::edit_account(&AccountRow {
                    name: "ops".into(),
                    user: "CORP\\ops".into(),
                    secret: "ops".into(),
                    state: "ok".into(),
                    used_by: vec![],
                }),
                Page::Hosts => Form::edit_host(&conn_body().hosts[1]),
                Page::Buckets => Form::edit_bucket(&conn_body().buckets[0]),
                _ => Form::edit_mailbox(&conn_body().mailboxes[0]),
            };
            assert!(!f.dirty(), "{page:?}");
            f.password.as_mut_string().push('x');
            assert!(f.dirty(), "{page:?}");
        }
    }

    /// The host form: an SSH host may have no password; an edit keeps, replaces or drops it.
    #[test]
    fn the_ssh_host_form_adds_with_or_without_a_password_keeps_it_or_drops_it() {
        let ssh = |f: &Form| match f.request("b", &[]) {
            Ok(Request::HostPut { password, password_value, .. }) => (password, password_value.map(|v| v.expose().to_owned())),
            other => panic!("{other:?}"),
        };
        // Adding with nothing typed is allowed for SSH, and refused for WinRM.
        let mut f = Form::add(Page::Hosts);
        f.name = "h1".into();
        f.type_address("ops@h1");
        f.x.host_key = "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU".into();
        f.x.key = "C:/keys/id".into();
        assert_eq!(ssh(&f), (None, None));
        assert_eq!(f.password_note(), "registry.pw_new_optional");
        // (A key and a password cannot both be given: the entry's own check says so.)
        assert!(f.request("b", &[]).is_ok());
        f.x.key.clear();
        f.password.as_mut_string().push_str(PW);
        assert_eq!(ssh(&f), (None, Some(PW.to_owned())));
        f.set_winrm(true);
        f.password = Hidden::default();
        assert_eq!(f.request("b", &[]).unwrap_err(), Line::PasswordEmpty);
        // An SSH host with a password: empty keeps it, typed replaces it, "do not use" leaves it out.
        let row = HostRow {
            name: "web2".into(),
            on: "ssh://ops@web2/".into(),
            kind: "ssh".into(),
            secret: Some("koyomi-host-web2".into()),
            host_key: Some("SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU".into()),
            state: "ok".into(),
            ..HostRow::default()
        };
        let mut f = Form::edit_host(&row);
        assert!(f.can_drop_password() && !f.dirty());
        assert_eq!(f.password_note(), "registry.pw_ok");
        assert_eq!(ssh(&f), (Some("secret:koyomi-host-web2".to_owned()), None));
        f.password.as_mut_string().push_str(PW);
        assert_eq!(ssh(&f), (None, Some(PW.to_owned())));
        // Ticking the box empties the field and sends neither a reference nor a value.
        f.set_drop_password(true);
        assert!(f.password.is_empty() && f.dirty());
        assert_eq!(ssh(&f), (None, None));
        // Switching to WinRM hides the box and clears the choice: WinRM needs a password.
        f.set_winrm(true);
        assert!(!f.drop_password && !f.can_drop_password());
        // An SSH host without a password offers no box, and says it signs in with its key.
        let bare = Form::edit_host(&conn_body().hosts[0]);
        assert!(!bare.can_drop_password());
        assert_eq!(bare.password_note(), "registry.pw_none_optional");
    }

    /// The S3 page asks for the secret access key by that name, and never invents a reference for a row without one.
    #[test]
    fn the_bucket_form_asks_for_the_secret_access_key_and_invents_no_reference() {
        let mut f = Form::add(Page::Buckets);
        f.name = "b1".into();
        f.type_address("s3://corp-b1/");
        f.x.region = "ap-northeast-1".into();
        f.x.key_id = "AKIAEXAMPLE".into();
        assert_eq!(f.request("b", &[]).unwrap_err(), Line::Key("registry.s3_key_empty"));
        f.password.as_mut_string().push_str(PW);
        let got = f.request("b", &[]);
        assert!(matches!(got, Ok(Request::BucketPut { password: None, password_value: Some(_), .. })), "{got:?}");
        // A row with no `secret` keeps an empty one (it used to be filled with the row's name).
        let row = BucketRow { name: "old".into(), url: "s3://old/".into(), state: "missing".into(), ..BucketRow::default() };
        let f = Form::edit_bucket(&row);
        assert!(f.secret.is_empty());
        assert_eq!(f.request("b", &[]).unwrap_err(), Line::Key("registry.s3_key_empty"));
        assert_eq!(f.password_note(), "registry.pw_missing");
    }

    /// The mailbox page: add with a typed password, keep on edit.
    #[test]
    fn the_mailbox_form_sends_the_typed_password_or_keeps_the_own_secret() {
        let mut f = Form::add(Page::Mailboxes);
        f.name = "m1".into();
        f.type_address("imaps://u@mail/INBOX");
        assert_eq!(f.request("b", &[]).unwrap_err(), Line::PasswordEmpty);
        f.password.as_mut_string().push_str(PW);
        assert!(matches!(f.request("b", &[]), Ok(Request::MailboxPut { password: None, password_value: Some(_), .. })));
        let f = Form::edit_mailbox(&conn_body().mailboxes[0]);
        match f.request("b", &[]) {
            Ok(Request::MailboxPut { password, password_value, .. }) => {
                assert_eq!(password.as_deref(), Some("secret:billing"));
                assert!(password_value.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    /// Every page's form, new and edited, with the field empty or typed: what the request holds for the password,
    /// and that no request prints the typed value.
    #[test]
    fn the_password_matrix_of_every_form() {
        const KEY: &str = "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU";
        const CERT: &str = "AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89";
        // (label, a new form, an edited form whose own secret is `own`, the refusal for a new form with no password)
        let new = |page: Page, winrm: bool| {
            let mut f = Form::add(page);
            f.name = "n1".into();
            match page {
                Page::Accounts => f.target = "CORP\\n1".into(),
                Page::Hosts => {
                    f.set_winrm(winrm);
                    f.type_address(if winrm { ".\\u@pc:5986" } else { "ops@h1" });
                    if winrm {
                        f.x.cert = CERT.into();
                    } else {
                        f.x.host_key = KEY.into();
                    }
                }
                Page::Buckets => {
                    f.type_address("s3://corp-b1/");
                    f.x.key_id = "AKIAEXAMPLE".into();
                    f.x.region = "ap-northeast-1".into();
                }
                _ => f.type_address("imaps://u@mail/INBOX"),
            }
            f
        };
        let edit = |page: Page, winrm: bool| match page {
            Page::Accounts => Form::edit_account(&AccountRow {
                name: "ops".into(),
                user: "CORP\\ops".into(),
                secret: "own".into(),
                state: "ok".into(),
                used_by: vec![],
            }),
            Page::Hosts => Form::edit_host(&HostRow {
                name: "h".into(),
                on: if winrm { "winrm://.\\u@pc:5986" } else { "ssh://ops@h/" }.into(),
                kind: if winrm { "winrm" } else { "ssh" }.into(),
                secret: Some("own".into()),
                host_key: (!winrm).then(|| KEY.to_owned()),
                cert: winrm.then(|| CERT.to_owned()),
                state: "ok".into(),
                ..HostRow::default()
            }),
            Page::Buckets => Form::edit_bucket(&BucketRow {
                name: "logs".into(),
                url: "s3://logs/".into(),
                key_id: Some("AKIAEXAMPLE".into()),
                region: Some("ap-northeast-1".into()),
                secret: Some("own".into()),
                state: "ok".into(),
                ..BucketRow::default()
            }),
            _ => Form::edit_mailbox(&MailboxRow {
                name: "billing".into(),
                mail: "imaps://u@mail.example.com/INBOX".into(),
                secret: "own".into(),
                state: "ok".into(),
                used_by: vec![],
            }),
        };
        let cases = [
            ("accounts", Page::Accounts, false, Line::PasswordEmpty),
            ("ssh host", Page::Hosts, false, Line::PasswordEmpty),
            ("winrm host", Page::Hosts, true, Line::PasswordEmpty),
            ("buckets", Page::Buckets, false, Line::Key("registry.s3_key_empty")),
            ("mailboxes", Page::Mailboxes, false, Line::PasswordEmpty),
        ];
        // What a request holds: (`password`, `password_value`). Also checks the request prints no value.
        let sent = |f: &Form| -> Result<(Option<String>, Option<String>), Line> {
            let req = f.request("b", &[])?;
            assert!(!format!("{req:?}").contains(PW), "a request printed the typed value");
            Ok(match req {
                Request::AccountPut { password, password_value, .. }
                | Request::HostPut { password, password_value, .. }
                | Request::BucketPut { password, password_value, .. }
                | Request::MailboxPut { password, password_value, .. } => (password, password_value.map(|v| v.expose().to_owned())),
                _ => panic!("not a registry put"),
            })
        };
        let typed = Some(PW.to_owned());
        let own = Some("secret:own".to_owned());
        for (label, page, winrm, refusal) in cases {
            // new, field empty: refused (an SSH host sends neither)
            let mut f = new(page, winrm);
            if page == Page::Hosts && !winrm {
                assert_eq!(sent(&f), Ok((None, None)), "{label}: new, empty");
            } else {
                assert_eq!(sent(&f), Err(refusal), "{label}: new, empty");
            }
            // new, typed
            f.password.as_mut_string().push_str(PW);
            assert_eq!(sent(&f), Ok((None, typed.clone())), "{label}: new, typed");
            // edit with its own secret, field empty
            let mut e = edit(page, winrm);
            assert_eq!(sent(&e), Ok((own.clone(), None)), "{label}: edit, empty");
            // edit with its own secret, typed
            e.password.as_mut_string().push_str(PW);
            assert_eq!(sent(&e), Ok((None, typed.clone())), "{label}: edit, typed");
        }
        // an SSH edit with "do not use a password"
        let mut e = edit(Page::Hosts, false);
        e.set_drop_password(true);
        assert_eq!(sent(&e), Ok((None, None)));
        // an SSH host with a key and a typed password is refused (a key and a password cannot both be given)
        let mut f = new(Page::Hosts, false);
        f.x.key = "C:/keys/id".into();
        f.password.as_mut_string().push_str(PW);
        let refused = f.request("b", &[]);
        assert!(matches!(refused, Err(Line::Raw(_))), "{refused:?}");
        assert!(!format!("{refused:?}").contains(PW));
    }

    fn db_row(name: &str, kind: &str, db: &str, secret: Option<&str>) -> DatabaseRow {
        DatabaseRow {
            name: name.into(),
            kind: kind.into(),
            db: db.into(),
            secret: secret.map(str::to_owned),
            tls: None,
            state: "ok".into(),
            used_by: vec![],
        }
    }

    fn db_loaded() -> Registry {
        let mut r = Registry::default();
        r.touch(1, Page::Databases);
        r.ask();
        let mut b = body();
        b.databases.push(db_row("pg", "postgres", "postgres://app@db1:5432/sales", Some("koyomi-db-pg")));
        b.databases.push(db_row("lite", "sqlite", "sqlite://C:\\data\\a.db", None));
        r.listing_answer(Ok(b.to_toml()));
        r
    }

    /// An add form with a PostgreSQL entry filled in, the password typed.
    fn pg_form() -> Form {
        let mut f = Form::add(Page::Databases);
        f.name = "sales".into();
        f.db.host = "db1".into();
        f.db.database = "sales".into();
        f.db.user = "app".into();
        f.password.as_mut_string().push_str(PW);
        f
    }

    const SHUT: View = View { far: None, open: false };
    const OPEN_VIEW: View = View { far: None, open: true };

    fn dbf(r: &Registry) -> &Form {
        r.forms[Page::Databases.idx()].as_ref().unwrap()
    }

    fn dbf_mut(r: &mut Registry) -> &mut Form {
        r.forms[Page::Databases.idx()].as_mut().unwrap()
    }

    fn db_test_click(r: &mut Registry) -> Option<Request> {
        match r.act(Click::DbTest, Page::Databases, &OPEN_VIEW) {
            Do::Send(req) => Some(req),
            Do::Nothing => None,
            other => panic!("{other:?}"),
        }
    }

    fn db_edit() -> Registry {
        let mut r = db_loaded();
        r.act(Click::OpenEdit("pg".into()), Page::Databases, &OPEN_VIEW);
        r
    }

    fn db_test_answer(r: &mut Registry, ticket: u64, result: Result<String, String>) {
        let req = Request::DatabaseTest { db: "x".into(), tls: None, entry: None, password_value: None, ticket };
        r.answered(&req, result);
    }

    #[test]
    fn the_buttons_hover_says_whether_the_fields_the_password_or_nothing_is_missing() {
        let mut f = pg_form();
        assert_eq!(db_test_why(&f), "registry.db_test_hover");
        f.db.port = "70000".into();
        assert_eq!(db_test_why(&f), "registry.db_test_fields", "a port that does not read, with a password typed");
        f.db.port = "5432".into();
        f.password = Hidden::default();
        assert_eq!(db_test_why(&f), "registry.db_test_need");
        let mut lite = Form::add(Page::Databases);
        lite.db.kind = DbKind::Sqlite;
        assert_eq!(db_test_why(&lite), "registry.db_test_fields", "an empty SQLite file");
        lite.db.file = r"C:\data\a.db".into();
        assert_eq!(db_test_why(&lite), "registry.db_test_hover");
    }

    #[test]
    fn the_test_button_needs_a_typed_password_or_the_saved_one_for_the_same_server_user_and_tls() {
        // A new entry: only a typed password.
        let mut r = db_loaded();
        r.act(Click::OpenAdd, Page::Databases, &OPEN_VIEW);
        let f = dbf_mut(&mut r);
        f.db.host = "db1".into();
        f.db.database = "sales".into();
        f.db.user = "app".into();
        assert!(!f.db_test_enabled(true));
        f.password.as_mut_string().push_str(PW);
        assert!(f.db_test_enabled(true));
        assert!(!f.db_test_enabled(false), "the person must be able to write");
        f.db.port = "70000".into();
        assert!(!f.db_test_enabled(true), "a field that does not read");
        f.db.port = "5432".into();
        // Editing, nothing typed: the saved password, while host, port, user and TLS are as opened.
        let mut r = db_edit();
        assert!(dbf(&r).db_test_enabled(true));
        dbf_mut(&mut r).db.host = "DB1".into();
        assert!(dbf(&r).db_test_enabled(true), "the host differs in case only");
        dbf_mut(&mut r).db.host = "db2".into();
        assert!(!dbf(&r).db_test_enabled(true));
        dbf_mut(&mut r).password.as_mut_string().push_str(PW);
        assert!(dbf(&r).db_test_enabled(true), "a typed password is enough");
        for change in [|d: &mut DbFields| d.port = "5433".into(), |d: &mut DbFields| d.user = "other".into()] {
            let mut r = db_edit();
            change(&mut dbf_mut(&mut r).db);
            assert!(!dbf(&r).db_test_enabled(true));
        }
        // A weaker TLS cannot use the saved password; a stronger one can (the row opens with the usual verify).
        let mut r = db_edit();
        dbf_mut(&mut r).tls = SqlTls::Off;
        assert!(!dbf(&r).db_test_enabled(true));
        // A row whose password is missing has nothing to use.
        let mut r = db_loaded();
        r.listing.as_mut().unwrap().databases.iter_mut().find(|d| d.name == "pg").unwrap().state = "missing".into();
        r.act(Click::OpenEdit("pg".into()), Page::Databases, &OPEN_VIEW);
        assert!(!dbf(&r).db_test_enabled(true));
        // SQLite takes no password.
        let mut r = db_loaded();
        r.act(Click::OpenEdit("lite".into()), Page::Databases, &OPEN_VIEW);
        assert!(dbf(&r).db_test_enabled(true));
    }

    #[test]
    fn a_click_sends_the_request_the_fields_make_and_waits_for_its_own_ticket() {
        // A typed password goes as a value, with no entry name.
        let mut r = db_edit();
        dbf_mut(&mut r).password.as_mut_string().push_str(PW);
        match db_test_click(&mut r) {
            Some(Request::DatabaseTest { db, tls, entry, password_value, ticket }) => {
                assert_eq!((db.as_str(), tls, entry, ticket), ("postgres://app@db1/sales", Some(SqlTls::Verify), None, 1));
                assert_eq!(password_value.as_ref().map(Hidden::expose), Some(PW));
            }
            other => panic!("{other:?}"),
        }
        assert!(dbf(&r).db_test.as_ref().is_some_and(|t| t.ticket == 1 && t.result.is_none()));
        // One at a time.
        assert!(db_test_click(&mut r).is_none());
        db_test_answer(&mut r, 2, Ok(String::new()));
        assert_eq!(dbf(&r).db_test.as_ref().unwrap().result, None, "another ticket");
        db_test_answer(&mut r, 1, Err("E2267 shown".into()));
        assert_eq!(dbf(&r).db_test.as_ref().unwrap().result, Some(Err("E2267 shown".to_owned())));
        db_test_answer(&mut r, 1, Ok(String::new()));
        assert_eq!(dbf(&r).db_test.as_ref().unwrap().result, Some(Err("E2267 shown".to_owned())), "the first answer stays");
        // Editing, nothing typed: the entry's name, no value.
        let mut r = db_edit();
        match db_test_click(&mut r) {
            Some(Request::DatabaseTest { entry, password_value, tls, .. }) => {
                assert_eq!((entry.as_deref(), password_value.is_none(), tls), (Some("pg"), true, Some(SqlTls::Verify)));
            }
            other => panic!("{other:?}"),
        }
        // SQLite: no TLS, no entry, no value.
        let mut r = db_loaded();
        r.act(Click::OpenEdit("lite".into()), Page::Databases, &OPEN_VIEW);
        match db_test_click(&mut r) {
            Some(Request::DatabaseTest { db, tls, entry, password_value, .. }) => {
                assert_eq!((db.as_str(), tls, entry, password_value.is_none()), ("sqlite://C:\\data\\a.db", None, None, true));
            }
            other => panic!("{other:?}"),
        }
        // A closed page, a stale form and a form that cannot send ask nothing.
        let mut r = db_edit();
        assert!(matches!(r.act(Click::DbTest, Page::Databases, &SHUT), Do::Nothing));
        dbf_mut(&mut r).stale = true;
        assert!(db_test_click(&mut r).is_none());
        let mut r = db_edit();
        r.write = true;
        assert!(db_test_click(&mut r).is_none());
    }

    #[test]
    fn a_change_of_any_field_or_of_the_password_drops_the_result_and_a_late_answer_finds_nothing() {
        let changes: [fn(&mut Form); 4] = [
            |f| f.db.host = "db9".into(),
            |f| f.db.database = "other".into(),
            |f| f.password.as_mut_string().push('x'),
            |f| f.password.as_mut_string().push(' '),
        ];
        for change in changes {
            let mut r = db_edit();
            dbf_mut(&mut r).password.as_mut_string().push_str(PW);
            db_test_click(&mut r).unwrap();
            db_test_answer(&mut r, 1, Ok(String::new()));
            dbf_mut(&mut r).drop_stale_db_test();
            assert!(dbf(&r).db_test.is_some(), "nothing changed yet");
            change(dbf_mut(&mut r));
            dbf_mut(&mut r).drop_stale_db_test();
            assert!(dbf(&r).db_test.is_none());
        }
        // A late answer, after the fields changed, is not shown.
        let mut r = db_edit();
        dbf_mut(&mut r).password.as_mut_string().push_str(PW);
        db_test_click(&mut r).unwrap();
        dbf_mut(&mut r).db.host = "db9".into();
        db_test_answer(&mut r, 1, Ok(String::new()));
        assert!(dbf(&r).db_test.is_none());
        // The password cleared: a test with a typed password is gone, and the saved one is another ask.
        let mut r = db_edit();
        dbf_mut(&mut r).password.as_mut_string().push_str(PW);
        db_test_click(&mut r).unwrap();
        dbf_mut(&mut r).password.as_mut_string().clear();
        dbf_mut(&mut r).drop_stale_db_test();
        assert!(dbf(&r).db_test.is_none());
        // The listing dropped under the page (another role or name): the test goes with it.
        let mut r = db_edit();
        dbf_mut(&mut r).password.as_mut_string().push_str(PW);
        db_test_click(&mut r).unwrap();
        r.dropped();
        assert!(r.forms[Page::Databases.idx()].as_ref().is_none_or(|f| f.db_test.is_none()));
    }

    #[test]
    fn the_ask_holds_the_password_only_as_a_hash_and_a_test_is_not_a_write() {
        let mut r = db_edit();
        dbf_mut(&mut r).password.as_mut_string().push_str(PW);
        db_test_click(&mut r).unwrap();
        let shown = format!("{:?}", dbf(&r));
        assert!(!shown.contains(PW), "{shown}");
        assert!(!r.write(), "a test is a read of the engine, not a save");
        // A request the thread did not take shows as the test's error, not as the page's.
        r.db_test_unsent("down".into());
        assert_eq!(dbf(&r).db_test.as_ref().unwrap().result, Some(Err("down".to_owned())));
        assert!(r.refusal().is_none());
    }

    #[test]
    fn a_new_postgres_entry_sends_the_fields_as_a_url_and_the_typed_password_apart() {
        let f = pg_form();
        assert_eq!(f.db.port, "5432");
        match f.request("b", &[]) {
            Ok(Request::DatabasePut { name, db, password, tls, password_value, .. }) => {
                assert_eq!((name.as_str(), db.as_str(), password, tls), ("sales", "postgres://app@db1/sales", None, None));
                assert_eq!(password_value.as_ref().map(Hidden::expose), Some(PW));
            }
            other => panic!("{other:?}"),
        }
        // Another port is written; the usual one is left out; an empty port is the usual one.
        let mut g = pg_form();
        g.db.port = "5433".into();
        g.tls = SqlTls::Trust;
        match g.request("b", &[]) {
            Ok(Request::DatabasePut { db, tls, .. }) => {
                assert_eq!((db.as_str(), tls), ("postgres://app@db1:5433/sales", Some(SqlTls::Trust)))
            }
            other => panic!("{other:?}"),
        }
        g.db.port = " ".into();
        assert!(matches!(g.request("b", &[]), Ok(Request::DatabasePut { ref db, .. }) if db == "postgres://app@db1/sales"));
    }

    #[test]
    fn a_field_that_does_not_make_a_url_is_refused_by_its_own_words_and_nothing_is_sent() {
        type Change = fn(&mut Form);
        let cases: [(&str, Change); 5] = [
            ("registry.db_bad_host", |f| f.db.host = "a b".into()),
            ("registry.db_bad_port", |f| f.db.port = "70000".into()),
            ("registry.db_bad_port", |f| f.db.port = "54x2".into()),
            ("registry.db_bad_database", |f| f.db.database = " ".into()),
            ("registry.db_bad_user", |f| f.db.user = "a:b".into()),
        ];
        for (key, change) in cases {
            let mut f = pg_form();
            change(&mut f);
            assert_eq!(f.request("b", &[]).unwrap_err(), Line::Key(key), "{key}");
        }
        let mut f = pg_form();
        f.password = Hidden::default();
        assert_eq!(f.request("b", &[]).unwrap_err(), Line::PasswordEmpty);
        let mut f = pg_form();
        f.db.host.clear();
        assert_eq!(f.request("b", &[]).unwrap_err(), Line::Key("registry.db_bad_host"));
        let mut f = Form::add(Page::Databases);
        f.name = "lite".into();
        f.db.kind = DbKind::Sqlite;
        f.db.file = "data.db".into();
        assert_eq!(f.request("b", &[]).unwrap_err(), Line::Key("registry.db_bad_file"));
        // The worded line reads in both languages.
        for lang in ["ja", "en"] {
            let (t, _) = crate::i18n::table(lang);
            for key in
                ["registry.db_bad_host", "registry.db_bad_port", "registry.db_bad_database", "registry.db_bad_user", "registry.db_bad_file"]
            {
                let text = Line::Key(key).text(&t);
                assert!(!text.is_empty() && !text.contains("registry."), "{lang} {key}");
            }
        }
    }

    #[test]
    fn a_new_entry_without_a_password_is_refused_and_a_sqlite_file_has_no_password_or_tls() {
        let mut f = pg_form();
        f.password.as_mut_string().clear();
        assert_eq!(f.request("b", &[]).unwrap_err(), Line::PasswordEmpty);
        let mut f = Form::add(Page::Databases);
        f.name = "lite".into();
        f.db.kind = DbKind::Sqlite;
        f.db.file = " C:\\data\\a b.db ".into();
        f.tls = SqlTls::Trust;
        match f.request("b", &[]) {
            Ok(Request::DatabasePut { db, password, tls, password_value, .. }) => {
                assert_eq!((db.as_str(), password, tls, password_value), ("sqlite://C:\\data\\a b.db", None, None, None));
            }
            other => panic!("{other:?}"),
        }
        // The name is checked as before.
        f.name = "a/b".into();
        assert!(matches!(f.request("b", &[]), Err(Line::Raw(_))));
        f.name = "main".into();
        assert_eq!(f.request("b", &["main".to_owned()]).unwrap_err(), Line::NameExists("main".into()));
    }

    #[test]
    fn editing_without_typing_keeps_the_row_text_and_its_secret() {
        let row = db_row("pg", "postgres", "postgres://app@db1:5432/sales", Some("koyomi-db-pg"));
        let f = Form::edit_database(&row).unwrap();
        assert_eq!((f.db.host.as_str(), f.db.port.as_str(), f.db.database.as_str(), f.db.user.as_str()), ("db1", "5432", "sales", "app"));
        assert!(!f.dirty());
        match f.request("b", &[]) {
            Ok(Request::DatabasePut { db, password, password_value, .. }) => {
                // The explicit port stays as it was written.
                assert_eq!(db, "postgres://app@db1:5432/sales");
                assert_eq!((password.as_deref(), password_value), (Some("secret:koyomi-db-pg"), None));
            }
            other => panic!("{other:?}"),
        }
        // A row that has no password to keep needs one.
        let mut bare = row.clone();
        bare.secret = None;
        assert_eq!(Form::edit_database(&bare).unwrap().request("b", &[]).unwrap_err(), Line::PasswordEmpty);
        // A changed place is built again.
        let mut g = Form::edit_database(&row).unwrap();
        g.db.database = "ops".into();
        assert!(g.dirty());
        assert!(matches!(g.request("b", &[]), Ok(Request::DatabasePut { ref db, .. }) if db == "postgres://app@db1/ops"));
    }

    #[test]
    fn a_password_only_change_is_dirty_and_sends_the_old_text() {
        let row = db_row("pg", "postgres", "postgres://app@db1:5432/sales", Some("koyomi-db-pg"));
        let mut f = Form::edit_database(&row).unwrap();
        f.password.as_mut_string().push_str(PW);
        assert!(f.dirty());
        match f.request("b", &[]) {
            Ok(Request::DatabasePut { db, password, password_value, .. }) => {
                assert_eq!((db.as_str(), password), ("postgres://app@db1:5432/sales", None));
                assert_eq!(password_value.as_ref().map(Hidden::expose), Some(PW));
            }
            other => panic!("{other:?}"),
        }
        // The TLS pick alone is a change too.
        let mut g = Form::edit_database(&row).unwrap();
        g.tls = SqlTls::Trust;
        assert!(g.dirty());
    }

    #[test]
    fn a_sql_server_row_is_not_opened_but_can_still_be_removed() {
        let mut r = db_loaded();
        assert!(matches!(r.act(Click::OpenEdit("main".into()), Page::Databases, &OPEN), Do::Nothing));
        assert!(r.forms[Page::Databases.idx()].is_none());
        r.act(Click::OpenEdit("pg".into()), Page::Databases, &OPEN);
        assert!(r.forms[Page::Databases.idx()].as_ref().is_some_and(|f| f.gen > 0 && f.editing.as_deref() == Some("pg")));
        r.act(Click::CloseForm, Page::Databases, &OPEN);
        r.act(Click::OpenEdit("lite".into()), Page::Databases, &OPEN);
        assert!(r.forms[Page::Databases.idx()].as_ref().is_some_and(|f| f.is_sqlite() && f.db.file == "C:\\data\\a.db"));
        r.act(Click::CloseForm, Page::Databases, &OPEN);
        r.act(Click::AskRemove("main".into()), Page::Databases, &OPEN);
        assert!(r.confirm.is_some());
        assert!(matches!(r.act(Click::Yes, Page::Databases, &OPEN), Do::Send(Request::DatabaseRemove { .. })));
    }

    #[test]
    fn the_typed_password_is_in_no_debug_text() {
        let f = pg_form();
        assert!(!format!("{f:?}").contains(PW));
        let req = f.request("b", &[]).unwrap();
        assert!(!format!("{req:?}").contains(PW));
        assert!(format!("{req:?}").contains("***"));
        // A refused save keeps the typed value in the form and says nothing of it.
        let mut g = pg_form();
        g.db.host = "a b".into();
        let why = g.request("b", &[]).unwrap_err();
        assert!(!format!("{why:?}").contains(PW));
    }

    #[test]
    fn a_chosen_file_goes_only_to_the_form_and_the_pc_it_was_asked_for() {
        let path = vec![std::path::PathBuf::from("C:\\x\\y.db")];
        let mut r = db_loaded();
        r.act(Click::OpenEdit("lite".into()), Page::Databases, &OPEN);
        let gen = r.forms[Page::Databases.idx()].as_ref().unwrap().gen;
        let file = |r: &Registry| r.forms[Page::Databases.idx()].as_ref().map(|f| f.db.file.clone());
        // Another field kind, another PC, another form number, nothing chosen: dropped.
        assert!(!r.put_browsed(Page::Databases, gen, None, Chosen::HostKey, &path));
        assert!(!r.put_browsed(Page::Databases, gen, Some("far"), Chosen::SqliteFile, &path));
        assert!(!r.put_browsed(Page::Databases, gen + 1, None, Chosen::SqliteFile, &path));
        assert!(!r.put_browsed(Page::Databases, gen, None, Chosen::SqliteFile, &[]));
        assert_eq!(file(&r).as_deref(), Some("C:\\data\\a.db"));
        assert!(r.put_browsed(Page::Databases, gen, None, Chosen::SqliteFile, &path));
        assert_eq!(file(&r).as_deref(), Some("C:\\x\\y.db"));
        // The form was closed and opened again: the old answer finds a new form.
        r.act(Click::CloseForm, Page::Databases, &OPEN);
        assert!(!r.put_browsed(Page::Databases, gen, None, Chosen::SqliteFile, &path));
        r.act(Click::OpenEdit("lite".into()), Page::Databases, &OPEN);
        assert!(!r.put_browsed(Page::Databases, gen, None, Chosen::SqliteFile, &path));
        assert_eq!(file(&r).as_deref(), Some("C:\\data\\a.db"));
        // Seen from another PC, the answer asked on this PC is not for this form.
        r.link = Some("far".into());
        let gen = r.forms[Page::Databases.idx()].as_ref().unwrap().gen;
        assert!(!r.put_browsed(Page::Databases, gen, None, Chosen::SqliteFile, &path));
        assert!(r.put_browsed(Page::Databases, gen, Some("far"), Chosen::SqliteFile, &path));
    }

    #[test]
    fn the_sqlite_chooser_starts_in_the_folder_of_the_field_when_it_exists() {
        let dir = std::env::temp_dir();
        assert_eq!(start_dir(&dir.join("a.db").display().to_string()), Some(dir.clone()));
        assert_eq!(start_dir(&format!("  {}  ", dir.join("a.db").display())), Some(dir.clone()));
        assert_eq!(start_dir(&dir.join("no-such-folder-koyomi").join("a.db").display().to_string()), None);
        assert_eq!(start_dir(""), None);
        assert_eq!(start_dir("a.db"), None);
    }

    #[test]
    fn a_secret_that_is_missing_gets_its_command() {
        assert_eq!(secret_command("free"), "koyomi secret set free");
        assert_eq!(secret_command("db.main-2_x"), "koyomi secret set db.main-2_x");
        // A hand-written name a shell would split or read is quoted.
        assert_eq!(secret_command("db pass"), "koyomi secret set \"db pass\"");
        assert_eq!(secret_command("a&b"), "koyomi secret set \"a&b\"");
        assert_eq!(kind_word("postgres"), "PostgreSQL");
    }

    #[test]
    fn a_change_of_role_drops_the_listing_and_keeps_what_was_typed() {
        let mut r = loaded();
        assert!(!r.role_told((Some(Role::Admin), Some("me".into()))));
        r.act(Click::OpenAdd, Page::Accounts, &OPEN);
        r.forms[0].as_mut().unwrap().name = "typed".into();
        assert!(r.role_told((Some(Role::Edit), Some("me".into()))));
        assert!(r.listing.is_none() && r.forms[0].as_ref().is_some_and(|f| f.stale));
    }
}

#[cfg(test)]
mod peek_tests {
    use super::*;

    const OPEN: View = View { far: None, open: true };
    const KEY: &str = "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU";

    /// An add form for a host, with `target` typed.
    fn hosts(target: &str) -> Registry {
        let mut r = Registry::default();
        r.touch(1, Page::Hosts);
        r.act(Click::OpenAdd, Page::Hosts, &OPEN);
        r.forms[Page::Hosts.idx()].as_mut().unwrap().type_address(target);
        r
    }

    fn form(r: &Registry) -> &Form {
        r.forms[Page::Hosts.idx()].as_ref().unwrap()
    }

    fn click(r: &mut Registry) -> Option<(PeekKind, String, u16, u64)> {
        match r.act(Click::Peek, Page::Hosts, &OPEN) {
            Do::Send(Request::HostPeek { kind, host, port, ticket }) => Some((kind, host, port, ticket)),
            Do::Nothing => None,
            other => panic!("{other:?}"),
        }
    }

    fn answer(r: &mut Registry, ticket: u64, value: &str) {
        let req = Request::HostPeek { kind: PeekKind::Ssh, host: "web1".into(), port: 22, ticket };
        r.answered(&req, Ok(value.to_owned()));
    }

    #[test]
    fn the_click_sends_the_address_read_with_its_default_port_and_a_new_ticket() {
        let mut r = hosts("ops@web1");
        assert_eq!(click(&mut r), Some((PeekKind::Ssh, "web1".into(), 22, 1)));
        assert!(form(&r).peek.as_ref().is_some_and(|p| p.ticket == 1 && p.result.is_none()));
        // One ask at a time: a second click while waiting sends nothing.
        assert_eq!(click(&mut r), None);
        let mut r = hosts("CORP\\u@pc");
        r.forms[Page::Hosts.idx()].as_mut().unwrap().set_winrm(true);
        assert_eq!(click(&mut r), Some((PeekKind::Winrm, "pc".into(), chore::WINRM_PORT, 1)));
    }

    #[test]
    fn the_button_is_off_for_an_address_that_does_not_read_or_the_engine_would_refuse() {
        for bad in ["", "   ", "web1:0", "[::1]", "a b", "u@", "web1/x"] {
            let r = hosts(bad);
            assert_eq!(form(&r).peek_target(), None, "{bad:?}");
        }
        let mut r = hosts("");
        assert_eq!(click(&mut r), None);
        assert!(form(&r).peek.is_none());
        // A WinRM address needs its user, as the host form's own check does.
        let mut r = hosts("pc");
        r.forms[Page::Hosts.idx()].as_mut().unwrap().set_winrm(true);
        assert_eq!(form(&r).peek_target(), None);
        // A closed page asks nothing.
        let mut r = hosts("web1");
        let shut = View { far: None, open: false };
        assert!(matches!(r.act(Click::Peek, Page::Hosts, &shut), Do::Nothing));
    }

    #[test]
    fn only_the_waiting_ticket_is_taken() {
        let mut r = hosts("web1");
        let (.., ticket) = click(&mut r).unwrap();
        answer(&mut r, ticket + 1, "SHA256:other");
        assert_eq!(form(&r).peek.as_ref().unwrap().result, None);
        answer(&mut r, ticket, KEY);
        assert_eq!(form(&r).peek.as_ref().unwrap().result, Some(Ok(KEY.to_owned())));
        // A second answer to the same ticket does not replace the first.
        answer(&mut r, ticket, "SHA256:later");
        assert_eq!(form(&r).peek.as_ref().unwrap().result, Some(Ok(KEY.to_owned())));
    }

    #[test]
    fn a_late_answer_is_dropped_after_the_address_changes_even_back_to_the_same() {
        let mut r = hosts("web1");
        let (.., a) = click(&mut r).unwrap();
        // The address is changed to B: the model itself drops the peek, with no help from the drawing.
        set_target(&mut r, "web2");
        answer(&mut r, a, "SHA256:stale");
        assert!(form(&r).peek.is_none());
        // Back to A and asked again: a new ticket; the old ticket's answer is dropped, the new one taken.
        set_target(&mut r, "web1");
        let (.., again) = click(&mut r).unwrap();
        assert_ne!(a, again);
        answer(&mut r, a, "SHA256:stale");
        assert_eq!(form(&r).peek.as_ref().unwrap().result, None);
        answer(&mut r, again, KEY);
        assert_eq!(form(&r).peek.as_ref().unwrap().result, Some(Ok(KEY.to_owned())));
    }

    fn set_target(r: &mut Registry, target: &str) {
        r.forms[Page::Hosts.idx()].as_mut().unwrap().type_address(target);
    }

    #[test]
    fn a_result_is_not_used_after_the_address_changes() {
        let mut r = hosts("web1");
        let (.., t) = click(&mut r).unwrap();
        answer(&mut r, t, KEY);
        set_target(&mut r, "web2");
        r.act(Click::UsePeek, Page::Hosts, &OPEN);
        assert_eq!(form(&r).x.host_key, "");
        assert!(form(&r).peek.is_none());
    }

    #[test]
    fn the_button_is_on_only_for_an_open_page_a_readable_address_and_no_ask_waiting() {
        let r = hosts("web1");
        assert!(form(&r).peek_enabled(true));
        assert!(!form(&r).peek_enabled(false));
        let r = hosts("web1:0");
        assert!(!form(&r).peek_enabled(true));
        let mut r = hosts("web1");
        let (.., t) = click(&mut r).unwrap();
        assert!(!form(&r).peek_enabled(true));
        answer(&mut r, t, KEY);
        assert!(form(&r).peek_enabled(true));
        let mut r = hosts("web1");
        let (.., t) = click(&mut r).unwrap();
        r.answered(&Request::HostPeek { kind: PeekKind::Ssh, host: "web1".into(), port: 22, ticket: t }, Err("E2253".into()));
        assert!(form(&r).peek.as_ref().unwrap().result.is_some());
        assert!(form(&r).peek_enabled(true));
    }

    #[test]
    fn a_saved_host_drops_the_peek_and_a_late_answer_finds_nothing() {
        let mut r = hosts("web1");
        r.listing = Some(RegistryBody::default());
        let f = r.forms[Page::Hosts.idx()].as_mut().unwrap();
        f.name = "h1".into();
        f.x.key = "id_ed25519".into();
        f.x.host_key = KEY.into();
        let (.., t) = click(&mut r).unwrap();
        let Do::Send(put) = r.act(Click::Save, Page::Hosts, &OPEN) else { panic!("the save was not sent: {:?}", form(&r).error.is_some()) };
        assert!(matches!(put, Request::HostPut { .. }));
        r.answered(&put, Ok(RegistryBody::default().to_toml()));
        assert!(r.forms[Page::Hosts.idx()].is_none());
        answer(&mut r, t, KEY);
        assert!(r.forms[Page::Hosts.idx()].is_none());
    }

    #[test]
    fn a_late_answer_finds_nothing_after_the_kind_changes_or_the_form_is_closed_and_reopened() {
        let mut r = hosts("u@web1");
        let (.., t) = click(&mut r).unwrap();
        r.forms[Page::Hosts.idx()].as_mut().unwrap().set_winrm(true);
        assert!(form(&r).peek.is_none());
        answer(&mut r, t, KEY);
        assert!(form(&r).peek.is_none());
        let mut r = hosts("web1");
        let (.., t) = click(&mut r).unwrap();
        r.act(Click::CloseForm, Page::Hosts, &OPEN);
        r.act(Click::OpenAdd, Page::Hosts, &OPEN);
        answer(&mut r, t, KEY);
        assert!(form(&r).peek.is_none());
        // A role change also drops what was asked.
        let mut r = hosts("web1");
        let (.., t) = click(&mut r).unwrap();
        r.dropped();
        answer(&mut r, t, KEY);
        assert!(r.forms[Page::Hosts.idx()].as_ref().is_none_or(|f| f.peek.is_none()));
    }

    #[test]
    fn use_fills_the_host_key_or_the_certificate_and_nothing_else() {
        let mut r = hosts("web1");
        let (.., t) = click(&mut r).unwrap();
        answer(&mut r, t, KEY);
        assert_eq!(form(&r).x.host_key, "");
        r.act(Click::UsePeek, Page::Hosts, &OPEN);
        assert_eq!((form(&r).x.host_key.as_str(), form(&r).x.cert.as_str()), (KEY, ""));
        // WinRM fills the certificate's field.
        let mut r = hosts("u@pc");
        r.forms[Page::Hosts.idx()].as_mut().unwrap().set_winrm(true);
        let (.., t) = click(&mut r).unwrap();
        answer(&mut r, t, "AA:BB");
        r.act(Click::UsePeek, Page::Hosts, &OPEN);
        assert_eq!((form(&r).x.host_key.as_str(), form(&r).x.cert.as_str()), ("", "AA:BB"));
        // An error answer fills nothing.
        let mut r = hosts("web1");
        let (.., t) = click(&mut r).unwrap();
        r.answered(&Request::HostPeek { kind: PeekKind::Ssh, host: "web1".into(), port: 22, ticket: t }, Err("E2253".into()));
        r.act(Click::UsePeek, Page::Hosts, &OPEN);
        assert_eq!(form(&r).x.host_key, "");
        assert_eq!(form(&r).peek.as_ref().unwrap().result, Some(Err("E2253".to_owned())));
    }

    #[test]
    fn a_peek_is_a_read_and_leaves_the_write_flag_alone() {
        let mut r = hosts("web1");
        let (.., t) = click(&mut r).unwrap();
        assert!(!r.write());
        answer(&mut r, t, KEY);
        assert!(!r.write());
        // A request the thread did not take shows as the peek's error, not as the page's.
        let mut r = hosts("web1");
        click(&mut r).unwrap();
        r.peek_unsent("down".into());
        assert_eq!(form(&r).peek.as_ref().unwrap().result, Some(Err("down".to_owned())));
        assert!(r.refusal().is_none());
    }

    #[test]
    fn empty_same_and_different_are_told_apart() {
        assert_eq!(verdict(false, "", KEY), Verdict::Empty);
        assert_eq!(verdict(false, "  ", KEY), Verdict::Empty);
        assert_eq!(verdict(false, &format!("  {KEY} "), KEY), Verdict::Same);
        assert_eq!(verdict(false, "SHA256:other", KEY), Verdict::Differs);
        // SSH is exact: case counts.
        assert_eq!(verdict(false, &KEY.to_lowercase(), KEY), Verdict::Differs);
        let hex = "ab".repeat(32);
        let colons = hex.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap().to_uppercase()).collect::<Vec<_>>().join(":");
        assert_eq!(verdict(true, &colons, &hex), Verdict::Same);
        assert_eq!(verdict(true, &hex.to_uppercase(), &colons), Verdict::Same);
        assert_eq!(verdict(true, "not a fingerprint", &hex), Verdict::Differs);
        assert_eq!(verdict(true, &"cd".repeat(32), &hex), Verdict::Differs);
    }

    #[test]
    fn the_peek_never_changes_what_is_saved() {
        let mut r = hosts("web1");
        let before = form(&r).request("b", &[]);
        let (.., t) = click(&mut r).unwrap();
        answer(&mut r, t, KEY);
        assert_eq!(format!("{:?}", form(&r).request("b", &[])), format!("{before:?}"));
    }

    #[test]
    fn a_password_in_a_row_from_an_older_engine_is_not_drawn() {
        let w = where_word("mysql://u:Hunter2zz@h/db");
        assert!(!w.contains("Hunter2zz"), "{w}");
        assert_eq!(where_word("postgres://u@h:5432/app"), "h:5432/app");
        assert_eq!(chore::shown_address("ssh://ops@web1:2222"), "ssh://ops@web1:2222");
        assert_eq!(chore::shown_address("imaps://me@example.com@imap.example.com/INBOX"), "imaps://me@example.com@imap.example.com/INBOX");
        assert_eq!(chore::shown_address("ssh://u:Hunter2zz@h:22"), "ssh://u:***@h:22");
    }
}

/// The host, S3 and mailbox forms take their address in parts.
#[cfg(test)]
mod addr_tests {
    use super::*;

    fn host(on: &str) -> Form {
        let winrm = on.to_ascii_lowercase().starts_with("winrm://");
        Form::edit_host(&HostRow {
            name: "h".into(),
            on: on.into(),
            kind: if winrm { "winrm" } else { "ssh" }.into(),
            host_key: (!winrm).then(|| "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU".into()),
            ..HostRow::default()
        })
    }

    fn bucket(url: &str) -> Form {
        Form::edit_bucket(&BucketRow { name: "b".into(), url: url.into(), ..BucketRow::default() })
    }

    fn mailbox(mail: &str) -> Form {
        Form::edit_mailbox(&MailboxRow { name: "m".into(), mail: mail.into(), secret: "m".into(), ..MailboxRow::default() })
    }

    /// 143.44: a new 接続先 PC, S3 or mailbox form starts in parts with the usual port filled in.
    #[test]
    fn a_new_form_starts_in_parts_with_the_usual_port_and_the_switch_moves_only_a_default_port() {
        let mut h = Form::add(Page::Hosts);
        assert!(!h.raw);
        assert_eq!((h.addr.user.as_str(), h.addr.host.as_str(), h.addr.port.as_str()), ("", "", "22"));
        h.set_winrm(true);
        assert_eq!(h.addr.port, "5986");
        h.set_winrm(false);
        assert_eq!(h.addr.port, "22");
        h.addr.port = "2222".into();
        h.set_winrm(true);
        assert_eq!(h.addr.port, "2222", "a port the person chose stays");
        h.addr.port.clear();
        h.set_winrm(false);
        assert_eq!(h.addr.port, "22", "an empty port takes the kind's own");
        let m = Form::add(Page::Mailboxes);
        assert_eq!((m.addr.port.as_str(), m.addr.folder.as_str(), m.raw), ("993", "INBOX", false));
        let b = Form::add(Page::Buckets);
        assert!(!b.raw && b.addr.bucket.is_empty());
    }

    /// 143.45: an edit that changes no part sends the stored text as it was (`:22`, an upper-case scheme).
    #[test]
    fn an_unchanged_edit_sends_the_stored_text_verbatim() {
        for on in ["ssh://ops@web1:22", "SSH://web1", "ssh://ops@web1:2222/", "winrm://.\\u@pc:5986", "winrm://CORP\\u@pc"] {
            let f = host(on);
            assert!(!f.raw && !f.dirty(), "{on}");
            assert_eq!(f.address().as_deref(), Ok(on));
        }
        for url in ["S3://logs/daily/", "s3://logs/daily", "s3://logs/"] {
            let f = bucket(url);
            assert!(!f.raw, "{url}");
            assert_eq!(f.address().as_deref(), Ok(url));
        }
        for mail in ["imaps://u@mail.example.com:993/INBOX", "imaps://u@mail.example.com"] {
            let f = mailbox(mail);
            assert!(!f.raw, "{mail}");
            assert_eq!(f.address().as_deref(), Ok(mail));
        }
        let f = host("ssh://ops@web1:22");
        assert_eq!((f.addr.user.as_str(), f.addr.host.as_str(), f.addr.port.as_str()), ("ops", "web1", "22"));
    }

    /// 143.46: a changed part is built into the text, and changing it back sends the stored text again.
    #[test]
    fn a_changed_part_is_built_and_changed_back_it_is_verbatim() {
        let mut f = host("ssh://ops@web1:2222/");
        f.addr.port = "22".into();
        assert!(f.dirty());
        let built = f.address().unwrap();
        assert_ne!(built, "ssh://ops@web1:2222/");
        let far = chore::ssh(&built).unwrap().unwrap();
        assert_eq!((far.user.as_deref(), far.host.as_str(), far.port), (Some("ops"), "web1", None));
        f.addr.port = "2222".into();
        assert_eq!(f.address().as_deref(), Ok("ssh://ops@web1:2222/"));
        f.addr.host = "web2".into();
        assert!(f.address().unwrap().contains("web2"));
        let mut f = host("winrm://.\\u@pc:5986");
        f.set_winrm(false);
        assert_eq!(f.address().as_deref(), Ok("ssh://.\\u@pc"), "the kind is part of the address");
        f.set_winrm(true);
        assert_eq!(f.address().as_deref(), Ok("winrm://.\\u@pc:5986"));
        let mut b = bucket("s3://logs/daily/");
        b.addr.folder = "weekly".into();
        assert_eq!(b.address().as_deref(), Ok("s3://logs/weekly/"));
        b.addr.folder = "daily".into();
        assert_eq!(b.address().as_deref(), Ok("s3://logs/daily/"));
        let mut m = mailbox("imaps://u@mail.example.com:993/INBOX");
        m.addr.folder = "Archive".into();
        assert!(m.address().unwrap().ends_with("/Archive"));
        m.addr.folder = "INBOX".into();
        assert_eq!(m.address().as_deref(), Ok("imaps://u@mail.example.com:993/INBOX"));
    }

    /// 143.47: a typed password alone does not touch the address that is sent.
    #[test]
    fn a_typed_password_alone_keeps_the_address_verbatim() {
        let mut f = host("ssh://ops@web1:2222/");
        f.password.as_mut_string().push_str("pw-not-to-show");
        match f.request("b", &[]) {
            Ok(Request::HostPut { on, password_value, .. }) => {
                assert_eq!(on, "ssh://ops@web1:2222/");
                assert!(password_value.is_some());
            }
            other => panic!("{other:?}"),
        }
        let mut m = mailbox("imaps://u@mail.example.com:993/INBOX");
        m.password.as_mut_string().push_str("pw-not-to-show");
        match m.request("b", &[]) {
            Ok(Request::MailboxPut { mail, .. }) => assert_eq!(mail, "imaps://u@mail.example.com:993/INBOX"),
            other => panic!("{other:?}"),
        }
    }

    /// 143.48: a stored address that does not read in parts is shown whole, can be changed and is sent as typed.
    #[test]
    fn an_address_that_does_not_read_in_parts_is_held_whole_and_can_be_fixed() {
        let mut f = host("ssh://a b@h");
        assert!(f.raw && !f.dirty());
        assert_eq!(f.target, "a b@h");
        assert_eq!(f.address().as_deref(), Ok("ssh://a b@h"));
        f.target = "ops@h2".into();
        assert!(f.dirty());
        assert_eq!(f.address().as_deref(), Ok("ssh://ops@h2"));
        // The engine reads this one (koyomi.toml may hold it), but the parts cannot: it is held whole.
        assert!(matches!(chore::ssh("ssh://ops@web1?x"), Some(Ok(_))));
        let f = host("ssh://ops@web1?x");
        assert!(f.raw && !f.dirty());
        assert_eq!((f.target.as_str(), f.address().as_deref()), ("ops@web1?x", Ok("ssh://ops@web1?x")));
        // A folder with a space at an end would be written trimmed, so a change to it could read as none: held whole.
        let mut b = bucket("s3://logs/ daily/");
        assert!(b.raw);
        b.target = "s3://logs/daily/".into();
        assert_eq!(b.address().as_deref(), Ok("s3://logs/daily/"));
        // A mailbox's folder is read trimmed (`when::mailbox`), so there the space means nothing and it reads in parts.
        let m = mailbox("imaps://u@mail.example.com/ x");
        assert_eq!((m.raw, m.addr.folder.as_str()), (false, "x"));
        // A host with neither scheme keeps the text as it is.
        let f = host("web1");
        assert!(f.raw);
        assert_eq!(f.address().as_deref(), Ok("web1"));
        // A text with a `{param}` in it reads, but the builder would refuse it: it is held whole too.
        let f = mailbox("imaps://u@mail.example.com/{box}");
        assert!(f.raw);
        assert_eq!(f.address().as_deref(), Ok("imaps://u@mail.example.com/{box}"));
        let mut b = bucket("s3://");
        assert!(b.raw);
        b.target = "s3://logs/".into();
        assert_eq!(b.address().as_deref(), Ok("s3://logs/"));
        // `entry.check()` still has the last word: a request from a raw form is checked as before.
        let mut f = host("ssh://a b@h");
        f.name = "h".into();
        assert!(f.request("b", &[]).is_err());
    }

    /// 143.49: a part the builder refuses is named by its own words and nothing is sent.
    #[test]
    fn a_refused_part_is_named() {
        let mut h = Form::add(Page::Hosts);
        assert_eq!(h.address(), Err(Line::Key("registry.addr_bad_host")));
        h.addr.host = "web1".into();
        h.addr.user = "a b".into();
        assert_eq!(h.address(), Err(Line::Key("registry.addr_bad_user")));
        h.addr.user = "ops".into();
        h.addr.port = "x".into();
        assert_eq!(h.address(), Err(Line::Key("registry.addr_bad_port")));
        h.addr.port = "0".into();
        assert_eq!(h.address(), Err(Line::Key("registry.addr_bad_port")));
        h.addr.port = "22".into();
        assert_eq!(h.address().as_deref(), Ok("ssh://ops@web1"));
        h.addr.user.clear();
        h.set_winrm(true);
        assert_eq!(h.address(), Err(Line::Key("registry.addr_winrm_user")));
        let mut b = Form::add(Page::Buckets);
        assert_eq!(b.address(), Err(Line::Key("registry.addr_bad_bucket")));
        b.addr.bucket = "logs".into();
        b.addr.folder = "a//b".into();
        assert_eq!(b.address(), Err(Line::Key("registry.addr_bad_folder")));
        let mut m = Form::add(Page::Mailboxes);
        assert_eq!(m.address(), Err(Line::Key("registry.addr_bad_user")));
        m.addr.user = "u".into();
        assert_eq!(m.address(), Err(Line::Key("registry.addr_bad_host")));
        m.addr.host = "mail.example.com".into();
        m.addr.folder = "/x".into();
        assert_eq!(m.address(), Err(Line::Key("registry.addr_bad_folder")));
        m.addr.folder = "INBOX".into();
        assert_eq!(m.address().as_deref(), Ok("imaps://u@mail.example.com"));
    }

    /// 143.50: the check button reads the parts, and stays off for an address the engine would refuse.
    #[test]
    fn the_peek_is_built_from_the_parts() {
        let mut f = Form::add(Page::Hosts);
        f.addr.host = "web1".into();
        assert_eq!(f.peek_target(), Some((PeekKind::Ssh, "web1".into(), 22)));
        f.addr.port = "2222".into();
        assert_eq!(f.peek_target(), Some((PeekKind::Ssh, "web1".into(), 2222)));
        f.addr.host = "[::1]".into();
        assert_eq!(f.peek_target(), None, "an IPv6 literal is not checked");
        f.addr.host = "ホスト".into();
        assert_eq!(f.peek_target(), None, "a non-ASCII name is not checked");
        f.addr.host = "web1".into();
        f.addr.port = "x".into();
        assert_eq!(f.peek_target(), None);
    }

    /// Every label and hint the parts are drawn with has words in both languages (how they look is checked by hand).
    #[test]
    fn the_parts_have_labels_and_hints_in_both_languages() {
        for lang in ["en", "ja"] {
            let (t, errors) = crate::i18n::table(lang);
            assert!(errors.is_empty(), "{errors:?}");
            for key in [
                "addr_user",
                "addr_user_hint_ssh",
                "addr_user_hint_winrm",
                "addr_host",
                "addr_host_hint",
                "addr_port",
                "addr_server",
                "addr_server_hint",
                "addr_mail_user_hint",
                "addr_folder",
                "addr_mail_folder_hint",
                "addr_bucket",
                "addr_bucket_hint",
                "addr_s3_folder_hint",
                "addr_raw_note",
                "addr_bad_user",
                "addr_bad_host",
                "addr_bad_port",
                "addr_bad_folder",
                "addr_bad_bucket",
                "addr_winrm_user",
            ] {
                let key = format!("registry.{key}");
                assert_ne!(t.tr(&key), key, "{lang} {key}");
            }
        }
    }
}
