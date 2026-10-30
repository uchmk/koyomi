//! `[http]` of koyomi.toml: while the engine runs it listens on this PC's loopback
//! (ito-http), so another program can start a net and follow its run without the CLI.
//! With `lan = true` it listens on the network instead, over TLS only, with a self-signed
//! certificate kept in `app_dir`'s `tls` folder; another PC pins its fingerprint
//! (`koyomi http fingerprint`, TODO 80).
//!
//! - `POST /runs` with `{"net": "<group>/<name>", "date"?, "from"?, "params"?}` is
//!   `koyomi run`: 202 and `{"run": "<id>"}`, or 422 and the engine's reason.
//! - `GET /runs/<id>` is the run as `koyomi status` / `history` know it, with `"over"`.
//! - Read only, for another PC's window, the Web UI and an HA standby (TODO 80, 81, 87):
//!   `GET /health` (the engine's status line), `GET /nets` (each definition's id, name,
//!   standing and hold), `GET /runs` (today's and the running ones; `?from=&to=` for days)
//!   and `GET /runs/<id>/log?job=<job>` (the tail of a job's log, secrets already `***`).
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

use crate::proto::{self, Request, Runs, Snapshot};

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
                    crate::say(&format!("koyomi: {}", listening(self.cfg.as_ref(), &s)), false);
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
        self.server.as_ref().map(|s| listening(self.cfg.as_ref(), s))
    }
}

fn listening(cfg: Option<&config::Http>, s: &Server) -> String {
    let port = s.port();
    if let Some(fp) = s.fingerprint() {
        return format!("HTTP: LAN の :{port} で TLS で待ち受けている（指紋 {}）", ito_http::tls::format(&fp));
    }
    let v6 = if cfg.is_some_and(|c| c.ipv6) { format!("・[::1]:{port}") } else { String::new() };
    format!("HTTP: 127.0.0.1:{port}{v6} で待ち受けている")
}

/// Where the LAN listener's certificate and key are kept: `app_dir`'s `tls`, beside the
/// person's other secrets.
pub fn cert_dir() -> Option<std::path::PathBuf> {
    ito_common::app_dir("koyomi").map(|d| d.join("tls"))
}

/// `koyomi http fingerprint`: this PC's certificate's fingerprint, made if there is none yet.
/// What another PC writes in its `[[remote]]`.
pub fn fingerprint() -> Result<String, String> {
    let dir = cert_dir().ok_or("設定のフォルダーが分からない")?;
    ito_http::tls::fingerprint(&dir).map(|fp| ito_http::tls::format(&fp)).map_err(|e| format!("{}: {e}", dir.display()))
}

fn start(cfg: &config::Http) -> Result<Server, String> {
    let name = cfg.secret();
    let token = ito_http::token_from_secret("koyomi", name).map_err(|e| format!("token の秘密 {name}: {e}（koyomi secret set {name}）"))?;
    let mut c = ito_http::Config::new(cfg.port, token);
    c.ipv6 = cfg.ipv6;
    let at = if cfg.lan {
        let dir = cert_dir().ok_or("設定のフォルダーが分からない")?;
        c.bind = ito_http::Bind::Lan { addr: None, tls: ito_http::TlsCfg::new(dir) };
        format!("LAN :{}", cfg.port)
    } else {
        format!("127.0.0.1:{}", cfg.port)
    };
    Server::start(c, |req| handle(&req, |r| ito_ipc::door::ask(&proto::door(), r)), log).map_err(|e| format!("{at}: {e}"))
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
    let log = path.strip_prefix("/runs/").and_then(|p| p.strip_suffix("/log")).filter(|id| !id.is_empty() && !id.contains('/'));
    match (req.method.as_str(), path, one, log) {
        ("POST", "/runs", ..) => start_run(req, ask),
        ("GET", "/runs", ..) => list_runs(req, ask),
        ("GET", "/health", ..) => health(ask),
        ("GET", "/nets", ..) => nets(ask),
        ("GET", _, Some(id), _) => look(id, ask),
        ("GET", _, _, Some(id)) => job_log(req, id, ask),
        (_, "/runs", ..) => Response::error(405, "POST /runs で流す・GET /runs で読む"),
        (_, "/health" | "/nets", ..) | (_, _, Some(_), _) | (_, _, _, Some(_)) => Response::error(405, "GET で読む"),
        _ => Response::error(404, "POST /runs・GET /runs・GET /runs/<id>・GET /runs/<id>/log・GET /nets・GET /health だけ"),
    }
}

