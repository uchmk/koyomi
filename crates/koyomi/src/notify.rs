//! Telling of a run that ended: the system's notification, the sound, the
//! webhooks and the mails, as `[notify]` in `koyomi.toml` turns them on
//! (ito-prefs's shape, the same table every uchmk app reads).
//!
//! Nothing written: only 異常 is told, through the system's notification, and
//! without a sound. The way a run was started (a time, the window, the
//! console) changes nothing here. A webhook address is kept in the OS
//! credential store (`secret:<name>`); one written in plain text is not sent
//! to until `hide_webhooks = false` says that is meant. A mail's password is
//! only ever `secret:<name>`.

use std::collections::VecDeque;
use std::sync::Mutex;

use chrono::{Duration, NaiveDateTime};

use crate::proto::Note;
use ito_i18n::Table;
use ito_notify::toast::App;
use ito_prefs::{Event, NotifyCfg, DONE_FAILED};
use koyomi_core::config::Config;
use koyomi_core::jobnet::Jobnet;
use koyomi_core::late::{self, Delay};
use koyomi_core::run::{Run, State};

use crate::show;

pub const APP: App<'static> = App { id: "uchmk.koyomi", name: "koyomi" };
/// The events a run can be told of: `done` (off unless written) and `failed`.
pub const EVENTS: [Event; 2] = DONE_FAILED;
/// The one event told when nothing is written.
pub const QUIET: &str = "failed";
/// Where `[notify]` is read from, as the notes name it.
const PATH: &str = "koyomi.toml";
/// Said of a plain-text webhook address while `hide_webhooks` is on.
pub fn plain() -> String {
    koyomi_core::msg::line(koyomi_core::msg::NOTIFY_PLAIN, &[])
}

fn icon(side: u32) -> Vec<u8> {
    crate::icon::pixels(side)
}

/// Why the text of a `koyomi.toml` would not pass `koyomi check`: its first problem, on one line.
pub fn settings_problem(text: &str) -> Option<String> {
    let first = |s: &str| s.lines().next().unwrap_or_default().trim().to_owned();
    match Config::parse(text) {
        Err(e) => Some(first(&e)),
        Ok(c) => parse(&c).1.first().map(|n| first(n)),
    }
}

/// [`koyomi_core::bundle::export`], refusing settings that `koyomi check` would refuse.
pub fn export(src: &koyomi_core::bundle::Source, pick: &koyomi_core::bundle::Pick) -> Result<koyomi_core::bundle::Bundle, String> {
    if pick.settings || pick.all {
        if let Some(e) = src.settings.as_deref().and_then(settings_problem) {
            return Err(koyomi_core::msg::line(koyomi_core::msg::NOTIFY_EXPORT_BAD, &[koyomi_core::msg::uncoded(&e)]));
        }
    }
    koyomi_core::bundle::export(src, pick)
}

/// `[notify]` of `config` with koyomi's defaults where nothing is written, and
/// the lines a person should see (an unknown event, a plain-text address).
/// The `secret:` addresses are not read back here: see [`NotifyCfg::reveal`].
/// This is what sends: a plain-text address under `hide_webhooks` is taken
/// out, so nothing goes there. The settings page reads with [`parse`].
pub fn read(config: &Config) -> (NotifyCfg, Vec<String>) {
    let (mut n, notes) = parse(config);
    if n.hide_webhooks() {
        // The note is already there, from `parse`.
        for p in &mut n.places {
            if ito_notify::is_http(&p.url) {
                p.url.clear();
            }
        }
    }
    (n, notes)
}

/// Like [`read`], but a plain-text address under `hide_webhooks` stays (the
/// settings' Notifications page shows it, and its first write moves it to
/// the credential store, as `hide_webhooks` means). The note is given all
/// the same.
pub fn parse(config: &Config) -> (NotifyCfg, Vec<String>) {
    let mut notes = Vec::new();
    let mut n: NotifyCfg = match toml::Value::Table(config.notify.clone()).try_into() {
        Ok(n) => n,
        Err(e) => {
            let m = e.message();
            notes.push(if m.contains("expected struct HookCfg") {
                koyomi_core::msg::line(koyomi_core::msg::NOTIFY_WEBHOOK_SHAPE, &[])
            } else {
                koyomi_core::msg::line(koyomi_core::msg::NOTIFY_SHAPE, &[m])
            });
            NotifyCfg::default()
        }
    };
    let quiet = || vec![QUIET.to_owned()];
    n.system.get_or_insert_with(quiet);
    n.webhook_states.get_or_insert_with(quiet);
    for h in &mut n.webhooks {
        h.states.get_or_insert_with(quiet);
    }
    for m in &mut n.mails {
        m.states.get_or_insert_with(quiet);
    }
    n.hide_webhooks.get_or_insert(true);
    n.check(&EVENTS, PATH, &mut notes);
    if n.hide_webhooks() && n.places.iter().any(|p| ito_notify::is_http(&p.url)) {
        notes.push(plain());
    }
    (n, notes)
}

