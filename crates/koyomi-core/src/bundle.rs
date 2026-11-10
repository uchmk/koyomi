//! Export and import: the bundle file `koyomi export` writes, `koyomi import`
//! reads and the engine's daily backup is.
//!
//! A bundle is one TOML file that carries other files as text, each as it is
//! on disk (comments and all), so what leaves one PC arrives on another
//! unchanged:
//!
//! ```toml
//! format = 1
//!
//! [[calendar]]
//! name = "会社"
//! text = """…"""
//!
//! [[group]]
//! name = "経理"
//! text = """…"""        # the group's _group.toml
//!
//! [[jobnet]]
//! id = "経理/月次締め"
//! text = """…"""
//!
//! [settings]            # koyomi.toml as a table; webhook addresses are "***"
//! ```
//!
//! Nothing here writes to disk: [`Source::read`] reads the settings folder,
//! [`export`] picks what goes out, [`plan`] says what an import would do to
//! what is there, and the engine does it (the versions, the files).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::approved::Store;
use crate::calendar::{self, Calendar, Calendars};
use crate::config::Config;
use crate::group::{self, Group};
use crate::jobnet::{self, Part};
use crate::msg;
use crate::scan::{self, Def, Standing};

/// The `format` this version writes and reads.
pub const FORMAT: u32 = 1;

/// What stands in for a webhook address in a bundle: the address never leaves the PC.
pub const MASK: &str = "***";

/// A bundle, read or about to be written.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub format: u32,
    #[serde(default, rename = "calendar", skip_serializing_if = "Vec::is_empty")]
    pub calendars: Vec<Named>,
    #[serde(default, rename = "group", skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Named>,
    #[serde(default, rename = "jobnet", skip_serializing_if = "Vec::is_empty")]
    pub jobnets: Vec<Net>,
    /// `koyomi.toml` as a table, when it is carried. Last: a table after the arrays of tables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<toml::Table>,
}

/// A calendar or a group file: its name and its text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Named {
    pub name: String,
    pub text: String,
}

/// A jobnet file: `グループ/名前` and its text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Net {
    pub id: String,
    pub text: String,
}

