//! What a person is shown about a definition: the tree's rows, and what an approval asks them to look at.
//! The window and the CLI both draw from here, so they say the same thing.

use chrono::{Days, NaiveDateTime};
use ito_a11y::State;
use ito_diff::Item;
use ito_i18n::Table;
use koyomi_core::calendar::Calendar;
use koyomi_core::jobnet::{self, Part, Wait};
use koyomi_core::run::State as RunState;
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

/// `▲ 月次締め  未確認`: one row of the tree. A confirmed net shows only its name (`月次締め`);
/// `⏸ 月次締め  保留` when `held`; an unconfirmed or invalid one keeps its mark, and gains 保留
/// when held.
pub fn row(t: &Table, d: &Def, held: bool) -> String {
    row_with(t, d, held, None)
}

/// [`row`] with the state of the net's latest run today at the end (`月次締め  ✓`), so
/// the tree says how it went without opening it.
pub fn row_with(t: &Table, d: &Def, held: bool, last: Option<RunState>) -> String {
    let confirmed = d.standing == Standing::Confirmed;
    // A confirmed net is the usual case, so it carries no mark and no word of its own: the last
    // run's mark at the end is then the only check on the row.
    let (mark, word) = match (confirmed, held) {
        (true, false) => (None, None),
        (true, true) => (Some(RunState::Held.mark()), Some(t.tr("state.held").to_owned())),
        (false, false) => (Some(state(d.standing).mark()), Some(word(t, d.standing))),
        (false, true) => (Some(state(d.standing).mark()), Some(format!("{}・{}", word(t, d.standing), t.tr("state.held")))),
    };
    let mut out = String::new();
    if let Some(m) = mark {
        out.push_str(&format!("{m} "));
    }
    out.push_str(d.label());
    if let Some(w) = word {
        out.push_str(&format!("  {w}"));
    }
    if let Some(l) = last {
        out.push_str(&format!("  {}", l.mark()));
    }
    out
}

/// The ids of the definitions whose label another one in the same group shares: the tree adds
/// the file name to these (see [`twin_row`]) so two rows are never alike.
pub fn twins(defs: &[Def]) -> std::collections::HashSet<String> {
    let mut seen: std::collections::HashMap<(&str, &str), Vec<&str>> = std::collections::HashMap::new();
    for d in defs {
        seen.entry((d.group.as_str(), d.label())).or_default().push(&d.id);
    }
    seen.into_values().filter(|v| v.len() > 1).flatten().map(str::to_owned).collect()
}

/// `row` with the file name after it (`ab  · ab-2`), for a definition in [`twins`].
pub fn twin_row(row: String, d: &Def) -> String {
    format!("{row}  · {}", d.name)
}

/// The tree as text: a line per group, then its definitions indented. `koyomi list` prints this.
pub fn tree(t: &Table, defs: &[Def], held: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut group: Option<&str> = None;
    let twin = twins(defs);
    for d in defs {
        if group != Some(d.group.as_str()) {
            group = Some(&d.group);
            out.push(d.group.clone());
        }
        let r = row(t, d, held.contains(&d.id));
        out.push(format!("  {}", if twin.contains(&d.id) { twin_row(r, d) } else { r }));
    }
    out
}

/// What the definition would run: per job its id, the executable, each argument on its own line, and the directory.
pub fn body(t: &Table, d: &Def) -> Vec<String> {
    let mut out = settings(t, d);
    out.extend(jobs(t, d));
    out
}

/// The net's own settings, one line each; nothing for the default ones.
pub fn settings(t: &Table, d: &Def) -> Vec<String> {
    let mut out = Vec::new();
    let Some(net) = &d.net else { return out };
    if net.missed == koyomi_core::jobnet::Missed::Once {
        out.push(t.tr("review.missed_once").to_owned());
    }
    if net.overlap == koyomi_core::jobnet::Overlap::Queue {
        out.push(t.tr("review.overlap_queue").to_owned());
    }
    if net.wake {
        out.push(t.tr("review.wake").to_owned());
    }
    if let Some(d) = net.late_start {
        out.push(t.trf("review.late_start", &[&d.text()]));
    }
    if let Some(d) = net.late_end {
        out.push(t.trf("review.late_end", &[&d.text()]));
    }
    if !net.watches_slow() {
        out.push(t.tr("review.late_slow_off").to_owned());
    }
    match net.slot_priority().cmp(&koyomi_core::jobnet::DEFAULT_PRIORITY) {
        std::cmp::Ordering::Greater => out.push(t.trf("review.priority_high", &[&net.slot_priority().to_string()])),
        std::cmp::Ordering::Less => out.push(t.trf("review.priority_low", &[&net.slot_priority().to_string()])),
        std::cmp::Ordering::Equal => {}
    }
    if !net.notify_to.is_empty() {
        out.push(t.trf("review.notify_to", &[&net.notify_to.join(", ")]));
    }
    if !net.quiet.is_empty() {
        let spans = net.quiet.iter().map(|s| s.text()).collect::<Vec<_>>().join(", ");
        let key = match net.on_quiet {
            koyomi_core::quiet::OnQuiet::Wait => "review.quiet_wait",
            koyomi_core::quiet::OnQuiet::Skip => "review.quiet_skip",
        };
        out.push(t.trf(key, &[&spans]));
    }
    out
}

