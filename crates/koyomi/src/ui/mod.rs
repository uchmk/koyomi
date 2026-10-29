//! The window's left tree, the detail beside it and the approval box.
//!
//! Nothing here touches the disk or a process: a thread keeps the engine up and asks it for the
//! list, and the window reads what that thread sent. An approval is sent the same way.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use ito_a11y::Role;
use koyomi_core::scan::{Def, Standing};

use crate::proto::{Request, Snapshot};
use crate::{client, review};

/// How often the engine is asked for the list.
const POLL: Duration = Duration::from_secs(1);

enum Msg {
    Starting,
    Snapshot(Snapshot),
    Down(String),
    Approved(Result<String, String>),
}

pub struct Approval {
    id: String,
    sha256: String,
}

pub struct Browser {
    rx: Receiver<Msg>,
    tx: Sender<Request>,
    snap: Option<Snapshot>,
    down: Option<String>,
    starting: bool,
    selected: Option<String>,
    asking: Option<Approval>,
    focus: usize,
    /// The last answer to an approval, shown under the detail.
    result: Option<Result<String, String>>,
}

impl Browser {
    pub fn start(ctx: &egui::Context) -> Browser {
        let (tx_msg, rx) = mpsc::channel::<Msg>();
        let (tx, rx_req) = mpsc::channel::<Request>();
        let ctx = ctx.clone();
        std::thread::spawn(move || loop {
            let send = |m: Msg| {
                let alive = tx_msg.send(m).is_ok();
                ctx.request_repaint();
                alive
            };
            if !client::running() {
                if !send(Msg::Starting) {
                    return;
                }
                if let Err(e) = client::start() {
                    if !send(Msg::Down(e)) {
                        return;
                    }
                    std::thread::sleep(POLL * 3);
                    continue;
                }
            }
            match rx_req.recv_timeout(POLL) {
                Ok(request) => {
                    if !send(Msg::Approved(client::call(&request))) {
                        return;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            let m = match client::call(&Request::List).and_then(|t| Snapshot::parse(&t)) {
                Ok(s) => Msg::Snapshot(s),
                Err(e) => Msg::Down(e),
            };
            if !send(m) {
                return;
            }
        });
        Browser { rx, tx, snap: None, down: None, starting: true, selected: None, asking: None, focus: 0, result: None }
    }

    fn drain(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Starting => self.starting = true,
                Msg::Snapshot(s) => {
                    self.starting = false;
                    self.down = None;
                    self.snap = Some(s);
                }
                Msg::Down(e) => {
                    self.starting = false;
                    self.down = Some(e);
                }
                Msg::Approved(r) => self.result = Some(r),
            }
        }
    }

    fn blocked(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| !s.warnings.is_empty())
    }

    fn selected_def(&self) -> Option<&Def> {
        let id = self.selected.as_deref()?;
        self.snap.as_ref()?.defs.iter().find(|d| d.id == id)
    }

