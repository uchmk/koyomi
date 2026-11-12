//! One job net's TOML file: what it may say in this version, and where it is wrong.
//!
//! Every message names a line, so `koyomi check` and the tree can say
//! "月次締め.toml の 12 行目: 知らない欄 rnu". A field the design has but this
//! version does not run is an error too (never ignored): a `timeout` that
//! was written and not kept is how a night job hangs.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};
use toml::Spanned;

use crate::chore::{Chore, Pass, SqlTls, Verify, WinrmAuth, GROUP_MAX};
use crate::late::Deadline;
use crate::msg;
use crate::quiet::{OnQuiet, Span};
use crate::when::{self, AfterOn, Kind, MonthRule, OnClosed, Session, When};

/// The `format` this version reads.
pub const FORMAT: i64 = 1;

/// A mistake in a file, at a line (1-based; 0 when it is not in the text).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub line: usize,
    pub message: String,
}

impl Problem {
    /// `月次締め.toml の 12 行目: 知らない欄 rnu`. A message with an ID keeps it at the head, the place goes last.
    pub fn in_file(&self, file: &str) -> String {
        let place = match self.line {
            0 => file.to_owned(),
            n => msg::note(msg::N_FILE_AT_LINE, &[file, &n.to_string()]),
        };
        if msg::is_coded(&self.message) {
            msg::note(msg::N_PROBLEM_CODED_AT, &[&self.message, &place])
        } else {
            msg::note(msg::N_PROBLEM_AT, &[&place, &self.message])
        }
    }
}

/// A job net that reads.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Jobnet {
    /// `name`, when the file has one.
    pub name: Option<String>,
    pub jobs: Vec<Job>,
    /// The times it starts on its own; none: by hand only.
    #[serde(default)]
    pub when: Vec<When>,
    /// What to do with a time that passed while the PC or the engine was off.
    #[serde(default)]
    pub missed: Missed,
    /// What to do with a time that comes while the net's last run is not over.
    #[serde(default)]
    pub overlap: Overlap,
    /// `calendar = "会社"`: `calendars/会社.toml`. None: the group's, or no calendar.
    #[serde(default)]
    pub calendar: Option<String>,
    /// `lock = "販売DB"`: nets with the same lock name never run at the same time
    /// (or as many as koyomi.toml's `[locks]` says; see [`crate::lock`]).
    #[serde(default)]
    pub lock: Option<String>,
    /// `lock = { name = "販売DB", shared = true }`: the net only reads, and goes with the other
    /// shared holders of the lock, never with an exclusive one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lock_shared: bool,
    /// `wake = true`: the engine wakes a sleeping PC a little before the net's next time.
    #[serde(default)]
    pub wake: bool,
    /// `[params]`: a value's name and its default, put in for `{param:name}`. A run by hand
    /// may give another value (`koyomi run --set name=value`); a scheduled one uses the default.
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    /// `late_start = "02:10"` or `"10m"`: told when a run has not begun by then.
    #[serde(default)]
    pub late_start: Option<Deadline>,
    /// `late_end = "06:00"` or `"4h"`: told when a run is not over by then.
    #[serde(default)]
    pub late_end: Option<Deadline>,
    /// `late_slow = false`: not told when a run takes twice its usual time (told when not written).
    #[serde(default)]
    pub late_slow: Option<bool>,
    /// `priority = 1..5`: which net's job gets a free slot first when the slots are short
    /// (5 first; 3 when not written).
    #[serde(default)]
    pub priority: Option<u8>,
    /// `require = ["ac", "network", "idle:10m"]`: what the PC must be in before a job starts.
    #[serde(default)]
    pub require: Require,
    /// `quiet = ["12:00-13:00"]`: hours a time of `[[when]]` does not start in.
    #[serde(default)]
    pub quiet: Vec<Span>,
    /// `on_quiet = "skip"`: such a time is skipped, not started when the hours are over.
    #[serde(default)]
    pub on_quiet: OnQuiet,
    /// `notify_to = ["accounting"]`: the notification places (by `name`) the
    /// net's notices go to; empty: the places with no name.
    #[serde(default)]
    pub notify_to: Vec<String>,
    /// `takeover = "rerun"`: when the standby takes over (`[standby]`), a run of this net the
    /// primary was running starts again there from the jobs that had not passed.
    #[serde(default)]
    pub takeover: Takeover,
}

/// `takeover`: what the standby does with a run of the net the primary was running when it took over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Takeover {
    /// Written down as not taken over; a person decides.
    #[default]
    Leave,
    /// Started again on the standby from the jobs that had not passed (it may run twice
    /// when the primary was only cut off, not down).
    Rerun,
}

/// `require`: the conditions of the PC a job waits for (the Task Scheduler's 条件 tab).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Require {
    /// `"ac"`: only on AC power.
    #[serde(default)]
    pub ac: bool,
    /// `"network"`: only while the network is up.
    #[serde(default)]
    pub network: bool,
    /// `"idle:10m"`: only after the PC has been left alone this many seconds.
    #[serde(default)]
    pub idle: Option<u64>,
}

impl Require {
    pub fn is_empty(&self) -> bool {
        *self == Require::default()
    }

    /// The words that write it back: `["ac", "network", "idle:10m"]`.
    pub fn words(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.ac {
            out.push("ac".to_owned());
        }
        if self.network {
            out.push("network".to_owned());
        }
        if let Some(s) = self.idle {
            out.push(if s % 60 == 0 { format!("idle:{}m", s / 60) } else { format!("idle:{s}s") });
        }
        out
    }

    fn parse_idle(s: &str) -> Option<u64> {
        let (n, mul) = match (s.strip_suffix('m'), s.strip_suffix('s')) {
            (Some(n), _) => (n, 60),
            (_, Some(n)) => (n, 1),
            _ => return None,
        };
        n.parse::<u64>().ok().filter(|n| *n > 0).and_then(|n| n.checked_mul(mul))
    }
}

/// What `priority` is when it is not written.
pub const DEFAULT_PRIORITY: u8 = 3;

/// The most passes of a loop part that run side by side (`max`).
pub const LOOP_MAX: u32 = 16;

impl Jobnet {
    /// The priority for a free slot: `priority`, or [`DEFAULT_PRIORITY`].
    pub fn slot_priority(&self) -> u8 {
        self.priority.unwrap_or(DEFAULT_PRIORITY)
    }

    /// Whether a run that takes twice its usual time is told of (`late_slow`, on unless false).
    pub fn watches_slow(&self) -> bool {
        self.late_slow != Some(false)
    }

    /// Whether a `[[when]]` counts business days or says `on_closed`, so the net must have a calendar.
    pub fn needs_calendar(&self) -> bool {
        self.when.iter().any(|w| w.on_closed.is_some() || matches!(&w.kind, Kind::Month { rule, .. } if rule.needs_calendar()))
    }
}

/// `missed`: a time of the schedule that passed while nothing could start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Missed {
    /// Recorded as 時刻を過ぎた; nothing runs.
    #[default]
    Skip,
    /// The last passed time runs once when the engine starts, marked 遅れて実行.
    Once,
}

/// `overlap`: a time of the schedule that comes while the last run still goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Overlap {
    /// Recorded as スキップ.
    #[default]
    Skip,
    /// Runs after the last run ends; one time waits at most.
    Queue,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub name: Option<String>,
    /// A sticky note drawn under the part on the editor's canvas; the engine does not read it.
    pub note: Option<String>,
    /// The executable first, then one argument per element. Never a shell
    /// line. Empty when the job has `shell` and `script` instead (or the
    /// job is a part that runs nothing).
    pub run: Vec<String>,
    pub dir: Option<String>,
    /// The lines that lead here: `after = ["a"]`, or `{ job = "a", on = "error" }`.
    pub after: Vec<Line>,
    pub pos: Option<(f64, f64)>,
    /// What the job is: a job that runs, or one of the parts (`kind = "branch"` …).
    #[serde(default, skip_serializing_if = "Part::is_job")]
    pub part: Part,
    /// Seconds the job may run before it is stopped, from `timeout = "30m"`.
    #[serde(default)]
    pub timeout: Option<u64>,
    /// Run it again after an abnormal end, from `retry = { count = 2, wait = "5m" }`.
    #[serde(default)]
    pub retry: Option<Retry>,
    /// `shell = "powershell"`: the engine runs `script` with that shell instead of `run`.
    #[serde(default)]
    pub shell: Option<String>,
    /// The shell's script, as written (one argument to the shell, never joined with anything).
    #[serde(default)]
    pub script: Option<String>,
    /// What each exit code means, from `judge = { warn = "1", error = "2.." }`.
    /// Without it, 0 is 正常 and every other code 異常.
    #[serde(default)]
    pub judge: Option<Judge>,
    /// Environment variables for the process, from `env = { A = "x", DB_PASS = "secret:db" }`.
    /// A value `secret:<name>` is read from the OS credential store when the job starts.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// How the process's output becomes text, from `encoding = "sjis"`.
    #[serde(default, skip_serializing_if = "Encoding::is_auto")]
    pub encoding: Encoding,
    /// `to = ["ops"]` of a `kind = "notify"` part: the places (by `name`) it
    /// tells, instead of the net's `notify_to`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<String>,
    /// The account the process runs as, from `run_as = { user = …, password = "secret:…" }`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_as: Option<RunAs>,
}

/// `encoding`: how a job's output is read into the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    /// UTF-8 for a line that is valid UTF-8, the console code page (Shift_JIS on
    /// Japanese Windows) for one that is not.
    #[default]
    Auto,
    /// Shift_JIS (CP932), whatever the system.
    Sjis,
    /// UTF-8, bad bytes replaced.
    Utf8,
}

impl Encoding {
    pub fn is_auto(&self) -> bool {
        *self == Encoding::Auto
    }

    /// As it is written in the file.
    pub fn text(self) -> &'static str {
        match self {
            Encoding::Auto => "auto",
            Encoding::Sjis => "sjis",
            Encoding::Utf8 => "utf8",
        }
    }

    pub fn of(text: &str) -> Option<Encoding> {
        [Encoding::Auto, Encoding::Sjis, Encoding::Utf8].into_iter().find(|e| e.text() == text)
    }
}

/// A line from the job `job` to the one that has it in `after`: which
/// result of `job` lets this one go on. Written as the id alone (`on = "ok"`),
/// or as a table: `{ job = "a", on = "error" }`, `{ job = "a", code = "1..3" }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub job: String,
    pub on: On,
}

impl Line {
    /// The plain line: goes on when `job` passed.
    pub fn ok(job: &str) -> Line {
        Line { job: job.to_owned(), on: On::Ok }
    }

    /// As it is written: `a`, `a (error)`, `a (any)`, `a (code 1..3)`, `a (else)`.
    pub fn text(&self) -> String {
        match &self.on {
            On::Ok => self.job.clone(),
            On::Error => format!("{} (error)", self.job),
            On::Any => format!("{} (any)", self.job),
            On::Code(r) => format!("{} (code {})", self.job, Judge::list(r)),
            On::Else => format!("{} (else)", self.job),
        }
    }
}

/// What result of the line's job lets it through.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum On {
    /// 正常 or 警告 (or skipped): the plain line.
    #[default]
    Ok,
    /// 異常 (an error, a timeout, a kill).
    Error,
    /// Ended, whichever way.
    Any,
    /// From a `kind = "branch"` part: its exit code is in these ranges.
    Code(Vec<Range>),
    /// From a `kind = "branch"` part: no `code` line from it matched.
    Else,
}

/// The TOML shape of a [`Line`]: the id alone, or `{ job, on }` / `{ job, code }`.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum LineRepr {
    Id(String),
    Full {
        job: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
}

impl Serialize for Line {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let repr = match &self.on {
            On::Ok => LineRepr::Id(self.job.clone()),
            On::Error => LineRepr::Full { job: self.job.clone(), on: Some("error".to_owned()), code: None },
            On::Any => LineRepr::Full { job: self.job.clone(), on: Some("any".to_owned()), code: None },
            On::Code(r) => LineRepr::Full { job: self.job.clone(), on: None, code: Some(Judge::list(r)) },
            On::Else => LineRepr::Full { job: self.job.clone(), on: None, code: Some("else".to_owned()) },
        };
        repr.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Line {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Line, D::Error> {
        match LineRepr::deserialize(d)? {
            LineRepr::Id(job) => Ok(Line::ok(&job)),
            LineRepr::Full { job, on, code } => line_of_parts(job, on.as_deref(), code.as_deref()).map_err(serde::de::Error::custom),
        }
    }
}

/// `{ job, on }` or `{ job, code }` as a line; the message says what is wrong.
fn line_of_parts(job: String, on: Option<&str>, code: Option<&str>) -> Result<Line, String> {
    let on = match (on, code) {
        (Some(_), Some(_)) => return Err(msg::line(msg::DEF_LINE_BOTH, &[])),
        (None, None) | (Some("ok"), None) => On::Ok,
        (Some("error"), None) => On::Error,
        (Some("any"), None) => On::Any,
        (Some(_), None) => return Err(msg::line(msg::DEF_CHOICE_BAD, &["after.on", "\"ok\", \"error\", \"any\""])),
        (None, Some("else")) => On::Else,
        (None, Some(c)) => On::Code(ranges(c).map_err(|e| msg::line(msg::DEF_VALUE_UNREADABLE, &["after.code", msg::uncoded(&e)]))?),
    };
    Ok(Line { job, on })
}

/// How a `kind = "join"` part waits for its lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Wait {
    /// Every line (the plain rule).
    #[default]
    All,
    /// The first line that is met.
    Any,
}

/// What a `[[job]]` is: a job that runs a command, or a part of the flow that runs nothing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Part {
    /// Runs `run` (or `shell` + `script`).
    #[default]
    Job,
    /// 繰り返し: runs `run` (or `shell` + `script`) once per file matching `file`, or `count`
    /// times; each pass gets `KOYOMI_FILE` and `KOYOMI_INDEX`. Up to `max` passes run side by
    /// side (one after another when it is not written).
    Loop {
        #[serde(default)]
        file: Option<String>,
        #[serde(default)]
        count: Option<u32>,
        #[serde(default)]
        max: Option<u32>,
    },
    /// 分岐: ends at once with its one predecessor's exit code; `code` lines choose the way.
    Branch,
    /// 待ち合わせ: ends when its lines are met, all of them or any one.
    Join { wait: Wait },
    /// 時刻待ち: ends at `at` on the run's day plus `days` (at once when that passed).
    Time {
        at: NaiveTime,
        #[serde(default)]
        days: u32,
    },
    /// ファイル待ち: ends when a file matching `file` is there and unchanged for `settle` seconds.
    File {
        file: String,
        #[serde(default)]
        settle: Option<u64>,
    },
    /// 通知: sends the run's notification as it stands, then ends.
    Notify,
    /// 確認待ち: waits for a person to approve or reject (screen, or `koyomi approve-step`).
    /// `message` is what the person decides on, shown with the question.
    Approval {
        #[serde(default)]
        message: Option<String>,
    },
    /// 子ジョブネット: `use = "グループ/名前"`'s jobs run here, in its place.
    Jobnet {
        #[serde(rename = "use")]
        use_: String,
    },
    /// よくある作業: deletes, zips, copies or empties files with no script written (`kind = "delete"` …).
    Chore(Chore),
}

impl Part {
    pub fn is_job(&self) -> bool {
        matches!(self, Part::Job)
    }

    /// Runs a process: a job, or a loop of them. It takes a slot and has `run`.
    pub fn runs_process(&self) -> bool {
        matches!(self, Part::Job | Part::Loop { .. } | Part::Chore(_))
    }

    /// `job`, `branch` …: the `kind` as written, and the key of its word.
    pub fn key(&self) -> &'static str {
        match self {
            Part::Job => "job",
            Part::Loop { .. } => "loop",
            Part::Branch => "branch",
            Part::Join { .. } => "join",
            Part::Time { .. } => "time",
            Part::File { .. } => "file",
            Part::Notify => "notify",
            Part::Approval { .. } => "approval",
            Part::Jobnet { .. } => "jobnet",
            Part::Chore(c) => c.key(),
        }
    }

    /// Ends in the run itself as soon as its lines are met: no process, no clock.
    pub fn is_instant(&self) -> bool {
        matches!(self, Part::Branch | Part::Join { .. } | Part::Jobnet { .. })
    }

    /// Waits for the clock or a file: the engine polls it.
    pub fn waits(&self) -> bool {
        matches!(self, Part::Time { .. } | Part::File { .. } | Part::Approval { .. })
    }
}

/// Which exit codes end a job as 警告 and which as 異常. A code in neither
/// list is 正常 only when it is 0; `error` wins where both lists have a code.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Judge {
    #[serde(default)]
    pub warn: Vec<Range>,
    #[serde(default)]
    pub error: Vec<Range>,
    /// A regex (smart-case): a line of the job's output that matches it makes the end 異常.
    #[serde(default)]
    pub error_output: Option<String>,
    /// A file (`{date}` allowed): when it exists after the job ended, the end is 正常 whatever the code.
    #[serde(default)]
    pub ok_file: Option<String>,
}

impl Judge {
    /// The ranges as they are written: `1,3..5,10..`.
    pub fn list(ranges: &[Range]) -> String {
        ranges.iter().map(Range::text).collect::<Vec<_>>().join(",")
    }
}

/// Exit codes from `from` to `to`, both inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub from: i32,
    pub to: i32,
}

impl Range {
    pub fn has(&self, code: i32) -> bool {
        (self.from..=self.to).contains(&code)
    }

    pub fn text(&self) -> String {
        match (self.from, self.to) {
            (a, b) if a == b => a.to_string(),
            (i32::MIN, i32::MAX) => "..".to_owned(),
            (i32::MIN, b) => format!("..{b}"),
            (a, i32::MAX) => format!("{a}.."),
            (a, b) => format!("{a}..{b}"),
        }
    }
}

/// `"1"`, `"2.."`, `"..-1"`, `"1..3"`, or several with commas: `"1,3..5"`.
pub fn ranges(text: &str) -> Result<Vec<Range>, String> {
    if text.trim().is_empty() {
        return Err(msg::line(msg::DEF_RANGE_EMPTY, &[]));
    }
    let code = |s: &str| s.trim().parse::<i32>().map_err(|_| msg::line(msg::DEF_RANGE_CODE_BAD, &[s.trim()]));
    let mut out = Vec::new();
    for part in text.split(',') {
        let part = part.trim();
        let (from, to) = match part.split_once("..") {
            None => {
                let n = code(part)?;
                (n, n)
            }
            Some((a, b)) => {
                let from = if a.trim().is_empty() { i32::MIN } else { code(a)? };
                let to = if b.trim().is_empty() { i32::MAX } else { code(b)? };
                if from > to {
                    return Err(msg::line(msg::DEF_RANGE_REVERSED, &[part]));
                }
                (from, to)
            }
        };
        out.push(Range { from, to });
    }
    Ok(out)
}

/// How often a job is run again after it ends abnormally, and how long to
/// wait before each time. A 打ち切り or 強制終了 is never run again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retry {
    /// Times it is run again, at most.
    pub count: u32,
    /// Seconds between the abnormal end and the next time.
    pub wait: u64,
}

/// `run_as = { user = 'CORP\batch', password = "secret:batch" }`: the account a
/// job runs as instead of the engine's. The password is never in the file:
/// `password` names the OS credential store's secret that holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunAs {
    /// `DOMAIN\user`, `user@domain`, or a local `user`.
    pub user: String,
    /// The secret's name (what follows `secret:`).
    pub secret: String,
}

impl RunAs {
    /// The domain and the user as Windows' logon takes them: `None` for a local
    /// account (`batch`, `.\batch`) or a `user@domain` one.
    pub fn split(&self) -> (Option<&str>, &str) {
        match self.user.split_once('\\') {
            Some((d, u)) if d != "." => (Some(d), u),
            Some((_, u)) => (None, u),
            None => (None, self.user.as_str()),
        }
    }
}

/// A Windows account as `run_as` takes it: `DOMAIN\user`, `user@domain` or
/// `user`, with nothing a logon would read otherwise (no space, quote or
/// control character).
pub fn valid_account(s: &str) -> bool {
    let (domain, user) = s.split_once('\\').unwrap_or((".", s));
    let bad = |t: &str| t.chars().any(|c| c.is_whitespace() || c.is_control() || "\"/[]:;|=,+*?<>".contains(c));
    !user.is_empty() && !domain.is_empty() && !user.contains('\\') && !bad(user) && !bad(domain)
}