/// "Send a test" on the settings' Notifications page, as the page sets
/// things now (`p`): every way, or only the webhook `what` names.
pub fn test(p: &ito_prefs::Notify, what: ito_prefs::Test, title: &str, body: &str) {
    ito_prefs::test_notify(&EVENTS, p, what, APP, icon, title, body);
}

/// The event a run that ended is, and the title and body told of it: the
/// state's symbol and word with the net's name, then the jobs that went wrong
/// (or the run's line when none did), the id and the business day.
pub fn words(t: &Table, r: &Run) -> (&'static str, String, String) {
    let event = if r.state.is_error() { "failed" } else { "done" };
    let late = if r.is_late() {
        koyomi_core::msg::note(koyomi_core::msg::N_NOTIFY_LATE, &[&koyomi_core::run::late_word()])
    } else {
        String::new()
    };
    let title = format!("{}{late}  {}", show::tag(t, r.state), r.title);
    let mut lines: Vec<String> = r.jobs.iter().filter(|j| j.state.is_error()).map(|j| show::job_line(t, j)).collect();
    if lines.is_empty() {
        lines.push(show::run_line(t, r));
    }
    lines.push(format!("{}  {}", r.id, r.date));
    (event, title, lines.join("\n"))
}

/// The run of the same net that ended just before `r` ended ok, if it ended
/// abnormally: what `r` has set right. `all` holds the runs known (`r` too).
pub fn recovered_from<'a>(all: &'a [Run], r: &Run) -> Option<&'a Run> {
    if !matches!(r.state, State::Ok | State::Warn) {
        return None;
    }
    let prev = all
        .iter()
        .filter(|p| {
            p.id != r.id && p.jobnet == r.jobnet && p.state.is_final() && !matches!(p.state, State::Held | State::Skipped | State::Missed)
        })
        .filter(|p| p.started < r.started)
        .max_by_key(|p| p.started)?;
    prev.state.is_error().then_some(prev)
}

/// The words for a net that ran well again after `prev` ended abnormally.
/// The event is `failed`: a person who is told of the trouble is told it is over.
pub fn recovery_words(t: &Table, r: &Run, prev: &Run) -> (&'static str, String, String) {
    let title = format!("{}  {}", t.tr("notify.recovered_title"), r.title);
    let body = [t.trf("notify.recovered", &[&r.title, &prev.id]), format!("{}  {}", r.id, r.date)].join(
        "
",
    );
    ("failed", title, body)
}

/// The words a `kind = "approval"` part sends when it begins to wait. The event is `failed`:
/// the one that is on when nothing is written, and a run that waits for a person is stuck.
pub fn ask_words(t: &Table, r: &Run, job: &str) -> (&'static str, String, String) {
    let label = r.job(job).map_or(job, |j| j.label());
    let title = format!("{}  {}: {label}", t.tr("notify.ask_title"), r.title);
    let body = [t.trf("notify.ask", &[&r.title, label]), format!("{}  {}", r.id, r.date)].join(
        "
",
    );
    ("failed", title, body)
}

/// The words a `kind = "notify"` part sends: the run as it stands, headed
/// `{state}  {title}: {part}`, the jobs that ended abnormally so far (or the
/// run's line), then the id and the day. The event is `failed` once a job
/// ended abnormally, else `done`.
pub fn part_words(t: &Table, r: &Run, job: &str) -> (&'static str, String, String) {
    let bad = r.jobs.iter().any(|j| j.state.is_error());
    let event = if bad { "failed" } else { "done" };
    let label = r.job(job).map_or(job, |j| j.label());
    let title = format!("{}  {}: {label}", show::tag(t, r.state), r.title);
    let mut lines: Vec<String> = r.jobs.iter().filter(|j| j.state.is_error()).map(|j| show::job_line(t, j)).collect();
    if lines.is_empty() {
        lines.push(show::run_line(t, r));
    }
    lines.push(format!("{}  {}", r.id, r.date));
    (event, title, lines.join("\n"))
}

/// `6:00`, or `10/31 6:00` when it is not the day of `now`.
fn moment(at: NaiveDateTime, now: NaiveDateTime) -> String {
    if at.date() == now.date() {
        at.format("%-H:%M").to_string()
    } else {
        at.format("%-m/%-d %-H:%M").to_string()
    }
}

