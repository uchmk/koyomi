//! The definitions staged to take over on a day (`next.toml`).
//!
//! A person writes the next definition of a net now and says from which day it
//! is used; until then the net runs as it is. At the start of that day the
//! engine writes it as the net's file (the old one kept under `versions/`) and,
//! when its text is what was approved here, confirms it. The approval is the
//! SHA-256 of the text, as in `approved.toml`: a hand edit of this file leaves
//! the definition to be confirmed again after the switch.

use std::path::Path;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// One staged definition, found by the id of the net it replaces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    /// The first day it is used: from this day's first time on.
    pub from: NaiveDate,
    /// SHA-256 of `text` as it was approved; another hash leaves it unconfirmed at the switch.
    pub sha256: String,
    /// SHA-256 of the net's file when it was staged: a file changed since is told at the switch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub text: String,
}

impl Entry {
    /// Whether `text` is what was approved.
    pub fn approved(&self) -> bool {
        crate::approved::hash(self.text.as_bytes()) == self.sha256
    }

    /// A seal over everything a person sees of this staged definition: `from`, `base` (`-` for
    /// none, `+<hash>` for one, so none and an empty one differ), the recorded `sha256` and the
    /// `text` itself. The text goes in beside the recorded hash so that a hand edit of just the
    /// text in `next.toml` changes the seal too. It is computed, never stored.
    pub fn seal(&self) -> String {
        let base = match &self.base {
            None => "-".to_owned(),
            Some(b) => format!("+{b}"),
        };
        let joined = format!(
            "{}
{base}
{}
{}",
            self.from.format("%Y-%m-%d"),
            self.sha256,
            self.text
        );
        crate::approved::hash(joined.as_bytes())
    }
}

/// The contents of `next.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub next: Vec<Entry>,
}

impl Store {
    /// The staged definitions in `text`, or why it does not read (its first line).
    pub fn parse(text: &str) -> Result<Store, String> {
        toml::from_str(text).map_err(|e: toml::de::Error| e.message().lines().next().unwrap_or_default().to_owned())
    }

    /// The staged definitions kept at `path`: none when there is no file yet. A file that is there
    /// but does not read is an error: writing over it would drop what was staged without a word.
    pub fn load(path: &Path) -> Result<Store, String> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Store::default()),
            Err(e) => return Err(e.to_string()),
        };
        let text = String::from_utf8(bytes).map_err(|_| "UTF-8 の文字として読めない".to_owned())?;
        Store::parse(text.strip_prefix('\u{feff}').unwrap_or(&text))
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.next.iter().find(|e| e.id == id)
    }

    /// Stages `entry`, in place of what was staged for its net: one next definition per net.
    pub fn set(&mut self, entry: Entry) {
        match self.next.iter_mut().find(|e| e.id == entry.id) {
            Some(e) => *e = entry,
            None => self.next.push(entry),
        }
        self.next.sort_by(|a, b| (a.from, &a.id).cmp(&(b.from, &b.id)));
    }

    /// Drops what was staged for `id`; `false` when there was nothing.
    pub fn forget(&mut self, id: &str) -> bool {
        let before = self.next.len();
        self.next.retain(|e| e.id != id);
        self.next.len() != before
    }

    /// What takes over on or before `today`, earliest first.
    pub fn due(&self, today: NaiveDate) -> Vec<Entry> {
        self.next.iter().filter(|e| e.from <= today).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approved::hash;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn entry(id: &str, from: &str, text: &str) -> Entry {
        Entry { id: id.to_owned(), from: d(from), sha256: hash(text.as_bytes()), base: None, text: text.to_owned() }
    }

    #[test]
    fn staged_definitions_round_trip_through_toml() {
        let mut s = Store::default();
        s.set(entry("経理/月次締め", "2026-12-01", "format = 1\n# 日本語\n"));
        s.set(Entry { base: Some(hash(b"old")), ..entry("共通/バックアップ", "2026-11-15", "x") });
        s.set(entry("経理/月次締め", "2027-01-01", "newer"));
        let text = s.to_toml();
        assert!(text.contains("from = \"2026-11-15\""), "{text}");
        let back = Store::parse(&text).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.next.len(), 2, "one per net");
        assert_eq!(back.next[0].id, "共通/バックアップ", "earliest first");
        assert_eq!(back.get("経理/月次締め").unwrap().text, "newer");
        assert!(back.get("経理/月次締め").unwrap().approved());
    }

    #[test]
    fn what_is_due_is_what_starts_on_or_before_today() {
        let mut s = Store::default();
        s.set(entry("a/x", "2026-12-01", "x"));
        s.set(entry("a/y", "2026-11-30", "y"));
        assert!(s.due(d("2026-11-29")).is_empty());
        assert_eq!(s.due(d("2026-11-30")).len(), 1);
        let ids: Vec<String> = s.due(d("2026-12-05")).into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["a/y", "a/x"]);
    }

    #[test]
    fn a_hand_edit_of_the_text_is_not_approved() {
        let mut e = entry("a/x", "2026-12-01", "x");
        e.text.push('\n');
        assert!(!e.approved());
    }

    /// every part of the entry changes the seal, even when the recorded hash stays the same.
    #[test]
    fn the_seal_follows_each_part_of_the_entry() {
        let e = entry("a/x", "2026-12-01", "x");
        assert_eq!(e.seal(), e.clone().seal(), "the same entry, the same seal");
        assert_eq!(e.seal().len(), 64);
        let later = Entry { from: d("2026-12-02"), ..e.clone() };
        assert_ne!(later.seal(), e.seal(), "only the day changed");
        let based = Entry { base: Some(hash(b"old")), ..e.clone() };
        assert_ne!(based.seal(), e.seal(), "a base where there was none");
        let empty = Entry { base: Some(String::new()), ..e.clone() };
        assert_ne!(empty.seal(), e.seal(), "an empty base is not none");
        assert_ne!(empty.seal(), based.seal());
        let edited = Entry {
            text: "x
"
            .to_owned(),
            ..e.clone()
        };
        assert_eq!(edited.sha256, e.sha256, "the recorded hash is untouched");
        assert_ne!(edited.seal(), e.seal(), "only the text changed");
        let other = Entry { sha256: hash(b"y"), ..e.clone() };
        assert_ne!(other.seal(), e.seal(), "only the recorded hash changed");
    }

    #[test]
    fn forget_drops_what_was_staged() {
        let mut s = Store::default();
        s.set(entry("a/x", "2026-12-01", "x"));
        assert!(s.forget("a/x"));
        assert!(!s.forget("a/x"));
        assert!(s.next.is_empty());
    }

    #[test]
    fn load_tells_a_missing_file_from_a_damaged_one() {
        let dir = std::env::temp_dir().join(format!("koyomi-next-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("next.toml");
        let _ = std::fs::remove_file(&p);
        assert_eq!(Store::load(&p), Ok(Store::default()), "no file yet: nothing staged");
        std::fs::write(&p, "next = [[ broken").unwrap();
        assert!(Store::load(&p).is_err());
        std::fs::write(&p, "[[next]]\nid = 'a/x'\nfrom = '12/1'\nsha256 = ''\ntext = ''\n").unwrap();
        assert!(Store::load(&p).is_err(), "a day that is not one");
        let mut s = Store::default();
        s.set(entry("a/x", "2026-12-01", "x"));
        std::fs::write(&p, s.to_toml()).unwrap();
        assert_eq!(Store::load(&p), Ok(s));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