type Field = Option<Spanned<toml::Value>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNet {
    format: Field,
    name: Field,
    #[serde(default)]
    job: Vec<Spanned<RawJob>>,
    #[serde(default)]
    when: Vec<Spanned<RawWhen>>,
    /// Only `"skip"` (the default) in this version.
    missed: Field,
    overlap: Field,
    calendar: Field,
    lock: Field,
    wake: Field,
    params: Field,
    late_start: Field,
    late_end: Field,
    late_slow: Field,
    priority: Field,
    require: Field,
    quiet: Field,
    on_quiet: Field,
    notify_to: Field,
    takeover: Field,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWhen {
    once: Field,
    every: Field,
    at: Field,
    n: Field,
    from: Field,
    to: Field,
    days: Field,
    day: Field,
    nth: Field,
    weekday: Field,
    date: Field,
    dates: Field,
    except: Field,
    start: Field,
    end: Field,
    on_closed: Field,
    business_day: Field,
    shift: Field,
    file: Field,
    settle: Field,
    log: Field,
    contains: Field,
    eventlog: Field,
    ids: Field,
    source: Field,
    mail: Field,
    password: Field,
    sender: Field,
    subject: Field,
    on_start: Field,
    after_jobnet: Field,
    idle: Field,
    session: Field,
    user: Field,
    on: Field,
    wait_for: Field,
    wait_until: Field,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJob {
    id: Field,
    name: Field,
    note: Field,
    run: Field,
    dir: Field,
    after: Field,
    pos: Field,
    kind: Field,
    #[serde(rename = "use")]
    use_: Field,
    wait: Field,
    at: Field,
    days: Field,
    file: Field,
    settle: Field,
    count: Field,
    max: Field,
    message: Field,
    to: Field,
    path: Field,
    #[serde(rename = "match")]
    pattern: Field,
    keep_days: Field,
    from: Field,
    into: Field,
    key: Field,
    host_key: Field,
    key_id: Field,
    region: Field,
    endpoint: Field,
    verify: Field,
    cert: Field,
    auth: Field,
    pass: Field,
    password: Field,
    on: Field,
    command: Field,
    url: Field,
    method: Field,
    body: Field,
    content_type: Field,
    headers: Field,
    expect: Field,
    cc: Field,
    subject: Field,
    files: Field,
    via: Field,
    db: Field,
    sql: Field,
    out: Field,
    rows: Field,
    tls: Field,
    shell: Field,
    script: Field,
    timeout: Field,
    retry: Field,
    judge: Field,
    env: Field,
    encoding: Field,
    run_as: Field,
}

/// The 1-based line of byte `offset` in `text`.
pub fn line_of(text: &str, offset: usize) -> usize {
    let end = offset.min(text.len());
    text.as_bytes()[..end].iter().filter(|&&b| b == b'\n').count() + 1
}

/// Reads `text`; every problem found, in line order, when it does not.
pub fn parse(text: &str) -> Result<Jobnet, Vec<Problem>> {
    let raw: RawNet = match toml::from_str(text) {
        Ok(r) => r,
        Err(e) => {
            let span = e.span();
            let line = span.as_ref().map_or(0, |s| line_of(text, s.start));
            let at = span.and_then(|s| text.get(s));
            return Err(vec![Problem { line, message: explain(e.message(), at) }]);
        }
    };
    let mut c = Checker { text, problems: Vec::new(), params: BTreeMap::new() };
    let (missed, overlap) = c.top(&raw);
    let calendar = c.calendar(&raw.calendar);
    let (lock, lock_shared) = c.lock(&raw.lock).unzip();
    let lock_shared = lock_shared.unwrap_or(false);
    let wake = c.wake(&raw.wake);
    let late_start = c.deadline(&raw.late_start, "late_start");
    let late_end = c.deadline(&raw.late_end, "late_end");
    let late_slow = c.late_slow(&raw.late_slow);
    let priority = c.priority(&raw.priority);
    let require = c.require(&raw.require);
    let quiet = c.quiet(&raw.quiet);
    let on_quiet = match c.choice(&raw.on_quiet, "on_quiet", &["wait", "skip"]) {
        Some("skip") => OnQuiet::Skip,
        _ => OnQuiet::Wait,
    };
    let notify_to = c.names(&raw.notify_to, "notify_to");
    let takeover = match c.choice(&raw.takeover, "takeover", &["leave", "rerun"]) {
        Some("rerun") => Takeover::Rerun,
        _ => Takeover::Leave,
    };
    c.params = c.params(&raw.params);
    let jobs = c.jobs(&raw.job);
    let when = c.whens(&raw.when);
    if c.problems.is_empty() {
        let params = std::mem::take(&mut c.params);
        Ok(Jobnet {
            name: c.string(&raw.name, "name"),
            jobs,
            when,
            missed,
            overlap,
            calendar,
            lock,
            lock_shared,
            wake,
            params,
            late_start,
            late_end,
            late_slow,
            priority,
            require,
            quiet,
            on_quiet,
            notify_to,
            takeover,
        })
    } else {
        c.problems.sort_by_key(|p| p.line);
        Err(c.problems)
    }
}

/// The reader's words for the toml crate's.
fn explain(message: &str, at: Option<&str>) -> String {
    if let Some(name) = message.strip_prefix("unknown field `").and_then(|r| r.split('`').next()) {
        return msg::line(msg::DEF_FIELD_UNKNOWN, &[name]);
    }
    if message.starts_with("duplicate key") {
        let place = at.unwrap_or("").trim();
        return msg::line(msg::DEF_KEY_DUPLICATE, &[if place.is_empty() { message } else { place }]);
    }
    if msg::is_coded(message) {
        // an error of ours that serde carried through the toml crate
        return message.to_owned();
    }
    msg::line(msg::DEF_TOML_SYNTAX, &[message])
}

struct Checker<'a> {
    text: &'a str,
    problems: Vec<Problem>,
    /// The net's `[params]`, read before its jobs so `{param:…}` can be checked against them.
    params: BTreeMap<String, String>,
}

/// A name `env` and `[params]` take: letters, digits and `_`, not starting with a digit.
pub(crate) fn plain_name(k: &str) -> bool {
    !k.is_empty() && !k.starts_with(|c: char| c.is_ascii_digit()) && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl Checker<'_> {
    /// Whether the `{date…}` / `{month…}` of `text` read, its `{param:…}` are in `[params]` and
    /// its `{var:…}` have names (koyomi.toml's `[vars]` is told by `koyomi check` and the run).
    fn template(&self, text: &str) -> Result<(), String> {
        crate::template::check(text)?;
        crate::template::check_vars(text)?;
        match crate::template::param_names(text).into_iter().find(|n| !self.params.contains_key(*n)) {
            Some(n) => Err(msg::line(msg::PARAM_MISSING, &[n])),
            None => Ok(()),
        }
    }

    /// `[params]`: names and their default values.
    fn params(&mut self, field: &Field) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        let Some(f) = field.as_ref() else { return out };
        let at = f.span().start;
        let Some(t) = f.get_ref().as_table() else {
            self.add(at, msg::line(msg::DEF_TABLE_BAD, &["params", "{ month = \"2026-09\" }"]));
            return out;
        };
        for (k, v) in t {
            if !plain_name(k) {
                self.add(at, msg::line(msg::DEF_NAME_BAD, &["params", k]));
            }
            let Some(s) = v.as_str() else {
                self.add(at, msg::line(msg::DEF_VALUE_STRING_BAD, &["params", k]));
                continue;
            };
            if let Err(e) = crate::template::check(s).and_then(|()| crate::template::check_vars(s)) {
                self.add(at, msg::line(msg::DEF_TEMPLATE_BAD, &[&format!("params.{k}"), msg::uncoded(&e)]));
            }
            out.insert(k.clone(), s.to_owned());
        }
        out
    }

    fn add(&mut self, offset: usize, message: impl Into<String>) {
        let line = line_of(self.text, offset);
        self.problems.push(Problem { line, message: message.into() });
    }

    /// `max` and `pass` of an `ssh` or a `winrm`, for `on = "group:…"`.
    fn fan(&mut self, j: &RawJob) -> (Option<u32>, Pass) {
        let max = j.max.as_ref().and_then(|f| match f.get_ref().as_integer() {
            Some(n) if (1..=GROUP_MAX as i64).contains(&n) => Some(n as u32),
            _ => {
                self.add(f.span().start, msg::line(msg::DEF_INT_RANGE, &["max", "1", &GROUP_MAX.to_string()]));
                None
            }
        });
        let pass = self.choice(&j.pass, "pass", &Pass::ALL.map(Pass::key)).and_then(Pass::of).unwrap_or_default();
        (max, pass)
    }

    /// A field that is one of `choices` when written: `missed = "once"`.
    fn choice<'a>(&mut self, field: &Field, name: &str, choices: &[&'a str]) -> Option<&'a str> {
        let f = field.as_ref()?;
        match f.get_ref().as_str().and_then(|s| choices.iter().copied().find(|c| *c == s)) {
            Some(c) => Some(c),
            None => {
                let list = choices.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", ");
                self.add(f.span().start, msg::line(msg::DEF_CHOICE_BAD, &[name, &list]));
                None
            }
        }
    }

    fn string(&mut self, field: &Field, name: &str) -> Option<String> {
        let f = field.as_ref()?;
        match f.get_ref() {
            toml::Value::String(s) => Some(s.clone()),
            _ => {
                self.add(f.span().start, msg::line(msg::DEF_STRING_BAD, &[name]));
                None
            }
        }
    }

    /// `env = { NAME = "value" }`: names a process can have, values plain text or `secret:<name>`.
    fn env(&mut self, field: &Field) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        let Some(f) = field else { return out };
        let at = f.span().start;
        let toml::Value::Table(t) = f.get_ref() else {
            self.add(at, msg::line(msg::DEF_TABLE_BAD, &["env", "{ DB_PASS = \"secret:db\" }"]));
            return out;
        };
        for (k, v) in t {
            if !plain_name(k) {
                self.add(at, msg::line(msg::DEF_NAME_BAD, &["env", k]));
            } else if k.to_ascii_uppercase().starts_with("KOYOMI_") {
                self.add(at, msg::line(msg::DEF_ENV_RESERVED, &[k]));
            }
            let Some(s) = v.as_str() else {
                self.add(at, msg::line(msg::DEF_VALUE_STRING_BAD, &["env", k]));
                continue;
            };
            match s.strip_prefix("secret:") {
                Some("") => self.add(at, msg::line(msg::DEF_SECRET_NAME_EMPTY, &[&format!("env.{k}")])),
                Some(_) => {}
                None => {
                    if let Err(e) = self.template(s) {
                        self.add(at, msg::line(msg::DEF_TEMPLATE_BAD, &[&format!("env.{k}"), msg::uncoded(&e)]));
                    }
                }
            }
            out.insert(k.clone(), s.to_owned());
        }
        out
    }

    /// `run_as = { user = 'CORP\batch', password = "secret:batch" }`: both are
    /// required, and the password only as a secret's name.
    fn run_as(&mut self, field: &Field) -> Option<RunAs> {
        const SHAPE: &str = "{ user = 'CORP\\batch', password = \"secret:batch\" }";
        let f = field.as_ref()?;
        let at = f.span().start;
        let toml::Value::Table(t) = f.get_ref() else {
            self.add(at, msg::line(msg::DEF_TABLE_BAD, &["run_as", SHAPE]));
            return None;
        };
        let mut ok = true;
        for k in t.keys() {
            if k != "user" && k != "password" {
                self.add(at, msg::line(msg::DEF_SUBKEY_UNKNOWN, &["run_as", k, "user, password"]));
                ok = false;
            }
        }
        let user = match t.get("user").map(|v| v.as_str()) {
            None => {
                self.add(at, msg::line(msg::DEF_SUBKEY_MISSING, &["run_as", "user", SHAPE]));
                None
            }
            Some(Some(u)) if valid_account(u) => Some(u.to_owned()),
            Some(_) => {
                self.add(at, msg::line(msg::DEF_RUN_AS_USER_BAD, &[]));
                None
            }
        };
        let secret = match t.get("password").map(|v| v.as_str()) {
            None => {
                self.add(at, msg::line(msg::DEF_SUBKEY_MISSING, &["run_as", "password", SHAPE]));
                None
            }
            Some(Some(p)) => match p.strip_prefix("secret:") {
                Some(name) if !name.trim().is_empty() => Some(name.to_owned()),
                Some(_) => {
                    self.add(at, msg::line(msg::DEF_SECRET_NAME_EMPTY, &["run_as.password"]));
                    None
                }
                None => {
                    self.add(at, msg::line(msg::DEF_PASSWORD_NOT_SECRET, &["run_as.password"]));
                    None
                }
            },
            Some(None) => {
                self.add(at, msg::line(msg::DEF_STRING_BAD, &["run_as.password"]));
                None
            }
        };
        match (ok, user, secret) {
            (true, Some(user), Some(secret)) => Some(RunAs { user, secret }),
            _ => None,
        }
    }

    fn timeout(&mut self, field: &Field) -> Option<u64> {
        self.duration_of(field, "timeout")
    }

    /// A duration written as `"30m"`.
    fn duration_of(&mut self, field: &Field, name: &str) -> Option<u64> {
        let f = field.as_ref()?;
        let text = match f.get_ref() {
            toml::Value::String(s) => s.clone(),
            _ => {
                self.add(f.span().start, msg::line(msg::DEF_STRING_BAD_E, &[name, "\"30m\""]));
                return None;
            }
        };
        match crate::duration::parse(&text) {
            Ok(secs) => Some(secs),
            Err(e) => {
                self.add(f.span().start, msg::line(msg::DEF_VALUE_UNREADABLE, &[name, msg::uncoded(&e)]));
                None
            }
        }
    }

    /// A field that only some kinds of `[[job]]` may have: `wait は kind = "join" にだけ書く`.
    fn only(&mut self, field: &Field, name: &str, kinds: &str) {
        if let Some(f) = field {
            self.add(f.span().start, msg::line(msg::DEF_ONLY_FOR_KINDS, &[name, kinds]));
        }
    }

    /// `kind` and the fields that go with it, as a [`Part`].
    fn part(&mut self, j: &RawJob, start: usize) -> Part {
        let kind = self
            .choice(
                &j.kind,
                "kind",
                &[
                    "job", "loop", "branch", "join", "time", "file", "notify", "approval", "jobnet", "delete", "zip", "copy", "empty",
                    "send", "fetch", "http", "ssh", "winrm", "mail", "sql",
                ],
            )
            .unwrap_or("job");
        if kind != "delete" && kind != "empty" {
            self.only(&j.path, "path", "\"delete\", \"empty\"");
        }
        if kind != "delete" && kind != "copy" {
            self.only(&j.pattern, "match", "\"delete\", \"copy\"");
        }
        if kind != "delete" {
            self.only(&j.keep_days, "keep_days", "\"delete\"");
        }
        if !["zip", "copy", "send", "fetch"].contains(&kind) {
            self.only(&j.from, "from", "\"zip\", \"copy\", \"send\", \"fetch\"");
            self.only(&j.into, "into", "\"zip\", \"copy\", \"send\", \"fetch\"");
        }
        if !["send", "fetch", "ssh"].contains(&kind) {
            self.only(&j.key, "key", "\"send\", \"fetch\", \"ssh\"");
            self.only(&j.host_key, "host_key", "\"send\", \"fetch\", \"ssh\"");
        }
        if kind != "send" && kind != "fetch" {
            for (field, name) in [(&j.key_id, "key_id"), (&j.region, "region"), (&j.endpoint, "endpoint"), (&j.verify, "verify")] {
                self.only(field, name, "\"send\", \"fetch\"");
            }
        }
        if !["send", "fetch", "ssh", "winrm", "sql"].contains(&kind) {
            self.only(&j.password, "password", "\"send\", \"fetch\", \"ssh\", \"winrm\", \"sql\"");
        }
        if kind != "ssh" && kind != "winrm" {
            self.only(&j.on, "on", "\"ssh\", \"winrm\"");
            self.only(&j.command, "command", "\"ssh\", \"winrm\"");
            self.only(&j.pass, "pass", "\"ssh\", \"winrm\"");
        }
        if kind != "winrm" {
            self.only(&j.cert, "cert", "\"winrm\"");
            self.only(&j.auth, "auth", "\"winrm\"");
        }
        if kind != "http" {
            for (field, name) in
                [(&j.url, "url"), (&j.method, "method"), (&j.content_type, "content_type"), (&j.headers, "headers"), (&j.expect, "expect")]
            {
                self.only(field, name, "\"http\"");
            }
        }
        if kind != "http" && kind != "mail" {
            self.only(&j.body, "body", "\"http\", \"mail\"");
        }
        if kind != "mail" {
            for (field, name) in [(&j.cc, "cc"), (&j.subject, "subject"), (&j.files, "files"), (&j.via, "via")] {
                self.only(field, name, "\"mail\"");
            }
        }
        if kind != "sql" {
            for (field, name) in [(&j.db, "db"), (&j.sql, "sql"), (&j.out, "out"), (&j.rows, "rows"), (&j.tls, "tls")] {
                self.only(field, name, "\"sql\"");
            }
        }
        if kind != "join" {
            self.only(&j.wait, "wait", "\"join\"");
        }
        if kind != "time" {
            self.only(&j.at, "at", "\"time\"");
            self.only(&j.days, "days", "\"time\"");
        }
        if !["file", "loop", "sql"].contains(&kind) {
            self.only(&j.file, "file", "\"file\", \"loop\", \"sql\"");
        }
        if kind != "file" {
            self.only(&j.settle, "settle", "\"file\"");
        }
        if kind != "loop" {
            self.only(&j.count, "count", "\"loop\"");
        }
        if kind != "approval" {
            self.only(&j.message, "message", "\"approval\"");
        }
        if !["loop", "ssh", "winrm"].contains(&kind) {
            self.only(&j.max, "max", "\"loop\", \"ssh\", \"winrm\"");
        }
        if kind != "jobnet" {
            self.only(&j.use_, "use", "\"jobnet\"");
        }
        match kind {
            "loop" => {
                let file = self.string(&j.file, "file").filter(|f| {
                    let ok = !f.trim().is_empty();
                    if !ok {
                        self.add(j.file.as_ref().map_or(start, |f| f.span().start), msg::line(msg::DEF_VALUE_EMPTY, &["file"]));
                    }
                    ok
                });
                let count = j.count.as_ref().and_then(|f| match f.get_ref().as_integer() {
                    Some(n) if (1..=10_000).contains(&n) => Some(n as u32),
                    _ => {
                        self.add(f.span().start, msg::line(msg::DEF_INT_RANGE, &["count", "1", "10000"]));
                        None
                    }
                });
                let max = j.max.as_ref().and_then(|f| match f.get_ref().as_integer() {
                    Some(n) if (1..=i64::from(LOOP_MAX)).contains(&n) => Some(n as u32),
                    _ => {
                        self.add(f.span().start, msg::line(msg::DEF_INT_RANGE, &["max", "1", &LOOP_MAX.to_string()]));
                        None
                    }
                });
                match (&j.file, &j.count) {
                    (Some(_), Some(f)) => self.add(f.span().start, msg::line(msg::DEF_LOOP_BOTH, &[])),
                    (None, None) => {
                        let shape = "file = 'C:\\in\\*.csv' / count = 3";
                        self.add(start, msg::line(msg::DEF_EITHER_REQUIRED, &["kind = \"loop\"", "file", "count", shape]))
                    }
                    _ => {}
                }
                Part::Loop { file, count, max }
            }
            "branch" => Part::Branch,
            "join" => {
                let wait = match self.choice(&j.wait, "wait", &["all", "any"]) {
                    Some("any") => Wait::Any,
                    _ => Wait::All,
                };
                Part::Join { wait }
            }
            "time" => {
                let at = match &j.at {
                    Some(f) => self.time_of(f, "at"),
                    None => {
                        self.add(start, msg::line(msg::DEF_JOB_KEY_REQUIRED, &["time", "at", "at = \"06:00\""]));
                        None
                    }
                };
                let days = match &j.days {
                    None => 0,
                    Some(f) => match f.get_ref().as_integer() {
                        Some(n) if (0..=366).contains(&n) => n as u32,
                        _ => {
                            self.add(f.span().start, msg::line(msg::DEF_INT_RANGE, &["days", "0", "366"]));
                            0
                        }
                    },
                };
                Part::Time { at: at.unwrap_or(NaiveTime::MIN), days }
            }
            "file" => {
                let file = match &j.file {
                    Some(_) => self.string(&j.file, "file").filter(|f| {
                        let ok = !f.trim().is_empty();
                        if !ok {
                            self.add(j.file.as_ref().map_or(start, |f| f.span().start), msg::line(msg::DEF_VALUE_EMPTY, &["file"]));
                        }
                        ok
                    }),
                    None => {
                        self.add(start, msg::line(msg::DEF_JOB_KEY_REQUIRED, &["file", "file", "file = 'C:\\in\\*.csv'"]));
                        None
                    }
                };
                let settle = self.duration_of(&j.settle, "settle");
                Part::File { file: file.unwrap_or_default(), settle }
            }
            "notify" => Part::Notify,
            "approval" => {
                let message = match &j.message {
                    Some(f) => self.string(&j.message, "message").filter(|m| {
                        let ok = !m.trim().is_empty();
                        if !ok {
                            self.add(f.span().start, msg::line(msg::DEF_VALUE_EMPTY, &["message"]));
                        }
                        ok
                    }),
                    None => None,
                };
                Part::Approval { message }
            }
            "jobnet" => {
                let use_ = match &j.use_ {
                    Some(f) => self.string(&j.use_, "use").filter(|u| {
                        let ok = is_id(u);
                        if !ok {
                            self.add(f.span().start, msg::line(msg::DEF_JOBNET_REF_BAD, &["use"]));
                        }
                        ok
                    }),
                    None => {
                        self.add(start, msg::line(msg::DEF_JOB_KEY_REQUIRED, &["jobnet", "use", "use = \"common/backup\""]));
                        None
                    }
                };
                Part::Jobnet { use_: use_.unwrap_or_default() }
            }
            "delete" | "zip" | "copy" | "empty" | "send" | "fetch" | "http" | "ssh" | "winrm" | "mail" | "sql" => {
                Part::Chore(self.chore(j, kind, start))
            }
            _ => Part::Job,
        }
    }

    /// A chore's fields: the paths it needs, `match`, `keep_days`; each path a template.
    fn chore(&mut self, j: &RawJob, kind: &str, start: usize) -> Chore {
        let path_of = |c: &mut Self, field: &Field, name: &str| -> String {
            let Some(f) = field else {
                c.add(start, msg::line(msg::DEF_JOB_KEY_REQUIRED_PLAIN, &[kind, name]));
                return String::new();
            };
            let s = c.string(field, name).unwrap_or_default();
            if s.trim().is_empty() {
                c.add(f.span().start, msg::line(msg::DEF_VALUE_EMPTY, &[name]));
            } else if let Err(e) = c.template(&s) {
                c.add(f.span().start, msg::line(msg::DEF_TEMPLATE_BAD, &[name, msg::uncoded(&e)]));
            }
            s
        };
        let pattern = self.string(&j.pattern, "match").filter(|p| !p.trim().is_empty());
        let chore = match kind {
            "delete" => {
                let path = path_of(self, &j.path, "path");
                let keep_days = match &j.keep_days {
                    None => {
                        self.add(start, msg::line(msg::DEF_JOB_KEY_REQUIRED, &["delete", "keep_days", "keep_days = 30"]));
                        1
                    }
                    Some(f) => match f.get_ref().as_integer() {
                        Some(n) if (1..=36_500).contains(&n) => n as u32,
                        _ => {
                            self.add(f.span().start, msg::line(msg::DEF_KEEP_DAYS_BAD, &[]));
                            1
                        }
                    },
                };
                Chore::Delete { path, pattern, keep_days }
            }
            "zip" => Chore::Zip { from: path_of(self, &j.from, "from"), into: path_of(self, &j.into, "into") },
            "copy" => Chore::Copy { from: path_of(self, &j.from, "from"), into: path_of(self, &j.into, "into"), pattern },
            "send" | "fetch" | "ssh" => {
                let (from, into) = match kind {
                    "ssh" => (self.far_on(j, kind, start), self.far_command(j, kind, start)),
                    _ => (path_of(self, &j.from, "from"), path_of(self, &j.into, "into")),
                };
                let key = self.string(&j.key, "key").filter(|k| !k.trim().is_empty());
                let host_key = self.string(&j.host_key, "host_key").map(|h| h.trim().to_owned()).filter(|h| !h.is_empty());
                for (value, field, name) in [(&key, &j.key, "key"), (&host_key, &j.host_key, "host_key")] {
                    if let (Some(v), Some(f)) = (value, field) {
                        if let Err(e) = self.template(v) {
                            self.add(f.span().start, msg::line(msg::DEF_TEMPLATE_BAD, &[name, msg::uncoded(&e)]));
                        }
                    }
                }
                // A `secret:` name, not a template: nothing in it is filled in.
                let password = self.string(&j.password, "password").map(|p| p.trim().to_owned()).filter(|p| !p.is_empty());
                // As it is or a `secret:` name, as `password`.
                let key_id = self.string(&j.key_id, "key_id").map(|k| k.trim().to_owned()).filter(|k| !k.is_empty());
                let region = self.string(&j.region, "region").map(|r| r.trim().to_owned()).filter(|r| !r.is_empty());
                let endpoint = self.string(&j.endpoint, "endpoint").map(|e| e.trim().to_owned()).filter(|e| !e.is_empty());
                for (value, field, name) in [(&region, &j.region, "region"), (&endpoint, &j.endpoint, "endpoint")] {
                    if let (Some(v), Some(f)) = (value, field) {
                        if let Err(e) = self.template(v) {
                            self.add(f.span().start, msg::line(msg::DEF_TEMPLATE_BAD, &[name, msg::uncoded(&e)]));
                        }
                    }
                }
                let verify = match kind {
                    "ssh" => None,
                    _ => self.choice(&j.verify, "verify", &Verify::ALL.map(Verify::key)).and_then(Verify::of),
                };
                match kind {
                    "send" => Chore::Send { from, into, key, host_key, password, key_id, region, endpoint, verify },
                    "fetch" => Chore::Fetch { from, into, key, host_key, password, key_id, region, endpoint, verify },
                    _ => {
                        let (max, pass) = self.fan(j);
                        Chore::Ssh { on: from, command: into, key, host_key, password, max, pass }
                    }
                }
            }
            "winrm" => {
                let (on, command) = (self.far_on(j, kind, start), self.far_command(j, kind, start));
                let cert = self.string(&j.cert, "cert").map(|c| c.trim().to_owned()).filter(|c| !c.is_empty());
                if let (Some(v), Some(f)) = (&cert, &j.cert) {
                    if let Err(e) = self.template(v) {
                        self.add(f.span().start, msg::line(msg::DEF_TEMPLATE_BAD, &["cert", msg::uncoded(&e)]));
                    }
                }
                let password = self.string(&j.password, "password").map(|p| p.trim().to_owned()).filter(|p| !p.is_empty());
                let auth = self.choice(&j.auth, "auth", &WinrmAuth::ALL.map(WinrmAuth::key)).and_then(WinrmAuth::of).unwrap_or_default();
                let (max, pass) = self.fan(j);
                Chore::Winrm { on, command, cert, password, auth, max, pass }
            }
            "http" => self.http(j, start),
            // Its own fields, checked once they all read.
            "mail" => return self.mail_job(j, start),
            "sql" => return self.sql_job(j, start),
            _ => Chore::Empty { path: path_of(self, &j.path, "path") },
        };
        // A missing field is already told; what is left is a root or a zip's name.
        if chore.paths().iter().all(|p| !p.trim().is_empty()) {
            if let Err(e) = chore.check() {
                self.add(start, e);
            }
        }
        chore
    }

    /// `kind = "ssh"`'s or `"winrm"`'s `on`: the PC, `ssh://[user@]host[:port]` or
    /// `winrm://user@host[:port]` (a template).
    fn far_on(&mut self, j: &RawJob, kind: &str, start: usize) -> String {
        let (example, form) = match kind {
            "winrm" => ("on = \"winrm://ops@server\"", "winrm://user@host[:port]"),
            _ => ("on = \"ssh://user@server\"", "ssh://[user@]host[:port]"),
        };
        let Some(f) = &j.on else {
            self.add(start, msg::line(msg::DEF_JOB_KEY_REQUIRED, &[kind, "on", example]));
            return String::new();
        };
        let s = self.string(&j.on, "on").unwrap_or_default();
        if s.trim().is_empty() {
            self.add(f.span().start, msg::line(msg::DEF_ON_EMPTY, &[form]));
        } else if let Err(e) = self.template(&s) {
            self.add(f.span().start, msg::line(msg::DEF_TEMPLATE_BAD, &["on", msg::uncoded(&e)]));
        }
        s
    }

    /// `kind = "ssh"`'s or `"winrm"`'s `command`: what runs on that PC, as it is
    /// written (a template).
    fn far_command(&mut self, j: &RawJob, kind: &str, start: usize) -> String {
        let Some(f) = &j.command else {
            let example = if kind == "winrm" { "command = \"ipconfig /all\"" } else { "command = \"df -h\"" };
            self.add(start, msg::line(msg::DEF_JOB_KEY_REQUIRED, &[kind, "command", example]));
            return String::new();
        };
        let s = self.string(&j.command, "command").unwrap_or_default();
        if s.trim().is_empty() {
            self.add(f.span().start, msg::line(msg::DEF_VALUE_EMPTY, &["command"]));
        } else if let Err(e) = self.template(&s) {
            self.add(f.span().start, msg::line(msg::DEF_TEMPLATE_BAD, &["command", msg::uncoded(&e)]));
        }
        s
    }

    /// `kind = "mail"`'s fields: the addresses `to` and `cc`, `subject`, `body` and
    /// `files` (each a template), and `via`, the `[[notify.mails]]` place it goes
    /// through. [`Chore::check`] once they all read.
    fn mail_job(&mut self, j: &RawJob, start: usize) -> Chore {
        let before = self.problems.len();
        let to = match &j.to {
            None => {
                self.add(start, msg::line(msg::DEF_JOB_KEY_REQUIRED, &["mail", "to", "to = [\"ops@example.com\"]"]));
                Vec::new()
            }
            Some(_) => self.texts(&j.to, "to", "to = [\"ops@example.com\"]"),
        };
        let cc = self.texts(&j.cc, "cc", "cc = [\"boss@example.com\"]");
        let subject = match &j.subject {
            None => {
                self.add(start, msg::line(msg::DEF_JOB_KEY_REQUIRED, &["mail", "subject", "subject = \"Sales {date}\""]));
                String::new()
            }
            Some(f) => {
                let s = self.string(&j.subject, "subject").unwrap_or_default();
                if s.trim().is_empty() {
                    self.add(f.span().start, msg::line(msg::DEF_VALUE_EMPTY, &["subject"]));
                } else if let Err(e) = self.template(&s) {
                    self.add(f.span().start, msg::line(msg::DEF_TEMPLATE_BAD, &["subject", msg::uncoded(&e)]));
                }
                s
            }
        };
        let body = self.string(&j.body, "body");
        if let (Some(b), Some(f)) = (&body, &j.body) {
            if let Err(e) = self.template(b) {
                self.add(f.span().start, msg::line(msg::DEF_TEMPLATE_BAD, &["body", msg::uncoded(&e)]));
            }
        }
        let files = self.texts(&j.files, "files", "files = ['D:\\out\\sales.csv']");
        let via = self.string(&j.via, "via").map(|v| v.trim().to_owned());
        if let (Some(""), Some(f)) = (via.as_deref(), &j.via) {
            self.add(f.span().start, msg::line(msg::DEF_VALUE_EMPTY, &["via"]));
        }
        let chore = Chore::Mail { to, cc, subject, body, files, via: via.filter(|v| !v.is_empty()) };
        if self.problems.len() == before {
            if let Err(e) = chore.check() {
                self.add(start, e);
            }
        }
        chore
    }

    /// A `sql`'s fields: `db`, `sql` or `file`, `out`, `rows` (a string or a whole
    /// number), `tls`, `password`; then the whole, once they all read.
    fn sql_job(&mut self, j: &RawJob, start: usize) -> Chore {
        let before = self.problems.len();
        let db = match &j.db {
            None => {
                self.add(start, msg::line(msg::DEF_JOB_KEY_REQUIRED, &["sql", "db", "db = \"sqlserver://db01/sales\""]));
                String::new()
            }
            Some(_) => self.sql_text(&j.db, "db").unwrap_or_default(),
        };
        if let Some(f) = &j.db {
            let t = db.trim();
            if !t.is_empty() && !t.starts_with('{') {
                if let Err(e) = crate::chore::sql_db(t) {
                    self.add(f.span().start, e);
                }
            }
        }
        let sql = self.sql_text(&j.sql, "sql");
        let file = self.sql_text(&j.file, "file");
        let out = self.sql_text(&j.out, "out");
        let password = self.string(&j.password, "password").map(|p| p.trim().to_owned()).filter(|p| !p.is_empty());
        let rows = j.rows.as_ref().and_then(|f| {
            let rows = match f.get_ref() {
                toml::Value::String(s) => s.trim().to_owned(),
                toml::Value::Integer(n) if *n >= 0 => n.to_string(),
                _ => {
                    self.add(f.span().start, msg::line(msg::DEF_ROWS_BAD, &[]));
                    return None;
                }
            };
            if let Err(e) = crate::chore::sql_rows(&rows) {
                self.add(f.span().start, e);
            }
            Some(rows)
        });
        let tls = self.choice(&j.tls, "tls", &SqlTls::ALL.map(SqlTls::key)).and_then(SqlTls::of).unwrap_or_default();
        let chore = Chore::Sql { db, password, sql, file, out, rows, tls };
        if self.problems.len() == before {
            if let Err(e) = chore.check() {
                self.add(start, e);
            }
        }
        chore
    }

    /// A text of a `sql` when written: a string, not empty, a template.
    fn sql_text(&mut self, field: &Field, name: &str) -> Option<String> {
        let at = field.as_ref()?.span().start;
        let s = self.string(field, name)?;
        if s.trim().is_empty() {
            self.add(at, msg::line(msg::DEF_VALUE_EMPTY, &[name]));
        } else if let Err(e) = self.template(&s) {
            self.add(at, msg::line(msg::DEF_TEMPLATE_BAD, &[name, msg::uncoded(&e)]));
        }
        Some(s)
    }

    /// An array of texts, none empty, each a template: a `mail`'s `to`, `cc`, `files`.
    fn texts(&mut self, field: &Field, name: &str, example: &str) -> Vec<String> {
        let Some(f) = field.as_ref() else { return Vec::new() };
        let at = f.span().start;
        let shape = msg::line(msg::DEF_ARRAY_BAD, &[name, example]);
        let Some(list) = f.get_ref().as_array() else {
            self.add(at, shape);
            return Vec::new();
        };
        let mut out = Vec::new();
        for v in list {
            match v.as_str().map(str::trim) {
                Some(s) if !s.is_empty() => {
                    if let Err(e) = self.template(s) {
                        self.add(at, msg::line(msg::DEF_TEMPLATE_BAD, &[name, msg::uncoded(&e)]));
                    }
                    out.push(s.to_owned());
                }
                _ => {
                    self.add(at, shape);
                    return Vec::new();
                }
            }
        }
        out
    }

    /// `kind = "http"`'s fields: `url` (a template) and the rest of the call.
    fn http(&mut self, j: &RawJob, start: usize) -> Chore {
        let template = |c: &mut Self, field: &Field, name: &str| -> Option<String> {
            let s = c.string(field, name)?;
            if let Err(e) = c.template(&s) {
                c.add(field.as_ref().map_or(start, |f| f.span().start), msg::line(msg::DEF_TEMPLATE_BAD, &[name, msg::uncoded(&e)]));
            }
            Some(s)
        };
        let url = match &j.url {
            None => {
                self.add(start, msg::line(msg::DEF_JOB_KEY_REQUIRED, &["http", "url", "url = \"https://api.example.com/jobs\""]));
                String::new()
            }
            Some(f) => {
                let s = template(self, &j.url, "url").unwrap_or_default();
                if s.trim().is_empty() {
                    self.add(f.span().start, msg::line(msg::DEF_VALUE_EMPTY, &["url"]));
                }
                s.trim().to_owned()
            }
        };
        let method = self.choice(&j.method, "method", &crate::chore::METHODS).unwrap_or("GET").to_owned();
        let body = template(self, &j.body, "body");
        let content_type = self.string(&j.content_type, "content_type").map(|c| c.trim().to_owned()).filter(|c| !c.is_empty());
        let mut headers = BTreeMap::new();
        if let Some(f) = &j.headers {
            let at = f.span().start;
            match f.get_ref().as_table() {
                None => self.add(at, msg::line(msg::DEF_TABLE_BAD, &["headers", "{ Authorization = \"secret:api\" }"])),
                Some(t) => {
                    for (k, v) in t {
                        if let Err(e) = crate::chore::header_name(k) {
                            self.add(at, e);
                        }
                        match v.as_str() {
                            None => self.add(at, msg::line(msg::DEF_VALUE_STRING_BAD, &["headers", k])),
                            Some(s) if s.strip_prefix("secret:") == Some("") => {
                                self.add(at, msg::line(msg::DEF_SECRET_NAME_EMPTY, &[&format!("headers.{k}")]))
                            }
                            Some(s) => {
                                headers.insert(k.clone(), s.to_owned());
                            }
                        }
                    }
                }
            }
        }
        let mut expect = Vec::new();
        if let Some(f) = &j.expect {
            match f.get_ref().as_array() {
                Some(a) if !a.is_empty() => {
                    for v in a {
                        match v.as_integer() {
                            Some(n) if (100..=599).contains(&n) => expect.push(n as u16),
                            _ => {
                                self.add(f.span().start, msg::line(msg::DEF_EXPECT_BAD, &[]));
                                break;
                            }
                        }
                    }
                }
                _ => self.add(f.span().start, msg::line(msg::DEF_EXPECT_BAD, &[])),
            }
        }
        Chore::Http { url, method, body, content_type, headers, expect }
    }

    /// `retry = { count = 2, wait = "5m" }`: both are required.
    fn retry(&mut self, field: &Field) -> Option<Retry> {
        const SHAPE: &str = "{ count = 2, wait = \"5m\" }";
        let f = field.as_ref()?;
        let at = f.span().start;
        let toml::Value::Table(t) = f.get_ref() else {
            self.add(at, msg::line(msg::DEF_TABLE_BAD, &["retry", SHAPE]));
            return None;
        };
        let mut ok = true;
        for k in t.keys() {
            if k != "count" && k != "wait" {
                self.add(at, msg::line(msg::DEF_SUBKEY_UNKNOWN, &["retry", k, "count, wait"]));
                ok = false;
            }
        }
        let count = match t.get("count") {
            None => {
                self.add(at, msg::line(msg::DEF_SUBKEY_MISSING, &["retry", "count", SHAPE]));
                None
            }
            Some(v) => match v.as_integer() {
                Some(n) if (1..=100).contains(&n) => Some(n as u32),
                _ => {
                    self.add(at, msg::line(msg::DEF_INT_RANGE, &["retry.count", "1", "100"]));
                    None
                }
            },
        };
        let wait = match t.get("wait") {
            None => {
                self.add(at, msg::line(msg::DEF_SUBKEY_MISSING, &["retry", "wait", SHAPE]));
                None
            }
            Some(v) => match v.as_str() {
                None => {
                    self.add(at, msg::line(msg::DEF_STRING_BAD_E, &["retry.wait", "\"5m\""]));
                    None
                }
                Some(s) => match crate::duration::parse(s) {
                    Ok(secs) => Some(secs),
                    Err(e) => {
                        self.add(at, msg::line(msg::DEF_VALUE_UNREADABLE, &["retry.wait", msg::uncoded(&e)]));
                        None
                    }
                },
            },
        };
        match (count, wait) {
            (Some(count), Some(wait)) if ok => Some(Retry { count, wait }),
            _ => None,
        }
    }

    fn top(&mut self, raw: &RawNet) -> (Missed, Overlap) {
        match &raw.format {
            None => self.problems.push(Problem { line: 1, message: msg::line(msg::DEF_FORMAT_MISSING, &[&FORMAT.to_string()]) }),
            Some(f) if f.get_ref().as_integer() != Some(FORMAT) => {
                self.add(f.span().start, msg::line(msg::DEF_FORMAT_UNSUPPORTED, &[&FORMAT.to_string()]));
            }
            Some(_) => {}
        }
        self.string(&raw.name, "name");
        let missed = match self.choice(&raw.missed, "missed", &["skip", "once"]) {
            Some("once") => Missed::Once,
            _ => Missed::Skip,
        };
        let overlap = match self.choice(&raw.overlap, "overlap", &["skip", "queue"]) {
            Some("queue") => Overlap::Queue,
            _ => Overlap::Skip,
        };
        (missed, overlap)
    }

    /// `calendar = "会社"`: a name a `calendars/<name>.toml` can have.
    fn calendar(&mut self, field: &Field) -> Option<String> {
        let f = field.as_ref()?;
        match f.get_ref().as_str() {
            Some(s) if crate::calendar::valid_name(s) => Some(s.to_owned()),
            _ => {
                self.add(f.span().start, msg::line(msg::DEF_CALENDAR_NAME_BAD, &[]));
                None
            }
        }
    }

    /// `lock = "販売DB"` (exclusive) or `lock = { name = "販売DB", shared = true }`: the name
    /// and whether it is shared. The name has the same rules as a calendar's (a file name, no path).
    fn lock(&mut self, field: &Field) -> Option<(String, bool)> {
        const SHAPE: &str = "{ name = \"salesdb\", shared = true }";
        let f = field.as_ref()?;
        let at = f.span().start;
        let t = match f.get_ref() {
            toml::Value::String(s) if crate::calendar::valid_name(s) => return Some((s.to_owned(), false)),
            toml::Value::String(_) => {
                self.add(at, msg::line(msg::DEF_LOCK_NAME_BAD, &[]));
                return None;
            }
            toml::Value::Table(t) => t,
            _ => {
                self.add(at, msg::line(msg::DEF_LOCK_SHAPE, &[]));
                return None;
            }
        };
        let mut ok = true;
        for k in t.keys() {
            if k != "name" && k != "shared" {
                self.add(at, msg::line(msg::DEF_SUBKEY_UNKNOWN, &["lock", k, "name, shared"]));
                ok = false;
            }
        }
        let name = match t.get("name").map(|v| v.as_str()) {
            None => {
                self.add(at, msg::line(msg::DEF_SUBKEY_MISSING, &["lock", "name", SHAPE]));
                None
            }
            Some(Some(s)) if crate::calendar::valid_name(s) => Some(s.to_owned()),
            Some(_) => {
                self.add(at, msg::line(msg::DEF_LOCK_NAME_BAD, &[]));
                None
            }
        };
        let shared = match t.get("shared").map(|v| v.as_bool()) {
            None => false,
            Some(Some(b)) => b,
            Some(None) => {
                self.add(at, msg::line(msg::DEF_LOCK_SHARED_BAD, &[]));
                ok = false;
                false
            }
        };
        if ok {
            name.map(|n| (n, shared))
        } else {
            None
        }
    }

    /// `wake = true`: wake the PC for the net's times.
    fn wake(&mut self, field: &Field) -> bool {
        let Some(f) = field.as_ref() else { return false };
        match f.get_ref().as_bool() {
            Some(b) => b,
            None => {
                self.add(f.span().start, msg::line(msg::DEF_WAKE_BAD, &[]));
                false
            }
        }
    }

    /// `late_start` / `late_end`: a time of day (`"06:00"`) or a length (`"30m"`).
    fn deadline(&mut self, field: &Field, name: &str) -> Option<Deadline> {
        let f = field.as_ref()?;
        let Some(s) = f.get_ref().as_str() else {
            self.add(f.span().start, msg::line(msg::DEF_STRING_BAD_E, &[name, "\"06:00\" / \"30m\""]));
            return None;
        };
        match Deadline::parse(s) {
            Ok(d) => Some(d),
            Err(e) => {
                self.add(f.span().start, msg::line(msg::DEF_VALUE_UNREADABLE, &[name, msg::uncoded(&e)]));
                None
            }
        }
    }

    /// `priority = 1..5`: a whole number from 1 to 5.
    fn priority(&mut self, field: &Field) -> Option<u8> {
        let f = field.as_ref()?;
        match f.get_ref().as_integer() {
            Some(n) if (1..=5).contains(&n) => Some(n as u8),
            _ => {
                self.add(f.span().start, msg::line(msg::DEF_PRIORITY_BAD, &[]));
                None
            }
        }
    }

    /// `require = ["ac", "network", "idle:10m"]`: what the PC must be in to start a job.
    fn require(&mut self, field: &Field) -> Require {
        let mut out = Require::default();
        let Some(f) = field.as_ref() else { return out };
        let shape = msg::line(msg::DEF_REQUIRE_BAD, &[]);
        let Some(list) = f.get_ref().as_array() else {
            self.add(f.span().start, shape);
            return out;
        };
        for v in list {
            match v.as_str().map(str::trim) {
                Some("ac") => out.ac = true,
                Some("network") => out.network = true,
                Some(w) if w.strip_prefix("idle:").and_then(Require::parse_idle).is_some() => {
                    out.idle = w.strip_prefix("idle:").and_then(Require::parse_idle);
                }
                _ => {
                    self.add(f.span().start, shape);
                    return Require::default();
                }
            }
        }
        out
    }

    /// `late_slow = false`: do not tell of a run that takes twice its usual time.
    fn late_slow(&mut self, field: &Field) -> Option<bool> {
        let f = field.as_ref()?;
        let b = f.get_ref().as_bool();
        if b.is_none() {
            self.add(f.span().start, msg::line(msg::DEF_LATE_SLOW_BAD, &[]));
        }
        b
    }

    /// `quiet = ["12:00-13:00", "22:00-02:00"]`: the spans that read.
    fn quiet(&mut self, field: &Field) -> Vec<Span> {
        let Some(f) = field.as_ref() else { return Vec::new() };
        let at = f.span().start;
        let Some(list) = f.get_ref().as_array() else {
            self.add(at, msg::line(msg::DEF_ARRAY_BAD, &["quiet", "quiet = [\"12:00-13:00\"]"]));
            return Vec::new();
        };
        let mut out = Vec::new();
        for v in list {
            let Some(s) = v.as_str() else {
                self.add(at, msg::line(msg::DEF_ARRAY_BAD, &["quiet", "quiet = [\"12:00-13:00\"]"]));
                continue;
            };
            match Span::parse(s) {
                Ok(s) => out.push(s),
                Err(e) => self.add(at, msg::line(msg::DEF_VALUE_UNREADABLE, &["quiet", msg::uncoded(&e)])),
            }
        }
        out
    }

    /// `notify_to = ["accounting"]` (or `to` of a notify part): names of
    /// notification places, each once.
    fn names(&mut self, field: &Field, name: &str) -> Vec<String> {
        let Some(f) = field.as_ref() else { return Vec::new() };
        let at = f.span().start;
        let shape = msg::line(msg::DEF_ARRAY_BAD, &[name, &format!("{name} = [\"accounting\"]")]);
        let Some(list) = f.get_ref().as_array() else {
            self.add(at, shape);
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        for v in list {
            match v.as_str().map(str::trim) {
                Some(s) if !s.is_empty() => {
                    if out.iter().any(|o| o == s) {
                        self.add(at, msg::line(msg::DEF_NAMES_DUPLICATE, &[name, s]));
                    } else {
                        out.push(s.to_owned());
                    }
                }
                _ => {
                    self.add(at, shape.clone());
                    return Vec::new();
                }
            }
        }
        out
    }

    /// The `[[when]]` entries that read; a mistake in one drops that entry.
    fn whens(&mut self, raw: &[Spanned<RawWhen>]) -> Vec<When> {
        let mut out = Vec::new();
        for entry in raw {
            let head = entry.span().start;
            let w = entry.get_ref();
            let kind = if w.once.is_some() {
                "once"
            } else if let Some(f) = &w.every {
                match f.get_ref().as_str() {
                    Some(s @ ("day" | "minutes" | "hours" | "week" | "month" | "year")) => s,
                    _ => {
                        self.add(
                            f.span().start,
                            msg::line(msg::DEF_CHOICE_BAD, &["every", "\"day\", \"minutes\", \"hours\", \"week\", \"month\", \"year\""]),
                        );
                        continue;
                    }
                }
            } else if w.dates.is_some() {
                "dates"
            } else if w.file.is_some() {
                "file"
            } else if w.log.is_some() {
                "log"
            } else if w.eventlog.is_some() {
                "eventlog"
            } else if w.mail.is_some() {
                "mail"
            } else if w.on_start.is_some() {
                "on_start"
            } else if w.after_jobnet.is_some() {
                "after_jobnet"
            } else if w.idle.is_some() {
                "idle"
            } else if w.session.is_some() {
                "session"
            } else {
                self.add(head, msg::line(msg::DEF_WHEN_KIND_MISSING, &[]));
                continue;
            };
            let allowed: &[&str] = match kind {
                "file" => &["file", "settle"],
                "log" => &["log", "contains"],
                "eventlog" => &["eventlog", "ids", "source"],
                "mail" => &["mail", "password", "sender", "subject"],
                "on_start" => &["on_start"],
                "idle" => &["idle"],
                "session" => &["session", "user"],
                "after_jobnet" => &["after_jobnet", "on"],
                "day" => &["every", "at", "wait_for", "wait_until"],
                "minutes" | "hours" => &["every", "n", "from", "to", "days", "wait_for", "wait_until"],
                "week" => &["every", "days", "at", "wait_for", "wait_until"],
                "month" => &["every", "day", "nth", "weekday", "business_day", "shift", "at", "wait_for", "wait_until"],
                "year" => &["every", "date", "at", "wait_for", "wait_until"],
                "once" => &["once", "wait_for", "wait_until"],
                _ => &["dates", "at", "wait_for", "wait_until"],
            };
            for (field, name) in [
                (&w.once, "once"),
                (&w.every, "every"),
                (&w.at, "at"),
                (&w.n, "n"),
                (&w.from, "from"),
                (&w.to, "to"),
                (&w.days, "days"),
                (&w.day, "day"),
                (&w.nth, "nth"),
                (&w.weekday, "weekday"),
                (&w.date, "date"),
                (&w.dates, "dates"),
                (&w.business_day, "business_day"),
                (&w.shift, "shift"),
                (&w.file, "file"),
                (&w.settle, "settle"),
                (&w.log, "log"),
                (&w.contains, "contains"),
                (&w.eventlog, "eventlog"),
                (&w.ids, "ids"),
                (&w.source, "source"),
                (&w.mail, "mail"),
                (&w.password, "password"),
                (&w.sender, "sender"),
                (&w.subject, "subject"),
                (&w.on_start, "on_start"),
                (&w.after_jobnet, "after_jobnet"),
                (&w.idle, "idle"),
                (&w.session, "session"),
                (&w.user, "user"),
                (&w.on, "on"),
                (&w.wait_for, "wait_for"),
                (&w.wait_until, "wait_until"),
            ] {
                if let Some(f) = field {
                    if !allowed.contains(&name) {
                        self.add(f.span().start, msg::line(msg::DEF_WHEN_KEY_UNUSED, &[name, kind]));
                    }
                }
            }
            let on_closed = match self.choice(&w.on_closed, "on_closed", &["prev", "next", "skip", "run"]) {
                Some("prev") => Some(OnClosed::Prev),
                Some("next") => Some(OnClosed::Next),
                Some("skip") => Some(OnClosed::Skip),
                Some("run") => Some(OnClosed::Run),
                _ => None,
            };
            // An event has no day to move to: it runs on its day or not at all.
            if matches!(kind, "file" | "log" | "eventlog" | "mail" | "on_start" | "idle" | "session")
                && matches!(on_closed, Some(OnClosed::Prev | OnClosed::Next))
            {
                let pos = w.on_closed.as_ref().map_or(head, |f| f.span().start);
                let what = match kind {
                    "log" | "eventlog" => "log / eventlog",
                    "mail" => "mail",
                    "idle" => "idle",
                    "session" => "session",
                    _ => "file / on_start",
                };
                self.add(pos, msg::line(msg::DEF_ON_CLOSED_BAD, &[what]));
            }
            if kind == "after_jobnet" && matches!(on_closed, Some(OnClosed::Prev | OnClosed::Next)) {
                let pos = w.on_closed.as_ref().map_or(head, |f| f.span().start);
                self.add(pos, msg::line(msg::DEF_ON_CLOSED_BAD, &["after_jobnet"]));
            }
            let before = self.problems.len();
            let kind = match kind {
                "once" => self.datetime(&w.once).map(|at| Kind::Once { at }),
                "file" => {
                    let file = self.event_file(&w.file);
                    let settle = self.duration_of(&w.settle, "settle");
                    file.map(|file| Kind::File { file, settle })
                }
                "log" => {
                    let log = self.log_file(&w.log);
                    let contains = match (self.string(&w.contains, "contains"), &w.contains) {
                        (Some(c), _) if !c.is_empty() => Some(c),
                        (Some(_), Some(f)) => {
                            self.add(f.span().start, msg::line(msg::DEF_VALUE_EMPTY, &["contains"]));
                            None
                        }
                        (_, None) => {
                            self.add(head, msg::line(msg::DEF_WHEN_KEY_MISSING, &["contains", "contains = \"ERROR\""]));
                            None
                        }
                        _ => None,
                    };
                    log.zip(contains).map(|(log, contains)| Kind::Log { log, contains })
                }
                "eventlog" => self.eventlog(w, head),
                "mail" => self.mail(w, head),
                "after_jobnet" => {
                    let on = match self.choice(&w.on, "on", &["ok", "error", "any"]) {
                        Some("error") => AfterOn::Error,
                        Some("any") => AfterOn::Any,
                        _ => AfterOn::Ok,
                    };
                    match self.string(&w.after_jobnet, "after_jobnet") {
                        Some(u) if is_id(&u) => Some(Kind::AfterJobnet { jobnet: u, on }),
                        Some(_) => {
                            let pos = w.after_jobnet.as_ref().map_or(head, |f| f.span().start);
                            self.add(pos, msg::line(msg::DEF_JOBNET_REF_BAD, &["after_jobnet"]));
                            None
                        }
                        None => None,
                    }
                }
                "on_start" => match w.on_start.as_ref().and_then(|f| f.get_ref().as_bool()) {
                    Some(true) => Some(Kind::OnStart),
                    _ => {
                        let pos = w.on_start.as_ref().map_or(head, |f| f.span().start);
                        self.add(pos, msg::line(msg::DEF_ON_START_BAD, &[]));
                        None
                    }
                },
                "idle" => match self.duration_of(&w.idle, "idle") {
                    Some(s) if s < 60 => {
                        let pos = w.idle.as_ref().map_or(head, |f| f.span().start);
                        self.add(pos, msg::line(msg::DEF_IDLE_SHORT, &[]));
                        None
                    }
                    Some(s) => Some(Kind::Idle { idle: s }),
                    None => None,
                },
                "session" => {
                    let names = Session::ALL.map(Session::name);
                    let session = self.choice(&w.session, "session", &names).and_then(Session::from_name);
                    let user = self.session_user(&w.user);
                    match (session, user) {
                        (Some(session), Some(user)) => Some(Kind::Session { session, user }),
                        _ => None,
                    }
                }
                "day" => self.at(&w.at, head).map(|at| Kind::Day { at }),
                "minutes" | "hours" => {
                    let n = self.n(&w.n, head);
                    let span = self.span_of(&w.from, &w.to, head);
                    let days = self.weekdays(&w.days).unwrap_or_default();
                    match (n, span, kind) {
                        (Some(n), Some((from, to)), "minutes") => Some(Kind::Minutes { n, from, to, days }),
                        (Some(n), Some((from, to)), _) => Some(Kind::Hours { n, from, to, days }),
                        _ => None,
                    }
                }
                "week" => {
                    let days = if w.days.is_none() {
                        self.add(head, msg::line(msg::DEF_WHEN_KEY_MISSING, &["days", "days = [\"mon\", \"fri\"]"]));
                        None
                    } else {
                        self.weekdays(&w.days)
                    };
                    let at = self.at(&w.at, head);
                    days.zip(at).map(|(days, at)| Kind::Week { days, at })
                }
                "month" => {
                    let rule = self.month_rule(w, head);
                    let at = self.at(&w.at, head);
                    rule.zip(at).map(|(rule, at)| Kind::Month { rule, at })
                }
                "year" => {
                    let md = match &w.date {
                        None => {
                            self.add(head, msg::line(msg::DEF_WHEN_KEY_MISSING, &["date", "date = \"04-01\""]));
                            None
                        }
                        Some(f) => match f.get_ref().as_str() {
                            None => {
                                self.add(f.span().start, msg::line(msg::DEF_STRING_BAD_E, &["date", "\"04-01\""]));
                                None
                            }
                            Some(s) => match when::month_day(s) {
                                Ok(md) => Some(md),
                                Err(e) => {
                                    self.add(f.span().start, e);
                                    None
                                }
                            },
                        },
                    };
                    let at = self.at(&w.at, head);
                    md.zip(at).map(|((month, day), at)| Kind::Year { month, day, at })
                }
                _ => {
                    let dates = self.dates(&w.dates, "dates");
                    if dates.is_empty() && self.problems.len() == before {
                        self.add(head, msg::line(msg::DEF_VALUE_EMPTY, &["dates"]));
                    }
                    let at = self.at(&w.at, head);
                    (!dates.is_empty()).then_some(dates).zip(at).map(|(dates, at)| Kind::Dates { dates, at })
                }
            };
            let except = self.dates(&w.except, "except");
            let start = self.date_of(&w.start);
            let end = self.date_of(&w.end);
            let wait_for = match self.string(&w.wait_for, "wait_for") {
                Some(u) if is_id(&u) => Some(u),
                Some(_) => {
                    let pos = w.wait_for.as_ref().map_or(head, |f| f.span().start);
                    self.add(pos, msg::line(msg::DEF_JOBNET_REF_BAD, &["wait_for"]));
                    None
                }
                None => None,
            };
            let wait_until = match self.string(&w.wait_until, "wait_until") {
                Some(t) => match when::time(&t) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        let pos = w.wait_until.as_ref().map_or(head, |f| f.span().start);
                        self.add(pos, e);
                        None
                    }
                },
                None => None,
            };
            if wait_until.is_some() && wait_for.is_none() && w.wait_for.is_none() {
                let pos = w.wait_until.as_ref().map_or(head, |f| f.span().start);
                self.add(pos, msg::line(msg::DEF_WAIT_UNTIL_ALONE, &[]));
            }
            if let (Some(s), Some(e)) = (start, end) {
                if s > e {
                    self.add(w.start.as_ref().map_or(head, |f| f.span().start), msg::line(msg::DEF_PERIOD_ORDER, &[]));
                }
            }
            if let Some(kind) = kind {
                if self.problems.len() == before {
                    out.push(When { kind, except, start, end, on_closed, wait_for, wait_until });
                }
            }
        }
        out
    }

    /// `[[when]] log`: an absolute path to one file (no `*` / `?`).
    fn log_file(&mut self, field: &Field) -> Option<String> {
        let f = field.as_ref()?;
        let pos = f.span().start;
        let text = self.string(field, "log")?;
        if text.trim().is_empty() {
            self.add(pos, msg::line(msg::DEF_VALUE_EMPTY, &["log"]));
            return None;
        }
        if !absolute(&text) {
            self.add(pos, msg::line(msg::DEF_PATH_NOT_ABSOLUTE, &["log", "log = 'D:\\app\\app.log'"]));
            return None;
        }
        if text.contains(['*', '?']) || text.ends_with(['/', '\\']) {
            self.add(pos, msg::line(msg::DEF_LOG_PATH_BAD, &[]));
            return None;
        }
        Some(text)
    }

    /// `[[when]] eventlog = "Security"` with `ids = [4625]` or `source = "…"` (or both).
    fn eventlog(&mut self, w: &RawWhen, head: usize) -> Option<Kind> {
        let pos = w.eventlog.as_ref().map_or(head, |f| f.span().start);
        let eventlog = self.string(&w.eventlog, "eventlog")?;
        if eventlog.trim().is_empty() {
            self.add(pos, msg::line(msg::DEF_VALUE_EMPTY, &["eventlog"]));
            return None;
        }
        let mut ids = Vec::new();
        if let Some(f) = &w.ids {
            let read: Option<Vec<u32>> = match f.get_ref() {
                toml::Value::Array(items) => {
                    items.iter().map(|v| v.as_integer().and_then(|n| u32::try_from(n).ok()).filter(|n| *n <= 65535)).collect()
                }
                _ => None,
            };
            match read {
                Some(read) => ids = read,
                None => {
                    self.add(f.span().start, msg::line(msg::DEF_IDS_BAD, &[]));
                    return None;
                }
            }
        }
        let source = self.string(&w.source, "source");
        if let (Some(s), Some(f)) = (&source, &w.source) {
            if s.trim().is_empty() {
                self.add(f.span().start, msg::line(msg::DEF_VALUE_EMPTY, &["source"]));
                return None;
            }
        }
        if ids.is_empty() && source.is_none() {
            self.add(head, msg::line(msg::DEF_EITHER_REQUIRED, &["eventlog", "ids", "source", "ids = [4625]"]));
            return None;
        }
        Some(Kind::EventLog { eventlog, ids, source })
    }

    /// `[[when]] mail = "imaps://…"` with `password = "secret:<name>"` and
    /// `sender` or `subject` (or both): text the mail's sender / subject holds.
    fn mail(&mut self, w: &RawWhen, head: usize) -> Option<Kind> {
        let before = self.problems.len();
        let pos = w.mail.as_ref().map_or(head, |f| f.span().start);
        let mail = self.string(&w.mail, "mail")?;
        if let Err(e) = when::mailbox(&mail) {
            self.add(pos, e);
        }
        let secret = match (&w.password, self.string(&w.password, "password")) {
            (None, _) => {
                self.add(head, msg::line(msg::DEF_WHEN_KEY_MISSING, &["password", "password = \"secret:mailpass\""]));
                None
            }
            (Some(f), Some(p)) => match p.strip_prefix("secret:") {
                Some(name) if !name.trim().is_empty() => Some(name.to_owned()),
                Some(_) => {
                    self.add(f.span().start, msg::line(msg::DEF_SECRET_NAME_EMPTY, &["password"]));
                    None
                }
                None => {
                    self.add(f.span().start, msg::line(msg::DEF_PASSWORD_NOT_SECRET, &["password"]));
                    None
                }
            },
            (Some(_), None) => None,
        };
        let sender = self.mail_text(&w.sender, "sender");
        let subject = self.mail_text(&w.subject, "subject");
        if self.problems.len() != before {
            return None;
        }
        if sender.is_none() && subject.is_none() {
            self.add(head, msg::line(msg::DEF_EITHER_REQUIRED, &["mail", "sender", "subject", "subject = \"Invoice\""]));
            return None;
        }
        Some(Kind::Mail { mail, secret: secret?, sender, subject })
    }

    /// `user` of a `session` start: `Some(None)` when not written (anyone's),
    /// `Some(Some(name))` for an account name, `None` when it is wrong (said).
    fn session_user(&mut self, field: &Field) -> Option<Option<String>> {
        let Some(f) = field else { return Some(None) };
        let s = self.string(field, "user")?;
        if when::valid_session_user(&s) {
            return Some(Some(s));
        }
        self.add(f.span().start, msg::line(msg::DEF_SESSION_USER_BAD, &[]));
        None
    }

    /// `sender` / `subject` of a `mail` start: text, not empty.
    fn mail_text(&mut self, field: &Field, name: &str) -> Option<String> {
        let s = self.string(field, name)?;
        if s.trim().is_empty() {
            self.add(field.as_ref().map_or(0, |f| f.span().start), msg::line(msg::DEF_VALUE_EMPTY, &[name]));
            return None;
        }
        Some(s)
    }

    /// `[[when]] file`: an absolute path, `*` / `?` only in the name part.
    fn event_file(&mut self, field: &Field) -> Option<String> {
        let f = field.as_ref()?;
        let pos = f.span().start;
        let text = self.string(field, "file")?;
        if text.trim().is_empty() {
            self.add(pos, msg::line(msg::DEF_VALUE_EMPTY, &["file"]));
            return None;
        }
        // Split by either separator: the check reads the same on every OS.
        let (dir, name) = text.rsplit_once(['/', '\\']).unwrap_or(("", text.as_str()));
        if !absolute(&text) {
            self.add(pos, msg::line(msg::DEF_PATH_NOT_ABSOLUTE, &["file", "file = 'D:\\recv\\*.csv'"]));
            return None;
        }
        if name.is_empty() {
            self.add(pos, msg::line(msg::DEF_FILE_NAME_MISSING, &[]));
            return None;
        }
        if dir.contains(['*', '?']) {
            self.add(pos, msg::line(msg::DEF_FILE_WILDCARD_DIR, &[]));
            return None;
        }
        Some(text)
    }

    fn at(&mut self, field: &Field, head: usize) -> Option<Vec<NaiveTime>> {
        let Some(f) = field else {
            self.add(head, msg::line(msg::DEF_WHEN_KEY_MISSING, &["at", "at = [\"02:00\"]"]));
            return None;
        };
        let toml::Value::Array(items) = f.get_ref() else {
            self.add(f.span().start, msg::line(msg::DEF_ARRAY_BAD, &["at", "at = [\"02:00\", \"14:00\"]"]));
            return None;
        };
        if items.is_empty() {
            self.add(f.span().start, msg::line(msg::DEF_VALUE_EMPTY, &["at"]));
            return None;
        }
        let mut out = Vec::new();
        for v in items {
            let Some(s) = v.as_str() else {
                self.add(f.span().start, msg::line(msg::DEF_STRING_BAD_E, &["at", "\"02:00\""]));
                continue;
            };
            match when::time(s) {
                Ok(t) => out.push(t),
                Err(e) => self.add(f.span().start, e),
            }
        }
        (out.len() == items.len()).then_some(out)
    }

    fn time_of(&mut self, f: &Spanned<toml::Value>, name: &str) -> Option<NaiveTime> {
        let Some(s) = f.get_ref().as_str() else {
            self.add(f.span().start, msg::line(msg::DEF_STRING_BAD_E, &[name, "\"09:00\""]));
            return None;
        };
        match when::time(s) {
            Ok(t) => Some(t),
            Err(e) => {
                self.add(f.span().start, e);
                None
            }
        }
    }

    /// `days`; all days when it is not written.
    fn weekdays(&mut self, field: &Field) -> Option<Vec<Weekday>> {
        let Some(f) = field else { return Some(Vec::new()) };
        let toml::Value::Array(items) = f.get_ref() else {
            self.add(f.span().start, msg::line(msg::DEF_ARRAY_BAD, &["days", "days = [\"mon\", \"fri\"]"]));
            return None;
        };
        if items.is_empty() {
            self.add(f.span().start, msg::line(msg::DEF_VALUE_EMPTY, &["days"]));
            return None;
        }
        let mut out = Vec::new();
        for v in items {
            let Some(s) = v.as_str() else {
                self.add(f.span().start, msg::line(msg::DEF_STRING_BAD_E, &["days", "\"mon\""]));
                continue;
            };
            match when::weekday(s) {
                Ok(d) => out.push(d),
                Err(e) => self.add(f.span().start, e),
            }
        }
        (out.len() == items.len()).then_some(out)
    }

    fn n(&mut self, field: &Field, head: usize) -> Option<u32> {
        let Some(f) = field else {
            self.add(head, msg::line(msg::DEF_WHEN_KEY_MISSING, &["n", "n = 15"]));
            return None;
        };
        match f.get_ref().as_integer() {
            Some(n) if (1..=1440).contains(&n) => Some(n as u32),
            _ => {
                self.add(f.span().start, msg::line(msg::DEF_INT_RANGE, &["n", "1", "1440"]));
                None
            }
        }
    }

    fn span_of(&mut self, from: &Field, to: &Field, head: usize) -> Option<(NaiveTime, NaiveTime)> {
        let (Some(a), Some(b)) = (from, to) else {
            if from.is_none() {
                self.add(head, msg::line(msg::DEF_WHEN_KEY_MISSING, &["from", "from = \"09:00\""]));
            }
            if to.is_none() {
                self.add(head, msg::line(msg::DEF_WHEN_KEY_MISSING, &["to", "to = \"18:00\""]));
            }
            return None;
        };
        let from = self.time_of(a, "from");
        let to = self.time_of(b, "to");
        let (from, to) = from.zip(to)?;
        if from > to {
            self.add(a.span().start, msg::line(msg::DEF_TIME_ORDER, &[]));
            return None;
        }
        Some((from, to))
    }

    fn month_rule(&mut self, w: &RawWhen, head: usize) -> Option<MonthRule> {
        if let Some(b) = &w.business_day {
            for (field, name) in [(&w.day, "day"), (&w.nth, "nth"), (&w.weekday, "weekday"), (&w.shift, "shift")] {
                if let Some(f) = field {
                    self.add(f.span().start, msg::line(msg::DEF_BUSINESS_DAY_CONFLICT, &[name]));
                }
            }
            return match b.get_ref().as_integer() {
                Some(n) if (1..=31).contains(&n) || (-31..=-1).contains(&n) => Some(MonthRule::BusinessDay { n: n as i32 }),
                _ => {
                    self.add(b.span().start, msg::line(msg::DEF_BUSINESS_DAY_BAD, &[]));
                    None
                }
            };
        }
        let shift = match &w.shift {
            None => Some(0),
            Some(f) if !matches!(w.day.as_ref().map(|d| d.get_ref()), Some(toml::Value::Integer(_))) => {
                self.add(f.span().start, msg::line(msg::DEF_SHIFT_NEEDS_DAY, &[]));
                None
            }
            Some(f) => match f.get_ref().as_integer() {
                Some(n) if (-31..=31).contains(&n) => Some(n as i32),
                _ => {
                    self.add(f.span().start, msg::line(msg::DEF_INT_RANGE, &["shift", "-31", "31"]));
                    None
                }
            },
        };
        match (&w.day, &w.nth, &w.weekday) {
            (Some(d), None, None) => match d.get_ref() {
                toml::Value::Integer(n) if (1..=31).contains(n) => shift.map(|shift| MonthRule::Day { day: *n as u32, shift }),
                toml::Value::String(s) if s == "last" => Some(MonthRule::Last),
                _ => {
                    self.add(d.span().start, msg::line(msg::DEF_DAY_BAD, &[]));
                    None
                }
            },
            (None, Some(n), Some(wd)) => {
                let nth = match n.get_ref().as_integer() {
                    Some(n) if (1..=5).contains(&n) => Some(n as u32),
                    _ => {
                        self.add(n.span().start, msg::line(msg::DEF_INT_RANGE, &["nth", "1", "5"]));
                        None
                    }
                };
                let weekday = match wd.get_ref().as_str() {
                    None => {
                        self.add(wd.span().start, msg::line(msg::DEF_STRING_BAD_E, &["weekday", "\"tue\""]));
                        None
                    }
                    Some(s) => match when::weekday(s) {
                        Ok(d) => Some(d),
                        Err(e) => {
                            self.add(wd.span().start, e);
                            None
                        }
                    },
                };
                nth.zip(weekday).map(|(nth, weekday)| MonthRule::Nth { nth, weekday })
            }
            _ => {
                self.add(head, msg::line(msg::DEF_MONTH_RULE_BAD, &[]));
                None
            }
        }
    }

    fn dates(&mut self, field: &Field, name: &str) -> Vec<NaiveDate> {
        let Some(f) = field else { return Vec::new() };
        let toml::Value::Array(items) = f.get_ref() else {
            self.add(f.span().start, msg::line(msg::DEF_ARRAY_BAD, &[name, &format!("{name} = [2026-12-30]")]));
            return Vec::new();
        };
        let mut out = Vec::new();
        for v in items {
            match when::date(v) {
                Ok(d) => out.push(d),
                Err(e) => self.add(f.span().start, e),
            }
        }
        out
    }

    fn date_of(&mut self, field: &Field) -> Option<NaiveDate> {
        let f = field.as_ref()?;
        match when::date(f.get_ref()) {
            Ok(d) => Some(d),
            Err(e) => {
                self.add(f.span().start, e);
                None
            }
        }
    }

    fn datetime(&mut self, field: &Field) -> Option<NaiveDateTime> {
        let f = field.as_ref()?;
        match when::datetime(f.get_ref()) {
            Ok(d) => Some(d),
            Err(e) => {
                self.add(f.span().start, e);
                None
            }
        }
    }

    /// `judge = { warn = "1", error = "2.." }`: either may be left out, not both.
    fn judge(&mut self, field: &Field) -> Option<Judge> {
        const SHAPE: &str = "{ warn = \"1\", error = \"2..\" }";
        let f = field.as_ref()?;
        let at = f.span().start;
        let toml::Value::Table(t) = f.get_ref() else {
            self.add(at, msg::line(msg::DEF_TABLE_BAD, &["judge", SHAPE]));
            return None;
        };
        let mut ok = true;
        let mut judge = Judge::default();
        for (k, v) in t {
            let into = match k.as_str() {
                "warn" => &mut judge.warn,
                "error" => &mut judge.error,
                "error_output" | "ok_file" => {
                    let toml::Value::String(s) = v else {
                        self.add(at, msg::line(msg::DEF_STRING_BAD, &[&format!("judge.{k}")]));
                        ok = false;
                        continue;
                    };
                    if k == "error_output" {
                        if let Err(e) = ito_match::Matcher::new(s) {
                            self.add(at, msg::line(msg::DEF_REGEX_BAD, &["judge.error_output", &e.to_string()]));
                            ok = false;
                        }
                        judge.error_output = Some(s.clone());
                    } else {
                        if let Err(e) = self.template(s) {
                            self.add(at, msg::line(msg::DEF_TEMPLATE_BAD, &["judge.ok_file", msg::uncoded(&e)]));
                            ok = false;
                        }
                        judge.ok_file = Some(s.clone());
                    }
                    continue;
                }
                _ => {
                    self.add(at, msg::line(msg::DEF_SUBKEY_UNKNOWN, &["judge", k, "warn, error, error_output, ok_file"]));
                    ok = false;
                    continue;
                }
            };
            let text = match v {
                toml::Value::String(s) => s.clone(),
                toml::Value::Integer(n) => n.to_string(),
                _ => {
                    self.add(at, msg::line(msg::DEF_STRING_BAD_E, &[&format!("judge.{k}"), "\"2..\""]));
                    ok = false;
                    continue;
                }
            };
            match ranges(&text) {
                Ok(r) => *into = r,
                Err(e) => {
                    self.add(at, msg::line(msg::DEF_VALUE_UNREADABLE, &[&format!("judge.{k}"), msg::uncoded(&e)]));
                    ok = false;
                }
            }
        }
        if ok && judge.warn.is_empty() && judge.error.is_empty() && judge.error_output.is_none() && judge.ok_file.is_none() {
            self.add(at, msg::line(msg::DEF_JUDGE_EMPTY, &[]));
            ok = false;
        }
        ok.then_some(judge)
    }

    fn jobs(&mut self, raw: &[Spanned<RawJob>]) -> Vec<Job> {
        let mut jobs = Vec::new();
        let mut seen = BTreeSet::new();
        for entry in raw {
            let start = entry.span().start;
            let j = entry.get_ref();
            let part = self.part(j, start);
            let id = match &j.id {
                None => {
                    self.add(start, msg::line(msg::DEF_ID_MISSING, &[]));
                    String::new()
                }
                Some(f) => match f.get_ref() {
                    toml::Value::String(s) if !s.is_empty() => {
                        if !seen.insert(s.clone()) {
                            self.add(f.span().start, msg::line(msg::DEF_ID_DUPLICATE, &[s]));
                        }
                        s.clone()
                    }
                    _ => {
                        self.add(f.span().start, msg::line(msg::DEF_ID_BAD, &[]));
                        String::new()
                    }
                },
            };
            let (run, shell, script, dir, retry, judge) = if part.runs_process() && !matches!(part, Part::Chore(_)) {
                let shell = self.choice(&j.shell, "shell", &["powershell"]).map(str::to_owned);
                let script = self.string(&j.script, "script");
                // Either `run`, or a shell with its script: never both, never a shell line in `run`.
                let run = match (&j.shell, &j.script) {
                    (Some(s), _) => {
                        if let Some(f) = &j.run {
                            self.add(f.span().start, msg::line(msg::DEF_RUN_SHELL_BOTH, &[]));
                        }
                        if j.script.is_none() {
                            self.add(s.span().start, msg::line(msg::DEF_SHELL_NEEDS_SCRIPT, &[]));
                        } else if script.as_deref().is_some_and(|t| t.trim().is_empty()) {
                            self.add(s.span().start, msg::line(msg::DEF_VALUE_EMPTY, &["script"]));
                        }
                        Vec::new()
                    }
                    (None, Some(f)) => {
                        self.add(f.span().start, msg::line(msg::DEF_SCRIPT_NEEDS_SHELL, &[]));
                        self.run(&j.run, start)
                    }
                    // A refused `kind` is one problem, not two.
                    (None, None)
                        if j.run.is_none() && part.is_job() && j.kind.as_ref().is_some_and(|f| f.get_ref().as_str() != Some("job")) =>
                    {
                        Vec::new()
                    }
                    (None, None) => self.run(&j.run, start),
                };
                let dir = self.string(&j.dir, "dir");
                let retry = self.retry(&j.retry);
                let judge = self.judge(&j.judge);
                (run, shell, script, dir, retry, judge)
            } else {
                // An `ssh` or `winrm` is judged by the far command's exit code, as a job is.
                let ssh = matches!(&part, Part::Chore(c) if c.far());
                for (field, name) in
                    [(&j.run, "run"), (&j.shell, "shell"), (&j.script, "script"), (&j.dir, "dir"), (&j.retry, "retry"), (&j.judge, "judge")]
                {
                    if !(name == "retry" && matches!(part, Part::Chore(_)) || name == "judge" && ssh) {
                        self.only(
                            field,
                            name,
                            match name {
                                "judge" => "\"job\", \"loop\", \"ssh\", \"winrm\"",
                                "retry" => {
                                    "\"job\", \"loop\", \"delete\", \"zip\", \"copy\", \"empty\", \"send\", \"fetch\", \"http\", \"ssh\", \"winrm\", \"mail\", \"sql\""
                                }
                                _ => "\"job\", \"loop\"",
                            },
                        );
                    }
                }
                let retry = if matches!(part, Part::Chore(_)) { self.retry(&j.retry) } else { None };
                let judge = if ssh { self.judge(&j.judge) } else { None };
                (Vec::new(), None, None, None, retry, judge)
            };
            for (text, name) in
                run.iter().map(|t| (t, "run")).chain(script.iter().map(|t| (t, "script"))).chain(dir.iter().map(|t| (t, "dir")))
            {
                if let Err(e) = self.template(text) {
                    self.add(start, msg::line(msg::DEF_TEMPLATE_BAD, &[name, msg::uncoded(&e)]));
                }
            }
            let (env, encoding) = if part.runs_process() && !matches!(part, Part::Chore(_)) {
                let encoding = self.choice(&j.encoding, "encoding", &["auto", "sjis", "utf8"]).and_then(Encoding::of).unwrap_or_default();
                (self.env(&j.env), encoding)
            } else if matches!(&part, Part::Chore(c) if c.far()) {
                // Asked of the far side (an sshd_config's AcceptEnv; WinRM's shell takes
                // them all); its output is read as it comes.
                self.only(&j.encoding, "encoding", "\"job\", \"loop\"");
                (self.env(&j.env), Encoding::Auto)
            } else {
                self.only(&j.env, "env", "\"job\", \"loop\", \"ssh\", \"winrm\"");
                self.only(&j.encoding, "encoding", "\"job\", \"loop\"");
                (BTreeMap::new(), Encoding::Auto)
            };
            // A `mail` reads the settings and the password of this account.
            let mail = matches!(&part, Part::Chore(Chore::Mail { .. }));
            let run_as = if part.runs_process() && !mail {
                self.run_as(&j.run_as)
            } else {
                self.only(&j.run_as, "run_as", "\"job\", \"loop\", \"delete\", \"zip\", \"copy\", \"empty\", \"send\", \"fetch\", \"http\", \"ssh\", \"winrm\", \"sql\"");
                None
            };
            let timeout = if part.runs_process() || matches!(part, Part::File { .. }) {
                self.timeout(&j.timeout)
            } else {
                self.only(
                    &j.timeout,
                    "timeout",
                    "\"job\", \"loop\", \"file\", \"delete\", \"zip\", \"copy\", \"empty\", \"send\", \"fetch\", \"http\", \"ssh\", \"winrm\", \"mail\", \"sql\"",
                );
                None
            };
            let after = self.after(&j.after);
            if part == Part::Branch && after.len() != 1 {
                self.add(start, msg::line(msg::DEF_BRANCH_AFTER_COUNT, &[]));
            }
            let name = self.string(&j.name, "name");
            let note = self.string(&j.note, "note");
            let pos = self.pos(&j.pos);
            // A `mail`'s `to` is its addresses, in its part.
            let to = if part == Part::Notify {
                self.names(&j.to, "to")
            } else {
                if !mail {
                    self.only(&j.to, "to", "\"notify\", \"mail\"");
                }
                Vec::new()
            };
            jobs.push(Job { id, name, note, run, dir, after, pos, part, timeout, retry, shell, script, judge, env, encoding, to, run_as });
        }
        let followed: BTreeSet<&str> = jobs.iter().flat_map(|j| j.after.iter().map(|l| l.job.as_str())).collect();
        for (job, entry) in jobs.iter().zip(raw) {
            let at = entry.get_ref().after.as_ref().map_or(entry.span().start, |f| f.span().start);
            for l in &job.after {
                if !seen.contains(l.job.as_str()) {
                    self.add(at, msg::line(msg::DEF_AFTER_UNKNOWN_ID, &[&l.job]));
                } else if matches!(l.on, On::Code(_) | On::Else) && !jobs.iter().any(|p| p.id == l.job && p.part == Part::Branch) {
                    self.add(at, msg::line(msg::DEF_CODE_LINE_NOT_BRANCH, &[&l.job]));
                }
            }
            // A part that nothing leads to, or that leads nowhere, does nothing: a mistake, not a quiet no-op.
            let (needs_after, needs_next) = match job.part {
                Part::Branch | Part::Join { .. } => (true, true),
                Part::Time { .. } | Part::File { .. } | Part::Approval { .. } => (false, true),
                Part::Notify => (true, false),
                Part::Job | Part::Loop { .. } | Part::Jobnet { .. } | Part::Chore(_) => (false, false),
            };
            if needs_after && job.after.is_empty() {
                self.add(entry.span().start, msg::line(msg::DEF_JOB_NO_BEFORE, &[&job.id]));
            }
            if needs_next && !followed.contains(job.id.as_str()) {
                self.add(entry.span().start, msg::line(msg::DEF_JOB_NO_NEXT, &[&job.id]));
            }
        }
        if let Some(cycle) = find_cycle(&jobs) {
            let first = jobs.iter().position(|j| j.id == cycle[0]).unwrap_or(0);
            self.add(raw[first].span().start, msg::line(msg::DEF_AFTER_CYCLE, &[&cycle.join(" -> ")]));
        }
        jobs
    }

    fn run(&mut self, field: &Field, job_start: usize) -> Vec<String> {
        let Some(f) = field else {
            self.add(job_start, msg::line(msg::DEF_RUN_MISSING, &[]));
            return Vec::new();
        };
        let at = f.span().start;
        let toml::Value::Array(items) = f.get_ref() else {
            self.add(at, msg::line(msg::DEF_ARRAY_BAD, &["run", "run = ['C:\\batch\\a.exe', '--x']"]));
            return Vec::new();
        };
        let strings: Vec<String> = items.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        if strings.len() != items.len() {
            self.add(at, msg::line(msg::DEF_ARRAY_BAD, &["run", "run = ['C:\\batch\\a.exe', '--x']"]));
        } else if strings.is_empty() {
            self.add(at, msg::line(msg::DEF_VALUE_EMPTY, &["run"]));
        } else if strings[0].trim().is_empty() {
            self.add(at, msg::line(msg::DEF_RUN_PROGRAM_EMPTY, &[]));
        }
        strings
    }

    fn after(&mut self, field: &Field) -> Vec<Line> {
        let Some(f) = field else { return Vec::new() };
        let at = f.span().start;
        let toml::Value::Array(items) = f.get_ref() else {
            self.add(at, msg::line(msg::DEF_ARRAY_BAD, &["after", "after = [\"extract\"]"]));
            return Vec::new();
        };
        let mut out = Vec::new();
        for v in items {
            match v {
                toml::Value::String(s) => out.push(Line::ok(s)),
                toml::Value::Table(t) => match self.line(t) {
                    Ok(l) => out.push(l),
                    Err(e) => self.add(at, e),
                },
                _ => self.add(at, msg::line(msg::DEF_AFTER_ELEMENT_BAD, &[])),
            }
        }
        out
    }

    /// One `{ job = "a", on = "error" }` / `{ job = "a", code = "1..3" }` of `after`.
    fn line(&mut self, t: &toml::Table) -> Result<Line, String> {
        for k in t.keys() {
            if !matches!(k.as_str(), "job" | "on" | "code") {
                return Err(msg::line(msg::DEF_SUBKEY_UNKNOWN, &["after", k, "job, on, code"]));
            }
        }
        let Some(job) = t.get("job").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) else {
            return Err(msg::line(msg::DEF_SUBKEY_MISSING, &["after", "job", "{ job = \"extract\", on = \"error\" }"]));
        };
        let job = job.to_owned();
        let text = |k: &str| match t.get(k) {
            None => Ok(None),
            Some(toml::Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(msg::line(msg::DEF_STRING_BAD, &[&format!("after.{k}")])),
        };
        let on = text("on")?;
        let code = text("code")?;
        line_of_parts(job, on.as_deref(), code.as_deref())
    }

    fn pos(&mut self, field: &Field) -> Option<(f64, f64)> {
        let f = field.as_ref()?;
        let num = |v: &toml::Value| v.as_float().or_else(|| v.as_integer().map(|i| i as f64));
        if let toml::Value::Array(a) = f.get_ref() {
            if let [x, y] = a.as_slice() {
                if let (Some(x), Some(y)) = (num(x), num(y)) {
                    return Some((x, y));
                }
            }
        }
        self.add(f.span().start, msg::line(msg::DEF_POS_BAD, &[]));
        None
    }
}

