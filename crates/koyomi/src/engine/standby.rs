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

use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use chrono::Local;
use ito_common::kept;
use ito_http::client::{self, Client};
use koyomi_core::config;
use koyomi_core::standby::{State, Step, Watch};
use serde::{Deserialize, Serialize};

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
}

impl Standby {
    /// Reads what the previous engine left: whether it had taken over.
    pub fn new(state_dir: &Path) -> Standby {
        let path = state_dir.join("standby.toml");
        let prev: Kept = kept::read(&path).unwrap_or_default();
        let (tx, rx) = mpsc::channel();
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

    /// What a person should see: the poll's trouble.
    pub fn notes(&self) -> Vec<String> {
        let name = self.cfg.as_ref().map(|c| c.watch.trim()).unwrap_or_default();
        self.note.iter().map(|n| format!("[standby] {name}: {n}")).collect()
    }
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
}
