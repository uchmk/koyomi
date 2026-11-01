//! `[standby]` of koyomi.toml (TODO 87 and 96, DECISIONS「複数の PC・Web UI・利用者とロール・HA」):
//! this engine stands by for the `[[remote]]` named `watch`. It polls that engine's
//! `GET /health` on a thread every `every`; while it answers, this one runs no schedule and
//! no event (a person's `koyomi run` still goes). After `misses` polls in a row that get no
//! answer, it takes over and runs the times from then on.
//!
//! While it watches, each poll also reads the runs going on there (`GET /runs?running=1`). On a
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

use chrono::Local;
use ito_common::kept;
use ito_http::client::{self, Client};
use koyomi_core::config;
use koyomi_core::run::Run;
use koyomi_core::standby::{Refusal, Seen, State, Step, Watch};
use serde::{Deserialize, Serialize};

/// How often a standby copies the definitions from the engine it watches.
pub const COPY_EVERY: Duration = Duration::from_secs(300);

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
        let misses = koyomi_core::standby::MISSES;
        Standby {
            cfg: None,
            remote: None,
            watch: if prev.active { Watch::resumed(misses, false, prev.giving) } else { Watch::new(misses, false) },
            writer: kept::Writer::new(path, true),
            since: prev.since,
            log: prev.log,
            last: None,
            tx,
            rx,
            in_flight: false,
            polled: false,
            seen: None,
            running: None,
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
            self.copy_note = Some(format!("写した定義を置けない: {}", why.join(" / ")));
        }
    }

    fn take(&mut self, name: &str, p: Poll) -> Option<Told> {
        self.polled = true;
        let seen = match p {
            Poll::Answered { seen, note, running } => {
                self.note = note;
                // Only the latest list is started from: an older one may hold runs that have
                // ended there since.
                match running {
                    Some(Ok(r)) => {
                        self.running = Some(r);
                        self.runs_note = None;
                    }
                    Some(Err(e)) => {
                        self.running = None;
                        self.runs_note = Some(e);
                    }
                    None => {
                        self.running = None;
                        self.runs_note = None;
                    }
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
                self.since = Local::now().format("%Y-%m-%d %H:%M").to_string();
                let why = self.missed_why.clone().unwrap_or_default();
                self.record(format!("{} {name} から引き継いだ（{why}）", stamp()));
                self.runs_note = None;
                let back = if auto {
                    format!("{name} が {misses} 回続けて答えたら戻す")
                } else {
                    "戻すのは koyomi standby giveback".to_owned()
                };
                let line = format!("{name} が答えなくなった（{why}）ので、{} からこの PC で流す。{back}", self.since);
                Some(Told::TookOver(line, self.running.take()))
            }
            Step::BothActive => Some(Told::Both(match seen {
                Seen::Asks => format!("{name} がまた答えている。分かれていた間は両方で流していた。向こうは次に聞いてきたら待つ"),
                _ if auto => format!("{name} がまた答えている。両方で流している。{name} が {misses} 回続けて答えたらこちらは止める"),
                _ => format!("{name} がまた答えている。両方で流している。どちらかを止めるか、こちらで koyomi standby giveback で戻す"),
            })),
            Step::Waiting => Some(Told::Waiting(if self.watch.giving() {
                format!("{name} が引き取りを待っている。次に聞いてきたら渡す")
            } else if auto {
                format!("{name} がまた答えて、引き取りを待っている。{misses} 回続けて答えたら渡す")
            } else {
                format!("{name} がまた答えて、引き取りを待っている。渡すのは koyomi standby giveback")
            })),
            Step::StepDown => {
                self.since.clear();
                self.last = None;
                self.record(format!("{} {name} に戻した（{misses} 回続けて答えた。give_back = \"auto\"）", stamp()));
                Some(Told::GaveBack(format!("{name} が {misses} 回続けて答えたので、こちらで流すのをやめた。また見張る")))
            }
        }
    }

    /// `koyomi standby giveback`. When the primary waits for this one (`[primary]`), it lets go
    /// at the primary's next ask, so the schedules are never left to nobody; otherwise at once.
    pub fn give_back(&mut self) -> Result<String, String> {
        let Some(name) = self.cfg.as_ref().map(|c| c.watch.trim().to_owned()) else {
            return Err("koyomi.toml に [standby] がない".to_owned());
        };
        if !self.watch.active() {
            return Err(format!("引き継いでいない（{name} を見張っている）"));
        }
        if !self.polled {
            // After a restart: whether the primary waits for this one is not known yet.
            return Err(format!("{name} をまだ見ていない。少しあとで"));
        }
        if self.seen == Some(Seen::Waiting) {
            if self.watch.giving() {
                return Ok(format!("{name} が次に聞いてきたら戻す（もう決めてある）"));
            }
            self.watch.hand_over();
            self.record(format!("{} 戻すと決めた（koyomi standby giveback。{name} が次に聞いてきたら渡す）", stamp()));
            return Ok(format!("{name} が引き取りを待っているので、次に聞いてきたら戻す"));
        }
        self.watch.give_back();
        self.since.clear();
        self.last = None;
        self.record(format!("{} {name} に戻した（koyomi standby giveback）", stamp()));
        Ok(match self.seen {
            Some(Seen::Nobody) => format!("{name} に戻した。向こうは今答えていない。また見張る"),
            _ => format!("{name} に戻した。また見張る"),
        })
    }

    /// `"ha"` of `GET /health`, read by a primary with `[primary]`; with `release`, that
    /// primary's `POST /standby/release`: let go when [`Watch::ready`], or say why not.
    pub fn handover(&mut self, release: bool) -> Result<serde_json::Value, String> {
        let Some((name, auto)) = self.cfg.as_ref().map(|c| (c.watch.trim().to_owned(), c.give_back == config::GiveBack::Auto)) else {
            return Err("koyomi.toml に [standby] がない".to_owned());
        };
        if release {
            if let Err(r) = self.watch.release() {
                return Err(refusal(&name, r));
            }
            self.since.clear();
            self.last = None;
            self.record(format!("{} {name} に戻した（向こうが引き取りに来た）", stamp()));
            self.told.push(Told::GaveBack(format!("{name} が引き取りに来たので戻した。また見張る")));
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
        }))
    }

    /// Written down with the time, and kept with the state.
    fn record(&mut self, line: String) {
        self.log.push(line);
        let over = self.log.len().saturating_sub(LOG);
        self.log.drain(..over);
        self.keep();
    }

    /// Written at once: it is rare, and a takeover must hold over a crash right after it.
    fn keep(&mut self) {
        let k = Kept { active: self.watch.active(), since: self.since.clone(), giving: self.watch.giving(), log: self.log.clone() };
        if let Ok(text) = toml::to_string(&k) {
            let _ = self.writer.save_now(text);
        }
    }

    /// The status line, when `[standby]` is written.
    pub fn line(&self) -> Option<String> {
        let cfg = self.cfg.as_ref()?;
        let name = cfg.watch.trim();
        Some(match self.watch.state() {
            State::Watching { missed: 0 } => format!("待機中: {name} を見張っている。こちらではスケジュールとイベントで流さない"),
            State::Watching { missed } => format!(
                "待機中: {name} を見張っている（答えなかった {missed} 回、{} 回で引き継ぐ）。こちらではスケジュールとイベントで流さない",
                cfg.misses
            ),
            State::Active { .. } => {
                let back = if self.watch.giving() {
                    format!("{name} が次に聞いてきたら戻す")
                } else if cfg.give_back == config::GiveBack::Auto {
                    format!("{name} が {} 回続けて答えたら戻す", cfg.misses)
                } else {
                    "koyomi standby giveback で戻す".to_owned()
                };
                format!("{name} から引き継いで流している（{} から。{back}）", self.since)
            }
        })
    }

    /// What `koyomi standby` says: the status line, the poll's and the copy's trouble, and the
    /// last handovers.
    pub fn status(&self) -> Option<String> {
        let line = self.line()?;
        let log = self.log.iter().rev().take(3).rev().map(|l| format!("[standby] 記録: {l}"));
        Some(std::iter::once(line).chain(self.notes()).chain(log).collect::<Vec<_>>().join("\n"))
    }

    /// What a person should see: the poll's trouble.
    pub fn notes(&self) -> Vec<String> {
        let name = self.cfg.as_ref().map(|c| c.watch.trim()).unwrap_or_default();
        self.note.iter().chain(&self.runs_note).chain(&self.copy_note).map(|n| format!("[standby] {name}: {n}")).collect()
    }
}

