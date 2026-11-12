//! The list of links in the status bar: this PC and every `[[remote]]`, each with a dot for its state and a
//! button that switches the window to it. The window still shows one engine at a time.
//!
//! What each row says comes from the window-wide watch (`watch.rs`), which asks every `[[remote]]` in the
//! background whether or not this list is open; opening the list only wakes it.

use std::collections::{BTreeSet, HashMap};

use ito_a11y::Role;
use ito_theme::scale::Text;

use crate::link::Link;
use crate::watch::{Fact, Seen};

/// The list's width and the tallest its scrolling part gets.
const WIDTH: f32 = 320.0;
const TALL: f32 = 420.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dot {
    Good,
    Bad,
    /// Not known (yet).
    Idle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Connect,
    Disconnect,
}

/// What one row says and does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowFacts {
    pub dot: Dot,
    /// The key of the second line.
    pub text: &'static str,
    /// What `{0}` of the text is (the failures nobody has dealt with); 0 for a text without it.
    pub count: usize,
    /// The button and the link it switches to; `None` for no button.
    pub action: Option<(Button, Link)>,
}

/// The row of `name` (`None` is this PC) while the window is on `link`, `down` when that engine does not answer,
/// and `fact` is what the watch found out. `watched` is false for a `[[remote]]` the window does not watch (it says
/// so instead of what the watch found; the one the window is on keeps its row).
pub fn row(link: &Link, down: bool, name: Option<&str>, fact: Option<&Fact>, watched: bool) -> RowFacts {
    if link.far() == name {
        let (dot, text) = match (name.is_some(), down) {
            (false, false) => (Dot::Good, "link.connected"),
            (false, true) => (Dot::Bad, "link.no_answer"),
            (true, false) => (Dot::Good, "link.remote_connected"),
            (true, true) => (Dot::Bad, "link.remote_no_answer"),
        };
        return RowFacts { dot, text, count: 0, action: name.map(|_| (Button::Disconnect, Link::Local)) };
    }
    let mut count = 0;
    let (dot, text) = match (name, fact.map(|f| (&f.seen, f.unacked))) {
        (None, _) => (Dot::Idle, "link.pc_engine"),
        (Some(_), _) if !watched => (Dot::Idle, "link.remote_unwatched"),
        (Some(_), None) => (Dot::Idle, "link.remote_checking"),
        (Some(_), Some((Seen::NoAnswer(_), _))) => (Dot::Bad, "link.remote_no_answer"),
        (Some(_), Some((Seen::Refused(_), _))) => (Dot::Bad, "link.remote_refused"),
        (Some(_), Some((Seen::Answering, Some(n)))) if n > 0 => {
            count = n;
            (Dot::Bad, "link.remote_unacked")
        }
        (Some(_), Some((Seen::Answering, _))) => (Dot::Good, "link.remote_ok"),
    };
    let to = name.map_or(Link::Local, |n| Link::Far(n.to_owned()));
    RowFacts { dot, text, count, action: Some((Button::Connect, to)) }
}

/// What the list brought out in one frame.
#[derive(Default)]
pub struct Outcome {
    /// The link a button picked.
    pub pick: Option<Link>,
    /// The manage-the-connections row was pressed.
    pub manage: bool,
    /// The list was opened in this frame (the watch asks at once).
    pub opened: bool,
}

/// The list's state: whether it is open.
pub struct Hosts {
    open: bool,
}

impl Hosts {
    pub fn new() -> Hosts {
        Hosts { open: false }
    }

