//! The list of links in the status bar: this PC and every `[[remote]]`, each with a dot for its state and a
//! button that switches the window to it. The window still shows one engine at a time.
//!
//! While the list is open, every `[[remote]]` that is not the current link is asked `GET /health` on threads
//! of their own (at most [`MOST`] at once), once when the list opens and again [`EVERY`] after a round has
//! finished. Nothing of this is kept.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ito_a11y::Role;
use ito_theme::scale::Text;

use crate::link::Link;

/// How long after a round has finished the next one starts (while the list is open).
const EVERY: Duration = Duration::from_secs(30);
/// How many `[[remote]]` are asked at once.
const MOST: usize = 8;
/// The longest one check may take.
const DEADLINE: Duration = Duration::from_secs(5);
/// The list's width and the tallest its scrolling part gets.
const WIDTH: f32 = 320.0;
const TALL: f32 = 420.0;

/// What a check found out about a `[[remote]]` that is not the current link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seen {
    Answering,
    /// Why not, for the row's hover text.
    NoAnswer(String),
}

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
    /// The button and the link it switches to; `None` for no button.
    pub action: Option<(Button, Link)>,
}

/// The row of `name` (`None` is this PC) while the window is on `link`, `down` when that engine does not answer,
/// and `seen` is what the last check found out.
pub fn row(link: &Link, down: bool, name: Option<&str>, seen: Option<&Seen>) -> RowFacts {
    if link.far() == name {
        let (dot, text) = match (name.is_some(), down) {
            (false, false) => (Dot::Good, "link.connected"),
            (false, true) => (Dot::Bad, "link.no_answer"),
            (true, false) => (Dot::Good, "link.remote_connected"),
            (true, true) => (Dot::Bad, "link.remote_no_answer"),
        };
        return RowFacts { dot, text, action: name.map(|_| (Button::Disconnect, Link::Local)) };
    }
    let (dot, text) = match (name, seen) {
        (None, _) => (Dot::Idle, "link.pc_engine"),
        (Some(_), None) => (Dot::Idle, "link.remote_checking"),
        (Some(_), Some(Seen::Answering)) => (Dot::Good, "link.remote_ok"),
        (Some(_), Some(Seen::NoAnswer(_))) => (Dot::Bad, "link.remote_no_answer"),
    };
    let to = name.map_or(Link::Local, |n| Link::Far(n.to_owned()));
    RowFacts { dot, text, action: Some((Button::Connect, to)) }
}

/// What the list brought out in one frame.
#[derive(Default)]
pub struct Outcome {
    /// The link a button picked.
    pub pick: Option<Link>,
    /// The manage-the-connections row was pressed.
    pub manage: bool,
}

/// One check: `GET /health` of the `[[remote]]` of that name. It blocks.
fn health(name: &str) -> Result<(), String> {
    crate::remote::ask_within(name, DEADLINE, |c| c.get("/health")).map(|_| ())
}

/// The list's state: whether it is open and what the checks have found.
pub struct Hosts {
    open: bool,
    ctx: egui::Context,
    tx: Sender<(String, Result<(), String>)>,
    rx: Receiver<(String, Result<(), String>)>,
    seen: HashMap<String, Seen>,
    /// Answers still to come of the round that runs.
    in_flight: usize,
    /// When the last round finished; `None` asks for one at once.
    done_at: Option<Instant>,
}

impl Hosts {
    pub fn new(ctx: &egui::Context) -> Hosts {
        let (tx, rx) = mpsc::channel();
        Hosts { open: false, ctx: ctx.clone(), tx, rx, seen: HashMap::new(), in_flight: 0, done_at: None }
    }

    fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.done_at = None;
            // The round starts in `tick`, which has already run this frame.
            self.ctx.request_repaint();
        }
    }

    /// Takes the answers that came. One for a name that is not in `names` any more is dropped.
    fn drain(&mut self, names: &[String], now: Instant) {
        while let Ok((name, result)) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            if names.contains(&name) {
                let seen = match result {
                    Ok(()) => Seen::Answering,
                    Err(why) => Seen::NoAnswer(why),
                };
                self.seen.insert(name, seen);
            }
            if self.in_flight == 0 {
                self.done_at = Some(now);
            }
        }
    }

    /// The names a round asks now: none while the list is closed, while a round is in flight, or before the
    /// next one is due. The current link is not asked.
    fn plan(&mut self, names: &[String], current: Option<&str>, now: Instant) -> Vec<String> {
        let due = self.done_at.is_none_or(|t| now.saturating_duration_since(t) >= EVERY);
        if !self.open || self.in_flight > 0 || !due {
            return Vec::new();
        }
        let list: Vec<String> = names.iter().filter(|n| Some(n.as_str()) != current).cloned().collect();
        if list.is_empty() {
            self.done_at = Some(now);
        }
        self.in_flight = list.len();
        list
    }

    /// Asks `list` with `check`, at most [`MOST`] at once. A late answer for a window that is gone goes nowhere.
    fn spawn(&self, list: Vec<String>, check: fn(&str) -> Result<(), String>) {
        let workers = MOST.min(list.len());
        let queue = Arc::new(Mutex::new(VecDeque::from(list)));
        for _ in 0..workers {
            let (queue, tx, ctx) = (queue.clone(), self.tx.clone(), self.ctx.clone());
            std::thread::spawn(move || loop {
                let next = queue.lock().ok().and_then(|mut q| q.pop_front());
                let Some(name) = next else { break };
                // Every name must answer, or the round stays in flight and the list never refreshes.
                let result = std::panic::catch_unwind(|| check(&name))
                    .unwrap_or_else(|_| Err(koyomi_core::msg::line(koyomi_core::msg::HOST_CHECK_STOPPED, &[&name])));
                let _ = tx.send((name, result));
                ctx.request_repaint();
            });
        }
    }

    fn tick(&mut self, names: &[String], current: Option<&str>) {
        let now = Instant::now();
        self.drain(names, now);
        let list = self.plan(names, current, now);
        if !list.is_empty() {
            self.spawn(list, health);
        }
        if self.open {
            self.ctx.request_repaint_after(EVERY);
        }
    }

    /// The status-bar button and, when it is open, the list above it.
    pub fn show(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table, link: &Link, down: bool, names: &[String]) -> Outcome {
        let c = ito_theme::colors();
        self.tick(names, link.far());
        let this_pc = t.tr("link.this_pc");
        let current = link.far().unwrap_or(this_pc);
        let facts = row(link, down, link.far(), None);
        let button = ui
            .horizontal(|ui| {
                ito_statusbar::dot(ui, colour(&c, facts.dot));
                ui.add(
                    egui::Label::new(egui::RichText::new(format!("{current} \u{25b4}")).size(Text::ChromeButton.size()).color(c.fg))
                        .sense(egui::Sense::click()),
                )
            })
            .inner
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        ito_a11y::say(&button, Role::Button, &format!("{} {current}", t.tr("link.pick")));
        ito_a11y::test_id(&button, "link.picker");
        if button.clicked() {
            self.toggle();
        }
        let mut out = Outcome::default();
        let mut open = self.open;
        let mut close = false;
        let seen = &self.seen;
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
                        let seen = name.and_then(|n| seen.get(n));
                        let facts = row(link, down, name, seen);
                        if let Some(to) = draw_row(ui, t, name, &facts, seen) {
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
                    ui.label(egui::RichText::new(t.tr(facts.text)).size(Text::ChromeSmall.size()).color(c.dim));
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
    if let Some(Seen::NoAnswer(why)) = seen {
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

    fn no(why: &str) -> Seen {
        Seen::NoAnswer(why.to_owned())
    }

    #[test]
    fn this_pc_current_is_connected_and_has_no_button() {
        let r = row(&Link::Local, false, None, None);
        assert_eq!(r, RowFacts { dot: Dot::Good, text: "link.connected", action: None });
    }

    #[test]
    fn this_pc_current_and_down_is_red_with_no_button() {
        let r = row(&Link::Local, true, None, None);
        assert_eq!(r, RowFacts { dot: Dot::Bad, text: "link.no_answer", action: None });
    }

    #[test]
    fn this_pc_not_current_connects_to_local() {
        let r = row(&far("kei"), false, None, None);
        assert_eq!(r, RowFacts { dot: Dot::Idle, text: "link.pc_engine", action: Some((Button::Connect, Link::Local)) });
        // What this PC's engine is doing is not known from a remote: the same with a failed link.
        assert_eq!(row(&far("kei"), true, None, None), r);
    }

    #[test]
    fn current_remote_disconnects_to_local() {
        let r = row(&far("kei"), false, Some("kei"), None);
        assert_eq!(r, RowFacts { dot: Dot::Good, text: "link.remote_connected", action: Some((Button::Disconnect, Link::Local)) });
    }

    #[test]
    fn current_remote_down_is_red_and_still_disconnects() {
        let r = row(&far("kei"), true, Some("kei"), Some(&Seen::Answering));
        assert_eq!(r, RowFacts { dot: Dot::Bad, text: "link.remote_no_answer", action: Some((Button::Disconnect, Link::Local)) });
    }

    #[test]
    fn other_remote_not_checked_yet_is_grey_and_connects() {
        for link in [Link::Local, far("kei")] {
            let r = row(&link, false, Some("works"), None);
            assert_eq!(r, RowFacts { dot: Dot::Idle, text: "link.remote_checking", action: Some((Button::Connect, far("works"))) });
        }
    }

    #[test]
    fn other_remote_answering_is_green() {
        let r = row(&Link::Local, false, Some("works"), Some(&Seen::Answering));
        assert_eq!(r, RowFacts { dot: Dot::Good, text: "link.remote_ok", action: Some((Button::Connect, far("works"))) });
    }

    #[test]
    fn a_failed_check_still_offers_connect() {
        let r = row(&Link::Local, false, Some("works"), Some(&no("timed out")));
        assert_eq!(r, RowFacts { dot: Dot::Bad, text: "link.remote_no_answer", action: Some((Button::Connect, far("works"))) });
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn an_answer_for_a_name_no_longer_listed_is_ignored() {
        let mut h = Hosts::new(&egui::Context::default());
        h.in_flight = 2;
        h.tx.send(("gone".into(), Ok(()))).unwrap();
        h.tx.send(("kei".into(), Err("no".into()))).unwrap();
        h.drain(&names(&["kei"]), Instant::now());
        assert!(!h.seen.contains_key("gone"));
        assert_eq!(h.seen.get("kei"), Some(&no("no")));
        assert_eq!(h.in_flight, 0, "both were counted");
        assert!(h.done_at.is_some());
    }

    #[test]
    fn a_check_that_panics_still_answers() {
        let h = Hosts::new(&egui::Context::default());
        h.spawn(names(&["kei"]), |_| panic!("a check that panics, on purpose"));
        let (name, result) = h.rx.recv_timeout(Duration::from_secs(10)).expect("the round ends");
        assert_eq!(name, "kei");
        assert!(result.is_err());
    }

    #[test]
    fn no_round_starts_while_one_is_in_flight_or_the_list_is_closed() {
        let mut h = Hosts::new(&egui::Context::default());
        let list = names(&["a", "b", "c"]);
        let now = Instant::now();
        assert!(h.plan(&list, None, now).is_empty(), "closed");
        h.toggle();
        assert_eq!(h.plan(&list, Some("b"), now), names(&["a", "c"]), "the current link is not asked");
        assert!(h.plan(&list, Some("b"), now).is_empty(), "one is in flight");
        h.in_flight = 0;
        h.done_at = Some(now);
        assert!(h.plan(&list, None, now + Duration::from_secs(29)).is_empty(), "not due");
        assert_eq!(h.plan(&list, None, now + EVERY).len(), 3);
    }

    #[test]
    fn opening_again_asks_at_once() {
        let mut h = Hosts::new(&egui::Context::default());
        let list = names(&["a"]);
        let now = Instant::now();
        h.toggle();
        assert_eq!(h.plan(&list, None, now).len(), 1);
        h.in_flight = 0;
        h.done_at = Some(now);
        h.toggle();
        h.toggle();
        assert_eq!(h.plan(&list, None, now).len(), 1);
    }

    #[test]
    fn a_list_with_only_the_current_link_finishes_the_round_at_once() {
        let mut h = Hosts::new(&egui::Context::default());
        h.toggle();
        assert!(h.plan(&names(&["a"]), Some("a"), Instant::now()).is_empty());
        assert_eq!(h.in_flight, 0);
        assert!(h.done_at.is_some());
    }

    #[test]
    fn the_manage_row_is_in_the_list_and_the_add_row_is_gone() {
        let (t, _) = crate::i18n::table("ja");
        assert_eq!(t.tr("link.manage"), "接続先を管理…");
        let ctx = egui::Context::default();
        let mut h = Hosts::new(&ctx);
        h.toggle();
        let list = names(&["kei"]);
        let mut out = None;
        for _ in 0..3 {
            let mut full = ctx.run_ui(egui::RawInput::default(), |ui| {
                out = Some(h.show(ui, &t, &Link::Local, false, &list));
            });
            full.textures_delta.clear();
        }
        let out = out.unwrap();
        assert!(!out.manage && out.pick.is_none(), "nothing is pressed by drawing");
    }

    #[test]
    fn spawn_answers_every_name_over_the_channel() {
        let mut h = Hosts::new(&egui::Context::default());
        let list: Vec<String> = (0..20).map(|i| format!("h{i}")).collect();
        h.toggle();
        let ask = h.plan(&list, None, Instant::now());
        h.spawn(ask, |n| if n == "h3" { Err("down".into()) } else { Ok(()) });
        let end = Instant::now() + Duration::from_secs(10);
        while h.in_flight > 0 && Instant::now() < end {
            h.drain(&list, Instant::now());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(h.in_flight, 0);
        assert_eq!(h.seen.len(), 20);
        assert_eq!(h.seen.get("h3"), Some(&no("down")));
        assert_eq!(h.seen.get("h4"), Some(&Seen::Answering));
    }
}
