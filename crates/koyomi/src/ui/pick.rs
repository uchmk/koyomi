//! Fields that used to be typed and can be picked instead: a net from the engine's list, the weekdays, a
//! number out of a short range, and a path from a file chooser. Typing stays for what a list cannot hold
//! (a glob, `{date}`, a far engine's path, a net the list does not know): a text that does not read as one of the
//! choices keeps its typed field, and the picker comes back as soon as the text reads.
//!
//! Everything that decides something is a plain function here (the text of a field in, the text out), so it is
//! tested without a window. Writing happens only when the person picks.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use ito_a11y::Role;
use ito_i18n::Table;
use koyomi_core::scan::Def;
use koyomi_core::when;

/// One net of the engine, as a list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetChoice {
    /// `経理/締め`: what the field holds.
    pub id: String,
    /// What the list shows: the id, then the net's title when it has one.
    pub label: String,
}

/// The nets of `defs` as choices, in the order the net list sorts them (by group, then name).
pub fn net_choices(defs: &[Def]) -> Vec<NetChoice> {
    let mut sorted: Vec<&Def> = defs.iter().collect();
    sorted.sort_by(|a, b| (&a.group, &a.name).cmp(&(&b.group, &b.name)));
    sorted
        .into_iter()
        .map(|d| match d.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            Some(title) => NetChoice { id: d.id.clone(), label: format!("{}  {title}", d.id) },
            None => NetChoice { id: d.id.clone(), label: d.id.clone() },
        })
        .collect()
}

/// The words a net list needs besides the nets.
pub struct NetWords<'a> {
    /// The choice that writes nothing (an optional field).
    pub none: &'a str,
    /// What an empty required field shows.
    pub choose: &'a str,
    /// Put after a value the list does not hold.
    pub missing: &'a str,
    /// Put after a value that names the net being edited.
    pub own: &'a str,
}

/// The entries of a net list as (value written, words shown), and which one is picked now.
///
/// The net being edited (`own`) is not offered. A value the list does not hold (a net that is gone, or one a
/// narrowed user cannot see) stays as the first choice, as written, with `missing` after it.
pub fn net_entries(nets: &[NetChoice], own: &str, value: &str, optional: bool, w: &NetWords) -> (Vec<(String, String)>, usize) {
    let mut out: Vec<(String, String)> = Vec::new();
    let shown = value.trim();
    if optional {
        out.push((String::new(), w.none.to_owned()));
    } else if shown.is_empty() {
        out.push((String::new(), w.choose.to_owned()));
    }
    let held = nets.iter().any(|n| n.id == shown && n.id != own);
    if !shown.is_empty() && !held {
        let suffix = if shown == own { w.own } else { w.missing };
        out.push((value.to_owned(), format!("{value}{suffix}")));
    }
    out.extend(nets.iter().filter(|n| n.id != own).map(|n| (n.id.clone(), n.label.clone())));
    let now = if shown.is_empty() {
        0
    } else if held {
        out.iter().position(|(v, _)| v == shown).unwrap_or(0)
    } else {
        out.iter().position(|(v, _)| v == value).unwrap_or(0)
    };
    (out, now)
}

/// The weekdays as the `[[when]]` writes them.
pub const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// The days a `days` text names, as seven switches in week order; `None` when some word is not a weekday.
/// An empty text names none.
pub fn days_set(text: &str) -> Option<[bool; 7]> {
    let mut set = [false; 7];
    for word in text.split([',', ' ', '\n']).map(str::trim).filter(|s| !s.is_empty()) {
        let day = when::weekday(word).ok()?;
        set[day.num_days_from_monday() as usize] = true;
    }
    Some(set)
}

/// The text of a set of days: week order, `mon, tue` (what the `[[when]]` writes). No day: empty.
pub fn days_text(set: &[bool; 7]) -> String {
    DAYS.iter().zip(set).filter(|(_, on)| **on).map(|(d, _)| *d).collect::<Vec<_>>().join(", ")
}

/// `set` with day `i` turned the other way.
pub fn days_toggle(set: &[bool; 7], i: usize) -> [bool; 7] {
    let mut out = *set;
    out[i] = !out[i];
    out
}

/// Every day: nothing named (what an empty `days` of `minutes` and `hours` means).
pub fn every_day(set: &[bool; 7]) -> bool {
    set.iter().all(|on| !on)
}

