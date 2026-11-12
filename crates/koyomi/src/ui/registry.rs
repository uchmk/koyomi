//! The アカウント, データベース, 接続先 PC, S3 and メールボックス pages of the settings: the engine's `[[account]]`,
//! `[[database]]`, `[[host]]`, `[[bucket]]` and `[[mailbox]]` of koyomi.toml, listed, added, changed and removed
//! through the door's `Registry` and the `…Put` and `…Remove` requests of each.
//!
//! A job names an entry (`run_as = "batch"`, `database = "sales"`, `on = "web1"`), so the secret's name and the
//! address are written once. The window only ever handles secret NAMES: no password is read, shown or sent here.
//!
//! The state is plain data, so the rules are tested without a window (as on the users page): what the page
//! may do ([`super::users::access`]), how a listing is taken, what an answer does ([`Registry::answered`])
//! and what a click asks for ([`Registry::act`]). The drawing is [`show`].

use koyomi_core::chore::{self, SqlDb, SqlTls, WinrmAuth};
use koyomi_core::registry::{BucketEntry, HostEntry, MailboxEntry};
use koyomi_core::{jobnet, msg, registry};

use super::users::{self, Access, Under};
use crate::proto::{AccountRow, BucketRow, DatabaseRow, HostRow, MailboxRow, RegistryBody, Request};

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
    /// The secret's name is empty or has a space in it.
    SecretEmpty,
}

impl Line {
    pub(super) fn text(&self, t: &ito_i18n::Table) -> String {
        match self {
            Line::Raw(s) | Line::Unsent(s) => s.clone(),
            Line::BaseChanged => t.tr("registry.base_changed").to_owned(),
            Line::ReplyBad => t.tr("registry.reply_bad").to_owned(),
            Line::NameExists(n) => t.trf("registry.name_exists", &[n]),
            Line::SecretEmpty => t.tr("registry.secret_empty").to_owned(),
        }
    }
}