/// [`jobs`] without what the jobs table already shows: only the jobs with more to say, each under its own
/// header line, and no `after` lines.
pub fn job_details(t: &Table, d: &Def) -> Vec<String> {
    let after = format!("  {}: ", t.tr("review.after"));
    let mut out: Vec<String> = Vec::new();
    let mut header: Option<String> = None;
    for l in jobs(t, d) {
        if !l.starts_with(' ') {
            header = Some(l);
        } else if !l.starts_with(&after) {
            out.extend(header.take());
            out.push(l);
        }
    }
    out
}

/// What each job would run (see [`body`]).
pub fn jobs(t: &Table, d: &Def) -> Vec<String> {
    let mut out = Vec::new();
    let Some(net) = &d.net else { return out };
    for j in &net.jobs {
        let name = j.name.as_deref().filter(|n| !n.is_empty()).map(|n| format!(" ({n})")).unwrap_or_default();
        out.push(format!("{} {}{name}", t.tr(&format!("part.{}", j.part.key())), j.id));
        for l in &j.after {
            out.push(format!("  {}: {}", t.tr("review.after"), l.text()));
        }
        match &j.part {
            Part::Job => {}
            Part::Loop { file, count } => {
                if let Some(f) = file {
                    out.push(format!("  {}: {f}", t.tr("review.loop_files")));
                }
                if let Some(n) = count {
                    out.push(format!("  {}: {}", t.tr("review.loop_count"), t.trf("review.loop_times", &[&n.to_string()])));
                }
            }
            Part::Branch | Part::Notify | Part::Approval => continue,
            Part::Join { wait } => {
                let how = t.tr(if *wait == Wait::All { "review.wait_all" } else { "review.wait_any" });
                out.push(format!("  {}: {how}", t.tr("review.wait")));
                continue;
            }
            Part::Time { at, days } => {
                let later = if *days > 0 { t.trf("review.days", &[&days.to_string()]) } else { String::new() };
                out.push(format!("  {}: {}{later}", t.tr("review.at"), at.format("%H:%M")));
                continue;
            }
            Part::File { file, settle } => {
                out.push(format!("  {}: {file}", t.tr("review.file_wait")));
                if let Some(s) = settle {
                    out.push(format!("  {}: {}", t.tr("review.settle"), koyomi_core::duration::text(*s)));
                }
                if let Some(secs) = j.timeout {
                    out.push(format!("  {}: {}", t.tr("review.timeout"), koyomi_core::duration::text(secs)));
                }
                continue;
            }
            Part::Jobnet { use_ } => {
                out.push(format!("  {}: {use_}", t.tr("review.use")));
                continue;
            }
        }
        if let (Some(shell), Some(script)) = (&j.shell, &j.script) {
            out.push(format!("  {}: {shell}", t.tr("review.shell")));
            for (i, line) in script.lines().enumerate() {
                let head = if i == 0 { format!("  {}: ", t.tr("review.script")) } else { "    ".to_owned() };
                out.push(format!("{head}{line}"));
            }
        } else {
            out.push(format!("  {}: {}", t.tr("review.exe"), j.run.first().map(String::as_str).unwrap_or("")));
            for (i, a) in j.run.iter().skip(1).enumerate() {
                out.push(format!("  {} {}: {a}", t.tr("review.arg"), i + 1));
            }
        }
        out.push(format!("  {}: {}", t.tr("review.dir"), j.dir.as_deref().unwrap_or(t.tr("review.nodir"))));
        for (k, v) in &j.env {
            out.push(format!("  {} {k}: {v}", t.tr("review.env")));
        }
        if !j.encoding.is_auto() {
            let key = format!("field.encoding_{}", j.encoding.text());
            out.push(format!("  {}: {}", t.tr("field.encoding"), t.tr(&key)));
        }
        if let Some(secs) = j.timeout {
            out.push(format!("  {}: {}", t.tr("review.timeout"), koyomi_core::duration::text(secs)));
        }
        if let Some(r) = j.retry {
            let text = t.trf("review.retry_text", &[&r.count.to_string(), &koyomi_core::duration::text(r.wait)]);
            out.push(format!("  {}: {text}", t.tr("review.retry")));
        }
        if let Some(g) = &j.judge {
            let mut parts = Vec::new();
            if !g.warn.is_empty() {
                parts.push(t.trf("review.judge_warn", &[&jobnet::Judge::list(&g.warn)]));
            }
            if !g.error.is_empty() {
                parts.push(t.trf("review.judge_error", &[&jobnet::Judge::list(&g.error)]));
            }
            if let Some(p) = &g.error_output {
                parts.push(t.trf("review.judge_output", &[p]));
            }
            if let Some(p) = &g.ok_file {
                parts.push(t.trf("review.judge_file", &[p]));
            }
            parts.push(t.tr("review.judge_rest").to_owned());
            out.push(format!("  {}: {}", t.tr("review.judge"), parts.join("、")));
        }
    }
    out
}

