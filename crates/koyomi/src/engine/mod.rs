//! `koyomi engine`: the one process that owns the definitions.
//!
//! It reads `jobnets/<group>/<name>.toml`, keeps `approved.toml`, watches the
//! folder, and answers the window and the CLI through the door (`proto`). It
//! outlives the window. Nothing here draws or asks a person anything.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use ito_ipc::door::{ask, Ask};
use koyomi_core::approved::Store;
use koyomi_core::scan::{scan, Def, Standing};

use crate::proto::{self, Request, Snapshot};

/// Where everything of the engine's lives.
#[derive(Clone, Debug)]
pub struct Paths {
    /// `app_dir("koyomi")`: the settings folder. Holds `jobnets/`.
    pub app: PathBuf,
    pub jobnets: PathBuf,
    /// `state_dir("koyomi")/approved.toml`.
    pub approved: PathBuf,
}

impl Paths {
    pub fn find() -> Result<Paths, String> {
        let app = ito_common::app_dir("koyomi").ok_or("設定のフォルダーが分からない")?;
        let state = ito_common::state_dir("koyomi").ok_or("状態のフォルダーが分からない")?;
        Ok(Paths { jobnets: app.join("jobnets"), app, approved: state.join("approved.toml") })
    }

    /// The approvals, or the line that says `approved.toml` does not read.
    pub fn store(&self) -> Result<Store, String> {
        Store::load(&self.approved).map_err(|e| format!("{} が読めない（{e}）。直すか退けるまで承認しない", shown(&self.approved)))
    }
}

/// Who other than this user can write what koyomi runs from, one line each. Empty: nothing in the way.
///
/// The definitions, the folders above them and `approved.toml` all count: whoever can change one
/// of them can make koyomi run a command nobody looked at.
pub fn access_warnings(paths: &Paths, files: &[PathBuf]) -> Vec<String> {
    let mut seen: Vec<(PathBuf, String)> = Vec::new();
    let mut check = |p: &Path| {
        if !p.exists() {
            return;
        }
        for f in ito_common::access::findings(p).unwrap_or_default() {
            let key = (f.path.clone(), f.who.clone());
            if !seen.contains(&key) {
                seen.push(key);
            }
        }
    };
    check(&paths.jobnets);
    check(&paths.app);
    check(&paths.approved);
    files.iter().for_each(|f| check(f));
    seen.into_iter().map(|(p, who)| format!("{} は {} が書き換えられる", shown(&p), who)).collect()
}

