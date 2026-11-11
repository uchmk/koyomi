//! The icon in the task tray (DECISIONS C1): the engine's state while the window is closed.
//!
//! Its badge is the worst of today (異常, then スケジュールを止めている, then 実行中), its tip
//! counts them and names the next time, and its menu opens the window, opens 今日 or the newest
//! failure (未対応を見る), pauses the schedule and ends the engine. The window is brought forward through its own door
//! (`proto::window_door`), or started when it does not answer.
//!
//! `koyomi tray` ([`apart`]) is the same icon in a process of its own, for an engine that
//! has none (`koyomi engine --no-tray`, a service): it asks the engine for the look and the
//! notices kept for it, shows those notices on this desktop, and its menu closes only itself.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use ito_tray::{Event, Icon, Item, State};
use koyomi_core::msg;
use koyomi_core::run::{Run, State as RunState};

use ito_prefs::NotifyCfg;

use crate::notify;
use crate::proto::{self, Request, Show, TrayView};

/// How often the badge and the tip are worked out again.
const LOOK: Duration = Duration::from_secs(1);
/// How long a window just started is waited for, to be told to show 今日.
const SHOW_WAIT: Duration = Duration::from_secs(15);
/// How often `koyomi tray` reads `[notify]` again (what it shows and sounds).
const REREAD: Duration = notify::REREAD;

/// What the tray shows: the badge, the tip and whether the menu offers to pause or resume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Look {
    pub state: State,
    pub tip: String,
    pub paused: bool,
    /// Job processes running: the engine is not ended from the menu while there are.
    pub busy: usize,
    /// Runs of today that failed and are not marked 対応済み: 未対応を見る (N) is offered while there are.
    pub errors: usize,
    /// The newest of those, up to `LISTED`: `(run id, menu label)`.
    pub fails: Vec<(String, String)>,
    /// How many more there are than `fails` lists.
    pub more: usize,
}

/// Failed runs the menu lists by name; the rest are "ほか N 件…".
const LISTED: usize = 5;

/// What a click or a menu choice asks the engine to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    Pause(bool),
    Quit,
}

pub struct Tray {
    tray: ito_tray::Tray,
    shown: Option<Look>,
    looked: Option<Instant>,
    /// `koyomi tray`: the menu closes the tray, not the engine.
    apart: bool,
}

impl Tray {
    /// Puts the icon in the tray; `None` where there is no tray (not Windows, a service session).
    pub fn start(t: &ito_i18n::Table) -> Option<Tray> {
        let side = 32;
        let icon = Icon::new(side, side, crate::icon::pixels(side))?;
        let tray = ito_tray::Tray::new(icon, t.tr("tray.starting")).ok()?;
        Some(Tray { tray, shown: None, looked: None, apart: false })
    }

    /// Whether it is time to work out the look again.
    pub fn due(&self) -> bool {
        self.looked.is_none_or(|t| t.elapsed() >= LOOK)
    }

    /// Shows `look`, touching the icon only where it changed.
    pub fn show(&mut self, t: &ito_i18n::Table, look: Look) {
        self.looked = Some(Instant::now());
        if self.shown.as_ref() == Some(&look) {
            return;
        }
        let old = self.shown.take();
        if old.as_ref().map(|o| o.state) != Some(look.state) {
            self.tray.set_state(Some(look.state));
        }
        if old.as_ref().map(|o| &o.tip) != Some(&look.tip) {
            self.tray.set_tip(&look.tip);
        }
        if old.as_ref().map(|o| (&o.tip, o.paused, o.busy > 0, o.errors, &o.fails, o.more))
            != Some((&look.tip, look.paused, look.busy > 0, look.errors, &look.fails, look.more))
        {
            self.tray.set_menu(menu(t, &look, self.apart));
        }
        self.shown = Some(look);
    }