impl Bundle {
    pub fn new() -> Bundle {
        Bundle { format: FORMAT, ..Default::default() }
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// How many things it carries.
    pub fn len(&self) -> usize {
        self.calendars.len() + self.groups.len() + self.jobnets.len() + usize::from(self.settings.is_some())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Reads a bundle's text, or says why it cannot be used (a newer `format` first).
pub fn parse(text: &str) -> Result<Bundle, String> {
    #[derive(Deserialize)]
    struct Head {
        format: Option<u32>,
    }
    let head: Head = toml::from_str(text).map_err(|e| msg::line(msg::BUNDLE_TOML_BAD, &[first_line(e.message())]))?;
    match head.format {
        None => return Err(msg::line(msg::BUNDLE_FORMAT_MISSING, &[&FORMAT.to_string()])),
        Some(f) if f > FORMAT => return Err(msg::line(msg::BUNDLE_FORMAT_NEWER, &[&f.to_string(), &FORMAT.to_string()])),
        Some(_) => {}
    }
    let b: Bundle = toml::from_str(text).map_err(|e| msg::line(msg::BUNDLE_TOML_BAD, &[first_line(e.message())]))?;
    for n in &b.jobnets {
        if !jobnet::is_id(&n.id) {
            return Err(msg::line(msg::BUNDLE_JOBNET_ID, &[&n.id]));
        }
    }
    for c in &b.calendars {
        if !calendar::valid_name(&c.name) {
            return Err(msg::line(msg::BUNDLE_NAME_BAD, &["calendar", &c.name]));
        }
    }
    for g in &b.groups {
        if !jobnet::is_id(&format!("{}/x", g.name)) {
            return Err(msg::line(msg::BUNDLE_NAME_BAD, &["group", &g.name]));
        }
    }
    Ok(b)
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or_default()
}

/// The settings folder as texts: what an export picks from and what an import is
/// compared with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Source {
    /// `koyomi.toml`, when there is one.
    pub settings: Option<String>,
    /// `calendars/<name>.toml` by name.
    pub calendars: BTreeMap<String, String>,
    /// `jobnets/<group>/_group.toml` by group.
    pub groups: BTreeMap<String, String>,
    /// `jobnets/<group>/<name>.toml` by `グループ/名前`.
    pub jobnets: BTreeMap<String, String>,
}

impl Source {
    /// Reads `app` (`jobnets/`, `calendars/`, `koyomi.toml`). A file that is not
    /// UTF-8 or cannot be read is left out: it is not a text to carry.
    pub fn read(app: &Path) -> Source {
        let text = |p: &Path| std::fs::read(p).ok().and_then(|b| String::from_utf8(b).ok());
        let mut src = Source { settings: text(&app.join("koyomi.toml")), ..Default::default() };
        if let Ok(rd) = std::fs::read_dir(app.join("calendars")) {
            for p in rd.flatten().map(|e| e.path()).filter(|p| p.is_file()) {
                let name = file_name(&p);
                if let Some(stem) = name.strip_suffix(".toml") {
                    if let Some(t) = text(&p) {
                        src.calendars.insert(stem.to_owned(), t);
                    }
                }
            }
        }
        let Ok(rd) = std::fs::read_dir(app.join("jobnets")) else { return src };
        for group in rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            let g = file_name(&group);
            let Ok(files) = std::fs::read_dir(&group) else { continue };
            for p in files.flatten().map(|e| e.path()).filter(|p| p.is_file()) {
                let name = file_name(&p);
                let Some(stem) = name.strip_suffix(".toml") else { continue };
                let Some(t) = text(&p) else { continue };
                if stem == "_group" {
                    src.groups.insert(g.clone(), t);
                } else if !stem.is_empty() {
                    src.jobnets.insert(format!("{g}/{stem}"), t);
                }
            }
        }
        src
    }

    /// `koyomi.toml` as a table, when it is there and reads.
    fn settings_table(&self) -> Option<toml::Table> {
        self.settings.as_deref().and_then(|t| toml::from_str(t).ok())
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// What `koyomi export` takes out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pick {
    /// `グループ/名前`, each with the calendar it counts by, its group file and its child nets.
    pub ids: Vec<String>,
    /// Every net of these groups, the same way.
    pub groups: Vec<String>,
    pub calendars: bool,
    pub settings: bool,
    pub all: bool,
}

impl Pick {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty() && self.groups.is_empty() && !self.calendars && !self.settings && !self.all
    }
}

/// The bundle of what `pick` names in `src`. A net or a group that is not there
/// is an error; a calendar or a child a net names but `src` lacks is simply not
/// carried (the import says so when it is missing there too).
pub fn export(src: &Source, pick: &Pick) -> Result<Bundle, String> {
    if pick.is_empty() {
        return Err(msg::line(msg::BUNDLE_PICK_EMPTY, &[]));
    }
    let mut ids: BTreeSet<String> = BTreeSet::new();
    let mut groups: BTreeSet<String> = BTreeSet::new();
    let mut cals: BTreeSet<String> = BTreeSet::new();
    for g in &pick.groups {
        let nets: Vec<&String> = src.jobnets.keys().filter(|id| id.strip_prefix(g.as_str()).is_some_and(|r| r.starts_with('/'))).collect();
        if nets.is_empty() && !src.groups.contains_key(g) {
            return Err(msg::line(msg::BUNDLE_GROUP_NONE, &[g]));
        }
        ids.extend(nets.into_iter().cloned());
        groups.insert(g.clone());
    }
    for id in &pick.ids {
        if !src.jobnets.contains_key(id) {
            return Err(msg::line(msg::BUNDLE_DEF_NONE, &[id]));
        }
        ids.insert(id.clone());
    }
    if pick.all {
        ids.extend(src.jobnets.keys().cloned());
    }
    // Each net brings its group file, its calendar and its children (and theirs).
    let mut todo: Vec<String> = ids.iter().cloned().collect();
    while let Some(id) = todo.pop() {
        let Some((g, _)) = id.split_once('/') else { continue };
        let shared = src.groups.get(g).map(|t| group::parse(t));
        if shared.is_some() {
            groups.insert(g.to_owned());
        }
        let Some(text) = src.jobnets.get(&id) else { continue };
        let Ok(net) = jobnet::parse(text) else { continue };
        let cal = net.calendar.clone().or_else(|| shared.and_then(|s| s.ok()).and_then(|s| s.calendar));
        if let Some(c) = cal.filter(|c| src.calendars.contains_key(c)) {
            cals.insert(c);
        }
        for j in &net.jobs {
            if let Part::Jobnet { use_ } = &j.part {
                if src.jobnets.contains_key(use_) && ids.insert(use_.clone()) {
                    todo.push(use_.clone());
                }
            }
        }
    }
    if pick.all {
        groups.extend(src.groups.keys().cloned());
    }
    if pick.calendars || pick.all {
        cals.extend(src.calendars.keys().cloned());
    }
    let mut b = Bundle::new();
    b.calendars = cals.into_iter().filter_map(|n| src.calendars.get(&n).map(|t| Named { name: n, text: t.clone() })).collect();
    b.groups = groups.into_iter().filter_map(|n| src.groups.get(&n).map(|t| Named { name: n, text: t.clone() })).collect();
    b.jobnets = ids.into_iter().filter_map(|id| src.jobnets.get(&id).map(|t| Net { id, text: t.clone() })).collect();
    if pick.settings || pick.all {
        if let Some(t) = &src.settings {
            let table: toml::Table =
                toml::from_str(t).map_err(|e| msg::line(msg::BUNDLE_SETTINGS_UNREADABLE, &[first_line(e.message())]))?;
            b.settings = Some(mask(&table));
        }
    }
    Ok(b)
}

/// `settings` with every webhook address replaced by [`MASK`]. A `secret:<name>`
/// is a name, not an address: it stays.
pub fn mask(settings: &toml::Table) -> toml::Table {
    let mut out = settings.clone();
    let Some(toml::Value::Table(notify)) = out.get_mut("notify") else { return out };
    let is_address = |v: &toml::Value| v.as_str().is_some_and(|s| !s.starts_with("secret:") && s != MASK);
    if notify.get("webhook").is_some_and(is_address) {
        notify.insert("webhook".to_owned(), toml::Value::String(MASK.to_owned()));
    }
    if let Some(toml::Value::Array(hooks)) = notify.get_mut("webhooks") {
        for h in hooks.iter_mut().filter_map(|h| h.as_table_mut()) {
            if h.get("url").is_some_and(is_address) {
                h.insert("url".to_owned(), toml::Value::String(MASK.to_owned()));
            }
        }
    }
    out
}

/// Puts the addresses of `current` back where `settings` has [`MASK`] (the same
/// key, the same place in `webhooks`). A masked address `current` does not have
/// is dropped: `webhook` goes, a `webhooks` entry goes. The notes say what went.
pub fn unmask(settings: &mut toml::Table, current: Option<&toml::Table>) -> Vec<String> {
    let mut notes = Vec::new();
    let Some(toml::Value::Table(notify)) = settings.get_mut("notify") else { return notes };
    let now = current.and_then(|c| c.get("notify")).and_then(|n| n.as_table());
    let masked = |v: Option<&toml::Value>| v.and_then(|v| v.as_str()) == Some(MASK);
    if masked(notify.get("webhook")) {
        match now.and_then(|n| n.get("webhook")).filter(|v| !masked(Some(v))) {
            Some(v) => {
                notify.insert("webhook".to_owned(), v.clone());
            }
            None => {
                notify.remove("webhook");
                notes.push(msg::line(msg::BUNDLE_WEBHOOK_DROPPED, &[]));
            }
        }
    }
    if let Some(toml::Value::Array(hooks)) = notify.get_mut("webhooks") {
        let now_hooks: Vec<toml::Value> = now.and_then(|n| n.get("webhooks")).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let mut i = 0;
        hooks.retain_mut(|h| {
            let at = i;
            i += 1;
            let Some(t) = h.as_table_mut() else { return true };
            if !masked(t.get("url")) {
                return true;
            }
            match now_hooks.get(at).and_then(|n| n.get("url")).filter(|v| !masked(Some(v))) {
                Some(v) => {
                    t.insert("url".to_owned(), v.clone());
                    true
                }
                None => {
                    notes.push(msg::line(msg::BUNDLE_WEBHOOKS_DROPPED, &[&(at + 1).to_string()]));
                    false
                }
            }
        });
    }
    notes
}

/// What to do where the import meets something that is there and differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    /// Write over it; the file as it was is kept as a version.
    Overwrite,
    /// Bring it in under `名前-2`, `名前-3` …; what is there stays.
    Rename,
    /// Leave it as it is.
    Skip,
}

