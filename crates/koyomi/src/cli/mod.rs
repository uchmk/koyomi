//! The commands that print and exit: `check`, `list`, `approve`, `run`, `rerun`, `status`, `log`, `kill`,
//! `hold`, `release`, `skip`, `pause`, `resume`, `engine`, `startup`.
//!
//! Exit codes: 0 done, 1 refused or not so (a problem in a file, not approved, an engine already up),
//! 2 used wrongly.

use ito_ipc::door::ask;
use koyomi_core::scan::{Def, Standing};

use std::time::Duration;

use koyomi_core::run::Run;

use crate::proto::{self, Request, Runs, Snapshot};
use crate::{client, engine, review, say, show};

pub const USAGE: &str = "\
usage: koyomi [--version | --help | env]
       koyomi check <file>           read a definition and say what is wrong with it (exit 1 if anything)
       koyomi list [--toml]          the definitions and where each stands
       koyomi approve <group/name>   show what it would run, and approve it on `y`
       koyomi run <group/name> [--from <job>] [--date YYYY-MM-DD] [--wait]
                                     start the net now (--from: at that job, the ones before it skipped;
                                     --date: for that business day; --wait: stay until it ends)
       koyomi rerun <run> [--from <job>] [--wait]
                                     run it again: what passed is carried over, the rest runs (--from: that job and after it too)
       koyomi next <group/name> [--count N]
                                     the next 10 times its [[when]] comes (N: that many)
       koyomi status [--today] [--toml]
                                     the runs still going (--today: every run of today)
       koyomi log <run> <job>        the tail of one job's log
       koyomi kill <run> [<job>]     stop one job, or the whole run
       koyomi hold <group/name>      hold the net: its times are written down as 保留 and nothing runs
       koyomi release <group/name>   take the net off hold
       koyomi skip <run> <job>       skip one job that has not started; the jobs after it go on
       koyomi pause | resume         stop the whole schedule / start it again (runs by hand still go)
       koyomi engine [status|stop]   run the engine in the foreground / ask it / stop it
       koyomi startup [on|off]       start the engine at sign-in, or stop doing so (nothing: say which)
";

/// What the sign-in entry runs: the engine, not the window.
pub const STARTUP_ARGS: [&str; 1] = ["engine"];

/// Words for the person at the console: common.toml's language.
pub fn table() -> ito_i18n::Table {
    let common = ito_common::base_dir().and_then(|b| ito_common::Common::read(&b).ok()).map(|(c, _)| c).unwrap_or_default();
    crate::i18n::table(&crate::i18n::resolve(common.language.as_deref())).0
}

/// Runs `args` (after the program name) when they are one of these commands; `None` when they are not.
pub fn run(args: &[String]) -> Option<i32> {
    let (first, rest) = args.split_first()?;
    Some(match first.as_str() {
        "check" => check(rest),
        "list" => list(rest),
        "approve" => approve(rest),
        "run" => run_cmd(rest),
        "rerun" => rerun_cmd(rest),
        "next" => next(rest),
        "status" => status(rest),
        "log" => log(rest),
        "kill" => kill(rest),
        "hold" => one_id(rest, "hold", |id| Request::Hold { id }),
        "release" => one_id(rest, "release", |id| Request::Release { id }),
        "skip" => skip(rest),
        "pause" => plain(rest, "pause", Request::Pause),
        "resume" => plain(rest, "resume", Request::Resume),
        "engine" => engine_cmd(rest),
        "startup" => startup_cmd(rest),
        _ => return None,
    })
}

fn usage(why: &str) -> i32 {
    say(&format!("koyomi: {why}\n\n{USAGE}"), true);
    2
}

