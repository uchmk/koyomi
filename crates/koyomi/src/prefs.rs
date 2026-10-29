//! What the settings screen reads and writes (ito's `docs/common-spec.md`):
//! common.toml (the language, the scale, the theme and the clock, shared by
//! every uchmk app), the person's themes and koyomi's keymap.toml. The files
//! are watched every 2 s and written on threads of their own; what was read
//! comes back as a [`Reload`], which [`Prefs::pump`] folds in. The UI thread
//! never touches the disk. (minato's prefs.rs, without the backup.)

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use ito_common::{common_path, edit_common, themes_dir, write_atomic, Common, CommonChange};
use ito_prefs::State;
use ito_theme::Theme;

/// How often the files are looked at.
const WATCH: Duration = Duration::from_secs(2);

/// A file read again, or a write that failed.
pub enum Reload {
    Common(Result<(Common, Vec<String>), String>),
    Themes(Vec<Theme>, Vec<String>),
    /// keymap.toml's text (`None` when there is none), or why it could not
    /// be read.
    Keymap(Result<Option<String>, String>),
    Failed(String),
}

/// keymap.toml in `base`'s `koyomi` folder.
pub fn keymap_path(base: &Path) -> PathBuf {
    ito_common::app_dir_in(base, "koyomi").join(crate::keymap::FILE)
}

/// The text of the file at `path`; `None` when there is none.
fn read_keymap(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Keys set fast one after another: each reads what the one before wrote,
/// and their keymaps are sent back in the same order.
static KEYMAP_ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Make `edits` in the keymap.toml at `path` (none yet is an empty one),
/// written only when that changes it. Call it off the UI thread.
pub fn write_keymap(path: &Path, edits: &[ito_keymap::Edit]) -> Result<(), String> {
    let at = |e: String| format!("{}: {e}", path.display());
    let text = read_keymap(path)?.unwrap_or_default();
    let new = ito_keymap::edit(&text, &crate::keymap::SPEC, edits, crate::keymap::NOOP).map_err(at)?;
    if new != text {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| at(e.to_string()))?;
        }
        write_atomic(path, &new).map_err(at)?;
    }
    Ok(())
}

/// `<name>.bak-<stamp>` next to `p`: where a file moved aside goes.
fn aside(p: &Path, stamp: u64) -> PathBuf {
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    p.with_file_name(format!("{name}.bak-{stamp}"))
}

/// Seconds since the epoch, for the names of the files moved aside.
fn stamp() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub struct Prefs {
    /// The `uchmk` folder; `None` when the OS has no config directory.
    pub base: Option<PathBuf>,
    pub common: Common,
    /// The built-in themes, then the person's.
    pub themes: Vec<Theme>,
    pub common_warn: Vec<String>,
    pub theme_warn: Vec<String>,
    /// The theme common.toml names but which is not there.
    pub theme_err: Option<String>,
    /// The screen's page, search and focus.
    pub state: State,
    /// The last write that failed, for the status bar.
    pub failed: Option<String>,
    /// keymap.toml's text, `None` when there is none.
    pub keymap: Option<String>,
    /// `Some(None)` when keymap.toml has been read again (the keys are to be
    /// made again from [`Prefs::keymap`]), `Some(Some(why))` when it could
    /// not be (to be said; the keys stay).
    pub keymap_new: Option<Option<String>>,
    /// The Keys page's search and the keys it is taking.
    pub keys_edit: ito_prefs::KeysEdit,
    tx: Sender<Reload>,
    rx: Receiver<Reload>,
    wake: Option<egui::Context>,
    /// The scale the keys asked for, until common.toml has it: an older
    /// read coming back meanwhile does not undo it.
    scale_wanted: Option<f32>,
    /// The thread that writes the scale, the newest one of a burst.
    scale_tx: Option<Sender<f32>>,
    /// The OS's light or dark, for `system` (egui's lags a switch on Windows).
    os_light: ito_common::OsLight,
}

