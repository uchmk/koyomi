//! Starts by mail: `[[when]] mail = "imaps://…"` with `sender` / `subject`
//! (text the mail's sender / subject holds, any case).
//!
//! Each mailbox is read on a thread of its own every [`POLL`], so a slow or
//! stuck server never holds the engine; what it finds comes back on a channel
//! that [`Mails::due`] takes. The mailbox is read only (nothing is marked read,
//! moved or deleted), from the newest mail when it is first watched: mail that
//! came before (or while the engine was down) does not start the net. All the
//! matches of one read start the net once, and the first is kept for the
//! record. A mailbox that cannot be read (no secret, a wrong password, no such
//! folder, the server down) is said in the cautions and tried again.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use koyomi_core::scan::{Def, Standing};
use koyomi_core::when::{self, Kind, Mailbox};

use super::logs::Hit;

/// How often a mailbox is read.
const POLL: Duration = Duration::from_secs(60);
/// A mail kept for the record is cut to this many characters.
const SHOWN: usize = 200;

/// A mail that came: its sender (`Name <address>` or the address) and subject.
#[derive(Clone, Debug, PartialEq)]
pub struct Mail {
    pub from: String,
    pub subject: String,
}

/// A mailbox open from its newest mail.
pub trait Inbox: Send {
    /// The mail that came since the last read (or the open), oldest first.
    fn poll(&mut self) -> Result<Vec<Mail>, String>;
}

/// Opens a mailbox with the password kept as the named secret.
type OpenFn = dyn Fn(&Mailbox, &str) -> Result<Box<dyn Inbox>, String> + Send + Sync;
pub type Open = Arc<OpenFn>;

enum Msg {
    Found(Vec<Mail>),
    /// Why the mailbox cannot be read now, or `None` when it can again.
    Problem(Option<String>),
}

struct Watch {
    def_id: String,
    kind: Kind,
    rx: Receiver<Msg>,
    /// Dropped with the watch, which ends its thread.
    _stop: Sender<()>,
    problem: Option<String>,
}

pub struct Mails {
    watches: Vec<Watch>,
    open: Open,
    every: Duration,
}

impl Mails {
    pub fn new() -> Mails {
        Mails::with(Arc::new(imap), POLL)
    }

    fn with(open: Open, every: Duration) -> Mails {
        Mails { watches: Vec::new(), open, every }
    }

    /// Takes the `mail` whens of the confirmed nets as the mailboxes to read.
    /// A rule kept from before reads on; a new one from the mailbox's newest mail now.
    pub fn set_rules(&mut self, defs: &[Def]) {
        let mut wanted: Vec<(String, Kind)> = Vec::new();
        for d in defs.iter().filter(|d| d.standing == Standing::Confirmed) {
            for w in d.net.iter().flat_map(|n| &n.when) {
                if matches!(w.kind, Kind::Mail { .. }) && !wanted.iter().any(|(id, k)| *id == d.id && *k == w.kind) {
                    wanted.push((d.id.clone(), w.kind.clone()));
                }
            }
        }
        self.watches.retain(|w| wanted.iter().any(|(id, k)| *id == w.def_id && *k == w.kind));
        for (def_id, kind) in wanted {
            if self.watches.iter().any(|w| w.def_id == def_id && w.kind == kind) {
                continue;
            }
            let Kind::Mail { mail, secret, .. } = &kind else { continue };
            let Ok(mailbox) = when::mailbox(mail) else { continue };
            let (tx, rx) = mpsc::channel();
            let (stop, stopped) = mpsc::channel();
            let (open, every, secret) = (Arc::clone(&self.open), self.every, secret.clone());
            let spawned = std::thread::Builder::new()
                .name("mail".into())
                .spawn(move || read_on(&mailbox, &secret, open.as_ref(), every, &tx, &stopped));
            let problem = spawned.err().map(|e| format!("読む糸を起こせない（{e}）"));
            self.watches.push(Watch { def_id, kind, rx, _stop: stop, problem });
        }
    }

