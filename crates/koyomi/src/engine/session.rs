//! Starts on a sign-in, a sign-out, a lock or an unlock: `[[when]] session = "lock"`.
//!
//! ito-power's `on_session` calls back on a thread of its own; the events wait in a
//! list until the engine's next tick takes them, and each net whose `session` matches
//! starts once a tick (with how many more came in the reason). The watch is made only
//! while a confirmed net has a `session` when, and dropped when none has.
//!
//! The service engine sees the events of every session but the services' own, and is
//! not told whose they were: anyone's lock starts the net. The engine a person started
//! sees only its own session, so mostly the lock and the unlock: it starts after the
//! sign-in and seldom lives through the sign-out. Both are said in the warnings there.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use ito_power::{SessionEvent, SessionWatch, WhyNot};
use koyomi_core::scan::{Def, Standing};
use koyomi_core::when::{Kind, Session};

use super::logs::Hit;

/// The events kept between two ticks at most; more in a tick change nothing but the count.
const KEEP: usize = 1000;

fn session_of(e: SessionEvent) -> Session {
    match e {
        SessionEvent::SignIn => Session::SignIn,
        SessionEvent::SignOut => Session::SignOut,
        SessionEvent::Lock => Session::Lock,
        SessionEvent::Unlock => Session::Unlock,
    }
}

/// What the reason of a run says happened.
pub fn what(s: Session) -> &'static str {
    match s {
        Session::SignIn => "サインインした",
        Session::SignOut => "サインアウトした",
        Session::Lock => "画面をロックした",
        Session::Unlock => "画面のロックを解いた",
    }
}

pub struct Sessions {
    /// The confirmed nets' `session` whens: the net and the event, each pair once.
    rules: Vec<(String, Session)>,
    /// `None` while no net wants it; an error stays until the next reload tries again.
    watch: Option<Result<SessionWatch, WhyNot>>,
    seen: Arc<Mutex<Vec<Session>>>,
}

impl Sessions {
    pub fn new() -> Sessions {
        Sessions { rules: Vec::new(), watch: None, seen: Arc::new(Mutex::new(Vec::new())) }
    }

    /// Takes the `session` whens of the confirmed nets as the rules, and starts or
    /// stops watching to match. A watch that failed is tried again here.
    pub fn set_rules(&mut self, defs: &[Def]) {
        self.rules = wanted(defs);
        if self.rules.is_empty() {
            self.watch = None;
            self.take();
        } else if !matches!(self.watch, Some(Ok(_))) {
            let seen = Arc::clone(&self.seen);
            self.watch = Some(ito_power::on_session(move |e| {
                if let Ok(mut s) = seen.lock() {
                    if s.len() < KEEP {
                        s.push(session_of(e));
                    }
                }
            }));
        }
    }

    fn take(&self) -> Vec<Session> {
        self.seen.lock().map(|mut s| std::mem::take(&mut *s)).unwrap_or_default()
    }

    /// The nets the events since the last tick start.
    pub fn due(&mut self) -> Vec<Hit> {
        if self.rules.is_empty() {
            return Vec::new();
        }
        step(&self.rules, &self.take())
    }

    /// Why the `session` whens do not start anything; one line a net (and event).
    pub fn problems(&self) -> Vec<String> {
        let failed = match &self.watch {
            Some(Err(e)) => Some(e),
            _ => None,
        };
        problems(&self.rules, failed, crate::service::RUNNING.load(Ordering::Relaxed))
    }
}

fn wanted(defs: &[Def]) -> Vec<(String, Session)> {
    let mut out: Vec<(String, Session)> = Vec::new();
    for d in defs.iter().filter(|d| d.standing == Standing::Confirmed) {
        for w in d.net.iter().flat_map(|n| &n.when) {
            if let Kind::Session { session } = w.kind {
                if !out.iter().any(|(id, s)| *id == d.id && *s == session) {
                    out.push((d.id.clone(), session));
                }
            }
        }
    }
    out
}

/// One hit for each rule an event matched, `count` the times it did.
fn step(rules: &[(String, Session)], events: &[Session]) -> Vec<Hit> {
    rules
        .iter()
        .filter_map(|(id, s)| {
            let count = events.iter().filter(|e| *e == s).count();
            (count > 0).then(|| Hit { def_id: id.clone(), kind: Kind::Session { session: *s }, count, first: String::new() })
        })
        .collect()
}

