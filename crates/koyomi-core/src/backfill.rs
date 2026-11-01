//! `koyomi run <net> --date A..B --only missing|failed`: which days of the range need running,
//! from the runs history has for the net (Airflow's backfill with "reprocess missing / failed").
//!
//! A run belongs to its business day (`Run::date`), not the day it started on. A record of a
//! time that never started (時刻を過ぎた, スキップ, 保留, 未確認で止めた) does not count as a run;
//! a run still going or waiting does.

use std::collections::BTreeSet;

use chrono::NaiveDate;

use crate::run::Run;

/// Which days `--only` keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Only {
    /// The days the net did not run on.
    Missing,
    /// Those, and the days whose latest run ended 異常.
    Failed,
}

impl Only {
    /// `missing` or `failed`.
    pub fn parse(text: &str) -> Option<Only> {
        match text {
            "missing" => Some(Only::Missing),
            "failed" => Some(Only::Failed),
            _ => None,
        }
    }
}

/// What one day of the range holds for the net.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Day<'a> {
    /// None of the net's times falls on it.
    NotPlanned,
    /// No run and no record.
    Nothing,
    /// Only records of times that never started: the latest of them.
    NotStarted(&'a Run),
    /// The latest run that started, going or over.
    Ran(&'a Run),
}

impl Day<'_> {
    /// Whether `--only` runs the day.
    pub fn picked(self, only: Only) -> bool {
        match self {
            Day::NotPlanned => false,
            Day::Nothing | Day::NotStarted(_) => true,
            Day::Ran(r) => only == Only::Failed && r.is_over() && r.state.is_error(),
        }
    }
}

/// Each of `days` as the runs of the net `id` leave it. With `planned`, a day not in it is
/// [`Day::NotPlanned`] whatever ran on it.
pub fn look<'a>(runs: &'a [Run], id: &str, days: &[NaiveDate], planned: Option<&BTreeSet<NaiveDate>>) -> Vec<(NaiveDate, Day<'a>)> {
    let latest = |a: &&Run, b: &&Run| (a.started, &a.id).cmp(&(b.started, &b.id));
    days.iter()
        .map(|&d| {
            if planned.is_some_and(|p| !p.contains(&d)) {
                return (d, Day::NotPlanned);
            }
            let date = d.format("%Y-%m-%d").to_string();
            let of_day = || runs.iter().filter(|r| r.jobnet == id && r.date == date);
            let never = |r: &&Run| r.not_started() && r.is_over();
            let day = match of_day().filter(|r| !never(r)).max_by(latest) {
                Some(r) => Day::Ran(r),
                None => of_day().max_by(latest).map_or(Day::Nothing, Day::NotStarted),
            };
            (d, day)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobnet::Jobnet;
    use crate::run::State;
    use chrono::NaiveDateTime;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    fn at(d: u32, h: u32) -> NaiveDateTime {
        day(d).and_hms_opt(h, 0, 0).unwrap()
    }

    /// A run of `net` for business day `d`, started on day `on` at hour `h`, ended as `state` (`None`: still going).
    fn run(net: &str, d: u32, on: u32, h: u32, state: State, over: bool) -> Run {
        let id = format!("{}-{h:02}0000-00", at(on, h).format("%Y%m%d"));
        let mut r = Run::not_run(&id, net, net, &day(d).to_string(), at(d, 2), state, "", &Jobnet::default(), at(on, h));
        if !over {
            r.ended = None;
        }
        r
    }

    fn picked(runs: &[Run], planned: Option<&BTreeSet<NaiveDate>>, only: Only) -> Vec<u32> {
        let days: Vec<NaiveDate> = (1..=7).map(day).collect();
        look(runs, "g/a", &days, planned).into_iter().filter(|(_, x)| x.picked(only)).map(|(d, _)| chrono::Datelike::day(&d)).collect()
    }

    #[test]
    fn only_takes_missing_or_failed() {
        assert_eq!(Only::parse("missing"), Some(Only::Missing));
        assert_eq!(Only::parse("failed"), Some(Only::Failed));
        assert_eq!(Only::parse("all"), None);
    }

    #[test]
    fn missing_days_are_those_without_a_run_of_their_business_day() {
        let runs = [
            // Day 1 ran on day 1; day 2 ran late, on day 3 (it still belongs to day 2).
            run("g/a", 1, 1, 2, State::Ok, true),
            run("g/a", 2, 3, 9, State::Error, true),
            // Day 3: only a time that passed while the PC was off, then a person skipped day 4.
            run("g/a", 3, 3, 8, State::Missed, true),
            run("g/a", 4, 4, 2, State::Skipped, true),
            // Day 5 is still going, day 6 waits; day 7 ran, but another net's run says nothing of it.
            run("g/a", 5, 5, 2, State::Running, false),
            run("g/a", 6, 6, 2, State::Waiting, false),
            run("g/b", 7, 7, 2, State::Ok, true),
        ];
        assert_eq!(picked(&runs, None, Only::Missing), [3, 4, 7]);
        assert_eq!(picked(&runs, None, Only::Failed), [2, 3, 4, 7]);
        let days: Vec<NaiveDate> = (1..=7).map(day).collect();
        let seen = look(&runs, "g/a", &days, None);
        assert_eq!(seen[2].1, Day::NotStarted(&runs[2]));
        assert_eq!(seen[1].1, Day::Ran(&runs[1]));
        assert_eq!(seen[6].1, Day::Nothing);
    }

    #[test]
    fn the_latest_run_of_the_day_decides_whether_it_failed() {
        let runs = [
            // Day 1 failed, then passed when run again; day 2 passed at 2:00 and failed at 17:00.
            run("g/a", 1, 1, 2, State::Timeout, true),
            run("g/a", 1, 2, 10, State::Warn, true),
            run("g/a", 2, 2, 2, State::Ok, true),
            run("g/a", 2, 2, 17, State::Killed, true),
            // Day 3 failed, and a later time of it was skipped: the failure still counts.
            run("g/a", 3, 3, 2, State::Error, true),
            run("g/a", 3, 3, 17, State::Skipped, true),
            // Day 4 has an error while still going: present until it ends.
            run("g/a", 4, 4, 2, State::RunningError, false),
            // Day 5 was locked (the definition was not approved): it never ran.
            run("g/a", 5, 5, 2, State::Locked, true),
            run("g/a", 6, 6, 2, State::Ok, true),
            run("g/a", 7, 7, 2, State::Ok, true),
        ];
        assert_eq!(picked(&runs, None, Only::Failed), [2, 3, 5]);
        assert_eq!(picked(&runs, None, Only::Missing), [5]);
    }

    #[test]
    fn with_a_plan_only_the_planned_days_count() {
        let runs = [run("g/a", 2, 2, 2, State::Error, true)];
        let planned: BTreeSet<NaiveDate> = [day(1), day(2), day(5)].into();
        assert_eq!(picked(&runs, Some(&planned), Only::Missing), [1, 5]);
        assert_eq!(picked(&runs, Some(&planned), Only::Failed), [1, 2, 5]);
        let days: Vec<NaiveDate> = (1..=7).map(day).collect();
        assert_eq!(look(&runs, "g/a", &days, Some(&planned))[3].1, Day::NotPlanned);
    }
}
