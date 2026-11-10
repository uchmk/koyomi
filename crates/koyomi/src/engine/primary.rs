//! `[primary]` of koyomi.toml (TODO 96, DECISIONS「複数の PC・Web UI・利用者とロール・HA」):
//! this engine runs the schedules, and the `[[remote]]` named `standby` is an engine with
//! `[standby]` watching this one. When this one starts, and every `every` after, it asks that
//! engine's `GET /health` (`"ha"`) whether it has taken over:
//!
//! - It stands by: this one runs the schedules and the events ([`Hold::Running`]).
//! - It has taken over and is ready to hand back: this one takes them back with
//!   `POST /standby/release`, and runs once the answer comes. Ready means `koyomi standby
//!   giveback` there, or `give_back = "auto"` and `misses` good polls in a row.
//! - It has taken over and is not ready: this one waits ([`Hold::Held`]); its times are not
//!   written down here, as on a standby, since the standby runs them.
//!
//! Until the first answer the times wait ([`Hold::Asking`]) and are judged once it knows; after
//! `misses` asks in a row that get no answer, it runs. An answer that cannot say (no
//! `[standby]` there, a wrong token) runs here, as without `[primary]`, and is said.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use ito_http::client::{self, Client};
use koyomi_core::config;
use koyomi_core::msg::{self, line};
use koyomi_core::standby::{Ask, Heard, Hold};

/// What one ask heard, and what a person should see.
type Asked = (Heard, Option<String>);

pub struct Primary {
    cfg: Option<config::Primary>,
    remote: Option<config::Remote>,
    ask: Ask,
    last: Option<Instant>,
    tx: mpsc::Sender<Asked>,
    rx: mpsc::Receiver<Asked>,
    in_flight: bool,
    /// What the last ask said was wrong.
    note: Option<String>,
    /// The standby had taken over since this one last ran: its coming back is told.
    was_held: bool,
}

impl Primary {
    pub fn new() -> Primary {
        let (tx, rx) = mpsc::channel();
        Primary {
            cfg: None,
            remote: None,
            ask: Ask::new(koyomi_core::standby::MISSES),
            last: None,
            tx,
            rx,
            in_flight: false,
            note: None,
            was_held: false,
        }
    }

    /// Takes in koyomi.toml's `[primary]` and the `[[remote]]` it names. A change asks again
    /// from the start: a new standby may have taken over.
    pub fn set(&mut self, cfg: Option<&config::Primary>, remotes: &[config::Remote]) {
        let remote = cfg.and_then(|c| remotes.iter().find(|r| r.name.trim() == c.standby.trim()).cloned());
        if remote != self.remote || cfg.map(|c| c.misses) != self.cfg.as_ref().map(|c| c.misses) {
            self.ask = Ask::new(cfg.map_or(koyomi_core::standby::MISSES, |c| c.misses));
            self.last = None;
            self.note = None;
        }
        self.note = match (cfg, &remote) {
            (Some(c), None) => Some(line(msg::PRIMARY_STANDBY_UNKNOWN, &[c.standby.trim()])),
            _ => self.note.take().filter(|_| remote.is_some()),
        };
        self.cfg = cfg.cloned();
        self.remote = remote;
    }

    /// Where it is; `None` without `[primary]` (or without its `[[remote]]`): it runs.
    pub fn hold(&self) -> Option<Hold> {
        self.remote.as_ref().map(|_| self.ask.hold())
    }

    /// Still asking: the times and the events wait.
    pub fn asking(&self) -> bool {
        matches!(self.hold(), Some(Hold::Asking { .. }))
    }

    /// Asks when due and takes in what came back; what to tell of when the hold changed.
    pub fn tick(&mut self) -> Option<(&'static str, String)> {
        let (Some(every), Some(remote)) = (self.cfg.as_ref().map(|c| c.every_secs()), self.remote.clone()) else {
            return None;
        };
        let mut told = None;
        while let Ok((heard, note)) = self.rx.try_recv() {
            self.in_flight = false;
            self.note = note;
            let before = self.ask.hold();
            if let Some(after) = self.ask.heard(heard) {
                told = says(remote.name.trim(), before, after, heard, self.was_held).or(told);
                match after {
                    Hold::Held => self.was_held = true,
                    Hold::Running => self.was_held = false,
                    Hold::Asking { .. } => {}
                }
            }
        }
        if self.in_flight || self.last.is_some_and(|l| l.elapsed() < Duration::from_secs(every)) {
            return told;
        }
        self.last = Some(Instant::now());
        self.in_flight = true;
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new().name("primary".into()).spawn(move || {
            let _ = tx.send(ask(&remote));
        });
        if spawned.is_err() {
            self.in_flight = false;
        }
        told
    }

    /// `"ha"` of `GET /health` here.
    pub fn json(&self) -> Option<serde_json::Value> {
        let cfg = self.cfg.as_ref()?;
        let hold = match self.hold()? {
            Hold::Asking { .. } => "asking",
            Hold::Held => "held",
            Hold::Running => "running",
        };
        Some(serde_json::json!({ "role": "primary", "standby": cfg.standby.trim(), "hold": hold }))
    }

