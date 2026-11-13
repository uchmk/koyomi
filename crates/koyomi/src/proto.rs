//! What the window and the CLI say to the engine through its door, and what the engine says back.
//!
//! One request and one reply per connection (`ito_ipc::door`). The reply is
//! text: TOML for `List`, `Show`, `Runs`, `History`, `Versions` and `Tray`, the log's tail for `Log`, a line for the rest.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::NaiveDateTime;
use ito_ipc::door::Door;
use koyomi_core::blackout::{self, Blackout};
use koyomi_core::calendar::Calendar;
use koyomi_core::held::{HeldJob, Skip};
use koyomi_core::role::Role;
use koyomi_core::run::{Run, State};
use koyomi_core::scan::{Def, Standing};
use koyomi_core::{quiet, when};
use serde::{Deserialize, Serialize};

/// Bumped when `Request` or what a reply holds changes shape.
pub const PROTO: u32 = 53;

/// A value that must not be printed: a password typed in the form. `Debug` writes `***`, and there is no
/// `Display`. On the wire it is the plain string.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Hidden(String);

impl Hidden {
    /// A value made from a string (the window builds its value in place with [`Hidden::as_mut_string`]).
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The value itself. Only for sending it on and storing it; never for a message.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// For a masked text field to edit in place.
    pub fn as_mut_string(&mut self) -> &mut String {
        &mut self.0
    }
}