/// Why the standby does not let go yet.
fn refusal(name: &str, r: Refusal) -> String {
    match r {
        Refusal::Idle => format!("引き継いでいない（{name} を見張っている）"),
        Refusal::Manual => "give_back = \"manual\"。こちらで koyomi standby giveback を流すまで渡さない".to_owned(),
        Refusal::NotYet { back, misses } => format!("{name} が答えたのは続けて {back} 回。{misses} 回で渡す"),
    }
}

fn stamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// One copy from `r`: the confirmed definitions there whose file differs from `here`.
fn fetch(r: &config::Remote, here: &BTreeMap<String, String>) -> Fetched {
    let name = r.secret();
    let token = match ito_secret::get("koyomi", name) {
        Ok(Some(t)) => t,
        Ok(None) => return Err(format!("定義を写せない: token の秘密 {name} がない")),
        Err(e) => return Err(format!("定義を写せない: token の秘密 {name} が読めない（{e}）")),
    };
    let c = Client::new(r.url.trim(), r.fingerprint.trim(), token).map_err(|e| format!("定義を写せない: {e}"))?;
    let list = c.get("/nets").map_err(|e| format!("定義を写せない: {e}"))?;
    if list.status != 200 {
        return Err(format!("定義を写せない: /nets が {} を返した", list.status));
    }
    let mut out = Vec::new();
    for id in wanted(&list.json, here)? {
        let reply = c.get(&format!("/nets/text?net={}", crate::remote::query(&id))).map_err(|e| format!("定義を写せない: {e}"))?;
        match reply.status {
            200 => {
                if let Some(text) = reply.json["text"].as_str() {
                    out.push(Copied { base: here.get(&id).cloned(), id, text: text.to_owned() });
                }
            }
            // Gone there since the list.
            404 => {}
            403 => {
                return Err("定義を写せない: 向こうでこの token に edit のロールが要る（向こうの [http] の利用者とロールを見る）".to_owned())
            }
            s => return Err(format!("定義を写せない: /nets/text が {s} を返した（{id}）")),
        }
    }
    Ok(out)
}