/// The path without Windows' `\\?\` prefix.
fn shown(p: &Path) -> String {
    let s = p.display().to_string();
    s.strip_prefix(r"\\?\").map(str::to_owned).unwrap_or(s)
}

struct State {
    paths: Paths,
    store: Store,
    /// Why `approved.toml` did not read: until it does, nothing is approved and nothing written over it.
    unreadable: Option<String>,
    defs: Vec<Def>,
    warnings: Vec<String>,
}

impl State {
    fn new(paths: Paths) -> State {
        let mut s = State { paths, store: Store::default(), unreadable: None, defs: Vec::new(), warnings: Vec::new() };
        s.reload();
        s.rescan();
        s
    }

    /// Reads `approved.toml` again; only while it does not read (the engine is its one writer).
    fn reload(&mut self) {
        match self.paths.store() {
            Ok(store) => {
                self.store = store;
                self.unreadable = None;
            }
            Err(e) => {
                self.store = Store::default();
                self.unreadable = Some(e);
            }
        }
    }

    fn rescan(&mut self) {
        if self.unreadable.is_some() {
            self.reload();
        }
        self.defs = scan(&self.paths.jobnets, &self.store);
        let files: Vec<PathBuf> = self.defs.iter().map(|d| self.paths.jobnets.join(&d.file)).collect();
        self.warnings = access_warnings(&self.paths, &files);
        self.warnings.extend(self.unreadable.clone());
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { defs: self.defs.clone(), warnings: self.warnings.clone() }
    }

    fn status(&self) -> String {
        let confirmed = self.defs.iter().filter(|d| d.standing == Standing::Confirmed).count();
        format!("koyomi engine pid {}: 定義 {} 件（確認済み {}）", std::process::id(), self.defs.len(), confirmed)
    }

    fn approve(&mut self, id: &str, sha256: &str) -> Result<String, String> {
        if !self.warnings.is_empty() {
            return Err(format!("承認できない: {}", self.warnings.join(" / ")));
        }
        let def = self.defs.iter().find(|d| d.id == id).ok_or_else(|| format!("定義がない: {id}"))?;
        if def.standing == Standing::Invalid {
            return Err(format!("定義の誤りがあり、承認できない: {}", def.file));
        }
        if def.sha256 != sha256 {
            return Err(format!("{} が確かめたあとに変わった。もう一度確かめてください", def.file));
        }
        let mut next = self.store.clone();
        next.approve(id, &def.sha256, &def.text);
        // Written before it counts, and whole: a crash leaves the old approvals.
        if let Some(dir) = self.paths.approved.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        ito_common::write_atomic(&self.paths.approved, next.to_toml())?;
        self.store = next;
        self.rescan();
        Ok(format!("確認した: {id}"))
    }

    fn answer(&mut self, request: &Request) -> Result<String, String> {
        match request {
            Request::Status => Ok(self.status()),
            Request::Stop => Ok("止める".to_owned()),
            Request::List => Ok(self.snapshot().to_toml()),
            Request::Show { id } => {
                let def = self.defs.iter().find(|d| &d.id == id).ok_or_else(|| format!("定義がない: {id}"))?;
                toml::to_string(def).map_err(|e| e.to_string())
            }
            Request::Approve { id, sha256 } => self.approve(id, sha256),
        }
    }
}

/// How often everything is looked at again even when the watcher is silent
/// (an access list changes without a file event).
const SWEEP: Duration = Duration::from_secs(5);
/// How long after the last file event the engine waits before it reads (an editor writes in bursts).
const DEBOUNCE: Duration = Duration::from_millis(200);

/// Runs until `koyomi engine stop`. The exit code: 0 after a stop, 1 when another engine is already up.
pub fn run() -> i32 {
    let paths = match Paths::find() {
        Ok(p) => p,
        Err(e) => {
            crate::say(&format!("koyomi: {e}"), true);
            return 1;
        }
    };
    let door = proto::door();
    if ask(&door, &Request::Status).is_ok() {
        crate::say("koyomi: もう動いている", true);
        return 1;
    }
    let _ = std::fs::create_dir_all(&paths.jobnets);
    let rx = proto::door().listen(|| {});

    // `listen` quietly waits when another engine holds the door; ask until an engine answers, and
    // leave if it is not this one. Two engines started together are told apart here.
    let (mine_tx, mine_rx) = mpsc::channel::<bool>();
    std::thread::spawn(move || {
        for _ in 0..60 {
            if let Ok(text) = ask(&proto::door(), &Request::Status) {
                let _ = mine_tx.send(text.contains(&format!("pid {}:", std::process::id())));
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = mine_tx.send(false);
    });

    let dirty = Arc::new(AtomicBool::new(true));
    let watching = Arc::new(AtomicBool::new(true));
    let mut watcher = {
        let (dirty, watching) = (dirty.clone(), watching.clone());
        ito_watch::Watcher::start(move |ev| match ev {
            ito_watch::Event::Watching(on) => watching.store(on, Ordering::Relaxed),
            _ => dirty.store(true, Ordering::Relaxed),
        })
    };
    watcher.watch(vec![paths.jobnets.clone()]);

    let mut state = State::new(paths);
    let mut last_scan = Instant::now();
    let mut changed_at: Option<Instant> = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ask { request, reply, expired }) => {
                let stop = matches!(request, Request::Stop);
                let out = state.answer(&request);
                if !expired.load(Ordering::Relaxed) {
                    let _ = reply.send(out);
                }
                if stop {
                    // The reply travels on the connection's thread.
                    std::thread::sleep(Duration::from_millis(150));
                    return 0;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return 1,
        }
        if let Ok(false) = mine_rx.try_recv() {
            crate::say("koyomi: もう動いている", true);
            return 1;
        }
        if dirty.swap(false, Ordering::Relaxed) {
            changed_at = Some(Instant::now());
        }
        let quiet = changed_at.is_some_and(|t| t.elapsed() >= DEBOUNCE);
        if quiet || last_scan.elapsed() >= SWEEP {
            changed_at = None;
            last_scan = Instant::now();
            state.rescan();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_survives_the_door() {
        let net = "format = 1\nname = \"月次締め\"\n[[job]]\nid = \"a\"\nrun = [\"x.exe\", \"-v\"]\npos = [1.0, 2.0]\n";
        let dir = std::env::temp_dir().join(format!("koyomi-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("経理")).unwrap();
        std::fs::write(dir.join("経理/月次締め.toml"), net).unwrap();
        std::fs::create_dir_all(dir.join("共通")).unwrap();
        std::fs::write(dir.join("共通/壊れ.toml"), "format = 1\nrnu = 1\n").unwrap();
        let snap = Snapshot { defs: scan(&dir, &Store::default()), warnings: vec!["w".into()] };
        let back = Snapshot::parse(&snap.to_toml()).unwrap();
        assert_eq!(back, snap);
        assert_eq!(back.defs.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
