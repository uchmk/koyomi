//! What a person is shown about a definition: the tree's rows, and what an approval asks them to look at.
//! The window and the CLI both draw from here, so they say the same thing.

use ito_a11y::State;
use ito_diff::Item;
use ito_i18n::Table;
use koyomi_core::scan::{Def, Standing};

/// The mark that goes before the standing: a symbol and a word, never colour alone.
pub fn state(s: Standing) -> State {
    match s {
        Standing::Confirmed => State::Done,
        Standing::Unconfirmed => State::Warning,
        Standing::Invalid => State::Error,
    }
}

pub fn word(t: &Table, s: Standing) -> String {
    t.tr(match s {
        Standing::Confirmed => "standing.confirmed",
        Standing::Unconfirmed => "standing.unconfirmed",
        Standing::Invalid => "standing.invalid",
    })
    .to_owned()
}

/// `▲ 月次締め  未確認`: one row of the tree.
pub fn row(t: &Table, d: &Def) -> String {
    format!("{} {}  {}", state(d.standing).mark(), d.label(), word(t, d.standing))
}

/// The tree as text: a line per group, then its definitions indented. `koyomi list` prints this.
pub fn tree(t: &Table, defs: &[Def]) -> Vec<String> {
    let mut out = Vec::new();
    let mut group: Option<&str> = None;
    for d in defs {
        if group != Some(d.group.as_str()) {
            group = Some(&d.group);
            out.push(d.group.clone());
        }
        out.push(format!("  {}", row(t, d)));
    }
    out
}

/// What the definition would run: per job its id, the executable, each argument on its own line, and the directory.
pub fn body(t: &Table, d: &Def) -> Vec<String> {
    let mut out = Vec::new();
    let Some(net) = &d.net else { return out };
    for j in &net.jobs {
        let name = j.name.as_deref().filter(|n| !n.is_empty()).map(|n| format!(" ({n})")).unwrap_or_default();
        out.push(format!("{} {}{name}", t.tr("review.job"), j.id));
        out.push(format!("  {}: {}", t.tr("review.exe"), j.run.first().map(String::as_str).unwrap_or("")));
        for (i, a) in j.run.iter().skip(1).enumerate() {
            out.push(format!("  {} {}: {a}", t.tr("review.arg"), i + 1));
        }
        out.push(format!("  {}: {}", t.tr("review.dir"), j.dir.as_deref().unwrap_or(t.tr("review.nodir"))));
    }
    out
}

/// The change from the last approved text, when there is one.
pub fn diff(d: &Def) -> Vec<Item> {
    match &d.previous {
        Some(old) => ito_diff::diff(old, &d.text, 3),
        None => Vec::new(),
    }
}

/// A diff as text lines: `+`, `-` or a space, then the line.
pub fn diff_text(items: &[Item]) -> Vec<String> {
    items
        .iter()
        .map(|i| match i {
            Item::Line(l) => format!("{}{}", l.kind.mark(), l.text),
            Item::Folded(n) => format!("… {n}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use koyomi_core::approved::Store;
    use koyomi_core::scan::read_def;

    fn table() -> Table {
        crate::i18n::table("ja").0
    }

    fn def(text: &str, store: &Store) -> Def {
        let dir = std::env::temp_dir().join(format!("koyomi-review-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("月次締め.toml");
        std::fs::write(&p, text).unwrap();
        read_def(&p, "経理", "月次締め", store)
    }

    const NET: &str =
        "format = 1\nname = \"月次締め\"\n[[job]]\nid = \"a\"\nrun = [\"C:/bin/x.exe\", \"--month\", \"2026-10\"]\ndir = \"C:/work\"\n";

    /// the row has a symbol and a word.
    #[test]
    fn a_row_has_a_mark_and_a_word() {
        let d = def(NET, &Store::default());
        assert_eq!(row(&table(), &d), "▲ 月次締め  未確認");
        assert_eq!(tree(&table(), &[d]), ["経理", "  ▲ 月次締め  未確認"]);
    }

    /// the exe, each argument on its own line, and the directory.
    #[test]
    fn the_body_shows_exe_args_one_per_line_and_dir() {
        let d = def(NET, &Store::default());
        let b = body(&table(), &d);
        assert!(b.iter().any(|l| l.contains("C:/bin/x.exe")));
        assert!(b.iter().any(|l| l.ends_with(": --month")));
        assert!(b.iter().any(|l| l.ends_with(": 2026-10")));
        assert!(b.iter().any(|l| l.ends_with(": C:/work")));
    }

    /// a re-approval shows what changed since the approved text.
    #[test]
    fn a_changed_file_shows_a_diff_against_the_approved_text() {
        let mut store = Store::default();
        let first = def(NET, &store);
        store.approve(&first.id, &first.sha256, &first.text);
        let d = def(&NET.replace("2026-10", "2026-11"), &store);
        let lines = diff_text(&diff(&d));
        assert!(lines.iter().any(|l| l.starts_with('-') && l.contains("2026-10")), "{lines:?}");
        assert!(lines.iter().any(|l| l.starts_with('+') && l.contains("2026-11")), "{lines:?}");
        assert!(diff(&first).is_empty(), "nothing to compare a first approval with");
    }
}
