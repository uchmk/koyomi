//! The 履歴 tab of the run page: one job in the past runs of its net. The pure parts live here (which rows, what
//! the usual time is, how a range of days is cut into requests); the page draws them in `ui/mod.rs`.

use chrono::{Duration, NaiveDate, NaiveDateTime};
use koyomi_core::run::{How, Run, State};
use std::collections::BTreeSet;

/// How many days one `History` request asks for.
pub const PART_DAYS: i64 = 30;

/// The days the window reads without being asked to (the last 30 days of `past_rows`).
pub const READ_DAYS: i64 = 30;

/// One run the job was in.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub run: String,
    pub date: String,
    /// When the run started (the sort key).
    pub run_started: NaiveDateTime,
    pub started: Option<NaiveDateTime>,
    pub ended: Option<NaiveDateTime>,
    pub took: Option<Duration>,
    pub state: State,
    pub exit: Option<i32>,
    pub retried: u32,
    pub how: How,
}

/// The rows of job `job` (its full id) in the runs of net `net`, newest first. One row a run id: the first run of
/// an id wins, so the iterator lists the live runs before any copy.
pub fn job_history<'a>(runs: impl Iterator<Item = &'a Run>, net: &str, job: &str) -> Vec<Row> {
    let mut seen = BTreeSet::new();
    let mut rows: Vec<Row> = runs
        .filter(|r| r.jobnet == net)
        .filter(|r| seen.insert(r.id.as_str()))
        .filter_map(|r| {
            let j = r.job(job)?;
            Some(Row {
                run: r.id.clone(),
                date: r.date.clone(),
                run_started: r.started,
                started: j.started,
                ended: j.ended,
                took: j.duration(),
                state: j.state,
                exit: j.exit,
                retried: j.retried,
                how: r.how,
            })
        })
        .collect();
    rows.sort_by(|a, b| b.run_started.cmp(&a.run_started).then_with(|| b.run.cmp(&a.run)));
    rows
}

/// How many rows ended normally with a time, and the median of those times (the mean of the two middle ones when
/// there is an even number).
pub fn usual(rows: &[Row]) -> Option<(usize, Duration)> {
    let mut took: Vec<Duration> = rows.iter().filter(|r| r.state == State::Ok).filter_map(|r| r.took).collect();
    if took.is_empty() {
        return None;
    }
    took.sort();
    let n = took.len();
    let mid = if n % 2 == 1 { took[n / 2] } else { (took[n / 2 - 1] + took[n / 2]) / 2 };
    Some((n, mid))
}

/// The requests that read the days `near` to `far` back from `today` (both counted, `near <= far`): parts of at
/// most 30 days, the newest part first, each as `(from, to)` dates both included.
pub fn parts(today: NaiveDate, near: i64, far: i64) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut hi = near;
    while hi <= far {
        let lo = (hi + PART_DAYS - 1).min(far);
        out.push(((today - Duration::days(lo)).to_string(), (today - Duration::days(hi)).to_string()));
        hi = lo + 1;
    }
    out
}

/// The days the next press of the more button reads up to: 90, then 365, then nothing.
pub fn next_target(read_to_days: i64) -> Option<i64> {
    match read_to_days {
        d if d < 90 => Some(90),
        d if d < 365 => Some(365),
        _ => None,
    }
}

/// What the more button has read of the older days of one net.
#[derive(Clone, Debug)]
pub struct JobPast {
    /// The far PC (`[[remote]]` name) it was read from; none is this PC.
    pub link: Option<String>,
    pub net: String,
    /// The engine's day when it began (a new day drops it).
    pub day: NaiveDate,
    /// How many days back are read (30 before the first press).
    pub read_to_days: i64,
    /// The days the running press reads up to.
    pub target: i64,
    /// How many parts the running press has.
    pub total: usize,
    /// The runs of `net` the answers held.
    pub runs: Vec<Run>,
    /// The part asked for and not answered yet.
    pub asking: Option<(String, String)>,
    /// The parts still to ask, newest first.
    pub left: Vec<(String, String)>,
    /// The engine's words when a part failed.
    pub error: Option<String>,
}

impl JobPast {
    pub fn new(link: Option<String>, net: &str, day: NaiveDate) -> JobPast {
        JobPast {
            link,
            net: net.to_owned(),
            day,
            read_to_days: READ_DAYS,
            target: READ_DAYS,
            total: 0,
            runs: Vec::new(),
            asking: None,
            left: Vec::new(),
            error: None,
        }
    }

    /// Whether this is still for that link, net and day.
    pub fn is_for(&self, link: Option<&str>, net: &str, day: NaiveDate) -> bool {
        self.link.as_deref() == link && self.net == net && self.day == day
    }

    /// How many parts of the running press are answered.
    pub fn done(&self) -> usize {
        self.total.saturating_sub(self.left.len() + usize::from(self.asking.is_some()))
    }

