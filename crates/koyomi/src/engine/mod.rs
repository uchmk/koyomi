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
use koyomi_core::jobnet::{Jobnet, Part};
use koyomi_core::late::{self, Delay};
use koyomi_core::msg;
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
mod session;
mod shells;
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
        let app = ito_common::app_dir("koyomi").ok_or_else(|| msg::line(msg::SETTINGS_DIR_UNKNOWN, &[]))?;
        let state = ito_common::state_dir("koyomi").ok_or_else(|| msg::line(msg::STATE_DIR_UNKNOWN, &[]))?;
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
        Config::load(&self.config).map_err(|e| load_failed(msg::CONFIG_LOAD_FAILED, &self.config, e))
    }

    /// The approvals, or the line that says `approved.toml` does not read.
    pub fn store(&self) -> Result<Store, String> {
        Store::load(&self.approved).map_err(|e| load_failed(msg::APPROVED_LOAD_FAILED, &self.approved, e))
    }

    /// What is on hold, or the line that says `held.toml` does not read.
    pub fn held(&self) -> Result<Held, String> {
        Held::load(&self.held).map_err(|e| load_failed(msg::HELD_LOAD_FAILED, &self.held, e))
    }

    /// The staged definitions, or the line that says `next.toml` does not read.
    pub fn staged(&self) -> Result<koyomi_core::next::Store, String> {
        koyomi_core::next::Store::load(&self.next).map_err(|e| load_failed(msg::NEXT_LOAD_FAILED, &self.next, e))
    }
}

/// A state file that does not read: its own coded reason as it is, else `id` around the OS's words.
fn load_failed(id: msg::Id, path: &Path, e: String) -> String {
    if msg::is_coded(&e) {
        e
    } else {
        msg::line(id, &[&shown(path), &e])
    }
}

/// Set to run the ACL tests (`scripts/verify-acl.sh`): the tests that need the machine's real permissions.
pub const ACL_TEST_ENV: &str = "KOYOMI_TEST_ACL";

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
    seen.into_iter().map(|(p, who)| msg::line(msg::FILE_WRITABLE_BY, &[&shown(&p), &who])).collect()
}

/// The warnings in `warnings` that bear on the net in `file`: those about the shared folders and
/// files, and those about this net's own file, but not those about another net's file.
pub fn warnings_about(paths: &Paths, warnings: &[String], defs: &[Def], file: &str) -> Vec<String> {
    let at = |f: &str| shown(&f.split('/').fold(paths.jobnets.clone(), |p, c| p.join(c)));
    let own = at(file);
    let others: Vec<String> = defs.iter().filter(|d| d.file != file).map(|d| at(&d.file)).collect();
    // A coded warning has its ID at the head, so the path is looked for anywhere in it.
    warnings.iter().filter(|w| w.contains(&own) || !others.iter().any(|o| w.contains(o.as_str()))).cloned().collect()
}

/// The path without Windows' `\\?\` prefix.
/// The warnings on one line, each without its ID: the line that holds them has the ID.
/// `BLOCKED_BY_WARNINGS`: `op` (an `N_OP_*` note) is refused while the warnings at the start stand.
fn blocked(op: msg::Note, warnings: &str) -> String {
    msg::line(msg::BLOCKED_BY_WARNINGS, &[&msg::note(op, &[]), warnings])
}

fn joined_warnings(warnings: &[String]) -> String {
    warnings.iter().map(|w| msg::uncoded(w)).collect::<Vec<_>>().join(" / ")
}

/// A bundle that does not read: a coded line stays as it is, anything else is the reason of one.
fn import_failed(e: String) -> String {
    if msg::is_coded(&e) {
        e
    } else {
        msg::line(msg::IMPORT_FAILED, &[&e])
    }
}

