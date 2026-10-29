//! What a person is shown about a definition: the tree's rows, and what an approval asks them to look at.
//! The window and the CLI both draw from here, so they say the same thing.

use chrono::{Days, NaiveDateTime};
use ito_a11y::State;
use ito_diff::Item;
use ito_i18n::Table;
use koyomi_core::jobnet;
use koyomi_core::scan::{Def, Standing};
use koyomi_core::when;

/// How many of the week's times are listed; the rest is a count.
pub const WEEK_ROWS: usize = 20;

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
        if let Some(secs) = j.timeout {
            out.push(format!("  {}: {}", t.tr("review.timeout"), koyomi_core::duration::text(secs)));
        }
    }
    out
}

/// How the next 7 days from `now` change with this approval: the count of
/// times before and after, then each time that is added (`+`) or goes away
/// (`-`), in order, `WEEK_ROWS` at most and the rest as a count. A first
/// approval lists the times as they will be. A writing mistake in `[[when]]`
/// (every minute, the month's end, a holiday) shows here as a number before
/// anything runs. Empty when neither version has a `[[when]]`.
pub fn week(t: &Table, d: &Def, now: NaiveDateTime) -> Vec<String> {
    let Some(net) = &d.net else { return Vec::new() };
    let until = now.checked_add_days(Days::new(7)).unwrap_or(now);
    let new = when::between(&net.when, now, until);
    let old: Option<Vec<NaiveDateTime>> = d.previous.as_deref().map(|text| match jobnet::parse(text) {
        Ok(net) => when::between(&net.when, now, until),
        Err(_) => Vec::new(),
    });
    if new.is_empty() && old.as_ref().is_none_or(|o| o.is_empty()) {
        return if net.when.is_empty() && d.previous.is_none() { Vec::new() } else { vec![t.tr("review.week_none").to_owned()] };
    }
    let mut out = Vec::new();
    // (time, mark): what is listed. Without a previous version, every time, unmarked.
    let rows: Vec<(NaiveDateTime, &str)> = match &old {
        None => {
            out.push(t.trf("review.week_count", &[&new.len().to_string()]));
            new.iter().map(|&at| (at, "  ")).collect()
        }
        Some(old) => {
            let delta = new.len() as i64 - old.len() as i64;
            let sign = if delta > 0 {
                format!("+{delta}")
            } else if delta < 0 {
                delta.to_string()
            } else {
                "±0".to_owned()
            };
            out.push(t.trf("review.week_change", &[&old.len().to_string(), &new.len().to_string(), &sign]));
            let mut rows: Vec<(NaiveDateTime, &str)> = new.iter().filter(|at| !old.contains(at)).map(|&at| (at, "+ ")).collect();
            rows.extend(old.iter().filter(|at| !new.contains(at)).map(|&at| (at, "- ")));
            rows.sort();
            rows
        }
    };
    for (at, mark) in rows.iter().take(WEEK_ROWS) {
        out.push(format!("{mark}{}", crate::show::when_row(t, *at)));
    }
    if rows.len() > WEEK_ROWS {
        out.push(t.trf("review.week_more", &[&(rows.len() - WEEK_ROWS).to_string()]));
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

    /// Each call writes its own file: the tests run side by side.
    fn def(text: &str, store: &Store) -> Def {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("koyomi-review-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("月次締め.toml");
        std::fs::write(&p, text).unwrap();
        read_def(&p, "経理", "月次締め", store)
    }

    const NET: &str =
        "format = 1\nname = \"月次締め\"\n[[job]]\nid = \"a\"\nrun = [\"C:/bin/x.exe\", \"--month\", \"2026-10\"]\ndir = \"C:/work\"\ntimeout = \"30m\"\n";

    /// the row has a symbol and a word.
    #[test]
    fn a_row_has_a_mark_and_a_word() {
        let d = def(NET, &Store::default());
        assert_eq!(row(&table(), &d), "▲ 月次締め  未確認");
        assert_eq!(tree(&table(), &[d]), ["経理", "  ▲ 月次締め  未確認"]);
    }

    /// the exe, each argument on its own line, the directory and the timeout.
    #[test]
    fn the_body_shows_exe_args_one_per_line_dir_and_timeout() {
        let d = def(NET, &Store::default());
        let b = body(&table(), &d);
        assert!(b.iter().any(|l| l.contains("C:/bin/x.exe")));
        assert!(b.iter().any(|l| l.ends_with(": --month")));
        assert!(b.iter().any(|l| l.ends_with(": 2026-10")));
        assert!(b.iter().any(|l| l.ends_with(": C:/work")));
        assert!(b.iter().any(|l| l.ends_with("打ち切り: 30m")), "{b:?}");
        let d = def(&NET.replace("timeout = \"30m\"\n", ""), &Store::default());
        assert!(!body(&table(), &d).iter().any(|l| l.contains("打ち切り")));
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

    const DAILY: &str = "format = 1
name = \"月次締め\"
[[when]]
every = \"day\"
at = [\"02:00\"]
[[job]]
id = \"a\"
run = [\"x\"]
";
    const QUARTER: &str = "format = 1
name = \"月次締め\"
[[when]]
every = \"minutes\"
n = 15
from = \"00:00\"
to = \"23:59\"
[[job]]
id = \"a\"
run = [\"x\"]
";

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    /// a first approval lists the week as it will be; a change shows the counts and what differs.
    #[test]
    fn the_week_counts_the_times_and_lists_what_is_added_or_gone() {
        let now = at("2026-10-30 09:00:00");
        let mut store = Store::default();
        let first = def(DAILY, &store);
        let w = week(&table(), &first, now);
        assert_eq!(w[0], "この先 7 日の予定: 7 回");
        assert_eq!(w[1], "  2026-10-31（土）02:00");
        assert_eq!(w.len(), 8, "{w:?}");
        store.approve(&first.id, &first.sha256, &first.text);
        let d = def(QUARTER, &store);
        let w = week(&table(), &d, now);
        assert_eq!(w[0], "7 日で 7 回 → 672 回（+665）");
        assert!(w[1].starts_with("+ 2026-10-30（金）09:15"), "{w:?}");
        assert!(!w.iter().any(|l| l.contains("02:00")), "2:00 stays, so it is not listed: {w:?}");
        assert_eq!(w.len(), 2 + WEEK_ROWS, "{w:?}");
        assert_eq!(w[w.len() - 1], "… 他 645 件");
        // Taking times away lists what goes.
        let second = def(QUARTER, &store);
        store.approve(&second.id, &second.sha256, &second.text);
        let d = def(DAILY, &store);
        let w = week(&table(), &d, now);
        assert_eq!(w[0], "7 日で 672 回 → 7 回（-665）");
        assert!(w[1].starts_with("- 2026-10-30（金）09:15"), "{w:?}");
        store.approve(&first.id, &first.sha256, &first.text);
        let d = def(
            &DAILY.replace(
                "at = [\"02:00\"]",
                "at = [\"02:00\"]
except = [\"2026-11-01\"]",
            ),
            &store,
        );
        assert_eq!(week(&table(), &d, now), vec!["7 日で 7 回 → 6 回（-1）", "- 2026-11-01（日）02:00"]);
        let d = def(NET, &Store::default());
        assert!(week(&table(), &d, now).is_empty(), "no [[when]] and nothing before: nothing to say");
        store.approve(&d.id, &d.sha256, &d.text);
        let d = def(&NET.replace("2026-10", "2026-11"), &store);
        assert_eq!(week(&table(), &d, now), vec!["7 日の予定はありません"], "a change that leaves no time says so");
    }
}