    /// Keeps the runs of this net that are new.
    pub fn take(&mut self, runs: Vec<Run>) {
        let mut have: BTreeSet<String> = self.runs.iter().map(|r| r.id.clone()).collect();
        for r in runs {
            if r.jobnet == self.net && have.insert(r.id.clone()) {
                self.runs.push(r);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koyomi_core::jobnet::{Job, Jobnet};
    use koyomi_core::run::Outcome;

    fn at(day: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap().and_hms_opt(h, 0, 0).unwrap()
    }

    /// A run of `net` on `day` with job `a` (and `b`) that took `mins` and exited `code`; `a` is left unstarted
    /// when `mins` is none.
    fn run(id: &str, net: &str, day: u32, mins: Option<i64>, code: i32) -> Run {
        let jobs = vec![
            Job { id: "a".into(), run: vec!["x".into()], ..Default::default() },
            Job { id: "net1/file1".into(), run: vec!["x".into()], ..Default::default() },
        ];
        let n = Jobnet { jobs, ..Default::default() };
        let mut r = Run::new(id, net, "t", &format!("2026-10-{day:02}"), How::Manual, &n, at(day, 9));
        if let Some(m) = mins {
            r.start("a", at(day, 9));
            r.finish("a", Outcome::Exited(code), at(day, 9) + Duration::minutes(m));
            r.start("net1/file1", at(day, 9));
            r.finish("net1/file1", Outcome::Exited(0), at(day, 9) + Duration::minutes(1));
        }
        r
    }

    #[test]
    fn rows_are_the_same_net_and_the_same_full_job_id() {
        let rs = [run("r1", "g/n", 1, Some(2), 0), run("r2", "g/other", 2, Some(2), 0), run("r3", "g/n", 3, Some(2), 0)];
        let ids = |job: &str| job_history(rs.iter(), "g/n", job).into_iter().map(|r| r.run).collect::<Vec<_>>();
        assert_eq!(ids("a"), ["r3", "r1"], "another net's same id is not taken, newest first");
        assert_eq!(ids("net1/file1"), ["r3", "r1"], "a child net's full id matches");
        assert!(ids("file1").is_empty(), "the short id is not the full one");
    }

    #[test]
    fn one_row_a_run_and_the_first_copy_wins() {
        let live = run("r1", "g/n", 1, Some(5), 0);
        let mut copy = run("r1", "g/n", 1, Some(9), 3);
        copy.jobs[0].state = State::Error;
        let rows = job_history([&live, &copy].into_iter(), "g/n", "a");
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].state, rows[0].exit), (State::Ok, Some(0)));
        assert_eq!(rows[0].took, Some(Duration::minutes(5)));
    }

    #[test]
    fn a_job_that_never_started_has_empty_times() {
        let r = run("r1", "g/n", 1, None, 0);
        let rows = job_history(std::iter::once(&r), "g/n", "a");
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].started, rows[0].ended, rows[0].took, rows[0].exit), (None, None, None, None));
        assert_eq!(rows[0].state, r.jobs[0].state);
    }

    #[test]
    fn newest_run_first() {
        let rs = [run("r1", "g/n", 1, Some(1), 0), run("r3", "g/n", 3, Some(1), 0), run("r2", "g/n", 2, Some(1), 0)];
        let ids: Vec<String> = job_history(rs.iter(), "g/n", "a").into_iter().map(|r| r.run).collect();
        assert_eq!(ids, ["r3", "r2", "r1"]);
    }

    #[test]
    fn the_usual_time_is_the_median_of_the_normal_ends() {
        let rs = [
            run("r1", "g/n", 1, Some(10), 0),
            run("r2", "g/n", 2, Some(2), 0),
            run("r3", "g/n", 3, Some(30), 0),
            run("r4", "g/n", 4, Some(99), 1),
        ];
        let rows = job_history(rs.iter(), "g/n", "a");
        assert_eq!(rows.len(), 4);
        assert_eq!(usual(&rows), Some((3, Duration::minutes(10))), "odd: the middle, the failed one left out");
        let rows = job_history(rs[..2].iter(), "g/n", "a");
        assert_eq!(usual(&rows), Some((2, Duration::minutes(6))), "even: the mean of the two middles");
        let rows = job_history(rs[3..].iter(), "g/n", "a");
        assert_eq!(usual(&rows), None, "no normal end: nothing");
        assert_eq!(usual(&[]), None);
    }

    #[test]
    fn a_range_is_cut_into_parts_of_30_days_newest_first() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 27).unwrap();
        let back = |n: i64| (today - Duration::days(n)).to_string();
        let p = parts(today, 31, 90);
        assert_eq!(p, [(back(60), back(31)), (back(90), back(61))]);
        let p = parts(today, 91, 365);
        assert_eq!(p.len(), 10);
        assert_eq!(p[0], (back(120), back(91)));
        assert_eq!(p[9], (back(365), back(361)), "the last part is the short one, ending exactly");
        for w in p.windows(2) {
            let (older_to, newer_from) = (&w[1].1, &w[0].0);
            let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
            assert_eq!(d(older_to) + Duration::days(1), d(newer_from), "no day twice, none left out");
        }
    }

    #[test]
    fn the_button_goes_90_then_365_then_away() {
        assert_eq!(next_target(30), Some(90));
        assert_eq!(next_target(90), Some(365));
        assert_eq!(next_target(365), None);
    }

    #[test]
    fn progress_counts_the_answered_parts() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 27).unwrap();
        let mut p = JobPast::new(None, "g/n", today);
        p.total = 3;
        p.left = parts(today, 31, 90);
        p.asking = Some(("a".into(), "b".into()));
        assert_eq!(p.done(), 0);
        p.asking = None;
        p.left.pop();
        assert_eq!(p.done(), 2);
    }
}