/// The words for `r` of `net` being late in `delay` at `now`: `⏰ 遅れ  {title}`,
/// what is late, then the id and the day. `usual` is the net's usual time
/// (what made it [`Delay::Slow`]). The event is `failed`: a late run is
/// something to look at, and that is told when nothing is written.
pub fn delay_words(
    t: &Table,
    net: &Jobnet,
    r: &Run,
    delay: Delay,
    usual: Option<Duration>,
    now: NaiveDateTime,
) -> (&'static str, String, String) {
    let title = format!("{}  {}", t.tr("notify.late_title"), r.title);
    let line = match delay {
        Delay::Start => {
            let by = net.late_start.zip(r.planned).map_or_else(String::new, |(d, p)| moment(d.from(p), now));
            match late::begun(r) {
                Some(b) => t.trf("notify.late_start_began", &[&r.title, &by, &moment(b, now)]),
                None => t.trf("notify.late_start", &[&r.title, &by]),
            }
        }
        Delay::End => {
            let by = net.late_end.map_or_else(String::new, |d| moment(d.from(r.planned.unwrap_or(r.started)), now));
            t.trf("notify.late_end", &[&r.title, &by])
        }
        Delay::Miss => {
            let by = net.late_end.map_or_else(String::new, |d| moment(d.from(r.planned.unwrap_or(r.started)), now));
            let at = usual.map_or_else(String::new, |u| moment(r.started + u, now));
            t.trf("notify.late_miss", &[&r.title, &by, &at])
        }
        Delay::Slow => {
            let so_far = late::begun(r).map_or(Duration::zero(), |b| now - b);
            t.trf("notify.late_slow", &[&r.title, &show::duration(t, usual.unwrap_or_default()), &show::duration(t, so_far)])
        }
    };
    (
        "failed",
        title,
        [line, format!("{}  {}", r.id, r.date)].join(
            "
",
        ),
    )
}

/// The words for a time of `net_id` (titled `title`) at `planned` that has not
/// begun by `by` (`late_start`) and has no run yet: it waits for the last run
/// to end or for another net. The event is `failed`, as [`delay_words`].
pub fn pending_words(
    t: &Table,
    net_id: &str,
    title: &str,
    planned: NaiveDateTime,
    by: NaiveDateTime,
    now: NaiveDateTime,
) -> (&'static str, String, String) {
    let head = format!("{}  {title}", t.tr("notify.late_title"));
    let body = [t.trf("notify.late_start", &[title, &moment(by, now)]), format!("{net_id}  {}", moment(planned, now))].join(
        "
",
    );
    ("failed", head, body)
}

/// Splits off the runs that are 時刻を過ぎた, in groups of one net each (in
/// the order the nets first come), so a net whose times the PC slept through
/// is told of once. The rest keep their order.
pub fn group_missed(runs: Vec<Run>) -> (Vec<Run>, Vec<Vec<Run>>) {
    let mut rest = Vec::new();
    let mut missed: Vec<Vec<Run>> = Vec::new();
    for r in runs {
        if r.state != State::Missed {
            rest.push(r);
        } else if let Some(g) = missed.iter_mut().find(|g| g[0].jobnet == r.jobnet) {
            g.push(r);
        } else {
            missed.push(vec![r]);
        }
    }
    (rest, missed)
}

/// The words for the times of one net the PC slept through (`runs`, one net,
/// at least one): `経理/月次締め: 2:00 の回は時刻を過ぎました（PC が止まっていました）`,
/// then each run's id and day. The event is `failed`: a time that passed is
/// something to look at.
pub fn missed_words(t: &Table, runs: &[Run]) -> (&'static str, String, String) {
    let first = &runs[0];
    let title = format!("{}  {}", show::tag(t, State::Missed), first.title);
    let same_day = runs.iter().all(|r| r.date == first.date);
    let times: Vec<String> = runs
        .iter()
        .filter_map(|r| r.planned)
        .map(|p| if same_day { p.format("%-H:%M").to_string() } else { p.format("%-m/%-d %-H:%M").to_string() })
        .collect();
    let mut lines = vec![t.trf("notify.missed", &[&first.jobnet, &times.join("、")])];
    lines.extend(runs.iter().map(|r| format!("{}  {}", r.id, r.date)));
    ("failed", title, lines.join("\n"))
}

/// How many of the last lines of a failed job's log a webhook is sent.
pub const TAIL_LINES: usize = 10;
/// How long the one line the system's notification gets may be, in characters.
const LINE_CHARS: usize = 200;

/// The ends of the logs of the jobs that went wrong: one line for the
/// system's notification, the last [`TAIL_LINES`] of each for a webhook.
/// The logs are written with the secrets already as `***`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tail {
    pub line: String,
    pub lines: String,
}

/// The [`Tail`] of `r`'s jobs that ended abnormally; `log` reads a job's log
/// (its end), and a job without one (a wait, a log not there) is passed over.
pub fn tail(t: &Table, r: &Run, log: impl Fn(&str) -> Option<String>) -> Tail {
    let mut out = Tail::default();
    let mut blocks = Vec::new();
    for j in r.jobs.iter().filter(|j| j.state.is_error()) {
        let Some(text) = log(&j.id) else { continue };
        let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect();
        let Some(last) = lines.last() else { continue };
        if out.line.is_empty() {
            let mut line: String = format!("{}: {last}", j.label());
            if line.chars().count() > LINE_CHARS {
                line = line.chars().take(LINE_CHARS - 1).collect::<String>() + "…";
            }
            out.line = line;
        }
        let from = lines.len().saturating_sub(TAIL_LINES);
        blocks.push(format!("{}\n{}", t.trf("notify.tail", &[j.label()]), lines[from..].join("\n")));
    }
    out.lines = blocks.join("\n\n");
    out
}

/// The places of `n` a net's notice goes to: those `to` names (`notify_to`,
/// or a notify part's `to`), or with no names, the places with no name.
/// The system's notification and the sound are kept as they are.
pub fn to(n: &NotifyCfg, to: &[String]) -> NotifyCfg {
    if to.is_empty() {
        n.unnamed()
    } else {
        n.only(&to.iter().map(String::as_str).collect::<Vec<_>>())
    }
}

