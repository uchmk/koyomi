//! `koyomi engine`: the one process that owns the definitions.
//!
//! It reads `jobnets/<group>/<name>.toml`, keeps `approved.toml`, watches the
//! folder, and answers the window and the CLI through the door (`proto`). It
//! outlives the window. Nothing here draws or asks a person anything.
//!
//! It is also the one place a run is told of (`notify`): the window and the
//! console only start runs, so a run is told of the same way however it started.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use ito_ipc::door::{ask, Ask};
use ito_prefs::NotifyCfg;
use koyomi_core::approved::{self, Store};
use koyomi_core::blackout::Blackout;
use koyomi_core::bundle::{self, Change};
use koyomi_core::calendar::Calendars;
use koyomi_core::config::Config;
use koyomi_core::group;
use koyomi_core::held::Held;
use koyomi_core::jobnet::Jobnet;
use koyomi_core::late::{self, Delay};
use koyomi_core::run::{How, Run, State as RunState};
use koyomi_core::scan::{def_of_text, link, scan_in, Def, Standing};
use koyomi_core::standby::Resume;
use koyomi_core::when::{self, Kind, OnClosed};

use crate::notify;
use crate::proto::{self, Request, Runs, Snapshot, Versions};

mod alive;
mod backup;
mod events;
pub(crate) mod files;
pub mod http;
mod idle;
mod logs;
mod mails;
mod ping;
mod primary;
mod retain;
mod runner;
mod schedule;
mod standby;
mod tray;

use backup::Backup;
use retain::Retain;
use runner::Runner;
use schedule::Schedule;
pub use tray::apart;

/// Where everything of the engine's lives.
#[derive(Clone, Debug)]
pub struct Paths {
    /// `app_dir("koyomi")`: the settings folder. Holds `jobnets/`, `calendars/` and `koyomi.toml`.
    pub app: PathBuf,
    pub jobnets: PathBuf,
    /// `app_dir("koyomi")/calendars`: `<name>.toml`, what `calendar = "<name>"` names.
    pub calendars: PathBuf,
    /// `app_dir("koyomi")/koyomi.toml`.
    pub config: PathBuf,
    /// `state_dir("koyomi")`: `approved.toml`, `history/`, `logs/`.
    pub state: PathBuf,
    /// `state_dir("koyomi")/approved.toml`.
    pub approved: PathBuf,
    /// `state_dir("koyomi")/held.toml`: what a person stopped by hand.
    pub held: PathBuf,
    /// `state_dir("koyomi")/next.toml`: the definitions staged to take over on a day (TODO 110).
    pub next: PathBuf,
}

impl Paths {
    pub fn find() -> Result<Paths, String> {
        let app = ito_common::app_dir("koyomi").ok_or("設定のフォルダーが分からない")?;
        let state = ito_common::state_dir("koyomi").ok_or("状態のフォルダーが分からない")?;
        Ok(Paths {
            jobnets: app.join("jobnets"),
            calendars: app.join("calendars"),
            config: app.join("koyomi.toml"),
            approved: state.join("approved.toml"),
            held: state.join("held.toml"),
            next: state.join("next.toml"),
            app,
            state,
        })
    }

    /// `koyomi.toml`, or why it does not read.
    pub fn config(&self) -> Result<Config, String> {
        Config::load(&self.config).map_err(|e| format!("{} が読めない（{e}）。直すまで流さない", shown(&self.config)))
    }

    /// The approvals, or the line that says `approved.toml` does not read.
    pub fn store(&self) -> Result<Store, String> {
        Store::load(&self.approved).map_err(|e| format!("{} が読めない（{e}）。直すか退けるまで承認しない", shown(&self.approved)))
    }

    /// What is on hold, or the line that says `held.toml` does not read.
    pub fn held(&self) -> Result<Held, String> {
        Held::load(&self.held).map_err(|e| format!("{} が読めない（{e}）。直すか退けるまで流さない", shown(&self.held)))
    }

    /// The staged definitions, or the line that says `next.toml` does not read.
    pub fn staged(&self) -> Result<koyomi_core::next::Store, String> {
        koyomi_core::next::Store::load(&self.next)
            .map_err(|e| format!("{} が読めない（{e}）。直すか退けるまで切り替えない", shown(&self.next)))
    }
}

/// Who other than this user can write what koyomi runs from, one line each. Empty: nothing in the way.
///
/// The definitions, the folders above them and `approved.toml` all count: whoever can change one
/// of them can make koyomi run a command nobody looked at.
pub fn access_warnings(paths: &Paths, files: &[PathBuf]) -> Vec<String> {
    let mut seen: Vec<(PathBuf, String)> = Vec::new();
    let mut check = |p: &Path| {
        if !p.exists() {
            return;
        }
        for f in ito_common::access::findings(p).unwrap_or_default() {
            let key = (f.path.clone(), f.who.clone());
            if !seen.contains(&key) {
                seen.push(key);
            }
        }
    };
    check(&paths.jobnets);
    check(&paths.app);
    check(&paths.approved);
    check(&paths.next);
    files.iter().for_each(|f| check(f));
    seen.into_iter().map(|(p, who)| format!("{} は {} が書き換えられる", shown(&p), who)).collect()
}

/// The warnings in `warnings` that bear on the net in `file`: those about the shared folders and
/// files, and those about this net's own file, but not those about another net's file.
pub fn warnings_about(paths: &Paths, warnings: &[String], defs: &[Def], file: &str) -> Vec<String> {
    let at = |f: &str| shown(&f.split('/').fold(paths.jobnets.clone(), |p, c| p.join(c)));
    let own = at(file);
    let others: Vec<String> = defs.iter().filter(|d| d.file != file).map(|d| at(&d.file)).collect();
    warnings.iter().filter(|w| w.starts_with(&own) || !others.iter().any(|o| w.starts_with(o.as_str()))).cloned().collect()
}

