//! Starts by events: `[[when]] file = …` (a file arriving) and `on_start = true`
//! (the engine's start). The clock is `schedule`; this is the rest.
//!
//! A file counts as arrived when it is there and, with `settle`, has kept its
//! size and change time for that long. Each file starts its net once: it is
//! remembered by path, size and change time until it goes or changes. Files
//! already there when a pattern is first watched are remembered as done, so a
//! restart does not run an old file again. The folders are watched with
//! `ito-watch`; they are also looked at every [`POLL`] in case the watch cannot
//! see them (a share, a folder not there yet).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use koyomi_core::scan::{Def, Standing};
use koyomi_core::when::Kind;

use super::files;

/// How often the folders are looked at while a file is settling.
const CHECK: Duration = Duration::from_millis(500);
/// How often they are looked at anyway, for a folder the watch cannot see.
const POLL: Duration = Duration::from_secs(5);

/// A file that came for a net: the net, which `file =` it matched, and the file.
#[derive(Clone, Debug, PartialEq)]
pub struct Arrival {
    pub def_id: String,
    pub pattern: String,
    pub path: PathBuf,
}

/// A file as last seen under a pattern.
struct Seen {
    len: u64,
    mtime: Option<SystemTime>,
    /// When it was last seen to change (or first seen).
    since: Instant,
    /// Whether this file, as it is, has already started (or been recorded for) its net.
    fired: bool,
}

struct Rule {
    settle: Option<u64>,
    seen: BTreeMap<PathBuf, Seen>,
}

pub struct Events {
    watcher: ito_watch::Watcher,
    changed: Arc<AtomicBool>,
    watching: Arc<AtomicBool>,
    /// By (net id, pattern): one `[[when]] file` each.
    rules: BTreeMap<(String, String), Rule>,
    last_check: Instant,
    last_poll: Instant,
    /// Look now, whatever the clocks say.
    force: bool,
    started: bool,
}

impl Events {
    pub fn new() -> Events {
        let changed = Arc::new(AtomicBool::new(false));
        let watching = Arc::new(AtomicBool::new(true));
        let watcher = {
            let (changed, watching) = (changed.clone(), watching.clone());
            ito_watch::Watcher::start(move |ev| match ev {
                ito_watch::Event::Watching(on) => watching.store(on, Ordering::Relaxed),
                _ => changed.store(true, Ordering::Relaxed),
            })
        };
        let now = Instant::now();
        Events { watcher, changed, watching, rules: BTreeMap::new(), last_check: now, last_poll: now, force: true, started: false }
    }

    /// Takes the `file` whens of the confirmed nets as the rules to watch by.
    /// A pattern new to the watch takes the files already there as done, and
    /// says so: `(net id, pattern, how many)` for each that had some.
    pub fn set_rules(&mut self, defs: &[Def]) -> Vec<(String, String, usize)> {
        let mut wanted: BTreeMap<(String, String), Option<u64>> = BTreeMap::new();
        for d in defs.iter().filter(|d| d.standing == Standing::Confirmed) {
            if let Some(net) = &d.net {
                for w in &net.when {
                    if let Kind::File { file, settle } = &w.kind {
                        wanted.insert((d.id.clone(), file.clone()), *settle);
                    }
                }
            }
        }
        let before = self.rules.len();
        self.rules.retain(|k, _| wanted.contains_key(k));
        let mut already = Vec::new();
        let now = Instant::now();
        for (key, settle) in wanted {
            match self.rules.get_mut(&key) {
                Some(r) => r.settle = settle,
                None => {
                    let seen: BTreeMap<PathBuf, Seen> = files::find_files(&key.1)
                        .into_iter()
                        .map(|(path, len, mtime)| (path, Seen { len, mtime, since: now, fired: true }))
                        .collect();
                    if !seen.is_empty() {
                        already.push((key.0.clone(), key.1.clone(), seen.len()));
                    }
                    self.rules.insert(key, Rule { settle, seen });
                    self.force = true;
                }
            }
        }
        if self.rules.len() != before {
            self.force = true;
        }
        let roots: BTreeSet<PathBuf> = self.rules.keys().map(|(_, p)| files::folder_of(p)).collect();
        self.watcher.watch(roots.into_iter().collect());
        already
    }

    /// Whether the watcher sees every folder asked for; when not, they are looked at every 5 s.
    pub fn watching(&self) -> bool {
        self.watching.load(Ordering::Relaxed) && self.watcher.is_running()
    }

    /// `true` once: the engine has just started (`on_start = true`).
    pub fn take_start(&mut self) -> bool {
        !std::mem::replace(&mut self.started, true)
    }

