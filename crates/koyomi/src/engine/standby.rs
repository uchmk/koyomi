//! `[standby]` of koyomi.toml (TODO 87 and 96, DECISIONS「複数の PC・Web UI・利用者とロール・HA」):
//! this engine stands by for the `[[remote]]` named `watch`. It polls that engine's
//! `GET /health` on a thread every `every`; while it answers, this one runs no schedule and
//! no event (a person's `koyomi run` still goes). After `misses` polls in a row that get no
//! answer, it takes over and runs the times from then on.
//!
//! While it watches, the runs going on there (`GET /runs?running=1`) are read every 2 s, apart from
//! the health poll, and the latest good list is kept (a failed read does not wipe it; one older
//! than 30 s is not trusted). A run that starts and whose PC dies within the 2 s cannot be known. On a
//! takeover the engine starts those of a net that says `takeover = "rerun"` here again, from the
//! jobs that had not passed, and writes the others down as not taken over, for a person.
//!
//! It hands back by `koyomi standby giveback`, or with `give_back = "auto"` by itself once the
//! primary has answered `misses` polls in a row. A primary with `[primary]` waits for this one:
//! it reads `"ha"` in `GET /health` here and takes the schedules back with
//! `POST /standby/release`, which lets go only when this one is ready ([`Standby::handover`]), so
//! they are never run on both PCs nor on neither. A person's giveback while it waits lets go at
//! its next ask. A primary without `[primary]` runs on its own once it is up: a giveback, or
//! `"auto"` stepping down, ends the time both run.
//!
//! The takeover, a giveback that waits for the primary to ask, and each handover are kept in
//! `standby.toml` in the state folder, so a restart while it has taken over does not wait for
//! the misses again.
//!
//! Only "nobody is there" counts as a miss: no connection, or TLS that does not finish. A
//! certificate that differs from the fingerprint, a token that is missing here, or an address
//! that does not read cannot tell whether the primary is up; they are said, and not counted.
//!
//! While it stands by, it also copies the definitions from there every [`COPY_EVERY`]: the
//! confirmed ones whose file differs from the one here (`GET /nets` gives the hashes, `GET
//! /nets/text` the text, which needs the `edit` role there). They land unconfirmed, as a
//! proposal does (TODO 83): a person confirms them in this PC's window, and one not confirmed
//! does not run after a takeover. A net that is only here is left as it is.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ito_common::kept;
use koyomi_core::config;
use koyomi_core::msg::{self, line};
use koyomi_core::run::Run;
use koyomi_core::standby::{Refusal, Seen, State, Step, Watch};
use koyomi_http::client::{self, Client};
use serde::{Deserialize, Serialize};

/// How often a standby copies the definitions from the engine it watches.
pub const COPY_EVERY: Duration = Duration::from_secs(300);

/// How often the runs going on there are read while standing by, apart from the health poll
/// (which may be 5 s or more): a run that starts and then the primary dies must not fall
/// between two reads.
const RUNS_EVERY: Duration = Duration::from_secs(2);

/// How old the last good list of runs may be and still be started from. Older than this, the
/// reads there have been failing for a while and the list may hold runs that ended since.
const RUNS_STALE: Duration = Duration::from_secs(30);

/// How many handovers `standby.toml` keeps.
const LOG: usize = 20;

/// A definition copied from the watched engine, to write here unconfirmed.
#[derive(Debug, Clone, PartialEq)]
pub struct Copied {
    pub id: String,
    pub text: String,
    /// The hash of the file here when it was fetched (`None`: there was none).
    pub base: Option<String>,
}

/// What one copy found, or why it could not.
type Fetched = Result<Vec<Copied>, String>;

/// `standby.toml`.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
struct Kept {
    #[serde(default)]
    active: bool,
    /// When it took over, local time.
    #[serde(default)]
    since: String,
    /// A person ran `koyomi standby giveback` while the primary waited: let go when it asks.
    #[serde(default)]
    giving: bool,
    /// When it last handed the schedules back, `YYYY-MM-DD HH:MM` (empty: not since this was kept).
    #[serde(default)]
    back: String,
    /// The handovers, oldest first: when, which way, and why.
    #[serde(default)]
    log: Vec<String>,
}

/// What one poll found.
#[derive(Debug, Clone, PartialEq)]
enum Poll {
    /// The engine there answered: whether it runs the schedules or waits, what a person should
    /// see when the answer was not 200, and the runs going on there (`None`: not read this time).
    Answered { seen: Seen, note: Option<String>, running: Option<Result<Vec<Run>, String>> },
    /// Nobody answered.
    Missed(String),
    /// Cannot tell: the certificate, the token or the address is wrong here.
    Unknown(String),
}

/// What the engine tells of.
#[derive(Debug, Clone, PartialEq)]
pub enum Told {
    /// Took over; the runs the primary was running at the last poll (`None`: they could not be read).
    TookOver(String, Option<Vec<Run>>),
    /// The primary runs the schedules too.
    Both(String),
    /// The primary is back and waits to be handed the schedules.
    Waiting(String),
    /// Handed the schedules back, and watches again.
    GaveBack(String),
}

pub struct Standby {
    cfg: Option<config::Standby>,
    remote: Option<config::Remote>,
    watch: Watch,
    writer: kept::Writer,
    since: String,
    /// When it last handed the schedules back, `YYYY-MM-DD HH:MM`.
    back: String,
    log: Vec<String>,
    last: Option<Instant>,
    tx: mpsc::Sender<Poll>,
    rx: mpsc::Receiver<Poll>,
    in_flight: bool,
    /// Some poll has come back since the start or since the primary changed.
    polled: bool,
    /// What the primary was at the last poll that could tell.
    seen: Option<Seen>,
    /// The runs going on there at the last poll (`None`: not read then).
    running: Option<Vec<Run>>,
    /// When `running` was read.
    running_at: Option<Instant>,
    runs_tx: mpsc::Sender<Result<Vec<Run>, String>>,
    runs_rx: mpsc::Receiver<Result<Vec<Run>, String>>,
    runs_last: Option<Instant>,
    runs_in_flight: bool,
    /// What the last poll said was wrong.
    note: Option<String>,
    /// Why the last read of the runs there did not go.
    runs_note: Option<String>,
    /// Why the last miss was a miss.
    missed_why: Option<String>,
    /// What happened outside a poll (a release), for the next tick to tell of.
    told: Vec<Told>,
    copy_tx: mpsc::Sender<Fetched>,
    copy_rx: mpsc::Receiver<Fetched>,
    copied_at: Option<Instant>,
    copying: bool,
    /// Why the last copy did not go, or which ones were not written here.
    copy_note: Option<String>,
}

