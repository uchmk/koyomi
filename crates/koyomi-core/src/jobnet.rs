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

use crate::when::{self, Kind, MonthRule, When};

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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub name: Option<String>,
    /// The executable first, then one argument per element. Never a shell line.
    pub run: Vec<String>,
    pub dir: Option<String>,
    pub after: Vec<String>,
    pub pos: Option<(f64, f64)>,
    /// Seconds the job may run before it is stopped, from `timeout = "30m"`.
    #[serde(default)]
    pub timeout: Option<u64>,
    /// Run it again after an abnormal end, from `retry = { count = 2, wait = "5m" }`.
    #[serde(default)]
    pub retry: Option<Retry>,
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
    // Of the design, not run by this version.
    calendar: Field,
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
    // Of the design, not run by this version.
    on_closed: Field,
    business_day: Field,
    shift: Field,
    file: Field,
    settle: Field,
    on_start: Field,
    after_jobnet: Field,
    on: Field,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJob {
    id: Field,
    name: Field,
    run: Field,
    dir: Field,
    after: Field,
    pos: Field,
    // Of the design, not run by this version.
    kind: Field,
    #[serde(rename = "use")]
    use_: Field,
    to: Field,
    shell: Field,
    script: Field,
    timeout: Field,
    retry: Field,
    judge: Field,
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
    let jobs = c.jobs(&raw.job);
    let when = c.whens(&raw.when);
    if c.problems.is_empty() {
        Ok(Jobnet { name: c.string(&raw.name, "name"), jobs, when, missed, overlap })
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

    fn timeout(&mut self, field: &Field) -> Option<u64> {
        let f = field.as_ref()?;
        let text = match f.get_ref() {
            toml::Value::String(s) => s.clone(),
            _ => {
                self.add(f.span().start, "timeout は \"30m\" のように文字列で書く（単位なしの数は秒か分か分からない）");
                return None;
            }
        };
        match crate::duration::parse(&text) {
            Ok(secs) => Some(secs),
            Err(e) => {
                self.add(f.span().start, format!("timeout {e}"));
                None
            }
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
        self.not_yet(&raw.calendar, "calendar");
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

    /// The `[[when]]` entries that read; a mistake in one drops that entry.
    fn whens(&mut self, raw: &[Spanned<RawWhen>]) -> Vec<When> {
        let mut out = Vec::new();
        for entry in raw {
            let head = entry.span().start;
            let w = entry.get_ref();
            for (field, name) in [
                (&w.on_closed, "on_closed"),
                (&w.business_day, "business_day"),
                (&w.shift, "shift"),
                (&w.file, "file"),
                (&w.settle, "settle"),
                (&w.on_start, "on_start"),
                (&w.after_jobnet, "after_jobnet"),
                (&w.on, "on"),
            ] {
                self.not_yet(field, name);
            }
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
            } else {
                self.add(head, "[[when]] には once か every か dates が要る");
                continue;
            };
            let allowed: &[&str] = match kind {
                "once" => &["once"],
                "day" => &["every", "at"],
                "minutes" | "hours" => &["every", "n", "from", "to", "days"],
                "week" => &["every", "days", "at"],
                "month" => &["every", "day", "nth", "weekday", "at"],
                "year" => &["every", "date", "at"],
                _ => &["dates", "at"],
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
            ] {
                if let Some(f) = field {
                    if !allowed.contains(&name) {
                        self.add(f.span().start, format!("{name} は {kind} では使わない"));
                    }
                }
            }
            let before = self.problems.len();
            let kind = match kind {
                "once" => self.datetime(&w.once).map(|at| Kind::Once { at }),
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
            if let (Some(s), Some(e)) = (start, end) {
                if s > e {
                    self.add(w.start.as_ref().map_or(head, |f| f.span().start), "start は end より前");
                }
            }
            if let Some(kind) = kind {
                if self.problems.len() == before {
                    out.push(When { kind, except, start, end });
                }
            }
        }
        out
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
        match (&w.day, &w.nth, &w.weekday) {
            (Some(d), None, None) => match d.get_ref() {
                toml::Value::Integer(n) if (1..=31).contains(n) => Some(MonthRule::Day { day: *n as u32 }),
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
                self.add(head, "month には day か、nth と weekday が要る（例: day = 25、day = \"last\"、nth = 2 と weekday = \"tue\"）");
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

    fn jobs(&mut self, raw: &[Spanned<RawJob>]) -> Vec<Job> {
        let mut jobs = Vec::new();
        let mut seen = BTreeSet::new();
        for entry in raw {
            let start = entry.span().start;
            let j = entry.get_ref();
            for (field, name) in
                [(&j.kind, "kind"), (&j.use_, "use"), (&j.to, "to"), (&j.shell, "shell"), (&j.script, "script"), (&j.judge, "judge")]
            {
                self.not_yet(field, name);
            }
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
            let run = self.run(&j.run, start);
            let after = self.after(&j.after);
            let dir = self.string(&j.dir, "dir");
            let name = self.string(&j.name, "name");
            let pos = self.pos(&j.pos);
            let timeout = self.timeout(&j.timeout);
            let retry = self.retry(&j.retry);
            jobs.push(Job { id, name, run, dir, after, pos, timeout, retry });
        }
        for (job, entry) in jobs.iter().zip(raw) {
            let at = entry.get_ref().after.as_ref().map_or(entry.span().start, |f| f.span().start);
            for a in job.after.iter().filter(|a| !seen.contains(a.as_str())) {
                self.add(at, format!("after が無い id を指している: {a}"));
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

    fn after(&mut self, field: &Field) -> Vec<String> {
        let Some(f) = field else { return Vec::new() };
        let at = f.span().start;
        let toml::Value::Array(items) = f.get_ref() else {
            self.add(at, "after は id の配列で書く（例: after = [\"extract\"]）");
            return Vec::new();
        };
        let mut out = Vec::new();
        for v in items {
            match v {
                toml::Value::String(s) => out.push(s.clone()),
                toml::Value::Table(_) => self.add(at, "この版ではまだ使えない: after の線の種類（{ job, on }）"),
                _ => self.add(at, "after の要素は id の文字列で書く"),
            }
        }
        out
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

/// The ids of a loop in `after` (`a → b → a`), if any.
fn find_cycle(jobs: &[Job]) -> Option<Vec<String>> {
    type After<'a> = BTreeMap<&'a str, &'a [String]>;
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
        for next in after.get(id).copied().unwrap_or(&[]) {
            if after.contains_key(next.as_str()) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(net.jobs[1].after, ["extract"]);
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
        let net = parse("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n").unwrap();
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
    fn fields_that_do_not_run_yet_are_refused_not_ignored() {
        for f in
            ["judge = { warn = \"1\" }", "kind = \"notify\"", "shell = \"powershell\"", "script = \"x\"", "use = \"a/b\"", "to = \"c\""]
        {
            let text = format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n{f}\n");
            let name = f.split(' ').next().unwrap();
            assert_eq!(problems(&text), [format!("a.toml の 5 行目: この版ではまだ使えない: {name}")], "{f}");
        }
        assert_eq!(problems("format = 1\ncalendar = \"会社\"\n"), ["a.toml の 2 行目: この版ではまだ使えない: calendar"]);
        assert!(problems("format = 1\nmissed = \"skip\"\noverlap = \"skip\"\n").is_empty());
        assert_eq!(problems("format = 1\nmissed = \"later\"\n"), ["a.toml の 2 行目: missed は \"skip\"・\"once\" のどれか"]);
        assert_eq!(problems("format = 1\noverlap = 1\n"), ["a.toml の 2 行目: overlap は \"skip\"・\"queue\" のどれか"]);
        let line = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\nafter = [{ job = \"b\", on = \"error\" }]\n";
        assert_eq!(problems(line), ["a.toml の 5 行目: この版ではまだ使えない: after の線の種類（{ job, on }）"]);
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
            Kind::Month { rule: MonthRule::Day { day: 25 }, at: vec![t("09:00")] }
        );
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
        assert_eq!(p("at = [\"02:00\"]"), ["a.toml の 2 行目: [[when]] には once か every か dates が要る"]);
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
            ["a.toml の 2 行目: month には day か、nth と weekday が要る（例: day = 25、day = \"last\"、nth = 2 と weekday = \"tue\"）"]
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
        for f in [
            "on_closed = \"before\"",
            "business_day = -1",
            "shift = \"before\"",
            "file = \"x\"",
            "settle = \"10s\"",
            "on_start = true",
            "after_jobnet = \"a/b\"",
            "on = \"ok\"",
        ] {
            let name = f.split(' ').next().unwrap();
            assert_eq!(
                p(&format!("every = \"day\"\nat = [\"02:00\"]\n{f}")),
                [format!("a.toml の 5 行目: この版ではまだ使えない: {name}")],
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
