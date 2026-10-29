//! The day's history: `history/<year>/<month>/<day>.toml`, a list of
//! `[[event]]` the engine appends to as runs start and end, and the runs
//! built back from it (what `koyomi status --today` and the 今日 list show,
//! and what the engine reads at start to find the runs it left behind).
//!
//! A run's events all go in the file of the day it started, so a run that
//! crosses midnight is in one piece.

use std::path::{Path, PathBuf};

use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

use crate::run::{How, JobRun, Run, State};

/// One line of history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    RunStart {
        at: NaiveDateTime,
        run: String,
        jobnet: String,
        title: String,
        date: String,
        how: How,
        planned: Option<NaiveDateTime>,
        jobs: Vec<JobBrief>,
    },
    JobStart {
        at: NaiveDateTime,
        run: String,
        job: String,
    },
    JobEnd {
        at: NaiveDateTime,
        run: String,
        job: String,
        state: State,
        exit: Option<i32>,
        reason: String,
    },
    /// The end, with every job's last state (the ones that never ran have theirs only here).
    RunEnd {
        at: NaiveDateTime,
        run: String,
        state: State,
        jobs: Vec<JobLast>,
        /// Why a run of the schedule was not started.
        #[serde(default)]
        reason: String,
    },
}

impl Event {
    pub fn run(&self) -> &str {
        match self {
            Event::RunStart { run, .. } | Event::JobStart { run, .. } | Event::JobEnd { run, .. } | Event::RunEnd { run, .. } => run,
        }
    }
}

/// A job as the run started: enough to draw it back (the command is in the run's `definition.toml`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobBrief {
    pub id: String,
    pub name: Option<String>,
    #[serde(default)]
    pub after: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobLast {
    pub id: String,
    pub state: State,
    #[serde(default)]
    pub reason: String,
}

/// The file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Day {
    #[serde(default, rename = "event")]
    pub events: Vec<Event>,
}