    /// The status line, when `[primary]` is written.
    pub fn line(&self) -> Option<String> {
        let cfg = self.cfg.as_ref()?;
        let name = cfg.standby.trim();
        Some(match self.hold() {
            None | Some(Hold::Running) => format!("[primary] {name} が待機している。この PC で流す"),
            Some(Hold::Asking { missed: 0 }) => format!("[primary] {name} に聞いている。答えるまでスケジュールとイベントは待たせる"),
            Some(Hold::Asking { missed }) => format!(
                "[primary] {name} に聞いている（答えなかった {missed} 回、{} 回でこの PC で流す）。スケジュールとイベントは待たせる",
                cfg.misses
            ),
            Some(Hold::Held) => format!("[primary] {name} が引き継いでいる。返されるまでこちらではスケジュールとイベントで流さない"),
        })
    }

    /// The line to show above the lists: only while it does not run.
    pub fn caution(&self) -> Option<String> {
        self.line().filter(|_| matches!(self.hold(), Some(Hold::Asking { .. } | Hold::Held)))
    }

    pub fn notes(&self) -> Vec<String> {
        let name = self.cfg.as_ref().map(|c| c.standby.trim()).unwrap_or_default();
        // A coded note names the connection and keeps its ID at the head.
        self.note.iter().map(|n| if msg::is_coded(n) { n.clone() } else { format!("[primary] {name}: {n}") }).collect()
    }

    /// Why a time or an event does not start here while the standby runs them.
    pub fn held(&self) -> Option<String> {
        let name = self.cfg.as_ref()?.standby.trim();
        (self.hold() == Some(Hold::Held)).then(|| format!("待機中（{name} が引き継いでいる）"))
    }
}

/// What to tell of when the hold went from `before` to `after` on `heard`.
fn says(name: &str, before: Hold, after: Hold, heard: Heard, was_held: bool) -> Option<(&'static str, String)> {
    let title = "koyomi: 待機の PC";
    Some(match (before, after, heard) {
        (Hold::Running, Hold::Held, _) => {
            (title, format!("{name} が引き継いで流していた。こちらはスケジュールとイベントで流すのをやめ、返されるのを待つ"))
        }
        (_, Hold::Held, _) => (title, format!("{name} が引き継いでいる。返されるまでこちらではスケジュールとイベントで流さない")),
        (_, Hold::Running, Heard::Idle) if was_held => (title, format!("{name} から戻った。この PC で流す")),
        (_, Hold::Running, Heard::Nobody) => (title, format!("{name} が答えないので、この PC で流す")),
        (_, Hold::Running, Heard::Unclear) => (title, format!("{name} に聞いても分からないので、この PC で流す")),
        _ => return None,
    })
}

/// What the standby said in `"ha"` of its `GET /health`.
#[derive(Debug, Clone, PartialEq)]
enum Said {
    /// It stands by.
    Idle,
    /// It has taken over and lets go when asked.
    Ready,
    /// It has taken over and does not let go yet: why.
    Active(Option<String>),
    /// It cannot say: why.
    Unclear(String),
}

fn said(name: &str, health: &serde_json::Value) -> Said {
    let ha = &health["ha"];
    if ha["role"] != "standby" {
        return Said::Unclear(line(msg::NO_STANDBY_THERE, &[name]));
    }
    match (ha["active"].as_bool(), ha["ready"].as_bool()) {
        (Some(false), _) => Said::Idle,
        (Some(true), Some(true)) => Said::Ready,
        (Some(true), _) => Said::Active(ha["why"].as_str().map(str::to_owned)),
        (None, _) => Said::Unclear(line(msg::HA_UNREADABLE, &[name])),
    }
}

fn ask(r: &config::Remote) -> Asked {
    let name = r.secret();
    let token = match ito_secret::get("koyomi", name) {
        Ok(Some(t)) => t,
        Ok(None) => return (Heard::Unclear, Some(line(msg::TOKEN_SECRET_MISSING, &[&r.name, name]))),
        Err(e) => return (Heard::Unclear, Some(line(msg::TOKEN_SECRET_UNREADABLE, &[&r.name, name, &e.to_string()]))),
    };
    let c = match Client::new(r.url.trim(), r.fingerprint.trim(), token) {
        Ok(c) => c,
        Err(e) => return failed(&r.name, e, Heard::Unclear),
    };
    let health = match c.get("/health") {
        Ok(h) if h.status == 200 => h,
        Ok(h) if h.status == 401 => return (Heard::Unclear, Some(line(msg::TOKEN_REFUSED, &[&r.name, name]))),
        Ok(h) => return (Heard::Unclear, Some(line(msg::UNEXPECTED_STATUS, &[&r.name, &format!("/health {}", h.status)]))),
        Err(e) => return failed(&r.name, e, Heard::Unclear),
    };
    match said(&r.name, &health.json) {
        Said::Idle => (Heard::Idle, None),
        Said::Active(why) => (Heard::Active, why),
        Said::Unclear(why) => (Heard::Unclear, Some(why)),
        // Once it has said it runs them, only no answer at all lets this one run: anything else
        // waits, so the schedules never run on both.
        Said::Ready => match c.post_json("/standby/release", &serde_json::json!({})) {
            Ok(reply) if reply.status == 200 => (Heard::Idle, None),
            Ok(reply) if reply.status == 403 => (Heard::Active, Some(line(msg::HANDOVER_REFUSED, &[&r.name, "run"]))),
            Ok(reply) => (
                Heard::Active,
                // The standby's own line (an engine from before the IDs sends plain text) goes through as it is.
                Some(reply.json["error"].as_str().map_or_else(
                    || line(msg::UNEXPECTED_STATUS, &[&r.name, &format!("/standby/release {}", reply.status)]),
                    str::to_owned,
                )),
            ),
            Err(e) => failed(&r.name, e, Heard::Active),
        },
    }
}