/// How the next 7 days from `now` change with this approval: the count of
/// times before and after, then each time that is added (`+`) or goes away
/// (`-`), in order, `WEEK_ROWS` at most and the rest as a count. A first
/// approval lists the times as they will be. A writing mistake in `[[when]]`
/// (every minute, the month's end, a holiday) shows here as a number before
/// anything runs. Empty when neither version has a `[[when]]`. `cal` is the
/// calendar the net counts by (both versions are counted by it).
pub fn week(t: &Table, d: &Def, cal: Option<&Calendar>, now: NaiveDateTime) -> Vec<String> {
    let Some(net) = &d.net else { return Vec::new() };
    let until = now.checked_add_days(Days::new(7)).unwrap_or(now);
    let new = when::between_in(&net.when, now, until, cal);
    let old: Option<Vec<NaiveDateTime>> = d.previous.as_deref().map(|text| match jobnet::parse(text) {
        Ok(net) => when::between_in(&net.when, now, until, cal),
        Err(_) => Vec::new(),
    });
    let mut out: Vec<String> = net.when.iter().filter_map(|w| crate::show::event_row(t, w)).collect();
    if new.is_empty() && old.as_ref().is_none_or(|o| o.is_empty()) {
        if !out.is_empty() || (net.when.is_empty() && d.previous.is_none()) {
            return out;
        }
        return vec![t.tr("review.week_none").to_owned()];
    }
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

    /// The jobs table shows id, kind, name and `after`; the lines under it must not repeat them.
    #[test]
    fn job_details_leave_out_what_the_table_shows() {
        let store = Store::default();
        let two = format!(
            "{NET}[[job]]
id = \"b\"
run = [\"C:/bin/y.exe\"]
after = [\"a\"]
"
        );
        let d = def(&two, &store);
        let t = table();
        let all = jobs(&t, &d);
        let det = job_details(&t, &d);
        let after = format!("{}: ", t.tr("review.after"));
        assert!(all.iter().any(|l| l.contains(&after)));
        assert!(!det.iter().any(|l| l.contains(&after)));
        assert!(det.len() < all.len());
        assert!(det.iter().any(|l| l.contains("C:/bin/x.exe")));
    }

    /// the row has a symbol and a word.
    #[test]
    fn a_row_has_a_mark_and_a_word() {
        let d = def(NET, &Store::default());
        assert_eq!(row(&table(), &d, false), "▲ 月次締め  未確認");
        assert_eq!(row(&table(), &d, true), "▲ 月次締め  未確認・保留");
        assert_eq!(row_with(&table(), &d, false, Some(RunState::Ok)), format!("▲ 月次締め  未確認  {}", RunState::Ok.mark()));
        assert_eq!(tree(&table(), std::slice::from_ref(&d), &[]), ["経理", "  ▲ 月次締め  未確認"]);
        let mut store = Store::default();
        store.approve("経理/月次締め", &koyomi_core::approved::hash(NET.as_bytes()), NET);
        let d = def(NET, &store);
        assert_eq!(row(&table(), &d, false), "月次締め");
        assert_eq!(row(&table(), &d, true), "⏸ 月次締め  保留");
        assert_eq!(row_with(&table(), &d, false, Some(RunState::Ok)), format!("月次締め  {}", RunState::Ok.mark()));
        assert_eq!(tree(&table(), &[d], &["経理/月次締め".to_owned()]), ["経理", "  ⏸ 月次締め  保留"]);
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
        assert!(!body(&table(), &d).iter().any(|l| l.contains("やり直し")));
        let d = def(&format!("{NET}retry = {{ count = 2, wait = \"5m\" }}\n"), &Store::default());
        assert!(body(&table(), &d).iter().any(|l| l.ends_with("やり直し: 2 回まで、5m おいて")), "{:?}", body(&table(), &d));
    }

    /// a net that takes a free slot before the others, or gives way, says so; the usual 3 says nothing.
    #[test]
    fn a_priority_other_than_the_usual_is_shown() {
        let with = |p: &str| {
            def(
                &NET.replacen(
                    "[[job]]",
                    &format!(
                        "priority = {p}
[[job]]"
                    ),
                    1,
                ),
                &Store::default(),
            )
        };
        assert!(body(&table(), &with("5")).iter().any(|l| l == "空きの枠を先に取る（priority = 5）"));
        assert!(body(&table(), &with("1")).iter().any(|l| l == "空きの枠を後に回す（priority = 1）"));
        assert!(!body(&table(), &with("3")).iter().any(|l| l.contains("priority")));
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
        let w = week(&table(), &first, None, now);
        assert_eq!(w[0], "この先 7 日の予定: 7 回");
        assert_eq!(w[1], "  2026-10-31（土）02:00");
        assert_eq!(w.len(), 8, "{w:?}");
        store.approve(&first.id, &first.sha256, &first.text);
        let d = def(QUARTER, &store);
        let w = week(&table(), &d, None, now);
        assert_eq!(w[0], "7 日で 7 回 → 672 回（+665）");
        assert!(w[1].starts_with("+ 2026-10-30（金）09:15"), "{w:?}");
        assert!(!w.iter().any(|l| l.contains("02:00")), "2:00 stays, so it is not listed: {w:?}");
        assert_eq!(w.len(), 2 + WEEK_ROWS, "{w:?}");
        assert_eq!(w[w.len() - 1], "… 他 645 件");
        // Taking times away lists what goes.
        let second = def(QUARTER, &store);
        store.approve(&second.id, &second.sha256, &second.text);
        let d = def(DAILY, &store);
        let w = week(&table(), &d, None, now);
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
        assert_eq!(week(&table(), &d, None, now), vec!["7 日で 7 回 → 6 回（-1）", "- 2026-11-01（日）02:00"]);
        let d = def(NET, &Store::default());
        assert!(week(&table(), &d, None, now).is_empty(), "no [[when]] and nothing before: nothing to say");
        store.approve(&d.id, &d.sha256, &d.text);
        let d = def(&NET.replace("2026-10", "2026-11"), &store);
        assert_eq!(week(&table(), &d, None, now), vec!["7 日の予定はありません"], "a change that leaves no time says so");
        // With a calendar the weekend is not counted: the week has 5 times, not 7.
        let cal = Calendar::parse("会社", "format = 1\nweekly_off = [\"sat\", \"sun\"]\n").unwrap();
        let d = def(DAILY, &Store::default());
        let w = week(&table(), &d, Some(&cal), now);
        assert_eq!(w[0], "この先 7 日の予定: 5 回");
        assert_eq!(w[1], "  2026-11-02（月）02:00");
    }

    #[test]
    fn two_rows_with_one_label_in_a_group_get_the_file_name() {
        let text = "format = 1
name = \"ab\"
[[job]]
id = \"a\"
run = [\"x.exe\"]
";
        let mut a = def(text, &Store::default());
        (a.id, a.group, a.name) = ("g/ab".to_owned(), "g".to_owned(), "ab".to_owned());
        let mut b = a.clone();
        (b.id, b.name) = ("g/ab-2".to_owned(), "ab-2".to_owned());
        let both = twins(&[a.clone(), b.clone()]);
        assert!(both.contains("g/ab") && both.contains("g/ab-2"));
        assert_eq!(twin_row("ab".to_owned(), &b), "ab  · ab-2");
        assert!(twins(&[a]).is_empty());
    }
}
