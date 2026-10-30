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

use crate::when::{self, AfterOn, Kind, MonthRule, OnClosed, When};

/// The `format` this version reads.
pub const FORMAT: i64 = 1;

/// A mistake in a file, at a line (1-based; 0 when it is not in the text).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub line: usize,
    pub message: String,
}

impl Problem {
    /// `月次締め.toml の 12 行目: 知らない欄 rnu`.
    pub fn in_file(&self, file: &str) -> String {
        match self.line {
            0 => format!("{file}: {}", self.message),
            n => format!("{file} の {n} 行目: {}", self.message),
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
    /// `lock = "販売DB"`: nets with the same lock name never run at the same time.
    #[serde(default)]
    pub lock: Option<String>,
    /// `wake = true`: the engine wakes a sleeping PC a little before the net's next time.
    #[serde(default)]
    pub wake: bool,
}

impl Jobnet {
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
        (Some(_), Some(_)) => return Err("after の線には on と code のどちらか 1 つを書く".to_owned()),
        (None, None) | (Some("ok"), None) => On::Ok,
        (Some("error"), None) => On::Error,
        (Some("any"), None) => On::Any,
        (Some(_), None) => return Err("after の線の on は \"ok\"・\"error\"・\"any\" のどれか".to_owned()),
        (None, Some("else")) => On::Else,
        (None, Some(c)) => On::Code(ranges(c).map_err(|e| format!("after の線の code {e}"))?),
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
    Approval,
    /// 子ジョブネット: `use = "グループ/名前"`'s jobs run here, in its place.
    Jobnet {
        #[serde(rename = "use")]
        use_: String,
    },
}

impl Part {
    pub fn is_job(&self) -> bool {
        matches!(self, Part::Job)
    }

    /// `job`, `branch` …: the `kind` as written, and the key of its word.
    pub fn key(&self) -> &'static str {
        match self {
            Part::Job => "job",
            Part::Branch => "branch",
            Part::Join { .. } => "join",
            Part::Time { .. } => "time",
            Part::File { .. } => "file",
            Part::Notify => "notify",
            Part::Approval => "approval",
            Part::Jobnet { .. } => "jobnet",
        }
    }

    /// Ends in the run itself as soon as its lines are met: no process, no clock.
    pub fn is_instant(&self) -> bool {
        matches!(self, Part::Branch | Part::Join { .. } | Part::Jobnet { .. })
    }

    /// Waits for the clock or a file: the engine polls it.
    pub fn waits(&self) -> bool {
        matches!(self, Part::Time { .. } | Part::File { .. } | Part::Approval)
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
    const HOW: &str = "は終了コードで書く（\"1\"・\"2..\"・\"1..3\" のように）";
    if text.trim().is_empty() {
        return Err("が空".to_owned());
    }
    let code = |s: &str| s.trim().parse::<i32>().map_err(|_| format!("\"{}\" {HOW}", s.trim()));
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
                    return Err(format!("\"{part}\": 始まりが終わりより大きい"));
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
    on_start: Field,
    after_jobnet: Field,
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
    // Of the design, not run by this version.
    to: Field,
    shell: Field,
    script: Field,
    timeout: Field,
    retry: Field,
    judge: Field,
    env: Field,
    encoding: Field,
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
    let mut c = Checker { text, problems: Vec::new() };
    let (missed, overlap) = c.top(&raw);
    let calendar = c.calendar(&raw.calendar);
    let lock = c.lock(&raw.lock);
    let wake = c.wake(&raw.wake);
    let jobs = c.jobs(&raw.job);
    let when = c.whens(&raw.when);
    if c.problems.is_empty() {
        Ok(Jobnet { name: c.string(&raw.name, "name"), jobs, when, missed, overlap, calendar, lock, wake })
    } else {
        c.problems.sort_by_key(|p| p.line);
        Err(c.problems)
    }
}

/// The reader's words for the toml crate's.
fn explain(message: &str, at: Option<&str>) -> String {
    if let Some(name) = message.strip_prefix("unknown field `").and_then(|r| r.split('`').next()) {
        return format!("知らない欄 {name}");
    }
    if message.starts_with("duplicate key") {
        return format!("同じ欄が 2 つある {}", at.unwrap_or("").trim()).trim_end().to_string();
    }
    format!("TOML の書き方が違う: {message}")
}

struct Checker<'a> {
    text: &'a str,
    problems: Vec<Problem>,
}

impl Checker<'_> {
    fn add(&mut self, offset: usize, message: impl Into<String>) {
        let line = line_of(self.text, offset);
        self.problems.push(Problem { line, message: message.into() });
    }

    fn not_yet(&mut self, field: &Field, name: &str) {
        if let Some(f) = field {
            self.add(f.span().start, format!("この版ではまだ使えない: {name}"));
        }
    }