fn problems(rules: &[(String, Session)], failed: Option<&WhyNot>, service: bool) -> Vec<String> {
    if let Some(e) = failed {
        let why = match e {
            WhyNot::Unsupported => "この OS では見えない".to_owned(),
            WhyNot::Failed(n) => format!("見張りを始められなかった（{n}）"),
            other => other.to_string(),
        };
        let mut ids: Vec<&str> = rules.iter().map(|(id, _)| id.as_str()).collect();
        ids.dedup();
        return ids
            .into_iter()
            .map(|id| format!("{id}: サインイン・ロックが分からない（{why}）ので、session の [[when]] では流れない"))
            .collect();
    }
    if service {
        return Vec::new();
    }
    rules
        .iter()
        .filter_map(|(id, s)| match s {
            Session::SignIn => Some(format!(
                "{id}: サインインはサービスのエンジンにしか見えない（このエンジンはサインインの後に始まる）ので、session = \"sign_in\" では流れない。サインインのたびに流すなら on_start = true"
            )),
            Session::SignOut => Some(format!(
                "{id}: サインアウトはサービスのエンジンにしか見えない（このエンジンはサインアウトで止まる）ので、session = \"sign_out\" では流れない"
            )),
            Session::Lock | Session::Unlock => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: &str, text: &str, standing: Standing) -> Def {
        let net = koyomi_core::jobnet::parse(text).unwrap();
        Def {
            id: id.to_owned(),
            group: "g".to_owned(),
            name: id.to_owned(),
            title: None,
            file: format!("{id}.toml"),
            standing,
            problems: Vec::new(),
            net: Some(net),
            sha256: String::new(),
            text: text.to_owned(),
            previous: None,
            calendar: None,
        }
    }

    fn when(s: &str) -> String {
        format!("format = 1\n[[when]]\nsession = \"{s}\"\n")
    }

    #[test]
    fn each_net_starts_once_a_tick_on_its_own_event() {
        let both = "format = 1\n[[when]]\nsession = \"lock\"\n[[when]]\nsession = \"unlock\"\n[[when]]\nsession = \"lock\"\n";
        let rules = wanted(&[
            def("g/a", both, Standing::Confirmed),
            def("g/b", &when("unlock"), Standing::Confirmed),
            def("g/c", &when("lock"), Standing::Unconfirmed),
        ]);
        assert_eq!(rules, [("g/a".to_owned(), Session::Lock), ("g/a".to_owned(), Session::Unlock), ("g/b".to_owned(), Session::Unlock)]);
        let hits = step(&rules, &[Session::Lock, Session::Lock, Session::SignIn]);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].def_id.as_str(), &hits[0].kind, hits[0].count), ("g/a", &Kind::Session { session: Session::Lock }, 2));
        assert_eq!(hits[0].first, "");
        let hits = step(&rules, &[Session::Unlock]);
        assert_eq!(hits.iter().map(|h| h.def_id.as_str()).collect::<Vec<_>>(), ["g/a", "g/b"]);
        assert!(step(&rules, &[]).is_empty());
    }

    #[test]
    fn the_events_map_one_to_one() {
        assert_eq!(session_of(SessionEvent::SignIn), Session::SignIn);
        assert_eq!(session_of(SessionEvent::SignOut), Session::SignOut);
        assert_eq!(session_of(SessionEvent::Lock), Session::Lock);
        assert_eq!(session_of(SessionEvent::Unlock), Session::Unlock);
        assert_eq!(what(Session::Lock), "画面をロックした");
    }

    #[test]
    fn sign_in_and_out_are_said_outside_the_service() {
        let rules = vec![("g/a".to_owned(), Session::SignIn), ("g/b".to_owned(), Session::SignOut), ("g/c".to_owned(), Session::Lock)];
        let p = problems(&rules, None, false);
        assert_eq!(p.len(), 2);
        assert!(p[0].starts_with("g/a: サインインはサービスのエンジンにしか見えない") && p[0].ends_with("on_start = true"), "{p:?}");
        assert!(p[1].starts_with("g/b: サインアウトはサービスのエンジンにしか見えない"), "{p:?}");
        assert!(problems(&rules, None, true).is_empty());
    }

    #[test]
    fn a_watch_that_cannot_start_is_said_once_a_net() {
        let rules = vec![("g/a".to_owned(), Session::Lock), ("g/a".to_owned(), Session::Unlock), ("g/b".to_owned(), Session::SignIn)];
        assert_eq!(
            problems(&rules, Some(&WhyNot::Unsupported), true),
            [
                "g/a: サインイン・ロックが分からない（この OS では見えない）ので、session の [[when]] では流れない",
                "g/b: サインイン・ロックが分からない（この OS では見えない）ので、session の [[when]] では流れない",
            ]
        );
        assert!(problems(&rules, Some(&WhyNot::Failed(5)), false)[0].contains("見張りを始められなかった（5）"));
    }

    #[test]
    fn no_rules_means_no_watch() {
        let mut s = Sessions::new();
        s.set_rules(&[def("g/a", &when("lock"), Standing::Unconfirmed)]);
        assert!(s.watch.is_none());
        assert!(s.due().is_empty());
        assert!(s.problems().is_empty());
    }
}
