//! koyomi: Small job scheduler and monitor (a light JP1)
//!
//! What is wired here: the window (its place kept, its own title bar with the
//! bell and the notice list), common.toml's theme, language and scale (and
//! the settings screen that writes them, Ctrl+,), `--version` / `--help`,
//! F12 (the bug report), `--keys` (a script that drives the window for a
//! test lane), keymap.toml (Ctrl+Q quits) and the key help (F1 or `?`).
//! What the uchmk apps also have and is still to wire is in TODO.md, each
//! item with the ito crate and an app that already does it.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod acked;
mod cli;
mod client;
mod cloud;
mod compare;
mod engine;
mod envreport;
mod eventlog;
mod glossary;
mod help;
mod i18n;
mod icon;
mod keymap;
mod link;
mod mail;
mod mcp;
mod notify;
mod plan;
mod prefs;
mod proto;
mod remote;
mod report;
mod review;
mod service;
mod settings;
mod sftp;
mod sheet;
mod show;
mod sql;
mod ssh;
mod ui;
mod winrm;

use std::path::PathBuf;

use ito_common::window::{Saved, Tracker};
use ito_keymap::Match;
use ito_keys::Key;
use ito_keyscript::pace::{Ended, Ending};
use ito_keyscript::{Do, Runner, Step};

/// The window's icon, in pixels square.
const ICON_PX: u32 = 128;

const REPO: &str = "https://github.com/uchmk/koyomi";

const ABOUT: &str = "koyomi — Small job scheduler and monitor (a light JP1)";

/// The sizes of the screen mock: body 13, buttons and status 12, small 11, the big heading 18;
/// a button's padding is 10 by 3.
fn type_scale(style: &mut egui::Style) {
    use egui::{
        FontFamily::{Monospace, Proportional},
        FontId, TextStyle,
    };
    style.text_styles.insert(TextStyle::Body, FontId::new(13.0, Proportional));
    style.text_styles.insert(TextStyle::Button, FontId::new(12.0, Proportional));
    style.text_styles.insert(TextStyle::Small, FontId::new(11.0, Proportional));
    style.text_styles.insert(TextStyle::Heading, FontId::new(18.0, Proportional));
    style.text_styles.insert(TextStyle::Monospace, FontId::new(12.0, Monospace));
    style.spacing.button_padding = egui::vec2(10.0, 3.0);
}

fn usage_text() -> String {
    format!(
        "{ABOUT}\n\n{}\n  env   print what a bug report needs (version, files, last run); no window\n\n  --keys SCRIPT  press these keys once the window is up, as kura's --keys;\n                 KOYOMI_KEYS_DONE names the file that says how it ended\n\n  {}\n",
        cli::USAGE.trim_end(),
        cli::NOTE.trim_end()
    )
}

struct App {
    table: ito_i18n::Table,
    /// common.toml, the themes and keymap.toml, kept current by a watcher;
    /// the settings screen's state.
    prefs: prefs::Prefs,
    settings_open: bool,
    /// The settings' box that asks before `keep_days` is shortened: the days
    /// wanted, and how many days' records would go at once.
    keep_ask: Option<(u32, usize)>,
    /// The key help (F1 or `?`) while it is open.
    help: help::Help,
    /// The first-run guide (an empty tree), and whether it has been offered this run.
    guide: ito_guide::Guide,
    guide_offered: bool,
    /// Starts at sign-in: what the OS says, why the last click did not
    /// stay (shown under the switch until the next click), and the thread
    /// asking it.
    startup: Option<bool>,
    startup_problem: Option<String>,
    startup_rx: Option<std::sync::mpsc::Receiver<(bool, Option<String>)>>,
    tracker: Option<Tracker>,
    lang: String,
    /// The last scale change, as the line the status bar shows.
    scale_note: Option<String>,
    /// What the bell lists, oldest first: a run that ended 異常 (a click opens
    /// it, by its id) and the engine's notes about `[notify]`.
    notices: Vec<ito_notices::Notice<String>>,
    /// Failed runs the person has dealt with (opened from the bell, or "all read").
    acked: acked::Acked,
    next_notice: u64,
    /// The list under the bell is open, and where the bell was last drawn.
    bell_open: bool,
    bell_at: egui::Rect,
    /// The command palette while it is open (Ctrl+Shift+P, or the bar's box).
    palette: Option<ito_palette::View>,
    browser: ui::Browser,
    /// The state folder (none while a script plays) and the one writer of its `state.toml`, which every
    /// browser of this run gets a clone of, so an old one cannot overwrite what a new one wrote.
    state: Option<PathBuf>,
    state_writer: Option<ito_common::kept::Writer>,
    /// A switch of the link that has been asked for and is not done yet.
    switching: Option<Switching>,
    /// The title last sent to the window.
    title_sent: String,
    /// The Japanese face, read off the UI thread; `None` once it is registered.
    font: Option<std::sync::mpsc::Receiver<Option<Vec<u8>>>>,
    /// The `--keys` script while it plays, and what its steps ask the frame to do.
    script: Option<Runner>,
    todo: Vec<Do>,
    /// The keys in force (defaults/keymap.toml under the person's own), the
    /// chord typed so far, and what the person's file got wrong (under the
    /// bell, once).
    keys: keymap::Keys,
    key_pending: Vec<Key>,
    key_notes: Vec<String>,
    /// The window's door: the engine's tray asks it to come forward (none while a script plays).
    shows: Option<std::sync::mpsc::Receiver<ito_ipc::door::Ask<proto::Show>>>,
}

/// A switch of the link on its way: first the question about unsaved edits, then the wait for what is still being
/// sent.
struct Switching {
    to: link::Link,
    /// The edits were confirmed away (or there were none): now it waits for the requests in flight.
    waiting: bool,
}

