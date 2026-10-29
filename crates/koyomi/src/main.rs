//! koyomi: Small job scheduler and monitor (a light JP1)
//!
//! What is wired here: the window (its place kept, its own title bar with the
//! bell and the notice list), common.toml's theme, language and scale,
//! `--version` / `--help`, and F12 (the bug report).
//! What the uchmk apps also have and is still to wire is in TODO.md, each
//! item with the ito crate and an app that already does it.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod cli;
mod client;
mod engine;
mod envreport;
mod i18n;
mod icon;
mod notify;
mod proto;
mod review;
mod show;
mod ui;

use ito_common::window::{Saved, Tracker};

/// The window's icon, in pixels square.
const ICON_PX: u32 = 128;

const REPO: &str = "https://github.com/uchmk/koyomi";

const ABOUT: &str = "koyomi — Small job scheduler and monitor (a light JP1)";

fn usage_text() -> String {
    format!("{ABOUT}\n\n{}\n  env   print what a bug report needs (version, files, last run); no window\n", cli::USAGE.trim_end())
}

struct App {
    table: ito_i18n::Table,
    common: ito_common::Common,
    tracker: Option<Tracker>,
    lang: String,
    /// The last scale change, as the line the status bar shows.
    scale_note: Option<String>,
    /// What the bell lists, oldest first: a run that ended 異常 (a click opens
    /// it, by its id) and the engine's notes about `[notify]`.
    notices: Vec<ito_notices::Notice<String>>,
    next_notice: u64,
    /// The list under the bell is open, and where the bell was last drawn.
    bell_open: bool,
    bell_at: egui::Rect,
    /// The command palette while it is open (Ctrl+Shift+P, or the bar's box).
    palette: Option<ito_palette::View>,
    browser: ui::Browser,
    /// The Japanese face, read off the UI thread; `None` once it is registered.
    font: Option<std::sync::mpsc::Receiver<Option<Vec<u8>>>>,
}

