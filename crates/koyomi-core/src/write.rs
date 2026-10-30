//! A job net written back as its file: what the editor saves and what
//! `koyomi new` starts from, and the check that lays the parser's problems on
//! the parts of the net (the editor paints those red).
//!
//! The text is the file's own shape (`[[when]]` then `[[job]]`, one field a
//! line) so a person can go on by hand. Comments of a hand-written file are
//! not kept: the engine puts the file as it was under `versions/` first.

use chrono::{NaiveDate, NaiveTime, Timelike, Weekday};

use crate::duration;
use crate::jobnet::{self, Job, Jobnet, Judge, Line, Missed, On, Overlap, Part, Wait};
use crate::when::{AfterOn, Kind, MonthRule, OnClosed, When};

/// The net as its file.
pub fn text(net: &Jobnet) -> String {
    with_lines(net).0
}

/// The net as its file, and for each job the 1-based lines its `[[job]]`
/// takes (`from..to`, `to` excluded): what [`check`] maps the problems with.
pub fn with_lines(net: &Jobnet) -> (String, Vec<(String, usize, usize)>) {
    let mut out = String::from("format = 1\n");
    if let Some(n) = &net.name {
        push(&mut out, "name", &q(n));
    }
    if net.missed != Missed::Skip {
        push(&mut out, "missed", "\"once\"");
    }
    if net.overlap != Overlap::Skip {
        push(&mut out, "overlap", "\"queue\"");
    }
    if let Some(c) = &net.calendar {
        push(&mut out, "calendar", &q(c));
    }
    if let Some(l) = &net.lock {
        push(&mut out, "lock", &q(l));
    }
    if net.wake {
        push(&mut out, "wake", "true");
    }
    if !net.params.is_empty() {
        out.push_str("\n[params]\n");
        for (k, v) in &net.params {
            push(&mut out, k, &q(v));
        }
    }
    for w in &net.when {
        when(w, &mut out);
    }
    let mut spans = Vec::new();
    for j in &net.jobs {
        let from = out.lines().count() + 2;
        job(j, &mut out);
        spans.push((j.id.clone(), from, out.lines().count() + 1));
    }
    (out, spans)
}

/// What a `[[job]]` of the net is wrong with, as the file would read: the
/// job's id when the problem sits in its `[[job]]`, `None` for the rest (a
/// `[[when]]`, the head).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub job: Option<String>,
    pub message: String,
}

/// The problems the file would have, each on its part.
pub fn check(net: &Jobnet) -> Vec<Found> {
    let (text, spans) = with_lines(net);
    match jobnet::parse(&text) {
        Ok(_) => Vec::new(),
        Err(problems) => problems
            .into_iter()
            .map(|p| {
                let job = spans.iter().find(|(_, from, to)| (*from..*to).contains(&p.line)).map(|(id, _, _)| id.clone());
                Found { job, message: p.message }
            })
            .collect(),
    }
}

/// The file `koyomi new` writes: one job that does nothing yet, and no
/// `[[when]]` (by hand only until a time is put in). It reads as it is.
pub fn template(name: &str) -> String {
    format!(
        "format = 1\nname = {}\n\n# 時刻は [[when]] で足す。無ければ手で流すだけ。\n# [[when]]\n# every = \"day\"\n# at = [\"02:00\"]\n\n[[job]]\nid = \"job1\"\nname = \"最初のジョブ\"\n# 実行ファイルと引数を 1 つずつ（シェルの 1 行ではない）\nrun = [\"cmd.exe\", \"/c\", \"echo hello\"]\n",
        q(name)
    )
}

fn push(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(" = ");
    out.push_str(value);
    out.push('\n');
}

