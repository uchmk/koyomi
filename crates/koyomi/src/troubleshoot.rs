//! こんな時はどうする (Troubleshooting): a card over the page that lists every message of the catalogue
//! (`koyomi_core::msg`) grouped by area, with a search field on top. Opened from the command palette.
//!
//! Esc closes it and a click outside closes it. While it is open the page under it reads no key (the window takes the
//! key events away while the page draws), so Enter, Backspace and the rest belong to the search field.

use egui::{pos2, vec2, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Ui};
use ito_a11y::Role;
use koyomi_core::msg::{self, Entry};

use crate::App;

const WIDTH: f32 = 760.0;

/// The card while it is open.
#[derive(Default)]
pub struct Troubleshoot {
    pub open: bool,
    query: String,
    /// The search field takes the focus once, when the card opens.
    fresh: bool,
}

impl Troubleshoot {
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.fresh = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Esc closes. Nothing else is taken, so the field can use every other key.
    pub fn keys(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.open = false;
        }
    }
}

/// Whether `entry` answers `query`: case-insensitive, a leading `KOYOMI-` ignored; the number, `when`, `text`, `cause`
/// and `action` are searched. An empty query matches all.
pub fn matches(entry: &Entry, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    let q = q.strip_prefix("koyomi-").unwrap_or(&q).trim();
    if q.is_empty() {
        return true;
    }
    [entry.id.number(), &entry.when, &entry.text, &entry.cause, &entry.action].iter().any(|f| f.to_lowercase().contains(q))
}

/// The card over `r` (the area the page has).
pub fn show(ui: &mut Ui, app: &mut App, r: Rect) {
    let c = ito_theme::colors();
    let t = &app.table;
    let w = (r.width() - 24.0).clamp(240.0, WIDTH);
    let card = Rect::from_center_size(r.center(), vec2(w, (r.height() - 24.0).max(160.0)));
    let title = t.tr("troubleshoot.title").to_owned();
    let search = t.tr("troubleshoot.search").to_owned();
    let none = t.tr("troubleshoot.none").to_owned();
    let footer = t.tr("troubleshoot.footer").to_owned();
    let (error, warning) = (t.tr("troubleshoot.error").to_owned(), t.tr("troubleshoot.warning").to_owned());
    let labels = [t.tr("troubleshoot.message").to_owned(), t.tr("troubleshoot.cause").to_owned(), t.tr("troubleshoot.action").to_owned()];
    let area_head = |name: &str, area: &str| t.trf("troubleshoot.area", &[name, area]);
    let mut list = msg::entries(&app.lang);
    list.sort_by(|a, b| (a.id.area(), a.id.number()).cmp(&(b.id.area(), b.id.number())));
    let tb = &mut app.troubleshoot;
    let resp = egui::Area::new(egui::Id::new("koyomi-troubleshoot"))
        .order(egui::Order::Foreground)
        .fixed_pos(r.min)
        .interactable(true)
        .show(ui.ctx(), |ui| {
            let resp = ui.allocate_rect(r, Sense::click());
            let p = ui.painter().clone();
            p.rect_filled(r, CornerRadius::ZERO, c.bg.gamma_multiply(0.6));
            p.rect_filled(card, CornerRadius::same(10), c.raised());
            p.rect_stroke(card, CornerRadius::same(10), Stroke::new(1.0, c.border_strong()), StrokeKind::Inside);
            let inner = card.shrink2(vec2(20.0, 16.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                ui.set_width(inner.width());
                let head = ui.label(RichText::new(&title).size(16.0).color(c.fg));
                ito_a11y::say(&head, Role::Heading, &title);
                ui.add_space(8.0);
                let field = ui.add(egui::TextEdit::singleline(&mut tb.query).hint_text(&search).desired_width(f32::INFINITY));
                ito_a11y::say(&field, Role::Label, &search);
                if tb.fresh {
                    field.request_focus();
                    tb.fresh = false;
                }
                ui.add_space(8.0);
                let shown: Vec<&Entry> = list.iter().filter(|e| matches(e, &tb.query)).collect();
                let height = (inner.bottom() - ui.cursor().top() - 24.0).max(40.0);
                egui::ScrollArea::vertical().id_salt("koyomi-troubleshoot-rows").max_height(height).auto_shrink([false, false]).show(
                    ui,
                    |ui| {
                        ui.set_width(ui.available_width());
                        if shown.is_empty() {
                            ui.label(RichText::new(&none).color(c.dim));
                        }
                        let mut area = "";
                        for e in shown {
                            if e.id.area() != area {
                                area = e.id.area();
                                ui.add_space(6.0);
                                let h = area_head(&e.area_name, area);
                                let r = ui.label(RichText::new(&h).strong().size(14.0).color(c.run));
                                ito_a11y::say(&r, Role::Heading, &h);
                            }
                            ui.add_space(6.0);
                            let kind = if e.id.is_warning() { &warning } else { &error };
                            let h = format!("{} {} {}", e.id.code(), kind, e.when);
                            let r = ui.add(egui::Label::new(RichText::new(&h).strong().color(c.fg)).wrap());
                            ito_a11y::say(&r, Role::Heading, &h);
                            for (label, value) in labels.iter().zip([&e.text, &e.cause, &e.action]) {
                                let line = format!("{label}: {value}");
                                let r = ui.add(egui::Label::new(RichText::new(&line).color(c.dim)).wrap());
                                ito_a11y::say(&r, Role::Label, &line);
                            }
                        }
                    },
                );
                ui.painter().text(
                    pos2(inner.left(), inner.bottom()),
                    egui::Align2::LEFT_BOTTOM,
                    &footer,
                    egui::FontId::proportional(11.5),
                    c.dim,
                );
            });
            resp
        })
        .inner;
    if resp.clicked() && resp.interact_pointer_pos().is_some_and(|pos| !card.contains(pos)) {
        tb.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Entry {
        msg::entries("ja").into_iter().find(|e| e.id == msg::CONNECT_FAILED).expect("E2101")
    }

    #[test]
    fn an_empty_query_matches_everything() {
        assert!(matches(&entry(), ""));
        assert!(matches(&entry(), "   "));
        assert!(matches(&entry(), "KOYOMI-"));
    }

    #[test]
    fn the_number_is_matched_with_or_without_the_prefix_in_any_case() {
        let e = entry();
        for q in ["E2101", "e2101", "KOYOMI-E2101", "koyomi-e2101", "2101", "  koyomi-E2101 "] {
            assert!(matches(&e, q), "{q}");
        }
        assert!(!matches(&e, "E2102"));
    }

    #[test]
    fn when_text_cause_and_action_are_searched() {
        let e = entry();
        for q in ["エンジンに接続できない", "に接続できません", "ファイアウォールで", "lan = true"] {
            assert!(matches(&e, q), "{q}");
        }
        assert!(!matches(&e, "カレンダー"));
        let en = msg::entries("en").into_iter().find(|e| e.id == msg::CONNECT_FAILED).expect("E2101");
        assert!(matches(&en, "CANNOT CONNECT"));
        assert!(matches(&en, "firewall"));
    }
}