    /// The files that have arrived and not yet started their net, as of now.
    /// Looks at the folders when the watch saw a change, when a file is
    /// settling, and every [`POLL`] anyway.
    pub fn due(&mut self) -> Vec<Arrival> {
        let changed = self.changed.swap(false, Ordering::Relaxed);
        let settling = self.rules.values().any(|r| r.seen.values().any(|s| !s.fired));
        let go = changed || self.force || (settling && self.last_check.elapsed() >= CHECK) || self.last_poll.elapsed() >= POLL;
        if !go {
            return Vec::new();
        }
        let now = Instant::now();
        self.force = false;
        self.last_check = now;
        if now.duration_since(self.last_poll) >= POLL {
            self.last_poll = now;
        }
        let mut out = Vec::new();
        for ((def_id, pattern), rule) in &mut self.rules {
            let found = files::find_files(pattern);
            let keep: BTreeSet<PathBuf> = found.iter().map(|f| f.0.clone()).collect();
            rule.seen.retain(|p, _| keep.contains(p));
            for (path, len, mtime) in found {
                match rule.seen.get_mut(&path) {
                    Some(s) if s.len == len && s.mtime == mtime => {}
                    Some(s) => {
                        // Written again: it counts as a new arrival once it settles.
                        *s = Seen { len, mtime, since: now, fired: false };
                    }
                    None => {
                        rule.seen.insert(path, Seen { len, mtime, since: now, fired: false });
                    }
                }
            }
            for (path, s) in &rule.seen {
                let settled = rule.settle.is_none_or(|secs| now.duration_since(s.since) >= Duration::from_secs(secs));
                if !s.fired && settled {
                    out.push(Arrival { def_id: def_id.clone(), pattern: pattern.clone(), path: path.clone() });
                }
            }
        }
        out
    }

    /// `path` of `pattern` has started (or been recorded for) its net: not again as it is.
    pub fn fired(&mut self, def_id: &str, pattern: &str, path: &std::path::Path) {
        if let Some(r) = self.rules.get_mut(&(def_id.to_owned(), pattern.to_owned())) {
            if let Some(s) = r.seen.get_mut(path) {
                s.fired = true;
            }
        }
    }

    /// Look at the folders on the next `due`, whatever the clocks say.
    #[cfg(test)]
    fn poke(&mut self) {
        self.force = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koyomi_core::scan::Def;

    fn def(id: &str, text: &str) -> Def {
        let net = koyomi_core::jobnet::parse(text).unwrap();
        Def {
            id: id.to_owned(),
            group: "g".to_owned(),
            name: id.to_owned(),
            title: None,
            file: format!("{id}.toml"),
            standing: Standing::Confirmed,
            problems: Vec::new(),
            net: Some(net),
            sha256: String::new(),
            text: text.to_owned(),
            previous: None,
            calendar: None,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koyomi-events-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn net(pattern: &str, settle: &str) -> String {
        let pattern = pattern.replace('\\', "/");
        format!("format = 1\n[[when]]\nfile = \"{pattern}\"\n{settle}\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n")
    }

    #[test]
    fn a_file_already_there_is_recorded_not_started_and_a_new_one_starts_once() {
        let dir = scratch("once");
        std::fs::write(dir.join("old.csv"), "1").unwrap();
        let pattern = dir.join("*.csv").display().to_string().replace('\\', "/");
        let d = def("g/n", &net(&pattern, ""));
        let mut ev = Events::new();
        assert_eq!(ev.set_rules(std::slice::from_ref(&d)), [("g/n".to_owned(), pattern.clone(), 1)]);
        assert!(ev.set_rules(std::slice::from_ref(&d)).is_empty());
        ev.poke();
        assert!(ev.due().is_empty());
        std::fs::write(dir.join("new.csv"), "1,2").unwrap();
        ev.poke();
        let due = ev.due();
        assert_eq!(due.len(), 1, "{due:?}");
        assert_eq!(due[0].def_id, "g/n");
        assert!(due[0].path.ends_with("new.csv"));
        // Until it is marked fired, it is due again; after, it is not.
        ev.poke();
        assert_eq!(ev.due().len(), 1);
        ev.fired("g/n", &pattern, &due[0].path);
        ev.poke();
        assert!(ev.due().is_empty());
        // Written again: once more. Gone: nothing.
        std::fs::write(dir.join("new.csv"), "1,2,3").unwrap();
        ev.poke();
        assert_eq!(ev.due().len(), 1);
        std::fs::remove_file(dir.join("new.csv")).unwrap();
        ev.poke();
        assert!(ev.due().is_empty());
        // A rule that goes is dropped; `.txt` never counted.
        std::fs::write(dir.join("note.txt"), "1").unwrap();
        ev.poke();
        assert!(ev.due().is_empty());
        ev.set_rules(&[]);
        assert!(ev.rules.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_settle_waits_for_the_size_to_stay() {
        let dir = scratch("settle");
        let pattern = dir.join("in.dat").display().to_string().replace('\\', "/");
        let d = def("g/n", &net(&pattern, "settle = \"1s\""));
        let mut ev = Events::new();
        ev.set_rules(std::slice::from_ref(&d));
        std::fs::write(dir.join("in.dat"), "1").unwrap();
        ev.poke();
        assert!(ev.due().is_empty());
        std::thread::sleep(Duration::from_millis(600));
        std::fs::write(dir.join("in.dat"), "12345").unwrap();
        ev.poke();
        assert!(ev.due().is_empty());
        std::thread::sleep(Duration::from_millis(600));
        ev.poke();
        assert!(ev.due().is_empty(), "changed 0.6 s ago: not settled yet");
        std::thread::sleep(Duration::from_millis(500));
        ev.poke();
        assert_eq!(ev.due().len(), 1);
        assert!(ev.take_start());
        assert!(!ev.take_start());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
