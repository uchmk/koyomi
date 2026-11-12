//! A path shown to the person comes with two buttons: open it, and copy it.
//!
//! Where the path lives decides whether the buttons make sense. A file this window wrote is on this PC
//! whichever engine it talks to; a path the engine reports (a run folder, a log, a net's file) is on the
//! engine's PC, and this window can open it only when that PC is this one.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ito_prefs::{IconButton, Mark};

/// Which PC a path is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// A file this window wrote, in this PC's state folder (an export, a CSV, the printed PDF).
    Here,
    /// A path on the engine's PC (a run folder, a log file, a net's `def.file`).
    Engine,
}

/// A path that came with a result line, and the text of that line. The path is shown only while that
/// line is the result: a later result, which carries no path of its own, never shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Made {
    pub text: String,
    pub path: PathBuf,
    pub place: Place,
}

impl Made {
    /// The path to show under `result`, when `result` is the line this path came with.
    pub fn for_result<'a>(made: &'a Option<Made>, result: &Option<Result<String, String>>) -> Option<&'a Made> {
        let made = made.as_ref()?;
        matches!(result, Some(Ok(text)) if *text == made.text).then_some(made)
    }
}

/// Whether the two buttons are drawn for a path at `place`, with the engine on this PC or not.
pub fn buttons_shown(place: Place, local_engine: bool) -> bool {
    match place {
        Place::Here => true,
        Place::Engine => local_engine,
    }
}

/// What the open button opens for `path`, and whether that is a folder: the path itself when there is one, else the
/// nearest folder above it that exists (a file or a folder that was removed, or not made yet).
pub fn open_target(path: &Path) -> (PathBuf, bool) {
    if path.is_dir() {
        return (path.to_path_buf(), true);
    }
    if path.exists() {
        return (path.to_path_buf(), false);
    }
    // The empty ancestor of a relative path is the current folder, where a relative path is read from.
    let parent = path.ancestors().skip(1).find_map(|p| {
        if p.as_os_str().is_empty() {
            Some(PathBuf::from("."))
        } else {
            p.is_dir().then(|| p.to_path_buf())
        }
    });
    (parent.unwrap_or_else(|| path.to_path_buf()), true)
}

/// A net's definition file on the engine's PC when that is this PC: `file` is the path as the person sees it
/// (`group/name.toml`), under the settings folder's `jobnets`.
pub fn net_file(file: &str) -> Option<PathBuf> {
    let paths = crate::engine::Paths::find().ok()?;
    Some(file.split('/').fold(paths.jobnets, |p, part| p.join(part)))
}

/// The text copied for `path`: the plain path, as this OS writes it, with no quotes.
pub fn copy_text(path: &Path) -> String {
    path.display().to_string()
}

/// Puts the path on the clipboard.
pub fn copy_path(path: &Path) -> Result<(), String> {
    ito_clipboard::clipboard!().set_text(&copy_text(path)).map_err(|e| e.to_string())
}

/// What the buttons last told, kept per path in the window's memory.
#[derive(Clone)]
enum Note {
    Copied(f64),
    Failed(String),
}

/// How long the "path copied" note stays.
const NOTE_SECONDS: f64 = 4.0;

/// How long what `open_target` found for a path is trusted before it is looked at again.
const LOOK_SECONDS: f64 = 2.0;

/// What `open_target` found for a path, and when (in egui time).
#[derive(Clone)]
struct Seen {
    target: PathBuf,
    folder: bool,
    at: f64,
}

/// Marks a path that a thread is looking at now.
#[derive(Clone, Copy)]
struct Looking;

