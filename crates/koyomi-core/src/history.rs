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

use crate::jobnet::{Line, Part};
use crate::late::Delay;
use crate::run::{How, JobRun, Moved, Run, State, WentBy};

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
        /// The calendar the run went by; absent in a day written before this was recorded.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        went_by: Option<WentBy>,
        /// The run this one runs again; absent for a first run and in a day written before this was recorded.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rerun_of: Option<String>,
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
        /// How many times the engine ran it again.
        #[serde(default)]
        retried: u32,
        /// The files a `send` or `fetch` moved.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        moved: Vec<Moved>,
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
    /// The run was late (`late_start`, `late_end`, `late_slow`): told, and marked on it.
    Late {
        at: NaiveDateTime,
        run: String,
        delay: Delay,
    },
    /// Something a person did by hand (`koyomi hold`, a save, a run by hand …); belongs to no run.
    Op {
        at: NaiveDateTime,
        op: String,
        target: String,
        #[serde(default)]
        detail: String,
        /// Who did it over HTTP (`[[http.user]]`'s name, `default` for `[http]`'s token); empty
        /// for this PC's window and CLI.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        by: String,
    },
}

impl Event {
    pub fn run(&self) -> &str {
        match self {
            Event::Op { .. } => "",
            Event::RunStart { run, .. }
            | Event::JobStart { run, .. }
            | Event::JobEnd { run, .. }
            | Event::RunEnd { run, .. }
            | Event::Late { run, .. } => run,
        }
    }
}

