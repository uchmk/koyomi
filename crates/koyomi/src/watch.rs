//! The window watches every `[[remote]]` engine in the background, whichever PC it shows.
//!
//! One thread per connection asks the engine's `Notices` (told here as this PC's own notifications, so the person
//! hears of a failure on any PC while the window is open) and, for the PCs the window is not on, today's `Runs`
//! (to count the failures nobody has dealt with) or just `GET /health` when the window cannot be driven with the
//! token. The window's status-bar list and one status-bar item show what the threads found out. This PC's own
//! engine is not watched here: its notices are the tray's.
//!
//! Watching is not lossless. The engine keeps the last 50 notices, and the first ask of a thread only learns
//! where the engine is, so what was kept before the window started, or before a connection was added or renamed
//! (a rename is a new name, so a new thread and a fresh cursor), is not told. A link switch and a URL change
//! never start a thread over: its cursor lives as long as the window.

use std::collections::{BTreeSet, HashMap};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use koyomi_core::role::Role;
use koyomi_core::run::Run;

use crate::link::Link;
use crate::notify::NoticesAt;
use crate::proto::{self, Note, Request, Runs};

/// How often a PC the window is not on is asked.
pub const WATCH_EVERY: Duration = Duration::from_secs(15);
/// How long a PC that does not answer (or cannot be used) is left alone.
pub const WATCH_RETRY: Duration = Duration::from_secs(60);
/// How often a PC the window is not on is asked for today's runs.
pub const RUNS_EVERY: Duration = Duration::from_secs(60);
/// The longest the `GET /health` that tells "refused" from "no answer" may take.
const HEALTH_DEADLINE: Duration = Duration::from_secs(5);

/// What the watch found out about a `[[remote]]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seen {
    Answering,
    /// The PC answers `GET /health` but the window cannot use it (the token, the role, the version); why.
    Refused(String),
    /// The PC does not answer; why.
    NoAnswer(String),
}

/// One connection's state for the status bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fact {
    pub seen: Seen,
    /// Today's failures nobody has dealt with there; `None` while not known (and for the PC the window is on,
    /// where the browser has its own).
    pub unacked: Option<usize>,
}

/// What a thread sends to the UI.
#[derive(Debug)]
enum Event {
    /// The thread's id says whose (see `Handle::id`).
    Fact(Fact),
    Notes(Box<Notes>),
}

/// Notices a PC's engine kept that are new to this window, with this PC's `[notify]` to tell them with.
#[derive(Debug)]
pub struct Notes {
    pub name: String,
    pub notes: Vec<Note>,
    pub cfg: ito_prefs::NotifyCfg,
}

/// What the UI tells a thread.
enum Cmd {
    /// Whether the window is on this PC now (the pace changes).
    Current(bool),
    /// Ask now (the status-bar list was opened).
    Wake,
    /// Connect afresh (the URL moved).
    Reconnect,
}

/// The calls a thread makes, so a test can stand in for the other PC.
pub trait Line {
    /// Connects; the caller's role over there.
    fn connect(&mut self) -> Result<Option<Role>, String>;
    /// Whether the connection is still good.
    fn up(&self) -> bool;
    fn call(&self, request: &Request) -> Result<String, String>;
    /// `GET /health`.
    fn health(&self) -> Result<(), String>;
    /// This PC's `[notify]`.
    fn here_cfg(&self) -> ito_prefs::NotifyCfg;
    /// The ids the window's `Acked` keeps for this PC; `None` when the file is there and does not read.
    fn acked(&self) -> Option<BTreeSet<String>>;
}

/// The real thing: the `[[remote]]` of that name.
struct Remote {
    name: String,
    conn: Option<crate::link::Conn>,
}

impl Line for Remote {
    fn connect(&mut self) -> Result<Option<Role>, String> {
        self.conn = None;
        let conn = Link::Far(self.name.clone()).start()?;
        let role = conn.role();
        self.conn = Some(conn);
        Ok(role)
    }

    fn up(&self) -> bool {
        self.conn.as_ref().is_some_and(crate::link::Conn::running)
    }

    fn call(&self, request: &Request) -> Result<String, String> {
        self.conn.as_ref().ok_or_else(|| "not connected".to_owned())?.call(request)
    }

    fn health(&self) -> Result<(), String> {
        crate::remote::ask_within(&self.name, HEALTH_DEADLINE, |c| c.get("/health")).map(|_| ())
    }

    fn here_cfg(&self) -> ito_prefs::NotifyCfg {
        crate::notify::here_cfg()
    }

    fn acked(&self) -> Option<BTreeSet<String>> {
        match ito_common::state_dir("koyomi") {
            Some(dir) => crate::acked::Acked::read_for(&dir, Some(&self.name)),
            None => Some(BTreeSet::new()),
        }
    }
}

