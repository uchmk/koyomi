//! The keys: `defaults/keymap.toml` built in, the person's own `keymap.toml`
//! (in koyomi's folder under the uchmk one) laid over it, as minato's.
//!
//! koyomi has one layer, `app`: the keys that work anywhere in the window.
//! The tree and the boxes take their own keys (Esc, Enter, Tab) before these.

use ito_keymap::{Binding, Layer, Match, Spec};
use ito_keys::Key;

/// The built-in keys, as the person's file is laid out.
pub const DEFAULTS: &str = include_str!("../defaults/keymap.toml");

/// The person's own file, in koyomi's folder.
pub const FILE: &str = "keymap.toml";

/// `run = "none"`: the key does nothing (unbinds a built-in one).
pub const NOOP: &str = "none";

const LAYERS: [Layer; 1] = [Layer::new("app")];

pub const SPEC: Spec = Spec { layers: &LAYERS, ignored: &[], defaults: DEFAULTS };

/// Keys koyomi takes before the keymap sees them: the bug report, the
/// palette, and the scale. A file that binds one is told so.
const FIXED: [&str; 6] = ["<F12>", "<C-S-p>", "<C-=>", "<C-+>", "<C-->", "<C-0>"];

/// What a key can run.
pub const ACTIONS: [&str; 3] = ["settings", "help", "quit"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    Run(&'static str),
    None,
}

/// The `run` of a line as a command; `None` with a warning when it is no
/// action of koyomi.
fn command(text: &str, warnings: &mut Vec<String>) -> Option<Cmd> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text == NOOP {
        return Some(Cmd::None);
    }
    match ACTIONS.iter().find(|a| **a == text) {
        Some(a) => Some(Cmd::Run(a)),
        None => {
            warnings.push(format!("keymap: `{text}` is no action of koyomi"));
            None
        }
    }
}

/// The bindings in force, by layer (koyomi has the one, `app`).
pub struct Keys {
    layers: Vec<(String, Vec<Binding<Cmd>>)>,
}

impl Keys {
    /// The defaults with `user` (its path for the warnings, and its text)
    /// laid over; what the file got wrong comes back as warnings.
    pub fn load(user: Option<(&str, &str)>) -> (Self, Vec<String>) {
        let files: Vec<(&str, &str)> = user.into_iter().collect();
        let mut warnings = Vec::new();
        let mut km = ito_keymap::load(&SPEC, &files, &mut |t, w| command(t, w));
        warnings.append(&mut km.warnings);
        let mut app = km.take("app");
        app.retain(|b| match Self::fixed(&b.on) {
            Some(k) => {
                warnings.push(format!("{}: [app] `{}` is fixed and cannot be bound", b.from, k.human()));
                false
            }
            None => true,
        });
        (Self { layers: vec![("app".to_owned(), app)] }, warnings)
    }

    fn app(&self) -> &[Binding<Cmd>] {
        &self.layers[0].1
    }

    /// The layers as the settings' Keys page lists them.
    pub fn layers(&self) -> &[(String, Vec<Binding<Cmd>>)] {
        &self.layers
    }

    /// The built-in keys alone, for the Keys page to show what a change
    /// departs from.
    pub fn built_in_layers() -> &'static [(String, Vec<Binding<Cmd>>)] {
        static BUILT_IN: std::sync::OnceLock<Vec<(String, Vec<Binding<Cmd>>)>> = std::sync::OnceLock::new();
        BUILT_IN.get_or_init(|| Self::load(None).0.layers)
    }

    /// The key in `keys` that koyomi takes before the keymap, if any.
    pub fn fixed(keys: &[Key]) -> Option<&Key> {
        keys.iter().find(|k| FIXED.iter().any(|f| Key::parse(f).as_ref() == Some(k)))
    }

    /// What `pressed` (the chord so far) runs.
    pub fn resolve(&self, pressed: &[Key]) -> Match<'_, Cmd> {
        ito_keymap::resolve(self.app(), pressed)
    }

    /// The keys that run `name`, as the palette shows them; empty when it
    /// is unbound.
    pub fn label(&self, name: &str) -> String {
        self.app()
            .iter()
            .find(|b| matches!(b.run.as_slice(), [Cmd::Run(n)] if *n == name))
            .filter(|b| matches!(self.resolve(&b.on), Match::Exact(e) if std::ptr::eq(e, *b)))
            .map(|b| ito_keys::human_seq(&b.on))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(k: &Keys, on: &str) -> Option<Vec<Cmd>> {
        let on: Vec<Key> = on.split_whitespace().map(|t| Key::parse(t).unwrap()).collect();
        match k.resolve(&on) {
            Match::Exact(b) => Some(b.run.clone()),
            _ => None,
        }
    }

    #[test]
    fn the_defaults_load_without_a_word_and_bind_every_action() {
        let (k, warnings) = Keys::load(None);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(run(&k, "<C-q>"), Some(vec![Cmd::Run("quit")]));
        assert_eq!(run(&k, "<F1>"), Some(vec![Cmd::Run("help")]));
        assert_eq!(run(&k, "<C-,>"), Some(vec![Cmd::Run("settings")]));
        assert_eq!(run(&k, "q"), None);
        for a in ACTIONS {
            assert!(k.app().iter().any(|b| b.run == [Cmd::Run(a)]), "{a}");
            assert!(!k.label(a).is_empty(), "{a}");
        }
        assert_eq!(k.label("quit"), "Ctrl+Q");
    }

    #[test]
    fn the_built_in_layers_are_the_defaults_once() {
        let b = Keys::built_in_layers();
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].0, "app");
        assert_eq!(b[0].1.len(), Keys::load(None).0.layers()[0].1.len());
        assert!(std::ptr::eq(b, Keys::built_in_layers()));
    }

    #[test]
    fn a_file_unbinds_rebinds_and_is_told_what_it_got_wrong() {
        let text = "[[app.keymap]]\non = \"<C-q>\"\nrun = \"none\"\n\n[[app.keymap]]\non = \"<C-w>\"\nrun = \"quit\"\n\n[[app.keymap]]\non = \"<F12>\"\nrun = \"help\"\n\n[[app.keymap]]\non = \"<C-x>\"\nrun = \"explode\"\n";
        let (k, warnings) = Keys::load(Some(("keymap.toml", text)));
        assert_eq!(run(&k, "<C-q>"), Some(vec![Cmd::None]));
        assert_eq!(run(&k, "<C-w>"), Some(vec![Cmd::Run("quit")]));
        assert_eq!(run(&k, "<F12>"), None);
        assert_eq!(run(&k, "<C-x>"), None);
        assert_eq!(k.label("quit"), "Ctrl+W");
        assert!(warnings.iter().any(|w| w.contains("`<F12>`") || w.contains("F12")), "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("explode")), "{warnings:?}");
    }
}
