//! `jobnets/<group>/<name>.toml` read into what the tree and `koyomi list` show.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::approved::{self, Store};
use crate::calendar::Calendars;
use crate::group::{self, Group};
use crate::jobnet::{self, Jobnet, Problem};
use crate::msg;
use crate::when::Kind;

/// Where a definition stands. Only [`Confirmed`](Standing::Confirmed) may ever run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// Looked at and approved, and not touched since.
    Confirmed,
    /// Not approved yet, or changed since it was (by hand, or by an import).
    Unconfirmed,
    /// Does not read; there is nothing to approve.
    Invalid,
}

/// One definition file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Def {
    /// `経理/月次締め`: what `koyomi approve` takes.
    pub id: String,
    pub group: String,
    /// The file stem; what the tree shows when the file has no `name`.
    pub name: String,
    /// The file's own `name`, when it reads and has one.
    pub title: Option<String>,
    /// The path as the person sees it: `経理/月次締め.toml`.
    pub file: String,
    pub standing: Standing,
    pub problems: Vec<Problem>,
    pub net: Option<Jobnet>,
    pub sha256: String,
    pub text: String,
    /// The text last approved, when this has changed since: what the next approval compares with.
    pub previous: Option<String>,
    /// The calendar this net counts by: its own `calendar`, else the group's `_group.toml`'s.
    #[serde(default)]
    pub calendar: Option<String>,
}

impl Def {
    /// Whether it is not approved or does not read (the tree's and the status bar's count).
    pub fn is_todo(&self) -> bool {
        self.standing != Standing::Confirmed
    }

    /// What the tree calls it.
    pub fn label(&self) -> &str {
        self.title.as_deref().filter(|t| !t.is_empty()).unwrap_or(&self.name)
    }
}

/// [`read_def_in`] with no group file and no calendars.
pub fn read_def(path: &Path, group: &str, name: &str, store: &Store) -> Def {
    read_def_in(path, group, name, store, Ok(&Group::default()), &Calendars::default())
}

/// Reads the file at `path` as the definition `id` in `group` (`file` is how it is shown).
/// `shared` is the group's `_group.toml` (or why it does not read); `cals` what `calendar` may name.
pub fn read_def_in(path: &Path, group: &str, name: &str, store: &Store, shared: Result<&Group, &String>, cals: &Calendars) -> Def {
    match std::fs::read(path) {
        Ok(b) => match std::str::from_utf8(&b) {
            Ok(t) => def_of_text(t, group, name, store, shared, cals),
            Err(_) => unreadable(&approved::hash(&b), group, name, msg::line(msg::DEF_NOT_UTF8, &[&format!("{group}/{name}.toml")])),
        },
        Err(e) => unreadable("", group, name, msg::line(msg::FILE_READ_FAILED, &[&path.display().to_string(), &e.to_string()])),
    }
}

/// A file that could not be read as text: invalid, with the one problem.
fn unreadable(sha256: &str, group: &str, name: &str, message: String) -> Def {
    Def {
        id: format!("{group}/{name}"),
        group: group.to_owned(),
        name: name.to_owned(),
        title: None,
        file: format!("{group}/{name}.toml"),
        standing: Standing::Invalid,
        problems: vec![Problem { line: 0, message }],
        net: None,
        sha256: sha256.to_owned(),
        text: String::new(),
        previous: None,
        calendar: None,
    }
}