impl Standby {
    /// Reads what the previous engine left: whether it had taken over, and the handovers.
    pub fn new(state_dir: &Path) -> Standby {
        let path = state_dir.join("standby.toml");
        let prev: Kept = kept::read(&path).unwrap_or_default();
        let (tx, rx) = mpsc::channel();
        let (copy_tx, copy_rx) = mpsc::channel();
        let (runs_tx, runs_rx) = mpsc::channel();
        let misses = koyomi_core::standby::MISSES;
        Standby {
            cfg: None,
            remote: None,
            watch: if prev.active { Watch::resumed(misses, false, prev.giving) } else { Watch::new(misses, false) },
            writer: kept::Writer::new(path, true),
            since: prev.since,
            back: prev.back,
            log: prev.log,
            last: None,
            tx,
            rx,
            in_flight: false,
            polled: false,
            seen: None,
            running: None,
            running_at: None,
            runs_tx,
            runs_rx,
            runs_last: None,
            runs_in_flight: false,
            note: None,
            runs_note: None,
            missed_why: None,
            told: Vec::new(),
            copy_tx,
            copy_rx,
            copied_at: None,
            copying: false,
            copy_note: None,
        }
    }

    /// Takes in koyomi.toml's `[standby]` and the `[[remote]]` it names.
    pub fn set(&mut self, cfg: Option<&config::Standby>, remotes: &[config::Remote]) {
        let remote = cfg.and_then(|c| remotes.iter().find(|r| r.name.trim() == c.watch.trim()).cloned());
        let how = |c: Option<&config::Standby>| c.map(|c| (c.misses, c.give_back == config::GiveBack::Auto));
        if how(cfg) != how(self.cfg.as_ref()) {
            let (misses, auto) = how(cfg).unwrap_or((koyomi_core::standby::MISSES, false));
            let giving = self.watch.giving();
            self.watch = if self.watch.active() { Watch::resumed(misses, auto, giving) } else { Watch::new(misses, auto) };
        }
        if remote != self.remote {
            self.last = None;
            self.note = None;
            self.copied_at = None;
            self.copy_note = None;
            self.polled = false;
            self.seen = None;
            self.running = None;
            self.running_at = None;
            self.runs_note = None;
        }
        self.cfg = cfg.cloned();
        self.remote = remote;
    }

    /// Stands by: the schedules and the events here wait.
    pub fn waiting(&self) -> bool {
        self.cfg.is_some() && !self.watch.active()
    }

    /// The `[[remote]]` `[standby]` watches, when it is written.
    pub fn watched(&self) -> Option<&str> {
        self.cfg.as_ref().map(|c| c.watch.trim())
    }

    /// Polls when due and takes in what came back; what to tell of.
    pub fn tick(&mut self) -> Vec<Told> {
        let mut told = std::mem::take(&mut self.told);
        let (Some(every), Some(remote)) = (self.cfg.as_ref().map(|c| c.every_secs()), self.remote.clone()) else {
            return told;
        };
        while let Ok(p) = self.rx.try_recv() {
            self.in_flight = false;
            told.extend(self.take(&remote.name, p));
        }
        while let Ok(r) = self.runs_rx.try_recv() {
            self.runs_in_flight = false;
            if self.waiting() {
                self.take_runs(r);
            }
        }
        if self.waiting() && !self.runs_in_flight && self.runs_last.is_none_or(|l| l.elapsed() >= RUNS_EVERY) {
            self.runs_last = Some(Instant::now());
            self.runs_in_flight = true;
            let (tx, remote) = (self.runs_tx.clone(), remote.clone());
            let spawned = std::thread::Builder::new().name("standby-runs".into()).spawn(move || {
                let _ = tx.send(runs_poll(&remote));
            });
            if spawned.is_err() {
                self.runs_in_flight = false;
            }
        }
        if self.in_flight || self.last.is_some_and(|l| l.elapsed() < Duration::from_secs(every)) {
            return told;
        }
        self.last = Some(Instant::now());
        self.in_flight = true;
        // The runs there are read only while standing by: they are what a takeover starts from.
        let (tx, runs) = (self.tx.clone(), self.waiting());
        let spawned = std::thread::Builder::new().name("standby".into()).spawn(move || {
            let _ = tx.send(poll(&remote, runs));
        });
        if spawned.is_err() {
            self.in_flight = false;
        }
        told
    }

    /// While it stands by: what the copy thread fetched, and the next copy started when due.
    /// `here` gives each definition's id and file hash on this PC. Nothing once it has taken
    /// over: a copy that comes back late must not leave the nets it runs unconfirmed.
    pub fn copies(&mut self, here: impl FnOnce() -> BTreeMap<String, String>) -> Vec<Copied> {
        let mut got = Vec::new();
        while let Ok(f) = self.copy_rx.try_recv() {
            self.copying = false;
            match f {
                Ok(c) => {
                    self.copy_note = None;
                    got.extend(c);
                }
                Err(e) => self.copy_note = Some(e),
            }
        }
        if !self.waiting() {
            // A copy's trouble means nothing while this PC runs; copy at once on a giveback.
            self.copy_note = None;
            self.copied_at = None;
            return Vec::new();
        }
        let Some(remote) = self.remote.clone() else { return got };
        if self.copying || self.copied_at.is_some_and(|t| t.elapsed() < COPY_EVERY) {
            return got;
        }
        self.copied_at = Some(Instant::now());
        self.copying = true;
        let (tx, here) = (self.copy_tx.clone(), here());
        let spawned = std::thread::Builder::new().name("standby-copy".into()).spawn(move || {
            let _ = tx.send(fetch(&remote, &here));
        });
        if spawned.is_err() {
            self.copying = false;
        }
        got
    }