/// The names `net` sends to (`notify_to`, a notify part's `to`) that no
/// place of `n` has: a notice for them would go to no webhook and no mail.
pub fn unknown(n: &NotifyCfg, net: &Jobnet) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in net.notify_to.iter().chain(net.jobs.iter().flat_map(|j| &j.to)) {
        let known = n.places.iter().any(|p| &p.name == name) || n.mail_places.iter().any(|m| &m.name == name);
        if !known && !out.contains(name) {
            out.push(name.clone());
        }
    }
    out
}

/// Which notice of a run it is, so one kind does not replace another of the
/// same run on the screen (a notice replaces the last one with its tag).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    End,
    Recovered,
    Ask,
    Part,
    Late,
    Missed,
}

/// The tag of `kind`'s notice of run `run`: the id, then `.` and a letter
/// for all but the end (well under the 64 characters Windows keeps).
pub fn tag(run: &str, kind: Kind) -> String {
    let letter = match kind {
        Kind::End => return run.to_owned(),
        Kind::Recovered => "r",
        Kind::Ask => "a",
        Kind::Part => "p",
        Kind::Late => "l",
        Kind::Missed => "m",
    };
    format!("{run}.{letter}")
}

/// The run a notice's tag names (see [`tag`]).
pub fn run_of(tag: &str) -> &str {
    tag.split_once('.').map_or(tag, |(run, _)| run)
}

/// What a click on the system's notification opens: the run its tag names,
/// with a button labelled `label` (Windows).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Open {
    pub tag: String,
    pub label: String,
}

/// A click on `kind`'s notice of run `run` opens the window on that run.
pub fn open(t: &Table, run: &str, kind: Kind) -> Option<Open> {
    Some(Open { tag: tag(run, kind), label: t.tr("notify.open").to_owned() })
}

/// Tells of `event` in every way `n` turns on for it; with `open`, a click on
/// the system's notification opens its run ([`run_of`]). While the engine has
/// no tray of its own ([`hand_to_tray`]), the system's notification and the
/// sound are kept for `koyomi tray` instead.
pub fn tell(n: &NotifyCfg, open: Option<&Open>, event: &str, title: &str, body: &str) {
    match keep_for_tray(n, open, event, title, body) {
        Some(rest) => rest.tell(&EVENTS, APP, icon, event, title, body),
        None => show(n, open, event, title, body),
    }
}

fn show(n: &NotifyCfg, open: Option<&Open>, event: &str, title: &str, body: &str) {
    match open {
        Some(o) => n.tell_open(&EVENTS, APP, icon, event, title, body, &o.tag, &o.label),
        None => n.tell(&EVENTS, APP, icon, event, title, body),
    }
}

/// Shows on this desktop what `koyomi tray` took from the engine: the
/// system's notification and the sound as `n` turns them on, nothing else.
/// A notice of a run opens it when clicked, with a button labelled `label`.
pub fn tell_here(n: &NotifyCfg, note: &Note, label: &str) {
    let mut here = n.clone();
    here.places.clear();
    here.mail_places.clear();
    let open = (!note.tag.is_empty()).then(|| Open { tag: note.tag.clone(), label: label.to_owned() });
    show(&here, open.as_ref(), &note.event, &note.title, &note.body);
}

/// How many notices are kept for a tray that does not come for them.
const KEPT: usize = 50;

#[derive(Default)]
struct Kept {
    last: u64,
    notes: VecDeque<Note>,
}

impl Kept {
    /// Keeps the notice when `n` shows or sounds `event`; then `n` without those two, for the rest.
    fn keep(&mut self, n: &NotifyCfg, open: Option<&Open>, event: &str, title: &str, body: &str) -> NotifyCfg {
        let on = |l: &Option<Vec<String>>| l.as_ref().is_some_and(|l| l.iter().any(|e| e == event));
        if on(&n.system) || on(&n.sound) {
            self.last += 1;
            self.notes.push_back(Note {
                seq: self.last,
                event: event.to_owned(),
                title: title.to_owned(),
                body: body.to_owned(),
                tag: open.map(|o| o.tag.clone()).unwrap_or_default(),
            });
            if self.notes.len() > KEPT {
                self.notes.pop_front();
            }
        }
        let mut rest = n.clone();
        rest.system = Some(Vec::new());
        rest.sound = Some(Vec::new());
        rest
    }

    /// The notices after `since`; a `since` past the last (the engine started
    /// again since the tray last asked) gets every one kept.
    fn after(&self, since: u64) -> Vec<Note> {
        let since = if since > self.last { 0 } else { since };
        self.notes.iter().filter(|n| n.seq > since).cloned().collect()
    }
}

/// The notices kept for `koyomi tray`; `None` while the engine shows them itself.
static FOR_TRAY: Mutex<Option<Kept>> = Mutex::new(None);