/// How many days month `month` (1 to 12) can have; February has 29 (`02-29` is a date).
pub fn days_in_month(month: u32) -> u32 {
    match month {
        2 => 29,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `04-01` as (month, day), when it reads.
pub fn month_day_parts(text: &str) -> Option<(u32, u32)> {
    when::month_day(text).ok()
}

/// `MM-DD`, with the day kept inside the month.
pub fn month_day_text(month: u32, day: u32) -> String {
    format!("{month:02}-{:02}", day.min(days_in_month(month)).max(1))
}

/// A whole number inside `lo..=hi` that the text is, when it is one.
pub fn number_in(text: &str, lo: u32, hi: u32) -> Option<u32> {
    text.trim().parse::<u32>().ok().filter(|n| (lo..=hi).contains(n))
}

/// The weekday (0 for Monday) a text names, when it names one.
pub fn weekday_index(text: &str) -> Option<usize> {
    when::weekday(text).ok().map(|d| d.num_days_from_monday() as usize)
}

/// The patterns of a user's nets field (`経理/*`, ids; space or comma separated).
fn net_patterns(text: &str) -> Vec<&str> {
    text.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).collect()
}

/// `field` with `add` after what is there, space separated; nothing is added twice.
pub fn net_append(field: &str, add: &str) -> String {
    if net_patterns(field).contains(&add) {
        return field.to_owned();
    }
    let kept = field.trim_end();
    if kept.is_empty() {
        add.to_owned()
    } else {
        format!("{kept} {add}")
    }
}

/// The choices of the users page's picker after 「すべて」: each group as `<group>/*`, then each id.
pub fn net_patterns_offered(ids: &[String]) -> Vec<String> {
    let mut groups: Vec<String> = ids.iter().filter_map(|id| id.split_once('/')).map(|(g, _)| format!("{g}/*")).collect();
    groups.sort();
    groups.dedup();
    groups.extend(ids.iter().cloned());
    groups
}

/// What a chooser is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    /// One folder.
    Folder,
    /// One file that is there to read (or a folder or a glob, which the person types).
    File,
    /// A file to write.
    Save,
    /// Files to attach, one path a line.
    Files,
}

/// The folder a chooser starts in: the one the field names when it is there (for a file, the folder the file is in).
/// A relative path or one with a glob or a `{date}` in it starts nowhere in particular.
pub fn start_dir(text: &str, want: Want) -> Option<PathBuf> {
    // The attachments are one a line: the last one is what the person was at.
    let line = if want == Want::Files { text.lines().map(str::trim).rfind(|l| !l.is_empty())? } else { text.trim() };
    if line.is_empty() || line.contains(['*', '?', '{']) {
        return None;
    }
    let path = Path::new(line);
    if !path.is_absolute() {
        return None;
    }
    if path.is_dir() {
        return Some(path.to_path_buf());
    }
    path.parent().filter(|p| p.is_dir()).map(Path::to_path_buf)
}

/// The text a chooser's answer makes: one path replaces the field; files are added a line each after what is there.
pub fn browsed_text(now: &str, want: Want, paths: &[PathBuf]) -> Option<String> {
    let first = paths.first()?;
    if want != Want::Files {
        return Some(first.display().to_string());
    }
    let mut out = now.trim_end_matches(['\r', '\n']).to_owned();
    for p in paths {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&p.display().to_string());
    }
    Some(out)
}

/// The file chooser that is open (one for the whole window) and the field it was opened for.
#[derive(Default)]
pub struct Browse {
    open: Option<Open>,
}

struct Open {
    /// The part (or the net) and the field.
    owner: String,
    rx: Receiver<Vec<PathBuf>>,
    /// What the person chose, until the field it was for takes it.
    answer: Option<Vec<PathBuf>>,
}

impl Browse {
    /// A chooser that stays open, for a test.
    #[cfg(test)]
    pub fn pending(owner: &str) -> Browse {
        let (tx, rx) = std::sync::mpsc::channel();
        std::mem::forget(tx);
        Browse { open: Some(Open { owner: owner.to_owned(), rx, answer: None }) }
    }

    /// Whether a chooser is open.
    pub fn busy(&self) -> bool {
        self.open.is_some()
    }

