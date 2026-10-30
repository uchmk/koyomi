//! The icon in the task tray (DECISIONS C1): the engine's state while the window is closed.
//!
//! Its badge is the worst of today (異常, then スケジュールを止めている, then 実行中), its tip
//! counts them and names the next time, and its menu opens the window, opens 今日, pauses the
//! schedule and ends the engine. The window is brought forward through its own door
//! (`proto::window_door`), or started when it does not answer.
//!
//! `koyomi tray` ([`apart`]) is the same icon in a process of its own, for an engine that
//! has none (`koyomi engine --no-tray`, a service): it asks the engine for the look and the
//! notices kept for it, shows those notices on this desktop, and its menu closes only itself.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use ito_tray::{Event, Icon, Item, State};
use koyomi_core::run::{Run, State as RunState};

use ito_prefs::NotifyCfg;

use super::Paths;
use crate::notify;
use crate::proto::{self, Request, Show, TrayView};

/// How often the badge and the tip are worked out again.
const LOOK: Duration = Duration::from_secs(1);
/// How long a window just started is waited for, to be told to show 今日.
const SHOW_WAIT: Duration = Duration::from_secs(15);
/// How often `koyomi tray` reads `[notify]` again (what it shows and sounds).
const REREAD: Duration = Duration::from_secs(30);