/// The path without Windows' `\\?\` prefix.
fn shown(p: &Path) -> String {
    let s = p.display().to_string();
    s.strip_prefix(r"\\?\").map(str::to_owned).unwrap_or(s)
}

/// A time of a net that is due but waits for the end of another net's run on
/// the same business day (`wait_for`), until `until` (`wait_until`).
struct Waiting {
    def: Def,
    planned: NaiveDateTime,
    for_net: String,
    until: Option<NaiveDateTime>,
}

/// A time of a net that came in its `quiet` hours and starts when they are
/// over (`until`); `late` when it was already late (`missed = "once"`).
struct Quieted {
    def: Def,
    planned: NaiveDateTime,
    until: NaiveDateTime,
    late: bool,
}

struct State {
    paths: Paths,
    store: Store,
    /// Why `approved.toml` did not read: until it does, nothing is approved and nothing written over it.
    unreadable: Option<String>,
    /// The nets on hold and whether the schedule is paused (`held.toml`).
    held: Held,
    /// Why `held.toml` did not read: until it does, nothing starts and nothing is written over it.
    held_bad: Option<String>,
    /// The definitions staged to take over on a day (`next.toml`).
    staged: koyomi_core::next::Store,
    /// Why `next.toml` did not read: until it does, nothing is switched and nothing written over it.
    staged_bad: Option<String>,
    /// The day the staged definitions were last looked at, and when: a failed switch is tried
    /// again every [`NEXT_RETRY`].
    staged_look: Option<(NaiveDate, Instant)>,
    /// Why a staged definition that is due did not take over, by net: told once, shown until it does.
    staged_failed: BTreeMap<String, String>,
    defs: Vec<Def>,
    /// `calendars/*.toml` as last read; the nets count their days by these.
    calendars: Calendars,
    warnings: Vec<String>,
    /// Why `koyomi.toml` does not read: until it does, nothing is started.
    bad_config: Option<String>,
    /// `[[blackout]]`: the planned stops; no schedule or event starts a net in them.
    blackouts: Vec<Blackout>,
    runner: Runner,
    /// The clock: when `[[when]]` was last looked at.
    schedule: Schedule,
    /// The rest of `[[when]]`: files arriving and the engine's start.
    events: events::Events,
    /// `[[when]] log` / `eventlog`: lines written to a log.
    logs: logs::Logs,
    /// `[[when]] mail`: mail coming to a mailbox.
    mails: mails::Mails,
    /// `[[when]] idle`: the PC left alone.
    idle: idle::Idle,
    /// The sweep of logs and history older than `keep_days`.
    retain: Retain,
    /// The daily backup of the settings folder.
    backup: Backup,
    alive: alive::Alive,
    /// `[ping]`: the outside watcher's address, called while the engine runs.
    ping: ping::Ping,
    ping_cfg: Option<koyomi_core::config::Ping>,
    /// `[notify] hide_webhooks` is off: a plain-text `[ping]` address may be called.
    ping_plain_ok: bool,
    /// `[http]`: the loopback listener that starts nets and reads runs for other programs.
    http: http::Http,
    http_cfg: Option<koyomi_core::config::Http>,
    /// `[standby]`: while the PC it watches answers, no schedule or event runs here.
    standby: standby::Standby,
    /// `[primary]`: asks the standby it names before the schedules run here, and waits while
    /// that one has them (TODO 96).
    primary: primary::Primary,
    /// Keeps the PC from sleeping while a job's process runs.
    awake: Option<ito_power::Awake>,
    /// The wake timer for the soonest time of a `wake = true` net; `None`: no net asks.
    wake: Option<Wake>,
    /// When the power settings were last read (once a minute).
    power_look: Option<Instant>,
    /// How long the PC idles before it sleeps, as last read.
    sleep_after: Option<Duration>,
    /// The times held back by `wait_for`.
    waiting: Vec<Waiting>,
    /// Times that wait for their net's `quiet` hours to be over (in memory, like `waiting`).
    quieted: Vec<Quieted>,
    /// The line about the time the engine was not running, and when it was first told.
    outage: Option<(String, Instant)>,
    /// `keep_days`, `archive` and `archive_days`: what the sweep is for.
    keep: retain::Settings,
    /// `[notify]` as runs are told with: the `secret:` addresses read back
    /// (on a thread; until it answers they are not sent to).
    notify: NotifyCfg,
    /// `[notify]` as it last read, to see a change; `None` before the first read.
    notify_raw: Option<NotifyCfg>,
    /// What a person should see about `[notify]`: from the reading, then from the credential store.
    notify_notes: Vec<String>,
    reveal_notes: Vec<String>,
    /// Which reading the thread is reading back for; an older answer is dropped.
    reveal_gen: u64,
    reveal_tx: mpsc::Sender<(u64, NotifyCfg, Vec<String>)>,
    reveal_rx: mpsc::Receiver<(u64, NotifyCfg, Vec<String>)>,
    /// The words a run is told in: common.toml's language.
    words: ito_i18n::Table,
    /// `acked.toml` as last read by the tray look, and when.
    acked: std::sync::Mutex<(Option<std::time::Instant>, std::collections::BTreeSet<String>)>,
    /// When the runs were last looked at for being late (every [`LATE_EVERY`]).
    late_look: Option<Instant>,
    /// How long each net usually takes, read from the history when one of its
    /// runs goes; dropped when a run of it ends.
    usual: HashMap<String, Option<chrono::Duration>>,
    /// The times (net, planned) told late in starting before they had a run:
    /// when the run comes, it is marked and not told again.
    late_told: HashSet<(String, NaiveDateTime)>,
}

/// How often the runs are looked at for being late.
const LATE_EVERY: Duration = Duration::from_secs(5);
/// How often a staged definition that could not take over is tried again.
const NEXT_RETRY: Duration = Duration::from_secs(60);
/// How many days of history say how long a net usually takes.
const USUAL_DAYS: u64 = 30;

impl State {
    fn new(paths: Paths) -> State {
        let (config, bad_config) = match paths.config() {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(e)),
        };
        let keep = retain::Settings::of(&config);
        let blackouts = config.blackout.clone();
        let mut runner = Runner::new(&paths.state, config);
        runner.set_jobnets(paths.jobnets.clone());
        let schedule = Schedule::new(&paths.state);
        let retain = Retain::new(&paths.state);
        let backup = Backup::new(&paths.app, &paths.state);
        let alive = alive::Alive::new(&paths.state);
        let standby = standby::Standby::new(&paths.state);
        let (reveal_tx, reveal_rx) = mpsc::channel();
        let mut s = State {
            paths,
            store: Store::default(),
            unreadable: None,
            held: Held::default(),
            held_bad: None,
            staged: Default::default(),
            staged_bad: None,
            staged_look: None,
            staged_failed: BTreeMap::new(),
            defs: Vec::new(),
            calendars: Calendars::default(),
            warnings: Vec::new(),
            bad_config,
            blackouts,
            runner,
            schedule,
            events: events::Events::new(),
            logs: logs::Logs::new(),
            mails: mails::Mails::new(),
            idle: idle::Idle::new(),
            retain,
            backup,
            alive,
            ping: ping::Ping::default(),
            ping_cfg: None,
            ping_plain_ok: false,
            http: http::Http::default(),
            http_cfg: None,
            standby,
            primary: primary::Primary::new(),
            awake: None,
            wake: None,
            power_look: None,
            sleep_after: None,
            waiting: Vec::new(),
            quieted: Vec::new(),
            outage: None,
            keep,
            notify: NotifyCfg::default(),
            notify_raw: None,
            notify_notes: Vec::new(),
            reveal_notes: Vec::new(),
            reveal_gen: 0,
            reveal_tx,
            reveal_rx,
            words: crate::cli::table(),
            acked: std::sync::Mutex::new((None, std::collections::BTreeSet::new())),
            late_look: None,
            usual: HashMap::new(),
            late_told: HashSet::new(),
        };
        s.reload();
        s.rescan();
        if let Some((line, crashed)) = s.alive.outage.take() {
            let title = if crashed { "koyomi: エンジンが落ちていた" } else { "koyomi: エンジンが止まっていた" };
            crate::notify::tell(&s.notify, None, "failed", title, &line);
            crate::say(&format!("koyomi: {line}"), false);
            s.outage = Some((line, Instant::now()));
        }
        s
    }

    /// Reads `koyomi.toml` again. A changed `[notify]` is used at once, and its
    /// `secret:` addresses are read back on a thread (the store can be slow).
    fn reload_config(&mut self) {
        match self.paths.config() {
            Ok(c) => {
                let (n, notes) = notify::read(&c);
                self.ping_plain_ok = !n.hide_webhooks();
                if self.notify_raw.as_ref() != Some(&n) {
                    self.notify_raw = Some(n.clone());
                    self.notify_notes = notes;
                    self.reveal_notes.clear();
                    self.notify = n.clone();
                    self.reveal_gen += 1;
                    let (gen, tx) = (self.reveal_gen, self.reveal_tx.clone());
                    std::thread::spawn(move || {
                        let mut n = n;
                        let notes = n.reveal("koyomi");
                        let _ = tx.send((gen, n, notes));
                    });
                }
                self.keep = retain::Settings::of(&c);
                self.ping_cfg = c.ping.clone();
                self.http_cfg = c.http.clone();
                self.standby.set(c.standby.as_ref(), &c.remote);
                self.primary.set(c.primary.as_ref(), &c.remote);
                self.blackouts = c.blackout.clone();
                self.runner.set_config(c);
                self.bad_config = None;
            }
            Err(e) => self.bad_config = Some(e),
        }
    }

    /// Takes in the addresses read back, and tells of the runs that ended.
    fn pump(&mut self) {
        while let Ok((gen, n, notes)) = self.reveal_rx.try_recv() {
            if gen == self.reveal_gen {
                self.notify = n;
                self.reveal_notes = notes;
            }
        }
        // A 通知 part tells of the run as it stands.
        for (run, job) in self.runner.take_notices() {
            let Some(r) = self.runner.runs().iter().find(|r| r.id == run) else { continue };
            let (event, title, body) = notify::part_words(&self.words, r, &job);
            let part = r.job(&job).map(|j| j.to.as_slice()).filter(|to| !to.is_empty());
            let places = notify::to(&self.notify, part.unwrap_or_else(|| self.notify_to(&r.jobnet)));
            notify::tell(&places, notify::open(&self.words, &r.id, notify::Kind::Part).as_ref(), event, &title, &body);
        }
        // A 確認待ち part asks a person.
        for (run, job) in self.runner.take_asks() {
            let Some(r) = self.runner.runs().iter().find(|r| r.id == run) else { continue };
            let (event, title, body) = notify::ask_words(&self.words, r, &job);
            let open = notify::open(&self.words, &r.id, notify::Kind::Ask);
            notify::tell(&notify::to(&self.notify, self.notify_to(&r.jobnet)), open.as_ref(), event, &title, &body);
        }
        let over = self.runner.take_over();
        for r in &over {
            self.usual.remove(&r.jobnet);
        }
        self.tick_after(&over);
        let (runs, missed) = notify::group_missed(over);
        // A time stopped by hand (⏸) is not told of: the person did it.
        for r in runs.into_iter().filter(|r| r.state != RunState::Held) {
            let (event, title, body) = notify::words(&self.words, &r);
            let tail = notify::tail(&self.words, &r, |job| self.runner.log_tail(&r.id, job).ok());
            let places = notify::to(&self.notify, self.notify_to(&r.jobnet));
            notify::tell_tail(&places, notify::open(&self.words, &r.id, notify::Kind::End).as_ref(), event, &title, &body, &tail);
            // A run that failed because a job was cut off or killed says so (2001 / 2002), not the plain 2000.
            let said = if r.state.is_error() {
                [RunState::Timeout, RunState::Killed].into_iter().find(|s| r.jobs.iter().any(|j| j.state == *s)).unwrap_or(r.state)
            } else {
                r.state
            };
            let (level, id) = crate::eventlog::of(said);
            crate::eventlog::write(level, id, &title, &body);
            // An abnormal end told earlier is told to be over.
            if let Some(prev) = notify::recovered_from(self.runner.runs(), &r) {
                let (event, title, body) = notify::recovery_words(&self.words, &r, prev);
                notify::tell(&places, notify::open(&self.words, &r.id, notify::Kind::Recovered).as_ref(), event, &title, &body);
                crate::eventlog::write(ito_eventlog::Level::Info, crate::eventlog::RECOVERED, &title, &body);
            }
        }
        for runs in missed {
            let (event, title, body) = notify::missed_words(&self.words, &runs);
            let (level, id) = crate::eventlog::of(RunState::Missed);
            crate::eventlog::write(level, id, &title, &body);
            let open = notify::open(&self.words, &runs[0].id, notify::Kind::Missed);
            notify::tell(&notify::to(&self.notify, self.notify_to(&runs[0].jobnet)), open.as_ref(), event, &title, &body);
        }
    }

    /// The `notify_to` of the net `id` (empty when it is gone).
    fn notify_to(&self, id: &str) -> &[String] {
        self.defs.iter().find(|d| d.id == id).and_then(|d| d.net.as_ref()).map_or(&[], |n| &n.notify_to)
    }

    /// Tells of the runs that are late (`late_start`, `late_end`, `late_slow`),
    /// each delay once, and marks them; and of the times that wait to start
    /// (`overlap = "queue"`, `wait_for`) past their `late_start`.
    fn tick_late(&mut self) {
        if self.late_look.is_some_and(|t| t.elapsed() < LATE_EVERY) {
            return;
        }
        self.late_look = Some(Instant::now());
        let now = ito_common::now::now_local().naive_local();
        let going: Vec<String> = self.runner.runs().iter().filter(|r| !r.is_over()).map(|r| r.jobnet.clone()).collect();
        for id in going {
            if !self.usual.contains_key(&id) {
                let today = now.date();
                let runs = self.runner.history(today - chrono::Days::new(USUAL_DAYS), today).unwrap_or_default();
                self.usual.insert(id.clone(), koyomi_core::run::usual_duration(&runs, &id));
            }
        }
        let mut found: Vec<(String, Delay)> = Vec::new();
        for r in self.runner.runs().iter().filter(|r| !r.is_over()) {
            let Some(net) = self.defs.iter().find(|d| d.id == r.jobnet).and_then(|d| d.net.as_ref()) else { continue };
            let usual = self.usual.get(&r.jobnet).copied().flatten();
            found.extend(late::overdue(net, r, usual, now).into_iter().map(|d| (r.id.clone(), d)));
        }
        for (id, delay) in found {
            self.runner.mark_delay(&id, delay, now);
            let Some(r) = self.runner.runs().iter().find(|r| r.id == id) else { continue };
            if delay == Delay::Start && r.planned.is_some_and(|p| self.late_told.remove(&(r.jobnet.clone(), p))) {
                continue;
            }
            let Some(net) = self.defs.iter().find(|d| d.id == r.jobnet).and_then(|d| d.net.as_ref()) else { continue };
            let usual = self.usual.get(&r.jobnet).copied().flatten();
            let (event, title, body) = notify::delay_words(&self.words, net, r, delay, usual, now);
            let open = notify::open(&self.words, &r.id, notify::Kind::Late);
            notify::tell(&notify::to(&self.notify, &net.notify_to), open.as_ref(), event, &title, &body);
        }
        let pending = self
            .runner
            .queued()
            .iter()
            .map(|q| (&q.def, q.planned))
            .chain(self.waiting.iter().map(|w| (&w.def, w.planned)))
            .chain(self.quieted.iter().map(|q| (&q.def, q.planned)));
        let mut tell = Vec::new();
        for (def, planned) in pending {
            let Some(by) = def.net.as_ref().and_then(|n| n.late_start).map(|d| d.from(planned)) else { continue };
            if now >= by && self.late_told.insert((def.id.clone(), planned)) {
                let title = def.title.clone().unwrap_or_else(|| def.name.clone());
                let places = notify::to(&self.notify, def.net.as_ref().map_or(&[][..], |n| &n.notify_to));
                tell.push((places, notify::pending_words(&self.words, &def.id, &title, planned, by, now)));
            }
        }
        for (places, (event, title, body)) in tell {
            notify::tell(&places, None, event, &title, &body);
        }
        // A time that never got a run (dropped from the queue) is not kept for ever.
        self.late_told.retain(|(_, p)| *p > now - chrono::Days::new(2));
    }

    /// Holds the PC awake while a job's process runs, or from a wake timer
    /// to the time it woke the PC for, and lets go when neither.
    /// What is left to do before the engine ends: the queue told why it will not run, the books written.
    fn shut_down(&mut self) {
        self.runner.drop_queue();
        self.runner.stop_all(Duration::from_secs(5));
        self.runner.flush();
        self.schedule.flush();
        self.retain.flush();
        self.backup.flush();
        self.alive.stopped();
        self.http.stop();
    }

    /// What the tray shows now.
    fn tray_look(&self) -> tray::Look {
        let now = ito_common::now::now_local().naive_local();
        let next = if self.held.paused { None } else { next_due(&self.defs, &self.calendars, &self.held, &self.blackouts, now, false) };
        // `acked.toml` is the window's; read it again at most every few seconds.
        let acked = {
            let mut c = self.acked.lock().unwrap_or_else(|e| e.into_inner());
            if c.0.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(3)) {
                *c = (
                    Some(std::time::Instant::now()),
                    ito_common::state_dir("koyomi").map(|d| crate::acked::Acked::read(&d)).unwrap_or_default(),
                );
            }
            c.1.clone()
        };
        tray::look(&self.words, &self.runner.today(), &acked, self.held.paused, self.runner.busy(), next.map(|(at, _)| at))
    }

    /// What `koyomi tray` asks for: the look, and the notices kept for it after `since`.
    fn tray_view(&self, since: Option<u64>) -> proto::TrayView {
        let look = self.tray_look();
        let (notes, last) = notify::for_tray(since);
        proto::TrayView {
            badge: tray::badge(look.state).to_owned(),
            tip: look.tip,
            paused: look.paused,
            busy: look.busy,
            errors: look.errors,
            own: !notify::handed_to_tray(),
            notes,
            last,
        }
    }

    fn tick_awake(&mut self) {
        let busy = self.runner.busy();
        let now = ito_common::now::now_local().naive_local();
        let waking = self.wake.as_ref().is_some_and(|w| w.set.is_ok() && w.waking(now));
        if (busy > 0 || waking) && self.awake.is_none() {
            let why = if busy > 0 { "koyomi: ジョブを実行中" } else { "koyomi: ジョブネットの時刻を待っている" };
            self.awake = Some(ito_power::Awake::hold(why));
        } else if busy == 0 && !waking {
            self.awake = None;
        }
    }

    /// Reads the power settings and sets the PC to wake a little before the
    /// soonest time of a `wake = true` net (once a minute; a timer already set
    /// for the same time is kept, one that failed is tried again).
    fn tick_wake(&mut self) {
        if self.power_look.is_some_and(|t| t.elapsed() < POWER_LOOK) {
            return;
        }
        self.power_look = Some(Instant::now());
        self.sleep_after = ito_power::sleep_after();
        let now = ito_common::now::now_local().naive_local();
        let next = if self.held.paused { None } else { next_wake(&self.defs, &self.calendars, &self.held, &self.blackouts, now) };
        let Some((start, id)) = next else {
            self.wake = None;
            return;
        };
        if self.wake.as_ref().is_some_and(|w| w.start == start && w.set.is_ok()) {
            if let Some(w) = &mut self.wake {
                w.id = id;
            }
            return;
        }
        let at = start - WAKE_LEAD;
        let set = match Local.from_local_datetime(&at).earliest() {
            Some(t) => ito_power::wake_at(t.into()),
            None => Err(ito_power::WhyNot::InThePast),
        };
        self.wake = Some(Wake { start, id, set });
    }

    /// The power settings for the window: they mark the next times.
    fn power(&self) -> proto::Power {
        let ok = self.wake.as_ref().filter(|w| w.set.is_ok());
        proto::Power {
            sleep_after: self.sleep_after.map(|d| d.as_secs()),
            cannot_wake: self.wake.as_ref().and_then(|w| w.set.as_ref().err()).and_then(|e| why_key(*e)).map(str::to_owned),
            wake_at: ok.map(|w| (w.start - WAKE_LEAD).format("%Y-%m-%d %H:%M").to_string()),
            wake_for: ok.map(|w| w.id.clone()),
        }
    }

    /// Starts what `[[when]]` says is due since the last look, and records
    /// each time that did not start with why (`schedule::judge`).
    fn tick_schedule(&mut self) {
        let now = ito_common::now::now();
        let Some(checked) = self.schedule.look(now) else { return };
        // Standing by, or a primary whose standby has them: the other PC runs these times.
        // Nothing is written for them here, and after a handover only the times from then on run.
        if self.standby.waiting() || self.primary.held().is_some() {
            self.schedule.looked(now, false);
            return;
        }
        // A primary still asking its standby: the times wait, and are judged once it knows.
        if self.primary.asking() {
            return;
        }
        let mut due: Vec<(usize, DateTime<Utc>, NaiveDateTime)> = Vec::new();
        for (i, d) in self.defs.iter().enumerate() {
            if let Some(net) = &d.net {
                let cal = d.calendar.as_deref().and_then(|c| self.calendars.get(c));
                due.extend(when::due_in(&Local, &net.when, cal, checked, now).into_iter().map(|(instant, planned)| (i, instant, planned)));
            }
        }
        for (k, (i, instant, planned)) in due.iter().enumerate() {
            let def = self.defs[*i].clone();
            // Moved with `koyomi shift`: `tick_shift` starts it at the new time.
            if self.held.is_shifted(&def.id, *planned) {
                continue;
            }
            // Skipped once with `koyomi skip-next`: written down, and the skip is used up.
            if !self.held.skips.is_empty() && self.held_bad.is_none() {
                let hit = self.held.take_skip(&def.id, *planned);
                let _ = self.save_held();
                if hit {
                    self.runner.not_run(&def, *planned, RunState::Skipped, "次の 1 回だけスキップ（koyomi skip-next）");
                    continue;
                }
            }
            if let Some((state, why)) = self.stopped(&def, planned.date(), *planned) {
                self.runner.not_run(&def, *planned, state, &why);
                continue;
            }
            let running = self.runner.runs().iter().any(|r| r.jobnet == def.id && !r.is_over());
            let last = !due[k + 1..].iter().any(|(j, _, _)| j == i);
            let known = self.knew(&def, *planned);
            let verdict =
                schedule::judge_known(&def, *planned, now - *instant, &self.warnings, self.bad_config.as_deref(), running, last, known);
            if let schedule::Verdict::Start { late } | schedule::Verdict::Queue { late } = verdict {
                match schedule::quiet(&def, *planned, now.with_timezone(&Local).naive_local()) {
                    Some(schedule::Quiet::Skip(why)) => {
                        self.runner.not_run(&def, *planned, RunState::Skipped, &why);
                        continue;
                    }
                    // One time of a net waits; the next ones in the same hours are not piled up behind it.
                    Some(schedule::Quiet::Wait(until)) => {
                        if let Some(first) = self.quieted.iter().find(|q| q.def.id == def.id) {
                            let why = format!("流さない時間帯。{} の回が終わりを待っている", first.planned.format("%-H:%M"));
                            self.runner.not_run(&def, *planned, RunState::Skipped, &why);
                        } else {
                            self.quieted.push(Quieted { def, planned: *planned, until, late });
                        }
                        continue;
                    }
                    None => {}
                }
            }
            self.begin(&def, *planned, verdict);
        }
        self.schedule.looked(now, !due.is_empty());
    }

    /// Polls the PC `[standby]` watches, and tells of a takeover, of both running and of a
    /// handover back. On a takeover the runs that were going there start again here from where
    /// they were for the nets that say so (`takeover = "rerun"`); the rest are written down.
    fn tick_standby(&mut self) {
        self.write_copies();
        for told in self.standby.tick() {
            let (title, line) = match told {
                standby::Told::TookOver(line, runs) => {
                    let from = self.standby.watched().unwrap_or_default().to_owned();
                    let taken = self.take_runs(runs, &from);
                    ("koyomi: 待機から引き継いだ", format!("{line}{taken}"))
                }
                standby::Told::Both(line) => ("koyomi: 両方で流している", line),
                standby::Told::Waiting(line) => ("koyomi: 引き取りを待っている", line),
                standby::Told::GaveBack(line) => ("koyomi: 戻した", line),
            };
            crate::notify::tell(&self.notify, None, "failed", title, &line);
            crate::say(&format!("koyomi: {line}"), false);
        }
    }

    /// Asks the standby `[primary]` names, and tells when it took the schedules or let them go.
    fn tick_primary(&mut self) {
        if let Some((title, line)) = self.primary.tick() {
            crate::notify::tell(&self.notify, None, "failed", title, &line);
            crate::say(&format!("koyomi: {line}"), false);
        }
    }

    /// The runs `from` had going when this standby took over: each of a net with
    /// `takeover = "rerun"` starts again here from the first job that had not ended ok; the rest
    /// are written down as not taken over, for a person to decide. What to add to the line.
    fn take_runs(&mut self, runs: Option<Vec<Run>>, from: &str) -> String {
        let Some(runs) = runs else {
            return "。向こうで動いていた回は読めていなかった".to_owned();
        };
        if runs.is_empty() {
            return String::new();
        }
        let today = ito_common::now::now_local().date_naive();
        let mut again = 0;
        let mut left = Vec::new();
        for theirs in &runs {
            let def = self.defs.iter().find(|d| d.id == theirs.jobnet && d.standing == Standing::Confirmed).cloned();
            let why = match (koyomi_core::standby::resume(theirs, def.as_ref().and_then(|d| d.net.as_ref())), &def) {
                (Resume::Rerun, Some(d)) => match self.stopped(d, today, ito_common::now::now_local().naive_local()) {
                    Some((_, why)) => why,
                    None => match self.runner.resume(d, theirs, from) {
                        Ok(_) => {
                            again += 1;
                            continue;
                        }
                        Err(e) => e,
                    },
                },
                (Resume::Rerun, None) => "この PC に流せる定義がない".to_owned(),
                (Resume::Leave(why), _) => why,
            };
            left.push(self.runner.left(theirs, def.as_ref(), from, &why));
        }
        let mut out = format!("。{from} で動いていた {} 回: 続きから流す {again} 回", runs.len());
        if !left.is_empty() {
            out.push_str(&format!("、引き継いでいない {}（koyomi rerun <id> で続きから流せる）", left.join("・")));
        }
        out
    }

    /// `"ha"` of `GET /health`, and with `release` the primary's `POST /standby/release`: with
    /// `[standby]`, whether it has the schedules and is ready to give them back; with
    /// `[primary]`, whether it runs them. A release is refused within 30 s of a time of a
    /// confirmed net, so no time falls between the two PCs while the answer travels.
    fn handover(&mut self, release: bool) -> Result<String, String> {
        if self.standby.watched().is_some() {
            if release {
                if let Some(near) = self.near_time() {
                    return Err(format!("時刻が近い（{near}）。少しあとで"));
                }
            }
            return self.standby.handover(release).map(|v| v.to_string());
        }
        match self.primary.json() {
            Some(_) if release => Err("この PC は [primary]。戻すのは待機の PC".to_owned()),
            Some(v) => Ok(v.to_string()),
            None => Err("koyomi.toml に [standby] も [primary] もない".to_owned()),
        }
    }

    /// A time of a confirmed net within 30 s of now, as `<net> <time>`.
    fn near_time(&self) -> Option<String> {
        let now = ito_common::now::now();
        let gap = chrono::Duration::seconds(30);
        self.defs.iter().filter(|d| d.standing == Standing::Confirmed).find_map(|d| {
            let net = d.net.as_ref()?;
            let cal = d.calendar.as_deref().and_then(|c| self.calendars.get(c));
            let due = when::due_in(&Local, &net.when, cal, now - gap, now + gap);
            due.first().map(|(_, planned)| format!("{} {}", d.id, planned.format("%H:%M:%S")))
        })
    }

    /// Writes what the standby copied from the engine it watches, unconfirmed (TODO 87). A
    /// net that calls another goes in after it: the ones refused are tried again while some
    /// go in.
    fn write_copies(&mut self) {
        let mut left = self.standby.copies(|| self.defs.iter().map(|d| (d.id.clone(), d.sha256.clone())).collect());
        let mut refused = Vec::new();
        while !left.is_empty() {
            let before = left.len();
            refused.clear();
            let mut next = Vec::new();
            for c in left {
                match self.write_def(&c.id, &c.text, c.base.as_deref(), false, false) {
                    Ok(()) => {
                        crate::say(&format!("koyomi: [standby] 写した（未確認。この PC の窓で確認するまで動かない）: {}", c.id), false)
                    }
                    Err(e) => {
                        refused.push(format!("{}: {e}", c.id));
                        next.push(c);
                    }
                }
            }
            if next.len() == before {
                break;
            }
            left = next;
        }
        self.standby.refused(refused);
    }

    /// Does what `verdict` says with the time `planned` of `def`.
    fn begin(&mut self, def: &Def, planned: NaiveDateTime, verdict: schedule::Verdict) {
        match verdict {
            schedule::Verdict::Start { late } => {
                if let Some(w) = self.wait_of(def, planned) {
                    self.waiting.push(w);
                    return;
                }
                let out = if late {
                    self.runner.start_late(def, planned)
                } else {
                    self.runner.start(def, planned.date(), How::Schedule, Some(planned))
                };
                if let Err(e) = out {
                    self.runner.not_run(def, planned, RunState::Locked, &e);
                }
            }
            schedule::Verdict::Queue { late } => self.runner.queue(def, planned, late),
            schedule::Verdict::NotRun(state, reason) => {
                self.runner.not_run(def, planned, state, &reason);
            }
        }
    }

    /// Starts the times that waited for their net's `quiet` hours when the
    /// hours are over, judged again then (the net may be held or running now).
    fn tick_quiet(&mut self) {
        if self.quieted.is_empty() || self.primary.asking() {
            return;
        }
        let now = ito_common::now::now_local().naive_local();
        for q in std::mem::take(&mut self.quieted) {
            if now < q.until {
                self.quieted.push(q);
                continue;
            }
            if let Some((state, why)) = self.stopped(&q.def, q.planned.date(), now) {
                self.runner.not_run(&q.def, q.planned, state, &why);
                continue;
            }
            let running = self.runner.runs().iter().any(|r| r.jobnet == q.def.id && !r.is_over());
            let verdict = match schedule::judge(
                &q.def,
                q.planned,
                chrono::Duration::zero(),
                &self.warnings,
                self.bad_config.as_deref(),
                running,
                true,
            ) {
                schedule::Verdict::Start { .. } => schedule::Verdict::Start { late: q.late },
                schedule::Verdict::Queue { .. } => schedule::Verdict::Queue { late: q.late },
                v => v,
            };
            self.begin(&q.def, q.planned, verdict);
        }
    }

    /// The wait a due time of `def` goes into: the net `wait_for` names is
    /// scheduled on the day and has not ended its run of it yet. `None`: start
    /// now (no `wait_for`, the net is not scheduled that day, or it has ended).
    fn wait_of(&self, def: &Def, planned: NaiveDateTime) -> Option<Waiting> {
        let w = def.net.as_ref()?.when.iter().find(|w| !w.is_event() && w.wait_for.is_some())?;
        let target = w.wait_for.clone()?;
        let day = planned.date();
        let other = self.defs.iter().find(|d| d.id == target && d.id != def.id)?;
        let cal = other.calendar.as_deref().and_then(|c| self.calendars.get(c));
        if !other.net.as_ref()?.when.iter().any(|x| !x.is_event() && !x.times_on_in(day, cal).is_empty()) {
            return None;
        }
        if wait_over(self.runner.runs(), &target, day) {
            return None;
        }
        Some(Waiting { def: def.clone(), planned, for_net: target, until: w.wait_until.map(|t| day.and_time(t)) })
    }

    /// Starts the times that waited when the net they wait for has ended its
    /// run of the day; one that passes `wait_until` first is recorded as an
    /// error (and told of, as an error is).
    fn tick_wait(&mut self) {
        if self.waiting.is_empty() {
            return;
        }
        let now = ito_common::now::now_local().naive_local();
        for w in std::mem::take(&mut self.waiting) {
            let day = w.planned.date();
            if wait_over(self.runner.runs(), &w.for_net, day) {
                if let Err(e) = self.runner.start(&w.def, day, How::Schedule, Some(w.planned)) {
                    self.runner.not_run(&w.def, w.planned, RunState::Locked, &e);
                }
            } else if w.until.is_some_and(|u| now >= u) {
                let why = format!(
                    "{} の終わりを {} まで待ったが、終わらなかった（wait_until）",
                    w.for_net,
                    w.until.map_or(String::new(), |u| u.format("%H:%M").to_string())
                );
                self.runner.not_run(&w.def, w.planned, RunState::Error, &why);
            } else {
                self.waiting.push(w);
            }
        }
    }

    /// Why `def` is not to start on `day` whatever its `[[when]]` says: `at` (its
    /// time) or now is in a planned stop (`[[blackout]]`), the holiday table does
    /// not reach the day (it may be a holiday, so it is not run as if it were not),
    /// the schedule is paused, or the net is held. Each is written down (⏸ for the
    /// two by hand), so nothing is lost in silence.
    fn stopped(&self, def: &Def, day: NaiveDate, at: NaiveDateTime) -> Option<(RunState, String)> {
        let now = ito_common::now::now_local().naive_local();
        if let Some(b) = koyomi_core::blackout::find(&self.blackouts, at).or_else(|| koyomi_core::blackout::find(&self.blackouts, now)) {
            return Some((RunState::Skipped, b.reason()));
        }
        let cal = def.calendar.as_deref().and_then(|c| self.calendars.get(c));
        if let Some(until) = cal.filter(|c| c.stale(day)).and_then(|c| c.until()) {
            return Some((RunState::Locked, format!("祝日の表が古い（{until} まで）")));
        }
        if self.standby.waiting() {
            return Some((RunState::Skipped, "待機中（見張っている PC が流す）".to_owned()));
        }
        if let Some(why) = self.primary.held() {
            return Some((RunState::Skipped, why));
        }
        if self.held.paused {
            return Some((RunState::Held, "スケジュールを止めている（koyomi resume）".to_owned()));
        }
        if self.held.is_held(&def.id) {
            return Some((RunState::Held, format!("保留中（koyomi release {}）", def.id)));
        }
        None
    }

    /// Why an event on `day` does not start `def` by its `when`: the day is
    /// outside `start` / `end` or in `except`, or it is a closed day and
    /// `on_closed` is not `"run"`. `what` is the event, for the record.
    fn off_day(&self, def: &Def, w: &when::When, day: NaiveDate, what: &str) -> Option<String> {
        if !w.runs_on(day) {
            return Some(format!("{what}が、except か start / end の外なので流さない"));
        }
        let cal = def.calendar.as_deref().and_then(|c| self.calendars.get(c));
        if cal.is_some_and(|c| !c.is_business(day)) && w.on_closed_in(cal) != OnClosed::Run {
            return Some(format!("{what}が、休業日なので流さない（on_closed = \"run\" で流せる）"));
        }
        None
    }

    /// Starts what an event says: the nets with `on_start = true` once, when
    /// the engine starts; a net with `file = …` for each file that has come
    /// (`events::Events`); a net with `log`, `eventlog`, `mail` or `idle` when
    /// that comes (`logs`, `mails`, `idle`). What does not start is recorded
    /// with why, as a time is. A file that comes while its net runs waits for
    /// the run to end; the others are recorded as not run.
    fn tick_events(&mut self) {
        // A primary still asking its standby: the events wait until it knows.
        if self.primary.asking() {
            return;
        }
        let now = ito_common::now::now_local().naive_local();
        let today = now.date();
        if self.events.take_start() {
            for def in self.defs.clone() {
                let Some(w) = def.net.as_ref().and_then(|n| n.when.iter().find(|w| matches!(w.kind, Kind::OnStart))) else { continue };
                if let Some(why) = self.off_day(&def, w, today, "起動した") {
                    self.runner.not_run(&def, now, RunState::Skipped, &why);
                    continue;
                }
                if let Some((state, why)) = self.stopped(&def, today, now) {
                    self.runner.not_run(&def, now, state, &why);
                    continue;
                }
                match schedule::judge(&def, now, chrono::Duration::zero(), &self.warnings, self.bad_config.as_deref(), false, true) {
                    schedule::Verdict::Start { .. } | schedule::Verdict::Queue { .. } => {
                        if let Err(e) = self.runner.start_event(&def, today, How::Start, "起動したとき", None) {
                            self.runner.not_run(&def, now, RunState::Locked, &e);
                        }
                    }
                    schedule::Verdict::NotRun(state, reason) => {
                        self.runner.not_run(&def, now, state, &reason);
                    }
                }
            }
        }
        for a in self.events.due() {
            let path = a.path.display().to_string();
            let Some(def) = self.defs.iter().find(|d| d.id == a.def_id).cloned() else {
                self.events.fired(&a.def_id, &a.pattern, &a.path);
                continue;
            };
            let Some(w) =
                def.net.as_ref().and_then(|n| n.when.iter().find(|w| matches!(&w.kind, Kind::File { file, .. } if *file == a.pattern)))
            else {
                self.events.fired(&a.def_id, &a.pattern, &a.path);
                continue;
            };
            if let Some(why) = self.off_day(&def, w, today, &format!("{path} が来た")) {
                self.runner.not_run(&def, now, RunState::Skipped, &why);
                self.events.fired(&a.def_id, &a.pattern, &a.path);
                continue;
            }
            if let Some((state, why)) = self.stopped(&def, today, now) {
                self.runner.not_run(&def, now, state, &why);
                self.events.fired(&a.def_id, &a.pattern, &a.path);
                continue;
            }
            // Still running: the file keeps; it starts the net when the run ends.
            if self.runner.runs().iter().any(|r| r.jobnet == def.id && !r.is_over()) {
                continue;
            }
            match schedule::judge(&def, now, chrono::Duration::zero(), &self.warnings, self.bad_config.as_deref(), false, true) {
                schedule::Verdict::Start { .. } | schedule::Verdict::Queue { .. } => {
                    if let Err(e) = self.runner.start_event(&def, today, How::File, &format!("{path} が来た"), Some(path)) {
                        self.runner.not_run(&def, now, RunState::Locked, &e);
                    }
                }
                schedule::Verdict::NotRun(state, reason) => {
                    self.runner.not_run(&def, now, state, &reason);
                }
            }
            self.events.fired(&a.def_id, &a.pattern, &a.path);
        }
        let hits: Vec<logs::Hit> = self.logs.due().into_iter().chain(self.mails.due()).chain(self.idle.due()).collect();
        for hit in hits {
            let Some(def) = self.defs.iter().find(|d| d.id == hit.def_id).cloned() else { continue };
            let Some(w) = def.net.as_ref().and_then(|n| n.when.iter().find(|w| w.kind == hit.kind)).cloned() else { continue };
            let what = match &hit.kind {
                Kind::Log { log, .. } => format!("{log} に行が書かれた"),
                Kind::EventLog { eventlog, .. } => format!("イベントログ {eventlog} に記録された"),
                Kind::Mail { mail, .. } => format!("{mail} にメールが来た"),
                Kind::Idle { idle: secs } => format!("PC が {} 使われなかった", idle::span_text(*secs)),
                _ => continue,
            };
            let how = match &hit.kind {
                Kind::Mail { .. } => How::Mail,
                Kind::Idle { .. } => How::Idle,
                _ => How::Log,
            };
            let more = if hit.count > 1 { format!("（ほか {} 件）", hit.count - 1) } else { String::new() };
            // The PC left alone has no line to keep.
            let tail = if hit.first.is_empty() { more } else { format!(": {}{more}", hit.first) };
            let reason = format!("{what}{tail}");
            if let Some(why) = self.off_day(&def, &w, today, &what) {
                self.runner.not_run(&def, now, RunState::Skipped, &format!("{why}{tail}"));
                continue;
            }
            if let Some((state, why)) = self.stopped(&def, today, now) {
                self.runner.not_run(&def, now, state, &why);
                continue;
            }
            // A line, a mail or the PC left alone is not kept as a file is: one that comes while the net runs is told of, not run later.
            if self.runner.runs().iter().any(|r| r.jobnet == def.id && !r.is_over()) {
                self.runner.not_run(&def, now, RunState::Skipped, &format!("{what}が、前の回がまだ動いているので流さない{tail}"));
                continue;
            }
            match schedule::judge(&def, now, chrono::Duration::zero(), &self.warnings, self.bad_config.as_deref(), false, true) {
                schedule::Verdict::Start { .. } | schedule::Verdict::Queue { .. } => {
                    if let Err(e) = self.runner.start_event(&def, today, how, &reason, None) {
                        self.runner.not_run(&def, now, RunState::Locked, &e);
                    }
                }
                schedule::Verdict::NotRun(state, reason) => {
                    self.runner.not_run(&def, now, state, &reason);
                }
            }
        }
    }

    /// Starts the nets with `[[when]] after_jobnet = …` on each run of `over`
    /// that ended as their `on` says, however that run was started. The run
    /// takes the business day of the run it follows (`KOYOMI_DATE`). A net
    /// starts this way once a business day: a second end on the same day is
    /// recorded as ⊘ スキップ and told of, so a person runs it by hand if it
    /// is wanted. A net still running from before does not start again.
    fn tick_after(&mut self, over: &[Run]) {
        let now = ito_common::now::now_local().naive_local();
        for prev in over {
            for (def, w) in followers(&self.defs, prev) {
                let date = NaiveDate::parse_from_str(&prev.date, "%Y-%m-%d").unwrap_or(now.date());
                let what = format!("{} が終わった（{}）", prev.jobnet, crate::show::word(&self.words, prev.state));
                if let Some(why) = self.off_day(&def, &w, date, &what) {
                    self.runner.not_run(&def, now, RunState::Skipped, &why);
                    continue;
                }
                if let Some((state, why)) = self.stopped(&def, date, now) {
                    self.runner.not_run(&def, now, state, &why);
                    continue;
                }
                if let Some(done) = followed(self.runner.runs(), &def.id, &prev.date) {
                    let why = format!(
                        "{what}が、同じ業務日（{}）に既に流した（{}）: 後続は流していない。要るなら手で流す（koyomi run {}）",
                        prev.date, done.id, def.id
                    );
                    self.runner.not_run(&def, now, RunState::Skipped, &why);
                    continue;
                }
                if self.runner.runs().iter().any(|r| r.jobnet == def.id && !r.is_over()) {
                    self.runner.not_run(&def, now, RunState::Skipped, &format!("{what}が、前の回がまだ動いているので流さない"));
                    continue;
                }
                match schedule::judge(&def, now, chrono::Duration::zero(), &self.warnings, self.bad_config.as_deref(), false, true) {
                    schedule::Verdict::Start { .. } | schedule::Verdict::Queue { .. } => {
                        if let Err(e) = self.runner.start_event(&def, date, How::After, &what, None) {
                            self.runner.not_run(&def, now, RunState::Locked, &e);
                        }
                    }
                    schedule::Verdict::NotRun(state, reason) => {
                        self.runner.not_run(&def, now, state, &reason);
                    }
                }
            }
        }
    }

    /// Reads `approved.toml` again; only while it does not read (the engine is its one writer).
    fn reload(&mut self) {
        match self.paths.store() {
            Ok(store) => {
                self.store = store;
                self.unreadable = None;
            }
            Err(e) => {
                self.store = Store::default();
                self.unreadable = Some(e);
            }
        }
        self.reload_held();
    }

    /// Reads `held.toml` again: a person may edit it while the engine runs.
    fn reload_held(&mut self) {
        match self.paths.held() {
            Ok(held) => {
                self.held = held;
                self.held_bad = None;
            }
            Err(e) => {
                self.held = Held::default();
                self.held_bad = Some(e);
            }
        }
        self.runner.set_held(&self.held);
    }

    /// Reads `next.toml` again: a person may edit it (a text changed by hand takes over unconfirmed).
    fn reload_staged(&mut self) {
        let staged = match self.paths.staged() {
            Ok(staged) => {
                self.staged_bad = None;
                staged
            }
            Err(e) => {
                self.staged_bad = Some(e);
                Default::default()
            }
        };
        if staged != self.staged {
            self.staged = staged;
            self.staged_look = None;
            let staged = &self.staged;
            self.staged_failed.retain(|id, _| staged.get(id).is_some());
        }
    }

    fn rescan(&mut self) {
        if self.unreadable.is_some() {
            self.reload();
        } else {
            self.reload_held();
        }
        self.reload_staged();
        self.reload_config();
        self.calendars = Calendars::load(&self.paths.calendars);
        self.defs = scan_in(&self.paths.jobnets, &self.store, &self.calendars);
        self.runner.set_children(Self::children_of(&self.defs));
        self.runner.set_calendars(
            self.calendars.map.clone(),
            self.defs.iter().filter_map(|d| Some((d.id.clone(), d.calendar.clone()?))).collect(),
        );
        // Files there before the watch began are not run: they may be old, and
        // a restart must not run them again. The record says they were there.
        let now = ito_common::now::now_local().naive_local();
        for (id, pattern, n) in self.events.set_rules(&self.defs) {
            if let Some(def) = self.defs.iter().find(|d| d.id == id).cloned() {
                self.runner.not_run(&def, now, RunState::Skipped, &format!("見張り始めたときに既に {n} 個あった（{pattern}）: 流さない"));
            }
        }
        self.logs.set_rules(&self.defs);
        self.mails.set_rules(&self.defs);
        self.idle.set_rules(&self.defs);
        let files: Vec<PathBuf> = self.defs.iter().map(|d| self.paths.jobnets.join(&d.file)).collect();
        self.warnings = access_warnings(&self.paths, &files);
        self.warnings.extend(self.unreadable.clone());
        self.warnings.extend(self.held_bad.clone());
        self.warnings.extend(self.staged_bad.clone());
    }

    /// What a `kind = "jobnet"` part may use: each confirmed net, or why the others cannot be.
    fn children_of(defs: &[Def]) -> BTreeMap<String, Result<Jobnet, String>> {
        defs.iter()
            .map(|d| {
                let net = match (d.standing, &d.net) {
                    (Standing::Confirmed, Some(n)) => Ok(n.clone()),
                    (Standing::Unconfirmed, _) => Err(format!("子ジョブネット {} が未確認", d.id)),
                    _ => Err(format!("子ジョブネット {} に定義の誤りがある", d.id)),
                };
                (d.id.clone(), net)
            })
            .collect()
    }

    fn snapshot(&self) -> Snapshot {
        let mut warnings = self.warnings.clone();
        warnings.extend(self.bad_config.clone());
        warnings.extend(self.logs.problems());
        warnings.extend(self.mails.problems());
        warnings.extend(self.idle.problems());
        let ping = self.ping.notes(self.ping_cfg.as_ref(), self.ping_plain_ok);
        let notes = self
            .notify_notes
            .iter()
            .chain(&self.reveal_notes)
            .cloned()
            .chain(ping)
            .chain(self.http.notes())
            .chain(self.standby.notes())
            .chain(self.primary.notes())
            // Not a reason to stop: the folders are looked at every 5 s instead, and a file still starts its net.
            .chain((!self.events.watching()).then(|| "待つファイルのフォルダーを見張れない（5 秒ごとに見る）".to_owned()))
            .collect();
        Snapshot {
            defs: self.defs.clone(),
            warnings,
            notes,
            held: self.held.held.clone(),
            held_jobs: self.held.jobs.clone(),
            skips: self.held.skips.clone(),
            paused: self.held.paused,
            cautions: self
                .outage_line()
                .into_iter()
                .chain(self.standby.line())
                .chain(self.primary.caution())
                .chain(self.calendars.cautions(ito_common::now::now_local().date_naive(), CAUTION_DAYS))
                .chain(self.staged_failed.values().cloned())
                .collect(),
            calendar_cautions: self.calendars.cautions_by_calendar(ito_common::now::now_local().date_naive(), CAUTION_DAYS),
            calendars: self.calendars.map.clone(),
            power: self.power(),
            blackouts: self.blackouts.clone(),
            staged: self.staged.next.clone(),
        }
    }

    fn status(&self) -> String {
        let confirmed = self.defs.iter().filter(|d| d.standing == Standing::Confirmed).count();
        let running = self.runner.runs().iter().filter(|r| !r.is_over()).count();
        let mut out = format!(
            "koyomi engine pid {}: 定義 {} 件（確認済み {}）、動いている実行 {}\n{}\n{}",
            std::process::id(),
            self.defs.len(),
            confirmed,
            running,
            self.retain.line(),
            self.backup.line()
        );
        if let Some(line) = self.http.line() {
            out.push_str(&format!("\n{line}"));
        }
        if let Some(line) = self.standby.line() {
            out.push_str(&format!("\n{line}"));
        }
        if let Some(line) = self.primary.line() {
            out.push_str(&format!("\n{line}"));
        }
        if self.held.paused {
            out.push_str("\nスケジュールを止めている（koyomi resume で戻す）");
        }
        for id in &self.held.held {
            out.push_str(&format!("\n保留中: {id}（koyomi release {id} で戻す）"));
        }
        for h in &self.held.jobs {
            out.push_str(&format!("\n保留中: {} の {}（koyomi release {} {} で戻す）", h.net, h.job, h.net, h.job));
        }
        let p = self.power();
        if let (Some(at), Some(id)) = (&p.wake_at, &p.wake_for) {
            out.push_str(&format!(
                "
{}",
                self.words.trf("power.waking", &[at, id])
            ));
        }
        if let Some(why) = &p.cannot_wake {
            out.push_str(&format!(
                "
{}",
                self.words.trf("power.not_waking", &[self.words.tr(&format!("power.why_{why}"))])
            ));
        }
        for q in self.runner.queued() {
            out.push_str(&format!(
                "\n待ち: {}  {} の回は前の回が終わってから流す（{} から待っている）",
                q.def.label(),
                q.planned.format("%-H:%M"),
                q.since.format("%-H:%M")
            ));
        }
        out
    }

    /// The sweep of old records: started for a new day or new settings
    /// (not while koyomi.toml does not read: a typo must not sweep), its
    /// end told on the console.
    fn tick_retain(&mut self) {
        if self.bad_config.is_some() {
            return;
        }
        if let Some(line) = self.retain.tick(self.keep) {
            crate::say(&format!("koyomi: {line}"), false);
        }
    }

    /// The daily backup: its end told on the console.
    /// The outage the engine woke up to, for the window's band: for an hour after the start.
    fn outage_line(&self) -> Option<String> {
        self.outage.as_ref().filter(|(_, at)| at.elapsed() < OUTAGE_SHOWN).map(|(l, _)| l.clone())
    }

    fn tick_backup(&mut self) {
        if let Some(line) = self.backup.tick() {
            crate::say(&format!("koyomi: {line}"), false);
        }
    }

    /// Starts `id` by hand, at the job `from` when there is one. Refused for
    /// every reason a run must not start quietly.
    fn start(
        &mut self,
        id: &str,
        date: Option<&str>,
        from: Option<&str>,
        dry: bool,
        set: &BTreeMap<String, String>,
    ) -> Result<String, String> {
        let def = self.startable(id)?.clone();
        let date = match date {
            Some(d) => NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| format!("業務日は YYYY-MM-DD: {d}"))?,
            None => ito_common::now::now_local().date_naive(),
        };
        if dry {
            return self.runner.dry(&def, date, set);
        }
        self.runner.start_manual(&def, date, from, set)
    }

    /// Starts `run` again ([`Runner::rerun`]): with the definition that run
    /// had, but only while the net is still confirmed as it stands now.
    fn rerun(&mut self, run: &str, from: Option<&str>, next: Option<RunState>) -> Result<String, String> {
        let prev = self.runner.previous(run)?;
        self.startable(&prev.jobnet)?;
        self.runner.rerun(run, from, next)
    }

    /// The definition `id` as it may be started: nothing warned, koyomi.toml reads, and it is confirmed.
    fn startable(&self, id: &str) -> Result<&Def, String> {
        if !self.warnings.is_empty() {
            return Err(format!("流せない: {}", self.warnings.join(" / ")));
        }
        if let Some(e) = &self.bad_config {
            return Err(format!("流せない: {e}"));
        }
        let def = self.defs.iter().find(|d| d.id == id).ok_or_else(|| format!("定義がない: {id}"))?;
        match def.standing {
            Standing::Invalid => Err(format!("定義の誤りがあり、流せない: {}", def.file)),
            Standing::Unconfirmed => Err(format!("未確認のため流さない: {}（koyomi approve {id}）", def.file)),
            Standing::Confirmed if self.held.is_held(id) => Err(format!("保留中のため流さない: {id}（koyomi release {id}）")),
            Standing::Confirmed => Ok(def),
        }
    }

    /// `held.toml` can be written: it read, and nothing else is in the way.
    fn held_writable(&self) -> Result<(), String> {
        match &self.held_bad {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }
    fn save_held(&self) -> Result<(), String> {
        if let Some(dir) = self.paths.held.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        ito_common::write_atomic(&self.paths.held, self.held.to_toml())
    }
    fn hold(&mut self, id: &str) -> Result<String, String> {
        self.held_writable()?;
        let label = self.defs.iter().find(|d| d.id == id).map(Def::label).ok_or_else(|| format!("定義がない: {id}"))?.to_owned();
        if !self.held.hold(id) {
            return Err(format!("もう保留している: {id}"));
        }
        self.save_held()?;
        for q in self.runner.unqueue(Some(id)) {
            self.runner.not_run(&q.def, q.planned, RunState::Held, &format!("待っている間に保留した（koyomi release {id}）"));
        }
        Ok(format!("保留した: {label}（koyomi release {id} で戻す）"))
    }
    fn release(&mut self, id: &str) -> Result<String, String> {
        self.held_writable()?;
        if !self.held.release(id) {
            return Err(format!("保留していない: {id}"));
        }
        self.save_held()?;
        Ok(format!("保留を解いた: {id}"))
    }
    /// `koyomi hold <net> <job>`: a run of `id` that comes to `job` waits there as ⏸ 保留.
    fn hold_job(&mut self, id: &str, job: &str) -> Result<String, String> {
        self.held_writable()?;
        let def = self.defs.iter().find(|d| d.id == id).ok_or_else(|| format!("定義がない: {id}"))?;
        let net = def.net.as_ref().ok_or_else(|| format!("{id} が読めない"))?;
        let label = net
            .jobs
            .iter()
            .find(|j| j.id == job)
            .map(|j| j.name.clone().unwrap_or_else(|| j.id.clone()))
            .ok_or_else(|| format!("{id} にジョブがない: {job}"))?;
        if !self.held.hold_job(id, job) {
            return Err(format!("もう保留している: {id} の {job}"));
        }
        self.save_held()?;
        self.runner.set_held(&self.held);
        Ok(format!("保留した: {} の {label}（そこに着いた回は待つ。koyomi release {id} {job} で戻す）", def.label()))
    }
    fn release_job(&mut self, id: &str, job: &str) -> Result<String, String> {
        self.held_writable()?;
        if !self.held.release_job(id, job) {
            return Err(format!("保留していない: {id} の {job}"));
        }
        self.save_held()?;
        self.runner.set_held(&self.held);
        Ok(format!("保留を解いた: {id} の {job}"))
    }
    /// `koyomi shift`: the time `from` of `id` starts at `to` once (`None`: the move is taken back).
    fn shift(&mut self, id: &str, from: &str, to: Option<&str>) -> Result<String, String> {
        self.held_writable()?;
        let at = |s: &str| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").map_err(|_| format!("時刻は YYYY-MM-DD HH:MM で書く: {s}"));
        let from = at(from)?;
        let Some(to) = to else {
            if !self.held.unshift(id, from) {
                return Err(format!("{} の {} はずらしていない", id, from.format("%Y-%m-%d %H:%M")));
            }
            self.save_held()?;
            return Ok("ずらしを戻した".to_owned());
        };
        let to = at(to)?;
        let def = self.defs.iter().find(|d| d.id == id).ok_or_else(|| format!("定義がない: {id}"))?;
        let net = def.net.as_ref().ok_or_else(|| format!("定義が読めない: {id}"))?;
        let cal = def.calendar.as_deref().and_then(|c| self.calendars.get(c));
        if !net.when.iter().any(|w| !w.is_event() && w.times_on_in(from.date(), cal).contains(&from.time())) {
            return Err(format!("{id} にその時刻はない: {}（koyomi next {id} で見る）", from.format("%Y-%m-%d %H:%M")));
        }
        if to <= from {
            return Err("ずらす先は、元の時刻より後にする".to_owned());
        }
        if to <= ito_common::now::now_local().naive_local() {
            return Err("ずらす先が過ぎている".to_owned());
        }
        self.held.shift(id, from, to);
        self.save_held()?;
        Ok(format!("{} の {} を {} にずらした（定義は変えない）", id, from.format("%Y-%m-%d %H:%M"), to.format("%Y-%m-%d %H:%M")))
    }
    /// `koyomi skip-next`: the next time of `id` that is not yet skipped or moved does not start
    /// (`cancel`: every skip of `id` is taken back).
    fn skip_next(&mut self, id: &str, cancel: bool) -> Result<String, String> {
        self.held_writable()?;
        if cancel {
            if !self.held.unskip(id) {
                return Err(format!("{id} は飛ばしていない"));
            }
            self.save_held()?;
            return Ok(format!("{id} を飛ばすのをやめた"));
        }
        let def = self.defs.iter().find(|d| d.id == id).ok_or_else(|| format!("定義がない: {id}"))?;
        let net = def.net.as_ref().ok_or_else(|| format!("定義が読めない: {id}"))?;
        if def.standing != Standing::Confirmed {
            return Err(format!("{id} は承認されていないので、時刻では流れない"));
        }
        let cal = def.calendar.as_deref().and_then(|c| self.calendars.get(c));
        let held = &self.held;
        let at = koyomi_core::held::next_open(&net.when, cal, ito_common::now::now_local().naive_local(), |t| {
            held.is_skipped(id, t) || held.is_shifted(id, t)
        })
        .ok_or_else(|| format!("{id} にこの先の時刻がない（koyomi next {id} で見る）"))?;
        self.held.skip(id, at);
        self.save_held()?;
        Ok(format!("{} の {} を飛ばす（定義は変えない。戻すのは koyomi skip-next {} --cancel）", id, at.format("%Y-%m-%d %H:%M"), id))
    }
    /// Starts the times that were moved once their new time comes.
    fn tick_shift(&mut self) {
        if self.held.shifts.is_empty() || self.held_bad.is_some() || self.primary.asking() {
            return;
        }
        let now = ito_common::now::now_local().naive_local();
        let due: Vec<_> = self.held.shifts.iter().filter(|s| s.to <= now).cloned().collect();
        if due.is_empty() {
            return;
        }
        for s in &due {
            self.held.unshift(&s.id, s.from);
        }
        let _ = self.save_held();
        for s in due {
            let Some(def) = self.defs.iter().find(|d| d.id == s.id).cloned() else { continue };
            if let Some((state, why)) = self.stopped(&def, s.from.date(), now) {
                self.runner.not_run(&def, s.from, state, &why);
            } else if let Err(e) = self.runner.start(&def, s.from.date(), How::Schedule, Some(s.from)) {
                self.runner.not_run(&def, s.from, RunState::Locked, &e);
            }
        }
    }
    fn pause(&mut self, on: bool) -> Result<String, String> {
        self.held_writable()?;
        if self.held.paused == on {
            return Err(if on { "もう止めている（koyomi resume で戻す）" } else { "止めていない" }.to_owned());
        }
        self.held.paused = on;
        self.save_held()?;
        if on {
            for q in self.runner.unqueue(None) {
                self.runner.not_run(&q.def, q.planned, RunState::Held, "待っている間にスケジュールを止めた（koyomi resume）");
            }
        }
        Ok(if on { "スケジュールを止めた（koyomi resume で戻す）" } else { "スケジュールを戻した" }.to_owned())
    }

    /// Whether the engine could have known the time `planned` of `def`: its file was not written
    /// after that time. Only a net that is not confirmed is asked (a confirmed one was approved
    /// as it stands, so its times were known).
    fn knew(&self, def: &Def, planned: NaiveDateTime) -> bool {
        if def.standing == Standing::Confirmed {
            return true;
        }
        let at = def.file.split('/').fold(self.paths.jobnets.clone(), |p, c| p.join(c));
        let Ok(modified) = std::fs::metadata(at).and_then(|m| m.modified()) else { return true };
        let modified: chrono::DateTime<Local> = modified.into();
        modified.naive_local() <= planned
    }

    /// The warnings that bear on the net in `file`: those about the shared folders and files, and
    /// those about this net's own file, but not those about another net's file.
    fn warnings_for(&self, file: &str) -> Vec<String> {
        warnings_about(&self.paths, &self.warnings, &self.defs, file)
    }

    fn approve(&mut self, id: &str, sha256: &str) -> Result<String, String> {
        let def = self.defs.iter().find(|d| d.id == id).ok_or_else(|| format!("定義がない: {id}"))?;
        let own = self.warnings_for(&def.file);
        if !own.is_empty() {
            return Err(format!("承認できない: {}", own.join(" / ")));
        }
        if def.standing == Standing::Invalid {
            return Err(format!("定義の誤りがあり、承認できない: {}", def.file));
        }
        if def.sha256 != sha256 {
            return Err(format!("{} が確かめたあとに変わった。もう一度確かめてください", def.file));
        }
        let mut next = self.store.clone();
        next.approve(id, &def.sha256, &def.text);
        // Written before it counts, and whole: a crash leaves the old approvals.
        if let Some(dir) = self.paths.approved.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        ito_common::write_atomic(&self.paths.approved, next.to_toml())?;
        self.store = next;
        self.rescan();
        Ok(format!("確認した: {id}"))
    }

    /// The editor's save (see [`Request::Save`]): checked as a file would be, the old
    /// file kept under `versions/`, written whole, and approved at once.
    fn save(&mut self, id: &str, text: &str, base_sha256: Option<&str>, force: bool) -> Result<String, String> {
        self.write_def(id, text, base_sha256, force, true)?;
        Ok(format!("保存した: {id}"))
    }

    /// A definition sent over HTTP (see [`Request::Propose`]): written as a save is, but left
    /// unconfirmed for this PC's window to confirm.
    fn propose(&mut self, id: &str, text: &str, base_sha256: Option<&str>) -> Result<String, String> {
        self.write_def(id, text, base_sha256, false, false)?;
        let left = self.defs.iter().find(|d| d.id == id).is_some_and(|d| d.standing != Standing::Confirmed);
        Ok(if left {
            format!("置いた（未確認。この PC の窓で確認するまで動かない）: {id}")
        } else {
            format!("置いた（確認済みと同じ中身）: {id}")
        })
    }

    /// Checks `text` as a file would be, keeps the old file under `versions/` and writes it
    /// whole; with `approve`, confirms it too.
    fn write_def(&mut self, id: &str, text: &str, base_sha256: Option<&str>, force: bool, approve: bool) -> Result<(), String> {
        if !self.warnings.is_empty() {
            return Err(format!("保存できない: {}", self.warnings.join(" / ")));
        }
        let (group, name) = split_id(id)?;
        let path = self.paths.jobnets.join(group).join(format!("{name}.toml"));
        // The file as it is now, not as the last scan saw it: an edit by hand a moment ago counts.
        let now = std::fs::read(&path).ok().map(|b| approved::hash(&b));
        if !force && now.as_deref() != base_sha256 {
            return Err(format!("ほかで変わっています: {id}"));
        }
        let def = self.checked_def(id, text, "保存できない")?;
        self.keep_version(&path, group, name)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{} を作れない: {e}", shown(dir)))?;
        }
        ito_common::write_atomic(&path, text)?;
        if approve {
            let mut next = self.store.clone();
            next.approve(id, &def.sha256, text);
            if let Some(dir) = self.paths.approved.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            ito_common::write_atomic(&self.paths.approved, next.to_toml())?;
            self.store = next;
        }
        self.rescan();
        Ok(())
    }

    /// `text` as the definition of `id` it would be once written, checked as a file is (with its
    /// group's `_group.toml` and against the other nets); refused, `doing` first, with an error.
    fn checked_def(&self, id: &str, text: &str, doing: &str) -> Result<Def, String> {
        let (group, name) = split_id(id)?;
        let shared = group::load(&self.paths.jobnets.join(group).join("_group.toml"));
        let def = def_of_text(text, group, name, &self.store, shared.as_ref(), &self.calendars);
        let mut all: Vec<Def> = self.defs.iter().filter(|d| d.id != id).cloned().collect();
        all.push(def);
        link(&mut all);
        let def = all.pop().expect("pushed");
        if def.standing == Standing::Invalid {
            let why: Vec<String> = def.problems.iter().map(|p| p.in_file(&def.file)).collect();
            return Err(format!("{doing}:\n{}", why.join("\n")));
        }
        Ok(def)
    }

    /// See [`Request::Stage`].
    fn stage(&mut self, id: &str, text: &str, from: &str) -> Result<String, String> {
        let from = NaiveDate::parse_from_str(from, "%Y-%m-%d").map_err(|_| format!("日付は YYYY-MM-DD で書く: {from}"))?;
        self.stage_on(id, text, from, ito_common::now::now_local().date_naive())
    }

    /// Stages `text` to take over `id` on `from`, approved now, when `today` is before it.
    fn stage_on(&mut self, id: &str, text: &str, from: NaiveDate, today: NaiveDate) -> Result<String, String> {
        if !self.warnings.is_empty() {
            return Err(format!("置けない: {}", self.warnings.join(" / ")));
        }
        if from <= today {
            return Err(format!("{from} は今日か過ぎた日。今日から使うなら、ふつうに保存する"));
        }
        let (group, name) = split_id(id)?;
        let path = self.paths.jobnets.join(group).join(format!("{name}.toml"));
        let live = std::fs::read(&path).map_err(|_| format!("定義がない: {id}（新しいジョブネットは今作って保留にしておく）"))?;
        if live == text.as_bytes() {
            return Err(format!("今の定義と同じ中身: {id}"));
        }
        self.checked_def(id, text, "置けない")?;
        let mut staged = self.staged.clone();
        staged.set(koyomi_core::next::Entry {
            id: id.to_owned(),
            from,
            sha256: approved::hash(text.as_bytes()),
            base: Some(approved::hash(&live)),
            text: text.to_owned(),
        });
        self.save_staged(&staged)?;
        self.staged = staged;
        self.staged_failed.remove(id);
        self.staged_look = None;
        Ok(format!("{from} から使う定義を置いた: {id}（それまでは今の定義で流す）"))
    }

    /// See [`Request::Unstage`].
    fn unstage(&mut self, id: &str) -> Result<String, String> {
        if let Some(e) = &self.staged_bad {
            return Err(e.clone());
        }
        let from = self.staged.get(id).map(|e| e.from).ok_or_else(|| format!("{id} に次の定義は置いていない"))?;
        let mut staged = self.staged.clone();
        staged.forget(id);
        self.save_staged(&staged)?;
        self.staged = staged;
        self.staged_failed.remove(id);
        Ok(format!("{from} から使う定義をやめた: {id}（今の定義のまま流す）"))
    }

    /// `next.toml` can be written: it read.
    fn staged_writable(&self) -> Result<(), String> {
        match &self.staged_bad {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    fn save_staged(&self, staged: &koyomi_core::next::Store) -> Result<(), String> {
        if let Some(dir) = self.paths.next.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        ito_common::write_atomic(&self.paths.next, staged.to_toml())
    }

    /// On a new day (and on the engine's start), the staged definitions due take over before any
    /// time of the day starts; one that could not is tried again every [`NEXT_RETRY`].
    fn tick_next(&mut self) {
        if self.staged.next.is_empty() || self.staged_bad.is_some() {
            return;
        }
        let today = ito_common::now::now_local().date_naive();
        if let Some((day, at)) = self.staged_look {
            if day == today && (self.staged_failed.is_empty() || at.elapsed() < NEXT_RETRY) {
                return;
            }
        }
        self.staged_look = Some((today, Instant::now()));
        self.switch_staged(today);
    }

    /// Each staged definition due on `today` takes over ([`State::switch`]), told and written
    /// down; one that cannot is told once, and stays staged and shown until it does.
    fn switch_staged(&mut self, today: NaiveDate) {
        for e in self.staged.due(today) {
            let to = notify::to(&self.notify, self.notify_to(&e.id));
            match self.switch(&e) {
                Ok(line) => {
                    self.staged_failed.remove(&e.id);
                    crate::say(&format!("koyomi: {line}"), false);
                    self.runner.record_op("switch", &e.id, &line, "");
                    notify::tell(&to, None, "done", "koyomi: 定義を切り替えた", &line);
                }
                Err(why) => {
                    let why = why.lines().collect::<Vec<_>>().join(" ");
                    let line = format!("{} を {} からの定義に切り替えられなかった（今までの定義のまま）: {why}", e.id, e.from);
                    if self.staged_failed.get(&e.id) != Some(&line) {
                        crate::say(&format!("koyomi: {line}"), true);
                        notify::tell(&to, None, "failed", "koyomi: 定義を切り替えられなかった", &line);
                        self.staged_failed.insert(e.id.clone(), line);
                    }
                }
            }
        }
    }

    /// Writes the staged `e` as its net's file (the file there kept under `versions/` first),
    /// confirms it when its text is what was approved, and then drops it from `next.toml`.
    /// What to tell.
    fn switch(&mut self, e: &koyomi_core::next::Entry) -> Result<String, String> {
        if !self.warnings.is_empty() {
            return Err(self.warnings.join(" / "));
        }
        let (group, name) = split_id(&e.id)?;
        let path = self.paths.jobnets.join(group).join(format!("{name}.toml"));
        let live = std::fs::read(&path).map_err(|_| "今の定義のファイルがない".to_owned())?;
        self.checked_def(&e.id, &e.text, "定義の誤り")?;
        let changed = e.base.as_deref().is_some_and(|b| b != approved::hash(&live));
        let approved = e.approved();
        // The file first: a switch cut short is done again, never lost.
        self.keep_version(&path, group, name)?;
        ito_common::write_atomic(&path, &e.text)?;
        if approved {
            let mut store = self.store.clone();
            store.approve(&e.id, &e.sha256, &e.text);
            ito_common::write_atomic(&self.paths.approved, store.to_toml())?;
            self.store = store;
        }
        let mut staged = self.staged.clone();
        staged.forget(&e.id);
        self.save_staged(&staged)?;
        self.staged = staged;
        self.rescan();
        let mut line = format!("{} を {} からの定義にした", e.id, e.from);
        if !approved {
            line.push_str("（置いたあとに手で直されていたので未確認。確かめるまで流さない）");
        }
        if changed {
            line.push_str("。置いたあとに今の定義が直されていた（直したものは版に残した）");
        }
        Ok(line)
    }

    /// Copies the file at `path`, when there is one, to
    /// `versions/<group>/<name>/<YYYYMMDD-HHMMSS>.toml` before it is written over.
    fn keep_version(&self, path: &Path, group: &str, name: &str) -> Result<(), String> {
        self.keep_version_in(path, &self.versions_dir(group, name))
    }

    /// Copies the file at `path`, when there is one, to `dir/<YYYYMMDD-HHMMSS>.toml`.
    fn keep_version_in(&self, path: &Path, dir: &Path) -> Result<(), String> {
        let Ok(old) = std::fs::read(path) else { return Ok(()) };
        std::fs::create_dir_all(dir).map_err(|e| format!("{} を作れない: {e}", shown(dir)))?;
        let stamp = ito_common::now::now_local().format("%Y%m%d-%H%M%S").to_string();
        let mut version = stamp.clone();
        let mut n = 1;
        while dir.join(format!("{version}.toml")).exists() {
            n += 1;
            version = format!("{stamp}-{n}");
        }
        ito_common::write_atomic(&dir.join(format!("{version}.toml")), old)
    }

    /// Brings a bundle in. The plan is answered whatever happens; it is carried
    /// out only when nothing is undecided or wrong and `dry_run` is off. What is
    /// written over is kept under `versions/` first; a net that comes in is
    /// unconfirmed, like a file written by hand.
    fn import(&mut self, text: &str, dry_run: bool, on_conflict: Option<&str>) -> Result<String, String> {
        if !self.warnings.is_empty() {
            return Err(format!("取り込めない: {}", self.warnings.join(" / ")));
        }
        let on_conflict = on_conflict.map(bundle::Conflict::parse).transpose()?;
        // A Task Scheduler task comes as its XML: one net, and notes on what did not come over.
        let (b, notes) = if koyomi_core::task::is_xml(text) {
            koyomi_core::task::bundle(text).map_err(|e| format!("取り込めない: {e}"))?
        } else {
            (bundle::parse(text).map_err(|e| format!("取り込めない: {e}"))?, Vec::new())
        };
        if b.is_empty() {
            return Err("取り込めない: ファイルに何も入っていない".to_owned());
        }
        let current = bundle::Source::read(&self.paths.app);
        let mut p = bundle::plan(&b, &current, on_conflict);
        // The settings must pass what `koyomi check` passes, or nothing is written.
        for item in p.items.iter_mut().filter(|i| i.kind == bundle::Kind::Settings && i.problems.is_empty()) {
            if let Some(e) = crate::notify::settings_problem(&item.new) {
                item.problems.push(format!("koyomi.toml が正しくない: {e}"));
            }
        }
        let mut lines = Vec::new();
        for item in &p.items {
            lines.push(item.line());
            if item.change == Change::Replace {
                if let Some(old) = &item.old {
                    lines.extend(diff_lines(old, &item.new));
                }
            }
            lines.extend(item.problems.iter().map(|p| format!("  問題: {p}")));
        }
        lines.extend(notes);
        lines.extend(p.notes.iter().cloned());
        let mut reply = proto::Plan {
            lines,
            conflicts: p.conflicts(),
            problems: p.problems(),
            undecided: p.undecided,
            writes: p.writes(),
            applied: false,
        };
        if dry_run || p.undecided || p.problems() > 0 || !p.writes() {
            return Ok(reply.to_toml());
        }
        let mut written = 0;
        for item in &p.items {
            let at = match &item.change {
                Change::Add | Change::Replace => item.name.clone(),
                Change::Rename(to) => to.clone(),
                Change::Same | Change::Skip => continue,
            };
            let versions = self.paths.state.join("versions");
            let (path, dir) = match item.kind {
                bundle::Kind::Settings => (self.paths.config.clone(), versions.join("_settings")),
                bundle::Kind::Calendar => (self.paths.calendars.join(format!("{at}.toml")), versions.join("_calendars").join(&at)),
                bundle::Kind::Group => (self.paths.jobnets.join(&at).join("_group.toml"), versions.join("_groups").join(&at)),
                bundle::Kind::Jobnet => {
                    let (group, name) = split_id(&at)?;
                    (self.paths.jobnets.join(group).join(format!("{name}.toml")), self.versions_dir(group, name))
                }
            };
            let done = (|| {
                self.keep_version_in(&path, &dir)?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("{} を作れない: {e}", shown(parent)))?;
                }
                ito_common::write_atomic(&path, item.new.clone())
            })();
            if let Err(e) = done {
                self.rescan();
                return Err(format!("取り込みの途中で止まった（{written} 件は書いた。前の中身は versions/ にある）: {e}"));
            }
            written += 1;
        }
        self.rescan();
        reply.applied = true;
        Ok(reply.to_toml())
    }

    fn versions_dir(&self, group: &str, name: &str) -> PathBuf {
        self.paths.state.join("versions").join(group).join(name)
    }

    /// The versions kept of `id`, newest first.
    fn versions(&self, id: &str) -> Result<String, String> {
        let (group, name) = split_id(id)?;
        let mut versions: Vec<String> = match std::fs::read_dir(self.versions_dir(group, name)) {
            Ok(entries) => entries.flatten().filter_map(|e| e.file_name().to_str()?.strip_suffix(".toml").map(str::to_owned)).collect(),
            Err(_) => Vec::new(),
        };
        versions.sort();
        versions.reverse();
        Ok(Versions { versions }.to_toml())
    }

    /// The text of a kept version (`current`: the file as it is).
    fn version_text(&self, id: &str, version: &str) -> Result<String, String> {
        let (group, name) = split_id(id)?;
        let path = if version == "current" {
            self.paths.jobnets.join(group).join(format!("{name}.toml"))
        } else {
            if version.is_empty() || !version.chars().all(|c| c.is_ascii_digit() || c == '-') {
                return Err(format!("版の名前は 20261030-101500 のような形: {version}"));
            }
            self.versions_dir(group, name).join(format!("{version}.toml"))
        };
        let bytes = std::fs::read(&path).map_err(|_| format!("版がない: {id} の {version}"))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Puts a kept version back as the file (the file as it is is kept first). Not approved:
    /// what comes back is looked at like a file written by hand.
    fn restore(&mut self, id: &str, version: &str) -> Result<String, String> {
        if !self.warnings.is_empty() {
            return Err(format!("戻せない: {}", self.warnings.join(" / ")));
        }
        let (group, name) = split_id(id)?;
        if version.is_empty() || !version.chars().all(|c| c.is_ascii_digit() || c == '-') {
            return Err(format!("版の名前は 20261030-101500 のような形: {version}"));
        }
        let from = self.versions_dir(group, name).join(format!("{version}.toml"));
        let text = std::fs::read(&from).map_err(|_| format!("版がない: {id} の {version}"))?;
        let path = self.paths.jobnets.join(group).join(format!("{name}.toml"));
        self.keep_version(&path, group, name)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{} を作れない: {e}", shown(dir)))?;
        }
        ito_common::write_atomic(&path, text)?;
        self.rescan();
        Ok(format!("{version} に戻した: {id}（未確認。確かめてから流す）"))
    }

    /// The net `id` as a file there is, its path, and that it does not run now: what a rename,
    /// a copy and a delete start from.
    fn net_file(&self, id: &str, doing: &str) -> Result<(PathBuf, String), String> {
        if !self.warnings.is_empty() {
            return Err(format!("{doing}: {}", self.warnings.join(" / ")));
        }
        let (group, name) = split_id(id)?;
        let path = self.paths.jobnets.join(group).join(format!("{name}.toml"));
        let bytes = std::fs::read(&path).map_err(|_| format!("定義がない: {id}"))?;
        Ok((path, String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// Why `to` cannot be a new net's id (not one, or there is one), if it cannot.
    fn free_id(&self, to: &str) -> Result<PathBuf, String> {
        let (group, name) = split_id(to)?;
        let path = self.paths.jobnets.join(group).join(format!("{name}.toml"));
        if path.exists() || self.defs.iter().any(|d| d.id == to) {
            return Err(format!("もうある: {to}"));
        }
        Ok(path)
    }

    /// Refused while a run of `id` is not over.
    fn not_running(&self, id: &str) -> Result<(), String> {
        if self.runner.runs().iter().any(|r| r.jobnet == id && !r.is_over()) {
            return Err(format!("実行中のため変えられない: {id}"));
        }
        Ok(())
    }

    /// The lines of `koyomi uses` for the nets that name `id`, and how to go on.
    fn named_by(&self, id: &str, by: &[String], how: &str) -> String {
        let mut lines = vec![format!("{} 個のジョブネットが {id} を使っている（{how}）:", by.len())];
        lines.extend(
            koyomi_core::uses::find(&self.defs, id)
                .into_iter()
                .filter(|l| !l.starts_with("calendar") && !l.starts_with("program"))
                .map(|l| format!("  {l}")),
        );
        lines.join("\n")
    }

    /// See [`Request::Rename`].
    fn rename(&mut self, id: &str, to: &str, fix: bool) -> Result<String, String> {
        let (from_path, text) = self.net_file(id, "名前を変えられない")?;
        let to_path = self.free_id(to)?;
        self.not_running(id)?;
        self.held_writable()?;
        self.staged_writable()?;
        let by = koyomi_core::rename::referrers(&self.defs, id);
        if !by.is_empty() && !fix {
            return Err(self.named_by(id, &by, "--fix でいっしょに直す"));
        }
        // Every file is worked out before one is written.
        let confirmed = |def_id: &str| self.defs.iter().any(|d| d.id == def_id && d.standing == Standing::Confirmed);
        let (from_group, from_name) = split_id(id)?;
        let (to_group, to_name) = split_id(to)?;
        let own = koyomi_core::rename::retarget(&text, id, to).unwrap_or_else(|_| text.clone());
        let own = koyomi_core::rename::retitle(&own, from_name, to_name);
        let mut others = Vec::new();
        // The files rewritten, by net: (before, after), for what was staged on them.
        let mut rewritten = BTreeMap::from([(id.to_owned(), (approved::hash(text.as_bytes()), approved::hash(own.as_bytes())))]);
        for r in &by {
            let (path, old) = self.net_file(r, "名前を変えられない")?;
            let new = koyomi_core::rename::retarget(&old, id, to).map_err(|e| format!("{r} の参照を直せない（{e}）。手で直してから"))?;
            rewritten.insert(r.clone(), (approved::hash(old.as_bytes()), approved::hash(new.as_bytes())));
            others.push((r.clone(), path, new, confirmed(r)));
        }
        let staged = self.staged_renamed(id, to, &rewritten);
        let mut next = self.store.clone();
        let was_confirmed = confirmed(id);
        if let Some(dir) = to_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{} を作れない: {e}", shown(dir)))?;
        }
        ito_common::write_atomic(&to_path, &own)?;
        next.forget(id);
        if was_confirmed {
            next.approve(to, &approved::hash(own.as_bytes()), &own);
        }
        for (r, path, new, was) in &others {
            let (g, n) = split_id(r)?;
            self.keep_version(path, g, n)?;
            ito_common::write_atomic(path, new)?;
            if *was {
                next.approve(r, &approved::hash(new.as_bytes()), new);
            }
        }
        if let Some(dir) = self.paths.approved.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        ito_common::write_atomic(&self.paths.approved, next.to_toml())?;
        self.store = next;
        // The file as it was goes with its versions, under the new name.
        self.keep_version(&from_path, from_group, from_name)?;
        self.move_versions(&self.versions_dir(from_group, from_name), &self.versions_dir(to_group, to_name))?;
        std::fs::remove_file(&from_path).map_err(|e| format!("{} を消せない: {e}", shown(&from_path)))?;
        if self.held.carry(id, Some(to)) {
            self.save_held()?;
        }
        if staged != self.staged {
            self.save_staged(&staged)?;
            self.staged = staged;
            self.staged_failed.remove(id);
        }
        self.rescan();
        let mut said = format!("名前を変えた: {id} → {to}");
        if !others.is_empty() {
            said.push_str(&format!("（参照も直した: {}）", by.join(", ")));
        }
        Ok(said)
    }

    /// What is staged once `id` is renamed `to`: its own next definition goes with it under the
    /// new name, and every staged text that names it names `to` (as far as it can be rewritten).
    /// A text approved stays approved; a net's file rewritten by the rename (`rewritten`: before
    /// and after) is not told as changed by hand at the switch.
    fn staged_renamed(&self, id: &str, to: &str, rewritten: &BTreeMap<String, (String, String)>) -> koyomi_core::next::Store {
        let (from_name, to_name) = (split_id(id).map(|p| p.1).unwrap_or_default(), split_id(to).map(|p| p.1).unwrap_or_default());
        let mut out = koyomi_core::next::Store::default();
        for mut e in self.staged.next.iter().cloned() {
            let was = e.approved();
            let mut text = koyomi_core::rename::retarget(&e.text, id, to).unwrap_or_else(|_| e.text.clone());
            if let Some((before, after)) = rewritten.get(&e.id) {
                if e.base.as_ref() == Some(before) {
                    e.base = Some(after.clone());
                }
            }
            if e.id == id {
                text = koyomi_core::rename::retitle(&text, from_name, to_name);
                e.id = to.to_owned();
            }
            if text != e.text {
                e.text = text;
                if was {
                    e.sha256 = approved::hash(e.text.as_bytes());
                }
            }
            out.set(e);
        }
        out
    }

    /// Moves every kept version in `from` to `to` (one kept there already stays: same names get `-2`).
    fn move_versions(&self, from: &Path, to: &Path) -> Result<(), String> {
        let Ok(entries) = std::fs::read_dir(from) else { return Ok(()) };
        std::fs::create_dir_all(to).map_err(|e| format!("{} を作れない: {e}", shown(to)))?;
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let stem = name.strip_suffix(".toml").unwrap_or(&name).to_owned();
            let mut target = to.join(&name);
            let mut n = 1;
            while target.exists() {
                n += 1;
                target = to.join(format!("{stem}-{n}.toml"));
            }
            std::fs::rename(e.path(), &target).map_err(|err| format!("{} を移せない: {err}", shown(&e.path())))?;
        }
        let _ = std::fs::remove_dir(from);
        Ok(())
    }

    /// See [`Request::Copy`].
    fn copy(&mut self, id: &str, to: &str, title: Option<&str>) -> Result<String, String> {
        let (_, text) = self.net_file(id, "複製できない")?;
        let to_path = self.free_id(to)?;
        let text = match title {
            Some(title) => koyomi_core::rename::set_title(&text, title),
            None => koyomi_core::rename::retitle(&text, split_id(id)?.1, split_id(to)?.1),
        };
        if let Some(dir) = to_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{} を作れない: {e}", shown(dir)))?;
        }
        ito_common::write_atomic(&to_path, text)?;
        self.rescan();
        Ok(format!("複製した: {id} → {to}（未確認。確かめてから流す）"))
    }

    /// See [`Request::Replace`].
    fn replace(&mut self, from: &str, to: &str, apply: bool) -> Result<String, String> {
        if from.is_empty() {
            return Err("探す文字が空".to_owned());
        }
        if from == to {
            return Err("探す文字と直す文字が同じ".to_owned());
        }
        let mut ids: Vec<String> = self.defs.iter().map(|d| d.id.clone()).collect();
        ids.sort();
        // Every file is worked out before one is written.
        let mut found = Vec::new();
        for id in ids {
            let (path, text) = self.net_file(&id, "置き換えられない")?;
            let hits = koyomi_core::replace::hits(&text, from, to);
            if !hits.is_empty() {
                found.push((id, path, text, hits));
            }
        }
        if found.is_empty() {
            return Ok(format!("見つからない: {from}"));
        }
        let lines = found.iter().map(|f| f.3.len()).sum::<usize>();
        if !apply {
            let mut out = Vec::new();
            for (id, _, _, hits) in &found {
                out.push(id.clone());
                for h in hits {
                    out.push(format!("  {}: - {}", h.line, h.before.trim()));
                    out.push(format!("  {}: + {}", h.line, h.after.trim()));
                }
            }
            out.push(format!("{} 個のジョブネットの {lines} 行（--apply で置き換える。直したジョブネットは未確認に戻る）", found.len()));
            return Ok(out.join(
                "
",
            ));
        }
        self.held_writable()?;
        let mut next = Vec::new();
        for (id, path, text, _) in &found {
            self.not_running(id)?;
            let new = koyomi_core::replace::apply(text, from, to).map_err(|e| format!("{id} が読めなくなる（{e}）。何も書かなかった"))?;
            next.push((id.clone(), path.clone(), new));
        }
        let mut written: Vec<&str> = Vec::new();
        for (id, path, new) in &next {
            let (g, n) = split_id(id)?;
            let put = self.keep_version(path, g, n).and_then(|()| ito_common::write_atomic(path, new));
            if let Err(e) = put {
                self.rescan();
                let done = if written.is_empty() {
                    "どれも書いていない".to_owned()
                } else {
                    format!("書いたのは {}", written.join(", "))
                };
                return Err(format!("{id} を書けない: {e}（{done}）"));
            }
            written.push(id);
        }
        self.rescan();
        Ok(format!("{} 個のジョブネットの {lines} 行を置き換えた（未確認。直す前の形は版に残した）", next.len()))
    }

    /// See [`Request::Delete`].
    fn delete(&mut self, id: &str) -> Result<String, String> {
        let (path, _) = self.net_file(id, "消せない")?;
        self.not_running(id)?;
        self.held_writable()?;
        self.staged_writable()?;
        let by = koyomi_core::rename::referrers(&self.defs, id);
        if !by.is_empty() {
            return Err(self.named_by(id, &by, "先にそちらを直す"));
        }
        let (group, name) = split_id(id)?;
        self.keep_version(&path, group, name)?;
        std::fs::remove_file(&path).map_err(|e| format!("{} を消せない: {e}", shown(&path)))?;
        let mut next = self.store.clone();
        if next.forget(id) {
            ito_common::write_atomic(&self.paths.approved, next.to_toml())?;
            self.store = next;
        }
        if self.held.carry(id, None) {
            self.save_held()?;
        }
        let mut staged = self.staged.clone();
        if staged.forget(id) {
            self.save_staged(&staged)?;
            self.staged = staged;
        }
        self.staged_failed.remove(id);
        self.rescan();
        Ok(format!("消した: {id}（前の形は版に残した。koyomi versions {id} と koyomi restore で戻せる）"))
    }

    fn answer(&mut self, request: &Request) -> Result<String, String> {
        let (request, by) = match request {
            Request::Over { by, request } if over_http(request) => (&**request, by.as_str()),
            Request::Over { .. } => return Err("HTTP からはできない".to_owned()),
            r => (r, ""),
        };
        let out = self.dispatch(request);
        if let Ok(text) = &out {
            if let Some((op, target)) = by_hand(request) {
                let detail: String = text.lines().next().unwrap_or_default().chars().take(120).collect();
                self.runner.record_op(op, &target, &detail, by);
            }
        }
        out
    }

    fn dispatch(&mut self, request: &Request) -> Result<String, String> {
        match request {
            Request::Status => Ok(self.status()),
            Request::Stop => Ok("止める".to_owned()),
            Request::List => Ok(self.snapshot().to_toml()),
            Request::Show { id } => {
                let def = self.defs.iter().find(|d| &d.id == id).ok_or_else(|| format!("定義がない: {id}"))?;
                toml::to_string(def).map_err(|e| e.to_string())
            }
            Request::Approve { id, sha256 } => self.approve(id, sha256),
            Request::Run { id, date, from, dry, params } => self.start(id, date.as_deref(), from.as_deref(), *dry, params),
            Request::Rerun { run, from, next } => self.rerun(run, from.as_deref(), *next),
            Request::Kill { run, job } => self.runner.kill(run, job.as_deref()),
            Request::Runs => Ok(Runs { runs: self.runner.today() }.to_toml()),
            Request::History { from, to } => {
                let day = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| format!("日付は YYYY-MM-DD で書く: {s}"));
                Ok(Runs { runs: self.runner.history(day(from)?, day(to)?)? }.to_toml())
            }
            Request::Log { run, job } => self.runner.log_tail(run, job),
            Request::Hold { id, job: None } => self.hold(id),
            Request::Release { id, job: None } => self.release(id),
            Request::Hold { id, job: Some(job) } => self.hold_job(id, job),
            Request::Release { id, job: Some(job) } => self.release_job(id, job),
            Request::Skip { run, job } => self.runner.skip(run, job),
            Request::Answer { run, job, yes } => self.runner.answer(run, job, *yes),
            Request::Shift { id, from, to } => self.shift(id, from, to.as_deref()),
            Request::SkipNext { id, cancel } => self.skip_next(id, *cancel),
            Request::Pause => self.pause(true),
            Request::Resume => self.pause(false),
            Request::Standby { give_back: true } => self.standby.give_back(),
            Request::Standby { give_back: false } => self
                .standby
                .status()
                .or_else(|| self.primary.line())
                .ok_or_else(|| "koyomi.toml に [standby] も [primary] もない".to_owned()),
            Request::Handover { release } => self.handover(*release),
            Request::Save { id, text, base_sha256, force } => self.save(id, text, base_sha256.as_deref(), *force),
            Request::Versions { id } => self.versions(id),
            Request::VersionText { id, version } => self.version_text(id, version),
            Request::Restore { id, version } => self.restore(id, version),
            Request::Rename { id, to, fix } => self.rename(id, to, *fix),
            Request::Copy { id, to, title } => self.copy(id, to, title.as_deref()),
            Request::Replace { from, to, apply } => self.replace(from, to, *apply),
            Request::Delete { id } => self.delete(id),
            Request::Import { text, dry_run, on_conflict } => self.import(text, *dry_run, on_conflict.as_deref()),
            Request::Uses { target } => Ok(koyomi_core::uses::find(&self.defs, target).join(
                "
",
            )),
            Request::Ops { from, to } => {
                let day = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| format!("日付は YYYY-MM-DD で書く: {s}"));
                Ok(self.runner.ops(day(from)?, day(to)?)?.join(
                    "
",
                ))
            }
            Request::Tray { since } => Ok(self.tray_view(*since).to_toml()),
            Request::Propose { id, text, base_sha256 } => self.propose(id, text, base_sha256.as_deref()),
            Request::Stage { id, text, from } => self.stage(id, text, from),
            Request::Unstage { id } => self.unstage(id),
            Request::Over { .. } => Err("HTTP からはできない".to_owned()),
        }
    }
}