impl std::fmt::Debug for Hidden {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

/// How long the door waits for the engine to answer a [`Request::HostPeek`] (the peek itself takes up to
/// about 20 s).
pub const PEEK_ANSWER_WAIT: Duration = Duration::from_secs(25);

/// How long the window's far link waits for that answer: above [`PEEK_ANSWER_WAIT`], so the engine's own
/// reply (or its refusal) arrives before the link gives up.
pub const PEEK_CALL_WAIT: Duration = Duration::from_secs(30);

pub const ENV: &str = "KOYOMI_ADDRESS";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    /// One line on the running engine.
    Status,
    /// The engine ends after it answers.
    Stop,
    /// Every definition and the warnings, as a [`Snapshot`].
    List,
    /// One definition, as a [`Def`] (what an approval shows).
    Show {
        id: String,
    },
    /// Approve `id` as it reads now, if it still hashes to `sha256`.
    Approve {
        id: String,
        sha256: String,
    },
    /// Start the net `id` now for the business day `date` (`YYYY-MM-DD`; today when none),
    /// at the job `from` when there is one (the jobs before it are skipped).
    /// The reply is the run's id.
    Run {
        id: String,
        date: Option<String>,
        #[serde(default)]
        from: Option<String>,
        /// Start nothing: the reply is each job's command as it would run.
        #[serde(default)]
        dry: bool,
        /// Values for the net's `[params]` in place of their defaults (`--set name=value`).
        /// Always written: postcard is not self-describing, so a skipped field shifts the rest.
        #[serde(default)]
        params: BTreeMap<String, String>,
    },
    /// Start `run` again with the definition it had: what passed is carried
    /// over, the rest runs (from the job `from`, when there is one). With `next` (⊘ スキップ or
    /// 警告), `from` does not run but ends as that, and the jobs after it go. The reply is the new run's id.
    Rerun {
        run: String,
        from: Option<String>,
        next: Option<State>,
    },
    /// The text of `definition.toml` as it was written when `run` started (`secret:<name>` stays a
    /// reference). The reply is that text. Refused for a run that never began and for one whose
    /// copy is gone.
    RunDefinition {
        run: String,
    },
    /// Stop one job of a run, or the whole run.
    Kill {
        run: String,
        job: Option<String>,
    },
    /// Today's runs and the ones still going, as [`Runs`].
    Runs,
    /// The runs that started on the days `from` to `to` (`YYYY-MM-DD`, both
    /// included, at most a year), oldest first, as [`Runs`]: what `koyomi history`
    /// and the window's 履歴 show. The narrowing by net and state is theirs.
    History {
        from: String,
        to: String,
    },
    /// The tail of one job's log.
    Log {
        run: String,
        job: String,
    },
    /// Put the net `id` on hold: its times are written down as ⏸ 保留 and do not run, nor does `Run`.
    /// With `job`, only that job: a run that comes to it waits there as ⏸ 保留 until it is released.
    Hold {
        id: String,
        job: Option<String>,
    },
    /// Take the net `id` (or its job `job`) off hold.
    Release {
        id: String,
        job: Option<String>,
    },
    /// Skip the job `job` of `run` before it starts: ⊘, and the jobs after it go on.
    Skip {
        run: String,
        job: String,
    },
    /// Answer the 確認待ち part `job` of `run`: `yes` lets the jobs after it go on, else it ends 異常.
    Answer {
        run: String,
        job: String,
        yes: bool,
    },
    /// Start the time `from` (`YYYY-MM-DD HH:MM`) of the net `id` at `to` this once; the definition stays.
    /// Without `to` the move is taken back.
    Shift {
        id: String,
        from: String,
        to: Option<String>,
    },
    /// Skip the next time of the net `id` this once (it is written down as skipped); the definition
    /// stays. Each call skips the time after the ones already skipped. With `cancel`, the skips are taken back.
    SkipNext {
        id: String,
        cancel: bool,
    },
    /// Stop the whole schedule: every time is written down as ⏸ 保留 until `Resume`. Runs by hand still go.
    Pause,
    Resume,
    /// `[standby]`: where this engine stands (watching or taken over); with `give_back`, hand the
    /// schedules back to the PC it watches and watch again.
    Standby {
        give_back: bool,
    },
    /// The HA handshake (TODO 96): `"ha"` of `GET /health` (JSON: a standby says whether it has
    /// the schedules and may give them back, a primary whether it runs them); with `release`, a
    /// primary's `POST /standby/release` taking them back from this standby.
    Handover {
        release: bool,
    },
    /// `request` came over HTTP from `by` (`[[http.user]]`'s name, or `default` for `[http]`'s
    /// token): written down with who did it. Only what the HTTP paths ask for goes through.
    Over {
        by: String,
        request: Box<Request>,
    },
    /// The editor's save: `text` becomes `jobnets/<id>.toml` and counts as confirmed
    /// (the editor shows what it saves). `base_sha256` is what the file hashed to when
    /// the editor read it (`None`: a new net); when the file is not that now, the save
    /// is refused with `ほかで変わっています` unless `force`. The file as it was goes
    /// under `versions/` first.
    Save {
        id: String,
        text: String,
        base_sha256: Option<String>,
        force: bool,
    },
    /// The versions kept of the net `id`, newest first, as [`Versions`].
    Versions {
        id: String,
    },
    /// The text of the version `version` of the net `id` (`current`: the file as it is now).
    VersionText {
        id: String,
        version: String,
    },
    /// Put the version `version` of the net `id` back as its file (the file as it is
    /// goes under `versions/` first). The net is unconfirmed until approved.
    Restore {
        id: String,
        version: String,
    },
    /// Give the net `id` the id `to` (another group moves it). Refused while it runs, and while
    /// other nets name it unless `fix`: then their `use`, `wait_for` and `after_jobnet` are
    /// pointed at `to` too. Each file written over goes under `versions/` first; the versions of
    /// `id`, its approval and its hold go with it.
    Rename {
        id: String,
        to: String,
        fix: bool,
    },
    /// Write the net `id`'s file again as `to`, unconfirmed. `title` is the copy's `name` (empty:
    /// none, so the tree shows the file name); without it a name that was the old file name's
    /// (or empty) follows the new one.
    Copy {
        id: String,
        to: String,
        #[serde(default)]
        title: Option<String>,
    },
    /// Every line of every net that holds `from`, as it is and as it would read with `to`. With
    /// `apply` the files are written that way, each one under `versions/` first, and the nets
    /// are unconfirmed again. Refused while a net it would change runs, or when a result no
    /// longer reads as a net (nothing is written then).
    Replace {
        from: String,
        to: String,
        apply: bool,
    },
    /// Take the net `id`'s file away (kept under `versions/` first, so `Restore` brings it
    /// back). Refused while it runs or other nets name it.
    Delete {
        id: String,
    },
    /// Bring a bundle (`koyomi export`'s file) in: answers a [`Plan`]. With
    /// `dry_run` nothing is written. `on_conflict` is `overwrite`, `rename` or
    /// `skip`; without it a bundle that meets something different is only planned.
    Import {
        text: String,
        dry_run: bool,
        on_conflict: Option<String>,
    },
    /// Where a calendar, a net (`グループ/名前`) or a program is used, one line each.
    Uses {
        target: String,
    },
    /// What people did by hand from `from` to `to` (`YYYY-MM-DD`), one line each, oldest first.
    Ops {
        from: String,
        to: String,
    },
    /// What `koyomi tray` shows, as a [`TrayView`], with the notices kept after
    /// `since` (`None`: none, only the number of the last one).
    Tray {
        since: Option<u64>,
    },
    /// The notices the engine keeps for a window on another PC to show (`notify::notices`), as a
    /// [`NoticesView`]. `boot` and `since` are what the last answer said (`None` for both: the
    /// first ask, which only learns where the engine is). Another `boot` than the engine's
    /// (it started again) gets every notice kept, whatever `since` is.
    Notices {
        boot: Option<String>,
        since: Option<u64>,
    },
    /// A definition sent over HTTP by someone with the `edit` role (TODO 83): checked and
    /// written as [`Request::Save`] does (`base_sha256` the same way), but not confirmed.
    /// It stays unconfirmed, and runs only after this PC's window confirms it.
    Propose {
        id: String,
        text: String,
        base_sha256: Option<String>,
    },
    /// Keep `text` as the definition of `id` from the day `from` (`YYYY-MM-DD`, after today) on
    /// (TODO 110): approved now, checked as a save is, and kept in `next.toml`. Until that day
    /// the net runs as it is; at its start the engine writes the text as the net's file (the
    /// old one under `versions/`) before any time of the day starts. One per net: staging again
    /// replaces it. Only from this PC's window or command line, like an approval.
    ///
    /// `expect` (from a window that read the definition and the staged one before the person
    /// edited; `None` from the command line): what they must still be, else it is refused with
    /// `ほかで変わっています`. `POST /door` takes only a request with it.
    Stage {
        id: String,
        text: String,
        from: String,
        #[serde(default)]
        expect: Option<StageExpect>,
    },
    /// Drop what was staged for `id`: the net runs as it is.
    Unstage {
        id: String,
    },
    /// The text of the calendar `name` (`calendars/<name>.toml`) and its SHA-256, as a [`CalendarBody`].
    CalendarText {
        name: String,
    },
    /// Write `text` as the calendar `name`; the file as it was goes under `versions/_calendars/`.
    /// `base_sha256` is what the file hashed to when it was read (`None`: a new calendar); when it
    /// is not that now, the write is refused with `ほかで変わっています`.
    CalendarPut {
        name: String,
        text: String,
        base_sha256: Option<String>,
    },
    /// Give the calendar `from` the name `to`; refused when `to` is there.
    CalendarRename {
        from: String,
        to: String,
    },
    /// Take the calendar `name` away (kept under `versions/_calendars/`), if it still hashes to `base_sha256`.
    CalendarDelete {
        name: String,
        base_sha256: String,
    },
    /// Write `bytes` as the holiday table, over the one there (which is kept).
    HolidayTable {
        bytes: Vec<u8>,
    },
    /// Everything in the settings folder as one bundle (`koyomi export --all`), as its TOML.
    ExportAll,
    /// The `[[http.user]]` of koyomi.toml and how the engine sees each one's token, as a
    /// [`koyomi_core::users::Listing`] (its `base` is what the two requests below are made against).
    /// Never a token value.
    Users,
    /// Give the user `name` the role `role` and `nets` and `roles` (`None`: the key is taken out), if
    /// koyomi.toml still hashes to `base`. The engine's sweep takes it up within a few seconds.
    UserSet {
        name: String,
        role: String,
        nets: Option<Vec<String>>,
        roles: Option<BTreeMap<String, String>>,
        base: String,
    },
    /// Take the user `name` out of koyomi.toml, if it still hashes to `base`.
    UserRemove {
        name: String,
        base: String,
    },
    /// Add the user `name` with a token the engine makes, if koyomi.toml still hashes to `base`. Only the
    /// token's hash goes into koyomi.toml; the token is in the reply and nowhere else.
    UserAdd {
        name: String,
        role: String,
        nets: Option<Vec<String>>,
        roles: Option<BTreeMap<String, String>>,
        base: String,
    },
    /// Make a new token for the user `name` (the old one stops working), if koyomi.toml still hashes to
    /// `base`. The token is in the reply and nowhere else.
    UserReset {
        name: String,
        base: String,
    },
    /// The text of koyomi.toml and its SHA-256, as a [`ConfigBody`] (a missing file is an empty text).
    ConfigText,
    /// Write `text` as koyomi.toml; the file as it was goes under `versions/_settings/`. `base_sha256` is
    /// what the file hashed to when it was read; when it is not that now, nothing is written. The engine
    /// takes the new file up within a few seconds.
    ConfigPut {
        text: String,
        base_sha256: String,
    },
    /// The registered accounts and databases of koyomi.toml with the state of their secrets and the nets
    /// that name them, as a [`RegistryBody`]. Never a password: only its `secret:` name.
    Registry,
    /// Add or change the `[[account]]` `name` (`password` is `secret:<name>`), if koyomi.toml still hashes
    /// to `base_sha256`. Everything else in the file stays as written.
    AccountPut {
        name: String,
        user: String,
        password: String,
        base_sha256: String,
    },
    /// Remove the `[[account]]` `name`. Refused while a net names it, unless `force`.
    AccountRemove {
        name: String,
        base_sha256: String,
        #[serde(default)]
        force: bool,
    },
    /// Add or change the `[[database]]` `name`.
    DatabasePut {
        name: String,
        db: String,
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        tls: Option<koyomi_core::chore::SqlTls>,
        base_sha256: String,
        /// A password typed in the form. With it, `password` must be `None`: the engine picks the secret
        /// (`koyomi-db-<name>`, or the entry's own when nothing else uses it), stores the value there
        /// and writes `password = "secret:<that>"`.
        #[serde(default)]
        password_value: Option<Hidden>,
    },
    /// Remove the `[[database]]` `name`. Refused while a net names it, unless `force`.
    DatabaseRemove {
        name: String,
        base_sha256: String,
        #[serde(default)]
        force: bool,
    },
    /// Add or change the `[[host]]` `name`: a PC an `ssh` / `winrm` job runs on or an SFTP `send` / `fetch` uses.
    /// `password` is `secret:<name>`. Everything else in the file stays as written.
    HostPut {
        name: String,
        on: String,
        #[serde(default)]
        host_key: Option<String>,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        cert: Option<String>,
        #[serde(default)]
        auth: Option<String>,
        base_sha256: String,
    },
    /// Remove the `[[host]]` `name`. Refused while a net names it, unless `force`.
    HostRemove {
        name: String,
        base_sha256: String,
        #[serde(default)]
        force: bool,
    },
    /// Connect from the engine's PC to `host:port` and say what the host shows: the SSH host key
    /// (`SHA256:…`) or the WinRM certificate's fingerprint. Nothing is logged in and nothing is
    /// written; no user or password travels. `ticket` is the window's own number: the engine ignores it.
    HostPeek {
        kind: PeekKind,
        host: String,
        port: u16,
        ticket: u64,
    },
    /// Add or change the `[[bucket]]` `name`: an S3 place a `send` / `fetch` uses. `password` (and `key_id`,
    /// when it is not plain) is `secret:<name>`.
    BucketPut {
        name: String,
        url: String,
        #[serde(default)]
        key_id: Option<String>,
        password: String,
        #[serde(default)]
        region: Option<String>,
        #[serde(default)]
        endpoint: Option<String>,
        base_sha256: String,
    },
    /// Remove the `[[bucket]]` `name`. Refused while a net names it, unless `force`.
    BucketRemove {
        name: String,
        base_sha256: String,
        #[serde(default)]
        force: bool,
    },
    /// Add or change the `[[mailbox]]` `name`: a mailbox a `[[when]] mailbox` watches. `password` is `secret:<name>`.
    MailboxPut {
        name: String,
        mail: String,
        password: String,
        base_sha256: String,
    },
    /// Remove the `[[mailbox]]` `name`. Refused while a net names it, unless `force`.
    MailboxRemove {
        name: String,
        base_sha256: String,
        #[serde(default)]
        force: bool,
    },
    /// Connect from the engine's PC to the database `db` (built by the form with `chore::db_url`: `postgres://…` or
    /// `sqlite://…`; SQL Server is refused) and run a probe that changes nothing. The password is `password_value` when
    /// typed, else the saved one of the `[[database]]` `entry` (the form being edited), used only for the same server,
    /// port and user and a TLS not weaker than the saved one. Nothing is written. `ticket` is the window's own number:
    /// the engine ignores it.
    DatabaseTest {
        db: String,
        #[serde(default)]
        tls: Option<koyomi_core::chore::SqlTls>,
        #[serde(default)]
        entry: Option<String>,
        #[serde(default)]
        password_value: Option<Hidden>,
        ticket: u64,
    },
}