impl Conflict {
    pub fn parse(s: &str) -> Result<Conflict, String> {
        match s {
            "overwrite" => Ok(Conflict::Overwrite),
            "rename" => Ok(Conflict::Rename),
            "skip" => Ok(Conflict::Skip),
            _ => Err(msg::line(msg::BUNDLE_ON_CONFLICT, &[s])),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Settings,
    Calendar,
    Group,
    Jobnet,
}

impl Kind {
    pub fn word(self) -> String {
        let note = match self {
            Kind::Settings => msg::N_BUNDLE_KIND_SETTINGS,
            Kind::Calendar => msg::N_BUNDLE_KIND_CALENDAR,
            Kind::Group => msg::N_BUNDLE_KIND_GROUP,
            Kind::Jobnet => msg::N_BUNDLE_KIND_JOBNET,
        };
        msg::note(note, &[])
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// Not there yet.
    Add,
    /// There, the same text: nothing to do.
    Same,
    /// There and different: written over (a version is kept first).
    Replace,
    /// There and different: brought in as this name instead.
    Rename(String),
    /// There and different: left alone.
    Skip,
}

/// One thing the import would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    /// The name in the bundle (`グループ/名前` for a net).
    pub name: String,
    pub change: Change,
    /// The text there now, when something is there.
    pub old: Option<String>,
    /// The text that would be written (the settings unmasked, as TOML).
    pub new: String,
    /// Why it cannot be brought in; an item with any is not written.
    pub problems: Vec<String>,
}

impl Item {
    /// `追加: ジョブネット 経理/月次締め`.
    pub fn line(&self) -> String {
        let what = format!("{} {}", self.kind.word(), self.name);
        match &self.change {
            Change::Add => msg::note(msg::N_BUNDLE_ADD, &[&what]),
            Change::Same => msg::note(msg::N_BUNDLE_SAME, &[&what]),
            Change::Replace => msg::note(msg::N_BUNDLE_REPLACE, &[&what]),
            Change::Rename(to) => msg::note(msg::N_BUNDLE_RENAME, &[&what, to]),
            Change::Skip => msg::note(msg::N_BUNDLE_SKIP, &[&what]),
        }
    }

    /// Different from what is there: the person's choice decides it.
    pub fn conflicts(&self) -> bool {
        matches!(self.change, Change::Replace | Change::Rename(_) | Change::Skip)
    }
}

/// What an import would do, item by item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub items: Vec<Item>,
    pub notes: Vec<String>,
    /// A conflict was met and no [`Conflict`] was given: nothing may be written.
    pub undecided: bool,
}

impl Plan {
    pub fn conflicts(&self) -> usize {
        self.items.iter().filter(|i| i.conflicts()).count()
    }