/// What may come over HTTP: what its paths ask for. Not an approval or a save, which only this
/// PC's window gives (DECISIONS「実行するコマンドの安全」の初回の確認); a definition comes as
/// a proposal, left unconfirmed.
fn over_http(request: &Request) -> bool {
    matches!(
        request,
        Request::Status
            | Request::List
            | Request::Runs
            | Request::History { .. }
            | Request::Log { .. }
            | Request::Run { .. }
            | Request::Kill { .. }
            | Request::Rerun { .. }
            | Request::Skip { .. }
            | Request::Hold { .. }
            | Request::Release { .. }
            | Request::SkipNext { .. }
            | Request::Show { .. }
            | Request::Propose { .. }
            | Request::Handover { .. }
    )
}

/// The name and target a request is written down under when it succeeds: what a person did by hand.
fn by_hand(request: &Request) -> Option<(&'static str, String)> {
    Some(match request {
        Request::Approve { id, .. } => ("approve", id.clone()),
        Request::Run { id, dry: false, .. } => ("run", id.clone()),
        Request::Rerun { run, .. } => ("rerun", run.clone()),
        Request::Kill { run, job } => ("kill", job.as_ref().map_or(run.clone(), |j| format!("{run} {j}"))),
        Request::Hold { id, job } => ("hold", job.as_ref().map_or(id.clone(), |j| format!("{id} {j}"))),
        Request::Release { id, job } => ("release", job.as_ref().map_or(id.clone(), |j| format!("{id} {j}"))),
        Request::Skip { run, job } => ("skip", format!("{run} {job}")),
        Request::Answer { run, job, yes } => (if *yes { "approve-step" } else { "reject-step" }, format!("{run} {job}")),
        Request::Shift { id, .. } => ("shift", id.clone()),
        Request::SkipNext { id, cancel } => (if *cancel { "unskip-next" } else { "skip-next" }, id.clone()),
        Request::Pause => ("pause", String::new()),
        Request::Resume => ("resume", String::new()),
        Request::Standby { give_back: true } => ("giveback", String::new()),
        Request::Handover { release: true } => ("handover-release", String::new()),
        Request::Save { id, .. } => ("save", id.clone()),
        Request::Propose { id, .. } => ("propose", id.clone()),
        Request::Stage { id, from, .. } => ("stage", format!("{id} {from}")),
        Request::Unstage { id } => ("unstage", id.clone()),
        Request::Restore { id, .. } => ("restore", id.clone()),
        Request::Rename { id, to, .. } => ("rename", format!("{id} {to}")),
        Request::Copy { id, to, .. } => ("copy", format!("{id} {to}")),
        Request::Replace { from, to, apply: true } => ("replace", format!("{from:?} → {to:?}")),
        Request::Delete { id } => ("delete", id.clone()),
        Request::Import { dry_run: false, .. } => ("import", String::new()),
        _ => return None,
    })
}