/// What a window read before it staged a definition: the live definition's SHA-256 (the version the
/// text was edited from), and the seal of the staged one it saw ([`koyomi_core::next::Entry::seal`];
/// `None`: none staged).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageExpect {
    pub live: String,
    #[serde(default)]
    pub staged: Option<String>,
}

/// What `CalendarText` answers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarBody {
    pub text: String,
    pub sha256: String,
}

impl CalendarBody {
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// Read by a window of another PC, which is wired apart from the engine.
    pub fn parse(text: &str) -> Result<CalendarBody, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

/// What `ConfigText` answers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigBody {
    pub text: String,
    pub sha256: String,
}

impl ConfigBody {
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// Read by a window of another PC, which is wired apart from the engine.
    pub fn parse(text: &str) -> Result<ConfigBody, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

/// What `Registry` and the four writes answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryBody {
    /// SHA-256 of koyomi.toml as read: the `base_sha256` of the next write.
    pub sha256: String,
    #[serde(default)]
    pub accounts: Vec<AccountRow>,
    #[serde(default)]
    pub databases: Vec<DatabaseRow>,
    #[serde(default)]
    pub hosts: Vec<HostRow>,
    #[serde(default)]
    pub buckets: Vec<BucketRow>,
    #[serde(default)]
    pub mailboxes: Vec<MailboxRow>,
}

/// One `[[host]]`; `kind` is `ssh` or `winrm` (empty when `on` does not read), `secret` the password's
/// `secret:` name, `state` is `ok`, `missing`, `unreadable` or `not-needed` (no password), and `used_by` the
/// nets that name it. The login keys are shown as written (a key file path and the fingerprints, never a secret).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostRow {
    pub name: String,
    pub on: String,
    pub kind: String,
    #[serde(default)]
    pub host_key: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub cert: Option<String>,
    #[serde(default)]
    pub auth: Option<String>,
    #[serde(default)]
    pub secret: Option<String>,
    pub state: String,
    #[serde(default)]
    pub used_by: Vec<String>,
}

/// One `[[bucket]]`. `key_id` is as written when it is plain, and `None` when it is a `secret:<name>`: then
/// `key_id_secret` is that name and `key_id_state` its state. `secret` is the password's name and `state` its state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BucketRow {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub key_id: Option<String>,
    #[serde(default)]
    pub key_id_secret: Option<String>,
    #[serde(default)]
    pub key_id_state: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub secret: Option<String>,
    pub state: String,
    #[serde(default)]
    pub used_by: Vec<String>,
}

/// One `[[mailbox]]`; `secret` is the password's `secret:` name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxRow {
    pub name: String,
    pub mail: String,
    pub secret: String,
    pub state: String,
    #[serde(default)]
    pub used_by: Vec<String>,
}

/// One `[[account]]`; `state` is `ok`, `missing`, `unreadable` (its secret), and `used_by` the nets that name it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRow {
    pub name: String,
    pub user: String,
    /// The secret's name (the `secret:` is taken off).
    pub secret: String,
    pub state: String,
    #[serde(default)]
    pub used_by: Vec<String>,
}

/// One `[[database]]`; `state` is `ok`, `missing`, `unreadable` or `not-needed` (no password).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseRow {
    pub name: String,
    /// `sqlserver`, `postgres` or `sqlite`.
    pub kind: String,
    pub db: String,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub tls: Option<String>,
    pub state: String,
    #[serde(default)]
    pub used_by: Vec<String>,
}

impl RegistryBody {
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// Read by a window of another PC, which is wired apart from the engine.
    pub fn parse(text: &str) -> Result<RegistryBody, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

/// A `[[database]]` as a window may list it for a sql job: its name and kind, nothing else.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseName {
    pub name: String,
    pub kind: String,
}

/// A `[[host]]` as a window may list it for a job form: its name and kind (`ssh` / `winrm`), nothing else.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostName {
    pub name: String,
    pub kind: String,
}

/// A `[groups]` entry as a window may list it: its name and the kind of its members (`ssh` / `winrm`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupName {
    pub name: String,
    pub kind: String,
}

/// What a [`Request::HostPeek`] asks the host for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PeekKind {
    /// The SSH host key, as `SHA256:…`.
    Ssh,
    /// The WinRM (HTTPS) certificate's fingerprint.
    Winrm,
}

/// Whether `host` and `port` may be peeked at: 1 to 253 characters of ASCII letters, digits, `.`, `-` and
/// `_` (so no IPv6 literal, `@`, `:`, `/` or space), and a port that is not 0. The window checks it before
/// it offers the button and the engine before it connects, with this one rule.
pub fn peek_target_ok(host: &str, port: u16) -> bool {
    (1..=253).contains(&host.len()) && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')) && port != 0
}

/// The role `POST /door` asks for to pass `r` on to the engine; `None`: it cannot be done from there
/// (it is for this PC alone, or it is not for HTTP to confirm). A `match` without a wildcard: a new
/// request does not compile until it is placed here.
///
/// `List` needs `edit` because the snapshot holds every definition's text, which `view` and `run`
/// may not read (`role::needs`). `Save{force}` and `Stage` without `expect` are not let through:
/// they would confirm what no one compared.
pub fn door_needs(r: &Request) -> Option<Role> {
    match r {
        Request::Status | Request::Runs | Request::History { .. } | Request::Log { .. } | Request::Ops { .. } => Some(Role::View),
        Request::Handover { release: false } => Some(Role::View),
        Request::Run { .. }
        | Request::Rerun { .. }
        | Request::Kill { .. }
        | Request::Skip { .. }
        | Request::Answer { .. }
        | Request::Hold { .. }
        | Request::Release { .. }
        | Request::SkipNext { .. }
        | Request::Shift { .. } => Some(Role::Run),
        Request::List
        | Request::RunDefinition { .. }
        | Request::Show { .. }
        | Request::Versions { .. }
        | Request::VersionText { .. }
        | Request::Uses { .. }
        | Request::CalendarText { .. }
        | Request::Notices { .. }
        | Request::Propose { .. }
        | Request::Copy { .. }
        | Request::Restore { .. }
        | Request::Replace { .. } => Some(Role::Edit),
        Request::Save { force: false, .. }
        | Request::Approve { .. }
        | Request::Stage { expect: Some(_), .. }
        | Request::Unstage { .. }
        | Request::Import { .. }
        | Request::Rename { .. }
        | Request::Delete { .. }
        | Request::Pause
        | Request::Resume
        | Request::CalendarPut { .. }
        | Request::CalendarRename { .. }
        | Request::CalendarDelete { .. }
        | Request::HolidayTable { .. }
        | Request::ExportAll
        | Request::Users
        | Request::ConfigText
        | Request::ConfigPut { .. }
        | Request::Registry
        | Request::AccountPut { .. }
        | Request::AccountRemove { .. }
        | Request::DatabasePut { .. }
        | Request::DatabaseRemove { .. }
        | Request::HostPut { .. }
        | Request::HostRemove { .. }
        | Request::HostPeek { .. }
        | Request::DatabaseTest { .. }
        | Request::BucketPut { .. }
        | Request::BucketRemove { .. }
        | Request::MailboxPut { .. }
        | Request::MailboxRemove { .. }
        | Request::UserSet { .. }
        | Request::UserRemove { .. }
        | Request::UserAdd { .. }
        | Request::UserReset { .. } => Some(Role::Admin),
        Request::Stop
        | Request::Tray { .. }
        | Request::Standby { .. }
        | Request::Handover { release: true }
        | Request::Over { .. }
        | Request::Save { force: true, .. }
        | Request::Stage { expect: None, .. } => None,
    }
}

