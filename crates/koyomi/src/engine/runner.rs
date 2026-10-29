//! The runs: the engine's jobs as processes, their logs, and the day's history.
//!
//! Each job runs on a thread of its own through `ito_proc::run_lines`, which
//! writes the lines to `logs/<day>/<run>/<job>.log` and tells the main loop
//! how it ended. The order between jobs is `koyomi_core::run`'s; this file
//! only starts what it says is ready and keeps the files current.
//!
//! A job is stopped through one flag, whoever asks: `kill` from a person, or
//! the timer a `timeout` starts. `ito_proc` then kills the whole process tree.
//! Which of the two asked first is kept beside the flag, so the job ends as
//! 強制終了 or 打ち切り and never as the other.
//!
//! A job with `retry` that ends abnormally (an exit code other than 0, or a
//! start that failed) is run again on the same thread after `wait`, up to
//! `count` times, into the same log; each time has its own `timeout`. A
//! 打ち切り or 強制終了 ends it, and a kill during the wait does too.
//!
//! A time of the schedule that comes while the net's last run goes may wait
//! for it (`overlap = "queue"`): the wait is kept here, not in the history,
//! and the time starts in the `tick` that sees the run end. One time waits
//! per net; the engine records the rest as スキップ when it stops.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDate, NaiveDateTime};
use ito_common::kept;
use koyomi_core::config::Config;
use koyomi_core::history::{self, Day};
use koyomi_core::jobnet::{Judge, Retry};
use koyomi_core::run::{verdict, How, JobRun, Outcome, Run, State};
use koyomi_core::scan::Def;

/// How much of a log `koyomi log` and the window get: the last 64 KB.
pub const TAIL: u64 = 64 * 1024;

/// A job's process ended.
struct Done {
    run: String,
    job: String,
    outcome: Outcome,
    reason: String,
    /// How many times the job was run again before this end.
    retried: u32,
}

/// Nobody asked the job to stop yet.
const WHY_NONE: u8 = 0;
/// A person (or the run's kill) asked: 強制終了.
const WHY_KILLED: u8 = 1;
/// Its `timeout` passed: 打ち切り.
const WHY_TIMEOUT: u8 = 2;

/// Says who asked first; the second asker changes nothing.
fn ask_stop(why: &AtomicU8, stop: &AtomicBool, who: u8) {
    let _ = why.compare_exchange(WHY_NONE, who, Ordering::SeqCst, Ordering::SeqCst);
    stop.store(true, Ordering::SeqCst);
}

/// One running job.
struct Active {
    run: String,
    job: String,
    stop: Arc<AtomicBool>,
    why: Arc<AtomicU8>,
}

/// A time of the schedule that waits for the net's last run to end.
#[derive(Clone, Debug)]
pub struct Queued {
    pub def: Def,
    pub planned: NaiveDateTime,
    /// Marked 遅れて実行 when it starts (`missed = "once"`).
    pub late: bool,
    /// When it was put to wait.
    pub since: NaiveDateTime,
}

pub struct Runner {
    /// `state_dir/logs`.
    logs: PathBuf,
    /// `state_dir/history`.
    history: PathBuf,
    config: Config,
    runs: Vec<Run>,
    active: Vec<Active>,
    /// The history files that may still change, by the day they are of.
    days: BTreeMap<NaiveDate, (Day, kept::Writer)>,
    tx: mpsc::Sender<Done>,
    rx: mpsc::Receiver<Done>,
    /// The second the last run id was made in, and how many were made in it.
    last_id: (String, u32),
    /// The runs that ended since [`Runner::take_over`], for the engine to tell of.
    over: Vec<Run>,
    /// The times that wait for a run of their net to end, oldest first.
    queue: Vec<Queued>,
}

impl Runner {
    /// Reads today's and yesterday's history: a run the last engine left unfinished is closed as 異常.
    pub fn new(state_dir: &Path, config: Config) -> Runner {
        let (tx, rx) = mpsc::channel();
        let mut r = Runner {
            logs: state_dir.join("logs"),
            history: state_dir.join("history"),
            config,
            runs: Vec::new(),
            active: Vec::new(),
            days: BTreeMap::new(),
            tx,
            rx,
            last_id: (String::new(), 0),
            over: Vec::new(),
            queue: Vec::new(),
        };
        let today = Local::now().date_naive();
        for day in [today - chrono::Days::new(1), today] {
            let file = history::path(&r.history, day);
            let loaded = Day::load(&file).unwrap_or_default();
            let runs = loaded.runs();
            r.days.insert(day, (loaded, kept::Writer::new(file, true)));
            r.runs.extend(runs);
        }
        let now = Local::now().naive_local();
        let mut closed = Vec::new();
        for run in &mut r.runs {
            if !run.is_over() {
                run.abandon(now);
                closed.push(run.clone());
            }
        }
        for run in closed {
            r.record(&run, history::run_end(&run));
            r.over.push(run);
        }
        r
    }

    /// The runs that ended since the last call, oldest first: each once.
    pub fn take_over(&mut self) -> Vec<Run> {
        std::mem::take(&mut self.over)
    }

    pub fn set_config(&mut self, config: Config) {
        self.config = config;
    }

    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    #[cfg(test)]
    pub fn run(&self, id: &str) -> Option<&Run> {
        self.runs.iter().find(|r| r.id == id)
    }

    /// The runs the day's list shows: started today, or not over yet.
    pub fn today(&self) -> Vec<Run> {
        let today = Local::now().date_naive();
        self.runs.iter().filter(|r| r.started.date() == today || !r.is_over()).cloned().collect()
    }

    /// Starts `def` for the business day `date`; `planned` is the time a schedule
    /// meant it for. Refused while the same net runs.
    pub fn start(&mut self, def: &Def, date: NaiveDate, how: How, planned: Option<NaiveDateTime>) -> Result<String, String> {
        self.begin(def, date, how, planned, None)
    }

    /// Starts `def` at the job `from`: the jobs before it are ⊘ スキップ ([`Run::starting_at`]).
    pub fn start_at(&mut self, def: &Def, date: NaiveDate, from: &str) -> Result<String, String> {
        self.begin(def, date, How::Manual, None, Some(from))
    }

