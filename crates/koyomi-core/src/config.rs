//! `koyomi.toml` in the settings folder: how many jobs run at once, how long
//! history is kept (and whether it goes into `archive/` zips first), how big one job's log may grow, `[notify]` (read by
//! the app with ito-prefs's shape; kept whole here), `[ping]`, the
//! address the engine calls while it is alive, `[http]`, where it
//! listens for other programs (on this PC, or with `lan` on the network over
//! TLS), `[[remote]]`, the other PCs' engines this one calls, `[standby]`, which of them
//! this one stands by for (HA), `[primary]`, which of them stands by for this one, and
//! `[locks]`, how many nets may hold a `lock` at once, and `work_dir`, where a job without
//! `dir` runs, `[[blackout]]`, the planned stops in which nothing starts by itself,
//! `[vars]`, the values every net reads as `{var:name}`, and `[groups]`, the PCs an
//! `ssh` or a `winrm` with `on = "group:name"` runs on.
//!
//! Missing file: the defaults. A file that does not read is an error the
//! engine shows and does not run with (a typo must not quietly mean "4").

use crate::msg;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `work_dir = "definition"`: a job without `dir` runs in the folder its net's file is in.
pub const DEFINITION: &str = "definition";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Jobs running at the same time, over every net.
    pub concurrency: usize,
    /// Days of history and logs kept.
    pub keep_days: u32,
    /// Put what `keep_days` lets go into `archive/YYYY-MM.zip` before it is deleted.
    pub archive: bool,
    /// Months whose last day is this many days old leave `archive/`; 0 keeps them forever.
    pub archive_days: u32,
    /// One job's log stops growing here; the rest of its output is not written.
    pub log_max_mb: u64,
    /// `[notify]` as written: which events are told how. The app reads it.
    pub notify: toml::Table,
    /// `[ping]`: an outside watcher (healthchecks.io and the like) to call while the engine runs.
    pub ping: Option<Ping>,
    /// `[http]`: start nets and read runs over HTTP from this PC. Off when not written.
    pub http: Option<Http>,
    /// `[[remote]]`: other PCs' engines, called over HTTPS with their certificate pinned.
    pub remote: Vec<Remote>,
    /// `[standby]`: stand by for one `[[remote]]`; run the schedules here only once it stops answering.
    pub standby: Option<Standby>,
    /// `[primary]`: the `[[remote]]` that stands by for this PC; ask it before running the schedules.
    pub primary: Option<Primary>,
    /// `[locks]`: `"API枠" = 3`, how many nets may hold that `lock` exclusively at once.
    /// A name not listed has 1. Kept as written so [`Config::parse`] can say what is wrong in Japanese.
    pub locks: toml::Table,
    /// Where a job without `dir` runs, and what a relative `dir` is read from. Not written: the
    /// engine's own working folder (where it was started; the service's is `C:\Windows\System32`).
    /// [`DEFINITION`]: the folder its net's file is in (`jobnets/<group>`). Else one folder, an
    /// absolute path, for every net.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_dir: Option<String>,
    /// `[[blackout]]`: the planned stops; no schedule or event starts a net in them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blackout: Vec<crate::blackout::Blackout>,
    /// `[vars]`: `share = '\\nas\backup'`, read by every net as `{var:share}`. Kept as written so
    /// [`Config::parse`] can say what is wrong in Japanese; [`Config::vars`] gives the values.
    #[serde(skip_serializing_if = "toml::Table::is_empty")]
    pub vars: toml::Table,
    /// `[groups]`: `web = ["ssh://ops@web1", { on = "ssh://ops@web2", host_key = "SHA256:…" }]`,
    /// the PCs an `ssh` or a `winrm` with `on = "group:web"` runs on. Kept as written so
    /// [`Config::parse`] can say what is wrong in Japanese; [`Config::members`] gives them.
    #[serde(skip_serializing_if = "toml::Table::is_empty")]
    pub groups: toml::Table,
}

/// One PC of a `[groups]` group: its address, and its own host key (ssh) or
/// certificate (winrm) when it has one; else the job's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub on: String,
    pub host_key: Option<String>,
    pub cert: Option<String>,
}

/// The group `name` of `[groups]` as written in `v`: what its PCs are reached by
/// (`"ssh"` or `"winrm"`, the same for each) and the PCs.
fn group_of(name: &str, v: &toml::Value) -> Result<(&'static str, Vec<Member>), String> {
    let not_array = || msg::line(msg::CFG_GROUP_NOT_ARRAY, &[name]);
    let list = match v.as_array() {
        Some(l) if !l.is_empty() => l,
        _ => return Err(not_array()),
    };
    if list.len() > crate::chore::GROUP_MAX {
        return Err(msg::line(msg::CFG_GROUP_TOO_MANY, &[name, &crate::chore::GROUP_MAX.to_string()]));
    }
    let mut scheme = None;
    let mut members: Vec<Member> = Vec::new();
    for item in list {
        let (on, host_key, cert) = match item {
            toml::Value::String(on) => (on.as_str(), None, None),
            toml::Value::Table(t) => {
                if let Some(k) = t.keys().find(|k| !["on", "host_key", "cert"].contains(&k.as_str())) {
                    return Err(msg::line(msg::CFG_GROUP_UNKNOWN_KEY, &[name, k]));
                }
                let text = |key: &str| match t.get(key) {
                    None => Ok(None),
                    Some(toml::Value::String(x)) => Ok(Some(x.trim())),
                    Some(_) => Err(msg::line(msg::CFG_GROUP_NOT_STRING, &[name, key])),
                };
                let Some(on) = text("on")? else {
                    return Err(msg::line(msg::CFG_GROUP_NO_ON, &[name]));
                };
                (on, text("host_key")?.filter(|x| !x.is_empty()), text("cert")?.filter(|x| !x.is_empty()))
            }
            _ => return Err(not_array()),
        };
        let on = on.trim();
        if [Some(on), host_key, cert].into_iter().flatten().any(|x| x.contains('{')) {
            return Err(msg::line(msg::CFG_GROUP_BRACES, &[name, on]));
        }
        let this = match (crate::chore::ssh(on), crate::chore::winrm(on)) {
            (Some(r), _) => r.map(|_| "ssh"),
            (_, Some(r)) => r.map(|_| "winrm"),
            _ => return Err(msg::line(msg::CFG_GROUP_ADDRESS, &[name, on])),
        }
        .map_err(|e| msg::line(msg::CFG_GROUP_MEMBER, &[name, msg::uncoded(&e)]))?;
        match scheme {
            Some(s) if s != this => return Err(msg::line(msg::CFG_GROUP_MIXED, &[name])),
            _ => scheme = Some(this),
        }
        if let Some(h) = host_key {
            if this != "ssh" {
                return Err(msg::line(msg::CFG_GROUP_KEY_SCHEME, &[name, "host_key", "ssh", on]));
            }
            crate::chore::host_key_ok(h).map_err(|e| msg::line(msg::CFG_GROUP_MEMBER, &[name, msg::uncoded(&e)]))?;
        }
        if let Some(c) = cert {
            if this != "winrm" {
                return Err(msg::line(msg::CFG_GROUP_KEY_SCHEME, &[name, "cert", "winrm", on]));
            }
            crate::chore::cert_ok(c).map_err(|e| msg::line(msg::CFG_GROUP_MEMBER, &[name, msg::uncoded(&e)]))?;
        }
        if members.iter().any(|m| m.on.eq_ignore_ascii_case(on)) {
            return Err(msg::line(msg::CFG_GROUP_DUPLICATE, &[name, on]));
        }
        members.push(Member { on: on.to_owned(), host_key: host_key.map(str::to_owned), cert: cert.map(str::to_owned) });
    }
    Ok((scheme.unwrap_or("ssh"), members))
}