fn shown(p: &Path) -> String {
    let s = p.display().to_string();
    s.strip_prefix(r"\\?\").map(str::to_owned).unwrap_or(s)
}

/// The first name `dir/<name>.<ext>` is not taken under: `stamp`, else `stamp-2`, `stamp-3` ...
fn free_version(dir: &Path, stamp: &str, ext: &str) -> String {
    let mut version = stamp.to_owned();
    let mut n = 1;
    while dir.join(format!("{version}.{ext}")).exists() {
        n += 1;
        version = format!("{stamp}-{n}");
    }
    version
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
    /// Whether reloading asks the machine who can write what koyomi runs (`access_warnings`). Off in a normal
    /// test run (the machine's ACL would decide its results); on in production, and in the `KOYOMI_TEST_ACL` run.
    access_check: bool,
    /// Why `koyomi.toml` does not read: until it does, nothing is started.
    bad_config: Option<String>,
    /// `[[blackout]]`: the planned stops; no schedule or event starts a net in them.
    blackouts: Vec<Blackout>,
    /// The shells this PC has (name, absolute path), looked for at start and on each sweep.
    shells: Vec<(String, PathBuf)>,
    /// `shell` in `koyomi.toml`, as written.
    shell_written: Option<String>,
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
    /// `[[when]] session`: a sign-in, a sign-out, a lock or an unlock.
    sessions: session::Sessions,
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
    /// The words a run is told in (a notice's title and body, `koyomi status`): the engine's language,
    /// the same as its messages and reasons, so one notice is in one language (see [`engine_words`]).
    words: ito_i18n::Table,
    /// The words of the tray on this PC (its tip and menu): this PC's language (common.toml's, else
    /// the OS's), as `koyomi tray`'s own menu is, since only someone at this PC reads them.
    tray_words: ito_i18n::Table,
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
            access_check: !cfg!(test) || std::env::var_os(ACL_TEST_ENV).is_some(),
            bad_config,
            blackouts,
            shells: Vec::new(),
            shell_written: None,
            runner,
            schedule,
            events: events::Events::new(),
            logs: logs::Logs::new(),
            mails: mails::Mails::new(),
            idle: idle::Idle::new(),
            sessions: session::Sessions::new(),
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
            words: engine_words(),
            tray_words: crate::cli::table(),
            acked: std::sync::Mutex::new((None, std::collections::BTreeSet::new())),
            late_look: None,
            usual: HashMap::new(),
            late_told: HashSet::new(),
        };
        s.reload();
        s.rescan();
        if let Some((line, crashed)) = s.alive.outage.take() {
            let title = msg::note(if crashed { msg::N_TITLE_ENGINE_CRASHED } else { msg::N_TITLE_ENGINE_STOPPED }, &[]);
            crate::notify::tell(&s.notify, None, "failed", &format!("koyomi: {title}"), &line);
            crate::say(&line, false);
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
                self.shell_written = c.shell.clone();
                self.runner.set_config(c);
                self.bad_config = None;
            }
            Err(e) => self.bad_config = Some(e),
        }
    }

    /// Looks for the shells on this PC again (cheap) and hands them to the runner.
    fn reload_shells(&mut self) {
        self.shells = shells::find_here();
        self.runner.set_shells(self.shells.clone());
    }

    /// The shell a new script job starts with, and the one `koyomi.toml` wrote when this engine does not have it.
    fn default_shell(&self) -> (String, Option<String>) {
        shells::default_shell(&self.shells, self.shell_written.as_deref(), cfg!(windows))
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
        let fresh: Vec<Run> = over.iter().filter(|r| !self.runner.closed_stale(&r.id)).cloned().collect();
        self.tick_after(&fresh);
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
        self.runner.stop_all(self.runner.stop_wait());
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
        tray::look(&self.tray_words, &self.runner.today(), &acked, self.held.paused, self.runner.busy(), next.map(|(at, _)| at))
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
            fails: look.fails.into_iter().map(|(id, label)| proto::TrayFail { id, label }).collect(),
            more: look.more,
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
            let why = msg::note(if busy > 0 { msg::N_AWAKE_BUSY } else { msg::N_AWAKE_WAITING }, &[]);
            self.awake = Some(ito_power::Awake::hold(&format!("koyomi: {why}")));
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
                    self.runner.not_run(&def, *planned, RunState::Skipped, &msg::note(msg::N_SKIP_NEXT_REASON, &[]));
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
                            let why = msg::note(msg::N_QUIET_PENDING, &[&first.planned.format("%-H:%M").to_string()]);
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
                    let line = if taken.is_empty() { line } else { format!("{line} {taken}") };
                    (msg::note(msg::N_TITLE_TOOK_OVER, &[]), line)
                }
                standby::Told::Both(line) => (msg::note(msg::N_TITLE_BOTH, &[]), line),
                standby::Told::Waiting(line) => (msg::note(msg::N_TITLE_WAITING_TAKEOVER, &[]), line),
                standby::Told::GaveBack(line) => (msg::note(msg::N_TITLE_GAVE_BACK, &[]), line),
            };
            crate::notify::tell(&self.notify, None, "failed", &format!("koyomi: {title}"), &line);
            crate::say(&format!("koyomi: {line}"), false);
        }
    }

    /// Asks the standby `[primary]` names, and tells when it took the schedules or let them go.
    fn tick_primary(&mut self) {
        if let Some((title, line)) = self.primary.tick() {
            crate::notify::tell(&self.notify, None, "failed", &title, &line);
            crate::say(&format!("koyomi: {line}"), false);
        }
    }

    /// The runs `from` had going when this standby took over: each of a net with
    /// `takeover = "rerun"` starts again here from the first job that had not ended ok; the rest
    /// are written down as not taken over, for a person to decide. What to add to the line.
    fn take_runs(&mut self, runs: Option<Vec<Run>>, from: &str) -> String {
        let Some(runs) = runs else {
            return msg::note(msg::N_TAKE_UNREAD, &[from]);
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
                (Resume::Rerun, None) => msg::note(msg::N_STANDBY_NO_DEFINITION, &[]),
                (Resume::Leave(why), _) => why,
            };
            left.push(self.runner.left(theirs, def.as_ref(), from, msg::uncoded(&why)));
        }
        let mut out = msg::note(msg::N_TAKE_SUMMARY, &[from, &runs.len().to_string(), &again.to_string()]);
        if !left.is_empty() {
            out.push(' ');
            out.push_str(&msg::note(msg::N_TAKE_LEFT, &[&left.join(&msg::note(msg::N_RUN_LIST_JOIN, &[]))]));
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
                    return Err(msg::line(msg::HANDOVER_TOO_NEAR, &[&near]));
                }
            }
            return self.standby.handover(release).map(|v| v.to_string());
        }
        match self.primary.json() {
            Some(_) if release => Err(msg::line(msg::HANDOVER_NOT_STANDBY, &[])),
            Some(v) => Ok(v.to_string()),
            None => Err(msg::line(msg::NO_STANDBY_OR_PRIMARY, &[])),
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
                    Ok(()) => crate::say(&format!("koyomi: {}", msg::note(msg::N_STANDBY_COPIED, &[&c.id])), false),
                    Err(e) => {
                        refused.push(format!("{}: {}", c.id, koyomi_core::msg::uncoded(&e)));
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
                let why =
                    msg::note(msg::N_WAIT_UNTIL_OVER, &[&w.for_net, &w.until.map_or(String::new(), |u| u.format("%H:%M").to_string())]);
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
            return Some((RunState::Locked, msg::note(msg::N_HOLIDAY_STALE, &[&until.to_string()])));
        }
        if self.standby.waiting() {
            return Some((RunState::Skipped, msg::note(msg::N_STANDBY_WAITING, &[])));
        }
        if let Some(why) = self.primary.held() {
            return Some((RunState::Skipped, why));
        }
        if self.held.paused {
            return Some((RunState::Held, msg::note(msg::N_PAUSED, &[])));
        }
        if self.held.is_held(&def.id) {
            return Some((RunState::Held, msg::note(msg::N_HELD_NET, &[&def.id])));
        }
        None
    }

    /// Why an event on `day` does not start `def` by its `when`: the day is
    /// outside `start` / `end` or in `except`, or it is a closed day and
    /// `on_closed` is not `"run"`. `what` is the event, for the record.
    fn off_day(&self, def: &Def, w: &when::When, day: NaiveDate, what: &str) -> Option<String> {
        if !w.runs_on(day) {
            return Some(msg::note(msg::N_OFF_DAY_RANGE, &[what]));
        }
        let cal = def.calendar.as_deref().and_then(|c| self.calendars.get(c));
        if cal.is_some_and(|c| !c.is_business(day)) && w.on_closed_in(cal) != OnClosed::Run {
            return Some(msg::note(msg::N_OFF_DAY_CLOSED, &[what]));
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
                if let Some(why) = self.off_day(&def, w, today, &msg::note(msg::N_EVENT_START, &[])) {
                    self.runner.not_run(&def, now, RunState::Skipped, &why);
                    continue;
                }
                if let Some((state, why)) = self.stopped(&def, today, now) {
                    self.runner.not_run(&def, now, state, &why);
                    continue;
                }
                match schedule::judge(&def, now, chrono::Duration::zero(), &self.warnings, self.bad_config.as_deref(), false, true) {
                    schedule::Verdict::Start { .. } | schedule::Verdict::Queue { .. } => {
                        if let Err(e) = self.runner.start_event(&def, today, How::Start, &msg::note(msg::N_EVENT_START, &[]), None) {
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
            let arrived = msg::note(msg::N_EVENT_FILE, &[&path]);
            if let Some(why) = self.off_day(&def, w, today, &arrived) {
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
                    if let Err(e) = self.runner.start_event(&def, today, How::File, &arrived, Some(path)) {
                        self.runner.not_run(&def, now, RunState::Locked, &e);
                    }
                }
                schedule::Verdict::NotRun(state, reason) => {
                    self.runner.not_run(&def, now, state, &reason);
                }
            }
            self.events.fired(&a.def_id, &a.pattern, &a.path);
        }
        let hits: Vec<logs::Hit> =
            self.logs.due().into_iter().chain(self.mails.due()).chain(self.idle.due()).chain(self.sessions.due()).collect();
        for hit in hits {
            let Some(def) = self.defs.iter().find(|d| d.id == hit.def_id).cloned() else { continue };
            let Some(w) = def.net.as_ref().and_then(|n| n.when.iter().find(|w| w.kind == hit.kind)).cloned() else { continue };
            let what = match &hit.kind {
                Kind::Log { log, .. } => msg::note(msg::N_EVENT_LOG, &[log]),
                Kind::EventLog { eventlog, .. } => msg::note(msg::N_EVENT_EVENTLOG, &[eventlog]),
                Kind::Mail { mail, .. } => msg::note(msg::N_EVENT_MAIL, &[mail]),
                Kind::Idle { idle: secs } => msg::note(msg::N_EVENT_IDLE, &[&idle::span_text(*secs)]),
                Kind::Session { session, .. } => session::what(*session, &hit.first),
                _ => continue,
            };
            let how = match &hit.kind {
                Kind::Mail { .. } => How::Mail,
                Kind::Idle { .. } => How::Idle,
                Kind::Session { .. } => How::Session,
                _ => How::Log,
            };
            let more = if hit.count > 1 { msg::note(msg::N_EVENT_MORE, &[&(hit.count - 1).to_string()]) } else { String::new() };
            // The PC left alone has no line to keep; whose lock it was is said in `what`.
            let first = if matches!(hit.kind, Kind::Session { .. }) { "" } else { hit.first.as_str() };
            let tail = if first.is_empty() { more } else { msg::note(msg::N_EVENT_FIRST, &[first, &more]) };
            let with_tail = |s: &str| if tail.is_empty() { s.to_owned() } else { format!("{s} {tail}") };
            let reason = with_tail(&what);
            if let Some(why) = self.off_day(&def, &w, today, &what) {
                self.runner.not_run(&def, now, RunState::Skipped, &with_tail(&why));
                continue;
            }
            if let Some((state, why)) = self.stopped(&def, today, now) {
                self.runner.not_run(&def, now, state, &why);
                continue;
            }
            // A line, a mail, the PC left alone or a lock is not kept as a file is: one that comes while the net runs is told of, not run later.
            if self.runner.runs().iter().any(|r| r.jobnet == def.id && !r.is_over()) {
                self.runner.not_run(&def, now, RunState::Skipped, &with_tail(&msg::note(msg::N_EVENT_STILL_RUNNING, &[&what])));
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
                let what = msg::note(msg::N_EVENT_AFTER, &[&prev.jobnet, &crate::show::word(&self.words, prev.state)]);
                if let Some(why) = self.off_day(&def, &w, date, &what) {
                    self.runner.not_run(&def, now, RunState::Skipped, &why);
                    continue;
                }
                if let Some((state, why)) = self.stopped(&def, date, now) {
                    self.runner.not_run(&def, now, state, &why);
                    continue;
                }
                if let Some(done) = followed(self.runner.runs(), &def.id, &prev.date) {
                    let why = msg::note(msg::N_AFTER_SAME_DAY, &[&what, &prev.date, &done.id, &def.id]);
                    self.runner.not_run(&def, now, RunState::Skipped, &why);
                    continue;
                }
                if self.runner.runs().iter().any(|r| r.jobnet == def.id && !r.is_over()) {
                    self.runner.not_run(&def, now, RunState::Skipped, &msg::note(msg::N_EVENT_STILL_RUNNING, &[&what]));
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
        self.reload_shells();
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
                self.runner.not_run(&def, now, RunState::Skipped, &msg::note(msg::N_WATCH_EXISTING, &[&n.to_string(), &pattern]));
            }
        }
        self.logs.set_rules(&self.defs);
        self.mails.set_rules(&self.defs);
        self.idle.set_rules(&self.defs);
        self.sessions.set_rules(&self.defs);
        let files: Vec<PathBuf> = self.defs.iter().map(|d| self.paths.jobnets.join(&d.file)).collect();
        self.warnings = if self.access_check { access_warnings(&self.paths, &files) } else { Vec::new() };
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
                    (Standing::Unconfirmed, _) => Err(msg::line(msg::CHILD_UNCONFIRMED, &[&d.id])),
                    _ => Err(msg::line(msg::CHILD_INVALID, &[&d.id])),
                };
                (d.id.clone(), net)
            })
            .collect()
    }

    fn snapshot(&self) -> Snapshot {
        let mut warnings = self.warnings.clone();
        warnings.extend(self.bad_config.clone());
        let blocked = self.blocked_ids(&warnings);
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
            .chain((!self.events.watching()).then(|| msg::note(msg::N_EVENTS_NOT_WATCHED, &[])))
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
                .chain(self.os_cautions())
                // A [[when]] that cannot come here (a mailbox or log that does not read, an idle
                // or session this engine cannot see): the net is still valid and may be approved.
                .chain(self.logs.problems())
                .chain(self.mails.problems())
                .chain(self.idle.problems())
                .chain(self.sessions.problems())
                .collect(),
            calendar_cautions: self.calendars.cautions_by_calendar(ito_common::now::now_local().date_naive(), CAUTION_DAYS),
            calendars: self.calendars.map.clone(),
            power: self.power(),
            blackouts: self.blackouts.clone(),
            staged: self.staged.next.clone(),
            blocked,
            utc_offset_min: ito_common::now::now_local().offset().local_minus_utc() / 60,
            shells: shells::listed(&self.shells),
            shell: self.default_shell().0,
            accounts: self.runner.config().account.iter().map(|a| a.name.clone()).collect(),
            databases: self
                .runner
                .config()
                .database
                .iter()
                .map(|d| proto::DatabaseName { name: d.name.clone(), kind: d.kind().unwrap_or_default().to_owned() })
                .collect(),
            windows: Some(cfg!(windows)),
        }
    }

    /// The ids of the nets a warning bears on (approving one is refused), as the window's `blocked_for`
    /// judges each: a warning about another net's file does not count.
    fn blocked_ids(&self, warnings: &[String]) -> Vec<String> {
        if warnings.is_empty() {
            return Vec::new();
        }
        self.defs.iter().filter(|d| !warnings_about(&self.paths, warnings, &self.defs, &d.file).is_empty()).map(|d| d.id.clone()).collect()
    }

    /// What a definition uses that does not work on this PC, one line each. Only what the definition itself
    /// shows: a shell this engine does not have (any OS), and, off Windows, a chore written as PowerShell with no
    /// `pwsh`, `run_as`, an Event Log trigger. Not warnings, which would stop a net from being confirmed.
    fn os_cautions(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(w) = self.default_shell().1 {
            out.push(msg::line(msg::SHELL_DEFAULT_MISSING, &[&w, &self.default_shell().0]));
        }
        let has = |name: &str| self.shells.iter().any(|(n, _)| n == name);
        for d in &self.defs {
            let Some(net) = &d.net else { continue };
            let mut told: Vec<&str> = Vec::new();
            for j in &net.jobs {
                let Some(shell) = j.shell.as_deref().filter(|s| !has(s) && !told.contains(s)) else { continue };
                told.push(shell);
                out.push(msg::line(msg::OS_NO_SHELL, &[&d.id, &j.id, shell]));
            }
            out.extend(koyomi_core::registry::unknown_lines(self.runner.config(), &d.id, net));
            if cfg!(windows) {
                continue;
            }
            if !has("pwsh") && net.jobs.iter().any(|j| matches!(&j.part, Part::Chore(c) if !c.own())) {
                out.push(msg::line(msg::OS_NO_PWSH, &[&d.id]));
            }
            if net.jobs.iter().any(|j| j.run_as.is_some()) {
                out.push(msg::line(msg::OS_RUN_AS, &[&d.id]));
            }
            if net.when.iter().any(|w| matches!(w.kind, Kind::EventLog { .. })) {
                out.push(msg::line(msg::OS_EVENTLOG, &[&d.id]));
            }
        }
        out
    }

    fn status(&self) -> String {
        let confirmed = self.defs.iter().filter(|d| d.standing == Standing::Confirmed).count();
        let running = self.runner.runs().iter().filter(|r| !r.is_over()).count();
        let head = msg::note(msg::N_STATUS_HEAD, &[&self.defs.len().to_string(), &confirmed.to_string(), &running.to_string()]);
        let mut out = format!("koyomi engine pid {}: {head}\n{}\n{}", std::process::id(), self.retain.line(), self.backup.line());
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
            out.push_str(&format!("\n{}", msg::note(msg::N_STATUS_PAUSED, &[])));
        }
        for id in &self.held.held {
            out.push_str(&format!("\n{}", msg::note(msg::N_STATUS_HELD, &[id])));
        }
        for h in &self.held.jobs {
            out.push_str(&format!("\n{}", msg::note(msg::N_STATUS_HELD_JOB, &[&h.net, &h.job])));
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
                "\n{}",
                msg::note(
                    msg::N_STATUS_QUEUED,
                    &[q.def.label(), &q.planned.format("%-H:%M").to_string(), &q.since.format("%-H:%M").to_string()]
                )
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
            Some(d) => NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| msg::line(msg::BUSINESS_DAY_FORM_BAD, &[d]))?,
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
        if prev.never_began() {
            // Nothing ran, so there is no saved definition to continue from: only a fresh start of
            // the net as it stands now (and only while it may be started).
            if from.is_some() || next.is_some() {
                return Err(msg::line(msg::RERUN_NEVER_BEGAN_PARTIAL, &[]));
            }
            let def = self.startable(&prev.jobnet)?.clone();
            let date = prev.date.parse().map_err(|_| msg::line(msg::RUN_DATE_UNREADABLE, &[&prev.id]))?;
            return self.runner.start_manual(&def, date, None, &BTreeMap::new());
        }
        self.startable(&prev.jobnet)?;
        self.runner.rerun(run, from, next)
    }

    /// The definition `id` as it may be started: nothing warned, koyomi.toml reads, and it is confirmed.
    fn startable(&self, id: &str) -> Result<&Def, String> {
        if !self.warnings.is_empty() {
            return Err(blocked(msg::N_OP_RUN, &joined_warnings(&self.warnings)));
        }
        if let Some(e) = &self.bad_config {
            return Err(blocked(msg::N_OP_RUN, msg::uncoded(e)));
        }
        let def = self.defs.iter().find(|d| d.id == id).ok_or_else(|| msg::line(msg::JOBNET_MISSING, &[id]))?;
        match def.standing {
            Standing::Invalid => Err(msg::line(msg::RUN_INVALID, &[&def.file])),
            Standing::Unconfirmed => Err(msg::line(msg::RUN_UNCONFIRMED, &[id])),
            Standing::Confirmed if self.held.is_held(id) => Err(msg::line(msg::RUN_HELD, &[id])),
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
        let label = self.defs.iter().find(|d| d.id == id).map(Def::label).ok_or_else(|| msg::line(msg::JOBNET_MISSING, &[id]))?.to_owned();
        if !self.held.hold(id) {
            return Err(msg::line(msg::HOLD_ALREADY, &[id]));
        }
        self.save_held()?;
        for q in self.runner.unqueue(Some(id)) {
            self.runner.not_run(&q.def, q.planned, RunState::Held, &msg::note(msg::N_HOLD_WHILE_QUEUED, &[id]));
        }
        Ok(msg::note(msg::N_HELD_DONE, &[&label, id]))
    }
    fn release(&mut self, id: &str) -> Result<String, String> {
        self.held_writable()?;
        if !self.held.release(id) {
            return Err(msg::line(msg::HOLD_NOT, &[id]));
        }
        self.save_held()?;
        Ok(msg::note(msg::N_RELEASED, &[id]))
    }
    /// `koyomi hold <net> <job>`: a run of `id` that comes to `job` waits there as ⏸ 保留.
    fn hold_job(&mut self, id: &str, job: &str) -> Result<String, String> {
        self.held_writable()?;
        let def = self.defs.iter().find(|d| d.id == id).ok_or_else(|| msg::line(msg::JOBNET_MISSING, &[id]))?;
        let net = def.net.as_ref().ok_or_else(|| msg::line(msg::JOBNET_UNREADABLE, &[id]))?;
        let label = net
            .jobs
            .iter()
            .find(|j| j.id == job)
            .map(|j| j.name.clone().unwrap_or_else(|| j.id.clone()))
            .ok_or_else(|| msg::line(msg::NET_JOB_MISSING, &[id, job]))?;
        if !self.held.hold_job(id, job) {
            let shown =
                net.jobs.iter().find(|j| j.id == job).map_or_else(|| job.to_owned(), |j| koyomi_core::run::named(&j.id, j.name.as_deref()));
            return Err(msg::line(msg::HOLD_JOB_ALREADY, &[id, &shown]));
        }
        self.save_held()?;
        self.runner.set_held(&self.held);
        Ok(msg::note(msg::N_HELD_JOB_DONE, &[def.label(), &label, id, job]))
    }
    fn release_job(&mut self, id: &str, job: &str) -> Result<String, String> {
        self.held_writable()?;
        if !self.held.release_job(id, job) {
            let shown = self
                .defs
                .iter()
                .find(|d| d.id == id)
                .and_then(|d| d.net.as_ref())
                .and_then(|n| n.jobs.iter().find(|j| j.id == job))
                .map_or_else(|| job.to_owned(), |j| koyomi_core::run::named(&j.id, j.name.as_deref()));
            return Err(msg::line(msg::HOLD_JOB_NOT, &[id, &shown]));
        }
        self.save_held()?;
        self.runner.set_held(&self.held);
        Ok(msg::note(msg::N_RELEASED_JOB, &[id, job]))
    }
    /// `koyomi shift`: the time `from` of `id` starts at `to` once (`None`: the move is taken back).
    fn shift(&mut self, id: &str, from: &str, to: Option<&str>) -> Result<String, String> {
        self.held_writable()?;
        let at = |s: &str| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").map_err(|_| msg::line(msg::TIME_FORM_BAD, &[s]));
        let from = at(from)?;
        let Some(to) = to else {
            if !self.held.unshift(id, from) {
                return Err(msg::line(msg::SHIFT_NOT_SHIFTED, &[id, &from.format("%Y-%m-%d %H:%M").to_string()]));
            }
            self.save_held()?;
            return Ok(msg::note(msg::N_UNSHIFTED, &[]));
        };
        let to = at(to)?;
        let def = self.defs.iter().find(|d| d.id == id).ok_or_else(|| msg::line(msg::JOBNET_MISSING, &[id]))?;
        let net = def.net.as_ref().ok_or_else(|| msg::line(msg::JOBNET_UNREADABLE, &[id]))?;
        let cal = def.calendar.as_deref().and_then(|c| self.calendars.get(c));
        if !net.when.iter().any(|w| !w.is_event() && w.times_on_in(from.date(), cal).contains(&from.time())) {
            return Err(msg::line(msg::SHIFT_NO_SUCH_TIME, &[id, &from.format("%Y-%m-%d %H:%M").to_string()]));
        }
        if to <= from {
            return Err(msg::line(msg::SHIFT_BEFORE, &[]));
        }
        if to <= ito_common::now::now_local().naive_local() {
            return Err(msg::line(msg::SHIFT_PAST, &[]));
        }
        self.held.shift(id, from, to);
        self.save_held()?;
        Ok(msg::note(msg::N_SHIFTED, &[id, &from.format("%Y-%m-%d %H:%M").to_string(), &to.format("%Y-%m-%d %H:%M").to_string()]))
    }
    /// `koyomi skip-next`: the next time of `id` that is not yet skipped or moved does not start
    /// (`cancel`: every skip of `id` is taken back).
    fn skip_next(&mut self, id: &str, cancel: bool) -> Result<String, String> {
        self.held_writable()?;
        if cancel {
            if !self.held.unskip(id) {
                return Err(msg::line(msg::SKIP_NOT, &[id]));
            }
            self.save_held()?;
            return Ok(msg::note(msg::N_UNSKIPPED, &[id]));
        }
        let def = self.defs.iter().find(|d| d.id == id).ok_or_else(|| msg::line(msg::JOBNET_MISSING, &[id]))?;
        let net = def.net.as_ref().ok_or_else(|| msg::line(msg::JOBNET_UNREADABLE, &[id]))?;
        if def.standing != Standing::Confirmed {
            return Err(msg::line(msg::SKIP_UNCONFIRMED, &[id]));
        }
        let cal = def.calendar.as_deref().and_then(|c| self.calendars.get(c));
        let held = &self.held;
        let at = koyomi_core::held::next_open(&net.when, cal, ito_common::now::now_local().naive_local(), |t| {
            held.is_skipped(id, t) || held.is_shifted(id, t)
        })
        .ok_or_else(|| msg::line(msg::SKIP_NO_NEXT, &[id]))?;
        self.held.skip(id, at);
        self.save_held()?;
        Ok(msg::note(msg::N_SKIPPED_NEXT, &[id, &at.format("%Y-%m-%d %H:%M").to_string()]))
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
            return Err(msg::line(if on { msg::PAUSE_ALREADY } else { msg::PAUSE_NOT }, &[]));
        }
        self.held.paused = on;
        self.save_held()?;
        if on {
            for q in self.runner.unqueue(None) {
                self.runner.not_run(&q.def, q.planned, RunState::Held, &msg::note(msg::N_PAUSE_WHILE_QUEUED, &[]));
            }
        }
        Ok(msg::note(if on { msg::N_PAUSE_DONE } else { msg::N_RESUME_DONE }, &[]))
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
        // Checked against the file as it is now, not as the last scan saw it.
        self.rescan();
        self.approve_scanned(id, sha256)
    }

    /// [`Self::approve`] over the state of the scan just made.
    fn approve_scanned(&mut self, id: &str, sha256: &str) -> Result<String, String> {
        let def = self.defs.iter().find(|d| d.id == id).ok_or_else(|| msg::line(msg::JOBNET_MISSING, &[id]))?;
        let own = self.warnings_for(&def.file);
        if !own.is_empty() {
            return Err(msg::line(msg::APPROVE_BLOCKED, &[&joined_warnings(&own)]));
        }
        if def.standing == Standing::Invalid {
            return Err(msg::line(msg::APPROVE_INVALID, &[&def.file]));
        }
        if def.sha256 != sha256 {
            return Err(msg::line(msg::APPROVE_CHANGED, &[&def.file]));
        }
        let mut next = self.store.clone();
        next.approve(id, &def.sha256, &def.text);
        // Written before it counts, and whole: a crash leaves the old approvals.
        if let Some(dir) = self.paths.approved.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        ito_common::write_atomic(&self.paths.approved, next.to_toml()).map_err(|e| write_failed(&self.paths.approved, &e))?;
        self.store = next;
        self.rescan();
        Ok(msg::note(msg::N_NET_APPROVED, &[id]))
    }

    /// The editor's save (see [`Request::Save`]): checked as a file would be, the old
    /// file kept under `versions/`, written whole, and approved at once.
    fn save(&mut self, id: &str, text: &str, base_sha256: Option<&str>, force: bool) -> Result<String, String> {
        self.write_def(id, text, base_sha256, force, true)?;
        Ok(msg::note(msg::N_SAVED, &[id]))
    }

    /// A definition sent over HTTP (see [`Request::Propose`]): written as a save is, but left
    /// unconfirmed for this PC's window to confirm.
    fn propose(&mut self, id: &str, text: &str, base_sha256: Option<&str>) -> Result<String, String> {
        self.write_def(id, text, base_sha256, false, false)?;
        let left = self.defs.iter().find(|d| d.id == id).is_some_and(|d| d.standing != Standing::Confirmed);
        Ok(msg::note(if left { msg::N_PROPOSED_UNCONFIRMED } else { msg::N_PROPOSED_SAME }, &[id]))
    }

    /// Checks `text` as a file would be, keeps the old file under `versions/` and writes it
    /// whole; with `approve`, confirms it too.
    fn write_def(&mut self, id: &str, text: &str, base_sha256: Option<&str>, force: bool, approve: bool) -> Result<(), String> {
        if !self.warnings.is_empty() {
            return Err(koyomi_core::msg::line(koyomi_core::msg::SAVE_BLOCKED, &[&joined_warnings(&self.warnings)]));
        }
        let (group, name) = split_id(id)?;
        let path = self.paths.jobnets.join(group).join(format!("{name}.toml"));
        // The file as it is now, not as the last scan saw it: an edit by hand a moment ago counts.
        let now = std::fs::read(&path).ok().map(|b| approved::hash(&b));
        if !force && now.as_deref() != base_sha256 {
            return Err(koyomi_core::msg::line(koyomi_core::msg::SAVE_CHANGED_ELSEWHERE, &[id]));
        }
        let def = self.checked_def(id, text, koyomi_core::msg::DEF_INVALID_SAVE)?;
        self.keep_version(&path, group, name)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| dir_failed(dir, &e))?;
        }
        ito_common::write_atomic(&path, text).map_err(|e| write_failed(&path, &e))?;
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
    /// group's `_group.toml` and against the other nets); refused with the message `doing` (holes: the id and the
    /// problems), so the ID is at its head.
    fn checked_def(&self, id: &str, text: &str, doing: koyomi_core::msg::Id) -> Result<Def, String> {
        let (group, name) = split_id(id)?;
        let shared = group::load(&self.paths.jobnets.join(group).join("_group.toml"));
        let def = def_of_text(text, group, name, &self.store, shared.as_ref(), &self.calendars);
        let mut all: Vec<Def> = self.defs.iter().filter(|d| d.id != id).cloned().collect();
        all.push(def);
        link(&mut all);
        let def = all.pop().expect("pushed");
        if def.standing == Standing::Invalid {
            // One line, one ID: a problem with its own ID gives it up to the entry that holds it.
            let why: Vec<String> = def.problems.iter().map(|p| koyomi_core::msg::uncoded(&p.in_file(&def.file)).to_owned()).collect();
            return Err(koyomi_core::msg::line(doing, &[id, &why.join(" / ")]));
        }
        Ok(def)
    }

    /// See [`Request::Stage`].
    fn stage(&mut self, id: &str, text: &str, from: &str, expect: Option<&proto::StageExpect>) -> Result<String, String> {
        let from =
            NaiveDate::parse_from_str(from, "%Y-%m-%d").map_err(|_| koyomi_core::msg::line(koyomi_core::msg::STAGE_DATE_BAD, &[from]))?;
        self.stage_expecting(id, text, from, ito_common::now::now_local().date_naive(), expect)
    }

    /// Stages `text` to take over `id` on `from`, approved now, when `today` is before it.
    #[cfg(test)]
    fn stage_on(&mut self, id: &str, text: &str, from: NaiveDate, today: NaiveDate) -> Result<String, String> {
        self.stage_expecting(id, text, from, today, None)
    }

    /// [`Self::stage_on`] that first compares what the caller saw (`expect`) with what is here: the live
    /// definition's hash and the seal of the staged one, so a stage made over another person's change is
    /// refused. `None` (the CLI) compares nothing.
    fn stage_expecting(
        &mut self,
        id: &str,
        text: &str,
        from: NaiveDate,
        today: NaiveDate,
        expect: Option<&proto::StageExpect>,
    ) -> Result<String, String> {
        if !self.warnings.is_empty() {
            return Err(koyomi_core::msg::line(koyomi_core::msg::SAVE_BLOCKED, &[&joined_warnings(&self.warnings)]));
        }
        // `next.toml` as it is now, not as the last scan saw it: a hand edit a moment ago counts. When it no
        // longer reads, `staged` is empty and `warnings` is stale, so the write is refused here.
        self.reload_staged();
        self.staged_writable()?;
        if from <= today {
            return Err(koyomi_core::msg::line(koyomi_core::msg::STAGE_DATE_PAST, &[&from.to_string()]));
        }
        let (group, name) = split_id(id)?;
        let path = self.paths.jobnets.join(group).join(format!("{name}.toml"));
        let live = std::fs::read(&path).map_err(|_| koyomi_core::msg::line(koyomi_core::msg::STAGE_NO_LIVE, &[id]))?;
        if let Some(e) = expect {
            let here = self.staged.get(id).map(koyomi_core::next::Entry::seal);
            if e.live != approved::hash(&live) || e.staged != here {
                return Err(koyomi_core::msg::line(koyomi_core::msg::STAGE_CHANGED_ELSEWHERE, &[id]));
            }
        }
        if live == text.as_bytes() {
            return Err(koyomi_core::msg::line(koyomi_core::msg::STAGE_SAME, &[id]));
        }
        self.checked_def(id, text, koyomi_core::msg::DEF_INVALID_STAGE)?;
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
        Ok(msg::note(msg::N_STAGED, &[&from.to_string(), id]))
    }

    /// See [`Request::Unstage`].
    fn unstage(&mut self, id: &str) -> Result<String, String> {
        if let Some(e) = &self.staged_bad {
            return Err(e.clone());
        }
        let from = self.staged.get(id).map(|e| e.from).ok_or_else(|| msg::line(msg::UNSTAGE_NONE, &[id]))?;
        let mut staged = self.staged.clone();
        staged.forget(id);
        self.save_staged(&staged)?;
        self.staged = staged;
        self.staged_failed.remove(id);
        Ok(msg::note(msg::N_UNSTAGED, &[&from.to_string(), id]))
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
                    notify::tell(&to, None, "done", &format!("koyomi: {}", msg::note(msg::N_TITLE_SWITCHED, &[])), &line);
                }
                Err(why) => {
                    let why = msg::uncoded(&why).lines().collect::<Vec<_>>().join(" ");
                    let line = msg::line(msg::SWITCH_FAILED, &[&e.id, &e.from.to_string(), &why]);
                    if self.staged_failed.get(&e.id) != Some(&line) {
                        crate::say(&line, true);
                        notify::tell(&to, None, "failed", &format!("koyomi: {}", msg::note(msg::N_TITLE_SWITCH_FAILED, &[])), &line);
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
            return Err(joined_warnings(&self.warnings));
        }
        let (group, name) = split_id(&e.id)?;
        let path = self.paths.jobnets.join(group).join(format!("{name}.toml"));
        let live = std::fs::read(&path).map_err(|_| msg::line(msg::SWITCH_NO_LIVE, &[&e.id]))?;
        self.checked_def(&e.id, &e.text, koyomi_core::msg::DEF_INVALID_SWITCH)?;
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
        let mut line = msg::note(msg::N_SWITCHED, &[&e.id, &e.from.to_string()]);
        if !approved {
            line.push(' ');
            line.push_str(&msg::note(msg::N_SWITCHED_UNCONFIRMED, &[]));
        }
        if changed {
            line.push(' ');
            line.push_str(&msg::note(msg::N_SWITCHED_CHANGED, &[]));
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
        self.keep_copy(path, dir, "toml")
    }

    /// Copies the file at `path`, when there is one, to `dir/<YYYYMMDD-HHMMSS>.<ext>`. Only a file that is not
    /// there means there is nothing to keep; one that cannot be read is an error, and the caller writes nothing.
    fn keep_copy(&self, path: &Path, dir: &Path, ext: &str) -> Result<(), String> {
        let old = match std::fs::read(path) {
            Ok(old) => old,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(koyomi_core::msg::line(koyomi_core::msg::FILE_READ_FAILED, &[&shown(path), &e.to_string()])),
        };
        std::fs::create_dir_all(dir).map_err(|e| dir_failed(dir, &e))?;
        let stamp = ito_common::now::now_local().format("%Y%m%d-%H%M%S").to_string();
        let version = free_version(dir, &stamp, ext);
        let to = dir.join(format!("{version}.{ext}"));
        ito_common::write_atomic(&to, old).map_err(|e| write_failed(&to, &e))
    }

    /// Brings a bundle in. The plan is answered whatever happens; it is carried
    /// out only when nothing is undecided or wrong and `dry_run` is off. What is
    /// written over is kept under `versions/` first; a net that comes in is
    /// unconfirmed, like a file written by hand.
    ///
    /// `by` is the caller signed in to the engine's HTTP door (empty: this PC). Such a caller's koyomi.toml
    /// goes through the same guards as `ConfigPut`: its secrets must be usable and it must not cut the connection.
    fn import(&mut self, text: &str, dry_run: bool, on_conflict: Option<&str>, by: &str) -> Result<String, String> {
        self.import_with(text, dry_run, on_conflict, by, secret_usable)
    }

    /// [`Self::import`] with the credential store as `usable` (whether a `secret:` name holds a token).
    fn import_with(
        &mut self,
        text: &str,
        dry_run: bool,
        on_conflict: Option<&str>,
        by: &str,
        usable: impl Fn(&str) -> bool,
    ) -> Result<String, String> {
        if !self.warnings.is_empty() {
            return Err(blocked(msg::N_OP_IMPORT, &joined_warnings(&self.warnings)));
        }
        let on_conflict = on_conflict.map(bundle::Conflict::parse).transpose()?;
        // A Task Scheduler task comes as its XML: one net, and notes on what did not come over.
        let (b, notes) = if koyomi_core::task::is_xml(text) {
            koyomi_core::task::bundle(text).map_err(import_failed)?
        } else {
            (bundle::parse(text).map_err(import_failed)?, Vec::new())
        };
        if b.is_empty() {
            return Err(msg::line(msg::IMPORT_EMPTY, &[]));
        }
        // Every engine writer of koyomi.toml is serialised, whichever way it was asked: the lock is taken before the
        // plan reads the files and held until the import returns, so nothing lands between the plan and the writes.
        let _edit = crate::remote::edits();
        let current = bundle::Source::read(&self.paths.app);
        let mut p = bundle::plan(&b, &current, on_conflict);
        // The settings must pass what `koyomi check` passes, or nothing is written.
        for item in p.items.iter_mut().filter(|i| i.kind == bundle::Kind::Settings && i.problems.is_empty()) {
            if let Some(e) = crate::notify::settings_problem(&item.new) {
                item.problems.push(msg::line(msg::BUNDLE_SETTINGS_BAD, &[&e]));
            }
        }
        // A caller signed in to the HTTP door: what `ConfigPut` refuses is refused here too.
        if !by.is_empty() {
            for item in p
                .items
                .iter_mut()
                .filter(|i| i.kind == bundle::Kind::Settings && i.problems.is_empty() && matches!(i.change, Change::Add | Change::Replace))
            {
                // A text that does not read is already a problem above.
                let Ok(new) = koyomi_core::config::Config::parse(&item.new) else {
                    continue;
                };
                if let Some(name) = unusable_secret(&new, &usable) {
                    item.problems.push(msg::line(msg::CONFIG_SECRET, &[name]));
                }
                if cuts_link(item.old.as_deref().unwrap_or_default(), &new, by) {
                    item.problems.push(msg::line(msg::CONFIG_CUTS_LINK, &[by]));
                }
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
            lines.extend(item.problems.iter().map(|p| msg::note(msg::N_BUNDLE_PROBLEM, &[p])));
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
                    std::fs::create_dir_all(parent).map_err(|e| dir_failed(parent, &e))?;
                }
                ito_common::write_atomic(&path, item.new.clone())
            })();
            if let Err(e) = done {
                self.rescan();
                return Err(msg::line(msg::IMPORT_STOPPED, &[&written.to_string(), msg::uncoded(&e)]));
            }
            written += 1;
        }
        self.rescan();
        reply.applied = true;
        Ok(reply.to_toml())
    }

    /// The file of the calendar `name` (`calendars/<name>.toml`), after the name is checked: not empty, no
    /// path separator and nothing a file name cannot hold ([`koyomi_core::calendar::valid_name`]).
    fn calendar_file(&self, name: &str) -> Result<(String, PathBuf), String> {
        let n = name.trim();
        if n.is_empty() {
            return Err(koyomi_core::msg::line(koyomi_core::msg::CAL_NAME_EMPTY, &[]));
        }
        if !koyomi_core::calendar::valid_name(n) {
            return Err(koyomi_core::msg::line(koyomi_core::msg::CAL_NAME_BAD, &[n]));
        }
        Ok((n.to_owned(), self.paths.calendars.join(format!("{n}.toml"))))
    }

    /// The versions folder of calendar `name`: `versions/_calendars/<name>`.
    fn calendar_versions(&self, name: &str) -> PathBuf {
        self.paths.state.join("versions").join("_calendars").join(name)
    }

    /// See [`Request::CalendarText`].
    fn calendar_text(&self, name: &str) -> Result<String, String> {
        let (n, path) = self.calendar_file(name)?;
        let text = std::fs::read_to_string(&path).map_err(|_| koyomi_core::msg::line(koyomi_core::msg::CAL_MISSING, &[&n]))?;
        Ok(proto::CalendarBody { sha256: approved::hash(text.as_bytes()), text }.to_toml())
    }

    /// See [`Request::CalendarPut`]: a new calendar when `base_sha256` is `None` (nothing is written over),
    /// otherwise the text of the one with that hash; the old text is kept under `versions/_calendars/`.
    fn calendar_put(&mut self, name: &str, text: &str, base_sha256: Option<&str>) -> Result<String, String> {
        let (n, path) = self.calendar_file(name)?;
        koyomi_core::calendar::Calendar::parse(&n, text)?;
        let there = match std::fs::read(&path) {
            Ok(old) => Some(old),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(koyomi_core::msg::line(koyomi_core::msg::FILE_READ_FAILED, &[&shown(&path), &e.to_string()])),
        };
        match (there, base_sha256) {
            (None, None) => {}
            (Some(old), Some(base)) if approved::hash(&old) == base => {}
            (None, Some(_)) | (Some(_), Some(_)) => return Err(koyomi_core::msg::line(koyomi_core::msg::CAL_CHANGED_ELSEWHERE, &[&n])),
            (Some(_), None) => return Err(koyomi_core::msg::line(koyomi_core::msg::CALENDAR_EXISTS, &[&n])),
        }
        self.keep_version_in(&path, &self.calendar_versions(&n))?;
        std::fs::create_dir_all(&self.paths.calendars).map_err(|e| dir_failed(&self.paths.calendars, &e))?;
        ito_common::write_atomic(&path, text).map_err(|e| write_failed(&path, &e))?;
        self.rescan();
        Ok(msg::note(msg::N_CAL_WRITTEN, &[&n]))
    }

    /// See [`Request::CalendarRename`]: nothing is taken over. A name that differs only in case is allowed
    /// only when both paths are the same file (a case-insensitive file system); on a case-sensitive one
    /// they are two files and the destination is not replaced.
    fn calendar_rename(&mut self, from: &str, to: &str) -> Result<String, String> {
        let (f, from_path) = self.calendar_file(from)?;
        let (t, to_path) = self.calendar_file(to)?;
        if f == t {
            return Err(koyomi_core::msg::line(koyomi_core::msg::CAL_SAME_NAME, &[&f]));
        }
        if !from_path.exists() {
            return Err(koyomi_core::msg::line(koyomi_core::msg::CAL_MISSING, &[&f]));
        }
        let same_file = matches!(
            (std::fs::canonicalize(&from_path), std::fs::canonicalize(&to_path)),
            (Ok(a), Ok(b)) if a == b
        );
        if to_path.exists() && !same_file {
            return Err(koyomi_core::msg::line(koyomi_core::msg::CALENDAR_EXISTS, &[&t]));
        }
        std::fs::rename(&from_path, &to_path).map_err(|e| write_failed(&to_path, &e))?;
        self.rescan();
        Ok(msg::note(msg::N_CAL_RENAMED, &[&f, &t]))
    }

    /// See [`Request::CalendarDelete`]: the file kept under `versions/_calendars/` first.
    fn calendar_delete(&mut self, name: &str, base_sha256: &str) -> Result<String, String> {
        let (n, path) = self.calendar_file(name)?;
        let old = std::fs::read(&path).map_err(|_| koyomi_core::msg::line(koyomi_core::msg::CAL_MISSING, &[&n]))?;
        if approved::hash(&old) != base_sha256 {
            return Err(koyomi_core::msg::line(koyomi_core::msg::CAL_CHANGED_ELSEWHERE, &[&n]));
        }
        self.keep_version_in(&path, &self.calendar_versions(&n))?;
        std::fs::remove_file(&path)
            .map_err(|e| koyomi_core::msg::line(koyomi_core::msg::FILE_DELETE_FAILED, &[&shown(&path), &e.to_string()]))?;
        self.rescan();
        Ok(msg::note(msg::N_CAL_DELETED, &[&n]))
    }

    /// See [`Request::HolidayTable`]: the bytes written as they are when they read, over the table there
    /// (kept under `versions/_calendars/syukujitsu/` first).
    fn holiday_table(&mut self, bytes: &[u8]) -> Result<String, String> {
        let table = koyomi_core::calendar::Table::parse(&koyomi_core::calendar::Table::decode(bytes))?;
        let dest = self.paths.calendars.join(koyomi_core::calendar::TABLE_FILE);
        self.keep_copy(&dest, &self.calendar_versions("syukujitsu"), "csv")?;
        std::fs::create_dir_all(&self.paths.calendars).map_err(|e| dir_failed(&self.paths.calendars, &e))?;
        ito_common::write_atomic(&dest, bytes).map_err(|e| write_failed(&dest, &e))?;
        self.rescan();
        Ok(msg::note(msg::N_HOLIDAY_IMPORTED, &[koyomi_core::calendar::TABLE_FILE, &table.years.0.to_string(), &table.years.1.to_string()]))
    }

    /// See [`Request::ExportAll`]: every net, calendar, group and the settings as one bundle, as the
    /// window's 全部を書き出す makes it (the text, not a file).
    fn export_all(&self) -> Result<String, String> {
        let src = bundle::Source::read(&self.paths.app);
        let b = notify::export(&src, &bundle::Pick { all: true, ..Default::default() })?;
        if b.is_empty() {
            return Err(msg::line(msg::EXPORT_NOTHING, &[]));
        }
        Ok(b.to_toml())
    }

    /// koyomi.toml's text, for the users; `None` when there is no such file.
    fn users_text(&self) -> Result<Option<String>, String> {
        match std::fs::read_to_string(&self.paths.config) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(koyomi_core::msg::line(koyomi_core::msg::FILE_READ_FAILED, &[&shown(&self.paths.config), &e.to_string()])),
        }
    }

    /// See [`Request::Users`]: only the state of each token, never its value or the store's own words.
    fn users(&self) -> Result<String, String> {
        use koyomi_core::users::{Listed, Listing, Token};
        let text = self.users_text()?.unwrap_or_default();
        let user: Vec<Listed> = koyomi_core::users::listed(&text, |name| match ito_secret::get("koyomi", name) {
            Ok(Some(v)) if !v.is_empty() => Token::Ok,
            Ok(_) => Token::Missing,
            Err(_) => Token::Unreadable,
        })?;
        Ok(Listing { base: koyomi_core::users::sha256(&text), user }.to_toml())
    }

    /// koyomi.toml's text for a change of the users: it must be there, and still be what `base` says.
    fn users_base(&self, base: &str) -> Result<String, String> {
        let text = self.users_text()?.ok_or_else(|| koyomi_core::msg::line(koyomi_core::msg::USER_NO_FILE, &[]))?;
        if koyomi_core::users::sha256(&text) != base {
            return Err(koyomi_core::msg::line(koyomi_core::msg::USER_BASE_CHANGED, &[]));
        }
        Ok(text)
    }

    /// Like [`Self::users_base`], but a missing koyomi.toml is the empty text (what `Users` lists it as).
    fn users_base_or_empty(&self, base: &str) -> Result<String, String> {
        let text = self.users_text()?.unwrap_or_default();
        if koyomi_core::users::sha256(&text) != base {
            return Err(koyomi_core::msg::line(koyomi_core::msg::USER_BASE_CHANGED, &[]));
        }
        Ok(text)
    }

    /// Writes koyomi.toml, first keeping the old text under `versions/_settings/`. Nothing is reloaded
    /// here: the sweep reads it within a few seconds and the HTTP listener swaps the users then and stays open; the reply in
    /// flight is not cut (each connection has its own thread).
    fn users_write(&self, text: &str) -> Result<(), String> {
        self.keep_version_in(&self.paths.config, &self.paths.state.join("versions").join("_settings"))?;
        if let Some(dir) = self.paths.config.parent() {
            std::fs::create_dir_all(dir).map_err(|e| dir_failed(dir, &e))?;
        }
        ito_common::write_atomic(&self.paths.config, text).map_err(|e| write_failed(&self.paths.config, &e))
    }

    /// See [`Request::UserSet`].
    fn user_set(
        &self,
        name: &str,
        role: &str,
        nets: Option<Vec<String>>,
        roles: Option<BTreeMap<String, String>>,
        base: &str,
    ) -> Result<String, String> {
        let _edit = crate::remote::edits();
        let text = self.users_base(base)?;
        let out = koyomi_core::users::set(&text, name, role, nets, roles)?;
        self.users_write(&out)?;
        Ok(msg::note(msg::N_USER_SET_DONE, &[name.trim()]))
    }

    /// See [`Request::UserRemove`].
    fn user_remove(&self, name: &str, base: &str) -> Result<String, String> {
        let _edit = crate::remote::edits();
        let text = self.users_base(base)?;
        let out = koyomi_core::users::remove(&text, name)?;
        self.users_write(&out)?;
        Ok(msg::note(msg::N_USER_REMOVE_DONE_ENGINE, &[name.trim()]))
    }

    /// A new token and its hash. The token is for the reply and nowhere else.
    fn new_token() -> Result<(String, String), String> {
        let token = ito_http::new_token().map_err(|e| msg::line(msg::USER_TOKEN_FAILED, &[&e.to_string()]))?;
        let hash = ito_http::token_hash(&token);
        Ok((token, hash))
    }

    /// The reply of `UserAdd` and `UserReset`: the name, the token (shown only here) and a note.
    fn token_reply(name: &str, token: &str) -> Result<String, String> {
        let mut t = toml::Table::new();
        t.insert("name".into(), name.into());
        t.insert("token".into(), token.into());
        t.insert("note".into(), msg::note(msg::N_USER_REPLY_NOTE, &[]).into());
        toml::to_string(&t).map_err(|e| e.to_string())
    }

    /// See [`Request::UserAdd`].
    fn user_add(
        &self,
        name: &str,
        role: &str,
        nets: Option<Vec<String>>,
        roles: Option<BTreeMap<String, String>>,
        base: &str,
    ) -> Result<String, String> {
        let _edit = crate::remote::edits();
        let text = self.users_base_or_empty(base)?;
        let (token, hash) = Self::new_token()?;
        let out = koyomi_core::users::add(&text, name, role, nets, roles, &hash)?;
        self.users_write(&out)?;
        Self::token_reply(name.trim(), &token)
    }

    /// See [`Request::UserReset`].
    fn user_reset(&self, name: &str, base: &str) -> Result<String, String> {
        let _edit = crate::remote::edits();
        let text = self.users_base(base)?;
        let (token, hash) = Self::new_token()?;
        let out = koyomi_core::users::reset(&text, name, &hash)?;
        self.users_write(&out)?;
        Self::token_reply(name.trim(), &token)
    }

    /// See [`Request::ConfigText`]: a missing koyomi.toml is the empty text. The hash is the one `Users` gives.
    fn config_text(&self) -> Result<String, String> {
        let text = self.users_text()?.unwrap_or_default();
        Ok(proto::ConfigBody { sha256: koyomi_core::users::sha256(&text), text }.to_toml())
    }

    /// See [`Request::ConfigPut`]; `by` is the caller signed in to the engine's HTTP door (empty: this PC).
    fn config_put(&self, text: &str, base: &str, by: &str) -> Result<String, String> {
        self.config_put_with(text, base, by, secret_usable)
    }

    /// [`Self::config_put`] with the credential store as `usable` (whether a `secret:` name holds a token).
    /// The refusals come in this order, and each one writes nothing.
    fn config_put_with(&self, text: &str, base: &str, by: &str, usable: impl Fn(&str) -> bool) -> Result<String, String> {
        let _edit = crate::remote::edits();
        let old = self.users_text()?.unwrap_or_default();
        if koyomi_core::users::sha256(&old) != base {
            return Err(msg::line(msg::CONFIG_CHANGED, &[]));
        }
        let new = koyomi_core::config::Config::parse(text)?;
        if let Some(name) = unusable_secret(&new, &usable) {
            return Err(msg::line(msg::CONFIG_SECRET, &[name]));
        }
        if !by.is_empty() && cuts_link(&old, &new, by) {
            return Err(msg::line(msg::CONFIG_CUTS_LINK, &[by]));
        }
        self.users_write(text)?;
        Ok(msg::note(msg::N_CONFIG_WRITTEN, &[]))
    }

    /// The nets (confirmed or not) and staged definitions that name each account / database, by id.
    fn registry_uses(&self) -> (BTreeMap<String, Vec<String>>, BTreeMap<String, Vec<String>>) {
        let (mut accounts, mut databases): (BTreeMap<String, Vec<String>>, BTreeMap<String, Vec<String>>) = Default::default();
        let mut note = |id: &str, net: &koyomi_core::jobnet::Jobnet| {
            let names = koyomi_core::registry::names_used(net);
            for n in names.accounts {
                let v = accounts.entry(n).or_default();
                if !v.iter().any(|x| x == id) {
                    v.push(id.to_owned());
                }
            }
            for n in names.databases {
                let v = databases.entry(n).or_default();
                if !v.iter().any(|x| x == id) {
                    v.push(id.to_owned());
                }
            }
        };
        for d in &self.defs {
            if let Some(net) = &d.net {
                note(&d.id, net);
            }
        }
        for e in &self.staged.next {
            if let Ok(net) = koyomi_core::jobnet::parse(&e.text) {
                note(&e.id, &net);
            }
        }
        (accounts, databases)
    }

    /// See [`Request::Registry`]; `get` reads a secret (the credential store, or a test's own).
    fn registry(&self, get: &dyn Fn(&str) -> std::io::Result<Option<String>>) -> Result<String, String> {
        let text = self.users_text()?.unwrap_or_default();
        let config = koyomi_core::config::Config::parse(&text)?;
        let state = |name: Option<&str>| match name {
            None => "not-needed",
            Some(n) => match get(n) {
                Ok(Some(v)) if !v.is_empty() => "ok",
                Ok(_) => "missing",
                Err(_) => "unreadable",
            },
        };
        let (au, du) = self.registry_uses();
        let accounts = config
            .account
            .iter()
            .map(|a| proto::AccountRow {
                name: a.name.clone(),
                user: a.user.clone(),
                secret: a.secret().unwrap_or_default().to_owned(),
                state: if a.secret().is_none() { "missing" } else { state(a.secret()) }.to_owned(),
                used_by: au.get(&a.name).cloned().unwrap_or_default(),
            })
            .collect();
        let databases = config
            .database
            .iter()
            .map(|d| proto::DatabaseRow {
                name: d.name.clone(),
                kind: d.kind().unwrap_or_default().to_owned(),
                db: d.db.clone(),
                secret: d.secret().map(str::to_owned),
                tls: d.tls.map(|t| t.key().to_owned()),
                state: state(d.secret()).to_owned(),
                used_by: du.get(&d.name).cloned().unwrap_or_default(),
            })
            .collect();
        Ok(proto::RegistryBody { sha256: koyomi_core::users::sha256(&text), accounts, databases }.to_toml())
    }

    /// The four writes of [`Request::AccountPut`] and its kin. In order, each refusal writing nothing: the
    /// base hash (CONFIG_CHANGED), the edit itself, the file reading as a whole, a name still in use (unless
    /// `force`; `remove` is `(account?, name)`). Then the file is written, read again, and the new
    /// registry is the reply.
    fn registry_write(
        &mut self,
        base: &str,
        force: bool,
        build: &dyn Fn(&str) -> Result<String, String>,
        remove: Option<(bool, &str)>,
        get: &dyn Fn(&str) -> std::io::Result<Option<String>>,
    ) -> Result<String, String> {
        let _edit = crate::remote::edits();
        let old = self.users_text()?.unwrap_or_default();
        if koyomi_core::users::sha256(&old) != base {
            return Err(msg::line(msg::CONFIG_CHANGED, &[]));
        }
        let new = build(&old)?;
        koyomi_core::config::Config::parse(&new)?;
        if let (Some((account, name)), false) = (remove, force) {
            let (au, du) = self.registry_uses();
            let users = if account { au } else { du }.remove(name).unwrap_or_default();
            if !users.is_empty() {
                let shown = users.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
                let table = if account { "[[account]]" } else { "[[database]]" };
                return Err(msg::line(msg::REGISTRY_IN_USE, &[table, name, &shown, &users.len().to_string()]));
            }
        }
        self.users_write(&new)?;
        self.reload_config();
        if let Some(e) = &self.bad_config {
            return Err(e.clone());
        }
        self.registry(get)
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
                return Err(msg::line(msg::VERSION_FORM_BAD, &[version]));
            }
            self.versions_dir(group, name).join(format!("{version}.toml"))
        };
        let bytes = std::fs::read(&path).map_err(|_| msg::line(msg::VERSION_MISSING, &[id, version]))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Puts a kept version back as the file (the file as it is is kept first). Not approved:
    /// what comes back is looked at like a file written by hand.
    fn restore(&mut self, id: &str, version: &str) -> Result<String, String> {
        if !self.warnings.is_empty() {
            return Err(blocked(msg::N_OP_RESTORE, &joined_warnings(&self.warnings)));
        }
        let (group, name) = split_id(id)?;
        if version.is_empty() || !version.chars().all(|c| c.is_ascii_digit() || c == '-') {
            return Err(msg::line(msg::VERSION_FORM_BAD, &[version]));
        }
        let from = self.versions_dir(group, name).join(format!("{version}.toml"));
        let text = std::fs::read(&from).map_err(|_| msg::line(msg::VERSION_MISSING, &[id, version]))?;
        let path = self.paths.jobnets.join(group).join(format!("{name}.toml"));
        self.keep_version(&path, group, name)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| dir_failed(dir, &e))?;
        }
        ito_common::write_atomic(&path, text)?;
        self.rescan();
        let left = self.defs.iter().find(|d| d.id == id).is_some_and(|d| d.standing != Standing::Confirmed);
        Ok(msg::note(if left { msg::N_RESTORED_UNCONFIRMED } else { msg::N_RESTORED_SAME }, &[version, id]))
    }

    /// The net `id` as a file there is, its path, and that it does not run now: what a rename,
    /// a copy and a delete start from.
    fn net_file(&self, id: &str, doing: msg::Note) -> Result<(PathBuf, String), String> {
        if !self.warnings.is_empty() {
            return Err(blocked(doing, &joined_warnings(&self.warnings)));
        }
        let (group, name) = split_id(id)?;
        let path = self.paths.jobnets.join(group).join(format!("{name}.toml"));
        let bytes = std::fs::read(&path).map_err(|_| msg::line(msg::JOBNET_MISSING, &[id]))?;
        Ok((path, String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// Why `to` cannot be a new net's id (not one, or there is one), if it cannot.
    fn free_id(&self, to: &str) -> Result<PathBuf, String> {
        let (group, name) = split_id(to)?;
        let path = self.paths.jobnets.join(group).join(format!("{name}.toml"));
        if path.exists() || self.defs.iter().any(|d| d.id == to) {
            return Err(msg::line(msg::JOBNET_EXISTS, &[to]));
        }
        Ok(path)
    }

    /// Refused while a run of `id` is not over.
    fn not_running(&self, id: &str) -> Result<(), String> {
        if self.runner.runs().iter().any(|r| r.jobnet == id && !r.is_over()) {
            return Err(msg::line(msg::JOBNET_RUNNING, &[id]));
        }
        Ok(())
    }

    /// The lines of `koyomi uses` for the nets that name `id`, and how to go on.
    fn named_by(&self, id: &str, by: &[String], head: msg::Id) -> String {
        let mut lines = vec![msg::line(head, &[&by.len().to_string(), id])];
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
        let (from_path, text) = self.net_file(id, msg::N_OP_RENAME)?;
        let to_path = self.free_id(to)?;
        self.not_running(id)?;
        self.held_writable()?;
        self.staged_writable()?;
        let by = koyomi_core::rename::referrers(&self.defs, id);
        if !by.is_empty() && !fix {
            return Err(self.named_by(id, &by, msg::RENAME_USED));
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
            let (path, old) = self.net_file(r, msg::N_OP_RENAME)?;
            let new = koyomi_core::rename::retarget(&old, id, to).map_err(|e| {
                // A coded line keeps its ID at the head and gets the referrer after it.
                if msg::is_coded(&e) {
                    msg::note(msg::N_RENAMED_REFS_FAILED, &[&e, r])
                } else {
                    e
                }
            })?;
            rewritten.insert(r.clone(), (approved::hash(old.as_bytes()), approved::hash(new.as_bytes())));
            others.push((r.clone(), path, new, confirmed(r)));
        }
        let staged = self.staged_renamed(id, to, &rewritten);
        let mut next = self.store.clone();
        let was_confirmed = confirmed(id);
        if let Some(dir) = to_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| dir_failed(dir, &e))?;
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
        std::fs::remove_file(&from_path).map_err(|e| msg::line(msg::FILE_DELETE_FAILED, &[&shown(&from_path), &e.to_string()]))?;
        if self.held.carry(id, Some(to)) {
            self.save_held()?;
        }
        if staged != self.staged {
            self.save_staged(&staged)?;
            self.staged = staged;
            self.staged_failed.remove(id);
        }
        self.rescan();
        let mut said = msg::note(msg::N_RENAMED, &[id, to]);
        if !others.is_empty() {
            said.push(' ');
            said.push_str(&msg::note(msg::N_RENAMED_REFS, &[&by.join(&msg::note(msg::N_RUN_LIST_JOIN, &[]))]));
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
        std::fs::create_dir_all(to).map_err(|e| dir_failed(to, &e))?;
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let stem = name.strip_suffix(".toml").unwrap_or(&name).to_owned();
            let mut target = to.join(&name);
            let mut n = 1;
            while target.exists() {
                n += 1;
                target = to.join(format!("{stem}-{n}.toml"));
            }
            std::fs::rename(e.path(), &target).map_err(|err| msg::line(msg::FILE_MOVE_FAILED, &[&shown(&e.path()), &err.to_string()]))?;
        }
        let _ = std::fs::remove_dir(from);
        Ok(())
    }

    /// See [`Request::Copy`].
    fn copy(&mut self, id: &str, to: &str, title: Option<&str>) -> Result<String, String> {
        let (_, text) = self.net_file(id, msg::N_OP_COPY)?;
        let to_path = self.free_id(to)?;
        let text = match title {
            Some(title) => koyomi_core::rename::set_title(&text, title),
            None => koyomi_core::rename::retitle(&text, split_id(id)?.1, split_id(to)?.1),
        };
        if let Some(dir) = to_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| dir_failed(dir, &e))?;
        }
        ito_common::write_atomic(&to_path, text)?;
        self.rescan();
        Ok(msg::note(msg::N_COPIED, &[id, to]))
    }

    /// See [`Request::Replace`].
    fn replace(&mut self, from: &str, to: &str, apply: bool) -> Result<String, String> {
        if from.is_empty() {
            return Err(msg::line(msg::REPLACE_FROM_EMPTY, &[]));
        }
        if from == to {
            return Err(msg::line(msg::REPLACE_SAME, &[]));
        }
        let mut ids: Vec<String> = self.defs.iter().map(|d| d.id.clone()).collect();
        ids.sort();
        // Every file is worked out before one is written.
        let mut found = Vec::new();
        for id in ids {
            let (path, text) = self.net_file(&id, msg::N_OP_REPLACE)?;
            let hits = koyomi_core::replace::hits(&text, from, to);
            if !hits.is_empty() {
                found.push((id, path, text, hits));
            }
        }
        if found.is_empty() {
            return Ok(msg::note(msg::N_REPLACE_NONE, &[from]));
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
            out.push(msg::note(msg::N_REPLACE_PLAN, &[&found.len().to_string(), &lines.to_string()]));
            return Ok(out.join(
                "
",
            ));
        }
        self.held_writable()?;
        let mut next = Vec::new();
        for (id, path, text, _) in &found {
            self.not_running(id)?;
            let new =
                koyomi_core::replace::apply(text, from, to).map_err(|e| msg::line(msg::REPLACE_UNREADABLE, &[id, msg::uncoded(&e)]))?;
            next.push((id.clone(), path.clone(), new));
        }
        let mut written: Vec<&str> = Vec::new();
        for (id, path, new) in &next {
            let (g, n) = split_id(id)?;
            let put = self.keep_version(path, g, n).and_then(|()| ito_common::write_atomic(path, new));
            if let Err(e) = put {
                self.rescan();
                let done = if written.is_empty() {
                    msg::note(msg::N_REPLACE_WROTE_NONE, &[])
                } else {
                    msg::note(msg::N_REPLACE_WROTE, &[&written.join(&msg::note(msg::N_RUN_LIST_JOIN, &[]))])
                };
                return Err(msg::line(msg::REPLACE_WRITE_FAILED, &[id, msg::uncoded(&e), &done]));
            }
            written.push(id);
        }
        self.rescan();
        Ok(msg::note(msg::N_REPLACED, &[&next.len().to_string(), &lines.to_string()]))
    }

    /// See [`Request::Delete`].
    fn delete(&mut self, id: &str) -> Result<String, String> {
        let (path, _) = self.net_file(id, msg::N_OP_DELETE)?;
        self.not_running(id)?;
        self.held_writable()?;
        self.staged_writable()?;
        let by = koyomi_core::rename::referrers(&self.defs, id);
        if !by.is_empty() {
            return Err(self.named_by(id, &by, msg::DELETE_USED));
        }
        let (group, name) = split_id(id)?;
        self.keep_version(&path, group, name)?;
        std::fs::remove_file(&path).map_err(|e| msg::line(msg::FILE_DELETE_FAILED, &[&shown(&path), &e.to_string()]))?;
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
        Ok(msg::note(msg::N_DELETED, &[id]))
    }

    fn answer(&mut self, request: &Request) -> Result<String, String> {
        let (request, by) = match request {
            Request::Over { by, request } if over_http(request) => (&**request, by.as_str()),
            Request::Over { .. } => return Err(msg::line(msg::HTTP_FORBIDDEN, &[])),
            r => (r, ""),
        };
        // Over HTTPS an admin cannot take away their own admin: they would lock themselves out. Another
        // admin can, and so can someone at this PC (`by` is empty there).
        if !by.is_empty() {
            let own = match request {
                Request::UserSet { name, role, .. } => name.trim() == by && role.trim() != "admin",
                Request::UserRemove { name, .. } => name.trim() == by,
                _ => false,
            };
            if own {
                return Err(msg::line(msg::USER_OWN_ADMIN, &[]));
            }
            if matches!(request, Request::UserReset { name, .. } if name.trim() == by) {
                return Err(msg::line(msg::USER_RESET_OWN, &[]));
            }
        }
        let out = self.dispatch(request, by);
        if let Ok(text) = &out {
            if let Some((op, target)) = by_hand(request) {
                // The reply of a new token holds the token itself: it is not written down.
                let detail: String = if matches!(request, Request::UserAdd { .. } | Request::UserReset { .. }) {
                    String::new()
                } else {
                    text.lines().next().unwrap_or_default().chars().take(120).collect()
                };
                self.runner.record_op(op, &target, &detail, by);
            }
        }
        out
    }

    fn dispatch(&mut self, request: &Request, by: &str) -> Result<String, String> {
        match request {
            Request::Status => Ok(self.status()),
            Request::Stop => Ok(msg::note(msg::N_STOP_REPLY, &[])),
            Request::List => Ok(self.snapshot().to_toml()),
            Request::Show { id } => {
                let def = self.defs.iter().find(|d| &d.id == id).ok_or_else(|| msg::line(msg::JOBNET_MISSING, &[id]))?;
                toml::to_string(def).map_err(|e| e.to_string())
            }
            Request::Approve { id, sha256 } => self.approve(id, sha256),
            Request::Run { id, date, from, dry, params } => self.start(id, date.as_deref(), from.as_deref(), *dry, params),
            Request::Rerun { run, from, next } => self.rerun(run, from.as_deref(), *next),
            Request::RunDefinition { run } => self.runner.run_definition(run),
            Request::Kill { run, job } => self.runner.kill(run, job.as_deref()),
            Request::Runs => Ok(Runs { runs: self.runner.today() }.to_toml()),
            Request::History { from, to } => {
                let day = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| msg::line(msg::DATE_FORM_BAD, &[s]));
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
            Request::Standby { give_back: false } => {
                self.standby.status().or_else(|| self.primary.line()).ok_or_else(|| msg::line(msg::NO_STANDBY_OR_PRIMARY, &[]))
            }
            Request::Handover { release } => self.handover(*release),
            Request::Save { id, text, base_sha256, force } => self.save(id, text, base_sha256.as_deref(), *force),
            Request::Versions { id } => self.versions(id),
            Request::VersionText { id, version } => self.version_text(id, version),
            Request::Restore { id, version } => self.restore(id, version),
            Request::Rename { id, to, fix } => self.rename(id, to, *fix),
            Request::Copy { id, to, title } => self.copy(id, to, title.as_deref()),
            Request::Replace { from, to, apply } => self.replace(from, to, *apply),
            Request::Delete { id } => self.delete(id),
            Request::Import { text, dry_run, on_conflict } => self.import(text, *dry_run, on_conflict.as_deref(), by),
            Request::Uses { target } => Ok(koyomi_core::uses::find(&self.defs, target).join(
                "
",
            )),
            Request::Ops { from, to } => {
                let day = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| msg::line(msg::DATE_FORM_BAD, &[s]));
                Ok(self.runner.ops(day(from)?, day(to)?)?.join(
                    "
",
                ))
            }
            Request::Tray { since } => Ok(self.tray_view(*since).to_toml()),
            Request::Notices { boot, since } => Ok(notify::notices_view(boot.as_deref(), *since).to_toml()),
            Request::Propose { id, text, base_sha256 } => self.propose(id, text, base_sha256.as_deref()),
            Request::Stage { id, text, from, expect } => self.stage(id, text, from, expect.as_ref()),
            Request::Unstage { id } => self.unstage(id),
            Request::CalendarText { name } => self.calendar_text(name),
            Request::CalendarPut { name, text, base_sha256 } => self.calendar_put(name, text, base_sha256.as_deref()),
            Request::CalendarRename { from, to } => self.calendar_rename(from, to),
            Request::CalendarDelete { name, base_sha256 } => self.calendar_delete(name, base_sha256),
            Request::HolidayTable { bytes } => self.holiday_table(bytes),
            Request::ExportAll => self.export_all(),
            Request::Users => self.users(),
            Request::ConfigText => self.config_text(),
            Request::ConfigPut { text, base_sha256 } => self.config_put(text, base_sha256, by),
            Request::Registry => self.registry(&read_secret),
            Request::AccountPut { name, user, password, base_sha256 } => {
                let entry = koyomi_core::registry::AccountEntry {
                    name: name.trim().to_owned(),
                    user: user.trim().to_owned(),
                    password: password.trim().to_owned(),
                };
                self.registry_write(base_sha256, false, &|t| koyomi_core::registry::put_account(t, &entry), None, &read_secret)
            }
            Request::AccountRemove { name, base_sha256, force } => self.registry_write(
                base_sha256,
                *force,
                &|t| koyomi_core::registry::remove_account(t, name.trim()),
                Some((true, name.trim())),
                &read_secret,
            ),
            Request::DatabasePut { name, db, password, tls, base_sha256 } => {
                let entry = koyomi_core::registry::DatabaseEntry {
                    name: name.trim().to_owned(),
                    db: db.trim().to_owned(),
                    password: password.as_deref().map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned),
                    tls: *tls,
                };
                self.registry_write(base_sha256, false, &|t| koyomi_core::registry::put_database(t, &entry), None, &read_secret)
            }
            Request::DatabaseRemove { name, base_sha256, force } => self.registry_write(
                base_sha256,
                *force,
                &|t| koyomi_core::registry::remove_database(t, name.trim()),
                Some((false, name.trim())),
                &read_secret,
            ),
            Request::UserSet { name, role, nets, roles, base } => self.user_set(name, role, nets.clone(), roles.clone(), base),
            Request::UserRemove { name, base } => self.user_remove(name, base),
            Request::UserAdd { name, role, nets, roles, base } => self.user_add(name, role, nets.clone(), roles.clone(), base),
            Request::UserReset { name, base } => self.user_reset(name, base),
            Request::Over { .. } => Err(msg::line(msg::HTTP_FORBIDDEN, &[])),
        }
    }
}

