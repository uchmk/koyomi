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
use koyomi_core::run::{Run, State};
use koyomi_core::scan::{Def, Standing};
use koyomi_core::{quiet, when};
use serde::{Deserialize, Serialize};

/// Bumped when `Request` or what a reply holds changes shape.
pub const PROTO: u32 = 40;

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
    Stage {
        id: String,
        text: String,
        from: String,
    },
    /// Drop what was staged for `id`: the net runs as it is.
    Unstage {
        id: String,
    },
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
    /// One line each about the calendars: a file that does not read, the
    /// holiday table's age. Nothing is stopped by these.
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
    /// Nets whose last run of today failed (未対応を見る).
    #[serde(default)]
    pub errors: usize,
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
}