/// The difference between `old` and `new` as lines under a plan's item,
/// `  +` added, `  -` removed, the rest for context; long ones are cut.
fn diff_lines(old: &str, new: &str) -> Vec<String> {
    const MOST: usize = 40;
    let items = ito_diff::diff(old, new, 2);
    let mut out: Vec<String> = items
        .iter()
        .map(|i| match i {
            ito_diff::Item::Line(l) => match l.kind {
                ito_diff::Kind::Added => format!("  + {}", l.text),
                ito_diff::Kind::Removed => format!("  - {}", l.text),
                ito_diff::Kind::Same => format!("    {}", l.text),
            },
            ito_diff::Item::Folded(n) => format!("  … {n} 行"),
        })
        .collect();
    if out.len() > MOST {
        let rest = out.len() - MOST;
        out.truncate(MOST);
        out.push(format!("  … あと {rest} 行"));
    }
    out
}

/// `グループ/名前` as its two parts, or why it is not one.
fn split_id(id: &str) -> Result<(&str, &str), String> {
    if !koyomi_core::jobnet::is_id(id) {
        return Err(format!("グループ/名前 の形で書く: {id}"));
    }
    id.split_once('/').ok_or_else(|| format!("グループ/名前 の形で書く: {id}"))
}

/// How many days before the holiday table ends the window's band and `koyomi check` start to say so.
pub const CAUTION_DAYS: i64 = 90;

