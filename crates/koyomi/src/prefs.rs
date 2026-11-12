//! What the settings screen reads and writes (ito's `docs/common-spec.md`):
//! common.toml (the language, the scale, the theme and the clock, shared by
//! every uchmk app), the person's themes, koyomi's keymap.toml and the
//! `[notify]` table of koyomi.toml (the Notifications page; the rest of
//! koyomi.toml stays a file). The files are watched every 2 s and written on
//! threads of their own; what was read comes back as a [`Reload`], which
//! [`Prefs::pump`] folds in. The UI thread never touches the disk.
//! (minato's prefs.rs, without the backup.)

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use ito_common::{common_path, edit_common, themes_dir, write_atomic, Common, CommonChange};
use ito_notify::mail::Mail;
use ito_notify::webhook::{Level, Place};
use ito_prefs::{Notify, NotifyCfg, State};
use ito_theme::Theme;
use koyomi_core::config::Config;

use crate::notify::EVENTS;
use koyomi_core::msg;

/// How often the files are looked at.
const WATCH: Duration = Duration::from_secs(2);

/// A file read again, or a write that failed.
pub enum Reload {
    Common(Result<(Common, Vec<String>), String>),
    Themes(Vec<Theme>, Vec<String>),
    /// keymap.toml's text (`None` when there is none), or why it could not
    /// be read.
    Keymap(Result<Option<String>, String>),
    /// koyomi.toml: its limits, its `[notify]` with the `secret:` addresses
    /// read back, and what to warn of; or why the file could not be read.
    /// The number is the order the read was begun in (see [`next_read`]).
    Config(u64, Box<ConfigRead>),
    Failed(String),
}

/// koyomi.toml as the settings read it: its limits, its `[notify]`, what to warn of.
type ConfigRead = Result<(Config, NotifyCfg, Vec<String>), String>;

/// koyomi.toml in `base`'s `koyomi` folder.
pub fn config_path(base: &Path) -> PathBuf {
    ito_common::app_dir_in(base, "koyomi").join("koyomi.toml")
}

/// The koyomi.toml at `path` (none: the defaults), and its `[notify]` as
/// the page shows it, the `secret:` addresses read back from the credential
/// store. Blocks on the store: call it off the UI thread.
fn read_config(path: &Path) -> ConfigRead {
    let config = Config::load(path).map_err(|e| msg::coded_at(msg::FILE_READ_FAILED, path, &e))?;
    let (mut n, mut notes) = crate::notify::parse(&config);
    notes.extend(n.reveal("koyomi"));
    Ok((config, n, notes))
}

/// A read of koyomi.toml at `path` as a [`Reload`]. It takes the next number of `seq` before it reads the file, so
/// that [`Prefs::pump`] can drop a read that was begun earlier than one it has applied: an older read slowed by the
/// credential store must not undo a newer one. Blocks on the store: call it off the UI thread.
fn next_read(seq: &AtomicU64, path: &Path) -> Reload {
    let n = seq.fetch_add(1, Ordering::SeqCst) + 1;
    Reload::Config(n, Box::new(read_config(path)))
}

/// One lock for every writer of koyomi.toml from the window.
static CONFIG_ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Set `edits` (keys of `table`, `None` for the top of the file, already
/// TOML), `webhooks` and `mails` in the koyomi.toml at `path` (none yet is
/// an empty one), written only when that changes it; the rest of the file
/// keeps its lines and comments. Call it off the UI thread.
pub fn write_config(
    path: &Path,
    table: Option<&str>,
    edits: &[(&str, String)],
    webhooks: Option<&[Place]>,
    mails: Option<&[Mail]>,
) -> Result<(), String> {
    let at = |e: String| msg::coded_at(msg::FILE_WRITE_FAILED, path, &e);
    let _held = crate::remote::edits();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(msg::coded_at(msg::FILE_READ_FAILED, path, &e.to_string())),
    };
    let mut new = text.clone();
    for (key, value) in edits {
        new = ito_common::set_key(&new, table, key, value).map_err(at)?;
    }
    if let Some(places) = webhooks {
        new = ito_prefs::write_webhooks(&new, places, true).map_err(at)?;
    }
    if let Some(mails) = mails {
        new = write_mails(&new, mails).map_err(at)?;
    }
    if new != text {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| msg::coded_at(msg::DIR_CREATE_FAILED, dir, &e.to_string()))?;
        }
        write_atomic(path, &new).map_err(at)?;
    }
    Ok(())
}