    /// Looks for the answer of the open chooser; asks for another frame while it is still open.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(open) = &mut self.open else { return };
        if open.answer.is_some() {
            return;
        }
        match open.rx.try_recv() {
            Ok(paths) if paths.is_empty() => self.open = None,
            Ok(paths) => open.answer = Some(paths),
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(TryRecvError::Disconnected) => self.open = None,
        }
    }

    /// The answer, when it is for `owner`.
    fn take(&mut self, owner: &str) -> Option<Vec<PathBuf>> {
        let open = self.open.as_mut().filter(|o| o.owner == owner)?;
        let answer = open.answer.take()?;
        self.open = None;
        Some(answer)
    }

    /// An answer no field took (its job or its `[[when]]` is not the one open any more) is dropped.
    pub fn drop_answer(&mut self) {
        if self.open.as_ref().is_some_and(|o| o.answer.is_some()) {
            self.open = None;
        }
    }

    fn start(&mut self, owner: String, want: Want, title: String, dir: Option<PathBuf>) {
        let rx = ito_dialog::spawn(move || {
            let mut d = ito_dialog::Dialog::new().title(&title);
            if let Some(dir) = dir {
                d = d.directory(dir);
            }
            match want {
                Want::Folder => d.open_folder().into_iter().collect(),
                Want::File => d.open_file().into_iter().collect(),
                Want::Save => d.save_file().into_iter().collect(),
                Want::Files => d.open_files(),
            }
        });
        self.open = Some(Open { owner, rx, answer: None });
    }
}

/// The connection names the engine's snapshot lists, for the selects of the job form and the `[[when]]` form. They
/// are passed together (one small struct) so that `drawer` and `Ctx` do not grow an argument for each list.
#[derive(Clone, Copy, Debug, Default)]
pub struct Conns<'a> {
    pub hosts: &'a [crate::proto::HostName],
    pub buckets: &'a [String],
    pub mailboxes: &'a [String],
    pub groups: &'a [crate::proto::GroupName],
    pub mails: &'a [String],
}

/// What the fields of one editor frame share: the nets, the net being edited, whether the engine is this PC, the
/// part (or the net) the pane is about, and the chooser.
pub struct Ctx<'a> {
    pub nets: &'a [NetChoice],
    pub own: String,
    pub local: bool,
    /// The names of the shells the engine has (all five while it has not said).
    shells: Vec<(String, String)>,
    /// The engine's default shell ("" until it has said).
    default: String,
    /// The `[[account]]` names and the `[[database]]`s (name and kind) the engine has registered.
    accounts: Vec<String>,
    databases: Vec<crate::proto::DatabaseName>,
    /// Whether the engine runs on Windows (`None`: it has not said).
    windows: Option<bool>,
    /// The `[[host]]`, `[[bucket]]` and `[[mailbox]]` names, the `[groups]` and the `[[notify.mails]]` the engine has.
    conns: Conns<'a>,
    /// A link asked for this settings page (taken by the drawer after the frame).
    settings: std::cell::Cell<Option<usize>>,
    scope: String,
    browse: RefCell<Browse>,
}

impl<'a> Ctx<'a> {
    pub fn new(nets: &'a [NetChoice], own: &str, local: bool, scope: &str, browse: Browse) -> Self {
        Ctx {
            nets,
            own: own.to_owned(),
            local,
            shells: Vec::new(),
            default: String::new(),
            accounts: Vec::new(),
            databases: Vec::new(),
            windows: None,
            conns: Conns::default(),
            settings: std::cell::Cell::new(None),
            scope: scope.to_owned(),
            browse: RefCell::new(browse),
        }
    }

    /// With the accounts and databases the engine has registered, and whether it runs on Windows.
    pub fn with_registry(mut self, accounts: &[String], databases: &[crate::proto::DatabaseName], windows: Option<bool>) -> Self {
        self.accounts = accounts.to_vec();
        self.databases = databases.to_vec();
        self.windows = windows;
        self
    }

    /// With the hosts, buckets, mailboxes, groups and notify mails the engine has (an engine that lacks them has none).
    pub fn with_conns(mut self, conns: Conns<'a>) -> Self {
        self.conns = conns;
        self
    }

