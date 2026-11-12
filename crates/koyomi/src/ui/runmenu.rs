//! The run page's words about lines (the labels on the picture, why a job went on, why a failed run
//! continued), the previous run for a comparison, and the right-click menu of a job. Everything here is a
//! plain function over a run, so a test reads it without a window.

use ito_i18n::Table;
use koyomi_core::jobnet::{Judge, Line, On, Part};
use koyomi_core::run::{JobRun, Run, State};

use crate::proto::{Request, Snapshot};
use crate::show;

/// The word of a line kind, as the editor's legend has it: `成功したら`, `失敗したら` ...
pub fn line_word(t: &Table, on: &On) -> String {
    match on {
        On::Ok => t.tr("canvas.line_ok").to_owned(),
        On::Error => t.tr("canvas.line_error").to_owned(),
        On::Any => t.tr("canvas.line_any").to_owned(),
        On::Code(r) => format!("{} {}", t.tr("canvas.line_code"), Judge::list(r)),
        On::Else => t.tr("canvas.line_else").to_owned(),
    }
}

/// The label the picture puts on the middle of a line: the editor's mark and word. A plain (ok) line has none.
pub fn line_label(t: &Table, on: &On) -> Option<String> {
    let mark = match on {
        On::Ok => return None,
        On::Error => ito_a11y::State::Error.mark().to_owned(),
        On::Any => "*".to_owned(),
        On::Code(_) => return Some(line_word(t, on)),
        On::Else => "else".to_owned(),
    };
    Some(format!("{mark} {}", line_word(t, on)))
}

/// The word of a line kind without its detail (a code line is `終了コード`), for a sentence that names kinds.
fn kind_word(t: &Table, on: &On) -> String {
    match on {
        On::Code(_) => t.tr("canvas.line_code").to_owned(),
        _ => line_word(t, on),
    }
}

/// One line per line in the job's `after` that is met now: the predecessor, its state and the line's word.
/// It says the line IS met, never that it is what started the job (with a join-any several can be met).
pub fn met_lines(t: &Table, run: &Run, job: &JobRun) -> Vec<String> {
    job.after
        .iter()
        .filter_map(|l| {
            let pred = run.job(&l.job)?;
            run.line_met(l).then(|| t.trf("run.line_met", &[pred.label(), &show::tag(t, pred.state), &line_word(t, &l.on)]))
        })
        .collect()
}

/// Whether the line leaves a failed job behind and still goes on (an error / any / else / code line).
fn goes_past_failure(l: &Line) -> bool {
    !matches!(l.on, On::Ok)
}

/// Under the run's header: a job failed and the run went on through a met error / any / else / code line from
/// it. Names the failed jobs and the kinds of line actually met. The closing clause (what the run is) is
/// there only when the run is an error already; a run still going does not promise a result.
pub fn failure_continued(t: &Table, run: &Run) -> Option<String> {
    let mut failed: Vec<&str> = Vec::new();
    let mut kinds: Vec<String> = Vec::new();
    for j in &run.jobs {
        for l in j.after.iter().filter(|l| goes_past_failure(l) && run.line_met(l)) {
            let Some(pred) = run.job(&l.job).filter(|p| p.state.is_error()) else { continue };
            if !failed.contains(&pred.label()) {
                failed.push(pred.label());
            }
            let k = kind_word(t, &l.on);
            if !kinds.contains(&k) {
                kinds.push(k);
            }
        }
    }
    if failed.is_empty() {
        return None;
    }
    let names = failed.join(t.tr("run.list_sep"));
    let kinds = kinds.iter().map(|k| t.trf("run.quoted", &[k])).collect::<Vec<_>>().join(t.tr("run.list_sep"));
    let mut text = t.trf("run.failed_went_on", &[&names, &kinds]);
    if run.state.is_error() {
        text.push_str(&t.trf("run.failed_result", &[&show::tag(t, run.state)]));
    }
    Some(text)
}

/// The run that `b` is compared with by 前の回と比べる: the same net's newest run that ended and started
/// before `b`.
pub fn previous_run<'a>(runs: impl IntoIterator<Item = &'a Run>, b: &Run) -> Option<&'a Run> {
    runs.into_iter()
        .filter(|r| r.id != b.id && r.jobnet == b.jobnet && r.ended.is_some() && r.started < b.started)
        .max_by_key(|r| r.started)
}

