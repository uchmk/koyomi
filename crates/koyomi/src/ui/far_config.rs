//! The box that edits koyomi.toml of the PC whose engine the window is showing (an `admin` over HTTPS).
//!
//! The text is read with its hash (`ConfigText`) and written back only against that hash (`ConfigPut`), so a file changed
//! in between is not overwritten; the engine keeps the old text and refuses what would cut this very connection. This
//! file holds the box's state machine and its drawing; the requests are sent, and the answers routed here, by `Browser`.
//!
//! Esc closes the innermost thing: the question, else the box. A changed text asks before it is thrown away (by Close,
//! by Reload and by Esc). While a save is in flight the box takes no other press.

use ito_a11y::Role;

use crate::proto;

/// Where the box is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage {
    /// The text has been asked for and has not come.
    Loading,
    /// The text could not be read; Reload asks again.
    Failed,
    /// The text is in the field.
    Editing,
    /// A save was sent and has not been answered.
    Saving,
}

/// What a question about the changed text is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Then {
    Close,
    Reload,
}

/// The question about the changed text: what it is for, and the button the keyboard is on (0 discards, 1 keeps editing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ask {
    then: Then,
    focus: usize,
}

/// What a frame (or a press) of the box asks the window to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Act {
    Nothing,
    Close,
    /// Ask for the text.
    Read,
    /// Send `ConfigPut` with this text and the hash the text was read at.
    Write {
        text: String,
        base: String,
    },
}

/// The box's state.
pub(super) struct ConfigEdit {
    stage: Stage,
    /// What the field holds.
    pub text: String,
    /// The text as it was read.
    read: String,
    /// Its hash, which the save is made against.
    base: String,
    /// The last refusal or reading failure, kept under the field.
    line: Option<String>,
    ask: Option<Ask>,
    /// The field takes the focus once, when it appears.
    fresh: bool,
}

impl ConfigEdit {
    /// A box that has asked for the text.
    pub(super) fn new() -> ConfigEdit {
        ConfigEdit {
            stage: Stage::Loading,
            text: String::new(),
            read: String::new(),
            base: String::new(),
            line: None,
            ask: None,
            fresh: false,
        }
    }

    pub(super) fn stage(&self) -> Stage {
        self.stage
    }

    /// The last refusal or failure, if any.
    pub(super) fn line(&self) -> Option<&str> {
        self.line.as_deref()
    }

    /// Whether the field holds a text that was changed and is not saved.
    pub(super) fn dirty(&self) -> bool {
        matches!(self.stage, Stage::Editing | Stage::Saving) && self.text != self.read
    }

    /// The answer to `ConfigText`. One that nobody waits for (the box is not loading) is dropped.
    pub(super) fn loaded(&mut self, r: Result<String, String>) {
        if self.stage != Stage::Loading {
            return;
        }
        match r.and_then(|t| proto::ConfigBody::parse(&t)) {
            Ok(body) => {
                self.text = body.text.clone();
                self.read = body.text;
                self.base = body.sha256;
                self.line = None;
                self.stage = Stage::Editing;
                self.fresh = true;
            }
            Err(e) => {
                self.line = Some(e);
                self.stage = Stage::Failed;
            }
        }
    }

    /// Save: the request to send, if the box can save now.
    pub(super) fn save(&mut self) -> Act {
        if self.stage != Stage::Editing || self.ask.is_some() {
            return Act::Nothing;
        }
        self.stage = Stage::Saving;
        self.line = None;
        Act::Write { text: self.text.clone(), base: self.base.clone() }
    }

    /// The answer to `ConfigPut`: the engine's note when it went through (the box is then done), else the refusal is kept
    /// under the field and the text stays as typed.
    pub(super) fn saved(&mut self, r: Result<String, String>) -> Option<String> {
        if self.stage != Stage::Saving {
            return None;
        }
        match r {
            Ok(note) => Some(note),
            Err(e) => {
                self.line = Some(e);
                self.stage = Stage::Editing;
                None
            }
        }
    }

    /// A request that could not be handed to the thread: a read goes to Failed (Reload asks again), a save back to
    /// editing with the text as typed; `why` is kept under the field.
    pub(super) fn unsent(&mut self, why: String) {
        match self.stage {
            Stage::Loading => self.stage = Stage::Failed,
            Stage::Saving => self.stage = Stage::Editing,
            Stage::Failed | Stage::Editing => return,
        }
        self.line = Some(why);
    }

