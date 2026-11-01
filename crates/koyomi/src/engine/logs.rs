//! Starts by lines: `[[when]] log = …` with `contains = …` (a line written to
//! a file) and `eventlog = …` (a record written to a Windows event log).
//!
//! Each rule reads from where the log ends when it is first watched: what was
//! written before (or while the engine was down) does not start the net, as a
//! file already there does not. The logs are read every [`POLL`]; all the
//! matches of one read start the net once, and the first is kept for the record.
//! A log that cannot be read (a log file that cannot be opened, an event log
//! not there or not allowed) is said in the warnings and tried again.

use std::time::{Duration, Instant};

use koyomi_core::scan::{Def, Standing};
use koyomi_core::when::Kind;

/// How often the logs are read.
const POLL: Duration = Duration::from_secs(2);
/// A line kept for the record is cut to this many characters.
const SHOWN: usize = 200;

/// Lines that came for a net: the net, the `[[when]]` they matched, how many and the first.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub def_id: String,
    pub kind: Kind,
    pub count: usize,
    pub first: String,
}

enum Reader {
    File(ito_tail::Follower),
    /// `None` until the log opens (it is opened again on each read until it does).
    EventLog(Option<ito_eventlog::Subscription>),
}

struct Watch {
    def_id: String,
    kind: Kind,
    reader: Reader,
    /// Why the last read failed, while it does.
    problem: Option<String>,
}

pub struct Logs {
    watches: Vec<Watch>,
    last: Option<Instant>,
}

impl Logs {
    pub fn new() -> Logs {
        Logs { watches: Vec::new(), last: None }
    }

    /// Takes the `log` and `eventlog` whens of the confirmed nets as the rules to read by.
    /// A rule kept from before reads on from where it was; a new one from the log's end now.
    pub fn set_rules(&mut self, defs: &[Def]) {
        let mut wanted: Vec<(String, Kind)> = Vec::new();
        for d in defs.iter().filter(|d| d.standing == Standing::Confirmed) {
            for w in d.net.iter().flat_map(|n| &n.when) {
                if matches!(w.kind, Kind::Log { .. } | Kind::EventLog { .. }) && !wanted.iter().any(|(id, k)| *id == d.id && *k == w.kind) {
                    wanted.push((d.id.clone(), w.kind.clone()));
                }
            }
        }
        self.watches.retain(|w| wanted.iter().any(|(id, k)| *id == w.def_id && *k == w.kind));
        for (def_id, kind) in wanted {
            if self.watches.iter().any(|w| w.def_id == def_id && w.kind == kind) {
                continue;
            }
            let mut watch = Watch {
                reader: match &kind {
                    Kind::Log { log, .. } => Reader::File(ito_tail::Follower::new(log, ito_tail::From::End)),
                    _ => Reader::EventLog(None),
                },
                def_id,
                kind,
                problem: None,
            };
            watch.open();
            self.watches.push(watch);
        }
    }

    /// The nets whose logs have had a matching line since the last read. Reads every [`POLL`].
    pub fn due(&mut self) -> Vec<Hit> {
        if self.last.is_some_and(|l| l.elapsed() < POLL) {
            return Vec::new();
        }
        self.last = Some(Instant::now());
        self.read()
    }

    /// Why the logs that cannot be read now cannot, one line each.
    pub fn problems(&self) -> Vec<String> {
        self.watches
            .iter()
            .filter_map(|w| {
                let what = match &w.kind {
                    Kind::Log { log, .. } => log.clone(),
                    Kind::EventLog { eventlog, .. } => format!("イベントログ {eventlog}"),
                    _ => return None,
                };
                Some(format!("{}: {what} を読めない（{}）", w.def_id, w.problem.as_ref()?))
            })
            .collect()
    }

    fn read(&mut self) -> Vec<Hit> {
        let mut out = Vec::new();
        for w in &mut self.watches {
            let found = w.read();
            if let Some(first) = found.first() {
                let first: String = first.chars().take(SHOWN).collect();
                out.push(Hit { def_id: w.def_id.clone(), kind: w.kind.clone(), count: found.len(), first });
            }
        }
        out
    }
}

