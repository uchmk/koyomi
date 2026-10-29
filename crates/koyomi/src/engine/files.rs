//! Finding the files a `kind = "file"` part and a `[[when]] file` wait for:
//! a path, or a folder and a name with `*` / `?`.

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
    if !name.contains(['*', '?']) {
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
            (m.is_file() && glob_match(&name, &n)).then(|| (e.path(), m.len(), m.modified().ok()))
        })
        .collect();
    hits.sort_by(|a, b| a.0.cmp(&b.0));
    hits
}

/// The folder `pattern` looks in: its parent, or `.` for a bare name.
pub fn folder_of(pattern: &str) -> PathBuf {
    Path::new(pattern).parent().filter(|p| !p.as_os_str().is_empty()).map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// `*` (any run of characters) and `?` (any one character) in `pattern`,
/// against the whole of `name`; letters' case does not count on Windows.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let fold = |s: &str| if cfg!(windows) { s.to_lowercase() } else { s.to_owned() };
    let (p, n): (Vec<char>, Vec<char>) = (fold(pattern).chars().collect(), fold(name).chars().collect());
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ni));
            pi += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_match_takes_star_and_question_mark() {
        assert!(glob_match("*.csv", "sales.csv"));
        assert!(glob_match("sales-??.csv", "sales-01.csv"));
        assert!(!glob_match("sales-??.csv", "sales-001.csv"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("a*b*c", "aXXbYYc"));
        assert!(!glob_match("a*b*c", "aXXbYY"));
        assert!(!glob_match("*.csv", "sales.txt"));
        assert_eq!(glob_match("*.CSV", "sales.csv"), cfg!(windows));
    }

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
        assert_eq!(find_file(&pattern).unwrap().0, dir.join("a.csv"));
        let plain = dir.join("c.txt").display().to_string();
        assert_eq!(find_files(&plain).len(), 1);
        assert!(find_files(&dir.join("none.txt").display().to_string()).is_empty());
        assert!(find_files(&dir.join("gone").join("*.csv").display().to_string()).is_empty());
        assert_eq!(folder_of(&pattern), dir);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