/// The ids of the runs that ended abnormally (today's failures, when `runs` are today's), without the ones a rerun
/// took over (a failure that was run again is not one nobody has handled).
pub fn failed_ids(runs: &[Run]) -> impl Iterator<Item = &str> {
    let gone = koyomi_core::run::superseded(runs);
    runs.iter().filter(move |r| r.state.is_error() && !gone.contains(&r.id)).map(|r| r.id.as_str())
}

/// What one round found: what to tell the UI and how long to wait before the next.
struct Round {
    events: Vec<Event>,
    wait: Duration,
}

/// How long to wait after a round: closely for the PC the window is on, else [`WATCH_EVERY`].
fn cadence(current: bool, poll: Duration) -> Duration {
    if current {
        poll
    } else {
        WATCH_EVERY
    }
}

/// One connection's state between rounds.
struct Watcher {
    name: String,
    line: Box<dyn Line + Send>,
    connected: bool,
    /// When the connection was made (a failing `Notices` on the PC the window is on connects afresh, at most once
    /// in [`RUNS_EVERY`], so a token or a role that changed is read again).
    connected_at: Option<Instant>,
    role: Option<Role>,
    /// Where the engine's notices were last read; lives for the window's life.
    at: NoticesAt,
    /// This PC's `[notify]` and when it was read.
    cfg: Option<(ito_prefs::NotifyCfg, Instant)>,
    /// Today's failed ids from the last `Runs`.
    failed: Option<Vec<String>>,
    /// The ids dealt with, as last read.
    acked: BTreeSet<String>,
    /// When `Runs` was last asked; `None` asks at the next round that is not on this PC.
    runs_at: Option<Instant>,
}

impl Watcher {
    fn new(name: &str, line: Box<dyn Line + Send>) -> Watcher {
        Watcher {
            name: name.to_owned(),
            line,
            connected: false,
            connected_at: None,
            role: None,
            at: NoticesAt::default(),
            cfg: None,
            failed: None,
            acked: BTreeSet::new(),
            runs_at: None,
        }
    }

    /// The connection is dropped: the next round connects afresh (the URL, the token and the role are read again).
    fn reconnect(&mut self) {
        self.connected = false;
        self.runs_at = None;
    }

    /// One round at `now`, while the window is (`current`) on this PC or not.
    fn round(&mut self, now: Instant, current: bool) -> Round {
        let mut events = Vec::new();
        let fact = |seen, unacked| Event::Fact(Fact { seen, unacked });
        if !self.connected || !self.line.up() {
            self.connected = false;
            match self.line.connect() {
                Ok(role) => {
                    self.connected = true;
                    self.connected_at = Some(now);
                    self.role = role;
                }
                Err(why) => {
                    // The PC may answer and still be no use to the window: the list tells the two apart.
                    let seen = if self.line.health().is_ok() { Seen::Refused(why) } else { Seen::NoAnswer(why) };
                    events.push(fact(seen, None));
                    // The PC the window is on is tried again sooner: its notices are waited for.
                    return Round { events, wait: if current { WATCH_EVERY } else { WATCH_RETRY } };
                }
            }
        }
        // A failed `Notices` alone does not make the PC "not answering" (an older engine has none): off this PC,
        // `Runs` is asked at once and says; on it, the connection is made afresh (see `connected_at`).
        let (mut came, mut missed) = (false, false);
        if self.role.is_some_and(|r| r.allows(Role::Edit)) {
            match self.line.call(&self.at.request()).and_then(|t| proto::NoticesView::parse(&t)) {
                Ok(view) => {
                    let new = self.at.take(view);
                    if !new.is_empty() {
                        came = true;
                        if self.cfg.as_ref().is_none_or(|(_, at)| now.saturating_duration_since(*at) >= crate::notify::REREAD) {
                            self.cfg = Some((self.line.here_cfg(), now));
                        }
                        let cfg = self.cfg.as_ref().map(|(c, _)| c.clone()).unwrap_or_default();
                        events.push(Event::Notes(Box::new(Notes { name: self.name.clone(), notes: new, cfg })));
                    }
                }
                Err(_) => missed = true,
            }
        }
        let wait = cadence(current, crate::link::POLL_FAR);
        if current {
            if missed && self.connected_at.is_none_or(|at| now.saturating_duration_since(at) >= RUNS_EVERY) {
                self.reconnect();
            }
            // The browser has this PC's runs; asked again as soon as the window leaves.
            self.failed = None;
            self.runs_at = None;
            events.push(fact(Seen::Answering, None));
            return Round { events, wait };
        }
        let due = self.runs_at.is_none_or(|at| now.saturating_duration_since(at) >= RUNS_EVERY);
        if due || came || missed {
            match self.line.call(&Request::Runs).and_then(|t| Runs::parse(&t)) {
                Ok(runs) => {
                    self.failed = Some(failed_ids(&runs.runs).map(str::to_owned).collect());
                    self.runs_at = Some(now);
                }
                Err(why) => {
                    // Connected afresh next round, as the browser does after a failed list.
                    self.reconnect();
                    events.push(fact(Seen::NoAnswer(why), None));
                    return Round { events, wait };
                }
            }
        }
        // The window writes the acknowledgements: read again every round, so the count follows them.
        if let Some(set) = self.line.acked() {
            self.acked = set;
        }
        let unacked = self.failed.as_ref().map(|f| f.iter().filter(|id| !self.acked.contains(*id)).count());
        events.push(fact(Seen::Answering, unacked));
        Round { events, wait }
    }
}

