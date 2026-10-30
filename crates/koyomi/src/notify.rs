//! Telling of a run that ended: the system's notification, the sound and the
//! webhooks, as `[notify]` in `koyomi.toml` turns them on (ito-prefs's shape,
//! the same table every uchmk app reads).
//!
//! Nothing written: only 異常 is told, through the system's notification, and
//! without a sound. The way a run was started (a time, the window, the
//! console) changes nothing here. A webhook address is kept in the OS
//! credential store (`secret:<name>`); one written in plain text is not sent
//! to until `hide_webhooks = false` says that is meant.

use ito_i18n::Table;
use ito_notify::toast::App;
use ito_prefs::{Event, NotifyCfg, DONE_FAILED};
use koyomi_core::config::Config;
use koyomi_core::run::{Run, State};

use crate::show;

pub const APP: App<'static> = App { id: "uchmk.koyomi", name: "koyomi" };
/// The events a run can be told of: `done` (off unless written) and `failed`.
pub const EVENTS: [Event; 2] = DONE_FAILED;
/// The one event told when nothing is written.
const QUIET: &str = "failed";
/// Where `[notify]` is read from, as the notes name it.
const PATH: &str = "koyomi.toml";
/// Said of a plain-text webhook address while `hide_webhooks` is on.
pub const PLAIN: &str = "koyomi.toml: [notify] の webhook のアドレスが平文で書かれている。既定では OS の資格情報ストアに置く（`secret:<名前>`）。平文のままにするなら `hide_webhooks = false` と書く。それまでそこには送らない";

fn icon(side: u32) -> Vec<u8> {
    crate::icon::pixels(side)
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
            notes.push(format!("{PATH}: [notify]: {}", e.message()));
            NotifyCfg::default()
        }
    };
    let quiet = || vec![QUIET.to_owned()];
    n.system.get_or_insert_with(quiet);
    n.webhook_states.get_or_insert_with(quiet);
    for h in &mut n.webhooks {
        h.states.get_or_insert_with(quiet);
    }
    n.hide_webhooks.get_or_insert(true);
    n.check(&EVENTS, PATH, &mut notes);
    if n.hide_webhooks() && n.places.iter().any(|p| ito_notify::is_http(&p.url)) {
        notes.push(PLAIN.to_owned());
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
    let late = if r.is_late() { format!("（{}）", koyomi_core::run::LATE) } else { String::new() };
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

/// Tells of `event` in every way `n` turns on for it.
pub fn tell(n: &NotifyCfg, event: &str, title: &str, body: &str) {
    n.tell(&EVENTS, APP, icon, event, title, body);
}

#[cfg(test)]
mod tests {
    use super::*;
    use koyomi_core::run::{How, Outcome, State};

    fn cfg(notify: &str) -> Config {
        Config::parse(&format!("[notify]\n{notify}")).unwrap()
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
        assert_eq!(notes, vec![PLAIN.to_owned()]);
        assert_eq!(n.places.len(), 1);
        assert!(n.places[0].url.is_empty());
        let (n, notes) = read(&cfg("webhook = \"https://ntfy.sh/koyomi\"\nhide_webhooks = false\n"));
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(n.places[0].url, "https://ntfy.sh/koyomi");
        assert_eq!(n.places[0].states, vec!["failed".to_owned()]);
        // The settings page keeps the address, so it can show it and
        // move it to the store; it is noted all the same.
        let (n, notes) = parse(&cfg("webhook = \"https://ntfy.sh/koyomi\"\n"));
        assert_eq!(notes, vec![PLAIN.to_owned()]);
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
        assert!(notes[0].starts_with("koyomi.toml: [notify]:"), "{notes:?}");
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
        assert!(body.starts_with("✕ 異常終了  転記  09:00:02–09:00:05  0:00:03"), "{body}");
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
}