/// The add / edit form of one page. Nothing here is a password: `secret` is the NAME of a secret.
#[derive(Clone, Debug)]
pub(super) struct Form {
    pub page: Page,
    /// `Some(name)`: editing that entry (its name is fixed); `None`: adding.
    pub editing: Option<String>,
    pub name: String,
    /// The `user` of an account, the `db` of a database.
    pub target: String,
    /// The secret's name (without `secret:`).
    pub secret: String,
    /// The person typed the secret's name: it no longer follows the entry's name.
    secret_typed: bool,
    pub tls: SqlTls,
    /// The rest of the host, bucket and mailbox forms.
    pub x: Extra,
    /// What the form opened with: the form is dirty when it differs.
    orig: (String, String, String, SqlTls),
    orig_x: Extra,
    /// The role or the name at the engine changed under the page: 保存 waits until the person has looked at the new listing.
    pub stale: bool,
    pub error: Option<Line>,
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
        Form {
            page,
            editing: None,
            name: String::new(),
            target: String::new(),
            secret: String::new(),
            secret_typed: false,
            tls: SqlTls::Verify,
            x: Extra::default(),
            orig: (String::new(), String::new(), String::new(), SqlTls::Verify),
            orig_x: Extra::default(),
            stale: false,
            error: None,
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
        let mut f = Form::of(Page::Hosts, &row.name, &rest, &secret, SqlTls::Verify);
        f.orig_x = x.clone();
        f.x = x;
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
        let secret = row.secret.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| row.name.clone());
        let mut f = Form::of(Page::Buckets, &row.name, &row.url, &secret, SqlTls::Verify);
        f.orig_x = x.clone();
        f.x = x;
        f
    }

    fn edit_mailbox(row: &MailboxRow) -> Form {
        Form::of(Page::Mailboxes, &row.name, &row.mail, &row.secret, SqlTls::Verify)
    }

    fn edit_account(row: &AccountRow) -> Form {
        Form::of(Page::Accounts, &row.name, &row.user, &row.secret, SqlTls::Verify)
    }

    fn edit_database(row: &DatabaseRow) -> Form {
        let secret = row.secret.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| row.name.clone());
        let tls = row.tls.as_deref().and_then(SqlTls::of).unwrap_or_default();
        Form::of(Page::Databases, &row.name, &row.db, &secret, tls)
    }

    fn of(page: Page, name: &str, target: &str, secret: &str, tls: SqlTls) -> Form {
        Form {
            page,
            editing: Some(name.to_owned()),
            name: name.to_owned(),
            target: target.to_owned(),
            secret: secret.to_owned(),
            secret_typed: true,
            tls,
            x: Extra::default(),
            orig: (name.to_owned(), target.to_owned(), secret.to_owned(), tls),
            orig_x: Extra::default(),
            stale: false,
            error: None,
        }
    }

    pub(super) fn dirty(&self) -> bool {
        (&self.name, &self.target, &self.secret, self.tls) != (&self.orig.0, &self.orig.1, &self.orig.2, self.orig.3)
            || self.x != self.orig_x
    }

    /// Whether the secret's name follows the entry's name: a WinRM host needs one, an SSH host does not.
    fn follows_name(&self) -> bool {
        !self.secret_typed && (self.page != Page::Hosts || self.x.winrm)
    }

    /// The name was typed: the secret's name follows it until the person types the secret's name.
    fn name_typed(&mut self) {
        if self.follows_name() {
            self.secret = self.name.trim().to_owned();
        }
    }

    /// The secret's name was typed: an empty one follows the entry's name again (not an SSH host's, which has
    /// none then).
    fn secret_edited(&mut self) {
        self.secret_typed = !self.secret.trim().is_empty();
        if self.follows_name() {
            self.secret = self.name.trim().to_owned();
        }
    }

    /// The kind seg of a host was picked: a WinRM host's secret follows the name, an SSH host's that does not
    /// start empty.
    pub(super) fn set_winrm(&mut self, winrm: bool) {
        self.x.winrm = winrm;
        if winrm && self.secret.trim().is_empty() {
            self.secret_typed = false;
            self.name_typed();
        } else if !winrm && !self.secret_typed {
            self.secret.clear();
        }
    }

    /// Whether the form is for a WinRM host.
    fn is_winrm(&self) -> bool {
        self.page == Page::Hosts && self.x.winrm
    }

    /// The `on` this host form writes: the seg's scheme in front of the address as typed.
    pub(super) fn host_on(&self) -> String {
        format!("{}{}", scheme(self.x.winrm), self.target.trim())
    }

    /// The server a database's `db` is, when it reads.
    fn db(&self) -> Option<SqlDb> {
        chore::sql_db(self.target.trim()).ok()
    }

    /// Whether the database takes a password (a login: SQL Server with a user, or PostgreSQL).
    pub(super) fn needs_secret(&self) -> bool {
        match self.page {
            Page::Accounts => true,
            Page::Databases => matches!(self.db(), Some(SqlDb::SqlServer { user: Some(_), .. } | SqlDb::Postgres { .. })),
            Page::Hosts | Page::Buckets | Page::Mailboxes => true,
        }
    }

    /// Whether the secret's name may be left out: an SSH host's (it can log in with a key).
    fn secret_optional(&self) -> bool {
        self.page == Page::Hosts && !self.x.winrm
    }

    /// Whether the database is a SQLite file: it takes no TLS.
    pub(super) fn is_sqlite(&self) -> bool {
        matches!(self.db(), Some(SqlDb::Sqlite { .. }))
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
        let secret = self.secret.trim().to_owned();
        let secret_ok = !secret.is_empty() && !secret.chars().any(char::is_whitespace);
        let base = base.to_owned();
        match self.page {
            Page::Accounts => {
                let user = self.target.trim().to_owned();
                if !jobnet::valid_account(&user) {
                    return Err(Line::Raw(msg::line(msg::CFG_ACCOUNT_USER, &[&name])));
                }
                if !secret_ok {
                    return Err(Line::SecretEmpty);
                }
                Ok(Request::AccountPut { name, user, password: format!("secret:{secret}"), base_sha256: base })
            }
            Page::Databases => {
                let db = self.target.trim().to_owned();
                if db.contains(['{', '}']) {
                    return Err(Line::Raw(msg::line(msg::CFG_DATABASE_BRACES, &[&name])));
                }
                let password = self.needs_secret().then(|| format!("secret:{secret}"));
                if password.is_some() && !secret_ok {
                    return Err(Line::SecretEmpty);
                }
                // `verify` is what no `tls` means; SQLite takes none.
                let tls = (!self.is_sqlite() && self.tls != SqlTls::Verify).then_some(self.tls);
                chore::check_db(&db, password.as_deref(), tls.unwrap_or_default())
                    .map_err(|e| Line::Raw(msg::line(msg::CFG_DATABASE_BAD, &[&name, msg::uncoded(&e)])))?;
                Ok(Request::DatabasePut { name, db, password, tls, base_sha256: base })
            }
            Page::Hosts => {
                let winrm = self.is_winrm();
                let opt = |s: &str| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
                let password = if self.secret_optional() && secret.is_empty() {
                    None
                } else if secret_ok {
                    Some(format!("secret:{secret}"))
                } else {
                    return Err(Line::SecretEmpty);
                };
                // Only the keys of the chosen kind go: the other kind's rows are hidden.
                let entry = HostEntry {
                    name: name.clone(),
                    on: self.host_on(),
                    host_key: if winrm { None } else { opt(&self.x.host_key) },
                    key: if winrm { None } else { opt(&self.x.key) },
                    password: password.clone(),
                    cert: if winrm { opt(&self.x.cert) } else { None },
                    auth: (winrm && self.x.auth != WinrmAuth::Basic).then_some(self.x.auth),
                };
                entry.check().map_err(Line::Raw)?;
                Ok(Request::HostPut {
                    name,
                    on: entry.on,
                    host_key: entry.host_key,
                    key: entry.key,
                    password,
                    cert: entry.cert,
                    auth: entry.auth.map(|a| a.key().to_owned()),
                    base_sha256: base,
                })
            }
            Page::Buckets => {
                if !secret_ok {
                    return Err(Line::SecretEmpty);
                }
                let opt = |s: &str| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
                let entry = BucketEntry {
                    name: name.clone(),
                    url: self.target.trim().to_owned(),
                    key_id: opt(&self.x.key_id),
                    password: Some(format!("secret:{secret}")),
                    region: opt(&self.x.region),
                    endpoint: opt(&self.x.endpoint),
                };
                entry.check().map_err(Line::Raw)?;
                Ok(Request::BucketPut {
                    name,
                    url: entry.url,
                    key_id: entry.key_id,
                    password: format!("secret:{secret}"),
                    region: entry.region,
                    endpoint: entry.endpoint,
                    base_sha256: base,
                })
            }
            Page::Mailboxes => {
                if !secret_ok {
                    return Err(Line::SecretEmpty);
                }
                let entry = MailboxEntry { name: name.clone(), mail: self.target.trim().to_owned(), password: format!("secret:{secret}") };
                entry.check().map_err(Line::Raw)?;
                Ok(Request::MailboxPut { name, mail: entry.mail, password: entry.password, base_sha256: base })
            }
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
}

/// What the browser does after a click.
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
    /// The chooser of a host's key file, while it is open (only for an engine on this PC).
    key_browse: Option<std::sync::mpsc::Receiver<Vec<std::path::PathBuf>>>,
    /// The engine is on this PC (set each frame the page is drawn).
    local: bool,
    /// A write is out and not yet answered.
    write: bool,
    refusal: Option<Line>,
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
                    self.forms[i] = Some(Form::add(page));
                }
            }
            Click::OpenEdit(name) => {
                if self.can_write(v) && !self.forms[i].as_ref().is_some_and(Form::dirty) {
                    let form = self.listing.as_ref().and_then(|l| match page {
                        Page::Accounts => l.accounts.iter().find(|a| a.name == name).map(Form::edit_account),
                        Page::Databases => l.databases.iter().find(|d| d.name == name).map(Form::edit_database),
                        Page::Hosts => l.hosts.iter().find(|d| d.name == name).map(Form::edit_host),
                        Page::Buckets => l.buckets.iter().find(|d| d.name == name).map(Form::edit_bucket),
                        Page::Mailboxes => l.mailboxes.iter().find(|d| d.name == name).map(Form::edit_mailbox),
                    });
                    if form.is_some() {
                        self.forms[i] = form;
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
        Err(_) => db.to_owned(),
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
                            match &row.secret {
                                Some(s) => secret_lines(ui, t, page, (&row.name, s, &row.state)),
                                None => line(ui, &t.trf("registry.password_line", &[state_word(t, &row.state)])),
                            }
                            used_line(ui, t, &row.used_by);
                            row_end(r, ui, t, (page, &row.name), (can, here), &mut click);
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
                            line(ui, &t.trf("registry.kind_line", &[kind, &row.on]));
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
                            line(ui, &t.trf("registry.mail_line", &[&row.mail]));
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
fn row_end(
    r: &mut Registry,
    ui: &mut egui::Ui,
    t: &ito_i18n::Table,
    (page, name): (Page, &str),
    (can, editing): (bool, bool),
    click: &mut Option<Click>,
) {
    if editing {
        form_ui(r, ui, t, page, can, click);
        return;
    }
    let id = users::tid(name);
    let tag = page.tag();
    let on = can && !r.forms[page.idx()].as_ref().is_some_and(Form::dirty);
    ui.horizontal_wrapped(|ui| {
        if users::button(
            ui,
            on,
            None,
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

/// The add / edit form of `page`.
fn form_ui(r: &mut Registry, ui: &mut egui::Ui, t: &ito_i18n::Table, page: Page, can: bool, click: &mut Option<Click>) {
    let busy = r.write;
    let local = r.local;
    let Some(f) = r.forms[page.idx()].as_mut() else { return };
    let tag = page.tag();
    let id = format!("registry.{tag}.form");
    // The key file the person picked lands in the host form that is up.
    if let Some(rx) = &r.key_browse {
        match rx.try_recv() {
            Ok(paths) => {
                r.key_browse = None;
                if let (Some(p), Page::Hosts) = (paths.first(), page) {
                    f.x.key = p.display().to_string();
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(100)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => r.key_browse = None,
        }
    }
    let mut browsed = false;
    ui.add_enabled_ui(!busy, |ui| {
        ui.horizontal(|ui| {
            ui.label(t.tr("registry.name_field"));
            let adding = f.editing.is_none();
            let resp = ui.add_enabled(adding, egui::TextEdit::singleline(&mut f.name).desired_width(200.0));
            ito_a11y::say(&resp, ito_a11y::Role::Label, t.tr("registry.name_field"));
            ito_a11y::test_id(&resp, &format!("{id}.name"));
            if resp.changed() {
                f.name_typed();
            }
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
        ui.horizontal(|ui| {
            let (label, hint) = match page {
                Page::Accounts => ("registry.user_field", "registry.user_hint"),
                Page::Databases => ("registry.db_field", "field.sql_db_hint"),
                Page::Hosts if f.x.winrm => ("registry.host_on", "registry.host_on_hint_winrm"),
                Page::Hosts => ("registry.host_on", "registry.host_on_hint_ssh"),
                Page::Buckets => ("registry.bucket_url", "registry.bucket_url_hint"),
                Page::Mailboxes => ("registry.mailbox_mail", "registry.mailbox_mail_hint"),
            };
            ui.label(t.tr(label));
            if page == Page::Hosts {
                ui.label(egui::RichText::new(scheme(f.x.winrm)).monospace());
            }
            let resp = ui.add(egui::TextEdit::singleline(&mut f.target).hint_text(t.tr(hint)).desired_width(320.0));
            ito_a11y::say(&resp, ito_a11y::Role::Label, t.tr(label));
            ito_a11y::test_id(&resp, &format!("{id}.target"));
        });
        match page {
            Page::Hosts if !f.x.winrm => {
                typed_line(ui, t, &id, ("registry.host_key_field", "host_key", "registry.host_key_hint"), &mut f.x.host_key);
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
                        browsed = b.clicked();
                    }
                });
            }
            Page::Hosts => {
                typed_line(ui, t, &id, ("registry.cert_field", "cert", "registry.cert_hint"), &mut f.x.cert);
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
        if f.needs_secret() {
            ui.horizontal(|ui| {
                ui.label(t.tr("registry.secret_field"));
                let resp = ui.add(egui::TextEdit::singleline(&mut f.secret).desired_width(200.0));
                ito_a11y::say(&resp, ito_a11y::Role::Label, t.tr("registry.secret_field"));
                ito_a11y::test_id(&resp, &format!("{id}.secret"));
                if resp.changed() {
                    f.secret_edited();
                }
            });
            let note =
                egui::RichText::new(t.tr(if f.secret_optional() { "registry.secret_note_optional" } else { "registry.secret_note" }))
                    .color(ito_theme::colors().dim);
            ui.add(egui::Label::new(note).wrap());
        }
        if page == Page::Buckets {
            typed_line(ui, t, &id, ("registry.region_field", "region", "registry.region_hint"), &mut f.x.region);
            typed_line(ui, t, &id, ("registry.endpoint_field", "endpoint", "registry.endpoint_hint"), &mut f.x.endpoint);
        }
        if page == Page::Databases && !f.is_sqlite() {
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
        }
    });
    if browsed && r.key_browse.is_none() {
        let title = t.tr("registry.key_file").to_owned();
        r.key_browse = Some(ito_dialog::spawn(move || ito_dialog::Dialog::new().title(&title).open_file().into_iter().collect()));
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
            assert_eq!(f.host_on(), format!("{}{rest}", scheme(winrm)), "{on}");
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
        assert_eq!(f.request("b", &[]).err(), Some(Line::SecretEmpty));
        // A bucket and a mailbox.
        let f = Form::edit_bucket(&conn_body().buckets[0]);
        match f.request("b", &[]) {
            Ok(Request::BucketPut { name, url, password, .. }) => {
                assert_eq!((name.as_str(), url.as_str(), password.as_str()), ("logs", "s3://logs/daily/", "secret:logs"));
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
            },
            Request::HostRemove { name: "web1".into(), base_sha256: "base1".into(), force: false },
            Request::BucketPut {
                name: "logs".into(),
                url: "s3://logs/".into(),
                key_id: None,
                password: "secret:logs".into(),
                region: None,
                endpoint: None,
                base_sha256: "base1".into(),
            },
            Request::BucketRemove { name: "logs".into(), base_sha256: "base1".into(), force: false },
            Request::MailboxPut {
                name: "billing".into(),
                mail: "imaps://bill@mail".into(),
                password: "secret:bill".into(),
                base_sha256: "base1".into(),
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
    fn an_account_form_is_checked_before_it_is_sent_and_the_secret_follows_the_name() {
        let taken = vec!["ops".to_owned()];
        let mut f = Form::add(Page::Accounts);
        f.name = "batch".into();
        f.name_typed();
        assert_eq!(f.secret, "batch");
        f.target = "CORP\\batch".into();
        match f.request("base1", &taken) {
            Ok(Request::AccountPut { name, user, password, base_sha256 }) => {
                assert_eq!(
                    (name.as_str(), user.as_str(), password.as_str(), base_sha256.as_str()),
                    ("batch", "CORP\\batch", "secret:batch", "base1")
                );
            }
            other => panic!("{other:?}"),
        }
        // A secret name typed by the person stops following the name.
        f.secret = "other".into();
        f.secret_edited();
        f.name = "batch2".into();
        f.name_typed();
        assert_eq!(f.secret, "other");
        let mut bad = Form::add(Page::Accounts);
        bad.name = "a/b".into();
        bad.target = "CORP\\x".into();
        assert!(matches!(bad.request("b", &taken), Err(Line::Raw(_))));
        bad.name = "ops".into();
        assert_eq!(bad.request("b", &taken).unwrap_err(), Line::NameExists("ops".into()));
        bad.name = "new".into();
        bad.target = "bad user".into();
        bad.secret = "new".into();
        assert!(matches!(bad.request("b", &taken), Err(Line::Raw(_))));
        bad.target = "CORP\\x".into();
        bad.secret = "has space".into();
        assert_eq!(bad.request("b", &taken).unwrap_err(), Line::SecretEmpty);
        // Editing keeps the name (it is not checked against the taken ones).
        let mut e = Form::edit_account(&body().accounts[0]);
        assert!(matches!(e.request("b", &taken), Ok(Request::AccountPut { .. })));
        e.target = "CORP\\ops2".into();
        assert!(e.dirty());
    }

    #[test]
    fn a_database_form_sends_a_secret_and_tls_only_where_they_apply() {
        let mut f = Form::add(Page::Databases);
        f.name = "lite".into();
        f.name_typed();
        f.target = "sqlite://C:\\data\\a.db".into();
        match f.request("b", &[]) {
            Ok(Request::DatabasePut { password, tls, .. }) => assert_eq!((password, tls), (None, None)),
            other => panic!("{other:?}"),
        }
        f.target = "sqlserver://u@h/d".into();
        f.tls = SqlTls::Trust;
        match f.request("b", &[]) {
            Ok(Request::DatabasePut { password, tls, .. }) => {
                assert_eq!((password.as_deref(), tls), (Some("secret:lite"), Some(SqlTls::Trust)));
            }
            other => panic!("{other:?}"),
        }
        // SQL Server without a user signs in as the account: no password.
        f.target = "sqlserver://h/d".into();
        f.tls = SqlTls::Verify;
        match f.request("b", &[]) {
            Ok(Request::DatabasePut { password, tls, .. }) => assert_eq!((password, tls), (None, None)),
            other => panic!("{other:?}"),
        }
        f.target = "sqlserver://h/{x}".into();
        assert!(matches!(f.request("b", &[]), Err(Line::Raw(_))));
        f.target = "nonsense".into();
        assert!(matches!(f.request("b", &[]), Err(Line::Raw(_))));
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