impl Request {
    /// Whether the request leaves the engine as it was: a failed read can be asked again, and one
    /// that is not (a run, a save …) is never sent twice on its own. A `match` without a wildcard.
    #[allow(dead_code)] // the window of another PC decides its retries by this
    pub fn is_read(&self) -> bool {
        is_read(self)
    }
}

/// See [`Request::is_read`].
#[allow(dead_code)]
pub fn is_read(r: &Request) -> bool {
    match r {
        Request::Status
        | Request::Runs
        | Request::History { .. }
        | Request::Log { .. }
        | Request::Ops { .. }
        | Request::Handover { release: false }
        | Request::List
        | Request::RunDefinition { .. }
        | Request::Show { .. }
        | Request::Versions { .. }
        | Request::VersionText { .. }
        | Request::Uses { .. }
        | Request::CalendarText { .. }
        | Request::ExportAll
        | Request::Users
        | Request::ConfigText
        | Request::Registry
        | Request::Notices { .. }
        | Request::HostPeek { .. }
        | Request::Tray { .. } => true,
        Request::Stop
        | Request::Approve { .. }
        | Request::Run { .. }
        | Request::Rerun { .. }
        | Request::Kill { .. }
        | Request::Hold { .. }
        | Request::Release { .. }
        | Request::Skip { .. }
        | Request::Answer { .. }
        | Request::Shift { .. }
        | Request::SkipNext { .. }
        | Request::Pause
        | Request::Resume
        | Request::Standby { .. }
        | Request::Handover { release: true }
        | Request::Over { .. }
        | Request::Save { .. }
        | Request::Restore { .. }
        | Request::Rename { .. }
        | Request::Copy { .. }
        | Request::Replace { .. }
        | Request::Delete { .. }
        | Request::Import { .. }
        | Request::Propose { .. }
        | Request::Stage { .. }
        | Request::Unstage { .. }
        | Request::CalendarPut { .. }
        | Request::CalendarRename { .. }
        | Request::CalendarDelete { .. }
        | Request::HolidayTable { .. }
        | Request::ConfigPut { .. }
        | Request::AccountPut { .. }
        | Request::AccountRemove { .. }
        | Request::DatabasePut { .. }
        | Request::DatabaseRemove { .. }
        // A retry would sign in again, and a server with a lockout policy counts failed sign-ins.
        | Request::DatabaseTest { .. }
        | Request::HostPut { .. }
        | Request::HostRemove { .. }
        | Request::BucketPut { .. }
        | Request::BucketRemove { .. }
        | Request::MailboxPut { .. }
        | Request::MailboxRemove { .. }
        | Request::UserSet { .. }
        | Request::UserRemove { .. }
        | Request::UserAdd { .. }
        | Request::UserReset { .. } => false,
    }
}

pub fn door() -> Door<Request> {
    let mut door = Door::new("koyomi", ENV, PROTO);
    door.wait = answer_wait;
    door
}

/// How long the door waits for the engine's reply to `r`: the peek of a host and the test of a database take longer
/// than a read from memory, bare or inside `Over`; every other request keeps the door's usual time.
fn answer_wait(r: &Request) -> Duration {
    match r {
        Request::HostPeek { .. } | Request::DatabaseTest { .. } => PEEK_ANSWER_WAIT,
        Request::Over { request, .. } if matches!(**request, Request::HostPeek { .. } | Request::DatabaseTest { .. }) => PEEK_ANSWER_WAIT,
        _ => ito_ipc::door::ANSWER_WAIT,
    }
}

/// The variable that names another address for the window's door.
pub const WINDOW_ENV: &str = "KOYOMI_WINDOW_ADDRESS";

/// What the tray asks the window (DECISIONS C1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Show {
    /// Come forward.
    Front,
    /// Come forward on today's runs (ガント).
    Today,
    /// Come forward on the newest failure not yet marked 対応済み (今日 when there is none).
    Failed,
    /// Come forward with the box that skips a net's next time once.
    SkipNext,
    /// Come forward on this run (a click on its notice).
    Run(String),
}

/// The window's own door: the engine's tray brings it forward through it.
pub fn window_door() -> Door<Show> {
    Door::new("koyomi-window", WINDOW_ENV, PROTO)
}

/// What `List` answers: the tree, and why nothing can be approved (when it cannot).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub defs: Vec<Def>,
    /// One line each: `<path> は <who> が書き換えられる`, or that `approved.toml` does not read.
    /// Not empty: nothing can be approved.
    pub warnings: Vec<String>,
    /// One line each about `[notify]` (a plain-text address, an unknown event). Runs still start.
    #[serde(default)]
    pub notes: Vec<String>,
    /// The ids on hold (`koyomi hold`).
    #[serde(default)]
    pub held: Vec<String>,
    /// The jobs on hold (`koyomi hold <net> <job>`).
    #[serde(default)]
    pub held_jobs: Vec<HeldJob>,
    /// The times skipped once (`koyomi skip-next`), not yet come.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skips: Vec<Skip>,
    /// `koyomi pause`: no time starts anything.
    #[serde(default)]
    pub paused: bool,
    /// One line each about the calendars (a file that does not read, the
    /// holiday table's age), a planned stop, a standby, a staged switch that
    /// failed, and a `[[when]]` that cannot come here (a mailbox or event log
    /// that does not read, an idle or session this engine cannot see).
    /// Nothing is stopped by these, and every net can still be approved.
    #[serde(default)]
    pub cautions: Vec<String>,
    /// The calendar lines of `cautions` by the calendar they are about (not the engine's own).
    #[serde(default)]
    pub calendar_cautions: BTreeMap<String, Vec<String>>,
    /// `calendars/<name>.toml` as the engine read them, by name: what the
    /// window's month table and the next times count by.
    #[serde(default)]
    pub calendars: BTreeMap<String, Calendar>,
    /// This PC's power settings as the engine sees them: what marks the next times.
    #[serde(default)]
    pub power: Power,
    /// `[[blackout]]` in `koyomi.toml`: the planned stops, in which no time starts a net.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blackouts: Vec<Blackout>,
    /// The definitions staged to take over on a day (`next.toml`), earliest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub staged: Vec<koyomi_core::next::Entry>,
    /// The ids of the nets a warning bears on (approving them is refused), counted by the engine
    /// from the paths it knows: a window on another PC cannot.
    #[serde(default)]
    pub blocked: Vec<String>,
    /// The engine's offset from UTC in minutes: a window in another zone counts today differently.
    #[serde(default)]
    pub utc_offset_min: i32,
    /// The shells this engine has (what a job's `shell` may name), in `koyomi_core::shell::SHELLS`' order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shells: Vec<Shell>,
    /// The shell a new script job starts with (`shell` in `koyomi.toml`, or the OS's own); empty when the
    /// engine is too old to say.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shell: String,
    /// The names of the `[[account]]`s of koyomi.toml (what a job's `run_as` may name); never their user or secret.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<String>,
    /// The `[[database]]`s of koyomi.toml by name and kind (what a sql job's `database` may name).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub databases: Vec<DatabaseName>,
    /// Whether the engine runs on Windows (`run_as` works only there); `None` when the engine is too old to say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows: Option<bool>,
    /// The `[[host]]`s of koyomi.toml by name and kind (what an `ssh` / `winrm` `on` or a `send` / `fetch` `host`
    /// may name); never their address, login or secret.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hosts: Vec<HostName>,
    /// The names of the `[[bucket]]`s (what a `send` / `fetch` `bucket` may name); never their address or login.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buckets: Vec<String>,
    /// The names of the `[[mailbox]]`es (what a `[[when]]`'s `mailbox` may name); never their address or secret.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mailboxes: Vec<String>,
    /// The `[groups]` an `ssh` / `winrm` `on = "group:<name>"` may name, with the kind of their members; a group
    /// whose members cannot be read is left out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<GroupName>,
    /// The names of the `[[notify.mails]]` in file order: the first is the default `via` of a `mail` job.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mails: Vec<String>,
}

