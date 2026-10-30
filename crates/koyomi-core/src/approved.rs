//! The definitions a person has looked at and approved.
//!
//! An approval is the SHA-256 of the file's whole bytes (a comment counts: a
//! hand edit of any kind sends the job net back to 未確認). The approved text
//! is kept beside the hash so the next approval can show what changed.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Lower-case hex SHA-256 of `bytes`.
pub fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// One approved definition, found by its id (`経理/月次締め`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub sha256: String,
    pub text: String,
}

/// The contents of `approved.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub approved: Vec<Entry>,
}

impl Store {
    /// The approvals in `text`, or why it does not read (its first line).
    pub fn parse(text: &str) -> Result<Store, String> {
        toml::from_str(text).map_err(|e: toml::de::Error| e.message().lines().next().unwrap_or_default().to_owned())
    }

    /// The approvals kept at `path`: none when there is no file yet. A file that is there but does
    /// not read is an error, not "nothing approved": writing the next approval over it would drop
    /// every other one without a word.
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
        self.approved.iter().find(|e| e.id == id)
    }

    /// Whether `sha256` is what was approved for `id`.
    pub fn is_approved(&self, id: &str, sha256: &str) -> bool {
        self.get(id).is_some_and(|e| e.sha256 == sha256)
    }

    /// Approves `text` (whose bytes hash to `sha256`) for `id`, replacing the earlier approval.
    pub fn approve(&mut self, id: &str, sha256: &str, text: &str) {
        let entry = Entry { id: id.to_owned(), sha256: sha256.to_owned(), text: text.to_owned() };
        match self.approved.iter_mut().find(|e| e.id == id) {
            Some(e) => *e = entry,
            None => self.approved.push(entry),
        }
    }

    /// Drops the approval of `id` (a net that is gone or has a new name); `false` when there was none.
    pub fn forget(&mut self, id: &str) -> bool {
        let before = self.approved.len();
        self.approved.retain(|e| e.id != id);
        self.approved.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_sha256_hex() {
        assert_eq!(hash(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn a_comment_changes_the_hash() {
        assert_ne!(hash(b"format = 1\n"), hash(b"format = 1 # x\n"));
    }

    #[test]
    fn approvals_round_trip_through_toml() {
        let mut s = Store::default();
        s.approve("経理/月次締め", &hash(b"a"), "format = 1\n# 日本語\n");
        s.approve("共通/バックアップ", &hash(b"b"), "x");
        s.approve("経理/月次締め", &hash(b"c"), "newer");
        let back = Store::parse(&s.to_toml()).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.approved.len(), 2);
        assert!(back.is_approved("経理/月次締め", &hash(b"c")));
        assert!(!back.is_approved("経理/月次締め", &hash(b"a")));
        assert_eq!(back.get("経理/月次締め").unwrap().text, "newer");
    }

    #[test]
    fn a_damaged_file_is_an_error() {
        assert!(Store::parse("this is [[ not toml").is_err());
    }

    #[test]
    fn load_tells_a_missing_file_from_a_damaged_one() {
        let dir = std::env::temp_dir().join(format!("koyomi-approved-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("approved.toml");
        let _ = std::fs::remove_file(&p);
        assert_eq!(Store::load(&p), Ok(Store::default()), "no file yet: nothing approved");
        std::fs::write(&p, "approved = [[ broken").unwrap();
        assert!(Store::load(&p).is_err());
        std::fs::write(&p, [0x82, 0xa0]).unwrap();
        assert!(Store::load(&p).unwrap_err().contains("UTF-8"));
        let mut s = Store::default();
        s.approve("a/b", &hash(b"x"), "x");
        std::fs::write(&p, s.to_toml()).unwrap();
        assert_eq!(Store::load(&p), Ok(s));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