/// The name of a job of a net as the engine's list has it, else the id.
pub fn job_name(snap: Option<&Snapshot>, net: &str, job: &str) -> String {
    snap.and_then(|s| s.defs.iter().find(|d| d.id == net))
        .and_then(|d| d.net.as_ref())
        .and_then(|n| n.jobs.iter().find(|j| j.id == job))
        .and_then(|j| j.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| job.to_owned())
}

/// What a menu item does once it is chosen.
#[derive(Clone, Debug)]
pub enum Act {
    /// Ask the engine.
    Send(Request),
    /// Pick the job (its facts and log show).
    Pick,
    /// Open the net in the editor, at the job.
    Editor,
}

/// One entry of the menu.
#[derive(Clone, Debug)]
pub struct Item {
    pub label: String,
    pub enabled: bool,
    /// Why it is off, when it is.
    pub why: Option<String>,
    pub act: Act,
}

impl Item {
    fn on(label: String, act: Act) -> Item {
        Item { label, enabled: true, why: None, act }
    }
}

/// What the menu needs to know besides the run and the job.
#[derive(Clone, Debug, Default)]
pub struct Env {
    /// The engine's list carries a warning: nothing is asked of it.
    pub blocked: bool,
    /// The net can be run again (it is there and not on hold).
    pub again: bool,
    /// The ids of the net's jobs on hold.
    pub held: Vec<String>,
    /// The net is in the engine's list, so the editor can open it.
    pub net_there: bool,
}

/// The right-click menu of one job of one run: only what means something for them, worded with the job's name.
/// It acts on this run and this job, whatever is picked on the page.
pub fn menu_items(t: &Table, run: &Run, job: &JobRun, env: &Env) -> Vec<Item> {
    let name = job.label();
    let mut out = Vec::new();
    let ask = |r: Request| Act::Send(r);
    if run.is_over() {
        if run.never_began() {
            let mut item =
                Item::on(t.tr("run.rerun_whole").to_owned(), ask(Request::Rerun { run: run.id.clone(), from: None, next: None }));
            item.enabled = env.again && !env.blocked;
            out.push(item);
        } else {
            let from = Some(job.id.clone());
            let mut put = |label: String, next: Option<State>| {
                let mut item = Item::on(label, ask(Request::Rerun { run: run.id.clone(), from: from.clone(), next }));
                item.enabled = !env.blocked;
                out.push(item);
            };
            put(t.trf("run.rerun_from", &[name]), None);
            put(t.trf("run.rerun_next_skip", &[name]), Some(State::Skipped));
            put(t.trf("run.rerun_next_warn", &[name]), Some(State::Warn));
        }
    } else if job.pending() && job.state != State::Running {
        out.push(Item::on(t.trf("run.skip_from", &[name]), ask(Request::Skip { run: run.id.clone(), job: job.id.clone() })));
    }
    if !job.id.contains('/') {
        let held = env.held.contains(&job.id);
        let (key, id, job) = (if held { "run.release_job" } else { "run.hold_job" }, run.jobnet.clone(), Some(job.id.clone()));
        let mut item = Item::on(t.trf(key, &[name]), ask(if held { Request::Release { id, job } } else { Request::Hold { id, job } }));
        item.enabled = !env.blocked;
        out.push(item);
    }
    if job.state == State::Running && matches!(job.part, Part::Approval { .. }) {
        for (yes, key) in [(true, "run.approve"), (false, "run.reject")] {
            let label = t.trf(&format!("{key}_from"), &[name]);
            out.push(Item::on(label, ask(Request::Answer { run: run.id.clone(), job: job.id.clone(), yes })));
        }
    }
    out.push(Item::on(t.tr("run.ask_log").to_owned(), Act::Pick));
    if env.net_there {
        out.push(Item::on(t.tr("run.menu_editor").to_owned(), Act::Editor));
    }
    out
}

