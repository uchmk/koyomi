//! The runs: the engine's jobs as processes, their logs, and the day's history.
//!
//! Each job runs on a thread of its own through `ito_proc::run_lines_soft`, which
//! writes the lines to `logs/<day>/<run>/<job>.log` and tells the main loop
//! how it ended. The order between jobs is `koyomi_core::run`'s; this file
//! only starts what it says is ready and keeps the files current.
//!
//! A job is stopped through one flag, whoever asks: `kill` from a person, or
//! the timer a `timeout` starts. `ito_proc` then kills the whole process tree;
//! an `ssh` job is asked to stop first and killed only after its grace
//! ([`grace`]), so `koyomi ssh run` can ask the far command to stop too. The
//! ask reaches it without a console as well (`ITO_PROC_STOP_EVENT`), so also
//! from the release `koyomi.exe`, which has none.
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
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime};

use chrono::{Local, NaiveDate, NaiveDateTime};
use ito_common::kept;
use koyomi_core::calendar::Calendar;
use koyomi_core::chore::Chore;
use koyomi_core::config::Config;
use koyomi_core::held::Held;
use koyomi_core::history::{self, Day};
use koyomi_core::jobnet::{self, Jobnet, Judge, Part, Retry, RunAs};
use koyomi_core::late::Delay;
use koyomi_core::lock::{admit, Holder};
use koyomi_core::run::{verdict, How, JobRun, Outcome, Run, State};
use koyomi_core::scan::Def;
use koyomi_core::template;

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

/// The `[params]` values a run by hand was given, beside its `definition.toml`.
const PARAMS: &str = "params.toml";
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
    /// The slots it holds: 1, or the passes a loop part runs side by side.
    slots: usize,
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

/// What of `require` the PC is not in yet, as a reason; `None` when it is met (or cannot be known).
fn unmet(req: &koyomi_core::jobnet::Require) -> Option<String> {
    if cfg!(test) || req.is_empty() {
        return None;
    }
    unmet_by(req, ito_power::on_ac(), ito_power::network_up(), ito_power::idle_for())
}

/// [`unmet`] for the PC's state as given; a state that is `None` cannot be known and counts as met.
fn unmet_by(req: &koyomi_core::jobnet::Require, ac: Option<bool>, network: Option<bool>, idle: Option<Duration>) -> Option<String> {
    if req.ac && ac == Some(false) {
        return Some("AC 電源につながるのを待っています（require = \"ac\"）".to_owned());
    }
    if req.network && network == Some(false) {
        return Some("ネットワークにつながるのを待っています（require = \"network\"）".to_owned());
    }
    if let Some(s) = req.idle {
        if idle.is_some_and(|d| d.as_secs() < s) {
            return Some(format!("PC が {} 分使われなくなるのを待っています（require = \"idle\"）", s.div_ceil(60)));
        }
    }
    None
}

/// A part that waits: `kind = "time"` for a time, `kind = "file"` for a file.
struct Waiting {
    run: String,
    job: String,
    due: Due,
}

enum Due {
    /// The time, and how it reads (`06:00`, or with the day when it is another).
    Time { at: NaiveDateTime, text: String },
    /// A person's answer ([`Runner::answer`]); nothing ends it by itself.
    Person,
    File {
        pattern: String,
        /// Seconds the file must stay the same before it counts as arrived.
        settle: Option<u64>,
        /// Seconds the part may wait from `since` (its `timeout`).
        timeout: Option<u64>,
        since: Instant,
        /// The file last seen: its path, size and change time, and when it was first seen so.
        seen: Option<(PathBuf, u64, Option<SystemTime>, Instant)>,
    },
}

impl Due {
    /// What ends the wait now, when something does: the state, the reason, and the file found.
    fn check(&mut self, now: NaiveDateTime) -> Option<(State, String, Option<String>)> {
        match self {
            Due::Person => None,
            Due::Time { at, text } => (now >= *at).then(|| (State::Ok, format!("{text} になった"), None)),
            Due::File { pattern, settle, timeout, since, seen } => {
                let found = super::files::find_file(pattern);
                let arrived = match (found, *settle) {
                    (None, _) => {
                        *seen = None;
                        false
                    }
                    (Some((p, len, mtime)), None) => {
                        *seen = Some((p, len, mtime, Instant::now()));
                        true
                    }
                    (Some((p, len, mtime)), Some(s)) => match seen {
                        Some((sp, sl, sm, first)) if *sp == p && *sl == len && *sm == mtime => first.elapsed() >= Duration::from_secs(s),
                        _ => {
                            *seen = Some((p, len, mtime, Instant::now()));
                            false
                        }
                    },
                };
                if arrived {
                    let path = seen.as_ref().map(|s| s.0.display().to_string())?;
                    return Some((State::Ok, format!("{path} が来た"), Some(path)));
                }
                match *timeout {
                    Some(t) if since.elapsed() >= Duration::from_secs(t) => {
                        Some((State::Timeout, format!("打ち切り（{} を超えた）", koyomi_core::duration::text(t)), None))
                    }
                    _ => None,
                }
            }
        }
    }
}