/// What the tray shows: the badge, the tip and whether the menu offers to pause or resume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Look {
    pub state: State,
    pub tip: String,
    pub paused: bool,
    /// Job processes running: the engine is not ended from the menu while there are.
    pub busy: usize,
}

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
        if old.as_ref().map(|o| (o.paused, o.busy > 0)) != Some((look.paused, look.busy > 0)) {
            self.tray.set_menu(menu(t, look.paused, look.busy, self.apart));
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

/// The right-click menu. 終了 is greyed while a job's process runs: ending the engine would leave it unwatched.
/// `apart` (`koyomi tray`): it is 閉じる instead, which leaves the engine running.
pub fn menu(t: &ito_i18n::Table, paused: bool, busy: usize, apart: bool) -> Vec<Item> {
    let pause = if paused { Item::entry("resume", t.tr("tray.resume")) } else { Item::entry("pause", t.tr("tray.pause")) };
    let quit = if apart {
        Item::entry("quit", t.tr("tray.close"))
    } else if busy > 0 {
        Item::Entry { id: "quit".to_owned(), label: t.trf("tray.quit_busy", &[&busy.to_string()]), enabled: false, checked: false }
    } else {
        Item::entry("quit", t.tr("tray.quit"))
    };
    vec![Item::entry("open", t.tr("tray.open")), Item::entry("today", t.tr("tray.today")), Item::Separator, pause, Item::Separator, quit]
}

/// The look of today's runs: a net whose last run of today ended 異常 counts as one 異常
/// (a later good run clears it), and what is still going counts as running.
pub fn look(t: &ito_i18n::Table, runs: &[Run], paused: bool, busy: usize, next: Option<NaiveDateTime>) -> Look {
    let mut last: BTreeMap<&str, &Run> = BTreeMap::new();
    // A time that never ran (held, missed) neither raises nor clears 異常.
    for r in runs.iter().filter(|r| r.is_over() && (r.state.is_error() || r.state.passed())) {
        let e = last.entry(r.jobnet.as_str()).or_insert(r);
        if r.started >= e.started {
            *e = r;
        }
    }
    let errors = last.values().filter(|r| r.state.is_error()).count();
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
    let mut parts = vec!["koyomi".to_owned()];
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
    if parts.len() == 1 {
        parts.push(t.tr("tray.quiet").to_owned());
    }
    Look { state, tip: parts.join(" · "), paused, busy }
}

/// `koyomi tray`: the exit code is 0 when closed from the menu, 1 when another
/// `koyomi tray` is up, there is no tray here, or the engine has its own.
pub fn apart() -> i32 {
    let _single = match ito_common::single::acquire("koyomi", "tray") {
        Ok(g) => g,
        Err(held) => {
            crate::say(&format!("koyomi: トレイはもう動いている ({held})"), true);
            return 1;
        }
    };
    let t = crate::cli::table();
    let Some(mut tray) = Tray::start(&t) else {
        crate::say("koyomi: ここではタスク トレイが使えない", true);
        return 1;
    };
    tray.apart = true;
    let mut since = None;
    let mut cfg: Option<(NotifyCfg, Instant)> = None;
    loop {
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
                    crate::say("koyomi: エンジンが自分のトレイを持っている（koyomi engine --no-tray のときに使う）", true);
                    return 1;
                }
                Some(v) => {
                    if !v.notes.is_empty() {
                        if cfg.as_ref().is_none_or(|(_, at)| at.elapsed() >= REREAD) {
                            cfg = Some((notify_cfg(), Instant::now()));
                        }
                        if let Some((n, _)) = &cfg {
                            for note in &v.notes {
                                notify::tell_here(n, note);
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
        Some(v) => Look { state: from_badge(&v.badge).unwrap_or(State::Done), tip: v.tip.clone(), paused: v.paused, busy: v.busy },
        None => Look { state: State::Disabled, tip: format!("koyomi · {}", t.tr("tray.no_engine")), paused: false, busy: 0 },
    }
}

/// `koyomi.toml`'s `[notify]` as `koyomi tray` uses it: only what it shows and sounds.
fn notify_cfg() -> NotifyCfg {
    let config = Paths::find().ok().and_then(|p| p.config().ok()).unwrap_or_default();
    notify::read(&config).0
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
        // A window just started opens on its own; 今日 needs the door once it is up.
        if show == Show::Today {
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
        r.state = state;
        r.ended = state.is_final().then_some(started);
        r
    }

    fn ja() -> ito_i18n::Table {
        crate::i18n::table("ja").0
    }

    /// the tray shows 異常 for a net whose last run of today failed, and a later good run clears it.
    #[test]
    fn a_later_good_run_clears_the_error() {
        let t = ja();
        let failed = vec![run("a/x", at(8, 0), RunState::Error), run("a/y", at(8, 5), RunState::Ok)];
        let l = look(&t, &failed, false, 0, Some(at(9, 30)));
        assert_eq!(l.state, State::Error);
        assert_eq!(l.tip, "koyomi · 異常 1 · 次 09:30");
        let mut cleared = failed.clone();
        cleared.push(run("a/x", at(8, 30), RunState::Ok));
        let l = look(&t, &cleared, false, 0, None);
        assert_eq!(l.state, State::Done);
        assert_eq!(l.tip, "koyomi · 異常なし");
    }

    /// running and paused have their own badge and words; 異常 still wins over both.
    #[test]
    fn running_and_paused_show_in_the_badge_and_the_tip() {
        let t = ja();
        let going = vec![run("a/x", at(8, 0), RunState::Running)];
        let l = look(&t, &going, false, 1, Some(at(9, 0)));
        assert_eq!((l.state, l.tip.as_str()), (State::Running, "koyomi · 実行中 1 · 次 09:00"));
        let l = look(&t, &going, true, 1, Some(at(9, 0)));
        assert_eq!((l.state, l.tip.as_str()), (State::Disabled, "koyomi · 実行中 1 · スケジュールを止めている"));
        let mut both = going.clone();
        both.push(run("a/y", at(7, 0), RunState::Timeout));
        assert_eq!(look(&t, &both, true, 1, None).state, State::Error);
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
        let m = ids(&menu(&t, false, 0, false));
        assert_eq!(m.iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>(), ["open", "today", "pause", "quit"]);
        assert!(m.iter().all(|(_, on)| *on));
        let m = ids(&menu(&t, true, 2, false));
        assert_eq!(m[2].0, "resume");
        assert_eq!(m[3], ("quit".to_owned(), false));
    }

    /// `koyomi tray`'s menu closes only itself, so it is never greyed, and says so.
    #[test]
    fn the_tray_apart_closes_only_itself() {
        let t = ja();
        let items = menu(&t, false, 3, true);
        let Some(Item::Entry { id, label, enabled, .. }) = items.last() else { panic!("no last entry") };
        assert_eq!((id.as_str(), label.as_str(), *enabled), ("quit", "トレイを閉じる（エンジンは動き続ける）", true));
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
        assert_eq!((l.state, l.tip.as_str()), (State::Disabled, "koyomi · エンジンは動いていない"));
        let v = TrayView { badge: "error".into(), tip: "koyomi · 異常 1".into(), busy: 1, ..TrayView::default() };
        let l = apart_look(&t, Some(&v));
        assert_eq!((l.state, l.tip.as_str(), l.busy), (State::Error, "koyomi · 異常 1", 1));
    }
}
