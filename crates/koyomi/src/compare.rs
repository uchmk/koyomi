//! Two runs side by side (`koyomi compare`): how long each took, and for each
//! job its state, time and exit code in both, with the difference of the logs
//! where they differ. Usually a run that went wrong next to one that did not.

use chrono::Duration;
use ito_diff::Item;
use ito_i18n::Table;
use koyomi_core::run::{JobRun, Run};
use serde::Serialize;

use crate::show;

/// How many unchanged lines stay around each change in a log's difference.
pub const CONTEXT: usize = 2;

/// The jobs of both runs, each once: `a`'s in its order, then those only `b` has.
pub fn jobs<'a>(a: &'a Run, b: &'a Run) -> Vec<(&'a str, Option<&'a JobRun>, Option<&'a JobRun>)> {
    let mut out: Vec<_> = a.jobs.iter().map(|j| (j.id.as_str(), Some(j), b.jobs.iter().find(|k| k.id == j.id))).collect();
    out.extend(b.jobs.iter().filter(|k| a.jobs.iter().all(|j| j.id != k.id)).map(|k| (k.id.as_str(), None, Some(k))));
    out
}

/// Whether the job has a log to read: it is one that runs, and it started.
pub fn has_log(j: &JobRun) -> bool {
    j.part.runs_process() && j.started.is_some()
}

/// `+2分10秒`, `-5秒` or `±0秒`.
pub fn delta(t: &Table, a: Duration, b: Duration) -> String {
    let d = b - a;
    let sign = match d.num_seconds() {
        0 => '±',
        s if s > 0 => '+',
        _ => '-',
    };
    format!("{sign}{}", show::duration(t, d.abs()))
}

/// Whether the job ended differently in the two: another state or another exit code.
pub fn differs(a: Option<&JobRun>, b: Option<&JobRun>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.state != b.state || a.exit != b.exit,
        _ => true,
    }
}

/// One side of a job: `✓ 正常終了  1分  exit 0`, or `（なし）` when the run has no such job.
fn side(t: &Table, j: Option<&JobRun>) -> String {
    let Some(j) = j else { return t.tr("compare.none").to_owned() };
    let mut s = show::tag(t, j.state);
    if let Some(d) = j.duration() {
        s.push_str(&format!("  {}", show::duration(t, d)));
    }
    if let Some(e) = j.exit {
        s.push_str(&format!("  {}", t.trf("compare.exit", &[&e.to_string()])));
    }
    s
}

/// The logs' difference as lines, `+`, `-` or a space and then the line; empty when they are the same.
pub fn log_lines(a: &str, b: &str) -> Vec<String> {
    if a == b {
        return Vec::new();
    }
    ito_diff::diff(a, b, CONTEXT)
        .iter()
        .map(|i| match i {
            Item::Line(l) => format!("{}{}", l.kind.mark(), l.text),
            Item::Folded(n) => format!("… {n}"),
        })
        .collect()
}

/// What `koyomi compare` prints. `logs` holds, by job id, the two logs as far as they could be read.
pub fn lines(t: &Table, a: &Run, b: &Run, logs: &[(String, Option<String>, Option<String>)]) -> Vec<String> {
    let mut out = vec![format!("A  {}", show::run_line(t, a)), format!("B  {}", show::run_line(t, b))];
    if a.jobnet != b.jobnet {
        out.push(t.trf("compare.other_net", &[&a.jobnet, &b.jobnet]));
    }
    if let (Some(x), Some(y)) = (a.duration(), b.duration()) {
        out.push(t.trf("compare.took", &[&show::duration(t, x), &show::duration(t, y), &delta(t, x, y)]));
    }
    out.push(String::new());
    out.push(t.tr("compare.jobs").to_owned());
    for (id, x, y) in jobs(a, b) {
        let label = x.or(y).map_or(id, |j| j.label());
        let mark = if differs(x, y) { "≠" } else { "=" };
        let mut line = format!("{mark} {label}  {}  →  {}", side(t, x), side(t, y));
        if let (Some(d), Some(e)) = (x.and_then(JobRun::duration), y.and_then(JobRun::duration)) {
            line.push_str(&format!("  ({})", delta(t, d, e)));
        }
        out.push(format!("  {line}"));
    }
    for (id, x, y) in logs {
        let (Some(x), Some(y)) = (x, y) else { continue };
        let diff = log_lines(x, y);
        if diff.is_empty() {
            continue;
        }
        let label = jobs(a, b).into_iter().find(|j| j.0 == id).and_then(|j| j.1.or(j.2)).map_or(id.as_str(), |j| j.label());
        out.push(String::new());
        out.push(t.trf("compare.log", &[label]));
        out.extend(diff.into_iter().map(|l| format!("  {l}")));
    }
    out
}

/// `koyomi compare --toml`.
#[derive(Serialize)]
pub struct Compare {
    pub a: String,
    pub b: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a_seconds: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b_seconds: Option<i64>,
    pub job: Vec<JobRow>,
}

/// One job of [`Compare`]; a side is missing when that run has no such job.
#[derive(Serialize)]
pub struct JobRow {
    pub id: String,
    pub differs: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b: Option<Side>,
    /// The logs' difference, when both were read and they differ.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_diff: Option<String>,
}