    /// The copies that could not be written here, to show until a copy goes through.
    pub fn refused(&mut self, why: Vec<String>) {
        if !why.is_empty() {
            let why: Vec<&str> = why.iter().map(|w| msg::uncoded(w)).collect();
            self.copy_note = Some(line(msg::COPY_NOT_WRITTEN, &[&why.join(" / ")]));
        }
    }

    fn take(&mut self, name: &str, p: Poll) -> Option<Told> {
        self.polled = true;
        let seen = match p {
            Poll::Answered { seen, note, running } => {
                self.note = note;
                if let Some(r) = running {
                    self.take_runs(r);
                }
                seen
            }
            Poll::Missed(why) => {
                self.note = None;
                self.missed_why = Some(why);
                Seen::Nobody
            }
            Poll::Unknown(why) => {
                self.note = Some(why);
                return None;
            }
        };
        self.seen = Some(seen);
        let (misses, auto) =
            self.cfg.as_ref().map_or((koyomi_core::standby::MISSES, false), |c| (c.misses, c.give_back == config::GiveBack::Auto));
        match self.watch.poll(seen) {
            Step::Stay => None,
            Step::TakeOver => {
                self.since = ito_common::now::now_local().format("%Y-%m-%d %H:%M").to_string();
                let why = self.missed_why.clone().unwrap_or_default();
                self.record(&msg::note(msg::N_STANDBY_REC_TOOK_OVER, &[name, msg::uncoded(&why)]));
                // The latest good list is started from, unless it is too old to trust.
                let stale = self.running_at.is_none_or(|t| t.elapsed() > RUNS_STALE);
                if stale && self.running.take().is_some() && self.runs_note.is_none() {
                    self.runs_note = Some(line(msg::RUNS_TOO_OLD, &[name]));
                }
                self.running_at = None;
                let unread = self.running.is_none().then(|| self.runs_note.take()).flatten();
                self.runs_note = None;
                let back = if auto {
                    msg::note(msg::N_STANDBY_BACK_AUTO, &[name, &misses.to_string()])
                } else {
                    msg::note(msg::N_STANDBY_BACK_MANUAL, &[])
                };
                let why = msg::uncoded(&why);
                // Say why the runs there are not known, instead of only that they were not read.
                let line = match unread {
                    Some(n) => msg::note(msg::N_STANDBY_TOOK_OVER_UNREAD, &[name, why, &self.since, &back, msg::uncoded(&n)]),
                    None => msg::note(msg::N_STANDBY_TOOK_OVER, &[name, why, &self.since, &back]),
                };
                Some(Told::TookOver(line, self.running.take()))
            }
            Step::BothActive => Some(Told::Both(match seen {
                Seen::Asks => msg::note(msg::N_STANDBY_BOTH_ASKS, &[name]),
                _ if auto => msg::note(msg::N_STANDBY_BOTH_AUTO, &[name, &misses.to_string()]),
                _ => msg::note(msg::N_STANDBY_BOTH_MANUAL, &[name]),
            })),
            Step::Waiting => Some(Told::Waiting(if self.watch.giving() {
                msg::note(msg::N_STANDBY_WAIT_GIVING, &[name])
            } else if auto {
                msg::note(msg::N_STANDBY_WAIT_AUTO, &[name, &misses.to_string()])
            } else {
                msg::note(msg::N_STANDBY_WAIT_MANUAL, &[name])
            })),
            Step::StepDown => {
                self.since.clear();
                self.last = None;
                let misses = misses.to_string();
                self.record_back(&msg::note(msg::N_STANDBY_REC_GAVE_BACK_AUTO, &[name, &misses]));
                Some(Told::GaveBack(msg::note(msg::N_STANDBY_GAVE_BACK_AUTO, &[name, &misses])))
            }
        }
    }

    /// Takes in one read of the runs going on there. The latest good list is the one a takeover
    /// starts from (an older one may hold runs that ended since). A failed read keeps it: the
    /// reads fail exactly when the primary is dying, and an empty list there must not be what a
    /// takeover starts from when a good one was seen a moment before.
    fn take_runs(&mut self, r: Result<Vec<Run>, String>) {
        match r {
            Ok(list) => {
                self.running = Some(list);
                self.running_at = Some(Instant::now());
                self.runs_note = None;
            }
            Err(e) => self.runs_note = Some(e),
        }
    }

    /// `koyomi standby giveback`. When the primary waits for this one (`[primary]`), it lets go
    /// at the primary's next ask, so the schedules are never left to nobody; otherwise at once.
    pub fn give_back(&mut self) -> Result<String, String> {
        let Some(name) = self.cfg.as_ref().map(|c| c.watch.trim().to_owned()) else {
            return Err(line(msg::STANDBY_NOT_SET, &[]));
        };
        if !self.watch.active() {
            return Err(line(msg::STANDBY_NOT_ACTIVE, &[&name]));
        }
        if !self.polled {
            // After a restart: whether the primary waits for this one is not known yet.
            return Err(line(msg::STANDBY_NOT_POLLED, &[&name]));
        }
        if self.seen == Some(Seen::Waiting) {
            if self.watch.giving() {
                return Ok(msg::note(msg::N_STANDBY_GIVEBACK_DECIDED, &[&name]));
            }
            self.watch.hand_over();
            self.record(&msg::note(msg::N_STANDBY_REC_DECIDED, &[&name]));
            return Ok(msg::note(msg::N_STANDBY_GIVEBACK_WILL, &[&name]));
        }
        self.watch.give_back();
        self.since.clear();
        self.last = None;
        self.record_back(&msg::note(msg::N_STANDBY_REC_GAVE_BACK_CMD, &[&name]));
        Ok(match self.seen {
            Some(Seen::Nobody) => msg::note(msg::N_STANDBY_GIVEBACK_DONE_NOBODY, &[&name]),
            _ => msg::note(msg::N_STANDBY_GIVEBACK_DONE, &[&name]),
        })
    }