    fn begin(
        &mut self,
        def: &Def,
        date: NaiveDate,
        how: How,
        planned: Option<NaiveDateTime>,
        from: Option<&str>,
    ) -> Result<String, String> {
        let net = def.net.as_ref().ok_or_else(|| format!("{}: 定義の誤り", def.file))?;
        self.not_running(&def.id, def.label())?;
        let now = Local::now().naive_local();
        let id = self.new_id(now);
        let date = date.format("%Y-%m-%d").to_string();
        let mut run = match from {
            Some(from) => Run::starting_at(&id, &def.id, def.label(), &date, how, net, from, now)?,
            None => Run::new(&id, &def.id, def.label(), &date, how, net, now),
        };
        run.planned = planned;
        self.launch(run, &def.text)
    }

    /// Starts run `previous` again with the definition it ran, the copy beside
    /// its logs ([`Run::rerun`]). `previous` may be of another day.
    pub fn rerun(&mut self, previous: &str, from: Option<&str>) -> Result<String, String> {
        let prev = self.previous(previous)?;
        if !prev.is_over() {
            return Err(format!("{}: まだ終わっていない", prev.id));
        }
        self.not_running(&prev.jobnet, &prev.title)?;
        let path = self.run_dir(previous).join("definition.toml");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let net = koyomi_core::jobnet::parse(&text)
            .map_err(|ps| ps.iter().map(|p| p.in_file(&path.display().to_string())).collect::<Vec<_>>().join(" / "))?;
        let now = Local::now().naive_local();
        let id = self.new_id(now);
        let run = Run::rerun(&id, &prev, &net, from, now)?;
        self.launch(run, &text)
    }

    /// The run `id`: one kept here, or one read back from the history of the day in its id.
    pub fn previous(&self, id: &str) -> Result<Run, String> {
        if let Some(r) = self.runs.iter().find(|r| r.id == id) {
            return Ok(r.clone());
        }
        let day = id.get(0..8).and_then(|d| NaiveDate::parse_from_str(d, "%Y%m%d").ok()).ok_or_else(|| format!("実行がない: {id}"))?;
        let loaded = Day::load(&history::path(&self.history, day)).unwrap_or_default();
        loaded.runs().into_iter().find(|r| r.id == id).ok_or_else(|| format!("実行がない: {id}"))
    }

    fn not_running(&self, jobnet: &str, label: &str) -> Result<(), String> {
        if self.runs.iter().any(|r| r.jobnet == jobnet && !r.is_over()) {
            return Err(format!("{label}: 動いている間はもう一度は流せない"));
        }
        Ok(())
    }

    /// Keeps the definition beside the run's logs, records the start (and the
    /// jobs that are over from the start: skipped, or carried over from a
    /// previous run), and starts what is ready.
    fn launch(&mut self, run: Run, text: &str) -> Result<String, String> {
        let dir = self.run_dir(&run.id);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::write(dir.join("definition.toml"), text).map_err(|e| format!("{}: {e}", dir.display()))?;
        self.record(&run, history::run_start(&run));
        let over: Vec<JobRun> = run.jobs.iter().filter(|j| j.ended.is_some()).cloned().collect();
        for j in &over {
            self.record(&run, history::job_end(&run, j));
        }
        let id = run.id.clone();
        self.runs.push(run);
        self.tick();
        Ok(id)
    }

    /// Starts the time `planned` of `def` after its time (`missed = "once"`):
    /// the run says 遅れて実行 with both times.
    pub fn start_late(&mut self, def: &Def, planned: NaiveDateTime) -> Result<String, String> {
        let id = self.start(def, planned.date(), How::Schedule, Some(planned))?;
        if let Some(r) = self.runs.iter_mut().find(|r| r.id == id) {
            r.mark_late();
        }
        Ok(id)
    }

    /// Puts the time `planned` of `def` to wait for the net's last run to
    /// end (`overlap = "queue"`); it starts in the `tick` that sees that end,
    /// late when `late`. One time waits per net: another is recorded as
    /// スキップ, so nothing piles up behind a run that goes on and on. With
    /// no run to wait for, it starts now.
    pub fn queue(&mut self, def: &Def, planned: NaiveDateTime, late: bool) {
        if self.queue.iter().any(|q| q.def.id == def.id) {
            self.not_run(def, planned, State::Skipped, "前の回がまだ動いていました（その次の回が待っています）");
        } else if self.runs.iter().any(|r| r.jobnet == def.id && !r.is_over()) {
            self.queue.push(Queued { def: def.clone(), planned, late, since: Local::now().naive_local() });
        } else {
            self.go(def, planned, late);
        }
    }

    /// The times that wait for a run to end, oldest first.
    pub fn queued(&self) -> &[Queued] {
        &self.queue
    }

    /// Records each waiting time as スキップ: the engine stops, and nothing
    /// would start them.
    pub fn drop_queue(&mut self) {
        for q in self.unqueue(None) {
            self.not_run(&q.def, q.planned, State::Skipped, "待っている間にエンジンが止まった");
        }
    }
    /// Takes the times waiting for the net `net` (every net when `None`) out of the queue:
    /// the caller writes down why they will not run.
    pub fn unqueue(&mut self, net: Option<&str>) -> Vec<Queued> {
        let (gone, kept) = std::mem::take(&mut self.queue).into_iter().partition(|q| match net {
            None => true,
            Some(n) => q.def.id == n,
        });
        self.queue = kept;
        gone
    }
    /// A person skips `job` of `run` before it starts: it ends ⊘, and the jobs after it go on.
    pub fn skip(&mut self, run: &str, job: &str) -> Result<String, String> {
        let now = Local::now().naive_local();
        let r = self.runs.iter_mut().find(|r| r.id == run).ok_or_else(|| format!("実行がない: {run}"))?;
        if r.is_over() {
            return Err(format!("{run}: もう終わっている"));
        }
        r.skip(job, now)?;
        let r = r.clone();
        if let Some(j) = r.job(job) {
            self.record(&r, history::job_end(&r, j));
        }
        if r.is_over() {
            self.ended(r);
        }
        self.start_ready();
        Ok(format!("{run} の {job} を飛ばした"))
    }