/// `text` (koyomi.toml) with the top-level `watch_remotes` as `on` says: off adds `watch_remotes = false`, on removes
/// the key (it is the default). The rest of the file keeps its lines and comments.
fn watch_remotes_text(text: &str, on: bool) -> Result<String, String> {
    if on {
        ito_common::remove_key(text, None, "watch_remotes")
    } else {
        ito_common::set_key(text, None, "watch_remotes", "false")
    }
}

/// `text` with `mails` as its `[[notify.mails]]`. A place just added (no
/// `states`) is told of `failed` only, as one written without `states`;
/// `states` is written only when a place's differ from that.
fn write_mails(text: &str, mails: &[Mail]) -> Result<String, String> {
    let quiet = [crate::notify::QUIET.to_owned()];
    let mails: Vec<Mail> =
        mails.iter().map(|m| if m.states.is_empty() { Mail { states: quiet.to_vec(), ..m.clone() } } else { m.clone() }).collect();
    ito_prefs::write_mails(text, &mails, mails.iter().any(|m| m.states != quiet))
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
        Err(e) => Err(msg::coded_at(msg::FILE_READ_FAILED, path, &e.to_string())),
    }
}

/// Keys set fast one after another: each reads what the one before wrote,
/// and their keymaps are sent back in the same order.
static KEYMAP_ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Make `edits` in the keymap.toml at `path` (none yet is an empty one),
/// written only when that changes it. Call it off the UI thread.
pub fn write_keymap(path: &Path, edits: &[ito_keymap::Edit]) -> Result<(), String> {
    let at = |e: String| msg::coded_at(msg::FILE_WRITE_FAILED, path, &e);
    let text = read_keymap(path)?.unwrap_or_default();
    let new = ito_keymap::edit(&text, &crate::keymap::SPEC, edits, crate::keymap::NOOP).map_err(at)?;
    if new != text {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| msg::coded_at(msg::DIR_CREATE_FAILED, dir, &e.to_string()))?;
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
    /// `[notify]` of koyomi.toml as the Notifications page shows it (the
    /// `secret:` addresses read back), and what it warns of.
    pub notify: NotifyCfg,
    pub notify_warn: Vec<String>,
    /// The Notifications page's fields while they are typed in.
    pub notify_drafts: ito_prefs::Drafts,
    /// koyomi.toml's limits (`keep_days`, `concurrency`, `log_max_mb`) as
    /// the General page shows them; the defaults until it is read.
    pub config: Config,
    /// What the General page's number fields were last told off for, until
    /// the file is read again.
    pub config_warn: Option<String>,
    /// The General page's number fields while they are typed in.
    pub limit_drafts: ito_prefs::Drafts,
    /// Whether koyomi.toml has been read once (or there is no folder to read it from, and the defaults are all there
    /// is). Until then what [`Prefs::config`] says is a guess, and the watch of other PCs keeps what it has.
    pub config_known: bool,
    /// The number of the last read of koyomi.toml begun (see [`next_read`]), shared with the threads that read.
    seq: Arc<AtomicU64>,
    /// The number of the newest read of koyomi.toml that has been applied.
    applied: u64,
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
            notify: crate::notify::parse(&Config::default()).0,
            notify_warn: Vec::new(),
            notify_drafts: ito_prefs::Drafts::new(),
            config: Config::default(),
            config_warn: None,
            limit_drafts: ito_prefs::Drafts::new(),
            config_known: true,
            seq: Arc::new(AtomicU64::new(0)),
            applied: 0,
            tx,
            rx,
            wake: None,
            scale_wanted: None,
            scale_tx: None,
            os_light: ito_common::OsLight::default(),
        }
    }

    /// common.toml, the themes and keymap.toml in `base`. koyomi.toml's
    /// `[notify]` comes a moment later (the credential store is slow): a
    /// thread reads it and [`Prefs::pump`] takes it in.
    pub fn load(base: Option<PathBuf>) -> Self {
        let mut p = Self::empty();
        if let Some(b) = &base {
            let (tx, path, seq) = (p.tx.clone(), config_path(b), p.seq.clone());
            p.config_known = false;
            std::thread::spawn(move || {
                let _ = tx.send(next_read(&seq, &path));
            });
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
        let (common, themes, keymap, config) = (common_path(&base), themes_dir(&base), keymap_path(&base), config_path(&base));
        let mut paths = vec![common.clone(), themes.clone(), keymap.clone(), config.clone()];
        // A theme file edited in place; one added or taken away changes the
        // folder's time.
        if let Ok(entries) = std::fs::read_dir(&themes) {
            paths.extend(entries.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "toml")));
        }
        let (tx, ctx, seq) = (self.tx.clone(), ctx.clone(), self.seq.clone());
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
            if changed.contains(&config) {
                sent &= tx.send(next_read(&seq, &config)).is_ok();
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
                // Begun before one already applied: dropped, whatever it read.
                Reload::Config(n, _) if n <= self.applied => {}
                Reload::Config(n, r) => match *r {
                    Ok((c, n_cfg, w)) => {
                        self.applied = n;
                        self.config_known = true;
                        self.config = c;
                        self.config_warn = None;
                        (self.notify, self.notify_warn) = (n_cfg, w);
                    }
                    // Unreadable (being written, or a line the engine refuses
                    // too): the page keeps what it had, and says so.
                    Err(e) => {
                        self.applied = n;
                        self.notify_warn = vec![e];
                    }
                },
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

    /// Write the Notifications page's `page` into koyomi.toml's `[notify]` on
    /// a thread (only the keys that differ from the file; the webhook
    /// addresses go to the credential store while `hide_webhooks` is on),
    /// then read it again. The engine reads koyomi.toml again within 5 s.
    pub fn notify_edit(&self, page: Notify, no_folder: &str) {
        if page == self.notify.page(&EVENTS) {
            return;
        }
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        let seq = self.seq.clone();
        let Some(base) = self.base.clone() else {
            Self::send_back(&tx, &wake, Reload::Failed(no_folder.into()));
            return;
        };
        let now = self.notify.clone();
        std::thread::spawn(move || {
            let _one = CONFIG_ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
            let path = config_path(&base);
            let edits = now.edits(&EVENTS, &page);
            let r = match now.webhooks_to_write("koyomi", &EVENTS, &page) {
                Ok(webhooks) => match write_config(&path, Some("notify"), &edits, webhooks.as_deref(), None) {
                    Ok(()) => next_read(&seq, &path),
                    Err(e) => Reload::Failed(e),
                },
                Err(e) => Reload::Failed(msg::line(msg::WEBHOOK_STORE_FAILED, &[&e.to_string()])),
            };
            Self::send_back(&tx, &wake, r);
        });
    }

    /// Write the mail places as the Notifications page has them into
    /// koyomi.toml's `[[notify.mails]]` on a thread, then read it again.
    pub fn mails_edit(&self, mails: Vec<Mail>, no_folder: &str) {
        if mails == self.notify.mail_places {
            return;
        }
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        let seq = self.seq.clone();
        let Some(base) = self.base.clone() else {
            Self::send_back(&tx, &wake, Reload::Failed(no_folder.into()));
            return;
        };
        std::thread::spawn(move || {
            let _one = CONFIG_ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
            let path = config_path(&base);
            let r = match write_config(&path, Some("notify"), &[], None, Some(&mails)) {
                Ok(()) => next_read(&seq, &path),
                Err(e) => Reload::Failed(e),
            };
            Self::send_back(&tx, &wake, r);
        });
    }

    /// "Send a test here" on a mail place, as the page has it: its password
    /// is read from the credential store on a thread; what keeps it from
    /// going is said as a failure.
    pub fn mail_test(&self, m: Mail, title: String, body: String) {
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        std::thread::spawn(move || {
            if let Err(e) = ito_prefs::test_mail("koyomi", &m, Level::Done, &title, &body) {
                Self::send_back(&tx, &wake, Reload::Failed(e));
            }
        });
    }

    /// Write `edits` (koyomi.toml's top-level keys, `keep_days`,
    /// `concurrency`, `log_max_mb` and `archive`, as TOML) on a thread, then
    /// read the file again. One already as the file has it is not written.
    pub fn limits_edit(&self, edits: Vec<(&'static str, String)>, no_folder: &str) {
        let c = &self.config;
        let edits: Vec<(&str, String)> = edits
            .into_iter()
            .filter(|(k, v)| match *k {
                "keep_days" => *v != c.keep_days.to_string(),
                "concurrency" => *v != c.concurrency.to_string(),
                "log_max_mb" => *v != c.log_max_mb.to_string(),
                "archive" => *v != c.archive.to_string(),
                _ => true,
            })
            .collect();
        if edits.is_empty() {
            return;
        }
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        let seq = self.seq.clone();
        let Some(base) = self.base.clone() else {
            Self::send_back(&tx, &wake, Reload::Failed(no_folder.into()));
            return;
        };
        std::thread::spawn(move || {
            let _one = CONFIG_ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
            let path = config_path(&base);
            let r = match write_config(&path, None, &edits, None, None) {
                Ok(()) => next_read(&seq, &path),
                Err(e) => Reload::Failed(e),
            };
            Self::send_back(&tx, &wake, r);
        });
    }

    /// Read koyomi.toml again at once (after the window itself wrote it from the manage box), so that the watch of
    /// other PCs does not lag the 2 s look at the file. The read is numbered like every other (see [`next_read`]).
    pub fn reread_config(&self) {
        let Some(base) = self.base.clone() else { return };
        let (tx, wake, seq) = (self.tx.clone(), self.wake.clone(), self.seq.clone());
        std::thread::spawn(move || {
            let r = next_read(&seq, &config_path(&base));
            Self::send_back(&tx, &wake, r);
        });
    }

    /// Set whether this window watches the `[[remote]]` PCs at all: off writes `watch_remotes = false` into
    /// koyomi.toml on a thread, on removes the key (so a file with everything on again is one koyomi 0.37.0 and older
    /// still read), then the file is read again. Nothing is written when the file already says so.
    pub fn set_watch_remotes(&self, on: bool, no_folder: &str) {
        if on == self.config.watch_remotes {
            return;
        }
        let (tx, wake, seq) = (self.tx.clone(), self.wake.clone(), self.seq.clone());
        let Some(base) = self.base.clone() else {
            Self::send_back(&tx, &wake, Reload::Failed(no_folder.into()));
            return;
        };
        std::thread::spawn(move || {
            let _one = CONFIG_ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
            let _held = crate::remote::edits();
            let path = config_path(&base);
            let at = |e: String| msg::coded_at(msg::FILE_WRITE_FAILED, &path, &e);
            let done = (|| {
                let text = match std::fs::read_to_string(&path) {
                    Ok(t) => t,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                    Err(e) => return Err(msg::coded_at(msg::FILE_READ_FAILED, &path, &e.to_string())),
                };
                let new = watch_remotes_text(&text, on).map_err(at)?;
                if new != text {
                    if let Some(dir) = path.parent() {
                        std::fs::create_dir_all(dir).map_err(|e| msg::coded_at(msg::DIR_CREATE_FAILED, dir, &e.to_string()))?;
                    }
                    write_atomic(&path, &new).map_err(at)?;
                }
                Ok(())
            })();
            let r = match done {
                Ok(()) => next_read(&seq, &path),
                Err(e) => Reload::Failed(e),
            };
            Self::send_back(&tx, &wake, r);
        });
    }

    /// Open a settings file of the base (`common_path`, `config_path`), made
    /// empty first when there is none, with what the OS opens it with, on a
    /// thread. An empty file reads as the defaults, as no file does.
    pub fn open_file(&self, file: fn(&Path) -> PathBuf, no_folder: &str) {
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        let Some(base) = self.base.clone() else {
            Self::send_back(&tx, &wake, Reload::Failed(no_folder.into()));
            return;
        };
        std::thread::spawn(move || {
            let path = file(&base);
            let made = if path.exists() {
                Ok(())
            } else {
                path.parent()
                    .map_or(Ok(()), |d| std::fs::create_dir_all(d).map_err(|e| msg::coded_at(msg::DIR_CREATE_FAILED, d, &e.to_string())))
                    .and_then(|()| write_atomic(&path, "").map_err(|e| msg::coded_at(msg::FILE_WRITE_FAILED, &path, &e)))
            };
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
                        failed.push(msg::coded_at(msg::FILE_RENAME_FAILED, &path, &e.to_string()));
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
        let (choice, dark, light) = self.common.theme_choice(crate::settings::THEMES);
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
        // `high_contrast = true` draws every theme in its stronger colours, as the Web page does.
        let colors = if self.common.high_contrast_or_off() { colors.high_contrast() } else { colors };
        if colors != ito_theme::colors() {
            ito_theme::set(colors);
            ctx.set_visuals(colors.visuals());
        }
    }
}

pub(crate) fn open_path(path: &Path) -> Result<(), String> {
    ito_openin::open_default(path)
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
    fn a_mail_place_added_is_told_of_failed_and_states_is_written_only_when_it_differs() {
        let m = Mail {
            host: "smtp.example.com".into(),
            from: "a@example.com".into(),
            to: vec!["b@example.com".into()],
            port: 587,
            ..Mail::default()
        };
        let text = write_mails("", std::slice::from_ref(&m)).unwrap();
        assert!(text.contains("[[notify.mails]]") && text.contains("host = \"smtp.example.com\"") && !text.contains("states"));
        let (n, _) = crate::notify::parse(&Config::parse(&text).unwrap());
        assert_eq!(n.mail_places[0].states, ["failed"]);
        let both = Mail { states: vec!["done".into(), "failed".into()], ..m.clone() };
        let text = write_mails(&text, &[m, both]).unwrap();
        assert!(text.contains("states = [\"failed\"]") && text.contains("states = [\"done\", \"failed\"]"));
        assert!(!write_mails(&text, &[]).unwrap().contains("[[notify.mails]]"));
    }

    #[test]
    fn stopping_the_watch_adds_the_key_and_starting_it_again_removes_it() {
        let text = "# mine
keep_days = 30 # keep

[vars]
place = \"x\"
";
        let off = watch_remotes_text(text, false).unwrap();
        assert!(off.contains("# mine") && off.contains("keep_days = 30 # keep") && off.contains("place = \"x\""), "{off}");
        let cfg = Config::parse(&off).unwrap();
        assert!(!cfg.watch_remotes);
        let on = watch_remotes_text(&off, true).unwrap();
        assert!(!on.contains("watch_remotes") && on.contains("# mine") && on.contains("keep_days = 30 # keep"), "{on}");
        assert!(Config::parse(&on).unwrap().watch_remotes);
        // Already as asked: nothing to remove.
        assert_eq!(watch_remotes_text(text, true).unwrap(), text);
    }

    /// A read begun earlier but slow to come back does not undo a newer one.
    #[test]
    fn an_older_read_of_koyomi_toml_does_not_undo_a_newer_one() {
        let mut p = Prefs::empty();
        let read = |off: bool| -> Box<ConfigRead> {
            let config = Config { watch_remotes: !off, ..Config::default() };
            let (n, notes) = crate::notify::parse(&config);
            Box::new(Ok((config, n, notes)))
        };
        p.tx.send(Reload::Config(2, read(true))).unwrap();
        p.tx.send(Reload::Config(1, read(false))).unwrap();
        p.pump();
        assert!(!p.config.watch_remotes, "seq 2 stays");
        // An older failure is dropped too.
        p.tx.send(Reload::Config(1, Box::new(Err("old".into())))).unwrap();
        p.pump();
        assert!(p.notify_warn.is_empty());
        // A newer one is taken.
        p.tx.send(Reload::Config(3, read(false))).unwrap();
        p.pump();
        assert!(p.config.watch_remotes);
        assert!(p.config_known);
    }

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

    /// A PC with no theme in common.toml gets a built-in theme and no
    /// warning under the bell (the warning used to come every frame).
    #[test]
    fn no_theme_in_common_toml_is_a_built_in_one_and_no_warning() {
        let ctx = egui::Context::default();
        let mut p = Prefs::empty();
        p.apply_theme(&ctx);
        assert_eq!(p.theme_err, None);
    }

    #[test]
    fn a_file_moved_aside_keeps_its_name_with_a_stamp() {
        let p = aside(Path::new("C:/uchmk/common.toml"), 42);
        assert_eq!(p.file_name().unwrap().to_string_lossy(), "common.toml.bak-42");
        assert_eq!(p.parent(), Some(Path::new("C:/uchmk")));
    }
}