/// How often everything is looked at again even when the watcher is silent
/// (an access list changes without a file event).
const SWEEP: Duration = Duration::from_secs(5);
/// How long after the last file event the engine waits before it reads (an editor writes in bursts).
/// How long the window's band tells of the time the engine was not running.
const OUTAGE_SHOWN: Duration = Duration::from_secs(3600);
const DEBOUNCE: Duration = Duration::from_millis(200);
/// How often the power settings are read and the wake timer looked at.
const POWER_LOOK: Duration = Duration::from_secs(60);
/// How long before a `wake = true` net's time the PC is woken.
const WAKE_LEAD: chrono::Duration = chrono::Duration::minutes(2);

/// A wake timer set (or not) for a net's time.
struct Wake {
    /// The net's time it wakes the PC for (the timer is `WAKE_LEAD` before).
    start: NaiveDateTime,
    id: String,
    set: Result<ito_power::WakeHandle, ito_power::WhyNot>,
}

impl Wake {
    /// From the timer to a minute after the time: the PC is kept awake for the net.
    fn waking(&self, now: NaiveDateTime) -> bool {
        now >= self.start - WAKE_LEAD && now <= self.start + chrono::Duration::minutes(1)
    }
}

/// The soonest time after `now` of a confirmed `wake = true` net not on hold, and the net.
fn next_wake(defs: &[Def], calendars: &Calendars, held: &Held, stops: &[Blackout], now: NaiveDateTime) -> Option<(NaiveDateTime, String)> {
    next_due(defs, calendars, held, stops, now, true)
}