/// No connection, or TLS that does not finish: nobody there. Anything else is `otherwise`.
fn failed(name: &str, e: client::Error, otherwise: Heard) -> Asked {
    let heard = match e {
        client::Error::Connect(_) | client::Error::Tls(_) => Heard::Nobody,
        _ => otherwise,
    };
    (heard, Some(crate::remote::reason(name, &e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standby_says_whether_it_lets_go() {
        let ha = |v: serde_json::Value| said("pc2", &serde_json::json!({ "ok": true, "ha": v }));
        assert!(matches!(said("pc2", &serde_json::json!({ "ok": true })), Said::Unclear(_)));
        assert!(matches!(ha(serde_json::json!({ "role": "primary", "hold": "running" })), Said::Unclear(_)));
        assert_eq!(ha(serde_json::json!({ "role": "standby", "active": false, "ready": false })), Said::Idle);
        assert_eq!(ha(serde_json::json!({ "role": "standby", "active": true, "ready": true })), Said::Ready);
        assert_eq!(
            ha(serde_json::json!({ "role": "standby", "active": true, "ready": false, "why": "manual" })),
            Said::Active(Some("manual".into()))
        );
    }

    #[test]
    fn only_no_answer_is_nobody() {
        assert_eq!(failed("pc2", client::Error::Connect("refused".into()), Heard::Active).0, Heard::Nobody);
        assert_eq!(failed("pc2", client::Error::Tls("eof".into()), Heard::Unclear).0, Heard::Nobody);
        assert_eq!(failed("pc2", client::Error::Fingerprint("x".into()), Heard::Active).0, Heard::Active);
        assert_eq!(failed("pc2", client::Error::Fingerprint("x".into()), Heard::Unclear).0, Heard::Unclear);
    }

    #[test]
    fn a_change_of_hold_is_told_once_it_matters() {
        let asking = Hold::Asking { missed: 0 };
        assert!(says("pc2", asking, Hold::Running, Heard::Idle, false).is_none());
        assert!(says("pc2", asking, Hold::Held, Heard::Active, false).is_some_and(|(_, l)| l.contains("引き継いでいる")));
        assert!(says("pc2", Hold::Running, Hold::Held, Heard::Active, false).is_some_and(|(_, l)| l.contains("やめ")));
        assert!(says("pc2", Hold::Held, Hold::Running, Heard::Idle, true).is_some_and(|(_, l)| l.contains("戻った")));
        assert!(says("pc2", asking, Hold::Running, Heard::Idle, true).is_some_and(|(_, l)| l.contains("戻った")));
        assert!(says("pc2", asking, Hold::Running, Heard::Nobody, false).is_some_and(|(_, l)| l.contains("答えない")));
        assert!(says("pc2", Hold::Held, Hold::Asking { missed: 1 }, Heard::Nobody, true).is_none());
    }

    #[test]
    fn without_its_remote_it_runs_and_says_why() {
        let mut p = Primary::new();
        assert_eq!(p.hold(), None);
        let cfg = config::Primary { standby: "pc2".into(), every: "20s".into(), misses: 3 };
        p.set(Some(&cfg), &[]);
        assert_eq!(p.hold(), None);
        assert!(msg::PRIMARY_STANDBY_UNKNOWN.is_in(&p.notes()[0]) && p.notes()[0].contains("pc2"), "{:?}", p.notes());
        let r =
            config::Remote { name: "pc2".into(), url: "https://pc2:8790".into(), fingerprint: "AB".repeat(32), token: "secret:pc2".into() };
        p.set(Some(&cfg), std::slice::from_ref(&r));
        assert_eq!(p.hold(), Some(Hold::Asking { missed: 0 }));
        assert!(p.notes().is_empty());
        assert_eq!(p.json().unwrap()["hold"], "asking");
        assert!(p.caution().is_some());
        // The same again keeps where it is.
        p.ask.heard(Heard::Idle);
        p.set(Some(&cfg), &[r]);
        assert_eq!(p.hold(), Some(Hold::Running));
        assert!(p.caution().is_none());
    }
}