    /// The status-bar button and, when it is open, the list above it. `facts` is what the watch found out, `unwatched` the
    /// `[[remote]]` names it does not watch.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        t: &ito_i18n::Table,
        link: &Link,
        down: bool,
        names: &[String],
        facts: &HashMap<String, Fact>,
        unwatched: &BTreeSet<String>,
    ) -> Outcome {
        let c = ito_theme::colors();
        let this_pc = t.tr("link.this_pc");
        let current = link.far().unwrap_or(this_pc);
        let own = row(link, down, link.far(), None, true);
        let button = ui
            .horizontal(|ui| {
                ito_statusbar::dot(ui, colour(&c, own.dot));
                ui.add(
                    egui::Label::new(egui::RichText::new(format!("{current} \u{25b4}")).size(Text::ChromeButton.size()).color(c.fg))
                        .sense(egui::Sense::click()),
                )
            })
            .inner
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        ito_a11y::say(&button, Role::Button, &format!("{} {current}", t.tr("link.pick")));
        ito_a11y::test_id(&button, "link.picker");
        let mut out = Outcome::default();
        if button.clicked() {
            self.open = !self.open;
            out.opened = self.open;
        }
        let mut open = self.open;
        let mut close = false;
        egui::Popup::from_response(&button)
            .open_bool(&mut open)
            .align(egui::RectAlign::TOP_START)
            .align_alternatives(&[])
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .gap(4.0)
            .show(|ui| {
                ui.set_width(WIDTH);
                ui.label(egui::RichText::new(t.tr("link.heading")).size(Text::ChromeSmall.size()).color(c.dim));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(TALL).auto_shrink([false, true]).show(ui, |ui| {
                    for name in std::iter::once(None).chain(names.iter().map(|n| Some(n.as_str()))) {
                        let fact = name.and_then(|n| facts.get(n));
                        let rf = row(link, down, name, fact, !name.is_some_and(|n| unwatched.contains(n.trim())));
                        if let Some(to) = draw_row(ui, t, name, &rf, fact.map(|f| &f.seen)) {
                            out.pick = Some(to);
                            close = true;
                        }
                    }
                });
                ui.separator();
                let manage = t.tr("link.manage");
                let r = ui.selectable_label(false, manage);
                ito_a11y::say(&r, Role::Button, manage);
                ito_a11y::test_id(&r, "link.manage");
                if r.clicked() {
                    out.manage = true;
                    close = true;
                }
            });
        self.open = open && !close;
        out
    }
}

fn colour(c: &ito_theme::Colors, dot: Dot) -> egui::Color32 {
    match dot {
        Dot::Good => c.done,
        Dot::Bad => c.err,
        Dot::Idle => c.dim,
    }
}

/// One row of the list; the link its button picks.
fn draw_row(ui: &mut egui::Ui, t: &ito_i18n::Table, name: Option<&str>, facts: &RowFacts, seen: Option<&Seen>) -> Option<Link> {
    let c = ito_theme::colors();
    let label = name.unwrap_or_else(|| t.tr("link.this_pc"));
    let scope = ui.scope(|ui| {
        let mut picked = None;
        egui::Sides::new().height(36.0).show(
            ui,
            |ui| {
                ito_statusbar::dot(ui, colour(&c, facts.dot));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(egui::RichText::new(label).size(Text::ChromeButton.size()).color(c.fg));
                    let text = if facts.count > 0 { t.trf(facts.text, &[&facts.count.to_string()]) } else { t.tr(facts.text).to_owned() };
                    ui.label(egui::RichText::new(text).size(Text::ChromeSmall.size()).color(c.dim));
                });
            },
            |ui| {
                if let Some((button, to)) = &facts.action {
                    let word = t.tr(match button {
                        Button::Connect => "link.connect",
                        Button::Disconnect => "link.disconnect",
                    });
                    let r = ui.button(word);
                    ito_a11y::say(&r, Role::Button, &format!("{word} {label}"));
                    if r.clicked() {
                        picked = Some(to.clone());
                    }
                }
            },
        );
        picked
    });
    ito_a11y::test_id(&scope.response, &format!("link.row.{}", name.unwrap_or("here")));
    if let Some(Seen::NoAnswer(why) | Seen::Refused(why)) = seen {
        scope.response.on_hover_text(why);
    }
    scope.inner
}

#[cfg(test)]
mod tests {
    use super::*;

    fn far(n: &str) -> Link {
        Link::Far(n.to_owned())
    }

    fn fact(seen: Seen, unacked: Option<usize>) -> Fact {
        Fact { seen, unacked }
    }

    fn rf(dot: Dot, text: &'static str, count: usize, action: Option<(Button, Link)>) -> RowFacts {
        RowFacts { dot, text, count, action }
    }

    #[test]
    fn this_pc_current_is_connected_and_has_no_button() {
        assert_eq!(row(&Link::Local, false, None, None, true), rf(Dot::Good, "link.connected", 0, None));
    }

    #[test]
    fn this_pc_current_and_down_is_red_with_no_button() {
        assert_eq!(row(&Link::Local, true, None, None, true), rf(Dot::Bad, "link.no_answer", 0, None));
    }