/// The soonest time of a confirmed net that is not held (of a `wake = true` one when `wake`), with its id.
fn next_due(
    defs: &[Def],
    calendars: &Calendars,
    held: &Held,
    stops: &[Blackout],
    now: NaiveDateTime,
    wake: bool,
) -> Option<(NaiveDateTime, String)> {
    defs.iter()
        .filter(|d| d.standing == Standing::Confirmed && !held.is_held(&d.id))
        .filter_map(|d| {
            let net = d.net.as_ref().filter(|n| n.wake || !wake)?;
            let cal = d.calendar.as_deref().and_then(|c| calendars.get(c));
            // A time in a planned stop does not start, so it neither wakes the PC nor keeps it up.
            koyomi_core::blackout::outside(stops, now, 1, |after, k| {
                koyomi_core::quiet::next_starts(&net.when, &net.quiet, net.on_quiet, after, k, cal)
            })
            .into_iter()
            .next()
            .map(|at| (at, d.id.clone()))
        })
        .min()
}

/// The `power.why_*` key's tail for why the PC cannot be woken; `None`: nothing to tell (the time is passing now).
fn why_key(why: ito_power::WhyNot) -> Option<&'static str> {
    use ito_power::WhyNot;
    match why {
        WhyNot::TimersDisabled => Some("timers_off"),
        WhyNot::OnBattery => Some("battery"),
        WhyNot::Unsupported => Some("unsupported"),
        WhyNot::Failed(_) => Some("failed"),
        WhyNot::InThePast => None,
    }
}

/// Runs until `koyomi engine stop`. The exit code: 0 after a stop, 1 when another engine is already up.
/// Without `own_tray` (`koyomi engine --no-tray`) there is no icon here, and the
/// system's notifications and the sounds are kept for `koyomi tray` to show.
pub fn run(own_tray: bool) -> i32 {
    let paths = match Paths::find() {
        Ok(p) => p,
        Err(e) => {
            crate::say(&format!("koyomi: {e}"), true);
            return 1;
        }
    };
    let door = proto::door();
    if ask(&door, &Request::Status).is_ok() {
        crate::say("koyomi: もう動いている", true);
        return 1;
    }
    // The lock the OS drops with the process, so two engines started together cannot both pass.
    let _single = match ito_common::single::acquire("koyomi", "engine") {
        Ok(g) => g,
        Err(held) => {
            crate::say(&format!("koyomi: もう動いている ({held})"), true);
            return 1;
        }
    };
    let _ = std::fs::create_dir_all(&paths.jobnets);
    let _ = std::fs::create_dir_all(&paths.calendars);
    let rx = proto::door().listen(|| {});

    // `listen` quietly waits when another engine holds the door; ask until an engine answers, and
    // leave if it is not this one. Two engines started together are told apart here.
    let (mine_tx, mine_rx) = mpsc::channel::<bool>();
    std::thread::spawn(move || {
        for _ in 0..60 {
            if let Ok(text) = ask(&proto::door(), &Request::Status) {
                let _ = mine_tx.send(text.contains(&format!("pid {}:", std::process::id())));
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = mine_tx.send(false);
    });

    let dirty = Arc::new(AtomicBool::new(true));
    let watching = Arc::new(AtomicBool::new(true));
    let mut watcher = {
        let (dirty, watching) = (dirty.clone(), watching.clone());
        ito_watch::Watcher::start(move |ev| match ev {
            ito_watch::Event::Watching(on) => watching.store(on, Ordering::Relaxed),
            _ => dirty.store(true, Ordering::Relaxed),
        })
    };
    watcher.watch(vec![paths.jobnets.clone(), paths.calendars.clone()]);

    let mut state = State::new(paths);
    let mut tray = if own_tray { tray::Tray::start(&state.words) } else { None };
    if !own_tray {
        notify::hand_to_tray();
    }
    let mut last_scan = Instant::now();
    let mut changed_at: Option<Instant> = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ask { request, reply, expired }) => {
                let stop = matches!(request, Request::Stop);
                let out = state.answer(&request);
                if !expired.load(Ordering::Relaxed) {
                    let _ = reply.send(out);
                }
                if stop {
                    state.shut_down();
                    // The reply travels on the connection's thread.
                    std::thread::sleep(Duration::from_millis(150));
                    return 0;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // The listener is gone: end the way Stop does, so no job's process outlives us.
                state.shut_down();
                return 1;
            }
        }
        state.runner.tick();
        // First of all: a definition staged for today takes over before any time of the day starts.
        state.tick_next();
        // Before the schedule: a time that waited out the quiet hours goes ahead of one due at their end.
        state.tick_quiet();
        state.tick_schedule();
        state.tick_events();
        state.tick_wait();
        state.tick_shift();
        state.tick_retain();
        state.tick_backup();
        state.alive.tick();
        state.ping.tick(state.ping_cfg.as_ref(), state.ping_plain_ok);
        state.http.tick(state.http_cfg.as_ref());
        state.tick_standby();
        state.tick_primary();
        state.tick_wake();
        state.tick_awake();
        state.tick_late();
        state.pump();
        tray::follow_clicks();
        if let Some(tr) = tray.as_mut() {
            for want in tr.take() {
                match want {
                    tray::Want::Pause(on) => {
                        let _ = state.answer(if on { &Request::Pause } else { &Request::Resume });
                    }
                    // The menu greys 終了 while a job's process runs; a choice made just before one started waits.
                    tray::Want::Quit if state.runner.busy() == 0 => {
                        state.shut_down();
                        return 0;
                    }
                    tray::Want::Quit => {}
                }
            }
            if tr.due() {
                let look = state.tray_look();
                tr.show(&state.words, look);
            }
        }
        if let Ok(false) = mine_rx.try_recv() {
            crate::say("koyomi: もう動いている", true);
            return 1;
        }
        if dirty.swap(false, Ordering::Relaxed) {
            changed_at = Some(Instant::now());
        }
        let quiet = changed_at.is_some_and(|t| t.elapsed() >= DEBOUNCE);
        if quiet || last_scan.elapsed() >= SWEEP {
            changed_at = None;
            last_scan = Instant::now();
            state.rescan();
        }
    }
}

/// The nets with a `[[when]] after_jobnet` naming `prev`'s net whose `on`
/// takes the way `prev` ended, each with that `[[when]]`.
fn followers(defs: &[Def], prev: &Run) -> Vec<(Def, when::When)> {
    let mut out = Vec::new();
    for d in defs {
        let Some(net) = &d.net else { continue };
        for w in &net.when {
            if let Kind::AfterJobnet { jobnet, on } = &w.kind {
                if *jobnet == prev.jobnet && on.lets(prev.state) {
                    out.push((d.clone(), w.clone()));
                }
            }
        }
    }
    out
}

/// Whether the net `id` has ended its run of the business day `day` and none is going.
fn wait_over(runs: &[Run], id: &str, day: NaiveDate) -> bool {
    let day = day.format("%Y-%m-%d").to_string();
    let mine = || runs.iter().filter(|r| r.jobnet == id && r.date == day);
    mine().any(|r| r.is_over()) && mine().all(|r| r.is_over())
}