    /// The clicks and choices since the last call: the window ones are done here, the rest handed back.
    pub fn take(&self) -> Vec<Want> {
        let mut out = Vec::new();
        while let Ok(e) = self.tray.events().try_recv() {
            match e {
                Event::Click => front(Show::Front),
                Event::Menu(id) => match id.as_str() {
                    "open" => front(Show::Front),
                    "today" => front(Show::Today),
                    "failed" | "more" => front(Show::Failed),
                    other if other.starts_with("run:") => front(Show::Run(other["run:".len()..].to_owned())),
                    "skipnext" => front(Show::SkipNext),
                    "pause" => out.push(Want::Pause(true)),
                    "resume" => out.push(Want::Pause(false)),
                    "quit" => out.push(Want::Quit),
                    _ => {}
                },
            }
        }
        out
    }
}

/// The right-click menu, under a greyed line with the tip's counts. 終了 is greyed while a job's process
/// runs: ending the engine would leave it unwatched.
/// `apart` (`koyomi tray`): it is 閉じる instead, which leaves the engine running.
/// 未対応を見る (N) is greyed while no failure of today waits.
pub fn menu(t: &ito_i18n::Table, look: &Look, apart: bool) -> Vec<Item> {
    let (paused, busy) = (look.paused, look.busy);
    let head = Item::Entry { id: "head".to_owned(), label: look.tip.clone(), enabled: false, checked: false };
    let pause = if paused { Item::entry("resume", t.tr("tray.resume")) } else { Item::entry("pause", t.tr("tray.pause")) };
    let quit = if apart {
        Item::entry("quit", t.tr("tray.close"))
    } else if busy > 0 {
        Item::Entry { id: "quit".to_owned(), label: t.trf("tray.quit_busy", &[&busy.to_string()]), enabled: false, checked: false }
    } else {
        Item::entry("quit", t.tr("tray.quit"))
    };
    let failed = Item::Entry {
        id: "failed".to_owned(),
        label: t.trf("tray.failed", &[&look.errors.to_string()]),
        enabled: look.errors > 0,
        checked: false,
    };
    let mut items = vec![head, Item::Separator, Item::entry("open", t.tr("tray.open")), Item::entry("today", t.tr("tray.today")), failed];
    for (id, label) in &look.fails {
        items.push(Item::entry(&format!("run:{id}"), label));
    }
    if look.more > 0 {
        items.push(Item::entry("more", &t.trf("tray.more", &[&look.more.to_string()])));
    }
    items.extend([Item::Separator, Item::entry("skipnext", t.tr("tray.skip_next")), pause, Item::Separator, quit]);
    items
}

/// The look of today's runs: every run that ended 異常 and is not marked 対応済み (`acked`, the run ids
/// in `acked.toml`) counts as one 異常, the same count as the window's 未対応; what is still going
/// counts as running.
pub fn look(t: &ito_i18n::Table, runs: &[Run], acked: &BTreeSet<String>, paused: bool, busy: usize, next: Option<NaiveDateTime>) -> Look {
    let mut last: BTreeMap<&str, &Run> = BTreeMap::new();
    // A time that never ran (held, missed) neither raises nor clears a warning.
    for r in runs.iter().filter(|r| r.is_over() && (r.state.is_error() || r.state.passed())) {
        let e = last.entry(r.jobnet.as_str()).or_insert(r);
        if r.started >= e.started {
            *e = r;
        }
    }
    let errors = runs.iter().filter(|r| r.is_over() && r.state.is_error() && !acked.contains(&r.id)).count();
    let mut failed: Vec<&Run> = runs.iter().filter(|r| r.is_over() && r.state.is_error() && !acked.contains(&r.id)).collect();
    failed.sort_by_key(|a| std::cmp::Reverse(a.started));
    let fails: Vec<(String, String)> =
        failed.iter().take(LISTED).map(|r| (r.id.clone(), format!("✕ {} {}", r.title, r.started.format("%H:%M")))).collect();
    let more = errors.saturating_sub(fails.len());
    let warns = last.values().filter(|r| r.state == RunState::Warn).count();
    let running = runs.iter().filter(|r| !r.is_over()).count();
    let state = if errors > 0 {
        State::Error
    } else if paused {
        State::Disabled
    } else if running > 0 {
        State::Running
    } else if warns > 0 {
        State::Warning
    } else {
        State::Done
    };
    let mut parts = Vec::new();
    if errors > 0 {
        parts.push(t.trf("tray.errors", &[&errors.to_string()]));
    }
    if running > 0 {
        parts.push(t.trf("tray.running", &[&running.to_string()]));
    }
    if paused {
        parts.push(t.tr("tray.paused").to_owned());
    } else if let Some(at) = next {
        parts.push(t.trf("tray.next", &[&at.format("%H:%M").to_string()]));
    }
    if parts.is_empty() {
        parts.push(t.tr("tray.quiet").to_owned());
    }
    Look { state, tip: format!("koyomi: {}", parts.join(t.tr("tray.sep"))), paused, busy, errors, fails, more }
}

