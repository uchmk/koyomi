//! `[http]` of koyomi.toml: while the engine runs it listens on this PC's loopback
//! (ito-http), so another program can start a net and follow its run without the CLI.
//!
//! - `POST /runs` with `{"net": "<group>/<name>", "date"?, "from"?, "params"?}` is
//!   `koyomi run`: 202 and `{"run": "<id>"}`, or 422 and the engine's reason.
//! - `GET /runs/<id>` is the run as `koyomi status` / `history` know it, with `"over"`.
//!
//! Every request carries the bearer token kept as `secret:<name>` in the OS credential
//! store (read once when the listener starts); ito-http turns a missing or wrong one away
//! with 401 and never logs it. The requests go through the engine's own door, so they are
//! what the CLI may do and are written down as by-hand ops.

use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ito_http::{Response, Server};
use koyomi_core::config;
use koyomi_core::run::Run;
use serde::Deserialize;

use crate::proto::{self, Request, Runs};

/// How long after a failed start the listener is tried again.
const RETRY: Duration = Duration::from_secs(60);

pub struct Http {
    /// The `[http]` the listener is (or is being) started for.
    cfg: Option<config::Http>,
    server: Option<Server>,
    /// Why the last start failed.
    note: Option<String>,
    /// When a start was last asked for.
    tried: Option<Instant>,
    starting: bool,
    /// Which `[http]` a start was for; an answer for an older one is dropped.
    gen: u64,
    tx: mpsc::Sender<(u64, Result<Server, String>)>,
    rx: mpsc::Receiver<(u64, Result<Server, String>)>,
}

impl Default for Http {
    fn default() -> Http {
        let (tx, rx) = mpsc::channel();
        Http { cfg: None, server: None, note: None, tried: None, starting: false, gen: 0, tx, rx }
    }
}

impl Http {
    /// Starts, restarts or stops the listener to match `cfg`. The token is read and the
    /// port taken on a thread (the store can be slow); a failed start is tried again after
    /// [`RETRY`].
    pub fn tick(&mut self, cfg: Option<&config::Http>) {
        while let Ok((gen, r)) = self.rx.try_recv() {
            if gen != self.gen {
                continue;
            }
            self.starting = false;
            match r {
                Ok(s) => {
                    crate::say(&format!("koyomi: {}", listening(self.cfg.as_ref(), s.port())), false);
                    self.server = Some(s);
                    self.note = None;
                }
                Err(e) => self.note = Some(format!("koyomi.toml: [http] で待ち受けられない（{e}）。{} 秒ごとに試す", RETRY.as_secs())),
            }
        }
        if self.cfg.as_ref() != cfg {
            self.stop();
            self.cfg = cfg.cloned();
        }
        let Some(cfg) = &self.cfg else { return };
        if self.server.is_some() || self.starting || self.tried.is_some_and(|t| t.elapsed() < RETRY) {
            return;
        }
        self.tried = Some(Instant::now());
        self.starting = true;
        let (gen, tx, cfg) = (self.gen, self.tx.clone(), cfg.clone());
        let _ = std::thread::Builder::new().name("http".into()).spawn(move || {
            let _ = tx.send((gen, start(&cfg)));
        });
    }

    /// Stops listening (a changed `[http]`, or the engine ending).
    pub fn stop(&mut self) {
        self.gen += 1;
        self.starting = false;
        self.tried = None;
        self.note = None;
        if let Some(s) = self.server.take() {
            s.stop();
        }
    }

    /// What a person should see about `[http]`.
    pub fn notes(&self) -> Vec<String> {
        self.note.iter().cloned().collect()
    }

    /// The status line while it listens.
    pub fn line(&self) -> Option<String> {
        self.server.as_ref().map(|s| listening(self.cfg.as_ref(), s.port()))
    }
}

fn listening(cfg: Option<&config::Http>, port: u16) -> String {
    let v6 = if cfg.is_some_and(|c| c.ipv6) { format!("・[::1]:{port}") } else { String::new() };
    format!("HTTP: 127.0.0.1:{port}{v6} で待ち受けている")
}

