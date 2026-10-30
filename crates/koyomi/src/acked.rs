//! Failures the person has dealt with: the ids of the runs, kept in `acked.toml`
//! so the count of failures still waiting survives a restart.

use std::collections::BTreeSet;
use std::path::PathBuf;

use ito_common::kept;
use serde::{Deserialize, Serialize};

/// Older acknowledgements are dropped past this many (the runs themselves go with keep_days).
const KEEP: usize = 2000;

#[derive(Default, Serialize, Deserialize)]
struct Kept {
    #[serde(default)]
    acked: Vec<String>,
}

pub struct Acked {
    ids: BTreeSet<String>,
    writer: Option<kept::Writer>,
}

impl Acked {
    /// Reads `acked.toml` under `dir`; with no folder nothing is kept.
    pub fn load(dir: Option<PathBuf>) -> Acked {
        let Some(dir) = dir else { return Acked { ids: BTreeSet::new(), writer: None } };
        let path = dir.join("acked.toml");
        let last: Kept = kept::read(&path).unwrap_or_default();
        Acked { ids: last.acked.into_iter().collect(), writer: Some(kept::Writer::new(path, true)) }
    }

    #[cfg(test)]
    pub fn is_acked(&self, id: &str) -> bool {
        self.ids.contains(id)
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
    fn old_acks_are_dropped_past_the_limit() {
        let mut a = Acked::load(None);
        let ids: Vec<String> = (0..KEEP + 5).map(|i| format!("{i:05}")).collect();
        a.ack(ids.iter().map(String::as_str));
        assert_eq!(a.ids.len(), KEEP);
        assert!(!a.is_acked("00000") && a.is_acked(&format!("{:05}", KEEP + 4)));
    }
}
