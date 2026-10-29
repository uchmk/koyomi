//! The settings screen (Ctrl+,): `ito-prefs` draws the nav and the cards, and
//! the rows common to every uchmk app (the language, the size, the clock,
//! the window, the theme) write common.toml. koyomi's own page is the Keys
//! page (each operation's keys, written to keymap.toml). The definitions
//! and koyomi.toml are not settings: they stay files, as DECISIONS.md has it.

use egui::{vec2, Align2, FontId, Sense, Stroke, StrokeKind, Ui};
use ito_common::{themes_dir, CommonChange};
use ito_prefs::{section, MoreWords, Words};

use crate::App;

pub const GENERAL: usize = 0;
pub const THEME: usize = 1;
pub const KEYS: usize = 2;
pub const ADVANCED: usize = 3;
pub const ABOUT: usize = 4;

/// The keymap's layers and the words that name them on the Keys page.
const LAYERS: [(&str, &str); 1] = [("app", "keys.main")];

/// What common.toml means when it names no theme: follow the OS, with
/// koyomi's own dark and light ones.
pub const THEMES: (&str, &str, &str) = ("system", "koyomi Dark", "koyomi Light");

/// The rows on each page, for the search.
fn index(w: &'static Words, more: &'static MoreWords, lang: &str, t: &ito_i18n::Table) -> Vec<(usize, String)> {
    let keys = ito_prefs::keys_index(ito_prefs::KeyWords::of(lang)).into_iter().map(|t| (KEYS, t));
    let layers = LAYERS.iter().map(|(_, k)| (KEYS, t.tr(k)));
    vec![
        (GENERAL, w.language),
        (GENERAL, w.scale),
        (GENERAL, w.clock),
        (GENERAL, w.show_time),
        (GENERAL, w.time_format),
        (GENERAL, w.show_date),
        (GENERAL, w.date_format),
        (GENERAL, w.weekday),
        (GENERAL, "language"),
        (GENERAL, "scale"),
        (GENERAL, "[clock]"),
        (GENERAL, "[window]"),
        (THEME, w.mode),
        (THEME, w.themes),
        (THEME, w.preview),
        (THEME, "theme"),
        (THEME, "dark_theme"),
        (THEME, "light_theme"),
        (KEYS, "keymap.toml"),
        (KEYS, "prepend_keymap"),
    ]
    .into_iter()
    .chain(keys)
    .chain(layers)
    .chain(ito_prefs::window_index(more).into_iter().chain(ito_prefs::startup_index(more)).map(|t| (GENERAL, t)))
    .chain(ito_prefs::advanced_index(more).into_iter().map(|t| (ADVANCED, t)))
    .chain(ito_prefs::mono_font_index(more).into_iter().map(|t| (THEME, t)))
    .chain(ito_prefs::about_index(more, w).into_iter().map(|t| (ABOUT, t)))
    .map(|(p, t)| (p, t.to_owned()))
    .collect()
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let w = Words::of(&app.lang);
    let more = MoreWords::of(&app.lang);
    poll_startup(ui, app);
    // While a key is being set the page takes every key, Esc included.
    let keys = if app.prefs.keys_edit.busy() { ito_prefs::Keys::default() } else { ito_prefs::Keys::standard(ui, false) };
    let table = &app.table;
    let tr = |k: &str| table.tr(k).to_owned();
    let (adv, about) = (ito_prefs::Page::Advanced.words(more), ito_prefs::Page::About.words(more));
    let page_words = [
        (tr("settings.general"), tr("settings.general_lead")),
        (tr("settings.theme"), tr("settings.theme_lead")),
        (tr("settings.keys"), tr("settings.keys_lead")),
        (adv.0.to_owned(), adv.1.to_owned()),
        (about.0.to_owned(), about.1.to_owned()),
    ];
    let pages: Vec<(&str, &str)> = page_words.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let index = index(w, more, &app.lang, table);
    let index: Vec<(usize, &str)> = index.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let nav = ito_prefs::Nav { pages: &pages, index: &index, words: w, file: "common.toml" };
    let mut asked: Vec<CommonChange> = Vec::new();
    let layers = app.keys.layers().to_vec();
    let kw = ito_prefs::KeyWords::of(&app.lang);
    let own =
        app.prefs.base.as_deref().map_or_else(|| crate::keymap::FILE.to_owned(), |b| crate::prefs::keymap_path(b).display().to_string());
    let mut key_edits = Vec::new();
    let prefs = &mut app.prefs;
    let keys_edit = &mut prefs.keys_edit;
    let mut bug = false;
    let mut forget = false;
    let mut startup_toggled = None;
    let mut advanced = Vec::new();
    let startup_now = app.startup;
    let state_dir = ito_common::state_dir("koyomi");
    let (common, themes, base) = (&prefs.common, &prefs.themes, prefs.base.as_deref());
    let out = ito_prefs::show(ui, ito_theme::colors(), &mut prefs.state, &nav, keys, |ui, page, l| match page {
        GENERAL => {
            section(ui, l, &tr("settings.display"), |ui| {
                let now = common.language.as_deref().unwrap_or("auto");
                asked.extend(ito_prefs::language_row(ui, l, w, now, ""));
                ito_prefs::sep(ui, l);
                asked.extend(ito_prefs::scale_row(ui, l, w, common.scale_or_one(), ""));
            });
            asked.extend(ito_prefs::clock_card(ui, l, w, &common.clock_or(None)));
            let out = ito_prefs::window_card(ui, l, more, &common.window_or_default());
            asked.extend(out.changes);
            forget |= out.forget;
            startup_toggled = ito_prefs::startup_card(ui, l, more, startup_now);
        }
        THEME => {
            let (t, d, li) = common.theme_choice(Some(THEMES));
            let note = match base {
                Some(b) => format!("{} {}", tr("settings.theme_note"), themes_dir(b).display()),
                None => tr("settings.no_folder"),
            };
            let runs = [tr("state.ok"), tr("state.error"), tr("state.running")];
            asked.extend(ito_prefs::theme_page(ui, l, w, themes, (&t, &d, &li), &note, |ui| preview(ui, &tr("settings.sample"), &runs)));
            // Read at the next start: the fonts are loaded once, in the background.
            asked.extend(ito_prefs::mono_font_card(ui, l, more, common.mono_font_or_default()));
        }
        KEYS => {
            let layer_name = |n: &str| LAYERS.iter().find(|(id, _)| *id == n).map_or_else(|| n.to_owned(), |(_, k)| tr(k));
            let desc = |d: &str| table.tr_desc(d).to_owned();
            // Keys koyomi reads before the keymap: bound, they never run.
            let extra = |_: &str, on: &[ito_keys::Key]| -> Vec<String> {
                crate::keymap::Keys::fixed(on).map(|k| tr("keys.fixed").replace("{}", &k.human())).into_iter().collect()
            };
            let table = ito_prefs::KeyTable {
                now: &layers,
                built_in: crate::keymap::Keys::built_in_layers(),
                noop: crate::keymap::NOOP,
                own: &own,
                layer_name: &layer_name,
                tr: &desc,
                extra: &extra,
                unbound: &[],
                sup: false,
            };
            key_edits.extend(ito_prefs::keys_page(ui, l, kw, &table, keys_edit).unwrap_or_default());
        }
        ADVANCED => advanced.extend(ito_prefs::advanced_card(ui, l, more, state_dir.is_some())),
        ABOUT => {
            let system = ito_bugreport::os_line();
            let license = Some(env!("CARGO_PKG_LICENSE")).filter(|x| !x.is_empty());
            let about =
                ito_prefs::About { version: env!("CARGO_PKG_VERSION"), commit: option_env!("KOYOMI_COMMIT"), system: &system, license };
            bug |= !ito_prefs::about_page(ui, l, more, w, "koyomi", &about).is_empty();
        }
        _ => {}
    });
    let no_folder = app.table.tr("settings.no_folder").to_owned();
    if bug {
        app.report_bug(ui.ctx());
    }
    if forget {
        if let Some(dir) = &state_dir {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let _ = ito_prefs::forget_place(&dir);
            });
        }
    }
    if let Some(on) = startup_toggled {
        set_startup(ui.ctx(), app, on);
    }
    for a in advanced {
        match a {
            ito_prefs::Advanced::OpenConfigDir => match app.prefs.base.clone() {
                Some(b) => app.prefs.open_dir(b),
                None => app.scale_note = Some(no_folder.clone()),
            },
            ito_prefs::Advanced::OpenStateDir => {
                if let Some(d) = state_dir.clone() {
                    app.prefs.open_dir(d);
                }
            }
            // common.toml and keymap.toml are moved aside on a thread; the
            // definitions and koyomi.toml are the work, and stay.
            ito_prefs::Advanced::Reset => app.prefs.reset(&no_folder),
        }
    }
    if out.close {
        app.settings_open = false;
    }
    if out.open_file {
        app.prefs.open_common(&no_folder);
    }
    // Gone to another page: the keys stop being taken for this one.
    if app.prefs.state.page != KEYS {
        app.prefs.keys_edit.capture = None;
        app.prefs.keys_edit.listening = false;
    }
    app.prefs.keymap_edit(key_edits, &no_folder);
    app.prefs.common_change(asked, &no_folder);
}