/// What may come over HTTP: what its REST paths ask for, and what `POST /door` lets a window of another
/// PC send ([`proto::door_needs`], which asks `admin` of an approval, a save or an import; the REST paths
/// never make those, and a definition comes there as a proposal, left unconfirmed).
fn over_http(request: &Request) -> bool {
    proto::door_needs(request).is_some()
        || matches!(request, Request::List | Request::Show { .. } | Request::Propose { .. } | Request::Handover { .. })
}

/// A secret as the credential store has it.
fn read_secret(name: &str) -> std::io::Result<Option<String>> {
    ito_secret::get("koyomi", name)
}

/// Whether the credential store holds a non-empty token under `name` (ito-http refuses to start with an empty one).
fn secret_usable(name: &str) -> bool {
    matches!(ito_secret::get("koyomi", name), Ok(Some(v)) if !v.is_empty())
}

/// The first `secret:` name of `[http]` (its own token, then each user's) that `usable` says holds no token.
fn unusable_secret<'a>(config: &'a koyomi_core::config::Config, usable: &dyn Fn(&str) -> bool) -> Option<&'a str> {
    let http = config.http.as_ref()?;
    std::iter::once(http.secret()).chain(http.user.iter().map(|u| u.secret())).filter(|n| !n.is_empty()).find(|n| !usable(n))
}