/// A TOML basic string: quotes, backslashes and control characters escaped.
pub fn q(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn time(t: NaiveTime) -> String {
    if t.second() == 0 { t.format("%H:%M") } else { t.format("%H:%M:%S") }.to_string()
}

fn times(v: &[NaiveTime]) -> String {
    format!("[{}]", v.iter().map(|t| q(&time(*t))).collect::<Vec<_>>().join(", "))
}

fn weekday(w: Weekday) -> String {
    format!("{w:?}").to_lowercase()
}

fn weekdays(v: &[Weekday]) -> String {
    format!("[{}]", v.iter().map(|w| q(&weekday(*w))).collect::<Vec<_>>().join(", "))
}

fn dates(v: &[NaiveDate]) -> String {
    format!("[{}]", v.iter().map(|d| d.format("%Y-%m-%d").to_string()).collect::<Vec<_>>().join(", "))
}

fn when(w: &When, out: &mut String) {
    out.push_str("\n[[when]]\n");
    match &w.kind {
        Kind::Once { at } => push(out, "once", &at.format("%Y-%m-%dT%H:%M:%S").to_string()),
        Kind::Day { at } => {
            push(out, "every", "\"day\"");
            push(out, "at", &times(at));
        }
        Kind::Minutes { n, from, to, days } | Kind::Hours { n, from, to, days } => {
            push(out, "every", if matches!(w.kind, Kind::Minutes { .. }) { "\"minutes\"" } else { "\"hours\"" });
            push(out, "n", &n.to_string());
            push(out, "from", &q(&time(*from)));
            push(out, "to", &q(&time(*to)));
            if !days.is_empty() {
                push(out, "days", &weekdays(days));
            }
        }
        Kind::Week { days, at } => {
            push(out, "every", "\"week\"");
            push(out, "days", &weekdays(days));
            push(out, "at", &times(at));
        }
        Kind::Month { rule, at } => {
            push(out, "every", "\"month\"");
            match rule {
                MonthRule::Day { day, shift } => {
                    push(out, "day", &day.to_string());
                    if *shift != 0 {
                        push(out, "shift", &shift.to_string());
                    }
                }
                MonthRule::Last => push(out, "day", "\"last\""),
                MonthRule::Nth { nth, weekday: wd } => {
                    push(out, "nth", &nth.to_string());
                    push(out, "weekday", &q(&weekday(*wd)));
                }
                MonthRule::BusinessDay { n } => push(out, "business_day", &n.to_string()),
            }
            push(out, "at", &times(at));
        }
        Kind::Year { month, day, at } => {
            push(out, "every", "\"year\"");
            push(out, "date", &q(&format!("{month:02}-{day:02}")));
            push(out, "at", &times(at));
        }
        Kind::Dates { dates: ds, at } => {
            push(out, "dates", &dates(ds));
            push(out, "at", &times(at));
        }
        Kind::File { file, settle } => {
            push(out, "file", &q(file));
            if let Some(s) = settle {
                push(out, "settle", &q(&duration::text(*s)));
            }
        }
        Kind::OnStart => push(out, "on_start", "true"),
        Kind::AfterJobnet { jobnet, on } => {
            push(out, "after_jobnet", &q(jobnet));
            if *on != AfterOn::Ok {
                push(out, "on", &q(on.name()));
            }
        }
    }
    if !w.except.is_empty() {
        push(out, "except", &dates(&w.except));
    }
    if let Some(d) = w.start {
        push(out, "start", &d.format("%Y-%m-%d").to_string());
    }
    if let Some(d) = w.end {
        push(out, "end", &d.format("%Y-%m-%d").to_string());
    }
    if let Some(c) = w.on_closed {
        let word = match c {
            OnClosed::Prev => "prev",
            OnClosed::Next => "next",
            OnClosed::Skip => "skip",
            OnClosed::Run => "run",
        };
        push(out, "on_closed", &q(word));
    }
    if let Some(u) = &w.wait_for {
        push(out, "wait_for", &q(u));
    }
    if let Some(t) = w.wait_until {
        push(out, "wait_until", &q(&t.format("%H:%M").to_string()));
    }
}

fn line(l: &Line) -> String {
    match &l.on {
        On::Ok => q(&l.job),
        On::Error => format!("{{ job = {}, on = \"error\" }}", q(&l.job)),
        On::Any => format!("{{ job = {}, on = \"any\" }}", q(&l.job)),
        On::Code(r) => format!("{{ job = {}, code = {} }}", q(&l.job), q(&Judge::list(r))),
        On::Else => format!("{{ job = {}, code = \"else\" }}", q(&l.job)),
    }
}

fn job(j: &Job, out: &mut String) {
    out.push_str("\n[[job]]\n");
    push(out, "id", &q(&j.id));
    if let Some(n) = &j.name {
        push(out, "name", &q(n));
    }
    if let Some(n) = &j.note {
        push(out, "note", &q(n));
    }
    if !j.part.is_job() {
        push(out, "kind", &q(j.part.key()));
    }
    match &j.part {
        Part::Jobnet { use_ } => push(out, "use", &q(use_)),
        Part::Join { wait } => push(out, "wait", if *wait == Wait::Any { "\"any\"" } else { "\"all\"" }),
        Part::Time { at, days } => {
            push(out, "at", &q(&time(*at)));
            if *days != 0 {
                push(out, "days", &days.to_string());
            }
        }
        Part::File { file, settle } => {
            push(out, "file", &q(file));
            if let Some(s) = settle {
                push(out, "settle", &q(&duration::text(*s)));
            }
        }
        Part::Job | Part::Branch | Part::Notify | Part::Approval => {}
    }
    if !j.run.is_empty() {
        push(out, "run", &format!("[{}]", j.run.iter().map(|s| q(s)).collect::<Vec<_>>().join(", ")));
    }
    if let Some(s) = &j.shell {
        push(out, "shell", &q(s));
    }
    if let Some(s) = &j.script {
        push(out, "script", &q(s));
    }
    if let Some(d) = &j.dir {
        push(out, "dir", &q(d));
    }
    if !j.env.is_empty() {
        push(out, "env", &format!("{{ {} }}", j.env.iter().map(|(k, v)| format!("{k} = {}", q(v))).collect::<Vec<_>>().join(", ")));
    }
    if !j.after.is_empty() {
        push(out, "after", &format!("[{}]", j.after.iter().map(line).collect::<Vec<_>>().join(", ")));
    }
    if let Some(t) = j.timeout {
        push(out, "timeout", &q(&duration::text(t)));
    }
    if let Some(r) = &j.retry {
        push(out, "retry", &format!("{{ count = {}, wait = {} }}", r.count, q(&duration::text(r.wait))));
    }
    if let Some(jd) = &j.judge {
        let mut parts = Vec::new();
        if !jd.warn.is_empty() {
            parts.push(format!("warn = {}", q(&Judge::list(&jd.warn))));
        }
        if !jd.error.is_empty() {
            parts.push(format!("error = {}", q(&Judge::list(&jd.error))));
        }
        if let Some(p) = &jd.error_output {
            parts.push(format!("error_output = {}", q(p)));
        }
        if let Some(p) = &jd.ok_file {
            parts.push(format!("ok_file = {}", q(p)));
        }
        if !parts.is_empty() {
            push(out, "judge", &format!("{{ {} }}", parts.join(", ")));
        }
    }
    if !j.encoding.is_auto() {
        push(out, "encoding", &q(j.encoding.text()));
    }
    if let Some((x, y)) = j.pos {
        push(out, "pos", &format!("[{x:?}, {y:?}]"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"format = 1
name = "月次締め"
missed = "once"
overlap = "queue"
calendar = "会社"

[[when]]
every = "day"
at = ["02:00", "14:30:15"]
except = [2026-12-30]
start = 2026-01-01
end = 2027-12-31
on_closed = "prev"

[[when]]
every = "minutes"
n = 15
from = "09:00"
to = "17:00"
days = ["mon", "fri"]

[[when]]
every = "hours"
n = 2
from = "00:00"
to = "23:00"

[[when]]
every = "week"
days = ["tue"]
at = ["08:00"]

[[when]]
every = "month"
day = 25
shift = -3
at = ["09:00"]

[[when]]
every = "month"
day = "last"
at = ["09:00"]

[[when]]
every = "month"
nth = 2
weekday = "wed"
at = ["09:00"]

[[when]]
every = "month"
business_day = -1
at = ["09:00"]

[[when]]
every = "year"
date = "04-01"
at = ["09:00"]

[[when]]
dates = [2026-12-30, 2027-01-04]
at = ["09:00"]

[[when]]
once = 2026-11-30T21:00:00

[[when]]
file = "D:\\受信\\*.csv"
settle = "10s"
on_closed = "skip"

[[when]]
on_start = true

[[when]]
after_jobnet = "経理/売上取込"
on = "error"

[[job]]
id = "a"
name = "取り込み"
run = ["C:\\tools\\import.exe", "--all"]
dir = "C:\\work"
timeout = "30m"
retry = { count = 2, wait = "5m" }
judge = { warn = "1", error = "2.." }
encoding = "sjis"
pos = [0.0, 0.0]

[[job]]
id = "s"
shell = "powershell"
script = "Write-Host \"hi\"\nexit 0"
after = ["a"]

[[job]]
id = "b"
kind = "branch"
after = ["s"]

[[job]]
id = "c"
after = [{ job = "b", code = "1..3" }]
run = ["x.exe"]

[[job]]
id = "d"
after = [{ job = "b", code = "else" }]
run = ["y.exe"]

[[job]]
id = "j"
kind = "join"
wait = "any"
after = ["c", "d"]

[[job]]
id = "t"
kind = "time"
at = "09:00"
days = 1
after = ["j"]

[[job]]
id = "f"
kind = "file"
file = "D:\\受信\\done.txt"
settle = "30s"
after = ["t"]

[[job]]
id = "e"
run = ["z.exe"]
after = [{ job = "f", on = "error" }, { job = "t", on = "any" }]

[[job]]
id = "n"
kind = "notify"
after = ["e"]

[[job]]
id = "k"
kind = "jobnet"
use = "経理/売上取込"
after = ["n"]
"#;

    #[test]
    fn the_file_written_reads_back_as_the_same_net() {
        let net = jobnet::parse(FULL).unwrap_or_else(|p| panic!("{p:?}"));
        let text = text(&net);
        let again = jobnet::parse(&text).unwrap_or_else(|p| panic!("{p:?}\n{text}"));
        assert_eq!(again, net);
        // And once more: the writer is a fixed point.
        assert_eq!(text, super::text(&again));
    }

    #[test]
    fn the_smallest_net_is_a_few_lines() {
        let net = jobnet::parse("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n").unwrap();
        assert_eq!(text(&net), "format = 1\n\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n");
    }

    #[test]
    fn an_approval_part_survives_a_round_trip() {
        let src = "format = 1

[[job]]
id = \"ok\"
kind = \"approval\"

[[job]]
id = \"b\"
run = [\"x.exe\"]
after = [\"ok\"]
";
        let net = jobnet::parse(src).unwrap();
        assert_eq!(jobnet::parse(&text(&net)).unwrap().jobs.len(), 2);
    }

    #[test]
    fn wake_survives_a_round_trip() {
        let src = "format = 1
wake = true

[[job]]
id = \"a\"
run = [\"x.exe\"]
";
        let net = jobnet::parse(src).unwrap();
        assert!(net.wake);
        assert_eq!(text(&net), src);
        assert!(
            !jobnet::parse(&src.replace(
                "wake = true
",
                ""
            ))
            .unwrap()
            .wake
        );
        assert!(jobnet::parse(&src.replace("true", "\"yes\"")).is_err());
    }

    #[test]
    fn params_survive_a_round_trip_and_are_checked() {
        let src = "format = 1

[params]
month = \"{month-1:%Y-%m}\"

[[job]]
id = \"a\"
run = [\"x.exe\", \"{param:month}\"]
";
        let net = jobnet::parse(src).unwrap();
        assert_eq!(net.params["month"], "{month-1:%Y-%m}");
        assert_eq!(text(&net), src);
        let msg = |e: Vec<jobnet::Problem>| e.iter().map(|p| p.message.clone()).collect::<Vec<_>>().join(" / ");
        assert!(msg(jobnet::parse(&src.replace("{param:month}", "{param:day}")).unwrap_err()).contains("{param:day}: [params] にない"));
        assert!(msg(jobnet::parse(&src.replace("month = ", "1m = ")).unwrap_err()).contains("params の名前に使えない"));
        assert!(msg(jobnet::parse(&src.replace("\"{month-1:%Y-%m}\"", "3")).unwrap_err()).contains("params の値は文字列"));
    }

    #[test]
    fn a_lock_name_survives_a_round_trip() {
        let src = "format = 1
lock = \"db\"

[[job]]
id = \"a\"
run = [\"x.exe\"]
";
        let net = jobnet::parse(src).unwrap();
        assert_eq!(net.lock.as_deref(), Some("db"));
        assert_eq!(text(&net), src);
        assert!(jobnet::parse(
            "format = 1
lock = \"a/b\"
[[job]]
id = \"a\"
run = [\"x\"]
"
        )
        .is_err());
    }

    #[test]
    fn a_sticky_note_survives_a_round_trip() {
        let src = r#"format = 1

[[job]]
id = "a"
note = "line1\nline2"
run = ["x.exe"]
"#;
        let net = jobnet::parse(src).unwrap();
        assert_eq!(net.jobs[0].note.as_deref(), Some("line1\nline2"));
        assert_eq!(text(&net), src);
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(q("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
        assert_eq!(q("D:\\受信"), "\"D:\\\\受信\"");
    }

    #[test]
    fn the_check_lays_each_problem_on_its_part() {
        let mut net =
            jobnet::parse("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n[[job]]\nid = \"b\"\nrun = [\"y.exe\"]\nafter = [\"a\"]\n")
                .unwrap();
        assert!(check(&net).is_empty());
        // A branch with no line after it, and a job after a part that is not there.
        net.jobs[1].part = Part::Branch;
        net.jobs[0].after = vec![Line::ok("nope")];
        let found = check(&net);
        assert!(found.iter().any(|f| f.job.as_deref() == Some("b") && f.message.contains("孤立")), "{found:?}");
        assert!(found.iter().any(|f| f.job.as_deref() == Some("a") && f.message.contains("nope")), "{found:?}");
    }

    #[test]
    fn the_template_reads() {
        let net = jobnet::parse(&template("売上取込")).unwrap_or_else(|p| panic!("{p:?}"));
        assert_eq!(net.name.as_deref(), Some("売上取込"));
        assert_eq!(net.jobs.len(), 1);
        assert!(net.when.is_empty());
    }
}