    /// The nets whose mailboxes have had matching mail since the last call.
    pub fn due(&mut self) -> Vec<Hit> {
        let mut out = Vec::new();
        for w in &mut self.watches {
            let Kind::Mail { sender, subject, .. } = &w.kind else { continue };
            let mut found: Vec<Mail> = Vec::new();
            while let Ok(msg) = w.rx.try_recv() {
                match msg {
                    Msg::Found(mails) => found.extend(mails.into_iter().filter(|m| matches(m, sender.as_deref(), subject.as_deref()))),
                    Msg::Problem(p) => w.problem = p,
                }
            }
            if let Some(m) = found.first() {
                let first: String = format!("{}「{}」", m.from, m.subject).chars().take(SHOWN).collect();
                out.push(Hit { def_id: w.def_id.clone(), kind: w.kind.clone(), count: found.len(), first });
            }
        }
        out
    }

    /// Why the mailboxes that cannot be read now cannot, one line each.
    pub fn problems(&self) -> Vec<String> {
        self.watches
            .iter()
            .filter_map(|w| {
                let Kind::Mail { mail, .. } = &w.kind else { return None };
                Some(format!("{}: メールボックス {mail} を読めない（{}）", w.def_id, w.problem.as_ref()?))
            })
            .collect()
    }
}

/// Whether the mail's sender and subject hold the texts, any case.
fn matches(m: &Mail, sender: Option<&str>, subject: Option<&str>) -> bool {
    let holds = |text: &str, part: Option<&str>| part.is_none_or(|p| text.to_lowercase().contains(&p.to_lowercase()));
    holds(&m.from, sender) && holds(&m.subject, subject)
}

/// A watch's thread: opens the mailbox (again, until it opens), reads it every
/// `every`, and ends when the watch is dropped.
fn read_on(mailbox: &Mailbox, secret: &str, open: &OpenFn, every: Duration, tx: &Sender<Msg>, stop: &Receiver<()>) {
    let mut inbox: Option<Box<dyn Inbox>> = None;
    loop {
        let msg = match &mut inbox {
            None => match open(mailbox, secret) {
                Ok(i) => {
                    inbox = Some(i);
                    Msg::Problem(None)
                }
                Err(e) => Msg::Problem(Some(e)),
            },
            Some(i) => match i.poll() {
                Ok(mails) => {
                    if tx.send(Msg::Problem(None)).is_err() {
                        return;
                    }
                    Msg::Found(mails)
                }
                Err(e) => Msg::Problem(Some(e)),
            },
        };
        if tx.send(msg).is_err() {
            return;
        }
        match stop.recv_timeout(every) {
            Err(RecvTimeoutError::Timeout) => {}
            _ => return,
        }
    }
}

/// Opens the mailbox over IMAP with the password from the OS credential store.
fn imap(mailbox: &Mailbox, secret: &str) -> Result<Box<dyn Inbox>, String> {
    let password = match ito_secret::get("koyomi", secret) {
        Ok(Some(p)) => p,
        Ok(None) => return Err(format!("秘密 {secret} が資格情報ストアに無い。koyomi secret set {secret} で置く")),
        Err(e) => return Err(format!("秘密 {secret} が読めない（{e}）")),
    };
    let account =
        ito_mail::Account { host: mailbox.host.clone(), port: mailbox.port, user: mailbox.user.clone(), folder: mailbox.folder.clone() };
    let inbox = ito_mail::Inbox::open(&account, &password, ito_mail::From::End).map_err(|e| why(&e))?;
    Ok(Box::new(Imap(inbox)))
}

/// A mailbox read over IMAP by ito-mail; it connects again by itself after a drop.
struct Imap(ito_mail::Inbox);

impl Inbox for Imap {
    fn poll(&mut self) -> Result<Vec<Mail>, String> {
        let mails = self.0.poll().map_err(|e| why(&e))?;
        Ok(mails
            .into_iter()
            .map(|m| Mail {
                from: match m.name {
                    Some(name) if !name.is_empty() => format!("{name} <{}>", m.from),
                    _ => m.from,
                },
                subject: m.subject,
            })
            .collect())
    }
}