fn check(args: &[String]) -> i32 {
    let [file] = args else { return usage("check takes one file") };
    let path = std::path::Path::new(file);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| file.clone());
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            say(&format!("{name}: 読めない: {e}"), true);
            return 1;
        }
    };
    if name == "koyomi.toml" {
        return check_config(&name, &text);
    }
    let mut bad = false;
    // A file under the settings folder runs only if nobody else can change it.
    if let Ok(paths) = engine::Paths::find() {
        let inside = path.canonicalize().ok().filter(|p| paths.app.canonicalize().is_ok_and(|a| p.starts_with(a)));
        let files: Vec<_> = inside.into_iter().collect();
        for w in engine::access_warnings(&paths, &files) {
            say(&w, true);
            bad = true;
        }
    }
    if let Err(problems) = koyomi_core::jobnet::parse(&text) {
        for p in problems {
            say(&p.in_file(&name), true);
        }
        bad = true;
    }
    if !bad {
        say(&format!("{name}: 問題なし"), false);
    }
    i32::from(bad)
}

/// `koyomi check koyomi.toml`: the settings read, and what `[notify]` would do.
fn check_config(name: &str, text: &str) -> i32 {
    let config = match koyomi_core::config::Config::parse(text) {
        Ok(c) => c,
        Err(e) => {
            say(&format!("{name}: {e}"), true);
            return 1;
        }
    };
    let (_, notes) = crate::notify::read(&config);
    for n in &notes {
        say(n, true);
    }
    if notes.is_empty() {
        say(&format!("{name}: 問題なし"), false);
    }
    i32::from(!notes.is_empty())
}

fn snapshot() -> Result<Snapshot, String> {
    Snapshot::parse(&client::call(&Request::List)?)
}

fn list(args: &[String]) -> i32 {
    let toml = match args {
        [] => false,
        [a] if a == "--toml" => true,
        _ => return usage("list takes only --toml"),
    };
    let snap = match snapshot() {
        Ok(s) => s,
        Err(e) => {
            say(&format!("koyomi: {e}"), true);
            return 1;
        }
    };
    if toml {
        say(snap.to_toml().trim_end_matches('\n'), false);
    } else {
        for w in &snap.warnings {
            say(&format!("! {w}"), false);
        }
        for n in &snap.notes {
            say(&format!("注意: {n}"), false);
        }
        if snap.paused {
            say(&format!("{} スケジュールを止めている（koyomi resume で戻す）", koyomi_core::run::State::Held.mark()), false);
        }
        for line in review::tree(&table(), &snap.defs, &snap.held) {
            say(&line, false);
        }
    }
    0
}

fn approve(args: &[String]) -> i32 {
    let [id] = args else { return usage("approve takes one group/name") };
    let id = id.replace('\\', "/");
    let t = table();
    let fail = |text: &str| {
        say(text, true);
        1
    };
    let snap = match snapshot() {
        Ok(s) => s,
        Err(e) => return fail(&format!("koyomi: {e}")),
    };
    if !snap.warnings.is_empty() {
        for w in &snap.warnings {
            say(&format!("! {w}"), true);
        }
        return fail("承認できない: 他のユーザーが書き換えられる場所がある");
    }
    let def: Def = match client::call(&Request::Show { id: id.clone() }).and_then(|s| toml::from_str(&s).map_err(|e| e.to_string())) {
        Ok(d) => d,
        Err(e) => return fail(&format!("koyomi: {e}")),
    };
    match def.standing {
        Standing::Invalid => {
            for p in &def.problems {
                say(&p.in_file(&def.file), true);
            }
            return fail(&format!("{}: 定義の誤りがあり、承認できない", def.file));
        }
        Standing::Confirmed => {
            say(&format!("{}: すでに確認済み", def.file), false);
            return 0;
        }
        Standing::Unconfirmed => {}
    }
    say(&format!("{} {}", def.file, review::word(&t, def.standing)), false);
    for l in review::body(&t, &def) {
        say(&l, false);
    }
    for l in review::week(&t, &def, chrono::Local::now().naive_local()) {
        say(&l, false);
    }
    let diff = review::diff(&def);
    if !diff.is_empty() {
        say(t.tr("review.diff"), false);
        for l in review::diff_text(&diff) {
            say(&l, false);
        }
    }
    say("確かめたなら y を入力（それ以外は未確認のまま）:", false);
    let mut answer = String::new();
    let _ = std::io::stdin().read_line(&mut answer);
    if !answer.trim().eq_ignore_ascii_case("y") {
        return fail(&format!("{}: 未確認のまま", def.file));
    }
    match client::call(&Request::Approve { id, sha256: def.sha256 }) {
        Ok(text) => {
            say(&text, false);
            0
        }
        Err(e) => fail(&format!("koyomi: {e}")),
    }
}