/// One shell the engine found on its PC.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shell {
    /// `powershell`, `pwsh`, `bash`, `sh` or `zsh`.
    pub name: String,
    /// The absolute path the engine runs for it.
    pub path: String,
}

/// How this PC sleeps and whether the engine can wake it (`wake = true` nets).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Power {
    /// Seconds the PC idles before it sleeps on the power source it runs on; `None`: never, or not known.
    #[serde(default)]
    pub sleep_after: Option<u64>,
    /// Why the engine cannot wake the PC: `timers_off`, `battery`, `unsupported` or `failed`
    /// (a `power.why_*` key). `None`: it can, or no net asks it to.
    #[serde(default)]
    pub cannot_wake: Option<String>,
    /// `YYYY-MM-DD HH:MM`: when the engine has set the PC to wake, and for which net.
    #[serde(default)]
    pub wake_at: Option<String>,
    #[serde(default)]
    pub wake_for: Option<String>,
}

impl Snapshot {
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Result<Snapshot, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// Every start of an approved, not held net still to come today: the planned times as the quiet
    /// spans leave them (a skipped one is gone, a waiting one moves to the span's end). What the
    /// window's 今日 and the Web UI list as 待ち.
    /// A time in a planned stop (`[[blackout]]`) is not one: [`Snapshot::stopped_today`] lists those.
    pub fn starts_today(&self, now: NaiveDateTime) -> Vec<(&Def, NaiveDateTime)> {
        self.today(now).into_iter().filter(|(_, at)| blackout::find(&self.blackouts, *at).is_none()).collect()
    }

    /// The starts still to come today that a planned stop keeps from starting, with the stop.
    pub fn stopped_today(&self, now: NaiveDateTime) -> Vec<(&Def, NaiveDateTime, &Blackout)> {
        self.today(now).into_iter().filter_map(|(d, at)| blackout::find(&self.blackouts, at).map(|b| (d, at, b))).collect()
    }

    /// Every time still to come today of an approved, not held net, planned stops or not.
    fn today(&self, now: NaiveDateTime) -> Vec<(&Def, NaiveDateTime)> {
        let end = now.date().and_hms_opt(23, 59, 59).unwrap_or(now);
        let mut out = Vec::new();
        for d in self.defs.iter().filter(|d| self.starts(d)) {
            if let Some(net) = &d.net {
                let times = when::between_in(&net.when, now, end, self.calendar_of(d));
                out.extend(quiet::starts(&net.quiet, net.on_quiet, times).into_iter().filter(|at| *at <= end).map(|at| (d, at)));
            }
        }
        out
    }

    /// The next start of `d` after `from`, quiet spans and planned stops seen; none for a net not
    /// approved or on hold.
    pub fn next_start(&self, d: &Def, from: NaiveDateTime) -> Option<NaiveDateTime> {
        let net = d.net.as_ref().filter(|_| self.starts(d))?;
        let cal = self.calendar_of(d);
        blackout::outside(&self.blackouts, from, 1, |after, k| quiet::next_starts(&net.when, &net.quiet, net.on_quiet, after, k, cal))
            .into_iter()
            .next()
    }

    /// Whether a time of `d` starts it: approved and not on hold.
    fn starts(&self, d: &Def) -> bool {
        d.standing == Standing::Confirmed && !self.held.contains(&d.id)
    }

    /// The calendar `d` counts by, as the engine read it.
    fn calendar_of(&self, d: &Def) -> Option<&Calendar> {
        d.calendar.as_deref().and_then(|c| self.calendars.get(c))
    }
}

/// What `Versions` answers: `versions/<group>/<name>/<version>.toml`'s stems, newest first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Versions {
    #[serde(default)]
    pub versions: Vec<String>,
}

impl Versions {
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Result<Versions, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

/// What `Import` answers: what the import would do (or did), line by line.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    #[serde(default)]
    pub lines: Vec<String>,
    /// Things there and different, whose fate the choice decides.
    #[serde(default)]
    pub conflicts: usize,
    #[serde(default)]
    pub problems: usize,
    /// A choice is needed (`conflicts > 0` and no `on_conflict`): nothing was or can be written.
    #[serde(default)]
    pub undecided: bool,
    /// Something would be written (or was, when `applied`).
    #[serde(default)]
    pub writes: bool,
    #[serde(default)]
    pub applied: bool,
}

impl Plan {
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Result<Plan, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

/// A notice the engine keeps for `koyomi tray` to show, numbered from 1.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub seq: u64,
    pub event: String,
    pub title: String,
    pub body: String,
    /// What a click on it opens (`notify::run_of`); empty: it only shows.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag: String,
}

/// One failed run the tray menu lists.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrayFail {
    pub id: String,
    pub label: String,
}

/// What `Tray` answers: the tray's badge, tip and menu as the engine works them out.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrayView {
    /// `error`, `warning`, `running`, `paused` or `done`.
    #[serde(default)]
    pub badge: String,
    #[serde(default)]
    pub tip: String,
    #[serde(default)]
    pub paused: bool,
    /// Job processes running.
    #[serde(default)]
    pub busy: usize,
    /// Runs of today that failed and are not marked 対応済み (未対応を見る).
    #[serde(default)]
    pub errors: usize,
    /// The newest of those, listed in the menu.
    #[serde(default, rename = "fail")]
    pub fails: Vec<TrayFail>,
    /// How many more failed runs there are than `fails` lists.
    #[serde(default)]
    pub more: usize,
    /// The engine has its own tray (it was not started with `--no-tray`):
    /// `koyomi tray` would only be a second icon.
    #[serde(default)]
    pub own: bool,
    #[serde(default, rename = "note")]
    pub notes: Vec<Note>,
    /// The number of the last notice kept (0: none yet).
    #[serde(default)]
    pub last: u64,
}

/// What `Notices` answers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticesView {
    /// Names the engine process (empty: it keeps no notices, it shows them itself).
    #[serde(default)]
    pub boot: String,
    /// The number of the last notice kept (0: none yet).
    #[serde(default)]
    pub last: u64,
    #[serde(default, rename = "note")]
    pub notes: Vec<Note>,
}

impl NoticesView {
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Result<NoticesView, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

impl TrayView {
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Result<TrayView, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

/// What `Runs` answers: oldest first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Runs {
    #[serde(default, rename = "run")]
    pub runs: Vec<Run>,
}

impl Runs {
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Result<Runs, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request as the door sends it (postcard), read back.
    fn round(request: &Request) -> Request {
        let mut wire = Vec::new();
        ito_ipc::frame::write(&mut wire, &(PROTO, request)).unwrap();
        let (proto, back): (u32, Request) = ito_ipc::frame::read(&mut wire.as_slice()).unwrap();
        assert_eq!(proto, PROTO);
        back
    }

    fn peek(kind: PeekKind, host: &str, port: u16) -> Request {
        Request::HostPeek { kind, host: host.into(), port, ticket: 7 }
    }

    #[test]
    fn a_host_peek_reads_back_through_the_door_and_json_with_its_kind_in_lowercase() {
        for kind in [PeekKind::Ssh, PeekKind::Winrm] {
            let Request::HostPeek { kind: got, host, port, ticket } = round(&peek(kind, "h.example", 22)) else { panic!("not a peek") };
            assert_eq!((got, host.as_str(), port, ticket), (kind, "h.example", 22, 7));
        }
        assert_eq!(serde_json::to_string(&PeekKind::Ssh).unwrap(), "\"ssh\"");
        assert_eq!(serde_json::to_string(&PeekKind::Winrm).unwrap(), "\"winrm\"");
        let json = serde_json::to_string(&peek(PeekKind::Winrm, "h", 5986)).unwrap();
        assert!(
            matches!(serde_json::from_str::<Request>(&json), Ok(Request::HostPeek { kind: PeekKind::Winrm, port: 5986, .. })),
            "{json}"
        );
    }

