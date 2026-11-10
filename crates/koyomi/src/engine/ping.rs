//! `[ping]` of koyomi.toml: while the engine runs it calls an address every
//! `every`, so a watcher elsewhere (healthchecks.io, Uptime Kuma's push) tells
//! of it when the calls stop. That is when the PC went off or the engine died,
//! and koyomi cannot tell of it itself. The call goes through ito-notify's
//! webhook (the system's `curl`, a JSON POST, tried again once), on a thread;
//! a failed call only goes to stderr, since the watcher is the one to notice.
//!
//! The address is a `secret:<name>` in the OS credential store, read back on
//! each call (the store can be slow, and a changed value is taken at once).
//! One written in plain text is not called until `[notify] hide_webhooks =
//! false` says that is meant, as with the webhooks.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ito_notify::webhook::{self, Level};
use koyomi_core::config;
use koyomi_core::msg;

/// Said of a plain-text `[ping]` address while `hide_webhooks` is on.
pub fn plain() -> String {
    msg::line(msg::PING_URL_PLAIN, &[])
}

#[derive(Default)]
pub struct Ping {
    /// When the address was last called; `None`: not yet since `[ping]` was read.
    last: Option<Instant>,
    /// What the last read of the credential store said was wrong.
    note: Arc<Mutex<Option<String>>>,
}

impl Ping {
    /// Calls the address when it is due. `plain_ok`: `hide_webhooks` is off.
    pub fn tick(&mut self, cfg: Option<&config::Ping>, plain_ok: bool) {
        let Some(cfg) = cfg else {
            self.last = None;
            return;
        };
        if !due(self.last.map(|l| l.elapsed()), cfg.every_secs()) {
            return;
        }
        self.last = Some(Instant::now());
        let Some(url) = address(&cfg.url, plain_ok) else { return };
        let note = Arc::clone(&self.note);
        let _ = std::thread::Builder::new().name("ping".into()).spawn(move || {
            let url = match url {
                Address::Plain(u) => u,
                Address::Secret(name) => match ito_secret::get("koyomi", &name) {
                    Ok(Some(u)) => u,
                    Ok(None) => return set(&note, Some(msg::line(msg::PING_SECRET_MISSING, &[&name]))),
                    Err(e) => return set(&note, Some(msg::line(msg::PING_SECRET_UNREADABLE, &[&name, &e.to_string()]))),
                },
            };
            set(&note, None);
            let host = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
            webhook::send(url, "json".into(), Level::Done, "koyomi".into(), "alive".into(), host);
        });
    }

    /// What a person should see about `[ping]`.
    pub fn notes(&self, cfg: Option<&config::Ping>, plain_ok: bool) -> Vec<String> {
        let is_plain = cfg.is_some_and(|c| address(&c.url, plain_ok).is_none());
        let store = cfg.and_then(|_| self.note.lock().ok().and_then(|n| n.clone()));
        is_plain.then(plain).into_iter().chain(store).collect()
    }
}

fn set(note: &Mutex<Option<String>>, to: Option<String>) {
    if let Ok(mut n) = note.lock() {
        *n = to;
    }
}

enum Address {
    Plain(String),
    Secret(String),
}

/// Where to call, or `None` when a plain-text address may not be used.
fn address(url: &str, plain_ok: bool) -> Option<Address> {
    match webhook::reference(url) {
        Some(name) => Some(Address::Secret(name.to_owned())),
        None => plain_ok.then(|| Address::Plain(url.trim().to_owned())),
    }
}

/// Whether a call is due, `since` the last one (`None`: none yet).
fn due(since: Option<Duration>, every: u64) -> bool {
    since.is_none_or(|s| s >= Duration::from_secs(every))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_call_goes_at_once_and_the_next_after_the_interval() {
        assert!(due(None, 300));
        assert!(!due(Some(Duration::from_secs(299)), 300));
        assert!(due(Some(Duration::from_secs(300)), 300));
    }

    #[test]
    fn a_plain_address_waits_for_hide_webhooks_off_and_a_secret_is_read_back() {
        assert!(address("https://hc-ping.com/x", false).is_none());
        assert!(matches!(address("https://hc-ping.com/x", true), Some(Address::Plain(u)) if u == "https://hc-ping.com/x"));
        assert!(matches!(address("secret:ping", false), Some(Address::Secret(n)) if n == "ping"));
        let cfg = config::Ping { url: "https://hc-ping.com/x".into(), every: "5m".into() };
        let p = Ping::default();
        assert_eq!(p.notes(Some(&cfg), false), vec![plain()]);
        assert!(p.notes(Some(&cfg), true).is_empty());
        assert!(p.notes(None, false).is_empty());
    }
}