    /// A field that is one of `choices` when written: `missed = "once"`.
    fn choice<'a>(&mut self, field: &Field, name: &str, choices: &[&'a str]) -> Option<&'a str> {
        let f = field.as_ref()?;
        match f.get_ref().as_str().and_then(|s| choices.iter().copied().find(|c| *c == s)) {
            Some(c) => Some(c),
            None => {
                let list = choices.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join("・");
                self.add(f.span().start, format!("{name} は {list} のどれか"));
                None
            }
        }
    }

    fn string(&mut self, field: &Field, name: &str) -> Option<String> {
        let f = field.as_ref()?;
        match f.get_ref() {
            toml::Value::String(s) => Some(s.clone()),
            _ => {
                self.add(f.span().start, format!("{name} は文字列で書く"));
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
            self.add(at, "env は表で書く（例: env = { DB_PASS = \"secret:db\" }）");
            return out;
        };
        for (k, v) in t {
            let name_ok =
                !k.is_empty() && !k.starts_with(|c: char| c.is_ascii_digit()) && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !name_ok {
                self.add(at, format!("env の名前に使えない: {k}（英数字と _ だけ）"));
            } else if k.to_ascii_uppercase().starts_with("KOYOMI_") {
                self.add(at, format!("env の名前に使えない: {k}（KOYOMI_ は koyomi が渡す）"));
            }
            let Some(s) = v.as_str() else {
                self.add(at, format!("env の値は文字列で書く: {k}"));
                continue;
            };
            match s.strip_prefix("secret:") {
                Some("") => self.add(at, format!("env {k}: secret: の後に名前が要る")),
                Some(_) => {}
                None => {
                    if let Err(e) = crate::template::check(s) {
                        self.add(at, format!("env {k}: {e}"));
                    }
                }
            }
            out.insert(k.clone(), s.to_owned());
        }
        out
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
                self.add(f.span().start, format!("{name} は \"30m\" のように文字列で書く（単位なしの数は秒か分か分からない）"));
                return None;
            }
        };
        match crate::duration::parse(&text) {
            Ok(secs) => Some(secs),
            Err(e) => {
                self.add(f.span().start, format!("{name} {e}"));
                None
            }
        }
    }

    /// A field that only some kinds of `[[job]]` may have: `wait は kind = "join" にだけ書く`.
    fn only(&mut self, field: &Field, name: &str, kinds: &str) {
        if let Some(f) = field {
            self.add(f.span().start, format!("{name} は kind = {kinds} にだけ書く"));
        }
    }

    /// `kind` and the fields that go with it, as a [`Part`].
    fn part(&mut self, j: &RawJob, start: usize) -> Part {
        let kind =
            self.choice(&j.kind, "kind", &["job", "branch", "join", "time", "file", "notify", "approval", "jobnet"]).unwrap_or("job");
        if kind != "join" {
            self.only(&j.wait, "wait", "\"join\"");
        }
        if kind != "time" {
            self.only(&j.at, "at", "\"time\"");
            self.only(&j.days, "days", "\"time\"");
        }
        if kind != "file" {
            self.only(&j.file, "file", "\"file\"");
            self.only(&j.settle, "settle", "\"file\"");
        }
        if kind != "jobnet" {
            self.only(&j.use_, "use", "\"jobnet\"");
        }
        match kind {
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
                        self.add(start, "kind = \"time\" には at が要る（例: at = \"06:00\"）");
                        None
                    }
                };
                let days = match &j.days {
                    None => 0,
                    Some(f) => match f.get_ref().as_integer() {
                        Some(n) if (0..=366).contains(&n) => n as u32,
                        _ => {
                            self.add(f.span().start, "days は 0 以上の整数で書く（何日後の at か）");
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
                            self.add(j.file.as_ref().map_or(start, |f| f.span().start), "file が空");
                        }
                        ok
                    }),
                    None => {
                        self.add(start, "kind = \"file\" には file が要る（例: file = 'C:\\in\\*.csv'）");
                        None
                    }
                };
                let settle = self.duration_of(&j.settle, "settle");
                Part::File { file: file.unwrap_or_default(), settle }
            }
            "notify" => Part::Notify,
            "approval" => Part::Approval,
            "jobnet" => {
                let use_ = match &j.use_ {
                    Some(f) => self.string(&j.use_, "use").filter(|u| {
                        let ok = is_id(u);
                        if !ok {
                            self.add(f.span().start, "use は \"グループ/名前\" で書く（例: use = \"共通/バックアップ\"）");
                        }
                        ok
                    }),
                    None => {
                        self.add(start, "kind = \"jobnet\" には use が要る（例: use = \"共通/バックアップ\"）");
                        None
                    }
                };
                Part::Jobnet { use_: use_.unwrap_or_default() }
            }
            _ => Part::Job,
        }
    }

    /// `retry = { count = 2, wait = "5m" }`: both are required.
    fn retry(&mut self, field: &Field) -> Option<Retry> {
        const SHAPE: &str = "retry は { count = 2, wait = \"5m\" } の形で書く";
        let f = field.as_ref()?;
        let at = f.span().start;
        let toml::Value::Table(t) = f.get_ref() else {
            self.add(at, SHAPE);
            return None;
        };
        let mut ok = true;
        for k in t.keys() {
            if k != "count" && k != "wait" {
                self.add(at, format!("retry に知らない欄 {k}"));
                ok = false;
            }
        }
        let count = match t.get("count") {
            None => {
                self.add(at, format!("retry に count が無い（{SHAPE}）"));
                None
            }
            Some(v) => match v.as_integer() {
                Some(n) if (1..=100).contains(&n) => Some(n as u32),
                _ => {
                    self.add(at, "retry の count は 1 から 100 の整数で書く");
                    None
                }
            },
        };
        let wait = match t.get("wait") {
            None => {
                self.add(at, format!("retry に wait が無い（{SHAPE}）"));
                None
            }
            Some(v) => match v
                .as_str()
                .ok_or_else(|| " は \"5m\" のように文字列で書く".to_owned())
                .and_then(|s| crate::duration::parse(s).map_err(|e| format!(" {e}")))
            {
                Ok(secs) => Some(secs),
                Err(e) => {
                    self.add(at, format!("retry の wait{e}"));
                    None
                }
            },
        };
        match (count, wait) {
            (Some(count), Some(wait)) if ok => Some(Retry { count, wait }),
            _ => None,
        }
    }

    fn top(&mut self, raw: &RawNet) -> (Missed, Overlap) {
        match &raw.format {
            None => self.problems.push(Problem { line: 1, message: format!("format = {FORMAT} が要る") }),
            Some(f) if f.get_ref().as_integer() != Some(FORMAT) => {
                self.add(f.span().start, format!("この版が読める format は {FORMAT} だけ"));
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
                self.add(f.span().start, "calendar は calendars/<名前>.toml の <名前>（例: calendar = \"会社\"）");
                None
            }
        }
    }

    /// `lock = "販売DB"`: a name the same rules as a calendar's (a file name, no path).
    fn lock(&mut self, field: &Field) -> Option<String> {
        let f = field.as_ref()?;
        match f.get_ref().as_str() {
            Some(s) if crate::calendar::valid_name(s) => Some(s.to_owned()),
            _ => {
                self.add(f.span().start, "lock は名前（例: lock = \"販売DB\"）。同じ名前のジョブネットは同時に動かさない");
                None
            }
        }
    }

    /// `wake = true`: wake the PC for the net's times.
    fn wake(&mut self, field: &Field) -> bool {
        let Some(f) = field.as_ref() else { return false };
        match f.get_ref().as_bool() {
            Some(b) => b,
            None => {
                self.add(f.span().start, "wake は true か false（true: PC が眠っていても起こして流す）");
                false
            }
        }
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
                        self.add(f.span().start, "every は \"day\"・\"minutes\"・\"hours\"・\"week\"・\"month\"・\"year\" のどれか");
                        continue;
                    }
                }
            } else if w.dates.is_some() {
                "dates"
            } else if w.file.is_some() {
                "file"
            } else if w.on_start.is_some() {
                "on_start"
            } else if w.after_jobnet.is_some() {
                "after_jobnet"
            } else {
                self.add(head, "[[when]] には once か every か dates か file か on_start か after_jobnet が要る");
                continue;
            };
            let allowed: &[&str] = match kind {
                "file" => &["file", "settle"],
                "on_start" => &["on_start"],
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
                (&w.on_start, "on_start"),
                (&w.after_jobnet, "after_jobnet"),
                (&w.on, "on"),
                (&w.wait_for, "wait_for"),
                (&w.wait_until, "wait_until"),
            ] {
                if let Some(f) = field {
                    if !allowed.contains(&name) {
                        self.add(f.span().start, format!("{name} は {kind} では使わない"));
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
            if matches!(kind, "file" | "on_start") && matches!(on_closed, Some(OnClosed::Prev | OnClosed::Next)) {
                let pos = w.on_closed.as_ref().map_or(head, |f| f.span().start);
                self.add(pos, "file / on_start では on_closed は \"skip\" か \"run\"");
            }
            if kind == "after_jobnet" && matches!(on_closed, Some(OnClosed::Prev | OnClosed::Next)) {
                let pos = w.on_closed.as_ref().map_or(head, |f| f.span().start);
                self.add(pos, "after_jobnet では on_closed は \"skip\" か \"run\"");
            }
            let before = self.problems.len();
            let kind = match kind {
                "once" => self.datetime(&w.once).map(|at| Kind::Once { at }),
                "file" => {
                    let file = self.event_file(&w.file);
                    let settle = self.duration_of(&w.settle, "settle");
                    file.map(|file| Kind::File { file, settle })
                }
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
                            self.add(pos, "after_jobnet は \"グループ/名前\" で書く（例: after_jobnet = \"経理/売上取込\"）");
                            None
                        }
                        None => None,
                    }
                }
                "on_start" => match w.on_start.as_ref().and_then(|f| f.get_ref().as_bool()) {
                    Some(true) => Some(Kind::OnStart),
                    _ => {
                        let pos = w.on_start.as_ref().map_or(head, |f| f.span().start);
                        self.add(pos, "on_start は true だけ（使わないなら [[when]] ごと消す）");
                        None
                    }
                },
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
                        self.add(head, "days が要る（例: days = [\"mon\", \"fri\"]）");
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
                            self.add(head, "date が要る（例: date = \"04-01\"）");
                            None
                        }
                        Some(f) => match f
                            .get_ref()
                            .as_str()
                            .ok_or_else(|| "date は \"04-01\" のように文字列で書く".to_owned())
                            .and_then(when::month_day)
                        {
                            Ok(md) => Some(md),
                            Err(e) => {
                                self.add(f.span().start, e);
                                None
                            }
                        },
                    };
                    let at = self.at(&w.at, head);
                    md.zip(at).map(|((month, day), at)| Kind::Year { month, day, at })
                }
                _ => {
                    let dates = self.dates(&w.dates, "dates");
                    if dates.is_empty() && self.problems.len() == before {
                        self.add(head, "dates が空");
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
                    self.add(pos, "wait_for は \"グループ/名前\" で書く（例: wait_for = \"経理/売上取込\"）");
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
                self.add(pos, "wait_until は wait_for と一緒に書く");
            }
            if let (Some(s), Some(e)) = (start, end) {
                if s > e {
                    self.add(w.start.as_ref().map_or(head, |f| f.span().start), "start は end より前");
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

    /// `[[when]] file`: an absolute path, `*` / `?` only in the name part.
    fn event_file(&mut self, field: &Field) -> Option<String> {
        let f = field.as_ref()?;
        let pos = f.span().start;
        let text = self.string(field, "file")?;
        if text.trim().is_empty() {
            self.add(pos, "file が空");
            return None;
        }
        // Split by either separator: the check reads the same on every OS.
        let (dir, name) = text.rsplit_once(['/', '\\']).unwrap_or(("", text.as_str()));
        let drive = text.as_bytes().get(1) == Some(&b':') && text.as_bytes().first().is_some_and(u8::is_ascii_alphabetic);
        if !(drive || text.starts_with("\\\\") || text.starts_with('/')) {
            self.add(pos, "file は絶対パスで書く（例: file = 'D:\\受信\\*.csv'）");
            return None;
        }
        if name.is_empty() {
            self.add(pos, "file はファイル名まで書く（例: file = 'D:\\受信\\*.csv'）");
            return None;
        }
        if dir.contains(['*', '?']) {
            self.add(pos, "file の * / ? はファイル名の部分だけ（フォルダーには使えない）");
            return None;
        }
        Some(text)
    }

    fn at(&mut self, field: &Field, head: usize) -> Option<Vec<NaiveTime>> {
        let Some(f) = field else {
            self.add(head, "at が要る（例: at = [\"02:00\"]）");
            return None;
        };
        let toml::Value::Array(items) = f.get_ref() else {
            self.add(f.span().start, "at は時刻の配列で書く（例: at = [\"02:00\", \"14:00\"]）");
            return None;
        };
        if items.is_empty() {
            self.add(f.span().start, "at が空");
            return None;
        }
        let mut out = Vec::new();
        for v in items {
            match v.as_str().ok_or_else(|| "at の要素は \"02:00\" のような文字列で書く".to_owned()).and_then(when::time) {
                Ok(t) => out.push(t),
                Err(e) => self.add(f.span().start, e),
            }
        }
        (out.len() == items.len()).then_some(out)
    }

    fn time_of(&mut self, f: &Spanned<toml::Value>, name: &str) -> Option<NaiveTime> {
        match f.get_ref().as_str().ok_or_else(|| format!("{name} は \"09:00\" のような文字列で書く")).and_then(when::time) {
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
            self.add(f.span().start, "days は曜日の配列で書く（例: days = [\"mon\", \"fri\"]）");
            return None;
        };
        if items.is_empty() {
            self.add(f.span().start, "days が空");
            return None;
        }
        let mut out = Vec::new();
        for v in items {
            match v.as_str().ok_or_else(|| "days の要素は \"mon\" のような文字列で書く".to_owned()).and_then(when::weekday) {
                Ok(d) => out.push(d),
                Err(e) => self.add(f.span().start, e),
            }
        }
        (out.len() == items.len()).then_some(out)
    }

    fn n(&mut self, field: &Field, head: usize) -> Option<u32> {
        let Some(f) = field else {
            self.add(head, "n が要る（例: n = 15）");
            return None;
        };
        match f.get_ref().as_integer() {
            Some(n) if (1..=1440).contains(&n) => Some(n as u32),
            _ => {
                self.add(f.span().start, "n は 1 以上の整数で書く");
                None
            }
        }
    }

    fn span_of(&mut self, from: &Field, to: &Field, head: usize) -> Option<(NaiveTime, NaiveTime)> {
        let (Some(a), Some(b)) = (from, to) else {
            self.add(head, "from と to が要る（例: from = \"09:00\", to = \"18:00\"）");
            return None;
        };
        let from = self.time_of(a, "from");
        let to = self.time_of(b, "to");
        let (from, to) = from.zip(to)?;
        if from > to {
            self.add(a.span().start, "from は to より前");
            return None;
        }
        Some((from, to))
    }

    fn month_rule(&mut self, w: &RawWhen, head: usize) -> Option<MonthRule> {
        if let Some(b) = &w.business_day {
            for (field, name) in [(&w.day, "day"), (&w.nth, "nth"), (&w.weekday, "weekday"), (&w.shift, "shift")] {
                if let Some(f) = field {
                    self.add(f.span().start, format!("{name} は business_day と一緒に使わない"));
                }
            }
            return match b.get_ref().as_integer() {
                Some(n) if (1..=31).contains(&n) || (-31..=-1).contains(&n) => Some(MonthRule::BusinessDay { n: n as i32 }),
                _ => {
                    self.add(b.span().start, "business_day は 1 から 31 か -1 から -31 の数");
                    None
                }
            };
        }
        let shift = match &w.shift {
            None => Some(0),
            Some(f) if !matches!(w.day.as_ref().map(|d| d.get_ref()), Some(toml::Value::Integer(_))) => {
                self.add(f.span().start, "shift は day の数と使う（例: day = 20, shift = -3）");
                None
            }
            Some(f) => match f.get_ref().as_integer() {
                Some(n) if (-31..=31).contains(&n) => Some(n as i32),
                _ => {
                    self.add(f.span().start, "shift は -31 から 31 の数");
                    None
                }
            },
        };
        match (&w.day, &w.nth, &w.weekday) {
            (Some(d), None, None) => match d.get_ref() {
                toml::Value::Integer(n) if (1..=31).contains(n) => shift.map(|shift| MonthRule::Day { day: *n as u32, shift }),
                toml::Value::String(s) if s == "last" => Some(MonthRule::Last),
                _ => {
                    self.add(d.span().start, "day は 1 から 31 の数か \"last\"");
                    None
                }
            },
            (None, Some(n), Some(wd)) => {
                let nth = match n.get_ref().as_integer() {
                    Some(n) if (1..=5).contains(&n) => Some(n as u32),
                    _ => {
                        self.add(n.span().start, "nth は 1 から 5 の数");
                        None
                    }
                };
                let weekday =
                    match wd.get_ref().as_str().ok_or_else(|| "weekday は \"tue\" のような文字列で書く".to_owned()).and_then(when::weekday)
                    {
                        Ok(d) => Some(d),
                        Err(e) => {
                            self.add(wd.span().start, e);
                            None
                        }
                    };
                nth.zip(weekday).map(|(nth, weekday)| MonthRule::Nth { nth, weekday })
            }
            _ => {
                self.add(
                    head,
                    "month には day か、nth と weekday か、business_day が要る（例: day = 25、day = \"last\"、nth = 2 と weekday = \"tue\"、business_day = 5）",
                );
                None
            }
        }
    }

    fn dates(&mut self, field: &Field, name: &str) -> Vec<NaiveDate> {
        let Some(f) = field else { return Vec::new() };
        let toml::Value::Array(items) = f.get_ref() else {
            self.add(f.span().start, format!("{name} は日付の配列で書く（例: {name} = [2026-12-30]）"));
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
        const SHAPE: &str = "judge は { warn = \"1\", error = \"2..\" } の形で書く";
        let f = field.as_ref()?;
        let at = f.span().start;
        let toml::Value::Table(t) = f.get_ref() else {
            self.add(at, SHAPE);
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
                        self.add(at, format!("judge の {k} は文字列で書く"));
                        ok = false;
                        continue;
                    };
                    if k == "error_output" {
                        if let Err(e) = ito_match::Matcher::new(s) {
                            self.add(at, format!("judge の error_output が正規表現として読めない: {e}"));
                            ok = false;
                        }
                        judge.error_output = Some(s.clone());
                    } else {
                        if let Err(e) = crate::template::check(s) {
                            self.add(at, format!("judge の ok_file: {e}"));
                            ok = false;
                        }
                        judge.ok_file = Some(s.clone());
                    }
                    continue;
                }
                _ => {
                    self.add(at, format!("judge に知らない欄 {k}"));
                    ok = false;
                    continue;
                }
            };
            let text = match v {
                toml::Value::String(s) => s.clone(),
                toml::Value::Integer(n) => n.to_string(),
                _ => {
                    self.add(at, format!("judge の {k} は \"1\" や \"2..\" のように文字列で書く"));
                    ok = false;
                    continue;
                }
            };
            match ranges(&text) {
                Ok(r) => *into = r,
                Err(e) => {
                    self.add(at, format!("judge の {k} {e}"));
                    ok = false;
                }
            }
        }
        if ok && judge.warn.is_empty() && judge.error.is_empty() && judge.error_output.is_none() && judge.ok_file.is_none() {
            self.add(at, format!("judge が空（{SHAPE}）"));
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
            self.not_yet(&j.to, "to");
            let part = self.part(j, start);
            let id = match &j.id {
                None => {
                    self.add(start, "id が無い");
                    String::new()
                }
                Some(f) => match f.get_ref() {
                    toml::Value::String(s) if !s.is_empty() => {
                        if !seen.insert(s.clone()) {
                            self.add(f.span().start, format!("同じ id が 2 つある: {s}"));
                        }
                        s.clone()
                    }
                    _ => {
                        self.add(f.span().start, "id は空でない文字列で書く");
                        String::new()
                    }
                },
            };
            let (run, shell, script, dir, retry, judge) = if part.is_job() {
                let shell = self.choice(&j.shell, "shell", &["powershell"]).map(str::to_owned);
                let script = self.string(&j.script, "script");
                // Either `run`, or a shell with its script: never both, never a shell line in `run`.
                let run = match (&j.shell, &j.script) {
                    (Some(s), _) => {
                        if let Some(f) = &j.run {
                            self.add(f.span().start, "run と shell は両方書けない（run か、shell と script か）");
                        }
                        if j.script.is_none() {
                            self.add(s.span().start, "shell には script が要る");
                        } else if script.as_deref().is_some_and(|t| t.trim().is_empty()) {
                            self.add(s.span().start, "script が空");
                        }
                        Vec::new()
                    }
                    (None, Some(f)) => {
                        self.add(f.span().start, "script には shell = \"powershell\" が要る");
                        self.run(&j.run, start)
                    }
                    // A refused `kind` is one problem, not two.
                    (None, None) if j.run.is_none() && j.kind.as_ref().is_some_and(|f| f.get_ref().as_str() != Some("job")) => Vec::new(),
                    (None, None) => self.run(&j.run, start),
                };
                let dir = self.string(&j.dir, "dir");
                let retry = self.retry(&j.retry);
                let judge = self.judge(&j.judge);
                (run, shell, script, dir, retry, judge)
            } else {
                for (field, name) in
                    [(&j.run, "run"), (&j.shell, "shell"), (&j.script, "script"), (&j.dir, "dir"), (&j.retry, "retry"), (&j.judge, "judge")]
                {
                    self.only(field, name, "\"job\"");
                }
                (Vec::new(), None, None, None, None, None)
            };
            for (text, name) in
                run.iter().map(|t| (t, "run")).chain(script.iter().map(|t| (t, "script"))).chain(dir.iter().map(|t| (t, "dir")))
            {
                if let Err(e) = crate::template::check(text) {
                    self.add(start, format!("{name}: {e}"));
                }
            }
            let (env, encoding) = if part.is_job() {
                let encoding = self.choice(&j.encoding, "encoding", &["auto", "sjis", "utf8"]).and_then(Encoding::of).unwrap_or_default();
                (self.env(&j.env), encoding)
            } else {
                self.only(&j.env, "env", "\"job\"");
                self.only(&j.encoding, "encoding", "\"job\"");
                (BTreeMap::new(), Encoding::Auto)
            };
            let timeout = if part.is_job() || matches!(part, Part::File { .. }) {
                self.timeout(&j.timeout)
            } else {
                self.only(&j.timeout, "timeout", "\"job\" か \"file\"");
                None
            };
            let after = self.after(&j.after);
            if part == Part::Branch && after.len() != 1 {
                self.add(start, "kind = \"branch\" には after を 1 つ書く（その終了コードで分ける）");
            }
            let name = self.string(&j.name, "name");
            let note = self.string(&j.note, "note");
            let pos = self.pos(&j.pos);
            jobs.push(Job { id, name, note, run, dir, after, pos, part, timeout, retry, shell, script, judge, env, encoding });
        }
        let followed: BTreeSet<&str> = jobs.iter().flat_map(|j| j.after.iter().map(|l| l.job.as_str())).collect();
        for (job, entry) in jobs.iter().zip(raw) {
            let at = entry.get_ref().after.as_ref().map_or(entry.span().start, |f| f.span().start);
            for l in &job.after {
                if !seen.contains(l.job.as_str()) {
                    self.add(at, format!("after が無い id を指している: {}", l.job));
                } else if matches!(l.on, On::Code(_) | On::Else) && !jobs.iter().any(|p| p.id == l.job && p.part == Part::Branch) {
                    self.add(at, format!("code の線は kind = \"branch\" の部品からだけ引ける: {}", l.job));
                }
            }
            // A part that nothing leads to, or that leads nowhere, does nothing: a mistake, not a quiet no-op.
            let (needs_after, needs_next) = match job.part {
                Part::Branch | Part::Join { .. } => (true, true),
                Part::Time { .. } | Part::File { .. } | Part::Approval => (false, true),
                Part::Notify => (true, false),
                Part::Job | Part::Jobnet { .. } => (false, false),
            };
            if needs_after && job.after.is_empty() {
                self.add(entry.span().start, format!("孤立した部品: {}（after が無い）", job.id));
            }
            if needs_next && !followed.contains(job.id.as_str()) {
                self.add(entry.span().start, format!("孤立した部品: {}（後に続くものが無い）", job.id));
            }
        }
        if let Some(cycle) = find_cycle(&jobs) {
            let first = jobs.iter().position(|j| j.id == cycle[0]).unwrap_or(0);
            self.add(raw[first].span().start, format!("after が輪になっている: {}", cycle.join(" → ")));
        }
        jobs
    }

    fn run(&mut self, field: &Field, job_start: usize) -> Vec<String> {
        let Some(f) = field else {
            self.add(job_start, "run が無い");
            return Vec::new();
        };
        let at = f.span().start;
        let toml::Value::Array(items) = f.get_ref() else {
            self.add(at, "run は文字列の配列で書く（例: run = ['C:\\batch\\a.exe', '--x']）");
            return Vec::new();
        };
        let strings: Vec<String> = items.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        if strings.len() != items.len() {
            self.add(at, "run の要素は全部文字列で書く");
        } else if strings.is_empty() {
            self.add(at, "run が空");
        } else if strings[0].trim().is_empty() {
            self.add(at, "run の先頭（実行ファイル）が空");
        }
        strings
    }

    fn after(&mut self, field: &Field) -> Vec<Line> {
        let Some(f) = field else { return Vec::new() };
        let at = f.span().start;
        let toml::Value::Array(items) = f.get_ref() else {
            self.add(at, "after は id の配列で書く（例: after = [\"extract\"]）");
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
                _ => self.add(at, "after の要素は id の文字列か { job, on } の表で書く"),
            }
        }
        out
    }

    /// One `{ job = "a", on = "error" }` / `{ job = "a", code = "1..3" }` of `after`.
    fn line(&mut self, t: &toml::Table) -> Result<Line, String> {
        for k in t.keys() {
            if !matches!(k.as_str(), "job" | "on" | "code") {
                return Err(format!("after の線に知らない欄 {k}（job・on・code）"));
            }
        }
        let job = t.get("job").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).ok_or("after の線には job（id）が要る")?.to_owned();
        let text = |k: &str| match t.get(k) {
            None => Ok(None),
            Some(toml::Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("after の線の {k} は文字列で書く")),
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
        self.add(f.span().start, "pos は [x, y] の数 2 つで書く");
        None
    }
}