    /// Close.
    pub(super) fn wish_close(&mut self) -> Act {
        if self.stage == Stage::Saving || self.ask.is_some() {
            return Act::Nothing;
        }
        if self.dirty() {
            // The safe button first: Enter on a fresh question throws nothing away.
            self.ask = Some(Ask { then: Then::Close, focus: 1 });
            return Act::Nothing;
        }
        Act::Close
    }

    /// Reload.
    pub(super) fn wish_reload(&mut self) -> Act {
        if !matches!(self.stage, Stage::Editing | Stage::Failed) || self.ask.is_some() {
            return Act::Nothing;
        }
        if self.dirty() {
            self.ask = Some(Ask { then: Then::Reload, focus: 1 });
            return Act::Nothing;
        }
        self.start_read()
    }

    fn start_read(&mut self) -> Act {
        self.stage = Stage::Loading;
        self.line = None;
        self.text.clear();
        self.read.clear();
        Act::Read
    }

    /// Esc: the question first, else the box.
    pub(super) fn esc(&mut self) -> Act {
        if self.ask.take().is_some() {
            return Act::Nothing;
        }
        self.wish_close()
    }

    /// Whether a question is up.
    pub(super) fn asking(&self) -> bool {
        self.ask.is_some()
    }

    /// The question's answer: 0 throws the changes away and does what was asked, anything else keeps editing.
    pub(super) fn answer(&mut self, choice: usize) -> Act {
        let Some(ask) = self.ask.take() else {
            return Act::Nothing;
        };
        if choice != 0 {
            return Act::Nothing;
        }
        match ask.then {
            Then::Close => Act::Close,
            Then::Reload => self.start_read(),
        }
    }

    /// Tab on the question moves its button.
    fn move_focus(&mut self) {
        if let Some(a) = &mut self.ask {
            a.focus = 1 - a.focus;
        }
    }

    /// Enter on the question presses its button.
    fn enter(&mut self) -> Act {
        match self.ask {
            Some(a) => self.answer(a.focus),
            None => Act::Nothing,
        }
    }
}

