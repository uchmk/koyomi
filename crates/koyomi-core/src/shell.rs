//! The shells a job's `shell` may name, and the arguments each one is started with.
//!
//! Pure: which program is found on a PC is the engine's business; here the program's
//! path comes in and the argument list goes out.

/// What `shell` takes, in the order the window lists them.
pub const SHELLS: [&str; 5] = ["powershell", "pwsh", "bash", "sh", "zsh"];

/// `shell` as a list of quoted words, for a message (`"powershell", "pwsh", …`).
pub fn quoted_list() -> String {
    SHELLS.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(", ")
}

/// Whether `shell` is a PowerShell (the script goes as `-EncodedCommand`).
pub fn is_powershell(shell: &str) -> bool {
    matches!(shell, "powershell" | "pwsh")
}

/// Whether `shell` is one of the sh family (the script goes as the argument of `-c`).
pub fn is_posix(shell: &str) -> bool {
    matches!(shell, "bash" | "sh" | "zsh")
}

/// What to install for `shell`, as words for a message (`PowerShell 7（pwsh）`, `bash のパッケージ`).
pub fn install_hint(shell: &str) -> String {
    if is_powershell(shell) {
        crate::msg::note(crate::msg::N_SHELL_PKG_PWSH, &[])
    } else {
        crate::msg::note(crate::msg::N_SHELL_PKG, &[shell])
    }
}

/// The program `shell` is started as when the engine has no path for it: a bare name,
/// left to the OS to find.
pub fn default_exe(shell: &str, windows: bool) -> &'static str {
    match shell {
        "powershell" if windows => "powershell.exe",
        "powershell" | "pwsh" => "pwsh",
        "bash" => "bash",
        "zsh" => "zsh",
        _ => "sh",
    }
}

/// The argument list that starts `shell` (the program at `exe`) with `script`, or `None` for a
/// name that is not one of [`SHELLS`].
///
/// - PowerShell: no profile, nothing asked, plain text in, and the script as `-EncodedCommand`
///   (base64 of UTF-16), so no quote or `$` in it is read by anything but the shell itself.
/// - bash, sh, zsh: the script is ONE argument of `-c` (`koyomi` is `$0`), and nothing is added
///   to it. A CRLF in it becomes LF here (a `\r` would end up inside commands); the text as
///   written is not changed anywhere else.
pub fn argv(shell: &str, exe: &str, script: &str) -> Option<Vec<String>> {
    let own = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    if is_powershell(shell) {
        let mut v = vec![exe.to_owned()];
        v.extend(own(&["-NoProfile", "-NonInteractive", "-InputFormat", "Text", "-EncodedCommand"]));
        let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        v.push(crate::run::base64(&utf16));
        return Some(v);
    }
    let flags: &[&str] = match shell {
        "bash" => &["--noprofile", "--norc", "-c"],
        "sh" => &["-c"],
        "zsh" => &["-f", "-c"],
        _ => return None,
    };
    let mut v = vec![exe.to_owned()];
    v.extend(own(flags));
    v.push(script.replace("\r\n", "\n"));
    v.push("koyomi".to_owned());
    Some(v)
}

use std::path::{Path, PathBuf};

/// What the finder looks at.
pub struct Env<'a> {
    pub windows: bool,
    /// The `PATH` entries, in order.
    pub path: &'a [PathBuf],
    /// `%SystemRoot%` (Windows).
    pub system_root: Option<&'a Path>,
    /// `%ProgramFiles%` (Windows): Git for Windows lives under it.
    pub program_files: Option<&'a Path>,
    /// Whether `file` is there and can be run (off Windows: a file with an execute bit).
    pub runnable: &'a dyn Fn(&Path) -> bool,
}

/// The shells the engine has, with the absolute path it runs for each, in [`SHELLS`]' order.
pub fn find(env: &Env) -> Vec<(String, PathBuf)> {
    SHELLS.iter().filter_map(|name| Some(((*name).to_owned(), one(env, name)?))).collect()
}

/// This PC's own: the real `PATH` and file system.
pub fn find_here() -> Vec<(String, PathBuf)> {
    find_in_path(&std::env::var_os("PATH").unwrap_or_default())
}

