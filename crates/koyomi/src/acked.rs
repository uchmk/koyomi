//! Failures the person has dealt with: the ids of the runs, kept in `acked.toml`
//! so the count of failures still waiting survives a restart.

use std::collections::BTreeSet;
use std::path::PathBuf;

use ito_common::kept;
use serde::{Deserialize, Serialize};

/// `acked.toml` for this PC, `acked-<first 16 hex of the name's hash>.toml` for a `[[remote]]` (a name is not
/// always fit for a file name).
pub fn file_name(far: Option<&str>) -> String {
    match far {
        None => "acked.toml".to_owned(),
        Some(name) => {
            let hash = koyomi_core::approved::hash(name.as_bytes());
            format!("acked-{}.toml", &hash[..16])
        }
    }
}

/// Moves the acknowledgement file of the `[[remote]]` `old` to the name `new` under `dir`, when one is renamed. A
/// missing file is fine, the same path is left alone, and a file already at the new name is replaced: the connection
/// now called that carries its own acknowledgements.
pub fn move_file(dir: &std::path::Path, old: &str, new: &str) -> Result<(), String> {
    let (from, to) = (dir.join(file_name(Some(old))), dir.join(file_name(Some(new))));
    if from == to {
        return Ok(());
    }
    match std::fs::rename(&from, &to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Older acknowledgements are dropped past this many (the runs themselves go with keep_days).
const KEEP: usize = 2000;

#[derive(Default, Serialize, Deserialize)]
struct Kept {
    #[serde(default)]
    acked: Vec<String>,
}

pub struct Acked {
    ids: BTreeSet<String>,
    /// Set at the start and by every change, until [`Acked::take_changed`].
    dirty: bool,
    writer: Option<kept::Writer>,
}

impl Acked {
    /// Reads `acked.toml` under `dir`; with no folder nothing is kept.
    #[cfg(test)]
    pub fn load(dir: Option<PathBuf>) -> Acked {
        Acked::load_for(dir, None)
    }

    /// The file of the link `far` (a `[[remote]]` name; `None`: this PC's `acked.toml`). Run ids of another
    /// PC are its own, so each keeps a file of its own.
    pub fn load_for(dir: Option<PathBuf>, far: Option<&str>) -> Acked {
        let Some(dir) = dir else { return Acked { ids: BTreeSet::new(), dirty: true, writer: None } };
        let path = dir.join(file_name(far));
        let last: Kept = kept::read(&path).unwrap_or_default();
        Acked { ids: last.acked.into_iter().collect(), dirty: true, writer: Some(kept::Writer::new(path, true)) }
    }

    /// The ids in `acked.toml` under `dir`, for the engine's tray: the window is the one that writes it.
    pub fn read(dir: &std::path::Path) -> BTreeSet<String> {
        kept::read::<Kept>(&dir.join("acked.toml")).unwrap_or_default().acked.into_iter().collect()
    }

    /// The ids kept for the link `far` under `dir`, for whoever only watches it (the window's watch of the other
    /// PCs; the one writer is [`Acked`]). A missing file is an empty set; a file that is there and does not read is
    /// `None`, so the caller keeps the set it had.
    pub fn read_for(dir: &std::path::Path, far: Option<&str>) -> Option<BTreeSet<String>> {
        let path = dir.join(file_name(far));
        if !path.exists() {
            return Some(BTreeSet::new());
        }
        kept::read::<Kept>(&path).map(|k| k.acked.into_iter().collect())
    }

    #[cfg(test)]
    pub fn is_acked(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    /// The set, once after it changed (the first call included), for whoever shows it.
    pub fn take_changed(&mut self) -> Option<BTreeSet<String>> {
        std::mem::take(&mut self.dirty).then(|| self.ids.clone())
    }

    /// How many of `failed` (run ids) are still waiting.
    pub fn waiting<'a>(&self, failed: impl IntoIterator<Item = &'a str>) -> usize {
        failed.into_iter().filter(|id| !self.ids.contains(*id)).count()
    }

    /// Marks `ids` dealt with; true when something changed.
    pub fn ack<'a>(&mut self, ids: impl IntoIterator<Item = &'a str>) -> bool {
        let before = self.ids.len();
        self.ids.extend(ids.into_iter().map(str::to_owned));
        if self.ids.len() == before {
            return false;
        }
        self.dirty = true;
        while self.ids.len() > KEEP {
            self.ids.pop_first();
        }
        if let Some(w) = &self.writer {
            w.save_toml(&Kept { acked: self.ids.iter().cloned().collect() });
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_link_has_its_own_file() {
        assert_eq!(file_name(None), "acked.toml");
        let (a, b) = (file_name(Some("kei")), file_name(Some("eigyo")));
        assert_ne!(a, b);
        assert_eq!(a, file_name(Some("kei")));
        assert!(a.starts_with("acked-") && a.ends_with(".toml") && a.len() == "acked-.toml".len() + 16, "{a}");
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("koyomi-acked-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_file_follows_the_name_and_a_missing_one_or_the_same_path_is_fine() {
        let d = temp("move");
        std::fs::write(d.join(file_name(Some("kei"))), "acked = [\"r1\"]\n").unwrap();
        move_file(&d, "kei", "keiba").unwrap();
        assert!(!d.join(file_name(Some("kei"))).exists());
        assert_eq!(std::fs::read_to_string(d.join(file_name(Some("keiba")))).unwrap(), "acked = [\"r1\"]\n");
        // Nothing to move: fine, and nothing appears.
        move_file(&d, "none", "other").unwrap();
        assert!(!d.join(file_name(Some("other"))).exists());
        // The same path is left alone.
        move_file(&d, "keiba", "keiba").unwrap();
        assert!(d.join(file_name(Some("keiba"))).exists());
        // A file at the new name is replaced.
        std::fs::write(d.join(file_name(Some("old"))), "acked = [\"r9\"]\n").unwrap();
        move_file(&d, "old", "keiba").unwrap();
        assert_eq!(std::fs::read_to_string(d.join(file_name(Some("keiba")))).unwrap(), "acked = [\"r9\"]\n");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_watcher_reads_a_links_file_and_tells_a_missing_one_from_an_unreadable_one() {
        let d = temp("read-for");
        assert_eq!(Acked::read_for(&d, Some("kei")), Some(BTreeSet::new()), "missing: nothing acked");
        std::fs::write(d.join(file_name(Some("kei"))), "acked = [\"r1\", \"r2\"]").unwrap();
        let got = Acked::read_for(&d, Some("kei")).unwrap();
        assert_eq!(got.into_iter().collect::<Vec<_>>(), ["r1", "r2"]);
        assert_eq!(Acked::read_for(&d, Some("other")), Some(BTreeSet::new()), "each link has its own file");
        std::fs::write(d.join(file_name(Some("bad"))), "acked = [not toml").unwrap();
        assert_eq!(Acked::read_for(&d, Some("bad")), None, "there but unreadable");
        std::fs::write(d.join(file_name(None)), "acked = [\"m\"]").unwrap();
        assert_eq!(Acked::read_for(&d, None).map(|s| s.len()), Some(1));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn waiting_counts_only_what_is_not_acked() {
        let mut a = Acked::load(None);
        let failed = ["r1", "r2", "r3"];
        assert_eq!(a.waiting(failed), 3);
        assert!(a.ack(["r2"]));
        assert!(!a.ack(["r2"]));
        assert_eq!(a.waiting(failed), 2);
        assert!(a.is_acked("r2") && !a.is_acked("r1"));
    }

    #[test]
    fn a_change_is_handed_over_once() {
        let mut a = Acked::load(None);
        assert!(a.take_changed().is_some(), "the first call hands the set over");
        assert!(a.take_changed().is_none());
        a.ack(["r1"]);
        assert_eq!(a.take_changed().map(|s| s.len()), Some(1));
        a.ack(["r1"]);
        assert!(a.take_changed().is_none(), "nothing changed");
    }

    #[test]
    fn old_acks_are_dropped_past_the_limit() {
        let mut a = Acked::load(None);
        let ids: Vec<String> = (0..KEEP + 5).map(|i| format!("{i:05}")).collect();
        a.ack(ids.iter().map(String::as_str));
        assert_eq!(a.ids.len(), KEEP);
        assert!(!a.is_acked("00000") && a.is_acked(&format!("{:05}", KEEP + 4)));
    }
}