/// A job as the run started: enough to draw it back (the expanded commands are in the run's `commands.toml`,
/// for a run from this version on, and the definition in its `definition.toml`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobBrief {
    pub id: String,
    pub name: Option<String>,
    #[serde(default)]
    pub after: Vec<Line>,
    #[serde(default, skip_serializing_if = "Part::is_job")]
    pub part: Part,
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
            Err(e) => Err(crate::msg::io_words(&e)),
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
        went_by: run.went_by.clone(),
        rerun_of: run.rerun_of.clone(),
        jobs: run
            .jobs
            .iter()
            .map(|j| JobBrief { id: j.id.clone(), name: j.name.clone(), after: j.after.clone(), part: j.part.clone() })
            .collect(),
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
        retried: job.retried,
        moved: job.moved.clone(),
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
            Event::RunStart { at, run, jobnet, title, date, how, planned, went_by, rerun_of, jobs } => {
                let jobs = jobs
                    .iter()
                    .map(|b| JobRun {
                        id: b.id.clone(),
                        name: b.name.clone(),
                        run: Vec::new(),
                        dir: None,
                        after: b.after.clone(),
                        part: b.part.clone(),
                        file: None,
                        timeout: None,
                        retry: None,
                        shell: None,
                        script: None,
                        judge: None,
                        env: Default::default(),
                        encoding: Default::default(),
                        to: Vec::new(),
                        run_as: None,
                        database: None,
                        host: None,
                        bucket: None,
                        retried: 0,
                        state: State::NotRun,
                        started: None,
                        ended: None,
                        exit: None,
                        reason: String::new(),
                        moved: Vec::new(),
                    })
                    .collect();
                let mut r = Run::with_jobs(run, jobnet, title, date, *how, jobs, *at);
                r.planned = *planned;
                r.went_by = went_by.clone();
                r.rerun_of = rerun_of.clone();
                runs.retain(|x| x.id != r.id);
                runs.push(r);
            }
            Event::Late { run, delay, .. } => {
                if let Some(r) = runs.iter_mut().find(|r| &r.id == run) {
                    if !r.delays.contains(delay) {
                        r.delays.push(*delay);
                    }
                }
            }
            Event::JobStart { at, run, job } => {
                if let Some(r) = runs.iter_mut().find(|r| &r.id == run) {
                    r.start(job, *at);
                }
            }
            Event::JobEnd { at, run, job, state, exit, reason, retried, moved } => {
                if let Some(r) = runs.iter_mut().find(|r| &r.id == run) {
                    if let Some(j) = r.jobs.iter_mut().find(|j| &j.id == job) {
                        if j.started.is_none() {
                            j.started = Some(*at);
                        }
                        j.ended = Some(*at);
                        j.state = *state;
                        j.exit = *exit;
                        j.reason = reason.clone();
                        j.retried = *retried;
                        j.moved = moved.clone();
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
            Event::Op { .. } => {}
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobnet::{Job, Jobnet, Line};
    use crate::run::Outcome;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn job(id: &str, after: &[&str]) -> Job {
        Job {
            id: id.into(),
            name: Some(format!("{id}の名")),
            run: vec!["x".into()],
            after: after.iter().map(|s| Line::ok(s)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn an_op_is_kept_in_the_day_and_builds_no_run() {
        let day = Day {
            events: vec![Event::Op {
                at: at("2026-10-30 09:00:00"),
                op: "hold".into(),
                target: "g/a".into(),
                detail: String::new(),
                by: String::new(),
            }],
        };
        let back = Day::parse(&day.to_toml()).unwrap();
        assert_eq!(back, day);
        assert!(back.runs().is_empty());
    }

    fn net() -> Jobnet {
        Jobnet { name: None, jobs: vec![job("a", &[]), job("b", &["a"]), job("c", &["b"])], when: Vec::new(), ..Default::default() }
    }

    #[test]
    fn a_finished_run_comes_back_the_same() {
        let mut day = Day::default();
        let mut r = Run::new("20261030-090000-00", "g/n", "n", "2026-10-30", How::Manual, &net(), at("2026-10-30 09:00:00"));
        day.events.push(run_start(&r));
        r.start("a", at("2026-10-30 09:00:01"));
        day.events.push(job_start(&r, r.job("a").unwrap()));
        r.finish("a", Outcome::Exited(0), at("2026-10-30 09:00:02"));
        let moved = vec![
            Moved { name: "a.csv".into(), bytes: 10, sha256: Some("0f".repeat(32)), matched: Some(true) },
            Moved { name: "b.csv".into(), bytes: 0, sha256: None, matched: None },
        ];
        r.set_moved("a", moved.clone());
        day.events.push(job_end(&r, r.job("a").unwrap()));
        r.start("b", at("2026-10-30 09:00:03"));
        day.events.push(job_start(&r, r.job("b").unwrap()));
        r.finish("b", Outcome::Exited(1), at("2026-10-30 09:00:04"));
        day.events.push(job_end(&r, r.job("b").unwrap()));
        day.events.push(run_end(&r));
        assert_eq!(r.state, State::Error);
        let text = day.to_toml();
        assert!(text.contains("[[event]]") && text.contains("kind = \"run_start\""), "{text}");
        assert!(text.contains("name = \"a.csv\"") && text.contains("matched = true"), "{text}");
        let back = Day::parse(&text).unwrap();
        assert_eq!(back, day);
        let runs = back.runs();
        assert_eq!(runs.len(), 1);
        let mut expect = r.clone();
        expect.jobs.iter_mut().for_each(|j| j.run.clear());
        assert_eq!(runs[0], expect);
        assert_eq!(runs[0].job("a").unwrap().moved, moved);
        assert!(runs[0].job("b").unwrap().moved.is_empty());
        assert_eq!(runs[0].job("c").unwrap().state, State::NotRun);
        assert_eq!(runs[0].job("c").unwrap().reason, "先行の bの名 が異常終了しました");
    }

    #[test]
    fn the_calendar_a_run_went_by_comes_back_and_an_old_event_reads_as_not_recorded() {
        for went in [None, Some(WentBy::Every), Some(WentBy::Calendar("会社".into()))] {
            let mut r = Run::new("r", "g/n", "n", "2026-10-30", How::Manual, &net(), at("2026-10-30 09:00:00"));
            r.went_by = went.clone();
            let day = Day { events: vec![run_start(&r)] };
            let text = day.to_toml();
            assert_eq!(text.contains("went_by"), went.is_some(), "{text}");
            assert_eq!(Day::parse(&text).unwrap_or_else(|e| panic!("{e}: {went:?} {text}")).runs()[0].went_by, went);
        }
        let old = "[[event]]\nkind = \"run_start\"\nat = \"2026-10-30T09:00:00\"\nrun = \"r\"\njobnet = \"g/n\"\ntitle = \"n\"\ndate = \"2026-10-30\"\nhow = \"manual\"\njobs = []\n";
        assert_eq!(Day::parse(old).unwrap().runs()[0].went_by, None);
        // A day written before an approval had a message holds a bare `kind = "approval"` in its brief.
        let bare = old.replace("jobs = []\n", "[[event.jobs]]\nid = \"ok\"\npart = { kind = \"approval\" }\n");
        let runs = Day::parse(&bare).unwrap().runs();
        assert_eq!(runs[0].jobs[0].part, Part::Approval { message: None });
    }

    #[test]
    fn the_run_a_rerun_redoes_comes_back_and_an_old_event_reads_as_none() {
        let mut r = Run::new("r2", "g/n", "n", "2026-10-30", How::Rerun, &net(), at("2026-10-30 09:00:00"));
        r.rerun_of = Some("r1".into());
        let text = Day { events: vec![run_start(&r)] }.to_toml();
        assert!(text.contains("rerun_of = \"r1\""), "{text}");
        assert_eq!(Day::parse(&text).unwrap().runs()[0].rerun_of.as_deref(), Some("r1"));
        let first = Run::new("r1", "g/n", "n", "2026-10-30", How::Manual, &net(), at("2026-10-30 09:00:00"));
        assert!(!Day { events: vec![run_start(&first)] }.to_toml().contains("rerun_of"));
        let old = "[[event]]
kind = \"run_start\"
at = \"2026-10-30T09:00:00\"
run = \"r\"
jobnet = \"g/n\"
title = \"n\"
date = \"2026-10-30\"
how = \"rerun\"
jobs = []
";
        assert_eq!(Day::parse(old).unwrap().runs()[0].rerun_of, None);
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