/// Why a mailbox cannot be read, for the cautions (never with the password).
fn why(e: &ito_mail::Error) -> String {
    match e {
        ito_mail::Error::Connect(s) => format!("サーバーにつながらない: {s}"),
        ito_mail::Error::Tls(s) => format!("TLS でつながらない: {s}"),
        ito_mail::Error::Login => "ユーザーかパスワードが違う".to_owned(),
        ito_mail::Error::NoFolder => "そのフォルダーが無い".to_owned(),
        ito_mail::Error::Other(s) => s.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Instant;

    fn def(id: &str, text: &str) -> Def {
        let net = koyomi_core::jobnet::parse(text).unwrap();
        Def {
            id: id.to_owned(),
            group: "g".to_owned(),
            name: id.to_owned(),
            title: None,
            file: format!("{id}.toml"),
            standing: Standing::Confirmed,
            problems: Vec::new(),
            net: Some(net),
            sha256: String::new(),
            text: text.to_owned(),
            previous: None,
            calendar: None,
        }
    }

    /// A mailbox whose new mail the test puts in `queue`.
    struct Fake(Arc<Mutex<Vec<Mail>>>);

    impl Inbox for Fake {
        fn poll(&mut self) -> Result<Vec<Mail>, String> {
            Ok(std::mem::take(&mut *self.0.lock().unwrap()))
        }
    }

    fn mail(from: &str, subject: &str) -> Mail {
        Mail { from: from.to_owned(), subject: subject.to_owned() }
    }

    /// `due` until it has hits or a second has gone.
    fn wait(mails: &mut Mails) -> Vec<Hit> {
        let start = Instant::now();
        loop {
            let hits = mails.due();
            if !hits.is_empty() || start.elapsed() > Duration::from_secs(1) {
                return hits;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    const NET: &str = "format = 1\n[[when]]\nmail = \"imaps://me@example.com@imap.example.com\"\npassword = \"secret:m\"\n\
                       sender = \"billing@\"\nsubject = \"請求\"\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n";

    #[test]
    fn matching_mail_starts_the_net_once_a_read() {
        let queue = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (q, s) = (Arc::clone(&queue), Arc::clone(&seen));
        let open: Open = Arc::new(move |m: &Mailbox, secret: &str| {
            s.lock().unwrap().push((m.user.clone(), m.host.clone(), secret.to_owned()));
            Ok(Box::new(Fake(Arc::clone(&q))) as Box<dyn Inbox>)
        });
        let mut mails = Mails::with(open, Duration::from_millis(10));
        let d = def("g/m", NET);
        mails.set_rules(std::slice::from_ref(&d));
        queue.lock().unwrap().extend([
            mail("Billing <BILLING@corp.example>", "11月の請求書"),
            mail("billing@corp.example", "お知らせ"),
            mail("other@corp.example", "請求書"),
            mail("billing@corp.example", "請求書（再送）"),
        ]);
        let hits = wait(&mut mails);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!((hits[0].def_id.as_str(), hits[0].count), ("g/m", 2));
        assert_eq!(hits[0].first, "Billing <BILLING@corp.example>「11月の請求書」");
        assert!(mails.problems().is_empty());
        // Kept across a rescan: opened once, not again.
        mails.set_rules(std::slice::from_ref(&d));
        queue.lock().unwrap().push(mail("billing@x", "請求"));
        assert_eq!(wait(&mut mails)[0].first, "billing@x「請求」");
        assert_eq!(*seen.lock().unwrap(), [("me@example.com".to_owned(), "imap.example.com".to_owned(), "m".to_owned())]);
        mails.set_rules(&[]);
        assert!(mails.watches.is_empty());
    }

    #[test]
    fn a_mailbox_that_cannot_be_opened_is_a_warning_and_tried_again() {
        let tries = Arc::new(Mutex::new(0));
        let t = Arc::clone(&tries);
        let open: Open = Arc::new(move |_: &Mailbox, _: &str| {
            let mut n = t.lock().unwrap();
            *n += 1;
            if *n < 3 {
                Err("ログインできない".to_owned())
            } else {
                Ok(Box::new(Fake(Arc::new(Mutex::new(vec![mail("billing@x", "請求")])))) as Box<dyn Inbox>)
            }
        });
        let mut mails = Mails::with(open, Duration::from_millis(10));
        mails.set_rules(&[def("g/m", NET)]);
        let start = Instant::now();
        while mails.problems().is_empty() && start.elapsed() < Duration::from_secs(1) {
            mails.due();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(mails.problems(), ["g/m: メールボックス imaps://me@example.com@imap.example.com を読めない（ログインできない）"]);
        assert_eq!(wait(&mut mails).len(), 1);
        assert!(mails.problems().is_empty());
    }
}
