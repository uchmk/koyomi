//! The shells this engine's PC has, found by `koyomi_core::shell` (the same finder `koyomi check` uses), and the
//! default shell a new script job starts with.

use std::path::PathBuf;

use crate::proto::Shell;

pub use koyomi_core::shell::find_here;

/// The shell a new script job starts with, and the shell `koyomi.toml` wrote when this engine does not have it.
/// Written and found: that one. Otherwise the first found of Windows `powershell, pwsh, bash, sh, zsh`, elsewhere
/// `bash, sh, zsh, pwsh`; with none found, the OS's own name (`powershell`, or `sh`). A written shell that is not
/// found is given back too (for a caution).
pub fn default_shell(found: &[(String, PathBuf)], written: Option<&str>, windows: bool) -> (String, Option<String>) {
    let has = |n: &str| found.iter().any(|(f, _)| f == n);
    let order: &[&str] = if windows { &["powershell", "pwsh", "bash", "sh", "zsh"] } else { &["bash", "sh", "zsh", "pwsh"] };
    let os = order.iter().find(|n| has(n)).copied().unwrap_or(if windows { "powershell" } else { "sh" });
    match written {
        Some(w) if has(w) => (w.to_owned(), None),
        Some(w) => (os.to_owned(), Some(w.to_owned())),
        None => (os.to_owned(), None),
    }
}

/// The found shells as the snapshot carries them.
pub fn listed(found: &[(String, PathBuf)]) -> Vec<Shell> {
    found.iter().map(|(name, path)| Shell { name: name.clone(), path: path.display().to_string() }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(rest: &str) -> PathBuf {
        let mut p = PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" });
        for c in rest.split('/') {
            p.push(c);
        }
        p
    }

    #[test]
    fn the_default_follows_the_rules() {
        let have = |n: &[&str]| n.iter().map(|n| ((*n).to_owned(), abs(n))).collect::<Vec<_>>();
        let (bash_sh, only_sh) = (have(&["bash", "sh"]), have(&["sh"]));
        // Written and found.
        assert_eq!(default_shell(&bash_sh, Some("sh"), false), ("sh".into(), None));
        // Written and missing: the OS's default, and what was written for the caution.
        assert_eq!(default_shell(&only_sh, Some("zsh"), false), ("sh".into(), Some("zsh".into())));
        // Nothing written: the first found in the OS's order; none found: the OS's own name.
        let only_zsh = have(&["zsh", "pwsh"]);
        assert_eq!(default_shell(&only_zsh, None, false), ("zsh".into(), None));
        assert_eq!(default_shell(&have(&["pwsh"]), None, false), ("pwsh".into(), None));
        assert_eq!(default_shell(&have(&["bash", "pwsh"]), None, true), ("pwsh".into(), None));
        assert_eq!(default_shell(&[], None, false), ("sh".into(), None));
        assert_eq!(default_shell(&bash_sh, Some("zsh"), false), ("bash".into(), Some("zsh".into())));
        // Not written, per OS.
        assert_eq!(default_shell(&bash_sh, None, false), ("bash".into(), None));
        assert_eq!(default_shell(&only_sh, None, false), ("sh".into(), None));
        assert_eq!(default_shell(&[], None, true), ("powershell".into(), None));
        // A found shell wins over the OS name: Git Bash only (no PowerShell found) starts on bash.
        assert_eq!(default_shell(&bash_sh, None, true), ("bash".into(), None));
        assert_eq!(default_shell(&have(&["powershell", "bash"]), None, true), ("powershell".into(), None));
    }

    #[test]
    fn the_snapshot_lists_name_and_path() {
        let l = listed(&[("bash".into(), abs("bin/bash"))]);
        assert_eq!(l, [Shell { name: "bash".into(), path: abs("bin/bash").display().to_string() }]);
    }
}