/// The next times one net's `[[when]]` comes, as the window lists them.
fn next(args: &[String]) -> i32 {
    let mut id = None;
    let mut count = 10usize;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--count" {
            match it.next().and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=1000).contains(n)) {
                Some(n) => count = n,
                None => return usage("--count takes a number from 1 to 1000"),
            }
        } else if id.is_none() {
            id = Some(a.clone());
        } else {
            return usage("next takes one group/name");
        }
    }
    let Some(id) = id else { return usage("next takes one group/name") };
    let t = table();
    let def: Def = match client::call(&Request::Show { id: id.clone() }).and_then(|s| toml::from_str(&s).map_err(|e| e.to_string())) {
        Ok(d) => d,
        Err(e) => return fail(&format!("koyomi: {e}")),
    };
    let Some(net) = &def.net else {
        for p in &def.problems {
            say(&p.in_file(&def.file), true);
        }
        return fail(&format!("{}: 定義の誤り", def.file));
    };
    if net.when.is_empty() {
        say(t.tr("run.next_none"), false);
        return 0;
    }
    if def.standing != Standing::Confirmed {
        say(&format!("{}: {}", def.file, t.tr("run.next_unconfirmed")), false);
    }
    let rows = show::next_rows(&t, &net.when, chrono::Local::now().naive_local(), count);
    if rows.is_empty() {
        say(t.tr("run.next_over"), false);
    }
    for r in rows {
        say(&r, false);
    }
    0
}

fn runs() -> Result<Vec<Run>, String> {
    Runs::parse(&client::call(&Request::Runs)?).map(|r| r.runs)
}

fn fail(text: &str) -> i32 {
    say(&format!("koyomi: {text}"), true);
    1
}

fn run_cmd(args: &[String]) -> i32 {
    let mut rest = args.iter();
    let Some(id) = rest.next() else { return usage("run takes one group/name") };
    let id = id.replace('\\', "/");
    let (mut date, mut from, mut wait) = (None, None, false);
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--date" => match rest.next() {
                Some(d) => date = Some(d.clone()),
                None => return usage("--date takes YYYY-MM-DD"),
            },
            "--from" => match rest.next() {
                Some(j) => from = Some(j.clone()),
                None => return usage("--from takes a job"),
            },
            "--wait" => wait = true,
            _ => return usage("run takes --from, --date and --wait"),
        }
    }
    started(client::call(&Request::Run { id, date, from }), wait)
}

fn rerun_cmd(args: &[String]) -> i32 {
    let mut rest = args.iter();
    let Some(run) = rest.next() else { return usage("rerun takes one run") };
    let (mut from, mut wait) = (None, false);
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--from" => match rest.next() {
                Some(j) => from = Some(j.clone()),
                None => return usage("--from takes a job"),
            },
            "--wait" => wait = true,
            _ => return usage("rerun takes --from and --wait"),
        }
    }
    started(client::call(&Request::Rerun { run: run.clone(), from }), wait)
}

/// Says the run started; with `wait`, stays until it ends and says how (exit 1 unless it passed).
fn started(reply: Result<String, String>, wait: bool) -> i32 {
    let run = match reply {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };
    let t = table();
    say(&t.trf("run.started", &[&run]), false);
    if !wait {
        return 0;
    }
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let found = match runs() {
            Ok(rs) => rs.into_iter().find(|r| r.id == run),
            Err(e) => return fail(&e),
        };
        let Some(r) = found else { return fail(&format!("実行が見当たらない: {run}")) };
        if r.is_over() {
            say(&show::run_line(&t, &r), false);
            for j in &r.jobs {
                say(&format!("  {}", show::job_line(&t, j)), false);
            }
            return i32::from(!r.state.passed());
        }
    }
}