/// `[standby]`: poll the `[[remote]]` named `watch` every `every`; after `misses` polls in a row
/// it does not answer, take over its schedules (`koyomi standby giveback` hands them back, or with
/// `give_back = "auto"` the standby itself once the primary has answered `misses` polls in a row).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Standby {
    pub watch: String,
    /// As a length (`20s`). From 5 seconds to 10 minutes.
    #[serde(default = "every_default")]
    pub every: String,
    /// From 1 to 20.
    #[serde(default = "misses_default")]
    pub misses: u32,
    /// Who hands the schedules back once the primary answers again.
    #[serde(default)]
    pub give_back: GiveBack,
}

/// `[standby] give_back`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GiveBack {
    /// A person: `koyomi standby giveback`.
    #[default]
    Manual,
    /// The standby, once the primary has answered `misses` polls in a row.
    Auto,
}

/// `[primary]`: the `[[remote]]` named `standby` stands by for this PC (its `[standby]` watches
/// this one). Before the schedules run here, this one asks it whether it has taken over, and while
/// it has, waits for it to hand them back instead of running them twice. It asks every `every`;
/// after `misses` asks in a row it does not answer, the schedules run here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Primary {
    pub standby: String,
    #[serde(default = "every_default")]
    pub every: String,
    #[serde(default = "misses_default")]
    pub misses: u32,
}

fn every_default() -> String {
    format!("{}s", crate::standby::INTERVAL.as_secs())
}

fn misses_default() -> u32 {
    crate::standby::MISSES
}

fn every_secs(every: &str) -> u64 {
    crate::duration::parse(every).unwrap_or(crate::standby::INTERVAL.as_secs())
}

/// `[section]`'s `key` names a `[[remote]]`, `every` is 5 s to 10 min and `misses` 1 to 20.
fn check_poll(section: &str, key: &str, name: &str, every: &str, misses: u32, remotes: &[Remote]) -> Result<(), String> {
    let name = name.trim();
    if !remotes.iter().any(|r| r.name.trim() == name) {
        return Err(msg::line(msg::CFG_POLL_TARGET, &[section, key, name]));
    }
    let secs = crate::duration::parse(every).map_err(|e| msg::line(msg::CFG_EVERY_UNREADABLE, &[section, msg::uncoded(&e)]))?;
    if !(5..=600).contains(&secs) {
        return Err(msg::line(msg::CFG_EVERY_RANGE, &[section, "5s", "10m"]));
    }
    if !(1..=20).contains(&misses) {
        return Err(msg::line(msg::CFG_MISSES_RANGE, &[section]));
    }
    Ok(())
}

impl Standby {
    /// The seconds between polls; [`Config::parse`] has checked it reads.
    pub fn every_secs(&self) -> u64 {
        every_secs(&self.every)
    }

    fn check(&self, remotes: &[Remote]) -> Result<(), String> {
        check_poll("standby", "watch", &self.watch, &self.every, self.misses, remotes)
    }
}

impl Primary {
    /// The seconds between asks; [`Config::parse`] has checked it reads.
    pub fn every_secs(&self) -> u64 {
        every_secs(&self.every)
    }

    fn check(&self, remotes: &[Remote]) -> Result<(), String> {
        check_poll("primary", "standby", &self.standby, &self.every, self.misses, remotes)
    }
}

/// `[ping]`: the engine calls `url` every `every`, so a watcher elsewhere
/// notices when the calls stop (the PC went off, the engine died) and tells
/// of it; koyomi itself cannot, then.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ping {
    /// `secret:<name>` (kept in the OS credential store), or an http(s) address.
    pub url: String,
    /// How often, as a length (`5m`). From one minute to a day.
    #[serde(default = "Ping::every_default")]
    pub every: String,
}

impl Ping {
    fn every_default() -> String {
        "5m".to_owned()
    }

    /// The seconds between calls; [`Config::parse`] has checked it reads.
    pub fn every_secs(&self) -> u64 {
        crate::duration::parse(&self.every).unwrap_or(300)
    }

    fn check(&self) -> Result<(), String> {
        let url = self.url.trim();
        let secret = url.strip_prefix("secret:").is_some_and(|n| !n.trim().is_empty());
        if !secret && !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(msg::line(msg::CFG_PING_URL, &[]));
        }
        let secs = crate::duration::parse(&self.every).map_err(|e| msg::line(msg::CFG_EVERY_UNREADABLE, &["ping", msg::uncoded(&e)]))?;
        if !(60..=86_400).contains(&secs) {
            return Err(msg::line(msg::CFG_EVERY_RANGE, &["ping", "1m", "24h"]));
        }
        Ok(())
    }
}

/// `[http]`: the engine listens on `port` of 127.0.0.1 (and ::1 with `ipv6`), or with `lan`
/// on every address over TLS only, and answers a caller with the bearer token kept in the OS
/// credential store as `token`, or with one of the `[[http.user]]`'s tokens (TODO 83).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Http {
    pub port: u16,
    /// `secret:<name>`: never the token itself. A nameless caller with the `run` role; may be
    /// left out when there are `[[http.user]]`.
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub ipv6: bool,
    /// Listen on the network, over TLS with a self-signed certificate (kept in `app_dir`).
    #[serde(default)]
    pub lan: bool,
    /// `[[http.user]]`: one token per person, each with a role.
    #[serde(default)]
    pub user: Vec<HttpUser>,
    /// A second port, on this PC's Tailscale addresses only (plain HTTP; WireGuard encrypts):
    /// there the `[[http.user]]` with `tailscale` are let in by who Tailscale says is calling,
    /// with no token (TODO 105). Opt-in; tokens work there too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailnet: Option<u16>,
}

/// `[[http.user]]`: a person who calls over HTTP, with the role their token gets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpUser {
    /// Written down with what they did (`koyomi ops`).
    pub name: String,
    /// `view`, `run`, `edit` or `admin`.
    pub role: String,
    /// `secret:<name>`; may be left out when `tailscale` is set.
    #[serde(default)]
    pub token: String,
    /// Their Tailscale login name (`sato@example.com`), or a tag (`tag:koyomi-ha`) for a node
    /// with no person: on `[http] tailnet` they need no token (TODO 105).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale: Option<String>,
    /// The nets they reach, as `<group>/<name>` patterns (`経理/*`, TODO 94); none: every net.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nets: Option<Vec<String>>,
    /// A role per net, keyed by the same patterns (`{ "経理/*" = "run" }`, TODO 101): a net one
    /// names gets the strongest of those that name it; the rest keep `role`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<std::collections::BTreeMap<String, String>>,
}