/// `06:00` on the run's day, or `2026-11-02 06:00` when `days` moved it.
fn time_text(at: NaiveDateTime, days: u32) -> String {
    if days == 0 {
        at.format("%H:%M").to_string()
    } else {
        at.format("%Y-%m-%d %H:%M").to_string()
    }
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
    /// What a `kind = "jobnet"` part may use, by id: the net, or why it cannot be used.
    children: BTreeMap<String, Result<Jobnet, String>>,
    /// The parts that wait for a time or a file.
    waits: Vec<Waiting>,
    /// The `kind = "notify"` parts that ended since [`Runner::take_notices`]: (run, job).
    notices: Vec<(String, String)>,
    /// The `kind = "approval"` parts that began to wait since [`Runner::take_asks`]: (run, job).
    asks: Vec<(String, String)>,
    /// `calendars/*.toml` and the calendar each net counts by: what `{date-1bd}` in a job needs.
    calendars: BTreeMap<String, Calendar>,
    calendar_of: BTreeMap<String, String>,
    /// The jobs on hold (`koyomi hold <net> <job>`): a run that comes to one waits there.
    held: Held,
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
            children: BTreeMap::new(),
            calendars: BTreeMap::new(),
            calendar_of: BTreeMap::new(),
            waits: Vec::new(),
            notices: Vec::new(),
            asks: Vec::new(),
            held: Held::default(),
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

    /// The nets a `kind = "jobnet"` part may use (`use = "グループ/名前"`): each
    /// confirmed net, or why the others cannot be used.
    pub fn set_children(&mut self, children: BTreeMap<String, Result<Jobnet, String>>) {
        self.children = children;
    }

    /// The jobs on hold, as `held.toml` has them now. A held job that a run waits at and is no
    /// longer held goes on in the same run.
    pub fn set_held(&mut self, held: &Held) {
        self.held = Held { jobs: held.jobs.clone(), ..Held::default() };
        let now = Local::now().naive_local();
        for r in self.runs.iter_mut().filter(|r| !r.is_over()) {
            let net = r.jobnet.clone();
            r.unhold(|j| !self.held.is_job_held(&net, j), now);
        }
        self.start_ready();
    }

    /// The `kind = "notify"` parts that ended since the last call, oldest first: (run id, job id).
    pub fn take_asks(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.asks)
    }

    /// A person answers the 確認待ち part `job` of `run`: `yes` ends it 正常, else 異常 (却下).
    pub fn answer(&mut self, run: &str, job: &str, yes: bool) -> Result<String, String> {
        let i = self
            .waits
            .iter()
            .position(|w| w.run == run && w.job == job && matches!(w.due, Due::Person))
            .ok_or_else(|| format!("{run} の {job}: 確認を待っていない"))?;
        self.waits.remove(i);
        let now = Local::now().naive_local();
        let (state, reason) = if yes { (State::Ok, "承認された") } else { (State::Error, "却下された") };
        if let Some(r) = self.end_part(run, job, state, reason, None, now) {
            self.ended(r);
        }
        self.start_ready();
        Ok(format!("{run} の {job} を{}", if yes { "承認した" } else { "却下した" }))
    }

    pub fn take_notices(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.notices)
    }

    /// `net` with its child jobnets' jobs in place of their parts ([`jobnet::expand`]).
    /// The calendars, and the one each net counts by (`jobnet id` → `calendar name`).
    pub fn set_calendars(&mut self, calendars: BTreeMap<String, Calendar>, calendar_of: BTreeMap<String, String>) {
        self.calendars = calendars;
        self.calendar_of = calendar_of;
    }

    /// The net with its child nets in, and `{param:…}` (`set` over the defaults) and `{date}`
    /// in its jobs written out for the business day `date`.
    fn expand(&self, jobnet: &str, net: &Jobnet, date: NaiveDate, set: &BTreeMap<String, String>) -> Result<Jobnet, String> {
        let net = jobnet::expand(net, &|u| self.children.get(u).cloned().unwrap_or_else(|| Err(format!("子ジョブネットがない: {u}"))))?;
        let cal = self.calendar_of.get(jobnet).and_then(|c| self.calendars.get(c));
        let values = template::values(&net, set)?;
        let mut net = template::expand_net(&net, date, cal, &values)?;
        // An `ssh` runs koyomi itself (`koyomi ssh run`).
        let exe = std::env::current_exe().map_or_else(|_| "koyomi".to_owned(), |p| p.to_string_lossy().into_owned());
        koyomi_core::chore::fill(&mut net, &exe)?;
        Ok(net)
    }

    /// Each job of `def` as it would run for the business day `date`, one per line
    /// (`job: command`, the folder after `@`); nothing starts.
    pub fn dry(&self, def: &Def, date: NaiveDate, set: &BTreeMap<String, String>) -> Result<String, String> {
        let net = self.expand(&def.id, def.net.as_ref().ok_or_else(|| format!("{}: 定義の誤り", def.file))?, date, set)?;
        let mut out = String::new();
        for j in &net.jobs {
            let what = match (&j.script, j.run.is_empty()) {
                (Some(s), _) => format!("[{}] {}", j.shell.as_deref().unwrap_or("powershell"), s.trim_end().replace('\n', " ⏎ ")),
                (None, false) => j.run.join(" "),
                (None, true) => continue,
            };
            let at = j.dir.as_ref().map(|d| format!(" @ {d}")).unwrap_or_default();
            out.push_str(&format!("{}: {what}{at}\n", j.id));
        }
        Ok(out)
    }

    pub fn set_config(&mut self, config: Config) {
        self.config = config;
    }

    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// How many job processes are running now.
    pub fn busy(&self) -> usize {
        self.active.len()
    }

    /// How many slots the running jobs hold.
    fn slots_used(&self) -> usize {
        self.active.iter().map(|a| a.slots).sum()
    }

    #[cfg(test)]
    pub fn run(&self, id: &str) -> Option<&Run> {
        self.runs.iter().find(|r| r.id == id)
    }

    /// The runs that started on the days `from` to `to` (both included), oldest
    /// first: the ones kept here for the days this engine writes, the others read
    /// back from their day's file. At most a year, so a reply stays a reply.
    pub fn history(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<Run>, String> {
        if to < from {
            return Err(format!("範囲が逆: {from} → {to}"));
        }
        if (to - from).num_days() > 366 {
            return Err("範囲は 1 年まで".to_owned());
        }
        let mut out = Vec::new();
        let mut day = from;
        while day <= to {
            if self.days.contains_key(&day) {
                out.extend(self.runs.iter().filter(|r| r.started.date() == day).cloned());
            } else {
                let loaded = Day::load(&history::path(&self.history, day)).unwrap_or_default();
                out.extend(loaded.runs());
            }
            let Some(next) = day.succ_opt() else { break };
            day = next;
        }
        Ok(out)
    }

    /// The runs the day's list shows: started today, or not over yet.
    pub fn today(&self) -> Vec<Run> {
        let today = Local::now().date_naive();
        self.runs.iter().filter(|r| r.started.date() == today || !r.is_over()).cloned().collect()
    }

    /// Starts `def` for the business day `date`; `planned` is the time a schedule
    /// meant it for. Refused while the same net runs.
    pub fn start(&mut self, def: &Def, date: NaiveDate, how: How, planned: Option<NaiveDateTime>) -> Result<String, String> {
        let run = self.prepare(def, date, how, planned, None, &BTreeMap::new())?;
        self.launch(run, &def.text)
    }

    /// Starts `def` by hand with the `[params]` values `set` (the rest take their default),
    /// at the job `from` when there is one: the jobs before it are ⊘ スキップ ([`Run::starting_at`]).
    pub fn start_manual(
        &mut self,
        def: &Def,
        date: NaiveDate,
        from: Option<&str>,
        set: &BTreeMap<String, String>,
    ) -> Result<String, String> {
        let run = self.prepare(def, date, How::Manual, None, from, set)?;
        self.launch(run, &def.text)
    }

    /// Starts `def` for today because something happened: `how` says what
    /// (a file came, the engine started), `reason` goes on the run, and `file`
    /// is the file that came, `KOYOMI_FILE` for its jobs.
    pub fn start_event(&mut self, def: &Def, date: NaiveDate, how: How, reason: &str, file: Option<String>) -> Result<String, String> {
        let mut run = self.prepare(def, date, how, None, None, &BTreeMap::new())?;
        run.reason = reason.to_owned();
        run.file = file;
        self.launch(run, &def.text)
    }

    /// A new run of `def`, not launched yet: the net expanded, no other run of it going.
    fn prepare(
        &mut self,
        def: &Def,
        date: NaiveDate,
        how: How,
        planned: Option<NaiveDateTime>,
        from: Option<&str>,
        set: &BTreeMap<String, String>,
    ) -> Result<Run, String> {
        let net = self.expand(&def.id, def.net.as_ref().ok_or_else(|| format!("{}: 定義の誤り", def.file))?, date, set)?;
        self.not_running(&def.id, def.label(), net.lock.as_deref(), net.lock_shared)?;
        let now = Local::now().naive_local();
        let id = self.new_id(now);
        let date = date.format("%Y-%m-%d").to_string();
        let mut run = match from {
            Some(from) => Run::starting_at(&id, &def.id, def.label(), &date, how, &net, from, now)?,
            None => Run::new(&id, &def.id, def.label(), &date, how, &net, now),
        };
        run.planned = planned;
        run.lock = net.lock.clone();
        run.lock_shared = net.lock_shared;
        run.priority = net.slot_priority();
        run.require = net.require;
        run.params = set.clone();
        Ok(run)
    }

    /// Starts run `previous` again with the definition it ran, the copy beside
    /// its logs ([`Run::rerun`]). `previous` may be of another day. With `next`, `from` does not
    /// run but ends as that mark and the jobs after it go ([`Run::rerun_after`]).
    pub fn rerun(&mut self, previous: &str, from: Option<&str>, next: Option<State>) -> Result<String, String> {
        let prev = self.previous(previous)?;
        if !prev.is_over() {
            return Err(format!("{}: まだ終わっていない", prev.id));
        }
        let (lock, shared) = std::fs::read_to_string(self.run_dir(previous).join("definition.toml"))
            .ok()
            .and_then(|t| jobnet::parse(&t).ok())
            .map(|n| (n.lock, n.lock_shared))
            .unwrap_or_default();
        self.not_running(&prev.jobnet, &prev.title, lock.as_deref(), shared)?;
        let path = self.run_dir(previous).join("definition.toml");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let net =
            jobnet::parse(&text).map_err(|ps| ps.iter().map(|p| p.in_file(&path.display().to_string())).collect::<Vec<_>>().join(" / "))?;
        // The values a person gave the first time go again (`params.toml` beside the definition).
        let set: BTreeMap<String, String> = kept::read(&self.run_dir(previous).join(PARAMS)).unwrap_or_default();
        let net = self.expand(&prev.jobnet, &net, prev.date.parse().map_err(|_| format!("{}: 業務日が読めない", prev.id))?, &set)?;
        let now = Local::now().naive_local();
        let id = self.new_id(now);
        let mut run = match (from, next) {
            (Some(job), Some(mark)) => Run::rerun_after(&id, &prev, &net, job, mark, now)?,
            (None, Some(_)) => return Err("次から流すにはジョブが要る".to_owned()),
            _ => Run::rerun(&id, &prev, &net, from, now)?,
        };
        run.lock = net.lock.clone();
        run.lock_shared = net.lock_shared;
        run.priority = net.slot_priority();
        run.require = net.require;
        run.params = set;
        self.launch(run, &text)
    }

    /// Starts `theirs`, a run the primary `from` had going when this standby took over, again
    /// here with `def` from the first job that had not ended ok ([`Run::taken_over`]), with the
    /// values it was given.
    pub fn resume(&mut self, def: &Def, theirs: &Run, from: &str) -> Result<String, String> {
        let net = def.net.as_ref().ok_or_else(|| format!("{}: 定義の誤り", def.file))?;
        let date: NaiveDate = theirs.date.parse().map_err(|_| format!("{}: 業務日が読めない", theirs.id))?;
        let net = self.expand(&def.id, net, date, &theirs.params)?;
        self.not_running(&def.id, def.label(), net.lock.as_deref(), net.lock_shared)?;
        let now = Local::now().naive_local();
        let id = self.new_id(now);
        let mut run = Run::taken_over(&id, theirs, &net, from, now)?;
        run.lock = net.lock.clone();
        run.lock_shared = net.lock_shared;
        run.priority = net.slot_priority();
        run.require = net.require;
        run.params = theirs.params.clone();
        self.launch(run, &def.text)
    }

    /// Writes `theirs` down here as a run `from` had going that this standby did not take over,
    /// over at once with `why` ([`Run::left_over`]), beside the definition here when there is
    /// one, so `koyomi rerun` can start it from where it was. Its new id.
    pub fn left(&mut self, theirs: &Run, def: Option<&Def>, from: &str, why: &str) -> String {
        let now = Local::now().naive_local();
        let id = self.new_id(now);
        let run = Run::left_over(&id, theirs, from, why, now);
        if let Some(d) = def {
            let dir = self.run_dir(&id);
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("definition.toml"), &d.text);
            if let Some(kept) = Some(&run.params).filter(|p| !p.is_empty()).and_then(|p| toml::to_string(p).ok()) {
                let _ = std::fs::write(dir.join(PARAMS), kept);
            }
        }
        self.record(&run, history::run_start(&run));
        for j in run.jobs.iter().filter(|j| j.ended.is_some()) {
            self.record(&run, history::job_end(&run, j));
        }
        self.record(&run, history::run_end(&run));
        self.runs.push(run);
        id
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

    /// No other run of `jobnet` is going, and the net may take its `lock` (`shared` or not)
    /// beside the runs that hold it now ([`admit`], `[locks]`'s count).
    fn not_running(&self, jobnet: &str, label: &str, lock: Option<&str>, shared: bool) -> Result<(), String> {
        if self.runs.iter().any(|r| r.jobnet == jobnet && !r.is_over()) {
            return Err(format!("{label}: 動いている間はもう一度は流せない"));
        }
        if let Some(l) = lock {
            let holders: Vec<Holder> = self
                .runs
                .iter()
                .filter(|r| !r.is_over() && r.lock.as_deref() == Some(l))
                .map(|r| Holder { title: &r.title, shared: r.lock_shared })
                .collect();
            admit(l, shared, self.config.lock_count(l), &holders).map_err(|e| format!("{label}: {e}"))?;
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
        if !run.params.is_empty() {
            let kept = toml::to_string(&run.params).map_err(|e| e.to_string())?;
            std::fs::write(dir.join(PARAMS), kept).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
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

    /// Stops every run that is not over, so no job's process outlives the engine, and gives the
    /// jobs `grace` to be reaped (and written down as 強制終了).
    pub fn stop_all(&mut self, grace: std::time::Duration) {
        let ids: Vec<String> = self.runs.iter().filter(|r| !r.is_over()).map(|r| r.id.clone()).collect();
        for id in ids {
            let _ = self.kill(&id, None);
        }
        let end = std::time::Instant::now() + grace;
        while self.busy() > 0 && std::time::Instant::now() < end {
            self.tick();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        self.tick();
        if self.busy() > 0 {
            crate::say(&format!("koyomi: {} job(s) did not stop within {grace:?}", self.busy()), true);
        }
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
                if j.part.waits() {
                    // A wait has no process: it ends here.
                    self.waits.retain(|w| !(w.run == run && w.job == job));
                    if let Some(r) = self.end_part(run, job, State::Killed, "強制終了", None, now) {
                        self.ended(r);
                    }
                    return Ok(format!("{run} の {job} を止めた"));
                }
                let a = self.active.iter().find(|a| a.run == run && a.job == job).ok_or_else(|| format!("{job}: 動いていない"))?;
                ask_stop(&a.why, &a.stop, WHY_KILLED);
                Ok(format!("{run} の {job} を止める"))
            }
            None => {
                r.kill_pending(now);
                self.waits.retain(|w| w.run != run);
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
            match done.outcome {
                _ if done.reason.is_empty() => r.finish(&done.job, done.outcome, now),
                // A stop's reason says how it went (an `ssh` job's far command was asked too).
                Outcome::Killed | Outcome::Timeout => r.finish_noted(&done.job, done.outcome, &done.reason, now),
                _ => r.fail(&done.job, &done.reason, now),
            }
            let r = r.clone();
            if let Some(j) = r.job(&done.job) {
                self.record(&r, history::job_end(&r, j));
            }
            if r.is_over() {
                over.push(r);
            }
        }
        // The parts that wait: a time that came, a file that is there, or a wait that ran out.
        let mut i = 0;
        while i < self.waits.len() {
            let now = Local::now().naive_local();
            let w = &mut self.waits[i];
            let still = self.runs.iter().find(|r| r.id == w.run).and_then(|r| r.job(&w.job)).is_some_and(|j| j.state == State::Running);
            if !still {
                // Stopped by a kill, or gone with its run.
                self.waits.remove(i);
                continue;
            }
            let Some((state, reason, file)) = w.due.check(now) else {
                i += 1;
                continue;
            };
            let w = self.waits.remove(i);
            over.extend(self.end_part(&w.run, &w.job, state, &reason, file, now));
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

    /// Starts what is ready: a job's process, or a part's wait, notice, or end.
    fn start_ready(&mut self) {
        let limit = self.config.concurrency.max(1);
        let mut running = self.slots_used();
        let mut starts: Vec<(String, String, usize)> = Vec::new();
        // What needs a slot takes the free ones by the net's priority (5 first), then in the order
        // the runs began.
        let mut wanting: Vec<(usize, String)> = Vec::new();
        for r in self.runs.iter_mut() {
            for job in r.ready() {
                if let Some(by) = self.held.job_held_by(&r.jobnet, &job) {
                    let reason = format!("保留中（koyomi release {} {by}）", r.jobnet);
                    r.hold_job(&job, &reason);
                }
            }
        }
        for (i, r) in self.runs.iter().enumerate() {
            for job in r.ready() {
                // A part runs no process: it takes no slot.
                if r.job(&job).is_none_or(|j| j.part.runs_process()) {
                    wanting.push((i, job));
                } else {
                    starts.push((r.id.clone(), job, 1));
                }
            }
        }
        wanting.sort_by_key(|(i, _)| std::cmp::Reverse(self.runs[*i].priority));
        for (i, job) in wanting {
            let r = &mut self.runs[i];
            if let Some(why) = unmet(&r.require) {
                r.wait_because(&job, &why);
            } else if running < limit {
                let width = r.job(&job).map_or(1, |j| width(&j.part, limit - running));
                running += width;
                starts.push((r.id.clone(), job, width));
            } else {
                r.wait_for_slot(&job, limit);
            }
        }
        let mut over = Vec::new();
        for (run_id, job_id, width) in starts {
            let now = Local::now().naive_local();
            let Some(j) = self.runs.iter().find(|r| r.id == run_id).and_then(|r| r.job(&job_id)) else { continue };
            let (part, timeout) = (j.part.clone(), j.timeout);
            match part {
                Part::Job | Part::Loop { .. } | Part::Chore(_) => self.start_job(&run_id, &job_id, now, width),
                Part::Notify => {
                    self.notices.push((run_id.clone(), job_id.clone()));
                    over.extend(self.end_part(&run_id, &job_id, State::Ok, "通知した", None, now));
                }
                Part::Time { at, days } => {
                    let date = self.runs.iter().find(|r| r.id == run_id).and_then(|r| NaiveDate::parse_from_str(&r.date, "%Y-%m-%d").ok());
                    let due = date.and_then(|d| d.checked_add_days(chrono::Days::new(u64::from(days)))).map(|d| d.and_time(at));
                    match due {
                        Some(due) if due > now => {
                            let text = time_text(due, days);
                            let reason = format!("{text} を待っています");
                            self.wait_on(&run_id, &job_id, &reason, Due::Time { at: due, text }, now);
                        }
                        Some(due) => {
                            let reason = format!("{} を過ぎていた", time_text(due, days));
                            over.extend(self.end_part(&run_id, &job_id, State::Ok, &reason, None, now));
                        }
                        None => over.extend(self.end_part(&run_id, &job_id, State::Error, "日付が読めない", None, now)),
                    }
                }
                Part::Approval => {
                    self.wait_on(&run_id, &job_id, "確認を待っています", Due::Person, now);
                    self.asks.push((run_id.clone(), job_id.clone()));
                }
                Part::File { file, settle } => {
                    let due = Due::File { pattern: file.clone(), settle, timeout, since: Instant::now(), seen: None };
                    self.wait_on(&run_id, &job_id, &format!("{file} を待っています"), due, now);
                }
                // 分岐・待ち合わせ・子ジョブネット end in `Run::settle`, never here.
                Part::Branch | Part::Join { .. } | Part::Jobnet { .. } => {}
            }
        }
        for r in over {
            self.ended(r);
        }
    }

    /// Starts the job's process on a thread of its own, holding `width` slots.
    fn start_job(&mut self, run_id: &str, job_id: &str, now: NaiveDateTime, width: usize) {
        let Some(r) = self.runs.iter_mut().find(|r| r.id == run_id) else { return };
        r.start(job_id, now);
        let r = r.clone();
        let Some(j) = r.job(job_id) else { return };
        self.record(&r, history::job_start(&r, j));
        let stop = Arc::new(AtomicBool::new(false));
        let why = Arc::new(AtomicU8::new(WHY_NONE));
        self.active.push(Active { run: run_id.to_owned(), job: job_id.to_owned(), stop: stop.clone(), why: why.clone(), slots: width });
        let log = self.log_path(run_id, job_id);
        let job = JobSpec {
            run: run_id.to_owned(),
            jobnet: r.jobnet.clone(),
            date: r.date.clone(),
            id: job_id.to_owned(),
            command: j.command(),
            dir: j.dir.clone(),
            timeout: j.timeout,
            grace: grace(&j.part),
            retry: j.retry,
            judge: j.judge.clone(),
            env: j.env.clone(),
            encoding: j.encoding,
            run_as: j.run_as.clone(),
            file: r.file_for(job_id),
            each: match &j.part {
                Part::Loop { file: Some(f), .. } => Some(Each::Files(f.clone())),
                Part::Loop { count: Some(n), .. } => Some(Each::Count(*n)),
                _ => None,
            },
            index: None,
            width,
            io_ready: false,
            log,
            cap: self.config.log_max_bytes(),
        };
        let tx = self.tx.clone();
        let _ = std::thread::Builder::new().name("koyomi-job".into()).spawn(move || {
            let done = run_job(&job, &stop, &why);
            let _ = tx.send(done);
        });
    }

    /// A part starts to wait: 実行中 with `reason`, until `due` ends it.
    fn wait_on(&mut self, run_id: &str, job_id: &str, reason: &str, due: Due, now: NaiveDateTime) {
        let Some(r) = self.runs.iter_mut().find(|r| r.id == run_id) else { return };
        r.wait_on(job_id, reason, now);
        let r = r.clone();
        if let Some(j) = r.job(job_id) {
            self.record(&r, history::job_start(&r, j));
        }
        self.waits.push(Waiting { run: run_id.to_owned(), job: job_id.to_owned(), due });
    }

    /// A part that runs no process ends; the run comes back when that ended it.
    fn end_part(
        &mut self,
        run_id: &str,
        job_id: &str,
        state: State,
        reason: &str,
        file: Option<String>,
        now: NaiveDateTime,
    ) -> Option<Run> {
        let r = self.runs.iter_mut().find(|r| r.id == run_id)?;
        r.end_part(job_id, state, reason, file, now);
        let r = r.clone();
        if let Some(j) = r.job(job_id) {
            self.record(&r, history::job_end(&r, j));
        }
        r.is_over().then_some(r)
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

    /// Marks run `id` late in `delay` at `at`, and writes it down (once).
    pub fn mark_delay(&mut self, id: &str, delay: Delay, at: NaiveDateTime) {
        let Some(r) = self.runs.iter_mut().find(|r| r.id == id) else { return };
        if r.delays.contains(&delay) {
            return;
        }
        r.delays.push(delay);
        let run = r.clone();
        self.record(&run, history::Event::Late { at, run: run.id.clone(), delay });
    }

    /// Writes down a thing a person did by hand, in the file of today.
    pub fn record_op(&mut self, op: &str, target: &str, detail: &str, by: &str) {
        let at = Local::now().naive_local();
        let day = at.date();
        let (d, w) = self.days.entry(day).or_insert_with(|| {
            let file = history::path(&self.history, day);
            (Day::load(&file).unwrap_or_default(), kept::Writer::new(file, true))
        });
        d.events.push(history::Event::Op {
            at,
            op: op.to_owned(),
            target: target.to_owned(),
            detail: detail.to_owned(),
            by: by.to_owned(),
        });
        w.save_toml(d);
    }

    /// The things done by hand from `from` to `to` (both days), oldest first, one line each.
    pub fn ops(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<String>, String> {
        if to < from {
            return Err(format!("範囲が逆: {from} → {to}"));
        }
        if (to - from).num_days() > 366 {
            return Err("範囲は 1 年まで".to_owned());
        }
        let mut out = Vec::new();
        let mut day = from;
        while day <= to {
            let loaded;
            let events = match self.days.get(&day) {
                Some((d, _)) => &d.events,
                None => {
                    loaded = Day::load(&history::path(&self.history, day)).unwrap_or_default();
                    &loaded.events
                }
            };
            for e in events {
                if let history::Event::Op { at, op, target, detail, by } = e {
                    let sep = if detail.is_empty() { "" } else { "  " };
                    let by = if by.is_empty() { String::new() } else { format!("  （HTTP: {by}）") };
                    out.push(format!("{}  {op:<8} {target}{sep}{detail}{by}", at.format("%Y-%m-%d %H:%M:%S")));
                }
            }
            let Some(next) = day.succ_opt() else { break };
            day = next;
        }
        Ok(out)
    }

    /// Writes what is pending before the engine ends. The runs' processes keep going on their own.
    pub fn flush(&self) {
        for (d, w) in self.days.values() {
            let _ = w.save_now(d.to_toml());
        }
    }
}

#[derive(Clone)]
struct JobSpec {
    run: String,
    jobnet: String,
    date: String,
    id: String,
    command: Vec<String>,
    dir: Option<String>,
    /// Seconds before the timer asks the job to stop.
    timeout: Option<u64>,
    /// How long a stop waits after asking before it kills ([`grace`]).
    grace: Duration,
    retry: Option<Retry>,
    /// What the exit codes mean: a 警告 end is not run again.
    judge: Option<Judge>,
    /// The job's `env`; `secret:<name>` values are read when it starts.
    env: BTreeMap<String, String>,
    /// How the process's output is read.
    encoding: jobnet::Encoding,
    /// The account it runs as; its password is read when it starts.
    run_as: Option<RunAs>,
    /// The file a `kind = "file"` part before it found: `KOYOMI_FILE`.
    file: Option<String>,
    /// A loop part's passes; `None` for a job, which runs once.
    each: Option<Each>,
    /// The pass of a loop, from 1: `KOYOMI_INDEX`.
    index: Option<u32>,
    /// How many passes of a loop run side by side: the slots it holds.
    width: usize,
    /// `KOYOMI_IN` is written and `KOYOMI_OUT` cleared already (by a loop whose passes run side by side).
    io_ready: bool,
    log: PathBuf,
    cap: u64,
}

/// How long an `ssh` job asked to stop has before it is killed. `koyomi ssh run` sends
/// TERM to the far command and closes the channel, which ito-sftp gives 2 s; 10 s
/// leaves room for a slow link while a kill or a `timeout` still ends it soon.
const SSH_GRACE: Duration = Duration::from_secs(10);

/// How long a stop waits after asking `part` before it kills: [`SSH_GRACE`] for an
/// `ssh` job, which can pass the ask on to the far side. Anything else is killed at
/// once, as before: a script or program that does not watch for the ask would only
/// make a kill slower. Under `run_as` nothing can be asked: `ito_proc` gives another
/// account's process neither CTRL_BREAK nor the stop event its own children get
/// (`ITO_PROC_STOP_EVENT`), so an `ssh` job there is killed at once too, and its
/// far command is left to end by itself.
fn grace(part: &Part) -> Duration {
    match part {
        Part::Chore(Chore::Ssh { .. }) => SSH_GRACE,
        _ => Duration::ZERO,
    }
}

/// The slots a part takes when `free` are free: a loop with `max` as many of them as its
/// passes may use (at least 1), anything else 1.
fn width(part: &Part, free: usize) -> usize {
    match part {
        Part::Loop { max: Some(m), count, .. } => {
            let most = count.map_or(*m, |n| n.min(*m));
            (most as usize).min(free).max(1)
        }
        _ => 1,
    }
}

/// What a loop part runs over.
#[derive(Clone, Debug)]
enum Each {
    /// Once per file matching the pattern, by name.
    Files(String),
    /// This many times.
    Count(u32),
}

/// The `encoding` of a job as ito-proc reads it.
fn decoding(e: jobnet::Encoding) -> ito_proc::Encoding {
    match e {
        jobnet::Encoding::Auto => ito_proc::Encoding::Auto,
        jobnet::Encoding::Sjis => ito_proc::Encoding::ShiftJis,
        jobnet::Encoding::Utf8 => ito_proc::Encoding::Utf8,
    }
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
    let get = |name: &str| ito_secret::get("koyomi", name);
    let env = match resolve_env(&job.env, &get).and_then(|env| resolve_logon(env, job.run_as.as_ref(), &get)) {
        Ok(r) => r,
        Err(e) => {
            let _ = writeln!(file, "! {e}");
            return done(Outcome::Failed, e, 0);
        }
    };
    if let Some(r) = &job.run_as {
        let _ = writeln!(file, "▶ {} として動かす", r.user);
    }
    let Some(each) = &job.each else {
        let (outcome, reason, retried) = tries(job, (exe, args), &env, stop, why, &mut file, &mut size);
        return done(outcome, reason, retried);
    };
    // A loop: the passes one after another, each with its own tries. An abnormal
    // pass ends the loop; a 警告 pass is remembered and the loop goes on.
    let passes: Vec<(u32, Option<String>)> = match each {
        Each::Files(pattern) => {
            super::files::find_files(pattern).into_iter().zip(1..).map(|((p, ..), i)| (i, Some(p.display().to_string()))).collect()
        }
        Each::Count(n) => (1..=*n).map(|i| (i, None)).collect(),
    };
    let total = passes.len();
    if total == 0 {
        let _ = writeln!(file, "! 合うファイルが無いので、1 回も動かさない");
        return done(Outcome::Exited(0), String::new(), 0);
    }
    if job.width > 1 && total > 1 {
        let (outcome, reason, retried) = side_by_side(job, (exe, args), &env, &passes, (stop, why), &mut file, &mut size);
        return done(outcome, reason, retried);
    }
    let mut retried = 0;
    let mut warned = None;
    for (i, f) in passes {
        if stop.load(Ordering::SeqCst) {
            let _ = writeln!(file, "! 強制終了");
            return done(Outcome::Killed, String::new(), retried);
        }
        let mut pass = job.clone();
        pass.index = Some(i);
        if f.is_some() {
            pass.file = f.clone();
        }
        let what = f.as_deref().map(|f| format!(": {f}")).unwrap_or_default();
        let _ = writeln!(file, "▶ {i}/{total}{what}");
        let (outcome, reason, r) = tries(&pass, (exe, args), &env, stop, why, &mut file, &mut size);
        retried += r;
        let of = format!("{i}/{total} 回目{}", f.as_deref().map(|f| format!("（{f}）")).unwrap_or_default());
        match outcome {
            Outcome::Exited(code) => match verdict(job.judge.as_ref(), code) {
                State::Error => return done(Outcome::Failed, format!("{of}で異常: 終了コード {code}"), retried),
                State::Ok => {}
                _ => warned = warned.or(Some(code)),
            },
            Outcome::Failed => return done(Outcome::Failed, format!("{of}で異常: {reason}"), retried),
            Outcome::Killed | Outcome::Timeout => return done(outcome, String::new(), retried),
        }
    }
    done(Outcome::Exited(warned.unwrap_or(0)), String::new(), retried)
}

/// What a pass that ran aside sends back: its place in the passes, how it ended, and its lines.
type Pass = (usize, Outcome, String, u32, Vec<u8>);

/// A loop's passes, `job.width` of them at a time, each on a thread with its own stop (so a
/// pass's timeout ends only that pass). A pass writes its lines aside; they go into the log
/// under its heading when it ends. After an abnormal pass no more start, and the running ones
/// end as they would; a stop of the job reaches every running pass.
fn side_by_side(
    job: &JobSpec,
    command: (&str, &[String]),
    env: &Resolved,
    passes: &[(u32, Option<String>)],
    (stop, why): (&Arc<AtomicBool>, &Arc<AtomicU8>),
    file: &mut std::fs::File,
    size: &mut LogSize,
) -> (Outcome, String, u32) {
    let total = passes.len();
    let _ = std::fs::remove_file(out_path(&job.log, &job.id));
    let _ = std::fs::write(job.log.with_file_name(format!("{}.in.toml", job.id)), gather_outputs(&job.log, &job.id));
    let _ = writeln!(file, "▶ {total} 回を {} つずつ並べて動かす", job.width);
    let workers: Vec<_> =
        (0..job.width.min(total)).map(|_| (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU8::new(WHY_NONE)))).collect();
    let next = AtomicUsize::new(0);
    let halt = AtomicBool::new(false);
    let (tx, rx) = mpsc::channel::<Pass>();
    let mut retried = 0;
    let mut warned = None;
    let mut first: Option<(Outcome, String)> = None;
    std::thread::scope(|s| {
        for (own_stop, own_why) in &workers {
            let (tx, next, halt) = (tx.clone(), &next, &halt);
            s.spawn(move || {
                while !halt.load(Ordering::SeqCst) && !own_stop.load(Ordering::SeqCst) {
                    let k = next.fetch_add(1, Ordering::SeqCst);
                    let Some((i, f)) = passes.get(k) else { break };
                    let mut pass = job.clone();
                    pass.index = Some(*i);
                    pass.io_ready = true;
                    if f.is_some() {
                        pass.file = f.clone();
                    }
                    let mut lines = Vec::new();
                    let mut size = LogSize { written: 0, capped: false };
                    let (outcome, reason, r) = tries(&pass, command, env, own_stop, own_why, &mut lines, &mut size);
                    if !matches!(outcome, Outcome::Exited(code) if verdict(job.judge.as_ref(), code) != State::Error) {
                        halt.store(true, Ordering::SeqCst);
                    }
                    let _ = tx.send((k, outcome, reason, r, lines));
                }
            });
        }
        drop(tx);
        loop {
            if stop.load(Ordering::SeqCst) {
                let who = why.load(Ordering::SeqCst);
                for (own_stop, own_why) in &workers {
                    ask_stop(own_why, own_stop, who);
                }
            }
            let (k, outcome, reason, r, lines) = match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(p) => p,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            let (i, f) = &passes[k];
            let what = f.as_deref().map(|f| format!(": {f}")).unwrap_or_default();
            let mut text = format!("▶ {i}/{total}{what}\n").into_bytes();
            text.extend_from_slice(&lines);
            append(file, size, &text, job.cap);
            retried += r;
            let of = format!("{i}/{total} 回目{}", f.as_deref().map(|f| format!("（{f}）")).unwrap_or_default());
            let bad = match outcome {
                Outcome::Exited(code) => match verdict(job.judge.as_ref(), code) {
                    State::Error => Some((Outcome::Failed, format!("{of}で異常: 終了コード {code}"))),
                    State::Ok => None,
                    _ => {
                        warned = warned.or(Some(code));
                        None
                    }
                },
                Outcome::Failed => Some((Outcome::Failed, format!("{of}で異常: {reason}"))),
                Outcome::Killed | Outcome::Timeout => Some((outcome, String::new())),
            };
            if first.is_none() {
                first = bad;
            }
        }
    });
    if stop.load(Ordering::SeqCst) {
        return (Outcome::Killed, String::new(), retried);
    }
    match first {
        Some((outcome, reason)) => (outcome, reason, retried),
        None => (Outcome::Exited(warned.unwrap_or(0)), String::new(), retried),
    }
}

/// Puts `text` at the end of the log while it is under `cap`.
fn append(file: &mut std::fs::File, size: &mut LogSize, text: &[u8], cap: u64) {
    if size.capped {
        return;
    }
    size.written += text.len() as u64;
    if size.written > cap {
        size.capped = true;
        let _ = writeln!(file, "（ログが {} MB を超えたので、この先は書かない）", cap / (1024 * 1024));
    } else {
        let _ = file.write_all(text);
    }
}

/// Runs the job's process: once, or again after each abnormal end while its
/// `retry` allows. The last outcome, its reason and how many times it ran again.
fn tries(
    job: &JobSpec,
    command: (&str, &[String]),
    env: &Resolved,
    stop: &Arc<AtomicBool>,
    why: &Arc<AtomicU8>,
    file: &mut impl Write,
    size: &mut LogSize,
) -> (Outcome, String, u32) {
    let (count, wait) = job.retry.map_or((0, 0), |r| (r.count, r.wait));
    let mut retried = 0;
    loop {
        let (outcome, reason) = attempt(job, command, env, stop, why, file, size);
        let abnormal =
            matches!(outcome, Outcome::Exited(code) if verdict(job.judge.as_ref(), code) == State::Error) || outcome == Outcome::Failed;
        if !abnormal || retried >= count {
            return (outcome, reason, retried);
        }
        let _ = writeln!(file, "! やり直し ({}/{count}): {} 待つ", retried + 1, koyomi_core::duration::text(wait));
        // The wait, unless a person stops the job during it.
        let until = Instant::now() + Duration::from_secs(wait);
        loop {
            if stop.load(Ordering::SeqCst) {
                let _ = writeln!(file, "! 強制終了");
                return (Outcome::Killed, String::new(), retried);
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

/// The job's `env` ready for the process: plain values as they are, `secret:<name>` read
/// with `get`. Also the secret values, to hide them in the log. A missing secret is an error.
fn resolve_env(env: &BTreeMap<String, String>, get: &dyn Fn(&str) -> std::io::Result<Option<String>>) -> Result<Resolved, String> {
    let mut out = Resolved::default();
    for (k, v) in env {
        match v.strip_prefix("secret:") {
            None => out.vars.push((k.clone(), v.clone())),
            Some(name) => match get(name) {
                Ok(Some(s)) => {
                    out.vars.push((k.clone(), s.clone()));
                    out.secrets.push(s);
                }
                Ok(None) => return Err(format!("秘密がない: {name}（koyomi secret set {name}）")),
                Err(e) => return Err(format!("秘密を読めない: {name}: {e}")),
            },
        }
    }
    Ok(out)
}

/// Adds the password of the job's `run_as` account, read with `get`, to `env`
/// (and to what the log hides). A missing secret is an error.
fn resolve_logon(
    mut env: Resolved,
    run_as: Option<&RunAs>,
    get: &dyn Fn(&str) -> std::io::Result<Option<String>>,
) -> Result<Resolved, String> {
    let Some(r) = run_as else { return Ok(env) };
    let name = &r.secret;
    match get(name) {
        Ok(Some(password)) => {
            env.secrets.push(password.clone());
            env.logon = Some((r.clone(), password));
            Ok(env)
        }
        Ok(None) => Err(format!("{} のパスワードの秘密がない: {name}（koyomi secret set {name}）", r.user)),
        Err(e) => Err(format!("秘密を読めない: {name}: {e}")),
    }
}

/// Runs `cmd` as `account` (`run_as`), its lines to `on_line` as `run_lines_soft` does.
/// Another account's process cannot be asked to stop: whatever `grace` is, a stop
/// kills it at once (`ito_proc` ends it as `Killed`).
fn run_as(
    cmd: std::process::Command,
    account: &RunAs,
    password: &str,
    stop: &AtomicBool,
    grace: Duration,
    enc: ito_proc::Encoding,
    on_line: &mut dyn FnMut(ito_proc::Stream, &str) -> bool,
) -> std::io::Result<ito_proc::Lines> {
    let (domain, user) = account.split();
    let logon = ito_proc::Logon { user, domain, password };
    ito_proc::run_lines_as_soft(cmd, &logon, stop, grace, enc, on_line)
}

/// `exe` as a full path: a bare name is looked up on this process's `PATH`
/// (with `PATHEXT`), since another account's process does not look it up.
fn full_program(exe: &str) -> String {
    let p = std::path::Path::new(exe);
    if p.components().count() > 1 || p.is_absolute() {
        return exe.to_owned();
    }
    let exts: Vec<String> = if p.extension().is_some() {
        vec![String::new()]
    } else {
        std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into()).split(';').map(str::to_owned).collect()
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .flat_map(|dir| exts.iter().map(move |e| dir.join(format!("{exe}{e}"))))
        .find(|c| c.is_file())
        .map_or_else(|| exe.to_owned(), |c| c.display().to_string())
}

/// A job's `env` read: what to pass, and which of it to hide in the log.
#[derive(Default)]
struct Resolved {
    vars: Vec<(String, String)>,
    secrets: Vec<String>,
    /// The `run_as` account and its password.
    logon: Option<(RunAs, String)>,
}

/// Whether the `ok_file` of a job's `judge` exists (a relative one is read from the job's `dir`).
fn ok_file_exists(file: &str, dir: Option<&str>) -> bool {
    let p = std::path::Path::new(file);
    match dir {
        Some(d) if p.is_relative() => std::path::Path::new(d).join(p).exists(),
        _ => p.exists(),
    }
}

/// Where a job writes the values it hands on (`KOYOMI_OUT`): beside its log.
fn out_path(log: &std::path::Path, job: &str) -> PathBuf {
    log.with_file_name(format!("{job}.out.toml"))
}

/// The file a job's `KOYOMI_REASON` names: one per pass, as passes may run side by side.
fn reason_path(log: &std::path::Path, job: &str, index: Option<u32>) -> PathBuf {
    let pass = index.map(|i| format!(".{i}")).unwrap_or_default();
    log.with_file_name(format!("{job}{pass}.reason"))
}

/// What a job left in its `KOYOMI_REASON` file, taken away; `None` when it left nothing.
fn left_reason(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let _ = std::fs::remove_file(path);
    Some(text.trim().to_owned()).filter(|t| !t.is_empty())
}

/// What the other jobs of a run wrote to their `KOYOMI_OUT`, as one TOML text
/// with a table per job (`[extract]` then `rows = 3`). A file that is not TOML
/// is left out. `log` is the job's own log path, which names the run's folder.
fn gather_outputs(log: &std::path::Path, own: &str) -> String {
    let Some(dir) = log.parent() else { return String::new() };
    let mut all = toml::Table::new();
    let mut files: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    files.sort();
    for path in files {
        let Some(job) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".out.toml")) else { continue };
        if job == own {
            continue;
        }
        if let Some(t) = std::fs::read_to_string(&path).ok().and_then(|s| s.parse::<toml::Table>().ok()) {
            all.insert(job.to_owned(), toml::Value::Table(t));
        }
    }
    toml::to_string(&all).unwrap_or_default()
}

/// Runs the job's process once, to its end.
fn attempt(
    job: &JobSpec,
    (exe, args): (&str, &[String]),
    env: &Resolved,
    stop: &Arc<AtomicBool>,
    why: &Arc<AtomicU8>,
    file: &mut impl Write,
    size: &mut LogSize,
) -> (Outcome, String) {
    let program = if env.logon.is_some() { full_program(exe) } else { exe.to_owned() };
    let mut cmd = std::process::Command::new(&program);
    cmd.args(args);
    if let Some(dir) = &job.dir {
        cmd.current_dir(dir);
    }
    // Values between jobs: the job writes TOML to KOYOMI_OUT; KOYOMI_IN is
    // what the other jobs of the run wrote, a table per job.
    let out = out_path(&job.log, &job.id);
    let input = job.log.with_file_name(format!("{}.in.toml", job.id));
    if !job.io_ready {
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::write(&input, gather_outputs(&job.log, &job.id));
    }
    cmd.env("KOYOMI_OUT", &out).env("KOYOMI_IN", &input);
    // An SFTP `send` or `fetch` runs `koyomi sftp run` from its script (chore.rs).
    if let Ok(exe) = std::env::current_exe() {
        cmd.env("KOYOMI_EXE", exe);
    }
    // Where `koyomi ssh run` leaves why its command has no exit code (no connection,
    // a signal, a lost connection): the job's ✕ reason.
    let reason = reason_path(&job.log, &job.id, job.index);
    let _ = std::fs::remove_file(&reason);
    cmd.env("KOYOMI_REASON", &reason);
    cmd.env("KOYOMI_JOBNET", &job.jobnet).env("KOYOMI_JOB", &job.id).env("KOYOMI_RUN", &job.run).env("KOYOMI_DATE", &job.date);
    if let Some(f) = &job.file {
        cmd.env("KOYOMI_FILE", f);
    }
    if let Some(i) = job.index {
        cmd.env("KOYOMI_INDEX", i.to_string());
    }
    cmd.envs(env.vars.iter().map(|(k, v)| (k, v)));
    let hidden: Vec<&str> = env.secrets.iter().map(String::as_str).collect();
    let bad_output = job.judge.as_ref().and_then(|g| g.error_output.as_deref()).and_then(|p| ito_match::Matcher::new(p).ok());
    let mut bad_line: Option<String> = None;
    let mut on_line = |stream: ito_proc::Stream, line: &str| {
        if bad_line.is_none() && bad_output.as_ref().is_some_and(|m| m.is_match(line)) {
            bad_line = Some(ito_secret::mask(line, &hidden));
        }
        if size.capped {
            return false;
        }
        let prefix = if stream == ito_proc::Stream::Err { "! " } else { "" };
        let text = format!("{prefix}{}\n", ito_secret::mask(line, &hidden));
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
    let result = match &env.logon {
        None => ito_proc::run_lines_soft(cmd, stop, job.grace, decoding(job.encoding), &mut on_line),
        Some((account, password)) => run_as(cmd, account, password, stop, job.grace, decoding(job.encoding), &mut on_line),
    };
    // The process is over either way: let the timer go, and say in the log who stopped the job.
    over.store(true, Ordering::SeqCst);
    if let Some(t) = timer {
        let _ = t.join();
    }
    let left = left_reason(&reason);
    match result {
        // Asked (it ended within its grace) has an exit status, so `stopped()` alone would miss it.
        Ok(lines) if lines.ending != ito_proc::Ending::Exited => {
            // How it took the ask, from what it left in KOYOMI_REASON: `koyomi ssh run` says the far
            // command was asked to stop too. A kill has nothing to add.
            let note = match lines.ending {
                ito_proc::Ending::Asked => left.unwrap_or_default(),
                _ => String::new(),
            };
            if why.load(Ordering::SeqCst) == WHY_TIMEOUT {
                let text = koyomi_core::duration::text(job.timeout.unwrap_or(0));
                let _ = writeln!(file, "! 打ち切り（{text} を超えた）");
                (Outcome::Timeout, note)
            } else {
                let _ = writeln!(file, "! 強制終了");
                (Outcome::Killed, note)
            }
        }
        Ok(lines) => {
            let code = lines.status.and_then(|s| s.code()).unwrap_or(-1);
            // The job has said why in the log already (`! …`).
            if let Some(why) = left {
                return (Outcome::Failed, why);
            }
            if let Some(line) = bad_line {
                let _ = writeln!(file, "! 出力が error_output に合った");
                return (Outcome::Failed, format!("出力に異常の行があった: {}", line.trim()));
            }
            let found = job.judge.as_ref().and_then(|g| g.ok_file.as_deref()).filter(|f| ok_file_exists(f, job.dir.as_deref()));
            if let (Some(f), true) = (found, verdict(job.judge.as_ref(), code) != State::Ok) {
                let _ = writeln!(file, "! {f} があるので正常（終了コード {code}）");
                return (Outcome::Exited(0), String::new());
            }
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
            calendar: None,
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

    #[cfg(windows)]
    #[test]
    fn encoding_sjis_reads_the_output_as_shift_jis() {
        let dir = scratch("sjis");
        // "日本" in Shift_JIS.
        std::fs::write(dir.join("out.txt"), [0x93, 0xFA, 0x96, 0x7B, b'\r', b'\n']).unwrap();
        let file = dir.join("out.txt").display().to_string();
        let text = format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"cmd.exe\", \"/c\", \"type\", '{file}']\nencoding = \"sjis\"\n");
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        assert_eq!(wait_over(&mut r, &id).state, State::Ok);
        assert_eq!(r.log_tail(&id, "a").unwrap().trim(), "日本");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_standby_resumes_the_primarys_run_from_where_it_was_or_writes_it_down() {
        let dir = scratch("resume");
        let text = format!("format = 1\n[[job]]\nid = \"a\"\n{}\n[[job]]\nid = \"b\"\nafter = [\"a\"]\n{}\n", echo("one"), echo("two"));
        let d = def("net", &text);
        // What the standby read from the primary: a over, b still going.
        let mut theirs = {
            let mut a = Runner::new(&dir.join("a"), Config::default());
            let id = a.start(&d, Local::now().date_naive(), How::Schedule, None).unwrap();
            wait_over(&mut a, &id)
        };
        let b = theirs.jobs.iter_mut().find(|j| j.id == "b").unwrap();
        b.state = State::Running;
        b.ended = None;
        theirs.state = State::Running;
        theirs.ended = None;
        let mut r = Runner::new(&dir.join("b"), Config::default());
        let again = r.resume(&d, &theirs, "pc-a").unwrap();
        let run = wait_over(&mut r, &again);
        assert_eq!((run.state, run.job("a").unwrap().reason.as_str()), (State::Ok, koyomi_core::run::CARRIED));
        assert!(!r.log_path(&again, "a").exists(), "a ended ok there and must not run again");
        assert!(r.log_path(&again, "b").exists());
        assert!(run.reason.contains("pc-a で動いていた"), "{}", run.reason);
        // Not taken over: written down, over at once, and a rerun goes on from where it was.
        let left = r.left(&theirs, Some(&d), "pc-a", "takeover = \"rerun\" ではない");
        let run = r.run(&left).unwrap().clone();
        assert_eq!((run.state, run.job("b").unwrap().state), (State::Missed, State::NotRun));
        assert!(run.reason.contains("引き継いでいない"), "{}", run.reason);
        let rerun = r.rerun(&left, None, None).unwrap();
        let run = wait_over(&mut r, &rerun);
        assert_eq!((run.state, run.job("a").unwrap().reason.as_str()), (State::Ok, koyomi_core::run::CARRIED));
        assert!(!r.log_path(&rerun, "a").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rerun_runs_the_failed_job_again_and_carries_the_one_that_passed() {
        let dir = scratch("rerun");
        let text = format!("format = 1\n[[job]]\nid = \"a\"\n{}\n[[job]]\nid = \"b\"\nafter = [\"a\"]\n{}\n", echo("one"), fail_cmd());
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let first = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        assert!(r.rerun(&first, None, None).unwrap_err().contains("まだ終わっていない"));
        assert_eq!(wait_over(&mut r, &first).state, State::Error);
        let second = r.rerun(&first, None, None).unwrap();
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
        let third = r.rerun(&first, Some("a"), None).unwrap();
        let run = wait_over(&mut r, &third);
        assert_eq!(run.job("a").unwrap().exit, Some(0));
        assert!(r.log_path(&third, "a").exists());
        // From the jobs after a: a does not run and ends as ⊘ スキップ, b runs.
        let fourth = r.rerun(&first, Some("a"), Some(State::Skipped)).unwrap();
        let run = wait_over(&mut r, &fourth);
        assert_eq!(run.job("a").unwrap().state, State::Skipped);
        assert!(!r.log_path(&fourth, "a").exists(), "a must not run");
        assert!(r.log_path(&fourth, "b").exists());
        assert!(r.rerun(&first, Some("b"), Some(State::Warn)).unwrap_err().contains("後ろに流すジョブがない"));
        assert!(r.rerun(&first, None, Some(State::Skipped)).unwrap_err().contains("ジョブが要る"));
        assert!(r.rerun(&first, Some("x"), None).unwrap_err().contains("ジョブがない: x"));
        assert!(r.rerun("20200101-000000-00", None, None).unwrap_err().contains("実行がない"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_held_job_stops_its_run_there_and_a_release_goes_on_in_the_same_run() {
        let dir = scratch("hold-job");
        let text = format!(
            "format = 1
[[job]]
id = \"a\"
{}
[[job]]
id = \"b\"
after = [\"a\"]
{}
[[job]]
id = \"c\"
after = [\"b\"]
{}
[[job]]
id = \"side\"
{}
",
            echo("one"),
            echo("two"),
            echo("three"),
            echo("side")
        );
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let mut held = Held::default();
        held.hold_job(&d.id, "b");
        r.set_held(&held);
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let until = Instant::now() + Duration::from_secs(20);
        while r.run(&id).unwrap().job("b").unwrap().state != State::Held || r.run(&id).unwrap().job("side").unwrap().ended.is_none() {
            assert!(Instant::now() < until, "b was not held");
            r.tick();
            std::thread::sleep(Duration::from_millis(20));
        }
        let run = r.run(&id).unwrap().clone();
        assert!(!run.is_over(), "a run held at b is not over");
        assert_eq!(run.job("a").unwrap().state, State::Ok);
        assert_eq!(run.job("side").unwrap().state, State::Ok, "a branch beside the held job goes on");
        assert!(run.job("b").unwrap().reason.contains(&format!("koyomi release {} b", d.id)));
        assert_eq!(run.job("c").unwrap().state, State::Waiting, "the jobs after it wait");
        assert!(!r.log_path(&id, "b").exists());
        r.tick();
        assert_eq!(r.run(&id).unwrap().job("b").unwrap().state, State::Held, "a tick does not let it go");
        r.set_held(&Held::default());
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Ok);
        assert_eq!(r.log_tail(&id, "c").unwrap().trim(), "three");
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
        let id = r.start_manual(&d, Local::now().date_naive(), Some("b"), &BTreeMap::new()).unwrap();
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Ok);
        assert_eq!(run.job("a").unwrap().state, State::Skipped);
        assert!(!r.log_path(&id, "a").exists());
        assert_eq!(r.log_tail(&id, "b").unwrap().trim(), "two");
        r.flush();
        let back = Runner::new(&dir, Config::default());
        assert_eq!(back.run(&id).unwrap().job("a").unwrap().state, State::Skipped);
        assert!(r.start_manual(&d, Local::now().date_naive(), Some("x"), &BTreeMap::new()).unwrap_err().contains("ジョブがない: x"));
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
        assert_eq!(r.busy(), 2);
        assert_eq!(run.job("c").unwrap().state, State::Waiting);
        assert_eq!(run.job("c").unwrap().reason, "同時に動かす数（2）の空きを待っています");
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Ok);
        assert_eq!(r.busy(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_free_slot_goes_to_the_net_with_the_higher_priority() {
        let dir = scratch("priority");
        let sleep = if cfg!(windows) { "run = [\"cmd.exe\", \"/c\", \"ping -n 2 127.0.0.1 > nul\"]" } else { "run = [\"sleep\", \"1\"]" };
        let net = |p: &str| {
            format!(
                "format = 1
{p}[[job]]
id = \"a\"
{sleep}
"
            )
        };
        let mut r = Runner::new(&dir, Config { concurrency: 1, ..Config::default() });
        let today = Local::now().date_naive();
        let first = r.start(&def("first", &net("")), today, How::Manual, None).unwrap();
        let low = r
            .start(
                &def(
                    "low",
                    &net("priority = 1
"),
                ),
                today,
                How::Manual,
                None,
            )
            .unwrap();
        let high = r
            .start(
                &def(
                    "high",
                    &net("priority = 5
"),
                ),
                today,
                How::Manual,
                None,
            )
            .unwrap();
        assert_eq!(r.run(&high).unwrap().priority, 5);
        wait_over(&mut r, &first);
        r.tick();
        assert_eq!(r.run(&high).unwrap().running(), 1, "the higher priority takes the slot first");
        assert_eq!(r.run(&low).unwrap().running(), 0);
        wait_over(&mut r, &high);
        wait_over(&mut r, &low);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An `ssh` job is asked to stop and given time to pass the ask on; anything else is killed at once.
    #[test]
    fn only_an_ssh_job_gets_a_grace_before_the_kill() {
        let ssh = Part::Chore(Chore::Ssh {
            on: "ssh://ops@web01".into(),
            command: "sleep 600".into(),
            key: None,
            host_key: None,
            password: None,
        });
        // More than the 2 s ito-sftp gives the far command's TERM and close.
        assert!(grace(&ssh) > Duration::from_secs(2));
        assert_eq!(grace(&Part::Job), Duration::ZERO);
        assert_eq!(grace(&Part::Chore(Chore::Zip { from: "a".into(), into: "b.zip".into() })), Duration::ZERO);
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
    fn stopping_the_engine_stops_the_running_jobs() {
        let dir = scratch("stop-all");
        let long = if cfg!(windows) { "run = [\"cmd.exe\", \"/c\", \"ping -n 30 127.0.0.1 > nul\"]" } else { "run = [\"sleep\", \"30\"]" };
        let d = def(
            "net",
            &format!(
                "format = 1
[[job]]
id = \"a\"
{long}
"
            ),
        );
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        assert_eq!(r.busy(), 1);
        let asked = Instant::now();
        r.stop_all(Duration::from_secs(5));
        assert_eq!(r.busy(), 0, "no process is left");
        assert!(asked.elapsed() < Duration::from_secs(5));
        assert_eq!(r.run(&id).unwrap().job("a").unwrap().state, State::Killed);
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
    fn a_line_of_output_can_make_the_end_abnormal_and_a_file_can_make_it_normal() {
        let dir = scratch("judge-extra");
        let run_of = |r: &mut Runner, text: &str| {
            let d = def("net", text);
            let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
            wait_over(r, &id)
        };
        let mut r = Runner::new(&dir, Config::default());
        let text = format!(
            "format = 1
[[job]]
id = \"a\"
{}
judge = {{ error_output = \"^error\" }}
",
            echo("ERROR boom")
        );
        let a = run_of(&mut r, &text).job("a").unwrap().clone();
        assert_eq!((a.state, a.exit), (State::Error, None), "{a:?}");
        assert!(a.reason.contains("ERROR boom"), "{}", a.reason);
        let text = format!(
            "format = 1
[[job]]
id = \"a\"
{}
judge = {{ error_output = \"^error\" }}
",
            echo("all fine")
        );
        assert_eq!(run_of(&mut r, &text).state, State::Ok);
        let made = dir.join("made.txt");
        std::fs::write(&made, "x").unwrap();
        let ok_file = made.display().to_string().replace(char::from(92), "/");
        let text = format!(
            "format = 1
[[job]]
id = \"a\"
{}
judge = {{ ok_file = '{ok_file}' }}
",
            fail_cmd()
        );
        let run = run_of(&mut r, &text);
        assert_eq!((run.state, run.job("a").unwrap().exit), (State::Ok, Some(0)));
        let text = format!(
            "format = 1
[[job]]
id = \"a\"
{}
judge = {{ ok_file = '{ok_file}.none' }}
",
            fail_cmd()
        );
        assert_eq!(run_of(&mut r, &text).state, State::Error);
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
    fn the_chores_delete_old_files_empty_a_folder_and_copy() {
        if !cfg!(windows) {
            return;
        }
        let dir = scratch("chores");
        let (work, trash, into) = (dir.join("work"), dir.join("trash"), dir.join("into"));
        std::fs::create_dir_all(trash.join("sub")).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        for f in ["old.log", "new.log", "old.txt"] {
            std::fs::write(work.join(f), "x").unwrap();
        }
        let old = std::time::SystemTime::now() - Duration::from_secs(20 * 86_400);
        for f in ["old.log", "old.txt"] {
            std::fs::File::options().write(true).open(work.join(f)).unwrap().set_modified(old).unwrap();
        }
        std::fs::write(trash.join("sub").join("a"), "x").unwrap();
        std::fs::write(trash.join("b"), "x").unwrap();
        let text = format!(
            "format = 1\n[[job]]\nid = \"d\"\nkind = \"delete\"\npath = '{}'\nmatch = \"*.log\"\nkeep_days = 10\n\
             [[job]]\nid = \"e\"\nkind = \"empty\"\npath = '{}'\nafter = [\"d\"]\n\
             [[job]]\nid = \"c\"\nkind = \"copy\"\nfrom = '{}'\ninto = '{}'\nafter = [\"e\"]\n",
            work.display(),
            trash.display(),
            work.display(),
            into.display()
        );
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        for j in ["d", "e", "c"] {
            let job = run.job(j).unwrap();
            assert_eq!(job.state, State::Ok, "{j}: {job:?}\n{}", r.log_tail(&id, j).unwrap_or_default());
        }
        assert!(!work.join("old.log").exists());
        assert!(work.join("new.log").exists() && work.join("old.txt").exists());
        assert!(trash.is_dir() && std::fs::read_dir(&trash).unwrap().next().is_none());
        assert!(into.join("new.log").exists() && into.join("old.txt").exists());
        assert!(r.log_tail(&id, "d").unwrap().contains("1 file(s) deleted"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn send_and_fetch_copy_between_folders() {
        if !cfg!(windows) {
            return;
        }
        let dir = scratch("send");
        let (out, share, back) = (dir.join("out"), dir.join("share"), dir.join("back"));
        std::fs::create_dir_all(&out).unwrap();
        for f in ["a.csv", "b.csv", "c.txt"] {
            std::fs::write(out.join(f), "x").unwrap();
        }
        // An sftp:// side runs `koyomi sftp run` (KOYOMI_EXE), which is the test binary here:
        // chore.rs and sftp.rs are tested on their own.
        let text = format!(
            "format = 1\n[[job]]\nid = \"s\"\nkind = \"send\"\nfrom = '{}\\*.csv'\ninto = '{}'\n\
             [[job]]\nid = \"f\"\nkind = \"fetch\"\nfrom = '{}\\*.csv'\ninto = '{}'\nafter = [\"s\"]\n\
             [[job]]\nid = \"none\"\nkind = \"fetch\"\nfrom = '{}\\*.zip'\ninto = '{}'\nafter = [\"f\"]\n",
            out.display(),
            share.display(),
            share.display(),
            back.display(),
            share.display(),
            back.display()
        );
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        let log = |j: &str| r.log_tail(&id, j).unwrap_or_default();
        for (j, state) in [("s", State::Ok), ("f", State::Ok), ("none", State::Warn)] {
            assert_eq!(run.job(j).unwrap().state, state, "{j}\n{}", log(j));
        }
        assert!(share.join("a.csv").exists() && share.join("b.csv").exists() && !share.join("c.txt").exists());
        assert!(back.join("a.csv").exists() && back.join("b.csv").exists());
        assert!(log("s").contains("sent: "), "{}", log("s"));
        assert!(log("none").contains("no file to fetch"), "{}", log("none"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_reason_left_in_koyomi_reason_is_the_jobs() {
        // What `koyomi ssh run` does when the far command has no exit code.
        let dir = scratch("reason");
        let cmd = if cfg!(windows) {
            r#"run = ["cmd.exe", "/c", "echo no link>%KOYOMI_REASON%& exit 255"]"#
        } else {
            r#"run = ["sh", "-c", "echo no link > \"$KOYOMI_REASON\"; exit 255"]"#
        };
        let d = def("net", &format!("format = 1\n[[job]]\nid = \"a\"\n{cmd}\n"));
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        let a = run.job("a").unwrap();
        assert_eq!(a.state, State::Error, "{}", r.log_tail(&id, "a").unwrap_or_default());
        assert!(a.reason.contains("no link"), "{}", a.reason);
        // Taken away once read.
        fn reasons(dir: &std::path::Path, found: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    reasons(&p, found);
                } else if p.extension().is_some_and(|x| x == "reason") {
                    found.push(p);
                }
            }
        }
        let mut left = Vec::new();
        reasons(&dir, &mut left);
        assert!(left.is_empty(), "{left:?}");
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

    /// Ticks until `job` of `id` is 実行中, or gives up.
    fn wait_running(r: &mut Runner, id: &str, job: &str) {
        let until = Instant::now() + Duration::from_secs(10);
        while Instant::now() < until {
            r.tick();
            if r.run(id).and_then(|r| r.job(job)).is_some_and(|j| j.state == State::Running) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("{id}/{job} did not start");
    }

    #[test]
    fn a_notify_part_and_a_passed_time_end_at_once_and_the_notice_comes_out() {
        let dir = scratch("parts");
        let text = format!(
            "format = 1\n[[job]]\nid = \"a\"\n{}\n[[job]]\nid = \"t\"\nkind = \"time\"\nat = \"00:00\"\nafter = [\"a\"]\n[[job]]\nid = \"n\"\nkind = \"notify\"\nafter = [\"t\"]\n[[job]]\nid = \"b\"\n{}\nafter = [\"n\"]\n",
            echo("one"),
            echo("two")
        );
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Ok);
        let t = run.job("t").unwrap();
        assert_eq!(t.state, State::Ok);
        assert!(t.reason.ends_with("00:00 を過ぎていた"), "{:?}", t.reason);
        assert_eq!(run.job("n").unwrap().reason, "通知した");
        assert_eq!(r.take_notices(), vec![(id.clone(), "n".to_owned())]);
        assert!(r.take_notices().is_empty());
        assert_eq!(r.log_tail(&id, "b").unwrap().trim(), "two");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_part_waits_for_the_file_and_hands_it_on_as_koyomi_file() {
        let dir = scratch("file");
        let inbox = dir.join("in");
        std::fs::create_dir_all(&inbox).unwrap();
        let pattern = inbox.join("*.csv").display().to_string().replace('\\', "/");
        let show = if cfg!(windows) { echo("%KOYOMI_FILE%") } else { "run = [\"sh\", \"-c\", \"echo $KOYOMI_FILE\"]".to_owned() };
        let text = format!(
            "format = 1\n[[job]]\nid = \"f\"\nkind = \"file\"\nfile = \"{pattern}\"\n[[job]]\nid = \"b\"\n{show}\nafter = [\"f\"]\n"
        );
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        wait_running(&mut r, &id, "f");
        let f = r.run(&id).unwrap().job("f").unwrap().clone();
        assert!(f.reason.ends_with("を待っています"), "{:?}", f.reason);
        // Another name does not count; the right one does.
        std::fs::write(inbox.join("note.txt"), "x").unwrap();
        for _ in 0..5 {
            r.tick();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(r.run(&id).unwrap().job("f").unwrap().state, State::Running);
        std::fs::write(inbox.join("Sales.csv"), "1,2").unwrap();
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Ok);
        let f = run.job("f").unwrap();
        assert!(f.reason.ends_with("が来た"), "{:?}", f.reason);
        let got = f.file.clone().unwrap();
        assert!(got.ends_with("Sales.csv"), "{got}");
        assert_eq!(r.log_tail(&id, "b").unwrap().trim(), got);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_loop_runs_once_per_file_or_count_and_stops_at_an_abnormal_pass() {
        let dir = scratch("loop");
        let inbox = dir.join("in");
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::write(inbox.join("b.csv"), "2").unwrap();
        std::fs::write(inbox.join("a.csv"), "1").unwrap();
        std::fs::write(inbox.join("note.txt"), "x").unwrap();
        let pattern = inbox.join("*.csv").display().to_string().replace('\\', "/");
        let show = |var: &str| {
            if cfg!(windows) {
                echo(&format!("%KOYOMI_INDEX% %{var}%"))
            } else {
                format!("run = [\"sh\", \"-c\", \"echo $KOYOMI_INDEX ${var}\"]")
            }
        };
        let stop_at_2 = if cfg!(windows) {
            "run = [\"cmd.exe\", \"/c\", \"if %KOYOMI_INDEX%==2 (exit 3) else (echo %KOYOMI_INDEX%)\"]".to_owned()
        } else {
            "run = [\"sh\", \"-c\", \"[ $KOYOMI_INDEX = 2 ] && exit 3; echo $KOYOMI_INDEX\"]".to_owned()
        };
        let text = format!(
            "format = 1
[[job]]
id = \"files\"
kind = \"loop\"
file = \"{pattern}\"
{}
[[job]]
id = \"times\"
kind = \"loop\"
count = 3
{}
after = [\"files\"]
[[job]]
id = \"none\"
kind = \"loop\"
file = \"{}\"
{}
after = [\"times\"]
[[job]]
id = \"bad\"
kind = \"loop\"
count = 3
{stop_at_2}
after = [\"none\"]
",
            show("KOYOMI_FILE"),
            show("KOYOMI_INDEX"),
            inbox.join("*.none").display().to_string().replace('\\', "/"),
            echo("never"),
        );
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        // Once per matching file, by name, each with its index and path.
        let files = r.log_tail(&id, "files").unwrap();
        let lines: Vec<&str> = files.lines().filter(|l| !l.starts_with('▶')).map(str::trim).collect();
        assert_eq!(lines.len(), 2, "{files}");
        assert!(lines[0].starts_with("1 ") && lines[0].ends_with("a.csv"), "{files}");
        assert!(lines[1].starts_with("2 ") && lines[1].ends_with("b.csv"), "{files}");
        assert!(files.contains("▶ 1/2: ") && files.contains("▶ 2/2: "), "{files}");
        // Count times.
        let times = r.log_tail(&id, "times").unwrap();
        let lines: Vec<&str> = times.lines().filter(|l| !l.starts_with('▶')).map(str::trim).collect();
        assert_eq!(lines, ["1 1", "2 2", "3 3"], "{times}");
        assert_eq!(run.job("times").unwrap().state, State::Ok);
        // No file: nothing runs, and that is fine.
        assert_eq!(run.job("none").unwrap().state, State::Ok);
        assert!(!r.log_tail(&id, "none").unwrap().contains("never"));
        // An abnormal pass ends the loop as 異常 and says which pass.
        let bad = run.job("bad").unwrap();
        assert_eq!(bad.state, State::Error);
        assert!(bad.reason.contains("2/3 回目で異常: 終了コード 3"), "{:?}", bad.reason);
        assert!(!r.log_tail(&id, "bad").unwrap().contains("▶ 3/3"));
        assert_eq!(run.state, State::Error);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A command that takes about `secs` seconds, then prints `KOYOMI_INDEX`.
    fn slow_index(secs: u32) -> String {
        if cfg!(windows) {
            format!("run = [\"cmd.exe\", \"/c\", \"ping -n {} 127.0.0.1 > nul & echo %KOYOMI_INDEX%\"]", secs + 1)
        } else {
            format!("run = [\"sh\", \"-c\", \"sleep {secs}; echo $KOYOMI_INDEX\"]")
        }
    }

    #[test]
    fn a_loop_with_max_runs_its_passes_side_by_side_in_that_many_slots() {
        let dir = scratch("loopmax");
        let text = format!(
            "format = 1\n[[job]]\nid = \"many\"\nkind = \"loop\"\ncount = 4\nmax = 2\n{}\n[[job]]\nid = \"other\"\n{}\n",
            slow_index(2),
            echo("x")
        );
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config { concurrency: 2, ..Config::default() });
        let began = Instant::now();
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        // The loop holds both slots, so the other job waits for one.
        let run = r.run(&id).unwrap();
        assert_eq!(r.busy(), 1);
        assert_eq!(run.job("other").unwrap().state, State::Waiting);
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Ok);
        // Two at a time: about 4 s, where one after another takes 8 s.
        assert!(began.elapsed() < Duration::from_secs(7), "{:?}", began.elapsed());
        let log = r.log_tail(&id, "many").unwrap();
        assert!(log.contains("4 回を 2 つずつ並べて動かす"), "{log}");
        let mut lines: Vec<&str> = log.lines().filter(|l| !l.starts_with('▶')).map(str::trim).collect();
        lines.sort_unstable();
        assert_eq!(lines, ["1", "2", "3", "4"], "{log}");
        for i in 1..=4 {
            assert!(log.contains(&format!("▶ {i}/4")), "{log}");
        }
        // A loop takes no more slots than are free, nor than it has passes.
        let free = Part::Loop { file: None, count: Some(3), max: Some(8) };
        assert_eq!((width(&free, 5), width(&free, 2), width(&free, 0)), (3, 2, 1));
        assert_eq!(width(&Part::Loop { file: Some("a".into()), count: None, max: Some(4) }, 9), 4);
        assert_eq!(width(&Part::Loop { file: Some("a".into()), count: None, max: None }, 9), 1);
        assert_eq!(width(&Part::Job, 9), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_loop_side_by_side_starts_no_more_after_an_abnormal_pass_and_a_kill_reaches_every_pass() {
        let dir = scratch("loopmaxbad");
        let fail_first = if cfg!(windows) {
            "run = [\"cmd.exe\", \"/c\", \"if %KOYOMI_INDEX%==1 (exit 3) else (ping -n 3 127.0.0.1 > nul)\"]".to_owned()
        } else {
            "run = [\"sh\", \"-c\", \"[ $KOYOMI_INDEX = 1 ] && exit 3; sleep 2\"]".to_owned()
        };
        let d = def("net", &format!("format = 1\n[[job]]\nid = \"bad\"\nkind = \"loop\"\ncount = 5\nmax = 2\n{fail_first}\n"));
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        let bad = run.job("bad").unwrap();
        assert_eq!(bad.state, State::Error);
        assert!(bad.reason.contains("1/5 回目で異常: 終了コード 3"), "{:?}", bad.reason);
        // The pass running beside it ends as it would; no more start.
        let log = r.log_tail(&id, "bad").unwrap();
        assert!(log.contains("▶ 2/5"), "{log}");
        assert!(!log.contains("▶ 3/5"), "{log}");
        // A kill stops both running passes at once.
        let d = def("long", &format!("format = 1\n[[job]]\nid = \"a\"\nkind = \"loop\"\ncount = 2\nmax = 2\n{}\n", sleep(30)));
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let asked = Instant::now();
        r.kill(&id, None).unwrap();
        let run = wait_over(&mut r, &id);
        assert!(asked.elapsed() < Duration::from_secs(10), "the passes were not killed at once");
        assert_eq!(run.job("a").unwrap().state, State::Killed);
        let log = r.log_tail(&id, "a").unwrap();
        assert_eq!(log.matches("! 強制終了").count(), 2, "{log}");
        assert_eq!(r.busy(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_started_by_a_file_hands_it_to_its_jobs_as_koyomi_file() {
        let dir = scratch("event");
        let show = if cfg!(windows) { echo("%KOYOMI_FILE%") } else { "run = [\"sh\", \"-c\", \"echo $KOYOMI_FILE\"]".to_owned() };
        let d = def("net", &format!("format = 1\n[[when]]\nfile = 'D:\\\\in\\\\*.csv'\n[[job]]\nid = \"b\"\n{show}\n"));
        let mut r = Runner::new(&dir, Config::default());
        let today = Local::now().date_naive();
        let id = r.start_event(&d, today, How::File, "D:\\in\\Sales.csv が来た", Some("D:\\in\\Sales.csv".to_owned())).unwrap();
        let run = wait_over(&mut r, &id);
        assert_eq!((run.how, run.state, run.reason.as_str()), (How::File, State::Ok, "D:\\in\\Sales.csv が来た"));
        assert_eq!(run.planned, None);
        assert_eq!(r.log_tail(&id, "b").unwrap().trim(), "D:\\in\\Sales.csv");
        // While it runs, the same net is refused, as by hand.
        let id2 = r.start_event(&d, today, How::Start, "起動したとき", None).unwrap();
        assert_eq!(wait_over(&mut r, &id2).how, How::Start);
        // After another net: the run takes that run's business day, and the jobs see it.
        let show = if cfg!(windows) { echo("%KOYOMI_DATE%") } else { "run = [\"sh\", \"-c\", \"echo $KOYOMI_DATE\"]".to_owned() };
        let d2 = def("after", &format!("format = 1\n[[when]]\nafter_jobnet = \"g/net\"\n[[job]]\nid = \"b\"\n{show}\n"));
        let date = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        let id3 = r.start_event(&d2, date, How::After, "g/net が終わった（正常終了）", None).unwrap();
        let run = wait_over(&mut r, &id3);
        assert_eq!((run.how, run.date.as_str(), run.state), (How::After, "2026-10-28", State::Ok));
        assert_eq!(r.log_tail(&id3, "b").unwrap().trim(), "2026-10-28");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_approval_part_waits_for_a_person() {
        let dir = scratch("approval");
        let d = def(
            "net",
            &format!(
                "format = 1
[[job]]
id = \"ok\"
kind = \"approval\"
[[job]]
id = \"b\"
after = [\"ok\"]
{}
",
                echo("x")
            ),
        );
        let mut r = Runner::new(&dir, Config::default());
        let day = Local::now().date_naive();
        let id = r.start(&d, day, How::Manual, None).unwrap();
        r.tick();
        assert_eq!(r.take_asks(), vec![(id.clone(), "ok".to_owned())]);
        assert!(r.answer(&id, "nope", true).is_err());
        r.answer(&id, "ok", true).unwrap();
        assert_eq!(wait_over(&mut r, &id).state, State::Ok);
        // A rejection ends the run 異常 and the job after it does not run.
        let id2 = r.start(&d, day, How::Manual, None).unwrap();
        r.tick();
        r.answer(&id2, "ok", false).unwrap();
        let run = wait_over(&mut r, &id2);
        assert_eq!(run.state, State::Error);
        assert_eq!(run.job("b").unwrap().state, State::NotRun);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nets_with_the_same_lock_do_not_run_together() {
        let dir = scratch("lock");
        let net = |body: &str| {
            format!(
                "format = 1
lock = \"db\"
[[job]]
id = \"a\"
{body}
"
            )
        };
        let a = def("a", &net(&sleep(2)));
        let b = def("b", &net(&echo("x")));
        let c = def(
            "c",
            &format!(
                "format = 1
[[job]]
id = \"a\"
{}
",
                echo("x")
            ),
        );
        let mut r = Runner::new(&dir, Config::default());
        let day = Local::now().date_naive();
        let ida = r.start(&a, day, How::Manual, None).unwrap();
        let err = r.start(&b, day, How::Manual, None).unwrap_err();
        assert!(err.contains("排他") && err.contains("db"), "{err}");
        // A net without the lock is not held back.
        let idc = r.start(&c, day, How::Manual, None).unwrap();
        wait_over(&mut r, &idc);
        wait_over(&mut r, &ida);
        // Once the holder is over the lock is free.
        let idb = r.start(&b, day, How::Manual, None).unwrap();
        assert_eq!(wait_over(&mut r, &idb).state, State::Ok);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shared_holders_go_together_and_a_counted_lock_lets_that_many_in() {
        let dir = scratch("lock-shared");
        let net = |id: &str, lock: &str| {
            def(
                id,
                &format!(
                    "format = 1
lock = {lock}
[[job]]
id = \"a\"
{}
",
                    sleep(2)
                ),
            )
        };
        let shared = "{ name = \"db\", shared = true }";
        let (r1, r2, w) = (net("r1", shared), net("r2", shared), net("w", "\"db\""));
        let mut r = Runner::new(&dir, Config::default());
        let day = Local::now().date_naive();
        let id1 = r.start(&r1, day, How::Manual, None).unwrap();
        let id2 = r.start(&r2, day, How::Manual, None).unwrap();
        assert!(r.runs.iter().find(|x| x.id == id2).is_some_and(|x| x.lock_shared));
        let err = r.start(&w, day, How::Manual, None).unwrap_err();
        assert_eq!(err, "w: 排他「db」は r1、r2 が共有で使っている");
        wait_over(&mut r, &id1);
        wait_over(&mut r, &id2);
        // `[locks] api = 2`: two exclusive holders go, the third is kept out.
        r.set_config(Config::parse("[locks]\napi = 2\n").unwrap());
        let (a, b, c) = (net("a", "\"api\""), net("b", "\"api\""), net("c", "\"api\""));
        let ida = r.start(&a, day, How::Manual, None).unwrap();
        let idb = r.start(&b, day, How::Manual, None).unwrap();
        let err = r.start(&c, day, How::Manual, None).unwrap_err();
        assert_eq!(err, "c: 排他「api」は 2 本まで。a、b が使っている");
        wait_over(&mut r, &ida);
        wait_over(&mut r, &idb);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_job_hands_values_to_the_jobs_after_it_through_koyomi_out() {
        let dir = scratch("handoff");
        let (put, show) = if cfg!(windows) {
            (
                "run = [\"cmd.exe\", \"/c\", \"echo rows=5 >%KOYOMI_OUT%\"]".to_owned(),
                "run = [\"cmd.exe\", \"/c\", \"type %KOYOMI_IN%\"]".to_owned(),
            )
        } else {
            ("run = [\"sh\", \"-c\", \"echo rows=5 > $KOYOMI_OUT\"]".to_owned(), "run = [\"sh\", \"-c\", \"cat $KOYOMI_IN\"]".to_owned())
        };
        let d = def(
            "net",
            &format!(
                "format = 1
[[job]]
id = \"a\"
{put}
[[job]]
id = \"b\"
after = [\"a\"]
{show}
"
            ),
        );
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Ok);
        let seen = r.log_tail(&id, "b").unwrap();
        assert!(seen.contains("[a]") && seen.contains("rows = 5"), "{seen}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_secrets_are_read_and_hidden() {
        let mut env: BTreeMap<String, String> = BTreeMap::new();
        env.insert("A".into(), "plain".into());
        env.insert("B".into(), "secret:db".into());
        let store = |n: &str| Ok((n == "db").then(|| "hunter2".to_owned()));
        let got = resolve_env(&env, &store).unwrap();
        assert_eq!(got.vars, [("A".to_owned(), "plain".to_owned()), ("B".to_owned(), "hunter2".to_owned())]);
        let hidden: Vec<&str> = got.secrets.iter().map(String::as_str).collect();
        assert_eq!(ito_secret::mask("pw=hunter2!", &hidden), "pw=***!");
        env.insert("C".into(), "secret:no".into());
        assert!(resolve_env(&env, &store).err().unwrap().contains("秘密がない: no"));
    }

    #[cfg(windows)]
    #[test]
    fn a_bare_program_is_found_on_the_path_for_another_account() {
        assert!(full_program("cmd").to_ascii_lowercase().ends_with("\\system32\\cmd.exe"), "{}", full_program("cmd"));
        assert_eq!(full_program("C:/bin/x.exe"), "C:/bin/x.exe");
        assert_eq!(full_program("no-such-program-koyomi"), "no-such-program-koyomi");
    }

    #[test]
    fn the_run_as_password_is_read_from_the_store_and_hidden() {
        let store = |n: &str| Ok((n == "batch").then(|| "s3cret".to_owned()));
        let account = RunAs { user: r"CORP\batch".into(), secret: "batch".into() };
        let got = resolve_logon(Resolved::default(), Some(&account), &store).unwrap();
        let (who, password) = got.logon.as_ref().unwrap();
        assert_eq!((who, password.as_str()), (&account, "s3cret"));
        let hidden: Vec<&str> = got.secrets.iter().map(String::as_str).collect();
        assert_eq!(ito_secret::mask("pass s3cret", &hidden), "pass ***");
        assert!(got.vars.is_empty(), "the password is not handed to the job");
        let missing = RunAs { user: "batch".into(), secret: "none".into() };
        let e = resolve_logon(Resolved::default(), Some(&missing), &store).err().unwrap();
        assert!(e.contains("koyomi secret set none"), "{e}");
        assert!(resolve_logon(Resolved::default(), None, &store).unwrap().logon.is_none());
    }

    #[test]
    fn date_tokens_in_a_job_are_written_out_for_the_business_day() {
        let dir = scratch("tokens");
        let d = def(
            "net",
            &format!(
                "format = 1
calendar = \"会社\"
[[job]]
id = \"b\"
{}
",
                echo("m{month-1}_{date-1bd:%m%d}")
            ),
        );
        let mut r = Runner::new(&dir, Config::default());
        let cal = koyomi_core::calendar::Calendar { weekly_off: vec![chrono::Weekday::Sat, chrono::Weekday::Sun], ..Default::default() };
        r.set_calendars(BTreeMap::from([("会社".to_owned(), cal)]), BTreeMap::from([("net".to_owned(), "会社".to_owned())]));
        let seen = r.dry(&d, NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(), &BTreeMap::new()).unwrap();
        assert!(seen.starts_with("b: ") && seen.contains("m202610_1030"), "{seen}");
        assert!(r.runs().is_empty());
        let seen = r.dry(&d, NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(), &BTreeMap::new()).unwrap();
        assert!(seen.starts_with("b: ") && seen.contains("m202610_1030"), "{seen}");
        assert!(r.runs().is_empty());
        let id = r.start(&d, NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(), How::Manual, None).unwrap();
        wait_over(&mut r, &id);
        assert_eq!(r.log_tail(&id, "b").unwrap().trim(), "m202610_1030");
        // `bd` without a calendar is refused before anything runs.
        let d = def(
            "bare",
            &format!(
                "format = 1
[[job]]
id = \"b\"
{}
",
                echo("{date-1bd}")
            ),
        );
        let err = r.start(&d, NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(), How::Manual, None).unwrap_err();
        assert!(err.contains("calendar が要る"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn params_given_by_hand_fill_the_job_and_a_rerun_keeps_them() {
        let dir = scratch("params");
        let d = def(
            "net",
            &format!(
                "format = 1
[params]
month = \"{{month-1}}\"
dept = \"all\"
[[job]]
id = \"b\"
{}
",
                echo("{param:dept}_{param:month}")
            ),
        );
        let mut r = Runner::new(&dir, Config::default());
        let day = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let seen = r.dry(&d, day, &BTreeMap::new()).unwrap();
        assert!(seen.contains("all_202610"), "the defaults, date tokens and all: {seen}");
        let bad = r.dry(&d, day, &BTreeMap::from([("nope".to_owned(), "x".to_owned())])).unwrap_err();
        assert!(bad.contains("nope"), "{bad}");
        let set = BTreeMap::from([("dept".to_owned(), "acct".to_owned())]);
        let id = r.start_manual(&d, day, None, &set).unwrap();
        let run = wait_over(&mut r, &id);
        assert_eq!(run.params, set);
        assert_eq!(r.log_tail(&id, "b").unwrap().trim(), "acct_202610");
        let again = r.rerun(&id, Some("b"), None).unwrap();
        wait_over(&mut r, &again);
        assert_eq!(r.log_tail(&again, "b").unwrap().trim(), "acct_202610");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_waiting_part_is_killed_by_hand_and_the_run_ends_abnormally() {
        let dir = scratch("killwait");
        let text = format!(
            "format = 1\n[[job]]\nid = \"t\"\nkind = \"time\"\nat = \"00:00\"\ndays = 1\n[[job]]\nid = \"b\"\n{}\nafter = [\"t\"]\n",
            echo("two")
        );
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        wait_running(&mut r, &id, "t");
        assert!(r.kill(&id, Some("b")).unwrap_err().contains("動いていない"));
        assert!(r.kill(&id, Some("t")).unwrap().contains("止めた"));
        let run = r.run(&id).unwrap().clone();
        assert!(run.is_over());
        assert_eq!(run.job("t").unwrap().state, State::Killed);
        assert!(run.state.is_error());
        assert!(r.take_over().iter().any(|o| o.id == id));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_child_jobnet_runs_its_jobs_in_place_and_an_unconfirmed_one_is_refused() {
        let dir = scratch("child");
        let child = koyomi_core::jobnet::parse(&format!("format = 1\nname = \"子\"\n[[job]]\nid = \"x\"\n{}\n", echo("in-child"))).unwrap();
        let text = format!(
            "format = 1\n[[job]]\nid = \"a\"\n{}\n[[job]]\nid = \"c\"\nkind = \"jobnet\"\nuse = \"g/child\"\nafter = [\"a\"]\n[[job]]\nid = \"b\"\n{}\nafter = [\"c\"]\n",
            echo("one"),
            echo("two")
        );
        let d = def("net", &text);
        let mut r = Runner::new(&dir, Config::default());
        // Not known yet: the start is refused, not run half.
        assert!(r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap_err().contains("子ジョブネットがない"));
        r.set_children(BTreeMap::from([("g/child".to_owned(), Err("子ジョブネット g/child が未確認".to_owned()))]));
        assert!(r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap_err().contains("未確認"));
        r.set_children(BTreeMap::from([("g/child".to_owned(), Ok(child))]));
        let id = r.start(&d, Local::now().date_naive(), How::Manual, None).unwrap();
        let run = wait_over(&mut r, &id);
        assert_eq!(run.state, State::Ok);
        let ids: Vec<&str> = run.jobs.iter().map(|j| j.id.as_str()).collect();
        assert_eq!(ids, ["a", "c/x", "c", "b"]);
        assert_eq!(r.log_tail(&id, "c/x").unwrap().trim(), "in-child");
        let (a, x, b) = (run.job("a").unwrap(), run.job("c/x").unwrap(), run.job("b").unwrap());
        assert!(a.ended.unwrap() <= x.started.unwrap() && x.ended.unwrap() <= b.started.unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_condition_of_the_pc_holds_a_job_until_it_is_met() {
        use koyomi_core::jobnet::Require;
        let req = Require { ac: true, network: true, idle: Some(600) };
        let idle = Some(Duration::from_secs(900));
        assert!(unmet_by(&req, Some(false), Some(true), idle).unwrap().contains("AC"));
        assert!(unmet_by(&req, Some(true), Some(false), idle).unwrap().contains("ネットワーク"));
        assert!(unmet_by(&req, Some(true), Some(true), Some(Duration::from_secs(30))).unwrap().contains("10 分"));
        assert_eq!(unmet_by(&req, Some(true), Some(true), idle), None);
        assert_eq!(unmet_by(&req, None, None, None), None, "unknown is not a reason to hold");
        assert_eq!(unmet_by(&Require::default(), Some(false), Some(false), Some(Duration::ZERO)), None);
    }
}