/// A history row's 「<A> と比べる」: none while no run is marked, off for the marked run itself.
pub fn history_item(t: &Table, base: Option<&str>, row: &Run) -> Option<Item> {
    let a = base?;
    let mut item = Item::on(t.trf("run.compare_with", &[a]), Act::Pick);
    if row.id == a {
        item.enabled = false;
        item.why = Some(t.tr("run.compare_is_base").to_owned());
    }
    Some(item)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use koyomi_core::jobnet::{Job, Jobnet, Range};
    use koyomi_core::run::How;

    fn table() -> Table {
        crate::i18n::table("ja").0
    }

    fn at(h: u32, m: u32) -> chrono::NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 30).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    fn net(jobs: Vec<Job>) -> Jobnet {
        Jobnet { jobs, ..Default::default() }
    }

    fn job(id: &str, name: Option<&str>, after: Vec<Line>) -> Job {
        Job { id: id.into(), name: name.map(str::to_owned), run: vec!["x".into()], after, ..Default::default() }
    }

    /// The job as the engine left it: started at 9:00, ended a minute later (or still going with `None`).
    fn set(r: &mut Run, id: &str, state: State, over: bool) {
        let i = r.jobs.iter().position(|j| j.id == id).unwrap();
        r.jobs[i].state = state;
        r.jobs[i].started = Some(at(9, 0));
        r.jobs[i].ended = over.then(|| at(9, 1));
        r.jobs[i].exit = over.then_some(i32::from(state.is_error()));
    }

    fn line(job: &str, on: On) -> Line {
        Line { job: job.into(), on }
    }

    /// dice (さいころ) fails, then `next` follows it by a line of the given kind.
    fn failed_run(on: On) -> Run {
        let n = net(vec![job("dice", Some("さいころ"), vec![]), job("next", None, vec![line("dice", on)])]);
        let mut r = Run::new("r", "g/n", "n", "2026-10-30", How::Manual, &n, at(9, 0));
        set(&mut r, "dice", State::Error, true);
        r
    }

    #[test]
    fn labels_by_kind() {
        let t = table();
        assert_eq!(line_label(&t, &On::Ok), None);
        assert_eq!(line_label(&t, &On::Error).as_deref(), Some("✖ 失敗したら"));
        assert_eq!(line_label(&t, &On::Any).as_deref(), Some("* どちらでも"));
        assert_eq!(line_label(&t, &On::Else).as_deref(), Some("else それ以外"));
        let code = On::Code(vec![Range { from: 1, to: 3 }]);
        assert!(line_label(&t, &code).is_some_and(|l| l.starts_with("終了コード") && l.contains('1')));
    }

    #[test]
    fn a_job_after_an_error_line_says_it_is_met() {
        let t = table();
        let r = failed_run(On::Error);
        let next = r.job("next").unwrap();
        assert_eq!(met_lines(&t, &r, next), vec!["さいころ（✕ 異常終了）: 「失敗したら」の線を満たしています".to_owned()]);
        // The job that failed has no line before it.
        assert!(met_lines(&t, &r, r.job("dice").unwrap()).is_empty());
    }

    #[test]
    fn an_ok_line_from_a_failed_job_is_not_met() {
        let t = table();
        let r = failed_run(On::Ok);
        assert!(met_lines(&t, &r, r.job("next").unwrap()).is_empty());
    }

    #[test]
    fn a_join_any_lists_each_met_line() {
        let t = table();
        let n = net(vec![
            job("a", Some("あ"), vec![]),
            job("b", Some("い"), vec![]),
            job("c", None, vec![line("a", On::Any), line("b", On::Any)]),
        ]);
        let mut r = Run::new("r", "g/n", "n", "2026-10-30", How::Manual, &n, at(9, 0));
        set(&mut r, "a", State::Error, true);
        set(&mut r, "b", State::Ok, true);
        let lines = met_lines(&t, &r, r.job("c").unwrap());
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("あ（✕ 異常終了）") && lines[1].starts_with("い（✓ 正常終了）"));
        assert!(lines.iter().all(|l| l.contains("「どちらでも」の線を満たしています")));
    }

    #[test]
    fn the_header_line_fires_for_a_failure_that_went_on() {
        let t = table();
        let mut r = failed_run(On::Error);
        // The run is still going: no promise about its result.
        let going = failure_continued(&t, &r).unwrap();
        assert!(going.starts_with("さいころ が異常終了しましたが、「失敗したら」の線で続いています。"));
        assert!(!going.contains("回の結果は"));
        r.state = State::Error;
        let over = failure_continued(&t, &r).unwrap();
        assert!(over.ends_with("回の結果は ✕ 異常終了 になります。"), "{over}");
    }

    #[test]
    fn the_header_line_names_only_the_kinds_met() {
        let t = table();
        let r = failed_run(On::Any);
        let text = failure_continued(&t, &r).unwrap();
        assert!(text.contains("「どちらでも」") && !text.contains("失敗したら"), "{text}");
    }

    #[test]
    fn the_header_line_separates_two_kinds() {
        let t = table();
        let n = net(vec![
            job("dice", Some("さいころ"), vec![]),
            job("n1", None, vec![line("dice", On::Error)]),
            job("n2", None, vec![line("dice", On::Any)]),
        ]);
        let mut r = Run::new("r", "g/n", "n", "2026-10-30", How::Manual, &n, at(9, 0));
        set(&mut r, "dice", State::Error, true);
        let text = failure_continued(&t, &r).unwrap();
        assert!(text.contains("「失敗したら」、「どちらでも」の線で"), "{text}");
    }

    #[test]
    fn the_header_line_stays_quiet_beside_a_failure() {
        let t = table();
        // `next` only follows an ok job; `dice` failed beside it.
        let n = net(vec![job("good", None, vec![]), job("dice", Some("さいころ"), vec![]), job("next", None, vec![line("good", On::Ok)])]);
        let mut r = Run::new("r", "g/n", "n", "2026-10-30", How::Manual, &n, at(9, 0));
        set(&mut r, "good", State::Ok, true);
        set(&mut r, "dice", State::Error, true);
        assert!(failure_continued(&t, &r).is_none());
        // And a run with no failure says nothing either.
        let ok = failed_run(On::Ok);
        assert!(failure_continued(&t, &ok).is_none());
    }

    fn run_of(id: &str, jobnet: &str, started: chrono::NaiveDateTime, ended: Option<chrono::NaiveDateTime>) -> Run {
        let mut r = Run::new(id, jobnet, "n", "2026-10-30", How::Manual, &net(vec![job("a", None, vec![])]), started);
        r.ended = ended;
        r
    }

    #[test]
    fn the_previous_run_is_the_newest_ended_one_before() {
        let b = run_of("b", "g/n", at(12, 0), None);
        let runs = [
            run_of("old", "g/n", at(8, 0), Some(at(8, 5))),
            run_of("near", "g/n", at(10, 0), Some(at(10, 5))),
            run_of("other", "g/m", at(11, 0), Some(at(11, 5))),
            run_of("going", "g/n", at(11, 30), None),
            run_of("later", "g/n", at(13, 0), Some(at(13, 5))),
        ];
        assert_eq!(previous_run(runs.iter(), &b).map(|r| r.id.as_str()), Some("near"));
        let first = run_of("first", "g/n", at(7, 0), None);
        assert!(previous_run(runs.iter().filter(|r| r.id == "later"), &first).is_none());
        assert!(previous_run([].iter(), &b).is_none());
    }

    fn env() -> Env {
        Env { again: true, net_there: true, ..Default::default() }
    }

    fn labels(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    fn two_jobs() -> Run {
        let n = net(vec![job("dice", Some("さいころ"), vec![]), job("next", Some("つぎ"), vec![line("dice", On::Ok)])]);
        Run::new("r", "g/n", "n", "2026-10-30", How::Manual, &n, at(9, 0))
    }

    #[test]
    fn an_over_run_offers_the_reruns_from_the_job() {
        let t = table();
        let mut r = two_jobs();
        set(&mut r, "dice", State::Error, true);
        r.ended = Some(at(9, 2));
        assert!(r.is_over());
        let items = menu_items(&t, &r, r.job("dice").unwrap(), &env());
        assert_eq!(
            labels(&items),
            [
                "さいころ から再実行",
                "さいころ の次から（さいころ はスキップ）",
                "さいころ の次から（さいころ は警告終了）",
                "ジョブ さいころ を保留",
                "ログを見る",
                "エディターでこのジョブを開く",
            ]
        );
        assert!(matches!(&items[0].act, Act::Send(Request::Rerun { run, from: Some(f), next: None }) if run == "r" && f == "dice"));
        assert!(matches!(&items[1].act, Act::Send(Request::Rerun { next: Some(State::Skipped), .. })));
        assert!(matches!(&items[3].act, Act::Send(Request::Hold { id, job: Some(j) }) if id == "g/n" && j == "dice"));
    }

    #[test]
    fn a_running_run_offers_skip_for_a_pending_job_only() {
        let t = table();
        let mut r = two_jobs();
        set(&mut r, "dice", State::Running, false);
        assert!(!r.is_over());
        let pending = menu_items(&t, &r, r.job("next").unwrap(), &env());
        assert_eq!(labels(&pending)[0], "つぎ をスキップ");
        assert!(matches!(&pending[0].act, Act::Send(Request::Skip { run, job }) if run == "r" && job == "next"));
        let running = menu_items(&t, &r, r.job("dice").unwrap(), &env());
        assert!(labels(&running).iter().all(|l| !l.contains("スキップ") && !l.contains("再実行")));
    }

    #[test]
    fn a_held_job_offers_the_release() {
        let t = table();
        let r = two_jobs();
        let e = Env { held: vec!["next".into()], ..env() };
        let items = menu_items(&t, &r, r.job("next").unwrap(), &e);
        assert!(labels(&items).contains(&"ジョブ つぎ の保留を解除する"));
        assert!(items.iter().any(|i| matches!(&i.act, Act::Send(Request::Release { job: Some(j), .. }) if j == "next")));
    }

    #[test]
    fn a_waiting_approval_offers_the_answers() {
        let t = table();
        let n = net(vec![Job {
            id: "ok".into(),
            name: Some("確認".into()),
            part: Part::Approval { message: Some("m".into()) },
            ..Default::default()
        }]);
        let mut r = Run::new("r", "g/n", "n", "2026-10-30", How::Manual, &n, at(9, 0));
        set(&mut r, "ok", State::Running, false);
        let items = menu_items(&t, &r, r.job("ok").unwrap(), &env());
        let l = labels(&items);
        assert!(l.contains(&"確認 を承認する") && l.contains(&"確認 を却下する"), "{l:?}");
        assert!(items.iter().any(|i| matches!(&i.act, Act::Send(Request::Answer { yes: true, job, .. }) if job == "ok")));
        // A job that is not waiting for an answer has none.
        let other = two_jobs();
        let none = menu_items(&t, &other, other.job("next").unwrap(), &env());
        assert!(labels(&none).iter().all(|l| !l.contains("承認")));
    }

    #[test]
    fn a_run_that_never_began_offers_only_the_whole_rerun() {
        let t = table();
        let mut r = two_jobs();
        r.state = State::Missed;
        for j in &mut r.jobs {
            j.state = State::NotRun;
        }
        r.ended = Some(at(9, 0));
        assert!(r.never_began() && r.is_over());
        let items = menu_items(&t, &r, r.job("dice").unwrap(), &env());
        assert_eq!(labels(&items)[0], "最初から再実行する");
        assert!(items[0].enabled);
        assert!(labels(&items)[1..].iter().all(|l| !l.contains("から再実行") && !l.contains("の次から")));
        let held = menu_items(&t, &r, r.job("dice").unwrap(), &Env { again: false, ..env() });
        assert!(!held[0].enabled);
    }

    #[test]
    fn a_job_in_a_child_net_has_no_hold() {
        let t = table();
        let n = net(vec![job("kid/a", Some("子"), vec![])]);
        let r = Run::new("r", "g/n", "n", "2026-10-30", How::Manual, &n, at(9, 0));
        let items = menu_items(&t, &r, r.job("kid/a").unwrap(), &env());
        assert!(labels(&items).iter().all(|l| !l.contains("保留")));
    }

    #[test]
    fn the_history_row_offers_the_compare_unless_it_is_the_base() {
        let t = table();
        let row = run_of("b", "g/n", at(12, 0), Some(at(12, 1)));
        assert!(history_item(&t, None, &row).is_none());
        let item = history_item(&t, Some("a"), &row).unwrap();
        assert!(item.enabled && item.label == "a と比べる");
        let own = history_item(&t, Some("b"), &row).unwrap();
        assert!(!own.enabled && own.why.as_deref() == Some("比べる元そのものです"));
    }

    #[test]
    fn the_blocked_list_turns_the_asks_off() {
        let t = table();
        let mut r = two_jobs();
        set(&mut r, "dice", State::Error, true);
        r.ended = Some(at(9, 2));
        let items = menu_items(&t, &r, r.job("dice").unwrap(), &Env { blocked: true, ..env() });
        assert!(items.iter().filter(|i| matches!(i.act, Act::Send(_))).all(|i| !i.enabled));
        assert!(items.iter().filter(|i| matches!(i.act, Act::Pick | Act::Editor)).all(|i| i.enabled));
    }
}
