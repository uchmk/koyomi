//! One run of a job net: the copy of its jobs, their order, and the state of
//! each as it moves. The engine owns the processes; this module only says
//! which job may start next and what the run has become.
//!
//! The rules (DECISIONS.md, round 1 and D): a job with two or more `after`
//! starts when all of them have passed; one job that ends wrong makes the
//! net 異常 and the jobs after it 未実行; a run keeps the definition it started
//! with to the end.

use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

use crate::jobnet::{Encoding, Job, Jobnet, Judge, Line, On, Part, Range, Retry, Wait};

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
    /// `koyomi rerun` or 再実行: the jobs that passed before are carried over.
    Rerun,
    /// A file the net waits for came (`[[when]] file = …`).
    File,
    /// The engine started (`[[when]] on_start = true`).
    Start,
    /// A run of the net before it ended (`[[when]] after_jobnet = …`).
    After,
}

/// How a job ended, as the engine saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It ended by itself with this exit code; the job's `judge` says what it means ([`verdict`]).
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
    pub after: Vec<Line>,
    /// A job that runs, or a part of the flow (a branch, a join, a wait …).
    #[serde(default, skip_serializing_if = "Part::is_job")]
    pub part: Part,
    /// The file a `kind = "file"` part found: `KOYOMI_FILE` for the jobs after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Seconds it may run before the engine stops it.
    #[serde(default)]
    pub timeout: Option<u64>,
    /// How the engine runs it again after an abnormal end.
    #[serde(default)]
    pub retry: Option<Retry>,
    /// The shell and its script, when the job has them instead of `run`.
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub script: Option<String>,
    /// What each exit code means; without it only 0 is 正常.
    #[serde(default)]
    pub judge: Option<Judge>,
    /// Environment variables for the process (`secret:<name>` values are read when it starts).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// How its output is read into the log.
    #[serde(default, skip_serializing_if = "Encoding::is_auto")]
    pub encoding: Encoding,
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
            part: job.part.clone(),
            file: None,
            timeout: job.timeout,
            retry: job.retry,
            shell: job.shell.clone(),
            script: job.script.clone(),
            judge: job.judge.clone(),
            env: job.env.clone(),
            encoding: job.encoding,
            retried: 0,
            state: State::NotRun,
            started: None,
            ended: None,
            exit: None,
            reason: String::new(),
        }
    }

    /// What the engine starts: `run` as written, or the shell with the
    /// script as its one argument. The script goes as `-EncodedCommand`
    /// (base64 of UTF-16), so no quote or `$` in it is read by anything but
    /// the shell itself; no profile, nothing asked, and plain text in and
    /// out (without `-InputFormat Text`, PowerShell writes its errors as
    /// CLIXML when its input is not a console).
    pub fn command(&self) -> Vec<String> {
        match (self.shell.as_deref(), &self.script) {
            (Some(_), Some(script)) => {
                let exe = if cfg!(windows) { "powershell.exe" } else { "pwsh" };
                let mut v: Vec<String> = [exe, "-NoProfile", "-NonInteractive", "-InputFormat", "Text", "-EncodedCommand"]
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect();
                let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
                v.push(base64(&utf16));
                v
            }
            _ => self.run.clone(),
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

/// Standard base64 with padding, as `-EncodedCommand` wants it.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// What an exit code means for a job: 0 is 正常 and the rest 異常, unless
/// the job's `judge` lists the code as 警告 or 異常 (異常 wins where both do).
pub fn verdict(judge: Option<&Judge>, code: i32) -> State {
    match judge {
        Some(g) if g.error.iter().any(|r| r.has(code)) => State::Error,
        Some(g) if g.warn.iter().any(|r| r.has(code)) => State::Warn,
        _ if code == 0 => State::Ok,
        _ => State::Error,
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
    /// Why a run that never started was not run (時刻を過ぎた, スキップ, ロック),
    /// or that it started after its time (遅れて実行).
    #[serde(default)]
    pub reason: String,
    /// The file that started the run (`[[when]] file = …`): `KOYOMI_FILE` for
    /// a job no `kind = "file"` part comes before.
    #[serde(default)]
    pub file: Option<String>,
    /// The net's `lock` name while it runs: no other run with it goes at the same time.
    #[serde(default)]
    pub lock: Option<String>,
    /// The `[params]` values a person gave when starting it by hand (`--set`); the rest took their default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

/// How a run that started after its time (`missed = "once"`) begins its reason.
pub const LATE: &str = "遅れて実行";

/// How a job carried over from the previous run begins its reason (a rerun).
pub const CARRIED: &str = "前の回の結果";
/// The reason of a job a person skipped (`koyomi skip`): the ones after it go on as if it passed.
pub const SKIPPED: &str = "人がスキップした";

/// `from` and every job after it (by `after`, and on). `from` must be a job of `jobs`.
fn downstream(jobs: &[Job], from: &str) -> Result<BTreeSet<String>, String> {
    if !jobs.iter().any(|j| j.id == from) {
        return Err(format!("ジョブがない: {from}"));
    }
    let mut out: BTreeSet<String> = BTreeSet::from([from.to_owned()]);
    loop {
        let more: Vec<String> =
            jobs.iter().filter(|j| !out.contains(&j.id) && j.after.iter().any(|l| out.contains(&l.job))).map(|j| j.id.clone()).collect();
        if more.is_empty() {
            return Ok(out);
        }
        out.extend(more);
    }
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
            file: None,
            lock: None,
            params: BTreeMap::new(),
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
            file: None,
            lock: None,
            params: BTreeMap::new(),
        }
    }

    /// A run that starts at the job `from`: that job and everything after it
    /// run; the rest are ⊘ スキップ from the start (`koyomi run --from`).
    #[allow(clippy::too_many_arguments)]
    pub fn starting_at(
        id: &str,
        jobnet: &str,
        title: &str,
        date: &str,
        how: How,
        net: &Jobnet,
        from: &str,
        now: NaiveDateTime,
    ) -> Result<Run, String> {
        let redo = downstream(&net.jobs, from)?;
        let jobs = net
            .jobs
            .iter()
            .map(|j| {
                let mut jr = JobRun::from_job(j);
                if !redo.contains(&jr.id) {
                    jr.state = State::Skipped;
                    jr.ended = Some(now);
                    jr.reason = format!("{from} から流したので飛ばした");
                }
                jr
            })
            .collect();
        let mut run = Run::with_jobs(id, jobnet, title, date, how, jobs, now);
        run.reason = format!("{from} から");
        Ok(run)
    }

    /// `previous` (over) run again with `net`, the definition it ran: the jobs
    /// that passed (正常・警告・スキップ) are carried over as they ended, with
    /// [`CARRIED`] as their reason, and the rest run again. With `from`, that
    /// job and everything after it run again too. Finished work (an import, a
    /// send) is never done twice: a run where everything passed is refused
    /// unless `from` names a job.
    pub fn rerun(id: &str, previous: &Run, net: &Jobnet, from: Option<&str>, now: NaiveDateTime) -> Result<Run, String> {
        if !previous.is_over() {
            return Err(format!("{}: まだ終わっていない", previous.id));
        }
        let mut redo: BTreeSet<String> = previous.jobs.iter().filter(|j| !j.state.passed()).map(|j| j.id.clone()).collect();
        if let Some(from) = from {
            redo.extend(downstream(&net.jobs, from)?);
        }
        if redo.is_empty() {
            return Err(format!("{}: 全部済んでいる（流し直すなら --from <ジョブ>）", previous.id));
        }
        let mut jobs = Vec::with_capacity(net.jobs.len());
        for j in &net.jobs {
            let mut jr = JobRun::from_job(j);
            if !redo.contains(&jr.id) {
                let p = previous.job(&jr.id).ok_or_else(|| format!("{}: 前の回にジョブ {} はない", previous.id, jr.id))?;
                jr.state = p.state;
                jr.started = p.started;
                jr.ended = Some(p.ended.unwrap_or(now));
                jr.exit = p.exit;
                jr.retried = p.retried;
                jr.reason = if p.reason.is_empty() { CARRIED.to_owned() } else { format!("{CARRIED}: {}", p.reason) };
            }
            jobs.push(jr);
        }
        let mut run = Run::with_jobs(id, &previous.jobnet, &previous.title, &previous.date, How::Rerun, jobs, now);
        run.reason = match from {
            Some(f) => format!("再実行（{} を {f} から）", previous.id),
            None => format!("再実行（{}）", previous.id),
        };
        Ok(run)
    }

    /// Marks a run of the schedule that started after its time (`missed =
    /// "once"`): the reason reads 遅れて実行（2:00 の予定を 8:12 に流しました）.
    pub fn mark_late(&mut self) {
        if let Some(p) = self.planned {
            self.reason = format!("{LATE}（{} の予定を {} に流しました）", p.format("%-H:%M"), self.started.format("%-H:%M"));
        }
    }

    /// Started after its time (`missed = "once"`).
    pub fn is_late(&self) -> bool {
        self.reason.starts_with(LATE)
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
        matches!(self.state, State::Missed | State::Skipped | State::Locked | State::Held)
    }

    /// A person skips the job `id` before it starts: it ends ⊘ now with [`SKIPPED`], and the
    /// jobs after it go on as if it passed. A job that runs or has ended is refused.
    pub fn skip(&mut self, id: &str, now: NaiveDateTime) -> Result<(), String> {
        let j = self.jobs.iter_mut().find(|j| j.id == id).ok_or_else(|| format!("ジョブがない: {id}"))?;
        if j.state == State::Running {
            return Err(format!("{}: もう動いている", j.label()));
        }
        if !j.pending() {
            return Err(format!("{}: もう終わっている", j.label()));
        }
        j.state = State::Skipped;
        j.ended = Some(now);
        j.reason = SKIPPED.to_owned();
        self.settle(now);
        Ok(())
    }

    pub fn duration(&self) -> Option<chrono::Duration> {
        Some(self.ended? - self.started)
    }

    /// When this run is expected to end: it started at `started` and runs `average` long.
    /// None once it is over, or when it has already gone past that (no guess is better than a past time).
    pub fn expected_end(&self, average: chrono::Duration, now: NaiveDateTime) -> Option<NaiveDateTime> {
        if self.is_over() {
            return None;
        }
        let end = self.started + average;
        (end > now).then_some(end)
    }

    /// How many jobs run a process right now (a part that waits is not counted).
    pub fn running(&self) -> usize {
        self.jobs.iter().filter(|j| j.state == State::Running && j.part.is_job()).count()
    }

    /// The jobs the engine may start now: pending, and their lines are met.
    pub fn ready(&self) -> Vec<String> {
        if self.is_over() {
            return Vec::new();
        }
        let codes = codes_of(&self.jobs);
        self.jobs.iter().filter(|j| j.pending() && fate_of(j, &self.jobs, &codes).0 == Fate::Met).map(|j| j.id.clone()).collect()
    }

    /// The line `l` let its job's result through: it ended as the line says.
    pub fn line_met(&self, l: &Line) -> bool {
        line_fate(l, &self.jobs, &codes_of(&self.jobs)) == Fate::Met
    }

    /// A part that waits (`kind = "time"` / `"file"`) starts to: it reads as 実行中 with this reason.
    pub fn wait_on(&mut self, id: &str, reason: &str, now: NaiveDateTime) {
        if let Some(i) = self.index(id) {
            let j = &mut self.jobs[i];
            j.state = State::Running;
            j.started = Some(now);
            j.reason = reason.to_owned();
        }
        self.settle(now);
    }

    /// A part that runs no process ends: `state` with this reason, and the file it found when it is one.
    pub fn end_part(&mut self, id: &str, state: State, reason: &str, file: Option<String>, now: NaiveDateTime) {
        if let Some(i) = self.index(id) {
            let j = &mut self.jobs[i];
            if j.started.is_none() {
                j.started = Some(now);
            }
            j.state = state;
            j.ended = Some(now);
            j.reason = reason.to_owned();
            if file.is_some() {
                j.file = file;
            }
        }
        self.settle(now);
    }

    /// The last file a `kind = "file"` part found before `id` (by its lines,
    /// back to the start), else the file that started the run.
    pub fn file_for(&self, id: &str) -> Option<String> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![id.to_owned()];
        while let Some(x) = stack.pop() {
            let Some(j) = self.job(&x) else { continue };
            if x != id {
                if let Some(f) = &j.file {
                    return Some(f.clone());
                }
            }
            for l in &j.after {
                if seen.insert(l.job.clone()) {
                    stack.push(l.job.clone());
                }
            }
        }
        self.file.clone()
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
                Outcome::Exited(code) => {
                    j.exit = Some(code);
                    match verdict(j.judge.as_ref(), code) {
                        State::Ok => {
                            j.reason.clear();
                            State::Ok
                        }
                        State::Warn => {
                            j.reason = format!("終了コード {code}（警告）");
                            State::Warn
                        }
                        _ => {
                            j.reason = format!("終了コード {code}");
                            State::Error
                        }
                    }
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
            } else if j.state == State::Running && j.part.waits() {
                // A wait is no process: it ends here, as 強制終了.
                j.state = State::Killed;
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
        let codes = codes_of(&self.jobs);
        loop {
            let mut changed = false;
            for i in 0..self.jobs.len() {
                let j = &self.jobs[i];
                if !j.pending() {
                    continue;
                }
                // A station of a child jobnet ends when the child's jobs have.
                if let Part::Jobnet { .. } = j.part {
                    let prefix = format!("{}/", j.id);
                    let children: Vec<&JobRun> = self.jobs.iter().filter(|c| c.id.starts_with(&prefix)).collect();
                    if !children.is_empty() {
                        if children.iter().any(|c| c.pending() || matches!(c.state, State::Running | State::Held)) {
                            let j = &mut self.jobs[i];
                            if j.state != State::Waiting {
                                j.state = State::Waiting;
                                j.reason = "子ジョブネットの終わりを待っています".to_owned();
                            }
                            continue;
                        }
                        let (state, reason) = if let Some(e) = children.iter().find(|c| c.state.is_error()) {
                            (State::Error, format!("子ジョブネットの {} が異常終了", e.label()))
                        } else if children.iter().all(|c| c.state == State::NotRun) {
                            (State::NotRun, children[0].reason.clone())
                        } else if children.iter().any(|c| c.state == State::Warn) {
                            (State::Warn, "子ジョブネットに警告".to_owned())
                        } else {
                            (State::Ok, String::new())
                        };
                        let j = &mut self.jobs[i];
                        j.state = state;
                        j.started = j.started.or(Some(now));
                        j.ended = Some(now);
                        j.reason = reason;
                        changed = true;
                        continue;
                    }
                }
                let (fate, why) = fate_of(j, &self.jobs, &codes);
                match fate {
                    Fate::Dead => {
                        let j = &mut self.jobs[i];
                        j.state = State::NotRun;
                        j.ended = Some(now);
                        j.reason = why;
                        changed = true;
                    }
                    Fate::Open => {
                        let j = &mut self.jobs[i];
                        j.state = State::Waiting;
                        j.reason = why;
                    }
                    Fate::Met => {
                        // A part with no process and no clock ends here and now.
                        let (state, exit, reason) = match &j.part {
                            Part::Branch => match j.after.first().and_then(|l| self.job(&l.job)).and_then(|p| p.exit) {
                                Some(code) => (State::Ok, Some(code), format!("終了コード {code}")),
                                None => (State::Error, None, "終了コードが無い".to_owned()),
                            },
                            Part::Join { .. } | Part::Jobnet { .. } => (State::Ok, None, String::new()),
                            _ => continue,
                        };
                        let j = &mut self.jobs[i];
                        j.state = state;
                        j.started = Some(now);
                        j.ended = Some(now);
                        j.exit = exit;
                        j.reason = reason;
                        changed = true;
                    }
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

/// Whether a job's lines let it go on: yes, not yet, or never.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fate {
    Met,
    Open,
    Dead,
}

/// The ranges of every `code` line from each branch, over the run: an `else` line is met when none of them is.
fn codes_of(jobs: &[JobRun]) -> BTreeMap<String, Vec<Range>> {
    let mut out: BTreeMap<String, Vec<Range>> = BTreeMap::new();
    for j in jobs {
        for l in &j.after {
            if let On::Code(r) = &l.on {
                out.entry(l.job.clone()).or_default().extend(r.iter().copied());
            }
        }
    }
    out
}

/// What the state of the job a line comes from means for the line.
fn line_fate(l: &Line, jobs: &[JobRun], codes: &BTreeMap<String, Vec<Range>>) -> Fate {
    let Some(p) = jobs.iter().find(|j| j.id == l.job) else { return Fate::Dead };
    if p.pending() || matches!(p.state, State::Running | State::Held) {
        return Fate::Open;
    }
    let met = match &l.on {
        On::Ok => p.state.passed(),
        On::Error => p.state.is_error(),
        On::Any => p.state.passed() || p.state.is_error(),
        On::Code(r) => p.exit.is_some_and(|c| r.iter().any(|r| r.has(c))),
        On::Else => p.exit.is_some_and(|c| !codes.get(&l.job).is_some_and(|rs| rs.iter().any(|r| r.has(c)))),
    };
    if met {
        Fate::Met
    } else {
        Fate::Dead
    }
}

/// The fate of a job from all its lines (a `kind = "join"` with `wait = "any"`
/// goes on the first met line), with the reason for 実行待ち or 未実行.
fn fate_of(j: &JobRun, jobs: &[JobRun], codes: &BTreeMap<String, Vec<Range>>) -> (Fate, String) {
    let label = |id: &str| jobs.iter().find(|x| x.id == id).map_or(id.to_owned(), |x| x.label().to_owned());
    let fates: Vec<(&Line, Fate)> = j.after.iter().map(|l| (l, line_fate(l, jobs, codes))).collect();
    let any = matches!(j.part, Part::Join { wait: Wait::Any });
    let dead = |l: &Line| match l.on {
        On::Ok => format!("先行の {} が異常終了", label(&l.job)),
        _ => format!("{} の線の条件に合わなかった", label(&l.job)),
    };
    if any {
        if fates.iter().any(|(_, f)| *f == Fate::Met) {
            return (Fate::Met, String::new());
        }
        if !fates.is_empty() && fates.iter().all(|(_, f)| *f == Fate::Dead) {
            return (Fate::Dead, dead(fates[0].0));
        }
    } else if let Some((l, _)) = fates.iter().find(|(_, f)| *f == Fate::Dead) {
        return (Fate::Dead, dead(l));
    }
    let open: Vec<String> = fates.iter().filter(|(_, f)| *f != Fate::Met).map(|(l, _)| label(&l.job)).collect();
    if open.is_empty() {
        (Fate::Met, String::new())
    } else {
        (Fate::Open, format!("{} の終わりを待っています", open.join("・")))
    }
}

/// See [`Run::layers`]. A job whose `after` cannot be placed (a cycle) goes in the first layer.
pub fn layers(jobs: &[(&str, &[Line])]) -> Vec<Vec<usize>> {
    let index: BTreeMap<&str, usize> = jobs.iter().enumerate().map(|(i, (id, _))| (*id, i)).collect();
    let mut depth = vec![usize::MAX; jobs.len()];
    fn depth_of(i: usize, jobs: &[(&str, &[Line])], index: &BTreeMap<&str, usize>, depth: &mut [usize], stack: &mut Vec<usize>) -> usize {
        if depth[i] != usize::MAX {
            return depth[i];
        }
        if stack.contains(&i) {
            return 0;
        }
        stack.push(i);
        let d = jobs[i]
            .1
            .iter()
            .filter_map(|l| index.get(l.job.as_str()))
            .map(|&p| depth_of(p, jobs, index, depth, stack) + 1)
            .max()
            .unwrap_or(0);
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

/// The average time the last `last` runs of `jobnet` that ended 正常 took (none when no such run).
pub fn average_duration(runs: &[Run], jobnet: &str, last: usize) -> Option<chrono::Duration> {
    let mut done: Vec<&Run> = runs.iter().filter(|r| r.jobnet == jobnet && r.state == State::Ok && r.ended.is_some()).collect();
    done.sort_by_key(|r| r.started);
    let done = &done[done.len().saturating_sub(last)..];
    let total: i64 = done.iter().filter_map(|r| r.duration()).map(|d| d.num_seconds()).sum();
    (!done.is_empty()).then(|| chrono::Duration::seconds(total / done.len() as i64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobnet::{parse, ranges, Job};

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn job(id: &str, after: &[&str]) -> Job {
        Job { id: id.into(), run: vec!["x".into()], after: after.iter().map(|s| Line::ok(s)).collect(), ..Default::default() }
    }

    fn part(id: &str, part: Part, after: &[Line]) -> Job {
        Job { id: id.into(), part, after: after.to_vec(), ..Default::default() }
    }

    fn line(job: &str, on: On) -> Line {
        Line { job: job.into(), on }
    }

    fn net_of(jobs: Vec<Job>) -> Run {
        let net = Jobnet { jobs, ..Default::default() };
        Run::new("r1", "g/n", "n", "2026-10-30", How::Manual, &net, at("2026-10-30 09:00:00"))
    }

    #[test]
    fn the_average_and_expected_end_come_from_runs_that_passed() {
        let mut runs = Vec::new();
        for (i, secs) in [60, 120, 600].into_iter().enumerate() {
            let mut r = net_of(vec![job("a", &[])]);
            r.started = at("2026-10-30 09:00:00") + chrono::Duration::days(i as i64);
            r.ended = Some(r.started + chrono::Duration::seconds(secs));
            r.state = if secs == 600 { State::Error } else { State::Ok };
            runs.push(r);
        }
        // the failed one is left out
        assert_eq!(average_duration(&runs, "g/n", 10), Some(chrono::Duration::seconds(90)));
        // only the last one that passed
        assert_eq!(average_duration(&runs, "g/n", 1), Some(chrono::Duration::seconds(120)));
        assert_eq!(average_duration(&runs, "g/other", 10), None);
        let running = net_of(vec![job("a", &[])]);
        let avg = chrono::Duration::seconds(300);
        assert_eq!(running.expected_end(avg, at("2026-10-30 09:01:00")), Some(at("2026-10-30 09:05:00")));
        // already past the average: no guess
        assert_eq!(running.expected_end(avg, at("2026-10-30 09:10:00")), None);
    }

    /// an error line goes on when its job failed and dies when it passed; a plain line the other way round.
    #[test]
    fn an_error_line_fires_on_an_error_and_the_net_is_still_abnormal() {
        let jobs = || {
            vec![
                job("a", &[]),
                job("b", &["a"]),
                part("oops", Part::Notify, &[line("a", On::Error)]),
                part("oops2", Part::Notify, &[line("b", On::Error)]),
                job("end", &[]).after_any("b"),
            ]
        };
        let mut r = net_of(jobs());
        ended(&mut r, "a", 1, "2026-10-30 09:01:00");
        assert_eq!(r.job("b").unwrap().state, State::NotRun);
        assert_eq!(r.job("b").unwrap().reason, "先行の a が異常終了");
        assert_eq!(r.ready(), vec!["oops"], "the notify part may go, on a's error line");
        assert_eq!(r.job("oops2").unwrap().state, State::NotRun, "b never ran: its error line is dead");
        assert_eq!(r.job("oops2").unwrap().reason, "b の線の条件に合わなかった");
        assert_eq!(r.job("end").unwrap().state, State::NotRun, "b never ran: its any line is dead");
        assert_eq!(r.state, State::RunningError);
        r.end_part("oops", State::Ok, "通知した", None, at("2026-10-30 09:01:01"));
        assert_eq!(r.state, State::Error, "a notify on an error line does not make the run pass");
        assert!(r.is_over());
        let mut r = net_of(jobs());
        ended(&mut r, "a", 0, "2026-10-30 09:01:00");
        assert_eq!(r.ready(), vec!["b"]);
        assert_eq!(r.job("oops").unwrap().state, State::NotRun);
        assert_eq!(r.job("oops").unwrap().reason, "a の線の条件に合わなかった");
        assert_eq!(r.job("oops2").unwrap().state, State::Waiting, "b may still fail");
        ended(&mut r, "b", 0, "2026-10-30 09:02:00");
        assert_eq!(r.job("oops2").unwrap().state, State::NotRun);
        assert_eq!(r.ready(), vec!["end"], "the any line fires on a pass too");
        ended(&mut r, "end", 0, "2026-10-30 09:03:00");
        assert_eq!(r.state, State::Ok);
    }

    /// a branch ends at once with its job's exit code, and the `code` / `else` lines choose the way.
    #[test]
    fn a_branch_routes_by_the_exit_code() {
        let jobs = || {
            vec![
                job("chk", &[]),
                part("way", Part::Branch, &[line("chk", On::Any)]),
                job("one", &[]).after(line("way", On::Code(ranges("1..3").unwrap()))),
                job("rest", &[]).after(line("way", On::Else)),
            ]
        };
        let mut r = net_of(jobs());
        assert_eq!(r.job("way").unwrap().state, State::Waiting);
        ended(&mut r, "chk", 2, "2026-10-30 09:01:00");
        let way = r.job("way").unwrap();
        assert_eq!((way.state, way.exit, way.reason.as_str()), (State::Ok, Some(2), "終了コード 2"));
        assert_eq!(way.started, Some(at("2026-10-30 09:01:00")));
        assert_eq!(r.ready(), vec!["one"]);
        assert_eq!(r.job("rest").unwrap().state, State::NotRun);
        assert_eq!(r.job("rest").unwrap().reason, "way の線の条件に合わなかった");
        assert_eq!(r.state, State::RunningError, "chk's code 2 is still an error of the run");
        let mut r = net_of(jobs());
        ended(&mut r, "chk", 0, "2026-10-30 09:01:00");
        assert_eq!(r.ready(), vec!["rest"]);
        assert_eq!(r.job("one").unwrap().state, State::NotRun);
        ended(&mut r, "rest", 0, "2026-10-30 09:02:00");
        assert_eq!(r.state, State::Ok);
    }

    /// a join with `wait = "any"` goes on the first line met; `wait = "all"` on the last, and dies on the first dead line.
    #[test]
    fn a_join_waits_for_all_or_any() {
        let jobs =
            |wait| vec![job("a", &[]), job("b", &[]), part("j", Part::Join { wait }, &[Line::ok("a"), Line::ok("b")]), job("z", &["j"])];
        let mut r = net_of(jobs(Wait::Any));
        ended(&mut r, "a", 0, "2026-10-30 09:01:00");
        assert_eq!(r.job("j").unwrap().state, State::Ok);
        assert_eq!(r.ready(), vec!["b", "z"]);
        let mut r = net_of(jobs(Wait::Any));
        ended(&mut r, "a", 1, "2026-10-30 09:01:00");
        assert_eq!(r.job("j").unwrap().state, State::Waiting, "b may still pass");
        assert_eq!(r.job("j").unwrap().reason, "a・b の終わりを待っています");
        ended(&mut r, "b", 1, "2026-10-30 09:02:00");
        assert_eq!(r.job("j").unwrap().state, State::NotRun);
        assert_eq!(r.job("z").unwrap().state, State::NotRun);
        assert_eq!(r.state, State::Error);
        let mut r = net_of(jobs(Wait::All));
        ended(&mut r, "a", 0, "2026-10-30 09:01:00");
        assert_eq!(r.job("j").unwrap().state, State::Waiting);
        assert_eq!(r.job("j").unwrap().reason, "b の終わりを待っています");
        ended(&mut r, "b", 0, "2026-10-30 09:02:00");
        assert_eq!(r.job("j").unwrap().state, State::Ok);
        assert_eq!(r.ready(), vec!["z"]);
        let mut r = net_of(jobs(Wait::All));
        ended(&mut r, "a", 1, "2026-10-30 09:01:00");
        assert_eq!(r.job("j").unwrap().state, State::NotRun);
        assert_eq!(r.job("j").unwrap().reason, "先行の a が異常終了");
    }

    /// a wait (time, file) reads as 実行中 with its reason, ends by the engine, and is killed with the run.
    #[test]
    fn a_wait_runs_until_the_engine_ends_it() {
        let t = Part::Time { at: crate::when::time("06:00").unwrap(), days: 0 };
        let f = Part::File { file: "C:\\in\\*.csv".into(), settle: None };
        let mut r = net_of(vec![part("t", t.clone(), &[]), part("f", f.clone(), &[Line::ok("t")]), job("z", &["f"])]);
        assert_eq!(r.ready(), vec!["t"]);
        r.wait_on("t", "06:00 を待っています", at("2026-10-30 09:00:00"));
        assert_eq!(r.job("t").unwrap().state, State::Running);
        assert_eq!(r.running(), 0, "a wait holds no slot");
        assert_eq!(r.state, State::Running);
        r.end_part("t", State::Ok, "06:00 になった", None, at("2026-10-30 09:00:01"));
        assert_eq!(r.ready(), vec!["f"]);
        r.wait_on("f", "C:\\in\\*.csv を待っています", at("2026-10-30 09:00:01"));
        r.end_part("f", State::Ok, "C:\\in\\a.csv が来た", Some("C:\\in\\a.csv".into()), at("2026-10-30 09:00:02"));
        assert_eq!(r.file_for("z").as_deref(), Some("C:\\in\\a.csv"));
        assert_eq!(r.file_for("f"), None);
        assert_eq!(r.ready(), vec!["z"]);
        let mut r = net_of(vec![part("t", t, &[]), part("f", f, &[Line::ok("t")]), job("z", &["f"])]);
        r.wait_on("t", "06:00 を待っています", at("2026-10-30 09:00:00"));
        r.kill_pending(at("2026-10-30 09:00:03"));
        assert_eq!(r.job("t").unwrap().state, State::Killed);
        assert_eq!(r.job("t").unwrap().reason, "強制終了");
        assert!(r.is_over());
        assert_eq!(r.state, State::Error);
        let mut r = net_of(vec![part("f", Part::File { file: "x".into(), settle: None }, &[]), job("z", &["f"])]);
        r.wait_on("f", "x を待っています", at("2026-10-30 09:00:00"));
        r.end_part("f", State::Timeout, "打ち切り（1h を超えた）", None, at("2026-10-30 10:00:00"));
        assert_eq!(r.job("z").unwrap().state, State::NotRun);
        assert_eq!(r.state, State::Error);
    }

    /// the station of a child jobnet ends when the child's jobs have: an error of any of them is its error.
    #[test]
    fn a_child_jobnets_station_follows_its_jobs() {
        let jobs = || {
            vec![
                job("a", &[]),
                job("bk/c1", &["a"]),
                job("bk/c2", &["bk/c1"]),
                part("bk", Part::Jobnet { use_: "g/c".into() }, &[line("bk/c2", On::Any)]),
                job("z", &["bk"]),
            ]
        };
        let mut r = net_of(jobs());
        assert_eq!(r.job("bk").unwrap().reason, "子ジョブネットの終わりを待っています");
        ended(&mut r, "a", 0, "2026-10-30 09:01:00");
        ended(&mut r, "bk/c1", 0, "2026-10-30 09:02:00");
        assert_eq!(r.job("bk").unwrap().state, State::Waiting);
        ended(&mut r, "bk/c2", 0, "2026-10-30 09:03:00");
        assert_eq!(r.job("bk").unwrap().state, State::Ok);
        assert_eq!(r.ready(), vec!["z"]);
        let mut r = net_of(jobs());
        ended(&mut r, "a", 0, "2026-10-30 09:01:00");
        ended(&mut r, "bk/c1", 1, "2026-10-30 09:02:00");
        let bk = r.job("bk").unwrap();
        assert_eq!((bk.state, bk.reason.as_str()), (State::Error, "子ジョブネットの bk/c1 が異常終了"));
        assert_eq!(r.job("z").unwrap().state, State::NotRun);
        assert_eq!(r.state, State::Error);
        let mut r = net_of(jobs());
        ended(&mut r, "a", 1, "2026-10-30 09:01:00");
        let bk = r.job("bk").unwrap();
        assert_eq!((bk.state, bk.reason.as_str()), (State::NotRun, "先行の a が異常終了"));
        assert!(r.is_over());
    }

    impl Job {
        fn after(mut self, l: Line) -> Job {
            self.after.push(l);
            self
        }
        fn after_any(self, id: &str) -> Job {
            self.after(line(id, On::Any))
        }
    }

    /// a → b, a → c, (b, c) → d.
    fn diamond_net() -> Jobnet {
        Jobnet {
            name: None,
            jobs: vec![job("a", &[]), job("b", &["a"]), job("c", &["a"]), job("d", &["b", "c"])],
            when: Vec::new(),
            ..Default::default()
        }
    }

    fn diamond() -> Run {
        Run::new("r1", "g/n", "n", "2026-10-30", How::Manual, &diamond_net(), at("2026-10-30 09:00:00"))
    }

    fn ended(r: &mut Run, id: &str, code: i32, t: &str) {
        r.start(id, at(t));
        r.finish(id, Outcome::Exited(code), at(t));
    }

    #[test]
    fn a_rerun_carries_what_passed_and_runs_the_rest_again() {
        let now = at("2026-10-30 10:00:00");
        let net = diamond_net();
        let mut prev = diamond();
        ended(&mut prev, "a", 0, "2026-10-30 09:00:01");
        ended(&mut prev, "b", 0, "2026-10-30 09:00:02");
        ended(&mut prev, "c", 2, "2026-10-30 09:00:03");
        assert_eq!(prev.state, State::Error);
        assert_eq!(prev.job("d").unwrap().state, State::NotRun);
        let r = Run::rerun("r2", &prev, &net, None, now).unwrap();
        assert_eq!((r.how, r.state, r.reason.as_str()), (How::Rerun, State::Running, "再実行（r1）"));
        assert_eq!((r.jobnet.as_str(), r.title.as_str(), r.date.as_str()), ("g/n", "n", "2026-10-30"));
        let a = r.job("a").unwrap();
        assert_eq!((a.state, a.exit, a.reason.as_str()), (State::Ok, Some(0), CARRIED));
        assert_eq!((a.started, a.ended), (Some(at("2026-10-30 09:00:01")), Some(at("2026-10-30 09:00:01"))));
        assert_eq!(r.job("b").unwrap().state, State::Ok);
        assert_eq!(r.ready(), vec!["c"]);
        assert_eq!(r.job("d").unwrap().reason, "c の終わりを待っています");
        // From a chosen job: it and what follows run again, with whatever did not pass.
        let r = Run::rerun("r3", &prev, &net, Some("b"), now).unwrap();
        assert_eq!(r.reason, "再実行（r1 を b から）");
        assert_eq!(r.job("a").unwrap().reason, CARRIED);
        let mut ready = r.ready();
        ready.sort();
        assert_eq!(ready, vec!["b", "c"]);
        assert!(Run::rerun("r4", &prev, &net, Some("x"), now).unwrap_err().contains("ジョブがない: x"));
        // Not over yet: nothing to rerun. Everything passed: only from a job.
        assert!(Run::rerun("r5", &diamond(), &net, None, now).unwrap_err().contains("まだ終わっていない"));
        let mut done = diamond();
        for id in ["a", "b", "c", "d"] {
            ended(&mut done, id, 0, "2026-10-30 09:00:05");
        }
        assert!(Run::rerun("r6", &done, &net, None, now).unwrap_err().contains("全部済んでいる"));
        let r = Run::rerun("r7", &done, &net, Some("d"), now).unwrap();
        assert_eq!(r.ready(), vec!["d"]);
        assert_eq!(r.job("b").unwrap().reason, CARRIED);
    }

    #[test]
    fn a_warning_carried_over_keeps_its_reason() {
        let net = diamond_net();
        let mut prev = diamond();
        ended(&mut prev, "a", 0, "2026-10-30 09:00:01");
        prev.jobs[0].state = State::Warn;
        prev.jobs[0].reason = "終了コード 1（警告）".into();
        ended(&mut prev, "b", 3, "2026-10-30 09:00:02");
        ended(&mut prev, "c", 0, "2026-10-30 09:00:03");
        assert!(prev.is_over());
        let r = Run::rerun("r2", &prev, &net, None, at("2026-10-30 10:00:00")).unwrap();
        assert_eq!(r.job("a").unwrap().reason, "前の回の結果: 終了コード 1（警告）");
        assert_eq!(r.job("a").unwrap().state, State::Warn);
    }

    #[test]
    fn a_skipped_job_passes_and_a_running_or_ended_one_is_not_skipped() {
        let mut r = diamond();
        r.start("a", at("2026-10-30 09:00:00"));
        assert_eq!(r.skip("a", at("2026-10-30 09:00:01")).unwrap_err(), "a: もう動いている");
        assert_eq!(r.skip("x", at("2026-10-30 09:00:01")).unwrap_err(), "ジョブがない: x");
        // Waiting on a: skipped now, and d still waits for c.
        r.skip("b", at("2026-10-30 09:00:01")).unwrap();
        let b = r.job("b").unwrap();
        assert_eq!((b.state, b.ended, b.reason.as_str()), (State::Skipped, Some(at("2026-10-30 09:00:01")), SKIPPED));
        assert_eq!(r.job("d").unwrap().state, State::Waiting);
        assert_eq!(r.skip("b", at("2026-10-30 09:00:02")).unwrap_err(), "b: もう終わっている");
        r.finish("a", Outcome::Exited(0), at("2026-10-30 09:00:02"));
        assert_eq!(r.ready(), vec!["c"]);
        r.skip("c", at("2026-10-30 09:00:03")).unwrap();
        assert_eq!(r.ready(), vec!["d"]);
        r.start("d", at("2026-10-30 09:00:04"));
        r.finish("d", Outcome::Exited(0), at("2026-10-30 09:00:05"));
        assert!(r.is_over());
        assert_eq!(r.state, State::Ok);
        assert!(r.skip("d", at("2026-10-30 09:00:06")).is_err());
    }

    #[test]
    fn a_run_started_at_a_job_skips_the_ones_before() {
        let net = diamond_net();
        let now = at("2026-10-30 09:00:00");
        let r = Run::starting_at("r1", "g/n", "n", "2026-10-30", How::Manual, &net, "b", now).unwrap();
        let a = r.job("a").unwrap();
        assert_eq!((a.state, a.ended, a.reason.as_str()), (State::Skipped, Some(now), "b から流したので飛ばした"));
        assert_eq!(r.job("c").unwrap().state, State::Skipped);
        assert_eq!(r.ready(), vec!["b"]);
        assert_eq!(r.job("d").unwrap().reason, "b の終わりを待っています");
        assert_eq!((r.state, r.reason.as_str()), (State::Running, "b から"));
        assert!(Run::starting_at("r1", "g/n", "n", "2026-10-30", How::Manual, &net, "x", now).is_err());
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
    fn a_late_run_says_the_time_it_was_meant_for_and_the_time_it_started() {
        let net = parse("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n").unwrap();
        let now = at("2026-10-30 08:12:30");
        let mut run = Run::new("r1", "経理/月次締め", "月次締め", "2026-10-30", How::Schedule, &net, now);
        assert!(!run.is_late());
        run.mark_late();
        assert_eq!(run.reason, "", "no planned time: nothing to be late for");
        run.planned = Some(at("2026-10-30 02:00:00"));
        run.mark_late();
        assert_eq!(run.reason, "遅れて実行（2:00 の予定を 8:12 に流しました）");
        assert!(run.is_late());
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
        let net = Jobnet {
            name: None,
            jobs: vec![job("a", &[]), job("b", &["a"]), job("c", &["a", "b"])],
            when: Vec::new(),
            ..Default::default()
        };
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
    fn the_judge_says_what_an_exit_code_means() {
        assert_eq!((verdict(None, 0), verdict(None, 1), verdict(None, -1)), (State::Ok, State::Error, State::Error));
        let g = Judge { warn: ranges("1,3..5").unwrap(), error: ranges("2").unwrap(), ..Judge::default() };
        let v = |code| verdict(Some(&g), code);
        assert_eq!((v(0), v(1), v(2), v(3), v(5)), (State::Ok, State::Warn, State::Error, State::Warn, State::Warn));
        assert_eq!((v(6), v(-1)), (State::Error, State::Error));
        // 異常 wins where both lists have the code; 0 can be made 警告.
        let g = Judge { warn: ranges("0..9").unwrap(), error: ranges("4").unwrap(), ..Judge::default() };
        assert_eq!((verdict(Some(&g), 0), verdict(Some(&g), 4), verdict(Some(&g), 9)), (State::Warn, State::Error, State::Warn));
        // A 警告 end lets what follows run, and the run ends 警告.
        let mut a = job("a", &[]);
        a.judge = Some(Judge { warn: ranges("1").unwrap(), ..Judge::default() });
        let net = Jobnet { jobs: vec![a, job("b", &["a"])], ..Default::default() };
        let mut r = Run::new("r", "g/n", "n", "2026-10-30", How::Manual, &net, at("2026-10-30 09:00:00"));
        r.start("a", at("2026-10-30 09:00:01"));
        r.finish("a", Outcome::Exited(1), at("2026-10-30 09:00:02"));
        assert_eq!((r.job("a").unwrap().state, r.job("a").unwrap().reason.as_str()), (State::Warn, "終了コード 1（警告）"));
        assert_eq!(r.ready(), vec!["b"]);
        r.start("b", at("2026-10-30 09:00:03"));
        r.finish("b", Outcome::Exited(0), at("2026-10-30 09:00:04"));
        assert_eq!(r.state, State::Warn);
    }

    #[test]
    fn a_shell_job_starts_the_shell_with_its_script_as_one_argument() {
        let mut j = JobRun::from_job(&job("a", &[]));
        assert_eq!(j.command(), j.run);
        j.run.clear();
        j.shell = Some("powershell".to_owned());
        j.script = Some("Get-Date\nexit 1".to_owned());
        let exe = if cfg!(windows) { "powershell.exe" } else { "pwsh" };
        assert_eq!(
            j.command(),
            [exe, "-NoProfile", "-NonInteractive", "-InputFormat", "Text", "-EncodedCommand", "RwBlAHQALQBEAGEAdABlAAoAZQB4AGkAdAAgADEA"]
        );
        for (text, b64) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(text.as_bytes()), b64);
        }
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