impl Prefs {
    /// Nothing read: the defaults and the built-in themes (the tests).
    pub fn empty() -> Self {
        let (tx, rx) = channel();
        Self {
            base: None,
            common: Common::default(),
            themes: ito_theme::builtin(),
            common_warn: Vec::new(),
            theme_warn: Vec::new(),
            theme_err: None,
            state: State::default(),
            failed: None,
            keymap: None,
            keymap_new: None,
            keys_edit: ito_prefs::KeysEdit::default(),
            tx,
            rx,
            wake: None,
            scale_wanted: None,
            scale_tx: None,
            os_light: ito_common::OsLight::default(),
        }
    }

    /// common.toml, the themes and keymap.toml in `base`.
    pub fn load(base: Option<PathBuf>) -> Self {
        let mut p = Self::empty();
        if let Some(b) = &base {
            match Common::read(b) {
                Ok((c, w)) => (p.common, p.common_warn) = (c, w),
                Err(e) => p.common_warn = vec![e],
            }
            let (user, warn) = ito_theme::load_dir(&themes_dir(b));
            p.themes.extend(user);
            p.theme_warn = warn;
            match read_keymap(&keymap_path(b)) {
                Ok(t) => p.keymap = t,
                Err(e) => p.keymap_new = Some(Some(e)),
            }
        }
        p.base = base;
        p
    }