/// `GET /health`: the engine answers, with its status line. What an HA standby polls.
fn health(ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    match ask(&Request::Status) {
        Ok(line) => Response::ok(serde_json::json!({
            "ok": true,
            "version": env!("CARGO_PKG_VERSION"),
            "proto": proto::PROTO,
            "status": line.trim(),
        })),
        Err(e) => Response::error(503, &e),
    }
}

/// `GET /nets`: each definition's id, name, standing and hold. Not the text (it stays on this PC).
fn nets(ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let snap = match ask(&Request::List).and_then(|t| Snapshot::parse(&t)) {
        Ok(s) => s,
        Err(e) => return Response::error(500, &e),
    };
    let nets: Vec<_> = snap
        .defs
        .iter()
        .map(|d| {
            serde_json::json!({
                "id": d.id,
                "group": d.group,
                "name": d.label(),
                "standing": d.standing,
                "held": snap.held.contains(&d.id),
            })
        })
        .collect();
    Response::ok(serde_json::json!({ "nets": nets, "paused": snap.paused, "warnings": snap.warnings }))
}

/// `GET /runs`: today's and the running ones, or with `from` and `to` (`YYYY-MM-DD`) the days'.
fn list_runs(req: &ito_http::Request, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let r = match (req.query("from"), req.query("to")) {
        (None, None) => Request::Runs,
        (Some(from), to) => Request::History { from: from.to_owned(), to: to.unwrap_or(from).to_owned() },
        (None, Some(_)) => return Response::error(400, "to には from も要る"),
    };
    let days = matches!(r, Request::History { .. });
    match ask(&r).and_then(|t| Runs::parse(&t)) {
        Ok(runs) => Response::ok(serde_json::json!({ "runs": runs.runs.iter().map(run_json).collect::<Vec<_>>() })),
        Err(e) if days => Response::error(400, &e),
        Err(e) => Response::error(500, &e),
    }
}

/// `GET /runs/<id>/log?job=<job>`: the tail of the job's log, as `koyomi log` shows it.
fn job_log(req: &ito_http::Request, run: &str, ask: impl Fn(&Request) -> Result<String, String>) -> Response {
    let Some(job) = req.query("job").filter(|j| !j.is_empty()) else {
        return Response::error(400, "job が要る: /runs/<id>/log?job=<ジョブ>");
    };
    match ask(&Request::Log { run: run.to_owned(), job: job.to_owned() }) {
        Ok(text) => Response::ok(serde_json::json!({ "run": run, "job": job, "log": text })),
        Err(e) => Response::error(404, &e),
    }
}

