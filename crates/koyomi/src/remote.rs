//! Other PCs' engines (TODO 80, DECISIONS「複数の PC・Web UI・利用者とロール・HA」): each PC
//! runs its own engine, and this one calls another's `[http]` over HTTPS with its certificate
//! pinned by the fingerprint written in `[[remote]]` of koyomi.toml. No central server.
//!
//! - `koyomi http fingerprint`: this PC's fingerprint, to write in the other PC's `[[remote]]`.
//! - `koyomi remote peek <url>`: the fingerprint a PC shows, to compare with what it says there
//!   before trusting it (the only time a certificate is looked at unpinned).
//! - `koyomi remote [list]`, `status <name>`, `nets <name>`, `runs <name>`, `run <name> <net>`.
//! - The window switches to one of them with [`names`] and [`ask`] (`ui::far`).

use std::time::Duration;

use ito_http::client::{self, Client, Reply};
use koyomi_core::config::Remote;

use crate::engine::Paths;

const PEEK_WAIT: Duration = Duration::from_secs(10);

/// `koyomi http fingerprint`.
pub fn http_cmd(args: &[String]) -> i32 {
    match args {
        [op] if op == "fingerprint" => finish(crate::engine::http::fingerprint().map(|fp| {
            format!("{fp}\n（ほかの PC の koyomi.toml の [[remote]] に fingerprint = \"…\" で書く。[http] に lan = true が要る）")
        })),
        _ => usage("http takes fingerprint"),
    }
}

/// `koyomi remote …`.
pub fn cmd(args: &[String]) -> i32 {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let res = match a.as_slice() {
        [] | ["list"] => list(),
        ["peek", url] => peek(url),
        ["status", name] => with(name, status),
        ["nets", name] => with(name, nets),
        ["runs", name] => with(name, runs),
        ["run", name, net] => with(name, |r| run(r, net)),
        _ => return usage("remote takes list, peek <url>, status|nets|runs <name>, or run <name> <group/name>"),
    };
    finish(res)
}

fn usage(text: &str) -> i32 {
    crate::say(&format!("koyomi: {text}"), true);
    2
}

fn finish(res: Result<String, String>) -> i32 {
    match res {
        Ok(text) => {
            crate::say(&text, false);
            0
        }
        Err(e) => {
            crate::say(&format!("koyomi: {e}"), true);
            1
        }
    }
}

fn remotes() -> Result<Vec<Remote>, String> {
    Ok(Paths::find()?.config()?.remote)
}

/// The `[[remote]]` names, for the window's PC list. It reads koyomi.toml: not on the UI thread.
pub fn names() -> Result<Vec<String>, String> {
    Ok(remotes()?.iter().map(|r| r.name.trim().to_owned()).collect())
}

/// One call to the `[[remote]]` named `name`. It blocks: not on the UI thread.
pub fn ask(name: &str, f: impl FnOnce(&Client) -> Result<Reply, client::Error>) -> Result<serde_json::Value, String> {
    with(name, |r| call(r, f))
}

fn with<T>(name: &str, f: impl FnOnce(&Remote) -> Result<T, String>) -> Result<T, String> {
    let all = remotes()?;
    let r = all.iter().find(|r| r.name.trim() == name).ok_or_else(|| {
        let names: Vec<_> = all.iter().map(|r| r.name.trim()).collect();
        if names.is_empty() {
            format!("{name}: koyomi.toml に [[remote]] がない")
        } else {
            format!("{name}: [[remote]] にない（あるのは {}）", names.join("・"))
        }
    })?;
    f(r)
}

fn client(r: &Remote) -> Result<Client, String> {
    let name = r.secret();
    let token = ito_secret::get("koyomi", name)
        .map_err(|e| format!("{}: token の秘密 {name}: {e}", r.name))?
        .ok_or_else(|| format!("{}: token の秘密 {name} がない（koyomi secret set {name} に向こうの [http] の token を置く）", r.name))?;
    Client::new(r.url.trim(), r.fingerprint.trim(), token).map_err(|e| why(r, &e))
}

/// One call, with what went wrong in the person's words.
fn call(r: &Remote, f: impl FnOnce(&Client) -> Result<Reply, client::Error>) -> Result<serde_json::Value, String> {
    let reply = f(&client(r)?).map_err(|e| why(r, &e))?;
    match reply.status {
        200..=299 => Ok(reply.json),
        401 => Err(format!("{}: token が合わない（向こうの [http] の token と koyomi secret set {} を見る）", r.name, r.secret())),
        s => Err(format!("{}: {s} {}", r.name, reply.json["error"].as_str().unwrap_or_default())),
    }
}