    #[test]
    fn the_door_waits_longer_for_a_host_peek_bare_or_over_and_not_for_another_request() {
        let over = |request: Request| Request::Over { by: "x".into(), request: Box::new(request) };
        let door = door();
        assert_eq!((door.wait)(&peek(PeekKind::Ssh, "h", 22)), PEEK_ANSWER_WAIT);
        assert_eq!((door.wait)(&over(peek(PeekKind::Winrm, "h", 5986))), PEEK_ANSWER_WAIT);
        assert_eq!((door.wait)(&db_test(None)), PEEK_ANSWER_WAIT);
        assert_eq!((door.wait)(&over(db_test(None))), PEEK_ANSWER_WAIT);
        assert_eq!((door.wait)(&Request::Status), ito_ipc::door::ANSWER_WAIT);
        assert_eq!((door.wait)(&over(Request::Status)), ito_ipc::door::ANSWER_WAIT);
        assert!(PEEK_CALL_WAIT > PEEK_ANSWER_WAIT);
    }

    fn db_test(password: Option<&str>) -> Request {
        Request::DatabaseTest {
            db: "postgres://u@h/d".into(),
            tls: Some(koyomi_core::chore::SqlTls::Trust),
            entry: Some("main".into()),
            password_value: password.map(|p| Hidden::new(p.into())),
            ticket: 9,
        }
    }

    /// a database test reads back through the door and JSON, a client without the optional keys still reads, and its
    /// `Debug` never shows the password.
    #[test]
    fn a_database_test_reads_back_and_hides_its_password() {
        let ask = db_test(Some("pw-not-to-show"));
        assert!(!format!("{ask:?}").contains("pw-not-to-show") && format!("{ask:?}").contains("***"));
        let Request::DatabaseTest { db, tls, entry, password_value, ticket } = round(&ask) else { panic!("not a test") };
        assert_eq!(
            (db.as_str(), tls, entry.as_deref(), ticket),
            ("postgres://u@h/d", Some(koyomi_core::chore::SqlTls::Trust), Some("main"), 9)
        );
        assert_eq!(password_value.as_ref().map(Hidden::expose), Some("pw-not-to-show"));
        let json = serde_json::to_string(&ask).unwrap();
        let Ok(Request::DatabaseTest { password_value, .. }) = serde_json::from_str::<Request>(&json) else { panic!("json") };
        assert_eq!(password_value.as_ref().map(Hidden::expose), Some("pw-not-to-show"));
        let bare = r#"{"DatabaseTest":{"db":"sqlite:///x.db","ticket":1}}"#;
        assert!(matches!(
            serde_json::from_str::<Request>(bare),
            Ok(Request::DatabaseTest { tls: None, entry: None, password_value: None, ticket: 1, .. })
        ));
    }

    #[test]
    fn the_peek_rule_takes_names_and_ipv4_and_refuses_the_rest() {
        for ok in ["h", "web-1.example.co.jp", "192.168.0.10", "a_b", "Host.Example"] {
            assert!(peek_target_ok(ok, 22), "{ok}");
        }
        let long = "a".repeat(253);
        assert!(peek_target_ok(&long, 22));
        let too_long = "a".repeat(254);
        for bad in ["", "::1", "[::1]", "user@h", "h:22", "h/x", "h x", " h", "ホスト", too_long.as_str()] {
            assert!(!peek_target_ok(bad, 22), "{bad:?}");
        }
        assert!(!peek_target_ok("h", 0));
        assert!(peek_target_ok("h", 65535));
    }

    #[test]
    fn a_run_reads_back_with_and_without_params() {
        for params in [BTreeMap::new(), BTreeMap::from([("month".to_owned(), "2026-09".to_owned())])] {
            let sent = Request::Run { id: "g/a".into(), date: None, from: Some("b".into()), dry: true, params: params.clone() };
            let Request::Run { id, date, from, dry, params: got } = round(&sent) else { panic!("not a run") };
            assert_eq!((id.as_str(), date, from.as_deref(), dry, got), ("g/a", None, Some("b"), true, params));
        }
    }

    #[test]
    fn a_notices_view_reads_back() {
        let Request::Notices { boot, since } = round(&Request::Notices { boot: Some("1-2".into()), since: Some(4) }) else {
            panic!("not notices")
        };
        assert_eq!((boot.as_deref(), since), (Some("1-2"), Some(4)));
        let view = NoticesView {
            boot: "1-2".into(),
            last: 9,
            notes: vec![Note {
                seq: 9,
                event: "failed".into(),
                title: "t".into(),
                body: "b
c"
                .into(),
                tag: "r1.l".into(),
            }],
        };
        assert_eq!(NoticesView::parse(&view.to_toml()).unwrap(), view);
        assert_eq!(NoticesView::parse("").unwrap(), NoticesView::default());
    }

    #[test]
    fn a_tray_view_reads_back() {
        let Request::Tray { since } = round(&Request::Tray { since: Some(7) }) else { panic!("not a tray") };
        assert_eq!(since, Some(7));
        let view = TrayView {
            badge: "error".into(),
            tip: "koyomi: 異常 1".into(),
            busy: 2,
            errors: 2,
            fails: vec![TrayFail { id: "r1".into(), label: "✕ a/b 09:00".into() }],
            more: 1,
            notes: vec![
                Note { seq: 8, event: "failed".into(), title: "t".into(), body: "b\nc".into(), tag: String::new() },
                Note { seq: 9, event: "failed".into(), title: "u".into(), body: "d".into(), tag: "20261030-090000-01.l".into() },
            ],
            last: 8,
            ..TrayView::default()
        };
        assert_eq!(TrayView::parse(&view.to_toml()).unwrap(), view);
        assert_eq!(TrayView::parse("").unwrap(), TrayView::default());
        // A note without a tag (an engine before PROTO 35) reads as one that only shows.
        let old = TrayView::parse("last = 1\n[[note]]\nseq = 1\nevent = \"failed\"\ntitle = \"t\"\nbody = \"b\"\n").unwrap();
        assert_eq!(old.notes[0].tag, "");
    }

    fn id() -> String {
        "g/a".to_owned()
    }

    fn save(force: bool) -> Request {
        Request::Save { id: id(), text: "x".into(), base_sha256: None, force }
    }

    fn stage(expect: Option<StageExpect>) -> Request {
        Request::Stage { id: id(), text: "x".into(), from: "2099-12-01".into(), expect }
    }

    fn expect() -> Option<StageExpect> {
        Some(StageExpect { live: "a".into(), staged: None })
    }