/// What the palette can do. Add a variant and an entry in `commands` for each
/// command the app gets.
#[derive(Clone, Copy)]
enum Command {
    Edit(ui::canvas::Cmd),
    NewJobnet,
    NewCalendar,
    Guide,
    Versions,
    Export,
    Import,
    ReportBug,
    ResetScale,
    Settings,
    Help,
    Quit,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, saved: &Saved, script: Option<Runner>) -> Self {
        saved.put(cc);
        // What this run drew with, for the bug report and `koyomi env`.
        let mut used = ito_common::runinfo::RunInfo::new(env!("CARGO_PKG_VERSION"));
        if let Some(rs) = cc.wgpu_render_state.as_ref() {
            let info = rs.adapter.get_info();
            used.adapter = info.name;
            used.backend = format!("{:?}", info.backend);
            used.device = format!("{:?}", info.device_type);
            // The state JSON names the backend the pictures were drawn with.
            ito_keyscript::set_wgpu_backend(info.backend.to_str());
        }
        used.ppp = cc.egui_ctx.pixels_per_point();
        ito_common::runinfo::save("koyomi", &used);
        let state = if script.is_some() { None } else { ito_common::state_dir("koyomi") };
        let state_writer = state.as_ref().map(|d| ito_common::kept::Writer::new(d.join("state.toml"), true));
        let link = ui::kept_link(state.as_deref());
        let base = ito_common::base_dir();
        let mut prefs = prefs::Prefs::load(base.clone());
        let mut warnings: Vec<String> = prefs.common_warn.iter().chain(&prefs.theme_warn).cloned().collect();
        let lang = i18n::resolve(prefs.common.language.as_deref());
        let (table, errors) = i18n::table(&lang);
        warnings.extend(errors);
        let own = base.as_deref().map(prefs::keymap_path);
        let (keys, mut key_notes) = load_keys(own.as_deref());
        for w in warnings.iter().chain(&key_notes) {
            eprintln!("koyomi: {w}");
        }
        // What common.toml got wrong goes under the bell too, once.
        key_notes.extend(warnings);
        prefs.watch(&cc.egui_ctx);
        prefs.apply_theme(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(ito_theme::colors().visuals());
        cc.egui_ctx.all_styles_mut(type_scale);
        // The scale is common.toml's, so egui's own Ctrl+= / Ctrl+- stay out of it.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        cc.egui_ctx.set_zoom_factor(prefs.common.scale_or_one());
        let shows = script.is_none().then(|| {
            let ctx = cc.egui_ctx.clone();
            proto::window_door().listen(move || ctx.request_repaint())
        });
        Self {
            table,
            prefs,
            settings_open: false,
            keep_ask: None,
            help: help::Help::default(),
            guide: ito_guide::Guide::default(),
            guide_offered: false,
            startup: None,
            startup_problem: None,
            startup_rx: None,
            tracker: saved.tracker(),
            lang,
            scale_note: None,
            notices: Vec::new(),
            acked: acked::Acked::load_for(state.clone(), link.far()),
            next_notice: 0,
            bell_open: false,
            bell_at: egui::Rect::NOTHING,
            palette: None,
            // A script plays on a window that keeps no state of its own.
            browser: ui::Browser::start(&cc.egui_ctx, state.clone(), link, state_writer.clone()),
            state,
            state_writer,
            switching: None,
            title_sent: String::new(),
            font: Some(font_reader(&cc.egui_ctx)),
            script,
            todo: Vec::new(),
            keys,
            key_pending: Vec::new(),
            key_notes,
            shows,
        }
    }

    /// A switch of the link, the only place it happens: the question about unsaved edits, the wait for what is
    /// still being sent (no way to switch without waiting), then a new browser with its own notices and
    /// acknowledgements, and the link written to `state.toml`.
    fn pump_link(&mut self, ctx: &egui::Context) {
        if self.switching.is_none() {
            if let Some(to) = self.browser.take_link_wish() {
                let waiting = !self.browser.has_unsaved();
                self.switching = Some(Switching { to, waiting });
            }
        }
        let Some(sw) = &self.switching else { return };
        if !sw.waiting {
            let t = &self.table;
            let buttons = [t.tr("link.switch").to_owned(), t.tr("confirm.no").to_owned()];
            let body = [t.tr("link.unsaved_body").to_owned()];
            let q = ito_confirm::Question {
                title: t.tr("link.unsaved_title"),
                body: &body,
                buttons: &buttons,
                marks: &[Some(ito_prefs::Mark::Check), Some(ito_prefs::Mark::Close)],
                focus: Some(1),
                primary: Some(1),
                danger: true,
                ..Default::default()
            };
            let key = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape).then_some(1));
            match ito_confirm::show(ctx, &q, &ui::Browser::style()).or(key) {
                Some(0) => {
                    if let Some(sw) = &mut self.switching {
                        sw.waiting = true;
                    }
                }
                Some(_) => self.switching = None,
                None => {}
            }
            return;
        }
        let left = self.browser.switch_step();
        if left > 0 {
            let t = &self.table;
            let buttons = [t.tr("confirm.no").to_owned()];
            let body = [t.trf("link.wait_body", &[&left.to_string()])];
            let q = ito_confirm::Question {
                title: t.tr("link.wait_title"),
                body: &body,
                buttons: &buttons,
                marks: &[Some(ito_prefs::Mark::Close)],
                focus: Some(0),
                primary: Some(0),
                ..Default::default()
            };
            let key = ctx.input_mut(|i| {
                (i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) || i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                    .then_some(0)
            });
            if ito_confirm::show(ctx, &q, &ui::Browser::style()).or(key).is_some() {
                self.browser.cancel_switch();
                self.switching = None;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        if let Some(sw) = self.switching.take() {
            self.switch_link(ctx, sw.to);
        }
    }

    /// The new browser, with a clone of the one writer of `state.toml`; the bell and the acknowledgements
    /// of the old link go.
    fn switch_link(&mut self, ctx: &egui::Context, to: link::Link) {
        self.browser = ui::Browser::start(ctx, self.state.clone(), to.clone(), self.state_writer.clone());
        reset_for_link(&mut self.notices, &mut self.bell_open, &mut self.acked, self.state.clone(), &to);
        self.browser.save_state();
    }

    /// What the tray asked: come forward, on 今日 when it says so.
    fn take_shows(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.shows else { return };
        while let Ok(ask) = rx.try_recv() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            // The tray belongs to this PC: a run, a failure or a skip it names is not on another PC's engine.
            let far = self.browser.link().is_far();
            match &ask.request {
                proto::Show::Today => {
                    self.settings_open = false;
                    self.browser.show_today();
                }
                proto::Show::Failed if !far => {
                    self.settings_open = false;
                    self.browser.show_failed();
                }
                proto::Show::SkipNext if !far => {
                    self.settings_open = false;
                    self.browser.show_skip_next();
                }
                proto::Show::Run(id) if !far => {
                    self.settings_open = false;
                    self.browser.show_run(id);
                }
                proto::Show::Failed | proto::Show::SkipNext | proto::Show::Run(_) | proto::Show::Front => {}
            }
            ask.answer(Ok(String::new()));
        }
    }

    /// The keys of the frame that the keymap may run: after the fixed ones
    /// (F12, the palette, the scale) took theirs, and not while the palette
    /// is open. A bare character goes to the widget that has the focus.
    fn read_keys(&mut self, ctx: &egui::Context) {
        if self.palette.is_some() {
            return;
        }
        let typing = ctx.memory(|m| m.focused().is_some());
        let events = ctx.input(|i| i.events.clone());
        for e in events {
            let k = match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => ito_keys::from_egui(key, &modifiers),
                egui::Event::Text(t) if !typing => t.chars().next().map(Key::char),
                _ => None,
            };
            if let Some(k) = k {
                self.press(ctx, k);
            }
        }
    }

    /// One key of a chord: run what it completes, wait for more, or drop
    /// what led nowhere (and try the key alone).
    fn press(&mut self, ctx: &egui::Context, k: Key) {
        self.key_pending.push(k);
        let run = match self.keys.resolve(&self.key_pending) {
            Match::Exact(b) => b.run.clone(),
            Match::Pending(_) => return,
            Match::None => {
                let chord = self.key_pending.len() > 1;
                self.key_pending.clear();
                if chord {
                    self.press(ctx, k);
                }
                return;
            }
        };
        self.key_pending.clear();
        for c in run {
            self.run_key(ctx, c);
        }
    }

    /// What a bound key does.
    fn run_key(&mut self, ctx: &egui::Context, c: keymap::Cmd) {
        match c {
            keymap::Cmd::Run("quit") => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            keymap::Cmd::Run("settings") => self.open_settings(),
            keymap::Cmd::Run("help") => self.help.toggle(),
            // The editor has keys of its own; a new net starts from outside it.
            keymap::Cmd::Run("new_jobnet") if !self.browser.editing() => self.browser.new_box(None),
            keymap::Cmd::Run(_) | keymap::Cmd::None => {}
        }
    }

    /// The settings cover the tree; the help, if open, goes with it.
    fn open_settings(&mut self) {
        self.help.close();
        self.settings_open = true;
    }

    /// `<State:name>` and the done file: what the window shows, then `tail`.
    fn write_state(&self, path: PathBuf, tail: &str) {
        let text = format!("{}{tail}", self.browser.report());
        if let Err(e) = std::fs::write(&path, text) {
            say(&format!("koyomi --keys: {}: {e}", path.display()), true);
        }
    }

    /// What koyomi knows of its own screen, for the state JSON. What it does not know stays `None`, which the file
    /// writes as null: nothing is guessed. koyomi shows no in-app toasts (its notes are OS notifications), so none.
    fn keys_app_part(&self) -> ito_keyscript::state::App {
        let overlay = if self.palette.is_some() {
            Some("palette")
        } else if self.settings_open {
            Some("settings")
        } else if self.help.open {
            Some("help")
        } else {
            None
        };
        ito_keyscript::state::App {
            screen: Some(if self.browser.editing() { "editor" } else { "browser" }.to_owned()),
            overlay: Some(overlay.map(str::to_owned)),
            toasts: None,
            app_state: None,
        }
    }

    /// The steps of the `--keys` script that are not keys: pictures, state
    /// files, the theme and the language (set on this window's copy of
    /// common.toml, which `sync_common` applies; the owner's file is not
    /// written), quit, and the done file.
    fn script(&mut self, ctx: &egui::Context) {
        let Some(script) = &mut self.script else { return };
        if let Some(err) = script.shot(ctx) {
            say(&format!("koyomi --keys: {err}"), true);
        }
        for d in std::mem::take(&mut self.todo) {
            let Some(script) = &self.script else { return };
            match d {
                Do::State(name) => {
                    self.write_state(script.dir.join(format!("{name}.txt")), "");
                    // The state JSON is held until the app's part is in, in this same frame.
                    let part = self.keys_app_part();
                    if let Some(script) = &mut self.script {
                        if let Err(e) = script.write_state(ctx, &name, part) {
                            say(&format!("koyomi --keys: <State:{name}>: {e}"), true);
                        }
                    }
                }
                Do::Quit => {
                    // `<Quit>` finishes the run itself, so `on_exit` finds it over and writes nothing.
                    if let Some(done) = script.done.clone() {
                        self.write_state(done, &ito_keyscript::quit_report(script.left()));
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Do::Theme(name) => {
                    self.prefs.common.theme = Some(name.clone());
                    self.prefs.apply_theme(ctx);
                    if let Some(e) = self.prefs.theme_err.take() {
                        say(&format!("koyomi --keys: <Theme:{name}>: {e}"), true);
                    }
                }
                Do::Lang(code) => {
                    self.prefs.common.language = Some(code.clone());
                    for e in self.sync_lang() {
                        say(&format!("koyomi --keys: <Lang:{code}>: {e}"), true);
                    }
                }
                // koyomi defines no readings of its own.
                Do::Read { .. } => {}
                Do::Failed { step, why } => {
                    say(&format!("koyomi --keys: {step}: {why}"), true);
                    if let Some(done) = script.done.clone() {
                        self.write_state(done, &ito_keyscript::failed_report(&step, &why, script.left()));
                    }
                    std::process::exit(ito_keyscript::FAILED_EXIT);
                }
                // The run ended with a step gone in on a screen that had not settled: the tail goes to the done
                // file, is printed, and the run fails.
                Do::Unsettled(tail) => {
                    if let Some(done) = script.done.clone() {
                        self.write_state(done, &tail);
                    }
                    say(tail.trim_end(), true);
                    std::process::exit(ito_keyscript::FAILED_EXIT);
                }
                Do::Done => {
                    if let Some(done) = script.done.clone() {
                        self.write_state(done, "keys: done\n");
                    }
                }
            }
        }
        if let Some(due) = self.script.as_ref().and_then(Runner::due) {
            ctx.request_repaint_after(due);
        }
    }

    /// Registers the Japanese face once the reader thread has it, so names do not draw as boxes.
    fn take_font(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.font else { return };
        let bytes = match rx.try_recv() {
            Ok(b) => b,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(_) => None,
        };
        self.font = None;
        let mut fonts = egui::FontDefinitions::default();
        ito_fonts::register(&mut fonts, self.prefs.common.mono_font_or_default());
        if let Some(bytes) = bytes {
            fonts.font_data.insert("japanese".to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push("japanese".to_owned());
            }
        }
        ctx.set_fonts(fonts);
    }

    /// Ctrl+wheel and Ctrl+= / Ctrl+- / Ctrl+0: the whole window's scale, one
    /// step at a time. It is applied now and written to common.toml off the UI
    /// thread, so the other apps at the same desk follow within 2 seconds.
    fn rescale(&mut self, ctx: &egui::Context) {
        use ito_common::ScaleTo;
        let wheel = ctx.input(|i| if i.modifiers.command { i.smooth_scroll_delta.y } else { 0.0 });
        let to = if wheel > 0.0 {
            Some(ScaleTo::In)
        } else if wheel < 0.0 {
            Some(ScaleTo::Out)
        } else {
            let key = |k| ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, k));
            if key(egui::Key::Equals) || key(egui::Key::Plus) {
                Some(ScaleTo::In)
            } else if key(egui::Key::Minus) {
                Some(ScaleTo::Out)
            } else if key(egui::Key::Num0) {
                Some(ScaleTo::Reset)
            } else {
                None
            }
        };
        if let Some(to) = to {
            self.scale_to(ctx, to);
        }
    }

    fn scale_to(&mut self, ctx: &egui::Context, to: ito_common::ScaleTo) {
        let now = self.prefs.common.scale_or_one();
        let next = ito_common::scale_step(now, to);
        if next == now {
            return;
        }
        self.prefs.set_scale(next, self.table.tr("settings.no_folder"));
        ctx.set_zoom_factor(next);
        ito_statusbar::scale_changed(ctx, next);
        self.scale_note = Some(ito_common::scale_text(next, &self.lang));
    }

    /// common.toml changed (the settings, another app at the same desk, an
    /// editor): the language, the theme and the scale follow; keymap.toml's
    /// change reloads the keys. What a writer could not do goes under the bell.
    fn sync_common(&mut self, ctx: &egui::Context) {
        self.prefs.pump();
        if let Some(e) = self.prefs.failed.take() {
            self.key_notes.push(e);
        }
        match self.prefs.keymap_new.take() {
            Some(None) => {
                let own = self.prefs.base.as_deref().map(prefs::keymap_path);
                let (keys, notes) = load_keys(own.as_deref());
                self.keys = keys;
                self.key_pending.clear();
                self.key_notes.extend(notes);
            }
            Some(Some(e)) => self.key_notes.push(e),
            None => {}
        }
        for e in self.sync_lang() {
            self.key_notes.push(e);
        }
        self.prefs.apply_theme(ctx);
        if let Some(e) = self.prefs.theme_err.take() {
            self.key_notes.push(e);
        }
        let scale = self.prefs.common.scale_or_one();
        if ctx.zoom_factor() != scale {
            ctx.set_zoom_factor(scale);
        }
    }

    /// The table follows common.toml's language; the lang files' errors come back.
    fn sync_lang(&mut self) -> Vec<String> {
        let lang = i18n::resolve(self.prefs.common.language.as_deref());
        if lang == self.lang {
            return Vec::new();
        }
        let (table, errors) = i18n::table(&lang);
        self.lang = lang;
        self.table = table;
        errors
    }

    /// The bottom bar: the app's items on the left (add yours here), the clock
    /// from common.toml at the right end, the scale's words after a change.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let clock = self.prefs.common.clock_or(None);
        let spec = ito_statusbar::Spec { clock: &clock, lang: &self.lang, opacity: 1.0 };
        let waiting = self.acked.waiting(self.browser.failed_ids());
        let (engine, running, next) = self.browser.engine_facts(&self.table);
        let running_line = (running > 0).then(|| self.table.trf("status.running", &[&running.to_string()]));
        let next_line = next.map(|n| self.table.trf("status.next", &[&n.format("%m-%d %H:%M").to_string()]));
        let todo = self.browser.todo_count();
        let todo_line = (todo > 0).then(|| self.table.trf("status.todo", &[&todo.to_string()]));
        let ok_line = engine.is_none().then(|| self.table.tr("status.engine_ok").to_owned());
        let ha = self.browser.ha_fact(&self.table);
        let line = (waiting > 0).then(|| self.table.trf("status.unacked", &[&waiting.to_string()]));
        let link_line = self.browser.link_label();
        let sending = self.browser.pending();
        let sending_line = (self.browser.link().is_far() && sending > 0).then(|| self.table.trf("link.sending", &[&sending.to_string()]));
        let (browser, table) = (&mut self.browser, &self.table);
        ito_statusbar::show(
            ui,
            &spec,
            |ui| {
                browser.link_picker(ui, table);
                if let Some(l) = &link_line {
                    ito_statusbar::item(ui, l, ito_theme::colors().strong());
                }
                if let Some(l) = &sending_line {
                    ito_statusbar::item(ui, l, ito_theme::colors().wait);
                }
                if let Some(l) = &engine {
                    ito_statusbar::item(ui, l, ito_theme::colors().err);
                }
                if let Some(l) = &ok_line {
                    ito_statusbar::item(ui, l, ito_theme::colors().dim);
                }
                if let Some((l, tone)) = &ha {
                    ito_statusbar::item(ui, l, tone.color());
                }
                if let Some(l) = &running_line {
                    ito_statusbar::item(ui, l, ito_theme::colors().run);
                }
                if let Some(l) = &next_line {
                    ito_statusbar::item(ui, l, ito_theme::colors().dim);
                }
                if let Some(l) = &todo_line {
                    ito_statusbar::item(ui, l, ito_theme::colors().wait);
                }
                if let Some(l) = &line {
                    ito_statusbar::item(ui, l, ito_theme::colors().err);
                }
            },
            |_| {},
        );
    }

    /// The window's own bar: the name, the palette box, the bell and the
    /// caption buttons. The bell opens the list of notices.
    fn title_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let unread = ito_notices::unread(&self.notices);
        let bell = self.table.trf("title.bell", &[&unread.to_string()]);
        let spec = ito_titlebar::Spec {
            own: !cfg!(target_os = "macos"),
            left: if cfg!(target_os = "macos") { 76.0 } else { 0.0 },
            maximized,
            unread,
            any_error: ito_notices::any_error(&self.notices),
            centre: true,
            right_min: 0.0,
            words: ito_titlebar::Words {
                search: self.table.tr("title.search"),
                search_key: "Ctrl+Shift+P",
                bell: &bell,
                minimize: self.table.tr("title.minimize"),
                maximize: self.table.tr("title.maximize"),
                restore: self.table.tr("title.restore"),
                close: self.table.tr("title.close"),
            },
        };
        let mut name = self.table.tr("app.title").to_string();
        if let Some(l) = self.browser.link_label() {
            name = format!("{name} - {l}");
        }
        if name != self.title_sent {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(name.clone()));
            self.title_sent = name.clone();
        }
        let out = ito_titlebar::show(
            ui,
            &spec,
            // The logo, then the name (as the other uchmk apps' title bars).
            |ui, rect| ito_titlebar::brand(ui, rect, &name, icon::pixels),
            |_, _| {},
        );
        if let Some(r) = out.bell_rect {
            self.bell_at = r;
        }
        if out.bell {
            self.bell_open = !self.bell_open;
        }
        if out.search {
            self.palette = Some(ito_palette::View::new());
        }
        if let Some(op) = out.window {
            ito_titlebar::apply(&ctx, op, maximized);
        }
    }

    fn commands(&self) -> Vec<ito_palette::Entry<Command>> {
        let t = &self.table;
        let mut out = Vec::new();
        if self.browser.editing() {
            use ui::canvas::Cmd;
            for (key, hint, cmd) in [
                ("palette.edit_save", "Ctrl+S", Cmd::Save),
                ("palette.edit_align", "", Cmd::Align),
                ("palette.edit_fit", "", Cmd::Fit),
                ("palette.edit_flow", "", Cmd::Flow),
                ("palette.edit_undo", "Ctrl+Z", Cmd::Undo),
                ("palette.edit_redo", "Ctrl+Y", Cmd::Redo),
                ("palette.edit_close", "", Cmd::Close),
            ] {
                out.push(ito_palette::Entry::new(t.tr(key), hint, Command::Edit(cmd)));
            }
        }
        out.extend([
            ito_palette::Entry::new(t.tr("palette.new_jobnet"), self.keys.label("new_jobnet"), Command::NewJobnet),
            ito_palette::Entry::new(t.tr("palette.new_calendar"), "", Command::NewCalendar),
            ito_palette::Entry::new(t.tr("palette.guide"), "", Command::Guide),
            ito_palette::Entry::new(t.tr("palette.versions"), "", Command::Versions),
            ito_palette::Entry::new(t.tr("palette.export"), "", Command::Export),
            ito_palette::Entry::new(t.tr("palette.import"), "", Command::Import),
            ito_palette::Entry::new(t.tr("palette.report_bug"), "F12", Command::ReportBug),
            ito_palette::Entry::new(t.tr("palette.reset_scale"), "Ctrl+0", Command::ResetScale),
            ito_palette::Entry::new(t.tr("palette.settings"), self.keys.label("settings"), Command::Settings),
            ito_palette::Entry::new(t.tr("palette.help"), self.keys.label("help"), Command::Help),
            ito_palette::Entry::new(t.tr("palette.quit"), self.keys.label("quit"), Command::Quit),
        ]);
        out
    }

    /// Ctrl+Shift+P opens the palette; while it is open it takes the keys.
    fn command_palette(&mut self, ctx: &egui::Context) {
        let open = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::P));
        if open && self.palette.is_none() {
            self.palette = Some(ito_palette::View::new());
        }
        let Some(mut view) = self.palette.take() else { return };
        let c = ito_theme::colors();
        let style = ito_palette::Style {
            panel: c.raised(),
            border: c.border,
            text: c.fg,
            dim: c.dim,
            selected_bg: c.chosen(),
            selected_text: c.strong(),
            hit: c.wait,
            accent: c.done,
        };
        let words = ito_palette::Words { hint: self.table.tr("palette.hint"), none: self.table.tr("palette.none"), header: "", footer: "" };
        match ito_palette::show(ctx, &mut view, &self.commands(), &style, &words) {
            None => self.palette = Some(view),
            Some(ito_palette::Answer::Pick(Command::NewJobnet)) => self.browser.new_box(None),
            Some(ito_palette::Answer::Pick(Command::NewCalendar)) => self.browser.new_cal_box(),
            Some(ito_palette::Answer::Pick(Command::Guide)) => {
                self.guide_problems();
                self.guide.reopen();
            }
            Some(ito_palette::Answer::Pick(Command::Versions)) => self.browser.open_versions(),
            Some(ito_palette::Answer::Pick(Command::Export)) => self.browser.export(&self.table),
            Some(ito_palette::Answer::Pick(Command::Import)) => self.browser.import_box(),
            Some(ito_palette::Answer::Pick(Command::ReportBug)) => self.report_bug(ctx),
            Some(ito_palette::Answer::Pick(Command::ResetScale)) => self.scale_to(ctx, ito_common::ScaleTo::Reset),
            Some(ito_palette::Answer::Pick(Command::Settings)) => self.open_settings(),
            Some(ito_palette::Answer::Pick(Command::Help)) => self.help.toggle(),
            Some(ito_palette::Answer::Pick(Command::Quit)) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Some(ito_palette::Answer::Pick(Command::Edit(c))) => self.browser.edit(c),
            Some(_) => {}
        }
    }

    /// What a beginner meets first: the tree has no definition yet.
    fn guide_problems(&mut self) {
        let t = &self.table;
        let problems = if self.browser.tree_is_empty() {
            vec![ito_guide::Problem::new(
                t.tr("guide.empty"),
                vec![
                    ito_guide::Step::new("koyomi new sample/hello", t.tr("guide.new_cli")),
                    ito_guide::Step::new("koyomi approve sample/hello", t.tr("guide.approve")),
                    ito_guide::Step::new("koyomi run sample/hello", t.tr("guide.run")),
                ],
            )]
        } else {
            Vec::new()
        };
        self.guide.set(problems);
    }

    /// The guide opens once, when the engine has answered and there is nothing to run yet.
    fn show_guide(&mut self, ctx: &egui::Context) {
        if !self.guide_offered && self.browser.snapshot_known() {
            self.guide_offered = true;
            self.guide_problems();
        }
        if self.guide.is_open() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.guide.escape();
        }
        let t = &self.table;
        let footer = [t.tr("guide.footer")];
        let words = ito_guide::Words { title: t.tr("guide.title"), lead: t.tr("guide.lead"), footer: &footer };
        if let Some(text) = ito_guide::show(ctx, &mut self.guide, &words).copy {
            ctx.copy_text(text);
        }
    }

    /// The list under the bell, while it is open.
    fn notice_list(&mut self, ctx: &egui::Context) {
        if !self.bell_open {
            return;
        }
        let t = &self.table;
        let title = t.trf("notices.title", &[&ito_notices::unread(&self.notices).to_string()]);
        let words = ito_notices::Words {
            title: &title,
            mark_read: t.tr("notices.mark_read"),
            empty: t.tr("notices.empty"),
            ago: t.tr("notices.ago"),
            kinds: [t.tr("notices.ok"), t.tr("notices.info"), t.tr("notices.warn"), t.tr("notices.err"), t.tr("notices.wait")],
        };
        match ito_notices::show(ctx, self.bell_at, &self.notices, &words) {
            Some(ito_notices::Action::Open(go, id)) => {
                self.notices.iter_mut().filter(|n| n.id == id).for_each(|n| n.read = true);
                self.bell_open = false;
                if let Some(run) = go {
                    self.acked.ack([run.as_str()]);
                    self.browser.link_run(&run);
                }
            }
            Some(ito_notices::Action::ReadAll) => {
                self.notices.iter_mut().for_each(|n| n.read = true);
                let ids: Vec<String> = self.browser.failed_ids().map(str::to_owned).collect();
                self.acked.ack(ids.iter().map(String::as_str));
            }
            Some(ito_notices::Action::Close) => self.bell_open = false,
            None => {}
        }
    }

    /// A run that ended 異常, and a note about `[notify]`, go under the bell.
    /// (The system's notification is the engine's: it is the same whoever
    /// started the run, and shows with no window open.)
    fn take_notices(&mut self) {
        let mut add: Vec<(ito_notices::Kind, String, Option<String>, Option<String>)> = Vec::new();
        let (ended, missed) = notify::group_missed(self.browser.take_ended());
        for r in ended {
            if r.state.is_error() {
                let (_, title, body) = notify::words(&self.table, &r);
                add.push((ito_notices::Kind::Err, title, Some(body), Some(r.id.clone())));
            }
        }
        for runs in missed {
            let (_, title, body) = notify::missed_words(&self.table, &runs);
            add.push((ito_notices::Kind::Warn, title, Some(body), Some(runs[0].id.clone())));
        }
        for n in self.browser.take_notes().into_iter().chain(std::mem::take(&mut self.key_notes)) {
            add.push((ito_notices::Kind::Warn, n, None, None));
        }
        for (kind, title, note, go) in add {
            self.next_notice += 1;
            self.notices.push(ito_notices::Notice {
                id: self.next_notice,
                at: std::time::SystemTime::now(),
                kind,
                title,
                note,
                go,
                read: false,
            });
        }
    }

    /// F12: the browser opens the issue form filled with the version, the OS
    /// and what the app knows.
    fn report_bug(&self, ctx: &egui::Context) {
        let spec = ito_bugreport::Spec { app: "koyomi", version: env!("CARGO_PKG_VERSION"), repo: REPO, template: "bug_report.yml" };
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(std::path::PathBuf::from);
        let context = ito_bugreport::Context { home: home.as_deref(), ..Default::default() };
        let url = ito_bugreport::url(&spec, &[("context", ito_bugreport::context(&context))]);
        ctx.open_url(egui::OpenUrl::new_tab(url));
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.sync_common(&ctx);
        self.take_shows(&ctx);
        // The Keys page is taking a key: every key is that key, F12 too.
        let taking = self.settings_open && self.prefs.keys_edit.busy();
        if !taking && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F12)) {
            self.report_bug(&ctx);
        }
        if !taking {
            self.rescale(&ctx);
        }
        if let Some(t) = &mut self.tracker {
            t.follow(&ctx);
        }
        egui::Panel::top("titlebar")
            .exact_size(ito_titlebar::HEIGHT)
            .frame(egui::Frame::NONE.fill(ito_theme::colors().bg))
            .show(ui, |ui| self.title_bar(ui));
        let acks = self.browser.take_acks();
        self.acked.ack(acks.iter().map(String::as_str));
        if let Some(ids) = self.acked.take_changed() {
            self.browser.set_acked(ids);
        }
        egui::Panel::bottom("statusbar").exact_size(ito_statusbar::HEIGHT).frame(egui::Frame::NONE).show(ui, |ui| self.status_bar(ui));
        self.pump_link(&ctx);
        self.browser.easy = self.prefs.common.easy_or_off().then(|| self.lang.clone());
        self.take_notices();
        self.notice_list(&ctx);
        if !taking {
            self.command_palette(&ctx);
        }
        self.take_font(&ctx);
        self.show_guide(&ctx);
        if self.settings_open {
            // The engine's news still comes in while the settings cover the tree.
            self.browser.drain();
            settings::show(ui, self);
        } else {
            let r = ui.max_rect();
            if self.help.open {
                // Esc and the scrolling keys are the help's before the keymap reads them.
                self.help.keys(&ctx);
            }
            self.read_keys(&ctx);
            self.browser.show(ui, &self.table);
            if self.help.open {
                help::show(ui, self, r);
            }
        }
        self.script(&ctx);
        // Last, so the edges of the frameless window win over what is under them.
        if !cfg!(target_os = "macos") {
            ito_titlebar::resize_edges(&ctx);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        ito_theme::colors().bg.to_normalized_gamma_f32()
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(script) = &mut self.script {
            if let Some(d) = script.input(ctx, raw) {
                self.todo.push(d);
            }
        }
    }

    /// The window closed from outside while a script ran: the done file says
    /// so, else the lane waits out its timeout.
    fn on_exit(&mut self) {
        // The window's last place, written now rather than on the tracker's clock.
        if let Some(t) = &mut self.tracker {
            t.close(None);
        }
        let Some(script) = &mut self.script else { return };
        if !script.running() {
            return;
        }
        // Closed from outside while the script ran: the run's own tail says how it ended (koyomi writes no
        // state report of its own, so the tail is the whole done file).
        let Some(ended) = script.end(Ending::Closed) else { return };
        let Ended { tail, say: line, failed, .. } = ended;
        if let Some(done) = script.done.clone() {
            self.write_state(done, &tail);
        }
        if let Some(line) = line {
            say(&line, true);
        }
        if failed {
            std::process::exit(ito_keyscript::FAILED_EXIT);
        }
    }
}

