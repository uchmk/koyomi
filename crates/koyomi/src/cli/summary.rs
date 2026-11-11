//! `koyomi config [<file>]`: what koyomi.toml sets, in a few lines.
//!
//! Read straight from the file; the engine need not run. It tells counts and names, never a value:
//! no webhook address, mail address, token or `[vars]` value, and nothing `notify::read` says (its
//! notes can carry an address). The notification places are listed from `[notify]` as written.

use std::collections::BTreeMap;

use koyomi_core::config::Config;
use koyomi_core::msg;
use serde::Serialize;

use super::{emit, fail, named, out, toml as want_toml, usage};

#[derive(Serialize)]
struct Http {
    port: u16,
    lan: bool,
    ipv6: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tailnet: Option<u16>,
    own_token: bool,
}

#[derive(Serialize)]
struct Users {
    count: usize,
    per_role: BTreeMap<String, usize>,
}

#[derive(Serialize)]
struct Remote {
    name: String,
    url: String,
}

#[derive(Serialize)]
struct Place {
    name: String,
    kind: &'static str,
}

#[derive(Serialize)]
struct Summary {
    file: String,
    exists: bool,
    concurrency: usize,
    keep_days: u32,
    archive: bool,
    archive_days: u32,
    log_max_mb: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    http: Option<Http>,
    users: Users,
    remote: Vec<Remote>,
    notify: Vec<Place>,
    #[serde(skip_serializing_if = "Option::is_none")]
    standby: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    primary: Option<String>,
    blackout: usize,
    vars: usize,
    groups: usize,
    locks: usize,
}

/// The notification places by name and kind, from `[notify]` as written (the legacy `webhook` is one too).
fn places(notify: &toml::Table) -> Vec<Place> {
    let mut v = Vec::new();
    if notify.get("webhook").and_then(toml::Value::as_str).is_some_and(|s| !s.trim().is_empty()) {
        v.push(Place { name: "webhook".to_owned(), kind: "webhook" });
    }
    for (key, kind) in [("webhooks", "webhook"), ("mails", "mail")] {
        let Some(list) = notify.get(key).and_then(toml::Value::as_array) else { continue };
        for (i, item) in list.iter().enumerate() {
            let name = item.get("name").and_then(toml::Value::as_str).map(str::trim).filter(|n| !n.is_empty());
            v.push(Place { name: name.map_or_else(|| format!("#{}", i + 1), str::to_owned), kind });
        }
    }
    v
}

fn summarize(config: &Config, file: &str, exists: bool) -> Summary {
    let mut per_role = BTreeMap::new();
    let users = config.http.as_ref().map(|h| h.user.as_slice()).unwrap_or_default();
    for u in users {
        *per_role.entry(u.role.trim().to_owned()).or_insert(0) += 1;
    }
    Summary {
        file: file.to_owned(),
        exists,
        concurrency: config.concurrency,
        keep_days: config.keep_days,
        archive: config.archive,
        archive_days: config.archive_days,
        log_max_mb: config.log_max_mb,
        http: config.http.as_ref().map(|h| Http {
            port: h.port,
            lan: h.lan,
            ipv6: h.ipv6,
            tailnet: h.tailnet,
            own_token: !h.token.trim().is_empty(),
        }),
        users: Users { count: users.len(), per_role },
        remote: config.remote.iter().map(|r| Remote { name: r.name.clone(), url: r.url.clone() }).collect(),
        notify: places(&config.notify),
        standby: config.standby.as_ref().map(|s| s.watch.clone()),
        primary: config.primary.as_ref().map(|p| p.standby.clone()),
        blackout: config.blackout.len(),
        vars: config.vars.len(),
        groups: config.groups.len(),
        locks: config.locks.len(),
    }
}

fn yes_no(b: bool) -> String {
    msg::note(if b { msg::N_CFG_YES } else { msg::N_CFG_NO }, &[])
}

