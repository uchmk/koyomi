//! `koyomi file run`: what a `delete`, an `empty` or a `copy` runs. It reads the job
//! from the `KOYOMI_FILE_*` the engine sets and does the work with ito-fileops, in
//! this process: no PowerShell, no robocopy, so the engine runs the same on Debian.
//! Each text (the folders, the `match`) comes with the `|` the engine puts before it,
//! so a folder that starts with `secret:` is not read as a secret; it is dropped here.
//!
//! * `delete`: the files in a folder (not below it) that match `match` (every file
//!   without one) and were last written more than `keep_days` days ago; one line
//!   `deleted: <path>` each, then `<n> file(s) deleted`.
//! * `empty`: everything in a folder goes, the folder stays; one line `removed:
//!   <path>` for each top-level item, then `<n> item(s) removed`.
//! * `copy`: the folder `from` and the folders below it into `into`, only the files
//!   `match` names when it is written; nothing in `into` is deleted. A file with the
//!   same size and last-write time is not copied again. One line for each file
//!   copied, folder made, extra or mismatched item and failure, then one line of
//!   totals.
//!
//! Against the PowerShell they replace: `delete` takes hidden and system files too,
//! and leaves a file that is a link alone (`skipped:`); `empty` removes a link or a
//! junction inside as a link, and what it points to stays; `copy` does not follow a
//! folder link or junction (robocopy does), copies a file link's content, and never
//! writes through a link in `into`. A day is 24 hours. The log is a line for each
//! item and a last line of totals, not robocopy's table.
//!
//! It ends with 0 when it was done, with 1 (a warning) when a `copy` met a file and
//! a folder of the same name and left them alone, and else with
//! [`NO_EXIT`](crate::ssh) and why in the file `KOYOMI_REASON` names: a folder that is
//! missing or a link, a destination that cannot be made or overlaps the source
//! (`KOYOMI-E3264`), or files that failed after their retry (`KOYOMI-E3274`; the rest
//! was copied). A stop comes between files (an `empty` between its top-level items),
//! or while a failed file waits for its retry; a file in hand may be left half written,
//! which the next run copies again (its size or time differs).

use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use ito_fileops::{Event, Skip, TreeOptions, TreeResult};
use koyomi_core::chore::{folder_path, Chore};
use koyomi_core::msg::{self, line, note};

pub fn cmd(args: &[String]) -> i32 {
    match args {
        [op] if op == "run" => crate::ssh::run(exec),
        _ => {
            crate::say("koyomi: file takes run (what a delete, empty or copy job does)", true);
            2
        }
    }
}

/// Reads the job from the environment and runs it.
fn exec(stop: &AtomicBool) -> Result<i32, String> {
    let job = job_from(|name| crate::sftp::env_var("KOYOMI_FILE_", name))?;
    go(&job, &TreeOptions::default(), stop, &mut |l| crate::say(l, false))
}

/// The job `var` (a lookup of the `KOYOMI_FILE_*` by the name after the prefix, trimmed)
/// describes, checked.
fn job_from(var: impl Fn(&str) -> Option<String>) -> Result<Chore, String> {
    let text = |name: &str| var(name).map(|v| crate::mail::text(&v).trim().to_owned());
    let op =
        var("OP").ok_or_else(|| line(msg::JOB_ENV_MISSING, &["KOYOMI_FILE_OP", "kind = \"delete\", \"empty\" or \"copy\"", "file"]))?;
    let job = match op.as_str() {
        "delete" => {
            let keep_days = var("KEEP_DAYS").and_then(|v| v.parse::<u32>().ok()).filter(|d| *d > 0);
            Chore::Delete {
                path: text("PATH").unwrap_or_default(),
                pattern: text("MATCH").filter(|m| !m.is_empty()),
                keep_days: keep_days.ok_or_else(|| line(msg::CHORE_KEEP_DAYS, &[]))?,
            }
        }
        "empty" => Chore::Empty { path: text("PATH").unwrap_or_default() },
        "copy" => Chore::Copy {
            from: text("FROM").unwrap_or_default(),
            into: text("INTO").unwrap_or_default(),
            pattern: text("MATCH").filter(|m| !m.is_empty()),
        },
        _ => return Err(line(msg::DEF_CHOICE_BAD, &["KOYOMI_FILE_OP", "\"delete\", \"empty\", \"copy\""])),
    };
    // Checked again: the engine has filled in each `{…}` of the paths since the net was.
    job.check()?;
    Ok(job)
}

