//! What `koyomi check` looks at beyond the file's own shape: whether what a job net
//! names is there on this machine. The executable of each job, its working folder (a relative
//! `dir`, and a job without one, in the folder `koyomi.toml`'s `work_dir` gives), the
//! folder a `kind = "file"` part watches, and how long the holiday table of the net's
//! calendar lasts. Texts with a `{date…}` in them are not looked at (they have no
//! name until the day). And a word on a monthly `day` some months do not have.

use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use crate::calendar::{self, Calendars};
use crate::jobnet::{Jobnet, Part};
use crate::when::{Kind, MonthRule};

/// One thing found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Only a word in time (the holiday table is running out), not a fault.
    pub warn: bool,
    pub text: String,
}

fn fault(text: String) -> Finding {
    Finding { warn: false, text }
}

/// The findings for `net`, with `path_var` the `PATH` to search (and `exts` the suffixes to try),
/// and `work` the folder a job without `dir` runs in (`koyomi.toml`'s `work_dir`; `None`: the
/// engine's own, which is not known here).
pub fn check(
    net: &Jobnet,
    cals: &Calendars,
    today: NaiveDate,
    warn_days: i64,
    path_var: &str,
    exts: &[&str],
    work: Option<&Path>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut without_dir = false;
    for j in &net.jobs {
        let exe = match (&j.shell, j.run.first()) {
            (Some(shell), _) => Some(shell.as_str()),
            (None, Some(exe)) => Some(exe.as_str()),
            _ => None,
        };
        if let Some(exe) = exe.filter(|e| !e.contains('{')) {
            if find_exe(exe, path_var, exts).is_none() {
                out.push(fault(format!("{} の実行ファイルが見つからない: {exe}", j.id)));
            }
        }
        if let Some(dir) = j.dir.as_deref().filter(|d| !d.contains('{')) {
            let at = work.map_or_else(|| PathBuf::from(dir), |w| w.join(dir));
            if !at.is_dir() {
                out.push(fault(format!("{} の作業フォルダーがない: {}", j.id, at.display())));
            }
        }
        without_dir |= j.dir.is_none() && j.part.runs_process() && !matches!(j.part, Part::Chore(_));
        if let Part::File { file, .. } = &j.part {
            if !file.contains('{') {
                let dir = watched_dir(file);
                if !dir.as_os_str().is_empty() && !dir.is_dir() {
                    out.push(fault(format!("{} が見張るフォルダーがない: {}", j.id, dir.display())));
                }
            }
        }
    }
    if let Some(w) = work.filter(|w| without_dir && !w.is_dir()) {
        out.push(fault(format!("dir のないジョブの作業フォルダーがない: {}（koyomi.toml の work_dir）", w.display())));
    }
    for w in &net.when {
        if let Kind::Month { rule: MonthRule::Day { day: day @ 29..=31, .. }, .. } = &w.kind {
            let months = match *day {
                31 => "2・4・6・9・11 月",
                30 => "2 月",
                _ => "うるう年でない 2 月",
            };
            out.push(Finding {
                warn: true,
                text: format!("注意: [[when]] の day = {day} は {day} 日のない月（{months}）には流れない。月末に流すなら day = \"last\""),
            });
        }
    }
    if let Some(until) = net.calendar.as_ref().and_then(|n| cals.map.get(n)).and_then(|c| c.until()) {
        if let Some(line) = calendar::caution(until, today, warn_days) {
            out.push(Finding { warn: today <= until, text: line });
        }
    }
    out
}