fn start(cfg: &config::Http) -> Result<Server, String> {
    let name = cfg.secret();
    let token = ito_http::token_from_secret("koyomi", name).map_err(|e| format!("token の秘密 {name}: {e}（koyomi secret set {name}）"))?;
    let mut c = ito_http::Config::new(cfg.port, token);
    c.ipv6 = cfg.ipv6;
    Server::start(c, |req| handle(&req, |r| ito_ipc::door::ask(&proto::door(), r)), log).map_err(|e| format!("127.0.0.1:{}: {e}", cfg.port))
}

fn log(l: &ito_http::LogLine) {
    crate::say(&format!("http: {} {} {} {}ms", l.method, l.path, l.status, l.millis), false);
}

/// What `POST /runs` takes.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    net: String,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    params: BTreeMap<String, String>,
}

/// One request (the token already checked), asking the engine through `ask`.
fn handle(req: &ito_http::Request, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let path = req.path.trim_end_matches('/');
    let one = path.strip_prefix("/runs/").filter(|id| !id.is_empty() && !id.contains('/'));
    match (req.method.as_str(), path, one) {
        ("POST", "/runs", _) => start_run(req, ask),
        ("GET", _, Some(id)) => look(id, ask),
        (_, "/runs", _) => Response::error(405, "POST /runs で流す"),
        (_, _, Some(_)) => Response::error(405, "GET /runs/<id> で読む"),
        _ => Response::error(404, "POST /runs と GET /runs/<id> だけ"),
    }
}

fn start_run(req: &ito_http::Request, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let s = match req.json::<Start>() {
        Ok(Some(s)) if !s.net.trim().is_empty() => s,
        Ok(_) => return Response::error(400, "net が要る: {\"net\": \"<グループ>/<名前>\"}"),
        Err(e) => return Response::error(400, &format!("JSON を読めない: {e}")),
    };
    let r = Request::Run { id: s.net.trim().to_owned(), date: s.date, from: s.from, dry: false, params: s.params };
    match ask(&r) {
        Ok(id) => Response::new(202, serde_json::json!({ "run": id.trim() })),
        Err(e) => Response::error(422, &e),
    }
}

/// A run of today or still going, then one of the day its id says it started.
fn look(id: &str, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let Some(day) = started_on(id) else {
        return Response::error(404, &format!("実行の id は YYYYMMDD-HHMMSS-NN: {id}"));
    };
    let find = |req: &Request| -> Result<Option<Run>, String> { Ok(Runs::parse(&ask(req)?)?.runs.into_iter().find(|r| r.id == id)) };
    let found = find(&Request::Runs).and_then(|r| match r {
        Some(r) => Ok(Some(r)),
        None => find(&Request::History { from: day.clone(), to: day }),
    });
    match found {
        Ok(Some(run)) => {
            let over = run.is_over();
            let mut v = serde_json::to_value(&run).unwrap_or_default();
            if let Some(o) = v.as_object_mut() {
                o.insert("over".into(), over.into());
            }
            Response::ok(v)
        }
        Ok(None) => Response::error(404, &format!("その実行はない: {id}")),
        Err(e) => Response::error(500, &e),
    }
}