/// Take the OS's answer about starting at sign-in, and ask when nothing is
/// known yet (once the General page is up). The registry and the autostart
/// folders are read on a thread.
fn poll_startup(ui: &Ui, app: &mut App) {
    if let Some(v) = app.startup_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
        app.startup = Some(v);
        app.startup_rx = None;
    }
    if app.startup.is_none() && app.startup_rx.is_none() && app.prefs.state.page == GENERAL {
        app.startup_rx = Some(startup_task(ui.ctx(), None));
    }
}

/// The switch was clicked: set it on a thread, then read what the OS has.
fn set_startup(ctx: &egui::Context, app: &mut App, on: bool) {
    app.startup = None;
    app.startup_rx = Some(startup_task(ctx, Some(on)));
}

/// Read (and first set, when `set` is given) the sign-in start on a thread.
fn startup_task(ctx: &egui::Context, set: Option<bool>) -> std::sync::mpsc::Receiver<bool> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        if let (Some(on), Ok(exe)) = (set, std::env::current_exe()) {
            let _ = ito_common::startup::set("koyomi", &exe, &[], on);
        }
        let _ = tx.send(ito_common::startup::enabled("koyomi"));
        ctx.request_repaint();
    });
    rx
}

/// The Theme page's sample: a card with a run of each colour (ended ok,
/// ended abnormally, running) as the tree draws them.
fn preview(ui: &mut Ui, sample: &str, runs: &[String]) {
    let c = ito_theme::colors();
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().min(420.0), 96.0), Sense::hover());
    let p = ui.painter_at(r);
    p.rect_filled(r, 8.0, c.bg);
    p.rect_stroke(r, 8.0, Stroke::new(1.0, c.border), StrokeKind::Inside);
    let inner = r.shrink(12.0);
    let mono = FontId::monospace(12.0);
    p.text(inner.left_top(), Align2::LEFT_TOP, sample, FontId::proportional(12.0), c.dim);
    let marks = [("●", c.done), ("✗", c.err), ("▶", c.run)];
    let mut x = inner.left();
    for ((m, color), name) in marks.into_iter().zip(runs) {
        let name = name.as_str();
        let g = p.layout_no_wrap(format!("{m} {name}"), mono.clone(), color);
        if x + g.size().x > inner.right() {
            break;
        }
        let w = g.size().x;
        p.galley(egui::pos2(x, inner.top() + 30.0), g, color);
        x += w + 14.0;
    }
    let g = p.layout_no_wrap("09:00 nightly/backup".to_owned(), mono, c.fg);
    p.galley(egui::pos2(inner.left(), inner.top() + 54.0), g, c.fg);
    p.circle_filled(egui::pos2(inner.right() - 8.0, inner.top() + 8.0), 4.0, c.done);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ito_prefs::Page;

    #[test]
    fn pages_follow_ito_default_order() {
        let ours = [GENERAL, THEME, KEYS, ADVANCED, ABOUT];
        assert!(ours.windows(2).all(|w| w[0] < w[1]));
        let has = [Page::Theme, Page::Keys, Page::Advanced];
        assert_eq!(ito_prefs::default_order(&has), [Page::General, Page::Theme, Page::Keys, Page::Advanced, Page::About]);
    }

    /// The Keys page's words, and koyomi's own, are found by the search.
    #[test]
    fn the_search_finds_the_keys_page() {
        let (t, errors) = crate::i18n::table("en");
        assert!(errors.is_empty(), "{errors:?}");
        let idx = index(Words::of("en"), MoreWords::of("en"), "en", &t);
        assert!(idx.iter().any(|(p, t)| *p == KEYS && t == "keymap.toml"));
        assert!(idx.iter().any(|(p, t)| *p == KEYS && t == "Everywhere"));
        assert!(idx.iter().any(|(p, t)| *p == GENERAL && t == "[window]"));
    }
}
