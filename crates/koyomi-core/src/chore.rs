//! The everyday file chores a net can have as parts without a script:
//! delete old files, zip, copy with robocopy, empty a folder. Each is
//! written in the file as its own `kind` and runs as a PowerShell script the
//! engine writes when the run starts, so it has the job's slot, log,
//! `timeout` and `retry`.

use serde::{Deserialize, Serialize};

use crate::jobnet::{Jobnet, Judge, Part, Range};

/// One chore and what it works on. Paths may have `{date}` and `{param:…}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Chore {
    /// 古いファイルを消す: the files in `path` (not below it) matching `pattern`
    /// that were last written more than `keep_days` days ago.
    Delete {
        path: String,
        #[serde(default)]
        pattern: Option<String>,
        keep_days: u32,
    },
    /// ZIP にする: `from` (a file, a folder or a wildcard) into the `.zip` file `into`, replaced.
    Zip { from: String, into: String },
    /// robocopy で写す: the folder `from` and the folders below it into `into`; only
    /// the files matching `pattern` when it is there. Nothing is deleted in `into`.
    Copy {
        from: String,
        into: String,
        #[serde(default)]
        pattern: Option<String>,
    },
    /// フォルダーを空ける: everything in `path` goes; the folder stays.
    Empty { path: String },
}

/// The `kind` of each chore, in the order of the editor's shelf.
pub const KINDS: [&str; 4] = ["delete", "zip", "copy", "empty"];

impl Chore {
    /// The `kind` as written, and the key of its word.
    pub fn key(&self) -> &'static str {
        match self {
            Chore::Delete { .. } => "delete",
            Chore::Zip { .. } => "zip",
            Chore::Copy { .. } => "copy",
            Chore::Empty { .. } => "empty",
        }
    }

    /// A new, empty one of `kind` (for the shelf).
    pub fn blank(kind: &str) -> Option<Chore> {
        Some(match kind {
            "delete" => Chore::Delete { path: String::new(), pattern: None, keep_days: 30 },
            "zip" => Chore::Zip { from: String::new(), into: String::new() },
            "copy" => Chore::Copy { from: String::new(), into: String::new(), pattern: None },
            "empty" => Chore::Empty { path: String::new() },
            _ => return None,
        })
    }

    /// Every path of it, to fill in `{date}` and `{param:…}`.
    pub fn paths_mut(&mut self) -> Vec<(&'static str, &mut String)> {
        match self {
            Chore::Delete { path, .. } | Chore::Empty { path } => vec![("path", path)],
            Chore::Zip { from, into } | Chore::Copy { from, into, .. } => vec![("from", from), ("into", into)],
        }
    }

    /// Every path of it, as it is.
    pub fn paths(&self) -> Vec<&String> {
        match self {
            Chore::Delete { path, .. } | Chore::Empty { path } => vec![path],
            Chore::Zip { from, into } | Chore::Copy { from, into, .. } => vec![from, into],
        }
    }

    /// Why it may not run as it stands: a path that is empty, a drive's root
    /// for `delete` and `empty`, a zip that does not end in `.zip`.
    pub fn check(&self) -> Result<(), String> {
        let need = |name: &str, p: &str| if p.trim().is_empty() { Err(format!("{name} が空")) } else { Ok(()) };
        match self {
            Chore::Delete { path, keep_days, .. } => {
                need("path", path)?;
                not_root(path)?;
                if *keep_days == 0 {
                    return Err("keep_days は 1 以上（全部を消すなら kind = \"empty\"）".into());
                }
            }
            Chore::Empty { path } => {
                need("path", path)?;
                not_root(path)?;
            }
            Chore::Zip { from, into } => {
                need("from", from)?;
                need("into", into)?;
                if !into.trim().to_ascii_lowercase().ends_with(".zip") {
                    return Err(format!("into は .zip のファイル: {into}"));
                }
            }
            Chore::Copy { from, into, .. } => {
                need("from", from)?;
                need("into", into)?;
            }
        }
        Ok(())
    }

    /// The PowerShell script that does it, and how its exit code is judged.
    /// The paths are already filled in.
    pub fn script(&self) -> (String, Option<Judge>) {
        match self {
            Chore::Delete { path, pattern, keep_days } => {
                let s = format!(
                    "$ErrorActionPreference = 'Stop'\n\
                     $limit = (Get-Date).AddDays(-{keep_days})\n\
                     $old = @(Get-ChildItem -LiteralPath {p} -Filter {f} -File | Where-Object {{ $_.LastWriteTime -lt $limit }})\n\
                     foreach ($x in $old) {{ Remove-Item -LiteralPath $x.FullName -Force; Write-Output \"deleted: $($x.FullName)\" }}\n\
                     Write-Output \"$($old.Count) file(s) deleted\"\n",
                    p = quote(path),
                    f = quote(pattern.as_deref().unwrap_or("*")),
                );
                (s, None)
            }
            Chore::Zip { from, into } => {
                let s = format!(
                    "$ErrorActionPreference = 'Stop'\n\
                     $into = {i}\n\
                     $dir = Split-Path -Parent $into\n\
                     if ($dir) {{ New-Item -ItemType Directory -Force -Path $dir | Out-Null }}\n\
                     Compress-Archive -Path {f} -DestinationPath $into -Force\n\
                     Write-Output \"zipped: $into\"\n",
                    i = quote(into),
                    f = quote(from),
                );
                (s, None)
            }
            Chore::Copy { from, into, pattern } => {
                let only = pattern.as_deref().map(|p| format!(" {}", quote(p))).unwrap_or_default();
                // robocopy: 0–3 copied or nothing to do, 4–7 something did not match, 8 and up failed.
                let s = format!(
                    "& robocopy {f} {i}{only} /E /R:1 /W:1 /NP\n\
                     $c = $LASTEXITCODE\n\
                     if ($c -ge 8) {{ exit 2 }}\n\
                     if ($c -ge 4) {{ exit 1 }}\n\
                     exit 0\n",
                    f = quote(&folder(from)),
                    i = quote(&folder(into)),
                );
                (s, Some(Judge { warn: vec![Range { from: 1, to: 1 }], ..Default::default() }))
            }
            Chore::Empty { path } => {
                let s = format!(
                    "$ErrorActionPreference = 'Stop'\n\
                     $all = @(Get-ChildItem -LiteralPath {p} -Force)\n\
                     foreach ($x in $all) {{ Remove-Item -LiteralPath $x.FullName -Recurse -Force }}\n\
                     Write-Output \"$($all.Count) item(s) removed\"\n",
                    p = quote(path),
                );
                (s, None)
            }
        }
    }
}