/// Whether `text` is an absolute path: a drive (`D:\…`), a share (`\\server\…`) or `/…`.
fn absolute(text: &str) -> bool {
    let drive = text.as_bytes().get(1) == Some(&b':') && text.as_bytes().first().is_some_and(u8::is_ascii_alphabetic);
    drive || text.starts_with("\\\\") || text.starts_with('/')
}

/// A name Windows keeps for a device (`NUL`, `COM1`), also with an extension.
pub(crate) fn reserved(s: &str) -> bool {
    let stem = s.split('.').next().unwrap_or(s).trim_end().to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.ends_with(|c: char| ('1'..='9').contains(&c)))
}

/// `グループ/名前`: one `/`, both sides there, nothing that leaves the folder.
/// What `use`, `after_jobnet` and the editor's new net are checked with.
pub fn is_id(u: &str) -> bool {
    let mut parts = u.split('/');
    let (Some(g), Some(n), None) = (parts.next(), parts.next(), parts.next()) else { return false };
    // Nothing a path can use to leave the folder: a drive (`C:`), a stream (`a:b`), a wildcard,
    // or a name Windows trims (a trailing dot or space).
    let plain = |s: &str| {
        !s.trim().is_empty()
            && s != "."
            && s != ".."
            && !s.ends_with(['.', ' '])
            && !reserved(s)
            && !s.chars().any(|c| matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control())
    };
    plain(g) && plain(n)
}

