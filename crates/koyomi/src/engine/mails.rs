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

use koyomi_core::msg::{self, line};
use koyomi_core::registry::MailboxEntry;
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

/// Opens a mailbox (named by the jobnet whose rule reads it) with the password kept as the named secret.
type OpenFn = dyn Fn(&str, &Mailbox, &str) -> Result<Box<dyn Inbox>, String> + Send + Sync;
pub type Open = Arc<OpenFn>;

enum Msg {
    Found(Vec<Mail>),
    /// Why the mailbox cannot be read now, or `None` when it can again.
    Problem(Option<String>),
}

struct Watch {
    def_id: String,
    kind: Kind,
    /// The mailbox read and the name of its secret: the when's own, or the `[[mailbox]]` entry's it names.
    mail: String,
    secret: String,
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

    /// A `Mails` that opens nothing (the open fails with a plain text) and polls rarely: no network, no traffic.
    #[cfg(test)]
    pub(super) fn quiet() -> Mails {
        Mails::with(Arc::new(|_: &str, _: &Mailbox, _: &str| Err("quiet".to_owned())), Duration::from_secs(3600))
    }

    /// What is watched now: the net and the mailbox address read for it.
    #[cfg(test)]
    pub(super) fn watched(&self) -> Vec<(String, String)> {
        self.watches.iter().map(|w| (w.def_id.clone(), w.mail.clone())).collect()
    }

    fn with(open: Open, every: Duration) -> Mails {
        Mails { watches: Vec::new(), open, every }
    }

    /// Takes the `mail` whens of the confirmed nets as the mailboxes to read: the address and secret a when
    /// writes, or those of the `[[mailbox]]` entry (of `mailboxes`) it names. A watch is kept only while its
    /// net, rule, address and secret are all the same, so a changed or removed entry ends it (a changed one
    /// is read again from the mailbox's newest mail, as a new one is). A name no entry has is not watched
    /// and not told here (the net's caution W3795 says it).
    pub fn set_rules(&mut self, defs: &[Def], mailboxes: &[MailboxEntry]) {
        // (net, rule, address, secret name)
        let mut wanted: Vec<(String, Kind, String, String)> = Vec::new();
        for d in defs.iter().filter(|d| d.standing == Standing::Confirmed) {
            for w in d.net.iter().flat_map(|n| &n.when) {
                let Kind::Mail { mail, secret, mailbox, .. } = &w.kind else { continue };
                let (mail, secret) = match mailbox {
                    None => (mail.clone(), secret.clone()),
                    Some(name) => match mailboxes.iter().find(|m| m.name == *name) {
                        Some(m) => (m.mail.trim().to_owned(), m.secret().unwrap_or_default().to_owned()),
                        None => continue,
                    },
                };
                if !wanted.iter().any(|(id, k, _, _)| *id == d.id && *k == w.kind) {
                    wanted.push((d.id.clone(), w.kind.clone(), mail, secret));
                }
            }
        }
        let same = |w: &Watch, (id, k, mail, secret): &(String, Kind, String, String)| {
            w.def_id == *id && w.kind == *k && w.mail == *mail && w.secret == *secret
        };
        self.watches.retain(|w| wanted.iter().any(|x| same(w, x)));
        for want in wanted {
            if self.watches.iter().any(|w| same(w, &want)) {
                continue;
            }
            let (def_id, kind, mail, secret) = want;
            let Ok(mailbox) = when::mailbox(&mail) else { continue };
            let (tx, rx) = mpsc::channel();
            let (stop, stopped) = mpsc::channel();
            let (open, every, jobnet, secret_name) = (Arc::clone(&self.open), self.every, def_id.clone(), secret.clone());
            let spawned = std::thread::Builder::new()
                .name("mail".into())
                .spawn(move || read_on(&jobnet, &mailbox, &secret_name, open.as_ref(), every, &tx, &stopped));
            let problem = spawned.err().map(|e| e.to_string());
            self.watches.push(Watch { def_id, kind, mail, secret, rx, _stop: stop, problem });
        }
    }

