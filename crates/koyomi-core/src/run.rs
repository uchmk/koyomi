//! One run of a job net: the copy of its jobs, their order, and the state of
//! each as it moves. The engine owns the processes; this module only says
//! which job may start next and what the run has become.
//!
//! The rules (DECISIONS.md, round 1 and D): a job with two or more `after`
//! starts when all of them have passed; one job that ends wrong makes the
//! net 異常 and the jobs after it 未実行; a run keeps the definition it started
//! with to the end.

use std::collections::BTreeMap;

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

use crate::jobnet::{Job, Jobnet, Retry};

/// A job's or a run's state, as the screen's table has them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// 正常 ✓.
    Ok,
    /// 警告 △.
    Warn,
    /// 異常 ✕.
    Error,
    /// 異常 ⏱: stopped at its `timeout`.
    Timeout,
    /// 異常 ■: a person stopped it.
    Killed,
    /// 実行中 ▶.
    Running,
    /// 異常ありで実行中 ▶✕ (a run only).
    RunningError,
    /// 実行待ち ○, with the reason beside it.
    Waiting,
    /// 保留 ⏸.
    Held,
    /// スキップ ⊘.
    Skipped,
    /// 時刻を過ぎた ↷.
    Missed,
    /// 未確認で止めた 🔒.
    Locked,
    /// 未実行 ·: never started (not yet, not reached, or the run ended before it).
    NotRun,
}

impl State {
    /// Every state, in the order the table lists them.
    pub const ALL: [State; 13] = [
        State::Ok,
        State::Warn,
        State::Error,
        State::Timeout,
        State::Killed,
        State::Running,
        State::RunningError,
        State::Waiting,
        State::Held,
        State::Skipped,
        State::Missed,
        State::Locked,
        State::NotRun,
    ];

    /// The screen's symbol. Stand-ins until ito-a11y has these states (uchmk/ito#75).
    pub fn mark(self) -> &'static str {
        match self {
            State::Ok => "✓",
            State::Warn => "△",
            State::Error => "✕",
            State::Timeout => "⏱",
            State::Killed => "■",
            State::Running => "▶",
            State::RunningError => "▶✕",
            State::Waiting => "○",
            State::Held => "⏸",
            State::Skipped => "⊘",
            State::Missed => "↷",
            State::Locked => "🔒",
            State::NotRun => "·",
        }
    }

    /// The key of its word in the language tables (`state.<key>`).
    pub fn key(self) -> &'static str {
        match self {
            State::Ok => "ok",
            State::Warn => "warn",
            State::Error => "error",
            State::Timeout => "timeout",
            State::Killed => "killed",
            State::Running => "running",
            State::RunningError => "running_error",
            State::Waiting => "waiting",
            State::Held => "held",
            State::Skipped => "skipped",
            State::Missed => "missed",
            State::Locked => "locked",
            State::NotRun => "not_run",
        }
    }

    /// Counts as 異常 for the net and the notification.
    pub fn is_error(self) -> bool {
        matches!(self, State::Error | State::Timeout | State::Killed | State::Locked)
    }

    /// Ended well enough for the jobs after it to start (a skipped job lets them through).
    pub fn passed(self) -> bool {
        matches!(self, State::Ok | State::Warn | State::Skipped)
    }

    /// Nothing more happens to it.
    pub fn is_final(self) -> bool {
        !matches!(self, State::Running | State::RunningError | State::Waiting | State::Held | State::NotRun)
    }

    /// The colour family the screen paints it with.
    pub fn tone(self) -> Tone {
        match self {
            State::Ok => Tone::Ok,
            State::Warn | State::Missed => Tone::Warn,
            State::Error | State::Timeout | State::Killed | State::Locked => Tone::Error,
            State::Running | State::RunningError => Tone::Running,
            State::Held => Tone::Held,
            State::Waiting | State::Skipped | State::NotRun => Tone::Dim,
        }
    }
}

/// The colour families of the table: green, orange, red, blue, purple, grey.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Error,
    Running,
    Held,
    Dim,
}

/// Why a run started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    /// `koyomi run` or 今すぐ流す.
    Manual,
    /// A `[[when]]` line came due.
    Schedule,
}

