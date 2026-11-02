//! Finding the files a `kind = "file"` part and a `[[when]] file` wait for:
//! a path, or a folder and a name with `*` / `?` (ito-sftp's rule, as for SFTP).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A file as found: its path, size and change time.
pub type Found = (PathBuf, u64, Option<SystemTime>);

/// The file `pattern` names: a path, or a folder and a name with `*` / `?`
/// (the first by name when several match). Its size and change time with it.
pub fn find_file(pattern: &str) -> Option<Found> {
    find_files(pattern).into_iter().next()
}

/// Every file `pattern` names, by name. A path without `*` / `?` is at most
/// one file; a folder that cannot be read is no file.
pub fn find_files(pattern: &str) -> Vec<Found> {
    let pat = Path::new(pattern);
    let Some(name) = pat.file_name().map(|n| n.to_string_lossy().into_owned()) else { return Vec::new() };
    if !ito_sftp::has_wildcard(&name) {
        return match std::fs::metadata(pat).ok().filter(|m| m.is_file()) {
            Some(meta) => vec![(pat.to_path_buf(), meta.len(), meta.modified().ok())],
            None => Vec::new(),
        };
    }
    let dir = pat.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut hits: Vec<Found> = entries
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let m = e.metadata().ok()?;
            (m.is_file() && ito_sftp::glob_match(&name, &n, cfg!(windows))).then(|| (e.path(), m.len(), m.modified().ok()))
        })
        .collect();
    hits.sort_by(|a, b| a.0.cmp(&b.0));
    hits
}

/// The folder `pattern` looks in: its parent, or `.` for a bare name.
pub fn folder_of(pattern: &str) -> PathBuf {
    Path::new(pattern).parent().filter(|p| !p.as_os_str().is_empty()).map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_files_lists_the_matches_by_name_and_a_plain_path_is_one_file() {
        let dir = std::env::temp_dir().join(format!("koyomi-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["b.csv", "a.csv", "c.txt"] {
            std::fs::write(dir.join(n), "x").unwrap();
        }
        let pattern = dir.join("*.csv").display().to_string();
        let names: Vec<String> = find_files(&pattern).iter().map(|f| f.0.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["a.csv", "b.csv"]);
        // A name's case does not count on Windows.
        assert_eq!(find_files(&dir.join("*.CSV").display().to_string()).len(), if cfg!(windows) { 2 } else { 0 });
        assert_eq!(find_files(&dir.join("?.csv").display().to_string()).len(), 2);
        assert_eq!(find_file(&pattern).unwrap().0, dir.join("a.csv"));
        let plain = dir.join("c.txt").display().to_string();
        assert_eq!(find_files(&plain).len(), 1);
        assert!(find_files(&dir.join("none.txt").display().to_string()).is_empty());
        assert!(find_files(&dir.join("gone").join("*.csv").display().to_string()).is_empty());
        assert_eq!(folder_of(&pattern), dir);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