/// Whether writing `new` over `old` would cut the connection `by` is making (a caller signed in to the engine's HTTP door): `[http]` `port`, `lan`,
/// `ipv6` or `tailnet` changed, or their own `[[http.user]]` taken away or changed in any field. An `old`
/// that does not read, or has no `[http]`, cannot be compared and counts as cutting.
fn cuts_link(old: &str, new: &koyomi_core::config::Config, by: &str) -> bool {
    let Some(was) = koyomi_core::config::Config::parse(old).ok().and_then(|c| c.http) else {
        return true;
    };
    let Some(now) = &new.http else {
        return true;
    };
    if (was.port, was.lan, was.ipv6, was.tailnet) != (now.port, now.lan, now.ipv6, now.tailnet) {
        return true;
    }
    let own = |h: &koyomi_core::config::Http| h.user.iter().find(|u| u.name.trim() == by).cloned();
    own(&was) != own(now)
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
        Request::CalendarPut { name, .. } => ("calendar-save", name.clone()),
        Request::CalendarRename { from, to } => ("calendar-rename", format!("{from} {to}")),
        Request::CalendarDelete { name, .. } => ("calendar-delete", name.clone()),
        Request::HolidayTable { .. } => ("holiday-table", String::new()),
        Request::ConfigPut { .. } => ("config-put", String::new()),
        Request::AccountPut { name, .. } => ("account-put", name.trim().to_owned()),
        Request::AccountRemove { name, .. } => ("account-remove", name.trim().to_owned()),
        Request::DatabasePut { name, .. } => ("database-put", name.trim().to_owned()),
        Request::DatabaseRemove { name, .. } => ("database-remove", name.trim().to_owned()),
        Request::UserSet { name, .. } => ("user-set", name.clone()),
        Request::UserRemove { name, .. } => ("user-remove", name.clone()),
        Request::UserAdd { name, .. } => ("user-add", name.trim().to_owned()),
        Request::UserReset { name, .. } => ("user-reset", name.trim().to_owned()),
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
            ito_diff::Item::Folded(n) => msg::note(msg::N_DIFF_FOLDED, &[&n.to_string()]),
        })
        .collect();
    if out.len() > MOST {
        let rest = out.len() - MOST;
        out.truncate(MOST);
        out.push(msg::note(msg::N_DIFF_MORE, &[&rest.to_string()]));
    }
    out
}

