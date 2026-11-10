//! Which engine the window talks to: this PC's (over the door) or another PC's (over `POST /door`).
//!
//! [`Link::Local`] is what the window always did: it starts the engine when it is down. [`Link::Far`]
//! never starts anything: it reads the `[[remote]]` of that name, checks with `GET /me` that the token may
//! drive a window, and then sends each request as `POST /door`.

use std::time::Duration;

use ito_http::client::{self, Client, Reply};
use koyomi_core::config::Remote;
use koyomi_core::role::Role;

use crate::proto::{self, Request};
use crate::{client as local, remote};

/// How often the engine of this PC is asked for the list and the runs.
const POLL_LOCAL: Duration = Duration::from_secs(1);
/// How often another PC is asked: a request costs a TLS handshake there.
const POLL_FAR: Duration = Duration::from_secs(3);

/// The words for a token the window cannot be driven with.
const NOT_ADMITTED: &str = "この token では窓をつなげない（edit 以上で、絞り込みのない利用者が要る）";

/// Which engine a window talks to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// The engine of this PC, started when it is down.
    Local,
    /// The engine of the `[[remote]]` of this name.
    Far(String),
}

/// A link that has started: what it needs to send requests.
pub struct Conn {
    kind: Kind,
    role: Option<Role>,
}

enum Kind {
    Local,
    Far { remote: Remote, client: Client },
}

impl Link {
    /// The `[[remote]]` name, `None` for this PC.
    pub fn far(&self) -> Option<&str> {
        match self {
            Link::Local => None,
            Link::Far(name) => Some(name),
        }
    }

    pub fn is_far(&self) -> bool {
        matches!(self, Link::Far(_))
    }

    /// How often the list is asked for.
    pub fn poll(&self) -> Duration {
        match self {
            Link::Local => POLL_LOCAL,
            Link::Far(_) => POLL_FAR,
        }
    }

    /// Connects. Another PC's engine is never started from here; a name that is not a `[[remote]]` is an
    /// error and does not fall back to this PC.
    pub fn start(&self) -> Result<Conn, String> {
        match self {
            Link::Local => local::start().map(|()| Conn { kind: Kind::Local, role: None }),
            Link::Far(name) => remote::with(name, connect),
        }
    }
}

/// Connects to a `[[remote]]`: the client, then `GET /me`, which says whether this token may drive a window.
/// It never starts an engine.
fn connect(r: &Remote) -> Result<Conn, String> {
    let client = build(r)?;
    let reply = client.get("/me").map_err(|e| remote::why(r, &e))?;
    let role = match reply.status {
        200..=299 => admit(&reply.json)?,
        s => return Err(read_status(r, s, &reply.json)),
    };
    Ok(Conn { kind: Kind::Far { remote: r.clone(), client }, role: Some(role) })
}

/// The one place a client for another PC is made.
fn build(r: &Remote) -> Result<Client, String> {
    remote::client(r)
}

impl Conn {
    /// The caller's role over there; `None` for this PC.
    pub fn role(&self) -> Option<Role> {
        self.role
    }

    /// Whether the engine still answers. Another PC counts as up once started: a failed call says otherwise.
    pub fn running(&self) -> bool {
        match self.kind {
            Kind::Local => local::running(),
            Kind::Far { .. } => true,
        }
    }

    /// One request. It blocks: not on the UI thread.
    pub fn call(&self, request: &Request) -> Result<String, String> {
        match &self.kind {
            Kind::Local => local::call(request),
            Kind::Far { remote, client } => {
                let body = serde_json::json!({ "proto": proto::PROTO, "request": request });
                match client.post_json("/door", &body) {
                    Ok(reply) => read_reply(remote, request, &reply),
                    Err(e) => Err(unsure(request, &e, remote::why(remote, &e))),
                }
            }
        }
    }
}

/// The role of a `GET /me` answer when a window may be driven with it: `edit` or `admin`, not narrowed.
pub fn admit(me: &serde_json::Value) -> Result<Role, String> {
    let role = me["role"].as_str().and_then(|s| Role::parse(s).ok());
    match role {
        Some(r) if r >= Role::Edit && me["nets"].is_null() && me["roles"].is_null() => Ok(r),
        _ => Err(NOT_ADMITTED.to_owned()),
    }
}

/// What a `/door` answer says, in the person's words: the engine's own text for a yes, its own error for a no.
pub fn read_reply(r: &Remote, request: &Request, reply: &Reply) -> Result<String, String> {
    match reply.status {
        200 => {
            let j = &reply.json;
            match (j["ok"].as_bool(), j["text"].as_str(), j["error"].as_str()) {
                (Some(true), Some(text), _) => Ok(text.to_owned()),
                (Some(false), _, Some(error)) if !error.is_empty() => Err(error.to_owned()),
                // A 200 that says neither yes nor no is not a yes: nothing is confirmed.
                _ => Err(may_have_arrived(request, format!("{}: 答えが読めない", r.name))),
            }
        }
        s => Err(read_status(r, s, &reply.json)),
    }
}

/// `text` with the warning that a request which is not a read may have arrived. It is never sent again on its own.
fn may_have_arrived(request: &Request, text: String) -> String {
    if request.is_read() {
        text
    } else {
        format!("{text}。届いたか分からない。履歴を確かめてから、必要ならもう一度")
    }
}

fn read_status(r: &Remote, status: u16, json: &serde_json::Value) -> String {
    let error = json["error"].as_str().unwrap_or_default();
    match status {
        401 => format!("{}: token が合わない（向こうの [http] の token と koyomi secret set {} を見る）", r.name, r.secret()),
        403 | 409 => error.to_owned(),
        404 => format!("{}: 向こうのエンジンの版が古い（/door が無い）", r.name),
        s => format!("{s} {error}"),
    }
}

