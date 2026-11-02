//! Starts on a sign-in, a sign-out, a lock or an unlock: `[[when]] session = "lock"`,
//! and with `user = 'PC\taro'` only on that person's.
//!
//! ito-power's `on_session_with` calls back on a thread of its own with whose session
//! it was; the events wait in a list until the engine's next tick takes them, and each
//! net whose `session` (and `user`) matches starts once a tick, with who it was and how
//! many more came in the reason. The watch is made only while a confirmed net has a
//! `session` when, and dropped when none has.
//!
//! The service engine sees the events of every session but the services' own: without
//! `user`, anyone's lock starts the net. The engine a person started sees only its own
//! session, so mostly the lock and the unlock: it starts after the sign-in and seldom
//! lives through the sign-out, and a `user` that is someone else never comes. All three
//! are said in the warnings there.

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

/// What the reason of a run says happened, and who did it when that is known.
pub fn what(s: Session, who: &str) -> String {
    let did = match s {
        Session::SignIn => "サインインした",
        Session::SignOut => "サインアウトした",
        Session::Lock => "画面をロックした",
        Session::Unlock => "画面のロックを解いた",
    };
    if who.is_empty() {
        did.to_owned()
    } else {
        format!("{who} が{did}")
    }
}

/// One confirmed net's `session` when.
#[derive(Debug, Clone, PartialEq)]
struct Rule {
    id: String,
    session: Session,
    /// `None`: anyone's.
    user: Option<String>,
}

/// An event and whose session it was (`None` when Windows did not say).
type Event = (Session, Option<String>);

pub struct Sessions {
    /// The confirmed nets' `session` whens, each once.
    rules: Vec<Rule>,
    /// `None` while no net wants it; an error stays until the next reload tries again.
    watch: Option<Result<SessionWatch, WhyNot>>,
    seen: Arc<Mutex<Vec<Event>>>,
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
            self.watch = Some(ito_power::on_session_with(move |e, info| {
                if let Ok(mut s) = seen.lock() {
                    if s.len() < KEEP {
                        s.push((session_of(e), info.user.clone()));
                    }
                }
            }));
        }
    }

    fn take(&self) -> Vec<Event> {
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
        problems(&self.rules, failed, crate::service::RUNNING.load(Ordering::Relaxed), &me())
    }
}

/// The account this engine runs as, `DOMAIN\name` (`PC\name` for a local one).
fn me() -> String {
    let name = std::env::var("USERNAME").unwrap_or_default();
    match std::env::var("USERDOMAIN") {
        Ok(d) if !d.is_empty() && !name.is_empty() => format!("{d}\\{name}"),
        _ => name,
    }
}

fn wanted(defs: &[Def]) -> Vec<Rule> {
    let mut out: Vec<Rule> = Vec::new();
    for d in defs.iter().filter(|d| d.standing == Standing::Confirmed) {
        for w in d.net.iter().flat_map(|n| &n.when) {
            if let Kind::Session { session, user } = &w.kind {
                let rule = Rule { id: d.id.clone(), session: *session, user: user.clone() };
                if !out.contains(&rule) {
                    out.push(rule);
                }
            }
        }
    }
    out
}

/// Whether an event by `who` is one a rule for `want` takes. An event whose user is
/// not known is taken only by a rule for anyone.
fn by(want: Option<&str>, who: Option<&str>) -> bool {
    match (want, who) {
        (None, _) => true,
        (Some(w), Some(u)) => ito_power::same_user(w, u),
        (Some(_), None) => false,
    }
}

/// One hit for each rule an event matched, `count` the times it did and `first` whose
/// the first one was.
fn step(rules: &[Rule], events: &[Event]) -> Vec<Hit> {
    rules
        .iter()
        .filter_map(|r| {
            let mut hits = events.iter().filter(|(s, who)| *s == r.session && by(r.user.as_deref(), who.as_deref()));
            let (_, first) = hits.next()?;
            Some(Hit {
                def_id: r.id.clone(),
                kind: Kind::Session { session: r.session, user: r.user.clone() },
                count: 1 + hits.count(),
                first: first.clone().unwrap_or_default(),
            })
        })
        .collect()
}