    /// every request with the role `POST /door` asks for it: the table of the design, line by line.
    #[test]
    fn the_door_table_places_every_request() {
        use Role::{Admin, Edit, Run, View};
        let run = || Request::Run { id: id(), date: None, from: None, dry: false, params: BTreeMap::new() };
        let table: Vec<(Request, Option<Role>)> = vec![
            (Request::Status, Some(View)),
            (Request::Runs, Some(View)),
            (Request::History { from: "2026-11-01".into(), to: "2026-11-02".into() }, Some(View)),
            (Request::Log { run: "r".into(), job: "j".into() }, Some(View)),
            (Request::Ops { from: "2026-11-01".into(), to: "2026-11-02".into() }, Some(View)),
            (Request::Handover { release: false }, Some(View)),
            (Request::Handover { release: true }, None),
            (run(), Some(Run)),
            (Request::Rerun { run: "r".into(), from: None, next: None }, Some(Run)),
            (Request::Kill { run: "r".into(), job: None }, Some(Run)),
            (Request::Skip { run: "r".into(), job: "j".into() }, Some(Run)),
            (Request::Answer { run: "r".into(), job: "j".into(), yes: true }, Some(Run)),
            (Request::Hold { id: id(), job: None }, Some(Run)),
            (Request::Release { id: id(), job: None }, Some(Run)),
            (Request::SkipNext { id: id(), cancel: false }, Some(Run)),
            (Request::Shift { id: id(), from: "2026-11-01 09:00".into(), to: None }, Some(Run)),
            (Request::List, Some(Edit)),
            (Request::RunDefinition { run: "r".into() }, Some(Edit)),
            (Request::Show { id: id() }, Some(Edit)),
            (Request::Versions { id: id() }, Some(Edit)),
            (Request::VersionText { id: id(), version: "current".into() }, Some(Edit)),
            (Request::Uses { target: "g/a".into() }, Some(Edit)),
            (Request::CalendarText { name: "c".into() }, Some(Edit)),
            (Request::Propose { id: id(), text: "x".into(), base_sha256: None }, Some(Edit)),
            (Request::Copy { id: id(), to: "g/b".into(), title: None }, Some(Edit)),
            (Request::Restore { id: id(), version: "v".into() }, Some(Edit)),
            (Request::Replace { from: "a".into(), to: "b".into(), apply: true }, Some(Edit)),
            (save(false), Some(Admin)),
            (save(true), None),
            (Request::Approve { id: id(), sha256: "s".into() }, Some(Admin)),
            (stage(expect()), Some(Admin)),
            (stage(None), None),
            (Request::Unstage { id: id() }, Some(Admin)),
            (Request::Import { text: String::new(), dry_run: false, on_conflict: None }, Some(Admin)),
            (Request::Rename { id: id(), to: "g/b".into(), fix: false }, Some(Admin)),
            (Request::Delete { id: id() }, Some(Admin)),
            (Request::Pause, Some(Admin)),
            (Request::Resume, Some(Admin)),
            (Request::CalendarPut { name: "c".into(), text: String::new(), base_sha256: None }, Some(Admin)),
            (Request::CalendarRename { from: "c".into(), to: "d".into() }, Some(Admin)),
            (Request::CalendarDelete { name: "c".into(), base_sha256: "s".into() }, Some(Admin)),
            (Request::HolidayTable { bytes: vec![1] }, Some(Admin)),
            (Request::ExportAll, Some(Admin)),
            (Request::Users, Some(Admin)),
            (Request::ConfigText, Some(Admin)),
            (Request::ConfigPut { text: String::new(), base_sha256: "b".into() }, Some(Admin)),
            (Request::UserSet { name: "u".into(), role: "run".into(), nets: None, roles: None, base: "b".into() }, Some(Admin)),
            (Request::UserRemove { name: "u".into(), base: "b".into() }, Some(Admin)),
            (Request::UserAdd { name: "u".into(), role: "run".into(), nets: None, roles: None, base: "b".into() }, Some(Admin)),
            (Request::UserReset { name: "u".into(), base: "b".into() }, Some(Admin)),
            (Request::Stop, None),
            (Request::Tray { since: None }, None),
            (Request::Notices { boot: None, since: None }, Some(Edit)),
            (Request::Standby { give_back: false }, None),
            (Request::Standby { give_back: true }, None),
            (peek(PeekKind::Ssh, "h.example", 22), Some(Admin)),
            (peek(PeekKind::Winrm, "h.example", 5986), Some(Admin)),
            (db_test(None), Some(Admin)),
            (Request::Over { by: "x".into(), request: Box::new(Request::Status) }, None),
        ];
        for (r, want) in &table {
            assert_eq!(door_needs(r), *want, "{r:?}");
        }
    }

    /// what leaves the engine as it was is a read; everything else may not be asked twice on its own.
    #[test]
    fn only_what_changes_nothing_is_a_read() {
        let reads = [
            Request::Status,
            Request::Runs,
            Request::History { from: "a".into(), to: "b".into() },
            Request::Log { run: "r".into(), job: "j".into() },
            Request::Ops { from: "a".into(), to: "b".into() },
            Request::Handover { release: false },
            Request::List,
            Request::RunDefinition { run: "r".into() },
            Request::Show { id: id() },
            Request::Versions { id: id() },
            Request::VersionText { id: id(), version: "v".into() },
            Request::Uses { target: "t".into() },
            Request::CalendarText { name: "c".into() },
            Request::ExportAll,
            Request::Users,
            Request::ConfigText,
            Request::Tray { since: None },
            Request::Notices { boot: Some("b".into()), since: Some(3) },
            peek(PeekKind::Ssh, "h.example", 22),
        ];
        for r in &reads {
            assert!(r.is_read() && is_read(r), "{r:?}");
        }
        let writes = [
            Request::Handover { release: true },
            Request::Run { id: id(), date: None, from: None, dry: true, params: BTreeMap::new() },
            Request::Stop,
            Request::Pause,
            save(false),
            save(true),
            stage(expect()),
            stage(None),
            Request::Approve { id: id(), sha256: "s".into() },
            Request::CalendarPut { name: "c".into(), text: String::new(), base_sha256: None },
            Request::HolidayTable { bytes: Vec::new() },
            Request::ConfigPut { text: String::new(), base_sha256: "b".into() },
            Request::Propose { id: id(), text: "x".into(), base_sha256: None },
            Request::UserSet { name: "u".into(), role: "run".into(), nets: None, roles: None, base: "b".into() },
            Request::UserRemove { name: "u".into(), base: "b".into() },
            Request::UserAdd { name: "u".into(), role: "run".into(), nets: None, roles: None, base: "b".into() },
            Request::UserReset { name: "u".into(), base: "b".into() },
            Request::AccountPut { name: "a".into(), user: "u".into(), password: "p".into(), base_sha256: "b".into() },
            Request::AccountRemove { name: "a".into(), base_sha256: "b".into(), force: false },
            Request::DatabasePut {
                name: "d".into(),
                db: "sqlite://x".into(),
                password: None,
                tls: None,
                base_sha256: "b".into(),
                password_value: None,
            },
            Request::DatabaseRemove { name: "d".into(), base_sha256: "b".into(), force: false },
            Request::HostPut {
                name: "h".into(),
                on: "h.example".into(),
                host_key: Some("SHA256:x".into()),
                key: None,
                password: None,
                cert: None,
                auth: Some("ntlm".into()),
                base_sha256: "b".into(),
            },
            Request::HostRemove { name: "h".into(), base_sha256: "b".into(), force: false },
            Request::BucketPut {
                name: "bk".into(),
                url: "s3://x".into(),
                key_id: None,
                password: "secret:s".into(),
                region: None,
                endpoint: None,
                base_sha256: "b".into(),
            },
            Request::BucketRemove { name: "bk".into(), base_sha256: "b".into(), force: false },
            Request::MailboxPut { name: "mb".into(), mail: "imaps://u@h".into(), password: "secret:s".into(), base_sha256: "b".into() },
            Request::MailboxRemove { name: "mb".into(), base_sha256: "b".into(), force: true },
            db_test(None),
        ];
        for r in &writes {
            assert!(!r.is_read(), "{r:?}");
            if matches!(
                r,
                Request::HostPut { .. }
                    | Request::HostRemove { .. }
                    | Request::BucketPut { .. }
                    | Request::BucketRemove { .. }
                    | Request::MailboxPut { .. }
                    | Request::MailboxRemove { .. }
            ) {
                assert_eq!(door_needs(r), Some(Role::Admin), "{r:?}");
            }
            // Each one reads back through JSON as what it was.
            assert_eq!(format!("{:?}", round(r)), format!("{r:?}"));
        }
    }

    /// a typed password never shows in a request's `Debug`, and still travels through the door and JSON.
    #[test]
    fn a_typed_password_is_hidden_from_debug_and_travels_through_the_door_and_json() {
        let put = Request::DatabasePut {
            name: "d".into(),
            db: "postgres://u@h/d".into(),
            password: None,
            tls: None,
            base_sha256: "b".into(),
            password_value: Some(Hidden::new("pw-not-to-show".into())),
        };
        assert!(!format!("{put:?}").contains("pw-not-to-show"));
        assert!(format!("{put:?}").contains("***"));
        let Request::DatabasePut { password_value, .. } = round(&put) else { panic!("not a put") };
        assert_eq!(password_value.as_ref().map(Hidden::expose), Some("pw-not-to-show"));
        let json = serde_json::to_string(&put).unwrap();
        let Ok(Request::DatabasePut { password_value, .. }) = serde_json::from_str::<Request>(&json) else { panic!("json") };
        assert_eq!(password_value.as_ref().map(Hidden::expose), Some("pw-not-to-show"));
        // A put written without the value (a client before PROTO 52) reads as one with none.
        let old = r#"{"DatabasePut":{"name":"d","db":"sqlite://x","base_sha256":"b"}}"#;
        assert!(matches!(serde_json::from_str::<Request>(old), Ok(Request::DatabasePut { password_value: None, .. })));
    }

