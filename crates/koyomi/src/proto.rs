//! What the window and the CLI say to the engine through its door, and what the engine says back.
//!
//! One request and one reply per connection (`ito_ipc::door`). The reply is
//! text: TOML for `List`, `Show`, `Runs`, `History` and `Versions`, the log's tail for `Log`, a line for the rest.

use std::collections::BTreeMap;

use ito_ipc::door::Door;
use koyomi_core::calendar::Calendar;
use koyomi_core::run::Run;
use koyomi_core::scan::Def;
use serde::{Deserialize, Serialize};

/// Bumped when `Request` or what a reply holds changes shape.
pub const PROTO: u32 = 15;

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
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, String>,
    },
    /// Start `run` again with the definition it had: what passed is carried
    /// over, the rest runs (from the job `from`, when there is one). The reply is the new run's id.
    Rerun {
        run: String,
        from: Option<String>,
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
    Hold {
        id: String,
    },
    /// Take the net `id` off hold.
    Release {
        id: String,
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
    /// Stop the whole schedule: every time is written down as ⏸ 保留 until `Resume`. Runs by hand still go.
    Pause,
    Resume,
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
}

pub fn door() -> Door<Request> {
    Door::new("koyomi", ENV, PROTO)
}

/// The variable that names another address for the window's door.
pub const WINDOW_ENV: &str = "KOYOMI_WINDOW_ADDRESS";

/// What the tray asks the window (DECISIONS C1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Show {
    /// Come forward.
    Front,
    /// Come forward on today's runs (ガント).
    Today,
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
    /// `koyomi pause`: no time starts anything.
    #[serde(default)]
    pub paused: bool,
    /// One line each about the calendars: a file that does not read, the
    /// holiday table's age. Nothing is stopped by these.
    #[serde(default)]
    pub cautions: Vec<String>,
    /// `calendars/<name>.toml` as the engine read them, by name: what the
    /// window's month table and the next times count by.
    #[serde(default)]
    pub calendars: BTreeMap<String, Calendar>,
    /// This PC's power settings as the engine sees them: what marks the next times.
    #[serde(default)]
    pub power: Power,
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