    /// Look at the files and the OS's light or dark every 2 s from now on,
    /// and wake `ctx` when one has been read again.
    pub fn watch(&mut self, ctx: &egui::Context) {
        self.wake = Some(ctx.clone());
        let os_ctx = ctx.clone();
        self.os_light = ito_common::OsLight::watch(WATCH, move || os_ctx.request_repaint());
        let Some(base) = self.base.clone() else { return };
        let (common, themes, keymap) = (common_path(&base), themes_dir(&base), keymap_path(&base));
        let mut paths = vec![common.clone(), themes.clone(), keymap.clone()];
        // A theme file edited in place; one added or taken away changes the
        // folder's time.
        if let Ok(entries) = std::fs::read_dir(&themes) {
            paths.extend(entries.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "toml")));
        }
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        ito_common::watch(paths, WATCH, move |changed| {
            let mut sent = true;
            if changed.contains(&common) {
                sent &= tx.send(Reload::Common(Common::read(&base))).is_ok();
            }
            if changed.iter().any(|p| p.starts_with(&themes)) {
                let (user, warn) = ito_theme::load_dir(&themes);
                let all = ito_theme::builtin().into_iter().chain(user).collect();
                sent &= tx.send(Reload::Themes(all, warn)).is_ok();
            }
            if changed.contains(&keymap) {
                sent &= tx.send(Reload::Keymap(read_keymap(&keymap))).is_ok();
            }
            ctx.request_repaint();
            sent
        });
    }

    /// Fold in what the threads have read since the last frame.
    pub fn pump(&mut self) {
        while let Ok(r) = self.rx.try_recv() {
            match r {
                Reload::Common(Ok((mut c, w))) => {
                    if let Some(s) = self.scale_wanted {
                        if c.scale == Some(s) {
                            self.scale_wanted = None;
                        } else {
                            c.scale = Some(s);
                        }
                    }
                    (self.common, self.common_warn) = (c, w);
                }
                Reload::Common(Err(e)) => self.common_warn = vec![e],
                Reload::Themes(t, w) => (self.themes, self.theme_warn) = (t, w),
                // Unreadable (being written, say): the keys stay as they are.
                Reload::Keymap(Err(e)) => self.keymap_new = Some(Some(e)),
                Reload::Keymap(Ok(t)) => {
                    self.keymap = t;
                    self.keymap_new = Some(None);
                }
                Reload::Failed(e) => {
                    self.scale_wanted = None;
                    self.failed = Some(e);
                }
            }
        }
    }

    fn send_back(tx: &Sender<Reload>, wake: &Option<egui::Context>, r: Reload) {
        let _ = tx.send(r);
        if let Some(ctx) = wake {
            ctx.request_repaint();
        }
    }

    /// Write `changes` into common.toml on a thread, then read it again.
    pub fn common_change(&self, changes: Vec<CommonChange>, no_folder: &str) {
        if changes.is_empty() {
            return;
        }
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        let Some(base) = self.base.clone() else {
            Self::send_back(&tx, &wake, Reload::Failed(no_folder.into()));
            return;
        };
        std::thread::spawn(move || {
            let done = edit_common(&base, |text| changes.iter().try_fold(text.to_owned(), |t, c| c.apply(&t)));
            let r = match done {
                Ok(()) => Reload::Common(Common::read(&base)),
                Err(e) => Reload::Failed(e),
            };
            Self::send_back(&tx, &wake, r);
        });
    }

    /// The scale at once, and into common.toml on a thread of its own that
    /// writes only the newest of a burst of keys (kura's way).
    pub fn set_scale(&mut self, s: f32, no_folder: &str) {
        self.common.scale = Some(s);
        let Some(base) = self.base.clone() else {
            Self::send_back(&self.tx, &self.wake, Reload::Failed(no_folder.into()));
            return;
        };
        self.scale_wanted = Some(s);
        let tx = self.scale_tx.get_or_insert_with(|| {
            let (tx, rx) = channel::<f32>();
            let (back, wake) = (self.tx.clone(), self.wake.clone());
            std::thread::spawn(move || {
                while let Ok(mut s) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        s = newer;
                    }
                    let r = match edit_common(&base, |t| ito_common::scale_change(s).apply(t)) {
                        Ok(()) => Reload::Common(Common::read(&base)),
                        Err(e) => Reload::Failed(e),
                    };
                    Self::send_back(&back, &wake, r);
                }
            });
            tx
        });
        let _ = tx.send(s);
    }

    /// Make the Keys page's `edits` in keymap.toml on a thread (only what
    /// differs from the built-in keys is written), then read it again.
    pub fn keymap_edit(&self, edits: Vec<ito_keymap::Edit>, no_folder: &str) {
        if edits.is_empty() {
            return;
        }
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        let Some(base) = self.base.clone() else {
            Self::send_back(&tx, &wake, Reload::Failed(no_folder.into()));
            return;
        };
        std::thread::spawn(move || {
            let _one = KEYMAP_ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
            let path = keymap_path(&base);
            let r = match write_keymap(&path, &edits) {
                Ok(()) => Reload::Keymap(read_keymap(&path)),
                Err(e) => Reload::Failed(e),
            };
            Self::send_back(&tx, &wake, r);
        });
    }

    /// Open common.toml (made empty first when there is none) with what the
    /// OS opens it with, on a thread.
    pub fn open_common(&self, no_folder: &str) {
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        let Some(base) = self.base.clone() else {
            Self::send_back(&tx, &wake, Reload::Failed(no_folder.into()));
            return;
        };
        std::thread::spawn(move || {
            let path = common_path(&base);
            let made = if path.exists() { Ok(()) } else { write_atomic(&path, "") };
            if let Err(e) = made.and_then(|()| open_path(&path)) {
                Self::send_back(&tx, &wake, Reload::Failed(e));
            }
        });
    }

    /// Open a folder in the system's file manager, on a thread.
    pub fn open_dir(&self, dir: PathBuf) {
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        std::thread::spawn(move || {
            if let Err(e) = open_path(&dir) {
                Self::send_back(&tx, &wake, Reload::Failed(e));
            }
        });
    }

    /// The shared settings back to how koyomi starts: common.toml and
    /// keymap.toml are moved aside (`<name>.bak-<time>`), on a thread. The
    /// watcher then reads the missing files as defaults. koyomi's own
    /// koyomi.toml and the definitions are not touched: they are the work,
    /// not a preference.
    pub fn reset(&self, no_folder: &str) {
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        let Some(base) = self.base.clone() else {
            Self::send_back(&tx, &wake, Reload::Failed(no_folder.into()));
            return;
        };
        std::thread::spawn(move || {
            let stamp = stamp();
            let mut failed = Vec::new();
            for path in [common_path(&base), keymap_path(&base)] {
                if path.exists() {
                    if let Err(e) = std::fs::rename(&path, aside(&path, stamp)) {
                        failed.push(format!("{}: {e}", path.display()));
                    }
                }
            }
            let r = match failed.first() {
                None => Reload::Common(Common::read(&base)),
                Some(e) => Reload::Failed(e.clone()),
            };
            Self::send_back(&tx, &wake, r);
        });
    }

    /// The theme common.toml picks, in force from this frame on. Called
    /// every frame (the OS may have gone light or dark).
    pub fn apply_theme(&mut self, ctx: &egui::Context) {
        let os_light = self.os_light.light_or(ctx.system_theme() == Some(egui::Theme::Light));
        let (choice, dark, light) = self.common.theme_choice(Some(("system", "koyomi Dark", "koyomi Light")));
        let colors = match ito_theme::pick(&self.themes, &choice, &dark, &light, os_light) {
            Ok(t) => {
                self.theme_err = None;
                t.colors
            }
            Err(e) => {
                self.theme_err = Some(e);
                self.themes[0].colors
            }
        };
        if colors != ito_theme::colors() {
            ito_theme::set(colors);
            ctx.set_visuals(colors.visuals());
        }
    }
}