/// `text` with the warning that a request which is not a read may have arrived (a connection that broke or
/// timed out, or an answer that did not read). It is never sent again on its own.
fn unsure(request: &Request, e: &client::Error, text: String) -> String {
    if matches!(e, client::Error::Connect(_) | client::Error::Bad(_)) {
        may_have_arrived(request, text)
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn remote() -> Remote {
        Remote { name: "kei".into(), url: "https://h:1".into(), fingerprint: "ab".into(), token: "secret:kei-token".into() }
    }

    fn reply(status: u16, json: serde_json::Value) -> Reply {
        Reply { status, json }
    }

    #[test]
    fn far_never_starts_the_engine() {
        let before = local::STARTS.with(std::cell::Cell::get);
        let r = Link::Far("no-such-pc-for-this-test".into()).start();
        assert!(r.is_err());
        // A remote that is found but cannot be reached (no such secret, nothing listening) is an error too.
        let found = Remote {
            name: "kei".into(),
            url: "https://127.0.0.1:1".into(),
            fingerprint: "ab".repeat(32),
            token: "secret:no-such-secret-for-this-test".into(),
        };
        assert!(connect(&found).is_err());
        assert_eq!(local::STARTS.with(std::cell::Cell::get), before);
    }

    #[test]
    fn a_200_that_is_not_a_clear_yes_or_no_is_an_error() {
        let r = remote();
        let save = Request::Save { id: "g/a".into(), text: "x".into(), base_sha256: None, force: false };
        for body in [json!({"ok": true}), json!({"ok": false}), json!({"ok": false, "error": ""}), json!(null)] {
            let e = read_reply(&r, &save, &reply(200, body.clone())).unwrap_err();
            assert!(e.starts_with("kei: 答えが読めない") && e.contains("届いたか分からない"), "{body}: {e}");
            let e = read_reply(&r, &Request::List, &reply(200, body.clone())).unwrap_err();
            assert_eq!(e, "kei: 答えが読めない", "{body}");
        }
    }

    #[test]
    fn an_unknown_name_is_an_error_not_this_pc() {
        let e = Link::Far("no-such-pc-for-this-test".into()).start().err().expect("an error");
        assert!(e.contains("no-such-pc-for-this-test"), "{e}");
    }

    #[test]
    fn the_polls_differ() {
        assert_eq!(Link::Local.poll(), Duration::from_secs(1));
        assert_eq!(Link::Far("x".into()).poll(), Duration::from_secs(3));
    }

    #[test]
    fn a_yes_is_the_engines_text() {
        assert_eq!(
            read_reply(&remote(), &Request::List, &reply(200, json!({"ok": true, "text": "保存した: g/a"}))),
            Ok("保存した: g/a".to_owned())
        );
    }

    #[test]
    fn a_no_is_the_engines_error_verbatim() {
        let no = reply(200, json!({"ok": false, "error": "ほかで変わっています: g/a"}));
        assert_eq!(read_reply(&remote(), &Request::List, &no), Err("ほかで変わっています: g/a".to_owned()));
    }

    #[test]
    fn the_statuses_have_their_words() {
        let r = remote();
        let e = |s, j| read_reply(&r, &Request::List, &reply(s, j)).unwrap_err();
        assert!(e(401, json!({})).contains("token が合わない"));
        assert!(e(401, json!({})).contains("kei-token"));
        assert_eq!(e(403, json!({"error": "kei は view のロール。これには edit が要る"})), "kei は view のロール。これには edit が要る");
        assert!(e(404, json!({})).contains("/door が無い"));
        assert_eq!(
            e(409, json!({"error": "窓とエンジンの版が違う（窓 45、エンジン 44）"})),
            "窓とエンジンの版が違う（窓 45、エンジン 44）"
        );
        assert_eq!(e(500, json!({"error": "x"})), "500 x");
    }

    #[test]
    fn me_is_admitted_for_edit_and_admin_only_and_not_narrowed() {
        let me =
            |role: &str, nets: serde_json::Value, roles: serde_json::Value| json!({"who": "u", "role": role, "nets": nets, "roles": roles});
        assert_eq!(admit(&me("edit", json!(null), json!(null))), Ok(Role::Edit));
        assert_eq!(admit(&me("admin", json!(null), json!(null))), Ok(Role::Admin));
        assert_eq!(admit(&me("view", json!(null), json!(null))), Err(NOT_ADMITTED.to_owned()));
        assert_eq!(admit(&me("run", json!(null), json!(null))), Err(NOT_ADMITTED.to_owned()));
        assert_eq!(admit(&me("admin", json!(["a/*"]), json!(null))), Err(NOT_ADMITTED.to_owned()));
        assert_eq!(admit(&me("edit", json!(null), json!({"a/*": "view"}))), Err(NOT_ADMITTED.to_owned()));
    }

    #[test]
    fn only_a_request_that_is_not_a_read_is_called_unsure() {
        let e = client::Error::Connect("timed out".into());
        let save = Request::Save { id: "g/a".into(), text: "x".into(), base_sha256: None, force: false };
        assert!(unsure(&save, &e, "kei: x".into()).contains("届いたか分からない"));
        assert_eq!(unsure(&Request::List, &e, "kei: x".into()), "kei: x");
        for e in [client::Error::Fingerprint("f".into()), client::Error::Url("u".into()), client::Error::Tls("t".into())] {
            assert_eq!(unsure(&save, &e, "kei: x".into()), "kei: x");
        }
    }
}