/// The names to start and to stop, given the threads that run and the names now listed.
fn diff(running: &[String], names: &[String]) -> (Vec<(usize, String)>, Vec<String>) {
    let start = names.iter().enumerate().filter(|(_, n)| !running.contains(n)).map(|(i, n)| (i, n.clone())).collect();
    let stop = running.iter().filter(|n| !names.contains(n)).cloned().collect();
    (start, stop)
}

/// How long a thread waits before its first round, so the connections are not all asked in the same second: steps
/// of 0.7 s around [`WATCH_EVERY`] (not a divisor of it, so the 22nd and later fall between the first ones).
fn first_delay(index: usize) -> Duration {
    const STEP_MS: u64 = 700;
    Duration::from_millis(index as u64 * STEP_MS % WATCH_EVERY.as_millis() as u64)
}

/// Waits `d`, or less when told something. `false`: the window dropped its side, the thread ends.
fn wait(rx: &Receiver<Cmd>, d: Duration, current: &mut bool, reconnect: &mut bool) -> bool {
    let first = match rx.recv_timeout(d) {
        Ok(c) => c,
        Err(RecvTimeoutError::Timeout) => return true,
        Err(RecvTimeoutError::Disconnected) => return false,
    };
    for c in std::iter::once(first).chain(rx.try_iter()) {
        match c {
            Cmd::Current(now) => *current = now,
            Cmd::Wake => {}
            Cmd::Reconnect => *reconnect = true,
        }
    }
    true
}

/// The thread of one connection; `id` goes with all it sends. It ends when the window drops its channel.
fn run(mut w: Watcher, id: u64, rx: Receiver<Cmd>, tx: Sender<(u64, Event)>, ctx: egui::Context, delay: Duration, mut current: bool) {
    let (mut reconnect, mut last) = (false, None::<Fact>);
    if !wait(&rx, delay, &mut current, &mut reconnect) {
        return;
    }
    loop {
        if std::mem::take(&mut reconnect) {
            w.reconnect();
        }
        let round = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| w.round(Instant::now(), current)));
        let Ok(round) = round else {
            let why = koyomi_core::msg::line(koyomi_core::msg::HOST_CHECK_STOPPED, &[&w.name]);
            let _ = tx.send((id, Event::Fact(Fact { seen: Seen::NoAnswer(why), unacked: None })));
            ctx.request_repaint();
            return;
        };
        let mut told = false;
        for e in round.events {
            // A fact that did not change is not told again.
            if let Event::Fact(f) = &e {
                if last.as_ref() == Some(f) {
                    continue;
                }
                last = Some(f.clone());
            }
            told = true;
            if tx.send((id, e)).is_err() {
                return;
            }
        }
        if told {
            ctx.request_repaint();
        }
        if !wait(&rx, round.wait, &mut current, &mut reconnect) {
            return;
        }
    }
}

struct Handle {
    /// The thread's own number: a stopped thread's late word is not taken for that of a name added again.
    id: u64,
    tx: Sender<Cmd>,
    /// What the thread was last told about being on this PC.
    current: bool,
}

/// The window's side: owns the threads and what they found out.
pub struct Watch {
    ctx: egui::Context,
    make: fn(&str) -> Box<dyn Line + Send>,
    tx: Sender<(u64, Event)>,
    rx: Receiver<(u64, Event)>,
    threads: Vec<(String, Handle)>,
    /// The id of the last thread started.
    last_id: u64,
    facts: HashMap<String, Fact>,
}

impl Watch {
    pub fn new(ctx: &egui::Context) -> Watch {
        Watch::with(ctx, |name| Box::new(Remote { name: name.to_owned(), conn: None }))
    }

    fn with(ctx: &egui::Context, make: fn(&str) -> Box<dyn Line + Send>) -> Watch {
        let (tx, rx) = mpsc::channel();
        Watch { ctx: ctx.clone(), make, tx, rx, threads: Vec::new(), last_id: 0, facts: HashMap::new() }
    }

