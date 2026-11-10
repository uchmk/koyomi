//! Other PCs' engines (TODO 80, DECISIONS「複数の PC・Web UI・利用者とロール・HA」): each PC
//! runs its own engine, and this one calls another's `[http]` over HTTPS with its certificate
//! pinned by the fingerprint written in `[[remote]]` of koyomi.toml. No central server.
//!
//! - `koyomi http fingerprint`: this PC's fingerprint, to write in the other PC's `[[remote]]`.
//! - `koyomi remote peek <url>`: the fingerprint a PC shows, to compare with what it says there
//!   before trusting it (the only time a certificate is looked at unpinned).
//! - `koyomi remote [list]`, `status <name>`, `nets <name>`, `runs <name>`, `run <name> <net>`.
//! - The window switches to one of them with [`names`] and [`ask`] (`ui::far`).
//! - A net goes to another PC (TODO 111) with [`there`] (what it has now) and [`propose`]
//!   (`POST /nets/propose`, where it lands unconfirmed): the window's 別の PC に送る and
//!   `koyomi remote send <name> <net>`.

use std::path::PathBuf;
use std::time::Duration;

use ito_http::client::{self, Client, Reply};
use koyomi_core::config::Remote;
use koyomi_core::scan::{Def, Standing};

use crate::engine::Paths;
use crate::proto::Request;

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
        ["send", name, net] => send(name, &net.replace('\\', "/")),
        _ => return usage("remote takes list, peek <url>, status|nets|runs <name>, run <name> <group/name>, or send <name> <group/name>"),
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

/// `s` for a path segment or a query value: anything but a letter, a digit and `-._~` as `%XX`.
pub fn query(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn remotes() -> Result<Vec<Remote>, String> {
    Ok(Paths::find()?.config()?.remote)
}

/// Why a name cannot be a new `[[remote]]`'s.
#[derive(Debug, PartialEq, Eq)]
pub enum NameWhy {
    Empty,
    /// A `[[remote]]` has it already.
    Taken,
    /// Not usable as part of the secret's name `<name>-token`, or as a `[[remote]]` name.
    Bad,
}

/// Whether `name` may be a new `[[remote]]`'s: not empty, not one of `have`, and made of what a secret's name
/// and a `[[remote]]` name may hold (no space, no path separator, no control character).
pub fn check_name(name: &str, have: &[String]) -> Result<(), NameWhy> {
    let name = name.trim();
    if name.is_empty() {
        Err(NameWhy::Empty)
    } else if name.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\' | '"' | '\'' | ':')) {
        Err(NameWhy::Bad)
    } else if have.iter().any(|h| h.trim().eq_ignore_ascii_case(name)) {
        Err(NameWhy::Taken)
    } else {
        Ok(())
    }
}

/// `text` (koyomi.toml) with `new` added to its `[[remote]]` after every one of `remotes` (those already read
/// from it, so none is lost). Comments and the other tables stay. The result must read as a koyomi.toml.
/// It refuses a name the file has already (the box's own list may be old) and a secret another `[[remote]]` uses
/// (writing the token there would overwrite theirs).
pub fn with_remote(text: &str, remotes: &[Remote], new: &Remote) -> Result<String, String> {
    let have: Vec<String> = remotes.iter().map(|r| r.name.clone()).collect();
    match check_name(&new.name, &have) {
        Ok(()) => {}
        Err(NameWhy::Taken) => return Err(format!("{} はもう koyomi.toml にある", new.name.trim())),
        Err(_) => return Err(format!("{:?} はつなぎ先の名前に使えない", new.name.trim())),
    }
    let secret = new.secret();
    if !secret.is_empty() && remotes.iter().any(|r| r.secret().eq_ignore_ascii_case(secret)) {
        return Err(format!("{secret} はほかのつなぎ先が使っている"));
    }
    let quote = |s: &str| toml::Value::String(s.to_owned()).to_string();
    let block = |r: &Remote| -> Vec<(&'static str, String)> {
        vec![("name", quote(&r.name)), ("url", quote(&r.url)), ("fingerprint", quote(&r.fingerprint)), ("token", quote(&r.token))]
    };
    let blocks: Vec<Vec<(&str, String)>> = remotes.iter().chain(std::iter::once(new)).map(block).collect();
    let out = ito_common::set_tables(text, "remote", &blocks)?;
    koyomi_core::config::Config::parse(&out)?;
    Ok(out)
}