/// The folder a watched path starts in: everything before the first part with a wildcard.
fn watched_dir(pattern: &str) -> PathBuf {
    let p = Path::new(pattern);
    let has_wild = |c: &std::path::Component| c.as_os_str().to_string_lossy().contains(['*', '?']);
    let mut dir = PathBuf::new();
    for c in p.components() {
        if has_wild(&c) {
            return dir;
        }
        dir.push(c);
    }
    // No wildcard: the path is the file, its folder is what must be there.
    dir.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// Where `exe` is: as a path when it has a folder in it, else in the `PATH` folders (with each suffix of `exts`).
fn find_exe(exe: &str, path_var: &str, exts: &[&str]) -> Option<PathBuf> {
    let p = Path::new(exe);
    let with = |base: &Path| -> Option<PathBuf> {
        std::iter::once("").chain(exts.iter().copied()).map(|e| PathBuf::from(format!("{}{e}", base.display()))).find(|c| c.is_file())
    };
    if p.components().count() > 1 || p.is_absolute() {
        return with(p);
    }
    std::env::split_paths(path_var).find_map(|d| with(&d.join(exe)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobnet::parse;

    fn today() -> NaiveDate {
        "2026-10-30".parse().unwrap()
    }

    #[test]
    fn missing_exe_dir_and_watched_folder_are_named() {
        let tmp = std::env::temp_dir().join(format!("koyomi-pre-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let tool = tmp.join("tool.exe");
        std::fs::write(&tool, b"").unwrap();
        let t = tmp.display().to_string().replace(std::path::MAIN_SEPARATOR, "/");
        let text = format!(
            "format = 1\n[[job]]\nid = \"a\"\nrun = ['{t}/tool.exe']\ndir = '{t}'\n\
             [[job]]\nid = \"b\"\nafter = [\"a\"]\nrun = ['{t}/gone.exe']\ndir = '{t}/nope'\n\
             [[job]]\nid = \"c\"\nafter = [\"b\"]\nkind = \"file\"\nfile = '{t}/in/*.csv'\n\
             [[job]]\nid = \"d\"\nafter = [\"c\"]\nrun = [\"ok\", \"{{date}}\"]\ndir = '{t}/{{month}}'\n"
        );
        let net = parse(&text).unwrap();
        let got = check(&net, &Calendars::default(), today(), 60, &t, &[".exe"], None);
        let texts: Vec<_> = got.iter().map(|f| f.text.as_str()).collect();
        assert!(texts.iter().any(|x| x.starts_with("b の実行ファイルが見つからない")), "{texts:?}");
        assert!(texts.iter().any(|x| x.starts_with("b の作業フォルダーがない")), "{texts:?}");
        assert!(texts.iter().any(|x| x.starts_with("c が見張るフォルダーがない")), "{texts:?}");
        // `a` is all there, `d` finds `ok` on PATH only by its suffix (it is not there) but its dir has a token.
        assert!(!texts.iter().any(|x| x.starts_with("a ")), "{texts:?}");
        assert!(!texts.iter().any(|x| x.contains("d の作業")), "{texts:?}");
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    /// with `work_dir`, a relative `dir` is under its folder, and a job without `dir` needs it there.
    #[test]
    fn the_work_folder_holds_relative_dirs_and_jobs_without_one() {
        let tmp = std::env::temp_dir().join(format!("koyomi-pre3-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        let net =
            parse("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\ndir = \"sub\"\n[[job]]\nid = \"b\"\nafter = [\"a\"]\nrun = [\"x\"]\n")
                .unwrap();
        let texts = |work: &Path| -> Vec<String> {
            let found = check(&net, &Calendars::default(), today(), 60, "", &[], Some(work));
            found.into_iter().map(|f| f.text).filter(|t| !t.contains("実行ファイル")).collect()
        };
        assert_eq!(texts(&tmp), Vec::<String>::new());
        let gone = tmp.join("gone");
        assert_eq!(
            texts(&gone),
            [
                format!("a の作業フォルダーがない: {}", gone.join("sub").display()),
                format!("dir のないジョブの作業フォルダーがない: {}（koyomi.toml の work_dir）", gone.display())
            ]
        );
        // Every job has its `dir`: the folder itself is not needed.
        let placed = parse("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\ndir = \"sub\"\n").unwrap();
        assert!(check(&placed, &Calendars::default(), today(), 60, "", &[], Some(&tmp)).iter().all(|f| f.text.contains("実行ファイル")));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    /// `day = 29` to `31` runs in no month without that day: a word, not a fault; `last` gets none.
    #[test]
    fn a_month_day_some_months_lack_gets_a_word() {
        let words = |day: &str| -> Vec<Finding> {
            let net = parse(&format!("format = 1\n[[when]]\nevery = \"month\"\nday = {day}\nat = [\"21:00\"]\n")).unwrap();
            check(&net, &Calendars::default(), today(), 60, "", &[], None)
        };
        let w = words("31");
        assert_eq!(w.len(), 1);
        assert!(w[0].warn);
        assert_eq!(
            w[0].text,
            "注意: [[when]] の day = 31 は 31 日のない月（2・4・6・9・11 月）には流れない。月末に流すなら day = \"last\""
        );
        assert!(words("30")[0].text.contains("（2 月）"));
        assert!(words("29")[0].text.contains("（うるう年でない 2 月）"));
        assert!(words("28").is_empty() && words("\"last\"").is_empty());
    }

    #[test]
    fn a_bare_exe_is_looked_up_on_the_path_with_the_suffixes() {
        let tmp = std::env::temp_dir().join(format!("koyomi-pre2-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("tool.exe"), b"").unwrap();
        let p = tmp.display().to_string();
        assert!(find_exe("tool", &p, &[".exe"]).is_some());
        assert!(find_exe("tool", &p, &[]).is_none());
        assert!(find_exe("nothing", &p, &[".exe"]).is_none());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn the_watched_folder_stops_at_the_wildcard() {
        assert_eq!(watched_dir("D:/in/*.csv"), PathBuf::from("D:/in"));
        assert_eq!(watched_dir("D:/in/a.csv"), PathBuf::from("D:/in"));
        assert_eq!(watched_dir("D:/in/*/x/*.csv"), PathBuf::from("D:/in"));
    }
}