    /// Starts the schedule's time `planned` of `def`, late or not; what
    /// refuses it is recorded as 止めた.
    fn go(&mut self, def: &Def, planned: NaiveDateTime, late: bool) {
        let out = if late { self.start_late(def, planned) } else { self.start(def, planned.date(), How::Schedule, Some(planned)) };
        if let Err(e) = out {
            self.not_run(def, planned, State::Locked, &e);
        }
    }

    /// Records a scheduled time of `def` that did not start (時刻を過ぎた, スキップ,
    /// 未確認で止めた) as a run that is over at once, so the day shows it and
    /// nobody has to ask why nothing happened. `reason` is shown with it.
    pub fn not_run(&mut self, def: &Def, planned: NaiveDateTime, state: State, reason: &str) -> Option<String> {
        let net = def.net.as_ref()?;
        let now = Local::now().naive_local();
        let id = self.new_id(now);
        let run = Run::not_run(&id, &def.id, def.label(), &planned.format("%Y-%m-%d").to_string(), planned, state, reason, net, now);
        self.record(&run, history::run_start(&run));
        self.record(&run, history::run_end(&run));
        self.runs.push(run.clone());
        self.over.push(run);
        Some(id)
    }

    fn new_id(&mut self, now: NaiveDateTime) -> String {
        let second = now.format("%Y%m%d-%H%M%S").to_string();
        if self.last_id.0 == second {
            self.last_id.1 += 1;
        } else {
            self.last_id = (second.clone(), 0);
        }
        format!("{second}-{:02}", self.last_id.1)
    }

    /// `logs/<YYYY-MM-DD>/<run>`: the day is in the run's id.
    fn run_dir(&self, run: &str) -> PathBuf {
        let day = match run.get(0..8).and_then(|d| NaiveDate::parse_from_str(d, "%Y%m%d").ok()) {
            Some(d) => d.format("%Y-%m-%d").to_string(),
            None => "unknown".to_owned(),
        };
        self.logs.join(day).join(run)
    }

    pub fn log_path(&self, run: &str, job: &str) -> PathBuf {
        self.run_dir(run).join(format!("{job}.log"))
    }

    /// The last [`TAIL`] bytes of a job's log (whole lines).
    pub fn log_tail(&self, run: &str, job: &str) -> Result<String, String> {
        let path = self.log_path(run, job);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut from = bytes.len().saturating_sub(TAIL as usize);
        if from > 0 {
            from = bytes[from..].iter().position(|&b| b == b'\n').map_or(bytes.len(), |p| from + p + 1);
        }
        Ok(String::from_utf8_lossy(&bytes[from..]).into_owned())
    }

    /// Stops one job, or every job of the run; the ones not started will not start.
    pub fn kill(&mut self, run: &str, job: Option<&str>) -> Result<String, String> {
        let now = Local::now().naive_local();
        let r = self.runs.iter_mut().find(|r| r.id == run).ok_or_else(|| format!("実行がない: {run}"))?;
        if r.is_over() {
            return Err(format!("{run}: もう終わっている"));
        }
        match job {
            Some(job) => {
                let j = r.job(job).ok_or_else(|| format!("{run} にジョブ {job} はない"))?;
                if j.state != State::Running {
                    return Err(format!("{}: 動いていない", j.label()));
                }
                let a = self.active.iter().find(|a| a.run == run && a.job == job).ok_or_else(|| format!("{job}: 動いていない"))?;
                ask_stop(&a.why, &a.stop, WHY_KILLED);
                Ok(format!("{run} の {job} を止める"))
            }
            None => {
                r.kill_pending(now);
                let r = r.clone();
                for a in self.active.iter().filter(|a| a.run == run) {
                    ask_stop(&a.why, &a.stop, WHY_KILLED);
                }
                if r.is_over() {
                    // Nothing was running: the run ends here.
                    self.ended(r);
                }
                Ok(format!("{run} を止める"))
            }
        }
    }

    /// Takes in what ended and starts what is ready. Called often by the engine's loop.
    pub fn tick(&mut self) {
        let mut over: Vec<Run> = Vec::new();
        while let Ok(done) = self.rx.try_recv() {
            self.active.retain(|a| !(a.run == done.run && a.job == done.job));
            let now = Local::now().naive_local();
            let Some(r) = self.runs.iter_mut().find(|r| r.id == done.run) else { continue };
            r.retried(&done.job, done.retried);
            if done.reason.is_empty() {
                r.finish(&done.job, done.outcome, now);
            } else {
                r.fail(&done.job, &done.reason, now);
            }
            let r = r.clone();
            if let Some(j) = r.job(&done.job) {
                self.record(&r, history::job_end(&r, j));
            }
            if r.is_over() {
                over.push(r);
            }
        }
        for r in over {
            self.ended(r);
        }
        self.start_ready();
    }
    /// The run is over: it is written down, told of, and the time that waited for its net starts.
    fn ended(&mut self, r: Run) {
        let net = r.jobnet.clone();
        self.record(&r, history::run_end(&r));
        self.over.push(r);
        if let Some(i) = self.queue.iter().position(|q| q.def.id == net) {
            let q = self.queue.remove(i);
            self.go(&q.def, q.planned, q.late);
        }
    }