/// Where this PC's koyomi.toml is and what it would hold with `new` added. It writes nothing, and it blocks:
/// not on the UI thread.
pub fn prepare_add(new: &Remote) -> Result<(PathBuf, String), String> {
    let paths = Paths::find()?;
    let path = paths.config.clone();
    let text = ito_common::read_or_empty(&path)?;
    let remotes = koyomi_core::config::Config::parse(&text)?.remote;
    let out = with_remote(&text, &remotes, new)?;
    Ok((path, out))
}

/// The `[[remote]]` names, for the window's PC list. It reads koyomi.toml: not on the UI thread.
pub fn names() -> Result<Vec<String>, String> {
    Ok(remotes()?.iter().map(|r| r.name.trim().to_owned()).collect())
}

/// One call to the `[[remote]]` named `name`. It blocks: not on the UI thread.
pub fn ask(name: &str, f: impl FnOnce(&Client) -> Result<Reply, client::Error>) -> Result<serde_json::Value, String> {
    with(name, |r| call(r, f))
}

pub(crate) fn with<T>(name: &str, f: impl FnOnce(&Remote) -> Result<T, String>) -> Result<T, String> {
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

pub(crate) fn client(r: &Remote) -> Result<Client, String> {
    let name = r.secret();
    let token = ito_secret::get("koyomi", name)
        .map_err(|e| format!("{}: token の秘密 {name}: {e}", r.name))?
        .ok_or_else(|| format!("{}: token の秘密 {name} がない（koyomi secret set {name} に向こうの [http] の token を置く）", r.name))?;
    Client::new(r.url.trim(), r.fingerprint.trim(), token).map_err(|e| why(r, &e))
}

/// One call, with what went wrong in the person's words.
fn call(r: &Remote, f: impl FnOnce(&Client) -> Result<Reply, client::Error>) -> Result<serde_json::Value, String> {
    let reply = f(&client(r)?).map_err(|e| why(r, &e))?;
    answer(r, reply)
}

/// A reply's JSON when it is a yes, else what went wrong in the person's words.
fn answer(r: &Remote, reply: Reply) -> Result<serde_json::Value, String> {
    match reply.status {
        200..=299 => Ok(reply.json),
        401 => Err(format!("{}: token が合わない（向こうの [http] の token と koyomi secret set {} を見る）", r.name, r.secret())),
        s => Err(format!("{}: {s} {}", r.name, reply.json["error"].as_str().unwrap_or_default())),
    }
}

pub(crate) fn why(r: &Remote, e: &client::Error) -> String {
    match e {
        client::Error::Fingerprint(got) => format!(
            "{}: 証明書が koyomi.toml の fingerprint と違う（今見えたのは {got}）。向こうで koyomi http fingerprint を見て、同じなら書き直す。違えば別の PC につながっている",
            r.name
        ),
        client::Error::Connect(s) => format!("{}: つながらない（{s}）。向こうのエンジンと [http] の lan = true、ファイアウォールを見る", r.name),
        client::Error::Deadline => format!("{}: 時間内に終わらなかった（通信が遅いか、向こうが混んでいる）", r.name),
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

/// The fingerprint the PC at `url` shows, or why not in the person's words. It blocks: not on the UI thread.
pub(crate) fn peek_fingerprint(url: &str) -> Result<String, String> {
    client::peek_fingerprint(url.trim(), PEEK_WAIT).map_err(|e| format!("{url}: {}", peek_why(&e)))
}

fn peek(url: &str) -> Result<String, String> {
    let fp = peek_fingerprint(url)?;
    Ok(format!("fingerprint = \"{fp}\"\n向こうの PC で koyomi http fingerprint が同じものを出すのを確かめてから、[[remote]] に書く"))
}

/// What `peek` could not do, in Japanese (ito-http's own texts are English).
pub(crate) fn peek_why(e: &client::Error) -> String {
    match e {
        client::Error::Url(s) => format!("アドレスが読めない: {s}（https://ホスト[:ポート] で書く）"),
        client::Error::Connect(s) => {
            format!("つながらない（{s}）。向こうのエンジンと [http] の lan = true、ポート、ファイアウォールを見る")
        }
        client::Error::Tls(s) => format!("TLS が通らない: {s}"),
        client::Error::Fingerprint(got) => format!("証明書が信頼したものと違う（{got}）"),
        client::Error::Bad(s) => format!("答えが読めない: {s}"),
        client::Error::Deadline => "時間内に終わらなかった".to_owned(),
    }
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

/// What `GET /nets/text` says of a net on another PC (TODO 111).
#[derive(Debug, Clone, PartialEq)]
pub struct There {
    pub text: String,
    /// Sent back as `base`: a file changed there since it was read is not overwritten.
    pub sha256: String,
    /// `None` when that PC does not say.
    pub standing: Option<Standing>,
}

impl There {
    fn from_json(j: &serde_json::Value) -> There {
        There {
            text: j["text"].as_str().unwrap_or_default().to_owned(),
            sha256: j["sha256"].as_str().unwrap_or_default().to_owned(),
            standing: serde_json::from_value(j["standing"].clone()).ok(),
        }
    }
}

/// The net `net` as the `[[remote]]` named `name` has it, `None` when it has no such net.
/// That PC's token needs the `edit` role. It blocks: not on the UI thread.
pub fn there(name: &str, net: &str) -> Result<Option<There>, String> {
    with(name, |r| {
        let reply = client(r)?.get(&format!("/nets/text?net={}", query(net))).map_err(|e| why(r, &e))?;
        if reply.status == 404 {
            return Ok(None);
        }
        answer(r, reply).map(|j| Some(There::from_json(&j)))
    })
}

/// Sends `text` as the net `net` to the `[[remote]]` named `name` (`POST /nets/propose`), where it
/// lands unconfirmed until that PC's window confirms it. `base` is the hash [`there`] read, `None`
/// for a net that PC does not have. `Ok(true)` when the text is the one already confirmed there.
/// It blocks: not on the UI thread.
pub fn propose(name: &str, net: &str, text: &str, base: Option<&str>) -> Result<bool, String> {
    with(name, |r| {
        let mut body = serde_json::json!({ "net": net, "text": text });
        if let Some(b) = base {
            body["base"] = b.into();
        }
        let reply = client(r)?.post_json("/nets/propose", &body).map_err(|e| why(r, &e))?;
        if reply.status == 409 {
            return Err(format!("{}: 読んだあとで向こうの {net} が変わった（読み直してから送る）", r.name));
        }
        answer(r, reply).map(|j| j["confirmed"].as_bool() == Some(true))
    })
}

/// `koyomi remote send <name> <group/name>`: the change from what that PC has to this PC's net,
/// then `y` sends it.
fn send(name: &str, id: &str) -> Result<String, String> {
    let def: Def = crate::client::call(&Request::Show { id: id.to_owned() }).and_then(|s| toml::from_str(&s).map_err(|e| e.to_string()))?;
    if def.standing == Standing::Invalid {
        return Err(format!("{id} に定義の誤りがある。直してから送る（koyomi show {id}）"));
    }
    let there = there(name, id)?;
    if there.as_ref().is_some_and(|t| t.text == def.text) {
        return Ok(format!("{name} の {id} はこの PC と同じ中身（送らなかった）"));
    }
    let head = match &there {
        Some(_) => format!("{name} に {id} を送る（- 向こうの今 / + この PC）:"),
        None => format!("{name} には {id} がない。新しく置く:"),
    };
    crate::say(&head, false);
    let old = there.as_ref().map_or("", |t| t.text.as_str());
    for l in crate::review::diff_text(&ito_diff::diff(old, &def.text, 3)) {
        crate::say(&l, false);
    }
    if def.standing != Standing::Confirmed {
        crate::say("（この PC でもまだ未確認の定義）", false);
    }
    crate::say("送ると向こうでは未確認で置かれ、向こうの窓で確認するまで流れない。送るなら y を入力（それ以外は送らない）:", false);
    let mut typed = String::new();
    let _ = std::io::stdin().read_line(&mut typed);
    if !typed.trim().eq_ignore_ascii_case("y") {
        return Err(format!("{id}: 送らなかった"));
    }
    let same = propose(name, id, &def.text, there.as_ref().map(|t| t.sha256.as_str()))?;
    Ok(sent_line(name, id, same))
}

/// What a send says once that PC took it.
fn sent_line(name: &str, net: &str, same: bool) -> String {
    if same {
        format!("{name} に送った: {net}（向こうの確認済みと同じ中身。そのまま流れる）")
    } else {
        format!("{name} に送った: {net}（向こうでは未確認。{name} の窓で確認するまで流れない）")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn peek_errors_are_japanese() {
        let t = super::peek_why(&ito_http::client::Error::Connect("os error 10061".into()));
        assert!(t.starts_with("つながらない") && !t.contains("cannot reach"), "{t}");
    }

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

    /// What another PC has of a net is read from `GET /nets/text`; one that does not say its
    /// standing reads as none. A send says whether it waits for that PC's window.
    #[test]
    fn a_net_there_is_read_and_a_send_is_said() {
        let j = serde_json::json!({ "net": "g/a", "text": "format = 1\n", "sha256": "abc", "standing": "unconfirmed" });
        let want = There { text: "format = 1\n".into(), sha256: "abc".into(), standing: Some(Standing::Unconfirmed) };
        assert_eq!(There::from_json(&j), want);
        assert_eq!(There::from_json(&serde_json::json!({ "text": "x", "sha256": "s" })).standing, None);
        assert_eq!(sent_line("本番", "g/a", false), "本番 に送った: g/a（向こうでは未確認。本番 の窓で確認するまで流れない）");
        assert_eq!(sent_line("本番", "g/a", true), "本番 に送った: g/a（向こうの確認済みと同じ中身。そのまま流れる）");
    }

    fn remote_of(name: &str) -> Remote {
        Remote {
            name: name.into(),
            url: format!("https://{name}:8443"),
            fingerprint: "ab".repeat(32),
            token: format!("secret:{name}-token"),
        }
    }

    #[test]
    fn with_remote_adds_one_and_keeps_comments_other_tables_and_every_existing_remote() {
        let text = "# my settings\nkeep_days = 30 # keep\n\n[vars]\nplace = \"x\"\n\n# the factory\n[[remote]]\nname = \"fact\"\nurl = \"https://fact:8443\"\nfingerprint = \"".to_owned()
            + &"cd".repeat(32)
            + "\"\ntoken = \"secret:fact-token\"\n\n[[remote]]\nname = \"office\"\nurl = \"https://office:8443\"\nfingerprint = \""
            + &"ef".repeat(32)
            + "\"\ntoken = \"secret:office-token\"\n";
        let have = koyomi_core::config::Config::parse(&text).unwrap().remote;
        assert_eq!(have.len(), 2);
        let out = with_remote(&text, &have, &remote_of("new")).unwrap();
        for kept in ["# my settings", "keep_days = 30 # keep", "[vars]", "place = \"x\"", "# the factory"] {
            assert!(out.contains(kept), "{kept} lost:\n{out}");
        }
        let now = koyomi_core::config::Config::parse(&out).unwrap().remote;
        let names: Vec<&str> = now.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["fact", "office", "new"]);
        assert_eq!(now[0].fingerprint, "cd".repeat(32));
        assert_eq!(now[2].token, "secret:new-token");
        assert_eq!(now[2].url, "https://new:8443");
    }

    #[test]
    fn with_remote_works_on_an_empty_text() {
        let out = with_remote("", &[], &remote_of("new")).unwrap();
        let now = koyomi_core::config::Config::parse(&out).unwrap().remote;
        assert_eq!(now.len(), 1);
        assert_eq!(now[0].name, "new");
    }

    #[test]
    fn with_remote_refuses_a_name_the_file_has_in_any_case_and_a_secret_another_remote_uses() {
        let have = vec![remote_of("Fact")];
        let text = with_remote("", &[], &have[0]).unwrap();
        let e = with_remote(&text, &have, &remote_of("fact")).unwrap_err();
        assert!(e.contains("fact"), "{e}");
        assert!(with_remote(&text, &have, &remote_of("FACT")).is_err());
        let mut other = remote_of("new");
        other.token = "secret:FACT-token".into();
        let e = with_remote(&text, &have, &other).unwrap_err();
        assert_eq!(e, "FACT-token はほかのつなぎ先が使っている");
        // Nothing of either was written, and a fresh name and secret still go in.
        assert!(with_remote(&text, &have, &remote_of("new")).is_ok());
    }

    #[test]
    fn a_remote_that_does_not_read_back_is_not_written() {
        let mut bad = remote_of("new");
        bad.fingerprint = "short".into();
        assert!(with_remote("", &[], &bad).is_err());
    }

    #[test]
    fn a_remote_name_is_checked_for_empty_taken_and_secret_name_characters() {
        let have = vec!["Fact".to_owned()];
        assert_eq!(check_name("", &have), Err(NameWhy::Empty));
        assert_eq!(check_name("  ", &have), Err(NameWhy::Empty));
        assert_eq!(check_name("fact", &have), Err(NameWhy::Taken));
        assert_eq!(check_name(" Fact ", &have), Err(NameWhy::Taken));
        for bad in ["a b", "a/b", "a\\b", "a\"b", "a'b", "a:b", "a\tb", "a\nb"] {
            assert_eq!(check_name(bad, &have), Err(NameWhy::Bad), "{bad:?}");
        }
        assert_eq!(check_name("office-2", &have), Ok(()));
        assert_eq!(check_name("工場", &have), Ok(()));
    }
}