/// [`find_here`] over a `PATH` value given.
pub fn find_in_path(path_var: &std::ffi::OsStr) -> Vec<(String, PathBuf)> {
    let path: Vec<PathBuf> = std::env::split_paths(path_var).collect();
    let system_root = std::env::var_os("SystemRoot").map(PathBuf::from);
    let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
    find(&Env {
        windows: cfg!(windows),
        path: &path,
        system_root: system_root.as_deref(),
        program_files: program_files.as_deref(),
        runnable: &runnable,
    })
}

fn runnable(p: &Path) -> bool {
    let Ok(m) = std::fs::metadata(p) else { return false };
    if !m.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn one(env: &Env, name: &str) -> Option<PathBuf> {
    let file = |n: &str| if env.windows { format!("{n}.exe") } else { n.to_owned() };
    let first = |n: &str, skip: &dyn Fn(&Path) -> bool| {
        let f = file(n);
        // A relative entry would point somewhere else under a job's `dir`.
        env.path.iter().filter(|d| d.is_absolute()).map(|d| d.join(&f)).find(|p| !skip(p) && (env.runnable)(p))
    };
    let never = |_: &Path| false;
    match name {
        "powershell" if !env.windows => one(env, "pwsh"),
        "powershell" => {
            let own = env.system_root.map(|r| r.join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe"));
            own.filter(|p| (env.runnable)(p)).or_else(|| first("powershell", &never))
        }
        "bash" if env.windows => {
            // The WSL launcher runs in WSL, with other paths: it is not Git Bash.
            let wsl = |p: &Path| in_system32(env, p) || in_windows_apps(p);
            first("bash", &wsl).or_else(|| git_for_windows(env, "bash"))
        }
        "sh" if env.windows => first("sh", &never).or_else(|| git_for_windows(env, "sh")),
        n => first(n, &never),
    }
}

fn git_for_windows(env: &Env, name: &str) -> Option<PathBuf> {
    let p = env.program_files?.join("Git").join("bin").join(format!("{name}.exe"));
    (p.is_absolute() && (env.runnable)(&p)).then_some(p)
}

fn in_system32(env: &Env, p: &Path) -> bool {
    env.system_root.is_some_and(|r| lower(p).starts_with(&format!("{}\\system32\\", lower(r).trim_end_matches('\\'))))
}

fn in_windows_apps(p: &Path) -> bool {
    p.components().any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case("WindowsApps"))
}

/// The path as lower-case text with `\` for `/`, to compare Windows paths.
fn lower(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_shell_starts_its_own_way_with_the_script_as_one_argument() {
        let script = "echo \"a b\" 'c'\r\necho $HOME \\";
        assert_eq!(
            argv("bash", "/bin/bash", script).unwrap(),
            ["/bin/bash", "--noprofile", "--norc", "-c", "echo \"a b\" 'c'\necho $HOME \\", "koyomi"]
        );
        assert_eq!(argv("sh", "/bin/sh", "ls").unwrap(), ["/bin/sh", "-c", "ls", "koyomi"]);
        assert_eq!(argv("zsh", "zsh", "ls").unwrap(), ["zsh", "-f", "-c", "ls", "koyomi"]);
        let ps = ["-NoProfile", "-NonInteractive", "-InputFormat", "Text", "-EncodedCommand", "RwBlAHQALQBEAGEAdABlAAoAZQB4AGkAdAAgADEA"];
        for shell in ["powershell", "pwsh"] {
            let v = argv(shell, "X", "Get-Date\nexit 1").unwrap();
            assert_eq!((v[0].as_str(), &v[1..]), ("X", &ps.map(str::to_owned)[..]));
        }
        // PowerShell reads a CRLF itself, so the text goes in as written.
        let v = argv("pwsh", "p", "a\r\nb").unwrap();
        let utf16: Vec<u8> = "a\r\nb".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(v.last().unwrap(), &crate::run::base64(&utf16));
        assert!(argv("cmd", "cmd.exe", "dir").is_none());
    }

    #[test]
    fn the_bare_names_and_the_families() {
        assert_eq!(default_exe("powershell", true), "powershell.exe");
        assert_eq!(default_exe("powershell", false), "pwsh");
        assert_eq!(default_exe("pwsh", true), "pwsh");
        assert_eq!(SHELLS.iter().filter(|s| is_posix(s)).count(), 3);
        assert_eq!(SHELLS.iter().filter(|s| is_powershell(s)).count(), 2);
        assert_eq!(quoted_list(), "\"powershell\", \"pwsh\", \"bash\", \"sh\", \"zsh\"");
    }
}

#[cfg(test)]
mod finder_tests {
    use super::*;

    /// A path under the root of this OS (so it is absolute on both).
    fn abs(rest: &str) -> PathBuf {
        let mut p = PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" });
        for c in rest.split('/') {
            p.push(c);
        }
        p
    }

    fn found(windows: bool, path: &[&str], files: &[&str], extra: (Option<&str>, Option<&str>)) -> Vec<(String, PathBuf)> {
        let path: Vec<PathBuf> = path.iter().map(|p| if p.is_empty() { PathBuf::from("rel") } else { abs(p) }).collect();
        let files: Vec<PathBuf> = files.iter().map(|f| abs(f)).collect();
        let (root, pf) = (extra.0.map(abs), extra.1.map(abs));
        find(&Env {
            windows,
            path: &path,
            system_root: root.as_deref(),
            program_files: pf.as_deref(),
            runnable: &|p| files.iter().any(|f| f == p),
        })
    }

    fn names(f: &[(String, PathBuf)]) -> Vec<&str> {
        f.iter().map(|(n, _)| n.as_str()).collect()
    }

    #[test]
    fn off_windows_the_first_match_on_path_is_taken_and_powershell_is_pwsh() {
        let f = found(false, &["usr/bin", "bin"], &["bin/bash", "usr/bin/bash", "bin/sh", "usr/bin/pwsh"], (None, None));
        assert_eq!(names(&f), ["powershell", "pwsh", "bash", "sh"]);
        assert_eq!(f[0].1, f[1].1, "powershell is the pwsh found");
        assert_eq!(f[1].1, abs("usr/bin/pwsh"));
        assert_eq!(f[2].1, abs("usr/bin/bash"));
        // No pwsh: no powershell either.
        assert_eq!(names(&found(false, &["bin"], &["bin/sh"], (None, None))), ["sh"]);
    }

    #[test]
    fn a_relative_path_entry_is_not_counted() {
        let f = found(false, &["", "bin"], &["bin/zsh"], (None, None));
        assert_eq!(names(&f), ["zsh"]);
        assert!(f[0].1.is_absolute());
    }

    #[test]
    fn on_windows_the_wsl_bash_is_skipped_and_git_bash_is_taken() {
        let root = "Windows";
        let files =
            ["Windows/System32/bash.exe", "Users/me/AppData/Local/Microsoft/WindowsApps/bash.exe", "Program Files/Git/bin/bash.exe"];
        // Only the WSL launchers on PATH: Git for Windows' own is found by its folder.
        let f =
            found(true, &["Windows/System32", "Users/me/AppData/Local/Microsoft/WindowsApps"], &files, (Some(root), Some("Program Files")));
        assert_eq!(names(&f), ["bash"]);
        assert_eq!(f[0].1, abs("Program Files/Git/bin/bash.exe"));
        // Git Bash on PATH wins over the folder.
        let f = found(true, &["Windows/System32", "tools"], &[files[0], "tools/bash.exe"], (Some(root), Some("Program Files")));
        assert_eq!(f[0].1, abs("tools/bash.exe"));
        // Without Git for Windows there is no bash.
        assert!(found(true, &["Windows/System32"], &files[..1], (Some(root), Some("Program Files"))).is_empty());
        // sh: PATH first, then Git for Windows.
        let f = found(true, &[], &["Program Files/Git/bin/sh.exe"], (Some(root), Some("Program Files")));
        assert_eq!(names(&f), ["sh"]);
    }

    #[test]
    fn on_windows_powershell_is_the_one_in_system32_else_the_first_on_path() {
        let own = "Windows/System32/WindowsPowerShell/v1.0/powershell.exe";
        let f = found(true, &["tools"], &[own, "tools/powershell.exe", "tools/pwsh.exe"], (Some("Windows"), None));
        assert_eq!(names(&f), ["powershell", "pwsh"]);
        assert_eq!(f[0].1, abs(own));
        let f = found(true, &["tools"], &["tools/powershell.exe"], (Some("Windows"), None));
        assert_eq!(f[0].1, abs("tools/powershell.exe"));
        // Neither: not counted (nothing is assumed).
        assert!(found(true, &["tools"], &[], (Some("Windows"), None)).is_empty());
    }
}
