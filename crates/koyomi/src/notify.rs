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
use koyomi_core::run::Run;

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
pub fn read(config: &Config) -> (NotifyCfg, Vec<String>) {
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
    if n.hide_webhooks() {
        for p in &mut n.places {
            if ito_notify::is_http(&p.url) {
                notes.push(PLAIN.to_owned());
                p.url.clear();
            }
        }
    }
    (n, notes)
}

/// The event a run that ended is, and the title and body told of it: the
/// state's symbol and word with the net's name, then the jobs that went wrong
/// (or the run's line when none did), the id and the business day.
pub fn words(t: &Table, r: &Run) -> (&'static str, String, String) {
    let event = if r.state.is_error() { "failed" } else { "done" };
    let title = format!("{}  {}", show::tag(t, r.state), r.title);
    let mut lines: Vec<String> = r.jobs.iter().filter(|j| j.state.is_error()).map(|j| show::job_line(t, j)).collect();
    if lines.is_empty() {
        lines.push(show::run_line(t, r));
    }
    lines.push(format!("{}  {}", r.id, r.date));
    (event, title, lines.join("\n"))
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
}