/// A run as JSON: `koyomi status`'s fields and `"over"`.
fn run_json(run: &Run) -> serde_json::Value {
    let mut v = serde_json::to_value(run).unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.insert("over".into(), run.is_over().into());
    }
    v
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
        Ok(Some(run)) => Response::ok(run_json(&run)),
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
        assert_eq!(handle(&req("PUT", "/runs", ""), ask).status, 405);
        assert_eq!(handle(&req("DELETE", "/runs/20261031-090000-01", ""), ask).status, 405);
        assert_eq!(handle(&req("POST", "/nets", ""), ask).status, 405);
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

    fn with_query(mut r: ito_http::Request, q: &[(&str, &str)]) -> ito_http::Request {
        r.query = q.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        r
    }

    /// the read-only paths answer from the same door requests as the CLI.
    #[test]
    fn the_read_paths_answer_from_the_door() {
        let asked = RefCell::new(Vec::new());
        let ask = |r: &Request| {
            asked.borrow_mut().push(format!("{r:?}"));
            match r {
                Request::Status => Ok("動いている\n".to_owned()),
                Request::Runs => Ok(runs(&["20261031-090000-01"])),
                Request::History { from, .. } if from == "bad" => Err("日付は YYYY-MM-DD で書く: bad".to_owned()),
                Request::History { .. } => Ok(runs(&["20261030-230000-01", "20261030-235000-01"])),
                Request::Log { job, .. } if job == "copy" => Ok("line 1\n***\n".to_owned()),
                Request::Log { .. } => Err("ログがない".to_owned()),
                Request::List => Ok(Snapshot { held: vec!["g/b".into()], ..Default::default() }.to_toml()),
                _ => Err("no".to_owned()),
            }
        };
        let h = handle(&req("GET", "/health", ""), ask);
        assert_eq!(
            (h.status, h.json["status"].as_str(), h.json["proto"].as_u64()),
            (200, Some("動いている"), Some(u64::from(proto::PROTO)))
        );
        let r = handle(&req("GET", "/runs", ""), ask);
        assert_eq!((r.status, r.json["runs"][0]["over"].as_bool()), (200, Some(true)));
        let r = handle(&with_query(req("GET", "/runs", ""), &[("from", "2026-10-30")]), ask);
        assert_eq!(r.json["runs"].as_array().map(Vec::len), Some(2));
        assert!(asked.borrow().last().is_some_and(|a| a.contains("to: \"2026-10-30\"")), "{:?}", asked.borrow());
        assert_eq!(handle(&with_query(req("GET", "/runs", ""), &[("from", "bad")]), ask).status, 400);
        assert_eq!(handle(&with_query(req("GET", "/runs", ""), &[("to", "2026-10-30")]), ask).status, 400);
        let l = handle(&with_query(req("GET", "/runs/20261031-090000-01/log", ""), &[("job", "copy")]), ask);
        assert_eq!((l.status, l.json["log"].as_str()), (200, Some("line 1\n***\n")));
        assert_eq!(handle(&req("GET", "/runs/20261031-090000-01/log", ""), ask).status, 400);
        assert_eq!(handle(&with_query(req("GET", "/runs/20261031-090000-01/log", ""), &[("job", "x")]), ask).status, 404);
        let n = handle(&req("GET", "/nets", ""), ask);
        assert_eq!((n.status, n.json["nets"].as_array().map(Vec::len), n.json["paused"].as_bool()), (200, Some(0), Some(false)));
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

    /// on the LAN another PC reaches the same paths over TLS, only with the pinned certificate.
    #[test]
    fn another_pc_reads_over_tls_with_the_pinned_certificate() {
        let dir = std::env::temp_dir().join(format!("koyomi-http-tls-{}", std::process::id()));
        let mut cfg = ito_http::Config::new(0, "tok");
        cfg.bind = ito_http::Bind::Lan { addr: Some([127, 0, 0, 1].into()), tls: ito_http::TlsCfg::new(&dir) };
        let ask = |r: &Request| match r {
            Request::Status => Ok("動いている".to_owned()),
            _ => Err("no".to_owned()),
        };
        let server = Server::start(cfg, move |r| handle(&r, ask), |_| {}).expect("listen");
        let fp = ito_http::tls::format(&server.fingerprint().expect("a LAN bind has a certificate"));
        let url = format!("https://127.0.0.1:{}", server.port());
        assert_eq!(ito_http::client::peek_fingerprint(&url, Duration::from_secs(5)).as_deref(), Ok(fp.as_str()));
        let c = ito_http::client::Client::new(&url, &fp, "tok").expect("client");
        let h = c.get("/health").expect("health");
        assert_eq!((h.status, h.json["status"].as_str()), (200, Some("動いている")));
        let other = ito_http::tls::format(&[7; 32]);
        let wrong = ito_http::client::Client::new(&url, &other, "tok").expect("client").get("/health");
        assert!(matches!(wrong, Err(ito_http::client::Error::Fingerprint(ref got)) if *got == fp), "{wrong:?}");
        assert_eq!(ito_http::client::Client::new(&url, &fp, "nope").expect("client").get("/health").map(|r| r.status), Ok(401));
        server.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_day_comes_from_the_run_id() {
        assert_eq!(started_on("20261031-090000-01").as_deref(), Some("2026-10-31"));
        assert_eq!(started_on("2026103-090000-01"), None);
        assert_eq!(started_on("20261031"), None);
        assert_eq!(started_on("２０２６1031-0"), None);
    }
}