fn why(r: &Remote, e: &client::Error) -> String {
    match e {
        client::Error::Fingerprint(got) => format!(
            "{}: 証明書が koyomi.toml の fingerprint と違う（今見えたのは {got}）。向こうで koyomi http fingerprint を見て、同じなら書き直す。違えば別の PC につながっている",
            r.name
        ),
        client::Error::Connect(s) => format!("{}: つながらない（{s}）。向こうのエンジンと [http] の lan = true、ファイアウォールを見る", r.name),
        e => format!("{}: {e}", r.name),
    }
}

fn list() -> Result<String, String> {
    let all = remotes()?;
    if all.is_empty() {
        return Ok("[[remote]] がない（koyomi.toml に name・url・fingerprint・token を書く）".to_owned());
    }
    let lines: Vec<String> = all
        .iter()
        .map(|r| match call(r, |c| c.get("/health")) {
            Ok(j) => format!("{}  {}  {}", r.name, r.url, j["status"].as_str().unwrap_or("答えた")),
            Err(e) => format!("{}  {}  ✕ {}", r.name, r.url, e.strip_prefix(&format!("{}: ", r.name)).unwrap_or(&e)),
        })
        .collect();
    Ok(lines.join("\n"))
}

fn peek(url: &str) -> Result<String, String> {
    let fp = client::peek_fingerprint(url, PEEK_WAIT).map_err(|e| format!("{url}: {e}"))?;
    Ok(format!("fingerprint = \"{fp}\"\n向こうの PC で koyomi http fingerprint が同じものを出すのを確かめてから、[[remote]] に書く"))
}

fn status(r: &Remote) -> Result<String, String> {
    let j = call(r, |c| c.get("/health"))?;
    Ok(format!("{}: {}（koyomi {}）", r.name, j["status"].as_str().unwrap_or_default(), j["version"].as_str().unwrap_or("?")))
}

fn nets(r: &Remote) -> Result<String, String> {
    let j = call(r, |c| c.get("/nets"))?;
    Ok(nets_text(&j))
}

fn nets_text(j: &serde_json::Value) -> String {
    let mut lines: Vec<String> = Vec::new();
    if j["paused"].as_bool() == Some(true) {
        lines.push("（止めている: koyomi resume で戻す）".to_owned());
    }
    for n in j["nets"].as_array().into_iter().flatten() {
        let held = if n["held"].as_bool() == Some(true) { "  保留" } else { "" };
        lines.push(format!(
            "{}  {}  {}{held}",
            n["id"].as_str().unwrap_or("?"),
            n["name"].as_str().unwrap_or(""),
            n["standing"].as_str().unwrap_or("")
        ));
    }
    if lines.is_empty() {
        lines.push("ネットがない".to_owned());
    }
    lines.join("\n")
}

fn runs(r: &Remote) -> Result<String, String> {
    let j = call(r, |c| c.get("/runs"))?;
    Ok(runs_text(&j))
}

fn runs_text(j: &serde_json::Value) -> String {
    let lines: Vec<String> = j["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let over = if r["over"].as_bool() == Some(true) { "" } else { "  動いている" };
            format!(
                "{}  {}  {}{over}",
                r["id"].as_str().unwrap_or("?"),
                r["jobnet"].as_str().unwrap_or(""),
                r["title"].as_str().unwrap_or("")
            )
        })
        .collect();
    if lines.is_empty() {
        "今日の回はない".to_owned()
    } else {
        lines.join("\n")
    }
}

fn run(r: &Remote, net: &str) -> Result<String, String> {
    let j = call(r, |c| c.post_json("/runs", &serde_json::json!({ "net": net })))?;
    Ok(format!("{}: {net} を流した（{}）", r.name, j["run"].as_str().unwrap_or("?")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the lists say paused, held and running in a line each, and say when there is nothing.
    #[test]
    fn the_lists_read_as_lines() {
        let j = serde_json::json!({ "paused": true, "nets": [
            { "id": "g/a", "name": "A", "standing": "confirmed", "held": false },
            { "id": "g/b", "name": "B", "standing": "unconfirmed", "held": true },
        ]});
        assert_eq!(nets_text(&j), "（止めている: koyomi resume で戻す）\ng/a  A  confirmed\ng/b  B  unconfirmed  保留");
        assert_eq!(nets_text(&serde_json::json!({ "nets": [] })), "ネットがない");
        let j = serde_json::json!({ "runs": [
            { "id": "1", "jobnet": "g/a", "title": "A", "over": true },
            { "id": "2", "jobnet": "g/b", "title": "B", "over": false },
        ]});
        assert_eq!(runs_text(&j), "1  g/a  A\n2  g/b  B  動いている");
        assert_eq!(runs_text(&serde_json::json!({ "runs": [] })), "今日の回はない");
    }
}