/// `グループ/名前`: one `/`, both sides there, nothing that leaves the folder.
/// What `use`, `after_jobnet` and the editor's new net are checked with.
pub fn is_id(u: &str) -> bool {
    let mut parts = u.split('/');
    let (Some(g), Some(n), None) = (parts.next(), parts.next(), parts.next()) else { return false };
    let plain = |s: &str| !s.trim().is_empty() && s != "." && s != ".." && !s.contains('\\');
    plain(g) && plain(n)
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

fn expand_in(net: &Jobnet, lookup: &dyn Fn(&str) -> Result<Jobnet, String>, stack: &mut Vec<String>) -> Result<Jobnet, String> {
    let mut out = net.clone();
    out.jobs.clear();
    for j in &net.jobs {
        let Part::Jobnet { use_ } = &j.part else {
            out.jobs.push(j.clone());
            continue;
        };
        if stack.contains(use_) {
            return Err(format!("子ジョブネットの呼び合いが輪になっている: {} → {use_}", stack.join(" → ")));
        }
        let child = lookup(use_)?;
        stack.push(use_.clone());
        let child = expand_in(&child, lookup, stack)?;
        stack.pop();
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

    fn problems(text: &str) -> Vec<String> {
        match parse(text) {
            Ok(_) => Vec::new(),
            Err(p) => p.iter().map(|p| p.in_file("a.toml")).collect(),
        }
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
        assert_eq!(problems(&text), ["a.toml の 13 行目: 知らない欄 rnu"]);
    }

    #[test]
    fn toml_syntax_errors_name_a_line() {
        let p = problems("format = 1\nname = \n");
        assert_eq!(p.len(), 1);
        assert!(p[0].starts_with("a.toml の 2 行目: TOML の書き方が違う"), "{p:?}");
    }

    #[test]
    fn env_is_checked_and_kept() {
        let head = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";
        let net = parse(&format!("{head}env = {{ DB_PASS = \"secret:db\", OUT = \"{{month-1}}\" }}\n")).unwrap();
        assert_eq!(net.jobs[0].env["DB_PASS"], "secret:db");
        for (bad, want) in [
            ("env = { 1A = \"x\" }", "env の名前に使えない"),
            ("env = { KOYOMI_X = \"x\" }", "KOYOMI_"),
            ("env = { A = 1 }", "文字列"),
            ("env = { A = \"secret:\" }", "secret: の後に名前"),
            ("env = { A = \"{date-1x}\" }", "単位が読めない"),
        ] {
            let p = problems(&format!("{head}{bad}\n"));
            assert!(p.len() == 1 && p[0].contains(want), "{bad}: {p:?}");
        }
    }

    #[test]
    fn date_tokens_are_checked_in_run_script_and_dir() {
        let head = "format = 1\n[[job]]\nid = \"a\"\n";
        assert!(
            problems(&format!("{head}run = [\"x.exe\", \"{{month-1}}\", \"{{date-1bd:%Y%m%d}}\"]\ndir = 'D:\\{{date:%Y}}'\n")).is_empty()
        );
        let p = problems(&format!("{head}run = [\"x.exe\", \"{{date-1x}}\"]\n"));
        assert!(p.len() == 1 && p[0].contains("run: {date-1x}: 単位が読めない"), "{p:?}");
        let p = problems(&format!("{head}shell = \"powershell\"\nscript = \"Get-Date -f {{date:%Q}}\"\n"));
        assert!(p.len() == 1 && p[0].contains("script: ") && p[0].contains("書式が読めない"), "{p:?}");
    }

    #[test]
    fn run_must_be_a_non_empty_array_of_strings() {
        let head = "format = 1\n[[job]]\nid = \"a\"\n";
        assert_eq!(problems(head), ["a.toml の 2 行目: run が無い"]);
        assert_eq!(problems(&format!("{head}run = []\n")), ["a.toml の 4 行目: run が空"]);
        assert!(problems(&format!("{head}run = \"x.exe --a\"\n"))[0].starts_with("a.toml の 4 行目: run は文字列の配列で書く"));
        assert_eq!(problems(&format!("{head}run = [\"\"]\n")), ["a.toml の 4 行目: run の先頭（実行ファイル）が空"]);
        assert_eq!(problems(&format!("{head}run = [\"x\", 1]\n")), ["a.toml の 4 行目: run の要素は全部文字列で書く"]);
    }

    #[test]
    fn after_must_point_at_an_id_and_not_loop() {
        let t = |after_a: &str| {
            format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n{after_a}\n[[job]]\nid = \"b\"\nrun = [\"x\"]\nafter = [\"a\"]\n")
        };
        assert!(problems(&t("")).is_empty());
        assert_eq!(problems(&t("after = [\"zzz\"]")), ["a.toml の 5 行目: after が無い id を指している: zzz"]);
        assert_eq!(problems(&t("after = [\"b\"]")), ["a.toml の 2 行目: after が輪になっている: a → b → a"]);
        assert_eq!(problems(&t("after = [\"a\"]")), ["a.toml の 2 行目: after が輪になっている: a → a"]);
    }

    #[test]
    fn a_repeated_id_is_named_at_its_second_use() {
        let text = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n";
        assert_eq!(problems(text), ["a.toml の 6 行目: 同じ id が 2 つある: a"]);
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
        assert_eq!(problems(text), ["a.toml の 5 行目: timeout は \"30m\" のように文字列で書く（単位なしの数は秒か分か分からない）"]);
        let text = "format = 1
[[job]]
id = \"a\"
run = [\"x\"]
timeout = \"0m\"
";
        assert_eq!(problems(text), ["a.toml の 5 行目: timeout 0m: 0 より長くする"]);
        let text = "format = 1
[[job]]
id = \"a\"
run = [\"x\"]
timeout = \"30\"
";
        assert_eq!(problems(text), ["a.toml の 5 行目: timeout 30: 30m・1h30m・90s のように書く（h・m・s、この順）"]);
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
        let at = "a.toml の 5 行目: ";
        assert_eq!(problems(&job("retry = 2")), [format!("{at}retry は {{ count = 2, wait = \"5m\" }} の形で書く")]);
        assert_eq!(
            problems(&job("retry = { count = 2 }")),
            [format!("{at}retry に wait が無い（retry は {{ count = 2, wait = \"5m\" }} の形で書く）")]
        );
        assert_eq!(
            problems(&job("retry = { wait = \"5m\" }")),
            [format!("{at}retry に count が無い（retry は {{ count = 2, wait = \"5m\" }} の形で書く）")]
        );
        assert_eq!(problems(&job("retry = { count = 0, wait = \"5m\" }")), [format!("{at}retry の count は 1 から 100 の整数で書く")]);
        assert_eq!(problems(&job("retry = { count = 2, wait = 300 }")), [format!("{at}retry の wait は \"5m\" のように文字列で書く")]);
        assert_eq!(problems(&job("retry = { count = 2, wait = \"0s\" }")), [format!("{at}retry の wait 0s: 0 より長くする")]);
        assert_eq!(problems(&job("retry = { count = 2, wait = \"5m\", max = 1 }")), [format!("{at}retry に知らない欄 max")]);
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
        assert!(problems(&job("judge = { error_output = \"(\" }"))[0].contains("正規表現"));
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
        let at = "a.toml の 5 行目: ";
        let shape = "judge は { warn = \"1\", error = \"2..\" } の形で書く";
        assert_eq!(problems(&job("judge = \"1\"")), [format!("{at}{shape}")]);
        assert_eq!(problems(&job("judge = {}")), [format!("{at}judge が空（{shape}）")]);
        assert_eq!(problems(&job("judge = { ok = \"0\" }")), [format!("{at}judge に知らない欄 ok")]);
        assert_eq!(problems(&job("judge = { warn = true }")), [format!("{at}judge の warn は \"1\" や \"2..\" のように文字列で書く")]);
        assert_eq!(problems(&job("judge = { warn = \"\" }")), [format!("{at}judge の warn が空")]);
        assert_eq!(
            problems(&job("judge = { warn = \"x\" }")),
            [format!("{at}judge の warn \"x\" は終了コードで書く（\"1\"・\"2..\"・\"1..3\" のように）")]
        );
        assert_eq!(problems(&job("judge = { error = \"5..2\" }")), [format!("{at}judge の error \"5..2\": 始まりが終わりより大きい")]);
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
        assert_eq!(p("shell = \"powershell\""), ["a.toml の 4 行目: shell には script が要る"]);
        assert_eq!(p("shell = \"powershell\"\nscript = \" \""), ["a.toml の 4 行目: script が空"]);
        assert_eq!(p("shell = \"cmd\"\nscript = \"x\""), ["a.toml の 4 行目: shell は \"powershell\" のどれか"]);
        assert_eq!(p("run = [\"x\"]\nscript = \"x\""), ["a.toml の 5 行目: script には shell = \"powershell\" が要る"]);
        assert_eq!(
            p("run = [\"x\"]\nshell = \"powershell\"\nscript = \"x\""),
            ["a.toml の 4 行目: run と shell は両方書けない（run か、shell と script か）"]
        );
        assert_eq!(p("shell = \"powershell\"\nscript = 1"), ["a.toml の 5 行目: script は文字列で書く"]);
    }

    #[test]
    fn encoding_says_how_the_output_is_read() {
        let enc = |text: &str| parse(&format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n{text}\n")).unwrap().jobs[0].encoding;
        assert_eq!(enc(""), Encoding::Auto);
        assert_eq!(enc("encoding = \"sjis\""), Encoding::Sjis);
        assert_eq!(enc("encoding = \"utf8\""), Encoding::Utf8);
        assert_eq!(enc("encoding = \"auto\""), Encoding::Auto);
        let p = |text: &str| problems(&format!("format = 1\n[[job]]\nid = \"a\"\n{text}\n"));
        assert_eq!(p("run = [\"x\"]\nencoding = \"cp932\""), ["a.toml の 5 行目: encoding は \"auto\"・\"sjis\"・\"utf8\" のどれか"]);
        assert!(p("kind = \"notify\"\nencoding = \"sjis\"").contains(&"a.toml の 5 行目: encoding は kind = \"job\" にだけ書く".to_owned()));
    }

    #[test]
    fn fields_that_do_not_run_yet_are_refused_not_ignored() {
        let text = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\nto = \"c\"\n";
        assert_eq!(problems(text), ["a.toml の 5 行目: この版ではまだ使えない: to"]);
        assert_eq!(parse("format = 1\ncalendar = \"会社\"\n").unwrap().calendar.as_deref(), Some("会社"));
        assert_eq!(parse("format = 1\n").unwrap().calendar, None);
        let bad = "a.toml の 2 行目: calendar は calendars/<名前>.toml の <名前>（例: calendar = \"会社\"）";
        assert_eq!(problems("format = 1\ncalendar = \"a/b\"\n"), [bad]);
        assert_eq!(problems("format = 1\ncalendar = 1\n"), [bad]);
        assert!(problems("format = 1\nmissed = \"skip\"\noverlap = \"skip\"\n").is_empty());
        assert_eq!(problems("format = 1\nmissed = \"later\"\n"), ["a.toml の 2 行目: missed は \"skip\"・\"once\" のどれか"]);
        assert_eq!(problems("format = 1\noverlap = 1\n"), ["a.toml の 2 行目: overlap は \"skip\"・\"queue\" のどれか"]);
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
        assert_eq!(p("{ job = \"a\", on = \"bad\" }"), ["a.toml の 8 行目: after の線の on は \"ok\"・\"error\"・\"any\" のどれか"]);
        assert_eq!(p("{ job = \"a\", on = \"ok\", code = \"1\" }"), ["a.toml の 8 行目: after の線には on と code のどちらか 1 つを書く"]);
        assert_eq!(p("{ on = \"ok\" }"), ["a.toml の 8 行目: after の線には job（id）が要る"]);
        assert_eq!(p("{ job = \"a\", when = 1 }"), ["a.toml の 8 行目: after の線に知らない欄 when（job・on・code）"]);
        assert_eq!(
            p("{ job = \"a\", code = \"x\" }"),
            ["a.toml の 8 行目: after の線の code \"x\" は終了コードで書く（\"1\"・\"2..\"・\"1..3\" のように）"]
        );
        assert_eq!(p("{ job = \"a\", code = \"1\" }"), ["a.toml の 8 行目: code の線は kind = \"branch\" の部品からだけ引ける: a"]);
        assert_eq!(p("1"), ["a.toml の 8 行目: after の要素は id の文字列か { job, on } の表で書く"]);
    }

    /// each kind of part reads with its own fields, and refuses a job's fields.
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
        assert_eq!(net("join", "wait = \"one\"\n"), Err(vec!["a.toml の 8 行目: wait は \"all\"・\"any\" のどれか".to_owned()]));
        assert_eq!(net("time", "at = \"06:00\"\n"), Ok(Part::Time { at: when::time("06:00").unwrap(), days: 0 }));
        assert_eq!(net("time", "at = \"06:00\"\ndays = 1\n"), Ok(Part::Time { at: when::time("06:00").unwrap(), days: 1 }));
        assert_eq!(net("time", ""), Err(vec!["a.toml の 5 行目: kind = \"time\" には at が要る（例: at = \"06:00\"）".to_owned()]));
        assert_eq!(
            net("time", "at = \"06:00\"\ndays = -1\n"),
            Err(vec!["a.toml の 9 行目: days は 0 以上の整数で書く（何日後の at か）".to_owned()])
        );
        assert_eq!(net("file", "file = 'C:\\in\\*.csv'\n"), Ok(Part::File { file: "C:\\in\\*.csv".into(), settle: None }));
        assert_eq!(
            net("file", "file = 'C:\\in\\a.csv'\nsettle = \"10s\"\ntimeout = \"1h\"\n"),
            Ok(Part::File { file: "C:\\in\\a.csv".into(), settle: Some(10) })
        );
        assert_eq!(
            net("file", ""),
            Err(vec!["a.toml の 5 行目: kind = \"file\" には file が要る（例: file = 'C:\\in\\*.csv'）".to_owned()])
        );
        assert_eq!(net("file", "file = \" \"\n"), Err(vec!["a.toml の 8 行目: file が空".to_owned()]));
        assert_eq!(
            net("file", "file = \"a\"\nsettle = 10\n"),
            Err(vec!["a.toml の 9 行目: settle は \"30m\" のように文字列で書く（単位なしの数は秒か分か分からない）".to_owned()])
        );
        assert_eq!(net("notify", ""), Ok(Part::Notify));
        assert_eq!(net("jobnet", "use = \"共通/バックアップ\"\n"), Ok(Part::Jobnet { use_: "共通/バックアップ".into() }));
        assert_eq!(
            net("jobnet", ""),
            Err(vec!["a.toml の 5 行目: kind = \"jobnet\" には use が要る（例: use = \"共通/バックアップ\"）".to_owned()])
        );
        for u in ["a", "a/b/c", "/b", "a/", "../b", "a/..", "a\\b/c"] {
            assert_eq!(
                net("jobnet", &format!("use = \"{}\"\n", u.replace('\\', "\\\\"))),
                Err(vec!["a.toml の 8 行目: use は \"グループ/名前\" で書く（例: use = \"共通/バックアップ\"）".to_owned()]),
                "{u}"
            );
        }
        assert_eq!(net("branch", "run = [\"x\"]\n"), Err(vec!["a.toml の 8 行目: run は kind = \"job\" にだけ書く".to_owned()]));
        assert_eq!(
            net("branch", "timeout = \"1m\"\n"),
            Err(vec!["a.toml の 8 行目: timeout は kind = \"job\" か \"file\" にだけ書く".to_owned()])
        );
        assert_eq!(net("branch", "wait = \"any\"\n"), Err(vec!["a.toml の 8 行目: wait は kind = \"join\" にだけ書く".to_owned()]));
        assert_eq!(
            net("job", "use = \"a/b\"\nrun = [\"x\"]\n"),
            Err(vec!["a.toml の 8 行目: use は kind = \"jobnet\" にだけ書く".to_owned()])
        );
        assert_eq!(
            net("job", "at = \"06:00\"\nrun = [\"x\"]\n"),
            Err(vec!["a.toml の 8 行目: at は kind = \"time\" にだけ書く".to_owned()])
        );
        assert_eq!(
            net("step", ""),
            Err(vec![
                "a.toml の 7 行目: kind は \"job\"・\"branch\"・\"join\"・\"time\"・\"file\"・\"notify\"・\"approval\"・\"jobnet\" のどれか".to_owned()
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
            ["a.toml の 5 行目: 孤立した部品: j（after が無い）", "a.toml の 5 行目: 孤立した部品: j（後に続くものが無い）"]
        );
        assert_eq!(
            p("[[job]]\nid = \"t\"\nkind = \"time\"\nat = \"06:00\"\n"),
            ["a.toml の 5 行目: 孤立した部品: t（後に続くものが無い）"]
        );
        assert_eq!(p("[[job]]\nid = \"n\"\nkind = \"notify\"\n"), ["a.toml の 5 行目: 孤立した部品: n（after が無い）"]);
        assert_eq!(
            p("[[job]]\nid = \"b\"\nkind = \"branch\"\nafter = [\"a\", \"a\"]\n[[job]]\nid = \"z\"\nrun = [\"y\"]\nafter = [\"b\"]\n"),
            ["a.toml の 5 行目: kind = \"branch\" には after を 1 つ書く（その終了コードで分ける）"]
        );
        assert!(p("[[job]]\nid = \"n\"\nkind = \"jobnet\"\nuse = \"x/y\"\n").is_empty());
        assert_eq!(
            p("[[job]]\nid = \"b\"\nrun = [\"y\"]\nafter = [{ job = \"c\", on = \"any\" }]\n[[job]]\nid = \"c\"\nrun = [\"y\"]\nafter = [{ job = \"b\", on = \"error\" }]\n"),
            ["a.toml の 5 行目: after が輪になっている: b → c → b"]
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
                Err(format!("子ジョブネットがない: {u}"))
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
        assert_eq!(expand(&missing, &lookup).unwrap_err(), "子ジョブネットがない: x/y");
        let myself = parse("format = 1\n[[job]]\nid = \"me\"\nkind = \"jobnet\"\nuse = \"a/b\"\n").unwrap();
        let loop_ = |u: &str| if u == "a/b" { Ok(myself.clone()) } else { Err(String::new()) };
        assert_eq!(expand(&myself, &loop_).unwrap_err(), "子ジョブネットの呼び合いが輪になっている: a/b → a/b");
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
        assert_eq!(p("every = \"day\""), ["a.toml の 2 行目: at が要る（例: at = [\"02:00\"]）"]);
        assert_eq!(
            p("at = [\"02:00\"]"),
            ["a.toml の 2 行目: [[when]] には once か every か dates か file か on_start か after_jobnet が要る"]
        );
        assert_eq!(
            p("every = \"daily\"\nat = [\"02:00\"]"),
            ["a.toml の 3 行目: every は \"day\"・\"minutes\"・\"hours\"・\"week\"・\"month\"・\"year\" のどれか"]
        );
        assert_eq!(p("every = \"day\"\nat = [\"2 時\"]"), ["a.toml の 4 行目: 時刻は \"02:00\" のように書く: \"2 時\""]);
        assert_eq!(p("every = \"day\"\nat = []"), ["a.toml の 4 行目: at が空"]);
        assert_eq!(p("every = \"day\"\nat = [\"02:00\"]\nday = 3"), ["a.toml の 5 行目: day は day では使わない"]);
        assert_eq!(p("every = \"week\"\nat = [\"02:00\"]"), ["a.toml の 2 行目: days が要る（例: days = [\"mon\", \"fri\"]）"]);
        assert_eq!(
            p("every = \"week\"\ndays = [\"火\"]\nat = [\"02:00\"]"),
            ["a.toml の 4 行目: 曜日は mon・tue・wed・thu・fri・sat・sun のどれか: \"火\""]
        );
        assert_eq!(p("every = \"minutes\"\nfrom = \"09:00\"\nto = \"10:00\""), ["a.toml の 2 行目: n が要る（例: n = 15）"]);
        assert_eq!(p("every = \"minutes\"\nn = 0\nfrom = \"09:00\"\nto = \"10:00\""), ["a.toml の 4 行目: n は 1 以上の整数で書く"]);
        assert_eq!(p("every = \"hours\"\nn = 1"), ["a.toml の 2 行目: from と to が要る（例: from = \"09:00\", to = \"18:00\"）"]);
        assert_eq!(p("every = \"hours\"\nn = 1\nfrom = \"18:00\"\nto = \"09:00\""), ["a.toml の 5 行目: from は to より前"]);
        assert_eq!(
            p("every = \"month\"\nat = [\"09:00\"]"),
            ["a.toml の 2 行目: month には day か、nth と weekday か、business_day が要る（例: day = 25、day = \"last\"、nth = 2 と weekday = \"tue\"、business_day = 5）"]
        );
        assert_eq!(
            p("every = \"month\"\nbusiness_day = 0\nat = [\"09:00\"]"),
            ["a.toml の 4 行目: business_day は 1 から 31 か -1 から -31 の数"]
        );
        assert_eq!(
            p("every = \"month\"\nbusiness_day = 32\nat = [\"09:00\"]"),
            ["a.toml の 4 行目: business_day は 1 から 31 か -1 から -31 の数"]
        );
        assert_eq!(
            p("every = \"month\"\nbusiness_day = 1\nday = 2\nat = [\"09:00\"]"),
            ["a.toml の 5 行目: day は business_day と一緒に使わない"]
        );
        assert_eq!(p("every = \"month\"\nday = 20\nshift = 40\nat = [\"09:00\"]"), ["a.toml の 5 行目: shift は -31 から 31 の数"]);
        assert_eq!(
            p("every = \"month\"\nnth = 2\nweekday = \"tue\"\nshift = 1\nat = [\"09:00\"]"),
            ["a.toml の 6 行目: shift は day の数と使う（例: day = 20, shift = -3）"]
        );
        assert_eq!(
            p("every = \"month\"\nday = \"last\"\nshift = -1\nat = [\"09:00\"]"),
            ["a.toml の 5 行目: shift は day の数と使う（例: day = 20, shift = -3）"]
        );
        assert_eq!(p("every = \"day\"\nat = [\"02:00\"]\nbusiness_day = 1"), ["a.toml の 5 行目: business_day は day では使わない"]);
        assert_eq!(
            p("every = \"day\"\nat = [\"02:00\"]\non_closed = \"before\""),
            ["a.toml の 5 行目: on_closed は \"prev\"・\"next\"・\"skip\"・\"run\" のどれか"]
        );
        assert_eq!(p("every = \"month\"\nday = 32\nat = [\"09:00\"]"), ["a.toml の 4 行目: day は 1 から 31 の数か \"last\""]);
        assert_eq!(p("every = \"month\"\nnth = 6\nweekday = \"tue\"\nat = [\"09:00\"]"), ["a.toml の 4 行目: nth は 1 から 5 の数"]);
        assert_eq!(
            p("every = \"year\"\ndate = \"13-01\"\nat = [\"09:00\"]"),
            ["a.toml の 4 行目: date は \"04-01\" のように月-日で書く: \"13-01\""]
        );
        assert_eq!(p("dates = []\nat = [\"09:00\"]"), ["a.toml の 2 行目: dates が空"]);
        assert_eq!(p("dates = [\"30 日\"]\nat = [\"09:00\"]"), ["a.toml の 3 行目: 日付は 2026-11-30 のように書く: 30 日"]);
        assert_eq!(p("once = \"今夜\""), ["a.toml の 3 行目: 日時は 2026-11-30T21:00:00 のように書く（時差はつけない）: 今夜"]);
        assert_eq!(p("every = \"day\"\nat = [\"02:00\"]\nstart = 2027-01-01\nend = 2026-12-01"), ["a.toml の 5 行目: start は end より前"]);
        assert_eq!(p("every = \"day\"\nat = [\"02:00\"]\nfoo = 1"), ["a.toml の 5 行目: 知らない欄 foo"]);
        for f in ["after_jobnet = \"a/b\"", "on = \"ok\""] {
            let name = f.split(' ').next().unwrap();
            assert_eq!(
                p(&format!("every = \"day\"\nat = [\"02:00\"]\n{f}")),
                [format!("a.toml の 5 行目: {name} は day では使わない")],
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
        assert_eq!(
            p("after_jobnet = \"売上取込\""),
            ["a.toml の 3 行目: after_jobnet は \"グループ/名前\" で書く（例: after_jobnet = \"経理/売上取込\"）"]
        );
        assert_eq!(p("after_jobnet = 1"), ["a.toml の 3 行目: after_jobnet は文字列で書く"]);
        assert_eq!(p("after_jobnet = \"a/b\"\non = \"sometimes\""), ["a.toml の 4 行目: on は \"ok\"・\"error\"・\"any\" のどれか"]);
        assert_eq!(p("after_jobnet = \"a/b\"\nat = [\"02:00\"]"), ["a.toml の 4 行目: at は after_jobnet では使わない"]);
        assert_eq!(
            p("after_jobnet = \"a/b\"\non_closed = \"next\""),
            ["a.toml の 4 行目: after_jobnet では on_closed は \"skip\" か \"run\""]
        );
        assert_eq!(p("on = \"ok\""), ["a.toml の 2 行目: [[when]] には once か every か dates か file か on_start か after_jobnet が要る"]);
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
        assert_eq!(p("file = 'in\\*.csv'"), ["a.toml の 3 行目: file は絶対パスで書く（例: file = 'D:\\受信\\*.csv'）"]);
        assert_eq!(p("file = 'D:\\受信\\'"), ["a.toml の 3 行目: file はファイル名まで書く（例: file = 'D:\\受信\\*.csv'）"]);
        assert_eq!(p("file = 'D:\\*\\a.csv'"), ["a.toml の 3 行目: file の * / ? はファイル名の部分だけ（フォルダーには使えない）"]);
        assert_eq!(p("file = ''"), ["a.toml の 3 行目: file が空"]);
        assert_eq!(
            p("file = 'D:\\in\\a.csv'\nsettle = 10"),
            ["a.toml の 4 行目: settle は \"30m\" のように文字列で書く（単位なしの数は秒か分か分からない）"]
        );
        assert_eq!(p("file = 'D:\\in\\a.csv'\nat = [\"02:00\"]"), ["a.toml の 4 行目: at は file では使わない"]);
        assert_eq!(
            p("file = 'D:\\in\\a.csv'\non_closed = \"prev\""),
            ["a.toml の 4 行目: file / on_start では on_closed は \"skip\" か \"run\""]
        );
        assert_eq!(p("on_start = false"), ["a.toml の 3 行目: on_start は true だけ（使わないなら [[when]] ごと消す）"]);
        assert_eq!(p("on_start = true\nsettle = \"10s\""), ["a.toml の 4 行目: settle は on_start では使わない"]);
        assert_eq!(
            p("settle = \"10s\""),
            ["a.toml の 2 行目: [[when]] には once か every か dates か file か on_start か after_jobnet が要る"]
        );
        for f in ["file = 'D:\\in\\a.csv'", "settle = \"10s\"", "on_start = true"] {
            let name = f.split(' ').next().unwrap();
            assert_eq!(
                p(&format!("every = \"day\"\nat = [\"02:00\"]\n{f}")),
                [format!("a.toml の 5 行目: {name} は day では使わない")],
                "{f}"
            );
        }
    }

    #[test]
    fn format_must_be_one() {
        assert_eq!(problems("name = \"x\"\n"), ["a.toml の 1 行目: format = 1 が要る"]);
        assert_eq!(problems("format = 2\n"), ["a.toml の 1 行目: この版が読める format は 1 だけ"]);
    }

    #[test]
    fn several_problems_come_in_line_order() {
        let text = "format = 1\n[[job]]\nid = \"a\"\n[[job]]\nid = \"a\"\nrun = []\n";
        assert_eq!(
            problems(text),
            ["a.toml の 2 行目: run が無い", "a.toml の 5 行目: 同じ id が 2 つある: a", "a.toml の 6 行目: run が空"]
        );
    }

    #[test]
    fn line_of_counts_from_one() {
        assert_eq!(line_of("a\nb\nc", 0), 1);
        assert_eq!(line_of("a\nb\nc", 2), 2);
        assert_eq!(line_of("a\nb\nc", 99), 3);
    }
}