impl HttpUser {
    /// The role; [`Config::parse`] has checked it reads.
    pub fn role(&self) -> crate::role::Role {
        crate::role::Role::parse(self.role.trim()).unwrap_or(crate::role::Role::View)
    }

    /// The nets they reach; `None`: every net. [`Config::parse`] has checked the patterns read;
    /// ones that do not reach nothing.
    pub fn nets(&self) -> Option<crate::role::Nets> {
        self.nets.as_deref().map(|p| crate::role::Nets::parse(p).unwrap_or_default())
    }

    /// What they may do: `role`, `nets` and `roles` together. [`Config::parse`] has checked
    /// `roles` reads; one that does not adds nothing.
    pub fn access(&self) -> crate::role::Access {
        crate::role::Access {
            role: self.role(),
            nets: self.nets(),
            roles: self.roles.as_ref().and_then(|r| crate::role::per_net(r).ok()).unwrap_or_default(),
        }
    }

    /// The name the token is kept under; empty when they come only over the tailnet.
    pub fn secret(&self) -> &str {
        self.token.trim().strip_prefix("secret:").map(str::trim).unwrap_or_default()
    }

    /// Their Tailscale login or tag; [`Config::parse`] has checked it reads.
    pub fn tailscale(&self) -> Option<&str> {
        self.tailscale.as_deref().map(str::trim).filter(|t| !t.is_empty())
    }
}

/// `[[remote]]`: another PC's engine. `fingerprint` is its certificate's SHA-256 as
/// `koyomi http fingerprint` shows it there; a certificate that differs is refused.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Remote {
    /// What this PC calls it (`koyomi remote status <name>`).
    pub name: String,
    /// `https://host:port`.
    pub url: String,
    pub fingerprint: String,
    /// `secret:<name>`: the other PC's `[http]` token, kept here in the OS credential store.
    pub token: String,
}

impl Remote {
    /// The name the token is kept under; [`Config::parse`] has checked there is one.
    pub fn secret(&self) -> &str {
        self.token.trim().strip_prefix("secret:").map(str::trim).unwrap_or_default()
    }

    fn check(&self) -> Result<(), String> {
        let name = self.name.trim();
        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err(msg::line(msg::CFG_REMOTE_NAME, &[]));
        }
        let host = self.url.strip_prefix("https://").unwrap_or_default().trim_end_matches('/');
        if host.is_empty() || host.contains(['/', '?', '#', '@', ' ']) {
            return Err(msg::line(msg::CFG_REMOTE_URL, &[name]));
        }
        let hex: Vec<char> = self.fingerprint.chars().filter(|c| !matches!(c, ':' | ' ' | '-')).collect();
        if hex.len() != 64 || !hex.iter().all(char::is_ascii_hexdigit) {
            return Err(msg::line(msg::CFG_REMOTE_FINGERPRINT, &[name]));
        }
        if self.secret().is_empty() {
            return Err(msg::line(msg::CFG_TOKEN_NOT_SECRET, &[&format!("[[remote]] {name}")]));
        }
        Ok(())
    }
}

impl Http {
    /// The name the token is kept under; [`Config::parse`] has checked there is one.
    pub fn secret(&self) -> &str {
        self.token.trim().strip_prefix("secret:").map(str::trim).unwrap_or_default()
    }