/// How a job ended, as the engine saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It ended by itself with this exit code (0 is 正常; the rest 異常 in this version).
    Exited(i32),
    /// A person stopped it.
    Killed,
    /// It ran past its `timeout`.
    Timeout,
    /// It never ran (the executable was not found, or the engine died with it).
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobRun {
    pub id: String,
    pub name: Option<String>,
    /// The command as it was when the run started.
    #[serde(default)]
    pub run: Vec<String>,
    #[serde(default)]
    pub dir: Option<String>,
    #[serde(default)]
    pub after: Vec<String>,
    /// Seconds it may run before the engine stops it.
    #[serde(default)]
    pub timeout: Option<u64>,
    /// How the engine runs it again after an abnormal end.
    #[serde(default)]
    pub retry: Option<Retry>,
    /// How many times the engine ran it again.
    #[serde(default)]
    pub retried: u32,
    pub state: State,
    pub started: Option<NaiveDateTime>,
    /// When it ended, or when it became certain that it will not run.
    pub ended: Option<NaiveDateTime>,
    pub exit: Option<i32>,
    /// The state's reason in words: `売上取込の終わりを待っています`, `強制終了`.
    #[serde(default)]
    pub reason: String,
}

impl JobRun {
    pub fn from_job(job: &Job) -> JobRun {
        JobRun {
            id: job.id.clone(),
            name: job.name.clone(),
            run: job.run.clone(),
            dir: job.dir.clone(),
            after: job.after.clone(),
            timeout: job.timeout,
            retry: job.retry,
            retried: 0,
            state: State::NotRun,
            started: None,
            ended: None,
            exit: None,
            reason: String::new(),
        }
    }

    /// The name, or the id when there is none.
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }

    /// Still may start.
    pub fn pending(&self) -> bool {
        matches!(self.state, State::NotRun | State::Waiting) && self.ended.is_none()
    }

    pub fn duration(&self) -> Option<chrono::Duration> {
        Some(self.ended? - self.started?)
    }
}

/// One run of one net.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// `YYYYMMDD-HHMMSS-NN`, unique on this machine.
    pub id: String,
    /// The definition's id, `<group>/<name>`.
    pub jobnet: String,
    /// What the screen calls the net.
    pub title: String,
    /// The business day, `YYYY-MM-DD`.
    pub date: String,
    pub how: How,
    /// The time the schedule named (none for a manual run).
    pub planned: Option<NaiveDateTime>,
    pub started: NaiveDateTime,
    pub ended: Option<NaiveDateTime>,
    pub state: State,
    pub jobs: Vec<JobRun>,
    /// Why a run that never started was not run (時刻を過ぎた, スキップ, ロック).
    #[serde(default)]
    pub reason: String,
}

impl Run {
    /// A run that has not started a job yet: every job is 未実行 or 実行待ち.
    pub fn new(id: &str, jobnet: &str, title: &str, date: &str, how: How, net: &Jobnet, now: NaiveDateTime) -> Run {
        let jobs = net.jobs.iter().map(JobRun::from_job).collect();
        Run::with_jobs(id, jobnet, title, date, how, jobs, now)
    }

    pub fn with_jobs(id: &str, jobnet: &str, title: &str, date: &str, how: How, jobs: Vec<JobRun>, now: NaiveDateTime) -> Run {
        let mut run = Run {
            id: id.to_owned(),
            jobnet: jobnet.to_owned(),
            title: title.to_owned(),
            date: date.to_owned(),
            how,
            planned: None,
            started: now,
            ended: None,
            state: State::Running,
            jobs,
            reason: String::new(),
        };
        run.settle(now);
        run
    }

    /// A run of the schedule that was not started: over at once, every job
    /// `NotRun`, with `state` (Missed, Skipped or Locked) and `reason` saying why.
    #[allow(clippy::too_many_arguments)]
    pub fn not_run(
        id: &str,
        jobnet: &str,
        title: &str,
        date: &str,
        planned: NaiveDateTime,
        state: State,
        reason: &str,
        net: &Jobnet,
        now: NaiveDateTime,
    ) -> Run {
        let jobs = net.jobs.iter().map(|j| JobRun { ended: Some(now), ..JobRun::from_job(j) }).collect();
        Run {
            id: id.to_owned(),
            jobnet: jobnet.to_owned(),
            title: title.to_owned(),
            date: date.to_owned(),
            how: How::Schedule,
            planned: Some(planned),
            started: now,
            ended: Some(now),
            state,
            jobs,
            reason: reason.to_owned(),
        }
    }