fn problems(rules: &[Rule], failed: Option<&WhyNot>, service: bool, me: &str) -> Vec<String> {
    if let Some(e) = failed {
        let why = match e {
            WhyNot::Unsupported => "この OS では見えない".to_owned(),
            WhyNot::Failed(n) => format!("見張りを始められなかった（{n}）"),
            other => other.to_string(),
        };
        let mut ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
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
        .filter_map(|r| {
            let id = &r.id;
            match (r.session, r.user.as_deref()) {
                (Session::SignIn, _) => Some(format!(
                    "{id}: サインインはサービスのエンジンにしか見えない（このエンジンはサインインの後に始まる）ので、session = \"sign_in\" では流れない。サインインのたびに流すなら on_start = true"
                )),
                (Session::SignOut, _) => Some(format!(
                    "{id}: サインアウトはサービスのエンジンにしか見えない（このエンジンはサインアウトで止まる）ので、session = \"sign_out\" では流れない"
                )),
                (Session::Lock | Session::Unlock, Some(u)) if !me.is_empty() && !ito_power::same_user(u, me) => Some(format!(
                    "{id}: このエンジンには {me} のセッションしか見えないので、user = '{u}' のロック・解除では流れない。サービスのエンジンで動かす"
                )),
                (Session::Lock | Session::Unlock, _) => None,
            }
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

    fn rule(id: &str, session: Session, user: Option<&str>) -> Rule {
        Rule { id: id.to_owned(), session, user: user.map(str::to_owned) }
    }

    fn ev(session: Session, who: Option<&str>) -> Event {
        (session, who.map(str::to_owned))
    }

    #[test]
    fn each_net_starts_once_a_tick_on_its_own_event() {
        let both = "format = 1\n[[when]]\nsession = \"lock\"\n[[when]]\nsession = \"unlock\"\n[[when]]\nsession = \"lock\"\n";
        let rules = wanted(&[
            def("g/a", both, Standing::Confirmed),
            def("g/b", &when("unlock"), Standing::Confirmed),
            def("g/c", &when("lock"), Standing::Unconfirmed),
        ]);
        assert_eq!(rules, [rule("g/a", Session::Lock, None), rule("g/a", Session::Unlock, None), rule("g/b", Session::Unlock, None)]);
        let hits = step(&rules, &[ev(Session::Lock, Some("PC\\taro")), ev(Session::Lock, None), ev(Session::SignIn, None)]);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            (hits[0].def_id.as_str(), &hits[0].kind, hits[0].count),
            ("g/a", &Kind::Session { session: Session::Lock, user: None }, 2)
        );
        assert_eq!(hits[0].first, "PC\\taro");
        let hits = step(&rules, &[ev(Session::Unlock, None)]);
        assert_eq!(hits.iter().map(|h| h.def_id.as_str()).collect::<Vec<_>>(), ["g/a", "g/b"]);
        assert_eq!(hits[0].first, "");
        assert!(step(&rules, &[]).is_empty());
    }

    #[test]
    fn a_net_with_a_user_starts_only_on_that_persons_events() {
        let text = "format = 1\n[[when]]\nsession = \"lock\"\nuser = 'PC\\taro'\n[[when]]\nsession = \"lock\"\nuser = 'PC\\taro'\n[[when]]\nsession = \"lock\"\nuser = \"hanako\"\n";
        let rules = wanted(&[def("g/a", text, Standing::Confirmed)]);
        assert_eq!(rules, [rule("g/a", Session::Lock, Some("PC\\taro")), rule("g/a", Session::Lock, Some("hanako"))]);
        // Someone else's lock, a lock whose user is not known, and an unlock start nothing.
        assert!(
            step(&rules, &[ev(Session::Lock, Some("PC\\jiro")), ev(Session::Lock, None), ev(Session::Unlock, Some("PC\\taro"))]).is_empty()
        );
        // The name is read the way Windows reads it: any case, and any domain for `hanako`.
        let hits = step(
            &rules,
            &[ev(Session::Lock, Some("pc\\TARO")), ev(Session::Lock, Some("CORP\\Hanako")), ev(Session::Lock, Some("OTHER\\taro"))],
        );
        assert_eq!(hits.len(), 2);
        assert_eq!(
            (&hits[0].kind, hits[0].count, hits[0].first.as_str()),
            (&Kind::Session { session: Session::Lock, user: Some("PC\\taro".to_owned()) }, 1, "pc\\TARO")
        );
        assert_eq!((hits[1].count, hits[1].first.as_str()), (1, "CORP\\Hanako"));
    }

    #[test]
    fn the_events_map_one_to_one() {
        assert_eq!(session_of(SessionEvent::SignIn), Session::SignIn);
        assert_eq!(session_of(SessionEvent::SignOut), Session::SignOut);
        assert_eq!(session_of(SessionEvent::Lock), Session::Lock);
        assert_eq!(session_of(SessionEvent::Unlock), Session::Unlock);
        assert_eq!(what(Session::Lock, ""), "画面をロックした");
        assert_eq!(what(Session::SignIn, "PC\\taro"), "PC\\taro がサインインした");
    }

    #[test]
    fn sign_in_and_out_are_said_outside_the_service() {
        let rules =
            vec![rule("g/a", Session::SignIn, None), rule("g/b", Session::SignOut, Some("PC\\taro")), rule("g/c", Session::Lock, None)];
        let p = problems(&rules, None, false, "PC\\taro");
        assert_eq!(p.len(), 2);
        assert!(p[0].starts_with("g/a: サインインはサービスのエンジンにしか見えない") && p[0].ends_with("on_start = true"), "{p:?}");
        assert!(p[1].starts_with("g/b: サインアウトはサービスのエンジンにしか見えない"), "{p:?}");
        assert!(problems(&rules, None, true, "PC\\taro").is_empty());
    }

    #[test]
    fn someone_elses_lock_is_said_outside_the_service() {
        let rules = vec![
            rule("g/a", Session::Lock, Some("PC\\hanako")),
            rule("g/b", Session::Unlock, Some("taro")),
            rule("g/c", Session::Lock, Some("pc\\TARO")),
            rule("g/d", Session::Unlock, None),
        ];
        assert_eq!(
            problems(&rules, None, false, "PC\\taro"),
            ["g/a: このエンジンには PC\\taro のセッションしか見えないので、user = 'PC\\hanako' のロック・解除では流れない。サービスのエンジンで動かす"]
        );
        assert!(problems(&rules, None, true, "PC\\taro").is_empty());
        assert!(problems(&rules, None, false, "").is_empty());
    }

    #[test]
    fn a_watch_that_cannot_start_is_said_once_a_net() {
        let rules = vec![rule("g/a", Session::Lock, None), rule("g/a", Session::Unlock, None), rule("g/b", Session::SignIn, None)];
        assert_eq!(
            problems(&rules, Some(&WhyNot::Unsupported), true, "PC\\taro"),
            [
                "g/a: サインイン・ロックが分からない（この OS では見えない）ので、session の [[when]] では流れない",
                "g/b: サインイン・ロックが分からない（この OS では見えない）ので、session の [[when]] では流れない",
            ]
        );
        assert!(problems(&rules, Some(&WhyNot::Failed(5)), false, "PC\\taro")[0].contains("見張りを始められなかった（5）"));
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