/// The open and copy buttons for `path`, when `buttons_shown`. Drawn in a row at the cursor.
pub fn path_buttons(ui: &mut egui::Ui, t: &ito_i18n::Table, path: &Path, place: Place, local_engine: bool) {
    if !buttons_shown(place, local_engine) {
        return;
    }
    let shown = path.display().to_string();
    let id = egui::Id::new(("path-note", &shown));
    // The file system is never looked at on this thread (a share that does not answer would stall the window):
    // a thread looks, and the button is drawn from the last answer (as a file until there is one).
    let seen_id = egui::Id::new(("path-seen", &shown));
    let looking_id = egui::Id::new(("path-looking", &shown));
    let now = ui.input(|i| i.time);
    let seen = ui.ctx().data(|d| d.get_temp::<Seen>(seen_id));
    if seen.as_ref().is_none_or(|s| now - s.at > LOOK_SECONDS) && ui.ctx().data(|d| d.get_temp::<Looking>(looking_id)).is_none() {
        ui.ctx().data_mut(|d| d.insert_temp(looking_id, Looking));
        let ctx = ui.ctx().clone();
        let path = path.to_path_buf();
        std::thread::spawn(move || {
            let (target, folder) = open_target(&path);
            let at = ctx.input(|i| i.time);
            ctx.data_mut(|d| {
                d.insert_temp(seen_id, Seen { target, folder, at });
                d.remove::<Looking>(looking_id);
            });
            ctx.request_repaint();
        });
    }
    let (mark, key, hint) = match &seen {
        None => (Mark::File, "path.open_file", t.trf("path.open_hint", &[&shown])),
        Some(s) => {
            let (mark, key) = if s.folder { (Mark::Folder, "path.open_folder") } else { (Mark::File, "path.open_file") };
            let hint = if s.target == path { t.trf("path.open_hint", &[&shown]) } else { t.trf("path.parent_hint", &[&shown]) };
            (mark, key, hint)
        }
    };
    ui.horizontal_wrapped(|ui| {
        if IconButton::new(mark, t.tr(key)).show(ui).on_hover_text(hint).clicked() {
            let ctx = ui.ctx().clone();
            ctx.data_mut(|d| d.remove::<Note>(id));
            let path = path.to_path_buf();
            std::thread::spawn(move || {
                let (target, _) = open_target(&path);
                if let Err(e) = crate::prefs::open_path(&target) {
                    ctx.data_mut(|d| d.insert_temp(id, Note::Failed(e)));
                    ctx.request_repaint();
                }
            });
        }
        if IconButton::new(Mark::Copy, t.tr("path.copy")).show(ui).on_hover_text(t.trf("path.copy_hint", &[&shown])).clicked() {
            let note = match copy_path(path) {
                Ok(()) => Note::Copied(ui.input(|i| i.time)),
                Err(e) => Note::Failed(e),
            };
            ui.ctx().data_mut(|d| d.insert_temp(id, note));
        }
        match ui.ctx().data(|d| d.get_temp::<Note>(id)) {
            Some(Note::Copied(at)) if now - at < NOTE_SECONDS => {
                ui.label(egui::RichText::new(t.tr("path.copied")).color(ito_theme::colors().done));
                ui.ctx().request_repaint_after(Duration::from_secs_f64(NOTE_SECONDS - (now - at) + 0.05));
            }
            Some(Note::Copied(_)) | None => {}
            Some(Note::Failed(e)) => {
                ui.label(egui::RichText::new(e).color(ito_theme::colors().err));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The buttons: this window's own files always, the engine's paths only on this PC.
    #[test]
    fn buttons_follow_the_place_and_the_engine() {
        assert!(buttons_shown(Place::Here, true) && buttons_shown(Place::Here, false));
        assert!(buttons_shown(Place::Engine, true));
        assert!(!buttons_shown(Place::Engine, false), "a far engine's folders are not this PC's");
    }

    /// A path that is gone opens the nearest folder above it that is there.
    #[test]
    fn a_missing_path_opens_its_nearest_existing_parent() {
        let dir = std::env::temp_dir().join(format!("koyomi-paths-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.log");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(open_target(&file), (file.clone(), false), "a file opens itself");
        assert_eq!(open_target(&dir), (dir.clone(), true), "a folder opens itself");
        assert_eq!(open_target(&dir.join("gone.log")), (dir.clone(), true));
        assert_eq!(open_target(&dir.join("gone").join("deeper").join("x.log")), (dir.clone(), true));
        assert_eq!(
            open_target(Path::new("koyomi-no-such-dir-x/a.csv")),
            (PathBuf::from("."), true),
            "a relative path falls back to the current folder"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The copied text is the plain path.
    #[test]
    fn copying_puts_the_plain_path_on_the_clipboard() {
        let path = Path::new("C:\\state\\exports\\plan-2026-11.pdf");
        copy_path(path).unwrap();
        assert_eq!(ito_clipboard::clipboard!().get_text().unwrap().as_deref(), Some("C:\\state\\exports\\plan-2026-11.pdf"));
        assert_eq!(copy_text(path), path.display().to_string());
    }

    /// A path belongs to the line it came with, and to no other.
    #[test]
    fn a_path_is_shown_only_with_its_own_result() {
        let made = Some(Made { text: "done".into(), path: PathBuf::from("a"), place: Place::Here });
        assert!(Made::for_result(&made, &Some(Ok("done".into()))).is_some());
        assert!(Made::for_result(&made, &Some(Ok("another".into()))).is_none());
        assert!(Made::for_result(&made, &Some(Err("done".into()))).is_none());
        assert!(Made::for_result(&made, &None).is_none());
    }
}