fn status(args: &[String]) -> i32 {
    let (mut today, mut toml) = (false, false);
    for a in args {
        match a.as_str() {
            "--today" => today = true,
            "--toml" => toml = true,
            _ => return usage("status takes --today and --toml"),
        }
    }
    let rs = match runs() {
        Ok(rs) => rs,
        Err(e) => return fail(&e),
    };
    let rs: Vec<Run> = rs.into_iter().filter(|r| today || !r.is_over()).collect();
    if toml {
        say(Runs { runs: rs }.to_toml().trim_end_matches('\n'), false);
        return 0;
    }
    let t = table();
    if rs.is_empty() {
        say(if today { t.tr("run.none") } else { "動いている実行はありません" }, false);
    }
    for r in &rs {
        say(&show::run_line(&t, r), false);
        for j in &r.jobs {
            say(&format!("  {}", show::job_line(&t, j)), false);
        }
    }
    0
}

fn log(args: &[String]) -> i32 {
    let [run, job] = args else { return usage("log takes a run and a job") };
    match client::call(&Request::Log { run: run.clone(), job: job.clone() }) {
        Ok(text) => {
            say(text.trim_end_matches('\n'), false);
            0
        }
        Err(e) => fail(&e),
    }
}

/// Sends `request` and prints the answer: the commands with nothing to read back.
fn ask_and_say(request: &Request) -> i32 {
    match client::call(request) {
        Ok(text) => {
            say(&text, false);
            0
        }
        Err(e) => fail(&e),
    }
}

fn one_id(args: &[String], name: &str, make: fn(String) -> Request) -> i32 {
    match args {
        [id] => ask_and_say(&make(id.clone())),
        _ => usage(&format!("{name} takes a group/name")),
    }
}

fn plain(args: &[String], name: &str, request: Request) -> i32 {
    if args.is_empty() {
        ask_and_say(&request)
    } else {
        usage(&format!("{name} takes nothing"))
    }
}

fn skip(args: &[String]) -> i32 {
    match args {
        [run, job] => ask_and_say(&Request::Skip { run: run.clone(), job: job.clone() }),
        _ => usage("skip takes a run and a job"),
    }
}

fn kill(args: &[String]) -> i32 {
    let (run, job) = match args {
        [run] => (run.clone(), None),
        [run, job] => (run.clone(), Some(job.clone())),
        _ => return usage("kill takes a run and maybe a job"),
    };
    match client::call(&Request::Kill { run, job }) {
        Ok(text) => {
            say(&text, false);
            0
        }
        Err(e) => fail(&e),
    }
}

fn engine_cmd(args: &[String]) -> i32 {
    match args {
        [] => engine::run(),
        [a] if a == "status" => match ask(&proto::door(), &Request::Status) {
            Ok(text) => {
                say(&format!("{text}\n{}", startup_line()), false);
                0
            }
            Err(_) => {
                say("koyomi: エンジンは動いていない", true);
                1
            }
        },
        [a] if a == "stop" => match ask(&proto::door(), &Request::Stop) {
            Ok(_) => {
                say("koyomi: エンジンを止めた", false);
                0
            }
            Err(_) => {
                say("koyomi: エンジンは動いていない", true);
                1
            }
        },
        _ => usage("engine takes status or stop"),
    }
}

/// Whether the engine starts at sign-in, in a line.
fn startup_line() -> String {
    match ito_common::startup::enabled("koyomi") {
        true => "サインイン時の起動: あり".to_owned(),
        false => "サインイン時の起動: なし".to_owned(),
    }
}

fn startup_cmd(args: &[String]) -> i32 {
    let on = match args {
        [] => {
            say(&startup_line(), false);
            return 0;
        }
        [a] if a == "on" => true,
        [a] if a == "off" => false,
        _ => return usage("startup takes on or off"),
    };
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            say(&format!("koyomi: {e}"), true);
            return 1;
        }
    };
    match ito_common::startup::set("koyomi", &exe, &STARTUP_ARGS, on) {
        Ok(()) => {
            say(
                if on {
                    "koyomi: サインイン時にエンジンを起動する"
                } else {
                    "koyomi: サインイン時には起動しない"
                },
                false,
            );
            0
        }
        Err(e) => {
            say(&format!("koyomi: {e}"), true);
            1
        }
    }
}