/// `YYYY-MM-DD` from a run id's `YYYYMMDD-…`.
fn started_on(id: &str) -> Option<String> {
    let d = id.get(..8).filter(|d| d.bytes().all(|b| b.is_ascii_digit()) && id[8..].starts_with('-'))?;
    chrono::NaiveDate::parse_from_str(d, "%Y%m%d").ok().map(|d| d.format("%Y-%m-%d").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::{Read, Write};

    fn req(method: &str, path: &str, body: &str) -> ito_http::Request {
        ito_http::Request {
            method: method.into(),
            path: path.into(),
            query: Vec::new(),
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    fn runs(ids: &[&str]) -> String {
        let text: String = ids
            .iter()
            .map(|id| {
                format!(
                    "[[run]]\nid = \"{id}\"\njobnet = \"g/a\"\ntitle = \"a\"\ndate = \"2026-10-31\"\nhow = \"manual\"\nstarted = \"2026-10-31T09:00:00\"\nended = \"2026-10-31T09:01:00\"\nstate = \"ok\"\njobs = []\n"
                )
            })
            .collect();
        Runs::parse(&text).expect("runs").to_toml()
    }

    #[test]
    fn a_post_starts_the_net_as_koyomi_run_does() {
        let asked = RefCell::new(Vec::new());
        let ask = |r: &Request| {
            asked.borrow_mut().push(format!("{r:?}"));
            match r {
                Request::Run { id, .. } if id == "g/a" => Ok("20261031-090000-01".to_owned()),
                _ => Err("定義がない".to_owned()),
            }
        };
        let r = handle(&req("POST", "/runs", r#"{"net": "g/a", "params": {"day": "1"}}"#), ask);
        assert_eq!((r.status, r.json["run"].as_str()), (202, Some("20261031-090000-01")));
        assert!(asked.borrow()[0].contains("dry: false") && asked.borrow()[0].contains("\"day\""), "{:?}", asked.borrow());
        assert_eq!(handle(&req("POST", "/runs/", r#"{"net": "g/x"}"#), ask).status, 422);
        assert_eq!(handle(&req("POST", "/runs", ""), ask).status, 400);
        assert_eq!(handle(&req("POST", "/runs", r#"{"net": "g/a", "dry": true}"#), ask).status, 400);
        assert_eq!(handle(&req("POST", "/runs", "{"), ask).status, 400);
        assert_eq!(handle(&req("GET", "/runs", ""), ask).status, 405);
        assert_eq!(handle(&req("DELETE", "/runs/20261031-090000-01", ""), ask).status, 405);
        assert_eq!(handle(&req("GET", "/defs", ""), ask).status, 404);
    }

    #[test]
    fn a_get_finds_the_run_today_then_on_the_day_its_id_names() {
        let asked = RefCell::new(Vec::new());
        let ask = |r: &Request| {
            asked.borrow_mut().push(format!("{r:?}"));
            Ok(match r {
                Request::Runs => runs(&["20261031-090000-01"]),
                Request::History { from, to } if from == "2026-10-30" && to == "2026-10-30" => runs(&["20261030-230000-01"]),
                _ => runs(&[]),
            })
        };
        let r = handle(&req("GET", "/runs/20261031-090000-01", ""), ask);
        assert_eq!((r.status, r.json["jobnet"].as_str(), r.json["over"].as_bool()), (200, Some("g/a"), Some(true)));
        assert_eq!(asked.borrow().len(), 1);
        assert_eq!(handle(&req("GET", "/runs/20261030-230000-01", ""), ask).status, 200);
        assert_eq!(handle(&req("GET", "/runs/20261029-230000-01", ""), ask).status, 404);
        assert_eq!(handle(&req("GET", "/runs/nope", ""), ask).status, 404);
        assert_eq!(handle(&req("GET", "/runs/20261399-000000-01", ""), ask).status, 404);
    }

    /// The listener as the engine starts it: the token first, then the handler.
    #[test]
    fn a_request_without_the_token_is_turned_away() {
        let ask = |_: &Request| Ok::<_, String>("20261031-090000-01".to_owned());
        let server = Server::start(ito_http::Config::new(0, "tok"), move |r| handle(&r, ask), |_| {}).expect("listen");
        let send = |auth: &str| {
            let mut c = std::net::TcpStream::connect(("127.0.0.1", server.port())).expect("connect");
            let body = r#"{"net":"g/a"}"#;
            write!(
                c,
                "POST /runs HTTP/1.1\r\nHost: x\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .expect("send");
            let mut out = String::new();
            let _ = c.read_to_string(&mut out);
            out
        };
        assert!(send("").starts_with("HTTP/1.1 401"), "{}", send(""));
        assert!(send("Authorization: Bearer nope\r\n").starts_with("HTTP/1.1 401"));
        let ok = send("Authorization: Bearer tok\r\n");
        assert!(ok.starts_with("HTTP/1.1 202") && ok.contains("20261031-090000-01"), "{ok}");
        server.stop();
    }

    #[test]
    fn the_day_comes_from_the_run_id() {
        assert_eq!(started_on("20261031-090000-01").as_deref(), Some("2026-10-31"));
        assert_eq!(started_on("2026103-090000-01"), None);
        assert_eq!(started_on("20261031"), None);
        assert_eq!(started_on("２０２６1031-0"), None);
    }
}