/// The keys: the person's keymap.toml at `own` (none is fine) over the
/// defaults. What the file got wrong comes back as the lines to show.
fn load_keys(own: Option<&std::path::Path>) -> (keymap::Keys, Vec<String>) {
    let mut notes = Vec::new();
    let text = own.and_then(|p| match std::fs::read_to_string(p) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            notes.push(format!("{}: {e}", p.display()));
            None
        }
    });
    let path = own.map(|p| p.display().to_string()).unwrap_or_default();
    let (keys, warnings) = keymap::Keys::load(text.as_deref().map(|t| (path.as_str(), t)));
    notes.extend(warnings);
    (keys, notes)
}

/// `--keys SCRIPT` as its steps; `None` when the arguments are not that.
/// `-x` after `--keys` is the script's text, not an option.
/// keys.ps1 (ito) puts the fixture folder first, as kura takes it. koyomi has no folder to open, so that
/// one argument is read past and not used; without it the run would stop at "unknown argument".
fn keys_arg(args: &[String]) -> Result<Option<Vec<Step>>, String> {
    let args = match args {
        [dir, rest @ ..] if !dir.starts_with('-') && rest.first().is_some_and(|k| k == "--keys") => rest,
        _ => args,
    };
    match args {
        [k, ..] if k != "--keys" => Ok(None),
        [_] => Err("--keys needs a script".to_owned()),
        [_, s] => ito_keyscript::parse(s).map(Some).map_err(|e| format!("--keys: {e}")),
        [_, _, more, ..] => Err(format!("unknown argument `{more}`")),
        [] => Ok(None),
    }
}

