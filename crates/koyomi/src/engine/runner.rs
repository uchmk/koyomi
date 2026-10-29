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
use koyomi_core::run::{How, Outcome, Run, State};
use koyomi_core::scan::Def;

/// How much of a log `koyomi log` and the window get: the last 64 KB.
pub const TAIL: u64 = 64 * 1024;

/// A job's process ended.
struct Done {
    run: String,
    job: String,
    outcome: Outcome,
    reason: String,
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
        }
        r
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

    /// Starts `def` for the business day `date`. Refused while the same net runs.
    pub fn start(&mut self, def: &Def, date: NaiveDate, how: How) -> Result<String, String> {
        let net = def.net.as_ref().ok_or_else(|| format!("{}: 定義の誤り", def.file))?;
        if self.runs.iter().any(|r| r.jobnet == def.id && !r.is_over()) {
            return Err(format!("{}: 動いている間はもう一度は流せない", def.label()));
        }
        let now = Local::now().naive_local();
        let id = self.new_id(now);
        let run = Run::new(&id, &def.id, def.label(), &date.format("%Y-%m-%d").to_string(), how, net, now);
        // The run keeps the definition it started with, beside its logs.
        let dir = self.run_dir(&run.id);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::write(dir.join("definition.toml"), &def.text).map_err(|e| format!("{}: {e}", dir.display()))?;
        self.record(&run, history::run_start(&run));
        self.runs.push(run);
        self.tick();
        Ok(id)
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
                    self.record(&r, history::run_end(&r));
                }
                Ok(format!("{run} を止める"))
            }
        }
    }

    /// Takes in what ended and starts what is ready. Called often by the engine's loop.
    pub fn tick(&mut self) {
        let mut ended: Vec<Run> = Vec::new();
        while let Ok(done) = self.rx.try_recv() {
            self.active.retain(|a| !(a.run == done.run && a.job == done.job));
            let now = Local::now().naive_local();
            let Some(r) = self.runs.iter_mut().find(|r| r.id == done.run) else { continue };
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
                ended.push(r);
            }
        }
        for r in ended {
            self.record(&r, history::run_end(&r));
        }
        self.start_ready();
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
                command: j.run.clone(),
                dir: j.dir.clone(),
                timeout: j.timeout,
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
    log: PathBuf,
    cap: u64,
}

/// Raises `stop` for 打ち切り when `secs` pass before `stop` is raised by anyone.
fn timer(secs: u64, stop: Arc<AtomicBool>, why: Arc<AtomicU8>) -> std::thread::JoinHandle<()> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    std::thread::Builder::new()
        .name("koyomi-timer".into())
        .spawn(move || {
            while !stop.load(Ordering::SeqCst) {
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

/// Runs one job to its end on this thread, writing its lines to the log.
fn run_job(job: &JobSpec, stop: &Arc<AtomicBool>, why: &Arc<AtomicU8>) -> Done {
    let done = |outcome, reason: String| Done { run: job.run.clone(), job: job.id.clone(), outcome, reason };
    let Some((exe, args)) = job.command.split_first() else {
        return done(Outcome::Failed, "run が空".to_owned());
    };
    if let Some(dir) = job.log.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut file = match std::fs::File::create(&job.log) {
        Ok(f) => f,
        Err(e) => return done(Outcome::Failed, format!("ログを作れない: {}: {e}", job.log.display())),
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args);
    if let Some(dir) = &job.dir {
        cmd.current_dir(dir);
    }
    cmd.env("KOYOMI_JOBNET", &job.jobnet).env("KOYOMI_JOB", &job.id).env("KOYOMI_RUN", &job.run).env("KOYOMI_DATE", &job.date);
    let (mut written, mut capped) = (0u64, false);
    let mut on_line = |stream: ito_proc::Stream, line: &str| {
        if capped {
            return false;
        }
        let prefix = if stream == ito_proc::Stream::Err { "! " } else { "" };
        let text = format!("{prefix}{line}\n");
        written += text.len() as u64;
        if written > job.cap {
            capped = true;
            let _ = writeln!(file, "（ログが {} MB を超えたので、この先は書かない）", job.cap / (1024 * 1024));
        } else {
            let _ = file.write_all(text.as_bytes());
        }
        false
    };
    let timer = job.timeout.map(|secs| timer(secs, stop.clone(), why.clone()));
    let result = ito_proc::run_lines(cmd, stop, &mut on_line);
    // The job is over either way: let the timer go, and say in the log who stopped the job.
    stop.store(true, Ordering::SeqCst);
    if let Some(t) = timer {
        let _ = t.join();
    }
    match result {
        Ok(lines) if lines.stopped() => {
            if why.load(Ordering::SeqCst) == WHY_TIMEOUT {
                let text = koyomi_core::duration::text(job.timeout.unwrap_or(0));
                let _ = writeln!(file, "! 打ち切り（{text} を超えた）");
                done(Outcome::Timeout, String::new())
            } else {
                let _ = writeln!(file, "! 強制終了");
                done(Outcome::Killed, String::new())
            }
        }
        Ok(lines) => {
            let code = lines.status.and_then(|s| s.code()).unwrap_or(-1);
            done(Outcome::Exited(code), String::new())
        }
        Err(e) => {
            let _ = writeln!(file, "! {exe}: {e}");
            done(Outcome::Failed, format!("{exe} を起動できない: {e}"))
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
        let id = r.start(&d, Local::now().date_naive(), How::Manual).unwrap();
        assert!(r.start(&d, Local::now().date_naive(), How::Manual).unwrap_err().contains("動いている間は"));
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
        assert!(r2.start(&d, Local::now().date_naive(), How::Manual).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_exit_code_makes_the_run_an_error_and_stops_the_rest() {
        let dir = scratch("error");
        let text = format!("format = 1\n[[job]]\nid = \"a\"\n{}\n[[job]]\nid = \"b\"\nafter = [\"a\"]\n{}\n", fail_cmd(), echo("two"));
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual).unwrap();
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
        let id = r.start(&d, Local::now().date_naive(), How::Manual).unwrap();
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
        let id = r.start(&d, Local::now().date_naive(), How::Manual).unwrap();
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
        let id = r.start(&d, Local::now().date_naive(), How::Manual).unwrap();
        assert!(r.kill(&id, Some("b")).unwrap_err().contains("動いていない"));
        let asked = Instant::now();
        r.kill(&id, None).unwrap();
        let run = wait_over(&mut r, &id);
        assert!(asked.elapsed() < Duration::from_secs(10), "the tree was not killed at once");
        assert_eq!(run.state, State::Error);
        assert_eq!(run.job("a").unwrap().state, State::Killed);
        assert_eq!(run.job("a").unwrap().reason, "強制終了");
        assert!(r.log_tail(&id, "a").unwrap().ends_with("! 強制終了\n"));
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
        let id = r.start(&d, Local::now().date_naive(), How::Manual).unwrap();
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
        let id = r.start(&def("quick", &text), Local::now().date_naive(), How::Manual).unwrap();
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Ok);
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
        let r = Runner::new(&dir, Config::default());
        let back = r.run("20260101-000000-00").unwrap();
        assert_eq!(back.state, State::Error);
        assert_eq!(back.job("a").unwrap().reason, "エンジンが止まった");
        r.flush();
        let day = Day::load(&history::path(&hist, today)).unwrap();
        assert!(matches!(day.events.last(), Some(history::Event::RunEnd { state: State::Error, .. })));
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