#[cfg(windows)]
fn opener() -> std::process::Command {
    std::process::Command::new("explorer")
}

#[cfg(target_os = "macos")]
fn opener() -> std::process::Command {
    std::process::Command::new("open")
}

#[cfg(not(any(windows, target_os = "macos")))]
fn opener() -> std::process::Command {
    std::process::Command::new("xdg-open")
}

fn open_path(path: &Path) -> Result<(), String> {
    // explorer answers 1 even when it opened the file, so only a failure to
    // start it is said.
    opener().arg(path).status().map(drop).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ito_keymap::Edit;
    use ito_keys::Key;

    fn on(t: &str) -> Vec<Key> {
        t.split_whitespace().map(|k| Key::parse(k).unwrap()).collect()
    }

    /// Only what is changed is written; put back, nothing of it stays.
    #[test]
    fn the_keymap_gets_only_the_changes() {
        let dir = std::env::temp_dir().join(format!("koyomi-keymap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = keymap_path(&dir);
        let add = Edit::Add { layer: "app".into(), on: on("<C-S-h>"), run: "help".into(), desc: "Show the key help".into() };
        let reset = Edit::Reset { layer: "app".into(), run: "help".into() };
        // Added then put back in one go: no file is made.
        write_keymap(&path, &[add.clone(), reset.clone()]).unwrap();
        assert!(!path.exists());
        write_keymap(&path, &[add]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("<C-S-h>") && text.contains("prepend_keymap") && !text.contains("quit"), "{text}");
        // A built-in key taken away is a `none` in front of it.
        write_keymap(&path, &[Edit::Remove { layer: "app".into(), on: on("<C-q>"), run: "quit".into() }]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#"run = "none""#), "{text}");
        let (k, warnings) = crate::keymap::Keys::load(Some(("keymap.toml", &text)));
        assert!(warnings.is_empty(), "{warnings:?}");
        let help: Vec<_> = k.layers()[0].1.iter().filter(|b| b.raw == "help").map(|b| b.on.clone()).collect();
        assert!(help.contains(&on("<C-S-h>")), "{help:?}");
        write_keymap(&path, &[reset]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("help") && !text.contains("<C-S-h>"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_moved_aside_keeps_its_name_with_a_stamp() {
        let p = aside(Path::new("C:/uchmk/common.toml"), 42);
        assert_eq!(p.file_name().unwrap().to_string_lossy(), "common.toml.bak-42");
        assert_eq!(p.parent(), Some(Path::new("C:/uchmk")));
    }
}
