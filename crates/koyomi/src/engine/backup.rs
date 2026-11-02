//! The daily backup: once a day, on a thread, everything in the settings
//! folder (the jobnets, the group files, the calendars and `koyomi.toml` with
//! its webhook addresses masked) is written as one bundle to
//! `backups/<YYYY-MM-DD>.toml` in the state folder, the shape `koyomi export
//! --all` writes and `koyomi import` reads. The seven newest are kept. The
//! last one is kept in `backup.toml`, where `koyomi engine status` shows it.
//!
//! A folder with nothing in it is not backed up: the backups of the days
//! before must not be rotated out by empty ones.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use chrono::NaiveDate;
use ito_common::kept;
use koyomi_core::bundle::{export, Pick, Source};
use serde::{Deserialize, Serialize};

/// How many day files `backups/` keeps.
pub const KEEP: usize = 7;

/// `backup.toml`: the last backup.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
struct Kept {
    /// When it ran, local time.
    #[serde(default)]
    at: String,
    /// `backups/<YYYY-MM-DD>.toml`; empty when nothing was there to back up.
    #[serde(default)]
    file: String,
    /// How many things the bundle carries.
    #[serde(default)]
    count: usize,
    /// What went wrong (the write, the rotation).
    #[serde(default)]
    errors: Vec<String>,
}

/// What the thread answers.
struct Done {
    file: String,
    count: usize,
    errors: Vec<String>,
}

pub struct Backup {
    app_dir: PathBuf,
    state_dir: PathBuf,
    /// The day the last backup was started for.
    done_for: Option<NaiveDate>,
    rx: Option<Receiver<Done>>,
    last: Kept,
    writer: kept::Writer,
}

impl Backup {
    /// Reads `backup.toml` from `state_dir`.
    pub fn new(app_dir: &Path, state_dir: &Path) -> Backup {
        let path = state_dir.join("backup.toml");
        let last: Kept = kept::read(&path).unwrap_or_default();
        Backup {
            app_dir: app_dir.to_owned(),
            state_dir: state_dir.to_owned(),
            done_for: None,
            rx: None,
            last,
            writer: kept::Writer::new(path, true),
        }
    }

    /// Starts today's backup when it has not been, and takes in the answer of
    /// the one running. The line to tell on the console, when one ended.
    pub fn tick(&mut self) -> Option<String> {
        let today = ito_common::now::now_local().date_naive();
        if self.rx.is_none() && self.done_for != Some(today) {
            self.done_for = Some(today);
            let (tx, rx) = mpsc::channel();
            let app = self.app_dir.clone();
            let dir = self.state_dir.join("backups");
            std::thread::spawn(move || {
                let _ = tx.send(write(&app, &dir, today));
            });
            self.rx = Some(rx);
        }
        let got = self.rx.as_ref().and_then(|rx| rx.try_recv().ok())?;
        self.rx = None;
        self.last = Kept {
            at: ito_common::now::now_local().format("%Y-%m-%d %H:%M:%S").to_string(),
            file: got.file,
            count: got.count,
            errors: got.errors,
        };
        self.writer.save_toml(&self.last);
        Some(self.line())
    }

    /// The last backup in a line, for `koyomi engine status`.
    pub fn line(&self) -> String {
        let k = &self.last;
        if k.at.is_empty() {
            return "バックアップ: まだ".to_owned();
        }
        let mut s = if k.file.is_empty() {
            format!("バックアップ: {}、定義がないので書かなかった", k.at)
        } else {
            format!("バックアップ: {}、{} に {} 件（{} 日分を残す）", k.at, k.file, k.count, KEEP)
        };
        if !k.errors.is_empty() {
            s.push_str(&format!("、できなかった {} 件: {}", k.errors.len(), k.errors.join(" / ")));
        }
        s
    }

    /// Writes the last backup's record before the engine ends.
    pub fn flush(&self) {
        if let Ok(text) = toml::to_string(&self.last) {
            let _ = self.writer.save_now(text);
        }
    }
}

/// Writes `dir/<today>.toml` from `app` and drops the day files beyond [`KEEP`].
fn write(app: &Path, dir: &Path, today: NaiveDate) -> Done {
    let src = Source::read(app);
    let bundle = match export(&src, &Pick { all: true, ..Default::default() }) {
        Ok(b) => b,
        Err(e) => return Done { file: String::new(), count: 0, errors: vec![e] },
    };
    if bundle.is_empty() {
        return Done { file: String::new(), count: 0, errors: Vec::new() };
    }
    let mut errors = Vec::new();
    let name = format!("{}.toml", today.format("%Y-%m-%d"));
    if let Err(e) = std::fs::create_dir_all(dir) {
        errors.push(format!("{} を作れない: {e}", dir.display()));
    } else if let Err(e) = ito_common::write_atomic(&dir.join(&name), bundle.to_toml()) {
        errors.push(e);
    } else {
        errors.extend(rotate(dir));
    }
    Done { file: format!("backups/{name}"), count: bundle.len(), errors }
}

/// Removes the day files of `dir` beyond the [`KEEP`] newest.
fn rotate(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut days: Vec<PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(is_day_file)).collect();
    days.sort();
    days.reverse();
    days.iter().skip(KEEP).filter_map(|p| std::fs::remove_file(p).err().map(|e| format!("{} を消せない: {e}", p.display()))).collect()
}

/// `2026-10-30.toml`.
fn is_day_file(name: &str) -> bool {
    name.strip_suffix(".toml").is_some_and(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backup_is_written_once_a_day_and_seven_are_kept() {
        let root = std::env::temp_dir().join(format!("koyomi-backup-engine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let app = root.join("app");
        let state = root.join("state");
        std::fs::create_dir_all(app.join("jobnets/経理")).unwrap();
        std::fs::create_dir_all(state.join("backups")).unwrap();
        for d in 1..=9 {
            std::fs::write(state.join("backups").join(format!("2020-01-0{d}.toml")), "format = 1\n").unwrap();
        }
        std::fs::write(state.join("backups").join("note.txt"), "x").unwrap();
        // Nothing there: no file, and the old ones stay.
        let mut b = Backup::new(&app, &state);
        assert_eq!(b.line(), "バックアップ: まだ");
        let line = wait(&mut b);
        assert!(line.contains("書かなかった"), "{line}");
        assert_eq!(std::fs::read_dir(state.join("backups")).unwrap().count(), 10);
        assert_eq!(b.tick(), None);
        // Something there, another day: written and the oldest rotated out.
        std::fs::write(app.join("jobnets/経理/月次.toml"), "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n").unwrap();
        b.done_for = None;
        let line = wait(&mut b);
        assert!(line.contains("1 件"), "{line}");
        let today = ito_common::now::now_local().date_naive();
        let file = state.join("backups").join(format!("{}.toml", today.format("%Y-%m-%d")));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("[[jobnet]]") && text.contains("経理/月次"), "{text}");
        let mut names: Vec<String> =
            std::fs::read_dir(state.join("backups")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names.len(), KEEP + 1, "{names:?}");
        assert!(names.contains(&"note.txt".to_owned()));
        assert!(!names.contains(&"2020-01-01.toml".to_owned()) && !names.contains(&"2020-01-03.toml".to_owned()));
        b.flush();
        let back = Backup::new(&app, &state);
        assert_eq!(back.last.count, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn wait(b: &mut Backup) -> String {
        for _ in 0..300 {
            if let Some(line) = b.tick() {
                return line;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the backup answers");
    }
}
