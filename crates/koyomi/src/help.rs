//! The key help (F1 or `?`): what the keymap binds, the keys koyomi keeps
//! for itself, the keys of a box over the tree and of this list, laid out
//! by `ito_help` in a card over the tree. Esc, F1, `?` or a click outside
//! closes it; the arrows, the pages, Home, End and the wheel scroll it.

use std::cell::Cell;

use egui::{pos2, vec2, Align2, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui};
use ito_help::{Drawn, Group};

use crate::keymap::NOOP;
use crate::App;

const LINE: f32 = 20.0;
/// Two columns when the card is wide enough for them, one when it is not.
const COLUMN: f32 = 380.0;
const WIDTH: f32 = 900.0;

/// The help while it is open: where it is scrolled to, and what the last
/// draw found out (which the keys that scroll need).
#[derive(Default)]
pub struct Help {
    pub open: bool,
    scroll: usize,
    drawn: Cell<Option<Drawn>>,
}

impl Help {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.scroll = 0;
        self.drawn.set(None);
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Esc closes; the arrows, the pages, Home and End scroll. Enter, Space
    /// and Tab are taken too, so nothing behind the help is pressed through it.
    pub fn keys(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers as M};
        let mut delta: Option<i64> = None;
        let mut to: Option<usize> = None;
        let page = self.drawn.get().map_or(1, |d| d.visible.saturating_sub(1).max(1)) as i64;
        ctx.input_mut(|i| {
            if i.consume_key(M::NONE, Key::Escape) {
                self.open = false;
            }
            if i.consume_key(M::NONE, Key::ArrowDown) {
                delta = Some(1);
            }
            if i.consume_key(M::NONE, Key::ArrowUp) {
                delta = Some(-1);
            }
            if i.consume_key(M::NONE, Key::PageDown) {
                delta = Some(page);
            }
            if i.consume_key(M::NONE, Key::PageUp) {
                delta = Some(-page);
            }
            if i.consume_key(M::NONE, Key::Home) {
                to = Some(0);
            }
            if i.consume_key(M::NONE, Key::End) {
                to = Some(usize::MAX);
            }
            for k in [Key::Enter, Key::Space, Key::Tab] {
                i.consume_key(M::NONE, k);
                i.consume_key(M::SHIFT, k);
            }
        });
        if let Some(d) = delta {
            self.step(d);
        }
        if let Some(t) = to {
            self.scroll = t.min(self.drawn.get().map_or(0, |d| d.max));
        }
    }

    /// Scroll by `delta` lines, held to what the last draw allowed.
    fn step(&mut self, delta: i64) {
        let Some(d) = self.drawn.get() else { return };
        self.scroll = ito_help::scrolled(d.scroll, delta, d.max);
    }
}

/// The key list as titled groups: the keymap's layer (as it is now, so a
/// changed key shows), the keys koyomi reads itself, a box over the tree,
/// and this help.
pub fn groups(app: &App) -> Vec<Group> {
    let t = &app.table;
    let tr = |k: &str| t.tr(k).to_owned();
    let key = |k: &str| ito_keys::human_text(k).into_owned();
    let mut out = Vec::new();
    for (name, layer) in app.keys.layers() {
        let mut rows: Vec<(String, String)> = Vec::new();
        for b in ito_keymap::live(layer, NOOP) {
            let desc = if b.desc.is_empty() { b.raw.clone() } else { t.tr_desc(&b.desc).to_owned() };
            let keys = ito_keys::human_seq(&b.on);
            match rows.iter_mut().find(|(_, d)| *d == desc) {
                Some((k, _)) => {
                    k.push_str(" / ");
                    k.push_str(&keys);
                }
                None => rows.push((keys, desc)),
            }
        }
        // The one layer, "app", is named "keys.main" in the tables (as the
        // settings' Keys page names it); any other layer is named after itself.
        let title = if name == "app" { tr("keys.main") } else { tr(&format!("keys.{name}")) };
        out.push(Group { title, rows });
    }
    out.push(Group {
        title: tr("help.fixed"),
        rows: vec![
            (key("F12"), tr("palette.report_bug")),
            (key("Ctrl+Shift+P"), tr("title.search")),
            (format!("{} / {}", key("Ctrl++"), key("Ctrl+-")), tr("help.scale")),
            (key("Ctrl+0"), tr("palette.reset_scale")),
        ],
    });
    out.push(Group {
        title: tr("help.box"),
        rows: vec![
            ("Enter".to_owned(), tr("help.box_enter")),
            ("Tab".to_owned(), tr("help.box_tab")),
            ("Esc".to_owned(), tr("help.box_esc")),
        ],
    });
    out.push(Group {
        title: tr("help.title"),
        rows: vec![("↑ ↓ PgUp PgDn Home End".to_owned(), tr("help.scroll")), ("Esc".to_owned(), tr("help.close_row"))],
    });
    out
}