    fn check(&self) -> Result<(), String> {
        if self.port == 0 {
            return Err(msg::line(msg::CFG_HTTP_PORT, &[]));
        }
        if self.secret().is_empty() && (!self.token.trim().is_empty() || self.user.is_empty()) {
            return Err(msg::line(msg::CFG_TOKEN_NOT_SECRET, &["[http]"]));
        }
        if let Some(t) = self.tailnet {
            if t == 0 || t == self.port {
                return Err(msg::line(msg::CFG_TAILNET_PORT, &[]));
            }
            if !self.user.iter().any(|u| u.tailscale.is_some()) {
                return Err(msg::line(msg::CFG_TAILNET_NO_USER, &[]));
            }
        }
        let (mut names, mut tailscales) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
        for u in &self.user {
            let name = u.name.trim();
            if name.is_empty() || name.contains(char::is_whitespace) || name == "default" {
                return Err(msg::line(msg::CFG_USER_NAME, &[]));
            }
            if !names.insert(name) {
                return Err(msg::line(msg::CFG_DUPLICATE, &["[[http.user]]", "name", name]));
            }
            let role = crate::role::Role::parse(u.role.trim()).map_err(|e| msg::line(msg::CFG_USER_FIELD, &[name, msg::uncoded(&e)]))?;
            if role == crate::role::Role::Admin && (u.nets.is_some() || u.roles.is_some()) {
                return Err(msg::line(msg::CFG_USER_ADMIN_NARROWED, &[name]));
            }
            if u.secret().is_empty() && (!u.token.trim().is_empty() || u.tailscale.is_none()) {
                return Err(msg::line(msg::CFG_TOKEN_NOT_SECRET, &[&format!("[[http.user]] {name}")]));
            }
            if let Some(t) = &u.tailscale {
                let t = t.trim();
                let reads = match t.strip_prefix("tag:") {
                    Some(tag) => !tag.is_empty(),
                    None => t.contains('@') && !t.starts_with('@'),
                };
                if !reads || t.contains(char::is_whitespace) {
                    return Err(msg::line(msg::CFG_TAILSCALE_FORM, &[name, t]));
                }
                if self.tailnet.is_none() {
                    return Err(msg::line(msg::CFG_TAILSCALE_NEEDS_TAILNET, &[name]));
                }
                if !tailscales.insert(t.to_lowercase()) {
                    return Err(msg::line(msg::CFG_DUPLICATE, &["[[http.user]]", "tailscale", t]));
                }
            }
            if let Some(p) = &u.nets {
                crate::role::Nets::parse(p).map_err(|e| msg::line(msg::CFG_USER_FIELD, &[name, msg::uncoded(&e)]))?;
            }
            if let Some(r) = &u.roles {
                crate::role::per_net(r).map_err(|e| msg::line(msg::CFG_USER_FIELD, &[name, msg::uncoded(&e)]))?;
            }
        }
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Config {
        Config {
            concurrency: 4,
            keep_days: 90,
            archive: false,
            archive_days: 0,
            log_max_mb: 10,
            notify: toml::Table::new(),
            ping: None,
            http: None,
            remote: Vec::new(),
            standby: None,
            primary: None,
            locks: toml::Table::new(),
            work_dir: None,
            blackout: Vec::new(),
            vars: toml::Table::new(),
            groups: toml::Table::new(),
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        let c: Config = toml::from_str(text).map_err(|e| {
            let m = e.message();
            let place = e.span().map_or_else(
                || "koyomi.toml".to_owned(),
                |s| {
                    let line = text[..s.start.min(text.len())].matches('\n').count() + 1;
                    msg::note(msg::N_CONFIG_AT_LINE, &[&line.to_string()])
                },
            );
            if let Some(f) = m.strip_prefix("unknown field `").and_then(|r| r.split('`').next()) {
                msg::line(msg::CFG_UNKNOWN_KEY, &[&place, f])
            } else if let Some(f) = m.strip_prefix("missing field `").and_then(|r| r.split('`').next()) {
                msg::line(msg::CFG_MISSING_KEY, &[&place, f])
            } else if let Some(v) = m.strip_prefix("unknown variant `").and_then(|r| r.split('`').next()) {
                let ok: Vec<&str> = m.split('`').skip(3).step_by(2).collect();
                msg::line(msg::CFG_VALUE_UNKNOWN, &[&place, &format!("{v:?}"), &ok.join(&msg::note(msg::N_LIST_SEP, &[]))])
            } else {
                msg::line(msg::CFG_UNREADABLE, &[&place, m])
            }
        })?;
        if c.concurrency == 0 {
            return Err(msg::line(msg::CFG_LIMIT_TOO_LOW, &["concurrency"]));
        }
        if c.keep_days == 0 {
            return Err(msg::line(msg::CFG_LIMIT_TOO_LOW, &["keep_days"]));
        }
        if c.log_max_mb == 0 {
            return Err(msg::line(msg::CFG_LIMIT_TOO_LOW, &["log_max_mb"]));
        }
        if let Some(p) = &c.ping {
            p.check()?;
        }
        if let Some(h) = &c.http {
            h.check()?;
        }
        for (i, r) in c.remote.iter().enumerate() {
            r.check()?;
            if c.remote[..i].iter().any(|o| o.name.trim() == r.name.trim()) {
                return Err(msg::line(msg::CFG_DUPLICATE, &["[[remote]]", "name", r.name.trim()]));
            }
        }
        if let Some(s) = &c.standby {
            s.check(&c.remote)?;
        }
        if let Some(p) = &c.primary {
            if c.standby.is_some() {
                return Err(msg::line(msg::CFG_STANDBY_AND_PRIMARY, &[]));
            }
            p.check(&c.remote)?;
        }
        for (name, n) in &c.locks {
            if !crate::calendar::valid_name(name) {
                return Err(msg::line(msg::CFG_LOCK_NAME, &[name]));
            }
            if !n.as_integer().is_some_and(|n| n >= 1) {
                return Err(msg::line(msg::CFG_LOCK_COUNT, &[name]));
            }
        }
        if let Some(w) = &c.work_dir {
            let w = w.trim();
            if w != DEFINITION && !Path::new(w).is_absolute() {
                return Err(msg::line(msg::CFG_WORK_DIR, &[w, DEFINITION, "D:\\batch"]));
            }
        }
        for b in &c.blackout {
            b.check()?;
        }
        for (name, v) in &c.vars {
            if !crate::jobnet::plain_name(name) {
                return Err(msg::line(msg::CFG_NAME_NOT_PLAIN, &["[vars]", name, &format!("{{var:{name}}}")]));
            }
            let Some(s) = v.as_str() else {
                return Err(msg::line(msg::CFG_VARS_NOT_STRING, &[name]));
            };
            if !crate::template::param_names(s).is_empty() || !crate::template::var_names(s).is_empty() {
                return Err(msg::line(msg::CFG_VARS_NESTED, &[name]));
            }
            crate::template::check(s).map_err(|e| msg::line(msg::CFG_VARS_VALUE, &[name, msg::uncoded(&e)]))?;
        }
        for (name, v) in &c.groups {
            if !crate::jobnet::plain_name(name) {
                return Err(msg::line(msg::CFG_NAME_NOT_PLAIN, &["[groups]", name, &format!("on = \"group:{name}\"")]));
            }
            group_of(name, v)?;
        }
        Ok(c)
    }

    /// The PCs of the `[groups]` group `name`, for an `ssh` or a `winrm` (`kind`) with
    /// `on = "group:name"`.
    pub fn members(&self, name: &str, kind: &str) -> Result<Vec<Member>, String> {
        let Some(v) = self.groups.get(name) else {
            return Err(msg::line(msg::CFG_GROUP_MISSING, &[name]));
        };
        let (scheme, members) = group_of(name, v)?;
        if scheme != kind {
            return Err(msg::line(msg::CFG_GROUP_WRONG_KIND, &[name, scheme, kind]));
        }
        Ok(members)
    }

    /// `[vars]`' values, for `{var:…}` ([`Config::parse`] has seen that each is a string).
    pub fn vars(&self) -> std::collections::BTreeMap<String, String> {
        self.vars.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect()
    }

    /// The folder a job without `dir` runs in, with `folder` the one its net's file is in;
    /// `None` is the engine's own working folder.
    pub fn work_folder(&self, folder: Option<&Path>) -> Option<PathBuf> {
        match self.work_dir.as_deref().map(str::trim)? {
            DEFINITION => folder.map(Path::to_path_buf),
            p => Some(PathBuf::from(p)),
        }
    }

    /// How many nets may hold the lock `name` exclusively at once: `[locks]`'s number, or 1.
    pub fn lock_count(&self, name: &str) -> usize {
        self.locks.get(name).and_then(toml::Value::as_integer).map_or(1, |n| usize::try_from(n).unwrap_or(usize::MAX).max(1))
    }

    /// The file, or the defaults when there is none.
    pub fn load(path: &std::path::Path) -> Result<Config, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Config::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn log_max_bytes(&self) -> u64 {
        self.log_max_mb.saturating_mul(1024 * 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `said` is the line of `id` and holds `phrase`.
    fn has(said: impl AsRef<str>, id: msg::Id, phrase: &str) -> bool {
        id.is_in(said.as_ref()) && said.as_ref().contains(phrase)
    }

    #[test]
    fn missing_fields_are_the_defaults() {
        let c = Config::parse("concurrency = 2\n").unwrap();
        assert_eq!(c, Config { concurrency: 2, ..Config::default() });
        assert_eq!(Config::parse("").unwrap(), Config::default());
        let c = Config::parse(
            "archive = true
archive_days = 365
",
        )
        .unwrap();
        assert_eq!((c.archive, c.archive_days, c.keep_days), (true, 365, 90));
        assert!(!Config::default().archive);
    }

    #[test]
    fn a_typo_is_an_error_not_a_default() {
        assert!(Config::parse("concurency = 2\n").is_err());
        assert!(Config::parse("concurrency = 0\n").is_err());
        assert!(Config::parse("keep_days = 0\n").is_err());
        assert!(Config::parse("log_max_mb = 0\n").is_err());
        assert!(Config::parse("notify = 1\n").is_err());
    }

    /// `[groups]`: arrays of PCs reached the same way, each a string or a table with its own key.
    #[test]
    fn groups_read_their_pcs_and_are_checked() {
        let hk = "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU";
        let cert = "3F:2A:91:0C:5B:77:E4:18:A0:6D:C2:9F:41:B8:0E:D3:57:6A:FC:21:94:8B:3E:70:CD:12:65:A9:F0:4E:B7:83";
        let c = Config::parse(&format!(
            "[groups]\nweb = [\"ssh://ops@web1\", {{ on = \" ssh://ops@web2:2222 \", host_key = \"{hk}\" }}]\napp = [{{ on = 'winrm://CORP\\ops@app01', cert = \"{cert}\" }}]\n"
        ))
        .unwrap();
        let web = c.members("web", "ssh").unwrap();
        assert_eq!(
            web,
            [
                Member { on: "ssh://ops@web1".into(), host_key: None, cert: None },
                Member { on: "ssh://ops@web2:2222".into(), host_key: Some(hk.into()), cert: None },
            ]
        );
        assert_eq!(c.members("app", "winrm").unwrap()[0].cert.as_deref(), Some(cert));
        assert!(has(c.members("db", "ssh").unwrap_err(), msg::CFG_GROUP_MISSING, "グループ db が、koyomi.toml の [groups] にありません"));
        assert!(has(c.members("web", "winrm").unwrap_err(), msg::CFG_GROUP_WRONG_KIND, "グループ web の PC は ssh:// で指定されており"));
        assert!(has(c.members("app", "ssh").unwrap_err(), msg::CFG_GROUP_WRONG_KIND, "winrm://"));
        let bad = |t: &str| Config::parse(&format!("[groups]\n{t}\n")).unwrap_err();
        assert!(has(bad("\"web-1\" = ['ssh://h']"), msg::CFG_NAME_NOT_PLAIN, "名前 web-1 は使用できません"));
        assert!(has(bad("web = 'ssh://h'"), msg::CFG_GROUP_NOT_ARRAY, "グループ web が、PC の配列で指定されていません"));
        assert!(has(bad("web = []"), msg::CFG_GROUP_NOT_ARRAY, "PC の配列で指定されていません"));
        assert!(has(bad("web = [3]"), msg::CFG_GROUP_NOT_ARRAY, "PC の配列で指定されていません"));
        assert!(has(bad("web = ['h1']"), msg::CFG_GROUP_ADDRESS, "ssh://[ユーザー@]ホスト[:ポート]、または winrm://"));
        assert!(has(bad("web = ['ssh://h/tmp']"), msg::CFG_GROUP_MEMBER, "パスは指定できません"));
        assert!(has(bad("web = ['winrm://h']"), msg::CFG_GROUP_MEMBER, "ユーザーが指定されていません"));
        assert!(has(bad("web = ['ssh://h', 'winrm://u@h2']"), msg::CFG_GROUP_MIXED, "混在しています"));
        assert!(has(bad("web = ['ssh://h', ' SSH://h ']"), msg::CFG_GROUP_DUPLICATE, "同じ PC が重複して指定されています"));
        assert!(has(bad("web = ['{var:h}']"), msg::CFG_GROUP_BRACES, "{…} を含む値は指定できません"));
        assert!(has(bad("web = [{ host_key = 'x' }]"), msg::CFG_GROUP_NO_ON, "on が指定されていません"));
        assert!(has(bad("web = [{ on = 'ssh://h', user = 'x' }]"), msg::CFG_GROUP_UNKNOWN_KEY, "項目 user"));
        assert!(has(bad("web = [{ on = 'ssh://h', host_key = 'abc' }]"), msg::CFG_GROUP_MEMBER, "SHA256:"));
        assert!(has(bad("web = [{ on = 'ssh://h', cert = 'AB' }]"), msg::CFG_GROUP_KEY_SCHEME, "cert は winrm:// の PC にだけ"));
        assert!(has(bad("web = [{ on = 'winrm://u@h', host_key = 'x' }]"), msg::CFG_GROUP_KEY_SCHEME, "host_key は ssh:// の PC にだけ"));
        assert!(has(bad("web = [{ on = 'winrm://u@h', cert = 'AB' }]"), msg::CFG_GROUP_MEMBER, "AB:CD:…"));
        let many = (0..=crate::chore::GROUP_MAX).map(|i| format!("'ssh://h{i}'")).collect::<Vec<_>>().join(", ");
        assert!(has(bad(&format!("web = [{many}]")), msg::CFG_GROUP_TOO_MANY, "64 台を超えています"));
        assert!(Config::default().groups.is_empty());
    }

    /// `[vars]`: strings with plain names, which may hold the day's tokens but no other `{…:…}`.
    #[test]
    fn vars_read_and_are_checked() {
        let c = Config::parse("[vars]\nshare = '\\\\nas\\backup'\nday = '{date:%Y%m%d}'\n").unwrap();
        assert_eq!(c.vars().get("share").map(String::as_str), Some(r"\\nas\backup"));
        assert_eq!(c.vars().len(), 2);
        assert!(Config::default().vars().is_empty());
        let bad = |t: &str| Config::parse(&format!("[vars]\n{t}\n")).unwrap_err();
        assert!(has(bad("\"共有\" = 'x'"), msg::CFG_NAME_NOT_PLAIN, "名前 共有 は使用できません"));
        assert!(has(bad("n = 3"), msg::CFG_VARS_NOT_STRING, "[vars] の n が、文字列で指定されていません"));
        assert!(has(bad("a = '{var:b}'"), msg::CFG_VARS_NESTED, "{param:…} または {var:…} が含まれています"));
        assert!(has(bad("a = '{param:b}'"), msg::CFG_VARS_NESTED, "{param:…} または {var:…} が含まれています"));
        assert!(has(
            bad("a = '{date-1q}'"),
            msg::CFG_VARS_VALUE,
            "[vars] の a の値が正しくありません。（{date-1q} の単位 q を読み込めません"
        ));
    }

    /// `[[blackout]]`: read as written, checked, and a typo in one is an error.
    #[test]
    fn blackouts_read_and_are_checked() {
        let c = Config::parse(
            "[[blackout]]
from = \"2026-12-29\"
to = \"2027-01-03\"
why = \"年末年始\"

[[blackout]]
from = \"2026-11-14 22:00\"
to = \"2026-11-15 06:00\"
",
        )
        .unwrap();
        assert_eq!(c.blackout.len(), 2);
        assert_eq!((c.blackout[0].why.as_str(), c.blackout[1].why.as_str()), ("年末年始", ""));
        let e = Config::parse("[[blackout]]\nfrom = \"2026-12-29\"\nto = \"2026-12-28\"\n").unwrap_err();
        assert!(has(&e, msg::BLACKOUT_ORDER, "to（2026-12-28）が、from（2026-12-29）より前になっています"), "{e}");
        let e = Config::parse("[[blackout]]\nfrom = \"2026-12-29\"\n").unwrap_err();
        assert!(has(&e, msg::CFG_MISSING_KEY, "必須の項目 to") && e.contains("行目"), "{e}");
        let e = Config::parse("[[blackout]]\nfrom = \"2026-12-29\"\nto = \"2026-12-30\"\nreason = \"x\"\n").unwrap_err();
        assert!(has(&e, msg::CFG_UNKNOWN_KEY, "4 行目") && e.contains("未知の項目 reason"), "{e}");
        assert!(Config::default().blackout.is_empty());
    }

    /// `work_dir`: not written, the engine's folder; `"definition"`, the net's own; else one
    /// absolute folder. A relative one would hang on where the engine was started, so it is refused.
    #[test]
    fn work_dir_is_the_engine_s_the_definition_s_or_one_folder() {
        let net = Path::new("C:/koyomi/jobnets/経理");
        assert_eq!(Config::default().work_folder(Some(net)), None);
        let c = Config::parse("work_dir = \"definition\"\n").unwrap();
        assert_eq!((c.work_folder(Some(net)), c.work_folder(None)), (Some(net.to_path_buf()), None));
        let abs = std::env::temp_dir().display().to_string();
        let c = Config::parse(&format!("work_dir = '{abs}'\n")).unwrap();
        assert_eq!((c.work_folder(Some(net)), c.work_dir.as_deref()), (Some(PathBuf::from(&abs)), Some(abs.as_str())));
        for bad in ["batch", "", ".\\batch", "Definition"] {
            let e = Config::parse(&format!("work_dir = '{bad}'\n")).unwrap_err();
            assert!(has(&e, msg::CFG_WORK_DIR, "work_dir の値"), "{bad}: {e}");
        }
    }

    /// A value that is not one of the allowed words is said in Japanese, with its line.
    #[test]
    fn an_unknown_word_is_japanese_with_a_line() {
        let e = Config::parse(
            "[standby]
name = \"x\"
give_back = \"sometimes\"
",
        )
        .unwrap_err();
        assert!(
            e.contains("3 行目") && e.contains("\"sometimes\"") && e.contains("manual、auto") && msg::CFG_VALUE_UNKNOWN.is_in(&e),
            "{e}"
        );
        assert!(!e.is_ascii() && !e.contains("unknown variant"), "{e}");
    }

    /// `[locks]` gives a lock name more than one place; a name not listed has one.
    #[test]
    fn locks_count_places_and_a_name_not_listed_has_one() {
        let c = Config::parse("[locks]\n\"API枠\" = 3\n\"販売DB\" = 1\n").unwrap();
        assert_eq!((c.lock_count("API枠"), c.lock_count("販売DB"), c.lock_count("ほか")), (3, 1, 1));
        assert_eq!(Config::default().lock_count("API枠"), 1);
        for (bad, want) in [
            ("[locks]\n\"API枠\" = 0\n", "[locks] の API枠 の数が正しくありません"),
            ("[locks]\n\"API枠\" = -2\n", "の数が正しくありません"),
            ("[locks]\n\"API枠\" = \"3\"\n", "の数が正しくありません"),
            ("[locks]\n\"API枠\" = 2.5\n", "の数が正しくありません"),
            ("[locks]\n\"a/b\" = 2\n", "[locks] の a/b が、ロック名として使用できません"),
            ("[locks]\n\"\" = 2\n", "ロック名として使用できません"),
            ("locks = 3\n", ""),
        ] {
            let e = Config::parse(bad).unwrap_err();
            assert!(e.contains(want), "{bad}: {e}");
        }
    }

    #[test]
    fn a_ping_needs_an_address_and_a_sane_interval() {
        let c = Config::parse(
            "[ping]
url = \"secret:ping\"
",
        )
        .unwrap();
        assert_eq!(c.ping.as_ref().map(Ping::every_secs), Some(300));
        let c = Config::parse(
            "[ping]
url = \"https://hc-ping.com/x\"
every = \"1h\"
",
        )
        .unwrap();
        assert_eq!(c.ping.unwrap().every_secs(), 3600);
        assert!(Config::parse("").unwrap().ping.is_none());
        for bad in [
            "[ping]
",
            "[ping]
url = \"hc-ping.com/x\"
",
            "[ping]
url = \"secret:\"
",
            "[ping]
url = \"secret:p\"
every = \"30s\"
",
            "[ping]
url = \"secret:p\"
every = \"25h\"
",
            "[ping]
url = \"secret:p\"
every = \"5\"
",
            "[ping]
url = \"secret:p\"
evry = \"5m\"
",
        ] {
            assert!(Config::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn http_errors_are_japanese_with_a_line() {
        for t in [
            "[http]
token = \"secret:t\"
",
            "
[http]
port = 8790
host = \"0.0.0.0\"
",
            "[http]
port = \"x\"
",
            "[http]
port = 8790
ipv6 = 3
",
            "[http]
port = 70000
",
        ] {
            let e = Config::parse(t).unwrap_err();
            assert!(e.contains("行目") && !e.is_ascii(), "{e}");
        }
    }

    #[test]
    fn http_takes_a_port_and_the_token_only_as_a_secret() {
        let c = Config::parse("[http]\nport = 8790\ntoken = \"secret:koyomi-http\"\n").unwrap();
        let h = c.http.unwrap();
        assert_eq!((h.port, h.secret(), h.ipv6, h.lan), (8790, "koyomi-http", false, false));
        assert!(Config::parse("[http]\nport = 8790\ntoken = \"secret:t\"\nlan = true\n").unwrap().http.unwrap().lan);
        assert!(Config::parse("").unwrap().http.is_none());
        for bad in [
            "[http]\nport = 8790\ntoken = \"plain-token\"\n",
            "[http]\nport = 8790\ntoken = \"secret: \"\n",
            "[http]\nport = 0\ntoken = \"secret:t\"\n",
            "[http]\nport = 70000\ntoken = \"secret:t\"\n",
            "[http]\ntoken = \"secret:t\"\n",
            "[http]\nport = 8790\ntoken = \"secret:t\"\nhost = \"0.0.0.0\"\n",
        ] {
            assert!(Config::parse(bad).is_err(), "{bad}");
        }
    }

    /// each person has a token and a role; the nameless token may then be left out.
    #[test]
    fn http_users_have_a_role_and_a_secret_token() {
        let user =
            |name: &str, role: &str, token: &str| format!("[[http.user]]\nname = \"{name}\"\nrole = \"{role}\"\ntoken = \"{token}\"\n");
        let c =
            Config::parse(&format!("[http]\nport = 8790\n{}{}", user("sato", "view", "secret:sato"), user("ito", "edit", "secret:ito")))
                .unwrap();
        let h = c.http.unwrap();
        assert_eq!((h.secret(), h.user.len()), ("", 2));
        assert_eq!((h.user[0].role(), h.user[1].secret()), (crate::role::Role::View, "ito"));
        for bad in [
            "[http]\nport = 8790\n".to_owned(),
            format!("[http]\nport = 8790\n{}", user("sato", "owner", "secret:s")),
            format!("[http]\nport = 8790\n{}", user("sato", "view", "plain")),
            format!("[http]\nport = 8790\n{}", user("default", "view", "secret:s")),
            format!("[http]\nport = 8790\n{}", user("a b", "view", "secret:s")),
            format!("[http]\nport = 8790\n{}{}", user("sato", "view", "secret:s"), user("sato", "run", "secret:t")),
        ] {
            assert!(Config::parse(&bad).is_err(), "{bad}");
        }
    }

    /// `nets` narrows a person to some nets; without it they reach every net, as before.
    #[test]
    fn an_http_user_may_be_narrowed_to_some_nets() {
        let head = "[http]\nport = 8790\n[[http.user]]\nname = \"sato\"\nrole = \"edit\"\ntoken = \"secret:sato\"\n";
        let c = Config::parse(&format!("{head}nets = [\"経理/*\", \"共通/日次\"]\n")).unwrap();
        let n = c.http.unwrap().user[0].nets().unwrap();
        assert!(n.reaches("経理/月次") && n.reaches("共通/日次") && !n.reaches("共通/週次"));
        assert_eq!(Config::parse(head).unwrap().http.unwrap().user[0].nets(), None);
        let e = Config::parse(&format!("{head}nets = [\"経理\"]\n")).unwrap_err();
        assert!(
            has(
                &e,
                msg::CFG_USER_FIELD,
                "[[http.user]] sato の設定が正しくありません。（nets に指定した 経理 が、<グループ>/<名前> の形式ではありません"
            ),
            "{e}"
        );
        assert!(Config::parse(&format!("{head}nets = []\n")).unwrap_err().contains("nets が空です"));
    }

    /// `roles` gives a role per net; without it `role` is on every net, as before.
    #[test]
    fn an_http_user_may_have_a_role_per_net() {
        use crate::role::Role;
        let head = "[http]\nport = 8790\n[[http.user]]\nname = \"sato\"\nrole = \"view\"\ntoken = \"secret:sato\"\n";
        let c = Config::parse(&format!("{head}roles = {{ \"経理/*\" = \"run\", \"基盤/監視\" = \"edit\" }}\n")).unwrap();
        let a = c.http.unwrap().user[0].access();
        assert_eq!((a.role, a.nets.as_ref(), a.roles.len()), (Role::View, None, 2));
        assert_eq!((a.role_on("経理/月次"), a.role_on("基盤/監視"), a.role_on("人事/給与")), (Role::Run, Role::Edit, Role::View));
        let plain = Config::parse(head).unwrap().http.unwrap().user[0].access();
        assert_eq!(plain, crate::role::Access::all(Role::View));
        let e = Config::parse(&format!("{head}roles = {{ \"経理\" = \"run\" }}\n")).unwrap_err();
        assert!(
            has(
                &e,
                msg::CFG_USER_FIELD,
                "[[http.user]] sato の設定が正しくありません。（roles に指定した 経理 が、<グループ>/<名前> の形式ではありません"
            ),
            "{e}"
        );
        let e = Config::parse(&format!("{head}roles = {{ \"経理/*\" = \"admin\" }}\n")).unwrap_err();
        assert!(has(&e, msg::CFG_USER_FIELD, "roles の 経理/* に指定した role admin は使用できません"), "{e}");
        assert!(Config::parse(&format!("{head}roles = {{}}\n")).unwrap_err().contains("roles が空です"));
    }

    /// `admin` is a role of its own and is not narrowed: `nets` and `roles` are refused with it,
    /// and `admin` is not a role a net can be given.
    #[test]
    fn admin_is_not_narrowed() {
        use crate::role::Role;
        let head = "[http]
port = 8790
[[http.user]]
name = \"boss\"
role = \"admin\"
token = \"secret:boss\"
";
        assert_eq!(Config::parse(head).unwrap().http.unwrap().user[0].role(), Role::Admin);
        let e = Config::parse(&format!(
            "{head}nets = [\"経理/*\"]
"
        ))
        .unwrap_err();
        assert!(has(&e, msg::CFG_USER_ADMIN_NARROWED, "[[http.user]] boss は role = \"admin\" ですが"), "{e}");
        let e = Config::parse(&format!(
            "{head}roles = {{ \"経理/*\" = \"run\" }}
"
        ))
        .unwrap_err();
        assert!(has(&e, msg::CFG_USER_ADMIN_NARROWED, "[[http.user]] boss は role = \"admin\" ですが"), "{e}");
        let low = head.replace("admin", "run");
        let e = Config::parse(&format!(
            "{low}roles = {{ \"経理/*\" = \"admin\" }}
"
        ))
        .unwrap_err();
        assert!(has(&e, msg::CFG_USER_FIELD, "roles の 経理/* に指定した role admin は使用できません"), "{e}");
    }

    /// `[http] tailnet` and a person's `tailscale` go together; such a person needs no token.
    #[test]
    fn a_tailscale_login_or_tag_stands_in_for_the_token_on_the_tailnet() {
        let user = |name: &str, more: &str| format!("[[http.user]]\nname = \"{name}\"\nrole = \"run\"\n{more}");
        let ok = format!(
            "[http]\nport = 8790\ntailnet = 8791\n{}{}{}",
            user("sato", "tailscale = \"Sato@Example.com\"\n"),
            user("ha", "tailscale = \"tag:koyomi-ha\"\ntoken = \"secret:ha\"\n"),
            user("ito", "token = \"secret:ito\"\n")
        );
        let h = Config::parse(&ok).unwrap().http.unwrap();
        assert_eq!(h.tailnet, Some(8791));
        assert_eq!((h.user[0].secret(), h.user[0].tailscale()), ("", Some("Sato@Example.com")));
        assert_eq!((h.user[1].secret(), h.user[1].tailscale()), ("ha", Some("tag:koyomi-ha")));
        assert_eq!(h.user[2].tailscale(), None);
        let plain = Config::parse("[http]\nport = 8790\ntoken = \"secret:t\"\n").unwrap().http.unwrap();
        assert_eq!(plain.tailnet, None);
        for (bad, (id, why)) in [
            (
                format!("[http]\nport = 8790\n{}", user("sato", "tailscale = \"sato@example.com\"\n")),
                (msg::CFG_TAILSCALE_NEEDS_TAILNET, "[http] に tailnet がありません"),
            ),
            (
                format!("[http]\nport = 8790\ntailnet = 8791\n{}", user("ito", "token = \"secret:ito\"\n")),
                (msg::CFG_TAILNET_NO_USER, "tailscale で接続する利用者がいません"),
            ),
            (
                format!("[http]\nport = 8790\ntailnet = 8790\n{}", user("sato", "tailscale = \"sato@example.com\"\n")),
                (msg::CFG_TAILNET_PORT, "tailnet が正しくありません"),
            ),
            (
                format!("[http]\nport = 8790\ntailnet = 0\n{}", user("sato", "tailscale = \"sato@example.com\"\n")),
                (msg::CFG_TAILNET_PORT, "tailnet が正しくありません"),
            ),
            (
                format!("[http]\nport = 8790\ntailnet = 8791\n{}", user("sato", "tailscale = \"sato\"\n")),
                (msg::CFG_TAILSCALE_FORM, "tailscale が正しくありません"),
            ),
            (
                format!("[http]\nport = 8790\ntailnet = 8791\n{}", user("sato", "tailscale = \"tag:\"\n")),
                (msg::CFG_TAILSCALE_FORM, "tailscale が正しくありません"),
            ),
            (
                format!("[http]\nport = 8790\ntailnet = 8791\n{}", user("sato", "tailscale = \"sato @example.com\"\n")),
                (msg::CFG_TAILSCALE_FORM, "tailscale が正しくありません"),
            ),
            (
                format!("[http]\nport = 8790\ntailnet = 8791\n{}", user("sato", "tailscale = \"sato@example.com\"\ntoken = \"plain\"\n")),
                (msg::CFG_TOKEN_NOT_SECRET, "token が、secret:<名前> の形式で指定されていません"),
            ),
            (
                format!(
                    "[http]\nport = 8790\ntailnet = 8791\n{}{}",
                    user("sato", "tailscale = \"sato@example.com\"\n"),
                    user("kato", "tailscale = \"SATO@example.com\"\n")
                ),
                (msg::CFG_DUPLICATE, "tailscale SATO@example.com が重複しています"),
            ),
            (
                format!("[http]\nport = 8790\ntailnet = 8791\n{}{}", user("sato", "tailscale = \"sato@example.com\"\n"), user("ito", "")),
                (msg::CFG_TOKEN_NOT_SECRET, "ito の token が"),
            ),
        ] {
            let e = Config::parse(&bad).unwrap_err();
            assert!(has(&e, id, why), "{bad}: {e}");
        }
    }

    /// a remote needs an https address, a SHA-256 fingerprint and the token as a secret.
    #[test]
    fn remotes_are_pinned_https_with_a_secret_token() {
        let fp = "AB:".repeat(31) + "CD";
        let one = |name: &str, url: &str, fp: &str, token: &str| {
            format!("[[remote]]\nname = \"{name}\"\nurl = \"{url}\"\nfingerprint = \"{fp}\"\ntoken = \"{token}\"\n")
        };
        let c = Config::parse(&one("pc2", "https://pc2:8790", &fp, "secret:pc2")).unwrap();
        assert_eq!((c.remote[0].name.as_str(), c.remote[0].secret()), ("pc2", "pc2"));
        assert!(Config::parse("").unwrap().remote.is_empty());
        for bad in [
            one("pc2", "http://pc2:8790", &fp, "secret:pc2"),
            one("pc2", "https://pc2:8790/runs", &fp, "secret:pc2"),
            one("pc2", "https://pc2:8790", "AB:CD", "secret:pc2"),
            one("pc2", "https://pc2:8790", &fp, "plain"),
            one("", "https://pc2:8790", &fp, "secret:pc2"),
            one("pc2", "https://a:1", &fp, "secret:a") + &one("pc2", "https://b:1", &fp, "secret:b"),
        ] {
            assert!(Config::parse(&bad).is_err(), "{bad}");
        }
    }

    /// a standby names a remote, and polls at a sane pace.
    #[test]
    fn a_standby_watches_a_named_remote() {
        let fp = "AB:".repeat(31) + "CD";
        let remote = format!(
            "[[remote]]
name = \"pc1\"
url = \"https://pc1:8790\"
fingerprint = \"{fp}\"
token = \"secret:pc1\"
"
        );
        let c = Config::parse(&format!(
            "{remote}[standby]
watch = \"pc1\"
"
        ))
        .unwrap();
        let s = c.standby.unwrap();
        assert_eq!((s.watch.as_str(), s.every_secs(), s.misses), ("pc1", 20, 3));
        let s = Config::parse(&format!(
            "{remote}[standby]
watch = \"pc1\"
every = \"1m\"
misses = 5
"
        ))
        .unwrap()
        .standby
        .unwrap();
        assert_eq!((s.every_secs(), s.misses), (60, 5));
        for bad in [
            "[standby]
watch = \"pc1\"
"
            .to_owned(),
            format!(
                "{remote}[standby]
watch = \"pc2\"
"
            ),
            format!(
                "{remote}[standby]
watch = \"pc1\"
every = \"1s\"
"
            ),
            format!(
                "{remote}[standby]
watch = \"pc1\"
every = \"1h\"
"
            ),
            format!(
                "{remote}[standby]
watch = \"pc1\"
misses = 0
"
            ),
            format!(
                "{remote}[standby]
watch = \"pc1\"
mises = 3
"
            ),
            format!(
                "{remote}[standby]
watch = \"pc1\"
give_back = \"yes\"
"
            ),
        ] {
            assert!(Config::parse(&bad).is_err(), "{bad}");
        }
    }

    /// give_back is manual unless written; [primary] names the remote that stands by, never with [standby].
    #[test]
    fn a_primary_names_its_standby() {
        let fp = "AB:".repeat(31) + "CD";
        let remote = format!(
            "[[remote]]
name = \"pc2\"
url = \"https://pc2:8790\"
fingerprint = \"{fp}\"
token = \"secret:pc2\"
"
        );
        let s = Config::parse(&format!("{remote}[standby]\nwatch = \"pc2\"\n")).unwrap().standby.unwrap();
        assert_eq!(s.give_back, GiveBack::Manual);
        let s = Config::parse(&format!("{remote}[standby]\nwatch = \"pc2\"\ngive_back = \"auto\"\n")).unwrap().standby.unwrap();
        assert_eq!(s.give_back, GiveBack::Auto);
        let p = Config::parse(&format!("{remote}[primary]\nstandby = \"pc2\"\n")).unwrap().primary.unwrap();
        assert_eq!((p.standby.as_str(), p.every_secs(), p.misses), ("pc2", 20, 3));
        assert!(has(
            Config::parse(&format!("{remote}[primary]\nstandby = \"pc9\"\n")).unwrap_err(),
            msg::CFG_POLL_TARGET,
            "[primary] の standby に指定された pc9 は、[[remote]] の name にありません"
        ));
        assert!(has(
            Config::parse(&format!("{remote}[primary]\nstandby = \"pc2\"\nmisses = 30\n")).unwrap_err(),
            msg::CFG_MISSES_RANGE,
            "[primary] の misses が範囲外です"
        ));
        assert!(msg::CFG_STANDBY_AND_PRIMARY
            .is_in(&Config::parse(&format!("{remote}[primary]\nstandby = \"pc2\"\n[standby]\nwatch = \"pc2\"\n")).unwrap_err()));
        assert!(Config::parse("").unwrap().primary.is_none());
    }

    #[test]
    fn the_notify_table_is_kept_as_written() {
        let c = Config::parse("[notify]\nsystem = [\"failed\"]\n").unwrap();
        assert_eq!(c.notify.get("system").and_then(|v| v.as_array()).map(Vec::len), Some(1));
        assert!(Config::default().notify.is_empty());
    }
}