    /// Drops the watches that read with the secret `secret` (ASCII case-insensitive), so the next `set_rules` opens them
    /// again with what the secret holds now. A watch is kept while its address and secret name stay the same and an
    /// open session is not dropped by a poll error, so a password changed in the same secret would not be picked up
    /// otherwise. A reopened watch starts at the newest mail, as after any change of the mailbox's address: mail that
    /// came between the last read and the reopening is not started on.
    pub fn reopen(&mut self, secret: &str) {
        self.watches.retain(|w| !w.secret.trim().eq_ignore_ascii_case(secret.trim()));
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
                let Kind::Mail { mail, mailbox, .. } = &w.kind else { return None };
                let p = w.problem.as_ref()?;
                // The mailbox as the when wrote it: its registered name, or its address.
                let shown = mailbox.as_deref().unwrap_or(mail);
                // A coded line already names the mailbox and keeps its ID at the head.
                Some(if msg::is_coded(p) { p.clone() } else { line(msg::MAIL_UNREADABLE, &[&w.def_id, shown, p]) })
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
fn read_on(jobnet: &str, mailbox: &Mailbox, secret: &str, open: &OpenFn, every: Duration, tx: &Sender<Msg>, stop: &Receiver<()>) {
    let mut inbox: Option<Box<dyn Inbox>> = None;
    loop {
        let msg = match &mut inbox {
            None => match open(jobnet, mailbox, secret) {
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
fn imap(jobnet: &str, mailbox: &Mailbox, secret: &str) -> Result<Box<dyn Inbox>, String> {
    let password = match ito_secret::get("koyomi", secret) {
        Ok(Some(p)) => p,
        Ok(None) => return Err(line(msg::MAIL_SECRET_MISSING, &[jobnet, &label(mailbox), secret])),
        Err(e) => return Err(line(msg::MAIL_SECRET_UNREADABLE, &[jobnet, &label(mailbox), secret, &e.to_string()])),
    };
    let account =
        koyomi_mail::Account { host: mailbox.host.clone(), port: mailbox.port, user: mailbox.user.clone(), folder: mailbox.folder.clone() };
    let inbox = koyomi_mail::Inbox::open(&account, &password, koyomi_mail::From::End).map_err(|e| why(jobnet, mailbox, secret, &e))?;
    Ok(Box::new(Imap { inbox, jobnet: jobnet.to_owned(), mailbox: mailbox.clone(), secret: secret.to_owned() }))
}

/// A mailbox read over IMAP by koyomi-mail; it connects again by itself after a drop.
struct Imap {
    inbox: koyomi_mail::Inbox,
    jobnet: String,
    mailbox: Mailbox,
    secret: String,
}

impl Inbox for Imap {
    fn poll(&mut self) -> Result<Vec<Mail>, String> {
        let mails = self.inbox.poll().map_err(|e| why(&self.jobnet, &self.mailbox, &self.secret, &e))?;
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

/// The mailbox as a rule names it: `imaps://user@host[:port][/folder]`.
fn label(m: &Mailbox) -> String {
    let port = if m.port == 993 { String::new() } else { format!(":{}", m.port) };
    let folder = if m.folder.eq_ignore_ascii_case("INBOX") { String::new() } else { format!("/{}", m.folder) };
    format!("imaps://{}@{}{port}{folder}", m.user, m.host)
}

/// Why a mailbox cannot be read, for the cautions (never with the password).
fn why(jobnet: &str, m: &Mailbox, secret: &str, e: &koyomi_mail::Error) -> String {
    let label = label(m);
    match e {
        koyomi_mail::Error::Connect(s) => line(msg::MAIL_CONNECT, &[jobnet, &label, s]),
        koyomi_mail::Error::Tls(s) => line(msg::MAIL_TLS, &[jobnet, &label, s]),
        koyomi_mail::Error::Login => line(msg::MAIL_LOGIN, &[jobnet, &label, secret]),
        koyomi_mail::Error::NoFolder => line(msg::MAIL_NO_FOLDER, &[jobnet, &label]),
        koyomi_mail::Error::Other(s) => s.clone(),
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
        let open: Open = Arc::new(move |_: &str, m: &Mailbox, secret: &str| {
            s.lock().unwrap().push((m.user.clone(), m.host.clone(), secret.to_owned()));
            Ok(Box::new(Fake(Arc::clone(&q))) as Box<dyn Inbox>)
        });
        let mut mails = Mails::with(open, Duration::from_millis(10));
        let d = def("g/m", NET);
        mails.set_rules(std::slice::from_ref(&d), &[]);
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
        mails.set_rules(std::slice::from_ref(&d), &[]);
        queue.lock().unwrap().push(mail("billing@x", "請求"));
        assert_eq!(wait(&mut mails)[0].first, "billing@x「請求」");
        assert_eq!(*seen.lock().unwrap(), [("me@example.com".to_owned(), "imap.example.com".to_owned(), "m".to_owned())]);
        mails.set_rules(&[], &[]);
        assert!(mails.watches.is_empty());
    }

    #[test]
    fn a_mailbox_that_cannot_be_opened_is_a_warning_and_tried_again() {
        let tries = Arc::new(Mutex::new(0));
        let t = Arc::clone(&tries);
        let open: Open = Arc::new(move |_: &str, _: &Mailbox, _: &str| {
            let mut n = t.lock().unwrap();
            *n += 1;
            if *n < 3 {
                Err("ログインできない".to_owned())
            } else {
                Ok(Box::new(Fake(Arc::new(Mutex::new(vec![mail("billing@x", "請求")])))) as Box<dyn Inbox>)
            }
        });
        let mut mails = Mails::with(open, Duration::from_millis(10));
        mails.set_rules(&[def("g/m", NET)], &[]);
        let start = Instant::now();
        while mails.problems().is_empty() && start.elapsed() < Duration::from_secs(1) {
            mails.due();
            std::thread::sleep(Duration::from_millis(2));
        }
        let problems = mails.problems();
        assert_eq!(problems.len(), 1);
        assert!(msg::MAIL_UNREADABLE.is_in(&problems[0]), "{problems:?}");
        assert!(problems[0].contains("imaps://me@example.com@imap.example.com"), "{problems:?}");
        // The jobnet whose rule reads the mailbox comes first, then the mailbox.
        let (at_jobnet, at_mailbox) = (problems[0].find("g/m"), problems[0].find("imaps://"));
        assert!(at_jobnet.is_some() && at_jobnet < at_mailbox, "{problems:?}");
        assert_eq!(wait(&mut mails).len(), 1);
        assert!(mails.problems().is_empty());
    }

    const NAMED: &str = "format = 1
[[when]]
mailbox = \"ops\"
sender = \"billing@\"
[[job]]
id = \"a\"
run = [\"x\"]
";

    fn entry(mail: &str, password: &str) -> MailboxEntry {
        MailboxEntry { name: "ops".into(), mail: mail.into(), password: password.into() }
    }

    /// A mailbox a when names is read with the entry's address and secret; changing the entry replaces the watch,
    /// removing it ends the watch, adding it again starts one. An inline when is untouched by all of it.
    #[test]
    fn a_named_mailbox_is_watched_with_the_entrys_values_and_follows_it() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = Arc::clone(&seen);
        let open: Open = Arc::new(move |_: &str, m: &Mailbox, secret: &str| {
            s.lock().unwrap().push((m.user.clone(), m.host.clone(), secret.to_owned()));
            Ok(Box::new(Fake(Arc::new(Mutex::new(Vec::new())))) as Box<dyn Inbox>)
        });
        let mut mails = Mails::with(open, Duration::from_millis(10));
        let named = def("g/n", NAMED);
        let inline = def("g/m", NET);
        let defs = [named, inline];
        type Seen = Arc<Mutex<Vec<(String, String, String)>>>;
        let waited = |seen: &Seen, n: usize| {
            let start = Instant::now();
            while seen.lock().unwrap().len() < n && start.elapsed() < Duration::from_secs(1) {
                std::thread::sleep(Duration::from_millis(5));
            }
            seen.lock().unwrap().len()
        };
        let a = entry("imaps://a@one.example", "secret:pa");
        mails.set_rules(&defs, std::slice::from_ref(&a));
        assert_eq!(mails.watches.len(), 2);
        assert_eq!(waited(&seen, 2), 2);
        assert!(seen.lock().unwrap().contains(&("a".to_owned(), "one.example".to_owned(), "pa".to_owned())));
        // The same entry again: nothing is reopened (the inline one too).
        mails.set_rules(&defs, std::slice::from_ref(&a));
        assert_eq!((mails.watches.len(), waited(&seen, 3)), (2, 2));
        // A changed address: the named watch is replaced, the inline one is kept.
        let b = entry("imaps://b@two.example", "secret:pb");
        mails.set_rules(&defs, std::slice::from_ref(&b));
        assert_eq!((mails.watches.len(), waited(&seen, 3)), (2, 3));
        assert!(seen.lock().unwrap().contains(&("b".to_owned(), "two.example".to_owned(), "pb".to_owned())));
        // Removed: only the inline one is left, and no problem is told here.
        mails.set_rules(&defs, &[]);
        assert_eq!(mails.watches.len(), 1);
        assert!(mails.problems().is_empty());
        // Added again: watched again.
        mails.set_rules(&defs, std::slice::from_ref(&b));
        assert_eq!(mails.watches.len(), 2);
        assert_eq!(waited(&seen, 4), 4);
    }

    /// A password changed in the same secret is read by watches opened again: `reopen` drops the watches of that
    /// secret only (any case), and `set_rules` opens them once more.
    #[test]
    fn reopen_drops_the_watches_of_a_secret_and_they_open_again() {
        let opens = Arc::new(Mutex::new(Vec::new()));
        let o = Arc::clone(&opens);
        let open: Open = Arc::new(move |_: &str, _: &Mailbox, secret: &str| {
            o.lock().unwrap().push(secret.to_owned());
            Ok(Box::new(Fake(Arc::new(Mutex::new(Vec::new())))) as Box<dyn Inbox>)
        });
        let mut mails = Mails::with(open, Duration::from_millis(10));
        let defs = [def("g/n", NAMED), def("g/m", NET)];
        let a = entry("imaps://a@one.example", "secret:Koyomi-Mail-Ops");
        let waited = |n: usize| {
            let start = Instant::now();
            while opens.lock().unwrap().len() < n && start.elapsed() < Duration::from_secs(1) {
                std::thread::sleep(Duration::from_millis(5));
            }
            opens.lock().unwrap().len()
        };
        mails.set_rules(&defs, std::slice::from_ref(&a));
        assert_eq!(waited(2), 2);
        // Nothing changed: nothing is opened again.
        mails.set_rules(&defs, std::slice::from_ref(&a));
        assert_eq!((mails.watches.len(), waited(3)), (2, 2));
        // Another secret: the watches are kept.
        mails.reopen("other");
        assert_eq!(mails.watches.len(), 2);
        // The named mailbox's secret, in another case: only that watch is dropped and opened again.
        mails.reopen("koyomi-mail-ops");
        assert_eq!(mails.watches.len(), 1);
        mails.set_rules(&defs, std::slice::from_ref(&a));
        assert_eq!((mails.watches.len(), waited(3)), (2, 3));
    }
}