    /// the new requests and the staged one's `expect` travel through the door and through JSON.
    #[test]
    fn the_new_requests_read_back() {
        let want = StageExpect { live: "l".into(), staged: Some("s".into()) };
        let Request::Stage { expect: got, .. } = round(&stage(Some(want.clone()))) else { panic!("not a stage") };
        assert_eq!(got, Some(want));
        let put = Request::CalendarPut { name: "c".into(), text: "t".into(), base_sha256: Some("b".into()) };
        let Request::CalendarPut { name, base_sha256, .. } = round(&put) else { panic!("not a put") };
        assert_eq!((name.as_str(), base_sha256.as_deref()), ("c", Some("b")));
        let set = Request::UserSet {
            name: "u".into(),
            role: "run".into(),
            nets: Some(vec!["経理/*".into()]),
            roles: Some(BTreeMap::from([("基盤/*".to_owned(), "view".to_owned())])),
            base: "b".into(),
        };
        let Request::UserSet { name, nets, roles, base, .. } = round(&set) else { panic!("not a user set") };
        assert_eq!((name.as_str(), base.as_str()), ("u", "b"));
        assert_eq!(nets, Some(vec!["経理/*".to_owned()]));
        assert_eq!(roles.and_then(|r| r.get("基盤/*").cloned()).as_deref(), Some("view"));
        let add = Request::UserAdd { name: "u".into(), role: "view".into(), nets: Some(vec!["a/*".into()]), roles: None, base: "b".into() };
        let Request::UserAdd { name, nets, base, .. } = round(&add) else { panic!("not a user add") };
        assert_eq!((name.as_str(), base.as_str(), nets), ("u", "b", Some(vec!["a/*".to_owned()])));
        let Request::UserReset { name, base } = round(&Request::UserReset { name: "u".into(), base: "b".into() }) else {
            panic!("not a user reset")
        };
        assert_eq!((name.as_str(), base.as_str()), ("u", "b"));
        assert!(matches!(round(&Request::Users), Request::Users));
        let json = serde_json::to_string(&Request::HolidayTable { bytes: vec![1, 2] }).unwrap();
        assert!(matches!(serde_json::from_str::<Request>(&json), Ok(Request::HolidayTable { bytes }) if bytes == [1, 2]));
        // A stage written without `expect` (a client before PROTO 45) reads as one with none.
        let old: Request = serde_json::from_str(r#"{"Stage":{"id":"g/a","text":"x","from":"2099-12-01"}}"#).unwrap();
        assert!(matches!(old, Request::Stage { expect: None, .. }));
        let body = CalendarBody { text: "a = 1\n".into(), sha256: "abc".into() };
        assert_eq!(CalendarBody::parse(&body.to_toml()).unwrap(), body);
        let put = Request::ConfigPut { text: "t".into(), base_sha256: "b".into() };
        let Request::ConfigPut { text, base_sha256 } = round(&put) else { panic!("not a config put") };
        assert_eq!((text.as_str(), base_sha256.as_str()), ("t", "b"));
        assert!(matches!(round(&Request::ConfigText), Request::ConfigText));
        let cfg = ConfigBody {
            text: "[http]
port = 1
"
            .into(),
            sha256: "abc".into(),
        };
        assert_eq!(ConfigBody::parse(&cfg.to_toml()).unwrap(), cfg);
        assert!(matches!(round(&Request::Registry), Request::Registry));
        let put = Request::AccountPut { name: "a".into(), user: "u".into(), password: "p".into(), base_sha256: "b".into() };
        let Request::AccountPut { name, user, .. } = round(&put) else { panic!("not an account put") };
        assert_eq!((name.as_str(), user.as_str()), ("a", "u"));
        assert!(Request::Registry.is_read());
        assert_eq!(door_needs(&Request::Registry), Some(Role::Admin));
        assert_eq!(door_needs(&put), Some(Role::Admin));
    }

    /// The new rows and the names-only snapshot lists read back, and an older snapshot or body without them still reads.
    #[test]
    fn the_connections_read_back_and_an_old_text_reads_without_them() {
        let body = RegistryBody {
            sha256: "x".into(),
            hosts: vec![HostRow { name: "h".into(), ..HostRow::default() }],
            buckets: vec![BucketRow { name: "bk".into(), ..BucketRow::default() }],
            mailboxes: vec![MailboxRow { name: "mb".into(), ..MailboxRow::default() }],
            ..RegistryBody::default()
        };
        assert_eq!(RegistryBody::parse(&body.to_toml()).unwrap(), body);
        let snap = Snapshot {
            hosts: vec![HostName { name: "h".into(), kind: "winrm".into() }],
            buckets: vec!["bk".into()],
            mailboxes: vec!["mb".into()],
            groups: vec![GroupName { name: "web".into(), kind: "ssh".into() }],
            mails: vec!["m".into()],
            ..Snapshot::default()
        };
        assert_eq!(Snapshot::parse(&snap.to_toml()).unwrap(), snap);
        let old = Snapshot::parse(
            "defs = []
warnings = []
",
        )
        .unwrap();
        assert!(
            old.hosts.is_empty() && old.buckets.is_empty() && old.mailboxes.is_empty() && old.groups.is_empty() && old.mails.is_empty()
        );
    }

    /// The registry body reads back with every field and state, and an older body without the lists reads as empty.
    #[test]
    fn the_registry_body_round_trips_with_every_state() {
        let body = RegistryBody {
            sha256: "abc".into(),
            accounts: ["ok", "missing", "unreadable"]
                .iter()
                .map(|s| AccountRow {
                    name: format!("a-{s}"),
                    user: "CORP-batch".into(),
                    secret: "s".into(),
                    state: (*s).into(),
                    used_by: vec!["g/n".into()],
                })
                .collect(),
            databases: ["ok", "missing", "unreadable", "not-needed"]
                .iter()
                .map(|s| DatabaseRow {
                    name: format!("d-{s}"),
                    kind: "sqlite".into(),
                    db: "sqlite://x".into(),
                    secret: (*s != "not-needed").then(|| "pw".to_owned()),
                    tls: (*s == "ok").then(|| "verify".to_owned()),
                    state: (*s).into(),
                    used_by: Vec::new(),
                })
                .collect(),
            ..RegistryBody::default()
        };
        assert_eq!(RegistryBody::parse(&body.to_toml()).unwrap(), body);
        assert_eq!(RegistryBody::parse("sha256 = \"x\"\n").unwrap(), RegistryBody { sha256: "x".into(), ..Default::default() });
    }

    /// the snapshot reads back with the engine's blocked nets and offset, and without them (an older engine).
    #[test]
    fn the_snapshot_holds_what_a_remote_window_cannot_count() {
        let snap = Snapshot { blocked: vec!["g/a".into()], utc_offset_min: 540, ..Snapshot::default() };
        assert_eq!(Snapshot::parse(&snap.to_toml()).unwrap(), snap);
        let old = Snapshot::parse("defs = []\nwarnings = []\n").unwrap();
        assert_eq!((old.blocked.len(), old.utc_offset_min), (0, 0));
    }

    /// The shells and the default shell read back; an older engine's text has neither and still reads.
    #[test]
    fn the_snapshot_holds_the_shells_and_reads_without_them() {
        let snap = Snapshot {
            shells: vec![Shell { name: "bash".into(), path: "/bin/bash".into() }, Shell { name: "sh".into(), path: "/bin/sh".into() }],
            shell: "bash".into(),
            ..Snapshot::default()
        };
        assert_eq!(Snapshot::parse(&snap.to_toml()).unwrap(), snap);
        let old = Snapshot::parse(
            "defs = []
warnings = []
",
        )
        .unwrap();
        assert_eq!((old.shells.len(), old.shell.as_str()), (0, ""));
        // Nothing found, nothing written.
        assert!(!Snapshot::default().to_toml().contains("shell"));
    }
}