/// From now on the system's notification and the sound are kept for `koyomi tray`
/// (the engine runs where it cannot show them, or without a tray of its own).
pub fn hand_to_tray() {
    let mut k = FOR_TRAY.lock().unwrap_or_else(|e| e.into_inner());
    k.get_or_insert_with(Kept::default);
}

/// Whether the notices are kept for `koyomi tray`.
pub fn handed_to_tray() -> bool {
    FOR_TRAY.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// The notices kept after `since` (`None`: none), and the number of the last one.
pub fn for_tray(since: Option<u64>) -> (Vec<Note>, u64) {
    let k = FOR_TRAY.lock().unwrap_or_else(|e| e.into_inner());
    let Some(k) = k.as_ref() else { return (Vec::new(), 0) };
    (since.map(|s| k.after(s)).unwrap_or_default(), k.last)
}

/// When the notices are handed to the tray: keeps this one for it, and gives
/// `n` without the system's notification and the sound. `None`: the engine shows it itself.
fn keep_for_tray(n: &NotifyCfg, open: Option<&Open>, event: &str, title: &str, body: &str) -> Option<NotifyCfg> {
    let mut k = FOR_TRAY.lock().unwrap_or_else(|e| e.into_inner());
    Some(k.as_mut()?.keep(n, open, event, title, body))
}

/// Like [`tell`], with the end of the failed jobs' logs: the system's
/// notification (and the sound) gets its one line, the webhooks and the mails its lines.
pub fn tell_tail(n: &NotifyCfg, open: Option<&Open>, event: &str, title: &str, body: &str, tail: &Tail) {
    if tail.lines.is_empty() {
        return tell(n, open, event, title, body);
    }
    let mut here = n.clone();
    here.places.clear();
    here.mail_places.clear();
    tell(&here, open, event, title, &format!("{body}\n{}", tail.line));
    let mut hooks = n.clone();
    hooks.system = Some(Vec::new());
    hooks.sound = Some(Vec::new());
    hooks.tell(&EVENTS, APP, icon, event, title, &format!("{body}\n\n{}", tail.lines));
}

#[cfg(test)]
mod tests {
    /// Settings `koyomi check` refuses are refused by export too, in one Japanese line.
    #[test]
    fn settings_that_check_refuses_are_a_problem_on_one_line() {
        assert_eq!(super::settings_problem("keep_days = 30\n"), None);
        let e = super::settings_problem("[notify]\nwebhooks = [\"secret:x\"]\n").unwrap();
        assert!(!e.contains('\n') && !e.is_ascii(), "{e}");
        assert!(koyomi_core::msg::NOTIFY_WEBHOOK_SHAPE.is_in(&e), "{e}");
        let e = super::settings_problem("keep_days = 0\n").unwrap();
        assert!(koyomi_core::msg::CFG_LIMIT_TOO_LOW.is_in(&e) && e.contains("keep_days"), "{e}");
        let src = koyomi_core::bundle::Source { settings: Some("[notify]\nwebhooks = [\"secret:x\"]\n".to_owned()), ..Default::default() };
        let pick = koyomi_core::bundle::Pick { settings: true, ..Default::default() };
        let e = super::export(&src, &pick).unwrap_err();
        assert!(koyomi_core::msg::NOTIFY_EXPORT_BAD.is_in(&e), "{e}");
    }

    use super::*;
    use koyomi_core::run::{How, Outcome, State};

    fn cfg(notify: &str) -> Config {
        Config::parse(&format!("[notify]\n{notify}")).unwrap()
    }

    #[test]
    fn a_net_tells_the_places_it_names_or_else_the_unnamed_ones() {
        let place = |name: &str, url: &str| ito_notify::webhook::Place { name: name.into(), url: url.into(), ..Default::default() };
        let mut n = NotifyCfg::default();
        n.places = vec![place("", "https://a"), place("経理", "https://b"), place("ops", "https://c")];
        let urls = |c: NotifyCfg| c.places.into_iter().map(|p| p.url).collect::<Vec<_>>();
        assert_eq!(urls(super::to(&n, &[])), ["https://a"]);
        assert_eq!(urls(super::to(&n, &["ops".into(), "経理".into()])), ["https://b", "https://c"]);
        let net = koyomi_core::jobnet::parse(
            "format = 1\nnotify_to = [\"経理\", \"夜間\"]\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"n\"\nkind = \"notify\"\nafter = [\"a\"]\nto =[\"夜間\", \"ops\", \"保守\"]\n",
        )
        .unwrap();
        assert_eq!(unknown(&n, &net), ["夜間", "保守"]);
        n.mail_places = vec![ito_notify::mail::Mail { name: "保守".into(), ..Default::default() }];
        assert_eq!(unknown(&n, &net), ["夜間"]);
        let urls = |c: NotifyCfg| c.mail_places.into_iter().map(|m| m.name).collect::<Vec<_>>();
        assert_eq!(urls(super::to(&n, &["保守".into()])), ["保守"]);
        assert!(urls(super::to(&n, &[])).is_empty());
    }

    #[test]
    fn a_mail_is_told_of_failed_unless_written_and_its_password_is_a_secret() {
        let (n, notes) = read(&cfg(
            "[[notify.mails]]\nname = \"ops\"\nhost = \"smtp.example.com\"\nuser = \"jobs\"\npassword = \"secret:mail-ops\"\nfrom = \"jobs@example.com\"\nto = [\"ops@example.com\"]\n",
        ));
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(n.mail_places.len(), 1);
        let m = &n.mail_places[0];
        assert_eq!((m.name.as_str(), m.port, m.states.clone()), ("ops", 587, vec!["failed".to_owned()]));
        let (_, notes) = read(&cfg(
            "[[notify.mails]]\nhost = \"smtp.example.com\"\nuser = \"jobs\"\npassword = \"hunter2\"\nfrom = \"jobs@example.com\"\nto = [\"ops@example.com\"]\n",
        ));
        assert!(notes.iter().any(|w| w.contains("[[notify.mails]] 1") && w.contains("password")), "{notes:?}");
    }

    #[test]
    fn nothing_written_tells_only_of_failed_without_a_sound() {
        let (n, notes) = read(&Config::default());
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(n.system, Some(vec!["failed".to_owned()]));
        assert!(n.sound.as_deref().unwrap_or(&[]).is_empty(), "{:?}", n.sound);
        assert!(n.places.is_empty());
        assert!(n.hide_webhooks());
        let (n, notes) = read(&cfg("system = [\"done\", \"failed\"]\nsound = [\"failed\"]\n"));
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(n.system, Some(vec!["done".to_owned(), "failed".to_owned()]));
        assert_eq!(n.sound, Some(vec!["failed".to_owned()]));
    }

    #[test]
    fn a_plain_text_address_is_noted_and_not_sent_to_until_the_switch_is_off() {
        let (n, notes) = read(&cfg("webhook = \"https://ntfy.sh/koyomi\"\n"));
        assert_eq!(notes, vec![plain()]);
        assert_eq!(n.places.len(), 1);
        assert!(n.places[0].url.is_empty());
        let (n, notes) = read(&cfg("webhook = \"https://ntfy.sh/koyomi\"\nhide_webhooks = false\n"));
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(n.places[0].url, "https://ntfy.sh/koyomi");
        assert_eq!(n.places[0].states, vec!["failed".to_owned()]);
        // The settings page keeps the address, so it can show it and
        // move it to the store; it is noted all the same.
        let (n, notes) = parse(&cfg("webhook = \"https://ntfy.sh/koyomi\"\n"));
        assert_eq!(notes, vec![plain()]);
        assert_eq!(n.places[0].url, "https://ntfy.sh/koyomi");
    }

    #[test]
    fn a_secret_reference_stays_to_be_read_back_later() {
        let c = Config::parse("[[notify.webhooks]]\nurl = \"secret:ntfy\"\nstates = [\"done\", \"failed\"]\n").unwrap();
        let (n, notes) = read(&c);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(n.places.len(), 1);
        assert_eq!(n.places[0].url, "secret:ntfy");
        assert_eq!(n.places[0].states, vec!["done".to_owned(), "failed".to_owned()]);
    }

    #[test]
    fn an_unknown_event_or_shape_is_noted_not_quietly_dropped() {
        let (n, notes) = read(&cfg("system = [\"failed\", \"exploded\"]\n"));
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("exploded"), "{notes:?}");
        assert_eq!(n.system, Some(vec!["failed".to_owned()]));
        let (_, notes) = read(&cfg("system = \"failed\"\n"));
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(koyomi_core::msg::NOTIFY_SHAPE.is_in(&notes[0]), "{notes:?}");
    }

    #[test]
    fn the_words_name_the_net_and_the_jobs_that_went_wrong() {
        let (t, _) = crate::i18n::table("ja");
        let net = koyomi_core::jobnet::parse("format = 1\nname = \"月次締め\"\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"b\"\nname = \"転記\"\nafter = [\"a\"]\nrun = [\"y\"]\n").unwrap();
        let at = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
        let mut r = Run::new("20261030-090000-00", "g/n", "月次締め", "2026-10-30", How::Manual, &net, at("2026-10-30 09:00:00"));
        r.start("a", at("2026-10-30 09:00:01"));
        r.finish("a", Outcome::Exited(0), at("2026-10-30 09:00:02"));
        r.start("b", at("2026-10-30 09:00:02"));
        r.finish("b", Outcome::Exited(3), at("2026-10-30 09:00:05"));
        assert_eq!(r.state, State::Error);
        let (event, title, body) = words(&t, &r);
        assert_eq!(event, "failed");
        assert_eq!(title, "✕ 異常終了  月次締め");
        assert!(body.starts_with("✕ 異常終了  転記  09:00:02–09:00:05  3秒"), "{body}");
        assert!(body.ends_with("\n20261030-090000-00  2026-10-30"), "{body}");
        assert!(!body.contains("正常終了"), "{body}");

        let mut r = Run::new("r", "g/n", "月次締め", "2026-10-30", How::Manual, &net, at("2026-10-30 09:00:00"));
        r.start("a", at("2026-10-30 09:00:01"));
        r.finish("a", Outcome::Exited(0), at("2026-10-30 09:00:02"));
        r.start("b", at("2026-10-30 09:00:02"));
        r.finish("b", Outcome::Exited(0), at("2026-10-30 09:00:05"));
        let (event, title, body) = words(&t, &r);
        assert_eq!(event, "done");
        assert_eq!(title, "✓ 正常終了  月次締め");
        assert!(body.starts_with("✓ 正常終了  r  月次締め  2026-10-30"), "{body}");
    }

    #[test]
    fn a_late_run_says_what_is_late_and_by_when() {
        let (t, _) = crate::i18n::table("ja");
        let net = koyomi_core::jobnet::parse(
            "format = 1
late_start = \"10m\"
late_end = \"06:00\"
[[job]]
id = \"a\"
run = [\"x\"]
",
        )
        .unwrap();
        let at = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
        let mut r = Run::new("r", "g/n", "夜間", "2026-10-30", How::Schedule, &net, at("2026-10-30 22:00:00"));
        r.planned = Some(at("2026-10-30 22:00:00"));
        let now = at("2026-10-30 22:10:00");
        let (event, title, body) = delay_words(&t, &net, &r, Delay::Start, None, now);
        assert_eq!(event, "failed");
        assert_eq!(title, "⏰ 遅れ  夜間");
        assert_eq!(
            body,
            "夜間: 22:10 になっても始まっていない
r  2026-10-30"
        );
        r.start("a", at("2026-10-30 22:15:00"));
        let (_, _, body) = delay_words(&t, &net, &r, Delay::Start, None, at("2026-10-30 22:15:00"));
        assert!(body.starts_with("夜間: 22:10 を過ぎてから始まった（22:15）"), "{body}");
        let (_, _, body) = delay_words(&t, &net, &r, Delay::End, None, at("2026-10-31 06:00:00"));
        assert!(body.starts_with("夜間: 6:00 を過ぎても終わっていない"), "{body}");
        let (_, _, body) = delay_words(&t, &net, &r, Delay::End, None, now);
        assert!(body.starts_with("夜間: 10/31 6:00 を過ぎても"), "another day says the day: {body}");
        let (_, _, body) = delay_words(&t, &net, &r, Delay::Slow, Some(chrono::Duration::minutes(20)), at("2026-10-30 22:55:00"));
        assert!(body.starts_with("夜間: いつもは 20分、もう 40分 動いている"), "{body}");
        let (event, title, body) = pending_words(&t, "g/n", "夜間", at("2026-10-30 22:00:00"), at("2026-10-30 22:10:00"), now);
        assert_eq!((event, title.as_str()), ("failed", "⏰ 遅れ  夜間"));
        assert_eq!(
            body,
            "夜間: 22:10 になっても始まっていない
g/n  22:00"
        );
    }

    #[test]
    fn a_failed_job_hands_its_last_line_to_the_system_and_ten_lines_to_webhooks() {
        let (t, _) = crate::i18n::table("ja");
        let net = koyomi_core::jobnet::parse(
            "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"b\"\nname = \"転記\"\nrun = [\"y\"]\n",
        )
        .unwrap();
        let at = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
        let now = at("2026-10-30 09:00:00");
        let mut r = Run::new("r", "g/n", "n", "2026-10-30", How::Manual, &net, now);
        r.start("a", now);
        r.finish("a", Outcome::Exited(0), now);
        r.start("b", now);
        r.finish("b", Outcome::Exited(2), now);
        let log: String = (1..=14).map(|i| format!("line {i}\n")).collect::<String>() + "! pw=***\n\n";
        let got = tail(&t, &r, |job| (job != "c").then(|| log.clone()));
        assert_eq!(got.line, "転記: ! pw=***");
        let want: Vec<String> = (6..=14).map(|i| format!("line {i}")).chain(["! pw=***".to_owned()]).collect();
        assert_eq!(got.lines, format!("転記 のログの末尾\n{}", want.join("\n")));
        // The ok job's log is not sent, and a run with no log to read has no tail.
        assert!(!got.lines.contains("a のログ"));
        assert_eq!(tail(&t, &r, |_| None), Tail::default());
        let long = "x".repeat(300);
        assert_eq!(tail(&t, &r, |_| Some(long.clone())).line.chars().count(), 200);
    }

    #[test]
    fn a_net_that_ends_ok_after_an_abnormal_run_is_told_as_recovered() {
        let (t, _) = crate::i18n::table("ja");
        let net = koyomi_core::jobnet::parse(
            "format = 1
name = \"月次締め\"
[[job]]
id = \"a\"
run = [\"x\"]
",
        )
        .unwrap();
        let at = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
        let run = |id: &str, when: &str, code: i32| {
            let mut r = Run::new(id, "g/n", "月次締め", "2026-10-30", How::Manual, &net, at(when));
            r.start("a", at(when));
            r.finish("a", Outcome::Exited(code), at(when));
            r
        };
        let bad = run("r1", "2026-10-30 09:00:00", 3);
        let good = run("r2", "2026-10-30 10:00:00", 0);
        let again = run("r3", "2026-10-30 11:00:00", 0);
        let all = vec![bad.clone(), good.clone(), again.clone()];
        assert_eq!(recovered_from(&all, &good).map(|p| p.id.as_str()), Some("r1"));
        // The next ok run follows an ok run: nothing to tell.
        assert!(recovered_from(&all, &again).is_none());
        // An abnormal run is not a recovery.
        assert!(recovered_from(&all, &bad).is_none());
        let (event, title, body) = recovery_words(&t, &good, &bad);
        assert_eq!(event, "failed");
        assert!(title.ends_with("月次締め"), "{title}");
        assert!(body.contains("r1") && body.ends_with("r2  2026-10-30"), "{body}");
    }

    #[test]
    fn the_times_a_net_slept_through_are_told_once_per_net() {
        let (t, _) = crate::i18n::table("ja");
        let net = koyomi_core::jobnet::parse("format = 1\nname = \"月次締め\"\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n").unwrap();
        let at = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
        let now = at("2026-10-30 09:00:00");
        let miss = |id: &str, jobnet: &str, p: &str| Run::not_run(id, jobnet, "月次締め", &p[..10], at(p), State::Missed, "x", &net, now);
        let a = miss("20261030-090000-00", "経理/月次締め", "2026-10-30 02:00:00");
        let b = miss("20261030-090000-01", "経理/月次締め", "2026-10-30 06:30:00");
        let c = miss("20261030-090000-02", "経理/日次", "2026-10-30 07:00:00");
        let mut done = Run::new("r", "経理/日次", "日次", "2026-10-30", How::Manual, &net, now);
        done.start("a", now);
        done.finish("a", Outcome::Exited(0), now);
        let (rest, missed) = group_missed(vec![a, done, b, c]);
        assert_eq!(rest.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["r"]);
        assert_eq!(missed.len(), 2);
        let (event, title, body) = missed_words(&t, &missed[0]);
        assert_eq!(event, "failed");
        assert_eq!(title, "↷ 時刻を過ぎた  月次締め");
        assert_eq!(body, "経理/月次締め: 2:00、6:30 の回は時刻を過ぎました（PC が止まっていました）\n20261030-090000-00  2026-10-30\n20261030-090000-01  2026-10-30");
        let two = vec![miss("x", "g/n", "2026-10-29 02:00:00"), miss("y", "g/n", "2026-10-30 02:00:00")];
        let (_, _, body) = missed_words(&t, &two);
        assert!(body.starts_with("g/n: 10/29 2:00、10/30 2:00 の回は"), "{body}");
    }

    #[test]
    fn a_notice_of_a_run_opens_that_run_and_kinds_do_not_replace_each_other() {
        let id = "20261030-090000-01";
        let kinds = [Kind::End, Kind::Recovered, Kind::Ask, Kind::Part, Kind::Late, Kind::Missed];
        let tags: Vec<String> = kinds.iter().map(|k| tag(id, *k)).collect();
        assert_eq!(tags[0], id);
        for t in &tags {
            assert_eq!(run_of(t), id, "{t}");
            assert!(t.len() <= 64, "{t}");
            assert_eq!(tags.iter().filter(|u| *u == t).count(), 1, "{t}");
        }
        let (ja, _) = crate::i18n::table("ja");
        let (en, _) = crate::i18n::table("en");
        assert_eq!(open(&ja, id, Kind::End).map(|o| o.label), Some("開く".to_owned()));
        assert_eq!(open(&en, id, Kind::End).map(|o| o.label), Some("Open".to_owned()));
    }

    /// with the notices handed to `koyomi tray`, only what shows or sounds is kept,
    /// the webhooks still go, and a tray that comes back after a restart gets all kept.
    #[test]
    fn the_tray_gets_what_would_show_or_sound() {
        let mut n = NotifyCfg::default();
        n.system = Some(vec!["failed".into()]);
        n.sound = Some(vec!["warn".into()]);
        n.places = vec![Default::default()];
        let mut k = Kept::default();
        let rest = k.keep(&n, None, "failed", "t1", "b1");
        assert_eq!((rest.system.as_deref(), rest.sound.as_deref(), rest.places.len()), (Some(&[][..]), Some(&[][..]), 1));
        k.keep(&n, None, "ok", "t2", "b2");
        let open = Open { tag: tag("20261030-090000-01", Kind::Late), label: "開く".into() };
        k.keep(&n, Some(&open), "warn", "t3", "b3");
        assert_eq!(k.after(0).iter().map(|n| (n.seq, n.title.as_str())).collect::<Vec<_>>(), [(1, "t1"), (2, "t3")]);
        // The tray gets what a click opens, not the button's word (its own language says that).
        assert_eq!(k.after(0).iter().map(|n| n.tag.as_str()).collect::<Vec<_>>(), ["", "20261030-090000-01.l"]);
        assert_eq!(k.after(1).len(), 1);
        assert_eq!(k.after(2).len(), 0);
        assert_eq!(k.after(9).len(), 2);
        for i in 0..KEPT + 5 {
            k.keep(&n, None, "failed", &i.to_string(), "");
        }
        assert_eq!(k.notes.len(), KEPT);
        assert_eq!(k.after(0).first().map(|n| n.seq), Some(k.last - KEPT as u64 + 1));
    }
}