#[cfg(test)]
mod id_tests {
    use super::is_id;

    #[test]
    fn an_id_cannot_leave_the_folder() {
        assert!(is_id("経理/月次"));
        for bad in [
            "C:/x",
            "g/a:b",
            "g/a*",
            "g/..",
            "../x",
            "g/a.",
            "g/a ",
            "g\\x/y",
            "g/a|b",
            "g/\u{7}",
            "g/NUL",
            "con/x",
            "g/Com1.txt",
            "g/lpt9",
        ] {
            assert!(!is_id(bad), "{bad}");
        }
    }
}

/// The ids of a loop in `after` (`a → b → a`), if any.
fn find_cycle(jobs: &[Job]) -> Option<Vec<String>> {
    type After<'a> = BTreeMap<&'a str, &'a [Line]>;
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Open,
        Done,
    }
    fn visit<'a>(id: &'a str, after: &After<'a>, marks: &mut BTreeMap<&'a str, Mark>, path: &mut Vec<&'a str>) -> Option<Vec<String>> {
        match marks.get(id) {
            Some(Mark::Done) => return None,
            Some(Mark::Open) => {
                let from = path.iter().position(|p| *p == id).unwrap_or(0);
                let mut cycle: Vec<String> = path[from..].iter().map(|s| s.to_string()).collect();
                cycle.push(id.to_string());
                return Some(cycle);
            }
            None => {}
        }
        marks.insert(id, Mark::Open);
        path.push(id);
        for next in after.get(id).copied().unwrap_or(&[]).iter().map(|l| l.job.as_str()) {
            if after.contains_key(next) {
                if let Some(c) = visit(next, after, marks, path) {
                    return Some(c);
                }
            }
        }
        path.pop();
        marks.insert(id, Mark::Done);
        None
    }
    let after: After = jobs.iter().map(|j| (j.id.as_str(), j.after.as_slice())).collect();
    let mut marks = BTreeMap::new();
    jobs.iter().find_map(|j| visit(&j.id, &after, &mut marks, &mut Vec::new()))
}

/// The net with each `kind = "jobnet"` part replaced by the jobs of the net
/// it uses (`lookup` reads `use` as `グループ/名前`): a child's job `x` under the
/// part `p` is `p/x`, named `p's label/x's label`; the child's first jobs get
/// the part's own lines, and the part stays as the station that ends when the
/// child's last jobs have ended. The child's `[[when]]` is not used. A child
/// that uses a child is expanded the same way; a net that uses itself, by any
/// way round, is refused (`scan` says so before anything runs).
pub fn expand(net: &Jobnet, lookup: &dyn Fn(&str) -> Result<Jobnet, String>) -> Result<Jobnet, String> {
    expand_in(net, lookup, &mut Vec::new())
}

/// Each job of the [`expand`]ed `net` that runs a program, placed in the folder `base` gives for
/// the net it came from: `own`, or the child net whose part it is under. A job without `dir` runs
/// there and a relative `dir` is read from there; an absolute `dir`, and a net `base` gives no
/// folder for, stay as they are. Run after `{…}` are written out: a `{param:…}` may be a whole path.
pub fn place(net: &mut Jobnet, own: &str, base: &dyn Fn(&str) -> Option<std::path::PathBuf>) {
    let parts: Vec<(String, String)> = net
        .jobs
        .iter()
        .filter_map(|j| match &j.part {
            Part::Jobnet { use_ } => Some((format!("{}/", j.id), use_.clone())),
            _ => None,
        })
        .collect();
    for j in net.jobs.iter_mut().filter(|j| j.part.runs_process() && !matches!(j.part, Part::Chore(_))) {
        let from = parts.iter().filter(|(p, _)| j.id.starts_with(p.as_str())).max_by_key(|(p, _)| p.len()).map_or(own, |(_, u)| u.as_str());
        let Some(folder) = base(from) else { continue };
        j.dir = Some(match j.dir.take() {
            Some(d) if std::path::Path::new(&d).is_absolute() => d,
            Some(d) => folder.join(d).to_string_lossy().into_owned(),
            None => folder.to_string_lossy().into_owned(),
        });
    }
}