    /// The registered hosts, buckets and mailboxes, the groups and the notify mails.
    pub fn conns(&self) -> Conns<'a> {
        self.conns
    }

    /// The registered account names.
    pub fn accounts(&self) -> &[String] {
        &self.accounts
    }

    /// The registered databases.
    pub fn databases(&self) -> &[crate::proto::DatabaseName] {
        &self.databases
    }

    /// Whether the engine runs on Windows (`None`: it has not said).
    pub fn windows(&self) -> Option<bool> {
        self.windows
    }

    /// A link of the form asks for the settings page `page`.
    pub fn ask_settings(&self, page: usize) {
        self.settings.set(Some(page));
    }

    /// The settings page a link asked for, once.
    pub fn take_settings(&self) -> Option<usize> {
        self.settings.take()
    }

    /// With the shells the engine has (none: it has not said, so every shell koyomi knows is offered).
    pub fn with_shells(mut self, found: &[crate::proto::Shell], default: &str) -> Self {
        self.shells = found.iter().map(|s| (s.name.clone(), s.path.clone())).collect();
        self.default = default.to_owned();
        self
    }

    /// The engine's default shell ("" when it has not said).
    pub fn shell_default(&self) -> &str {
        &self.default
    }

    /// Where the engine found `shell` ("" when it has not said or lacks it).
    pub fn shell_path(&self, shell: &str) -> &str {
        self.shells.iter().find(|(n, _)| n == shell).map_or("", |(_, p)| p.as_str())
    }

    /// The shells a job's `shell` may be picked from.
    pub fn shell_names(&self) -> Vec<String> {
        if self.shells.is_empty() {
            koyomi_core::shell::SHELLS.iter().map(|s| (*s).to_owned()).collect()
        } else {
            self.shells.iter().map(|(n, _)| n.clone()).collect()
        }
    }

    /// A chooser is open (anything that renumbers the `[[when]]`s waits for it).
    pub fn busy(&self) -> bool {
        self.browse.borrow().busy()
    }

    /// The chooser state, to keep for the next frame.
    pub fn into_browse(self) -> Browse {
        self.browse.into_inner()
    }

    /// A select of the nets for the field `key`; `label` is its name for a screen reader. True when one was picked.
    pub fn net_select(&self, ui: &mut egui::Ui, t: &Table, key: &str, label: &str, value: &mut String, optional: bool) -> bool {
        let words = NetWords { none: t.tr("pick.none"), choose: t.tr("pick.choose"), missing: t.tr("pick.missing"), own: t.tr("pick.own") };
        let (entries, now) = net_entries(self.nets, &self.own, value, optional, &words);
        let labels: Vec<String> = entries.iter().map(|(_, l)| l.clone()).collect();
        match select_index(ui, key, label, now, &labels) {
            Some(i) if entries[i].0 != *value => {
                value.clone_from(&entries[i].0);
                true
            }
            _ => false,
        }
    }

    /// 参照… for the field `key` (in the part this frame is about): only for an engine on this PC. The answer of the
    /// chooser lands in `text` when this field is the one it was opened for. True when `text` changed.
    pub fn browse_button(&self, ui: &mut egui::Ui, t: &Table, key: &str, name: &str, want: Want, text: &mut String) -> bool {
        if !self.local {
            return false;
        }
        let owner = format!("{}\u{1}{key}", self.scope);
        let mut changed = false;
        let answer = self.browse.borrow_mut().take(&owner);
        if let Some(new) = answer.and_then(|paths| browsed_text(text, want, &paths)) {
            if new != *text {
                *text = new;
                changed = true;
            }
        }
        let busy = self.browse.borrow().busy();
        let look = ito_prefs::Look { c: ito_theme::colors(), q: "" };
        let r = ui.add_enabled_ui(!busy, |ui| ito_prefs::button_with(ui, look, t.tr("pick.browse"), ito_prefs::Tone::Plain)).inner;
        ito_a11y::say(&r, Role::Button, &format!("{name}: {}", t.tr("pick.browse")));
        if busy {
            r.on_disabled_hover_text(t.tr("pick.browsing"));
        } else if r.on_hover_text(t.tr("pick.browse_hover")).clicked() {
            let dir = start_dir(text, want);
            self.browse.borrow_mut().start(owner, want, name.to_owned(), dir);
        }
        changed
    }
}

/// A select over `labels` (an index is the value); `label` is its name for a screen reader. The index picked, when
/// it changed.
pub fn select_index(ui: &mut egui::Ui, id: &str, label: &str, now: usize, labels: &[String]) -> Option<usize> {
    let options: Vec<(usize, &str)> = labels.iter().enumerate().map(|(i, l)| (i, l.as_str())).collect();
    let out = ui.scope(|ui| ito_prefs::select(ui, id, now, &options));
    ito_a11y::say(&out.response, Role::Label, label);
    out.inner
}