    pub fn job(&self, id: &str) -> Option<&JobRun> {
        self.jobs.iter().find(|j| j.id == id)
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.jobs.iter().position(|j| j.id == id)
    }

    pub fn is_over(&self) -> bool {
        self.ended.is_some()
    }

    /// A run of the schedule that never started (time passed, skipped, locked).
    pub fn not_started(&self) -> bool {
        matches!(self.state, State::Missed | State::Skipped | State::Locked)
    }

    pub fn duration(&self) -> Option<chrono::Duration> {
        Some(self.ended? - self.started)
    }

    /// How many jobs run right now.
    pub fn running(&self) -> usize {
        self.jobs.iter().filter(|j| j.state == State::Running).count()
    }

    /// The jobs the engine may start now: pending, and every `after` has passed.
    pub fn ready(&self) -> Vec<String> {
        if self.is_over() {
            return Vec::new();
        }
        self.jobs
            .iter()
            .filter(|j| j.pending())
            .filter(|j| j.after.iter().all(|a| self.job(a).is_some_and(|p| p.state.passed())))
            .map(|j| j.id.clone())
            .collect()
    }

    /// Marks `id` started.
    pub fn start(&mut self, id: &str, now: NaiveDateTime) {
        if let Some(i) = self.index(id) {
            let j = &mut self.jobs[i];
            j.state = State::Running;
            j.started = Some(now);
            j.reason.clear();
        }
        self.settle(now);
    }

    /// A ready job that cannot start yet because `limit` jobs already run.
    pub fn wait_for_slot(&mut self, id: &str, limit: usize) {
        if let Some(i) = self.index(id) {
            let j = &mut self.jobs[i];
            j.state = State::Waiting;
            j.reason = format!("同時に動かす数（{limit}）の空きを待っています");
        }
    }

    /// Says the engine ran `id` again `times` times, before it is finished.
    pub fn retried(&mut self, id: &str, times: u32) {
        if let Some(i) = self.index(id) {
            self.jobs[i].retried = times;
        }
    }

    /// Marks `id` ended, and everything that follows from it. The reason says
    /// how many times the job was run again, when it was.
    pub fn finish(&mut self, id: &str, outcome: Outcome, now: NaiveDateTime) {
        if let Some(i) = self.index(id) {
            let j = &mut self.jobs[i];
            if j.started.is_none() {
                j.started = Some(now);
            }
            j.ended = Some(now);
            j.exit = None;
            j.state = match outcome {
                Outcome::Exited(0) => {
                    j.exit = Some(0);
                    j.reason.clear();
                    State::Ok
                }
                Outcome::Exited(code) => {
                    j.exit = Some(code);
                    j.reason = format!("終了コード {code}");
                    State::Error
                }
                Outcome::Killed => {
                    j.reason = "強制終了".to_owned();
                    State::Killed
                }
                Outcome::Timeout => {
                    j.reason = match j.timeout {
                        Some(secs) => format!("打ち切り（{} を超えた）", crate::duration::text(secs)),
                        None => "打ち切り".to_owned(),
                    };
                    State::Timeout
                }
                Outcome::Failed => {
                    if j.reason.is_empty() {
                        j.reason = "起動できなかった".to_owned();
                    }
                    State::Error
                }
            };
            if j.retried > 0 {
                let note = format!("やり直し {} 回", j.retried);
                j.reason = if j.reason.is_empty() { note } else { format!("{}、{note}", j.reason) };
            }
        }
        self.settle(now);
    }

    /// [`finish`](Run::finish) with [`Outcome::Failed`] and this reason.
    pub fn fail(&mut self, id: &str, reason: &str, now: NaiveDateTime) {
        if let Some(i) = self.index(id) {
            self.jobs[i].reason = reason.to_owned();
        }
        self.finish(id, Outcome::Failed, now);
    }

    /// A person stops the whole run: what has not started will not. The jobs that run are
    /// finished by the engine with [`Outcome::Killed`] once their processes are gone.
    pub fn kill_pending(&mut self, now: NaiveDateTime) {
        for j in &mut self.jobs {
            if j.pending() {
                j.state = State::NotRun;
                j.reason = "強制終了".to_owned();
                j.ended = Some(now);
            }
        }
        self.settle(now);
    }