impl Watch {
    /// Opens an event log not open yet; a log file needs nothing.
    fn open(&mut self) {
        let (Reader::EventLog(sub @ None), Kind::EventLog { eventlog, ids, source }) = (&mut self.reader, &self.kind) else { return };
        let filter = ito_eventlog::Filter { ids: ids.clone(), source: source.clone(), level: None };
        match ito_eventlog::Subscription::new(eventlog, &filter, ito_eventlog::From::End) {
            Ok(s) => {
                *sub = Some(s);
                self.problem = None;
            }
            Err(e) => self.problem = Some(e.to_string()),
        }
    }

    /// The matching lines (or records) since the last read.
    fn read(&mut self) -> Vec<String> {
        self.open();
        let read = match (&mut self.reader, &self.kind) {
            (Reader::File(f), Kind::Log { contains, .. }) => {
                f.poll().map(|lines| lines.into_iter().filter(|l| l.contains(contains.as_str())).collect()).map_err(|e| e.to_string())
            }
            (Reader::EventLog(Some(s)), Kind::EventLog { source, .. }) => s
                .poll()
                .map(|records| {
                    records
                        .into_iter()
                        .filter(|r| heard(source.as_deref(), &r.source))
                        .map(|r| format!("ID {} {}: {}", r.id, r.source, r.message.trim()))
                        .collect()
                })
                .map_err(|e| e.to_string()),
            _ => return Vec::new(),
        };
        match read {
            Ok(found) => {
                self.problem = None;
                found
            }
            Err(e) => {
                self.problem = Some(e);
                Vec::new()
            }
        }
    }
}

/// Whether an event-log record from `source` starts the net. koyomi writes each run's end to the
/// Application log itself (`crate::eventlog`, ids 1000, 2000, …): a trigger that names no source
/// would hear those and start again on its own run's end, over and over. So with no source named,
/// koyomi's own records are left out; a trigger that names `koyomi` gets them on purpose.
fn heard(wanted: Option<&str>, source: &str) -> bool {
    wanted.is_some() || !source.eq_ignore_ascii_case(crate::eventlog::SOURCE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn with_no_source_named_koyomis_own_run_ends_do_not_start_the_net() {
        assert!(!heard(None, "koyomi"));
        assert!(!heard(None, "Koyomi"));
        assert!(heard(None, "Application Error"));
        // Named on purpose (another net's failure starts this one): heard.
        assert!(heard(Some("koyomi"), "koyomi"));
    }

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

    fn append(path: &std::path::Path, text: &str) {
        std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap().write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn only_new_lines_with_the_text_start_the_net_once_a_read() {
        let dir = std::env::temp_dir().join(format!("koyomi-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.log");
        append(&path, "old ERROR before the watch\n");
        let log = path.display().to_string().replace('\\', "/");
        let d = def("g/n", &format!("format = 1\n[[when]]\nlog = \"{log}\"\ncontains = \"ERROR\"\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n"));
        let mut logs = Logs::new();
        logs.set_rules(std::slice::from_ref(&d));
        assert!(logs.read().is_empty());
        append(&path, "fine\nERROR one\nERROR two\n");
        let hits = logs.read();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!((hits[0].def_id.as_str(), hits[0].count, hits[0].first.as_str()), ("g/n", 2, "ERROR one"));
        assert!(logs.read().is_empty());
        // Kept across a rescan: the lines in between are not lost or read twice.
        append(&path, "ERROR three\n");
        logs.set_rules(std::slice::from_ref(&d));
        assert_eq!(logs.read()[0].first, "ERROR three");
        append(&path, "no match\n");
        assert!(logs.read().is_empty());
        logs.set_rules(&[]);
        assert!(logs.watches.is_empty());
        assert!(logs.problems().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_event_log_that_is_not_there_is_a_warning() {
        let d = def("g/e", "format = 1\n[[when]]\neventlog = \"koyomi-no-such-log-x\"\nids = [1]\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n");
        let mut logs = Logs::new();
        logs.set_rules(std::slice::from_ref(&d));
        assert!(logs.read().is_empty());
        let p = logs.problems();
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].starts_with("g/e: イベントログ koyomi-no-such-log-x を読めない（"), "{p:?}");
    }
}
