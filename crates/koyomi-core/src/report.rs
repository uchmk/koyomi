//! A month's report: for each job net how often it passed, how long it took on
//! average, and which of its jobs ended 異常 most (`koyomi report --month`).
//!
//! Only runs that are over count. A run that passed (正常 or 警告) and one that
//! ended 異常 make the success rate; the others (skipped, missed, not run …)
//! are counted apart so they neither help nor hurt it.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::run::{Run, State};

/// How many jobs a net's row names as the ones that failed most.
pub const WORST: usize = 3;

/// One job that ended 異常 in some runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Failing {
    pub id: String,
    pub times: usize,
}

/// One net's month.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    pub jobnet: String,
    pub title: String,
    /// Runs that are over.
    pub runs: usize,
    /// Of them, those that ended 正常 or 警告.
    pub passed: usize,
    /// Of them, those that ended 異常.
    pub failed: usize,
    /// Of them, those that neither passed nor failed (skipped, missed, not run).
    pub other: usize,
    /// Percent of `passed + failed` that passed; none when no run did either.
    pub rate: Option<u32>,
    /// Seconds the runs that ended 正常 took, on average.
    pub average: Option<i64>,
    /// The jobs that ended 異常 most often, most first.
    pub worst: Vec<Failing>,
}

/// The runs, a row per net (in the order of the nets' ids).
pub fn summarize(runs: &[Run]) -> Vec<Row> {
    let mut by: BTreeMap<&str, Vec<&Run>> = BTreeMap::new();
    for r in runs.iter().filter(|r| r.is_over()) {
        by.entry(r.jobnet.as_str()).or_default().push(r);
    }
    by.into_iter()
        .map(|(jobnet, rs)| {
            let passed = rs.iter().filter(|r| r.state.passed() && r.state != State::Skipped).count();
            let failed = rs.iter().filter(|r| r.state.is_error()).count();
            let secs: Vec<i64> = rs.iter().filter(|r| r.state == State::Ok).filter_map(|r| r.duration()).map(|d| d.num_seconds()).collect();
            let mut jobs: BTreeMap<&str, usize> = BTreeMap::new();
            for j in rs.iter().flat_map(|r| &r.jobs).filter(|j| j.state.is_error()) {
                *jobs.entry(j.id.as_str()).or_default() += 1;
            }
            let mut worst: Vec<Failing> = jobs.into_iter().map(|(id, times)| Failing { id: id.to_owned(), times }).collect();
            worst.sort_by(|a, b| b.times.cmp(&a.times).then_with(|| a.id.cmp(&b.id)));
            worst.truncate(WORST);
            Row {
                jobnet: jobnet.to_owned(),
                title: rs.last().map(|r| r.title.clone()).unwrap_or_default(),
                runs: rs.len(),
                passed,
                failed,
                other: rs.len() - passed - failed,
                rate: (passed + failed > 0).then(|| (passed as f64 * 100.0 / (passed + failed) as f64).round() as u32),
                average: (!secs.is_empty()).then(|| secs.iter().sum::<i64>() / secs.len() as i64),
                worst,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobnet::parse;
    use chrono::NaiveDateTime;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    /// A run of net `a` ended in `state` after `secs`, whose job `x` ended in `job`.
    fn run(net: &str, state: State, job: State, secs: i64) -> Run {
        let n = parse("format = 1\n[[job]]\nid = \"x\"\nrun = [\"x\"]\n").unwrap();
        let now = at("2026-10-01 00:00:00");
        let mut r = Run::new("id", net, net, "2026-10-01", crate::run::How::Manual, &n, now);
        r.jobs[0].state = job;
        r.state = state;
        r.ended = Some(now + chrono::Duration::seconds(secs));
        r
    }

    #[test]
    fn a_net_counts_its_rate_average_and_worst_jobs() {
        let rs = [
            run("a", State::Ok, State::Ok, 60),
            run("a", State::Ok, State::Ok, 120),
            run("a", State::Error, State::Error, 5),
            run("a", State::Skipped, State::Skipped, 0),
            run("b", State::Warn, State::Warn, 10),
        ];
        let got = summarize(&rs);
        assert_eq!(got.len(), 2);
        let a = &got[0];
        assert_eq!((a.runs, a.passed, a.failed, a.other, a.rate, a.average), (4, 2, 1, 1, Some(67), Some(90)));
        assert_eq!(a.worst, [Failing { id: "x".into(), times: 1 }]);
        let b = &got[1];
        assert_eq!((b.rate, b.average, b.worst.len()), (Some(100), None, 0));
    }

    #[test]
    fn runs_still_going_do_not_count() {
        let mut r = run("a", State::Running, State::Running, 0);
        r.ended = None;
        assert!(summarize(&[r]).is_empty());
    }
}