/// The seven days as switches (and 毎日 first when `every` is a word): a switch turned writes `text` in week order.
/// `text` must read (see [`days_set`]). True when one was pressed.
pub fn day_toggles(ui: &mut egui::Ui, t: &Table, label: &str, text: &mut String, every: Option<&str>) -> bool {
    let Some(set) = days_set(text) else { return false };
    let look = ito_prefs::Look { c: ito_theme::colors(), q: "" };
    let lang = t.tr("app.lang");
    let mut next = None;
    ui.horizontal_wrapped(|ui| {
        let one = |ui: &mut egui::Ui, words: &str, on: bool| -> bool {
            let tone = if on { ito_prefs::Tone::Primary } else { ito_prefs::Tone::Plain };
            let r = ito_prefs::button_with(ui, look, words, tone);
            let spoken = if on { ito_a11y::State::spoken(words, &[ito_a11y::State::Selected], lang) } else { words.to_owned() };
            ito_a11y::say(&r, Role::Button, &format!("{label}: {spoken}"));
            r.clicked()
        };
        if let Some(words) = every {
            if one(ui, words, every_day(&set)) {
                next = Some([false; 7]);
            }
        }
        for (i, day) in DAYS.iter().enumerate() {
            if one(ui, t.tr(&format!("weekday.{day}")), set[i]) {
                next = Some(days_toggle(&set, i));
            }
        }
    });
    match next {
        Some(new) if new != set || days_text(&new) != text.trim() => {
            *text = days_text(&new);
            true
        }
        _ => false,
    }
}