/// The ids to fetch from a `GET /nets` answer: confirmed there, and with a file hash that is
/// not the one in `here`.
fn wanted(list: &serde_json::Value, here: &BTreeMap<String, String>) -> Result<Vec<String>, String> {
    let nets = list["nets"].as_array().ok_or("定義を写せない: /nets の答えが読めない")?;
    let mut out = Vec::new();
    for n in nets {
        let Some(id) = n["id"].as_str() else { continue };
        let Some(sha) = n["sha256"].as_str() else {
            return Err("定義を写せない: 向こうの koyomi が古い（/nets にハッシュがない）。写すには向こうを新しくする".to_owned());
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
        Ok(None) => return Poll::Unknown(format!("token の秘密 {name} がない（koyomi secret set {name}）。見張れていない")),
        Err(e) => return Poll::Unknown(format!("token の秘密 {name} が読めない（{e}）。見張れていない")),
    };
    let c = match Client::new(r.url.trim(), r.fingerprint.trim(), token) {
        Ok(c) => c,
        Err(e) => return judge(Err(e), name),
    };
    match c.get("/health") {
        Ok(reply) if reply.status == 200 => {
            Poll::Answered { seen: seen_of(&reply.json), note: None, running: runs.then(|| running_of(&c)) }
        }
        Ok(reply) => judge(Ok(reply.status), name),
        Err(e) => judge(Err(e), name),
    }
}

/// What a `/health` reply (its status) or failure says.
fn judge(got: Result<u16, client::Error>, secret: &str) -> Poll {
    let answered = |note: Option<String>| Poll::Answered { seen: Seen::Running, note, running: None };
    match got {
        Ok(200) => answered(None),
        Ok(401) => answered(Some(format!("token が合わない（向こうの [http] の token と koyomi secret set {secret} を見る）"))),
        Ok(s) => answered(Some(format!("/health が {s} を返した"))),
        Err(client::Error::Connect(s)) => Poll::Missed(format!("つながらない: {s}")),
        Err(client::Error::Tls(s)) => Poll::Missed(format!("TLS が通らない: {s}")),
        Err(client::Error::Fingerprint(got)) => {
            Poll::Unknown(format!("証明書が fingerprint と違う（今見えたのは {got}）。別の PC かもしれないので、答えなかったとは数えない"))
        }
        Err(e) => Poll::Unknown(format!("{e}。見張れていない")),
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
fn running_of(c: &Client) -> Result<Vec<Run>, String> {
    let reply = c.get("/runs?running=1").map_err(|e| format!("動いている回が読めない: {e}"))?;
    if reply.status != 200 {
        return Err(format!("動いている回が読めない: /runs が {} を返した", reply.status));
    }
    let list = reply.json["runs"].as_array().ok_or("動いている回が読めない: /runs の答えが読めない")?;
    let mut out = Vec::new();
    for v in list {
        let r: Run = serde_json::from_value(v.clone())
            .map_err(|e| format!("動いている回が読めない（向こうの koyomi と版が違うかもしれない）: {e}"))?;
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
        config::Remote { name: "pc1".into(), url: "https://pc1:8790".into(), fingerprint: "AB".repeat(32), token: "secret:pc1".into() }
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
        assert_eq!(judge(Ok(200), "t"), answered(Seen::Running));
        assert!(matches!(judge(Ok(401), "t"), Poll::Answered { note: Some(_), .. }));
        assert!(matches!(judge(Err(client::Error::Connect("refused".into())), "t"), Poll::Missed(_)));
        assert!(matches!(judge(Err(client::Error::Tls("eof".into())), "t"), Poll::Missed(_)));
        assert!(matches!(judge(Err(client::Error::Fingerprint("x".into())), "t"), Poll::Unknown(_)));
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
        assert!(s.line().unwrap().contains("引き継いで"));
        assert!(matches!(s.take("pc1", answered(Seen::Running)), Some(Told::Both(_))));
        let mut again = Standby::new(&d);
        again.set(Some(&cfg(2)), &[remote()]);
        assert!(!again.waiting(), "a restart stays taken over");
        assert!(again.give_back().unwrap_err().contains("まだ見ていない"), "not before it knows whether the primary waits");
        assert!(again.take("pc1", answered(Seen::Running)).is_some());
        assert!(again.give_back().is_ok());
        assert!(again.waiting());
        assert!(again.give_back().is_err());
        assert!(again.status().unwrap().contains("記録"), "{:?}", again.status());
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
        s.take("pc1", Poll::Answered { seen: Seen::Running, note: None, running: Some(Err("動いている回が読めない: x".into())) });
        assert!(s.notes()[0].contains("動いている回が読めない"));
        assert!(matches!(s.take("pc1", Poll::Missed("refused".into())), Some(Told::TookOver(_, None))));
        assert!(s.notes().is_empty());
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
            matches!(s.take("pc1", Poll::Missed("refused".into())), Some(Told::TookOver(t, _)) if t.contains("2 回続けて答えたら戻す"))
        );
        assert!(matches!(s.take("pc1", answered(Seen::Running)), Some(Told::Both(t)) if t.contains("止める")));
        assert!(matches!(s.take("pc1", answered(Seen::Running)), Some(Told::GaveBack(_))));
        assert!(s.waiting());
        let again = Standby::new(&d);
        assert_eq!(again.log.len(), 2);
        assert!(again.log[0].contains("引き継いだ") && again.log[1].contains("give_back = \"auto\""), "{:?}", again.log);
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
        assert!(s.handover(true).unwrap_err().contains("見張っている"));
        assert!(matches!(s.take("pc1", Poll::Missed("refused".into())), Some(Told::TookOver(..))));
        let ha = s.handover(false).unwrap();
        assert_eq!((ha["active"].as_bool(), ha["ready"].as_bool()), (Some(true), Some(false)));
        assert!(ha["why"].as_str().unwrap().contains("manual"));
        assert!(s.handover(true).is_err());
        assert!(matches!(s.take("pc1", answered(Seen::Waiting)), Some(Told::Waiting(_))));
        assert!(s.give_back().unwrap().contains("次に聞いてきたら"));
        assert!(!s.waiting(), "it runs until the primary comes for them");
        assert!(s.give_back().unwrap().contains("もう決めてある"));
        let mut again = Standby::new(&d);
        again.set(Some(&cfg(1)), &[remote()]);
        assert_eq!(again.handover(false).unwrap()["ready"], true);
        assert_eq!(again.handover(true).unwrap()["active"], false);
        assert!(again.waiting());
        assert!(matches!(again.told.as_slice(), [Told::GaveBack(_)]));
        assert!(again.log.last().unwrap().contains("引き取りに来た"));

        let mut s = Standby::new(&dir("release-auto"));
        s.set(Some(&auto(2)), &[remote()]);
        s.take("pc1", Poll::Missed("refused".into()));
        s.take("pc1", Poll::Missed("refused".into()));
        s.take("pc1", answered(Seen::Waiting));
        assert!(s.handover(true).unwrap_err().contains("続けて 1 回"));
        s.take("pc1", answered(Seen::Waiting));
        assert!(s.handover(true).is_ok());
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(dir("release-auto"));
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
        assert_eq!(wanted(&list, &here).unwrap(), ["a/changed", "a/new"]);
        let old = serde_json::json!({ "nets": [{ "id": "a/x", "standing": "confirmed" }] });
        assert!(wanted(&old, &here).unwrap_err().contains("古い"));
        assert!(wanted(&serde_json::json!({}), &here).is_err());
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
        s.copy_tx.send(Err("定義を写せない: x".into())).unwrap();
        assert!(s.copies(BTreeMap::new).is_empty());
        assert_eq!(s.notes().len(), 1);
        // `koyomi standby` says the trouble under the status line.
        let status = s.status().unwrap();
        assert!(status.starts_with("待機中") && status.ends_with("定義を写せない: x"), "{status}");
        s.copy_tx.send(Ok(vec![one.clone()])).unwrap();
        assert_eq!(s.copies(BTreeMap::new), std::slice::from_ref(&one));
        assert!(s.notes().is_empty());
        s.refused(vec!["a/b: 保存できない".into()]);
        assert!(s.notes()[0].contains("置けない"));
        assert!(matches!(s.take("pc1", Poll::Missed("refused".into())), Some(Told::TookOver(..))));
        s.copy_tx.send(Ok(vec![one])).unwrap();
        assert!(s.copies(BTreeMap::new).is_empty());
        // The copy's trouble goes with the takeover, and a giveback copies at once.
        assert!(s.notes().is_empty());
        assert!(s.copied_at.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