/// The words of a `Skip`.
fn why(s: Skip) -> &'static str {
    match s {
        Skip::Link => "a link, not followed",
        Skip::Special => "not a file or folder",
        Skip::NotUnicode => "a name that is not Unicode",
        _ => "passed over",
    }
}

/// Logs what `ito-fileops` does that every kind shares; true when it was one of those.
fn common(e: &Event, out: &mut dyn FnMut(&str)) -> bool {
    match e {
        Event::Skipped { path, why: w } => out(&format!("skipped: {} ({})", path.display(), why(*w))),
        Event::CleanupFailed { path, error } => {
            out(&format!("! cleanup failed: {} ({})", path.display(), ito_common::ioerror::detail(error)))
        }
        _ => return false,
    }
    true
}

/// The last line of a `copy`.
fn summary(r: &TreeResult) -> String {
    format!(
        "{} copied, {} same, {} extra file(s), {} extra folder(s), {} mismatched, {} failed, {} skipped, {} folder(s) made, {} bytes",
        r.copied, r.same, r.extra_files, r.extra_dirs, r.mismatched, r.failed, r.skipped, r.folders_made, r.bytes
    )
}

/// Runs `job` (a `delete`, `empty` or `copy`, its `{…}` filled in) and says how it
/// ended: 0, 1 for the warning of a `copy`, or why not. Each line of the log goes to `out`.
fn go(job: &Chore, opts: &TreeOptions, stop: &AtomicBool, out: &mut dyn FnMut(&str)) -> Result<i32, String> {
    // ito-fileops looks at the flag in its loops only: an empty folder would be `Done`, a
    // missing one `Failed`, and a copy would make its destination first. The summary is
    // still said, with nothing done.
    if stop.load(std::sync::atomic::Ordering::Relaxed) {
        match job {
            Chore::Delete { .. } => out("0 file(s) deleted"),
            Chore::Empty { .. } => out("0 item(s) removed"),
            _ => out(&summary(&TreeResult::default())),
        }
        return Err(note(msg::N_STOPPED_FILE, &[]));
    }
    let op = job.key();
    let failed = |e: &ito_fileops::Error| line(msg::FILE_JOB_FAILED, &[op, &e.words(&msg::language())]);
    match job {
        Chore::Delete { path, pattern, keep_days } => {
            let older = Duration::from_secs(u64::from(*keep_days) * 86_400);
            let done =
                ito_fileops::delete_old(&folder_path(path), pattern.as_deref().unwrap_or("*"), older, SystemTime::now(), stop, &mut |e| {
                    if let Event::Deleted(p) = e {
                        out(&format!("deleted: {}", p.display()));
                    } else {
                        common(&e, &mut *out);
                    }
                });
            out(&format!("{} file(s) deleted", done.paths.len()));
            ended(&done.end, &failed)?;
            Ok(0)
        }
        Chore::Empty { path } => {
            let done = ito_fileops::empty(&folder_path(path), stop, &mut |e| {
                if let Event::Deleted(p) = e {
                    out(&format!("removed: {}", p.display()));
                } else {
                    common(&e, &mut *out);
                }
            });
            out(&format!("{} item(s) removed", done.removed));
            ended(&done.end, &failed)?;
            Ok(0)
        }
        Chore::Copy { from, into, pattern } => {
            let mut first: Option<String> = None;
            let done = ito_fileops::copy_tree(&folder_path(from), &folder_path(into), pattern.as_deref(), opts, stop, &mut |e| match e {
                Event::Copied { from, to, bytes } => out(&format!("copied: {} -> {} ({bytes} bytes)", from.display(), to.display())),
                Event::MadeFolder(p) => out(&format!("made folder: {}", p.display())),
                Event::ExtraFile(p) => out(&format!("extra file (kept): {}", p.display())),
                Event::ExtraDir(p) => out(&format!("extra folder (kept): {}", p.display())),
                Event::Mismatch { from, to } => out(&format!("! mismatch (left alone): {} / {}", from.display(), to.display())),
                Event::Retrying { path, attempt, error } => {
                    out(&format!("! retry {attempt}: {} ({})", path.display(), ito_common::ioerror::detail(error)))
                }
                Event::Failed { path, error } => {
                    out(&format!("! failed: {} ({})", path.display(), ito_common::ioerror::detail(error)));
                    first.get_or_insert_with(|| format!("{} ({})", path.display(), msg::io_words(error)));
                }
                Event::Same(_) => {}
                other => {
                    common(&other, &mut *out);
                }
            });
            out(&summary(&done));
            ended(&done.end, &failed)?;
            if done.failed > 0 {
                return Err(line(msg::COPY_SOME_FAILED, &[&done.failed.to_string(), first.as_deref().unwrap_or("")]));
            }
            Ok(i32::from(done.mismatched > 0))
        }
        _ => Err(line(msg::DEF_CHOICE_BAD, &["KOYOMI_FILE_OP", "\"delete\", \"empty\", \"copy\""])),
    }
}