    /// Follows the list of connections: starts the thread of a new name, stops that of a name that went, and tells
    /// every thread whether the window is on its PC (`current`). `names` is `None` while the list has not been read
    /// (or could not be): what runs keeps running. Returns the names that went.
    pub fn sync(&mut self, names: Option<&[String]>, current: Option<&str>) -> Vec<String> {
        let mut gone = Vec::new();
        if let Some(names) = names {
            let running: Vec<String> = self.threads.iter().map(|(n, _)| n.clone()).collect();
            let (start, stop) = diff(&running, names);
            for name in stop {
                // Dropping the handle ends the thread at its next wake or wait.
                self.threads.retain(|(n, _)| *n != name);
                self.facts.remove(&name);
                gone.push(name);
            }
            for (index, name) in start {
                let is_current = current == Some(name.as_str());
                let (tx, rx) = mpsc::channel();
                let w = Watcher::new(&name, (self.make)(&name));
                let (events, ctx) = (self.tx.clone(), self.ctx.clone());
                self.last_id += 1;
                let id = self.last_id;
                std::thread::spawn(move || run(w, id, rx, events, ctx, first_delay(index), is_current));
                self.threads.push((name, Handle { id, tx, current: is_current }));
            }
        }
        for (name, h) in &mut self.threads {
            let is_current = current == Some(name.as_str());
            if h.current != is_current {
                h.current = is_current;
                let _ = h.tx.send(Cmd::Current(is_current));
            }
        }
        gone
    }

    /// Takes what the threads sent: the facts are kept, the notices come back to be told.
    pub fn drain(&mut self) -> Vec<Notes> {
        let mut out = Vec::new();
        while let Ok((id, e)) = self.rx.try_recv() {
            // One from a thread that was stopped is dropped (its name may have been added again since).
            let Some((name, _)) = self.threads.iter().find(|(_, h)| h.id == id) else { continue };
            match e {
                Event::Fact(fact) => {
                    self.facts.insert(name.clone(), fact);
                }
                Event::Notes(n) => out.push(*n),
            }
        }
        out
    }

    pub fn facts(&self) -> &HashMap<String, Fact> {
        &self.facts
    }

    /// Every thread asks now (the status-bar list was opened, so a PC that came back is seen at once).
    pub fn wake_all(&self) {
        for (_, h) in &self.threads {
            let _ = h.tx.send(Cmd::Wake);
        }
    }

    /// The URL of the connection `name` changed: its thread connects afresh.
    pub fn reconnect(&self, name: &str) {
        if let Some((_, h)) = self.threads.iter().find(|(n, _)| n == name) {
            let _ = h.tx.send(Cmd::Reconnect);
        }
    }
}

/// The connections the window watches, from this PC's koyomi.toml as the window last read it: none when
/// `watch_remotes` is off, else the trimmed names of the `[[remote]]` whose `watch` is on, in file order.
pub fn watched(cfg: &koyomi_core::config::Config) -> Vec<String> {
    if !cfg.watch_remotes {
        return Vec::new();
    }
    cfg.remote.iter().filter(|r| r.watch).map(|r| r.name.trim().to_owned()).collect()
}

/// The trimmed names of every `[[remote]]` the window does not watch: all of them when `watch_remotes` is off.
pub fn unwatched(cfg: &koyomi_core::config::Config) -> BTreeSet<String> {
    cfg.remote.iter().filter(|r| !cfg.watch_remotes || !r.watch).map(|r| r.name.trim().to_owned()).collect()
}

/// The trimmed names of the `[[remote]]` with `watch = false` of their own (not counting `watch_remotes`): what the
/// manage box's switches show.
pub fn rows_off(cfg: &koyomi_core::config::Config) -> BTreeSet<String> {
    cfg.remote.iter().filter(|r| !r.watch).map(|r| r.name.trim().to_owned()).collect()
}