    pub fn problems(&self) -> usize {
        self.items.iter().map(|i| i.problems.len()).sum()
    }

    /// Something would be written.
    pub fn writes(&self) -> bool {
        self.items.iter().any(|i| matches!(i.change, Change::Add | Change::Replace | Change::Rename(_)))
    }

    /// The items, their problems under them, then the notes.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for i in &self.items {
            out.push(i.line());
            out.extend(i.problems.iter().map(|p| msg::note(msg::N_BUNDLE_PROBLEM, &[p])));
        }
        out.extend(self.notes.iter().cloned());
        out
    }
}

/// What bringing `b` into `current` would do. `on_conflict` is for what is there
/// and differs; with `None` such items are planned as overwrites and the plan is
/// `undecided`. The settings and a group file cannot be renamed: under
/// [`Conflict::Rename`] they are skipped.
pub fn plan(b: &Bundle, current: &Source, on_conflict: Option<Conflict>) -> Plan {
    let mut out = Plan::default();

    // Settings: compared as tables, the addresses put back first.
    if let Some(table) = &b.settings {
        let now = current.settings_table();
        let mut table = table.clone();
        let notes = unmask(&mut table, now.as_ref());
        let new = toml::to_string(&table).unwrap_or_default();
        let mut problems = Vec::new();
        if let Err(e) = toml::Value::Table(table.clone()).try_into::<Config>() {
            problems.push(msg::line(msg::BUNDLE_SETTINGS_BAD, &[first_line(&e.to_string())]));
        }
        let change = match (&now, &current.settings) {
            (Some(n), _) if *n == table => Change::Same,
            (_, Some(old)) => choose(&mut out, on_conflict, old, &new, Kind::Settings, "koyomi.toml", &|_| true),
            (_, None) => Change::Add,
        };
        if change != Change::Same && change != Change::Skip {
            out.notes.extend(notes);
            out.notes.push(msg::line(msg::BUNDLE_SETTINGS_COMMENTS, &[]));
        }
        out.items.push(Item { kind: Kind::Settings, name: "koyomi.toml".to_owned(), change, old: current.settings.clone(), new, problems });
    }

    // Calendars: the import's and the current ones together are what the nets may name.
    let mut cals = Calendars::default();
    for (name, text) in &current.calendars {
        if let Ok(c) = Calendar::parse(name, text) {
            cals.map.insert(name.clone(), c);
        }
    }
    let bundle_cals: BTreeSet<&str> = b.calendars.iter().map(|c| c.name.as_str()).collect();
    for c in &b.calendars {
        let mut problems = Vec::new();
        match Calendar::parse(&c.name, &c.text) {
            Ok(parsed) => {
                cals.map.insert(c.name.clone(), parsed);
            }
            // A coded line stays first (as `Problem::in_file` does), the place goes last.
            Err(e) if msg::is_coded(&e) => problems.push(msg::note(msg::N_PROBLEM_CODED_AT, &[&e, &format!("calendars/{}.toml", c.name)])),
            Err(e) => problems.push(msg::note(msg::N_PROBLEM_AT, &[&format!("calendars/{}.toml", c.name), &e])),
        }
        let old = current.calendars.get(&c.name).cloned();
        let change = match &old {
            None => Change::Add,
            Some(o) => choose(&mut out, on_conflict, o, &c.text, Kind::Calendar, &c.name, &|n| {
                current.calendars.contains_key(n) || bundle_cals.contains(n)
            }),
        };
        if let Change::Rename(to) = &change {
            if let Ok(parsed) = Calendar::parse(to, &c.text) {
                cals.map.insert(to.clone(), parsed);
            }
        }
        out.items.push(Item { kind: Kind::Calendar, name: c.name.clone(), change, old, new: c.text.clone(), problems });
    }

    // Group files.
    let mut shared: BTreeMap<String, Result<Group, String>> = BTreeMap::new();
    for (g, text) in &current.groups {
        shared.insert(g.clone(), group::parse(text));
    }
    for g in &b.groups {
        let mut problems = Vec::new();
        let parsed = group::parse(&g.text);
        if let Err(e) = &parsed {
            problems.push(format!("{}/_group.toml: {e}", g.name));
        }
        let old = current.groups.get(&g.name).cloned();
        let change = match &old {
            None => Change::Add,
            Some(o) => choose(&mut out, on_conflict, o, &g.text, Kind::Group, &g.name, &|_| true),
        };
        if matches!(change, Change::Add | Change::Replace) {
            shared.insert(g.name.clone(), parsed);
        }
        out.items.push(Item { kind: Kind::Group, name: g.name.clone(), change, old, new: g.text.clone(), problems });
    }

    // Nets: each checked as a file in its place would be, with the others around it.
    let bundle_ids: BTreeSet<&str> = b.jobnets.iter().map(|n| n.id.as_str()).collect();
    let store = Store::default();
    let mut planned: Vec<(usize, String)> = Vec::new();
    let mut defs: Vec<Def> = Vec::new();
    for n in &b.jobnets {
        let old = current.jobnets.get(&n.id).cloned();
        let change = match &old {
            None => Change::Add,
            Some(o) => choose(&mut out, on_conflict, o, &n.text, Kind::Jobnet, &n.id, &|id| {
                current.jobnets.contains_key(id) || bundle_ids.contains(id)
            }),
        };
        let at = match &change {
            Change::Rename(to) => to.clone(),
            _ => n.id.clone(),
        };
        if matches!(change, Change::Add | Change::Replace | Change::Rename(_)) {
            let (g, name) = at.split_once('/').unwrap_or((&at, ""));
            let s = shared.get(g).cloned().unwrap_or(Ok(Group::default()));
            defs.push(scan::def_of_text(&n.text, g, name, &store, s.as_ref(), &cals));
            planned.push((out.items.len(), at));
        }
        out.items.push(Item { kind: Kind::Jobnet, name: n.id.clone(), change, old, new: n.text.clone(), problems: Vec::new() });
    }
    let written: BTreeSet<&str> = planned.iter().map(|(_, at)| at.as_str()).collect();
    for (id, text) in &current.jobnets {
        if written.contains(id.as_str()) {
            continue;
        }
        let (g, name) = id.split_once('/').unwrap_or((id, ""));
        let s = shared.get(g).cloned().unwrap_or(Ok(Group::default()));
        defs.push(scan::def_of_text(text, g, name, &store, s.as_ref(), &cals));
    }
    scan::link(&mut defs);
    for (i, at) in &planned {
        if let Some(d) = defs.iter().find(|d| &d.id == at).filter(|d| d.standing == Standing::Invalid) {
            out.items[*i].problems.extend(d.problems.iter().map(|p| p.in_file(&d.file)));
        }
    }
    if out.items.iter().any(|i| matches!(i.change, Change::Rename(_))) {
        out.notes.push(msg::line(msg::BUNDLE_RENAME_REFS, &[]));
    }
    out
}