    #[test]
    fn this_pc_not_current_connects_to_local() {
        let r = row(&far("kei"), false, None, None, true);
        assert_eq!(r, rf(Dot::Idle, "link.pc_engine", 0, Some((Button::Connect, Link::Local))));
        // What this PC's engine is doing is not known from a remote: the same with a failed link.
        assert_eq!(row(&far("kei"), true, None, None, true), r);
    }

    #[test]
    fn current_remote_disconnects_to_local() {
        let r = row(&far("kei"), false, Some("kei"), None, true);
        assert_eq!(r, rf(Dot::Good, "link.remote_connected", 0, Some((Button::Disconnect, Link::Local))));
    }

    #[test]
    fn current_remote_down_is_red_and_still_disconnects() {
        let f = fact(Seen::Answering, Some(4));
        let r = row(&far("kei"), true, Some("kei"), Some(&f), true);
        assert_eq!(r, rf(Dot::Bad, "link.remote_no_answer", 0, Some((Button::Disconnect, Link::Local))));
    }

    #[test]
    fn other_remote_not_checked_yet_is_grey_and_connects() {
        for link in [Link::Local, far("kei")] {
            let r = row(&link, false, Some("works"), None, true);
            assert_eq!(r, rf(Dot::Idle, "link.remote_checking", 0, Some((Button::Connect, far("works")))));
        }
    }

    #[test]
    fn other_remote_by_what_the_watch_found() {
        let go = Some((Button::Connect, far("works")));
        let at = |f: Fact| row(&Link::Local, false, Some("works"), Some(&f), true);
        assert_eq!(at(fact(Seen::Answering, None)), rf(Dot::Good, "link.remote_ok", 0, go.clone()));
        assert_eq!(at(fact(Seen::Answering, Some(0))), rf(Dot::Good, "link.remote_ok", 0, go.clone()));
        assert_eq!(at(fact(Seen::Answering, Some(3))), rf(Dot::Bad, "link.remote_unacked", 3, go.clone()));
        assert_eq!(at(fact(Seen::NoAnswer("timed out".into()), None)), rf(Dot::Bad, "link.remote_no_answer", 0, go.clone()));
        assert_eq!(at(fact(Seen::Refused("token".into()), None)), rf(Dot::Bad, "link.remote_refused", 0, go));
    }

    #[test]
    fn an_unwatched_remote_says_so_and_the_current_one_is_unchanged() {
        let connect = Some((Button::Connect, far("works")));
        let f = fact(Seen::Answering, Some(3));
        for fact in [None, Some(&f)] {
            assert_eq!(row(&Link::Local, false, Some("works"), fact, false), rf(Dot::Idle, "link.remote_unwatched", 0, connect.clone()));
            assert_eq!(row(&far("kei"), false, Some("works"), fact, false), rf(Dot::Idle, "link.remote_unwatched", 0, connect.clone()));
        }
        // This PC's row is never "not watched"; the current link's row is as it was.
        assert_eq!(row(&far("kei"), false, None, None, false), rf(Dot::Idle, "link.pc_engine", 0, Some((Button::Connect, Link::Local))));
        assert_eq!(
            row(&far("kei"), false, Some("kei"), None, false),
            rf(Dot::Good, "link.remote_connected", 0, Some((Button::Disconnect, Link::Local)))
        );
    }

    #[test]
    fn the_unacked_text_carries_the_count() {
        let (t, _) = crate::i18n::table("en");
        assert!(t.trf("link.remote_unacked", &["3"]).ends_with("failures waiting 3"));
    }

    #[test]
    fn the_manage_row_is_in_the_list_and_the_add_row_is_gone() {
        let (t, _) = crate::i18n::table("ja");
        assert_eq!(t.tr("link.manage"), "接続先を管理…");
        let ctx = egui::Context::default();
        let mut h = Hosts::new();
        h.open = true;
        let list = vec!["kei".to_owned()];
        let mut facts = HashMap::new();
        facts.insert("kei".to_owned(), fact(Seen::Answering, Some(2)));
        let mut out = None;
        for _ in 0..3 {
            let mut full = ctx.run_ui(egui::RawInput::default(), |ui| {
                out = Some(h.show(ui, &t, &Link::Local, false, &list, &facts, &BTreeSet::new()));
            });
            full.textures_delta.clear();
        }
        let out = out.unwrap();
        assert!(!out.manage && out.pick.is_none() && !out.opened, "nothing is pressed by drawing");
    }
}
