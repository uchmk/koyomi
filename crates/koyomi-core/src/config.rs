//! `koyomi.toml` in the settings folder: how many jobs run at once, how long
//! history is kept, how big one job's log may grow, `[notify]` (read by
//! the app with ito-prefs's shape; kept whole here), `[ping]`, the
//! address the engine calls while it is alive, `[http]`, where it
//! listens for other programs (on this PC, or with `lan` on the network over
//! TLS), `[[remote]]`, the other PCs' engines this one calls, `[standby]`, which of them
//! this one stands by for (HA), and `[locks]`, how many nets may hold a `lock` at once.
//!
//! Missing file: the defaults. A file that does not read is an error the
//! engine shows and does not run with (a typo must not quietly mean "4").

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Jobs running at the same time, over every net.
    pub concurrency: usize,
    /// Days of history and logs kept.
    pub keep_days: u32,
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
    /// `[locks]`: `"API枠" = 3`, how many nets may hold that `lock` exclusively at once.
    /// A name not listed has 1. Kept as written so [`Config::parse`] can say what is wrong in Japanese.
    pub locks: toml::Table,
}

/// `[standby]`: poll the `[[remote]]` named `watch` every `every`; after `misses` polls in a row
/// it does not answer, take over its schedules (`koyomi standby giveback` hands them back).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Standby {
    pub watch: String,
    /// As a length (`20s`). From 5 seconds to 10 minutes.
    #[serde(default = "Standby::every_default")]
    pub every: String,
    /// From 1 to 20.
    #[serde(default = "Standby::misses_default")]
    pub misses: u32,
}

impl Standby {
    fn every_default() -> String {
        format!("{}s", crate::standby::INTERVAL.as_secs())
    }

    fn misses_default() -> u32 {
        crate::standby::MISSES
    }

    /// The seconds between polls; [`Config::parse`] has checked it reads.
    pub fn every_secs(&self) -> u64 {
        crate::duration::parse(&self.every).unwrap_or(crate::standby::INTERVAL.as_secs())
    }

    fn check(&self, remotes: &[Remote]) -> Result<(), String> {
        let watch = self.watch.trim();
        if !remotes.iter().any(|r| r.name.trim() == watch) {
            return Err(format!("[standby] の watch {watch:?} は [[remote]] の name で書く"));
        }
        let secs = crate::duration::parse(&self.every).map_err(|e| format!("[standby] の every: {e}"))?;
        if !(5..=600).contains(&secs) {
            return Err("[standby] の every は 5s から 10m まで".to_owned());
        }
        if !(1..=20).contains(&self.misses) {
            return Err("[standby] の misses は 1 から 20 まで".to_owned());
        }
        Ok(())
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
}

/// `[[http.user]]`: a person who calls over HTTP, with the role their token gets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpUser {
    /// Written down with what they did (`koyomi ops`).
    pub name: String,
    /// `view`, `run` or `edit`.
    pub role: String,
    /// `secret:<name>`.
    pub token: String,
    /// The nets they reach, as `<group>/<name>` patterns (`経理/*`, TODO 94); none: every net.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nets: Option<Vec<String>>,
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

    /// The name the token is kept under.
    pub fn secret(&self) -> &str {
        self.token.trim().strip_prefix("secret:").map(str::trim).unwrap_or_default()
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
        let mut names = std::collections::BTreeSet::new();
        for u in &self.user {
            let name = u.name.trim();
            if name.is_empty() || name.contains(char::is_whitespace) || name == "default" {
                return Err("[[http.user]] の name は空白を含まない名前（default は使わない）".to_owned());
            }
            if !names.insert(name) {
                return Err(format!("[[http.user]] の name {name} が 2 つある"));
            }
            crate::role::Role::parse(u.role.trim()).map_err(|e| format!("[[http.user]] {name} の {e}"))?;
            if u.secret().is_empty() {
                return Err(format!("[[http.user]] {name} の token は secret:<名前> で書く"));
            }
            if let Some(p) = &u.nets {
                crate::role::Nets::parse(p).map_err(|e| format!("[[http.user]] {name} の {e}"))?;
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
            log_max_mb: 10,
            notify: toml::Table::new(),
            ping: None,
            http: None,
            remote: Vec::new(),
            standby: None,
            locks: toml::Table::new(),
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        let c: Config = toml::from_str(text).map_err(|e| e.message().to_owned())?;
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
        for (name, n) in &c.locks {
            if !crate::calendar::valid_name(name) {
                return Err(format!("[locks] の {name:?} は lock の名前で書く（ジョブネットの lock と同じ。/ \\ : などは使えない）"));
            }
            if !n.as_integer().is_some_and(|n| n >= 1) {
                return Err(format!("[locks] の {name:?} は同時に持てる数で、1 以上（例: \"API枠\" = 3）"));
            }
        }
        Ok(c)
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
    }

    #[test]
    fn a_typo_is_an_error_not_a_default() {
        assert!(Config::parse("concurency = 2\n").is_err());
        assert!(Config::parse("concurrency = 0\n").is_err());
        assert!(Config::parse("keep_days = 0\n").is_err());
        assert!(Config::parse("log_max_mb = 0\n").is_err());
        assert!(Config::parse("notify = 1\n").is_err());
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
        ] {
            assert!(Config::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_notify_table_is_kept_as_written() {
        let c = Config::parse("[notify]\nsystem = [\"failed\"]\n").unwrap();
        assert_eq!(c.notify.get("system").and_then(|v| v.as_array()).map(Vec::len), Some(1));
        assert!(Config::default().notify.is_empty());
    }
}