/// What the palette can do. Add a variant and an entry in `commands` for each
/// command the app gets.
#[derive(Clone, Copy)]
enum Command {
    ReportBug,
    ResetScale,
    Quit,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, saved: &Saved) -> Self {
        saved.put(cc);
        // What this run drew with, for the bug report and `koyomi env`.
        let mut used = ito_common::runinfo::RunInfo::new(env!("CARGO_PKG_VERSION"));
        if let Some(rs) = cc.wgpu_render_state.as_ref() {
            let info = rs.adapter.get_info();
            used.adapter = info.name;
            used.backend = format!("{:?}", info.backend);
            used.device = format!("{:?}", info.device_type);
        }
        used.ppp = cc.egui_ctx.pixels_per_point();
        ito_common::runinfo::save("koyomi", &used);
        let base = ito_common::base_dir();
        let (common, mut warnings) = match base.as_deref().map(ito_common::Common::read) {
            Some(Ok(read)) => read,
            Some(Err(e)) => (Default::default(), vec![e]),
            None => Default::default(),
        };
        let lang = i18n::resolve(common.language.as_deref());
        let (table, errors) = i18n::table(&lang);
        warnings.extend(errors);
        for w in warnings {
            eprintln!("koyomi: {w}");
        }
        let themes = ito_theme::builtin();
        let choice = common.theme.as_deref().unwrap_or("system");
        let dark = common.dark_theme.as_deref().unwrap_or("koyomi Dark");
        let light = common.light_theme.as_deref().unwrap_or("koyomi Light");
        if let Ok(theme) = ito_theme::pick(&themes, choice, dark, light, false) {
            ito_theme::set(theme.colors);
        }
        cc.egui_ctx.set_visuals(ito_theme::colors().visuals());
        // The scale is common.toml's, so egui's own Ctrl+= / Ctrl+- stay out of it.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        cc.egui_ctx.set_zoom_factor(common.scale_or_one());
        Self {
            table,
            common,
            tracker: saved.tracker(),
            lang,
            scale_note: None,
            notices: Vec::new(),
            next_notice: 0,
            bell_open: false,
            bell_at: egui::Rect::NOTHING,
            palette: None,
            browser: ui::Browser::start(&cc.egui_ctx, ito_common::state_dir("koyomi")),
            font: Some(font_reader(&cc.egui_ctx)),
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
        ito_fonts::register(&mut fonts, self.common.mono_font.unwrap_or_default());
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
        let next = ito_common::scale_step(self.common.scale_or_one(), to);
        if next == self.common.scale_or_one() {
            return;
        }
        self.common.scale = Some(next);
        ctx.set_zoom_factor(next);
        ito_statusbar::scale_changed(ctx, next);
        self.scale_note = Some(ito_common::scale_text(next, &self.lang));
        if let Some(base) = ito_common::base_dir() {
            std::thread::spawn(move || {
                let change = ito_common::scale_change(next);
                if let Err(e) = ito_common::edit_common(&base, |text| change.apply(text)) {
                    eprintln!("koyomi: {e}");
                }
            });
        }
    }

    /// The bottom bar: the app's items on the left (add yours here), the clock
    /// from common.toml at the right end, the scale's words after a change.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let clock = self.common.clock_or(None);
        let spec = ito_statusbar::Spec { clock: &clock, lang: &self.lang, opacity: 1.0 };
        let version = env!("CARGO_PKG_VERSION");
        ito_statusbar::show(ui, &spec, |ui| ito_statusbar::item(ui, version, ito_theme::colors().dim), |_| {});
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
        let name = self.table.tr("app.title").to_string();
        let out = ito_titlebar::show(
            ui,
            &spec,
            |ui, rect| {
                let galley = ui.painter().layout_no_wrap(name, egui::FontId::proportional(13.0), ito_theme::colors().fg);
                let x = rect.left() + 12.0;
                let right = x + galley.size().x;
                ui.painter().galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, egui::Color32::WHITE);
                right
            },
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
        vec![
            ito_palette::Entry::new(t.tr("palette.report_bug"), "F12", Command::ReportBug),
            ito_palette::Entry::new(t.tr("palette.reset_scale"), "Ctrl+0", Command::ResetScale),
            ito_palette::Entry::new(t.tr("palette.quit"), "", Command::Quit),
        ]
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
            Some(ito_palette::Answer::Pick(Command::ReportBug)) => self.report_bug(ctx),
            Some(ito_palette::Answer::Pick(Command::ResetScale)) => self.scale_to(ctx, ito_common::ScaleTo::Reset),
            Some(ito_palette::Answer::Pick(Command::Quit)) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Some(_) => {}
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
                    self.browser.open_run(&run);
                }
            }
            Some(ito_notices::Action::ReadAll) => self.notices.iter_mut().for_each(|n| n.read = true),
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
        for n in self.browser.take_notes() {
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
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F12)) {
            self.report_bug(&ctx);
        }
        self.rescale(&ctx);
        if let Some(t) = &mut self.tracker {
            t.follow(&ctx);
        }
        egui::Panel::top("titlebar")
            .exact_size(ito_titlebar::HEIGHT)
            .frame(egui::Frame::NONE.fill(ito_theme::colors().bg))
            .show(ui, |ui| self.title_bar(ui));
        egui::Panel::bottom("statusbar").exact_size(ito_statusbar::HEIGHT).frame(egui::Frame::NONE).show(ui, |ui| self.status_bar(ui));
        self.take_notices();
        self.notice_list(&ctx);
        self.command_palette(&ctx);
        self.take_font(&ctx);
        self.browser.show(ui, &self.table);
        // Last, so the edges of the frameless window win over what is under them.
        if !cfg!(target_os = "macos") {
            ito_titlebar::resize_edges(&ctx);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        ito_theme::colors().bg.to_normalized_gamma_f32()
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
    let line = format!("{text}\n");
    match err {
        true => ito_console::err(&line),
        false => ito_console::out(&line),
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = cli::run(&args) {
        std::process::exit(code);
    }
    if let Some(a) = args.first() {
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
    let saved = Saved::load(ito_common::state_dir("koyomi"));
    let viewport = egui::ViewportBuilder::default()
        .with_title("koyomi")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([720.0, 420.0])
        .with_icon(std::sync::Arc::new(egui::IconData { rgba: icon::pixels(ICON_PX), width: ICON_PX, height: ICON_PX }));
    // The bar is the app's own (the Mac keeps its traffic lights over it).
    let viewport = ito_titlebar::frameless(viewport);
    let options = eframe::NativeOptions { viewport: saved.viewport(viewport), ..Default::default() };
    eframe::run_native("koyomi", options, Box::new(move |cc| Ok(Box::new(App::new(cc, &saved)))))
}
