//! `jobnets/<group>/<name>.toml` read into what the tree and `koyomi list` show.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::approved::{self, Store};
use crate::jobnet::{self, Jobnet, Problem};

/// Where a definition stands. Only [`Confirmed`](Standing::Confirmed) may ever run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// Looked at and approved, and not touched since.
    Confirmed,
    /// Not approved yet, or changed since it was (by hand, or by an import).
    Unconfirmed,
    /// Does not read; there is nothing to approve.
    Invalid,
}

/// One definition file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Def {
    /// `経理/月次締め`: what `koyomi approve` takes.
    pub id: String,
    pub group: String,
    /// The file stem; what the tree shows when the file has no `name`.
    pub name: String,
    /// The file's own `name`, when it reads and has one.
    pub title: Option<String>,
    /// The path as the person sees it: `経理/月次締め.toml`.
    pub file: String,
    pub standing: Standing,
    pub problems: Vec<Problem>,
    pub net: Option<Jobnet>,
    pub sha256: String,
    pub text: String,
    /// The text last approved, when this has changed since: what the next approval compares with.
    pub previous: Option<String>,
}

impl Def {
    /// What the tree calls it.
    pub fn label(&self) -> &str {
        self.title.as_deref().filter(|t| !t.is_empty()).unwrap_or(&self.name)
    }
}

/// Reads the file at `path` as the definition `id` in `group` (`file` is how it is shown).
pub fn read_def(path: &Path, group: &str, name: &str, store: &Store) -> Def {
    let id = format!("{group}/{name}");
    let file = format!("{group}/{name}.toml");
    let bytes = std::fs::read(path);
    let (sha256, text, read_problem) = match &bytes {
        Ok(b) => match std::str::from_utf8(b) {
            Ok(t) => (approved::hash(b), t.to_owned(), None),
            Err(_) => (approved::hash(b), String::new(), Some("UTF-8 の文字として読めない".to_owned())),
        },
        Err(e) => (String::new(), String::new(), Some(format!("読めない: {e}"))),
    };
    let (net, problems) = match read_problem {
        Some(message) => (None, vec![Problem { line: 0, message }]),
        None => match jobnet::parse(&text) {
            Ok(n) => (Some(n), Vec::new()),
            Err(p) => (None, p),
        },
    };
    let standing = if net.is_none() {
        Standing::Invalid
    } else if store.is_approved(&id, &sha256) {
        Standing::Confirmed
    } else {
        Standing::Unconfirmed
    };
    let previous = (standing == Standing::Unconfirmed).then(|| store.get(&id).map(|e| e.text.clone())).flatten();
    let title = net.as_ref().and_then(|n| n.name.clone());
    Def { id, group: group.to_owned(), name: name.to_owned(), title, file, standing, problems, net, sha256, text, previous }
}

/// Every `<dir>/<group>/<name>.toml` (not `_group.toml`), sorted by group then name.
pub fn scan(dir: &Path, store: &Store) -> Vec<Def> {
    let mut found: Vec<(String, String, PathBuf)> = Vec::new();
    for group in subdirs(dir) {
        let group_name = file_name(&group);
        let Ok(files) = std::fs::read_dir(&group) else { continue };
        for f in files.flatten() {
            let p = f.path();
            let name = file_name(&p);
            let Some(stem) = name.strip_suffix(".toml") else { continue };
            if stem.is_empty() || stem == "_group" || !p.is_file() {
                continue;
            }
            found.push((group_name.clone(), stem.to_owned(), p));
        }
    }
    found.sort();
    found.into_iter().map(|(g, n, p)| read_def(&p, &g, &n, store)).collect()
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET: &str = "format = 1\nname = \"月次締め\"\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("koyomi-core-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(dir: &Path, group: &str, name: &str, text: &str) {
        std::fs::create_dir_all(dir.join(group)).unwrap();
        std::fs::write(dir.join(group).join(name), text).unwrap();
    }

    #[test]
    fn groups_and_names_come_from_the_folders_and_start_unconfirmed() {
        let d = tmp("tree");
        put(&d, "経理", "月次締め.toml", NET);
        put(&d, "共通", "バックアップ.toml", NET);
        put(&d, "共通", "_group.toml", "x = 1");
        put(&d, "共通", "notes.txt", "x");
        let defs = scan(&d, &Store::default());
        let ids: Vec<_> = defs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["共通/バックアップ", "経理/月次締め"]);
        assert!(defs.iter().all(|d| d.standing == Standing::Unconfirmed));
        assert_eq!(defs[1].label(), "月次締め");
        assert_eq!(defs[1].file, "経理/月次締め.toml");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn approval_holds_until_the_bytes_change_even_by_a_comment() {
        let d = tmp("approve");
        put(&d, "経理", "月次締め.toml", NET);
        let mut store = Store::default();
        let first = &scan(&d, &store)[0];
        store.approve(&first.id, &first.sha256, &first.text);
        assert_eq!(scan(&d, &store)[0].standing, Standing::Confirmed);

        put(&d, "経理", "月次締め.toml", &format!("{NET}# memo\n"));
        let now = &scan(&d, &store)[0];
        assert_eq!(now.standing, Standing::Unconfirmed);
        assert_eq!(now.previous.as_deref(), Some(NET), "the approved text is what the next approval compares with");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_that_does_not_read_is_invalid_and_never_unconfirmed() {
        let d = tmp("invalid");
        put(&d, "経理", "壊れ.toml", "format = 1\n[[job]]\nid = \"a\"\nrnu = []\n");
        let defs = scan(&d, &Store::default());
        assert_eq!(defs[0].standing, Standing::Invalid);
        assert_eq!(defs[0].problems[0].in_file(&defs[0].file), "経理/壊れ.toml の 4 行目: 知らない欄 rnu");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_approved_file_that_later_breaks_is_invalid() {
        let d = tmp("broken");
        put(&d, "g", "n.toml", NET);
        let mut store = Store::default();
        let first = &scan(&d, &store)[0];
        store.approve(&first.id, &first.sha256, &first.text);
        put(&d, "g", "n.toml", "format = 1\n[[job]]\nid = \"a\"\nrun = []\n");
        assert_eq!(scan(&d, &store)[0].standing, Standing::Invalid);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_folder_is_an_empty_list() {
        assert!(scan(Path::new("Z:/koyomi-no-such-dir"), &Store::default()).is_empty());
    }
}