/// Draws the box and returns what it asks for.
pub(super) fn show(m: &mut ConfigEdit, ctx: &egui::Context, t: &ito_i18n::Table) -> Act {
    let (esc, enter, tab) = ctx.input_mut(|i| {
        let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
        let asking = m.asking();
        let enter = asking && i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
        let tab = asking && (i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) || i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab));
        (esc, enter, tab)
    });
    let mut act = Act::Nothing;
    if esc {
        act = m.esc();
    } else if enter {
        act = m.enter();
    } else if tab {
        m.move_focus();
    }
    let c = ito_theme::colors();
    let stage = m.stage();
    let asking = m.asking();
    let (mut save, mut close, mut reload) = (false, false, false);
    let mut field_focused = false;
    egui::Window::new(t.tr("far.config_title"))
        .id(egui::Id::new("far-config"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(640.0);
            let note = t.tr("far.config_note");
            let r = ui.add(egui::Label::new(egui::RichText::new(note).color(c.dim)).wrap());
            ito_a11y::say(&r, Role::Label, note);
            ui.add_space(6.0);
            match stage {
                Stage::Loading => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t.tr("far.config_loading"));
                    });
                }
                Stage::Failed => {
                    let text = t.tr("far.config_failed");
                    let r = ui.label(egui::RichText::new(text).color(c.err));
                    ito_a11y::say(&r, Role::Label, text);
                }
                Stage::Editing | Stage::Saving => {
                    let label = t.tr("far.config_field");
                    ui.add_enabled_ui(stage == Stage::Editing && !asking, |ui| {
                        egui::ScrollArea::vertical().id_salt("far-config-text").max_height(380.0).auto_shrink([false, true]).show(
                            ui,
                            |ui| {
                                let r = ui.add(
                                    egui::TextEdit::multiline(&mut m.text).code_editor().desired_width(f32::INFINITY).desired_rows(18),
                                );
                                ito_a11y::say(&r, Role::Label, label);
                                ito_a11y::test_id(&r, "far_config.field");
                                if m.fresh {
                                    r.request_focus();
                                    m.fresh = false;
                                }
                                field_focused = r.has_focus();
                            },
                        );
                    });
                }
            }
            if let Some(line) = m.line() {
                ui.add_space(6.0);
                let r = ui.add(egui::Label::new(egui::RichText::new(line).color(c.err)).wrap());
                ito_a11y::say(&r, Role::Label, line);
            }
            if stage == Stage::Saving {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(t.tr("far.config_saving"));
                });
            }
            ui.add_space(8.0);
            let idle = stage != Stage::Saving && !asking;
            ui.add_enabled_ui(idle, |ui| {
                egui::Sides::new().show(
                    ui,
                    |ui| {
                        let r = ito_prefs::IconButton::new(ito_prefs::Mark::Refresh, t.tr("far.config_reload")).show(ui);
                        ito_a11y::test_id(&r, "far_config.reload");
                        reload = r.clicked() && matches!(stage, Stage::Editing | Stage::Failed);
                    },
                    |ui| {
                        let r = ito_prefs::IconButton::new(ito_prefs::Mark::Close, t.tr("far.config_close")).show(ui);
                        ito_a11y::test_id(&r, "far_config.close");
                        close = r.clicked();
                        ui.add_enabled_ui(stage == Stage::Editing, |ui| {
                            let r = ito_prefs::IconButton::new(ito_prefs::Mark::Save, t.tr("far.config_save"))
                                .tone(ito_prefs::Tone::Primary)
                                .show(ui);
                            ito_a11y::test_id(&r, "far_config.save");
                            save = r.clicked();
                        });
                    },
                );
            });
        });
    // Ctrl+S saves while the field has the focus.
    if field_focused && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
        save = true;
    }
    if act == Act::Nothing {
        act = if save {
            m.save()
        } else if close {
            m.wish_close()
        } else if reload {
            m.wish_reload()
        } else {
            Act::Nothing
        };
    }
    if let Some(a) = m.ask {
        let buttons = [t.tr("far.config_discard").to_owned(), t.tr("far.config_keep").to_owned()];
        let body = [t.tr("far.config_ask_body").to_owned()];
        let q = ito_confirm::Question {
            title: t.tr("far.config_ask_title"),
            body: &body,
            buttons: &buttons,
            marks: &[Some(ito_prefs::Mark::Trash), Some(ito_prefs::Mark::Pencil)],
            focus: Some(a.focus),
            primary: Some(1),
            danger: true,
            ..Default::default()
        };
        if let Some(choice) = ito_confirm::show(ctx, &q, &super::Browser::style()) {
            let next = m.answer(choice);
            if next != Act::Nothing {
                act = next;
            }
        }
    }
    act
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(text: &str) -> String {
        proto::ConfigBody { text: text.to_owned(), sha256: koyomi_core::users::sha256(text) }.to_toml()
    }

    fn editing(text: &str) -> ConfigEdit {
        let mut m = ConfigEdit::new();
        m.loaded(Ok(body(text)));
        m
    }

    #[test]
    fn a_new_box_is_loading_and_holds_nothing() {
        let m = ConfigEdit::new();
        assert_eq!(m.stage(), Stage::Loading);
        assert!(!m.dirty());
        assert!(m.line().is_none());
    }

    #[test]
    fn the_text_that_comes_is_edited_and_saved_against_its_hash() {
        let mut m = editing("[http]\nport = 8700\n");
        assert_eq!(m.stage(), Stage::Editing);
        assert!(!m.dirty());
        m.text.push_str("# more\n");
        assert!(m.dirty());
        let hash = koyomi_core::users::sha256("[http]\nport = 8700\n");
        assert_eq!(m.save(), Act::Write { text: "[http]\nport = 8700\n# more\n".to_owned(), base: hash });
        assert_eq!(m.stage(), Stage::Saving);
        // Nothing else is sent while it waits.
        assert_eq!(m.save(), Act::Nothing);
        assert_eq!(m.wish_close(), Act::Nothing);
        assert_eq!(m.wish_reload(), Act::Nothing);
    }

    #[test]
    fn a_text_that_does_not_come_can_be_asked_for_again() {
        let mut m = ConfigEdit::new();
        m.loaded(Err("down".to_owned()));
        assert_eq!(m.stage(), Stage::Failed);
        assert_eq!(m.line(), Some("down"));
        assert_eq!(m.save(), Act::Nothing);
        assert_eq!(m.wish_reload(), Act::Read);
        assert_eq!(m.stage(), Stage::Loading);
        assert!(m.line().is_none());
        // A body that is not the engine's reads as a failure.
        m.loaded(Ok("not toml {".to_owned()));
        assert_eq!(m.stage(), Stage::Failed);
    }

    #[test]
    fn an_answer_nobody_waits_for_is_dropped() {
        let mut m = editing("a = 1\n");
        m.text.push('x');
        m.loaded(Ok(body("other")));
        assert_eq!(m.text, "a = 1\nx");
        assert_eq!(m.saved(Ok("done".to_owned())), None);
        assert_eq!(m.stage(), Stage::Editing);
    }

    #[test]
    fn a_refusal_is_kept_under_the_field_and_the_text_stays() {
        let mut m = editing("a = 1\n");
        m.text.push_str("b = 2\n");
        m.save();
        assert_eq!(m.saved(Err("refused".to_owned())), None);
        assert_eq!(m.stage(), Stage::Editing);
        assert_eq!(m.line(), Some("refused"));
        assert_eq!(m.text, "a = 1\nb = 2\n");
        assert!(m.dirty());
        // Saved again, the line goes.
        assert!(matches!(m.save(), Act::Write { .. }));
        assert!(m.line().is_none());
    }

    #[test]
    fn a_save_that_goes_through_hands_back_the_engines_note() {
        let mut m = editing("a = 1\n");
        m.text.push('x');
        m.save();
        assert_eq!(m.saved(Ok("written".to_owned())), Some("written".to_owned()));
    }

    #[test]
    fn a_save_that_could_not_be_sent_goes_back_to_editing_with_why() {
        let mut m = editing("a = 1\n");
        m.text.push('x');
        m.save();
        m.unsent("switching".to_owned());
        assert_eq!(m.stage(), Stage::Editing);
        assert_eq!(m.line(), Some("switching"));
        assert!(m.dirty());
    }

    #[test]
    fn a_read_that_could_not_be_sent_fails_the_box_and_reload_asks_again() {
        let mut m = ConfigEdit::new();
        m.unsent("gone".to_owned());
        assert_eq!(m.stage(), Stage::Failed);
        assert_eq!(m.line(), Some("gone"));
        assert_eq!(m.wish_reload(), Act::Read);
        // Nothing to undo once the text is in the field.
        m.loaded(Ok(body(
            "a = 1
",
        )));
        m.unsent("late".to_owned());
        assert_eq!(m.stage(), Stage::Editing);
        assert!(m.line().is_none());
    }

    #[test]
    fn an_unchanged_text_closes_and_reloads_without_asking() {
        let mut m = editing("a = 1\n");
        assert_eq!(m.wish_close(), Act::Close);
        assert_eq!(m.wish_reload(), Act::Read);
        assert_eq!(m.stage(), Stage::Loading);
    }

    #[test]
    fn a_changed_text_asks_before_close_and_before_reload() {
        let mut m = editing("a = 1\n");
        m.text.push('x');
        assert_eq!(m.wish_close(), Act::Nothing);
        assert!(m.asking());
        // Keep editing: nothing is lost.
        assert_eq!(m.answer(1), Act::Nothing);
        assert!(!m.asking());
        assert_eq!(m.text, "a = 1\nx");
        // Discard closes.
        m.wish_close();
        assert_eq!(m.answer(0), Act::Close);
        // Reload asks too, and discarding asks for the text again.
        assert_eq!(m.wish_reload(), Act::Nothing);
        assert!(m.asking());
        assert_eq!(m.answer(0), Act::Read);
        assert_eq!(m.stage(), Stage::Loading);
        assert!(!m.dirty());
    }

    #[test]
    fn esc_closes_the_question_first_and_then_the_box() {
        let mut m = editing("a = 1\n");
        m.text.push('x');
        assert_eq!(m.esc(), Act::Nothing);
        assert!(m.asking());
        assert_eq!(m.esc(), Act::Nothing);
        assert!(!m.asking());
        let mut clean = editing("a = 1\n");
        assert_eq!(clean.esc(), Act::Close);
    }

    #[test]
    fn the_keyboard_starts_on_keep_editing_and_tab_moves_it() {
        let mut m = editing("a = 1\n");
        m.text.push('x');
        m.wish_close();
        // Enter on a fresh question throws nothing away.
        assert_eq!(m.enter(), Act::Nothing);
        assert_eq!(m.text, "a = 1\nx");
        m.wish_close();
        m.move_focus();
        assert_eq!(m.enter(), Act::Close);
    }

    #[test]
    fn only_a_text_in_the_field_counts_as_unsaved() {
        let mut m = ConfigEdit::new();
        m.text.push('x');
        assert!(!m.dirty(), "loading");
        let mut m = editing("a = 1\n");
        m.text.clear();
        assert!(m.dirty());
        m.text.push_str("a = 1\n");
        assert!(!m.dirty());
    }
}
