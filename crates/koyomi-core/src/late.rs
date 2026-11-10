//! Watching for runs that are late: one that has not begun by `late_start`,
//! one not over by `late_end`, and one that runs far longer than it usually
//! does (`late_slow`, on unless written false).
//!
//! `late_start` and `late_end` are a time of day (`"06:00"`: the first 06:00
//! at or after the run's time, so a net at 22:00 means the next morning) or a
//! length after it (`"30m"`). The run's time is the one the schedule named;
//! `late_end` of a run with none (by hand, a file) counts from its start.

use chrono::{Duration, NaiveDateTime, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};

use crate::jobnet::Jobnet;
use crate::msg;
use crate::run::{Run, State};

/// When a run is late from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Deadline {
    /// `"06:00"`: that time of day.
    At(NaiveTime),
    /// `"30m"`: that many seconds after.
    After(u64),
}

impl Deadline {
    /// `"06:00"` or `"30m"`.
    pub fn parse(text: &str) -> Result<Deadline, String> {
        if text.contains(':') {
            crate::when::time(text).map(Deadline::At)
        } else {
            crate::duration::parse(text).map(Deadline::After).map_err(|_| msg::line(msg::DEADLINE_BAD, &[text]))
        }
    }

    /// As the file writes it.
    pub fn text(&self) -> String {
        match self {
            Deadline::At(t) if t.second() == 0 => t.format("%H:%M").to_string(),
            Deadline::At(t) => t.format("%H:%M:%S").to_string(),
            Deadline::After(s) => crate::duration::text(*s),
        }
    }

    /// The moment it names for a run whose time is `from`.
    pub fn from(&self, from: NaiveDateTime) -> NaiveDateTime {
        match self {
            Deadline::At(t) => {
                let same = from.date().and_time(*t);
                if same >= from {
                    same
                } else {
                    same + Duration::days(1)
                }
            }
            Deadline::After(s) => from + Duration::seconds(*s as i64),
        }
    }
}

/// What a run was late in. Each is told once and marked on the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Delay {
    /// Began after `late_start` (or has not begun by it).
    Start,
    /// Not over by `late_end`.
    End,
    /// Has run at least twice its usual time.
    Slow,
    /// Is expected (begun + usual time) to end after `late_end`; told before it passes.
    Miss,
}

impl Delay {
    pub const ALL: [Delay; 4] = [Delay::Start, Delay::End, Delay::Slow, Delay::Miss];

    /// The key of its word in the language table (`late.start` …).
    pub fn key(self) -> &'static str {
        match self {
            Delay::Start => "late.start",
            Delay::End => "late.end",
            Delay::Slow => "late.slow",
            Delay::Miss => "late.miss",
        }
    }
}

/// How many runs that ended 正常 an average needs before `late_slow` trusts it.
pub const SLOW_RUNS: usize = 3;
/// How far over its usual time a run must be, at the least, to be slow: a run
/// that usually takes 5 seconds is not late at 10.
pub const SLOW_MARGIN: i64 = 60;

/// The moment a run that usually takes `usual` and began at `started` is slow from.
pub fn slow_from(started: NaiveDateTime, usual: Duration) -> NaiveDateTime {
    started + (usual * 2).max(usual + Duration::seconds(SLOW_MARGIN))
}

/// The moment the first of `r`'s jobs began, if one has.
pub fn begun(r: &Run) -> Option<NaiveDateTime> {
    r.jobs.iter().filter_map(|j| j.started).min()
}