fn expand_in(net: &Jobnet, lookup: &dyn Fn(&str) -> Result<Jobnet, String>, stack: &mut Vec<String>) -> Result<Jobnet, String> {
    let mut out = net.clone();
    out.jobs.clear();
    for j in &net.jobs {
        let Part::Jobnet { use_ } = &j.part else {
            out.jobs.push(j.clone());
            continue;
        };
        if stack.contains(use_) {
            return Err(msg::line(msg::DEF_CHILD_LOOP, &[&format!("{} -> {use_}", stack.join(" -> "))]));
        }
        let child = lookup(use_)?;
        stack.push(use_.clone());
        let child = expand_in(&child, lookup, stack)?;
        stack.pop();
        // The child's `{param:…}` take the parent's value of the same name, else the child's default.
        for (k, v) in &child.params {
            out.params.entry(k.clone()).or_insert_with(|| v.clone());
        }
        let prefix = format!("{}/", j.id);
        let label = j.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| j.id.clone());
        let followed: BTreeSet<&str> = child.jobs.iter().flat_map(|c| c.after.iter().map(|l| l.job.as_str())).collect();
        let mut station = j.clone();
        if !child.jobs.is_empty() {
            station.after = child
                .jobs
                .iter()
                .filter(|c| !followed.contains(c.id.as_str()))
                .map(|c| Line { job: format!("{prefix}{}", c.id), on: On::Any })
                .collect();
        }
        for c in &child.jobs {
            let mut c = c.clone();
            let own = c.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| c.id.clone());
            c.id = format!("{prefix}{}", c.id);
            c.name = Some(format!("{label}/{own}"));
            c.pos = None;
            if c.after.is_empty() {
                c.after = j.after.clone();
            } else {
                for l in &mut c.after {
                    l.job = format!("{prefix}{}", l.job);
                }
            }
            out.jobs.push(c);
        }
        out.jobs.push(station);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_may_wait_for_another_net_until_a_time_of_day() {
        let job = "[[job]]
id = \"a\"
run = [\"x\"]
";
        let ok = format!(
            "format = 1
[[when]]
every = \"day\"
at = [\"09:00\"]
wait_for = \"経理/売上取込\"
wait_until = \"11:30\"
{job}"
        );
        let net = parse(&ok).unwrap();
        assert_eq!(net.when[0].wait_for.as_deref(), Some("経理/売上取込"));
        assert_eq!(net.when[0].wait_until, Some(chrono::NaiveTime::from_hms_opt(11, 30, 0).unwrap()));
        assert_eq!(parse(&crate::write::text(&net)).unwrap().when, net.when);
        for bad in [
            "wait_until = \"11:30\"
",
            "wait_for = \"x\"
",
            "wait_for = \"a/b\"
wait_until = \"25:00\"
",
        ] {
            let text = format!(
                "format = 1
[[when]]
every = \"day\"
at = [\"09:00\"]
{bad}{job}"
            );
            assert!(parse(&text).is_err(), "{bad}");
        }
        let ev = format!(
            "format = 1
[[when]]
on_start = true
wait_for = \"a/b\"
{job}"
        );
        assert!(parse(&ev).is_err());
    }

    const GOOD: &str = r#"format = 1
name = "月次締め"

[[job]]
id = "extract"
name = "売上の抽出"
run = ['C:\batch\extract.bat', '--month']
dir = 'C:\batch'
pos = [40, 80]

[[job]]
id = "zip"
run = ["zip.exe", "a b"]
after = ["extract"]
"#;

    /// A problem as the user reads it now: the message with its ID at the head, the place last.
    fn placed(message: String, line: u32) -> String {
        format!("{message}（a.toml の {line} 行目）")
    }

    fn problems(text: &str) -> Vec<String> {
        match parse(text) {
            Ok(_) => Vec::new(),
            Err(p) => p.iter().map(|p| p.in_file("a.toml")).collect(),
        }
    }

    /// `{var:…}` reads in a job and a `[params]` default; only a name it cannot have is told here.
    #[test]
    fn a_var_needs_a_plain_name_and_nothing_else_here() {
        let net = |param: &str, run: &str| format!("format = 1\n[params]\nat = '{param}'\n[[job]]\nid = \"a\"\nrun = ['{run}']\n");
        assert!(parse(&net(r"{var:share}\in", "copy {var:share} {param:at}")).is_ok());
        let empty_line = msg::line(msg::VAR_NAME_EMPTY, &[]);
        let empty = msg::uncoded(&empty_line);
        assert!(problems(&net("x", "copy {var:}")).iter().any(|p| msg::DEF_TEMPLATE_BAD.is_in(p) && p.contains(empty)));
        assert!(problems(&net("{var:共有}", "x")).iter().any(|p| msg::DEF_TEMPLATE_BAD.is_in(p) && p.contains("共有")));
    }

    #[test]
    fn a_good_file_reads() {
        let net = parse(GOOD).unwrap();
        assert_eq!(net.name.as_deref(), Some("月次締め"));
        assert_eq!(net.jobs.len(), 2);
        assert_eq!(net.jobs[0].run, [r"C:\batch\extract.bat", "--month"]);
        assert_eq!(net.jobs[0].pos, Some((40.0, 80.0)));
        assert_eq!(net.jobs[1].after, [Line::ok("extract")]);
    }

    #[test]
    fn an_unknown_field_is_named_with_its_line() {
        let text = GOOD.replace("run = [\"zip.exe\", \"a b\"]", "rnu = [\"zip.exe\"]");
        assert_eq!(problems(&text), [placed(msg::line(msg::DEF_FIELD_UNKNOWN, &["rnu"]), 13)]);
    }

    #[test]
    fn toml_syntax_errors_name_a_line() {
        let p = problems("format = 1\nname = \n");
        assert_eq!(p.len(), 1);
        assert!(msg::DEF_TOML_SYNTAX.is_in(&p[0]) && p[0].ends_with("（a.toml の 2 行目）"), "{p:?}");
    }

    #[test]
    fn env_is_checked_and_kept() {
        let head = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";
        let net = parse(&format!("{head}env = {{ DB_PASS = \"secret:db\", OUT = \"{{month-1}}\" }}\n")).unwrap();
        assert_eq!(net.jobs[0].env["DB_PASS"], "secret:db");
        for (bad, want) in [
            ("env = { 1A = \"x\" }", msg::DEF_NAME_BAD),
            ("env = { KOYOMI_X = \"x\" }", msg::DEF_ENV_RESERVED),
            ("env = { A = 1 }", msg::DEF_VALUE_STRING_BAD),
            ("env = { A = \"secret:\" }", msg::DEF_SECRET_NAME_EMPTY),
            ("env = { A = \"{date-1x}\" }", msg::DEF_TEMPLATE_BAD),
        ] {
            let p = problems(&format!("{head}{bad}\n"));
            assert!(p.len() == 1 && want.is_in(&p[0]), "{bad}: {p:?}");
        }
        let p = problems(&format!("{head}env = {{ A = \"{{date-1x}}\" }}\n"));
        assert!(p[0].contains("{date-1x} の単位 x を読み込めません"), "{p:?}");
    }

    #[test]
    fn run_as_takes_an_account_and_a_secret_never_a_password() {
        let head = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";
        let net = parse(&format!("{head}run_as = {{ user = 'CORP\\batch', password = \"secret:batch\" }}\n")).unwrap();
        let r = net.jobs[0].run_as.clone().unwrap();
        assert_eq!(r, RunAs { user: "CORP\\batch".into(), secret: "batch".into() });
        assert_eq!(r.split(), (Some("CORP"), "batch"));
        for (user, split) in [(".\\batch", (None, "batch")), ("batch@corp.local", (None, "batch@corp.local")), ("batch", (None, "batch"))] {
            assert_eq!(RunAs { user: user.into(), secret: "s".into() }.split(), split);
        }
        let chore = "format = 1\n[[job]]\nid = \"a\"\nkind = \"copy\"\nfrom = 'D:\\a'\ninto = '\\\\nas\\b'\n";
        assert!(parse(&format!("{chore}run_as = {{ user = \"batch\", password = \"secret:b\" }}\n")).unwrap().jobs[0].run_as.is_some());
        for (bad, want) in [
            ("run_as = { user = \"batch\", password = \"hunter2\" }", msg::DEF_PASSWORD_NOT_SECRET),
            ("run_as = { user = \"batch\", password = \"secret:\" }", msg::DEF_SECRET_NAME_EMPTY),
            ("run_as = { user = \"batch\" }", msg::DEF_SUBKEY_MISSING),
            ("run_as = { password = \"secret:b\" }", msg::DEF_SUBKEY_MISSING),
            ("run_as = { user = \"a b\", password = \"secret:b\" }", msg::DEF_RUN_AS_USER_BAD),
            ("run_as = { user = '\\batch', password = \"secret:b\" }", msg::DEF_RUN_AS_USER_BAD),
            ("run_as = { user = 'A\\B\\c', password = \"secret:b\" }", msg::DEF_RUN_AS_USER_BAD),
            ("run_as = { user = \"b\", password = \"secret:b\", shell = \"x\" }", msg::DEF_SUBKEY_UNKNOWN),
            ("run_as = \"batch\"", msg::DEF_TABLE_BAD),
        ] {
            let p = problems(&format!("{head}{bad}\n"));
            assert!(p.len() == 1 && want.is_in(&p[0]), "{bad}: {p:?}");
        }
        let p = problems("format = 1\n[[job]]\nid = \"a\"\nkind = \"notify\"\nrun_as = { user = \"b\", password = \"secret:b\" }\n");
        assert!(p.iter().any(|m| msg::DEF_ONLY_FOR_KINDS.is_in(m) && m.contains("run_as")), "{p:?}");
    }

    /// `lock` is a name (exclusive, as before) or `{ name, shared }`.
    #[test]
    fn a_lock_is_a_name_or_a_name_that_is_shared() {
        let job = "\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";
        let net = parse(&format!("format = 1\nlock = \"販売DB\"{job}")).unwrap();
        assert_eq!((net.lock.as_deref(), net.lock_shared), (Some("販売DB"), false));
        let net = parse(&format!("format = 1\nlock = {{ name = \"販売DB\", shared = true }}{job}")).unwrap();
        assert_eq!((net.lock.as_deref(), net.lock_shared), (Some("販売DB"), true));
        let net = parse(&format!("format = 1\nlock = {{ name = \"販売DB\" }}{job}")).unwrap();
        assert_eq!((net.lock.as_deref(), net.lock_shared), (Some("販売DB"), false));
        let net = parse(&format!("format = 1{job}")).unwrap();
        assert_eq!((net.lock, net.lock_shared), (None, false));
        for (bad, want) in [
            ("lock = \"a/b\"", msg::DEF_LOCK_NAME_BAD),
            ("lock = 3", msg::DEF_LOCK_SHAPE),
            ("lock = { shared = true }", msg::DEF_SUBKEY_MISSING),
            ("lock = { name = \"a/b\", shared = true }", msg::DEF_LOCK_NAME_BAD),
            ("lock = { name = \"db\", shared = \"yes\" }", msg::DEF_LOCK_SHARED_BAD),
            ("lock = { name = \"db\", count = 3 }", msg::DEF_SUBKEY_UNKNOWN),
        ] {
            let p = problems(&format!("format = 1\n{bad}{job}"));
            assert!(p.len() == 1 && want.is_in(&p[0]), "{bad}: {p:?}");
        }
    }

    #[test]
    fn date_tokens_are_checked_in_run_script_and_dir() {
        let head = "format = 1\n[[job]]\nid = \"a\"\n";
        assert!(
            problems(&format!("{head}run = [\"x.exe\", \"{{month-1}}\", \"{{date-1bd:%Y%m%d}}\"]\ndir = 'D:\\{{date:%Y}}'\n")).is_empty()
        );
        let p = problems(&format!("{head}run = [\"x.exe\", \"{{date-1x}}\"]\n"));
        assert!(p.len() == 1 && msg::DEF_TEMPLATE_BAD.is_in(&p[0]) && p[0].contains("{date-1x} の単位 x を読み込めません"), "{p:?}");
        let p = problems(&format!("{head}shell = \"powershell\"\nscript = \"Get-Date -f {{date:%Q}}\"\n"));
        assert!(
            p.len() == 1 && msg::DEF_TEMPLATE_BAD.is_in(&p[0]) && p[0].contains("script") && p[0].contains("の書式 %Q を読み込めません"),
            "{p:?}"
        );
    }

    #[test]
    fn run_must_be_a_non_empty_array_of_strings() {
        let head = "format = 1\n[[job]]\nid = \"a\"\n";
        assert_eq!(problems(head), [placed(msg::line(msg::DEF_RUN_MISSING, &[]), 2)]);
        assert_eq!(problems(&format!("{head}run = []\n")), [placed(msg::line(msg::DEF_VALUE_EMPTY, &["run"]), 4)]);
        assert!(problems(&format!("{head}run = \"x.exe --a\"\n"))[0].starts_with(&msg::DEF_ARRAY_BAD.code()));
        assert_eq!(problems(&format!("{head}run = [\"\"]\n")), [placed(msg::line(msg::DEF_RUN_PROGRAM_EMPTY, &[]), 4)]);
        assert_eq!(
            problems(&format!("{head}run = [\"x\", 1]\n")),
            [placed(msg::line(msg::DEF_ARRAY_BAD, &["run", "run = ['C:\\batch\\a.exe', '--x']"]), 4)]
        );
    }

    #[test]
    fn after_must_point_at_an_id_and_not_loop() {
        let t = |after_a: &str| {
            format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n{after_a}\n[[job]]\nid = \"b\"\nrun = [\"x\"]\nafter = [\"a\"]\n")
        };
        assert!(problems(&t("")).is_empty());
        assert_eq!(problems(&t("after = [\"zzz\"]")), [placed(msg::line(msg::DEF_AFTER_UNKNOWN_ID, &["zzz"]), 5)]);
        assert_eq!(problems(&t("after = [\"b\"]")), [placed(msg::line(msg::DEF_AFTER_CYCLE, &["a -> b -> a"]), 2)]);
        assert_eq!(problems(&t("after = [\"a\"]")), [placed(msg::line(msg::DEF_AFTER_CYCLE, &["a -> a"]), 2)]);
    }

    #[test]
    fn a_repeated_id_is_named_at_its_second_use() {
        let text = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n";
        assert_eq!(problems(text), [placed(msg::line(msg::DEF_ID_DUPLICATE, &["a"]), 6)]);
    }

    #[test]
    fn a_timeout_is_kept_in_seconds_and_a_bare_number_is_refused() {
        let net = parse(
            "format = 1
[[job]]
id = \"a\"
run = [\"x\"]
timeout = \"1h30m\"
",
        )
        .unwrap();
        assert_eq!(net.jobs[0].timeout, Some(5400));
        let net = parse(
            "format = 1
[[job]]
id = \"a\"
run = [\"x\"]
",
        )
        .unwrap();
        assert_eq!(net.jobs[0].timeout, None);
        let text = "format = 1
[[job]]
id = \"a\"
run = [\"x\"]
timeout = 30
";
        assert_eq!(problems(text), [placed(msg::line(msg::DEF_STRING_BAD_E, &["timeout", "\"30m\""]), 5)]);
        let text = "format = 1
[[job]]
id = \"a\"
run = [\"x\"]
timeout = \"0m\"
";
        let p = problems(text);
        assert!(
            p.len() == 1
                && msg::DEF_VALUE_UNREADABLE.is_in(&p[0])
                && p[0].contains("timeout")
                && p[0].ends_with("（a.toml の 5 行目）")
                && p[0].contains(msg::uncoded(&msg::line(msg::DURATION_ZERO, &["0m"]))),
            "{p:?}"
        );
        let text = "format = 1
[[job]]
id = \"a\"
run = [\"x\"]
timeout = \"30\"
";
        let p = problems(text);
        assert!(
            p.len() == 1
                && msg::DEF_VALUE_UNREADABLE.is_in(&p[0])
                && p[0].contains("timeout")
                && p[0].ends_with("（a.toml の 5 行目）")
                && p[0].contains("時間 30 を読み込めません"),
            "{p:?}"
        );
    }

    #[test]
    fn missed_and_overlap_read_and_are_skip_when_not_written() {
        let net = parse("format = 1\n[[job]]\nid = \"a\"\nshell = \"powershell\"\nscript = '''\nGet-Date\nexit 1'''\n").unwrap();
        assert_eq!((net.missed, net.overlap), (Missed::Skip, Overlap::Skip));
        let net = parse("format = 1\nmissed = \"once\"\noverlap = \"queue\"\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n").unwrap();
        assert_eq!((net.missed, net.overlap), (Missed::Once, Overlap::Queue));
    }

    #[test]
    fn a_retry_reads_its_count_and_wait_and_needs_both() {
        let job = |retry: &str| format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n{retry}\n");
        let net = parse(&job("retry = { count = 2, wait = \"5m\" }")).unwrap();
        assert_eq!(net.jobs[0].retry, Some(Retry { count: 2, wait: 300 }));
        assert_eq!(parse(&job("")).unwrap().jobs[0].retry, None);
        let shape = "{ count = 2, wait = \"5m\" }";
        assert_eq!(problems(&job("retry = 2")), [placed(msg::line(msg::DEF_TABLE_BAD, &["retry", shape]), 5)]);
        assert_eq!(problems(&job("retry = { count = 2 }")), [placed(msg::line(msg::DEF_SUBKEY_MISSING, &["retry", "wait", shape]), 5)]);
        assert_eq!(
            problems(&job("retry = { wait = \"5m\" }")),
            [placed(msg::line(msg::DEF_SUBKEY_MISSING, &["retry", "count", shape]), 5)]
        );
        assert_eq!(
            problems(&job("retry = { count = 0, wait = \"5m\" }")),
            [placed(msg::line(msg::DEF_INT_RANGE, &["retry.count", "1", "100"]), 5)]
        );
        assert_eq!(
            problems(&job("retry = { count = 2, wait = 300 }")),
            [placed(msg::line(msg::DEF_STRING_BAD_E, &["retry.wait", "\"5m\""]), 5)]
        );
        let p = problems(&job("retry = { count = 2, wait = \"0s\" }"));
        assert!(
            p.len() == 1 && msg::DEF_VALUE_UNREADABLE.is_in(&p[0]) && p[0].contains("retry.wait") && p[0].ends_with("（a.toml の 5 行目）"),
            "{p:?}"
        );
        assert_eq!(
            problems(&job("retry = { count = 2, wait = \"5m\", max = 1 }")),
            [placed(msg::line(msg::DEF_SUBKEY_UNKNOWN, &["retry", "max", "count, wait"]), 5)]
        );
    }

    #[test]
    fn a_judge_may_look_at_the_output_and_a_file() {
        let job = |judge: &str| {
            format!(
                "format = 1
[[job]]
id = \"a\"
run = [\"x\"]
{judge}
"
            )
        };
        let g =
            parse(&job("judge = { error_output = \"ERROR\", ok_file = \"C:/out/{date}.csv\" }")).unwrap().jobs[0].judge.clone().unwrap();
        assert_eq!((g.error_output.as_deref(), g.ok_file.as_deref()), (Some("ERROR"), Some("C:/out/{date}.csv")));
        {
            let s = ToString::to_string(&(problems(&job("judge = { error_output = \"(\" }"))[0]));
            assert!(msg::DEF_REGEX_BAD.is_in(&s) && s.contains("judge.error_output") && s.contains("正規表現"), "{s}");
        }
        assert!(problems(&job("judge = { ok_file = \"{date-}\" }"))[0].contains("ok_file"));
        assert!(problems(&job("judge = { ok_file = 1 }"))[0].contains("文字列"));
    }

    #[test]
    fn a_judge_reads_ranges_of_exit_codes() {
        let job = |judge: &str| format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n{judge}\n");
        let r = |from, to| Range { from, to };
        let net = parse(&job("judge = { warn = \"1\", error = \"2..\" }")).unwrap();
        assert_eq!(net.jobs[0].judge, Some(Judge { warn: vec![r(1, 1)], error: vec![r(2, i32::MAX)], ..Judge::default() }));
        let net = parse(&job("judge = { warn = \"1,3..5, ..-1\" }")).unwrap();
        let warn = vec![r(1, 1), r(3, 5), r(i32::MIN, -1)];
        assert_eq!(net.jobs[0].judge, Some(Judge { warn: warn.clone(), ..Judge::default() }));
        assert_eq!(Judge::list(&warn), "1,3..5,..-1");
        assert_eq!(Judge::list(&[r(i32::MIN, i32::MAX), r(2, i32::MAX)]), "..,2..");
        assert_eq!(parse(&job("judge = { error = 1 }")).unwrap().jobs[0].judge, Some(Judge { error: vec![r(1, 1)], ..Judge::default() }));
        assert_eq!(parse(&job("")).unwrap().jobs[0].judge, None);
        assert_eq!(
            problems(&job("judge = \"1\"")),
            [placed(msg::line(msg::DEF_TABLE_BAD, &["judge", "{ warn = \"1\", error = \"2..\" }"]), 5)]
        );
        assert_eq!(problems(&job("judge = {}")), [placed(msg::line(msg::DEF_JUDGE_EMPTY, &[]), 5)]);
        assert_eq!(
            problems(&job("judge = { ok = \"0\" }")),
            [placed(msg::line(msg::DEF_SUBKEY_UNKNOWN, &["judge", "ok", "warn, error, error_output, ok_file"]), 5)]
        );
        assert_eq!(problems(&job("judge = { warn = true }")), [placed(msg::line(msg::DEF_STRING_BAD_E, &["judge.warn", "\"2..\""]), 5)]);
        assert_eq!(
            problems(&job("judge = { warn = \"\" }")),
            [placed(msg::line(msg::DEF_VALUE_UNREADABLE, &["judge.warn", msg::uncoded(&msg::line(msg::DEF_RANGE_EMPTY, &[]))]), 5)]
        );
        assert_eq!(
            problems(&job("judge = { warn = \"x\" }")),
            [placed(msg::line(msg::DEF_VALUE_UNREADABLE, &["judge.warn", msg::uncoded(&msg::line(msg::DEF_RANGE_CODE_BAD, &["x"]))]), 5)]
        );
        assert_eq!(
            problems(&job("judge = { error = \"5..2\" }")),
            [placed(
                msg::line(msg::DEF_VALUE_UNREADABLE, &["judge.error", msg::uncoded(&msg::line(msg::DEF_RANGE_REVERSED, &["5..2"]))]),
                5
            )]
        );
    }

    #[test]
    fn a_shell_job_has_a_script_and_no_run() {
        let net = parse(
            "format = 1
[[job]]
id = \"a\"
shell = \"powershell\"
script = '''
Get-Date
exit 1'''
",
        )
        .unwrap();
        let j = &net.jobs[0];
        assert_eq!((j.shell.as_deref(), j.script.as_deref(), j.run.as_slice()), (Some("powershell"), Some("Get-Date\nexit 1"), &[][..]));
        let p = |text: &str| problems(&format!("format = 1\n[[job]]\nid = \"a\"\n{text}\n"));
        assert_eq!(p("shell = \"powershell\""), [placed(msg::line(msg::DEF_SHELL_NEEDS_SCRIPT, &[]), 4)]);
        assert_eq!(p("shell = \"powershell\"\nscript = \" \""), [placed(msg::line(msg::DEF_VALUE_EMPTY, &["script"]), 4)]);
        assert_eq!(p("shell = \"cmd\"\nscript = \"x\""), [placed(msg::line(msg::DEF_CHOICE_BAD, &["shell", "\"powershell\""]), 4)]);
        assert_eq!(p("run = [\"x\"]\nscript = \"x\""), [placed(msg::line(msg::DEF_SCRIPT_NEEDS_SHELL, &[]), 5)]);
        assert_eq!(p("run = [\"x\"]\nshell = \"powershell\"\nscript = \"x\""), [placed(msg::line(msg::DEF_RUN_SHELL_BOTH, &[]), 4)]);
        assert_eq!(p("shell = \"powershell\"\nscript = 1"), [placed(msg::line(msg::DEF_STRING_BAD, &["script"]), 5)]);
    }

    #[test]
    fn encoding_says_how_the_output_is_read() {
        let enc = |text: &str| parse(&format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n{text}\n")).unwrap().jobs[0].encoding;
        assert_eq!(enc(""), Encoding::Auto);
        assert_eq!(enc("encoding = \"sjis\""), Encoding::Sjis);
        assert_eq!(enc("encoding = \"utf8\""), Encoding::Utf8);
        assert_eq!(enc("encoding = \"auto\""), Encoding::Auto);
        let p = |text: &str| problems(&format!("format = 1\n[[job]]\nid = \"a\"\n{text}\n"));
        assert_eq!(
            p("run = [\"x\"]\nencoding = \"cp932\""),
            [placed(msg::line(msg::DEF_CHOICE_BAD, &["encoding", "\"auto\", \"sjis\", \"utf8\""]), 5)]
        );
        assert!(p("kind = \"notify\"\nencoding = \"sjis\"")
            .contains(&placed(msg::line(msg::DEF_ONLY_FOR_KINDS, &["encoding", "\"job\", \"loop\""]), 5)));
    }

    #[test]
    fn the_top_level_fields_read() {
        assert_eq!(parse("format = 1\ncalendar = \"会社\"\n").unwrap().calendar.as_deref(), Some("会社"));
        assert_eq!(parse("format = 1\n").unwrap().calendar, None);
        let bad = placed(msg::line(msg::DEF_CALENDAR_NAME_BAD, &[]), 2);
        assert_eq!(problems("format = 1\ncalendar = \"a/b\"\n"), std::slice::from_ref(&bad));
        assert_eq!(problems("format = 1\ncalendar = 1\n"), [bad]);
        assert!(problems("format = 1\nmissed = \"skip\"\noverlap = \"skip\"\n").is_empty());
        assert_eq!(
            problems("format = 1\nmissed = \"later\"\n"),
            [placed(msg::line(msg::DEF_CHOICE_BAD, &["missed", "\"skip\", \"once\""]), 2)]
        );
        assert_eq!(problems("format = 1\noverlap = 1\n"), [placed(msg::line(msg::DEF_CHOICE_BAD, &["overlap", "\"skip\", \"queue\""]), 2)]);
    }

    #[test]
    fn notices_go_to_the_places_a_net_or_a_part_names() {
        let text = "format = 1
notify_to = [\"経理\", \"ops\"]

[[job]]
id = \"a\"
run = [\"x\"]

[[job]]
id = \"n\"
kind = \"notify\"
after = [\"a\"]
to = [\"夜間\"]
";
        let n = parse(text).unwrap();
        assert_eq!(n.notify_to, ["経理", "ops"]);
        assert_eq!(n.jobs[1].to, ["夜間"]);
        assert_eq!(parse(&crate::write::text(&n)).unwrap(), n);
        assert!(parse("format = 1\n").unwrap().notify_to.is_empty());
        let shape = placed(msg::line(msg::DEF_ARRAY_BAD, &["notify_to", "notify_to = [\"accounting\"]"]), 2);
        assert_eq!(problems("format = 1\nnotify_to = \"経理\"\n"), std::slice::from_ref(&shape));
        assert_eq!(problems("format = 1\nnotify_to = [\"\"]\n"), [shape]);
        assert_eq!(
            problems("format = 1\nnotify_to = [\"a\", \"a\"]\n"),
            [placed(msg::line(msg::DEF_NAMES_DUPLICATE, &["notify_to", "a"]), 2)]
        );
        assert_eq!(
            problems("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\nto = [\"c\"]\n"),
            [placed(msg::line(msg::DEF_ONLY_FOR_KINDS, &["to", "\"notify\", \"mail\""]), 5)]
        );
    }

    /// `takeover = "rerun"` is read and written back; not written, the standby leaves the run.
    #[test]
    fn a_net_says_whether_the_standby_runs_it_again() {
        let text = "format = 1\ntakeover = \"rerun\"\n\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n";
        let n = parse(text).unwrap();
        assert_eq!(n.takeover, Takeover::Rerun);
        assert_eq!(crate::write::text(&n), text);
        assert_eq!(parse("format = 1\n").unwrap().takeover, Takeover::Leave);
        assert_eq!(parse("format = 1\ntakeover = \"leave\"\n").unwrap().takeover, Takeover::Leave);
        assert_eq!(
            problems("format = 1\ntakeover = \"yes\"\n"),
            [placed(msg::line(msg::DEF_CHOICE_BAD, &["takeover", "\"leave\", \"rerun\""]), 2)]
        );
    }

    const FLOW: &str = r#"format = 1
[[job]]
id = "a"
run = ["x"]
[[job]]
id = "b"
run = ["y"]
after = ["a"]
[[job]]
id = "oops"
kind = "notify"
after = [{ job = "a", on = "error" }, { job = "b", on = "error" }]
[[job]]
id = "end"
run = ["z"]
after = [{ job = "b", on = "any" }]
"#;

    /// `after` takes the id alone (ok), `{ job, on = "ok" | "error" | "any" }`, and `{ job, code }` from a branch.
    #[test]
    fn each_kind_of_line_reads() {
        let net = parse(FLOW).unwrap();
        assert_eq!(net.jobs[1].after, [Line::ok("a")]);
        assert_eq!(net.jobs[2].part, Part::Notify);
        assert_eq!(net.jobs[2].after, [Line { job: "a".into(), on: On::Error }, Line { job: "b".into(), on: On::Error }]);
        assert_eq!(net.jobs[3].after, [Line { job: "b".into(), on: On::Any }]);
        assert_eq!(net.jobs[2].after[0].text(), "a (error)");
        let branch = "format = 1
[[job]]
id = \"chk\"
run = [\"x\"]
[[job]]
id = \"way\"
kind = \"branch\"
after = [\"chk\"]
[[job]]
id = \"one\"
run = [\"y\"]
after = [{ job = \"way\", code = \"1..3\" }]
[[job]]
id = \"rest\"
run = [\"z\"]
after = [{ job = \"way\", code = \"else\" }]
";
        let net = parse(branch).unwrap();
        assert_eq!(net.jobs[1].part, Part::Branch);
        assert_eq!(net.jobs[2].after, [Line { job: "way".into(), on: On::Code(ranges("1..3").unwrap()) }]);
        assert_eq!(net.jobs[3].after, [Line { job: "way".into(), on: On::Else }]);
        assert_eq!(net.jobs[2].after[0].text(), "way (code 1..3)");
        // The lines keep their shape through TOML (the history writes them back).
        let text = toml::to_string(&net).unwrap();
        assert!(text.contains("after = [\"chk\"]"), "{text}");
        assert!(text.contains("code = \"1..3\""), "{text}");
        assert_eq!(toml::from_str::<Jobnet>(&text).unwrap(), net);
        let p = |l: &str| {
            problems(&format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"b\"\nrun = [\"y\"]\nafter = [{l}]\n"))
        };
        assert_eq!(
            p("{ job = \"a\", on = \"bad\" }"),
            [placed(msg::line(msg::DEF_CHOICE_BAD, &["after.on", "\"ok\", \"error\", \"any\""]), 8)]
        );
        assert_eq!(p("{ job = \"a\", on = \"ok\", code = \"1\" }"), [placed(msg::line(msg::DEF_LINE_BOTH, &[]), 8)]);
        assert_eq!(
            p("{ on = \"ok\" }"),
            [placed(msg::line(msg::DEF_SUBKEY_MISSING, &["after", "job", "{ job = \"extract\", on = \"error\" }"]), 8)]
        );
        assert_eq!(p("{ job = \"a\", when = 1 }"), [placed(msg::line(msg::DEF_SUBKEY_UNKNOWN, &["after", "when", "job, on, code"]), 8)]);
        assert_eq!(
            p("{ job = \"a\", code = \"x\" }"),
            [placed(msg::line(msg::DEF_VALUE_UNREADABLE, &["after.code", msg::uncoded(&msg::line(msg::DEF_RANGE_CODE_BAD, &["x"]))]), 8)]
        );
        assert_eq!(p("{ job = \"a\", code = \"1\" }"), [placed(msg::line(msg::DEF_CODE_LINE_NOT_BRANCH, &["a"]), 8)]);
        assert_eq!(p("1"), [placed(msg::line(msg::DEF_AFTER_ELEMENT_BAD, &[]), 8)]);
    }

    /// each kind of part reads with its own fields, and refuses a job's fields.
    #[test]
    fn the_chores_read_and_what_would_harm_is_refused() {
        let net = |kind: &str, extra: &str| {
            parse(&format!(
                "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"p\"\nkind = \"{kind}\"\n{extra}after = [\"a\"]\n"
            ))
            .map(|n| (n.jobs[1].part.clone(), n.jobs[1].retry))
            .map_err(|p| p.iter().map(|p| p.in_file("a.toml")).collect::<Vec<_>>())
        };
        assert_eq!(
            net("delete", "path = 'D:\\logs'\nmatch = \"*.log\"\nkeep_days = 30\n"),
            Ok((Part::Chore(Chore::Delete { path: "D:\\logs".into(), pattern: Some("*.log".into()), keep_days: 30 }), None))
        );
        assert_eq!(
            net("copy", "from = 'D:\\out'\ninto = '\\\\nas\\share\\{date}'\nretry = { count = 2, wait = \"1m\" }\n"),
            Ok((
                Part::Chore(Chore::Copy { from: "D:\\out".into(), into: "\\\\nas\\share\\{date}".into(), pattern: None }),
                Some(Retry { count: 2, wait: 60 })
            ))
        );
        assert_eq!(
            net("zip", "from = 'a'\ninto = 'b.zip'\n").map(|p| p.0),
            Ok(Part::Chore(Chore::Zip { from: "a".into(), into: "b.zip".into() }))
        );
        assert_eq!(net("empty", "path = 'D:\\tmp'\n").map(|p| p.0), Ok(Part::Chore(Chore::Empty { path: "D:\\tmp".into() })));
        let bad = |kind: &str, extra: &str| net(kind, extra).unwrap_err().join("\n");
        {
            let s = ToString::to_string(&(bad("empty", "")));
            assert!(msg::DEF_JOB_KEY_REQUIRED_PLAIN.is_in(&s) && s.contains("empty"), "{s}");
        }
        assert!(msg::CHORE_NOT_ROOT.is_in(&bad("empty", "path = 'C:\\'\n")));
        assert!(bad("empty", "path = ''\n").contains("path"), "{}", bad("empty", "path = ''\n"));
        {
            let s = ToString::to_string(&(bad("delete", "path = 'D:\\logs'\n")));
            assert!(msg::DEF_JOB_KEY_REQUIRED.is_in(&s) && s.contains("keep_days"), "{s}");
        }
        {
            let s = ToString::to_string(&(bad("delete", "path = 'D:\\logs'\nkeep_days = 0\n")));
            assert!(msg::DEF_KEEP_DAYS_BAD.is_in(&s) && s.contains("keep_days"), "{s}");
        }
        assert!(bad("zip", "from = 'a'\ninto = 'b.7z'\n").contains(".zip"));
        assert!(bad("copy", "from = 'a'\ninto = 'b'\nrun = [\"x\"]\n").contains("run"));
        assert!(bad("copy", "from = 'a'\ninto = 'b'\npath = 'c'\n").contains("path"));
        assert!(bad("zip", "from = 'a'\ninto = 'b.zip'\nmatch = '*'\n").contains("match"));
        assert!(bad("empty", "path = 'a'\nencoding = \"utf8\"\n").contains("encoding"));
        assert_eq!(
            net(
                "send",
                "from = 'D:\\out\\*.csv'\ninto = \"sftp://partner@ftp.example.com/in/{date}\"\nkey = 'C:\\keys\\partner'\nhost_key = \"SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU\"\nverify = \"readback\"\n"
            )
            .map(|p| p.0),
            Ok(Part::Chore(Chore::Send {
                from: "D:\\out\\*.csv".into(),
                into: "sftp://partner@ftp.example.com/in/{date}".into(),
                key: Some("C:\\keys\\partner".into()),
                host_key: Some("SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU".into()),
                password: None,
                key_id: None,
                region: None,
                endpoint: None,
                verify: Some(Verify::Readback),
            }))
        );
        {
            let s = ToString::to_string(&(bad("send", "from = 'a'\ninto = 'b'\nverify = \"md5\"\n")));
            assert!(msg::DEF_CHOICE_BAD.is_in(&s) && s.contains("verify"), "{s}");
        }
        {
            let s = ToString::to_string(&(bad("copy", "from = 'a'\ninto = 'b'\nverify = \"sha256\"\n")));
            assert!(msg::DEF_ONLY_FOR_KINDS.is_in(&s) && s.contains("verify"), "{s}");
        }
        assert!(bad("ssh", "from = 'h'\ninto = 'ls'\nverify = \"sha256\"\n").contains("verify は kind"));
        assert_eq!(
            net("fetch", "from = '\\\\nas\\share\\out\\*.csv'\ninto = 'D:\\in'\n").map(|p| p.0),
            Ok(Part::Chore(Chore::Fetch {
                from: "\\\\nas\\share\\out\\*.csv".into(),
                into: "D:\\in".into(),
                key: None,
                host_key: None,
                password: None,
                key_id: None,
                region: None,
                endpoint: None,
                verify: None,
            }))
        );
        assert!(msg::CHORE_LOCAL_ONLY.is_in(&bad("send", "from = 'sftp://h/out'\ninto = 'D:\\in'\n")));
        assert!(msg::CHORE_LOCAL_ONLY.is_in(&bad("fetch", "from = 'sftp://h/out/*.csv'\ninto = 'sftp://h/in'\n")));
        assert!(msg::CHORE_ADDR_CHARS.is_in(&bad("send", "from = 'a'\ninto = 'sftp://-oProxyCommand=calc/in'\n")));
        assert!(bad("copy", "from = 'a'\ninto = 'b'\nkey = 'c'\n").contains("key"));
        assert!(bad("copy", "from = 'a'\ninto = 'b'\npassword = 'secret:c'\n").contains("password"));
        assert!(msg::CHORE_HOST_KEY_MISSING.is_in(&bad("send", "from = 'a'\ninto = 'sftp://h/in'\n")));
        {
            let s = ToString::to_string(&(bad("send", "from = 'a'\n")));
            assert!(msg::DEF_JOB_KEY_REQUIRED_PLAIN.is_in(&s) && s.contains("send"), "{s}");
        }
        assert_eq!(
            net(
                "fetch",
                "from = \"s3://logs/daily/*.csv\"\ninto = 'D:\\in'\nkey_id = \"secret:s3-id\"\npassword = \"secret:s3\"\nendpoint = \"http://nas:9000\"\n"
            )
            .map(|p| p.0),
            Ok(Part::Chore(Chore::Fetch {
                from: "s3://logs/daily/*.csv".into(),
                into: "D:\\in".into(),
                key: None,
                host_key: None,
                password: Some("secret:s3".into()),
                key_id: Some("secret:s3-id".into()),
                region: None,
                endpoint: Some("http://nas:9000".into()),
                verify: None,
            }))
        );
        assert!(msg::CHORE_S3_KEY_ID_MISSING
            .is_in(&bad("send", "from = 'a'\ninto = 's3://logs/in/'\npassword = 'secret:s3'\nregion = 'us-east-1'\n")));
        {
            let s = ToString::to_string(
                &(bad("send", "from = 'a'\ninto = 's3://logs/in/'\nkey_id = 'k'\npassword = 'secret:s3'\nregion = '{param:r}'\n")),
            );
            assert!(msg::DEF_TEMPLATE_BAD.is_in(&s) && s.contains("region"), "{s}");
        }
        {
            let s = ToString::to_string(&(bad("ssh", "from = 'h'\ninto = 'ls'\nkey_id = 'k'\n")));
            assert!(msg::DEF_JOB_KEY_REQUIRED.is_in(&s) && s.contains(&msg::DEF_ONLY_FOR_KINDS.code()) && s.contains("key_id"), "{s}");
        }
        assert!(bad("copy", "from = 'a'\ninto = 'b'\nregion = 'us-east-1'\n").contains("region は kind"));
        assert!(bad("copy", "from = 'a'\ninto = 'b'\nendpoint = 'http://nas:9000'\n").contains("endpoint は kind"));
        assert_eq!(
            net(
                "http",
                "url = \"https://api.example.com/days/{date}\"\nmethod = \"POST\"\nbody = '{\"ok\": true}'\nheaders = { Authorization = \"secret:api\", X-Day = \"1\" }\nexpect = [200, 201]\nretry = { count = 1, wait = \"1m\" }\n"
            ),
            Ok((
                Part::Chore(Chore::Http {
                    url: "https://api.example.com/days/{date}".into(),
                    method: "POST".into(),
                    body: Some("{\"ok\": true}".into()),
                    content_type: None,
                    headers: [("Authorization".to_owned(), "secret:api".to_owned()), ("X-Day".to_owned(), "1".to_owned())].into(),
                    expect: vec![200, 201],
                }),
                Some(Retry { count: 1, wait: 60 })
            ))
        );
        {
            let s = ToString::to_string(&(bad("http", "")));
            assert!(msg::DEF_JOB_KEY_REQUIRED.is_in(&s) && s.contains("http"), "{s}");
        }
        assert!(msg::CHORE_HTTP_URL.is_in(&bad("http", "url = 'ftp://x'\n")));
        {
            let s = ToString::to_string(&(bad("http", "url = 'https://x'\nmethod = \"get\"\n")));
            assert!(msg::DEF_CHOICE_BAD.is_in(&s) && s.contains("method"), "{s}");
        }
        assert!(msg::CHORE_HTTP_BODY_METHOD.is_in(&bad("http", "url = 'https://x'\nbody = 'a'\n")));
        {
            let s = ToString::to_string(&(bad("http", "url = 'https://x'\nexpect = [200, 99]\n")));
            assert!(msg::DEF_EXPECT_BAD.is_in(&s) && s.contains("expect"), "{s}");
        }
        {
            let s = ToString::to_string(&(bad("http", "url = 'https://x'\nexpect = []\n")));
            assert!(msg::DEF_EXPECT_BAD.is_in(&s) && s.contains("expect"), "{s}");
        }
        assert!(msg::CHORE_HEADER_CONTENT_TYPE.is_in(&bad("http", "url = 'https://x'\nheaders = { Content-Type = \"a\" }\n")));
        {
            let s = ToString::to_string(&(bad("http", "url = 'https://x'\nheaders = { A = \"secret:\" }\n")));
            assert!(msg::DEF_SECRET_NAME_EMPTY.is_in(&s) && s.contains("secret"), "{s}");
        }
        assert!(msg::CHORE_CONTENT_TYPE_NO_BODY.is_in(&bad("http", "url = 'https://x'\ncontent_type = 'text/plain'\n")));
        assert!(bad("http", "url = 'https://x'\nenv = { A = \"b\" }\n").contains("env"));
        {
            let s = ToString::to_string(&(bad("copy", "from = 'a'\ninto = 'b'\nurl = 'https://x'\n")));
            assert!(msg::DEF_ONLY_FOR_KINDS.is_in(&s) && s.contains("url"), "{s}");
        }
    }

    /// `kind = "ssh"` takes its PC, its command as written, the login of send and
    /// fetch, and a job's `judge` and `env`.
    #[test]
    fn an_ssh_reads_with_its_command_judge_and_env() {
        let parse_ssh = |extra: &str| {
            parse(&format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"p\"\nkind = \"ssh\"\n{extra}after = [\"a\"]\n"))
                .map(|n| n.jobs[1].clone())
                .map_err(|p| p.iter().map(|p| p.in_file("a.toml")).collect::<Vec<_>>().join("\n"))
        };
        let hk = "host_key = \"SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU\"\n";
        let j = parse_ssh(&format!(
            "on = \"ssh://ops@db1:2222\"\ncommand = \"\"\"\nbackup.sh --day {{date}} \"a b\"\"\"\"\n{hk}judge = {{ warn = \"1\", error_output = \"FATAL\" }}\nenv = {{ DAY = \"{{date}}\" }}\nretry = {{ count = 1, wait = \"1m\" }}\n"
        ))
        .unwrap();
        assert_eq!(
            j.part,
            Part::Chore(Chore::Ssh {
                on: "ssh://ops@db1:2222".into(),
                command: "backup.sh --day {date} \"a b\"".into(),
                key: None,
                host_key: Some("SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU".into()),
                password: None,
                max: None,
                pass: Pass::All,
            })
        );
        let judge = j.judge.unwrap();
        assert_eq!((judge.warn, judge.error_output.as_deref()), (vec![Range { from: 1, to: 1 }], Some("FATAL")));
        assert_eq!(j.env.get("DAY").map(String::as_str), Some("{date}"));
        assert_eq!(j.retry, Some(Retry { count: 1, wait: 60 }));
        let bad = |extra: &str| parse_ssh(extra).unwrap_err();
        {
            let s = ToString::to_string(&(bad(&format!("command = 'df -h'\n{hk}"))));
            assert!(msg::DEF_JOB_KEY_REQUIRED.is_in(&s) && s.contains("ssh"), "{s}");
        }
        {
            let s = ToString::to_string(&(bad(&format!("on = 'ssh://h'\n{hk}"))));
            assert!(msg::DEF_JOB_KEY_REQUIRED.is_in(&s) && s.contains("ssh"), "{s}");
        }
        assert!(bad(&format!("on = ''\ncommand = 'x'\n{hk}")).contains("on が空"));
        assert!(bad(&format!("on = 'ssh://h'\ncommand = ' '\n{hk}")).contains("command が空"));
        assert!(msg::CHORE_HOST_KEY_MISSING.is_in(&bad("on = 'ssh://h'\ncommand = 'x'\n")));
        assert!(bad("on = 'ssh://h'\ncommand = 'x'\nhost_key = 'abc'\n").contains("SHA256:"));
        assert!(msg::CHORE_ON_BAD.is_in(&bad(&format!("on = 'h'\ncommand = 'x'\n{hk}"))));
        assert!(msg::CHORE_ADDR_PATH.is_in(&bad(&format!("on = 'ssh://h/tmp'\ncommand = 'x'\n{hk}"))));
        assert!(msg::CHORE_PASSWORD_SECRET.is_in(&bad(&format!("on = 'ssh://h'\ncommand = 'x'\n{hk}password = 'hunter2'\n"))));
        assert!(bad(&format!("on = 'ssh://h'\ncommand = 'x'\n{hk}run = ['y']\n")).contains("run"));
        assert!(bad(&format!("on = 'ssh://h'\ncommand = 'x'\n{hk}encoding = 'utf8'\n")).contains("encoding"));
        assert!(bad(&format!("on = 'ssh://h'\ncommand = 'x {{date+1q}}'\n{hk}")).contains("{date+1q} の単位 q を読み込めません"));
        // A group of PCs in koyomi.toml: how many at once and when it passes.
        let fan = parse_ssh("on = 'group:web'\ncommand = 'uptime'\nmax = 4\npass = 'any'\n").unwrap();
        assert!(matches!(fan.part, Part::Chore(Chore::Ssh { max: Some(4), pass: Pass::Any, .. })));
        {
            let s = ToString::to_string(&(bad("on = 'group:web'\ncommand = 'x'\nmax = 0\n")));
            assert!(msg::DEF_INT_RANGE.is_in(&s) && s.contains("max"), "{s}");
        }
        {
            let s = ToString::to_string(&(bad("on = 'group:web'\ncommand = 'x'\nmax = 'all'\n")));
            assert!(msg::DEF_INT_RANGE.is_in(&s) && s.contains("max"), "{s}");
        }
        {
            let s = ToString::to_string(&(bad("on = 'group:web'\ncommand = 'x'\npass = 'some'\n")));
            assert!(msg::DEF_CHOICE_BAD.is_in(&s) && s.contains("pass"), "{s}");
        }
        assert!(msg::CHORE_GROUP_ONLY.is_in(&bad(&format!("on = 'ssh://h'\ncommand = 'x'\n{hk}max = 2\n"))));
        let other = parse(
            "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\non = 'ssh://h'\ncommand = 'y'\n[[job]]\nid = \"c\"\nkind = \"copy\"\nfrom = 'a'\ninto = 'b'\njudge = { warn = \"1\" }\nafter = [\"a\"]\n",
        )
        .unwrap_err()
        .iter()
        .map(|p| p.in_file("a.toml"))
        .collect::<Vec<_>>()
        .join("\n");
        for key in ["on", "command"] {
            assert!(other.contains(&msg::line(msg::DEF_ONLY_FOR_KINDS, &[key, "\"ssh\", \"winrm\""])), "{other}");
        }
        let copy = parse("format = 1\n[[job]]\nid = \"c\"\nkind = \"copy\"\nfrom = 'a'\ninto = 'b'\nmax = 2\npass = 'any'\n")
            .unwrap_err()
            .iter()
            .map(|p| p.in_file("a.toml"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(copy.contains(&msg::line(msg::DEF_ONLY_FOR_KINDS, &["max", "\"loop\", \"ssh\", \"winrm\""])), "{copy}");
        assert!(copy.contains(&msg::line(msg::DEF_ONLY_FOR_KINDS, &["pass", "\"ssh\", \"winrm\""])), "{copy}");
        assert!(other.contains(&msg::line(msg::DEF_ONLY_FOR_KINDS, &["judge", "\"job\", \"loop\", \"ssh\", \"winrm\""])), "{other}");
    }

    /// `kind = "winrm"` takes its PC with a user, its command as written, the
    /// certificate and a `secret:` password, and a job's `judge` and `env`.
    #[test]
    fn a_winrm_reads_with_its_command_certificate_judge_and_env() {
        let parse_winrm = |extra: &str| {
            parse(&format!(
                "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"w\"\nkind = \"winrm\"\n{extra}after = [\"a\"]\n"
            ))
            .map(|n| n.jobs[1].clone())
            .map_err(|p| p.iter().map(|p| p.in_file("a.toml")).collect::<Vec<_>>().join("\n"))
        };
        let cert = "3F:2A:91:0C:5B:77:E4:18:A0:6D:C2:9F:41:B8:0E:D3:57:6A:FC:21:94:8B:3E:70:CD:12:65:A9:F0:4E:B7:83";
        let login = format!("cert = \"{cert}\"\npassword = \"secret:ops\"\n");
        let j = parse_winrm(&format!(
            "on = 'winrm://CORP\\ops@app01:5986'\ncommand = 'backup.cmd {{date}} \"a b\"'\n{login}judge = {{ warn = \"1\" }}\nenv = {{ DAY = \"{{date}}\" }}\nretry = {{ count = 1, wait = \"1m\" }}\n"
        ))
        .unwrap();
        assert_eq!(
            j.part,
            Part::Chore(Chore::Winrm {
                on: "winrm://CORP\\ops@app01:5986".into(),
                command: "backup.cmd {date} \"a b\"".into(),
                cert: Some(cert.into()),
                password: Some("secret:ops".into()),
                auth: WinrmAuth::Basic,
                max: None,
                pass: Pass::All,
            })
        );
        let ntlm = parse_winrm(&format!("on = 'winrm://CORP\\ops@app01'\ncommand = 'x'\n{login}auth = \"ntlm\"\n")).unwrap();
        assert!(matches!(ntlm.part, Part::Chore(Chore::Winrm { auth: WinrmAuth::Ntlm, .. })));
        // A group's PCs may each have their own certificate in koyomi.toml.
        let fan = parse_winrm("on = 'group:app'\ncommand = 'x'\npassword = 'secret:ops'\nmax = 2\n").unwrap();
        assert!(matches!(fan.part, Part::Chore(Chore::Winrm { max: Some(2), pass: Pass::All, cert: None, .. })));
        assert_eq!(j.judge.unwrap().warn, vec![Range { from: 1, to: 1 }]);
        assert_eq!(j.env.get("DAY").map(String::as_str), Some("{date}"));
        assert_eq!(j.retry, Some(Retry { count: 1, wait: 60 }));
        let bad = |extra: &str| parse_winrm(extra).unwrap_err();
        {
            let s = ToString::to_string(&(bad(&format!("command = 'x'\n{login}"))));
            assert!(msg::DEF_JOB_KEY_REQUIRED.is_in(&s) && s.contains("winrm"), "{s}");
        }
        {
            let s = ToString::to_string(&(bad(&format!("on = 'winrm://u@h'\n{login}"))));
            assert!(msg::DEF_JOB_KEY_REQUIRED.is_in(&s) && s.contains("winrm"), "{s}");
        }
        {
            let s = ToString::to_string(&(bad(&format!("on = ''\ncommand = 'x'\n{login}"))));
            assert!(msg::DEF_ON_EMPTY.is_in(&s) && s.contains("winrm"), "{s}");
        }
        assert!(msg::CHORE_WINRM_USER.is_in(&bad(&format!("on = 'winrm://h'\ncommand = 'x'\n{login}"))));
        assert!(msg::CHORE_WINRM_CERT_MISSING.is_in(&bad("on = 'winrm://u@h'\ncommand = 'x'\npassword = 'secret:ops'\n")));
        assert!(msg::CHORE_PASSWORD_MISSING.is_in(&bad(&format!("on = 'winrm://u@h'\ncommand = 'x'\ncert = \"{cert}\"\n"))));
        {
            let s = ToString::to_string(&(bad(&format!("on = 'winrm://u@h'\ncommand = 'x'\n{login}host_key = 'SHA256:x'\n"))));
            assert!(msg::DEF_ONLY_FOR_KINDS.is_in(&s) && s.contains("host_key"), "{s}");
        }
        assert!(bad(&format!("on = 'winrm://u@h'\ncommand = 'x'\n{login}key = 'C:\\k'\n")).contains("key は kind"));
        assert!(bad(&format!("on = 'winrm://u@h'\ncommand = 'x'\n{login}encoding = 'utf8'\n")).contains("encoding"));
        assert!(bad("on = 'winrm://u@h'\ncommand = 'x'\ncert = '{date+1q}'\npassword = 'secret:ops'\n")
            .contains("{date+1q} の単位 q を読み込めません"));
        for auth in ["'kerberos'", "'NTLM'", "1"] {
            {
                let s = ToString::to_string(&(bad(&format!("on = 'winrm://u@h'\ncommand = 'x'\n{login}auth = {auth}\n"))));
                assert!(msg::DEF_CHOICE_BAD.is_in(&s) && s.contains("auth"), "{s}");
            }
        }
        let other = parse(&format!(
            "format = 1\n[[job]]\nid = \"s\"\nkind = \"ssh\"\non = 'ssh://h'\ncommand = 'y'\nhost_key = \"SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU\"\ncert = \"{cert}\"\n"
        ))
        .unwrap_err()
        .iter()
        .map(|p| p.in_file("a.toml"))
        .collect::<Vec<_>>()
        .join("\n");
        assert!(msg::DEF_ONLY_FOR_KINDS.is_in(&other) && other.contains("cert") && other.contains("\"winrm\""), "{other}");
        let other = parse("format = 1\n[[job]]\nid = \"j\"\nrun = [\"x\"]\nauth = \"ntlm\"\n").unwrap_err();
        assert!(
            other.iter().any(|p| {
                let s = p.in_file("a.toml");
                msg::DEF_ONLY_FOR_KINDS.is_in(&s) && s.contains("auth") && s.contains("\"winrm\"")
            }),
            "{other:?}"
        );
    }

    /// `kind = "mail"` takes its addresses, subject, body, files and place, and
    /// the `retry` and `timeout` of a part; not a `run_as`, `env` or `judge`.
    #[test]
    fn a_mail_reads_with_its_addresses_subject_body_and_files() {
        let parse_mail = |extra: &str| {
            parse(&format!(
                "format = 1\n[params]\nwho = \"ops@x\"\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"m\"\nkind = \"mail\"\n{extra}after = [\"a\"]\n"
            ))
            .map(|n| n.jobs[1].clone())
            .map_err(|p| p.iter().map(|p| p.in_file("a.toml")).collect::<Vec<_>>().join("\n"))
        };
        let j = parse_mail(
            "to = [' ops@example.com ', '{param:who}']\ncc = ['経理 <keiri@x>']\nsubject = '集計 {date}'\nbody = \"\"\"\n本文\n\"\"\"\nfiles = ['D:\\out\\*.csv']\nvia = ' 社内 '\nretry = { count = 2, wait = \"1m\" }\ntimeout = \"5m\"\n",
        )
        .unwrap();
        assert_eq!(
            j.part,
            Part::Chore(Chore::Mail {
                to: vec!["ops@example.com".into(), "{param:who}".into()],
                cc: vec!["経理 <keiri@x>".into()],
                subject: "集計 {date}".into(),
                body: Some("本文\n".into()),
                files: vec!["D:\\out\\*.csv".into()],
                via: Some("社内".into()),
            })
        );
        assert!(j.to.is_empty());
        assert_eq!(j.retry, Some(Retry { count: 2, wait: 60 }));
        let bad = |extra: &str| parse_mail(extra).unwrap_err();
        {
            let s = ToString::to_string(&(bad("subject = 's'\n")));
            assert!(msg::DEF_JOB_KEY_REQUIRED.is_in(&s) && s.contains("mail"), "{s}");
        }
        {
            let s = ToString::to_string(&(bad("to = ['a@x']\n")));
            assert!(msg::DEF_JOB_KEY_REQUIRED.is_in(&s) && s.contains("mail"), "{s}");
        }
        {
            let s = ToString::to_string(&(bad("to = 'a@x'\nsubject = 's'\n")));
            assert!(msg::DEF_ARRAY_BAD.is_in(&s) && s.contains("to"), "{s}");
        }
        {
            let s = ToString::to_string(&(bad("to = ['a@x', '']\nsubject = 's'\n")));
            assert!(msg::DEF_ARRAY_BAD.is_in(&s) && s.contains("to"), "{s}");
        }
        assert!(msg::CHORE_MAIL_TO_EMPTY.is_in(&bad("to = []\nsubject = 's'\n")));
        assert!(bad("to = ['a@x, b@x']\nsubject = 's'\n").contains("a@x, b@x はメールのアドレスとして"));
        assert!(bad("to = ['a@x']\ncc = ['nobody']\nsubject = 's'\n").contains("nobody はメールのアドレスとして"));
        assert!(bad("to = ['a@x']\nsubject = ' '\n").contains("subject が空"));
        assert!(bad("to = ['a@x']\nsubject = '{date+1q}'\n").contains("{date+1q} の単位 q を読み込めません"));
        {
            let s = ToString::to_string(&(bad("to = ['a@x']\nsubject = 's'\nfiles = [1]\n")));
            assert!(msg::DEF_ARRAY_BAD.is_in(&s) && s.contains("files"), "{s}");
        }
        assert!(bad("to = ['a@x']\nsubject = 's'\nvia = ''\n").contains("via が空"));
        assert!(bad("to = ['{param:nobody}']\nsubject = 's'\n").contains("nobody"));
        for (extra, name) in [("run_as = 'ops'\n", "run_as"), ("env = { A = 'b' }\n", "env"), ("judge = { warn = '1' }\n", "judge")] {
            assert!(bad(&format!("to = ['a@x']\nsubject = 's'\n{extra}")).contains(name), "{extra}");
        }
        let other = |kind: &str, extra: &str| {
            parse(&format!("format = 1\n[[job]]\nid = \"j\"\n{kind}{extra}"))
                .unwrap_err()
                .iter()
                .map(|p| p.in_file("a.toml"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        for field in ["cc = ['a@x']", "subject = 's'", "files = ['a']", "via = 'x'"] {
            let name = field.split(' ').next().unwrap();
            let s = other("run = [\"x\"]\n", &format!("{field}\n"));
            assert!(msg::DEF_ONLY_FOR_KINDS.is_in(&s) && s.contains(name) && s.contains("\"mail\""), "{field}");
        }
        assert!(other(
            "run = [\"x\"]
",
            "body = 'x'
"
        )
        .contains(&msg::line(msg::DEF_ONLY_FOR_KINDS, &["body", "\"http\", \"mail\""])));
    }

    /// `kind = "sql"` takes its database, SQL or file, out file, rows and tls,
    /// the password of a login, and the `retry`, `timeout` and `run_as` of a part;
    /// not an `env` or `judge`.
    #[test]
    fn a_sql_reads_with_its_database_sql_and_rows() {
        let parse_sql = |extra: &str| {
            parse(&format!(
                "format = 1\n[params]\nday = \"{{date}}\"\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"q\"\nkind = \"sql\"\n{extra}after = [\"a\"]\n"
            ))
            .map(|n| n.jobs[1].clone())
            .map_err(|p| p.iter().map(|p| p.in_file("a.toml")).collect::<Vec<_>>().join("\n"))
        };
        let j = parse_sql(
            "db = 'sqlserver://ops@db01:14330/売上'\npassword = ' secret:db '\nsql = \"\"\"\nselect * from t where d = '{param:day}'\n\"\"\"\nout = 'D:\\out\\{date}.csv'\nrows = '1..'\ntls = 'trust'\nretry = { count = 2, wait = \"1m\" }\ntimeout = \"5m\"\nrun_as = { user = 'CORP\\batch', password = \"secret:batch\" }\n",
        )
        .unwrap();
        assert_eq!(
            j.part,
            Part::Chore(Chore::Sql {
                db: "sqlserver://ops@db01:14330/売上".into(),
                password: Some("secret:db".into()),
                sql: Some("select * from t where d = '{param:day}'\n".into()),
                file: None,
                out: Some("D:\\out\\{date}.csv".into()),
                rows: Some("1..".into()),
                tls: SqlTls::Trust,
            })
        );
        assert_eq!(j.retry, Some(Retry { count: 2, wait: 60 }));
        assert!(j.run_as.is_some());
        let j = parse_sql("db = 'sqlite://C:\\data\\app.db'\nfile = 'D:\\sql\\clean.sql'\nrows = 0\n").unwrap();
        let Part::Chore(Chore::Sql { file, rows, tls, .. }) = &j.part else { panic!("{:?}", j.part) };
        assert_eq!((file.as_deref(), rows.as_deref(), *tls), (Some("D:\\sql\\clean.sql"), Some("0"), SqlTls::Verify));
        let bad = |extra: &str| parse_sql(extra).unwrap_err();
        {
            let s = ToString::to_string(&(bad("sql = 'select 1'\n")));
            assert!(msg::DEF_JOB_KEY_REQUIRED.is_in(&s) && s.contains("sql"), "{s}");
        }
        assert!(msg::CHORE_DB_SCHEME.is_in(&bad("db = 'mysql://x/y'\nsql = 'select 1'\n")));
        assert!(msg::CHORE_SQL_ONE_OF.is_in(&bad("db = 'sqlite://C:\\a.db'\n")));
        assert!(bad("db = 'sqlite://C:\\a.db'\nsql = ' '\n").contains("sql が空"));
        {
            let s = ToString::to_string(&(bad("db = 'sqlite://C:\\a.db'\nsql = 1\n")));
            assert!(msg::DEF_STRING_BAD.is_in(&s) && s.contains("sql"), "{s}");
        }
        assert!(bad("db = 'sqlite://C:\\a.db'\nsql = 'select {param:nobody}'\n").contains("nobody"));
        assert!(bad("db = 'sqlite://C:\\a.db'\nsql = 'select 1'\nrows = -1\n").contains("0 以上の整数"));
        assert!(msg::CHORE_ROWS.is_in(&bad("db = 'sqlite://C:\\a.db'\nsql = 'select 1'\nrows = '5..1'\n")));
        {
            let s = ToString::to_string(&(bad("db = 'sqlite://C:\\a.db'\nsql = 'select 1'\ntls = 'none'\n")));
            assert!(msg::DEF_CHOICE_BAD.is_in(&s) && s.contains("tls"), "{s}");
        }
        assert!(msg::CHORE_PASSWORD_MISSING.is_in(&bad("db = 'postgres://app@pg/app'\nsql = 'select 1'\n")));
        for (extra, name) in [("env = { A = 'b' }\n", "env"), ("judge = { warn = '1' }\n", "judge")] {
            assert!(bad(&format!("db = 'sqlite://C:\\a.db'\nsql = 'select 1'\n{extra}")).contains(name), "{extra}");
        }
        let other = |kind: &str, extra: &str| {
            parse(&format!("format = 1\n[[job]]\nid = \"j\"\n{kind}{extra}"))
                .unwrap_err()
                .iter()
                .map(|p| p.in_file("a.toml"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        for field in ["db = 'x'", "sql = 's'", "out = 'a'", "rows = '1'", "tls = 'off'"] {
            let name = field.split(' ').next().unwrap();
            let s = other("run = [\"x\"]\n", &format!("{field}\n"));
            assert!(msg::DEF_ONLY_FOR_KINDS.is_in(&s) && s.contains(name) && s.contains("\"sql\""), "{field}");
        }
        let s = other("run = [\"x\"]\n", "file = 'a'\n");
        assert!(msg::DEF_ONLY_FOR_KINDS.is_in(&s) && s.contains("file") && s.contains("\"loop\", \"sql\""), "{s}");
        let s = other("run = [\"x\"]\n", "password = 'secret:a'\n");
        assert!(msg::DEF_ONLY_FOR_KINDS.is_in(&s) && s.contains("password") && s.contains("\"sql\""), "{s}");
    }

    #[test]
    fn each_kind_of_part_reads() {
        let net = |kind: &str, extra: &str| {
            parse(&format!(
                "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"p\"\nkind = \"{kind}\"\n{extra}after = [\"a\"]\n[[job]]\nid = \"z\"\nrun = [\"y\"]\nafter = [\"p\"]\n"
            ))
            .map(|n| n.jobs[1].part.clone())
            .map_err(|p| p.iter().map(|p| p.in_file("a.toml")).collect::<Vec<_>>())
        };
        assert_eq!(net("branch", ""), Ok(Part::Branch));
        assert_eq!(net("join", ""), Ok(Part::Join { wait: Wait::All }));
        assert_eq!(net("join", "wait = \"any\"\n"), Ok(Part::Join { wait: Wait::Any }));
        assert_eq!(net("join", "wait = \"one\"\n"), Err(vec![placed(msg::line(msg::DEF_CHOICE_BAD, &["wait", "\"all\", \"any\""]), 8)]));
        assert_eq!(net("time", "at = \"06:00\"\n"), Ok(Part::Time { at: when::time("06:00").unwrap(), days: 0 }));
        assert_eq!(net("time", "at = \"06:00\"\ndays = 1\n"), Ok(Part::Time { at: when::time("06:00").unwrap(), days: 1 }));
        assert_eq!(net("time", ""), Err(vec![placed(msg::line(msg::DEF_JOB_KEY_REQUIRED, &["time", "at", "at = \"06:00\""]), 5)]));
        assert_eq!(net("time", "at = \"06:00\"\ndays = -1\n"), Err(vec![placed(msg::line(msg::DEF_INT_RANGE, &["days", "0", "366"]), 9)]));
        assert_eq!(net("file", "file = 'C:\\in\\*.csv'\n"), Ok(Part::File { file: "C:\\in\\*.csv".into(), settle: None }));
        assert_eq!(
            net("file", "file = 'C:\\in\\a.csv'\nsettle = \"10s\"\ntimeout = \"1h\"\n"),
            Ok(Part::File { file: "C:\\in\\a.csv".into(), settle: Some(10) })
        );
        assert_eq!(
            net("file", ""),
            Err(vec![placed(msg::line(msg::DEF_JOB_KEY_REQUIRED, &["file", "file", "file = 'C:\\in\\*.csv'"]), 5)])
        );
        assert_eq!(net("file", "file = \" \"\n"), Err(vec![placed(msg::line(msg::DEF_VALUE_EMPTY, &["file"]), 8)]));
        assert_eq!(
            net("file", "file = \"a\"\nsettle = 10\n"),
            Err(vec![placed(msg::line(msg::DEF_STRING_BAD_E, &["settle", "\"30m\""]), 9)])
        );
        assert_eq!(net("notify", ""), Ok(Part::Notify));
        assert_eq!(
            net("loop", "file = 'C:\\in\\*.csv'\nrun = [\"x\"]\n"),
            Ok(Part::Loop { file: Some("C:\\in\\*.csv".into()), count: None, max: None })
        );
        assert_eq!(
            net("loop", "count = 3\nrun = [\"x\"]\nretry = { count = 1, wait = \"1s\" }\ntimeout = \"1m\"\n"),
            Ok(Part::Loop { file: None, count: Some(3), max: None })
        );
        assert_eq!(net("loop", "count = 3\nmax = 2\nrun = [\"x\"]\n"), Ok(Part::Loop { file: None, count: Some(3), max: Some(2) }));
        assert_eq!(
            net("loop", "count = 3\nmax = 17\nrun = [\"x\"]\n"),
            Err(vec![placed(msg::line(msg::DEF_INT_RANGE, &["max", "1", "16"]), 9)])
        );
        assert_eq!(
            net("job", "max = 2\nrun = [\"x\"]\n"),
            Err(vec![placed(msg::line(msg::DEF_ONLY_FOR_KINDS, &["max", "\"loop\", \"ssh\", \"winrm\""]), 8)])
        );
        assert_eq!(
            net("loop", "run = [\"x\"]\n"),
            Err(vec![placed(
                msg::line(msg::DEF_EITHER_REQUIRED, &["kind = \"loop\"", "file", "count", "file = 'C:\\in\\*.csv' / count = 3"]),
                5
            )])
        );
        assert_eq!(net("loop", "file = \"a\"\ncount = 2\nrun = [\"x\"]\n"), Err(vec![placed(msg::line(msg::DEF_LOOP_BOTH, &[]), 9)]));
        assert_eq!(
            net("loop", "count = 0\nrun = [\"x\"]\n"),
            Err(vec![placed(msg::line(msg::DEF_INT_RANGE, &["count", "1", "10000"]), 8)])
        );
        assert_eq!(net("loop", "count = 2\n"), Err(vec![placed(msg::line(msg::DEF_RUN_MISSING, &[]), 5)]));
        assert_eq!(
            net("job", "count = 2\nrun = [\"x\"]\n"),
            Err(vec![placed(msg::line(msg::DEF_ONLY_FOR_KINDS, &["count", "\"loop\""]), 8)])
        );
        assert_eq!(net("jobnet", "use = \"共通/バックアップ\"\n"), Ok(Part::Jobnet { use_: "共通/バックアップ".into() }));
        assert_eq!(
            net("jobnet", ""),
            Err(vec![placed(msg::line(msg::DEF_JOB_KEY_REQUIRED, &["jobnet", "use", "use = \"common/backup\""]), 5)])
        );
        for u in ["a", "a/b/c", "/b", "a/", "../b", "a/..", "a\\b/c"] {
            assert_eq!(
                net("jobnet", &format!("use = \"{}\"\n", u.replace('\\', "\\\\"))),
                Err(vec![placed(msg::line(msg::DEF_JOBNET_REF_BAD, &["use"]), 8)]),
                "{u}"
            );
        }
        assert_eq!(
            net("branch", "run = [\"x\"]\n"),
            Err(vec![placed(msg::line(msg::DEF_ONLY_FOR_KINDS, &["run", "\"job\", \"loop\""]), 8)])
        );
        assert_eq!(
            net("branch", "timeout = \"1m\"\n"),
            Err(vec![placed(msg::line(msg::DEF_ONLY_FOR_KINDS, &["timeout", "\"job\", \"loop\", \"file\", \"delete\", \"zip\", \"copy\", \"empty\", \"send\", \"fetch\", \"http\", \"ssh\", \"winrm\", \"mail\", \"sql\""]), 8)])
        );
        assert_eq!(net("branch", "wait = \"any\"\n"), Err(vec![placed(msg::line(msg::DEF_ONLY_FOR_KINDS, &["wait", "\"join\""]), 8)]));
        assert_eq!(
            net("job", "use = \"a/b\"\nrun = [\"x\"]\n"),
            Err(vec![placed(msg::line(msg::DEF_ONLY_FOR_KINDS, &["use", "\"jobnet\""]), 8)])
        );
        assert_eq!(
            net("job", "at = \"06:00\"\nrun = [\"x\"]\n"),
            Err(vec![placed(msg::line(msg::DEF_ONLY_FOR_KINDS, &["at", "\"time\""]), 8)])
        );
        assert_eq!(
            net("step", ""),
            Err(vec![
                placed(msg::line(msg::DEF_CHOICE_BAD, &["kind", "\"job\", \"loop\", \"branch\", \"join\", \"time\", \"file\", \"notify\", \"approval\", \"jobnet\", \"delete\", \"zip\", \"copy\", \"empty\", \"send\", \"fetch\", \"http\", \"ssh\", \"winrm\", \"mail\", \"sql\""]), 7)
            ])
        );
        // A part of the flow ends at once (no process): the TOML of a run keeps its kind.
        let net = parse(&format!(
            "{FLOW}[[job]]\nid = \"w\"\nkind = \"time\"\nat = \"06:00\"\n[[job]]\nid = \"after_w\"\nrun = [\"x\"]\nafter = [\"w\"]\n"
        ))
        .unwrap();
        let text = toml::to_string(&net).unwrap();
        assert!(text.contains("kind = \"time\""), "{text}");
        assert!(!text.contains("kind = \"job\""), "{text}");
        assert_eq!(toml::from_str::<Jobnet>(&text).unwrap(), net);
    }

    /// a part nothing leads to or that leads nowhere is a mistake, and a branch takes one line in.
    #[test]
    fn an_orphan_part_is_refused() {
        let p = |body: &str| problems(&format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n{body}"));
        assert_eq!(
            p("[[job]]\nid = \"j\"\nkind = \"join\"\n"),
            [placed(msg::line(msg::DEF_JOB_NO_BEFORE, &["j"]), 5), placed(msg::line(msg::DEF_JOB_NO_NEXT, &["j"]), 5)]
        );
        assert_eq!(p("[[job]]\nid = \"t\"\nkind = \"time\"\nat = \"06:00\"\n"), [placed(msg::line(msg::DEF_JOB_NO_NEXT, &["t"]), 5)]);
        assert_eq!(p("[[job]]\nid = \"n\"\nkind = \"notify\"\n"), [placed(msg::line(msg::DEF_JOB_NO_BEFORE, &["n"]), 5)]);
        assert_eq!(
            p("[[job]]\nid = \"b\"\nkind = \"branch\"\nafter = [\"a\", \"a\"]\n[[job]]\nid = \"z\"\nrun = [\"y\"]\nafter = [\"b\"]\n"),
            [placed(msg::line(msg::DEF_BRANCH_AFTER_COUNT, &[]), 5)]
        );
        assert!(p("[[job]]\nid = \"n\"\nkind = \"jobnet\"\nuse = \"x/y\"\n").is_empty());
        assert_eq!(
            p("[[job]]\nid = \"b\"\nrun = [\"y\"]\nafter = [{ job = \"c\", on = \"any\" }]\n[[job]]\nid = \"c\"\nrun = [\"y\"]\nafter = [{ job = \"b\", on = \"error\" }]\n"),
            [placed(msg::line(msg::DEF_AFTER_CYCLE, &["b -> c -> b"]), 5)]
        );
    }

    /// a child jobnet's jobs take the part's place, under its id and name, and a loop of uses is refused.
    #[test]
    fn a_child_jobnet_expands_in_place() {
        let parent = parse(
            "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"bk\"\nname = \"控え\"\nkind = \"jobnet\"\nuse = \"共通/バックアップ\"\nafter = [\"a\"]\n[[job]]\nid = \"z\"\nrun = [\"y\"]\nafter = [\"bk\"]\n",
        )
        .unwrap();
        let child = parse("format = 1\n[[when]]\nevery = \"day\"\nat = [\"01:00\"]\n[[job]]\nid = \"c1\"\nrun = [\"c\"]\n[[job]]\nid = \"c2\"\nname = \"二\"\nrun = [\"c\"]\nafter = [\"c1\"]\n[[job]]\nid = \"c3\"\nrun = [\"c\"]\nafter = [\"c1\"]\n").unwrap();
        let lookup = |u: &str| {
            if u == "共通/バックアップ" {
                Ok(child.clone())
            } else {
                Err(msg::line(msg::CHILD_MISSING, &[u]))
            }
        };
        let net = expand(&parent, &lookup).unwrap();
        let ids: Vec<&str> = net.jobs.iter().map(|j| j.id.as_str()).collect();
        assert_eq!(ids, ["a", "bk/c1", "bk/c2", "bk/c3", "bk", "z"]);
        assert_eq!(net.jobs[1].after, [Line::ok("a")], "the child's first job takes the part's lines");
        assert_eq!(net.jobs[2].after, [Line::ok("bk/c1")]);
        assert_eq!(net.jobs[2].name.as_deref(), Some("控え/二"));
        assert_eq!(net.jobs[1].name.as_deref(), Some("控え/c1"));
        assert_eq!(net.jobs[4].part, Part::Jobnet { use_: "共通/バックアップ".into() });
        assert_eq!(net.jobs[4].after, [Line { job: "bk/c2".into(), on: On::Any }, Line { job: "bk/c3".into(), on: On::Any }]);
        assert_eq!(net.jobs[5].after, [Line::ok("bk")]);
        assert!(net.when.is_empty() && net.jobs.iter().all(|j| j.pos.is_none()));
        let missing = parse("format = 1\n[[job]]\nid = \"bk\"\nkind = \"jobnet\"\nuse = \"x/y\"\n").unwrap();
        assert_eq!(expand(&missing, &lookup).unwrap_err(), msg::line(msg::CHILD_MISSING, &["x/y"]));
        let myself = parse("format = 1\n[[job]]\nid = \"me\"\nkind = \"jobnet\"\nuse = \"a/b\"\n").unwrap();
        let loop_ = |u: &str| if u == "a/b" { Ok(myself.clone()) } else { Err(String::new()) };
        assert_eq!(expand(&myself, &loop_).unwrap_err(), msg::line(msg::DEF_CHILD_LOOP, &["a/b -> a/b"]));
    }

    /// each job goes to the folder of the net it came from, a child's (and its child's) to theirs;
    /// a relative `dir` is under it, an absolute one kept, a part that runs nothing left alone.
    #[test]
    fn place_puts_each_job_in_its_own_net_s_folder() {
        use std::path::PathBuf;
        let abs = std::env::temp_dir().display().to_string();
        let parent = parse(&format!(
            "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"b\"\nrun = [\"x\"]\ndir = \"sub\"\nafter = [\"a\"]\n\
             [[job]]\nid = \"c\"\nrun = [\"x\"]\ndir = '{abs}'\nafter = [\"b\"]\n[[job]]\nid = \"bk\"\nkind = \"jobnet\"\nuse = \"共通/控え\"\nafter = [\"c\"]\n\
             [[job]]\nid = \"t\"\nkind = \"time\"\nat = \"06:00\"\nafter = [\"bk\"]\n[[job]]\nid = \"z\"\nrun = [\"x\"]\nafter = [\"t\"]\n"
        ))
        .unwrap();
        let child = parse("format = 1\n[[job]]\nid = \"c1\"\nrun = [\"c\"]\n[[job]]\nid = \"in\"\nkind = \"jobnet\"\nuse = \"共通/奥\"\nafter = [\"c1\"]\n").unwrap();
        let inner = parse("format = 1\n[[job]]\nid = \"i1\"\nrun = [\"c\"]\ndir = \"logs\"\n").unwrap();
        let lookup = |u: &str| match u {
            "共通/控え" => Ok(child.clone()),
            "共通/奥" => Ok(inner.clone()),
            _ => Err(msg::line(msg::CHILD_MISSING, &[u])),
        };
        let mut net = expand(&parent, &lookup).unwrap();
        let root = std::env::temp_dir().join("jobnets");
        place(&mut net, "経理/月次", &|id| Some(root.join(id)));
        let dir = |id: &str| net.jobs.iter().find(|j| j.id == id).unwrap().dir.clone().map(PathBuf::from);
        assert_eq!(dir("a"), Some(root.join("経理/月次")));
        assert_eq!(dir("b"), Some(root.join("経理/月次").join("sub")));
        assert_eq!(dir("c"), Some(PathBuf::from(&abs)));
        assert_eq!(dir("bk/c1"), Some(root.join("共通/控え")));
        assert_eq!(dir("bk/in/i1"), Some(root.join("共通/奥").join("logs")));
        assert_eq!((dir("bk"), dir("bk/in"), dir("t")), (None, None, None));
        assert_eq!(dir("z"), Some(root.join("経理/月次")));
        // No folder for the net: as written.
        let mut net = expand(&parent, &lookup).unwrap();
        place(&mut net, "経理/月次", &|_| None);
        assert_eq!(net, expand(&parent, &lookup).unwrap());
    }

    #[test]
    fn each_kind_of_when_reads() {
        let net = |when: &str| {
            parse(&format!("format = 1\n[[when]]\n{when}\n"))
                .map(|n| n.when)
                .map_err(|p| p.iter().map(|p| p.in_file("a.toml")).collect::<Vec<_>>())
        };
        let t = |s: &str| when::time(s).unwrap();
        let d = |s: &str| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        assert_eq!(net("once = 2026-11-30T21:00:00").unwrap()[0].kind, Kind::Once { at: d("2026-11-30").and_time(t("21:00")) });
        assert_eq!(net("every = \"day\"\nat = [\"02:00\", \"14:00\"]").unwrap()[0].kind, Kind::Day { at: vec![t("02:00"), t("14:00")] });
        assert_eq!(
            net("every = \"minutes\"\nn = 15\nfrom = \"09:00\"\nto = \"18:00\"\ndays = [\"mon\", \"fri\"]").unwrap()[0].kind,
            Kind::Minutes { n: 15, from: t("09:00"), to: t("18:00"), days: vec![Weekday::Mon, Weekday::Fri] }
        );
        assert_eq!(
            net("every = \"hours\"\nn = 2\nfrom = \"00:00\"\nto = \"23:59\"").unwrap()[0].kind,
            Kind::Hours { n: 2, from: t("00:00"), to: t("23:59"), days: Vec::new() }
        );
        assert_eq!(
            net("every = \"week\"\ndays = [\"tue\"]\nat = [\"08:30\"]").unwrap()[0].kind,
            Kind::Week { days: vec![Weekday::Tue], at: vec![t("08:30")] }
        );
        assert_eq!(
            net("every = \"month\"\nday = 25\nat = [\"09:00\"]").unwrap()[0].kind,
            Kind::Month { rule: MonthRule::Day { day: 25, shift: 0 }, at: vec![t("09:00")] }
        );
        assert_eq!(
            net("every = \"month\"\nday = 20\nshift = -3\nat = [\"09:00\"]").unwrap()[0].kind,
            Kind::Month { rule: MonthRule::Day { day: 20, shift: -3 }, at: vec![t("09:00")] }
        );
        assert_eq!(
            net("every = \"month\"\nbusiness_day = -1\nat = [\"21:00\"]").unwrap()[0].kind,
            Kind::Month { rule: MonthRule::BusinessDay { n: -1 }, at: vec![t("21:00")] }
        );
        let w = net("every = \"day\"\nat = [\"02:00\"]\non_closed = \"prev\"").unwrap();
        assert_eq!(w[0].on_closed, Some(OnClosed::Prev));
        assert_eq!(net("every = \"day\"\nat = [\"02:00\"]").unwrap()[0].on_closed, None);
        let needs = |body: &str| parse(&format!("format = 1\n[[when]]\n{body}\n")).unwrap().needs_calendar();
        assert!(needs("every = \"month\"\nbusiness_day = 1\nat = [\"09:00\"]"));
        assert!(needs("every = \"month\"\nday = 1\nshift = 1\nat = [\"09:00\"]"));
        assert!(needs("every = \"day\"\nat = [\"09:00\"]\non_closed = \"skip\""));
        assert!(!needs("every = \"month\"\nday = 1\nat = [\"09:00\"]"));
        assert_eq!(
            net("every = \"month\"\nday = \"last\"\nat = [\"21:00\"]").unwrap()[0].kind,
            Kind::Month { rule: MonthRule::Last, at: vec![t("21:00")] }
        );
        assert_eq!(
            net("every = \"month\"\nnth = 2\nweekday = \"tue\"\nat = [\"10:00\"]").unwrap()[0].kind,
            Kind::Month { rule: MonthRule::Nth { nth: 2, weekday: Weekday::Tue }, at: vec![t("10:00")] }
        );
        assert_eq!(
            net("every = \"year\"\ndate = \"04-01\"\nat = [\"09:00\"]").unwrap()[0].kind,
            Kind::Year { month: 4, day: 1, at: vec![t("09:00")] }
        );
        assert_eq!(
            net("dates = [2026-12-30, \"2027-01-04\"]\nat = [\"09:00\"]").unwrap()[0].kind,
            Kind::Dates { dates: vec![d("2026-12-30"), d("2027-01-04")], at: vec![t("09:00")] }
        );
        let w = &net("every = \"day\"\nat = [\"02:00\"]\nexcept = [2026-12-31]\nstart = 2026-11-01\nend = \"2027-03-31\"").unwrap()[0];
        assert_eq!(w.except, vec![d("2026-12-31")]);
        assert_eq!((w.start, w.end), (Some(d("2026-11-01")), Some(d("2027-03-31"))));
        let two = parse("format = 1\n[[when]]\nevery = \"day\"\nat = [\"02:00\"]\n[[when]]\nonce = 2026-11-30T21:00:00\n").unwrap();
        assert_eq!(two.when.len(), 2);
        assert!(parse(GOOD).unwrap().when.is_empty());
    }

    #[test]
    fn a_when_names_its_mistake_with_its_line() {
        let p = |when: &str| problems(&format!("format = 1\n[[when]]\n{when}\n"));
        assert_eq!(p("every = \"day\""), [placed(msg::line(msg::DEF_WHEN_KEY_MISSING, &["at", "at = [\"02:00\"]"]), 2)]);
        assert_eq!(p("at = [\"02:00\"]"), [placed(msg::line(msg::DEF_WHEN_KIND_MISSING, &[]), 2)]);
        assert_eq!(
            p("every = \"daily\"\nat = [\"02:00\"]"),
            [placed(msg::line(msg::DEF_CHOICE_BAD, &["every", "\"day\", \"minutes\", \"hours\", \"week\", \"month\", \"year\""]), 3)]
        );
        let t = p("every = \"day\"\nat = [\"2 時\"]");
        assert!(t.len() == 1 && t[0].contains(&crate::msg::TIME_BAD.code()) && t[0].ends_with("（a.toml の 4 行目）"), "{t:?}");
        assert_eq!(p("every = \"day\"\nat = []"), [placed(msg::line(msg::DEF_VALUE_EMPTY, &["at"]), 4)]);
        assert_eq!(p("every = \"day\"\nat = [\"02:00\"]\nday = 3"), [placed(msg::line(msg::DEF_WHEN_KEY_UNUSED, &["day", "day"]), 5)]);
        assert_eq!(
            p("every = \"week\"\nat = [\"02:00\"]"),
            [placed(msg::line(msg::DEF_WHEN_KEY_MISSING, &["days", "days = [\"mon\", \"fri\"]"]), 2)]
        );
        let w = p("every = \"week\"\ndays = [\"火\"]\nat = [\"02:00\"]");
        assert!(w.len() == 1 && w[0].contains("曜日 \"火\" を読み込めません") && w[0].ends_with("（a.toml の 4 行目）"), "{w:?}");
        assert_eq!(
            p("every = \"minutes\"\nfrom = \"09:00\"\nto = \"10:00\""),
            [placed(msg::line(msg::DEF_WHEN_KEY_MISSING, &["n", "n = 15"]), 2)]
        );
        assert_eq!(
            p("every = \"minutes\"\nn = 0\nfrom = \"09:00\"\nto = \"10:00\""),
            [placed(msg::line(msg::DEF_INT_RANGE, &["n", "1", "1440"]), 4)]
        );
        assert_eq!(
            p("every = \"hours\"\nn = 1"),
            [
                placed(msg::line(msg::DEF_WHEN_KEY_MISSING, &["from", "from = \"09:00\""]), 2),
                placed(msg::line(msg::DEF_WHEN_KEY_MISSING, &["to", "to = \"18:00\""]), 2)
            ]
        );
        assert_eq!(p("every = \"hours\"\nn = 1\nfrom = \"18:00\"\nto = \"09:00\""), [placed(msg::line(msg::DEF_TIME_ORDER, &[]), 5)]);
        assert_eq!(p("every = \"month\"\nat = [\"09:00\"]"), [placed(msg::line(msg::DEF_MONTH_RULE_BAD, &[]), 2)]);
        assert_eq!(p("every = \"month\"\nbusiness_day = 0\nat = [\"09:00\"]"), [placed(msg::line(msg::DEF_BUSINESS_DAY_BAD, &[]), 4)]);
        assert_eq!(p("every = \"month\"\nbusiness_day = 32\nat = [\"09:00\"]"), [placed(msg::line(msg::DEF_BUSINESS_DAY_BAD, &[]), 4)]);
        assert_eq!(
            p("every = \"month\"\nbusiness_day = 1\nday = 2\nat = [\"09:00\"]"),
            [placed(msg::line(msg::DEF_BUSINESS_DAY_CONFLICT, &["day"]), 5)]
        );
        assert_eq!(
            p("every = \"month\"\nday = 20\nshift = 40\nat = [\"09:00\"]"),
            [placed(msg::line(msg::DEF_INT_RANGE, &["shift", "-31", "31"]), 5)]
        );
        assert_eq!(
            p("every = \"month\"\nnth = 2\nweekday = \"tue\"\nshift = 1\nat = [\"09:00\"]"),
            [placed(msg::line(msg::DEF_SHIFT_NEEDS_DAY, &[]), 6)]
        );
        assert_eq!(
            p("every = \"month\"\nday = \"last\"\nshift = -1\nat = [\"09:00\"]"),
            [placed(msg::line(msg::DEF_SHIFT_NEEDS_DAY, &[]), 5)]
        );
        assert_eq!(
            p("every = \"day\"\nat = [\"02:00\"]\nbusiness_day = 1"),
            [placed(msg::line(msg::DEF_WHEN_KEY_UNUSED, &["business_day", "day"]), 5)]
        );
        assert_eq!(
            p("every = \"day\"\nat = [\"02:00\"]\non_closed = \"before\""),
            [placed(msg::line(msg::DEF_CHOICE_BAD, &["on_closed", "\"prev\", \"next\", \"skip\", \"run\""]), 5)]
        );
        assert_eq!(p("every = \"month\"\nday = 32\nat = [\"09:00\"]"), [placed(msg::line(msg::DEF_DAY_BAD, &[]), 4)]);
        assert_eq!(
            p("every = \"month\"\nnth = 6\nweekday = \"tue\"\nat = [\"09:00\"]"),
            [placed(msg::line(msg::DEF_INT_RANGE, &["nth", "1", "5"]), 4)]
        );
        let d = p("every = \"year\"\ndate = \"13-01\"\nat = [\"09:00\"]");
        assert!(
            d.len() == 1 && d[0].starts_with("KOYOMI-E3391 ") && d[0].contains("13-01") && d[0].ends_with("（a.toml の 4 行目）"),
            "{d:?}"
        );
        assert_eq!(p("dates = []\nat = [\"09:00\"]"), [placed(msg::line(msg::DEF_VALUE_EMPTY, &["dates"]), 2)]);
        let d = p("dates = [\"30 日\"]\nat = [\"09:00\"]");
        assert!(
            d.len() == 1 && d[0].starts_with("KOYOMI-E3389 ") && d[0].contains("30 日") && d[0].ends_with("（a.toml の 3 行目）"),
            "{d:?}"
        );
        let o = p("once = \"今夜\"");
        assert!(o.len() == 1 && o[0].starts_with("KOYOMI-E") && o[0].contains("今夜") && o[0].ends_with("（a.toml の 3 行目）"), "{o:?}");
        assert_eq!(
            p("every = \"day\"\nat = [\"02:00\"]\nstart = 2027-01-01\nend = 2026-12-01"),
            [placed(msg::line(msg::DEF_PERIOD_ORDER, &[]), 5)]
        );
        assert_eq!(p("every = \"day\"\nat = [\"02:00\"]\nfoo = 1"), [placed(msg::line(msg::DEF_FIELD_UNKNOWN, &["foo"]), 5)]);
        for f in ["after_jobnet = \"a/b\"", "on = \"ok\""] {
            let name = f.split(' ').next().unwrap();
            assert_eq!(
                p(&format!("every = \"day\"\nat = [\"02:00\"]\n{f}")),
                [placed(msg::line(msg::DEF_WHEN_KEY_UNUSED, &[name, "day"]), 5)],
                "{f}"
            );
        }
    }

    #[test]
    fn an_after_jobnet_when_names_the_net_before_and_which_end_of_it() {
        let p = |when: &str| problems(&format!("format = 1\n[[when]]\n{when}\n"));
        let net = parse(
            "format = 1\n[[when]]\nafter_jobnet = \"経理/売上取込\"\n[[when]]\nafter_jobnet = \"経理/売上取込\"\non = \"error\"\non_closed = \"run\"\n",
        )
        .unwrap();
        assert_eq!(net.when[0].kind, Kind::AfterJobnet { jobnet: "経理/売上取込".to_owned(), on: AfterOn::Ok });
        assert!(net.when[0].is_event());
        assert_eq!(net.when[1].kind, Kind::AfterJobnet { jobnet: "経理/売上取込".to_owned(), on: AfterOn::Error });
        assert_eq!(net.when[1].on_closed, Some(OnClosed::Run));
        assert_eq!(p("after_jobnet = \"売上取込\""), [placed(msg::line(msg::DEF_JOBNET_REF_BAD, &["after_jobnet"]), 3)]);
        assert_eq!(p("after_jobnet = 1"), [placed(msg::line(msg::DEF_STRING_BAD, &["after_jobnet"]), 3)]);
        assert_eq!(
            p("after_jobnet = \"a/b\"\non = \"sometimes\""),
            [placed(msg::line(msg::DEF_CHOICE_BAD, &["on", "\"ok\", \"error\", \"any\""]), 4)]
        );
        assert_eq!(
            p("after_jobnet = \"a/b\"\nat = [\"02:00\"]"),
            [placed(msg::line(msg::DEF_WHEN_KEY_UNUSED, &["at", "after_jobnet"]), 4)]
        );
        assert_eq!(p("after_jobnet = \"a/b\"\non_closed = \"next\""), [placed(msg::line(msg::DEF_ON_CLOSED_BAD, &["after_jobnet"]), 4)]);
        assert_eq!(p("on = \"ok\""), [placed(msg::line(msg::DEF_WHEN_KIND_MISSING, &[]), 2)]);
    }

    #[test]
    fn an_event_when_is_a_file_or_the_start() {
        let p = |when: &str| problems(&format!("format = 1\n[[when]]\n{when}\n"));
        let net =
            parse("format = 1\n[[when]]\nfile = 'D:\\受信\\*.csv'\nsettle = \"10s\"\n[[when]]\non_start = true\nexcept = [2026-12-31]\n")
                .unwrap();
        assert_eq!(net.when.len(), 2);
        assert_eq!(net.when[0].kind, Kind::File { file: "D:\\受信\\*.csv".to_owned(), settle: Some(10) });
        assert!(net.when[0].is_event());
        assert_eq!(net.when[1].kind, Kind::OnStart);
        assert_eq!(net.when[1].except, [NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()]);
        // A UNC path and a plain file (no wildcard) are fine; the settle may be left out.
        let net = parse("format = 1\n[[when]]\nfile = '\\\\nas\\in\\done.flag'\n").unwrap();
        assert_eq!(net.when[0].kind, Kind::File { file: "\\\\nas\\in\\done.flag".to_owned(), settle: None });
        assert_eq!(p("file = 'in\\*.csv'"), [placed(msg::line(msg::DEF_PATH_NOT_ABSOLUTE, &["file", "file = 'D:\\recv\\*.csv'"]), 3)]);
        assert_eq!(p("file = 'D:\\受信\\'"), [placed(msg::line(msg::DEF_FILE_NAME_MISSING, &[]), 3)]);
        assert_eq!(p("file = 'D:\\*\\a.csv'"), [placed(msg::line(msg::DEF_FILE_WILDCARD_DIR, &[]), 3)]);
        assert_eq!(p("file = ''"), [placed(msg::line(msg::DEF_VALUE_EMPTY, &["file"]), 3)]);
        assert_eq!(p("file = 'D:\\in\\a.csv'\nsettle = 10"), [placed(msg::line(msg::DEF_STRING_BAD_E, &["settle", "\"30m\""]), 4)]);
        assert_eq!(p("file = 'D:\\in\\a.csv'\nat = [\"02:00\"]"), [placed(msg::line(msg::DEF_WHEN_KEY_UNUSED, &["at", "file"]), 4)]);
        assert_eq!(p("file = 'D:\\in\\a.csv'\non_closed = \"prev\""), [placed(msg::line(msg::DEF_ON_CLOSED_BAD, &["file / on_start"]), 4)]);
        assert_eq!(p("on_start = false"), [placed(msg::line(msg::DEF_ON_START_BAD, &[]), 3)]);
        assert_eq!(p("on_start = true\nsettle = \"10s\""), [placed(msg::line(msg::DEF_WHEN_KEY_UNUSED, &["settle", "on_start"]), 4)]);
        let net = parse("format = 1\n[[when]]\nidle = \"10m\"\n").unwrap();
        assert_eq!(net.when[0].kind, Kind::Idle { idle: 600 });
        assert!(net.when[0].is_event());
        assert_eq!(p("idle = \"30s\""), [placed(msg::line(msg::DEF_IDLE_SHORT, &[]), 3)]);
        assert_eq!(p("idle = 10"), [placed(msg::line(msg::DEF_STRING_BAD_E, &["idle", "\"30m\""]), 3)]);
        assert_eq!(p("idle = \"10m\"\nat = [\"02:00\"]"), [placed(msg::line(msg::DEF_WHEN_KEY_UNUSED, &["at", "idle"]), 4)]);
        assert_eq!(p("idle = \"10m\"\non_closed = \"next\""), [placed(msg::line(msg::DEF_ON_CLOSED_BAD, &["idle"]), 4)]);
        for (text, session) in
            [("sign_in", Session::SignIn), ("sign_out", Session::SignOut), ("lock", Session::Lock), ("unlock", Session::Unlock)]
        {
            let net = parse(&format!("format = 1\n[[when]]\nsession = \"{text}\"\n")).unwrap();
            assert_eq!(net.when[0].kind, Kind::Session { session, user: None });
            assert!(net.when[0].is_event());
        }
        let net = parse("format = 1\n[[when]]\nsession = \"lock\"\nuser = 'PC\\taro'\n").unwrap();
        assert_eq!(net.when[0].kind, Kind::Session { session: Session::Lock, user: Some("PC\\taro".to_owned()) });
        let net = parse("format = 1\n[[when]]\nsession = \"sign_in\"\nuser = \"taro\"\n").unwrap();
        assert_eq!(net.when[0].kind, Kind::Session { session: Session::SignIn, user: Some("taro".to_owned()) });
        let wrong = [placed(msg::line(msg::DEF_SESSION_USER_BAD, &[]), 4)];
        assert_eq!(p("session = \"lock\"\nuser = \"taro@corp.example\""), wrong);
        assert_eq!(p("session = \"lock\"\nuser = \"\""), wrong);
        assert_eq!(p("session = \"lock\"\nuser = 'PC\\'"), wrong);
        assert_eq!(p("session = \"lock\"\nuser = 1"), [placed(msg::line(msg::DEF_STRING_BAD, &["user"]), 4)]);
        assert_eq!(p("idle = \"10m\"\nuser = \"taro\""), [placed(msg::line(msg::DEF_WHEN_KEY_UNUSED, &["user", "idle"]), 4)]);
        assert_eq!(
            p("session = \"logon\""),
            [placed(msg::line(msg::DEF_CHOICE_BAD, &["session", "\"sign_in\", \"sign_out\", \"lock\", \"unlock\""]), 3)]
        );
        assert_eq!(p("session = \"lock\"\nidle = \"10m\""), [placed(msg::line(msg::DEF_WHEN_KEY_UNUSED, &["session", "idle"]), 3)]);
        assert_eq!(p("session = \"lock\"\nat = [\"02:00\"]"), [placed(msg::line(msg::DEF_WHEN_KEY_UNUSED, &["at", "session"]), 4)]);
        assert_eq!(p("session = \"unlock\"\non_closed = \"prev\""), [placed(msg::line(msg::DEF_ON_CLOSED_BAD, &["session"]), 4)]);
        assert_eq!(p("settle = \"10s\""), [placed(msg::line(msg::DEF_WHEN_KIND_MISSING, &[]), 2)]);
        for f in ["file = 'D:\\in\\a.csv'", "settle = \"10s\"", "on_start = true"] {
            let name = f.split(' ').next().unwrap();
            assert_eq!(
                p(&format!("every = \"day\"\nat = [\"02:00\"]\n{f}")),
                [placed(msg::line(msg::DEF_WHEN_KEY_UNUSED, &[name, "day"]), 5)],
                "{f}"
            );
        }
    }

    #[test]
    fn a_log_when_is_a_line_with_a_text_or_an_event_log_record() {
        let p = |when: &str| problems(&format!("format = 1\n[[when]]\n{when}\n"));
        let net = parse(
            "format = 1\n[[when]]\nlog = 'D:\\app\\app.log'\ncontains = \"ERROR\"\n\
             [[when]]\neventlog = \"Security\"\nids = [4625, 4740]\non_closed = \"run\"\n\
             [[when]]\neventlog = \"Application\"\nsource = \"MyApp\"\n",
        )
        .unwrap();
        assert_eq!(net.when[0].kind, Kind::Log { log: "D:\\app\\app.log".to_owned(), contains: "ERROR".to_owned() });
        assert_eq!(net.when[1].kind, Kind::EventLog { eventlog: "Security".to_owned(), ids: vec![4625, 4740], source: None });
        assert_eq!(
            net.when[2].kind,
            Kind::EventLog { eventlog: "Application".to_owned(), ids: Vec::new(), source: Some("MyApp".to_owned()) }
        );
        assert!(net.when.iter().all(When::is_event));
        assert_eq!(p("log = 'D:\\app\\app.log'"), [placed(msg::line(msg::DEF_WHEN_KEY_MISSING, &["contains", "contains = \"ERROR\""]), 2)]);
        assert_eq!(p("log = 'D:\\app\\app.log'\ncontains = \"\""), [placed(msg::line(msg::DEF_VALUE_EMPTY, &["contains"]), 4)]);
        assert_eq!(
            p("log = 'app.log'\ncontains = \"E\""),
            [placed(msg::line(msg::DEF_PATH_NOT_ABSOLUTE, &["log", "log = 'D:\\app\\app.log'"]), 3)]
        );
        assert_eq!(p("log = 'D:\\app\\*.log'\ncontains = \"E\""), [placed(msg::line(msg::DEF_LOG_PATH_BAD, &[]), 3)]);
        assert_eq!(p("contains = \"E\""), [placed(msg::line(msg::DEF_WHEN_KIND_MISSING, &[]), 2)]);
        assert_eq!(
            p("eventlog = \"System\""),
            [placed(msg::line(msg::DEF_EITHER_REQUIRED, &["eventlog", "ids", "source", "ids = [4625]"]), 2)]
        );
        assert_eq!(p("eventlog = \"System\"\nids = [70000]"), [placed(msg::line(msg::DEF_IDS_BAD, &[]), 4)]);
        assert_eq!(p("eventlog = \"\"\nids = [1]"), [placed(msg::line(msg::DEF_VALUE_EMPTY, &["eventlog"]), 3)]);
        assert_eq!(p("eventlog = \"System\"\nsource = \" \""), [placed(msg::line(msg::DEF_VALUE_EMPTY, &["source"]), 4)]);
        assert_eq!(
            p("eventlog = \"System\"\nids = [1]\ncontains = \"E\""),
            [placed(msg::line(msg::DEF_WHEN_KEY_UNUSED, &["contains", "eventlog"]), 5)]
        );
        assert_eq!(p("log = 'D:\\a.log'\ncontains = \"E\"\nids = [1]"), [placed(msg::line(msg::DEF_WHEN_KEY_UNUSED, &["ids", "log"]), 5)]);
        assert_eq!(
            p("log = 'D:\\a.log'\ncontains = \"E\"\non_closed = \"next\""),
            [placed(msg::line(msg::DEF_ON_CLOSED_BAD, &["log / eventlog"]), 5)]
        );
    }

    #[test]
    fn a_mail_when_is_a_mailbox_a_secret_and_a_sender_or_subject() {
        let p = |when: &str| problems(&format!("format = 1\n[[when]]\n{when}\n"));
        let net = parse(
            "format = 1\n[[when]]\nmail = \"imaps://me@example.com@imap.example.com\"\npassword = \"secret:mail\"\n\
             subject = \"請求書\"\non_closed = \"run\"\n",
        )
        .unwrap();
        assert_eq!(
            net.when[0].kind,
            Kind::Mail {
                mail: "imaps://me@example.com@imap.example.com".to_owned(),
                secret: "mail".to_owned(),
                sender: None,
                subject: Some("請求書".to_owned())
            }
        );
        assert!(net.when[0].is_event());
        let m = "mail = \"imaps://a@b\"\n";
        assert_eq!(
            p(&format!("{m}password = \"secret:m\"")),
            [placed(msg::line(msg::DEF_EITHER_REQUIRED, &["mail", "sender", "subject", "subject = \"Invoice\""]), 2)]
        );
        assert_eq!(
            p(&format!("{m}sender = \"x\"")),
            [placed(msg::line(msg::DEF_WHEN_KEY_MISSING, &["password", "password = \"secret:mailpass\""]), 2)]
        );
        assert_eq!(
            p(&format!("{m}password = \"hunter2\"\nsender = \"x\"")),
            [placed(msg::line(msg::DEF_PASSWORD_NOT_SECRET, &["password"]), 4)]
        );
        assert_eq!(
            p(&format!("{m}password = \"secret:\"\nsender = \"x\"")),
            [placed(msg::line(msg::DEF_SECRET_NAME_EMPTY, &["password"]), 4)]
        );
        assert_eq!(p(&format!("{m}password = \"secret:m\"\nsender = \"\"")), [placed(msg::line(msg::DEF_VALUE_EMPTY, &["sender"]), 5)]);
        let tls = p("mail = \"imap://a@b\"\npassword = \"secret:m\"\nsender = \"x\"");
        assert!(tls.len() == 1 && crate::msg::MAIL_NOT_TLS.is_in(&tls[0]) && tls[0].ends_with("（a.toml の 3 行目）"), "{tls:?}");
        assert_eq!(
            p(&format!("{m}password = \"secret:m\"\nsender = \"x\"\ncontains = \"E\"")),
            [placed(msg::line(msg::DEF_WHEN_KEY_UNUSED, &["contains", "mail"]), 6)]
        );
        assert_eq!(
            p(&format!("{m}password = \"secret:m\"\nsender = \"x\"\non_closed = \"prev\"")),
            [placed(msg::line(msg::DEF_ON_CLOSED_BAD, &["mail"]), 6)]
        );
        assert_eq!(
            p("every = \"day\"\nat = [\"02:00\"]\nsubject = \"x\""),
            [placed(msg::line(msg::DEF_WHEN_KEY_UNUSED, &["subject", "day"]), 5)]
        );
    }

    #[test]
    fn format_must_be_one() {
        assert_eq!(problems("name = \"x\"\n"), [placed(msg::line(msg::DEF_FORMAT_MISSING, &["1"]), 1)]);
        assert_eq!(problems("format = 2\n"), [placed(msg::line(msg::DEF_FORMAT_UNSUPPORTED, &["1"]), 1)]);
    }

    #[test]
    fn several_problems_come_in_line_order() {
        let text = "format = 1\n[[job]]\nid = \"a\"\n[[job]]\nid = \"a\"\nrun = []\n";
        assert_eq!(
            problems(text),
            [
                placed(msg::line(msg::DEF_RUN_MISSING, &[]), 2),
                placed(msg::line(msg::DEF_ID_DUPLICATE, &["a"]), 5),
                placed(msg::line(msg::DEF_VALUE_EMPTY, &["run"]), 6)
            ]
        );
    }

    #[test]
    fn line_of_counts_from_one() {
        assert_eq!(line_of("a\nb\nc", 0), 1);
        assert_eq!(line_of("a\nb\nc", 2), 2);
        assert_eq!(line_of("a\nb\nc", 99), 3);
    }

    #[test]
    fn priority_is_one_to_five_and_survives_a_round_trip() {
        let text = "format = 1
priority = 5

[[job]]
id = \"a\"
run = [\"x\"]
";
        let n = parse(text).unwrap();
        assert_eq!((n.priority, n.slot_priority()), (Some(5), 5));
        assert_eq!(parse(&crate::write::text(&n)).unwrap(), n);
        assert_eq!(
            parse(
                "format = 1
[[job]]
id = \"a\"
run = [\"x\"]
"
            )
            .unwrap()
            .slot_priority(),
            DEFAULT_PRIORITY
        );
        for bad in ["0", "6", "\"high\""] {
            let e = parse(&format!(
                "format = 1
priority = {bad}
[[job]]
id = \"a\"
run = [\"x\"]
"
            ))
            .unwrap_err();
            assert!(msg::DEF_PRIORITY_BAD.is_in(&e[0].message), "{}", e[0].message);
        }
    }

    #[test]
    fn require_reads_ac_network_and_idle_and_survives_a_round_trip() {
        let text = "format = 1
require = [\"ac\", \"network\", \"idle:10m\"]

[[job]]
id = \"a\"
run = [\"x\"]
";
        let n = parse(text).unwrap();
        assert_eq!(n.require, Require { ac: true, network: true, idle: Some(600) });
        assert_eq!(parse(&crate::write::text(&n)).unwrap(), n);
        assert!(parse(
            "format = 1
[[job]]
id = \"a\"
run = [\"x\"]
"
        )
        .unwrap()
        .require
        .is_empty());
        for bad in ["\"ac\"", "[\"wifi\"]", "[\"idle:0m\"]", "[\"idle:x\"]", "[\"idle:10分\"]", "[\"idle:400000000000000000m\"]"] {
            let e = parse(&format!(
                "format = 1
require = {bad}
[[job]]
id = \"a\"
run = [\"x\"]
"
            ))
            .unwrap_err();
            assert!(msg::DEF_REQUIRE_BAD.is_in(&e[0].message), "{}", e[0].message);
        }
    }

    #[test]
    fn an_approval_takes_a_message_and_old_forms_still_read() {
        let net = |extra: &str| {
            parse(&format!(
                "format = 1\n[[job]]\nid = \"ok\"\nkind = \"approval\"\n{extra}\n[[job]]\nid = \"b\"\nrun = [\"x\"]\nafter = [\"ok\"]\n"
            ))
        };
        let part = |n: &Jobnet| n.jobs[0].part.clone();
        assert_eq!(part(&net("").unwrap()), Part::Approval { message: None });
        let with = net("message = \"請求書の金額を確かめる\"").unwrap();
        assert_eq!(part(&with), Part::Approval { message: Some("請求書の金額を確かめる".into()) });
        assert_eq!(parse(&crate::write::text(&with)).unwrap(), with, "written back and read again");
        for bad in ["message = 3", "message = \"\"", "message = \"  \""] {
            let e = net(bad).unwrap_err();
            assert!(msg::DEF_STRING_BAD.is_in(&e[0].message) || msg::DEF_VALUE_EMPTY.is_in(&e[0].message), "{bad}: {}", e[0].message);
        }
        // Only an approval has one.
        let e = parse("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\nmessage = \"hi\"\n").unwrap_err();
        assert!(e[0].message.contains("message"), "{}", e[0].message);
        // A part stored by an older koyomi (a run's file, a history day) holds a bare `kind = "approval"`.
        let old: Part = toml::from_str("kind = \"approval\"").unwrap();
        assert_eq!(old, Part::Approval { message: None });
        let new: Part = toml::from_str("kind = \"approval\"\nmessage = \"x\"").unwrap();
        assert_eq!(new, Part::Approval { message: Some("x".into()) });
    }
}