/// The run of `id` that already started after another net on the business day `date`, if any.
fn followed<'a>(runs: &'a [Run], id: &str, date: &str) -> Option<&'a Run> {
    runs.iter().find(|r| r.jobnet == id && r.how == How::After && r.date == date)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// over HTTP only what its paths ask for goes through; never an approval or a save.
    #[test]
    fn only_the_http_paths_come_over_http() {
        assert!(over_http(&Request::Status) && over_http(&Request::Runs));
        assert!(over_http(&Request::Run { id: "g/a".into(), date: None, from: None, dry: false, params: Default::default() }));
        assert!(over_http(&Request::Kill { run: "20261031-090000-01".into(), job: None }));
        assert!(over_http(&Request::Rerun { run: "20261031-090000-01".into(), from: None, next: None }));
        assert!(over_http(&Request::Hold { id: "g/a".into(), job: None }));
        assert!(over_http(&Request::SkipNext { id: "g/a".into(), cancel: false }));
        assert!(!over_http(&Request::Approve { id: "g/a".into(), sha256: String::new() }));
        assert!(!over_http(&Request::Stop) && !over_http(&Request::Pause));
        assert!(!over_http(&Request::Delete { id: "g/a".into() }));
        assert!(!over_http(&Request::Over { by: "x".into(), request: Box::new(Request::Status) }));
    }

    fn def_with(id: &str, text: &str, standing: Standing) -> Def {
        let (group, name) = id.split_once('/').unwrap();
        Def {
            id: id.to_owned(),
            group: group.to_owned(),
            name: name.to_owned(),
            title: None,
            file: format!("{id}.toml"),
            standing,
            problems: Vec::new(),
            net: Some(koyomi_core::jobnet::parse(text).unwrap()),
            sha256: String::new(),
            text: text.to_owned(),
            previous: None,
            calendar: None,
        }
    }

    /// the PC is woken for the soonest time of a confirmed `wake = true` net that is not held, and kept awake to it.
    #[test]
    fn the_wake_timer_is_for_the_soonest_net_that_asks() {
        let net = |at: &str, wake: bool| {
            format!(
                "format = 1
wake = {wake}
[[when]]
every = \"day\"
at = [\"{at}\"]
[[job]]
id = \"a\"
run = [\"x\"]
"
            )
        };
        let now = NaiveDateTime::parse_from_str("2026-10-30 09:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let mut defs = vec![
            def_with("夜/早い", &net("01:00", false), Standing::Confirmed),
            def_with("夜/未確認", &net("01:30", true), Standing::Unconfirmed),
            def_with("夜/遅い", &net("03:00", true), Standing::Confirmed),
            def_with("夜/中", &net("02:00", true), Standing::Confirmed),
        ];
        let cals = Calendars::default();
        let mut held = Held::default();
        let at = |s: &str| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        assert_eq!(next_wake(&defs, &cals, &held, &[], now), Some((at("2026-10-31 02:00"), "夜/中".to_owned())));
        // A time in a planned stop does not wake the PC: the next one past it does.
        let stop = [Blackout { from: "2026-10-31 01:45".into(), to: "2026-10-31 02:30".into(), why: "電源工事".into() }];
        assert_eq!(next_wake(&defs, &cals, &held, &stop, now), Some((at("2026-10-31 03:00"), "夜/遅い".to_owned())));
        held.held.push("夜/中".to_owned());
        assert_eq!(next_wake(&defs, &cals, &held, &[], now), Some((at("2026-10-31 03:00"), "夜/遅い".to_owned())));
        defs.truncate(2);
        assert_eq!(next_wake(&defs, &cals, &held, &[], now), None);
        let w = Wake { start: at("2026-10-31 02:00"), id: String::new(), set: Err(ito_power::WhyNot::OnBattery) };
        assert!(!w.waking(at("2026-10-31 01:57")));
        assert!(w.waking(at("2026-10-31 01:58")) && w.waking(at("2026-10-31 02:01")));
        assert!(!w.waking(at("2026-10-31 02:02")));
        assert_eq!(why_key(ito_power::WhyNot::OnBattery), Some("battery"));
        assert_eq!(why_key(ito_power::WhyNot::InThePast), None);
    }

    #[test]
    fn a_net_after_another_follows_the_ends_its_on_names_once_a_business_day() {
        let text = "format = 1\n[[when]]\nafter_jobnet = \"経理/売上取込\"\non = \"error\"\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n";
        let net = koyomi_core::jobnet::parse(text).unwrap();
        let def = Def {
            id: "経理/集計".to_owned(),
            group: "経理".to_owned(),
            name: "集計".to_owned(),
            title: None,
            file: "経理/集計.toml".to_owned(),
            standing: Standing::Confirmed,
            problems: Vec::new(),
            net: Some(net.clone()),
            sha256: String::new(),
            text: text.to_owned(),
            previous: None,
            calendar: None,
        };
        let now = ito_common::now::now_local().naive_local();
        let mut prev = Run::new("p1", "経理/売上取込", "売上取込", "2026-10-29", How::Rerun, &net, now);
        prev.state = RunState::Error;
        let f = followers(std::slice::from_ref(&def), &prev);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].0.id, "経理/集計");
        prev.state = RunState::Ok;
        assert!(followers(std::slice::from_ref(&def), &prev).is_empty());
        let mut other = Run::new("p2", "経理/他", "他", "2026-10-29", How::Manual, &net, now);
        other.state = RunState::Error;
        assert!(followers(std::slice::from_ref(&def), &other).is_empty());
        let mut done = Run::new("f1", "経理/集計", "集計", "2026-10-29", How::After, &net, now);
        assert_eq!(followed(std::slice::from_ref(&done), "経理/集計", "2026-10-29").map(|r| r.id.as_str()), Some("f1"));
        assert!(followed(std::slice::from_ref(&done), "経理/集計", "2026-10-30").is_none());
        done.how = How::Manual;
        assert!(followed(std::slice::from_ref(&done), "経理/集計", "2026-10-29").is_none());
    }

    #[test]
    fn a_wait_ends_only_when_every_run_of_the_net_that_day_is_over() {
        let net = koyomi_core::jobnet::parse(
            "format = 1
[[job]]
id = \"a\"
run = [\"x\"]
",
        )
        .unwrap();
        let now = ito_common::now::now_local().naive_local();
        let day: NaiveDate = "2026-10-29".parse().unwrap();
        let mut r = Run::new("p1", "経理/売上取込", "売上取込", "2026-10-29", How::Schedule, &net, now);
        r.state = RunState::Running;
        assert!(!wait_over(std::slice::from_ref(&r), "経理/売上取込", day));
        assert!(!wait_over(&[], "経理/売上取込", day));
        r.state = RunState::Error;
        r.ended = Some(now);
        assert!(wait_over(std::slice::from_ref(&r), "経理/売上取込", day));
        assert!(!wait_over(std::slice::from_ref(&r), "経理/売上取込", day.succ_opt().unwrap()));
        assert!(!wait_over(std::slice::from_ref(&r), "経理/他", day));
    }

    fn temp_paths(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("koyomi-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let app = root.join("app");
        let state = root.join("state");
        std::fs::create_dir_all(app.join("jobnets")).unwrap();
        std::fs::create_dir_all(app.join("calendars")).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        Paths {
            jobnets: app.join("jobnets"),
            calendars: app.join("calendars"),
            config: app.join("koyomi.toml"),
            approved: state.join("approved.toml"),
            held: state.join("held.toml"),
            next: state.join("next.toml"),
            app,
            state,
        }
    }

    fn standing_of(s: &State, id: &str) -> Option<Standing> {
        s.defs.iter().find(|d| d.id == id).map(|d| d.standing)
    }

    /// H1: a job is held and let go through held.toml, which keeps the jobs.
    #[test]
    fn a_job_is_held_and_let_go_through_the_held_file() {
        let paths = temp_paths("hold-job");
        let mut s = State::new(paths.clone());
        let net = "format = 1

[[job]]
id = \"a\"
name = \"取込\"
run = [\"x.exe\"]
";
        assert!(s.save("経理/集計", net, None, false).is_ok());
        assert!(s.hold_job("経理/集計", "nope").unwrap_err().contains("ジョブがない"));
        assert!(s.hold_job("経理/無い", "a").is_err());
        assert!(s.hold_job("経理/集計", "a").unwrap().contains("取込"));
        assert!(s.hold_job("経理/集計", "a").unwrap_err().contains("もう保留している"));
        let back = Held::parse(&std::fs::read_to_string(&paths.held).unwrap()).unwrap();
        assert!(back.is_job_held("経理/集計", "a"));
        assert!(!back.is_held("経理/集計"), "holding a job does not hold the net");
        assert!(s.release_job("経理/集計", "a").is_ok());
        assert!(s.release_job("経理/集計", "a").unwrap_err().contains("保留していない"));
        let back = Held::parse(&std::fs::read_to_string(&paths.held).unwrap()).unwrap();
        assert!(back.jobs.is_empty());
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// 30.4: a difference without a choice is refused and nothing is written.
    /// 30.5: overwrite keeps the old file as a version; the net comes in unconfirmed.
    /// 30.6: rename adds beside, skip leaves; a broken bundle is refused whatever the choice.
    #[test]
    fn an_import_is_planned_decided_kept_and_comes_in_unconfirmed() {
        let paths = temp_paths("import");
        let mut s = State::new(paths.clone());
        let one = "format = 1\nname = \"一\"\n\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";
        let two = "format = 1\nname = \"二\"\n\n[[job]]\nid = \"a\"\nrun = [\"y.exe\"]\n";
        assert!(s.save("経理/集計", one, None, false).is_ok());
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Confirmed));
        let mut b = bundle::Bundle::new();
        b.jobnets.push(bundle::Net { id: "経理/集計".to_owned(), text: two.to_owned() });
        b.jobnets.push(bundle::Net { id: "経理/新規".to_owned(), text: one.to_owned() });
        b.calendars.push(bundle::Named { name: "会社".to_owned(), text: "format = 1\nholidays = \"japan\"\n".to_owned() });
        let text = b.to_toml();
        let plan = |s: &mut State, dry: bool, on: Option<&str>| proto::Plan::parse(&s.import(&text, dry, on).unwrap()).unwrap();
        // A dry run only says. The net that differs makes the plan undecided.
        let p = plan(&mut s, true, None);
        assert!(!p.applied && p.undecided && p.writes && p.conflicts == 1 && p.problems == 0, "{p:?}");
        assert!(p.lines.iter().any(|l| l.contains("経理/集計")) && p.lines.iter().any(|l| l.contains("- name = \"一\"")), "{:?}", p.lines);
        // Without a choice nothing is done, dry run or not.
        let p = plan(&mut s, false, None);
        assert!(!p.applied && p.undecided, "{p:?}");
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/集計.toml")).unwrap(), one);
        assert!(!paths.jobnets.join("経理/新規.toml").exists());
        // Something wrong in the file: refused whatever the choice.
        assert!(s.import("format = 1\n", false, Some("overwrite")).unwrap_err().contains("何も入っていない"));
        assert!(s.import("nope", false, None).unwrap_err().starts_with("取り込めない"));
        let mut broken = bundle::Bundle::new();
        broken.jobnets.push(bundle::Net {
            id: "経理/親".to_owned(),
            text: "format = 1\n[[job]]\nid = \"k\"\nkind = \"jobnet\"\nuse = \"無い/子\"\n".to_owned(),
        });
        let p = proto::Plan::parse(&s.import(&broken.to_toml(), false, Some("overwrite")).unwrap()).unwrap();
        assert!(!p.applied && p.problems == 1, "{p:?}");
        assert!(!paths.jobnets.join("経理/親.toml").exists());
        assert!(s.import(&text, false, Some("sideways")).is_err());
        // Skip: the one that differs stays, the rest comes in.
        let p = plan(&mut s, false, Some("skip"));
        assert!(p.applied, "{p:?}");
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/集計.toml")).unwrap(), one);
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/新規.toml")).unwrap(), one);
        assert!(paths.calendars.join("会社.toml").exists());
        assert_eq!(standing_of(&s, "経理/新規"), Some(Standing::Unconfirmed));
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Confirmed));
        // Overwrite: the old file is kept as a version and the net is unconfirmed again.
        let p = plan(&mut s, false, Some("overwrite"));
        assert!(p.applied, "{p:?}");
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/集計.toml")).unwrap(), two);
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Unconfirmed));
        let versions = Versions::parse(&s.versions("経理/集計").unwrap()).unwrap().versions;
        assert_eq!(versions.len(), 1, "{versions:?}");
        assert_eq!(std::fs::read_to_string(paths.state.join("versions/経理/集計").join(format!("{}.toml", versions[0]))).unwrap(), one);
        // The same again: nothing to write.
        let p = plan(&mut s, false, Some("overwrite"));
        assert!(!p.applied && !p.writes && p.conflicts == 0, "{p:?}");
        // Rename: a different one comes in beside what is there.
        b.jobnets[0].text = one.to_owned();
        let p = proto::Plan::parse(&s.import(&b.to_toml(), false, Some("rename")).unwrap()).unwrap();
        assert!(p.applied, "{p:?}");
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/集計.toml")).unwrap(), two);
        let names: Vec<String> = std::fs::read_dir(paths.jobnets.join("経理"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 3, "{names:?}");
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    #[test]
    fn skip_next_takes_the_next_times_in_turn_and_cancel_takes_them_back() {
        let paths = temp_paths("skipnext");
        let mut s = State::new(paths.clone());
        let net = "format = 1
[[when]]
every = \"day\"
at = [\"02:00\"]

[[job]]
id = \"a\"
run = [\"x.exe\"]
";
        assert!(s.save("経理/夜", net, None, false).is_ok());
        assert!(s.skip_next("経理/無い", false).is_err());
        assert_eq!(s.skip_next("経理/夜", true), Err("経理/夜 は飛ばしていない".to_owned()));
        let first = s.skip_next("経理/夜", false).unwrap();
        let second = s.skip_next("経理/夜", false).unwrap();
        assert!(first.contains("02:00") && second.contains("02:00") && first != second, "{first} / {second}");
        let times: Vec<_> = s.held.skips_of("経理/夜").collect();
        assert_eq!(times.len(), 2);
        assert_eq!(times[1] - times[0], chrono::Duration::days(1));
        assert_eq!(Held::load(&paths.held).unwrap().skips.len(), 2, "kept in held.toml");
        assert_eq!(s.snapshot().skips.len(), 2);
        assert!(s.skip_next("経理/夜", true).is_ok());
        assert!(s.held.skips.is_empty());
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// An import of settings that `koyomi check` refuses writes nothing, even on overwrite.
    #[test]
    fn import_refuses_settings_that_check_refuses() {
        let paths = temp_paths("import-bad");
        let mut s = State::new(paths.clone());
        let bad = "format = 1\n\n[settings]\nkeep_days = 30\n\n[settings.notify]\nwebhooks = [\"secret:x\"]\n";
        let before = std::fs::read_to_string(&paths.config).ok();
        for (dry, over) in [(true, None), (false, Some("overwrite")), (false, None)] {
            let r = s.import(bad, dry, over).unwrap();
            let plan = proto::Plan::parse(&r).unwrap();
            assert!(plan.problems > 0 && !plan.applied, "{r}");
            assert!(plan.lines.iter().any(|l| l.contains("問題: koyomi.toml が正しくない")), "{r}");
        }
        assert_eq!(std::fs::read_to_string(&paths.config).ok(), before, "nothing written");
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// A warning about one net's file stops that net from being approved, not the others.
    #[test]
    fn approve_refuses_only_the_net_with_a_warning() {
        let paths = temp_paths("approve-own");
        let mut s = State::new(paths.clone());
        let net = "format = 1
name = \"n\"

[[job]]
id = \"a\"
run = [\"x.exe\"]
";
        s.propose("経理/甲", net, None).unwrap();
        s.propose("経理/乙", net, None).unwrap();
        let bad = shown(&paths.jobnets.join("経理").join("甲.toml"));
        s.warnings = vec![format!("{bad} は Everyone が書き換えられる")];
        let sha = |s: &State, id: &str| s.defs.iter().find(|d| d.id == id).unwrap().sha256.clone();
        let (a, b) = (sha(&s, "経理/甲"), sha(&s, "経理/乙"));
        let e = s.approve("経理/甲", &a).unwrap_err();
        assert!(e.starts_with("承認できない: ") && e.contains("Everyone"), "{e}");
        // The CLI/window pre-check asks the same question and gives the same answer.
        assert!(!warnings_about(&paths, &s.warnings, &s.defs, "経理/甲.toml").is_empty());
        assert!(warnings_about(&paths, &s.warnings, &s.defs, "経理/乙.toml").is_empty());
        assert_eq!(s.approve("経理/乙", &b), Ok("確認した: 経理/乙".to_owned()));
        assert_eq!(standing_of(&s, "経理/乙"), Some(Standing::Confirmed));
        assert_ne!(standing_of(&s, "経理/甲"), Some(Standing::Confirmed));
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    #[test]
    fn a_save_is_checked_kept_and_confirmed_and_a_restore_is_not() {
        let paths = temp_paths("save");
        let mut s = State::new(paths.clone());
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        let one = "format = 1\nname = \"一\"\n\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";
        let two = "format = 1\nname = \"二\"\n\n[[job]]\nid = \"a\"\nrun = [\"y.exe\"]\n";
        // A new net: nothing to be based on.
        assert_eq!(s.save("経理/集計", one, None, false), Ok("保存した: 経理/集計".to_owned()));
        let file = paths.jobnets.join("経理").join("集計.toml");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), one);
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Confirmed));
        assert_eq!(Versions::parse(&s.versions("経理/集計").unwrap()).unwrap().versions.len(), 0);
        // Saved again on a stale base: refused; on the right base: the old file is kept.
        let hash = approved::hash(one.as_bytes());
        assert_eq!(s.save("経理/集計", two, Some("stale"), false), Err("ほかで変わっています: 経理/集計".to_owned()));
        assert_eq!(s.save("経理/集計", two, None, false), Err("ほかで変わっています: 経理/集計".to_owned()));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), one);
        assert!(s.save("経理/集計", two, Some(&hash), false).is_ok());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), two);
        let versions = Versions::parse(&s.versions("経理/集計").unwrap()).unwrap().versions;
        assert_eq!(versions.len(), 1);
        assert_eq!(std::fs::read_to_string(paths.state.join("versions/経理/集計").join(format!("{}.toml", versions[0]))).unwrap(), one);
        // What does not read is not saved, whatever the base.
        let err = s.save("経理/集計", "format = 1\n[[job]]\nid = \"a\"\n", Some("x"), true).unwrap_err();
        assert!(err.starts_with("保存できない:"), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), two);
        // A loop through a child net is caught against the other nets too.
        let child = "format = 1\n[[job]]\nid = \"k\"\nkind = \"jobnet\"\nuse = \"経理/集計\"\n";
        assert!(s.save("経理/子", child, None, false).is_ok());
        let looped = "format = 1\n[[job]]\nid = \"k\"\nkind = \"jobnet\"\nuse = \"経理/子\"\n";
        let err = s.save("経理/集計", looped, None, true).unwrap_err();
        assert!(err.contains("輪") || err.contains("子ジョブネット"), "{err}");
        // The text of a version, and of the file as it is, for the version box.
        assert_eq!(s.version_text("経理/集計", &versions[0]).unwrap(), one);
        assert_eq!(s.version_text("経理/集計", "current").unwrap(), two);
        assert!(s.version_text("経理/集計", "../x").is_err() && s.version_text("経理/集計", "20000101-000000").is_err());
        // Not a group/name: refused before anything is touched.
        assert!(s.save("集計", two, None, true).is_err());
        assert!(s.save("../x/y", two, None, true).is_err());
        // A restore puts the version back, keeps today's, and leaves the net unconfirmed.
        assert_eq!(s.restore("経理/集計", "20000101-000000"), Err("版がない: 経理/集計 の 20000101-000000".to_owned()));
        assert!(s.restore("経理/集計", "../../approved").is_err());
        assert!(s.restore("経理/集計", "nope").is_err());
        let said = s.restore("経理/集計", &versions[0]).unwrap();
        assert!(said.contains("未確認"), "{said}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), one);
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Unconfirmed));
        assert_eq!(Versions::parse(&s.versions("経理/集計").unwrap()).unwrap().versions.len(), 2);
        // Forced over a hand edit: saved, and the hand edit kept as a version.
        std::fs::write(&file, "format = 1\n").unwrap();
        assert!(s.save("経理/集計", two, Some(&hash), false).is_err());
        assert!(s.save("経理/集計", two, Some(&hash), true).is_ok());
        assert_eq!(Versions::parse(&s.versions("経理/集計").unwrap()).unwrap().versions.len(), 3);
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Confirmed));
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// What comes over HTTP is written as a save is, but stays unconfirmed until the window confirms it.
    #[test]
    fn a_proposal_is_written_but_left_unconfirmed() {
        let paths = temp_paths("propose");
        let mut s = State::new(paths.clone());
        let one = "format = 1\nname = \"一\"\n\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";
        let two = "format = 1\nname = \"二\"\n\n[[job]]\nid = \"a\"\nrun = [\"y.exe\"]\n";
        let file = paths.jobnets.join("経理").join("集計.toml");
        // A new net: written, and unconfirmed.
        let said = s.propose("経理/集計", one, None).unwrap();
        assert!(said.contains("未確認"), "{said}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), one);
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Unconfirmed));
        // Confirmed in the window, then changed over HTTP: unconfirmed again, the old one kept.
        assert!(s.save("経理/集計", one, Some(&approved::hash(one.as_bytes())), false).is_ok());
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Confirmed));
        let hash = approved::hash(one.as_bytes());
        assert_eq!(s.propose("経理/集計", two, Some("stale")), Err("ほかで変わっています: 経理/集計".to_owned()));
        assert!(s.propose("経理/集計", two, Some(&hash)).is_ok());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), two);
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Unconfirmed));
        assert_eq!(Versions::parse(&s.versions("経理/集計").unwrap()).unwrap().versions.len(), 2);
        // What does not read is not written.
        let two_hash = approved::hash(two.as_bytes());
        assert!(s.propose("経理/集計", "format = 1\n[[job]]\nid = \"a\"\n", Some(&two_hash)).unwrap_err().starts_with("保存できない:"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), two);
        // It comes over HTTP; a save does not.
        assert!(over_http(&Request::Propose { id: "経理/集計".into(), text: two.into(), base_sha256: None }));
        assert!(!over_http(&Request::Save { id: "経理/集計".into(), text: two.into(), base_sha256: None, force: true }));
        assert!(!over_http(&Request::Approve { id: "経理/集計".into(), sha256: two_hash }));
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    #[test]
    fn a_rename_carries_the_net_and_fixes_its_users_a_copy_is_unconfirmed_and_a_delete_is_kept() {
        let paths = temp_paths("rename");
        let mut s = State::new(paths.clone());
        let parent = "format = 1\nname = \"親\"\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";
        let child = "format = 1\nname = \"子\" # 親を呼ぶ\n[[job]]\nid = \"k\"\nkind = \"jobnet\"\nuse = \"経理/親\"\n";
        assert!(s.save("経理/親", parent, None, false).is_ok());
        assert!(s.save("経理/子", child, None, false).is_ok());
        assert!(s.hold("経理/親").is_ok());
        // Named by another net: refused, and nothing moved.
        let err = s.rename("経理/親", "総務/親", false).unwrap_err();
        assert!(err.contains("child net  経理/子") && err.contains("--fix"), "{err}");
        assert!(paths.jobnets.join("経理/親.toml").exists());
        assert_eq!(s.rename("経理/親", "経理/子", true), Err("もうある: 経理/子".to_owned()));
        assert_eq!(s.rename("経理/無い", "経理/新", true), Err("定義がない: 経理/無い".to_owned()));
        assert!(s.rename("経理/親", "総務", true).is_err());
        // With fix: moved, the user pointed at it, both still confirmed, the hold carried.
        let said = s.rename("経理/親", "総務/親", true).unwrap();
        assert!(said.contains("参照も直した: 経理/子"), "{said}");
        assert!(!paths.jobnets.join("経理/親.toml").exists());
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("総務/親.toml")).unwrap(), parent);
        assert_eq!(
            std::fs::read_to_string(paths.jobnets.join("経理/子.toml")).unwrap(),
            child.replace("use = \"経理/親\"", "use = \"総務/親\"")
        );
        assert_eq!(standing_of(&s, "総務/親"), Some(Standing::Confirmed));
        assert_eq!(standing_of(&s, "経理/子"), Some(Standing::Confirmed));
        assert_eq!(standing_of(&s, "経理/親"), None);
        assert!(s.held.is_held("総務/親") && !s.held.is_held("経理/親"));
        // The file as it was went with the net's versions; the user's old file is kept too.
        assert_eq!(Versions::parse(&s.versions("総務/親").unwrap()).unwrap().versions.len(), 1);
        assert_eq!(Versions::parse(&s.versions("経理/親").unwrap()).unwrap().versions.len(), 0);
        assert_eq!(Versions::parse(&s.versions("経理/子").unwrap()).unwrap().versions.len(), 1);
        // A copy is unconfirmed; the original is not touched.
        assert!(s.copy("総務/親", "総務/親2", None).unwrap().contains("未確認"));
        assert_eq!(standing_of(&s, "総務/親2"), Some(Standing::Unconfirmed));
        assert_eq!(s.copy("総務/親", "経理/子", None), Err("もうある: 経理/子".to_owned()));
        // A delete is refused while another net names it, and kept as a version when not.
        assert!(s.delete("総務/親").unwrap_err().contains("先にそちらを直す"));
        assert!(s.delete("経理/子").unwrap().contains("消した"));
        assert!(!paths.jobnets.join("経理/子.toml").exists());
        assert_eq!(standing_of(&s, "経理/子"), None);
        assert!(s.store.get("経理/子").is_none());
        let versions = Versions::parse(&s.versions("経理/子").unwrap()).unwrap().versions;
        assert_eq!(versions.len(), 2);
        // Brought back by a restore, unconfirmed.
        assert!(s.restore("経理/子", &versions[0]).is_ok());
        assert_eq!(standing_of(&s, "経理/子"), Some(Standing::Unconfirmed));
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    /// TODO 110: a definition staged for a day takes over on it, approved, the old one kept.
    #[test]
    fn a_staged_definition_takes_over_on_its_day_and_not_before() {
        let paths = temp_paths("stage");
        let mut s = State::new(paths.clone());
        let now = "format = 1\nname = \"締め\"\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";
        let next = "format = 1\nname = \"締め\"\n[[job]]\nid = \"a\"\nrun = [\"y.exe\"]\n";
        let file = paths.jobnets.join("経理/締め.toml");
        assert!(s.save("経理/締め", now, None, false).is_ok());
        let today = day("2026-11-02");
        // Refused: a day not after today, a net not there, the same text, a text with a fault.
        assert!(s.stage_on("経理/締め", next, today, today).unwrap_err().contains("今日か過ぎた日"));
        assert!(s.stage_on("経理/無い", next, day("2026-12-01"), today).unwrap_err().contains("定義がない"));
        assert!(s.stage_on("経理/締め", now, day("2026-12-01"), today).unwrap_err().contains("同じ中身"));
        let err = s.stage_on("経理/締め", "format = [\n", day("2026-12-01"), today).unwrap_err();
        assert!(err.starts_with("置けない:"), "{err}");
        assert!(s.stage("経理/締め", next, "12/1").unwrap_err().contains("YYYY-MM-DD"));
        assert!(!paths.next.exists());
        // Staged: kept in next.toml and shown; the net runs as it is.
        assert!(s.stage_on("経理/締め", next, day("2026-12-01"), today).unwrap().contains("2026-12-01 から使う"));
        assert_eq!(koyomi_core::next::Store::load(&paths.next).unwrap().get("経理/締め").unwrap().text, next);
        assert_eq!(s.snapshot().staged.len(), 1);
        s.switch_staged(day("2026-11-30"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), now);
        // On the day: written, confirmed, the old one kept, and no longer staged.
        s.switch_staged(day("2026-12-01"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), next);
        assert_eq!(standing_of(&s, "経理/締め"), Some(Standing::Confirmed));
        assert!(s.staged.next.is_empty() && s.staged_failed.is_empty());
        assert!(koyomi_core::next::Store::load(&paths.next).unwrap().next.is_empty());
        assert_eq!(Versions::parse(&s.versions("経理/締め").unwrap()).unwrap().versions.len(), 1);
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// TODO 110: a staged text changed by hand takes over unconfirmed; one with a fault stays
    /// staged, shown as a caution, until it is fixed or dropped.
    #[test]
    fn a_staged_text_edited_by_hand_switches_unconfirmed_and_a_broken_one_waits() {
        let paths = temp_paths("stage-hand");
        let mut s = State::new(paths.clone());
        let net = |run: &str| format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"{run}\"]\n");
        assert!(s.save("経理/a", &net("a.exe"), None, false).is_ok());
        assert!(s.save("経理/b", &net("b.exe"), None, false).is_ok());
        let today = day("2026-11-02");
        assert!(s.stage_on("経理/a", &net("a2.exe"), day("2026-12-01"), today).is_ok());
        assert!(s.stage_on("経理/b", &net("b2.exe"), day("2026-12-01"), today).is_ok());
        // By hand: a's staged text changed, b's broken, and a's file changed after it was staged.
        let mut st = koyomi_core::next::Store::load(&paths.next).unwrap();
        st.next[0].text = net("a3.exe");
        st.next[1].text = "format = [\n".to_owned();
        std::fs::write(&paths.next, st.to_toml()).unwrap();
        std::fs::write(paths.jobnets.join("経理/a.toml"), net("a1.exe")).unwrap();
        s.rescan();
        assert_eq!(s.staged.get("経理/a").unwrap().text, net("a3.exe"));
        let a = s.staged.get("経理/a").unwrap().clone();
        let line = s.switch(&a).unwrap();
        assert!(line.contains("未確認") && line.contains("直されていた"), "{line}");
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/a.toml")).unwrap(), net("a3.exe"));
        assert_eq!(standing_of(&s, "経理/a"), Some(Standing::Unconfirmed));
        s.switch_staged(day("2026-12-01"));
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/b.toml")).unwrap(), net("b.exe"));
        assert_eq!(standing_of(&s, "経理/b"), Some(Standing::Confirmed));
        assert!(s.staged.get("経理/b").is_some());
        let cautions = s.snapshot().cautions;
        assert!(cautions.iter().any(|c| c.contains("経理/b") && c.contains("切り替えられなかった")), "{cautions:?}");
        // Dropped: the net runs as it is, and the caution goes.
        assert!(s.unstage("経理/b").unwrap().contains("やめた"));
        assert!(s.staged_failed.is_empty() && s.staged.next.is_empty());
        assert!(s.unstage("経理/b").is_err());
        // A next.toml that does not read stops every change, and nothing is written over it.
        std::fs::write(&paths.next, "next = [[").unwrap();
        s.rescan();
        assert!(s.warnings.iter().any(|w| w.contains("next.toml")), "{:?}", s.warnings);
        assert!(s.stage_on("経理/b", &net("b3.exe"), day("2026-12-01"), today).is_err());
        assert!(s.unstage("経理/b").is_err());
        assert_eq!(std::fs::read_to_string(&paths.next).unwrap(), "next = [[");
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// TODO 110: a rename takes what was staged with it, and points staged texts at the new
    /// name; a delete drops it.
    #[test]
    fn a_rename_carries_what_was_staged_and_a_delete_drops_it() {
        let paths = temp_paths("stage-rename");
        let mut s = State::new(paths.clone());
        let parent = "format = 1\nname = \"親\"\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";
        let child = "format = 1\nname = \"子\"\n[[job]]\nid = \"k\"\nkind = \"jobnet\"\nuse = \"経理/親\"\n";
        assert!(s.save("経理/親", parent, None, false).is_ok());
        assert!(s.save("経理/子", child, None, false).is_ok());
        let today = day("2026-11-02");
        assert!(s.stage_on("経理/親", &parent.replace("x.exe", "y.exe"), day("2026-12-01"), today).is_ok());
        assert!(s.stage_on("経理/子", &child.replace("[[job]]", "# 次\n[[job]]"), day("2026-12-01"), today).is_ok());
        assert!(s.rename("経理/親", "総務/親", true).is_ok());
        assert!(s.staged.get("経理/親").is_none());
        assert!(s.staged.get("総務/親").unwrap().approved());
        let c = s.staged.get("経理/子").unwrap().clone();
        assert!(c.text.contains("use = \"総務/親\"") && c.approved(), "{}", c.text);
        assert_eq!(koyomi_core::next::Store::load(&paths.next).unwrap(), s.staged);
        // The rename's own change to the child's file is not told as a change by hand.
        let line = s.switch(&c).unwrap();
        assert!(!line.contains("直されていた") && !line.contains("未確認"), "{line}");
        assert_eq!(standing_of(&s, "経理/子"), Some(Standing::Confirmed));
        s.switch_staged(day("2026-12-01"));
        assert_eq!(standing_of(&s, "総務/親"), Some(Standing::Confirmed));
        assert!(std::fs::read_to_string(paths.jobnets.join("総務/親.toml")).unwrap().contains("y.exe"));
        // A delete drops what was staged.
        let live = std::fs::read_to_string(paths.jobnets.join("経理/子.toml")).unwrap();
        assert!(s.stage_on("経理/子", &live.replace("# 次\n", ""), day("2027-01-01"), today).is_ok());
        assert!(s.delete("経理/子").is_ok());
        assert!(s.staged.next.is_empty());
        assert!(koyomi_core::next::Store::load(&paths.next).unwrap().next.is_empty());
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    #[test]
    fn a_copy_or_rename_retitles_a_net_still_named_by_its_old_file_and_keeps_a_custom_title() {
        let paths = temp_paths("retitle");
        let mut s = State::new(paths.clone());
        let plain = "# note
format = 1
name = \"ab\"
[[job]]
id = \"a\"
run = [\"x.exe\"]
";
        let custom = plain.replace("name = \"ab\"", "name = \"Month end\"");
        assert!(s.save("g/ab", plain, None, false).is_ok());
        assert!(s.save("g/me", &custom, None, false).is_ok());
        let label = |s: &State, id: &str| s.defs.iter().find(|d| d.id == id).map(|d| d.label().to_owned());
        assert!(s.copy("g/ab", "g/ab-2", None).is_ok());
        assert_eq!(label(&s, "g/ab-2").as_deref(), Some("ab-2"));
        assert!(std::fs::read_to_string(paths.jobnets.join("g/ab-2.toml")).unwrap().starts_with(
            "# note
"
        ));
        assert!(s.copy("g/me", "g/me-2", None).is_ok());
        assert_eq!(label(&s, "g/me-2").as_deref(), Some("Month end"));
        assert!(s.rename("g/ab-2", "g/cd", false).is_ok());
        assert_eq!(label(&s, "g/cd").as_deref(), Some("cd"));
        assert!(s.rename("g/me-2", "g/other", false).is_ok());
        assert_eq!(label(&s, "g/other").as_deref(), Some("Month end"));
        // An edited 表示名 is written as typed; a cleared one leaves no `name =`, so the tree shows the file.
        assert!(s.copy("g/ab", "g/ab-3", Some("Edited")).is_ok());
        assert_eq!(label(&s, "g/ab-3").as_deref(), Some("Edited"));
        assert!(s.copy("g/me", "g/me-3", Some("")).is_ok());
        assert!(!std::fs::read_to_string(paths.jobnets.join("g/me-3.toml")).unwrap().contains("name = \"Month"));
        assert_eq!(label(&s, "g/me-3").as_deref(), Some("me-3"));
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    #[test]
    fn a_replace_shows_the_lines_and_with_apply_writes_them_unconfirmed_and_kept() {
        let paths = temp_paths("replace");
        let mut s = State::new(paths.clone());
        let one = "format = 1
name = \"一\"
[[job]]
id = \"a\"
run = [\"//old/tool.exe\", \"-v\"]
";
        let two = "format = 1
name = \"二\"
[[job]]
id = \"a\"
run = [\"x.exe\"]
";
        assert!(s.save("経理/一", one, None, false).is_ok());
        assert!(s.save("経理/二", two, None, false).is_ok());
        assert_eq!(s.replace("", "x", false), Err("探す文字が空".to_owned()));
        assert_eq!(s.replace("x", "x", true), Err("探す文字と直す文字が同じ".to_owned()));
        assert_eq!(s.replace("//nowhere", "x", true), Ok("見つからない: //nowhere".to_owned()));
        // Shown, not written.
        let shown = s.replace("//old", "//new", false).unwrap();
        assert!(
            shown.contains("経理/一") && shown.contains("5: + run = [\"//new/tool.exe\", \"-v\"]") && !shown.contains("経理/二"),
            "{shown}"
        );
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/一.toml")).unwrap(), one);
        // A result that does not read is refused, and nothing is written.
        let err = s.replace("\"a\"", "a", true).unwrap_err();
        assert!(err.contains("何も書かなかった"), "{err}");
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/二.toml")).unwrap(), two);
        // Applied: written, the old form kept, unconfirmed again; the other net stays confirmed.
        let said = s.replace("//old", "//new", true).unwrap();
        assert!(said.contains("1 個のジョブネットの 1 行"), "{said}");
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/一.toml")).unwrap(), one.replace("//old", "//new"));
        assert_eq!(standing_of(&s, "経理/一"), Some(Standing::Unconfirmed));
        assert_eq!(standing_of(&s, "経理/二"), Some(Standing::Confirmed));
        assert_eq!(Versions::parse(&s.versions("経理/一").unwrap()).unwrap().versions.len(), 1);
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    #[test]
    fn a_snapshot_survives_the_door() {
        let net =
            "format = 1\nname = \"月次締め\"\ncalendar = \"会社\"\n[[job]]\nid = \"a\"\nrun = [\"x.exe\", \"-v\"]\npos = [1.0, 2.0]\n";
        let dir = std::env::temp_dir().join(format!("koyomi-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("経理")).unwrap();
        std::fs::write(dir.join("経理/月次締め.toml"), net).unwrap();
        std::fs::create_dir_all(dir.join("共通")).unwrap();
        std::fs::write(dir.join("共通/壊れ.toml"), "format = 1\nrnu = 1\n").unwrap();
        let cal_dir = std::env::temp_dir().join(format!("koyomi-snap-cals-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cal_dir);
        std::fs::create_dir_all(&cal_dir).unwrap();
        std::fs::write(
            cal_dir.join("会社.toml"),
            "format = 1\nholidays = \"japan\"\nweekly_off = [\"sat\", \"sun\"]\nclosed = [2026-11-20, { date = 2026-12-29, name = \"年末\" }]\nopen = [{ date = 2026-11-21, name = \"棚卸\" }]\n",
        )
        .unwrap();
        let cals = Calendars::load(&cal_dir);
        let snap = Snapshot {
            defs: scan_in(&dir, &Store::default(), &cals),
            warnings: vec!["w".into()],
            notes: vec!["n".into()],
            held: vec!["共通/x".into()],
            held_jobs: vec![koyomi_core::held::HeldJob { net: "共通/x".into(), job: "b".into() }],
            skips: Vec::new(),
            paused: true,
            cautions: vec!["c".into()],
            calendar_cautions: Default::default(),
            calendars: cals.map.clone(),
            power: proto::Power {
                sleep_after: Some(1800),
                cannot_wake: None,
                wake_at: Some("2026-10-31 01:58".into()),
                wake_for: Some("夜/中".into()),
            },
            blackouts: vec![Blackout { from: "2026-12-29".into(), to: "2027-01-03".into(), why: "年末年始".into() }],
            staged: vec![koyomi_core::next::Entry {
                id: "共通/x".into(),
                from: NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(),
                sha256: approved::hash(b"format = 1\n"),
                base: None,
                text: "format = 1\n".into(),
            }],
        };
        let back = Snapshot::parse(&snap.to_toml()).unwrap();
        assert_eq!(back, snap);
        assert_eq!(back.defs.len(), 2);
        assert_eq!(back.defs[1].calendar.as_deref(), Some("会社"));
        let cal = &back.calendars["会社"];
        assert_eq!(cal.closed.len(), 2);
        assert_eq!(cal.open.values().next().map(String::as_str), Some("棚卸"));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&cal_dir);
    }
}