/// What `r` of `net` is late in at `now` and is not marked with yet.
/// `usual` is how long the net usually takes (see [`crate::run::usual_duration`]).
pub fn overdue(net: &Jobnet, r: &Run, usual: Option<Duration>, now: NaiveDateTime) -> Vec<Delay> {
    let mut out = Vec::new();
    let running = !r.is_over();
    if let (Some(d), Some(p)) = (net.late_start, r.planned) {
        let by = d.from(p);
        let late = match begun(r) {
            Some(b) => b > by,
            None => running && now >= by,
        };
        if late && r.state != State::Held {
            out.push(Delay::Start);
        }
    }
    if let Some(d) = net.late_end {
        if running && now >= d.from(r.planned.unwrap_or(r.started)) {
            out.push(Delay::End);
        }
    }
    if let (Some(d), true, Some(u)) = (net.late_end, running, usual) {
        let by = d.from(r.planned.unwrap_or(r.started));
        // `usual` counts from the run's start, so this does too.
        if now < by && r.started + u > by {
            out.push(Delay::Miss);
        }
    }
    if let (true, true, Some(u)) = (net.watches_slow(), running, usual) {
        if let Some(b) = begun(r) {
            if now >= slow_from(b, u) {
                out.push(Delay::Slow);
            }
        }
    }
    out.retain(|d| !r.delays.contains(d));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobnet::Job;
    use crate::run::How;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn net(start: Option<&str>, end: Option<&str>) -> Jobnet {
        Jobnet {
            jobs: vec![Job { id: "a".into(), run: vec!["x".into()], ..Default::default() }],
            late_start: start.map(|s| Deadline::parse(s).unwrap()),
            late_end: end.map(|s| Deadline::parse(s).unwrap()),
            ..Default::default()
        }
    }

    fn run(net: &Jobnet, planned: &str) -> Run {
        let mut r = Run::new("r1", "g/n", "n", "2026-10-30", How::Schedule, net, at(planned));
        r.planned = Some(at(planned));
        r
    }

    #[test]
    fn a_deadline_reads_as_a_time_or_a_length() {
        assert_eq!(Deadline::parse("06:00"), Ok(Deadline::At(NaiveTime::from_hms_opt(6, 0, 0).unwrap())));
        assert_eq!(Deadline::parse("1h30m"), Ok(Deadline::After(5400)));
        assert!(Deadline::parse("30").is_err());
        assert!(Deadline::parse("25:00").is_err());
        assert_eq!(Deadline::parse("06:00").unwrap().text(), "06:00");
        assert_eq!(Deadline::parse("90m").unwrap().text(), "1h30m");
    }

    #[test]
    fn a_time_of_day_is_the_first_one_at_or_after_the_run() {
        let six = Deadline::parse("06:00").unwrap();
        assert_eq!(six.from(at("2026-10-30 22:00")), at("2026-10-31 06:00"));
        assert_eq!(six.from(at("2026-10-30 02:00")), at("2026-10-30 06:00"));
        assert_eq!(six.from(at("2026-10-30 06:00")), at("2026-10-30 06:00"));
        assert_eq!(Deadline::After(1800).from(at("2026-10-30 22:00")), at("2026-10-30 22:30"));
    }

    #[test]
    fn a_run_not_over_by_late_end_is_late_once() {
        let n = net(None, Some("06:00"));
        let mut r = run(&n, "2026-10-30 22:00");
        r.start("a", at("2026-10-30 22:00"));
        assert!(overdue(&n, &r, None, at("2026-10-31 05:59")).is_empty());
        assert_eq!(overdue(&n, &r, None, at("2026-10-31 06:00")), vec![Delay::End]);
        r.delays.push(Delay::End);
        assert!(overdue(&n, &r, None, at("2026-10-31 07:00")).is_empty(), "told once");
    }

    #[test]
    fn a_run_expected_to_end_after_late_end_is_told_before_it_passes() {
        let n = net(None, Some("06:00"));
        let mut r = run(&n, "2026-10-30 22:00");
        r.started = at("2026-10-31 04:00");
        r.start("a", at("2026-10-31 04:00"));
        let usual = Some(Duration::hours(3));
        assert_eq!(overdue(&n, &r, usual, at("2026-10-31 04:01")), vec![Delay::Miss]);
        assert!(overdue(&n, &r, Some(Duration::hours(1)), at("2026-10-31 04:01")).is_empty(), "expected in time");
        assert!(overdue(&n, &r, None, at("2026-10-31 04:01")).is_empty(), "no usual time yet");
        r.delays.push(Delay::Miss);
        assert!(overdue(&n, &r, usual, at("2026-10-31 04:02")).is_empty(), "told once");
        assert_eq!(overdue(&n, &r, usual, at("2026-10-31 06:00")), vec![Delay::End], "still told when it passes");
    }

    #[test]
    fn a_run_that_has_not_begun_by_late_start_is_late() {
        let n = net(Some("10m"), None);
        let r = run(&n, "2026-10-30 02:00");
        assert!(overdue(&n, &r, None, at("2026-10-30 02:09")).is_empty());
        assert_eq!(overdue(&n, &r, None, at("2026-10-30 02:10")), vec![Delay::Start]);
        let mut began = run(&n, "2026-10-30 02:00");
        began.start("a", at("2026-10-30 02:05"));
        assert!(overdue(&n, &began, None, at("2026-10-30 03:00")).is_empty(), "it began in time");
    }

    #[test]
    fn a_run_twice_its_usual_time_is_slow_but_not_a_short_one_by_seconds() {
        let n = net(None, None);
        let mut r = run(&n, "2026-10-30 02:00");
        r.start("a", at("2026-10-30 02:00"));
        let ten = Duration::minutes(10);
        assert!(overdue(&n, &r, Some(ten), at("2026-10-30 02:19")).is_empty());
        assert_eq!(overdue(&n, &r, Some(ten), at("2026-10-30 02:20")), vec![Delay::Slow]);
        assert!(overdue(&n, &r, Some(Duration::seconds(5)), at("2026-10-30 02:01")).is_empty(), "a minute over at the least");
        let off = Jobnet { late_slow: Some(false), ..n.clone() };
        assert!(overdue(&off, &r, Some(ten), at("2026-10-30 03:00")).is_empty());
        assert!(overdue(&n, &r, None, at("2026-10-30 03:00")).is_empty(), "no usual time yet");
    }

    #[test]
    fn the_keys_read_and_survive_a_round_trip() {
        let text = "format = 1
late_start = \"02:10\"
late_end = \"4h\"
late_slow = false

[[job]]
id = \"a\"
run = [\"x\"]
";
        let n = crate::jobnet::parse(text).unwrap();
        assert_eq!(n.late_start, Some(Deadline::At(NaiveTime::from_hms_opt(2, 10, 0).unwrap())));
        assert_eq!(n.late_end, Some(Deadline::After(4 * 3600)));
        assert!(!n.watches_slow());
        assert_eq!(crate::jobnet::parse(&crate::write::text(&n)).unwrap(), n);
        let bad = crate::jobnet::parse(
            "format = 1
late_end = \"6\"

[[job]]
id = \"a\"
run = [\"x\"]
",
        )
        .unwrap_err();
        assert!(bad[0].message.starts_with("late_end: "), "{}", bad[0].message);
        assert!(crate::jobnet::parse(
            "format = 1

[[job]]
id = \"a\"
run = [\"x\"]
"
        )
        .unwrap()
        .watches_slow());
    }
}