/// What to do with `new` where `old` is there, under the choice (or none yet).
fn choose(
    out: &mut Plan,
    on_conflict: Option<Conflict>,
    old: &str,
    new: &str,
    kind: Kind,
    name: &str,
    taken: &dyn Fn(&str) -> bool,
) -> Change {
    if old == new {
        return Change::Same;
    }
    if on_conflict.is_none() {
        out.undecided = true;
    }
    match on_conflict.unwrap_or(Conflict::Overwrite) {
        Conflict::Overwrite => Change::Replace,
        Conflict::Skip => Change::Skip,
        Conflict::Rename if matches!(kind, Kind::Settings | Kind::Group) => {
            out.notes.push(msg::line(msg::BUNDLE_CANNOT_RENAME, &[&kind.word(), name]));
            Change::Skip
        }
        Conflict::Rename => Change::Rename(free_name(name, taken)),
    }
}

/// `name-2`, `name-3` …: the first not `taken`. A net's `グループ/名前-2`.
fn free_name(name: &str, taken: &dyn Fn(&str) -> bool) -> String {
    for n in 2.. {
        let candidate = format!("{name}-{n}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET: &str = "format = 1\nname = \"月次締め\"\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";
    const CAL: &str = "format = 1\nholidays = \"japan\"\n";
    const GROUP: &str = "format = 1\ncalendar = \"会社\"\n";

    fn settings() -> toml::Table {
        toml::from_str("concurrency = 2\n[notify]\nwebhook = \"https://a.example/x\"\n[[notify.webhooks]]\nurl = \"https://b.example/y\"\nstates = [\"err\"]\n[[notify.webhooks]]\nurl = \"secret:hook2\"\n").unwrap()
    }

    fn src() -> Source {
        let mut s = Source::default();
        s.calendars.insert("会社".to_owned(), CAL.to_owned());
        s.calendars.insert("工場".to_owned(), CAL.to_owned());
        s.groups.insert("経理".to_owned(), GROUP.to_owned());
        s.jobnets.insert("経理/月次締め".to_owned(), NET.to_owned());
        s.jobnets.insert("経理/子".to_owned(), NET.to_owned());
        s.jobnets.insert(
            "総務/点検".to_owned(),
            NET.replace("\"a\"", "\"b\"")
                .replace("x.exe\"]\n", "x.exe\"]\n[[job]]\nid = \"c\"\nkind = \"jobnet\"\nuse = \"経理/子\"\nafter = [{ job = \"b\" }]\n"),
        );
        s.settings = Some(toml::to_string(&settings()).unwrap());
        s
    }

    #[test]
    fn round_trip_keeps_texts_and_settings() {
        let mut b = Bundle::new();
        b.calendars.push(Named { name: "会社".to_owned(), text: "a\r\nb \"\"\" c\\d\n".to_owned() });
        b.groups.push(Named { name: "経理".to_owned(), text: GROUP.to_owned() });
        b.jobnets.push(Net { id: "経理/月次締め".to_owned(), text: NET.to_owned() });
        b.settings = Some(mask(&settings()));
        let text = b.to_toml();
        assert!(text.starts_with("format = 1\n"), "{text}");
        assert!(text.contains("[[calendar]]") && text.contains("[[jobnet]]") && text.contains("[settings]"), "{text}");
        assert_eq!(parse(&text).unwrap(), b);
        assert!(parse(&text).unwrap().jobnets[0].text == NET);
    }

    #[test]
    fn parse_refuses_what_it_cannot_use() {
        let e = parse("x = 1\n").unwrap_err();
        assert!(msg::BUNDLE_FORMAT_MISSING.is_in(&e) && e.contains("format = 1"), "{e}");
        let e = parse("format = 9\n").unwrap_err();
        assert!(msg::BUNDLE_FORMAT_NEWER.is_in(&e) && e.contains("format = 9"), "{e}");
        let e = parse("format = 1\n[[jobnet]]\nid = \"x\"\ntext = \"\"\n").unwrap_err();
        assert!(msg::BUNDLE_JOBNET_ID.is_in(&e) && e.contains("グループ/名前"), "{e}");
        let e = parse("format = 1\n[[calendar]]\nname = \"_x\"\ntext = \"\"\n").unwrap_err();
        assert!(msg::BUNDLE_NAME_BAD.is_in(&e) && e.contains("name"), "{e}");
        let e = parse("format = 1\nnope = 1\n").unwrap_err();
        assert!(msg::BUNDLE_TOML_BAD.is_in(&e) && e.contains("TOML"), "{e}");
        assert_eq!(parse("format = 1\n").unwrap(), Bundle::new());
    }

    #[test]
    fn mask_hides_addresses_and_unmask_puts_them_back() {
        let masked = mask(&settings());
        let notify = masked["notify"].as_table().unwrap();
        assert_eq!(notify["webhook"].as_str(), Some(MASK));
        let hooks = notify["webhooks"].as_array().unwrap();
        assert_eq!(hooks[0]["url"].as_str(), Some(MASK));
        assert_eq!(hooks[1]["url"].as_str(), Some("secret:hook2"));
        assert_eq!(masked["concurrency"].as_integer(), Some(2));

        let mut back = masked.clone();
        assert!(unmask(&mut back, Some(&settings())).is_empty());
        assert_eq!(back, settings());

        let mut alone = masked.clone();
        let notes = unmask(&mut alone, None);
        assert_eq!(notes.len(), 2, "{notes:?}");
        let notify = alone["notify"].as_table().unwrap();
        assert!(notify.get("webhook").is_none());
        assert_eq!(notify["webhooks"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn export_pulls_group_calendar_and_children() {
        let s = src();
        let b = export(&s, &Pick { ids: vec!["総務/点検".to_owned()], ..Default::default() }).unwrap();
        let ids: Vec<&str> = b.jobnets.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["経理/子", "総務/点検"]);
        assert_eq!(b.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["経理"]);
        assert_eq!(b.calendars.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["会社"]);
        assert!(b.settings.is_none());

        let b = export(&s, &Pick { groups: vec!["経理".to_owned()], settings: true, ..Default::default() }).unwrap();
        assert_eq!(b.jobnets.len(), 2);
        assert_eq!(b.settings.as_ref().unwrap()["notify"]["webhook"].as_str(), Some(MASK));

        let b = export(&s, &Pick { all: true, ..Default::default() }).unwrap();
        assert_eq!((b.jobnets.len(), b.calendars.len(), b.groups.len()), (3, 2, 1));
        assert!(b.settings.is_some());

        assert!(export(&s, &Pick::default()).is_err());
        let e = export(&s, &Pick { ids: vec!["x/y".to_owned()], ..Default::default() }).unwrap_err();
        assert!(msg::BUNDLE_DEF_NONE.is_in(&e) && e.contains("x/y"), "{e}");
        let e = export(&s, &Pick { groups: vec!["x".to_owned()], ..Default::default() }).unwrap_err();
        assert!(msg::BUNDLE_GROUP_NONE.is_in(&e) && e.contains('x'), "{e}");
        assert!(msg::BUNDLE_PICK_EMPTY.is_in(&export(&s, &Pick::default()).unwrap_err()));
        assert!(msg::BUNDLE_ON_CONFLICT.is_in(&Conflict::parse("merge").unwrap_err()));
    }

    #[test]
    fn plan_sorts_items_and_waits_for_a_choice() {
        let s = src();
        let mut b = export(&s, &Pick { all: true, ..Default::default() }).unwrap();
        // Everything the same: nothing to write, nothing to decide.
        let p = plan(&b, &s, None);
        assert!(p.items.iter().all(|i| i.change == Change::Same), "{:?}", p.lines());
        assert!(!p.undecided && !p.writes() && p.problems() == 0);

        b.jobnets[0].text.push_str("# changed\n");
        b.jobnets.push(Net { id: "総務/新規".to_owned(), text: NET.to_owned() });
        let p = plan(&b, &s, None);
        assert!(p.undecided);
        assert_eq!(p.conflicts(), 1);
        assert_eq!(p.lines()[p.items.len() - 1], "追加: ジョブネット 総務/新規");
        let p = plan(&b, &s, Some(Conflict::Skip));
        assert!(!p.undecided);
        assert!(p.items.iter().any(|i| i.change == Change::Skip));
        let p = plan(&b, &s, Some(Conflict::Rename));
        let r = p.items.iter().find(|i| matches!(i.change, Change::Rename(_))).unwrap();
        assert_eq!(r.change, Change::Rename("経理/子-2".to_owned()));
        assert!(p.notes.iter().any(|n| msg::BUNDLE_RENAME_REFS.is_in(n)), "{:?}", p.notes);

        // A net that does not read, and one whose child is not there.
        b.jobnets.push(Net { id: "総務/壊れ".to_owned(), text: "format = 1\nrnu = 1\n".to_owned() });
        b.jobnets.push(Net {
            id: "総務/孤児".to_owned(), text: NET.replace("run = [\"x.exe\"]", "kind = \"jobnet\"\nuse = \"無い/子\"")
        });
        let p = plan(&b, &s, Some(Conflict::Overwrite));
        assert_eq!(p.problems(), 2, "{:?}", p.lines());
        assert!(p.lines().iter().any(|l| l.contains("総務/壊れ.toml") && l.contains("rnu")));
        assert!(p.lines().iter().any(|l| l.contains(&crate::msg::CHILD_MISSING.code()) && l.contains("無い/子")));
    }

    #[test]
    fn a_calendar_that_does_not_read_keeps_its_id_first_and_its_file_last() {
        let s = src();
        let mut b = export(&s, &Pick { all: true, ..Default::default() }).unwrap();
        b.calendars[0].text = "format = 1
rnu = 1
"
        .to_owned();
        let name = b.calendars[0].name.clone();
        let p = plan(&b, &s, Some(Conflict::Overwrite));
        let line = p.items.iter().flat_map(|i| &i.problems).find(|l| l.contains(&format!("calendars/{name}.toml"))).expect("a problem");
        assert!(crate::msg::is_coded(line), "{line}");
        assert!(line.ends_with(&format!("（calendars/{name}.toml）")), "{line}");
    }

    #[test]
    fn plan_settings_put_addresses_back_and_group_rename_skips() {
        let s = src();
        let mut b = export(&s, &Pick { all: true, ..Default::default() }).unwrap();
        b.settings.as_mut().unwrap().insert("concurrency".to_owned(), toml::Value::Integer(3));
        b.groups[0].text.push_str("# x\n");
        let p = plan(&b, &s, Some(Conflict::Rename));
        let settings = p.items.iter().find(|i| i.kind == Kind::Settings).unwrap();
        assert_eq!(settings.change, Change::Skip);
        assert!(p.items.iter().any(|i| i.kind == Kind::Group && i.change == Change::Skip));
        let p = plan(&b, &s, Some(Conflict::Overwrite));
        let settings = p.items.iter().find(|i| i.kind == Kind::Settings).unwrap();
        assert_eq!(settings.change, Change::Replace);
        assert!(settings.new.contains("https://a.example/x") && settings.new.contains("concurrency = 3"), "{}", settings.new);
        assert!(p.notes.iter().any(|n| msg::BUNDLE_SETTINGS_COMMENTS.is_in(n) && n.contains("コメントは消えます")), "{:?}", p.notes);

        // Settings that are not koyomi.toml.
        b.settings.as_mut().unwrap().insert("nope".to_owned(), toml::Value::Integer(1));
        let p = plan(&b, &s, Some(Conflict::Overwrite));
        assert_eq!(p.problems(), 1);

        // No settings there yet: added.
        let empty = Source { calendars: s.calendars.clone(), ..Default::default() };
        let b = export(&s, &Pick { settings: true, calendars: true, ..Default::default() }).unwrap();
        let p = plan(&b, &empty, None);
        assert_eq!(p.items[0].change, Change::Add);
        assert!(!p.undecided);
    }

    #[test]
    fn source_reads_the_folder() {
        let d = std::env::temp_dir().join(format!("koyomi-core-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("jobnets/経理")).unwrap();
        std::fs::create_dir_all(d.join("calendars")).unwrap();
        std::fs::write(d.join("jobnets/経理/月次締め.toml"), NET).unwrap();
        std::fs::write(d.join("jobnets/経理/_group.toml"), GROUP).unwrap();
        std::fs::write(d.join("jobnets/経理/memo.txt"), "x").unwrap();
        std::fs::write(d.join("calendars/会社.toml"), CAL).unwrap();
        std::fs::write(d.join("koyomi.toml"), "concurrency = 1\n").unwrap();
        let s = Source::read(&d);
        assert_eq!(s.jobnets.keys().collect::<Vec<_>>(), ["経理/月次締め"]);
        assert_eq!(s.groups["経理"], GROUP);
        assert_eq!(s.calendars["会社"], CAL);
        assert_eq!(s.settings.as_deref(), Some("concurrency = 1\n"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