/// Turns the end of an operation into an error unless it was done.
fn ended(end: &ito_fileops::End, failed: &dyn Fn(&ito_fileops::Error) -> String) -> Result<(), String> {
    match end {
        ito_fileops::End::Done => Ok(()),
        ito_fileops::End::Stopped => Err(note(msg::N_STOPPED_FILE, &[])),
        ito_fileops::End::Failed(e) => Err(failed(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Dir {
            let dir = std::env::temp_dir().join(format!("koyomi-file-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Dir(dir)
        }

        fn at(&self, p: &str) -> PathBuf {
            self.0.join(p)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write(p: &Path, body: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn age(p: &Path, days: u64) {
        let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(days * 86_400)).unwrap();
    }

    fn fast() -> TreeOptions {
        TreeOptions { wait: Duration::from_millis(10), ..Default::default() }
    }

    fn run(job: &Chore) -> Result<i32, String> {
        run_lines(job).0
    }

    /// The end of `job` and the lines it said.
    fn run_lines(job: &Chore) -> (Result<i32, String>, Vec<String>) {
        let mut lines = Vec::new();
        let r = go(job, &fast(), &AtomicBool::new(false), &mut |l| lines.push(l.to_owned()));
        (r, lines)
    }

    /// A link to a folder: a junction on Windows (no admin needed), a symlink elsewhere.
    fn link_dir(target: &Path, link: &Path) {
        #[cfg(windows)]
        {
            let out = std::process::Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(link).arg(target).output().unwrap();
            assert!(out.status.success(), "mklink /J failed: {}", String::from_utf8_lossy(&out.stdout));
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    fn delete(path: &Path, pattern: Option<&str>) -> Chore {
        Chore::Delete { path: path.display().to_string(), pattern: pattern.map(str::to_owned), keep_days: 30 }
    }

    fn copy(from: &Path, into: &Path, pattern: Option<&str>) -> Chore {
        Chore::Copy { from: from.display().to_string(), into: into.display().to_string(), pattern: pattern.map(str::to_owned) }
    }

    #[test]
    fn a_delete_removes_the_old_files_that_match() {
        let dir = Dir::new("delete");
        let logs = dir.at("logs");
        for n in ["a.log", "b.log", "c.txt"] {
            write(&logs.join(n), "x");
        }
        age(&logs.join("a.log"), 60);
        age(&logs.join("c.txt"), 60);
        let (r, lines) = run_lines(&delete(&logs, Some("*.log")));
        assert_eq!(r, Ok(0));
        assert_eq!(lines, [format!("deleted: {}", logs.join("a.log").display()), "1 file(s) deleted".to_owned()]);
        assert!(!logs.join("a.log").exists() && logs.join("b.log").exists() && logs.join("c.txt").exists());
        // Without a pattern every old file goes.
        assert_eq!(run(&delete(&logs, None)), Ok(0));
        assert!(!logs.join("c.txt").exists() && logs.join("b.log").exists());
        let e = run(&delete(&dir.at("missing"), None)).unwrap_err();
        assert!(e.starts_with("KOYOMI-E3264") && e.contains("missing"), "{e}");
    }

    #[test]
    fn an_empty_leaves_the_folder_and_nothing_in_it() {
        let dir = Dir::new("empty");
        let out = dir.at("out");
        write(&out.join("a.txt"), "1");
        write(&out.join("sub/b.txt"), "2");
        assert_eq!(run(&Chore::Empty { path: out.display().to_string() }), Ok(0));
        assert!(out.is_dir() && std::fs::read_dir(&out).unwrap().next().is_none());
        let e = run(&Chore::Empty { path: dir.at("missing").display().to_string() }).unwrap_err();
        assert!(e.starts_with("KOYOMI-E3264"), "{e}");
    }

    #[test]
    fn a_copy_makes_the_tree_and_keeps_what_only_the_destination_has() {
        let dir = Dir::new("copy");
        let (from, into) = (dir.at("from"), dir.at("into"));
        write(&from.join("a.csv"), "1");
        write(&from.join("sub/b.txt"), "22");
        std::fs::create_dir_all(from.join("empty")).unwrap();
        assert_eq!(run(&copy(&from, &into, None)), Ok(0));
        assert_eq!(std::fs::read_to_string(into.join("sub/b.txt")).unwrap(), "22");
        assert!(into.join("a.csv").is_file() && into.join("empty").is_dir());
        // Again: nothing to do. A file only in `into` stays.
        write(&into.join("only.txt"), "z");
        let (r, lines) = run_lines(&copy(&from, &into, None));
        assert_eq!(r, Ok(0));
        assert_eq!(
            lines.last().map(String::as_str),
            Some("0 copied, 2 same, 1 extra file(s), 0 extra folder(s), 0 mismatched, 0 failed, 0 skipped, 0 folder(s) made, 0 bytes")
        );
        assert!(into.join("only.txt").is_file());
    }

    #[test]
    fn a_copy_with_a_match_takes_those_files_and_all_the_folders() {
        let dir = Dir::new("match");
        let (from, into) = (dir.at("from"), dir.at("into"));
        write(&from.join("a.csv"), "1");
        write(&from.join("a.txt"), "2");
        write(&from.join("sub/b.csv"), "3");
        write(&from.join("sub/b.txt"), "4");
        assert_eq!(run(&copy(&from, &into, Some("*.csv"))), Ok(0));
        assert!(into.join("a.csv").is_file() && into.join("sub/b.csv").is_file());
        assert!(!into.join("a.txt").exists() && !into.join("sub/b.txt").exists());
    }

    #[test]
    fn a_copy_warns_for_a_file_over_a_folder_and_fails_for_what_it_cannot_start() {
        let dir = Dir::new("warn");
        let (from, into) = (dir.at("from"), dir.at("into"));
        write(&from.join("x"), "file");
        std::fs::create_dir_all(into.join("x")).unwrap();
        assert_eq!(run(&copy(&from, &into, None)), Ok(1));
        let e = run(&copy(&dir.at("missing"), &into, None)).unwrap_err();
        assert!(e.starts_with("KOYOMI-E3264"), "{e}");
        // `into` inside `from`.
        let e = run(&copy(&from, &from.join("deeper"), None)).unwrap_err();
        assert!(e.starts_with("KOYOMI-E3264"), "{e}");
    }

    #[cfg(windows)]
    #[test]
    fn a_copy_that_cannot_write_a_file_copies_the_rest_and_says_which() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = Dir::new("locked");
        let (from, into) = (dir.at("from"), dir.at("into"));
        write(&from.join("a.txt"), "new");
        write(&from.join("b.txt"), "other");
        write(&into.join("a.txt"), "old!");
        age(&into.join("a.txt"), 10);
        // The handle that set the time is closed (age() drops it); this one stays open and shares nothing.
        let held = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(into.join("a.txt")).unwrap();
        let e = run(&copy(&from, &into, None)).unwrap_err();
        drop(held);
        assert!(e.starts_with("KOYOMI-E3274") && e.contains("a.txt"), "{e}");
        assert_eq!(std::fs::read_to_string(into.join("b.txt")).unwrap(), "other");
    }

    #[test]
    fn a_stop_before_the_start_changes_nothing() {
        let dir = Dir::new("stop");
        let (from, into) = (dir.at("from"), dir.at("into"));
        write(&from.join("a.log"), "1");
        age(&from.join("a.log"), 60);
        let stop = AtomicBool::new(true);
        let stopped = Err(note(msg::N_STOPPED_FILE, &[]));
        let said = |job: &Chore| {
            let mut lines = Vec::new();
            let r = go(job, &fast(), &stop, &mut |l| lines.push(l.to_owned()));
            (r, lines)
        };
        assert_eq!(said(&delete(&from, None)), (stopped.clone(), vec!["0 file(s) deleted".to_owned()]));
        assert_eq!(said(&Chore::Empty { path: from.display().to_string() }), (stopped.clone(), vec!["0 item(s) removed".to_owned()]));
        assert_eq!(
            said(&copy(&from, &into, None)),
            (
                stopped,
                vec!["0 copied, 0 same, 0 extra file(s), 0 extra folder(s), 0 mismatched, 0 failed, 0 skipped, 0 folder(s) made, 0 bytes"
                    .to_owned()]
            )
        );
        assert!(from.join("a.log").exists() && !into.exists());
    }

    /// A folder that is a link is refused however its path ends: `empty` and `delete` must not
    /// reach into what it points to (a trailing separator makes some systems follow the link).
    #[test]
    fn a_link_is_refused_with_a_separator_or_a_dot_at_the_end() {
        let dir = Dir::new("link");
        let (target, link) = (dir.at("target"), dir.at("link"));
        write(&target.join("keep.txt"), "x");
        age(&target.join("keep.txt"), 60);
        link_dir(&target, &link);
        let sep = std::path::MAIN_SEPARATOR;
        for end in [format!("{sep}"), format!("{sep}."), format!("{sep}.{sep}")] {
            let path = format!("{}{end}", link.display());
            let e = run(&Chore::Empty { path: path.clone() });
            assert!(e.as_ref().is_err_and(|e| e.starts_with("KOYOMI-E3264")), "empty {path:?}: {e:?}");
            assert!(target.join("keep.txt").exists(), "empty {path:?} reached into the link target");
            let e = run(&Chore::Delete { path: path.clone(), pattern: None, keep_days: 30 });
            assert!(e.as_ref().is_err_and(|e| e.starts_with("KOYOMI-E3264")), "delete {path:?}: {e:?}");
            assert!(target.join("keep.txt").exists(), "delete {path:?} reached into the link target");
        }
        // Without it, as before.
        assert!(run(&Chore::Empty { path: link.display().to_string() }).is_err());
        assert!(target.join("keep.txt").exists());
    }

    fn job(pairs: &[(&str, &str)]) -> Result<Chore, String> {
        let pairs: Vec<(String, String)> = pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        job_from(|name| pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()))
    }

    #[test]
    fn the_job_is_read_from_the_environment() {
        assert_eq!(
            job(&[("OP", "delete"), ("PATH", "|D:\\logs"), ("MATCH", "|*.log"), ("KEEP_DAYS", "7")]),
            Ok(Chore::Delete { path: "D:\\logs".into(), pattern: Some("*.log".into()), keep_days: 7 })
        );
        assert_eq!(job(&[("OP", "empty"), ("PATH", "|D:\\tmp")]), Ok(Chore::Empty { path: "D:\\tmp".into() }));
        assert_eq!(
            job(&[("OP", "copy"), ("FROM", "|D:\\a"), ("INTO", "|E:\\b")]),
            Ok(Chore::Copy { from: "D:\\a".into(), into: "E:\\b".into(), pattern: None })
        );
        // The marker keeps `secret:` from being read as a secret; here it is a folder's name.
        assert_eq!(job(&[("OP", "empty"), ("PATH", "|secret:logs")]), Ok(Chore::Empty { path: "secret:logs".into() }));
        assert!(msg::JOB_ENV_MISSING.is_in(&job(&[]).unwrap_err()));
        assert_eq!(job(&[("OP", "move")]), Err(line(msg::DEF_CHOICE_BAD, &["KOYOMI_FILE_OP", "\"delete\", \"empty\", \"copy\""])));
        for days in [Some("0"), Some("x"), None] {
            let mut v = vec![("OP", "delete"), ("PATH", "|D:\\logs")];
            v.extend(days.map(|d| ("KEEP_DAYS", d)));
            assert_eq!(job(&v), Err(line(msg::CHORE_KEEP_DAYS, &[])));
        }
        let e = job(&[("OP", "empty"), ("PATH", "|C:\\")]).unwrap_err();
        assert!(msg::CHORE_NOT_ROOT.is_in(&e), "{e}");
    }
}