/// How many failures nobody has dealt with there are on the PCs the window is not on, counting only the ones that
/// answer (a PC that is off must not keep a red item; its dot in the list says it).
pub fn unacked_total(facts: &HashMap<String, Fact>, current: Option<&str>) -> usize {
    facts.iter().filter(|(n, f)| Some(n.as_str()) != current && f.seen == Seen::Answering).filter_map(|(_, f)| f.unacked).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// What the fake PC does.
    #[derive(Default)]
    struct Script {
        connect: Option<String>,
        health_ok: bool,
        /// The next answers to `Notices`; once they are used up, the last one again (none ever: an older engine).
        notices: Vec<String>,
        last_view: Option<String>,
        /// `Notices` fails whatever is scripted.
        notices_fail: bool,
        runs: Option<Result<String, String>>,
        acked: Option<BTreeSet<String>>,
        asked: Vec<&'static str>,
        connects: usize,
    }

    #[derive(Clone)]
    struct Fake(Arc<Mutex<Script>>, Option<Role>);

    impl Line for Fake {
        fn connect(&mut self) -> Result<Option<Role>, String> {
            let mut s = self.0.lock().unwrap();
            s.connects += 1;
            match &s.connect {
                Some(e) => Err(e.clone()),
                None => Ok(self.1),
            }
        }
        fn up(&self) -> bool {
            true
        }
        fn call(&self, request: &Request) -> Result<String, String> {
            let mut s = self.0.lock().unwrap();
            match request {
                Request::Notices { .. } => {
                    s.asked.push("notices");
                    if s.notices_fail {
                        return Err("E: down".into());
                    }
                    if !s.notices.is_empty() {
                        s.last_view = Some(s.notices.remove(0));
                    }
                    s.last_view.clone().ok_or_else(|| "E: unknown request".into())
                }
                Request::Runs => {
                    s.asked.push("runs");
                    s.runs.clone().unwrap_or_else(|| Ok(String::new()))
                }
                _ => Err("unexpected".into()),
            }
        }
        fn health(&self) -> Result<(), String> {
            if self.0.lock().unwrap().health_ok {
                Ok(())
            } else {
                Err("no answer".into())
            }
        }
        fn here_cfg(&self) -> ito_prefs::NotifyCfg {
            ito_prefs::NotifyCfg::default()
        }
        fn acked(&self) -> Option<BTreeSet<String>> {
            self.0.lock().unwrap().acked.clone()
        }
    }

    fn view(boot: &str, last: u64, titles: &[&str]) -> String {
        let notes = titles
            .iter()
            .enumerate()
            .map(|(i, t)| Note {
                seq: last - titles.len() as u64 + 1 + i as u64,
                event: "failed".into(),
                title: (*t).into(),
                ..Note::default()
            })
            .collect();
        proto::NoticesView { boot: boot.into(), last, notes }.to_toml()
    }

    fn runs(ids: &[(&str, bool)]) -> String {
        let runs = ids
            .iter()
            .map(|(id, bad)| {
                let at = chrono::NaiveDate::from_ymd_opt(2026, 10, 30).unwrap().and_hms_opt(2, 0, 0).unwrap();
                let mut r = Run::new(id, "g/n", "n", "2026-10-30", koyomi_core::run::How::Manual, &Default::default(), at);
                r.state = if *bad { koyomi_core::run::State::Error } else { koyomi_core::run::State::Ok };
                r
            })
            .collect();
        Runs { runs }.to_toml()
    }

    fn watcher(role: Option<Role>) -> (Watcher, Arc<Mutex<Script>>) {
        let script = Arc::new(Mutex::new(Script { health_ok: true, ..Script::default() }));
        (Watcher::new("kei", Box::new(Fake(script.clone(), role))), script)
    }

    fn fact_of(r: &Round) -> Fact {
        r.events.iter().find_map(|e| if let Event::Fact(f) = e { Some(f.clone()) } else { None }).expect("a fact")
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn names_are_started_stopped_or_kept() {
        let (start, stop) = diff(&names(&["a", "b"]), &names(&["b", "c", "d"]));
        assert_eq!(start, [(1, "c".to_owned()), (2, "d".to_owned())]);
        assert_eq!(stop, ["a"]);
        let (start, stop) = diff(&names(&["a"]), &names(&["a"]));
        assert!(start.is_empty() && stop.is_empty());
    }

    #[test]
    fn the_first_round_is_spread_and_the_pace_follows_the_link() {
        assert_eq!(first_delay(0), Duration::ZERO);
        assert_eq!(first_delay(3), Duration::from_millis(2100));
        let firsts: BTreeSet<Duration> = (0..150).map(first_delay).collect();
        assert_eq!(firsts.len(), 150, "no two of the first 150 start together");
        assert!(firsts.iter().all(|d| *d < WATCH_EVERY));
        assert_eq!(cadence(true, Duration::from_secs(3)), Duration::from_secs(3));
        assert_eq!(cadence(false, Duration::from_secs(3)), WATCH_EVERY);
    }

    #[test]
    fn the_first_ask_only_learns_and_the_next_tells_what_is_new() {
        let (mut w, s) = watcher(Some(Role::Edit));
        let t0 = Instant::now();
        s.lock().unwrap().notices = vec![view("b", 4, &[]), view("b", 5, &["net x failed"])];
        let r = w.round(t0, false);
        assert!(!r.events.iter().any(|e| matches!(e, Event::Notes(_))));
        let r = w.round(t0 + WATCH_EVERY, false);
        let notes: Vec<&Notes> = r.events.iter().filter_map(|e| if let Event::Notes(n) = e { Some(&**n) } else { None }).collect();
        assert_eq!((notes.len(), notes[0].name.as_str(), notes[0].notes[0].title.as_str()), (1, "kei", "net x failed"));
    }

    #[test]
    fn below_edit_no_notices_are_asked() {
        let (mut w, s) = watcher(Some(Role::View));
        w.round(Instant::now(), false);
        assert!(!s.lock().unwrap().asked.contains(&"notices"));
    }

    #[test]
    fn a_connect_that_fails_is_refused_when_health_answers_and_no_answer_when_not() {
        let (mut w, s) = watcher(Some(Role::Edit));
        s.lock().unwrap().connect = Some("E: token".into());
        let r = w.round(Instant::now(), false);
        assert_eq!(r.wait, WATCH_RETRY);
        assert_eq!(fact_of(&r), Fact { seen: Seen::Refused("E: token".into()), unacked: None }, "/health answers");
        s.lock().unwrap().health_ok = false;
        let r = w.round(Instant::now(), false);
        assert_eq!(fact_of(&r).seen, Seen::NoAnswer("E: token".into()));
        assert_eq!(s.lock().unwrap().connects, 2, "it connects again at the next round");
        assert_eq!(w.round(Instant::now(), true).wait, WATCH_EVERY, "the PC the window is on is tried sooner");
    }

    #[test]
    fn a_notices_error_keeps_the_pc_answering() {
        let (mut w, s) = watcher(Some(Role::Edit));
        // No notices scripted: the call fails (an older engine).
        s.lock().unwrap().runs = Some(Ok(runs(&[])));
        let r = w.round(Instant::now(), false);
        assert_eq!(fact_of(&r).seen, Seen::Answering);
    }

    #[test]
    fn a_notices_error_off_this_pc_asks_runs_at_once_so_a_pc_that_went_is_seen() {
        let (mut w, s) = watcher(Some(Role::Edit));
        let t0 = Instant::now();
        {
            let mut s = s.lock().unwrap();
            s.notices = vec![view("b", 1, &[])];
            s.runs = Some(Ok(runs(&[("r1", true)])));
        }
        assert_eq!(fact_of(&w.round(t0, false)).unacked, Some(1));
        // The PC went: Notices fails, and Runs (not due for a minute) is asked anyway and fails too.
        {
            let mut s = s.lock().unwrap();
            s.notices_fail = true;
            s.runs = Some(Err("E: down".into()));
        }
        let r = w.round(t0 + WATCH_EVERY, false);
        assert_eq!(fact_of(&r), Fact { seen: Seen::NoAnswer("E: down".into()), unacked: None }, "no stale count");
    }

    #[test]
    fn a_notices_error_on_this_pc_connects_afresh_at_most_once_a_minute() {
        let (mut w, s) = watcher(Some(Role::Edit));
        let t0 = Instant::now();
        s.lock().unwrap().notices = vec![view("b", 1, &[])];
        w.round(t0, true);
        assert_eq!(s.lock().unwrap().connects, 1);
        // Notices fails from now on (a token that changed, or an older engine): soon after connecting, nothing.
        s.lock().unwrap().notices_fail = true;
        w.round(t0 + Duration::from_secs(3), true);
        w.round(t0 + Duration::from_secs(6), true);
        assert_eq!(s.lock().unwrap().connects, 1);
        // A minute after: connected afresh at the next round, and then not again for a minute.
        w.round(t0 + RUNS_EVERY, true);
        w.round(t0 + RUNS_EVERY + Duration::from_secs(3), true);
        w.round(t0 + RUNS_EVERY + Duration::from_secs(6), true);
        assert_eq!(s.lock().unwrap().connects, 2);
        assert!(!s.lock().unwrap().asked.contains(&"runs"), "on this PC the browser has the runs");
    }

    #[test]
    fn runs_are_asked_only_off_this_link_every_minute_or_after_notices() {
        let (mut w, s) = watcher(Some(Role::Edit));
        let t0 = Instant::now();
        let asked = || s.lock().unwrap().asked.iter().filter(|a| **a == "runs").count();
        s.lock().unwrap().notices = vec![view("b", 0, &[])];
        // On this PC: never.
        for i in 0..3 {
            w.round(t0 + WATCH_EVERY * i, true);
        }
        assert_eq!(asked(), 0);
        // Off it: at once, then not before RUNS_EVERY.
        w.round(t0 + WATCH_EVERY * 3, false);
        assert_eq!(asked(), 1);
        w.round(t0 + WATCH_EVERY * 4, false);
        w.round(t0 + WATCH_EVERY * 5, false);
        assert_eq!(asked(), 1);
        w.round(t0 + WATCH_EVERY * 3 + RUNS_EVERY, false);
        assert_eq!(asked(), 2);
        // Notices came: asked in that round, before the minute is up; nothing new after asks nothing.
        s.lock().unwrap().notices = vec![view("b", 1, &["n"]), view("b", 1, &[])];
        w.round(t0 + WATCH_EVERY * 3 + RUNS_EVERY + Duration::from_secs(1), false);
        assert_eq!(asked(), 3);
        w.round(t0 + WATCH_EVERY * 3 + RUNS_EVERY + Duration::from_secs(16), false);
        assert_eq!(asked(), 3);
        // Back on this PC and away again: asked at once.
        w.round(t0 + WATCH_EVERY * 9, true);
        w.round(t0 + WATCH_EVERY * 9 + Duration::from_secs(3), false);
        assert_eq!(asked(), 4);
    }

    #[test]
    fn the_unacked_count_is_the_failed_ids_not_in_the_acked_set_and_follows_acks() {
        let (mut w, s) = watcher(Some(Role::Edit));
        let t0 = Instant::now();
        {
            let mut s = s.lock().unwrap();
            s.runs = Some(Ok(runs(&[("r1", true), ("r2", true), ("r3", false), ("r4", true)])));
            s.acked = Some(["r1".to_owned()].into());
        }
        assert_eq!(fact_of(&w.round(t0, false)).unacked, Some(2));
        // The person acked r2 meanwhile: no new Runs, the count falls.
        s.lock().unwrap().acked = Some(["r1".to_owned(), "r2".to_owned()].into());
        assert_eq!(fact_of(&w.round(t0 + WATCH_EVERY, false)).unacked, Some(1));
        // An unreadable file keeps the set it had.
        s.lock().unwrap().acked = None;
        assert_eq!(fact_of(&w.round(t0 + WATCH_EVERY * 2, false)).unacked, Some(1));
        // On this PC the browser counts: nothing here.
        assert_eq!(fact_of(&w.round(t0 + WATCH_EVERY * 3, true)).unacked, None);
    }

    /// A run of `g/n` with `state`, started at `min` minutes past 2:00, and the run it redoes.
    fn chain_run(id: &str, min: u32, state: koyomi_core::run::State, of: Option<&str>) -> Run {
        let at = chrono::NaiveDate::from_ymd_opt(2026, 10, 30).unwrap().and_hms_opt(2, min, 0).unwrap();
        let mut r = Run::new(id, "g/n", "n", "2026-10-30", koyomi_core::run::How::Rerun, &Default::default(), at);
        r.state = state;
        r.rerun_of = of.map(str::to_owned);
        r
    }

    #[test]
    fn a_failure_that_a_rerun_took_over_is_not_a_failed_id() {
        use koyomi_core::run::State;
        let a = chain_run("a", 0, State::Error, None);
        let ok = vec![a.clone(), chain_run("b", 5, State::Ok, Some("a"))];
        assert_eq!(failed_ids(&ok).count(), 0);
        let bad = vec![a, chain_run("b", 5, State::Error, Some("a"))];
        assert_eq!(failed_ids(&bad).collect::<Vec<_>>(), ["b"]);
    }

    #[test]
    fn a_runs_error_is_no_answer_and_connects_afresh() {
        let (mut w, s) = watcher(Some(Role::Edit));
        s.lock().unwrap().runs = Some(Err("E: down".into()));
        let r = w.round(Instant::now(), false);
        assert_eq!(fact_of(&r), Fact { seen: Seen::NoAnswer("E: down".into()), unacked: None });
        s.lock().unwrap().runs = Some(Ok(runs(&[])));
        let r = w.round(Instant::now(), false);
        assert_eq!(fact_of(&r).seen, Seen::Answering);
        assert_eq!(s.lock().unwrap().connects, 2);
    }

    #[test]
    fn the_status_bar_sum_leaves_out_the_current_pc_and_the_ones_that_do_not_answer() {
        let f = |seen, unacked| Fact { seen, unacked };
        let mut facts = HashMap::new();
        facts.insert("a".to_owned(), f(Seen::Answering, Some(2)));
        facts.insert("b".to_owned(), f(Seen::Answering, Some(3)));
        facts.insert("c".to_owned(), f(Seen::NoAnswer("x".into()), Some(9)));
        facts.insert("d".to_owned(), f(Seen::Refused("x".into()), None));
        facts.insert("e".to_owned(), f(Seen::Answering, None));
        assert_eq!(unacked_total(&facts, None), 5);
        assert_eq!(unacked_total(&facts, Some("b")), 2);
    }

    #[test]
    fn threads_follow_the_list_and_facts_of_names_that_went_are_dropped() {
        fn make(_: &str) -> Box<dyn Line + Send> {
            Box::new(Fake(
                Arc::new(Mutex::new(Script { health_ok: true, runs: Some(Ok(String::new())), ..Script::default() })),
                Some(Role::Edit),
            ))
        }
        let mut w = Watch::with(&egui::Context::default(), make);
        assert!(w.sync(None, None).is_empty(), "not listed yet: nothing starts");
        assert!(w.threads.is_empty());
        w.sync(Some(&names(&["a"])), None);
        let end = Instant::now() + Duration::from_secs(10);
        while !w.facts.contains_key("a") && Instant::now() < end {
            w.drain();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(w.facts.get("a").map(|f| f.seen.clone()), Some(Seen::Answering));
        // A listing that failed (None) keeps what runs.
        w.sync(None, Some("a"));
        assert_eq!(w.threads.len(), 1);
        let gone = w.sync(Some(&names(&[])), None);
        assert_eq!(gone, ["a"]);
        assert!(w.facts.is_empty() && w.threads.is_empty());
        // Added again: what the stopped thread still sends is not taken for the new one's.
        w.sync(Some(&names(&["a"])), None);
        let stale = Fact { seen: Seen::NoAnswer("old".into()), unacked: Some(9) };
        w.tx.send((1, Event::Fact(stale.clone()))).unwrap();
        w.tx.send((1, Event::Notes(Box::new(Notes { name: "a".into(), notes: Vec::new(), cfg: Default::default() })))).unwrap();
        assert!(w.drain().is_empty(), "the old thread's notices are dropped");
        assert_ne!(w.facts.get("a"), Some(&stale));
    }

    fn remotes_cfg(watch_remotes: bool, rows: &[(&str, bool)]) -> koyomi_core::config::Config {
        let remote = |(name, watch): &(&str, bool)| koyomi_core::config::Remote {
            name: format!(" {name} "),
            url: "https://h:1".into(),
            fingerprint: "ab".repeat(32),
            token: format!("secret:{name}"),
            watch: *watch,
        };
        koyomi_core::config::Config { watch_remotes, remote: rows.iter().map(remote).collect(), ..Default::default() }
    }

    #[test]
    fn what_is_watched_follows_the_file_and_the_global_switch() {
        let on = remotes_cfg(true, &[("a", true), ("b", false), ("c", true)]);
        assert_eq!(watched(&on), ["a", "c"]);
        assert_eq!(unwatched(&on), BTreeSet::from(["b".to_owned()]));
        assert_eq!(rows_off(&on), BTreeSet::from(["b".to_owned()]));
        let off = remotes_cfg(false, &[("a", true), ("b", false)]);
        assert!(watched(&off).is_empty());
        assert_eq!(unwatched(&off), BTreeSet::from(["a".to_owned(), "b".to_owned()]));
        assert_eq!(rows_off(&off), BTreeSet::from(["b".to_owned()]), "rows keep their own flag");
        assert!(watched(&Default::default()).is_empty() && unwatched(&Default::default()).is_empty());
    }

    #[test]
    fn a_notice_of_a_thread_that_was_dropped_is_not_returned() {
        fn make(_: &str) -> Box<dyn Line + Send> {
            Box::new(Fake(Arc::new(Mutex::new(Script { health_ok: true, ..Script::default() })), Some(Role::Edit)))
        }
        let mut w = Watch::with(&egui::Context::default(), make);
        w.sync(Some(&names(&["a"])), None);
        let id = w.threads[0].1.id;
        w.sync(Some(&names(&[])), None);
        w.tx.send((id, Event::Notes(Box::new(Notes { name: "a".into(), notes: Vec::new(), cfg: Default::default() })))).unwrap();
        assert!(w.drain().is_empty());
        w.sync(Some(&names(&["a"])), None);
        assert_ne!(w.threads[0].1.id, id, "the name added again is a new thread");
        w.tx.send((id, Event::Notes(Box::new(Notes { name: "a".into(), notes: Vec::new(), cfg: Default::default() })))).unwrap();
        assert!(w.drain().is_empty());
    }

    #[test]
    fn a_thread_that_panics_says_so_and_ends() {
        struct Boom;
        impl Line for Boom {
            fn connect(&mut self) -> Result<Option<Role>, String> {
                panic!("on purpose")
            }
            fn up(&self) -> bool {
                false
            }
            fn call(&self, _: &Request) -> Result<String, String> {
                Err(String::new())
            }
            fn health(&self) -> Result<(), String> {
                Ok(())
            }
            fn here_cfg(&self) -> ito_prefs::NotifyCfg {
                ito_prefs::NotifyCfg::default()
            }
            fn acked(&self) -> Option<BTreeSet<String>> {
                None
            }
        }
        let (tx, rx) = mpsc::channel();
        let (_keep, crx) = mpsc::channel::<Cmd>();
        let w = Watcher::new("kei", Box::new(Boom));
        run(w, 7, crx, tx, egui::Context::default(), Duration::ZERO, false);
        match rx.try_recv() {
            Ok((7, Event::Fact(f))) => assert!(matches!(f.seen, Seen::NoAnswer(_))),
            other => panic!("{other:?}"),
        }
    }
}