    /// The engine that ran this is gone: whatever was running is 異常, the rest 未実行.
    pub fn abandon(&mut self, now: NaiveDateTime) {
        for j in &mut self.jobs {
            if j.state == State::Running {
                j.state = State::Error;
                j.ended = Some(now);
                j.reason = "エンジンが止まった".to_owned();
            } else if j.pending() || j.state == State::Held {
                j.state = State::NotRun;
                j.ended = Some(now);
                j.reason = "エンジンが止まった".to_owned();
            }
        }
        self.settle(now);
    }

    /// Re-derives what follows from the jobs' states (history replay sets them by hand).
    pub fn resettle(&mut self, now: NaiveDateTime) {
        self.settle(now);
    }

    /// Recomputes the waiting reasons, the 未実行 after an error, and the run's own state.
    fn settle(&mut self, now: NaiveDateTime) {
        if self.is_over() {
            return;
        }
        let labels: BTreeMap<String, String> = self.jobs.iter().map(|j| (j.id.clone(), j.label().to_owned())).collect();
        loop {
            let states: BTreeMap<String, (State, bool)> = self.jobs.iter().map(|j| (j.id.clone(), (j.state, j.pending()))).collect();
            let mut changed = false;
            for j in &mut self.jobs {
                if !j.pending() {
                    continue;
                }
                // A predecessor that ended without passing, or that will never run.
                let blocked = j.after.iter().find(|a| match states.get(*a) {
                    Some((s, pending)) => !pending && !s.passed() && !matches!(s, State::Running | State::Held),
                    None => true,
                });
                if let Some(a) = blocked {
                    j.state = State::NotRun;
                    j.ended = Some(now);
                    j.reason = format!("先行の {} が異常終了", labels.get(a).unwrap_or(a));
                    changed = true;
                    continue;
                }
                let waiting_for: Vec<&str> = j
                    .after
                    .iter()
                    .filter(|a| !states.get(*a).is_some_and(|(s, _)| s.passed()))
                    .map(|a| labels.get(a).map_or(a.as_str(), String::as_str))
                    .collect();
                if !waiting_for.is_empty() {
                    j.state = State::Waiting;
                    j.reason = format!("{} の終わりを待っています", waiting_for.join("・"));
                }
            }
            if !changed {
                break;
            }
        }
        let any_error = self.jobs.iter().any(|j| j.state.is_error());
        let busy = self.jobs.iter().any(|j| j.state == State::Running || j.pending() || j.state == State::Held);
        self.state = match (busy, any_error) {
            (true, false) => State::Running,
            (true, true) => State::RunningError,
            (false, true) => State::Error,
            (false, false) => {
                if self.jobs.iter().any(|j| j.state == State::Warn) {
                    State::Warn
                } else {
                    State::Ok
                }
            }
        };
        if !busy {
            self.ended = Some(now);
        }
    }

    /// The jobs in layers for drawing: a job sits one layer after the last of its `after`.
    /// Each layer keeps the file's order.
    pub fn layers(&self) -> Vec<Vec<usize>> {
        layers(&self.jobs.iter().map(|j| (j.id.as_str(), j.after.as_slice())).collect::<Vec<_>>())
    }
}