/// A folder that cannot be made, as a coded line.
fn dir_failed(dir: &Path, e: &dyn std::fmt::Display) -> String {
    koyomi_core::msg::line(koyomi_core::msg::DIR_CREATE_FAILED, &[&shown(dir), &e.to_string()])
}

/// A file that cannot be written, as a coded line.
fn write_failed(path: &Path, e: &dyn std::fmt::Display) -> String {
    koyomi_core::msg::line(koyomi_core::msg::FILE_WRITE_FAILED, &[&shown(path), &e.to_string()])
}

/// `グループ/名前` as its two parts, or why it is not one.
fn split_id(id: &str) -> Result<(&str, &str), String> {
    if !koyomi_core::jobnet::is_id(id) {
        return Err(koyomi_core::msg::line(koyomi_core::msg::JOBNET_ID_FORM, &[id]));
    }
    id.split_once('/').ok_or_else(|| koyomi_core::msg::line(koyomi_core::msg::JOBNET_ID_FORM, &[id]))
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

/// The words the engine tells a run in (notices, `koyomi status`): the language its messages are in,
/// which [`run`] sets once at start from common.toml (`en`, else Japanese, `auto` too). Not the OS's:
/// under a service with `LANG=en_US.UTF-8` a notice read "✕ ended abnormally … 終了コード 1".
fn engine_words() -> ito_i18n::Table {
    crate::i18n::table(&msg::language()).0
}

/// Runs until `koyomi engine stop`. The exit code: 0 after a stop, 1 when another engine is already up.
/// Without `own_tray` (`koyomi engine --no-tray`) there is no icon here, and the
/// system's notifications and the sounds are kept for `koyomi tray` to show.
pub fn run(own_tray: bool) -> i32 {
    // The engine answers in common.toml's language, else Japanese (not the OS's: a service often runs
    // with `LANG=C`). This overrides the CLI's value; foreground, `--no-tray` and the service all pass here.
    let common = ito_common::base_dir().and_then(|b| ito_common::Common::read(&b).ok()).map(|(c, _)| c).unwrap_or_default();
    koyomi_core::msg::set_language(crate::i18n::engine_language(common.language.as_deref()));
    // SIGTERM and SIGINT (systemd's stop) set a flag the loop below watches, so the engine ends the way
    // `Stop` does and no job's process outlives it. Windows keeps its own ending (a console close or a
    // logoff); the handler is not installed there.
    #[cfg(unix)]
    let asked_to_stop = ito_proc::stop_requested();
    let paths = match Paths::find() {
        Ok(p) => p,
        Err(e) => {
            crate::say(&e, true);
            return 1;
        }
    };
    let door = proto::door();
    if ask(&door, &Request::Status).is_ok() {
        crate::say(&msg::line(msg::ENGINE_ALREADY_RUNNING, &[]), true);
        return 1;
    }
    // The lock the OS drops with the process, so two engines started together cannot both pass.
    let _single = match ito_common::single::acquire("koyomi", "engine") {
        Ok(g) => g,
        Err(held) => {
            crate::say(&msg::line(msg::ENGINE_LOCK_HELD, &[&held.to_string()]), true);
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

    // Before `State::new`, which can take long: the system's notifications go to `koyomi tray` from the start.
    if !own_tray {
        notify::hand_to_tray();
    }
    let mut state = State::new(paths);
    let mut tray = if own_tray { tray::Tray::start(&state.tray_words) } else { None };
    let mut last_scan = Instant::now();
    let mut changed_at: Option<Instant> = None;
    loop {
        let got = rx.recv_timeout(Duration::from_millis(50));
        #[cfg(unix)]
        if asked_to_stop.load(Ordering::Relaxed) {
            state.shut_down();
            return 0;
        }
        match got {
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
                tr.show(&state.tray_words, look);
            }
        }
        if let Ok(false) = mine_rx.try_recv() {
            crate::say(&msg::line(msg::ENGINE_ALREADY_RUNNING, &[]), true);
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

    /// A notice's state word and its reason come in one language: the messages' (tests never set it,
    /// so Japanese), whatever the OS this runs on speaks.
    #[test]
    fn the_engine_tells_a_run_in_its_messages_language() {
        assert_eq!(msg::language(), "ja");
        let words = engine_words();
        assert_eq!(crate::show::word(&words, koyomi_core::run::State::Error), "異常終了");
        assert_eq!(msg::note(msg::N_RUN_EXIT, &["1"]), "終了コード 1");
    }

    #[test]
    fn warnings_are_joined_without_their_ids() {
        let a = msg::line(msg::OS_NO_PWSH, &["経理/a"]);
        let b = msg::line(msg::OS_RUN_AS, &["経理/a"]);
        let joined = joined_warnings(&[a.clone(), b.clone()]);
        assert!(!msg::is_coded(&joined) && !joined.contains("KOYOMI-"), "{joined}");
        assert!(joined.contains(msg::uncoded(&a)) && joined.contains(msg::uncoded(&b)), "{joined}");
        assert_eq!(joined_warnings(&[]), "");
    }

    #[test]
    fn a_failed_import_keeps_a_coded_reason_and_wraps_a_plain_one() {
        let coded = msg::line(msg::IMPORT_EMPTY, &[]);
        assert_eq!(import_failed(coded.clone()), coded);
        let wrapped = import_failed("bad toml".to_owned());
        assert!(msg::IMPORT_FAILED.is_in(&wrapped) && wrapped.contains("bad toml"), "{wrapped}");
    }

    /// over HTTP only what its paths ask for goes through; never an approval or a save.
    #[test]
    fn only_the_http_paths_come_over_http() {
        assert!(over_http(&Request::Status) && over_http(&Request::Runs));
        assert!(over_http(&Request::Run { id: "g/a".into(), date: None, from: None, dry: false, params: Default::default() }));
        assert!(over_http(&Request::Kill { run: "20261031-090000-01".into(), job: None }));
        assert!(over_http(&Request::Rerun { run: "20261031-090000-01".into(), from: None, next: None }));
        assert!(over_http(&Request::Hold { id: "g/a".into(), job: None }));
        assert!(over_http(&Request::SkipNext { id: "g/a".into(), cancel: false }));
        // What `POST /door` lets an admin send comes too (the REST paths never make these requests).
        assert!(over_http(&Request::Approve { id: "g/a".into(), sha256: String::new() }));
        assert!(over_http(&Request::Pause) && over_http(&Request::Delete { id: "g/a".into() }));
        // What is for this PC alone never does.
        assert!(!over_http(&Request::Stop));
        assert!(!over_http(&Request::Save { id: "g/a".into(), text: String::new(), base_sha256: None, force: true }));
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

    /// A run that never began reruns from the start as a new run of the same day, not from the middle, and a
    /// held net stays refused; a run that began and lost its copy is refused with the path.
    #[test]
    fn a_run_that_never_began_reruns_from_the_start() {
        let paths = temp_paths("rerun-never-began");
        let mut s = State::new(paths.clone());
        let net = "format = 1\n\n[[job]]\nid = \"a\"\nrun = [\"koyomi-no-such-program\"]\n";
        s.save("経理/集計", net, None, false).unwrap();
        let def = s.defs.iter().find(|d| d.id == "経理/集計").cloned().unwrap();
        let planned = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap().and_hms_opt(9, 0, 0).unwrap();
        let missed = s.runner.not_run(&def, planned, RunState::Missed, "past its time").unwrap();
        // From the middle: refused with the new code, and no run is made.
        for (from, next) in [(Some("a"), None), (None, Some(RunState::Skipped)), (Some("a"), Some(RunState::Skipped))] {
            let e = s.rerun(&missed, from, next).unwrap_err();
            assert!(msg::RERUN_NEVER_BEGAN_PARTIAL.is_in(&e), "{e}");
        }
        // From the start: a new run of the same business day, started by hand.
        let again = s.rerun(&missed, None, None).unwrap();
        assert_ne!(again, missed);
        let run = s.runner.previous(&again).unwrap();
        assert_eq!((run.date.as_str(), run.how, run.jobnet.as_str()), ("2026-10-29", How::Manual, "経理/集計"));
        for _ in 0..300 {
            s.runner.tick();
            if s.runner.previous(&again).unwrap().is_over() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // A held net stays refused.
        s.hold("経理/集計").unwrap();
        let e = s.rerun(&missed, None, None).unwrap_err();
        assert!(msg::RUN_HELD.is_in(&e), "{e}");
        s.release("経理/集計").unwrap();
        // A run that began but whose copy is gone: the new code, with the path.
        let copy = s.runner.log_path(&again, "a").parent().unwrap().join("definition.toml");
        assert_eq!(s.runner.run_definition(&again).unwrap(), net);
        std::fs::remove_file(&copy).unwrap();
        let e = s.rerun(&again, None, None).unwrap_err();
        assert!(msg::RERUN_COPY_UNREADABLE.is_in(&e) && e.contains("definition.toml"), "{e}");
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// `Request::RunDefinition` answers the text of the copy as written, and says why when there is none.
    #[test]
    fn a_run_definition_is_the_copy_and_says_why_when_there_is_none() {
        let paths = temp_paths("run-definition");
        let mut s = State::new(paths.clone());
        let net = "format = 1\n\n[[job]]\nid = \"a\"\nrun = [\"koyomi-no-such-program\"]\nenv = { K = \"secret:token\" }\n";
        s.save("経理/集計", net, None, false).unwrap();
        let def = s.defs.iter().find(|d| d.id == "経理/集計").cloned().unwrap();
        let planned = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap().and_hms_opt(9, 0, 0).unwrap();
        let missed = s.runner.not_run(&def, planned, RunState::Missed, "past its time").unwrap();
        let ask = |s: &mut State, run: &str| s.answer(&Request::RunDefinition { run: run.to_owned() });
        let e = ask(&mut s, &missed).unwrap_err();
        assert!(msg::RUN_DEFINITION_NEVER_BEGAN.is_in(&e), "{e}");
        let e = ask(&mut s, "20200101-000000-00").unwrap_err();
        assert!(msg::RUN_NOT_FOUND.is_in(&e), "{e}");
        let id = s.runner.start_manual(&def, planned.date(), None, &BTreeMap::new()).unwrap();
        // The reply is the file's text, the reference to the secret as it was written.
        let copy = s.runner.log_path(&id, "a").parent().unwrap().join("definition.toml");
        let text = ask(&mut s, &id).unwrap();
        assert_eq!(text, std::fs::read_to_string(&copy).unwrap());
        assert!(text.contains("secret:token"));
        std::fs::remove_file(&copy).unwrap();
        let e = ask(&mut s, &id).unwrap_err();
        assert!(msg::RUN_DEFINITION_GONE.is_in(&e) && e.contains("definition.toml"), "{e}");
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// Every start records the calendar the net counts by, as the real scan resolved it (the group's
    /// `_group.toml` calendar for a net that names none); a net with none records every.
    #[test]
    fn a_start_records_the_calendar_the_run_went_by() {
        use koyomi_core::run::WentBy;
        let paths = temp_paths("went-by");
        let mut s = State::new(paths.clone());
        let net = "format = 1\n\n[[job]]\nid = \"a\"\nrun = [\"koyomi-no-such-program\"]\n";
        // The calendar and the group's file are on disk, and the engine's own scan hands the runner the table.
        std::fs::write(paths.calendars.join("会社.toml"), "format = 1\nholidays = \"japan\"\n").unwrap();
        std::fs::create_dir_all(paths.jobnets.join("経理")).unwrap();
        std::fs::write(paths.jobnets.join("経理").join("_group.toml"), "format = 1\ncalendar = \"会社\"\n").unwrap();
        s.rescan();
        s.save("経理/集計", net, None, false).unwrap();
        s.save("人事/月次", net, None, false).unwrap();
        s.rescan();
        assert_eq!(s.defs.iter().find(|d| d.id == "経理/集計").and_then(|d| d.calendar.as_deref()), Some("会社"));
        let day = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
        for (net, want) in [("経理/集計", WentBy::Calendar("会社".to_owned())), ("人事/月次", WentBy::Every)] {
            let def = s.defs.iter().find(|d| d.id == net).cloned().unwrap();
            let manual = s.runner.start_manual(&def, day, None, &BTreeMap::new()).unwrap();
            assert_eq!(s.runner.previous(&manual).unwrap().went_by, Some(want.clone()), "{net}");
            let missed = s.runner.not_run(&def, day.and_hms_opt(9, 0, 0).unwrap(), RunState::Missed, "x").unwrap();
            assert_eq!(s.runner.previous(&missed).unwrap().went_by, Some(want));
        }
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
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
        let e = s.hold_job("経理/集計", "nope").unwrap_err();
        assert!(msg::NET_JOB_MISSING.is_in(&e) && e.contains("nope"), "{e}");
        assert!(s.hold_job("経理/無い", "a").is_err());
        assert!(s.hold_job("経理/集計", "a").unwrap().contains("取込"));
        assert!(msg::HOLD_JOB_ALREADY.is_in(&s.hold_job("経理/集計", "a").unwrap_err()));
        let back = Held::parse(&std::fs::read_to_string(&paths.held).unwrap()).unwrap();
        assert!(back.is_job_held("経理/集計", "a"));
        assert!(!back.is_held("経理/集計"), "holding a job does not hold the net");
        assert!(s.release_job("経理/集計", "a").is_ok());
        assert!(msg::HOLD_JOB_NOT.is_in(&s.release_job("経理/集計", "a").unwrap_err()));
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
        let plan = |s: &mut State, dry: bool, on: Option<&str>| proto::Plan::parse(&s.import(&text, dry, on, "").unwrap()).unwrap();
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
        assert!(msg::IMPORT_EMPTY.is_in(
            &s.import(
                "format = 1
",
                false,
                Some("overwrite"),
                ""
            )
            .unwrap_err()
        ));
        let bad = s.import("nope", false, None, "").unwrap_err();
        assert!(msg::is_coded(&bad), "{bad}");
        let mut broken = bundle::Bundle::new();
        broken.jobnets.push(bundle::Net {
            id: "経理/親".to_owned(),
            text: "format = 1\n[[job]]\nid = \"k\"\nkind = \"jobnet\"\nuse = \"無い/子\"\n".to_owned(),
        });
        let p = proto::Plan::parse(&s.import(&broken.to_toml(), false, Some("overwrite"), "").unwrap()).unwrap();
        assert!(!p.applied && p.problems == 1, "{p:?}");
        assert!(!paths.jobnets.join("経理/親.toml").exists());
        assert!(s.import(&text, false, Some("sideways"), "").is_err());
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
        let p = proto::Plan::parse(&s.import(&b.to_toml(), false, Some("rename"), "").unwrap()).unwrap();
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
        let not = s.skip_next("経理/夜", true).unwrap_err();
        assert!(msg::SKIP_NOT.is_in(&not) && not.contains("経理/夜"), "{not}");
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
            let r = s.import(bad, dry, over, "").unwrap();
            let plan = proto::Plan::parse(&r).unwrap();
            assert!(plan.problems > 0 && !plan.applied, "{r}");
            assert!(
                plan.lines
                    .iter()
                    .any(|l| l.trim_start().strip_prefix("問題:").is_some_and(|r| msg::BUNDLE_SETTINGS_BAD.is_in(r.trim_start()))),
                "{r}"
            );
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
        // (`approve` scans first, which would recompute the injected warning away.)
        let e = s.approve_scanned("経理/甲", &a).unwrap_err();
        assert!(msg::APPROVE_BLOCKED.is_in(&e) && e.contains("Everyone"), "{e}");
        // The CLI/window pre-check asks the same question and gives the same answer.
        assert!(!warnings_about(&paths, &s.warnings, &s.defs, "経理/甲.toml").is_empty());
        assert!(warnings_about(&paths, &s.warnings, &s.defs, "経理/乙.toml").is_empty());
        assert_eq!(s.approve("経理/乙", &b), Ok(msg::note(msg::N_NET_APPROVED, &["経理/乙"])));
        assert_eq!(standing_of(&s, "経理/乙"), Some(Standing::Confirmed));
        assert_ne!(standing_of(&s, "経理/甲"), Some(Standing::Confirmed));
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    #[test]
    fn a_when_that_cannot_come_is_a_caution_and_approving_goes_on() {
        let paths = temp_paths("when-caution");
        let mut s = State::new(paths.clone());
        // Someone else's lock: the engine a person runs never sees it (or no session is seen at all).
        let other = r#"format = 1
name = "他人"

[[when]]
session = "lock"
user = 'NOPE\nobody'

[[job]]
id = "a"
run = ["x.exe"]
"#;
        let plain = "format = 1\nname = \"n\"\n\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";
        assert_eq!(s.save("保守/他人", other, None, false), Ok(msg::note(msg::N_SAVED, &["保守/他人"])));
        s.propose("保守/乙", plain, None).unwrap();
        let snap = s.snapshot();
        assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
        // Where the OS gives no sign-in and lock events (not Windows) the caution says that nothing is watched instead.
        let caution = if cfg!(windows) { koyomi_core::msg::SESSION_USER_HIDDEN } else { koyomi_core::msg::SESSION_WATCH_FAILED };
        assert!(snap.cautions.iter().any(|c| caution.is_in(c) && c.contains("保守/他人")), "{:?}", snap.cautions);
        // The window and the CLI ask the snapshot's warnings before they approve: nothing stands in the way.
        assert!(warnings_about(&paths, &snap.warnings, &snap.defs, "保守/乙.toml").is_empty());
        let sha = s.defs.iter().find(|d| d.id == "保守/乙").unwrap().sha256.clone();
        assert_eq!(s.approve("保守/乙", &sha), Ok(msg::note(msg::N_NET_APPROVED, &["保守/乙"])));
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
        assert_eq!(s.save("経理/集計", one, None, false), Ok(msg::note(msg::N_SAVED, &["経理/集計"])));
        let file = paths.jobnets.join("経理").join("集計.toml");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), one);
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Confirmed));
        assert_eq!(Versions::parse(&s.versions("経理/集計").unwrap()).unwrap().versions.len(), 0);
        // Saved again on a stale base: refused; on the right base: the old file is kept.
        let hash = approved::hash(one.as_bytes());
        assert_eq!(
            s.save("経理/集計", two, Some("stale"), false),
            Err(koyomi_core::msg::line(koyomi_core::msg::SAVE_CHANGED_ELSEWHERE, &["経理/集計"]))
        );
        assert_eq!(
            s.save("経理/集計", two, None, false),
            Err(koyomi_core::msg::line(koyomi_core::msg::SAVE_CHANGED_ELSEWHERE, &["経理/集計"]))
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), one);
        assert!(s.save("経理/集計", two, Some(&hash), false).is_ok());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), two);
        let versions = Versions::parse(&s.versions("経理/集計").unwrap()).unwrap().versions;
        assert_eq!(versions.len(), 1);
        assert_eq!(std::fs::read_to_string(paths.state.join("versions/経理/集計").join(format!("{}.toml", versions[0]))).unwrap(), one);
        // What does not read is not saved, whatever the base.
        let err = s.save("経理/集計", "format = 1\n[[job]]\nid = \"a\"\n", Some("x"), true).unwrap_err();
        assert!(koyomi_core::msg::DEF_INVALID_SAVE.is_in(&err) && err.contains("経理/集計"), "{err}");
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
        assert_eq!(s.restore("経理/集計", "20000101-000000"), Err(msg::line(msg::VERSION_MISSING, &["経理/集計", "20000101-000000"])));
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
        assert_eq!(
            s.propose("経理/集計", two, Some("stale")),
            Err(koyomi_core::msg::line(koyomi_core::msg::SAVE_CHANGED_ELSEWHERE, &["経理/集計"]))
        );
        assert!(s.propose("経理/集計", two, Some(&hash)).is_ok());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), two);
        assert_eq!(standing_of(&s, "経理/集計"), Some(Standing::Unconfirmed));
        assert_eq!(Versions::parse(&s.versions("経理/集計").unwrap()).unwrap().versions.len(), 2);
        // What does not read is not written.
        let two_hash = approved::hash(two.as_bytes());
        assert!(s.propose("経理/集計", "format = 1\n[[job]]\nid = \"a\"\n", Some(&two_hash)).unwrap_err().contains("KOYOMI-E3106"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), two);
        // It comes over HTTP; a forced save does not (an approval only through the door, for an admin).
        assert!(over_http(&Request::Propose { id: "経理/集計".into(), text: two.into(), base_sha256: None }));
        assert!(!over_http(&Request::Save { id: "経理/集計".into(), text: two.into(), base_sha256: None, force: true }));
        assert_eq!(proto::door_needs(&Request::Approve { id: "経理/集計".into(), sha256: two_hash }), Some(koyomi_core::role::Role::Admin));
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    #[test]
    fn a_reference_that_cannot_be_followed_keeps_its_id_and_names_the_referrer() {
        let paths = temp_paths("rename-unfollowed");
        let mut s = State::new(paths.clone());
        let parent = "format = 1
name = \"親\"
[[job]]
id = \"a\"
run = [\"x.exe\"]
";
        // The child names the parent in a string with escapes, which the rewrite does not follow.
        let child = "format = 1\nname = \"子\"\n[[job]]\nid = \"k\"\nkind = \"jobnet\"\nuse = \"\\u7d4c\\u7406/\\u89aa\"\n";
        assert!(s.save("経理/親", parent, None, false).is_ok());
        assert!(s.save("経理/子", child, None, false).is_ok());
        let err = s.rename("経理/親", "総務/親", true).unwrap_err();
        assert!(msg::RETARGET_UNFOLLOWED.is_in(&err), "{err}");
        let head = msg::line(msg::RETARGET_UNFOLLOWED, &["経理/親"]);
        assert_eq!(err, msg::note(msg::N_RENAMED_REFS_FAILED, &[&head, "経理/子"]));
        assert!(paths.jobnets.join("経理/親.toml").exists() && !paths.jobnets.join("総務/親.toml").exists());
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
        assert!(msg::RENAME_USED.is_in(&err) && err.contains("child net  経理/子"), "{err}");
        assert!(paths.jobnets.join("経理/親.toml").exists());
        assert_eq!(s.rename("経理/親", "経理/子", true), Err(msg::line(msg::JOBNET_EXISTS, &["経理/子"])));
        assert_eq!(s.rename("経理/無い", "経理/新", true), Err(msg::line(msg::JOBNET_MISSING, &["経理/無い"])));
        assert!(s.rename("経理/親", "総務", true).is_err());
        // With fix: moved, the user pointed at it, both still confirmed, the hold carried.
        let said = s.rename("経理/親", "総務/親", true).unwrap();
        assert!(said.contains(&msg::note(msg::N_RENAMED_REFS, &["経理/子"])), "{said}");
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
        assert_eq!(s.copy("総務/親", "経理/子", None), Err(msg::line(msg::JOBNET_EXISTS, &["経理/子"])));
        // A delete is refused while another net names it, and kept as a version when not.
        assert!(msg::DELETE_USED.is_in(&s.delete("総務/親").unwrap_err()));
        assert!(s.delete("経理/子").unwrap().contains("削除しました"));
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

    /// One line, one ID: a net naming a missing calendar is refused with E3106 at the head and no second ID inside.
    #[test]
    fn a_refused_save_carries_one_id_only() {
        let paths = temp_paths("one-id");
        let mut s = State::new(paths);
        let text = "format = 1
calendar = \"無い\"
[[job]]
id = \"a\"
run = [\"x.exe\"]
";
        let err = s.save("経理/締め", text, None, false).unwrap_err();
        assert!(koyomi_core::msg::DEF_INVALID_SAVE.is_in(&err), "{err}");
        assert_eq!(err.matches("KOYOMI-").count(), 1, "{err}");
        assert!(err.contains("無い"), "{err}");
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
        assert!(s.stage_on("経理/締め", next, today, today).unwrap_err().contains("KOYOMI-E3108"));
        assert!(s.stage_on("経理/無い", next, day("2026-12-01"), today).unwrap_err().contains("KOYOMI-E3110"));
        assert!(s.stage_on("経理/締め", now, day("2026-12-01"), today).unwrap_err().contains("KOYOMI-E3112"));
        let err = s.stage_on("経理/締め", "format = [\n", day("2026-12-01"), today).unwrap_err();
        assert!(koyomi_core::msg::DEF_INVALID_STAGE.is_in(&err), "{err}");
        assert!(s.stage("経理/締め", next, "12/1", None).unwrap_err().contains("KOYOMI-E3109"));
        assert!(!paths.next.exists());
        // Staged: kept in next.toml and shown; the net runs as it is.
        assert!(s.stage_on("経理/締め", next, day("2026-12-01"), today).unwrap() == msg::note(msg::N_STAGED, &["2026-12-01", "経理/締め"]));
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
        assert!(
            line.contains(&msg::note(msg::N_SWITCHED_UNCONFIRMED, &[])) && line.contains(&msg::note(msg::N_SWITCHED_CHANGED, &[])),
            "{line}"
        );
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/a.toml")).unwrap(), net("a3.exe"));
        assert_eq!(standing_of(&s, "経理/a"), Some(Standing::Unconfirmed));
        s.switch_staged(day("2026-12-01"));
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/b.toml")).unwrap(), net("b.exe"));
        assert_eq!(standing_of(&s, "経理/b"), Some(Standing::Confirmed));
        assert!(s.staged.get("経理/b").is_some());
        let cautions = s.snapshot().cautions;
        assert!(cautions.iter().any(|c| msg::SWITCH_FAILED.is_in(c) && c.contains("経理/b")), "{cautions:?}");
        // Dropped: the net runs as it is, and the caution goes.
        assert!(s.unstage("経理/b").unwrap().contains("取り消しました"));
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
        assert!(
            !line.contains(&msg::note(msg::N_SWITCHED_CHANGED, &[])) && !line.contains(&msg::note(msg::N_SWITCHED_UNCONFIRMED, &[])),
            "{line}"
        );
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
        assert_eq!(s.replace("", "x", false), Err(msg::line(msg::REPLACE_FROM_EMPTY, &[])));
        assert_eq!(s.replace("x", "x", true), Err(msg::line(msg::REPLACE_SAME, &[])));
        assert_eq!(s.replace("//nowhere", "x", true), Ok(msg::note(msg::N_REPLACE_NONE, &["//nowhere"])));
        // Shown, not written.
        let shown = s.replace("//old", "//new", false).unwrap();
        assert!(
            shown.contains("経理/一") && shown.contains("5: + run = [\"//new/tool.exe\", \"-v\"]") && !shown.contains("経理/二"),
            "{shown}"
        );
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/一.toml")).unwrap(), one);
        // A result that does not read is refused, and nothing is written.
        let err = s.replace("\"a\"", "a", true).unwrap_err();
        assert!(msg::REPLACE_UNREADABLE.is_in(&err) && err.contains("経理/一"), "{err}");
        assert_eq!(std::fs::read_to_string(paths.jobnets.join("経理/二.toml")).unwrap(), two);
        // Applied: written, the old form kept, unconfirmed again; the other net stays confirmed.
        let said = s.replace("//old", "//new", true).unwrap();
        assert!(said.contains(&msg::note(msg::N_REPLACED, &["1", "1"])), "{said}");
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
            blocked: vec!["共通/x".into()],
            utc_offset_min: 540,
            shells: vec![proto::Shell { name: "bash".into(), path: "/bin/bash".into() }],
            shell: "bash".into(),
            accounts: vec!["batch".into()],
            databases: vec![proto::DatabaseName { name: "sales".into(), kind: "sqlserver".into() }],
            windows: Some(true),
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

    /// W1: a stage made by a window of another PC says what it saw; one over a change it did not see is refused.
    #[test]
    fn a_stage_made_over_another_persons_change_is_refused() {
        let paths = temp_paths("stage-expect");
        let mut s = State::new(paths.clone());
        let net = |run: &str| format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"{run}\"]\n");
        assert!(s.save("経理/a", &net("a.exe"), None, false).is_ok());
        let today = day("2026-11-02");
        let live = approved::hash(net("a.exe").as_bytes());
        let seen = |live: &str, staged: Option<String>| proto::StageExpect { live: live.to_owned(), staged };
        let stage =
            |s: &mut State, text: String, from: &str, e: &proto::StageExpect| s.stage_expecting("経理/a", &text, day(from), today, Some(e));
        // The live definition changed after the window read it.
        let stale = stage(&mut s, net("a2.exe"), "2026-12-01", &seen(&approved::hash(b"old"), None)).unwrap_err();
        assert!(koyomi_core::msg::STAGE_CHANGED_ELSEWHERE.is_in(&stale), "{stale}");
        assert!(!paths.next.exists());
        // Nothing staged when it saw one, and one staged when it saw none.
        let ghost = Some("0".repeat(64));
        assert!(stage(&mut s, net("a2.exe"), "2026-12-01", &seen(&live, ghost)).unwrap_err().contains("KOYOMI-E3111"));
        // The first goes in; the second, who saw nothing staged, is refused.
        assert!(stage(&mut s, net("a2.exe"), "2026-12-01", &seen(&live, None)).is_ok());
        let second = stage(&mut s, net("a3.exe"), "2026-12-05", &seen(&live, None)).unwrap_err();
        assert!(koyomi_core::msg::STAGE_CHANGED_ELSEWHERE.is_in(&second), "{second}");
        assert_eq!(s.staged.get("経理/a").unwrap().text, net("a2.exe"));
        // Saw it, and replaces it.
        let first = s.staged.get("経理/a").unwrap().clone();
        assert!(stage(&mut s, net("a3.exe"), "2026-12-05", &seen(&live, Some(first.seal()))).is_ok());
        // The same text with only the day moved is another seal, so a window that saw the old day is refused.
        let second = s.staged.get("経理/a").unwrap().clone();
        assert_ne!(first.seal(), second.seal());
        assert!(stage(&mut s, net("a3.exe"), "2026-12-06", &seen(&live, Some(first.seal()))).is_err());
        assert!(stage(&mut s, net("a3.exe"), "2026-12-06", &seen(&live, Some(second.seal()))).is_ok());
        // A staged text changed by hand in next.toml, the recorded hash left: the seal moves, so it is refused.
        let before = s.staged.get("経理/a").unwrap().clone();
        let mut st = koyomi_core::next::Store::load(&paths.next).unwrap();
        st.next[0].text = net("a4.exe");
        std::fs::write(&paths.next, st.to_toml()).unwrap();
        s.rescan();
        assert_ne!(s.staged.get("経理/a").unwrap().seal(), before.seal());
        assert!(stage(&mut s, net("a5.exe"), "2026-12-07", &seen(&live, Some(before.seal()))).is_err());
        // Without `expect` (the CLI) nothing is compared.
        assert!(s.stage_on("経理/a", &net("a6.exe"), day("2026-12-08"), today).is_ok());
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// W1: a calendar is put, renamed and deleted by the engine; the old text stays under `versions/_calendars/`.
    #[test]
    fn a_calendar_is_put_renamed_and_deleted_with_its_versions_kept() {
        let paths = temp_paths("calendar-ops");
        let mut s = State::new(paths.clone());
        let one = "format = 1\nholidays = \"japan\"\nweekly_off = [\"sat\", \"sun\"]\n";
        let two = "format = 1\nholidays = \"none\"\nweekly_off = [\"sun\"]\n";
        let versions = |name: &str| std::fs::read_dir(paths.state.join("versions/_calendars").join(name)).map(|d| d.count()).unwrap_or(0);
        // A new one: no base, and nothing is written over.
        assert!(s.calendar_put("会社", one, None).is_ok());
        assert_eq!(std::fs::read_to_string(paths.calendars.join("会社.toml")).unwrap(), one);
        assert!(s.calendars.get("会社").is_some(), "read again after the change");
        assert!(s.calendar_put("会社", two, None).unwrap_err().contains("KOYOMI-E4101"));
        let body = proto::CalendarBody::parse(&s.calendar_text("会社").unwrap()).unwrap();
        assert_eq!((body.text.as_str(), body.sha256.as_str()), (one, approved::hash(one.as_bytes()).as_str()));
        // A replace needs the hash it saw; the old text is kept.
        assert!(s.calendar_put("会社", two, Some(&approved::hash(b"other"))).unwrap_err().contains("KOYOMI-E4105"));
        assert_eq!(versions("会社"), 0);
        assert!(s.calendar_put("会社", two, Some(&body.sha256)).is_ok());
        assert_eq!(versions("会社"), 1);
        let only = std::fs::read_dir(paths.state.join("versions/_calendars/会社")).unwrap().next().unwrap().unwrap().path();
        assert_eq!(std::fs::read(&only).unwrap(), one.as_bytes(), "the kept copy is the old file's bytes");
        assert!(s.calendar_put("会社", "format = [\n", Some(&approved::hash(two.as_bytes()))).is_err());
        // A rename does not take over a calendar there.
        assert!(s.calendar_put("工場", one, None).is_ok());
        assert!(s.calendar_rename("会社", "工場").unwrap_err().contains("KOYOMI-E4101"));
        assert!(s.calendar_rename("無い", "新").unwrap_err().contains("KOYOMI-E4104"));
        assert!(s.calendar_rename("会社", "本社").is_ok());
        assert!(!paths.calendars.join("会社.toml").exists() && paths.calendars.join("本社.toml").exists());
        assert!(s.calendars.get("本社").is_some() && s.calendars.get("会社").is_none());
        // A delete needs the hash too, and keeps the file.
        assert!(s.calendar_delete("本社", &approved::hash(b"other")).unwrap_err().contains("KOYOMI-E4105"));
        assert!(paths.calendars.join("本社.toml").exists());
        assert!(s.calendar_delete("本社", &approved::hash(two.as_bytes())).is_ok());
        assert!(!paths.calendars.join("本社.toml").exists());
        assert_eq!(versions("本社"), 1);
        assert!(s.calendar_delete("本社", &approved::hash(two.as_bytes())).unwrap_err().contains("KOYOMI-E4104"));
        // A name that is not a file name is refused whatever the request.
        for bad in ["../x", "a/b", "a\\b", "", "  ", "..", "a."] {
            assert!(s.calendar_put(bad, one, None).is_err(), "{bad:?}");
            assert!(s.calendar_text(bad).is_err(), "{bad:?}");
            assert!(s.calendar_rename("工場", bad).is_err(), "{bad:?}");
            assert!(s.calendar_delete(bad, "x").is_err(), "{bad:?}");
        }
        assert!(!paths.app.join("x.toml").exists() && !paths.calendars.join("..").join("x.toml").exists());
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// W1: the holiday table is written as it came, over the one there (kept), and only when it reads.
    #[test]
    fn a_holiday_table_replaces_the_one_there_and_keeps_it() {
        let paths = temp_paths("holiday-table");
        let mut s = State::new(paths.clone());
        let csv = "国民の祝日・休日月日,国民の祝日・休日名称\r\n2026/1/1,元日\r\n2026/1/12,成人の日\r\n";
        let file = paths.calendars.join(koyomi_core::calendar::TABLE_FILE);
        assert!(s.holiday_table(b"not a table").is_err() && !file.exists());
        assert!(s.holiday_table(csv.as_bytes()).is_ok());
        assert_eq!(std::fs::read(&file).unwrap(), csv.as_bytes());
        let kept = paths.state.join("versions/_calendars/syukujitsu");
        assert!(!kept.exists());
        let more = format!("{csv}2026/2/11,建国記念の日\r\n");
        assert!(s.holiday_table(more.as_bytes()).is_ok());
        assert_eq!(std::fs::read(&file).unwrap(), more.as_bytes());
        assert_eq!(std::fs::read_dir(&kept).unwrap().count(), 1);
        let only = std::fs::read_dir(&kept).unwrap().next().unwrap().unwrap().path();
        assert_eq!(std::fs::read(&only).unwrap(), csv.as_bytes(), "the kept copy is the old table's bytes");
        // A Shift_JIS table (as the Cabinet Office writes it) is accepted and written as it came.
        let sjis = b"2028/1/1,\x8c\xb3\x93\xfa\r\n";
        assert!(s.holiday_table(sjis).is_ok());
        assert_eq!(std::fs::read(&file).unwrap(), sjis);
        assert_eq!(std::fs::read_dir(&kept).unwrap().count(), 2, "the table it replaced was kept too");
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// The next free name of a version is `stamp`, then `stamp-2`, `stamp-3`.
    #[test]
    fn free_version_skips_the_names_that_are_taken() {
        let paths = temp_paths("free-version");
        let dir = paths.app.join("v");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_version(&dir, "20260101-000000", "toml"), "20260101-000000");
        std::fs::write(dir.join("20260101-000000.toml"), "").unwrap();
        assert_eq!(free_version(&dir, "20260101-000000", "toml"), "20260101-000000-2");
        std::fs::write(dir.join("20260101-000000-2.toml"), "").unwrap();
        assert_eq!(free_version(&dir, "20260101-000000", "toml"), "20260101-000000-3");
        assert_eq!(free_version(&dir, "20260101-000000", "csv"), "20260101-000000", "another extension is another name");
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// A file that cannot be read is not "no file": the copy is not skipped, and nothing is created.
    #[test]
    fn a_file_that_cannot_be_read_stops_the_keeping_and_creates_nothing() {
        let paths = temp_paths("keep-unreadable");
        let s = State::new(paths.clone());
        let dir = paths.state.join("versions/x");
        let not_a_file = paths.app.join("is-a-folder.toml");
        std::fs::create_dir_all(&not_a_file).unwrap();
        let e = s.keep_copy(&not_a_file, &dir, "toml").unwrap_err();
        assert!(koyomi_core::msg::FILE_READ_FAILED.is_in(&e) && e.contains("is-a-folder.toml"), "{e}");
        assert!(!dir.exists(), "nothing is created in the versions folder");
        // No file at all is nothing to keep.
        assert!(s.keep_copy(&paths.app.join("none.toml"), &dir, "toml").is_ok());
        assert!(!dir.exists());
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// W1: the snapshot carries the nets a warning bears on and the engine's UTC offset.
    #[test]
    fn the_snapshot_names_the_blocked_nets_and_the_engines_offset() {
        let paths = temp_paths("blocked");
        let mut s = State::new(paths.clone());
        let net = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"a.exe\"]\n";
        assert!(s.save("経理/a", net, None, false).is_ok());
        assert!(s.save("経理/b", net, None, false).is_ok());
        let snap = s.snapshot();
        assert!(snap.blocked.is_empty());
        assert_eq!(snap.utc_offset_min, ito_common::now::now_local().offset().local_minus_utc() / 60);
        // A warning about one net's file blocks that net and not the other.
        s.warnings.push(format!("{} は書き換えられない", shown(&paths.jobnets.join("経理").join("a.toml"))));
        assert_eq!(s.snapshot().blocked, vec!["経理/a".to_owned()]);
        // A warning about the shared files blocks every net.
        s.warnings.push("共有のフォルダーに書けない".to_owned());
        assert_eq!(s.snapshot().blocked.len(), 2);
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// The snapshot says which shells this PC has and the default one; the cautions name a shell the engine lacks
    /// (one line per net and shell, on every OS), and the shell `koyomi.toml` wrote when it is missing.
    #[test]
    fn the_snapshot_lists_the_shells_and_the_cautions_name_the_missing_ones() {
        let paths = temp_paths("shells");
        let mut s = State::new(paths.clone());
        let here = |name: &str| PathBuf::from(if cfg!(windows) { "C:/x" } else { "/x" }).join(name);
        s.shells = vec![("bash".into(), here("bash")), ("sh".into(), here("sh"))];
        let net = "format = 1
[[job]]
id = \"a\"
shell = \"zsh\"
script = \"x\"
[[job]]
id = \"b\"
shell = \"zsh\"
script = \"y\"
[[job]]
id = \"c\"
shell = \"bash\"
script = \"z\"
[[job]]
id = \"d\"
shell = \"pwsh\"
script = \"w\"
";
        s.save("経理/a", net, None, false).unwrap();
        s.shells = vec![("bash".into(), here("bash")), ("sh".into(), here("sh"))];
        let snap = s.snapshot();
        assert_eq!(snap.shells.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["bash", "sh"]);
        let lines: Vec<&String> = snap.cautions.iter().filter(|c| msg::OS_NO_SHELL.is_in(c)).collect();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].contains("経理/a") && lines[0].contains("zsh") && lines[0].contains(" a "), "{}", lines[0]);
        assert!(lines[1].contains("pwsh"), "{}", lines[1]);
        // Not a warning: the net can still be confirmed.
        assert!(!msg::OS_NO_SHELL.is_warning() || snap.warnings.iter().all(|w| !msg::OS_NO_SHELL.is_in(w)));
        // The default: not written → the first found (bash, as bash and sh are found); written and missing → a caution and that one.
        let os = "bash";
        assert_eq!(snap.shell, os);
        assert!(!snap.cautions.iter().any(|c| msg::SHELL_DEFAULT_MISSING.is_in(c)));
        s.shell_written = Some("zsh".into());
        let snap = s.snapshot();
        assert_eq!(snap.shell, os);
        assert!(snap.cautions.iter().any(|c| msg::SHELL_DEFAULT_MISSING.is_in(c) && c.contains("zsh")), "{:?}", snap.cautions);
        s.shell_written = Some("sh".into());
        let snap = s.snapshot();
        assert_eq!(snap.shell, "sh");
        assert!(!snap.cautions.iter().any(|c| msg::SHELL_DEFAULT_MISSING.is_in(c)));
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// A `run_as` or a `database` that koyomi.toml does not register is a caution of its net (W3754, W3755) on
    /// every OS, and not once it is registered.
    #[test]
    fn the_cautions_name_an_account_or_a_database_that_is_not_registered() {
        let paths = temp_paths("unregistered");
        let mut s = State::new(paths.clone());
        let net = "format = 1
[[job]]
id = \"a\"
run = [\"x.exe\"]
run_as = \"ops\"
[[job]]
id = \"q\"
kind = \"sql\"
sql = \"select 1\"
database = \"main\"
";
        s.save("経理/a", net, None, false).unwrap();
        let snap = s.snapshot();
        let acc: Vec<&String> = snap.cautions.iter().filter(|c| msg::OS_ACCOUNT_UNKNOWN.is_in(c)).collect();
        let dbs: Vec<&String> = snap.cautions.iter().filter(|c| msg::OS_DATABASE_UNKNOWN.is_in(c)).collect();
        assert_eq!((acc.len(), dbs.len()), (1, 1), "{:?}", snap.cautions);
        assert!(acc[0].contains("経理/a") && acc[0].contains("ops"), "{}", acc[0]);
        assert!(dbs[0].contains("main"), "{}", dbs[0]);
        // Not a warning: the net can still be confirmed.
        assert!(snap.warnings.iter().all(|w| !msg::OS_ACCOUNT_UNKNOWN.is_in(w) && !msg::OS_DATABASE_UNKNOWN.is_in(w)));
        let config = Config::parse(
            "[[account]]
name = \"ops\"
user = \"batch\"
password = \"secret:ops\"
[[database]]
name = \"main\"
db = \"sqlite://C:\\\\data\\\\a.db\"
",
        )
        .unwrap();
        s.runner.set_config(config);
        let snap = s.snapshot();
        assert!(
            !snap.cautions.iter().any(|c| msg::OS_ACCOUNT_UNKNOWN.is_in(c) || msg::OS_DATABASE_UNKNOWN.is_in(c)),
            "{:?}",
            snap.cautions
        );
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// W1: what a window of another PC does is written down under the caller's name, and what the
    /// door lets through is let through `Over`.
    #[test]
    fn a_door_request_is_recorded_under_the_caller_and_the_door_set_comes_in() {
        let paths = temp_paths("door-by");
        let mut s = State::new(paths.clone());
        let over = |by: &str, request: Request| Request::Over { by: by.to_owned(), request: Box::new(request) };
        assert!(s.answer(&over("ito", Request::Pause)).is_ok());
        let today = ito_common::now::now_local().date_naive();
        let ops = s.runner.ops(today, today).unwrap().join("\n");
        assert!(ops.contains("pause") && ops.contains("ito"), "{ops}");
        // The set `POST /door` passes is let in; `Stop` never is.
        assert!(s.answer(&over("ito", Request::Resume)).is_ok());
        assert!(s.answer(&over("ito", Request::Stop)).is_err());
        let net = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"a.exe\"]\n";
        let save = Request::Save { id: "経理/a".into(), text: net.into(), base_sha256: None, force: false };
        assert!(s.answer(&over("root", save)).is_ok());
        assert!(s.runner.ops(today, today).unwrap().join("\n").contains("save"));
        let put = Request::CalendarPut { name: "会社".into(), text: "format = 1\n".into(), base_sha256: None };
        assert!(s.answer(&over("root", put)).is_ok());
        assert!(s.runner.ops(today, today).unwrap().join("\n").contains("calendar-save"));
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// The users are listed, changed and removed through the door; a stale base and an admin taking
    /// away their own admin over HTTPS are refused, and each change is written down.
    #[test]
    fn the_users_are_changed_through_the_door() {
        use koyomi_core::users::{Listing, Token};
        let paths = temp_paths("door-users");
        std::fs::create_dir_all(&paths.app).unwrap();
        let file = "# settings
[http]
port = 8700
tailnet = 8701

[[http.user]]
name = \"boss\"
role = \"admin\"
tailscale = \"boss@example.com\"

[[http.user]]
name = \"ann\"
role = \"run\"
tailscale = \"ann@example.com\"
nets = [\"a/*\"]
";
        std::fs::write(&paths.config, file).unwrap();
        let mut s = State::new(paths.clone());
        let over = |by: &str, request: Request| Request::Over { by: by.to_owned(), request: Box::new(request) };
        let listing = |s: &mut State| Listing::parse(&s.answer(&Request::Users).unwrap()).unwrap();
        let first = listing(&mut s);
        assert_eq!(first.user.len(), 2);
        assert!(first.user.iter().all(|u| u.token == Token::None));
        // A stale base is refused and nothing is written.
        let set = |base: &str, name: &str, role: &str| Request::UserSet {
            name: name.into(),
            role: role.into(),
            nets: None,
            roles: None,
            base: base.into(),
        };
        assert!(s.answer(&set("0000", "ann", "edit")).is_err());
        assert_eq!(std::fs::read_to_string(&paths.config).unwrap(), file);
        // Someone at this PC (no `by`) changes a role; the rest of the file stays as it was.
        assert!(s.answer(&set(&first.base, "ann", "edit")).is_ok());
        let text = std::fs::read_to_string(&paths.config).unwrap();
        assert!(
            text.starts_with(
                "# settings
[http]
port = 8700
"
            ) && text.contains("role = \"edit\""),
            "{text}"
        );
        assert!(!text.contains("nets"), "{text}");
        // Over HTTPS an admin cannot take away their own admin, nor remove themselves.
        let base = listing(&mut s).base;
        assert!(s.answer(&over("boss", set(&base, "boss", "run"))).is_err());
        assert!(s.answer(&over("boss", Request::UserRemove { name: "boss".into(), base: base.clone() })).is_err());
        assert!(s.answer(&over("boss", set(&base, "boss", "admin"))).is_ok());
        // Another admin can; the last admin cannot go.
        let base = listing(&mut s).base;
        assert!(s.answer(&over("ann", set(&base, "ann", "admin"))).is_ok());
        let base = listing(&mut s).base;
        assert!(s.answer(&over("ann", Request::UserRemove { name: "boss".into(), base })).is_ok());
        let base = listing(&mut s).base;
        assert_eq!(listing(&mut s).user.len(), 1);
        assert!(s.answer(&Request::UserRemove { name: "ann".into(), base }).is_err());
        let today = ito_common::now::now_local().date_naive();
        let ops = s.runner.ops(today, today).unwrap().join(
            "
",
        );
        assert!(ops.contains("user-set") && ops.contains("user-remove"), "{ops}");
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// koyomi.toml of a test: two users by hash, no credential store needed.
    fn config_file(port: u16, boss_role: &str) -> String {
        let hash = ito_http::token_hash("one");
        format!(
            "[http]\nport = {port}\nlan = true\n\n[[http.user]]\nname = \"boss\"\nrole = \"{boss_role}\"\ntoken = \"sha256:{hash}\"\n\n[[http.user]]\nname = \"ann\"\nrole = \"run\"\ntoken = \"sha256:{hash}\"\n"
        )
    }

    fn config_base(s: &mut State) -> String {
        proto::ConfigBody::parse(&s.answer(&Request::ConfigText).unwrap()).unwrap().sha256
    }

    fn config_put_over(by: &str, text: &str, base: &str) -> Request {
        let put = Request::ConfigPut { text: text.to_owned(), base_sha256: base.to_owned() };
        if by.is_empty() {
            put
        } else {
            Request::Over { by: by.to_owned(), request: Box::new(put) }
        }
    }

    /// Accounts and databases are written through the door, one entry at a time: the rest of the file stays,
    /// a removal that a net still names is refused without `force`, and the snapshot lists names only.
    #[test]
    fn accounts_and_databases_are_written_one_entry_at_a_time() {
        let paths = temp_paths("door-registry");
        std::fs::create_dir_all(paths.jobnets.join("g")).unwrap();
        std::fs::write(
            paths.jobnets.join("g").join("a.toml"),
            "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\nrun_as = \"batch\"\n[[job]]\nid = \"q\"\nkind = \"sql\"\ndatabase = \"sales\"\nsql = \"select 1\"\n",
        )
        .unwrap();
        let mut s = State::new(paths.clone());
        let head = "# keep me\nconcurrency = 2\n";
        std::fs::write(&paths.config, head).unwrap();
        let get = |n: &str| -> std::io::Result<Option<String>> {
            match n {
                "batch" => Ok(Some("pw".into())),
                "gone" => Ok(None),
                _ => Err(std::io::Error::other("no")),
            }
        };
        let body = |s: &State| proto::RegistryBody::parse(&s.registry(&get).unwrap()).unwrap();
        let base = body(&s).sha256;
        let put_account = |s: &mut State, base: &str| {
            s.registry_write(
                base,
                false,
                &|t| {
                    koyomi_core::registry::put_account(
                        t,
                        &koyomi_core::registry::AccountEntry {
                            name: "batch".into(),
                            user: "CORP\\b".into(),
                            password: "secret:batch".into(),
                        },
                    )
                },
                None,
                &get,
            )
        };
        // A stale base writes nothing.
        let e = put_account(&mut s, "0000").unwrap_err();
        assert!(koyomi_core::msg::CONFIG_CHANGED.is_in(&e), "{e}");
        assert_eq!(std::fs::read_to_string(&paths.config).unwrap(), head);
        let reply = proto::RegistryBody::parse(&put_account(&mut s, &base).unwrap()).unwrap();
        assert!(std::fs::read_to_string(&paths.config).unwrap().starts_with(head));
        let a = &reply.accounts[0];
        assert_eq!(
            (a.name.as_str(), a.secret.as_str(), a.state.as_str(), a.used_by.clone()),
            ("batch", "batch", "ok", vec!["g/a".to_owned()])
        );
        // The runner took the new settings up at once.
        assert_eq!(s.runner.config().account.len(), 1);
        let snap = Snapshot::parse(&s.snapshot().to_toml()).unwrap();
        assert_eq!((snap.accounts, snap.windows), (vec!["batch".to_owned()], Some(cfg!(windows))));
        // A database with a missing secret, and one without a password.
        let put_db = |s: &mut State, base: &str, name: &str, db: &str, pw: Option<&str>| {
            s.registry_write(
                base,
                false,
                &|t| {
                    koyomi_core::registry::put_database(
                        t,
                        &koyomi_core::registry::DatabaseEntry { name: name.into(), db: db.into(), password: pw.map(Into::into), tls: None },
                    )
                },
                None,
                &get,
            )
        };
        let r =
            proto::RegistryBody::parse(&put_db(&mut s, &reply.sha256, "sales", "sqlserver://ops@db01/sales", Some("secret:gone")).unwrap())
                .unwrap();
        let r = proto::RegistryBody::parse(&put_db(&mut s, &r.sha256, "plain", "sqlite://C:\\data\\a.db", None).unwrap()).unwrap();
        let states: Vec<(&str, &str)> = r.databases.iter().map(|d| (d.name.as_str(), d.state.as_str())).collect();
        assert_eq!(states, [("sales", "missing"), ("plain", "not-needed")]);
        assert_eq!((r.databases[0].kind.as_str(), r.databases[0].used_by.clone()), ("sqlserver", vec!["g/a".to_owned()]));
        // A name a net uses is not removed without force; an unused one is.
        let remove = |s: &mut State, base: &str, account: bool, name: &str, force: bool| {
            s.registry_write(
                base,
                force,
                &|t| if account { koyomi_core::registry::remove_account(t, name) } else { koyomi_core::registry::remove_database(t, name) },
                Some((account, name)),
                &get,
            )
        };
        let e = remove(&mut s, &r.sha256, true, "batch", false).unwrap_err();
        assert!(koyomi_core::msg::REGISTRY_IN_USE.is_in(&e) && e.contains("g/a"), "{e}");
        assert!(remove(&mut s, &r.sha256, false, "plain", false).is_ok());
        let base = body(&s).sha256;
        let r = proto::RegistryBody::parse(&remove(&mut s, &base, true, "batch", true).unwrap()).unwrap();
        assert!(r.accounts.is_empty());
        let e = remove(&mut s, &r.sha256, true, "batch", true).unwrap_err();
        assert!(koyomi_core::msg::REGISTRY_NAME_UNKNOWN.is_in(&e), "{e}");
        // No password is ever in a reply.
        assert!(!s.registry(&get).unwrap().contains("pw"));
        let text = std::fs::read_to_string(&paths.config).unwrap();
        assert!(text.contains("# keep me") && text.contains("concurrency = 2"), "{text}");
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// koyomi.toml is read with its hash (a missing one as the empty text), written back only against that
    /// hash, and the old text is kept under `versions/_settings/`.
    #[test]
    fn koyomi_toml_is_read_and_written_through_the_door() {
        let paths = temp_paths("door-config");
        std::fs::create_dir_all(&paths.app).unwrap();
        let mut s = State::new(paths.clone());
        let body = proto::ConfigBody::parse(&s.answer(&Request::ConfigText).unwrap()).unwrap();
        assert_eq!((body.text.as_str(), body.sha256.as_str()), ("", koyomi_core::users::sha256("").as_str()));
        // A new file from nothing.
        let first = config_file(8700, "admin");
        s.answer(&config_put_over("", &first, &body.sha256)).unwrap();
        assert_eq!(std::fs::read_to_string(&paths.config).unwrap(), first);
        let body = proto::ConfigBody::parse(&s.answer(&Request::ConfigText).unwrap()).unwrap();
        assert_eq!((body.text.as_str(), body.sha256.as_str()), (first.as_str(), koyomi_core::users::sha256(&first).as_str()));
        // The same hash `Users` lists.
        let listing = koyomi_core::users::Listing::parse(&s.answer(&Request::Users).unwrap()).unwrap();
        assert_eq!(listing.base, body.sha256);
        // A wrong hash writes nothing.
        let second = config_file(8700, "admin") + "# edited\n";
        let e = s.answer(&config_put_over("", &second, "0000")).unwrap_err();
        assert!(koyomi_core::msg::CONFIG_CHANGED.is_in(&e), "{e}");
        assert_eq!(std::fs::read_to_string(&paths.config).unwrap(), first);
        // The right one writes, and keeps the old text.
        let note = s.answer(&config_put_over("", &second, &body.sha256)).unwrap();
        assert!(!note.is_empty());
        assert_eq!(std::fs::read_to_string(&paths.config).unwrap(), second);
        let kept: Vec<_> = std::fs::read_dir(paths.state.join("versions").join("_settings")).unwrap().flatten().collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(std::fs::read_to_string(kept[0].path()).unwrap(), first);
        // A text that does not read is refused, as it is, and nothing changes.
        let base = config_base(&mut s);
        let e = s.answer(&config_put_over("", "[http\nport =", &base)).unwrap_err();
        assert_eq!(e, koyomi_core::config::Config::parse("[http\nport =").unwrap_err());
        assert_eq!(std::fs::read_to_string(&paths.config).unwrap(), second);
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// For a caller signed in to the HTTP door (`by` non-empty) what would cut the connection making the change is refused; the same change from this PC goes through.
    #[test]
    fn a_change_that_cuts_the_https_connection_is_refused_over_https_only() {
        let paths = temp_paths("door-config-cut");
        std::fs::create_dir_all(&paths.app).unwrap();
        let file = config_file(8700, "admin");
        std::fs::write(&paths.config, &file).unwrap();
        let mut s = State::new(paths.clone());
        let hash = ito_http::token_hash("one");
        let other = ito_http::token_hash("two");
        let boss = format!("name = \"boss\"\nrole = \"admin\"\ntoken = \"sha256:{hash}\"");
        let cuts = [
            file.replace("port = 8700", "port = 8701"),
            file.replace("lan = true", "lan = false"),
            file.replace("lan = true", "lan = true\nipv6 = true"),
            file.replacen("role = \"admin\"", "role = \"edit\"", 1),
            file.replace(&boss, &format!("name = \"boss\"\nrole = \"admin\"\ntoken = \"sha256:{other}\"")),
            file.replace(&boss, &format!("name = \"chief\"\nrole = \"admin\"\ntoken = \"sha256:{hash}\"")),
        ];
        for text in &cuts {
            assert_ne!(text, &file);
            let base = config_base(&mut s);
            let e = s.answer(&config_put_over("boss", text, &base)).unwrap_err();
            assert!(koyomi_core::msg::CONFIG_CUTS_LINK.is_in(&e), "{e}");
            assert_eq!(std::fs::read_to_string(&paths.config).unwrap(), file);
        }
        // What is not the link may change over HTTPS: another user's role.
        let ann_to_view = file.replace("role = \"run\"", "role = \"view\"");
        let base = config_base(&mut s);
        s.answer(&config_put_over("boss", &ann_to_view, &base)).unwrap();
        // What cut the link goes through from this PC.
        let moved = ann_to_view.replace("port = 8700", "port = 8701");
        let base = config_base(&mut s);
        s.answer(&config_put_over("", &moved, &base)).unwrap();
        assert_eq!(std::fs::read_to_string(&paths.config).unwrap(), moved);
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// A bundle whose `[settings]` is `config`.
    fn settings_bundle(config: &str) -> String {
        let mut t = toml::Table::new();
        t.insert("format".to_owned(), toml::Value::Integer(1));
        t.insert("settings".to_owned(), toml::Value::Table(toml::from_str(config).unwrap()));
        toml::to_string(&t).unwrap()
    }

    /// An import is held to what `ConfigPut` holds: for a caller signed in to the HTTP door it may not cut the
    /// connection or name a secret that holds no token, and nothing is written; from this PC it goes through.
    #[test]
    fn an_import_of_koyomi_toml_is_guarded_like_a_config_put() {
        let paths = temp_paths("import-config");
        std::fs::create_dir_all(&paths.app).unwrap();
        let file = config_file(8700, "admin");
        std::fs::write(&paths.config, &file).unwrap();
        let mut s = State::new(paths.clone());
        let moved = settings_bundle(&file.replace("port = 8700", "port = 8701"));
        let r = s.import_with(&moved, false, Some("overwrite"), "boss", |_| true).unwrap();
        let plan = proto::Plan::parse(&r).unwrap();
        assert!(plan.problems > 0 && !plan.applied, "{r}");
        assert!(plan.lines.iter().any(|l| l.contains("E2249")), "{r}");
        assert_eq!(std::fs::read_to_string(&paths.config).unwrap(), file);
        // A secret that holds no token.
        let named = file.clone()
            + "
[[http.user]]
name = \"zed\"
role = \"run\"
token = \"secret:zed\"
";
        let r = s.import_with(&settings_bundle(&named), false, Some("overwrite"), "boss", |_| false).unwrap();
        let plan = proto::Plan::parse(&r).unwrap();
        assert!(plan.problems > 0 && !plan.applied, "{r}");
        assert!(plan.lines.iter().any(|l| l.contains("E2248") && l.contains("zed")), "{r}");
        assert_eq!(std::fs::read_to_string(&paths.config).unwrap(), file);
        // The same import from this PC goes through.
        let r = s.import_with(&moved, false, Some("overwrite"), "", |_| true).unwrap();
        assert!(proto::Plan::parse(&r).unwrap().applied, "{r}");
        assert!(std::fs::read_to_string(&paths.config).unwrap().contains("8701"));
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// A `secret:` token that cannot be read, or reads empty, is refused; one that reads is let through.
    #[test]
    fn a_secret_token_that_does_not_read_is_refused() {
        let paths = temp_paths("door-config-secret");
        std::fs::create_dir_all(&paths.app).unwrap();
        let s = State::new(paths.clone());
        let base = koyomi_core::users::sha256("");
        let shared = "[http]\nport = 8700\ntoken = \"secret:shared\"\n";
        let e = s.config_put_with(shared, &base, "", |_| false).unwrap_err();
        assert!(koyomi_core::msg::CONFIG_SECRET.is_in(&e) && e.contains("shared"), "{e}");
        assert!(!paths.config.exists());
        let user = "[http]\nport = 8700\n\n[[http.user]]\nname = \"ann\"\nrole = \"run\"\ntoken = \"secret:ann\"\n";
        let e = s.config_put_with(user, &base, "", |name| name != "ann").unwrap_err();
        assert!(koyomi_core::msg::CONFIG_SECRET.is_in(&e) && e.contains("ann"), "{e}");
        assert!(!paths.config.exists());
        s.config_put_with(user, &base, "", |name| name == "ann").unwrap();
        assert_eq!(std::fs::read_to_string(&paths.config).unwrap(), user);
        // A user who needs no token needs no store.
        let tail =
            "[http]\nport = 8700\ntailnet = 8701\n\n[[http.user]]\nname = \"ann\"\nrole = \"run\"\ntailscale = \"ann@example.com\"\n";
        s.config_put_with(tail, &koyomi_core::users::sha256(user), "", |_| false).unwrap();
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// A user is added and given a new token through the door: the token is in the reply and the file
    /// holds only its hash, and neither the ops record nor the kept old text holds the token.
    #[test]
    fn a_user_is_added_and_given_a_new_token_through_the_door() {
        use koyomi_core::users::{Listing, Token};
        let paths = temp_paths("door-users-add");
        std::fs::create_dir_all(&paths.app).unwrap();
        let mut s = State::new(paths.clone());
        let over = |by: &str, request: Request| Request::Over { by: by.to_owned(), request: Box::new(request) };
        let listing = |s: &mut State| Listing::parse(&s.answer(&Request::Users).unwrap()).unwrap();
        // No koyomi.toml yet: an empty list.
        let first = listing(&mut s);
        assert!(first.user.is_empty());
        let add = |base: &str, name: &str, role: &str| Request::UserAdd {
            name: name.into(),
            role: role.into(),
            nets: None,
            roles: None,
            base: base.into(),
        };
        assert!(s.answer(&add("0000", "boss", "admin")).is_err());
        // Without an [http] there is nothing to add to: refused, and no file is made.
        let e = s.answer(&add(&first.base, "boss", "admin")).unwrap_err();
        assert!(koyomi_core::msg::USER_NO_HTTP.is_in(&e), "{e}");
        assert!(!paths.config.exists());
        std::fs::write(&paths.config, "[http]\nport = 8700\ntoken = \"secret:t\"\n").unwrap();
        let first = listing(&mut s);
        let reply: toml::Table = s.answer(&add(&first.base, "boss", "admin")).unwrap().parse().unwrap();
        let token1 = reply["token"].as_str().unwrap().to_owned();
        assert_eq!(reply["name"].as_str(), Some("boss"));
        assert_eq!(token1.len(), 64);
        let text = std::fs::read_to_string(&paths.config).unwrap();
        assert!(text.contains(&format!("sha256:{}", ito_http::token_hash(&token1))), "{text}");
        assert!(!text.contains(&token1));
        let now = listing(&mut s);
        assert_eq!(now.user.len(), 1);
        assert_eq!(now.user[0].token, Token::Hash);
        // A name that is there is refused; a second user is added after the first.
        assert!(s.answer(&add(&now.base, "boss", "view")).is_err());
        let reply: toml::Table = s.answer(&add(&now.base, "ann", "view")).unwrap().parse().unwrap();
        let token2 = reply["token"].as_str().unwrap().to_owned();
        assert_ne!(token1, token2);
        // Over HTTPS nobody makes their own token again; another admin can.
        let base = listing(&mut s).base;
        assert!(s.answer(&over("boss", Request::UserReset { name: "boss".into(), base: base.clone() })).is_err());
        let reply: toml::Table = s.answer(&over("boss", Request::UserReset { name: "ann".into(), base })).unwrap().parse().unwrap();
        let token3 = reply["token"].as_str().unwrap().to_owned();
        let base = listing(&mut s).base;
        assert!(s.answer(&Request::UserReset { name: "nobody".into(), base }).is_err());
        let text = std::fs::read_to_string(&paths.config).unwrap();
        assert!(text.contains(&ito_http::token_hash(&token3)) && !text.contains(&ito_http::token_hash(&token2)), "{text}");
        // The old text was kept; the token is in no record.
        let kept = paths.state.join("versions").join("_settings");
        assert!(std::fs::read_dir(&kept).unwrap().count() >= 1);
        let today = ito_common::now::now_local().date_naive();
        let ops = s.runner.ops(today, today).unwrap().join("\n");
        assert!(ops.contains("user-add") && ops.contains("user-reset"), "{ops}");
        for t in [&token1, &token2, &token3] {
            assert!(!ops.contains(t.as_str()));
        }
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// W1: a restore, a copy and a replace leave the net they write unconfirmed, never confirmed.
    #[test]
    fn restore_copy_and_replace_leave_the_net_unconfirmed() {
        let paths = temp_paths("unconfirmed");
        let mut s = State::new(paths.clone());
        let one = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"one.exe\"]\n";
        let two = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"two.exe\"]\n";
        assert!(s.save("経理/a", one, None, false).is_ok());
        let live = |p: &Paths| approved::hash(&std::fs::read(p.jobnets.join("経理").join("a.toml")).unwrap());
        assert!(s.save("経理/a", two, Some(&live(&paths)), false).is_ok());
        assert_eq!(standing_of(&s, "経理/a"), Some(Standing::Confirmed));
        let version = Versions::parse(&s.versions("経理/a").unwrap()).unwrap().versions[0].clone();
        assert!(s.restore("経理/a", &version).is_ok());
        assert_eq!(standing_of(&s, "経理/a"), Some(Standing::Unconfirmed));
        let other = if std::fs::read_to_string(paths.jobnets.join("経理").join("a.toml")).unwrap() == two { one } else { two };
        assert!(s.save("経理/a", other, Some(&live(&paths)), false).is_ok());
        assert_eq!(standing_of(&s, "経理/a"), Some(Standing::Confirmed));
        assert!(s.copy("経理/a", "経理/b", None).is_ok());
        assert_eq!(standing_of(&s, "経理/b"), Some(Standing::Unconfirmed));
        assert_eq!(standing_of(&s, "経理/a"), Some(Standing::Confirmed));
        assert!(s.replace(if other == two { "two.exe" } else { "one.exe" }, "three.exe", true).is_ok());
        assert_eq!(standing_of(&s, "経理/a"), Some(Standing::Unconfirmed));
        assert_eq!(standing_of(&s, "経理/b"), Some(Standing::Unconfirmed));
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// F1: `next.toml` edited by hand after the window read it, with no rescan since, is seen by `stage`.
    #[test]
    fn a_stage_sees_a_hand_edit_of_next_toml_made_since_the_last_scan() {
        let paths = temp_paths("stage-fresh");
        let mut s = State::new(paths.clone());
        let net = |run: &str| format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"{run}\"]\n");
        assert!(s.save("経理/a", &net("a.exe"), None, false).is_ok());
        let today = day("2026-11-02");
        let live = approved::hash(net("a.exe").as_bytes());
        let seen = |staged: Option<String>| proto::StageExpect { live: live.clone(), staged };
        assert!(s.stage_expecting("経理/a", &net("a2.exe"), day("2026-12-01"), today, Some(&seen(None))).is_ok());
        let first = s.staged.get("経理/a").unwrap().seal();
        // A hand edit; no `rescan()`.
        let mut st = koyomi_core::next::Store::load(&paths.next).unwrap();
        st.next[0].text = net("a3.exe");
        std::fs::write(&paths.next, st.to_toml()).unwrap();
        let err = s.stage_expecting("経理/a", &net("a4.exe"), day("2026-12-02"), today, Some(&seen(Some(first.clone())))).unwrap_err();
        assert!(koyomi_core::msg::STAGE_CHANGED_ELSEWHERE.is_in(&err), "{err}");
        assert_eq!(std::fs::read_to_string(&paths.next).unwrap(), st.to_toml());
        // `next.toml` that no longer reads: refused with and without `expect`, the file untouched.
        std::fs::write(&paths.next, "next = [\n").unwrap();
        let broken = std::fs::read(&paths.next).unwrap();
        assert!(s.stage_expecting("経理/a", &net("a5.exe"), day("2026-12-03"), today, Some(&seen(Some(first)))).is_err());
        assert!(s.stage_expecting("経理/a", &net("a5.exe"), day("2026-12-03"), today, None).is_err());
        assert_eq!(std::fs::read(&paths.next).unwrap(), broken);
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// F2: an approval is checked against the file as it is, not as the last scan saw it.
    #[test]
    fn an_approval_is_refused_when_the_file_changed_since_the_hash_was_read() {
        let paths = temp_paths("approve-fresh");
        let mut s = State::new(paths.clone());
        let net = |run: &str| format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"{run}\"]\n");
        s.propose("経理/a", &net("a.exe"), None).unwrap();
        let sha = s.defs.iter().find(|d| d.id == "経理/a").unwrap().sha256.clone();
        let before = s.store.to_toml();
        // Changed by hand; no `rescan()`.
        std::fs::write(paths.jobnets.join("経理").join("a.toml"), net("b.exe")).unwrap();
        let err = s.approve("経理/a", &sha).unwrap_err();
        assert!(msg::APPROVE_CHANGED.is_in(&err), "{err}");
        assert_eq!(s.store.to_toml(), before);
        assert_eq!(standing_of(&s, "経理/a"), Some(Standing::Unconfirmed));
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// F3: a case-only rename works; on a case-sensitive file system it never replaces another file.
    #[test]
    fn a_case_only_calendar_rename_works_and_never_replaces_another_file() {
        let paths = temp_paths("cal-case");
        let mut s = State::new(paths.clone());
        let one = "format = 1\nholidays = \"none\"\n";
        assert!(s.calendar_put("Office", one, None).is_ok());
        assert!(s.calendar_rename("Office", "office").is_ok());
        let names: Vec<String> =
            std::fs::read_dir(&paths.calendars).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().any(|n| n == "office.toml") && !names.iter().any(|n| n == "Office.toml"), "{names:?}");
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn a_calendar_rename_does_not_replace_a_file_that_differs_only_in_case() {
        let paths = temp_paths("cal-case-unix");
        let mut s = State::new(paths.clone());
        let one = "format = 1\nholidays = \"none\"\n";
        let two = "format = 1\nholidays = \"japan\"\n";
        assert!(s.calendar_put("Office", one, None).is_ok());
        assert!(s.calendar_put("office", two, None).is_ok());
        let err = s.calendar_rename("Office", "office").unwrap_err();
        assert!(koyomi_core::msg::CALENDAR_EXISTS.is_in(&err), "{err}");
        assert_eq!(std::fs::read_to_string(paths.calendars.join("Office.toml")).unwrap(), one);
        assert_eq!(std::fs::read_to_string(paths.calendars.join("office.toml")).unwrap(), two);
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// F4: a calendar that cannot be read is not taken for a missing one.
    #[test]
    fn a_calendar_that_cannot_be_read_is_not_written_over() {
        let paths = temp_paths("cal-unreadable");
        let mut s = State::new(paths.clone());
        std::fs::create_dir_all(paths.calendars.join("x.toml")).unwrap();
        let err = s.calendar_put("x", "format = 1\n", None).unwrap_err();
        assert!(koyomi_core::msg::FILE_READ_FAILED.is_in(&err) && err.contains("x.toml"), "{err}");
        assert!(paths.calendars.join("x.toml").is_dir());
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// F5: the name check is the calendar module's.
    #[test]
    fn a_calendar_name_is_checked_by_the_calendar_modules_rule() {
        let paths = temp_paths("cal-names");
        let mut s = State::new(paths.clone());
        let one = "format = 1\nholidays = \"none\"\n";
        assert!(s.calendar_put("工場", one, None).is_ok());
        for bad in ["CON", "..", "../x", "a/b", ""] {
            assert!(s.calendar_text(bad).is_err(), "text {bad:?}");
            assert!(s.calendar_put(bad, one, None).is_err(), "put {bad:?}");
            assert!(s.calendar_rename("工場", bad).is_err(), "rename to {bad:?}");
            assert!(s.calendar_rename(bad, "新").is_err(), "rename from {bad:?}");
            assert!(s.calendar_delete(bad, "x").is_err(), "delete {bad:?}");
        }
        assert!(paths.calendars.join("工場.toml").exists());
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// F6: a restore to what is approved says so.
    #[test]
    fn a_restore_to_the_approved_text_says_it_is_confirmed() {
        let paths = temp_paths("restore-reply");
        let mut s = State::new(paths.clone());
        let one = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"one.exe\"]\n";
        let two = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"two.exe\"]\n";
        assert!(s.save("経理/a", one, None, false).is_ok());
        assert!(s.propose("経理/a", two, Some(&approved::hash(one.as_bytes()))).is_ok());
        assert_eq!(standing_of(&s, "経理/a"), Some(Standing::Unconfirmed));
        let version = Versions::parse(&s.versions("経理/a").unwrap()).unwrap().versions[0].clone();
        let reply = s.restore("経理/a", &version).unwrap();
        assert_eq!(reply, msg::note(msg::N_RESTORED_SAME, &[&version, "経理/a"]));
        assert_eq!(standing_of(&s, "経理/a"), Some(Standing::Confirmed));
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }

    /// F7: an approval that comes over the door is checked as one from the window is, and is written
    /// down under the caller's name.
    #[test]
    fn an_approval_over_the_door_is_checked_and_recorded_under_the_caller() {
        let paths = temp_paths("door-approve");
        let mut s = State::new(paths.clone());
        let net = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"a.exe\"]\n";
        s.propose("経理/a", net, None).unwrap();
        let sha = s.defs.iter().find(|d| d.id == "経理/a").unwrap().sha256.clone();
        let over = |sha: &str| Request::Over {
            by: "alice".into(),
            request: Box::new(Request::Approve { id: "経理/a".into(), sha256: sha.into() }),
        };
        let today = ito_common::now::now_local().date_naive();
        let before = s.store.to_toml();
        assert!(s.answer(&over(&approved::hash(b"other"))).is_err());
        assert_eq!(s.store.to_toml(), before);
        assert_eq!(standing_of(&s, "経理/a"), Some(Standing::Unconfirmed));
        assert!(!s.runner.ops(today, today).unwrap().join("\n").contains("approve"));
        assert!(s.answer(&over(&sha)).is_ok());
        assert_eq!(standing_of(&s, "経理/a"), Some(Standing::Confirmed));
        let ops = s.runner.ops(today, today).unwrap().join("\n");
        assert!(ops.contains("approve") && ops.contains("alice"), "{ops}");
        let _ = std::fs::remove_dir_all(paths.app.parent().unwrap());
    }
}
