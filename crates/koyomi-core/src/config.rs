//! `koyomi.toml` in the settings folder: how many jobs run at once, how long
//! history is kept (and whether it goes into `archive/` zips first), how big one job's log may grow, `[notify]` (read by
//! the app with ito-prefs's shape; kept whole here), `[ping]`, the
//! address the engine calls while it is alive, `[http]`, where it
//! listens for other programs (on this PC, or with `lan` on the network over
//! TLS), `[[remote]]`, the other PCs' engines this one calls, `[standby]`, which of them
//! this one stands by for (HA), `[primary]`, which of them stands by for this one, and
//! `[locks]`, how many nets may hold a `lock` at once, and `work_dir`, where a job without
//! `dir` runs, `[[blackout]]`, the planned stops in which nothing starts by itself, and
//! `[vars]`, the values every net reads as `{var:name}`.
//!
//! Missing file: the defaults. A file that does not read is an error the
//! engine shows and does not run with (a typo must not quietly mean "4").

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
        return Err(format!("[{section}] の {key} {name:?} は [[remote]] の name で書く"));
    }
    let secs = crate::duration::parse(every).map_err(|e| format!("[{section}] の every: {e}"))?;
    if !(5..=600).contains(&secs) {
        return Err(format!("[{section}] の every は 5s から 10m まで"));
    }
    if !(1..=20).contains(&misses) {
        return Err(format!("[{section}] の misses は 1 から 20 まで"));
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
            return Err("[ping] の url は secret:<名前> か http(s):// で始まるアドレス".to_owned());
        }
        let secs = crate::duration::parse(&self.every).map_err(|e| format!("[ping] の every: {e}"))?;
        if !(60..=86_400).contains(&secs) {
            return Err("[ping] の every は 1m から 24h まで".to_owned());
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
    /// `view`, `run` or `edit`.
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
            return Err("[[remote]] の name は空白を含まない名前".to_owned());
        }
        let host = self.url.strip_prefix("https://").unwrap_or_default().trim_end_matches('/');
        if host.is_empty() || host.contains(['/', '?', '#', '@', ' ']) {
            return Err(format!("[[remote]] {name} の url は https://<PC>:<port> で書く（LAN では TLS だけ）"));
        }
        let hex: Vec<char> = self.fingerprint.chars().filter(|c| !matches!(c, ':' | ' ' | '-')).collect();
        if hex.len() != 64 || !hex.iter().all(char::is_ascii_hexdigit) {
            return Err(format!(
                "[[remote]] {name} の fingerprint は SHA-256（向こうで koyomi http fingerprint、こちらで koyomi remote peek <url>）"
            ));
        }
        if self.secret().is_empty() {
            return Err(format!("[[remote]] {name} の token は secret:<名前> で書く"));
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
            return Err("[http] の port は 1 から 65535".to_owned());
        }
        if self.secret().is_empty() && (!self.token.trim().is_empty() || self.user.is_empty()) {
            return Err("[http] の token は secret:<名前> で書く（値は koyomi secret set <名前> で OS の資格情報に置く）".to_owned());
        }
        if let Some(t) = self.tailnet {
            if t == 0 || t == self.port {
                return Err("[http] の tailnet は port と別のポート（1 から 65535）".to_owned());
            }
            if !self.user.iter().any(|u| u.tailscale.is_some()) {
                return Err("[http] の tailnet で通す人がいない（[[http.user]] に tailscale = \"<ログイン名>\" を書く）".to_owned());
            }
        }
        let (mut names, mut tailscales) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
        for u in &self.user {
            let name = u.name.trim();
            if name.is_empty() || name.contains(char::is_whitespace) || name == "default" {
                return Err("[[http.user]] の name は空白を含まない名前（default は使わない）".to_owned());
            }
            if !names.insert(name) {
                return Err(format!("[[http.user]] の name {name} が 2 つある"));
            }
            crate::role::Role::parse(u.role.trim()).map_err(|e| format!("[[http.user]] {name} の {e}"))?;
            if u.secret().is_empty() && (!u.token.trim().is_empty() || u.tailscale.is_none()) {
                return Err(format!("[[http.user]] {name} の token は secret:<名前> で書く"));
            }
            if let Some(t) = &u.tailscale {
                let t = t.trim();
                let reads = match t.strip_prefix("tag:") {
                    Some(tag) => !tag.is_empty(),
                    None => t.contains('@') && !t.starts_with('@'),
                };
                if !reads || t.contains(char::is_whitespace) {
                    return Err(format!(
                        "[[http.user]] {name} の tailscale は Tailscale のログイン名（sato@example.com）かタグ（tag:koyomi-ha）で書く: {t}"
                    ));
                }
                if self.tailnet.is_none() {
                    return Err(format!(
                        "[[http.user]] {name} の tailscale は [http] の tailnet（tailnet で待ち受けるポート）と一緒に書く"
                    ));
                }
                if !tailscales.insert(t.to_lowercase()) {
                    return Err(format!("[[http.user]] の tailscale {t} が 2 つある"));
                }
            }
            if let Some(p) = &u.nets {
                crate::role::Nets::parse(p).map_err(|e| format!("[[http.user]] {name} の {e}"))?;
            }
            if let Some(r) = &u.roles {
                crate::role::per_net(r).map_err(|e| format!("[[http.user]] {name} の {e}"))?;
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
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        let c: Config = toml::from_str(text).map_err(|e| {
            let m = e.message();
            let line =
                e.span().map(|s| format!("{} 行目: ", text[..s.start.min(text.len())].matches('\n').count() + 1)).unwrap_or_default();
            if let Some(f) = m.strip_prefix("unknown field `").and_then(|r| r.split('`').next()) {
                format!("{line}知らない欄 {f}")
            } else if let Some(f) = m.strip_prefix("missing field `").and_then(|r| r.split('`').next()) {
                format!("{line}欄 {f} が無い")
            } else {
                m.to_owned()
            }
        })?;
        if c.concurrency == 0 {
            return Err("concurrency は 1 以上".to_owned());
        }
        if c.keep_days == 0 {
            return Err("keep_days は 1 以上".to_owned());
        }
        if c.log_max_mb == 0 {
            return Err("log_max_mb は 1 以上".to_owned());
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
                return Err(format!("[[remote]] の name {} が 2 つある", r.name.trim()));
            }
        }
        if let Some(s) = &c.standby {
            s.check(&c.remote)?;
        }
        if let Some(p) = &c.primary {
            if c.standby.is_some() {
                return Err("[standby] と [primary] は一緒に書けない（待つ側か、待たれる側のどちらか）".to_owned());
            }
            p.check(&c.remote)?;
        }
        for (name, n) in &c.locks {
            if !crate::calendar::valid_name(name) {
                return Err(format!("[locks] の {name:?} は lock の名前で書く（ジョブネットの lock と同じ。/ \\ : などは使えない）"));
            }
            if !n.as_integer().is_some_and(|n| n >= 1) {
                return Err(format!("[locks] の {name:?} は同時に持てる数で、1 以上（例: \"API枠\" = 3）"));
            }
        }
        if let Some(w) = &c.work_dir {
            let w = w.trim();
            if w != DEFINITION && !Path::new(w).is_absolute() {
                return Err(format!(
                    "work_dir は \"{DEFINITION}\"（定義のあるフォルダー）か絶対パスで書く（例: work_dir = 'D:\\batch'）: {w}"
                ));
            }
        }
        for b in &c.blackout {
            b.check()?;
        }
        for (name, v) in &c.vars {
            if !crate::jobnet::plain_name(name) {
                return Err(format!("[vars] の名前に使えない: {name}（英数字と _ だけ。{{var:{name}}} で読むので）"));
            }
            let Some(s) = v.as_str() else {
                return Err(format!("[vars] の {name} は文字列で書く（例: {name} = '\\\\nas\\backup'）"));
            };
            if !crate::template::param_names(s).is_empty() || !crate::template::var_names(s).is_empty() {
                return Err(format!("[vars] の {name} に {{param:…}}・{{var:…}} は書けない（{{date}} などは書ける）"));
            }
            crate::template::check(s).map_err(|e| format!("[vars] の {name}: {e}"))?;
        }
        Ok(c)
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

    /// `[vars]`: strings with plain names, which may hold the day's tokens but no other `{…:…}`.
    #[test]
    fn vars_read_and_are_checked() {
        let c = Config::parse("[vars]\nshare = '\\\\nas\\backup'\nday = '{date:%Y%m%d}'\n").unwrap();
        assert_eq!(c.vars().get("share").map(String::as_str), Some(r"\\nas\backup"));
        assert_eq!(c.vars().len(), 2);
        assert!(Config::default().vars().is_empty());
        let bad = |t: &str| Config::parse(&format!("[vars]\n{t}\n")).unwrap_err();
        assert!(bad("\"共有\" = 'x'").contains("[vars] の名前に使えない: 共有"));
        assert!(bad("n = 3").contains("[vars] の n は文字列で書く"));
        assert!(bad("a = '{var:b}'").contains("{param:…}・{var:…} は書けない"));
        assert!(bad("a = '{param:b}'").contains("{param:…}・{var:…} は書けない"));
        assert!(bad("a = '{date-1q}'").contains("[vars] の a: {date-1q}: 単位が読めない"));
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
        assert!(e.contains("[[blackout]] の to（2026-12-28）が from（2026-12-29）より前"), "{e}");
        let e = Config::parse("[[blackout]]\nfrom = \"2026-12-29\"\n").unwrap_err();
        assert!(e.contains("欄 to が無い") && e.contains("行目"), "{e}");
        let e = Config::parse("[[blackout]]\nfrom = \"2026-12-29\"\nto = \"2026-12-30\"\nreason = \"x\"\n").unwrap_err();
        assert!(e.contains("4 行目: 知らない欄 reason"), "{e}");
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
            assert!(e.starts_with("work_dir は \"definition\"（定義のあるフォルダー）か絶対パスで書く"), "{bad}: {e}");
        }
    }

    /// `[locks]` gives a lock name more than one place; a name not listed has one.
    #[test]
    fn locks_count_places_and_a_name_not_listed_has_one() {
        let c = Config::parse("[locks]\n\"API枠\" = 3\n\"販売DB\" = 1\n").unwrap();
        assert_eq!((c.lock_count("API枠"), c.lock_count("販売DB"), c.lock_count("ほか")), (3, 1, 1));
        assert_eq!(Config::default().lock_count("API枠"), 1);
        for (bad, want) in [
            ("[locks]\n\"API枠\" = 0\n", "[locks] の \"API枠\" は同時に持てる数で、1 以上"),
            ("[locks]\n\"API枠\" = -2\n", "1 以上"),
            ("[locks]\n\"API枠\" = \"3\"\n", "1 以上"),
            ("[locks]\n\"API枠\" = 2.5\n", "1 以上"),
            ("[locks]\n\"a/b\" = 2\n", "[locks] の \"a/b\" は lock の名前で書く"),
            ("[locks]\n\"\" = 2\n", "lock の名前で書く"),
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
            format!("[http]\nport = 8790\n{}", user("sato", "admin", "secret:s")),
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
        assert!(e.contains("[[http.user]] sato の nets は <グループ>/<名前>"), "{e}");
        assert!(Config::parse(&format!("{head}nets = []\n")).unwrap_err().contains("空にしない"));
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
        assert!(e.contains("[[http.user]] sato の roles のキーは <グループ>/<名前>"), "{e}");
        let e = Config::parse(&format!("{head}roles = {{ \"経理/*\" = \"admin\" }}\n")).unwrap_err();
        assert!(e.contains("[[http.user]] sato の roles の 経理/* の role は"), "{e}");
        assert!(Config::parse(&format!("{head}roles = {{}}\n")).unwrap_err().contains("空にしない"));
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
        for (bad, why) in [
            (
                format!("[http]\nport = 8790\n{}", user("sato", "tailscale = \"sato@example.com\"\n")),
                "tailnet（tailnet で待ち受けるポート）と一緒に",
            ),
            (format!("[http]\nport = 8790\ntailnet = 8791\n{}", user("ito", "token = \"secret:ito\"\n")), "tailnet で通す人がいない"),
            (format!("[http]\nport = 8790\ntailnet = 8790\n{}", user("sato", "tailscale = \"sato@example.com\"\n")), "port と別のポート"),
            (format!("[http]\nport = 8790\ntailnet = 0\n{}", user("sato", "tailscale = \"sato@example.com\"\n")), "port と別のポート"),
            (
                format!("[http]\nport = 8790\ntailnet = 8791\n{}", user("sato", "tailscale = \"sato\"\n")),
                "ログイン名（sato@example.com）かタグ",
            ),
            (
                format!("[http]\nport = 8790\ntailnet = 8791\n{}", user("sato", "tailscale = \"tag:\"\n")),
                "ログイン名（sato@example.com）かタグ",
            ),
            (
                format!("[http]\nport = 8790\ntailnet = 8791\n{}", user("sato", "tailscale = \"sato @example.com\"\n")),
                "ログイン名（sato@example.com）かタグ",
            ),
            (
                format!("[http]\nport = 8790\ntailnet = 8791\n{}", user("sato", "tailscale = \"sato@example.com\"\ntoken = \"plain\"\n")),
                "token は secret:",
            ),
            (
                format!(
                    "[http]\nport = 8790\ntailnet = 8791\n{}{}",
                    user("sato", "tailscale = \"sato@example.com\"\n"),
                    user("kato", "tailscale = \"SATO@example.com\"\n")
                ),
                "tailscale SATO@example.com が 2 つある",
            ),
            (
                format!("[http]\nport = 8790\ntailnet = 8791\n{}{}", user("sato", "tailscale = \"sato@example.com\"\n"), user("ito", "")),
                "ito の token は secret:",
            ),
        ] {
            let e = Config::parse(&bad).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
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
        assert_eq!(
            Config::parse(&format!("{remote}[primary]\nstandby = \"pc9\"\n")).unwrap_err(),
            "[primary] の standby \"pc9\" は [[remote]] の name で書く"
        );
        assert_eq!(
            Config::parse(&format!("{remote}[primary]\nstandby = \"pc2\"\nmisses = 30\n")).unwrap_err(),
            "[primary] の misses は 1 から 20 まで"
        );
        assert_eq!(
            Config::parse(&format!("{remote}[primary]\nstandby = \"pc2\"\n[standby]\nwatch = \"pc2\"\n")).unwrap_err(),
            "[standby] と [primary] は一緒に書けない（待つ側か、待たれる側のどちらか）"
        );
        assert!(Config::parse("").unwrap().primary.is_none());
    }

    #[test]
    fn the_notify_table_is_kept_as_written() {
        let c = Config::parse("[notify]\nsystem = [\"failed\"]\n").unwrap();
        assert_eq!(c.notify.get("system").and_then(|v| v.as_array()).map(Vec::len), Some(1));
        assert!(Config::default().notify.is_empty());
    }
}