/// The card over `r` (the area the tree has): dimmed around, the groups in
/// columns that fit without scrolling when they can. A click outside the
/// card closes it; the wheel scrolls it by lines, as the arrow keys do.
pub fn show(ui: &mut Ui, app: &mut App, r: Rect) {
    let c = ito_theme::colors();
    let groups = groups(app);
    let w = (r.width() - 24.0).clamp(200.0, WIDTH);
    let room = (r.height() - 24.0 - 86.0).max(LINE);
    let cols = ito_help::columns_for(w - 40.0, COLUMN, 2);
    let plan = ito_help::plan(&groups, room, LINE, cols, 0.8);
    let tall = plan.columns.iter().map(Vec::len).max().unwrap_or(0) as f32 * LINE * plan.scale;
    let need = 50.0 + tall + 36.0;
    let card = Rect::from_center_size(r.center(), vec2(w, need.min(r.height() - 24.0).max(120.0)));
    let title = app.table.tr("help.title").to_owned();
    let close = app.table.tr("help.close").to_owned();
    let resp = egui::Area::new(egui::Id::new("koyomi-help"))
        .order(egui::Order::Foreground)
        .fixed_pos(r.min)
        .interactable(true)
        .show(ui.ctx(), |ui| {
            let resp = ui.allocate_rect(r, Sense::click());
            let p = ui.painter().clone();
            p.rect_filled(r, CornerRadius::ZERO, c.bg.gamma_multiply(0.6));
            p.rect_filled(card, CornerRadius::same(10), c.raised());
            p.rect_stroke(card, CornerRadius::same(10), Stroke::new(1.0, c.border_strong()), StrokeKind::Inside);
            p.text(pos2(card.left() + 20.0, card.top() + 18.0), Align2::LEFT_TOP, &title, FontId::proportional(16.0), c.fg);
            let body = Rect::from_min_max(pos2(card.left() + 20.0, card.top() + 50.0), pos2(card.right() - 20.0, card.bottom() - 36.0));
            let wheel = ui.input(|i| i.smooth_scroll_delta.y);
            if wheel != 0.0 {
                app.help.step((-wheel / LINE).round() as i64);
            }
            let style = ito_help::Style {
                font: FontId::monospace(12.0),
                row_h: LINE,
                head: c.run,
                keys: c.fg,
                desc: c.dim,
                warn: c.err,
                dim: c.dim,
                bar: c.border_strong(),
            };
            let drawn = ito_help::draw_plan(&p, body, &plan, app.help.scroll, &style);
            app.help.drawn.set(Some(drawn));
            p.text(pos2(card.left() + 20.0, card.bottom() - 22.0), Align2::LEFT_CENTER, &close, FontId::proportional(11.5), c.dim);
            resp
        })
        .inner;
    if resp.clicked() && resp.interact_pointer_pos().is_some_and(|pos| !card.contains(pos)) {
        app.help.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_scroll_only_once_something_was_drawn() {
        let mut h = Help::default();
        h.toggle();
        assert!(h.open);
        h.step(5);
        assert_eq!(h.scroll, 0, "nothing drawn, nothing to scroll");
        h.drawn.set(Some(Drawn { visible: 10, max: 30, scroll: 0, above: 0, below: 0 }));
        h.step(9);
        assert_eq!(h.scroll, 9);
        h.drawn.set(Some(Drawn { visible: 10, max: 30, scroll: 9, above: 0, below: 0 }));
        h.step(100);
        assert_eq!(h.scroll, 30);
        h.toggle();
        assert!(!h.open && h.scroll == 0);
    }
}