impl Day {
    pub fn parse(text: &str) -> Result<Day, String> {
        toml::from_str(text).map_err(|e| e.message().to_owned())
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// The file, or an empty day when there is none.
    pub fn load(path: &Path) -> Result<Day, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Day::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Day::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// The runs as the events leave them, oldest first.
    pub fn runs(&self) -> Vec<Run> {
        replay(&self.events)
    }
}

/// `<dir>/<year>/<month>/<day>.toml`.
pub fn path(dir: &Path, day: NaiveDate) -> PathBuf {
    dir.join(day.format("%Y").to_string()).join(day.format("%m").to_string()).join(day.format("%d.toml").to_string())
}

pub fn run_start(run: &Run) -> Event {
    Event::RunStart {
        at: run.started,
        run: run.id.clone(),
        jobnet: run.jobnet.clone(),
        title: run.title.clone(),
        date: run.date.clone(),
        how: run.how,
        planned: run.planned,
        jobs: run.jobs.iter().map(|j| JobBrief { id: j.id.clone(), name: j.name.clone(), after: j.after.clone() }).collect(),
    }
}

pub fn job_start(run: &Run, job: &JobRun) -> Event {
    Event::JobStart { at: job.started.unwrap_or(run.started), run: run.id.clone(), job: job.id.clone() }
}

pub fn job_end(run: &Run, job: &JobRun) -> Event {
    Event::JobEnd {
        at: job.ended.or(job.started).unwrap_or(run.started),
        run: run.id.clone(),
        job: job.id.clone(),
        state: job.state,
        exit: job.exit,
        reason: job.reason.clone(),
    }
}

pub fn run_end(run: &Run) -> Event {
    Event::RunEnd {
        at: run.ended.unwrap_or(run.started),
        run: run.id.clone(),
        state: run.state,
        jobs: run.jobs.iter().map(|j| JobLast { id: j.id.clone(), state: j.state, reason: j.reason.clone() }).collect(),
        reason: run.reason.clone(),
    }
}

/// Builds the runs back. An event for a run that never started is dropped.
pub fn replay(events: &[Event]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for ev in events {
        match ev {
            Event::RunStart { at, run, jobnet, title, date, how, planned, jobs } => {
                let jobs = jobs
                    .iter()
                    .map(|b| JobRun {
                        id: b.id.clone(),
                        name: b.name.clone(),
                        run: Vec::new(),
                        dir: None,
                        after: b.after.clone(),
                        timeout: None,
                        state: State::NotRun,
                        started: None,
                        ended: None,
                        exit: None,
                        reason: String::new(),
                    })
                    .collect();
                let mut r = Run::with_jobs(run, jobnet, title, date, *how, jobs, *at);
                r.planned = *planned;
                runs.retain(|x| x.id != r.id);
                runs.push(r);
            }
            Event::JobStart { at, run, job } => {
                if let Some(r) = runs.iter_mut().find(|r| &r.id == run) {
                    r.start(job, *at);
                }
            }
            Event::JobEnd { at, run, job, state, exit, reason } => {
                if let Some(r) = runs.iter_mut().find(|r| &r.id == run) {
                    if let Some(j) = r.jobs.iter_mut().find(|j| &j.id == job) {
                        if j.started.is_none() {
                            j.started = Some(*at);
                        }
                        j.ended = Some(*at);
                        j.state = *state;
                        j.exit = *exit;
                        j.reason = reason.clone();
                    }
                    // Re-derive what follows (未実行 after an error, the waits).
                    r.resettle(*at);
                }
            }
            Event::RunEnd { at, run, state, jobs, reason } => {
                if let Some(r) = runs.iter_mut().find(|r| &r.id == run) {
                    for last in jobs {
                        if let Some(j) = r.jobs.iter_mut().find(|j| j.id == last.id) {
                            j.state = last.state;
                            j.reason = last.reason.clone();
                            if j.ended.is_none() && j.state.is_final() {
                                j.ended = Some(*at);
                            }
                        }
                    }
                    r.state = *state;
                    r.ended = Some(*at);
                    r.reason = reason.clone();
                }
            }
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobnet::{Job, Jobnet};
    use crate::run::Outcome;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn job(id: &str, after: &[&str]) -> Job {
        Job {
            id: id.into(),
            name: Some(format!("{id}の名")),
            run: vec!["x".into()],
            dir: None,
            after: after.iter().map(|s| s.to_string()).collect(),
            pos: None,
            timeout: None,
        }
    }

    fn net() -> Jobnet {
        Jobnet { name: None, jobs: vec![job("a", &[]), job("b", &["a"]), job("c", &["b"])], when: Vec::new() }
    }

    #[test]
    fn a_finished_run_comes_back_the_same() {
        let mut day = Day::default();
        let mut r = Run::new("20261030-090000-00", "g/n", "n", "2026-10-30", How::Manual, &net(), at("2026-10-30 09:00:00"));
        day.events.push(run_start(&r));
        r.start("a", at("2026-10-30 09:00:01"));
        day.events.push(job_start(&r, r.job("a").unwrap()));
        r.finish("a", Outcome::Exited(0), at("2026-10-30 09:00:02"));
        day.events.push(job_end(&r, r.job("a").unwrap()));
        r.start("b", at("2026-10-30 09:00:03"));
        day.events.push(job_start(&r, r.job("b").unwrap()));
        r.finish("b", Outcome::Exited(1), at("2026-10-30 09:00:04"));
        day.events.push(job_end(&r, r.job("b").unwrap()));
        day.events.push(run_end(&r));
        assert_eq!(r.state, State::Error);
        let text = day.to_toml();
        assert!(text.contains("[[event]]") && text.contains("kind = \"run_start\""), "{text}");
        let back = Day::parse(&text).unwrap();
        assert_eq!(back, day);
        let runs = back.runs();
        assert_eq!(runs.len(), 1);
        let mut expect = r.clone();
        expect.jobs.iter_mut().for_each(|j| j.run.clear());
        assert_eq!(runs[0], expect);
        assert_eq!(runs[0].job("c").unwrap().state, State::NotRun);
        assert_eq!(runs[0].job("c").unwrap().reason, "先行の bの名 が異常終了");
    }

    #[test]
    fn an_unfinished_run_comes_back_running() {
        let mut day = Day::default();
        let mut r = Run::new("r", "g/n", "n", "2026-10-30", How::Manual, &net(), at("2026-10-30 09:00:00"));
        day.events.push(run_start(&r));
        r.start("a", at("2026-10-30 09:00:01"));
        day.events.push(job_start(&r, r.job("a").unwrap()));
        let runs = Day::parse(&day.to_toml()).unwrap().runs();
        assert_eq!(runs[0].state, State::Running);
        assert_eq!(runs[0].job("a").unwrap().state, State::Running);
        assert!(!runs[0].is_over());
    }

    #[test]
    fn the_path_has_year_month_day_folders() {
        let p = path(Path::new("h"), NaiveDate::from_ymd_opt(2026, 3, 5).unwrap());
        assert_eq!(p, Path::new("h").join("2026").join("03").join("05.toml"));
    }
}