/// `koyomi tray`: the exit code is 0 when closed from the menu, 1 when another
/// `koyomi tray` is up, there is no tray here, or the engine has its own.
pub fn apart() -> i32 {
    let _single = match ito_common::single::acquire("koyomi", "tray") {
        Ok(g) => g,
        Err(held) => {
            crate::say(&msg::line(msg::TRAY_ALREADY_RUNNING, &[&held.to_string()]), true);
            return 1;
        }
    };
    let t = crate::cli::table();
    let Some(mut tray) = Tray::start(&t) else {
        crate::say(&msg::line(msg::TRAY_UNAVAILABLE, &[]), true);
        return 1;
    };
    tray.apart = true;
    let mut since = None;
    let mut cfg: Option<(NotifyCfg, Instant)> = None;
    loop {
        follow_clicks();
        for want in tray.take() {
            match want {
                Want::Pause(on) => {
                    let _ = ito_ipc::door::ask(&proto::door(), if on { &Request::Pause } else { &Request::Resume });
                }
                Want::Quit => return 0,
            }
        }
        if tray.due() {
            let view = ito_ipc::door::ask(&proto::door(), &Request::Tray { since }).ok().and_then(|text| TrayView::parse(&text).ok());
            match view {
                Some(v) if v.own => {
                    crate::say(&msg::line(msg::ENGINE_HAS_TRAY, &[]), true);
                    return 1;
                }
                Some(v) => {
                    if !v.notes.is_empty() {
                        if cfg.as_ref().is_none_or(|(_, at)| at.elapsed() >= REREAD) {
                            cfg = Some((notify_cfg(), Instant::now()));
                        }
                        if let Some((n, _)) = &cfg {
                            for note in &v.notes {
                                notify::tell_here(n, note, t.tr("notify.open"));
                            }
                        }
                    }
                    since = Some(v.last);
                    tray.show(&t, apart_look(&t, Some(&v)));
                }
                None => tray.show(&t, apart_look(&t, None)),
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// What `koyomi tray` shows for the engine's answer; `None`: no engine answers.
fn apart_look(t: &ito_i18n::Table, view: Option<&TrayView>) -> Look {
    match view {
        Some(v) => Look {
            state: from_badge(&v.badge).unwrap_or(State::Done),
            tip: v.tip.clone(),
            paused: v.paused,
            busy: v.busy,
            errors: v.errors,
            fails: v.fails.iter().map(|f| (f.id.clone(), f.label.clone())).collect(),
            more: v.more,
        },
        None => Look {
            state: State::Disabled,
            tip: format!("koyomi: {}", t.tr("tray.no_engine")),
            paused: false,
            busy: 0,
            errors: 0,
            fails: Vec::new(),
            more: 0,
        },
    }
}

/// `koyomi.toml`'s `[notify]` as `koyomi tray` uses it: only what it shows and sounds.
fn notify_cfg() -> NotifyCfg {
    notify::here_cfg()
}

/// The badge as `proto::TrayView` names it.
pub fn badge(state: State) -> &'static str {
    match state {
        State::Error | State::Conflict => "error",
        State::Warning => "warning",
        State::Running | State::Focused | State::Selected => "running",
        State::Disabled => "paused",
        _ => "done",
    }
}

/// The badge `proto::TrayView` names; `None` for a name this one does not know.
pub fn from_badge(name: &str) -> Option<State> {
    Some(match name {
        "error" => State::Error,
        "warning" => State::Warning,
        "running" => State::Running,
        "paused" => State::Disabled,
        "done" => State::Done,
        _ => return None,
    })
}

/// The notices of runs clicked since the last call: the window comes forward on each run.
pub fn follow_clicks() {
    for tag in ito_prefs::take_clicks() {
        front(Show::Run(notify::run_of(&tag).to_owned()));
    }
}

/// Brings the window forward, or starts it and then asks; on a thread, so the engine does not wait.
pub fn front(show: Show) {
    std::thread::spawn(move || {
        if ito_ipc::door::ask(&proto::window_door(), &show).is_ok() {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        if ito_proc::spawn_detached(std::process::Command::new(exe)).is_err() {
            return;
        }
        // A window just started opens on its own; the rest (今日, a run…) need the door once it is up.
        if show != Show::Front {
            let until = Instant::now() + SHOW_WAIT;
            while Instant::now() < until {
                std::thread::sleep(Duration::from_millis(200));
                if ito_ipc::door::ask(&proto::window_door(), &show).is_ok() {
                    return;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use koyomi_core::run::How;

    fn at(h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 30).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    fn run(net: &str, started: NaiveDateTime, state: RunState) -> Run {
        let mut r = Run::with_jobs("id", net, net, "2026-10-30", How::Manual, Vec::new(), started);
        r.id = format!("{net}-{}", started.format("%H%M"));
        r.state = state;
        r.ended = state.is_final().then_some(started);
        r
    }

    fn ja() -> ito_i18n::Table {
        crate::i18n::table("ja").0
    }

    /// the tray counts every failed run of today not marked 対応済み, as the window does; marking clears it.
    #[test]
    fn the_error_count_is_the_windows_unhandled_count() {
        let t = ja();
        let failed =
            vec![run("a/x", at(8, 0), RunState::Error), run("a/x", at(8, 10), RunState::Error), run("a/y", at(8, 5), RunState::Ok)];
        let l = look(&t, &failed, &BTreeSet::new(), false, 0, Some(at(9, 30)));
        assert_eq!((l.state, l.errors), (State::Error, 2));
        assert_eq!(l.tip, "koyomi: 異常 2・次 09:30");
        // A later good run of the same net does not clear what was not dealt with.
        let mut later = failed.clone();
        later.push(run("a/x", at(8, 30), RunState::Ok));
        assert_eq!(look(&t, &later, &BTreeSet::new(), false, 0, None).errors, 2);
        // Marking one 対応済み takes it off; marking both clears the badge.
        let one: BTreeSet<String> = ["a/x-0800".to_owned()].into();
        assert_eq!(look(&t, &later, &one, false, 0, None).errors, 1);
        let both: BTreeSet<String> = ["a/x-0800".to_owned(), "a/x-0810".to_owned()].into();
        let l = look(&t, &later, &both, false, 0, None);
        assert_eq!((l.state, l.errors, l.tip.as_str()), (State::Done, 0, "koyomi: 異常なし"));
    }

    /// running and paused have their own badge and words; 異常 still wins over both.
    #[test]
    fn running_and_paused_show_in_the_badge_and_the_tip() {
        let t = ja();
        let going = vec![run("a/x", at(8, 0), RunState::Running)];
        let l = look(&t, &going, &BTreeSet::new(), false, 1, Some(at(9, 0)));
        assert_eq!((l.state, l.tip.as_str()), (State::Running, "koyomi: 実行中 1・次 09:00"));
        let l = look(&t, &going, &BTreeSet::new(), true, 1, Some(at(9, 0)));
        assert_eq!((l.state, l.tip.as_str()), (State::Disabled, "koyomi: 実行中 1・スケジュールを停止しています"));
        let mut both = going.clone();
        both.push(run("a/y", at(7, 0), RunState::Timeout));
        assert_eq!(look(&t, &both, &BTreeSet::new(), true, 1, None).state, State::Error);
    }

    /// the menu offers the opposite of the pause state, and greys 終了 while a job's process runs.
    #[test]
    fn the_menu_follows_pause_and_busy() {
        let t = ja();
        let ids = |items: &[Item]| -> Vec<(String, bool)> {
            items
                .iter()
                .filter_map(|i| match i {
                    Item::Entry { id, enabled, .. } => Some((id.clone(), *enabled)),
                    Item::Separator => None,
                })
                .collect()
        };
        let quiet = look(&t, &[], &BTreeSet::new(), false, 0, None);
        let items = menu(&t, &quiet, false);
        let Some(Item::Entry { label, .. }) = items.first() else { panic!("no head") };
        assert_eq!(label, "koyomi: 異常なし");
        let m = ids(&items);
        assert_eq!(m.iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>(), ["head", "open", "today", "failed", "skipnext", "pause", "quit"]);
        assert_eq!(m.iter().map(|(_, on)| *on).collect::<Vec<_>>(), [false, true, true, false, true, true, true]);
        let m = ids(&menu(&t, &look(&t, &[], &BTreeSet::new(), true, 2, None), false));
        assert_eq!(m[5].0, "resume");
        assert_eq!(m[6], ("quit".to_owned(), false));
        // 未対応を見る counts the failed runs of today not marked 対応済み, and opens once there is one.
        let failed = [run("a/x", at(8, 0), RunState::Error), run("a/y", at(8, 0), RunState::Error)];
        let items = menu(&t, &look(&t, &failed, &BTreeSet::new(), false, 0, None), false);
        let Some(Item::Entry { label, enabled, .. }) = items.get(4) else { panic!("no failed entry") };
        assert_eq!((label.as_str(), *enabled), ("未対応を見る (2)", true));
    }

    /// The menu lists the newest unhandled failures by name (at most five), then "ほか N 件…".
    #[test]
    fn the_menu_lists_the_failed_runs() {
        let t = ja();
        let runs: Vec<Run> = (0..7).map(|i| run(&format!("g/n{i}"), at(8 + i, 0), RunState::Error)).collect();
        let acked: BTreeSet<String> = ["g/n6-1400".to_owned()].into();
        let l = look(&t, &runs, &acked, false, 0, None);
        assert_eq!((l.errors, l.fails.len(), l.more), (6, 5, 1));
        assert_eq!(l.fails[0], ("g/n5-1300".to_owned(), "✕ g/n5 13:00".to_owned()));
        let items = menu(&t, &l, false);
        let ids: Vec<&str> = items.iter().filter_map(|i| if let Item::Entry { id, .. } = i { Some(id.as_str()) } else { None }).collect();
        assert_eq!(&ids[3..10], ["failed", "run:g/n5-1300", "run:g/n4-1200", "run:g/n3-1100", "run:g/n2-1000", "run:g/n1-0900", "more"]);
        let Some(Item::Entry { label, .. }) = items.iter().find(|i| matches!(i, Item::Entry { id, .. } if id == "more")) else { panic!() };
        assert_eq!(label, "ほか 1 件…");
        // Nothing failed: no list.
        assert!(look(&t, &[], &BTreeSet::new(), false, 0, None).fails.is_empty());
    }

    /// `koyomi tray`'s menu closes only itself, so it is never greyed, and says so.
    #[test]
    fn the_tray_apart_closes_only_itself() {
        let t = ja();
        let items = menu(&t, &look(&t, &[], &BTreeSet::new(), false, 3, None), true);
        let Some(Item::Entry { id, label, enabled, .. }) = items.last() else { panic!("no last entry") };
        assert_eq!((id.as_str(), label.as_str(), *enabled), ("quit", "トレイを閉じる（エンジンは起動したままです）", true));
    }

    /// the badge goes over the door by name and comes back the same; no engine shows grey.
    #[test]
    fn the_badge_reads_back_and_no_engine_is_grey() {
        for s in [State::Error, State::Warning, State::Running, State::Disabled, State::Done] {
            assert_eq!(from_badge(badge(s)), Some(s));
        }
        assert_eq!(from_badge("later"), None);
        let t = ja();
        let l = apart_look(&t, None);
        assert_eq!((l.state, l.tip.as_str()), (State::Disabled, "koyomi: エンジンは起動していません"));
        let v = TrayView { badge: "error".into(), tip: "koyomi: 異常 1".into(), busy: 1, errors: 1, ..TrayView::default() };
        let l = apart_look(&t, Some(&v));
        assert_eq!((l.state, l.tip.as_str(), l.busy, l.errors), (State::Error, "koyomi: 異常 1", 1, 1));
    }
}