/// Reads the Japanese face on a thread of its own (it is tens of megabytes) and repaints when it is in.
fn font_reader(ctx: &egui::Context) -> std::sync::mpsc::Receiver<Option<Vec<u8>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(ito_common::fonts::read_first(ito_common::fonts::JAPANESE).map(|(_, b)| b));
        ctx.request_repaint();
    });
    rx
}

/// Puts a line where whoever typed the command is looking, on standard error
/// when `err`. A release build on Windows has no console of its own;
/// `ito_console` writes to the console it was started from, or to the pipe
/// `koyomi.com` hands on, and lets go of a write that fails.
fn say(text: &str, err: bool) {
    if let Some(log) = LOG.get() {
        use std::io::Write;
        let stamp = ito_common::now::now_local().format("%Y-%m-%d %H:%M:%S");
        let mut f = log.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(f, "{stamp} {text}");
        return;
    }
    let line = format!("{text}\n");
    match err {
        true => ito_console::err(&line),
        false => ito_console::out(&line),
    }
}

/// Where [`say`] writes instead, once set: the service's `engine.log`, as it has no console.
static LOG: std::sync::OnceLock<std::sync::Mutex<std::fs::File>> = std::sync::OnceLock::new();

/// Sends [`say`] to the end of `path` from now on. A file bigger than `keep` is set aside as
/// `<name>.old.log` first, so it does not grow without end. Stays on the console if it cannot open.
fn log_to(path: &std::path::Path, keep: u64) {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > keep) {
        let _ = std::fs::rename(path, path.with_extension("old.log"));
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = LOG.set(std::sync::Mutex::new(f));
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = cli::run(&args) {
        std::process::exit(code);
    }
    let done = std::env::var_os("KOYOMI_KEYS_DONE").filter(|v| !v.is_empty()).map(PathBuf::from);
    let script = match keys_arg(&args) {
        Ok(s) => s,
        Err(e) => {
            // A run started detached never sees the message: the done file carries it (ito #193).
            if let Some(done) = &done {
                let _ = std::fs::write(done, ito_keyscript::refused_report(&e));
            }
            say(&format!("koyomi: {e}\n\n{}", usage_text()), true);
            std::process::exit(2);
        }
    };
    if let (Some(_), Some(done)) = (&script, &done) {
        ito_keyscript::install_panic_report(done.clone());
    }
    // The window talks to the engine at KOYOMI_ADDRESS. A script without it would reach the real engine (its jobs
    // included), so it stops before the window opens.
    if script.is_some() && std::env::var_os(proto::ENV).is_none_or(|v| v.is_empty()) {
        let why = "KOYOMI_ADDRESS がない。--keys の窓は本物のエンジンにつながるので、回ごとに別の名前を渡してから走らせる";
        if let Some(done) = &done {
            let _ = std::fs::write(done, ito_keyscript::refused_report(why));
        }
        say(&format!("koyomi --keys: {why}"), true);
        std::process::exit(2);
    }
    if let (Some(a), None) = (args.first(), &script) {
        match a.as_str() {
            "--version" | "-V" => say(&ito_bugreport::version_line("koyomi", env!("CARGO_PKG_VERSION")), false),
            "--help" | "-h" => say(&usage_text(), false),
            "env" | "--env" => say(envreport::text().trim_end_matches('\n'), false),
            other => {
                say(&format!("koyomi: unknown argument `{other}`\n\n{}", usage_text()), true);
                std::process::exit(2);
            }
        }
        return Ok(());
    }
    // A script plays on a window that keeps no place, and leaves none.
    let saved = if script.is_none() { Saved::load(ito_common::state_dir("koyomi")) } else { Saved::load(None) };
    let viewport = egui::ViewportBuilder::default()
        .with_title("koyomi")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([720.0, 420.0])
        .with_icon(std::sync::Arc::new(egui::IconData { rgba: icon::pixels(ICON_PX), width: ICON_PX, height: ICON_PX }));
    // The bar is the app's own (the Mac keeps its traffic lights over it).
    let viewport = ito_titlebar::frameless(viewport);
    // GL first on Windows: Vulkan once lost its staging buffer mid-frame and took the app down
    // (egui-wgpu panicked, then the swapchain panicked again while unwinding). WGPU_BACKEND still wins.
    let (wgpu_options, _) = ito_gpu::options(None);
    let options = eframe::NativeOptions { viewport: saved.viewport(viewport), wgpu_options, ..Default::default() };
    eframe::run_native(
        "koyomi",
        options,
        Box::new(move |cc| {
            let script = script.map(|steps| {
                // The state JSON (`<State:>` and `<Snap:>`): the window hands ito its part in `script()`.
                let mut r = Runner::new(steps, done).app("koyomi", env!("CARGO_PKG_VERSION")).state_json(true);
                r.watch(cc.egui_ctx.clone());
                r
            });
            Ok(Box::new(App::new(cc, &saved, script)))
        }),
    )
}

