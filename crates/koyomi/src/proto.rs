//! What the window and the CLI say to the engine through its door, and what the engine says back.
//!
//! One request and one reply per connection (`ito_ipc::door`). The reply is
//! text: TOML for `List`, `Show`, `Runs`, `History`, `Versions` and `Tray`, the log's tail for `Log`, a line for the rest.

use std::collections::BTreeMap;

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
pub const PROTO: u32 = 45;

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
        | Request::Show { .. }
        | Request::Versions { .. }
        | Request::VersionText { .. }
        | Request::Uses { .. }
        | Request::CalendarText { .. }
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
        | Request::ExportAll => Some(Role::Admin),
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
        | Request::Show { .. }
        | Request::Versions { .. }
        | Request::VersionText { .. }
        | Request::Uses { .. }
        | Request::CalendarText { .. }
        | Request::ExportAll
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
        | Request::HolidayTable { .. } => false,
    }
}

pub fn door() -> Door<Request> {
    Door::new("koyomi", ENV, PROTO)
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

    #[test]
    fn a_run_reads_back_with_and_without_params() {
        for params in [BTreeMap::new(), BTreeMap::from([("month".to_owned(), "2026-09".to_owned())])] {
            let sent = Request::Run { id: "g/a".into(), date: None, from: Some("b".into()), dry: true, params: params.clone() };
            let Request::Run { id, date, from, dry, params: got } = round(&sent) else { panic!("not a run") };
            assert_eq!((id.as_str(), date, from.as_deref(), dry, got), ("g/a", None, Some("b"), true, params));
        }
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
            (Request::Stop, None),
            (Request::Tray { since: None }, None),
            (Request::Standby { give_back: false }, None),
            (Request::Standby { give_back: true }, None),
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
            Request::Show { id: id() },
            Request::Versions { id: id() },
            Request::VersionText { id: id(), version: "v".into() },
            Request::Uses { target: "t".into() },
            Request::CalendarText { name: "c".into() },
            Request::ExportAll,
            Request::Tray { since: None },
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
            Request::Propose { id: id(), text: "x".into(), base_sha256: None },
        ];
        for r in &writes {
            assert!(!r.is_read(), "{r:?}");
        }
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
        let json = serde_json::to_string(&Request::HolidayTable { bytes: vec![1, 2] }).unwrap();
        assert!(matches!(serde_json::from_str::<Request>(&json), Ok(Request::HolidayTable { bytes }) if bytes == [1, 2]));
        // A stage written without `expect` (a client before PROTO 45) reads as one with none.
        let old: Request = serde_json::from_str(r#"{"Stage":{"id":"g/a","text":"x","from":"2099-12-01"}}"#).unwrap();
        assert!(matches!(old, Request::Stage { expect: None, .. }));
        let body = CalendarBody { text: "a = 1\n".into(), sha256: "abc".into() };
        assert_eq!(CalendarBody::parse(&body.to_toml()).unwrap(), body);
    }

    /// the snapshot reads back with the engine's blocked nets and offset, and without them (an older engine).
    #[test]
    fn the_snapshot_holds_what_a_remote_window_cannot_count() {
        let snap = Snapshot { blocked: vec!["g/a".into()], utc_offset_min: 540, ..Snapshot::default() };
        assert_eq!(Snapshot::parse(&snap.to_toml()).unwrap(), snap);
        let old = Snapshot::parse("defs = []\nwarnings = []\n").unwrap();
        assert_eq!((old.blocked.len(), old.utc_offset_min), (0, 0));
    }
}