    /// Draws the warning band, the tree, the detail and, when asked for, the approval box.
    ///
    /// While the box is up it is the only thing that answers: its keys are taken before anything
    /// behind it is drawn, what is behind is disabled, and no widget keeps the keyboard focus.
    pub fn show(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        self.drain();
        let ctx = ui.ctx().clone();
        let modal = self.asking.is_some();
        let key = if modal { self.box_keys(&ctx) } else { None };
        let c = ito_theme::colors();
        if let Some(snap) = self.snap.as_ref().filter(|s| !s.warnings.is_empty()) {
            let lines: Vec<String> = snap.warnings.clone();
            egui::Panel::top("band").frame(egui::Frame::NONE.fill(c.wait_bg()).inner_margin(8.0)).show(ui, |ui| {
                let mark = ito_a11y::State::Warning.mark();
                let head = format!("{mark} {}", t.tr("band.warning"));
                let r = ui.label(egui::RichText::new(head.clone()).strong().color(c.wait_text()));
                ito_a11y::say(&r, Role::Label, &head);
                for l in &lines {
                    ui.label(egui::RichText::new(l).color(c.wait_text()));
                }
            });
        }
        egui::Panel::left("tree").default_size(300.0).show(ui, |ui| {
            if modal {
                ui.disable();
            }
            self.tree(ui, t);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if modal {
                ui.disable();
            }
            self.detail(ui, t);
        });
        self.confirm(&ctx, t, key);
        if modal {
            // Tab moves egui's focus before any of this runs; drop it so the next Enter or Space
            // cannot press a button behind the box (or in it: the box's own focus is `self.focus`).
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
    }

    /// Esc (cancel) and Enter (the button the keyboard is on) answer the box; Tab and Shift+Tab move between its two buttons.
    fn box_keys(&mut self, ctx: &egui::Context) -> Option<usize> {
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                Some(1)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                Some(self.focus)
            } else {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) || i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab) {
                    self.focus = (self.focus + 1) % 2;
                }
                None
            }
        })
    }

    fn tree(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let Some(snap) = &self.snap else {
            let text = match (&self.down, self.starting) {
                (Some(e), _) => t.trf("tree.failed", &[e]),
                (None, _) => t.tr("tree.starting").to_owned(),
            };
            ui.label(text);
            return;
        };
        if snap.defs.is_empty() {
            ui.add(egui::Label::new(t.tr("tree.empty")).wrap());
            return;
        }
        if let Some(e) = &self.down {
            ui.label(egui::RichText::new(t.trf("tree.failed", &[e])).color(c.err));
        }
        let mut pick = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut group: Option<&str> = None;
            for d in &snap.defs {
                if group != Some(d.group.as_str()) {
                    group = Some(&d.group);
                    let r = ui.label(egui::RichText::new(&d.group).strong().color(c.dim));
                    ito_a11y::say(&r, Role::Heading, &d.group);
                }
                let text = review::row(t, d);
                let on = self.selected.as_deref() == Some(d.id.as_str());
                let r = ui.selectable_label(on, format!("  {text}"));
                ito_a11y::say(&r, Role::Row, &text);
                if r.clicked() {
                    pick = Some(d.id.clone());
                }
            }
        });
        if pick.is_some() {
            self.selected = pick;
            self.result = None;
        }
    }

    fn detail(&mut self, ui: &mut egui::Ui, t: &ito_i18n::Table) {
        let c = ito_theme::colors();
        let Some(def) = self.selected_def().cloned() else { return };
        let head = review::row(t, &def);
        ui.heading(&head);
        ui.label(egui::RichText::new(&def.file).color(c.dim));
        ui.add_space(6.0);
        for p in &def.problems {
            let line = p.in_file(&def.file);
            ui.label(egui::RichText::new(format!("{} {line}", ito_a11y::State::Error.mark())).color(c.err));
        }
        for l in review::body(t, &def) {
            ui.label(l);
        }
        ui.add_space(8.0);
        let can = def.standing == Standing::Unconfirmed && !self.blocked();
        let r = ui.add_enabled(can, egui::Button::new(t.tr("confirm.approve")));
        ito_a11y::say(&r, Role::Button, t.tr("confirm.approve"));
        if r.clicked() {
            self.asking = Some(Approval { id: def.id.clone(), sha256: def.sha256.clone() });
            self.focus = 0;
        }
        match &self.result {
            Some(Ok(m)) => {
                ui.label(egui::RichText::new(format!("{} {m}", ito_a11y::State::Done.mark())).color(c.done));
            }
            Some(Err(e)) => {
                ui.label(egui::RichText::new(format!("{} {e}", ito_a11y::State::Error.mark())).color(c.err));
            }
            None => {}
        }
    }

    /// The box that shows what would run, with the change since the last approval.
    fn confirm(&mut self, ctx: &egui::Context, t: &ito_i18n::Table, key: Option<usize>) {
        let Some(ask) = &self.asking else { return };
        let Some(def) = self.snap.as_ref().and_then(|s| s.defs.iter().find(|d| d.id == ask.id)).cloned() else {
            self.asking = None;
            return;
        };
        // What was looked at must be what is approved: a change since closes the box.
        if def.sha256 != ask.sha256 || def.standing != Standing::Unconfirmed || self.blocked() {
            self.asking = None;
            return;
        }
        let c = ito_theme::colors();
        let style = ito_confirm::Style {
            panel: c.raised(),
            border: c.border,
            accent: Some(c.wait),
            title: c.strong(),
            text: c.fg,
            dim: c.dim,
            warn: c.wait_text(),
            button_bg: c.chosen(),
            button_text: c.fg,
        };
        let body = review::body(t, &def);
        let diff = review::diff(&def);
        let rows = [(t.tr("review.file").to_owned(), def.file.clone())];
        let buttons = [t.tr("confirm.yes").to_owned(), t.tr("confirm.no").to_owned()];
        let q = ito_confirm::Question {
            title: t.tr("confirm.title"),
            rows: &rows,
            body: &body,
            diff: &diff,
            buttons: &buttons,
            hint: t.tr("confirm.hint"),
            focus: Some(self.focus),
            primary: Some(0),
            ..Default::default()
        };
        match ito_confirm::show(ctx, &q, &style).or(key) {
            Some(0) => {
                let (id, sha256) = (ask.id.clone(), ask.sha256.clone());
                let _ = self.tx.send(Request::Approve { id, sha256 });
                self.asking = None;
            }
            Some(_) => self.asking = None,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koyomi_core::approved::Store;
    use koyomi_core::scan::read_def;

    const NET: &str = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";

    fn defs() -> Vec<Def> {
        let dir = std::env::temp_dir().join(format!("koyomi-view-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        ["bom", "バックアップ", "月次締め"]
            .iter()
            .map(|n| {
                let p = dir.join(format!("{n}.toml"));
                std::fs::write(&p, NET).unwrap();
                read_def(&p, "試験", n, &Store::default())
            })
            .collect()
    }

    /// A browser on a fixed list with no engine behind it, asking about `bom`.
    fn asking() -> Browser {
        let (_, rx) = mpsc::channel();
        let (tx, _) = mpsc::channel();
        let defs = defs();
        let ask = Approval { id: defs[0].id.clone(), sha256: defs[0].sha256.clone() };
        let selected = Some(defs[0].id.clone());
        let snap = Some(Snapshot { defs, warnings: Vec::new() });
        Browser { rx, tx, snap, down: None, starting: false, selected, asking: Some(ask), focus: 0, result: None }
    }

    fn frame(ctx: &egui::Context, size: egui::Vec2, events: Vec<egui::Event>, b: &mut Browser) {
        let t = crate::i18n::table("ja").0;
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| b.show(ui, &t));
        out.textures_delta.clear();
    }

    fn key(k: egui::Key) -> Vec<egui::Event> {
        let e = |pressed| egui::Event::Key { key: k, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
        vec![e(true), e(false)]
    }

    /// The box's title, rows and buttons are on the screen, however many frames it is up.
    #[test]
    #[ignore = "ito-confirm grows every frame: uchmk/ito#71"]
    fn the_confirm_box_stays_inside_the_window() {
        for size in [egui::vec2(1280.0, 800.0), egui::vec2(640.0, 360.0)] {
            let ctx = egui::Context::default();
            let mut b = asking();
            for _ in 0..30 {
                frame(&ctx, size, Vec::new(), &mut b);
            }
            let r = ctx.memory(|m| m.area_rect(egui::Id::new("ito-confirm"))).expect("drawn");
            assert!(r.top() >= 0.0 && r.bottom() <= size.y, "{size:?}: {r:?}");
        }
    }

    /// A box after a taller one is as tall as what it holds.
    #[test]
    #[ignore = "ito-confirm grows every frame: uchmk/ito#71"]
    fn a_short_box_after_a_tall_one_is_short() {
        let ctx = egui::Context::default();
        let size = egui::vec2(1280.0, 800.0);
        let mut b = asking();
        let height = |ctx: &egui::Context| ctx.memory(|m| m.area_rect(egui::Id::new("ito-confirm"))).expect("drawn").height();
        for _ in 0..3 {
            frame(&ctx, size, Vec::new(), &mut b);
        }
        let short = height(&ctx);
        let long: String = (0..40)
            .map(|i| {
                format!(
                    "# line {i}
"
                )
            })
            .collect();
        b.snap.as_mut().unwrap().defs[0].previous = Some(long);
        for _ in 0..3 {
            frame(&ctx, size, Vec::new(), &mut b);
        }
        let tall = height(&ctx);
        b.snap.as_mut().unwrap().defs[0].previous = None;
        for _ in 0..10 {
            frame(&ctx, size, Vec::new(), &mut b);
        }
        assert!(height(&ctx) <= short + 1.0, "short {short} tall {tall} again {}", height(&ctx));
    }

    /// 4.5: Tab and Enter answer the box and reach nothing behind it.
    #[test]
    fn keys_do_not_reach_the_tree_behind_the_box() {
        let ctx = egui::Context::default();
        let mut b = asking();
        let size = egui::vec2(1280.0, 800.0);
        frame(&ctx, size, Vec::new(), &mut b);
        frame(&ctx, size, key(egui::Key::Tab), &mut b);
        assert_eq!(b.focus, 1);
        assert!(ctx.memory(|m| m.focused()).is_none(), "nothing behind the box takes the focus");
        frame(&ctx, size, key(egui::Key::Enter), &mut b);
        assert!(b.asking.is_none(), "Enter on the cancel button closes the box");
        assert_eq!(b.selected.as_deref(), Some("試験/bom"), "the tree did not pick another row");
    }
}