/// See [`Run::layers`]. A job whose `after` cannot be placed (a cycle) goes in the first layer.
pub fn layers(jobs: &[(&str, &[String])]) -> Vec<Vec<usize>> {
    let index: BTreeMap<&str, usize> = jobs.iter().enumerate().map(|(i, (id, _))| (*id, i)).collect();
    let mut depth = vec![usize::MAX; jobs.len()];
    fn depth_of(i: usize, jobs: &[(&str, &[String])], index: &BTreeMap<&str, usize>, depth: &mut [usize], stack: &mut Vec<usize>) -> usize {
        if depth[i] != usize::MAX {
            return depth[i];
        }
        if stack.contains(&i) {
            return 0;
        }
        stack.push(i);
        let d =
            jobs[i].1.iter().filter_map(|a| index.get(a.as_str())).map(|&p| depth_of(p, jobs, index, depth, stack) + 1).max().unwrap_or(0);
        stack.pop();
        depth[i] = d;
        d
    }
    let mut stack = Vec::new();
    for i in 0..jobs.len() {
        depth_of(i, jobs, &index, &mut depth, &mut stack);
    }
    let mut out: Vec<Vec<usize>> = Vec::new();
    for (i, d) in depth.iter().enumerate() {
        while out.len() <= *d {
            out.push(Vec::new());
        }
        out[*d].push(i);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobnet::Job;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn job(id: &str, after: &[&str]) -> Job {
        Job {
            id: id.into(),
            name: None,
            run: vec!["x".into()],
            dir: None,
            after: after.iter().map(|s| s.to_string()).collect(),
            pos: None,
            timeout: None,
            retry: None,
        }
    }

    /// a → b, a → c, (b, c) → d.
    fn diamond() -> Run {
        let net =
            Jobnet { name: None, jobs: vec![job("a", &[]), job("b", &["a"]), job("c", &["a"]), job("d", &["b", "c"])], when: Vec::new() };
        Run::new("r1", "g/n", "n", "2026-10-30", How::Manual, &net, at("2026-10-30 09:00:00"))
    }

    #[test]
    fn a_job_with_two_predecessors_waits_for_both() {
        let mut r = diamond();
        assert_eq!(r.ready(), vec!["a"]);
        assert_eq!(r.job("d").unwrap().state, State::Waiting);
        assert_eq!(r.job("d").unwrap().reason, "b・c の終わりを待っています");
        r.start("a", at("2026-10-30 09:00:01"));
        assert_eq!(r.state, State::Running);
        r.finish("a", Outcome::Exited(0), at("2026-10-30 09:00:02"));
        let mut ready = r.ready();
        ready.sort();
        assert_eq!(ready, vec!["b", "c"]);
        r.start("b", at("2026-10-30 09:00:03"));
        r.finish("b", Outcome::Exited(0), at("2026-10-30 09:00:04"));
        assert_eq!(r.ready(), vec!["c"]);
        assert_eq!(r.job("d").unwrap().reason, "c の終わりを待っています");
        r.start("c", at("2026-10-30 09:00:05"));
        r.finish("c", Outcome::Exited(0), at("2026-10-30 09:00:06"));
        assert_eq!(r.ready(), vec!["d"]);
        r.start("d", at("2026-10-30 09:00:07"));
        r.finish("d", Outcome::Exited(0), at("2026-10-30 09:00:09"));
        assert_eq!(r.state, State::Ok);
        assert_eq!(r.ended, Some(at("2026-10-30 09:00:09")));
        assert_eq!(r.duration().unwrap().num_seconds(), 9);
    }

    #[test]
    fn one_error_makes_the_net_an_error_and_the_rest_not_run() {
        let mut r = diamond();
        r.start("a", at("2026-10-30 09:00:01"));
        r.finish("a", Outcome::Exited(0), at("2026-10-30 09:00:02"));
        r.start("b", at("2026-10-30 09:00:03"));
        r.start("c", at("2026-10-30 09:00:03"));
        r.finish("b", Outcome::Exited(3), at("2026-10-30 09:00:04"));
        // c still runs: the run is 異常ありで実行中, and d will never start.
        assert_eq!(r.state, State::RunningError);
        assert_eq!(r.job("b").unwrap().exit, Some(3));
        assert_eq!(r.job("d").unwrap().state, State::NotRun);
        assert_eq!(r.job("d").unwrap().reason, "先行の b が異常終了");
        assert!(r.ready().is_empty());
        r.finish("c", Outcome::Exited(0), at("2026-10-30 09:00:05"));
        assert_eq!(r.state, State::Error);
        assert!(r.is_over());
    }

    #[test]
    fn the_times_a_job_was_run_again_are_in_its_reason() {
        let mut r = diamond();
        r.start("a", at("2026-10-30 09:00:01"));
        r.retried("a", 2);
        r.finish("a", Outcome::Exited(3), at("2026-10-30 09:00:02"));
        assert_eq!(r.job("a").unwrap().state, State::Error);
        assert_eq!(r.job("a").unwrap().reason, "終了コード 3、やり直し 2 回");
        let mut r = diamond();
        r.start("a", at("2026-10-30 09:00:01"));
        r.retried("a", 1);
        r.finish("a", Outcome::Exited(0), at("2026-10-30 09:00:02"));
        assert_eq!(r.job("a").unwrap().state, State::Ok);
        assert_eq!(r.job("a").unwrap().reason, "やり直し 1 回");
        let mut r = diamond();
        r.start("a", at("2026-10-30 09:00:01"));
        r.finish("a", Outcome::Exited(0), at("2026-10-30 09:00:02"));
        assert_eq!(r.job("a").unwrap().reason, "");
    }

    #[test]
    fn timeout_and_kill_are_errors_told_apart_by_words() {
        let mut r = diamond();
        r.start("a", at("2026-10-30 09:00:01"));
        r.finish("a", Outcome::Timeout, at("2026-10-30 09:00:02"));
        assert_eq!(r.job("a").unwrap().state, State::Timeout);
        assert_eq!(r.job("a").unwrap().reason, "打ち切り");
        let mut t = diamond();
        t.jobs[0].timeout = Some(1800);
        t.start("a", at("2026-10-30 09:00:01"));
        t.finish("a", Outcome::Timeout, at("2026-10-30 09:30:02"));
        assert_eq!(t.job("a").unwrap().reason, "打ち切り（30m を超えた）");
        assert!(State::Timeout.is_error() && State::Killed.is_error());
        assert_eq!(State::Timeout.tone(), State::Error.tone());
        assert_eq!(r.state, State::Error);
        let mut k = diamond();
        k.start("a", at("2026-10-30 09:00:01"));
        k.kill_pending(at("2026-10-30 09:00:02"));
        assert_eq!(k.state, State::Running);
        k.finish("a", Outcome::Killed, at("2026-10-30 09:00:03"));
        assert_eq!(k.job("a").unwrap().state, State::Killed);
        assert_eq!(k.job("b").unwrap().state, State::NotRun);
        assert_eq!(k.job("b").unwrap().reason, "強制終了");
        assert_eq!(k.state, State::Error);
    }

    #[test]
    fn a_failed_start_is_an_error_with_its_reason() {
        let mut r = diamond();
        r.fail("a", "x が見つからない", at("2026-10-30 09:00:01"));
        assert_eq!(r.job("a").unwrap().state, State::Error);
        assert_eq!(r.job("a").unwrap().reason, "x が見つからない");
        assert_eq!(r.state, State::Error);
        assert!(r.jobs.iter().skip(1).all(|j| j.state == State::NotRun));
    }

    #[test]
    fn an_abandoned_run_is_an_error_everywhere() {
        let mut r = diamond();
        r.start("a", at("2026-10-30 09:00:01"));
        r.abandon(at("2026-10-30 09:30:00"));
        assert_eq!(r.job("a").unwrap().state, State::Error);
        assert_eq!(r.job("a").unwrap().reason, "エンジンが止まった");
        assert_eq!(r.job("d").unwrap().state, State::NotRun);
        assert_eq!(r.state, State::Error);
        assert_eq!(r.ended, Some(at("2026-10-30 09:30:00")));
    }

    #[test]
    fn waiting_for_a_slot_keeps_the_job_ready() {
        let mut r = diamond();
        r.wait_for_slot("a", 1);
        assert_eq!(r.job("a").unwrap().state, State::Waiting);
        assert_eq!(r.job("a").unwrap().reason, "同時に動かす数（1）の空きを待っています");
        assert_eq!(r.ready(), vec!["a"]);
    }

    #[test]
    fn layers_follow_the_longest_path() {
        let r = diamond();
        assert_eq!(r.layers(), vec![vec![0], vec![1, 2], vec![3]]);
        let net = Jobnet { name: None, jobs: vec![job("a", &[]), job("b", &["a"]), job("c", &["a", "b"])], when: Vec::new() };
        let r = Run::new("r", "g/n", "n", "2026-10-30", How::Manual, &net, at("2026-10-30 09:00:00"));
        assert_eq!(r.layers(), vec![vec![0], vec![1], vec![2]]);
    }

    #[test]
    fn every_state_has_a_mark_and_a_word_key() {
        let all = State::ALL;
        let marks: std::collections::BTreeSet<&str> = all.iter().map(|s| s.mark()).collect();
        assert_eq!(marks.len(), all.len());
        let keys: std::collections::BTreeSet<&str> = all.iter().map(|s| s.key()).collect();
        assert_eq!(keys.len(), all.len());
    }

    #[test]
    fn a_run_round_trips_through_toml() {
        let mut r = diamond();
        r.start("a", at("2026-10-30 09:00:01"));
        r.finish("a", Outcome::Exited(2), at("2026-10-30 09:00:02"));
        let text = toml::to_string(&r).unwrap();
        let back: Run = toml::from_str(&text).unwrap();
        assert_eq!(back, r);
    }
}
