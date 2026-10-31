//! `[standby]` of koyomi.toml (TODO 87, DECISIONS「複数の PC・Web UI・利用者とロール・HA」):
//! this engine stands by for the `[[remote]]` named `watch`. It polls that engine's
//! `GET /health` on a thread every `every`; while it answers, this one runs no schedule and
//! no event (a person's `koyomi run` still goes). After `misses` polls in a row that get no
//! answer, it takes over and runs the times from then on. Runs that were going on there are
//! not taken over. It hands back only by `koyomi standby giveback`.
//!
//! The takeover is kept in `standby.toml` in the state folder, so a restart while it has
//! taken over does not wait for the misses again.
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
use koyomi_core::standby::{State, Step, Watch};
use serde::{Deserialize, Serialize};

/// How often a standby copies the definitions from the engine it watches.
pub const COPY_EVERY: Duration = Duration::from_secs(300);

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
}

/// What one poll found.
#[derive(Debug, Clone, PartialEq)]
enum Poll {
    /// The engine there answered; with what a person should see when the answer was not 200.
    Answered(Option<String>),
    /// Nobody answered.
    Missed(String),
    /// Cannot tell: the certificate, the token or the address is wrong here.
    Unknown(String),
}

/// What the engine tells after a tick.
pub enum Told {
    TookOver(String),
    Both(String),
}

pub struct Standby {
    cfg: Option<config::Standby>,
    remote: Option<config::Remote>,
    watch: Watch,
    writer: kept::Writer,
    since: String,
    last: Option<Instant>,
    tx: mpsc::Sender<Poll>,
    rx: mpsc::Receiver<Poll>,
    in_flight: bool,
    /// What the last poll said was wrong.
    note: Option<String>,
    /// Why the last miss was a miss.
    missed_why: Option<String>,
    copy_tx: mpsc::Sender<Fetched>,
    copy_rx: mpsc::Receiver<Fetched>,
    copied_at: Option<Instant>,
    copying: bool,
    /// Why the last copy did not go, or which ones were not written here.
    copy_note: Option<String>,
}