/// [`read_def_in`] for text already in hand: what the engine checks before it
/// saves what the editor sends.
pub fn def_of_text(text: &str, group: &str, name: &str, store: &Store, shared: Result<&Group, &String>, cals: &Calendars) -> Def {
    let id = format!("{group}/{name}");
    let file = format!("{group}/{name}.toml");
    let sha256 = approved::hash(text.as_bytes());
    let (mut net, mut problems) = match jobnet::parse(text) {
        Ok(n) => (Some(n), Vec::new()),
        Err(p) => (None, p),
    };
    let mut calendar = None;
    if let Some(n) = &net {
        let mut bad = |message: String| problems.push(Problem { line: 0, message });
        match shared {
            Ok(g) => {
                calendar = n.calendar.clone().or_else(|| g.calendar.clone());
                match &calendar {
                    Some(c) => {
                        if let Some(why) = cals.problem(c) {
                            bad(why);
                        }
                    }
                    None if n.needs_calendar() => bad(msg::line(msg::DEF_NEEDS_CALENDAR, &[])),
                    None => {}
                }
            }
            Err(e) => bad(e.clone()),
        }
    }
    if !problems.is_empty() {
        net = None;
    }
    let standing = if net.is_none() {
        Standing::Invalid
    } else if store.is_approved(&id, &sha256) {
        Standing::Confirmed
    } else {
        Standing::Unconfirmed
    };
    let previous = (standing == Standing::Unconfirmed).then(|| store.get(&id).map(|e| e.text.clone())).flatten();
    let title = net.as_ref().and_then(|n| n.name.clone());
    Def {
        id,
        group: group.to_owned(),
        name: name.to_owned(),
        title,
        file,
        standing,
        problems,
        net,
        sha256,
        text: text.to_owned(),
        previous,
        calendar,
    }
}

/// [`scan_in`] with no calendars: a net that names one is invalid.
pub fn scan(dir: &Path, store: &Store) -> Vec<Def> {
    scan_in(dir, store, &Calendars::default())
}

/// Every `<dir>/<group>/<name>.toml` (not `_group.toml`), sorted by group then
/// name; a net whose child jobnets (`kind = "jobnet"`) are missing, invalid
/// or call each other round is invalid too ([`link`]).
pub fn scan_in(dir: &Path, store: &Store, cals: &Calendars) -> Vec<Def> {
    let mut defs = scan_plain(dir, store, cals);
    link(&mut defs);
    defs
}

/// What a `kind = "jobnet"` part of `net` cannot use among `defs`: a `use`
/// that is not there, one with a definition error, or a call that comes
/// round to itself. `None` when every child can be used.
pub fn child_problem(net: &Jobnet, defs: &[Def]) -> Option<String> {
    let lookup = |u: &str| -> Result<Jobnet, String> {
        match defs.iter().find(|d| d.id == u) {
            None => Err(msg::line(msg::CHILD_MISSING, &[u])),
            Some(d) => match &d.net {
                Some(n) => Ok(n.clone()),
                None => Err(msg::line(msg::CHILD_INVALID, &[u])),
            },
        }
    };
    jobnet::expand(net, &lookup).err()
}

/// What the `[[when]] after_jobnet` lines of `net` (the net `id`) cannot
/// follow among `defs`: a net that is not there, or a chain that comes round
/// to `id` (A after B, B after A). `None` when each can be followed. A net
/// before with a definition error is no problem here: it just never runs.
pub fn after_problem(id: &str, net: &Jobnet, defs: &[Def]) -> Option<String> {
    let befores = |net: &Jobnet| -> Vec<String> {
        net.when
            .iter()
            .filter_map(|w| match &w.kind {
                Kind::AfterJobnet { jobnet, .. } => Some(jobnet.clone()),
                _ => None,
            })
            .collect()
    };
    for before in befores(net) {
        if !defs.iter().any(|d| d.id == before) {
            return Some(msg::line(msg::BEFORE_MISSING, &[&before]));
        }
    }
    // Walks the nets before, before those, …; a way back to `id` is a loop.
    fn walk(at: &str, target: &str, path: &mut Vec<String>, befores: &dyn Fn(&str) -> Vec<String>) -> Option<Vec<String>> {
        for b in befores(at) {
            if b == target {
                let mut p = path.clone();
                p.push(b);
                return Some(p);
            }
            // A loop that does not pass `target` is that net's own problem.
            if path.contains(&b) {
                continue;
            }
            path.push(b.clone());
            if let Some(p) = walk(&b, target, path, befores) {
                return Some(p);
            }
            path.pop();
        }
        None
    }
    let of = |i: &str| defs.iter().find(|d| d.id == i).and_then(|d| d.net.as_ref()).map(befores).unwrap_or_default();
    let mut path = vec![id.to_owned()];
    walk(id, id, &mut path, &of).map(|p| msg::line(msg::BEFORE_LOOP, &[&p.join(" → ")]))
}