    fn start_ready(&mut self) {
        let limit = self.config.concurrency.max(1);
        let mut running = self.active.len();
        let mut starts: Vec<(String, String)> = Vec::new();
        for r in &mut self.runs {
            for job in r.ready() {
                if running < limit {
                    running += 1;
                    starts.push((r.id.clone(), job));
                } else {
                    r.wait_for_slot(&job, limit);
                }
            }
        }
        for (run_id, job_id) in starts {
            let now = Local::now().naive_local();
            let Some(r) = self.runs.iter_mut().find(|r| r.id == run_id) else { continue };
            r.start(&job_id, now);
            let r = r.clone();
            let Some(j) = r.job(&job_id) else { continue };
            self.record(&r, history::job_start(&r, j));
            let stop = Arc::new(AtomicBool::new(false));
            let why = Arc::new(AtomicU8::new(WHY_NONE));
            self.active.push(Active { run: run_id.clone(), job: job_id.clone(), stop: stop.clone(), why: why.clone() });
            let log = self.log_path(&run_id, &job_id);
            let job = JobSpec {
                run: run_id,
                jobnet: r.jobnet.clone(),
                date: r.date.clone(),
                id: job_id,
                command: j.command(),
                dir: j.dir.clone(),
                timeout: j.timeout,
                retry: j.retry,
                judge: j.judge.clone(),
                log,
                cap: self.config.log_max_bytes(),
            };
            let tx = self.tx.clone();
            let _ = std::thread::Builder::new().name("koyomi-job".into()).spawn(move || {
                let done = run_job(&job, &stop, &why);
                let _ = tx.send(done);
            });
        }
    }

    /// Appends `event` to the file of the day `run` started, and writes the file.
    fn record(&mut self, run: &Run, event: history::Event) {
        let day = run.started.date();
        let (d, w) = self.days.entry(day).or_insert_with(|| {
            let file = history::path(&self.history, day);
            (Day::load(&file).unwrap_or_default(), kept::Writer::new(file, true))
        });
        d.events.push(event);
        w.save_toml(d);
    }

    /// Writes what is pending before the engine ends. The runs' processes keep going on their own.
    pub fn flush(&self) {
        for (d, w) in self.days.values() {
            let _ = w.save_now(d.to_toml());
        }
    }
}

struct JobSpec {
    run: String,
    jobnet: String,
    date: String,
    id: String,
    command: Vec<String>,
    dir: Option<String>,
    /// Seconds before the timer asks the job to stop.
    timeout: Option<u64>,
    retry: Option<Retry>,
    /// What the exit codes mean: a 警告 end is not run again.
    judge: Option<Judge>,
    log: PathBuf,
    cap: u64,
}

/// Raises `stop` for 打ち切り when `secs` pass before `stop` is raised by
/// anyone or `over` says the process ended.
fn timer(secs: u64, stop: Arc<AtomicBool>, why: Arc<AtomicU8>, over: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    std::thread::Builder::new()
        .name("koyomi-timer".into())
        .spawn(move || {
            while !stop.load(Ordering::SeqCst) && !over.load(Ordering::SeqCst) {
                let now = Instant::now();
                if now >= deadline {
                    ask_stop(&why, &stop, WHY_TIMEOUT);
                    return;
                }
                std::thread::sleep((deadline - now).min(Duration::from_millis(100)));
            }
        })
        .expect("spawn the timeout timer")
}

/// What the log has taken so far, against the cap.
struct LogSize {
    written: u64,
    capped: bool,
}

/// Runs one job to its end on this thread, writing its lines to the log:
/// once, or again after each abnormal end while its `retry` allows.
fn run_job(job: &JobSpec, stop: &Arc<AtomicBool>, why: &Arc<AtomicU8>) -> Done {
    let done = |outcome, reason: String, retried: u32| Done { run: job.run.clone(), job: job.id.clone(), outcome, reason, retried };
    let Some((exe, args)) = job.command.split_first() else {
        return done(Outcome::Failed, "run が空".to_owned(), 0);
    };
    if let Some(dir) = job.log.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut file = match std::fs::File::create(&job.log) {
        Ok(f) => f,
        Err(e) => return done(Outcome::Failed, format!("ログを作れない: {}: {e}", job.log.display()), 0),
    };
    let mut size = LogSize { written: 0, capped: false };
    let (count, wait) = job.retry.map_or((0, 0), |r| (r.count, r.wait));
    let mut retried = 0;
    loop {
        let (outcome, reason) = attempt(job, exe, args, stop, why, &mut file, &mut size);
        let abnormal =
            matches!(outcome, Outcome::Exited(code) if verdict(job.judge.as_ref(), code) == State::Error) || outcome == Outcome::Failed;
        if !abnormal || retried >= count {
            return done(outcome, reason, retried);
        }
        let _ = writeln!(file, "! やり直し ({}/{count}): {} 待つ", retried + 1, koyomi_core::duration::text(wait));
        // The wait, unless a person stops the job during it.
        let until = Instant::now() + Duration::from_secs(wait);
        loop {
            if stop.load(Ordering::SeqCst) {
                let _ = writeln!(file, "! 強制終了");
                return done(Outcome::Killed, String::new(), retried);
            }
            let now = Instant::now();
            if now >= until {
                break;
            }
            std::thread::sleep((until - now).min(Duration::from_millis(100)));
        }
        retried += 1;
    }
}