/// What a switch of the link leaves behind: the notices and the open bell of the old link go, and the
/// acknowledgements are those of the new link's own file.
fn reset_for_link(
    notices: &mut Vec<ito_notices::Notice<String>>,
    bell_open: &mut bool,
    acked: &mut acked::Acked,
    state: Option<PathBuf>,
    to: &link::Link,
) {
    notices.clear();
    *bell_open = false;
    *acked = acked::Acked::load_for(state, to.far());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_argument_is_a_script_and_anything_after_it_is_refused() {
        let a = |v: &[&str]| keys_arg(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(matches!(a(&[]), Ok(None)));
        assert!(matches!(a(&["--version"]), Ok(None)));
        assert_eq!(a(&["--keys", "j<Enter><Shot:cpu>"]).unwrap().unwrap().len(), 3);
        assert!(matches!(a(&["--keys", "-x"]), Ok(Some(_))));
        assert_eq!(a(&["--keys"]).unwrap_err(), "--keys needs a script");
        assert!(a(&["--keys", "<Nope>"]).unwrap_err().starts_with("--keys:"));
        assert_eq!(a(&["--keys", "j", "--more"]).unwrap_err(), "unknown argument `--more`");
        // keys.ps1 puts the fixture folder first; koyomi reads past it.
        assert_eq!(a(&["C:\\fixtures", "--keys", "j<Enter>"]).unwrap().unwrap().len(), 2);
        assert_eq!(a(&["C:\\fixtures", "--keys"]).unwrap_err(), "--keys needs a script");
        assert!(matches!(a(&["C:\\fixtures"]), Ok(None)));
        assert!(matches!(a(&["--version", "--keys", "j"]), Ok(None)));
    }

    #[test]
    fn a_switch_leaves_the_new_links_own_acknowledgements_and_no_notices() {
        let dir = std::env::temp_dir().join(format!("koyomi-reset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(acked::file_name(None)),
            "acked = [\"mine\"]
",
        )
        .unwrap();
        std::fs::write(
            dir.join(acked::file_name(Some("kei"))),
            "acked = [\"theirs\"]
",
        )
        .unwrap();
        let mut acked = acked::Acked::load_for(Some(dir.clone()), None);
        assert!(acked.is_acked("mine"));
        let mut notices = vec![ito_notices::Notice {
            id: 1,
            at: std::time::SystemTime::now(),
            kind: ito_notices::Kind::Warn,
            title: "old".into(),
            note: None,
            go: None,
            read: false,
        }];
        let mut bell = true;
        reset_for_link(&mut notices, &mut bell, &mut acked, Some(dir.clone()), &link::Link::Far("kei".into()));
        assert!(notices.is_empty() && !bell);
        assert!(acked.is_acked("theirs") && !acked.is_acked("mine"));
        reset_for_link(&mut notices, &mut bell, &mut acked, Some(dir.clone()), &link::Link::Local);
        assert!(acked.is_acked("mine") && !acked.is_acked("theirs"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