/// 追加 ▾ beside the users page's nets field: 「すべて」 empties it, then each group and each id is added.
pub fn net_adder(ui: &mut egui::Ui, t: &Table, id: &str, field: &mut String, ids: &[String]) {
    let words = t.tr("pick.add");
    let r = egui::ComboBox::from_id_salt(id).selected_text(words).show_ui(ui, |ui| {
        if ui.selectable_label(false, t.tr("pick.all")).clicked() {
            field.clear();
        }
        for p in net_patterns_offered(ids) {
            if ui.selectable_label(false, &p).clicked() {
                *field = net_append(field, &p);
            }
        }
    });
    ito_a11y::say(&r.response, Role::Button, &format!("{}: {words}", t.tr("users.nets_field")));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nets() -> Vec<NetChoice> {
        let def = |group: &str, name: &str, title: Option<&str>| Def {
            id: format!("{group}/{name}"),
            group: group.into(),
            name: name.into(),
            title: title.map(str::to_owned),
            file: String::new(),
            standing: koyomi_core::scan::Standing::Confirmed,
            problems: Vec::new(),
            net: None,
            sha256: String::new(),
            text: String::new(),
            previous: None,
            calendar: None,
        };
        net_choices(&[def("人事", "b", None), def("経理", "締め", Some("月次締め")), def("人事", "a", None)])
    }

    fn words() -> NetWords<'static> {
        NetWords { none: "none", choose: "choose", missing: " (gone)", own: " (self)" }
    }

    #[test]
    fn nets_are_sorted_with_their_title() {
        let n = nets();
        let ids: Vec<&str> = n.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["人事/a", "人事/b", "経理/締め"]);
        assert_eq!(n[2].label, "経理/締め  月次締め");
        assert_eq!(n[0].label, "人事/a");
    }

    #[test]
    fn the_own_net_is_left_out_and_a_missing_one_stays_marked() {
        let n = nets();
        // Required and empty: the placeholder first, nothing else written.
        let (e, now) = net_entries(&n, "人事/a", "", false, &words());
        assert_eq!((e[0].0.as_str(), e[0].1.as_str(), now), ("", "choose", 0));
        assert!(e.iter().all(|(v, _)| v != "人事/a"));
        // Optional: 「なし」 first and picked while empty.
        let (e, now) = net_entries(&n, "", "", true, &words());
        assert_eq!((e[0].1.as_str(), now, e.len()), ("none", 0, 4));
        // A value the list holds is picked, with no empty choice once it is set.
        let (e, now) = net_entries(&n, "", "人事/b", false, &words());
        assert_eq!(e[now].0, "人事/b");
        assert!(e.iter().all(|(v, _)| !v.is_empty()));
        // A value the list does not hold stays as written, marked, and is picked.
        let (e, now) = net_entries(&n, "", "旧/消えた", false, &words());
        assert_eq!((e[now].0.as_str(), e[now].1.as_str()), ("旧/消えた", "旧/消えた (gone)"));
        assert_eq!(e.len(), 4);
        // The net itself named: kept, and said so (it is not offered).
        let (e, now) = net_entries(&n, "人事/a", "人事/a", false, &words());
        assert_eq!(e[now].1, "人事/a (self)");
        assert_eq!(e.iter().filter(|(v, _)| v == "人事/a").count(), 1);
    }

    #[test]
    fn days_read_and_write_in_week_order() {
        let set = days_set("fri, mon,tue").unwrap();
        assert_eq!(set, [true, true, false, false, true, false, false]);
        assert_eq!(days_text(&set), "mon, tue, fri");
        assert_eq!(days_set("").unwrap(), [false; 7]);
        assert!(every_day(&[false; 7]) && !every_day(&set));
        assert_eq!(days_set("mon, funday"), None);
        // The canonical text round trips.
        for text in ["", "mon", "mon, tue, wed, thu, fri", "sat, sun"] {
            assert_eq!(days_text(&days_set(text).unwrap()), text);
        }
        // Turning the last day off goes back to every day.
        let one = days_set("wed").unwrap();
        assert!(every_day(&days_toggle(&one, 2)));
        assert!(!every_day(&days_toggle(&[false; 7], 0)));
    }

    #[test]
    fn a_month_has_the_days_it_has() {
        assert_eq!([1, 2, 4, 12].map(days_in_month), [31, 29, 30, 31]);
        assert_eq!(month_day_parts("02-29"), Some((2, 29)));
        assert_eq!(month_day_parts("13-40"), None);
        assert_eq!(month_day_parts("04-31"), None);
        assert_eq!(month_day_text(4, 31), "04-30");
        assert_eq!(month_day_text(2, 29), "02-29");
        assert_eq!(month_day_text(11, 5), "11-05");
        assert_eq!(number_in(" 31 ", 1, 31), Some(31));
        assert_eq!(number_in("32", 1, 31), None);
        assert_eq!(number_in("1,15", 1, 31), None);
        assert_eq!(weekday_index("wed"), Some(2));
        assert_eq!(weekday_index("x"), None);
    }

    #[test]
    fn the_users_picker_appends_once() {
        assert_eq!(net_append("", "経理/*"), "経理/*");
        assert_eq!(net_append("経理/*", "人事/a"), "経理/* 人事/a");
        assert_eq!(net_append("経理/*, 人事/a", "人事/a"), "経理/*, 人事/a");
        assert_eq!(net_append("経理/* ", "人事/a"), "経理/* 人事/a");
        let ids = ["経理/b".to_owned(), "人事/a".to_owned(), "経理/a".to_owned()];
        assert_eq!(net_patterns_offered(&ids), ["人事/*", "経理/*", "経理/b", "人事/a", "経理/a"]);
    }

    #[test]
    fn a_chooser_starts_where_the_field_is() {
        let dir = std::env::temp_dir();
        let text = dir.display().to_string();
        assert_eq!(start_dir(&text, Want::Folder), Some(dir.clone()));
        let file = dir.join("no-such-file-for-koyomi.txt");
        assert_eq!(start_dir(&file.display().to_string(), Want::File), Some(dir.clone()));
        let glob = dir.join("*.csv");
        assert_eq!(start_dir(&glob.display().to_string(), Want::File), None);
        assert_eq!(start_dir("", Want::Folder), None);
        assert_eq!(start_dir("relative/dir", Want::Folder), None);
        assert_eq!(start_dir("{date}", Want::Folder), None);
        let lines = format!("nowhere-x\n{text}\n");
        assert_eq!(start_dir(&lines, Want::Files), Some(dir));
    }

    #[test]
    fn a_chosen_path_fills_the_field() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(browsed_text("old", Want::File, &[p("C:/a.txt")]), Some(p("C:/a.txt").display().to_string()));
        assert_eq!(browsed_text("x", Want::Folder, &[]), None);
        let out = browsed_text("C:/a.txt\n", Want::Files, &[p("C:/b.txt"), p("C:/c.txt")]).unwrap();
        assert_eq!(
            out.lines().collect::<Vec<_>>(),
            ["C:/a.txt", &p("C:/b.txt").display().to_string(), &p("C:/c.txt").display().to_string()]
        );
        assert_eq!(browsed_text("", Want::Files, &[p("C:/b.txt")]), Some(p("C:/b.txt").display().to_string()));
    }
}