    /// `"ha"` of `GET /health`, read by a primary with `[primary]`; with `release`, that
    /// primary's `POST /standby/release`: let go when [`Watch::ready`], or say why not.
    pub fn handover(&mut self, release: bool) -> Result<serde_json::Value, String> {
        let Some((name, auto)) = self.cfg.as_ref().map(|c| (c.watch.trim().to_owned(), c.give_back == config::GiveBack::Auto)) else {
            return Err(line(msg::STANDBY_NOT_SET, &[]));
        };
        if release {
            if let Err(r) = self.watch.release() {
                return Err(refusal(&name, r));
            }
            self.since.clear();
            self.last = None;
            self.record_back(&msg::note(msg::N_STANDBY_REC_GAVE_BACK_RELEASE, &[&name]));
            self.told.push(Told::GaveBack(msg::note(msg::N_STANDBY_GAVE_BACK_RELEASE, &[&name])));
        }
        let ready = self.watch.ready();
        Ok(serde_json::json!({
            "role": "standby",
            "watch": name,
            "active": self.watch.active(),
            "ready": ready.is_ok(),
            "auto": auto,
            "giving": self.watch.giving(),
            "why": ready.err().map(|r| refusal(&name, r)),
            "since": (!self.since.is_empty()).then(|| self.since.clone()),
            "back": (!self.back.is_empty()).then(|| self.back.clone()).or_else(|| last_back(&self.log)),
        }))
    }

    /// Written down with the time, and kept with the state.
    fn record(&mut self, note: &str) {
        self.record_at(&stamp(), note);
    }

    /// A handback: also kept as its own time, which the window shows (`YYYY-MM-DD HH:MM`).
    fn record_back(&mut self, note: &str) {
        let at = stamp();
        self.back = at.get(..16).unwrap_or_default().to_owned();
        self.record_at(&at, note);
    }

    fn record_at(&mut self, at: &str, note: &str) {
        self.log.push(format!("{at} {note}"));
        let over = self.log.len().saturating_sub(LOG);
        self.log.drain(..over);
        self.keep();
    }

    /// Written at once: it is rare, and a takeover must hold over a crash right after it.
    fn keep(&mut self) {
        let k = Kept {
            active: self.watch.active(),
            since: self.since.clone(),
            giving: self.watch.giving(),
            back: self.back.clone(),
            log: self.log.clone(),
        };
        if let Ok(text) = toml::to_string(&k) {
            let _ = self.writer.save_now(text);
        }
    }

    /// The status line, when `[standby]` is written.
    pub fn line(&self) -> Option<String> {
        let cfg = self.cfg.as_ref()?;
        let name = cfg.watch.trim();
        Some(match self.watch.state() {
            State::Watching { missed: 0 } => msg::note(msg::N_STANDBY_LINE_WATCHING, &[name]),
            State::Watching { missed } => {
                msg::note(msg::N_STANDBY_LINE_WATCHING_MISSED, &[name, &missed.to_string(), &cfg.misses.to_string()])
            }
            State::Active { .. } => {
                let back = if self.watch.giving() {
                    msg::note(msg::N_STANDBY_BACK_NEXT, &[name])
                } else if cfg.give_back == config::GiveBack::Auto {
                    msg::note(msg::N_STANDBY_BACK_AUTO, &[name, &cfg.misses.to_string()])
                } else {
                    msg::note(msg::N_STANDBY_BACK_MANUAL, &[])
                };
                msg::note(msg::N_STANDBY_LINE_ACTIVE, &[name, &self.since, &back])
            }
        })
    }

    /// What `koyomi standby` says: the status line, the poll's and the copy's trouble, and the
    /// last handovers.
    pub fn status(&self) -> Option<String> {
        let line = self.line()?;
        let log = self.log.iter().rev().take(3).rev().map(|l| msg::note(msg::N_STANDBY_RECORD_LINE, &[l]));
        Some(std::iter::once(line).chain(self.notes()).chain(log).collect::<Vec<_>>().join("\n"))
    }

    /// What a person should see: the poll's trouble.
    pub fn notes(&self) -> Vec<String> {
        let name = self.cfg.as_ref().map(|c| c.watch.trim()).unwrap_or_default();
        self.note
            .iter()
            .chain(&self.runs_note)
            .chain(&self.copy_note)
            .map(|n| if msg::is_coded(n) { n.clone() } else { format!("[standby] {name}: {n}") })
            .collect()
    }
}

/// Why the standby does not let go yet.
fn refusal(name: &str, r: Refusal) -> String {
    match r {
        Refusal::Idle => line(msg::STANDBY_NOT_ACTIVE, &[name]),
        Refusal::Manual => line(msg::GIVEBACK_MANUAL, &[]),
        Refusal::NotYet { back, misses } => line(msg::GIVEBACK_NOT_YET, &[name, &back.to_string(), &misses.to_string()]),
    }
}