/// Runs the job's process once, to its end.
fn attempt(
    job: &JobSpec,
    exe: &str,
    args: &[String],
    stop: &Arc<AtomicBool>,
    why: &Arc<AtomicU8>,
    file: &mut std::fs::File,
    size: &mut LogSize,
) -> (Outcome, String) {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args);
    if let Some(dir) = &job.dir {
        cmd.current_dir(dir);
    }
    cmd.env("KOYOMI_JOBNET", &job.jobnet).env("KOYOMI_JOB", &job.id).env("KOYOMI_RUN", &job.run).env("KOYOMI_DATE", &job.date);
    let mut on_line = |stream: ito_proc::Stream, line: &str| {
        if size.capped {
            return false;
        }
        let prefix = if stream == ito_proc::Stream::Err { "! " } else { "" };
        let text = format!("{prefix}{line}\n");
        size.written += text.len() as u64;
        if size.written > job.cap {
            size.capped = true;
            let _ = writeln!(file, "（ログが {} MB を超えたので、この先は書かない）", job.cap / (1024 * 1024));
        } else {
            let _ = file.write_all(text.as_bytes());
        }
        false
    };
    let over = Arc::new(AtomicBool::new(false));
    let timer = job.timeout.map(|secs| timer(secs, stop.clone(), why.clone(), over.clone()));
    let result = ito_proc::run_lines(cmd, stop, &mut on_line);
    // The process is over either way: let the timer go, and say in the log who stopped the job.
    over.store(true, Ordering::SeqCst);
    if let Some(t) = timer {
        let _ = t.join();
    }
    match result {
        Ok(lines) if lines.stopped() => {
            if why.load(Ordering::SeqCst) == WHY_TIMEOUT {
                let text = koyomi_core::duration::text(job.timeout.unwrap_or(0));
                let _ = writeln!(file, "! 打ち切り（{text} を超えた）");
                (Outcome::Timeout, String::new())
            } else {
                let _ = writeln!(file, "! 強制終了");
                (Outcome::Killed, String::new())
            }
        }
        Ok(lines) => {
            let code = lines.status.and_then(|s| s.code()).unwrap_or(-1);
            (Outcome::Exited(code), String::new())
        }
        Err(e) => {
            let _ = writeln!(file, "! {exe}: {e}");
            (Outcome::Failed, format!("{exe} を起動できない: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koyomi_core::approved::hash;
    use koyomi_core::scan::Standing;
    use std::time::{Duration, Instant};

    fn def(id: &str, text: &str) -> Def {
        let net = koyomi_core::jobnet::parse(text).unwrap();
        Def {
            id: id.into(),
            group: "g".into(),
            name: id.into(),
            title: net.name.clone(),
            file: format!("g/{id}.toml"),
            standing: Standing::Confirmed,
            problems: Vec::new(),
            net: Some(net),
            sha256: hash(text.as_bytes()),
            text: text.into(),
            previous: None,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koyomi-runner-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A command that takes about `secs` seconds.
    fn sleep(secs: u32) -> String {
        if cfg!(windows) {
            format!("run = [\"cmd.exe\", \"/c\", \"ping -n {} 127.0.0.1 > nul\"]", secs + 1)
        } else {
            format!("run = [\"sleep\", \"{secs}\"]")
        }
    }

    /// A command that exists on every machine the tests run on.
    fn echo(text: &str) -> String {
        if cfg!(windows) {
            format!("run = [\"cmd.exe\", \"/c\", \"echo {text}\"]")
        } else {
            format!("run = [\"echo\", \"{text}\"]")
        }
    }

    fn fail_cmd() -> String {
        if cfg!(windows) {
            "run = [\"cmd.exe\", \"/c\", \"exit 3\"]".to_owned()
        } else {
            "run = [\"sh\", \"-c\", \"exit 3\"]".to_owned()
        }
    }

    fn wait_over(r: &mut Runner, id: &str) -> Run {
        let until = Instant::now() + Duration::from_secs(20);
        while Instant::now() < until {
            r.tick();
            if let Some(run) = r.run(id).filter(|r| r.is_over()) {
                return run.clone();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("{id} did not end");
    }

    #[test]
    fn a_net_runs_in_order_and_writes_its_logs_and_history() {
        let dir = scratch("order");
        let text = format!("format = 1\n[[job]]\nid = \"a\"\n{}\n[[job]]\nid = \"b\"\nafter = [\"a\"]\n{}\n", echo("one"), echo("two"));
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        assert!(r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap_err().contains("動いている間は"));
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Ok);
        let (a, b) = (run.job("a").unwrap(), run.job("b").unwrap());
        assert!(a.ended.unwrap() <= b.started.unwrap());
        assert_eq!(a.exit, Some(0));
        assert_eq!(r.log_tail(&id, "a").unwrap().trim(), "one");
        assert_eq!(r.log_tail(&id, "b").unwrap().trim(), "two");
        assert!(r.run_dir(&id).join("definition.toml").exists());
        r.flush();
        let day = Day::load(&history::path(&dir.join("history"), Local::now().date_naive())).unwrap();
        let back = day.runs();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].state, State::Ok);
        assert_eq!(back[0].job("b").unwrap().state, State::Ok);
        // A second runner finds the finished run, and starts the net again.
        let mut r2 = Runner::new(&dir, Config::default());
        assert_eq!(r2.today().len(), 1);
        assert!(r2.start(&d, Local::now().date_naive(), How::Manual, None).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rerun_runs_the_failed_job_again_and_carries_the_one_that_passed() {
        let dir = scratch("rerun");
        let text = format!("format = 1\n[[job]]\nid = \"a\"\n{}\n[[job]]\nid = \"b\"\nafter = [\"a\"]\n{}\n", echo("one"), fail_cmd());
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let first = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        assert!(r.rerun(&first, None).unwrap_err().contains("まだ終わっていない"));
        assert_eq!(wait_over(&mut r, &first).state, State::Error);
        let second = r.rerun(&first, None).unwrap();
        assert_ne!(second, first);
        let run = wait_over(&mut r, &second);
        assert_eq!((run.how, run.state), (How::Rerun, State::Error));
        let a = run.job("a").unwrap();
        assert_eq!((a.state, a.exit, a.reason.as_str()), (State::Ok, Some(0), koyomi_core::run::CARRIED));
        assert!(!r.log_path(&second, "a").exists(), "a must not run again");
        assert!(r.log_path(&second, "b").exists());
        assert!(r.run_dir(&second).join("definition.toml").exists());
        // The history has a's end from the start: a new runner reads the rerun as it was.
        r.flush();
        let r2 = Runner::new(&dir, Config::default());
        let back = r2.run(&second).unwrap();
        assert_eq!(back.state, State::Error);
        assert_eq!(back.job("a").unwrap().state, State::Ok);
        assert_eq!(back.job("a").unwrap().reason, koyomi_core::run::CARRIED);
        // From a job: that one runs again too.
        let third = r.rerun(&first, Some("a")).unwrap();
        let run = wait_over(&mut r, &third);
        assert_eq!(run.job("a").unwrap().exit, Some(0));
        assert!(r.log_path(&third, "a").exists());
        assert!(r.rerun(&first, Some("x")).unwrap_err().contains("ジョブがない: x"));
        assert!(r.rerun("20200101-000000-00", None).unwrap_err().contains("実行がない"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_skipped_job_does_not_run_and_the_ones_after_it_go_on() {
        let dir = scratch("skip");
        let text = format!(
            "format = 1\n[[job]]\nid = \"a\"\n{}\n[[job]]\nid = \"b\"\nafter = [\"a\"]\n{}\n[[job]]\nid = \"c\"\nafter = [\"b\"]\n{}\n",
            sleep(2),
            echo("two"),
            echo("three")
        );
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        assert!(r.skip(&id, "a").unwrap_err().contains("もう動いている"));
        assert!(r.skip(&id, "x").unwrap_err().contains("ジョブがない: x"));
        assert!(r.skip("20200101-000000-00", "a").unwrap_err().contains("実行がない"));
        assert_eq!(r.skip(&id, "b").unwrap(), format!("{id} の b を飛ばした"));
        let b = r.run(&id).unwrap().job("b").unwrap().clone();
        assert_eq!((b.state, b.reason.as_str()), (State::Skipped, koyomi_core::run::SKIPPED));
        // A time that waits for this net is taken out of the queue when asked.
        let planned = Local::now().naive_local();
        r.queue(&d, planned, false);
        assert_eq!(r.queued().len(), 1);
        assert_eq!(r.unqueue(Some("other")).len(), 0);
        assert_eq!(r.unqueue(Some("net")).len(), 1);
        assert!(r.queued().is_empty());
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Ok);
        assert_eq!(run.job("c").unwrap().state, State::Ok);
        assert!(!r.log_path(&id, "b").exists(), "b must not run");
        assert_eq!(r.log_tail(&id, "c").unwrap().trim(), "three");
        assert!(r.skip(&id, "c").unwrap_err().contains("もう終わっている"));
        // The history has b's skip: a new runner reads the run as it was.
        r.flush();
        let back = Runner::new(&dir, Config::default());
        assert_eq!(back.run(&id).unwrap().job("b").unwrap().reason, koyomi_core::run::SKIPPED);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_start_at_a_job_skips_the_ones_before_it() {
        let dir = scratch("startat");
        let text = format!("format = 1\n[[job]]\nid = \"a\"\n{}\n[[job]]\nid = \"b\"\nafter = [\"a\"]\n{}\n", echo("one"), echo("two"));
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start_at(&d, Local::now().date_naive(), "b").unwrap();
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Ok);
        assert_eq!(run.job("a").unwrap().state, State::Skipped);
        assert!(!r.log_path(&id, "a").exists());
        assert_eq!(r.log_tail(&id, "b").unwrap().trim(), "two");
        r.flush();
        let back = Runner::new(&dir, Config::default());
        assert_eq!(back.run(&id).unwrap().job("a").unwrap().state, State::Skipped);
        assert!(r.start_at(&d, Local::now().date_naive(), "x").unwrap_err().contains("ジョブがない: x"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_exit_code_makes_the_run_an_error_and_stops_the_rest() {
        let dir = scratch("error");
        let text = format!("format = 1\n[[job]]\nid = \"a\"\n{}\n[[job]]\nid = \"b\"\nafter = [\"a\"]\n{}\n", fail_cmd(), echo("two"));
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Error);
        assert_eq!(run.job("a").unwrap().state, State::Error);
        assert_eq!(run.job("a").unwrap().exit, Some(3));
        assert_eq!(run.job("b").unwrap().state, State::NotRun);
        assert!(!r.log_path(&id, "b").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_executable_is_an_error_with_its_reason() {
        let dir = scratch("missing");
        let d = def("net", "format = 1\n[[job]]\nid = \"a\"\nrun = [\"koyomi-no-such-program-xyz\"]\n");
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Error);
        assert!(run.job("a").unwrap().reason.contains("起動できない"), "{}", run.job("a").unwrap().reason);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrency_holds_the_third_job_back() {
        let dir = scratch("slots");
        let sleep = if cfg!(windows) { "run = [\"cmd.exe\", \"/c\", \"ping -n 2 127.0.0.1 > nul\"]" } else { "run = [\"sleep\", \"1\"]" };
        let text = format!("format = 1\n[[job]]\nid = \"a\"\n{sleep}\n[[job]]\nid = \"b\"\n{sleep}\n[[job]]\nid = \"c\"\n{sleep}\n");
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config { concurrency: 2, ..Config::default() });
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let run = r.run(&id).unwrap();
        assert_eq!(run.running(), 2);
        assert_eq!(run.job("c").unwrap().state, State::Waiting);
        assert_eq!(run.job("c").unwrap().reason, "同時に動かす数（2）の空きを待っています");
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Ok);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kill_ends_the_run_with_the_job_marked_killed() {
        let dir = scratch("kill");
        let long = if cfg!(windows) { "run = [\"cmd.exe\", \"/c\", \"ping -n 30 127.0.0.1 > nul\"]" } else { "run = [\"sleep\", \"30\"]" };
        let text = format!("format = 1\n[[job]]\nid = \"a\"\n{long}\n[[job]]\nid = \"b\"\nafter = [\"a\"]\n{long}\n");
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        assert!(r.kill(&id, Some("b")).unwrap_err().contains("動いていない"));
        let asked = Instant::now();
        r.kill(&id, None).unwrap();
        let run = wait_over(&mut r, &id);
        assert!(asked.elapsed() < Duration::from_secs(10), "the tree was not killed at once");
        assert_eq!(run.state, State::Error);
        assert_eq!(run.job("a").unwrap().state, State::Killed);
        assert_eq!(run.job("a").unwrap().reason, "強制終了");
        assert!(r.log_tail(&id, "a").unwrap().ends_with("! 強制終了\n"));
        // The engine is handed the run once, to tell of it.
        let over: Vec<String> = r.take_over().iter().map(|r| r.id.clone()).collect();
        assert_eq!(over, vec![id.clone()]);
        assert!(r.take_over().is_empty());
        assert_eq!(run.job("b").unwrap().state, State::NotRun);
        assert!(r.kill(&id, None).unwrap_err().contains("もう終わっている"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_timeout_stops_the_job_and_says_so_apart_from_a_kill() {
        let dir = scratch("timeout");
        let long = if cfg!(windows) { "run = [\"cmd.exe\", \"/c\", \"ping -n 30 127.0.0.1 > nul\"]" } else { "run = [\"sleep\", \"30\"]" };
        let text =
            format!("format = 1\n[[job]]\nid = \"a\"\n{long}\ntimeout = \"1s\"\n[[job]]\nid = \"b\"\nafter = [\"a\"]\n{}\n", echo("two"));
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let started = Instant::now();
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        assert!(started.elapsed() < Duration::from_secs(10), "the timeout did not stop the job");
        assert_eq!(run.state, State::Error);
        let a = run.job("a").unwrap();
        assert_eq!(a.state, State::Timeout);
        assert_eq!(a.reason, "打ち切り（1s を超えた）");
        assert_eq!(run.job("b").unwrap().state, State::NotRun);
        assert!(r.log_tail(&id, "a").unwrap().ends_with("! 打ち切り（1s を超えた）\n"));
        // A job that ends in time is not touched by its timer.
        let text = format!("format = 1\n[[job]]\nid = \"a\"\n{}\ntimeout = \"1h\"\n", echo("quick"));
        let id = r.start(&def("quick", &text), Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Ok);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A command that fails until `flag` exists, and makes it on the way.
    fn fail_once(flag: &Path) -> String {
        // Backslashes doubled for the TOML string.
        let f = flag.display().to_string().replace('\\', "\\\\");
        if cfg!(windows) {
            // No quotes: cmd.exe does not read the `\"` the process start writes (the temp folder has no space).
            format!("run = [\"cmd.exe\", \"/c\", \"if exist {f} (exit 0) else (echo x> {f} & exit 3)\"]")
        } else {
            format!("run = [\"sh\", \"-c\", \"if [ -e '{f}' ]; then exit 0; else touch '{f}'; exit 3; fi\"]")
        }
    }

    #[test]
    fn a_retry_runs_the_job_again_after_the_wait_and_counts_the_times() {
        let dir = scratch("retry");
        let flag = dir.join("flag");
        let text = format!("format = 1\n[[job]]\nid = \"a\"\n{}\nretry = {{ count = 2, wait = \"1s\" }}\n", fail_once(&flag));
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let started = Instant::now();
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        assert!(started.elapsed() >= Duration::from_secs(1), "the wait was not waited");
        assert_eq!(run.state, State::Ok, "{:?}", run.job("a"));
        let a = run.job("a").unwrap();
        assert_eq!(a.retried, 1);
        assert_eq!(a.reason, "やり直し 1 回");
        let log = r.log_tail(&id, "a").unwrap();
        assert!(log.contains("! やり直し (1/2): 1s 待つ"), "{log}");
        assert!(!log.contains("(2/2)"), "{log}");
        // The count is in the history too.
        r.flush();
        let day = Day::load(&history::path(&dir.join("history"), Local::now().date_naive())).unwrap();
        assert_eq!(day.runs()[0].job("a").unwrap().retried, 1);
        // Still failing after every time: an error that says how many times.
        let text = format!("format = 1\n[[job]]\nid = \"a\"\n{}\nretry = {{ count = 2, wait = \"1s\" }}\n", fail_cmd());
        let id = r.start(&def("always", &text), Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Error);
        let a = run.job("a").unwrap();
        assert_eq!(a.retried, 2);
        assert_eq!(a.exit, Some(3));
        assert_eq!(a.reason, "終了コード 3、やり直し 2 回");
        let log = r.log_tail(&id, "a").unwrap();
        assert!(log.contains("(1/2)") && log.contains("(2/2)"), "{log}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_warn_exit_lets_the_next_job_run_and_is_not_run_again() {
        let dir = scratch("warn");
        let text = format!(
            "format = 1\n[[job]]\nid = \"a\"\n{}\njudge = {{ warn = \"3\" }}\nretry = {{ count = 1, wait = \"1s\" }}\n[[job]]\nid = \"b\"\nafter = [\"a\"]\n{}\n",
            fail_cmd(),
            echo("two")
        );
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Warn, "{:?}", run.job("a"));
        let a = run.job("a").unwrap();
        assert_eq!((a.state, a.exit, a.retried, a.reason.as_str()), (State::Warn, Some(3), 0, "終了コード 3（警告）"));
        assert_eq!(run.job("b").unwrap().state, State::Ok);
        assert!(!r.log_tail(&id, "a").unwrap().contains("やり直し"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_powershell_script_runs_as_one_argument_and_its_exit_code_is_judged() {
        if !cfg!(windows) {
            return;
        }
        let dir = scratch("shell");
        let text = "format = 1\n[[job]]\nid = \"a\"\nshell = \"powershell\"\nscript = '''\nWrite-Output \"hi `\"there`\"\"\nexit 4'''\njudge = { warn = \"4\" }\n";
        let d = def("net", text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        let a = run.job("a").unwrap();
        assert_eq!((a.state, a.exit), (State::Warn, Some(4)), "{a:?}");
        // powershell.exe may write a CLIXML progress record ("preparing
        // modules") to stderr; only its stdout is checked.
        let log = r.log_tail(&id, "a").unwrap();
        let out: Vec<&str> = log.lines().filter(|l| !l.starts_with("! ")).collect();
        assert_eq!(out, ["hi \"there\""], "{log}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_kill_during_the_wait_ends_the_job_as_killed() {
        let dir = scratch("retry-kill");
        let text = format!("format = 1\n[[job]]\nid = \"a\"\n{}\nretry = {{ count = 1, wait = \"30s\" }}\n", fail_cmd());
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        // Until the process has failed and the wait begun.
        let until = Instant::now() + Duration::from_secs(10);
        while Instant::now() < until && !r.log_tail(&id, "a").map(|l| l.contains("やり直し")).unwrap_or(false) {
            r.tick();
            std::thread::sleep(Duration::from_millis(20));
        }
        let asked = Instant::now();
        r.kill(&id, None).unwrap();
        let run = wait_over(&mut r, &id);
        assert!(asked.elapsed() < Duration::from_secs(5), "the wait did not end at the kill");
        let a = run.job("a").unwrap();
        assert_eq!(a.state, State::Killed);
        assert_eq!(a.retried, 0);
        assert_eq!(a.reason, "強制終了");
        assert!(r.log_tail(&id, "a").unwrap().ends_with("! 強制終了\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_the_last_engine_left_is_closed_as_an_error() {
        let dir = scratch("abandon");
        let hist = dir.join("history");
        let today = Local::now().date_naive();
        let now = Local::now().naive_local();
        let net = koyomi_core::jobnet::parse("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n").unwrap();
        let mut run = Run::new("20260101-000000-00", "g/net", "net", "2026-01-01", How::Manual, &net, now);
        let mut day = Day::default();
        day.events.push(history::run_start(&run));
        run.start("a", now);
        day.events.push(history::job_start(&run, run.job("a").unwrap()));
        ito_common::write_atomic(&history::path(&hist, today), day.to_toml()).unwrap();
        let mut r = Runner::new(&dir, Config::default());
        let back = r.run("20260101-000000-00").unwrap();
        assert_eq!(back.state, State::Error);
        assert_eq!(back.job("a").unwrap().reason, "エンジンが止まった");
        assert_eq!(r.take_over().len(), 1, "the closed run is told of too");
        r.flush();
        let day = Day::load(&history::path(&hist, today)).unwrap();
        assert!(matches!(day.events.last(), Some(history::Event::RunEnd { state: State::Error, .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_late_start_says_both_times_and_a_queued_time_starts_when_the_run_ends() {
        let dir = scratch("queue");
        let d = def(
            "net",
            &format!(
                "format = 1
overlap = \"queue\"
[[job]]
id = \"a\"
{}
",
                sleep(2)
            ),
        );
        let mut r = Runner::new(&dir, Config::default());
        let p1 = NaiveDateTime::parse_from_str("2026-10-30 02:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let p2 = NaiveDateTime::parse_from_str("2026-10-30 02:01:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let p3 = NaiveDateTime::parse_from_str("2026-10-30 02:02:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let first = r.start_late(&d, p1).unwrap();
        let run = r.run(&first).unwrap();
        assert!(run.is_late());
        assert!(run.reason.starts_with("遅れて実行（2:00 の予定を "), "{}", run.reason);
        assert!(run.reason.ends_with(" に流しました）"), "{}", run.reason);
        // The next time waits; a third is skipped, not piled up.
        r.queue(&d, p2, false);
        assert_eq!(r.queued().len(), 1);
        assert_eq!(r.runs().len(), 1, "a waiting time is not a run yet");
        r.queue(&d, p3, false);
        assert_eq!(r.queued().len(), 1);
        let skipped = r.runs().iter().find(|x| x.planned == Some(p3)).unwrap();
        assert_eq!(skipped.state, State::Skipped);
        assert_eq!(skipped.reason, "前の回がまだ動いていました（その次の回が待っています）");
        let t0 = Instant::now();
        loop {
            r.tick();
            if r.runs().iter().any(|x| x.planned == Some(p2)) {
                break;
            }
            assert!(t0.elapsed() < Duration::from_secs(20), "the waiting time never started");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(r.run(&first).unwrap().is_over(), "it started once the first run was over");
        let second = r.runs().iter().find(|x| x.planned == Some(p2)).unwrap();
        assert_eq!(second.how, How::Schedule);
        assert!(!second.is_late());
        assert!(!second.is_over());
        assert!(r.queued().is_empty());
        // With no run to wait for, a queued time starts at once. Then the engine stops: what waits is recorded.
        let mut r2 = Runner::new(&scratch("queue2"), Config::default());
        r2.queue(&d, p1, true);
        assert!(r2.queued().is_empty());
        assert!(r2.runs()[0].is_late());
        r2.queue(&d, p2, false);
        assert_eq!(r2.queued().len(), 1);
        r2.drop_queue();
        assert!(r2.queued().is_empty());
        let dropped = r2.runs().iter().find(|x| x.planned == Some(p2)).unwrap();
        assert_eq!((dropped.state, dropped.reason.as_str()), (State::Skipped, "待っている間にエンジンが止まった"));
        let _ = r2.kill(&r2.runs()[0].id.clone(), None);
    }

    #[test]
    fn a_time_that_did_not_start_is_a_run_over_at_once_with_its_reason() {
        let dir = scratch("notrun");
        let d = def(
            "net",
            &format!(
                "format = 1
[[job]]
id = \"a\"
{}
",
                echo("one")
            ),
        );
        let mut r = Runner::new(&dir, Config::default());
        let planned = NaiveDateTime::parse_from_str("2026-10-30 02:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let id = r.not_run(&d, planned, State::Missed, "2:00 に PC が止まっていました").unwrap();
        let run = r.run(&id).unwrap().clone();
        assert!(run.is_over());
        assert_eq!(run.state, State::Missed);
        assert_eq!(run.how, How::Schedule);
        assert_eq!(run.planned, Some(planned));
        assert_eq!(run.date, "2026-10-30");
        assert_eq!(run.reason, "2:00 に PC が止まっていました");
        assert!(run.jobs.iter().all(|j| j.state == State::NotRun));
        assert!(!r.log_path(&id, "a").exists(), "nothing ran, nothing logged");
        let over = r.take_over();
        assert_eq!(over.len(), 1, "told of once");
        assert_eq!(over[0].id, id);
        assert!(r.take_over().is_empty());
        // The net is free to start: a time that passed holds nothing.
        assert!(r.start(&d, Local::now().date_naive(), How::Manual, None).is_ok());
        r.flush();
        let day = Day::load(&history::path(&dir.join("history"), Local::now().date_naive())).unwrap();
        let back = day.runs();
        let kept = back.iter().find(|x| x.id == id).expect("in the day's history");
        assert_eq!(kept.state, State::Missed);
        assert_eq!(kept.reason, "2:00 に PC が止まっていました");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_log_tail_is_whole_lines_at_the_end() {
        let dir = scratch("tail");
        let r = Runner::new(&dir, Config::default());
        let path = r.log_path("20260101-000000-00", "a");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut text = String::new();
        for i in 0..5000 {
            text.push_str(&format!("line {i:05} ............................\n"));
        }
        std::fs::write(&path, &text).unwrap();
        let tail = r.log_tail("20260101-000000-00", "a").unwrap();
        assert!(tail.starts_with("line "));
        assert!(tail.ends_with("line 04999 ............................\n"));
        assert!(tail.len() as u64 <= TAIL);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