/// The summary as the lines a person reads.
fn lines(s: &Summary) -> Vec<String> {
    let mut v = vec![msg::note(msg::N_CFG_FILE, &[&s.file])];
    if !s.exists {
        v.push(msg::note(msg::N_CFG_NO_FILE, &[&s.file]));
    }
    v.push(msg::note(msg::N_CFG_CONCURRENCY, &[&s.concurrency.to_string()]));
    v.push(msg::note(msg::N_CFG_KEEP, &[&s.keep_days.to_string(), &yes_no(s.archive), &s.archive_days.to_string()]));
    v.push(msg::note(msg::N_CFG_LOG_MAX, &[&s.log_max_mb.to_string()]));
    match &s.http {
        None => v.push(msg::note(msg::N_CFG_HTTP_NONE, &[])),
        Some(h) => {
            let reach = msg::note(if h.lan { msg::N_CFG_HTTP_LAN } else { msg::N_CFG_HTTP_LOOPBACK }, &[]);
            let tailnet = h.tailnet.map_or_else(|| msg::note(msg::N_CFG_NONE, &[]), |p| p.to_string());
            v.push(msg::note(msg::N_CFG_HTTP, &[&h.port.to_string(), &reach, &yes_no(h.ipv6), &tailnet, &yes_no(h.own_token)]));
        }
    }
    if s.users.count == 0 {
        v.push(msg::note(msg::N_CFG_USERS_NONE, &[]));
    } else {
        let per: Vec<String> = s.users.per_role.iter().map(|(r, n)| format!("{r} {n}")).collect();
        v.push(msg::note(msg::N_CFG_USERS, &[&s.users.count.to_string(), &per.join(", ")]));
    }
    if s.remote.is_empty() {
        v.push(msg::note(msg::N_CFG_REMOTE_NONE, &[]));
    }
    for r in &s.remote {
        v.push(msg::note(msg::N_CFG_REMOTE, &[&r.name, &r.url]));
    }
    let count = |kind: &str| s.notify.iter().filter(|p| p.kind == kind).count().to_string();
    v.push(msg::note(msg::N_CFG_NOTIFY, &[&s.notify.len().to_string(), &count("webhook"), &count("mail")]));
    for p in &s.notify {
        let kind = msg::note(if p.kind == "mail" { msg::N_CFG_NOTIFY_MAIL } else { msg::N_CFG_NOTIFY_WEBHOOK }, &[]);
        v.push(msg::note(msg::N_CFG_NOTIFY_PLACE, &[&p.name, &kind]));
    }
    if let Some(w) = &s.standby {
        v.push(msg::note(msg::N_CFG_STANDBY, &[w]));
    }
    if let Some(p) = &s.primary {
        v.push(msg::note(msg::N_CFG_PRIMARY, &[p]));
    }
    v.push(msg::note(msg::N_CFG_BLACKOUT, &[&s.blackout.to_string()]));
    v.push(msg::note(msg::N_CFG_COUNTS, &[&s.vars.to_string(), &s.groups.to_string(), &s.locks.to_string()]));
    v
}

/// The summary of `text` (the contents of koyomi.toml at `file`; `None`: there is no such file) in
/// words, and as TOML.
fn build(text: Option<&str>, file: &str) -> Result<(Vec<String>, String), String> {
    let config = match text {
        Some(t) => Config::parse(t).map_err(|e| named("koyomi.toml", &e))?,
        None => Config::default(),
    };
    let s = summarize(&config, file, text.is_some());
    let toml = toml::to_string(&s).map_err(|e| e.to_string())?;
    Ok((lines(&s), toml))
}

pub fn cmd(args: &[String]) -> i32 {
    let (path, explicit) = match args {
        [] => match crate::engine::Paths::find() {
            Ok(p) => (p.config, false),
            Err(e) => return fail(&e),
        },
        [file] if !file.starts_with('-') => (std::path::PathBuf::from(file), true),
        _ => return usage("config takes nothing or a koyomi.toml"),
    };
    let shown = path.display().to_string();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => None,
        Err(e) => return fail(&msg::line(msg::FILE_READ_FAILED, &[&shown, &e.to_string()])),
    };
    match build(text.as_deref(), &shown) {
        Ok((v, toml)) => {
            if want_toml() {
                emit(&toml);
            } else {
                out(&v.join("\n"), false);
            }
            0
        }
        Err(e) => {
            out(&e, true);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "\
concurrency = 3

[notify]
webhook = \"https://hooks.example.com/services/SECRET-WEBHOOK-PATH\"

[[notify.webhooks]]
name = \"chat\"
url = \"https://hooks.example.com/chat/SECRET-CHAT-PATH\"

[[notify.mails]]
name = \"ops\"
host = \"smtp.example.com\"
user = \"jobs\"
password = \"secret:mail-ops\"
from = \"jobs@example.com\"
to = [\"ops-secret-person@example.com\"]

[vars]
share = \"hunter2-dummy-secret-value\"

[http]
port = 8700
lan = true
token = \"secret:koyomi-http\"

[[http.user]]
name = \"boss\"
role = \"admin\"
token = \"secret:boss-token\"

[[http.user]]
name = \"ann\"
role = \"run\"
token = \"secret:ann-token\"
";

    #[test]
    fn the_summary_tells_counts_and_names_not_values() {
        let (v, toml) = build(Some(FILE), "koyomi.toml").unwrap();
        let plain = v.join("\n");
        for text in [&plain, &toml] {
            for hidden in [
                "SECRET-WEBHOOK-PATH",
                "SECRET-CHAT-PATH",
                "ops-secret-person",
                "jobs@example.com",
                "smtp.example.com",
                "hunter2",
                "boss-token",
                "ann-token",
                "koyomi-http",
                "mail-ops",
            ] {
                assert!(!text.contains(hidden), "{hidden} is in {text}");
            }
            assert!(text.contains("chat") && text.contains("ops"), "{text}");
        }
        assert!(plain.contains("8700"), "{plain}");
    }

    #[test]
    fn the_places_are_counted_by_kind() {
        let config = Config::parse(FILE).unwrap();
        let s = summarize(&config, "koyomi.toml", true);
        assert_eq!(s.notify.len(), 3);
        assert_eq!(s.notify.iter().filter(|p| p.kind == "mail").count(), 1);
        assert_eq!(s.users.count, 2);
        assert_eq!(s.users.per_role.get("admin"), Some(&1));
        assert_eq!(s.vars, 1);
    }

    #[test]
    fn no_file_is_the_defaults() {
        let (v, _) = build(None, "koyomi.toml").unwrap();
        assert!(v.len() >= 6);
        assert!(build(Some("concurrency = ["), "koyomi.toml").is_err());
    }
}