fn stamp() -> String {
    ito_common::now::now_local().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// When it last handed the schedules back, from the handovers written down: `YYYY-MM-DD HH:MM`.
/// Only a log from before `back` was kept is read this way, by the words it was written in.
fn last_back(log: &[String]) -> Option<String> {
    // an engine from before the IDs
    log.iter().rev().find(|l| l.contains(" に戻した（")).and_then(|l| l.get(..16)).map(str::to_owned)
}

/// One copy from `r`: the confirmed definitions there whose file differs from `here`.
fn fetch(r: &config::Remote, here: &BTreeMap<String, String>) -> Fetched {
    let name = r.secret();
    let token = match ito_secret::get("koyomi", name) {
        Ok(Some(t)) => t,
        Ok(None) => return Err(line(msg::TOKEN_SECRET_MISSING, &[&r.name, name])),
        Err(e) => return Err(line(msg::TOKEN_SECRET_UNREADABLE, &[&r.name, name, &e.to_string()])),
    };
    let reason = |e: client::Error| crate::remote::reason(&r.name, &e);
    let c = Client::new(r.url.trim(), r.fingerprint.trim(), token).map_err(reason)?;
    let list = c.get("/nets").map_err(reason)?;
    if list.status == 401 {
        return Err(line(msg::TOKEN_REFUSED, &[&r.name, name]));
    }
    if list.status != 200 {
        return Err(line(msg::UNEXPECTED_STATUS, &[&r.name, &format!("/nets {}", list.status)]));
    }
    let mut out = Vec::new();
    for id in wanted(&r.name, &list.json, here)? {
        let reply = c.get(&format!("/nets/text?net={}", crate::remote::query(&id))).map_err(reason)?;
        match reply.status {
            200 => {
                if let Some(text) = reply.json["text"].as_str() {
                    out.push(Copied { base: here.get(&id).cloned(), id, text: text.to_owned() });
                }
            }
            // Gone there since the list.
            404 => {}
            403 => return Err(line(msg::HANDOVER_REFUSED, &[&r.name, "edit"])),
            s => return Err(line(msg::UNEXPECTED_STATUS, &[&r.name, &format!("/nets/text {s} ({id})")])),
        }
    }
    Ok(out)
}

/// The ids to fetch from a `GET /nets` answer: confirmed there, and with a file hash that is
/// not the one in `here`.
fn wanted(name: &str, list: &serde_json::Value, here: &BTreeMap<String, String>) -> Result<Vec<String>, String> {
    let nets = list["nets"].as_array().ok_or_else(|| line(msg::REPLY_UNREADABLE, &[name, "/nets"]))?;
    let mut out = Vec::new();
    for n in nets {
        let Some(id) = n["id"].as_str() else { continue };
        let Some(sha) = n["sha256"].as_str() else {
            return Err(line(msg::COPY_REMOTE_OLD, &[name]));
        };
        if n["standing"] == "confirmed" && here.get(id).map(String::as_str) != Some(sha) {
            out.push(id.to_owned());
        }
    }
    Ok(out)
}

/// One poll of `r`'s `/health`; with `runs`, also the runs going on there.
fn poll(r: &config::Remote, runs: bool) -> Poll {
    let name = r.secret();
    let token = match ito_secret::get("koyomi", name) {
        Ok(Some(t)) => t,
        Ok(None) => return Poll::Unknown(line(msg::TOKEN_SECRET_MISSING, &[&r.name, name])),
        Err(e) => return Poll::Unknown(line(msg::TOKEN_SECRET_UNREADABLE, &[&r.name, name, &e.to_string()])),
    };
    let c = match Client::new(r.url.trim(), r.fingerprint.trim(), token) {
        Ok(c) => c,
        Err(e) => return judge(Err(e), &r.name, name),
    };
    match c.get("/health") {
        Ok(reply) if reply.status == 200 => {
            Poll::Answered { seen: seen_of(&reply.json), note: None, running: runs.then(|| running_of(&r.name, &c)) }
        }
        Ok(reply) => judge(Ok(reply.status), &r.name, name),
        Err(e) => judge(Err(e), &r.name, name),
    }
}

/// One read of the runs going on there, on its own thread, apart from the health poll.
fn runs_poll(r: &config::Remote) -> Result<Vec<Run>, String> {
    let name = r.secret();
    let token = match ito_secret::get("koyomi", name) {
        Ok(Some(t)) => t,
        Ok(None) => return Err(line(msg::TOKEN_SECRET_MISSING, &[&r.name, name])),
        Err(e) => return Err(line(msg::TOKEN_SECRET_UNREADABLE, &[&r.name, name, &e.to_string()])),
    };
    let c = Client::new(r.url.trim(), r.fingerprint.trim(), token).map_err(|e| crate::remote::reason(&r.name, &e))?;
    running_of(&r.name, &c)
}

/// What a `/health` reply (its status) or failure of the connection `name` (its token in `secret`) says.
fn judge(got: Result<u16, client::Error>, name: &str, secret: &str) -> Poll {
    let answered = |note: Option<String>| Poll::Answered { seen: Seen::Running, note, running: None };
    match got {
        Ok(200) => answered(None),
        Ok(401) => answered(Some(line(msg::TOKEN_REFUSED, &[name, secret]))),
        Ok(s) => answered(Some(line(msg::UNEXPECTED_STATUS, &[name, &format!("/health {s}")]))),
        Err(e @ (client::Error::Connect(_) | client::Error::Tls(_))) => Poll::Missed(crate::remote::reason(name, &e)),
        // Another PC may answer: it is not counted as no answer.
        Err(e) => Poll::Unknown(crate::remote::reason(name, &e)),
    }
}

/// What a 200 of `/health` says the primary does. One with `[primary]` says so in `"ha"`: while
/// it asks or is held it runs no schedule; otherwise it runs them, and asks before it goes on.
/// One without runs them on its own.
fn seen_of(health: &serde_json::Value) -> Seen {
    let ha = &health["ha"];
    if ha["role"] != "primary" {
        return Seen::Running;
    }
    match ha["hold"].as_str() {
        Some("asking" | "held") => Seen::Waiting,
        _ => Seen::Asks,
    }
}

/// The runs going on there (`GET /runs?running=1`), for a takeover to start from. A koyomi too
/// old for `running` answers with all of today's: the ones over are dropped here.
fn running_of(name: &str, c: &Client) -> Result<Vec<Run>, String> {
    // One more try: a reply that fails right after /health must not leave a takeover with no runs.
    let reply = c.get("/runs?running=1").or_else(|_| c.get("/runs?running=1")).map_err(|e| crate::remote::reason(name, &e))?;
    if reply.status != 200 {
        return Err(line(msg::UNEXPECTED_STATUS, &[name, &format!("/runs {}", reply.status)]));
    }
    let list = reply.json["runs"].as_array().ok_or_else(|| line(msg::REPLY_UNREADABLE, &[name, "/runs"]))?;
    let mut out = Vec::new();
    for v in list {
        let r: Run = serde_json::from_value(v.clone()).map_err(|e| line(msg::RUNS_UNREADABLE, &[name, &e.to_string()]))?;
        if !r.is_over() {
            out.push(r);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use koyomi_core::jobnet::Jobnet;
    use koyomi_core::run::How;

    fn cfg(misses: u32) -> config::Standby {
        config::Standby { watch: "pc1".into(), every: "20s".into(), misses, give_back: config::GiveBack::Manual }
    }

    fn auto(misses: u32) -> config::Standby {
        config::Standby { give_back: config::GiveBack::Auto, ..cfg(misses) }
    }

    fn remote() -> config::Remote {
        config::Remote {
            name: "pc1".into(),
            url: "https://pc1:8790".into(),
            fingerprint: "AB".repeat(32),
            token: "secret:pc1".into(),
            watch: true,
        }
    }

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("koyomi-standby-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn answered(seen: Seen) -> Poll {
        Poll::Answered { seen, note: None, running: None }
    }

    fn a_run(id: &str) -> Run {
        let at = chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap().and_hms_opt(2, 0, 0).unwrap();
        Run::new(id, "g/n", "n", "2026-11-01", How::Schedule, &Jobnet::default(), at)
    }

    /// only nobody there is a miss; a wrong certificate or a wrong token is not.
    #[test]
    fn only_nobody_there_is_a_miss() {
        assert_eq!(judge(Ok(200), "pc1", "t"), answered(Seen::Running));
        assert!(matches!(judge(Ok(401), "pc1", "t"), Poll::Answered { note: Some(_), .. }));
        assert!(matches!(judge(Err(client::Error::Connect("refused".into())), "pc1", "t"), Poll::Missed(_)));
        assert!(matches!(judge(Err(client::Error::Tls("eof".into())), "pc1", "t"), Poll::Missed(_)));
        assert!(matches!(judge(Err(client::Error::Fingerprint("x".into())), "pc1", "t"), Poll::Unknown(_)));
    }

    /// a primary with `[primary]` says in `"ha"` whether it waits; one without runs on its own.
    #[test]
    fn the_health_says_whether_the_primary_waits() {
        assert_eq!(seen_of(&serde_json::json!({ "ok": true })), Seen::Running);
        assert_eq!(seen_of(&serde_json::json!({ "ha": { "role": "standby" } })), Seen::Running);
        assert_eq!(seen_of(&serde_json::json!({ "ha": { "role": "primary", "hold": "held" } })), Seen::Waiting);
        assert_eq!(seen_of(&serde_json::json!({ "ha": { "role": "primary", "hold": "asking" } })), Seen::Waiting);
        assert_eq!(seen_of(&serde_json::json!({ "ha": { "role": "primary", "hold": "running" } })), Seen::Asks);
    }

    /// it waits while the primary answers, takes over after the misses, keeps that over a
    /// restart, and watches again after giveback.
    #[test]
    fn it_takes_over_and_keeps_it_over_a_restart() {
        let d = dir("keep");
        let mut s = Standby::new(&d);
        assert!(!s.waiting());
        s.set(Some(&cfg(2)), &[remote()]);
        assert!(s.waiting());
        assert!(s.take("pc1", Poll::Unknown("cert".into())).is_none());
        assert!(s.take("pc1", Poll::Missed("refused".into())).is_none());
        assert_eq!(s.notes().len(), 0);
        assert!(matches!(s.take("pc1", Poll::Missed("refused".into())), Some(Told::TookOver(t, None)) if t.contains("refused")));
        assert!(!s.waiting());
        let l = s.line().unwrap();
        assert!(l.contains("pc1") && l.contains(&s.since) && l.ends_with(&msg::note(msg::N_STANDBY_BACK_MANUAL, &[])), "{l}");
        assert!(matches!(s.take("pc1", answered(Seen::Running)), Some(Told::Both(_))));
        let mut again = Standby::new(&d);
        again.set(Some(&cfg(2)), &[remote()]);
        assert!(!again.waiting(), "a restart stays taken over");
        assert_eq!(
            again.give_back().unwrap_err(),
            line(msg::STANDBY_NOT_POLLED, &["pc1"]),
            "not before it knows whether the primary waits"
        );
        assert!(again.take("pc1", answered(Seen::Running)).is_some());
        assert!(again.give_back().is_ok());
        assert!(again.waiting());
        assert!(again.give_back().is_err());
        let record = msg::note(msg::N_STANDBY_RECORD_LINE, &[again.log.last().unwrap()]);
        assert!(again.status().unwrap().contains(&record), "{:?}", again.status());
        again.set(None, &[remote()]);
        assert!(!again.waiting() && again.line().is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// the takeover starts from the runs read at the last answered poll; a read that failed
    /// leaves none to start from, and so does a poll that did not read them.
    #[test]
    fn the_runs_there_are_kept_for_the_takeover() {
        let d = dir("runs");
        let mut s = Standby::new(&d);
        s.set(Some(&cfg(1)), &[remote()]);
        let runs = vec![a_run("20261101-020000-00")];
        s.take("pc1", Poll::Answered { seen: Seen::Running, note: None, running: Some(Ok(runs.clone())) });
        assert!(s.take("pc1", Poll::Unknown("cert".into())).is_none());
        assert_eq!(s.take("pc1", Poll::Missed("refused".into())).map(|t| matches!(t, Told::TookOver(_, Some(r)) if r == runs)), Some(true));
        assert!(s.give_back().is_ok());
        s.take("pc1", Poll::Answered { seen: Seen::Running, note: None, running: Some(Ok(runs.clone())) });
        let unread = |why: &str| line(msg::RUNS_UNREADABLE, &["pc1", why]);
        s.take("pc1", Poll::Answered { seen: Seen::Running, note: None, running: Some(Err(unread("x"))) });
        assert_eq!(s.notes()[0], unread("x"));
        // The failed read keeps the good list seen a moment before: it fails as the primary dies.
        assert_eq!(s.take("pc1", Poll::Missed("refused".into())).map(|t| matches!(t, Told::TookOver(_, Some(r)) if r == runs)), Some(true));
        // With no good list at all, the line says why.
        let d2 = dir("runs-none");
        let mut n = Standby::new(&d2);
        n.set(Some(&cfg(1)), &[remote()]);
        n.take("pc1", Poll::Answered { seen: Seen::Running, note: None, running: Some(Err(unread("x"))) });
        match n.take("pc1", Poll::Missed("refused".into())) {
            // The reason is kept, and its ID is not repeated inside the note.
            Some(Told::TookOver(text, None)) => assert!(text.contains(msg::uncoded(&unread("x"))) && !msg::is_coded(&text), "{text}"),
            other => panic!("{other:?}"),
        }
        assert!(s.notes().is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// a run that starts after one health poll is caught by the faster read of the runs; the
    /// reads that fail as the primary dies do not wipe it.
    #[test]
    fn a_run_seen_between_polls_survives_the_failing_reads() {
        let d = dir("runs-fast");
        let mut s = Standby::new(&d);
        s.set(Some(&cfg(2)), &[remote()]);
        s.take("pc1", Poll::Answered { seen: Seen::Running, note: None, running: Some(Ok(Vec::new())) });
        let runs = vec![a_run("20261101-020000-00")];
        s.take_runs(Ok(runs.clone()));
        s.take_runs(Err(line(msg::RUNS_UNREADABLE, &["pc1", "reset"])));
        assert!(s.take("pc1", Poll::Missed("refused".into())).is_none());
        s.take_runs(Err(line(msg::RUNS_UNREADABLE, &["pc1", "refused"])));
        let told = s.take("pc1", Poll::Missed("refused".into()));
        assert!(matches!(&told, Some(Told::TookOver(_, Some(ref r))) if *r == runs), "{told:?}");
        // A list that ended there since is replaced by the newer one.
        let d3 = dir("runs-fast2");
        let mut e = Standby::new(&d3);
        e.set(Some(&cfg(1)), &[remote()]);
        e.take_runs(Ok(runs));
        e.take_runs(Ok(Vec::new()));
        assert!(matches!(e.take("pc1", Poll::Missed("x".into())), Some(Told::TookOver(_, Some(r))) if r.is_empty()));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// auto: it steps down from a primary that runs on its own once it answered the misses in
    /// a row, and writes each handover down.
    #[test]
    fn auto_steps_down_and_writes_it_down() {
        let d = dir("auto");
        let mut s = Standby::new(&d);
        s.set(Some(&auto(2)), &[remote()]);
        s.take("pc1", Poll::Missed("refused".into()));
        assert!(
            matches!(s.take("pc1", Poll::Missed("refused".into())), Some(Told::TookOver(t, _)) if t.contains(&msg::note(msg::N_STANDBY_BACK_AUTO, &["pc1", "2"])))
        );
        assert!(
            matches!(s.take("pc1", answered(Seen::Running)), Some(Told::Both(t)) if t == msg::note(msg::N_STANDBY_BOTH_AUTO, &["pc1", "2"]))
        );
        assert!(matches!(s.take("pc1", answered(Seen::Running)), Some(Told::GaveBack(_))));
        assert!(s.waiting());
        let again = Standby::new(&d);
        assert_eq!(again.log.len(), 2);
        assert!(again.log[0].ends_with(&msg::note(msg::N_STANDBY_REC_TOOK_OVER, &["pc1", "refused"])), "{:?}", again.log);
        assert!(again.log[1].ends_with(&msg::note(msg::N_STANDBY_REC_GAVE_BACK_AUTO, &["pc1", "2"])), "{:?}", again.log);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// a waiting primary is let go to only when this one is ready: a person's giveback, or
    /// with auto the misses answered; the giveback holds over a restart.
    #[test]
    fn a_waiting_primary_is_let_go_to_when_ready() {
        let d = dir("release");
        let mut s = Standby::new(&d);
        assert!(s.handover(false).is_err(), "no [standby]");
        s.set(Some(&cfg(1)), &[remote()]);
        assert_eq!(s.handover(false).unwrap()["active"], false);
        assert_eq!(s.handover(true).unwrap_err(), line(msg::STANDBY_NOT_ACTIVE, &["pc1"]));
        assert!(matches!(s.take("pc1", Poll::Missed("refused".into())), Some(Told::TookOver(..))));
        let ha = s.handover(false).unwrap();
        assert_eq!((ha["active"].as_bool(), ha["ready"].as_bool()), (Some(true), Some(false)));
        assert_eq!(ha["why"].as_str(), Some(line(msg::GIVEBACK_MANUAL, &[]).as_str()));
        assert_eq!((ha["since"].as_str().map(str::len), ha["back"].as_str()), (Some(16), None), "{ha}");
        assert!(s.handover(true).is_err());
        assert!(matches!(s.take("pc1", answered(Seen::Waiting)), Some(Told::Waiting(_))));
        assert_eq!(s.give_back().unwrap(), msg::note(msg::N_STANDBY_GIVEBACK_WILL, &["pc1"]));
        assert!(!s.waiting(), "it runs until the primary comes for them");
        assert_eq!(s.give_back().unwrap(), msg::note(msg::N_STANDBY_GIVEBACK_DECIDED, &["pc1"]));
        let mut again = Standby::new(&d);
        again.set(Some(&cfg(1)), &[remote()]);
        assert_eq!(again.handover(false).unwrap()["ready"], true);
        let back = again.handover(true).unwrap();
        assert_eq!((back["active"].as_bool(), back["since"].as_str()), (Some(false), None));
        assert_eq!(back["back"].as_str(), again.log.last().and_then(|l| l.get(..16)));
        assert!(back["back"].as_str().is_some());
        assert!(again.waiting());
        assert!(matches!(again.told.as_slice(), [Told::GaveBack(_)]));
        assert!(again.log.last().unwrap().ends_with(&msg::note(msg::N_STANDBY_REC_GAVE_BACK_RELEASE, &["pc1"])));

        let mut s = Standby::new(&dir("release-auto"));
        s.set(Some(&auto(2)), &[remote()]);
        s.take("pc1", Poll::Missed("refused".into()));
        s.take("pc1", Poll::Missed("refused".into()));
        s.take("pc1", answered(Seen::Waiting));
        assert_eq!(s.handover(true).unwrap_err(), line(msg::GIVEBACK_NOT_YET, &["pc1", "1", "2"]));
        s.take("pc1", answered(Seen::Waiting));
        assert!(s.handover(true).is_ok());
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(dir("release-auto"));
    }

    /// the last give-back is read off the handovers written down; a decision to give back is not one.
    #[test]
    fn the_last_give_back_comes_from_the_log() {
        let log = |ls: &[&str]| ls.iter().map(|l| (*l).to_owned()).collect::<Vec<_>>();
        assert_eq!(last_back(&[]), None);
        assert_eq!(last_back(&log(&["2026-10-30 09:00:01 pc1 から引き継いだ（refused）"])), None);
        let ls = log(&[
            "2026-10-30 09:00:01 pc1 に戻した（koyomi standby giveback）",
            "2026-10-31 08:00:00 pc1 から引き継いだ（refused）",
            "2026-10-31 10:15:30 pc1 に戻した（向こうが引き取りに来た）",
            "2026-10-31 11:00:00 戻すと決めた（koyomi standby giveback。pc1 が次に聞いてきたら渡す）",
        ]);
        assert_eq!(last_back(&ls).as_deref(), Some("2026-10-31 10:15"));
    }

    /// the time of the last handback is kept as its own field: it does not depend on the words of
    /// a log line, and an old log is still read by its old words.
    #[test]
    fn the_last_give_back_is_kept_apart_from_the_words() {
        let d = dir("back-field");
        let mut s = Standby::new(&d);
        s.set(Some(&cfg(1)), &[remote()]);
        assert!(matches!(s.take("pc1", Poll::Missed("refused".into())), Some(Told::TookOver(..))));
        assert_eq!(s.handover(false).unwrap()["back"], serde_json::Value::Null);
        assert!(s.give_back().is_ok());
        let back = s.handover(false).unwrap()["back"].as_str().map(str::to_owned);
        assert_eq!(back.as_deref().map(str::len), Some(16));
        // A restart reads it back, though no log line says "handed back" in the old words.
        let mut again = Standby::new(&d);
        again.log.clear();
        again.set(Some(&cfg(1)), &[remote()]);
        assert_eq!(again.handover(false).unwrap()["back"].as_str().map(str::to_owned), back);
        // A file kept before `back` existed has none: the old log is what tells.
        let kept: Kept = toml::from_str("active = false\nsince = \"\"\ngiving = false\nlog = [\"x\"]\n").unwrap();
        assert_eq!((kept.back.as_str(), kept.log.len()), ("", 1));
        let mut old = Standby::new(&dir("back-old"));
        old.log = vec!["2026-10-31 10:15:30 pc1 に戻した（koyomi standby giveback）".to_owned()];
        old.set(Some(&cfg(1)), &[remote()]);
        assert_eq!(old.handover(false).unwrap()["back"].as_str(), Some("2026-10-31 10:15"));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(dir("back-old"));
    }

    /// only the confirmed nets there whose file is not the one here are fetched; a peer
    /// without the hashes is said to be too old.
    #[test]
    fn only_what_changed_there_is_fetched() {
        let list = serde_json::json!({ "nets": [
            { "id": "a/same", "standing": "confirmed", "sha256": "1" },
            { "id": "a/changed", "standing": "confirmed", "sha256": "2" },
            { "id": "a/new", "standing": "confirmed", "sha256": "3" },
            { "id": "a/unconfirmed", "standing": "unconfirmed", "sha256": "4" },
            { "id": "a/invalid", "standing": "invalid", "sha256": "5" },
        ]});
        let here = BTreeMap::from([("a/same".to_owned(), "1".to_owned()), ("a/changed".to_owned(), "old".to_owned())]);
        assert_eq!(wanted("pc1", &list, &here).unwrap(), ["a/changed", "a/new"]);
        let old = serde_json::json!({ "nets": [{ "id": "a/x", "standing": "confirmed" }] });
        assert_eq!(wanted("pc1", &old, &here).unwrap_err(), line(msg::COPY_REMOTE_OLD, &["pc1"]));
        assert_eq!(wanted("pc1", &serde_json::json!({}), &here).unwrap_err(), line(msg::REPLY_UNREADABLE, &["pc1", "/nets"]));
    }

    /// what the copy thread brings is handed over while standing by, and dropped once it
    /// has taken over; a failure is shown until a copy goes through.
    #[test]
    fn copies_are_dropped_after_a_takeover() {
        let d = dir("copy");
        let mut s = Standby::new(&d);
        let one = Copied { id: "a/b".into(), text: "x".into(), base: None };
        // Not standing by: nothing is started or handed over.
        s.copy_tx.send(Ok(vec![one.clone()])).unwrap();
        assert!(s.copies(BTreeMap::new).is_empty());
        assert!(s.copied_at.is_none());
        s.set(Some(&cfg(1)), &[remote()]);
        s.copied_at = Some(Instant::now());
        let trouble = line(msg::COPY_REMOTE_OLD, &["pc1"]);
        s.copy_tx.send(Err(trouble.clone())).unwrap();
        assert!(s.copies(BTreeMap::new).is_empty());
        assert_eq!(s.notes().len(), 1);
        // `koyomi standby` says the trouble under the status line.
        let status = s.status().unwrap();
        assert!(status.starts_with(&msg::note(msg::N_STANDBY_LINE_WATCHING, &["pc1"])) && status.ends_with(&trouble), "{status}");
        s.copy_tx.send(Ok(vec![one.clone()])).unwrap();
        assert_eq!(s.copies(BTreeMap::new), std::slice::from_ref(&one));
        assert!(s.notes().is_empty());
        s.refused(vec!["a/b: 保存できない".into()]);
        assert!(msg::COPY_NOT_WRITTEN.is_in(&s.notes()[0]) && s.notes()[0].contains("a/b: 保存できない"), "{:?}", s.notes());
        assert!(matches!(s.take("pc1", Poll::Missed("refused".into())), Some(Told::TookOver(..))));
        s.copy_tx.send(Ok(vec![one])).unwrap();
        assert!(s.copies(BTreeMap::new).is_empty());
        // The copy's trouble goes with the takeover, and a giveback copies at once.
        assert!(s.notes().is_empty());
        assert!(s.copied_at.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