/// Marks each net whose child jobnets cannot be used, or whose `after_jobnet`
/// cannot be followed, as invalid, with why (a line-0 problem).
pub fn link(defs: &mut [Def]) {
    let bad: Vec<(usize, String)> = defs
        .iter()
        .enumerate()
        .filter_map(|(i, d)| d.net.as_ref().and_then(|n| child_problem(n, defs).or_else(|| after_problem(&d.id, n, defs))).map(|m| (i, m)))
        .collect();
    for (i, message) in bad {
        let d = &mut defs[i];
        d.problems.push(Problem { line: 0, message });
        d.net = None;
        d.standing = Standing::Invalid;
    }
}

/// [`scan_in`] without the look at the child jobnets.
pub fn scan_plain(dir: &Path, store: &Store, cals: &Calendars) -> Vec<Def> {
    let mut found: Vec<(String, String, PathBuf, Result<Group, String>)> = Vec::new();
    for group in subdirs(dir) {
        let group_name = file_name(&group);
        let shared = group::load(&group.join("_group.toml"));
        let Ok(files) = std::fs::read_dir(&group) else { continue };
        for f in files.flatten() {
            let p = f.path();
            let name = file_name(&p);
            let Some(stem) = name.strip_suffix(".toml") else { continue };
            if stem.is_empty() || stem == "_group" || !p.is_file() {
                continue;
            }
            found.push((group_name.clone(), stem.to_owned(), p, shared.clone()));
        }
    }
    found.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    found.into_iter().map(|(g, n, p, shared)| read_def_in(&p, &g, &n, store, shared.as_ref(), cals)).collect()
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET: &str = "format = 1\nname = \"月次締め\"\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n";

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("koyomi-core-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(dir: &Path, group: &str, name: &str, text: &str) {
        std::fs::create_dir_all(dir.join(group)).unwrap();
        std::fs::write(dir.join(group).join(name), text).unwrap();
    }

    #[test]
    fn groups_and_names_come_from_the_folders_and_start_unconfirmed() {
        let d = tmp("tree");
        put(&d, "経理", "月次締め.toml", NET);
        put(&d, "共通", "バックアップ.toml", NET);
        put(&d, "共通", "_group.toml", "format = 1\n");
        put(&d, "共通", "notes.txt", "x");
        let defs = scan(&d, &Store::default());
        let ids: Vec<_> = defs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["共通/バックアップ", "経理/月次締め"]);
        assert!(defs.iter().all(|d| d.standing == Standing::Unconfirmed));
        assert_eq!(defs[1].label(), "月次締め");
        assert_eq!(defs[1].file, "経理/月次締め.toml");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn approval_holds_until_the_bytes_change_even_by_a_comment() {
        let d = tmp("approve");
        put(&d, "経理", "月次締め.toml", NET);
        let mut store = Store::default();
        let first = &scan(&d, &store)[0];
        store.approve(&first.id, &first.sha256, &first.text);
        assert_eq!(scan(&d, &store)[0].standing, Standing::Confirmed);

        put(&d, "経理", "月次締め.toml", &format!("{NET}# memo\n"));
        let now = &scan(&d, &store)[0];
        assert_eq!(now.standing, Standing::Unconfirmed);
        assert_eq!(now.previous.as_deref(), Some(NET), "the approved text is what the next approval compares with");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_that_does_not_read_is_invalid_and_never_unconfirmed() {
        let d = tmp("invalid");
        put(&d, "経理", "壊れ.toml", "format = 1\n[[job]]\nid = \"a\"\nrnu = []\n");
        let defs = scan(&d, &Store::default());
        assert_eq!(defs[0].standing, Standing::Invalid);
        assert_eq!(defs[0].problems[0].in_file(&defs[0].file), "経理/壊れ.toml の 4 行目: 知らない欄 rnu");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_approved_file_that_later_breaks_is_invalid() {
        let d = tmp("broken");
        put(&d, "g", "n.toml", NET);
        let mut store = Store::default();
        let first = &scan(&d, &store)[0];
        store.approve(&first.id, &first.sha256, &first.text);
        put(&d, "g", "n.toml", "format = 1\n[[job]]\nid = \"a\"\nrun = []\n");
        assert_eq!(scan(&d, &store)[0].standing, Standing::Invalid);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_calendar_comes_from_the_net_or_the_group_and_must_exist() {
        let d = tmp("calendar");
        let cal_dir = tmp("calendar-cals");
        std::fs::write(cal_dir.join("会社.toml"), "format = 1\nholidays = \"japan\"\n").unwrap();
        let cals = Calendars::load(&cal_dir);
        put(&d, "経理", "_group.toml", "format = 1\ncalendar = \"会社\"\n");
        put(&d, "経理", "a.toml", NET);
        let with = |c: &str| NET.replace("format = 1\n", &format!("format = 1\ncalendar = \"{c}\"\n"));
        const FIFTH: &str = "[[when]]\nevery = \"month\"\nbusiness_day = 5\nat = [\"09:00\"]\n";
        put(&d, "経理", "b.toml", &with("工場"));
        put(&d, "営業", "c.toml", &format!("{NET}{FIFTH}"));
        put(&d, "営業", "d.toml", &format!("{}{FIFTH}", with("会社")));
        put(&d, "壊れ", "_group.toml", "x = 1\n");
        put(&d, "壊れ", "e.toml", NET);
        let defs = scan_in(&d, &Store::default(), &cals);
        let by = |id: &str| defs.iter().find(|d| d.id == id).unwrap();
        assert_eq!(by("経理/a").calendar.as_deref(), Some("会社"), "the group's calendar");
        assert_eq!(by("経理/a").standing, Standing::Unconfirmed);
        assert_eq!(by("経理/b").standing, Standing::Invalid);
        let shown = by("経理/b").problems[0].in_file("経理/b.toml");
        // The calendar's own coded line stays at the head, the file it was found in goes last.
        assert_eq!(shown, format!("{}（経理/b.toml）", crate::msg::line(crate::msg::CAL_MISSING, &["工場"])));
        assert!(msg::DEF_NEEDS_CALENDAR.is_in(&by("営業/c").problems[0].message));
        assert_eq!(by("営業/d").standing, Standing::Unconfirmed);
        assert_eq!(by("壊れ/e").standing, Standing::Invalid);
        assert!(
            msg::GROUP_FILE_UNKNOWN_KEY.is_in(&by("壊れ/e").problems[0].message)
                && by("壊れ/e").problems[0].message.contains("未知の項目 x")
        );
        assert_eq!(
            scan(&d, &Store::default()).iter().find(|d| d.id == "経理/a").unwrap().standing,
            Standing::Invalid,
            "without calendars a named one is missing"
        );
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&cal_dir);
    }

    #[test]
    fn an_after_jobnet_must_be_there_and_must_not_come_round() {
        let d = tmp("after");
        let after = |u: &str| format!("format = 1\n[[when]]\nafter_jobnet = \"{u}\"\n[[job]]\nid = \"a\"\nrun = [\"x.exe\"]\n");
        put(&d, "g", "a.toml", &after("g/b"));
        put(&d, "g", "b.toml", &after("g/c"));
        put(&d, "g", "c.toml", &after("g/a"));
        put(&d, "g", "d.toml", &after("g/none"));
        put(&d, "g", "e.toml", &after("g/a"));
        put(&d, "g", "f.toml", &after("g/f"));
        let defs = scan(&d, &Store::default());
        let of = |id: &str| {
            let x = defs.iter().find(|x| x.id == id).unwrap();
            (x.standing, x.problems.iter().map(|p| p.message.clone()).collect::<Vec<_>>())
        };
        assert_eq!(of("g/a"), (Standing::Invalid, vec![msg::line(msg::BEFORE_LOOP, &["g/a → g/b → g/c → g/a"])]));
        assert_eq!(of("g/b"), (Standing::Invalid, vec![msg::line(msg::BEFORE_LOOP, &["g/b → g/c → g/a → g/b"])]));
        assert_eq!(of("g/d"), (Standing::Invalid, vec![msg::line(msg::BEFORE_MISSING, &["g/none"])]));
        assert_eq!(of("g/f"), (Standing::Invalid, vec![msg::line(msg::BEFORE_LOOP, &["g/f → g/f"])]));
        // e follows a, which is in a loop; e itself is not: a fine net (a will never run).
        assert_eq!(of("g/e"), (Standing::Unconfirmed, vec![]));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_folder_is_an_empty_list() {
        assert!(scan(Path::new("Z:/koyomi-no-such-dir"), &Store::default()).is_empty());
    }
}