#[derive(Serialize)]
pub struct Side {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
}

impl Compare {
    pub fn new(a: &Run, b: &Run, logs: &[(String, Option<String>, Option<String>)]) -> Compare {
        let side = |j: Option<&JobRun>| {
            j.map(|j| Side { state: j.state.key().to_owned(), seconds: j.duration().map(|d| d.num_seconds()), exit: j.exit })
        };
        let job = jobs(a, b)
            .into_iter()
            .map(|(id, x, y)| {
                let log_diff = logs
                    .iter()
                    .find(|l| l.0 == id)
                    .and_then(|l| Some(log_lines(l.1.as_deref()?, l.2.as_deref()?)))
                    .filter(|d| !d.is_empty())
                    .map(|d| d.join("\n"));
                JobRow { id: id.to_owned(), differs: differs(x, y), a: side(x), b: side(y), log_diff }
            })
            .collect();
        Compare {
            a: a.id.clone(),
            b: b.id.clone(),
            a_seconds: a.duration().map(|d| d.num_seconds()),
            b_seconds: b.duration().map(|d| d.num_seconds()),
            job,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;
    use koyomi_core::jobnet::{Job, Jobnet};
    use koyomi_core::run::{How, State};

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn run(id: &str, ids: &[&str]) -> Run {
        let net = Jobnet {
            jobs: ids.iter().map(|i| Job { id: (*i).into(), run: vec!["x".into()], ..Default::default() }).collect(),
            ..Default::default()
        };
        Run::new(id, "g/n", "n", "2026-10-30", How::Manual, &net, at("2026-10-30 02:00:00"))
    }

    fn end(r: &mut Run, job: &str, state: State, exit: i32, secs: i64) {
        let j = r.jobs.iter_mut().find(|j| j.id == job).unwrap();
        j.state = state;
        j.exit = Some(exit);
        j.started = Some(at("2026-10-30 02:00:00"));
        j.ended = Some(at("2026-10-30 02:00:00") + Duration::seconds(secs));
    }

    #[test]
    fn the_jobs_of_both_come_once_in_order() {
        let a = run("A", &["x", "y"]);
        let b = run("B", &["y", "z"]);
        let ids: Vec<_> = jobs(&a, &b).iter().map(|j| (j.0, j.1.is_some(), j.2.is_some())).collect();
        assert_eq!(ids, vec![("x", true, false), ("y", true, true), ("z", false, true)]);
    }

    #[test]
    fn a_delta_is_signed() {
        let (t, _) = crate::i18n::table("ja");
        assert_eq!(delta(&t, Duration::seconds(60), Duration::seconds(190)), "+2分10秒");
        assert_eq!(delta(&t, Duration::seconds(60), Duration::seconds(55)), "-5秒");
        assert_eq!(delta(&t, Duration::seconds(60), Duration::seconds(60)), "±0秒");
    }

    #[test]
    fn a_job_that_ended_otherwise_is_marked_and_its_log_differs() {
        let (t, _) = crate::i18n::table("en");
        let (mut a, mut b) = (run("A", &["x", "y"]), run("B", &["x", "y"]));
        end(&mut a, "x", State::Ok, 0, 60);
        end(&mut b, "x", State::Ok, 0, 70);
        end(&mut a, "y", State::Ok, 0, 10);
        end(&mut b, "y", State::Error, 2, 5);
        let logs = vec![
            ("x".to_owned(), Some("same\n".to_owned()), Some("same\n".to_owned())),
            ("y".to_owned(), Some("start\nloaded 10\nend\n".to_owned()), Some("start\nno such file\n".to_owned())),
        ];
        let out = lines(&t, &a, &b, &logs);
        let x = out.iter().find(|l| l.contains(" x ")).unwrap();
        assert!(x.trim_start().starts_with("= x"), "{x}");
        assert!(x.ends_with("(+10 s)"), "{x}");
        let y = out.iter().find(|l| l.trim_start().starts_with("≠ y")).unwrap();
        assert!(y.contains("exit 0") && y.contains("exit 2"), "{y}");
        assert!(out.iter().any(|l| l == "  -loaded 10"), "{out:?}");
        assert!(out.iter().any(|l| l == "  +no such file"), "{out:?}");
        assert!(!out.iter().any(|l| l.contains("same")), "a log that is the same is not shown: {out:?}");
        let c = Compare::new(&a, &b, &logs);
        assert!(!c.job[0].differs && c.job[0].log_diff.is_none());
        assert!(c.job[1].differs && c.job[1].log_diff.as_deref().unwrap().contains("+no such file"));
        assert!(toml::to_string(&c).unwrap().contains("[[job]]"));
    }

    #[test]
    fn a_job_only_one_run_has_says_so() {
        let (t, _) = crate::i18n::table("en");
        let out = lines(&t, &run("A", &["x"]), &run("B", &["x", "z"]), &[]);
        let z = out.iter().find(|l| l.contains(" z ")).unwrap();
        assert!(z.contains(t.tr("compare.none")), "{z}");
        assert!(z.trim_start().starts_with('≠'), "{z}");
    }
}