impl Standby {
    /// Reads what the previous engine left: whether it had taken over.
    pub fn new(state_dir: &Path) -> Standby {
        let path = state_dir.join("standby.toml");
        let prev: Kept = kept::read(&path).unwrap_or_default();
        let (tx, rx) = mpsc::channel();
        let (copy_tx, copy_rx) = mpsc::channel();
        Standby {
            cfg: None,
            remote: None,
            watch: if prev.active { Watch::resumed(koyomi_core::standby::MISSES) } else { Watch::new(koyomi_core::standby::MISSES) },
            writer: kept::Writer::new(path, true),
            since: prev.since,
            last: None,
            tx,
            rx,
            in_flight: false,
            note: None,
            missed_why: None,
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
        if cfg.map(|c| c.misses) != self.cfg.as_ref().map(|c| c.misses) {
            let misses = cfg.map_or(koyomi_core::standby::MISSES, |c| c.misses);
            self.watch = if self.watch.active() { Watch::resumed(misses) } else { Watch::new(misses) };
        }
        if remote != self.remote {
            self.last = None;
            self.note = None;
            self.copied_at = None;
            self.copy_note = None;
        }
        self.cfg = cfg.cloned();
        self.remote = remote;
    }

    /// Stands by: the schedules and the events here wait.
    pub fn waiting(&self) -> bool {
        self.cfg.is_some() && !self.watch.active()
    }

    /// Polls when due and takes in what came back; what to tell of.
    pub fn tick(&mut self) -> Option<Told> {
        let (every, remote) = (self.cfg.as_ref()?.every_secs(), self.remote.clone()?);
        let mut told = None;
        while let Ok(p) = self.rx.try_recv() {
            self.in_flight = false;
            told = self.take(&remote.name, p).or(told);
        }
        if self.in_flight || self.last.is_some_and(|l| l.elapsed() < Duration::from_secs(every)) {
            return told;
        }
        self.last = Some(Instant::now());
        self.in_flight = true;
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new().name("standby".into()).spawn(move || {
            let _ = tx.send(poll(&remote));
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
        let answered = match p {
            Poll::Answered(note) => {
                self.note = note;
                true
            }
            Poll::Missed(why) => {
                self.note = None;
                self.missed_why = Some(why);
                false
            }
            Poll::Unknown(why) => {
                self.note = Some(why);
                return None;
            }
        };
        match self.watch.poll(answered) {
            Step::Stay => None,
            Step::TakeOver => {
                self.since = Local::now().format("%Y-%m-%d %H:%M").to_string();
                self.keep();
                let why = self.missed_why.clone().unwrap_or_default();
                Some(Told::TookOver(format!(
                    "{name} が答えなくなった（{why}）ので、{} からこの PC で流す。向こうで動いていた回は引き継いでいない。戻すのは koyomi standby giveback",
                    self.since
                )))
            }
            Step::BothActive => Some(Told::Both(format!(
                "{name} がまた答えている。両方で流している。どちらかを止めるか、こちらで koyomi standby giveback で戻す"
            ))),
        }
    }

    /// `koyomi standby giveback`.
    pub fn give_back(&mut self) -> Result<String, String> {
        let Some(name) = self.cfg.as_ref().map(|c| c.watch.trim().to_owned()) else {
            return Err("koyomi.toml に [standby] がない".to_owned());
        };
        if !self.watch.give_back() {
            return Err(format!("引き継いでいない（{name} を見張っている）"));
        }
        self.since.clear();
        self.last = None;
        self.keep();
        Ok(format!("{name} に戻した。また見張る"))
    }

    /// Written at once: it is rare, and a takeover must hold over a crash right after it.
    fn keep(&mut self) {
        if let Ok(text) = toml::to_string(&Kept { active: self.watch.active(), since: self.since.clone() }) {
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
            State::Active { .. } => format!("{name} から引き継いで流している（{} から。koyomi standby giveback で戻す）", self.since),
        })
    }

    /// What `koyomi standby` says: the status line, then the poll's and the copy's trouble.
    pub fn status(&self) -> Option<String> {
        let line = self.line()?;
        Some(std::iter::once(line).chain(self.notes()).collect::<Vec<_>>().join(
            "
",
        ))
    }

    /// What a person should see: the poll's trouble.
    pub fn notes(&self) -> Vec<String> {
        let name = self.cfg.as_ref().map(|c| c.watch.trim()).unwrap_or_default();
        self.note.iter().chain(&self.copy_note).map(|n| format!("[standby] {name}: {n}")).collect()
    }
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

/// One poll of `r`'s `/health`.
fn poll(r: &config::Remote) -> Poll {
    let name = r.secret();
    let token = match ito_secret::get("koyomi", name) {
        Ok(Some(t)) => t,
        Ok(None) => return Poll::Unknown(format!("token の秘密 {name} がない（koyomi secret set {name}）。見張れていない")),
        Err(e) => return Poll::Unknown(format!("token の秘密 {name} が読めない（{e}）。見張れていない")),
    };
    match Client::new(r.url.trim(), r.fingerprint.trim(), token).and_then(|c| c.get("/health")) {
        Ok(reply) => judge(Ok(reply.status), r.secret()),
        Err(e) => judge(Err(e), r.secret()),
    }
}

/// What a `/health` reply (its status) or failure says.
fn judge(got: Result<u16, client::Error>, secret: &str) -> Poll {
    match got {
        Ok(200) => Poll::Answered(None),
        Ok(401) => Poll::Answered(Some(format!("token が合わない（向こうの [http] の token と koyomi secret set {secret} を見る）"))),
        Ok(s) => Poll::Answered(Some(format!("/health が {s} を返した"))),
        Err(client::Error::Connect(s)) => Poll::Missed(format!("つながらない: {s}")),
        Err(client::Error::Tls(s)) => Poll::Missed(format!("TLS が通らない: {s}")),
        Err(client::Error::Fingerprint(got)) => {
            Poll::Unknown(format!("証明書が fingerprint と違う（今見えたのは {got}）。別の PC かもしれないので、答えなかったとは数えない"))
        }
        Err(e) => Poll::Unknown(format!("{e}。見張れていない")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(misses: u32) -> config::Standby {
        config::Standby { watch: "pc1".into(), every: "20s".into(), misses }
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

    /// only nobody there is a miss; a wrong certificate or a wrong token is not.
    #[test]
    fn only_nobody_there_is_a_miss() {
        assert_eq!(judge(Ok(200), "t"), Poll::Answered(None));
        assert!(matches!(judge(Ok(401), "t"), Poll::Answered(Some(_))));
        assert!(matches!(judge(Err(client::Error::Connect("refused".into())), "t"), Poll::Missed(_)));
        assert!(matches!(judge(Err(client::Error::Tls("eof".into())), "t"), Poll::Missed(_)));
        assert!(matches!(judge(Err(client::Error::Fingerprint("x".into())), "t"), Poll::Unknown(_)));
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
        assert!(matches!(s.take("pc1", Poll::Missed("refused".into())), Some(Told::TookOver(t)) if t.contains("refused")));
        assert!(!s.waiting());
        assert!(s.line().unwrap().contains("引き継いで"));
        assert!(matches!(s.take("pc1", Poll::Answered(None)), Some(Told::Both(_))));
        let mut again = Standby::new(&d);
        again.set(Some(&cfg(2)), &[remote()]);
        assert!(!again.waiting(), "a restart stays taken over");
        assert!(again.give_back().is_ok());
        assert!(again.waiting());
        assert!(again.give_back().is_err());
        again.set(None, &[remote()]);
        assert!(!again.waiting() && again.line().is_none());
        let _ = std::fs::remove_dir_all(&d);
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
        assert!(matches!(s.take("pc1", Poll::Missed("refused".into())), Some(Told::TookOver(_))));
        s.copy_tx.send(Ok(vec![one])).unwrap();
        assert!(s.copies(BTreeMap::new).is_empty());
        // The copy's trouble goes with the takeover, and a giveback copies at once.
        assert!(s.notes().is_empty());
        assert!(s.copied_at.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