/// Gives each chore of `net` (its paths filled in) the script that does it, so it
/// runs as a PowerShell job; a root or an empty path after the filling stops the run.
pub fn fill(net: &mut Jobnet) -> Result<(), String> {
    for j in &mut net.jobs {
        if let Part::Chore(c) = &j.part {
            c.check().map_err(|e| format!("{}: {e}", j.id))?;
            let (script, judge) = c.script();
            j.shell = Some("powershell".to_owned());
            j.script = Some(script);
            j.judge = judge;
            j.run.clear();
        }
    }
    Ok(())
}

/// A PowerShell string that is `s` as it is: `'…'`, a `'` doubled.
fn quote(s: &str) -> String {
    format!("'{}'", s.trim().replace('\'', "''"))
}

/// A folder for robocopy: no `\` at the end (it would escape the closing quote),
/// but a drive's root keeps one as `D:\.`.
fn folder(p: &str) -> String {
    let t = p.trim().trim_end_matches(['\\', '/']);
    if t.len() == 2 && t.ends_with(':') {
        format!("{t}\\.")
    } else {
        t.to_owned()
    }
}

/// `delete` and `empty` never work on a drive or share root.
fn not_root(path: &str) -> Result<(), String> {
    let t = path.trim().trim_end_matches(['\\', '/']);
    let share_root = t.strip_prefix("\\\\").is_some_and(|rest| rest.split(['\\', '/']).filter(|s| !s.is_empty()).count() <= 2);
    if t.is_empty() || (t.len() == 2 && t.ends_with(':')) || share_root {
        return Err(format!("ドライブや共有の一番上は扱わない: {path}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scripts_quote_their_paths_and_copy_judges_robocopy() {
        let d = Chore::Delete { path: "D:\\logs\\it's".into(), pattern: Some("*.log".into()), keep_days: 30 };
        let (s, j) = d.script();
        assert!(s.contains("-LiteralPath 'D:\\logs\\it''s' -Filter '*.log'"), "{s}");
        assert!(s.contains("AddDays(-30)"), "{s}");
        assert!(j.is_none());
        let c = Chore::Copy { from: "D:\\out\\".into(), into: "E:\\".into(), pattern: None };
        let (s, j) = c.script();
        assert!(s.starts_with("& robocopy 'D:\\out' 'E:\\.' /E"), "{s}");
        assert_eq!(j.unwrap().warn, vec![Range { from: 1, to: 1 }]);
        let z = Chore::Zip { from: "D:\\out\\*.csv".into(), into: "D:\\arc\\a.zip".into() };
        assert!(z.script().0.contains("Compress-Archive -Path 'D:\\out\\*.csv'"));
    }

    #[test]
    fn roots_empty_paths_and_a_zip_without_zip_are_refused() {
        for p in ["C:\\", "C:", "\\\\nas\\share", "\\\\nas\\share\\", ""] {
            assert!(Chore::Empty { path: p.into() }.check().is_err(), "{p}");
        }
        assert!(Chore::Empty { path: "\\\\nas\\share\\tmp".into() }.check().is_ok());
        assert!(Chore::Delete { path: "D:\\logs".into(), pattern: None, keep_days: 0 }.check().is_err());
        assert!(Chore::Zip { from: "a".into(), into: "b.7z".into() }.check().is_err());
        assert!(Chore::Zip { from: "a".into(), into: "b.ZIP".into() }.check().is_ok());
    }
}
