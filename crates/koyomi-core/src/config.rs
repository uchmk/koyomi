//! `koyomi.toml` in the settings folder: how many jobs run at once, how long
//! history is kept, how big one job's log may grow, `[notify]` (read by
//! the app with ito-prefs's shape; kept whole here), `[ping]`, the
//! address the engine calls while it is alive, `[http]`, where it
//! listens for other programs (on this PC, or with `lan` on the network over
//! TLS), and `[[remote]]`, the other PCs' engines this one calls.
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
/// credential store as `token`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Http {
    pub port: u16,
    /// `secret:<name>`: never the token itself.
    pub token: String,
    #[serde(default)]
    pub ipv6: bool,
    /// Listen on the network, over TLS with a self-signed certificate (kept in `app_dir`).
    #[serde(default)]
    pub lan: bool,
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
        if self.secret().is_empty() {
            return Err("[http] の token は secret:<名前> で書く（値は koyomi secret set <名前> で OS の資格情報に置く）".to_owned());
        }
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Config {
        Config { concurrency: 4, keep_days: 90, log_max_mb: 10, notify: toml::Table::new(), ping: None, http: None, remote: Vec::new() }
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
        Ok(c)
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

    #[test]
    fn the_notify_table_is_kept_as_written() {
        let c = Config::parse("[notify]\nsystem = [\"failed\"]\n").unwrap();
        assert_eq!(c.notify.get("system").and_then(|v| v.as_array()).map(Vec::len), Some(1));
        assert!(Config::default().notify.is_empty());
    }
}
