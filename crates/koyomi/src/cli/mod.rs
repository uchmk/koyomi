//! The commands that print and exit: `check`, `list`, `show`, `approve`, `run`, `rerun`, `next`, `status`,
//! `history`, `log`, `kill`, `hold`, `release`, `skip`, `pause`, `resume`, `calendar`, `engine`, `startup`, `secret`,
//! `archive` (in `archive.rs`).
//!
//! Every command takes `--help` (its lines of the usage) and `--toml` (the answer as TOML, for
//! another program: the commands that answer with data print it as such, the rest print `ok`
//! and the `lines` they would have said).
//!
//! Exit codes: 0 done, 1 refused or not so (a problem in a file, not approved, an engine already up),
//! 2 used wrongly.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use chrono::{Datelike, NaiveDate};
use ito_ipc::door::ask;
use koyomi_core::backfill::Only;
use koyomi_core::calendar::{Calendar, Calendars, Day, Table, TABLE_FILE};
use koyomi_core::run::{Run, State};
use koyomi_core::scan::{Def, Standing};
use serde::Serialize;

use crate::proto::{self, Request, Runs, Snapshot};
use crate::{client, engine, review, say, show};

mod archive;

pub const USAGE: &str = "\
usage: koyomi [--version | --help | env]
       koyomi check <file>           read a definition (or koyomi.toml, _group.toml, calendars/<name>.toml)
                                     and say what is wrong with it (exit 1 if anything)
       koyomi list [--group <group>] [--csv <file>]
                                     the definitions and where each stands (--group: that group's only;
                                     --csv: every job of them as a CSV table, - for standard output)
       koyomi show <group/name>      the definition as the engine reads it: what `approve` shows, without asking
       koyomi approve <group/name>   show what it would run, and approve it on `y`
       koyomi run <group/name> [--from <job>] [--date YYYY-MM-DD[..YYYY-MM-DD] [--only missing|failed]] [--set name=value]... [--dry] [--wait]
                                     start the net now (--from: at that job, the ones before it skipped;
                                     --date: for that business day, or each day of A..B in turn; --only: of A..B, only the days the net
                                     has no run for (missing), or those and the days whose last run failed (failed); left out are
                                     the days off its schedule, those whose last record is a skip, and those older than keep_days
                                     with no record left (archive/ is read too); refused on a standby that has not taken over;
                                     --dry: start nothing, print each job's command with the dates written out, or with --only
                                     each day and why it runs or not; --set: a value for the net's [params]; --wait: stay until it ends)
       koyomi rerun <run> [--from <job> | --next <job> [--warn]] [--wait]
                                     run it again: what passed is carried over, the rest runs (--from: that job and after it too;
                                     --next: the jobs after it, the job itself ends as skipped, or as warn with --warn)
       koyomi next <group/name> [--count N]
                                     the next 10 times its [[when]] comes (N: that many)
       koyomi status [--today]       the runs still going (--today: every run of today)
       koyomi history [--from YYYY-MM-DD] [--to YYYY-MM-DD] [--status <state>] [--csv <file>] [<group/name>]
                                     the runs of those days, oldest first (the last 7 days when none;
                                     --status: ok, warn, error, timeout, killed, running, running_error,
                                     waiting, held, skipped, missed, locked or not_run; --csv: one row per run;
                                     the days that went into archive/ are read from there)
       koyomi log <run> <job>        the tail of one job's log (gone from logs/: from archive/)
       koyomi archive [list]         the month zips that archive = true in koyomi.toml keeps in archive/,
                                     and the days of history and logs each holds
       koyomi archive show <YYYY-MM-DD>  that day's runs as archive/ keeps them
       koyomi archive log <run> <job>    that job's whole log from archive/
       koyomi compare <run-a> <run-b>
                                     two runs side by side: how long each job took, its state and exit code, and how the logs differ
       koyomi kill <run> [<job>]     stop one job, or the whole run
       koyomi hold <group/name> [<job>]
                                     hold the net: its times are written down as 保留 and nothing runs;
                                     with <job>, a run that comes to that job waits there as 保留 (the jobs
                                     after it wait too) and goes on as the same run once it is released
       koyomi release <group/name> [<job>]
                                     take the net (or its job) off hold
       koyomi skip <run> <job>       skip one job that has not started; the jobs after it go on
       koyomi approve-step <run> <job> [--no]
                                     answer a 確認待ち part: let the run go on, or --no to end it 異常
       koyomi shift <group/name> <from> <to|--cancel>
                                     start its time <from> at <to> this once (HH:MM is today, or YYYY-MM-DD HH:MM);
                                     the definition does not change; --cancel takes the move back
       koyomi skip-next <group/name> [--cancel]
                                     do not start its next time, once (written down as skipped); again skips the
                                     time after it; the definition does not change; --cancel takes the skips back
       koyomi ops [--from YYYY-MM-DD] [--to YYYY-MM-DD]
                                     what people did by hand (run, kill, hold, save, approve ...), oldest first (today when none)
       koyomi uses <calendar|group/name|program>
                                     where it is used: nets by calendar, child nets, nets that wait for or follow it, jobs that run it
       koyomi pause | resume         stop the whole schedule / start it again (runs by hand still go)
       koyomi standby [giveback]     with [standby]: watching the other PC or taken over; giveback hands the schedules back
                                     (give_back = auto does it once the other PC answers again; with [primary] there,
                                     it takes them back itself). With [primary]: whether the standby has them
       koyomi calendar <name> [--month YYYY-MM]
                                     the month as calendars/<name>.toml counts it (this month when none)
       koyomi report [YYYY-MM] [--csv <file>]
                                     the month's report (this month when none): for each net how many runs, the share that
                                     passed, how long the good ones took on average, and the jobs that ended 異常 most
       koyomi plan [YYYY-MM] [--pdf <file>] [--print] [--csv <file>]
                                     what starts on each day of the month (this month when none), by the approved
                                     nets' times and calendars; --pdf: write it as A4 pages; --print: send that PDF to the printer;
                                     --csv: one row per start
       koyomi map [--dot | --mermaid]
                                     how the nets depend on each other (child net, after_jobnet, wait_for), one line each;
                                     --dot: a Graphviz graph; --mermaid: a Mermaid flowchart
       koyomi new <group/name>       write a new definition from the template (one job, no time yet; unconfirmed)
       koyomi versions <group/name>  the versions kept before each save and restore, newest first
       koyomi restore <group/name> <version>
                                     put that version back as the file (unconfirmed until approved)
       koyomi rename <group/name> <group/name> [--fix]
                                     give the net a new name or group (its versions, approval and hold go with it);
                                     refused while other nets use it, unless --fix points them at the new name too
       koyomi copy <group/name> <group/name>
                                     write the net again under the new name (unconfirmed)
       koyomi replace <find> <with> [--apply]
                                     every line of every net that holds <find>, as it is and as it would
                                     read; with --apply they are written so (unconfirmed; the old form is kept as a version)
       koyomi delete <group/name>    take the net away on `y` (kept as a version: restore brings it back);
                                     refused while other nets use it
       koyomi stage <group/name> <file> --from YYYY-MM-DD
                                     use <file> as the net from that day's first time on: the change from the
                                     net now is shown, and `y` approves and keeps it (one per net); until that day
                                     the net runs as it is
       koyomi stage <group/name> --cancel
                                     drop what was staged: the net runs as it is
       koyomi export -o <file> [-n <group/name>]... [--group <group>] [--calendars] [--settings] [--all]
                                     write those definitions (with their group file, calendar and child nets),
                                     the calendars, koyomi.toml (webhook addresses as ***) or everything as one file
       koyomi import <file> [--dry-run] [--on-conflict overwrite|rename|skip]
                                     bring such a file in: say what would change, and do it when nothing is
                                     different there or --on-conflict says what to do (what is written over is
                                     kept under versions/; the nets come in unconfirmed; --dry-run: only say)
                                     a Task Scheduler task's XML (schtasks /query /xml) comes in as one net
       koyomi engine [status|stop]   run the engine in the foreground / ask it / stop it
       koyomi engine --no-tray       the engine without its tray icon: the system's notifications and the
                                     sounds wait for koyomi tray to show them
       koyomi tray                   the tray icon and the notifications for an engine started with --no-tray
                                     (its menu closes only the tray)
       koyomi service [install|uninstall|start|stop|status]  run the engine as a Windows service under your own account, so nets run without anyone signed in (install and uninstall need an administrator; what the engine says goes to engine.log)
       koyomi startup [on|off]       start the engine at sign-in, or stop doing so (nothing: say which)
       koyomi secret set|delete|check <name>  keep a value (read from stdin) in the OS credential store for a job's env or [ping] (secret:<name>)
       koyomi sftp hostkey <host>[:<port>]  the server's host key, as the line a send, fetch or ssh job takes (host_key = SHA256:…)
       koyomi ssh run                what an ssh job runs (kind = ssh): its command on the PC in on = ssh://…
       koyomi winrm cert <host>[:<port>]  the certificate a Windows PC shows for WinRM over HTTPS, as the line a winrm job takes (cert = AB:CD:…)
       koyomi winrm run              what a winrm job runs (kind = winrm): its command on the Windows PC in on = winrm://…
       koyomi mail run               what a mail job runs (kind = mail): its mail, through a [[notify.mails]] place
       koyomi http fingerprint       this PC's certificate fingerprint for [http] lan = true, to write in another PC's [[remote]]
       koyomi remote [list]          the other PCs' engines in [[remote]] of koyomi.toml, and whether each answers
       koyomi remote peek <url>      the fingerprint https://<PC>:<port> shows, to compare with koyomi http fingerprint there
       koyomi remote status|nets|runs <name>  that PC's engine: its status line, its nets, today's runs
       koyomi remote run <name> <group/name>  start a net on that PC (its [http] token must be kept here as the secret [[remote]] names)
       koyomi remote send <name> <group/name>
                                     this PC's net to that PC: the change from what it has is shown, and `y` sends
                                     it; it lands there unconfirmed and runs once that PC's window confirms it
                                     (that PC's token needs the edit role)
       koyomi mcp                    an MCP server on stdin/stdout for Claude Code: claude mcp add koyomi -- koyomi mcp
";

/// What every command takes, under the usage.
pub const NOTE: &str = "every command takes --help (its lines) and --toml (the answer as TOML, for other programs)\n\
a --csv file is UTF-8 with a BOM, for Excel; one ending in .xlsx is an Excel workbook\n";

/// What the sign-in entry runs: the engine, not the window; only the tray when the service runs the engine.
pub fn startup_args() -> [&'static str; 1] {
    if crate::service::state().is_some() {
        ["tray"]
    } else {
        ["engine"]
    }
}

/// Set to `1`, it lets a service or the sign-in start point at an exe in Temp anyway: the owner's
/// tests install from there (TODO 89).
pub const ALLOW_TEMP: &str = "KOYOMI_ALLOW_TEMP";

/// Refuses to point a service or the sign-in start at an exe in a Temp folder: cleanup deletes it,
/// and Defender took such a one for persistence (TODO 89, DECISIONS「Windows のサービスとして動かす」).
pub fn refuse_temp(exe: &std::path::Path) -> Result<(), String> {
    temp_words(exe, ito_common::in_temp(exe), std::env::var(ALLOW_TEMP).is_ok_and(|v| v.trim() == "1"))
}

fn temp_words(exe: &std::path::Path, in_temp: bool, allowed: bool) -> Result<(), String> {
    if !in_temp || allowed {
        return Ok(());
    }
    Err(format!(
        "{} は Temp のフォルダーにあり、掃除で消える。`~\\.cargo\\bin` か Program Files に置いて、そこから回す（試すときは {ALLOW_TEMP}=1 で通る）",
        exe.display()
    ))
}

/// Words for the person at the console: common.toml's language.
pub fn table() -> ito_i18n::Table {
    let common = ito_common::base_dir().and_then(|b| ito_common::Common::read(&b).ok()).map(|(c, _)| c).unwrap_or_default();
    crate::i18n::table(&crate::i18n::resolve(common.language.as_deref())).0
}

const COMMANDS: [&str; 50] = [
    "check",
    "list",
    "show",
    "approve",
    "run",
    "rerun",
    "next",
    "status",
    "history",
    "log",
    "archive",
    "compare",
    "kill",
    "hold",
    "release",
    "skip",
    "approve-step",
    "shift",
    "skip-next",
    "ops",
    "uses",
    "pause",
    "resume",
    "standby",
    "calendar",
    "plan",
    "report",
    "map",
    "new",
    "versions",
    "restore",
    "rename",
    "copy",
    "replace",
    "delete",
    "stage",
    "export",
    "import",
    "engine",
    "tray",
    "service",
    "startup",
    "secret",
    "sftp",
    "ssh",
    "winrm",
    "mail",
    "http",
    "remote",
    "mcp",
];

/// Runs `args` (after the program name) when they are one of these commands; `None` when they are not.
///
/// `--help` anywhere after the command prints its lines of the usage. `--toml` anywhere
/// makes the answer TOML (see [`finish`]).
pub fn run(args: &[String]) -> Option<i32> {
    // `--toml` may come before the command too (`koyomi --toml list`).
    let bare: Vec<String> = args.iter().filter(|a| *a != "--toml").cloned().collect();
    let (first, rest) = bare.split_first()?;
    if !COMMANDS.contains(&first.as_str()) {
        return None;
    }
    if rest.iter().any(|a| a == "--help" || a == "-h") {
        say(&help_for(first), false);
        return Some(0);
    }
    if bare.len() != args.len() {
        CAPTURE.with(|c| *c.borrow_mut() = Some(Vec::new()));
    }
    let code = match first.as_str() {
        "check" => check(rest),
        "list" => list(rest),
        "show" => show_cmd(rest),
        "approve" => approve(rest),
        "run" => run_cmd(rest),
        "rerun" => rerun_cmd(rest),
        "next" => next(rest),
        "status" => status(rest),
        "history" => history(rest),
        "plan" => plan(rest),
        "report" => report(rest),
        "map" => map_cmd(rest),
        "log" => log(rest),
        "archive" => archive::cmd(rest),
        "compare" => compare(rest),
        "kill" => kill(rest),
        "hold" => id_and_job(rest, "hold", |id, job| Request::Hold { id, job }),
        "release" => id_and_job(rest, "release", |id, job| Request::Release { id, job }),
        "skip" => skip(rest),
        "approve-step" => approve_step(rest),
        "shift" => shift(rest),
        "skip-next" => skip_next(rest),
        "ops" => ops(rest),
        "uses" => uses(rest),
        "pause" => plain(rest, "pause", Request::Pause),
        "resume" => plain(rest, "resume", Request::Resume),
        "standby" => match rest {
            [] => ask_and_say(&Request::Standby { give_back: false }),
            [op] if op == "giveback" => ask_and_say(&Request::Standby { give_back: true }),
            _ => usage("standby takes nothing or giveback"),
        },
        "calendar" => calendar(rest),
        "new" => new_cmd(rest),
        "versions" => versions(rest),
        "restore" => restore(rest),
        "rename" => rename(rest),
        "copy" => copy(rest),
        "replace" => replace(rest),
        "delete" => delete(rest),
        "stage" => stage(rest),
        "export" => export(rest),
        "import" => import(rest),
        "engine" => engine_cmd(rest),
        "tray" if rest.is_empty() => engine::apart(),
        "tray" => usage("tray takes nothing"),
        "service" => crate::service::cmd(rest),
        "startup" => startup_cmd(rest),
        "secret" => secret_cmd(rest),
        "sftp" => crate::sftp::cmd(rest),
        "ssh" => crate::ssh::cmd(rest),
        "winrm" => crate::winrm::cmd(rest),
        "mail" => crate::mail::cmd(rest),
        "http" => crate::remote::http_cmd(rest),
        "remote" => crate::remote::cmd(rest),
        "mcp" => crate::mcp::run(),
        _ => return None,
    };
    Some(finish(code))
}

/// The usage's lines for `cmd`: the line naming it, the ones that go on under it, and the note.
fn help_for(cmd: &str) -> String {
    let mut lines = Vec::new();
    let mut inside = false;
    for l in USAGE.lines() {
        if let Some(h) = l.trim_start().strip_prefix("koyomi ") {
            let words = h.split("  ").next().unwrap_or(h);
            inside = words.split([' ', '|']).any(|w| w == cmd);
        }
        if inside {
            lines.push(l);
        }
    }
    if lines.is_empty() {
        return format!("{USAGE}\n{NOTE}");
    }
    format!("usage:\n{}\n\n{NOTE}", lines.join("\n"))
}

thread_local! {
    /// Under `--toml`: the lines the command said, printed at the end as `lines`.
    /// `None` prints them as they come.
    static CAPTURE: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// Whether `--toml` was given.
fn toml() -> bool {
    CAPTURE.with(|c| c.borrow().is_some())
}

/// Says a line, or keeps it for the end under `--toml`.
fn out(text: &str, err: bool) {
    let kept = CAPTURE.with(|c| match c.borrow_mut().as_mut() {
        Some(lines) => {
            lines.extend(text.lines().map(str::to_owned));
            true
        }
        None => false,
    });
    if !kept {
        say(text, err);
    }
}

/// Prints `text`, the command's answer as TOML. The lines kept so far were for a person: dropped.
fn emit(text: &str) {
    CAPTURE.with(|c| *c.borrow_mut() = None);
    say(text.trim_end_matches('\n'), false);
}

/// What `--toml` prints for a command that answers in words: whether it went through, and the words.
#[derive(Serialize)]
struct Said {
    ok: bool,
    lines: Vec<String>,
}

/// Under `--toml`, prints what was kept as [`Said`] unless the command printed its own TOML.
fn finish(code: i32) -> i32 {
    if let Some(lines) = CAPTURE.with(|c| c.borrow_mut().take()) {
        let text = toml::to_string(&Said { ok: code == 0, lines }).unwrap_or_default();
        say(text.trim_end_matches('\n'), false);
    }
    code
}

fn usage(why: &str) -> i32 {
    say(&format!("koyomi: {why}\n\n{USAGE}"), true);
    2
}

fn check(args: &[String]) -> i32 {
    let [file] = args else { return usage("check takes one file") };
    let path = std::path::Path::new(file);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| file.clone());
    if name == TABLE_FILE {
        return check_table(&name, path);
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            out(&format!("{name}: 読めない: {e}"), true);
            return 1;
        }
    };
    if name == "koyomi.toml" {
        return check_config(&name, &text);
    }
    let paths = engine::Paths::find().ok();
    let parent = path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if parent == "calendars" {
        return check_calendar(&name, &text, path);
    }
    if name == "_group.toml" {
        return match koyomi_core::group::parse(&text) {
            Ok(_) => {
                out(&format!("{name}: 問題なし"), false);
                0
            }
            Err(e) => {
                out(&format!("{name}: {e}"), true);
                1
            }
        };
    }
    let mut bad = false;
    // A file under the settings folder runs only if nobody else can change it.
    if let Some(paths) = &paths {
        let inside = path.canonicalize().ok().filter(|p| paths.app.canonicalize().is_ok_and(|a| p.starts_with(a)));
        let files: Vec<_> = inside.into_iter().collect();
        for w in engine::access_warnings(paths, &files) {
            out(&w, true);
            bad = true;
        }
    }
    // Read as the engine would: with the group's `_group.toml` beside it and the calendars of the settings folder.
    let shared = path.parent().map(|p| koyomi_core::group::load(&p.join("_group.toml"))).unwrap_or(Ok(Default::default()));
    let cals = paths.as_ref().map(|p| Calendars::load(&p.calendars)).unwrap_or_default();
    let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| name.clone());
    let def = koyomi_core::scan::read_def_in(path, &parent, &stem, &Default::default(), shared.as_ref(), &cals);
    let mut problems = def.problems.clone();
    if let (Some(net), Some(paths)) = (&def.net, &paths) {
        // The child jobnets it uses, as the engine would find them.
        let defs = koyomi_core::scan::scan_plain(&paths.jobnets, &Default::default(), &cals);
        problems.extend(koyomi_core::scan::child_problem(net, &defs).map(|message| koyomi_core::jobnet::Problem { line: 0, message }));
        // And the nets it runs after: there, and not coming round to it.
        let id = format!("{parent}/{stem}");
        problems.extend(koyomi_core::scan::after_problem(&id, net, &defs).map(|message| koyomi_core::jobnet::Problem { line: 0, message }));
    }
    for p in &problems {
        out(&p.in_file(&name), true);
        bad = true;
    }
    // What the net names, on this machine: only when the file itself reads.
    if let Some(net) = def.net.as_ref().filter(|_| problems.is_empty()) {
        let path_var = std::env::var("PATH").unwrap_or_default();
        let exts: Vec<String> = if cfg!(windows) {
            std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned()).split(';').map(str::to_owned).collect()
        } else {
            Vec::new()
        };
        let exts: Vec<&str> = exts.iter().map(String::as_str).collect();
        let today = chrono::Local::now().date_naive();
        let config = paths.as_ref().and_then(|p| koyomi_core::config::Config::load(&p.config).ok());
        // Where a job without `dir` runs: `work_dir = "definition"` is this file's own folder.
        let folder = std::path::absolute(path).ok().and_then(|p| p.parent().map(std::path::Path::to_path_buf));
        let work = config.as_ref().and_then(|c| c.work_folder(folder.as_deref()));
        for f in koyomi_core::preflight::check(net, &cals, today, engine::CAUTION_DAYS, &path_var, &exts, work.as_deref()) {
            out(&format!("{name}: {}", f.text), true);
            bad |= !f.warn;
        }
        // The places it names, in `koyomi.toml`: a name none has is told to no webhook and no mail.
        if let Some(config) = config {
            let (n, _) = crate::notify::read(&config);
            let unknown = crate::notify::unknown(&n, net);
            if !unknown.is_empty() {
                out(&format!("{name}: 注意: koyomi.toml の [notify] にその名前の通知先がない: {}", unknown.join(", ")), true);
            }
            // A `mail` job's place to send through: one it would not find fails when it runs.
            for w in crate::mail::unknown(&n.mail_places, net) {
                out(&format!("{name}: 注意: {w}"), true);
            }
            // A `{var:…}` that `[vars]` does not have stops the run that reads it.
            let vars = config.vars();
            for v in koyomi_core::template::vars_of(net).into_iter().filter(|v| !vars.contains_key(v)) {
                out(&format!("{name}: 注意: {{var:{v}}} が koyomi.toml の [vars] にない（流すと止まる）"), true);
            }
            // An `ssh` or a `winrm` on a group: the group must be there, and each PC needs a key.
            for w in group_warnings(&config, net) {
                out(&format!("{name}: 注意: {w}（流すと止まる）"), true);
            }
        }
    }
    if !bad {
        out(&format!("{name}: 問題なし"), false);
    }
    i32::from(bad)
}

/// What an `ssh` or a `winrm` with `on = "group:…"` would stop on, by koyomi.toml:
/// a group it does not have, or PCs with no host key (certificate) where the job has
/// none either.
fn group_warnings(config: &koyomi_core::config::Config, net: &koyomi_core::jobnet::Jobnet) -> Vec<String> {
    use koyomi_core::chore::{self, Chore};
    use koyomi_core::jobnet::Part;
    let mut out = Vec::new();
    for j in &net.jobs {
        let (kind, key, on, own) = match &j.part {
            Part::Chore(Chore::Ssh { on, host_key, .. }) => ("ssh", "host_key", on, host_key.is_some()),
            Part::Chore(Chore::Winrm { on, cert, .. }) => ("winrm", "cert", on, cert.is_some()),
            _ => continue,
        };
        let Some(name) = chore::group(on) else { continue };
        match config.members(name, kind) {
            Err(e) => out.push(format!("{}: {e}", j.id)),
            Ok(_) if own => {}
            Ok(members) => {
                let bare: Vec<&str> = members
                    .iter()
                    .filter(|m| if kind == "ssh" { m.host_key.is_none() } else { m.cert.is_none() })
                    .map(|m| m.on.as_str())
                    .collect();
                if !bare.is_empty() {
                    out.push(format!("{}: 組 {name} の {} に {key} がなく、ジョブにもない", j.id, bare.join(", ")));
                }
            }
        }
    }
    out
}

/// `koyomi check calendars/<name>.toml`: the file reads, and how long its holiday table lasts.
fn check_calendar(name: &str, text: &str, path: &std::path::Path) -> i32 {
    let stem = name.strip_suffix(".toml").unwrap_or(name);
    let mut cal = match Calendar::parse(stem, text) {
        Ok(c) => c,
        Err(e) => {
            out(&format!("{name}: {e}"), true);
            return 1;
        }
    };
    // The Cabinet Office's table beside it is what the engine would use too.
    if cal.holidays {
        match Table::load(&path.with_file_name(TABLE_FILE)) {
            Ok(t) => cal.table = t,
            Err(e) => out(&format!("{TABLE_FILE}: {e}"), true),
        }
    }
    let today = chrono::Local::now().date_naive();
    let mut bad = false;
    if let Some(until) = cal.until() {
        if let Some(line) = koyomi_core::calendar::caution(until, today, engine::CAUTION_DAYS) {
            out(&format!("{name}: {line}"), true);
            // Past the table: the days after it will not run. Before: a word in time, not a fault.
            bad = cal.stale(today);
            if bad {
                return 1;
            }
            return 0;
        }
    }
    out(&format!("{name}: 問題なし"), false);
    i32::from(bad)
}

/// `koyomi check calendars/syukujitsu.csv`: the Cabinet Office's table reads, and how far it goes.
fn check_table(name: &str, path: &std::path::Path) -> i32 {
    match Table::load(path) {
        Ok(Some(t)) => {
            let (a, b) = t.years;
            let until = koyomi_core::calendar::until(Some(&t));
            out(&format!("{name}: 問題なし（{a}〜{b} 年の祝日 {} 日。祝日は {until} まで分かる）", t.days.len()), false);
            0
        }
        Ok(None) => {
            out(&format!("{name}: 読めない: ファイルがない"), true);
            1
        }
        Err(e) => {
            out(&format!("{name}: {e}"), true);
            1
        }
    }
}

/// `koyomi check koyomi.toml`: the settings read, and what `[notify]` would do.
fn check_config(name: &str, text: &str) -> i32 {
    let config = match koyomi_core::config::Config::parse(text) {
        Ok(c) => c,
        Err(e) => {
            out(&format!("{name}: {e}"), true);
            return 1;
        }
    };
    let (_, notes) = crate::notify::read(&config);
    for n in &notes {
        out(n, true);
    }
    if notes.is_empty() {
        out(&format!("{name}: 問題なし"), false);
    }
    i32::from(!notes.is_empty())
}

/// `--csv <to>`: writes `table` and says where (nothing said for `-`, the table is the output).
fn wrote(to: &str, table: &ito_table::Table) -> i32 {
    if let Err(e) = crate::sheet::write(to, table) {
        return fail(&format!("koyomi: {e}"));
    }
    if to != "-" {
        out(to, false);
    }
    0
}

fn snapshot() -> Result<Snapshot, String> {
    Snapshot::parse(&client::call(&Request::List)?)
}

fn list(args: &[String]) -> i32 {
    let (mut group, mut csv) = (None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let slot = match a.as_str() {
            "--group" => &mut group,
            "--csv" => &mut csv,
            _ => return usage("list takes --group <group> and --csv <file>"),
        };
        match it.next() {
            Some(v) => *slot = Some(v.clone()),
            None => return usage(&format!("{a} takes a value")),
        }
    }
    let mut snap = match snapshot() {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    if let Some(g) = &group {
        if !snap.defs.iter().any(|d| &d.group == g) {
            return fail(&format!("グループがない: {g}"));
        }
        snap.defs.retain(|d| &d.group == g);
    }
    if let Some(to) = csv {
        return wrote(&to, &crate::sheet::jobs(&table(), &snap.defs));
    }
    if toml() {
        emit(&snap.to_toml());
        return 0;
    }
    for w in &snap.warnings {
        out(&format!("! {w}"), false);
    }
    for n in snap.notes.iter().chain(&snap.cautions) {
        out(&format!("注意: {n}"), false);
    }
    if snap.paused {
        out(&format!("{} スケジュールを止めている（koyomi resume で戻す）", State::Held.mark()), false);
    }
    for line in review::tree(&table(), &snap.defs, &snap.held) {
        out(&line, false);
    }
    for e in snap.staged.iter().filter(|e| group.as_ref().is_none_or(|g| e.id.starts_with(&format!("{g}/")))) {
        out(&format!("→ {} から次の定義: {}", e.from, e.id), false);
    }
    0
}

/// `koyomi show <group/name>`: the definition as the engine reads it, the way
/// `approve` shows it before asking (held is said too). `--toml`: the definition.
fn show_cmd(args: &[String]) -> i32 {
    let [id] = args else { return usage("show takes one group/name") };
    let id = id.replace('\\', "/");
    let snap = match snapshot() {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    let text = match client::call(&Request::Show { id: id.clone() }) {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    if toml() {
        emit(&text);
        return 0;
    }
    let def: Def = match toml::from_str(&text) {
        Ok(d) => d,
        Err(e) => return fail(&e.to_string()),
    };
    let t = table();
    let mut head = format!("{} {}", def.file, review::word(&t, def.standing));
    if snap.held.contains(&def.id) {
        head.push_str(&format!("  {} {}", State::Held.mark(), t.tr("run.hold")));
    }
    out(&head, false);
    if let Some(e) = snap.staged.iter().find(|e| e.id == def.id) {
        out(&format!("→ {} から次の定義に切り替わる（koyomi stage {} --cancel でやめる）", e.from, def.id), false);
    }
    for p in &def.problems {
        out(&p.in_file(&def.file), true);
    }
    if def.net.is_some() {
        for l in review::body(&t, &def) {
            out(&l, false);
        }
        for l in review::week_in(&t, &def, &snap.calendars, chrono::Local::now().naive_local()) {
            out(&l, false);
        }
    }
    0
}

fn approve(args: &[String]) -> i32 {
    let [id] = args else { return usage("approve takes one group/name") };
    let id = id.replace('\\', "/");
    let t = table();
    let fail = |text: &str| {
        out(text, true);
        1
    };
    let snap = match snapshot() {
        Ok(s) => s,
        Err(e) => return fail(&format!("koyomi: {e}")),
    };
    if let Some(file) = snap.defs.iter().find(|d| d.id == id).map(|d| d.file.clone()) {
        let own = match engine::Paths::find() {
            Ok(paths) => engine::warnings_about(&paths, &snap.warnings, &snap.defs, &file),
            Err(_) => snap.warnings.clone(),
        };
        if !own.is_empty() {
            for w in &own {
                out(&format!("! {w}"), true);
            }
            return fail("承認できない: 上の警告を直してから（警告がある間は何も確認できない）");
        }
    }
    let def: Def = match client::call(&Request::Show { id: id.clone() }).and_then(|s| toml::from_str(&s).map_err(|e| e.to_string())) {
        Ok(d) => d,
        Err(e) => return fail(&format!("koyomi: {e}")),
    };
    match def.standing {
        Standing::Invalid => {
            for p in &def.problems {
                out(&p.in_file(&def.file), true);
            }
            return fail(&format!("{}: 定義の誤りがあり、承認できない", def.file));
        }
        Standing::Confirmed => {
            out(&format!("{}: すでに確認済み", def.file), false);
            return 0;
        }
        Standing::Unconfirmed => {}
    }
    say(&format!("{} {}", def.file, review::word(&t, def.standing)), false);
    for l in review::body(&t, &def) {
        say(&l, false);
    }
    for l in review::week_in(&t, &def, &snap.calendars, chrono::Local::now().naive_local()) {
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
            out(&text, false);
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
    let id = id.replace('\\', "/");
    let t = table();
    let snap = match snapshot() {
        Ok(s) => s,
        Err(e) => return fail(&format!("koyomi: {e}")),
    };
    let Some(def) = snap.defs.iter().find(|d| d.id == id) else { return fail(&format!("定義がない: {id}")) };
    let Some(net) = &def.net else {
        for p in &def.problems {
            out(&p.in_file(&def.file), true);
        }
        return fail(&format!("{}: 定義の誤り", def.file));
    };
    let cal = def.calendar.as_deref().and_then(|c| snap.calendars.get(c));
    let now = chrono::Local::now().naive_local();
    let unconfirmed = def.standing != Standing::Confirmed;
    if toml() {
        let mut next: Vec<NextRow> =
            net.when.iter().filter_map(|w| show::event_row(&t, w)).map(|e| NextRow { at: None, event: Some(e), stop: None }).collect();
        next.extend(koyomi_core::quiet::next_starts(&net.when, &net.quiet, net.on_quiet, now, count, cal).into_iter().map(|at| NextRow {
            at: Some(at.format("%Y-%m-%d %H:%M:%S").to_string()),
            event: None,
            stop: koyomi_core::blackout::find(&snap.blackouts, at).map(|b| b.reason()),
        }));
        emit(&toml::to_string(&Next { unconfirmed, next }).unwrap_or_default());
        return 0;
    }
    if net.when.is_empty() {
        out(t.tr("run.next_none"), false);
        return 0;
    }
    if unconfirmed {
        out(&format!("{}: {}", def.file, t.tr("run.next_unconfirmed")), false);
    }
    let rows = show::next_rows(
        &t,
        &net.when,
        (&net.quiet, net.on_quiet),
        cal,
        now,
        count,
        Some(&show::Sleep::of(&snap.power, net.wake)),
        &snap.blackouts,
    );
    if rows.is_empty() {
        out(t.tr("run.next_over"), false);
    }
    for r in rows {
        out(&r, false);
    }
    0
}

/// `koyomi next --toml`: `unconfirmed`, then a `[[next]]` with `at` (a time) or `event` (what starts it) each;
/// a time in a planned stop has `stop`, why it will not start.
#[derive(Serialize)]
struct Next {
    unconfirmed: bool,
    next: Vec<NextRow>,
}

#[derive(Serialize)]
struct NextRow {
    #[serde(skip_serializing_if = "Option::is_none")]
    at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<String>,
}

/// `koyomi calendar --toml`: the month, its business days, and a `[[day]]` each with
/// `kind` (`business`, `open`, `closed`, `holiday`, `weekly_off`) and the name written for it.
#[derive(Serialize)]
struct Month {
    calendar: String,
    month: String,
    business_days: usize,
    day: Vec<DayRow>,
}

#[derive(Serialize)]
struct DayRow {
    date: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

fn day_row(date: NaiveDate, day: Day) -> DayRow {
    let kind = match &day {
        Day::Business => "business",
        Day::Open(_) => "open",
        Day::Closed(_) => "closed",
        Day::Holiday(_) => "holiday",
        Day::WeeklyOff => "weekly_off",
    };
    let name = match day {
        Day::Open(n) | Day::Closed(n) if !n.is_empty() => Some(n),
        Day::Holiday(n) => Some(n.clone()),
        _ => None,
    };
    DayRow { date: date.to_string(), kind, name }
}

/// `koyomi calendar <name> [--month YYYY-MM]`: the month's days as the engine counts them.
fn plan(args: &[String]) -> i32 {
    let (mut month, mut pdf, mut print, mut csv) = (None, None, false, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--pdf" => match it.next() {
                Some(f) => pdf = Some(std::path::PathBuf::from(f)),
                None => return usage("--pdf takes a file"),
            },
            "--print" => print = true,
            "--csv" => match it.next() {
                Some(f) => csv = Some(f.clone()),
                None => return usage("--csv takes a file"),
            },
            m if month.is_none() && !m.starts_with("--") => match NaiveDate::parse_from_str(&format!("{m}-01"), "%Y-%m-%d") {
                Ok(d) => month = Some((d.year(), d.month())),
                Err(_) => return usage("plan takes YYYY-MM"),
            },
            _ => return usage("plan takes YYYY-MM, --pdf <file>, --print and --csv <file>"),
        }
    }
    let snap = match snapshot() {
        Ok(s) => s,
        Err(e) => return fail(&format!("koyomi: {e}")),
    };
    let today = chrono::Local::now().date_naive();
    let (y, m) = month.unwrap_or((today.year(), today.month()));
    let mut days = crate::plan::month(&snap.defs, &snap.calendars, y, m);
    let t = table();
    crate::plan::mark(&mut days, &snap.blackouts, |b| crate::show::stop_mark(&t, b));
    if let Some(to) = csv {
        let code = wrote(&to, &crate::sheet::plan(&table(), &days));
        if code != 0 || (pdf.is_none() && !print) {
            return code;
        }
    }
    if pdf.is_none() && !print {
        out(&crate::plan::text(&days), false);
        return 0;
    }
    // --print alone writes the PDF where the OS keeps temporary files.
    let file = pdf.unwrap_or_else(|| std::env::temp_dir().join(format!("koyomi-plan-{y}-{m:02}.pdf")));
    if let Err(e) = crate::plan::pdf(&days, y, m, &file) {
        return fail(&format!("koyomi: {e}"));
    }
    out(&file.display().to_string(), false);
    if print {
        if let Err(e) = ito_export::print(&file) {
            return fail(&format!("koyomi: {e}"));
        }
    }
    0
}

fn map_cmd(args: &[String]) -> i32 {
    let (mut dot, mut mermaid) = (false, false);
    for a in args {
        match a.as_str() {
            "--dot" => dot = true,
            "--mermaid" => mermaid = true,
            _ => return usage("map takes --dot or --mermaid"),
        }
    }
    if dot && mermaid {
        return usage("map takes --dot or --mermaid, not both");
    }
    let snap = match snapshot() {
        Ok(s) => s,
        Err(e) => return fail(&format!("koyomi: {e}")),
    };
    let links = koyomi_core::uses::links(&snap.defs);
    if dot || mermaid {
        out(&koyomi_core::uses::diagram(&links, mermaid), false);
    } else if links.is_empty() {
        out("(no links)", false);
    } else {
        out(&koyomi_core::uses::lines(&links), false);
    }
    0
}

fn calendar(args: &[String]) -> i32 {
    let mut name = None;
    let mut month = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--month" {
            match it.next().and_then(|m| NaiveDate::parse_from_str(&format!("{m}-01"), "%Y-%m-%d").ok()) {
                Some(d) => month = Some((d.year(), d.month())),
                None => return usage("--month takes YYYY-MM"),
            }
        } else if name.is_none() {
            name = Some(a.clone());
        } else {
            return usage("calendar takes one name");
        }
    }
    let Some(name) = name else { return usage("calendar takes one name") };
    let t = table();
    let snap = match snapshot() {
        Ok(s) => s,
        Err(e) => return fail(&format!("koyomi: {e}")),
    };
    let Some(cal) = snap.calendars.get(&name) else {
        for c in snap.cautions.iter().filter(|c| c.starts_with(&format!("calendars/{name}.toml"))) {
            out(c, true);
        }
        return fail(&t.trf("calendar.missing", &[&name]));
    };
    let today = chrono::Local::now().date_naive();
    let (y, m) = month.unwrap_or((today.year(), today.month()));
    if toml() {
        let days = show::month_days(cal, y, m);
        let month = Month {
            calendar: name.clone(),
            month: format!("{y}-{m:02}"),
            business_days: days.iter().filter(|(_, d)| d.is_business()).count(),
            day: days.into_iter().map(|(date, d)| day_row(date, d)).collect(),
        };
        emit(&toml::to_string(&month).unwrap_or_default());
        return 0;
    }
    for l in show::month_lines(&t, cal, y, m, today) {
        out(&l, false);
    }
    0
}

fn runs() -> Result<Vec<Run>, String> {
    Runs::parse(&client::call(&Request::Runs)?).map(|r| r.runs)
}

fn fail(text: &str) -> i32 {
    out(&format!("koyomi: {text}"), true);
    1
}

fn run_cmd(args: &[String]) -> i32 {
    let mut rest = args.iter();
    let Some(id) = rest.next() else { return usage("run takes one group/name") };
    let id = id.replace('\\', "/");
    let (mut date, mut from, mut wait, mut dry, mut only) = (None, None, false, false, None);
    let mut params = BTreeMap::new();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--dry" => dry = true,
            "--only" => match rest.next().and_then(|o| Only::parse(o.as_str())) {
                Some(o) => only = Some(o),
                None => return usage("--only takes missing or failed"),
            },
            "--set" => match rest.next().and_then(|kv| kv.split_once('=')) {
                Some((k, v)) if !k.trim().is_empty() => {
                    params.insert(k.trim().to_owned(), v.to_owned());
                }
                _ => return usage("--set takes name=value"),
            },
            "--date" => match rest.next() {
                Some(d) => date = Some(d.clone()),
                None => return usage("--date takes YYYY-MM-DD or YYYY-MM-DD..YYYY-MM-DD"),
            },
            "--from" => match rest.next() {
                Some(j) => from = Some(j.clone()),
                None => return usage("--from takes a job"),
            },
            "--wait" => wait = true,
            _ => return usage("run takes --from, --date, --only, --set, --dry and --wait"),
        }
    }
    if only.is_some() && !date.as_deref().is_some_and(|d| d.contains("..")) {
        return usage("--only goes with --date A..B");
    }
    if let Some(why) = only.and_then(|_| standby_refusal()) {
        return fail(&why);
    }
    if let Some(range) = date.as_deref().filter(|d| d.contains("..")) {
        let mut days = match day_range(range) {
            Ok(d) => d,
            Err(e) => return usage(&e),
        };
        if let Some(only) = only {
            days = match pick_days(&id, &days, only, dry) {
                Ok(d) => d,
                Err(e) => return fail(&e),
            };
            if dry {
                return 0;
            }
        }
        // One run per day, each to its end before the next starts (a net runs once at a time); stop at the first failure.
        for day in days {
            let date = Some(day.to_string());
            let code = if dry {
                match client::call(&Request::Run { id: id.clone(), date, from: from.clone(), dry, params: params.clone() }) {
                    Ok(t) => {
                        out(t.trim_end(), false);
                        0
                    }
                    Err(e) => fail(&e),
                }
            } else {
                started(client::call(&Request::Run { id: id.clone(), date, from: from.clone(), dry, params: params.clone() }), true)
            };
            if code != 0 {
                return code;
            }
        }
        return 0;
    }
    if dry {
        return match client::call(&Request::Run { id, date, from, dry, params }) {
            Ok(t) => {
                out(t.trim_end(), false);
                0
            }
            Err(e) => fail(&e),
        };
    }
    started(client::call(&Request::Run { id, date, from, dry, params }), wait)
}

/// The days of `A..B` (both ends included, at most a year).
fn day_range(text: &str) -> Result<Vec<NaiveDate>, String> {
    let (a, b) = text.split_once("..").ok_or("--date range is A..B")?;
    let day = |t: &str| NaiveDate::parse_from_str(t.trim(), "%Y-%m-%d").map_err(|_| format!("not a date: {t}"));
    let (a, b) = (day(a)?, day(b)?);
    if b < a {
        return Err("--date range ends before it starts".to_owned());
    }
    let days: Vec<NaiveDate> = a.iter_days().take_while(|d| *d <= b).take(367).collect();
    if days.len() > 366 {
        return Err("--date range is longer than a year".to_owned());
    }
    Ok(days)
}

/// The days of `days` that `--only` runs, by the net's runs for each business day. With `dry`,
/// every day is listed with why it runs or not; otherwise one line says which days run.
fn pick_days(id: &str, days: &[NaiveDate], only: Only, dry: bool) -> Result<Vec<NaiveDate>, String> {
    let Some(&first) = days.first() else { return Ok(Vec::new()) };
    let runs = runs_since(first)?;
    let planned = planned_days(id, days)?;
    let seen = koyomi_core::backfill::look(&runs, id, days, planned.as_ref(), kept_from());
    let picked: Vec<NaiveDate> = seen.iter().filter(|(_, d)| d.picked(only)).map(|(d, _)| *d).collect();
    let t = table();
    if dry {
        for (date, day) in &seen {
            out(&backfill_line(&t, *date, *day, only), false);
        }
    } else if picked.is_empty() {
        out("流す日はない", false);
    } else {
        let list: Vec<String> = picked.iter().map(NaiveDate::to_string).collect();
        out(&format!("{} 日のうち {} 日を流す: {}", days.len(), picked.len(), list.join(", ")), false);
    }
    if seen.iter().any(|(_, d)| matches!(d, koyomi_core::backfill::Day::Gone)) {
        out("記録が残っていない日は流さない。範囲の日を全部流すなら --only を外す", false);
    }
    Ok(picked)
}

/// The oldest day history keeps (koyomi.toml's `keep_days`); `None` when koyomi.toml cannot be read.
fn kept_from() -> Option<NaiveDate> {
    let config = koyomi_core::config::Config::load(&engine::Paths::find().ok()?.config).ok()?;
    Some(koyomi_core::retain::cutoff(chrono::Local::now().date_naive(), config.keep_days))
}

/// Why `--only` is refused here: the engine is a standby that has not taken over (it says where
/// it stands as the `"ha"` of `GET /health`). `None` on any other PC.
fn standby_refusal() -> Option<String> {
    let ha: serde_json::Value = serde_json::from_str(&client::call(&Request::Handover { release: false }).ok()?).ok()?;
    let watch = ha["watch"].as_str().filter(|_| ha["role"] == "standby");
    koyomi_core::backfill::refused_on_standby(watch, ha["active"].as_bool().unwrap_or(false))
}

/// One day of `run --only --dry`: whether it runs, and why.
fn backfill_line(t: &ito_i18n::Table, date: NaiveDate, day: koyomi_core::backfill::Day, only: Only) -> String {
    use koyomi_core::backfill::Day;
    let verdict = if day.picked(only) { "流す    " } else { "流さない" };
    let why = match day {
        Day::NotPlanned => "予定のない日".to_owned(),
        Day::Nothing => "実行の記録がない".to_owned(),
        Day::Gone => "記録が残っていない（keep_days より前）".to_owned(),
        Day::NotStarted(r) | Day::Skipped(r) | Day::Ran(r) => {
            let reason = if r.reason.is_empty() { String::new() } else { format!("  {}", koyomi_core::run::one_line(&r.reason)) };
            format!("{}  {}{reason}", show::tag(t, r.state), r.id)
        }
    };
    format!("{verdict}  {date}  {why}")
}

/// The runs whose business day may fall from `first` on: the ones that started from then (or
/// today, if that is earlier) to today, a year at a time (the engine reads the days the sweep put
/// into `archive/` from there), and the ones still going.
fn runs_since(first: NaiveDate) -> Result<Vec<Run>, String> {
    let today = chrono::Local::now().date_naive();
    let mut from = first.min(today);
    let mut by_id = BTreeMap::new();
    while from <= today {
        let to = (from + chrono::Duration::days(366)).min(today);
        for r in Runs::parse(&client::call(&Request::History { from: from.to_string(), to: to.to_string() })?)?.runs {
            by_id.insert(r.id.clone(), r);
        }
        from = to + chrono::Duration::days(1);
    }
    for r in runs()? {
        by_id.insert(r.id.clone(), r);
    }
    Ok(by_id.into_values().collect())
}

/// The days of `days` the net's times fall on (its calendar and quiet hours counted), or none to
/// count every day: a net that starts on events or by hand only, or one the engine does not plan.
fn planned_days(id: &str, days: &[NaiveDate]) -> Result<Option<BTreeSet<NaiveDate>>, String> {
    let snap = snapshot()?;
    let Some(def) = snap.defs.iter().find(|d| d.id == id) else { return Ok(None) };
    let timed = def.net.as_ref().is_some_and(|n| n.when.iter().any(|w| !w.is_event()));
    let Some(&first) = days.first() else { return Ok(None) };
    if !timed || def.standing != Standing::Confirmed {
        return Ok(None);
    }
    let plan = crate::plan::days(std::slice::from_ref(def), &snap.calendars, first, days.len() as u32);
    Ok(Some(plan.into_iter().filter(|(_, starts)| !starts.is_empty()).map(|(d, _)| d).collect()))
}

fn rerun_cmd(args: &[String]) -> i32 {
    let mut rest = args.iter();
    let Some(run) = rest.next() else { return usage("rerun takes one run") };
    let (mut from, mut next, mut warn, mut wait) = (None, false, false, false);
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--from" | "--next" if from.is_some() => return usage("rerun takes one of --from and --next"),
            "--from" | "--next" => match rest.next() {
                Some(j) => (from, next) = (Some(j.clone()), a == "--next"),
                None => return usage(&format!("{a} takes a job")),
            },
            "--warn" => warn = true,
            "--wait" => wait = true,
            _ => return usage("rerun takes --from, --next, --warn and --wait"),
        }
    }
    if warn && !next {
        return usage("--warn goes with --next");
    }
    let next = next.then_some(if warn { State::Warn } else { State::Skipped });
    started(client::call(&Request::Rerun { run: run.clone(), from, next }), wait)
}

/// Says the run started; with `wait`, stays until it ends and says how (exit 1 unless it passed).
fn started(reply: Result<String, String>, wait: bool) -> i32 {
    let run = match reply {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };
    let t = table();
    out(&t.trf("run.started", &[&run]), false);
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
            say_runs(&t, std::slice::from_ref(&r));
            return i32::from(!r.state.passed());
        }
    }
}

fn status(args: &[String]) -> i32 {
    let today = match args {
        [] => false,
        [a] if a == "--today" => true,
        _ => return usage("status takes --today"),
    };
    let rs = match runs() {
        Ok(rs) => rs,
        Err(e) => return fail(&e),
    };
    let rs: Vec<Run> = rs.into_iter().filter(|r| today || !r.is_over()).collect();
    if toml() {
        emit(&Runs { runs: rs }.to_toml());
        return 0;
    }
    let t = table();
    if rs.is_empty() {
        out(if today { t.tr("run.none") } else { "動いている実行はありません" }, false);
    }
    // The last 30 days tell how long each running net usually takes.
    let past = {
        let to = chrono::Local::now().date_naive();
        let reply = client::call(&Request::History { from: (to - chrono::Duration::days(30)).to_string(), to: to.to_string() });
        reply.and_then(|s| Runs::parse(&s)).map(|r| r.runs).unwrap_or_default()
    };
    let now = chrono::Local::now().naive_local();
    for r in &rs {
        out(&show::run_line(&t, r), false);
        if let Some(avg) = koyomi_core::run::average_duration(&past, &r.jobnet, 10) {
            let end = r.expected_end(avg, now).map(show::hms);
            let line = match end {
                Some(e) => t.trf("run.average_end", &[&show::duration(&t, avg), &e]),
                None => t.trf("run.average", &[&show::duration(&t, avg)]),
            };
            out(&format!("  {line}"), false);
        }
        for j in &r.jobs {
            out(&format!("  {}", show::job_line(&t, j)), false);
        }
    }
    if let Some(line) = client::call(&Request::Status).ok().as_deref().and_then(http_line) {
        out(line, false);
    }
    0
}

/// The `HTTP: …` line of the engine's status text, if it listens.
fn http_line(status: &str) -> Option<&str> {
    status.lines().find(|l| l.starts_with("HTTP: "))
}

/// The runs as `status` prints them: a line each, its jobs under it.
fn say_runs(t: &ito_i18n::Table, rs: &[Run]) {
    for r in rs {
        out(&show::run_line(t, r), false);
        for j in &r.jobs {
            out(&format!("  {}", show::job_line(t, j)), false);
        }
    }
}

/// `koyomi history [--from D] [--to D] [--status <state>] [<group/name>]`: the runs that
/// started on those days (the last 7 when none), oldest first, narrowed by state and net here.
fn history(args: &[String]) -> i32 {
    let (mut from, mut to, mut state, mut net, mut csv) = (None, None, None, None, None);
    let day = |d: Option<&String>| d.and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--from" => match day(it.next()) {
                Some(d) => from = Some(d),
                None => return usage("--from takes YYYY-MM-DD"),
            },
            "--to" => match day(it.next()) {
                Some(d) => to = Some(d),
                None => return usage("--to takes YYYY-MM-DD"),
            },
            "--status" => match it.next().and_then(|k| State::ALL.iter().find(|s| s.key() == k)) {
                Some(s) => state = Some(*s),
                None => {
                    let keys: Vec<&str> = State::ALL.iter().map(|s| s.key()).collect();
                    return usage(&format!("--status takes one of {}", keys.join(", ")));
                }
            },
            "--csv" => match it.next() {
                Some(f) => csv = Some(f.clone()),
                None => return usage("--csv takes a file"),
            },
            _ if net.is_none() && !a.starts_with("--") => net = Some(a.replace('\\', "/")),
            _ => return usage("history takes --from, --to, --status, --csv and one group/name"),
        }
    }
    let to = to.unwrap_or_else(|| chrono::Local::now().date_naive());
    let from = from.unwrap_or(to - chrono::Duration::days(7));
    // The engine reads the days the sweep put into archive/ from there.
    let reply = client::call(&Request::History { from: from.to_string(), to: to.to_string() });
    let rs = match reply.and_then(|s| Runs::parse(&s)) {
        Ok(r) => r.runs,
        Err(e) => return fail(&e),
    };
    let rs: Vec<Run> =
        rs.into_iter().filter(|r| net.as_deref().is_none_or(|n| r.jobnet == n) && state.is_none_or(|s| r.state == s)).collect();
    if let Some(to) = csv {
        return wrote(&to, &crate::sheet::runs(&table(), &rs));
    }
    if toml() {
        emit(&Runs { runs: rs }.to_toml());
        return 0;
    }
    let t = table();
    if rs.is_empty() {
        out(t.tr("run.history_none"), false);
    }
    say_runs(&t, &rs);
    0
}

/// `koyomi report [YYYY-MM] [--csv <file>]`: the runs that started in the month, summed up per net.
fn report(args: &[String]) -> i32 {
    let (mut month, mut csv) = (None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--csv" => match it.next() {
                Some(f) => csv = Some(f.clone()),
                None => return usage("--csv takes a file"),
            },
            m if month.is_none() && !m.starts_with("--") => match NaiveDate::parse_from_str(&format!("{m}-01"), "%Y-%m-%d") {
                Ok(d) => month = Some(d),
                Err(_) => return usage("report takes YYYY-MM"),
            },
            _ => return usage("report takes YYYY-MM and --csv <file>"),
        }
    }
    let first = month.unwrap_or_else(|| {
        let today = chrono::Local::now().date_naive();
        today.with_day(1).unwrap_or(today)
    });
    let last = (first + chrono::Duration::days(31)).with_day(1).map_or(first, |n| n - chrono::Duration::days(1));
    let reply = client::call(&Request::History { from: first.to_string(), to: last.to_string() });
    let runs = match reply.and_then(|s| Runs::parse(&s)) {
        Ok(r) => r.runs,
        Err(e) => return fail(&e),
    };
    let rows = koyomi_core::report::summarize(&runs);
    let t = table();
    if let Some(to) = csv {
        return wrote(&to, &crate::sheet::report(&t, &rows));
    }
    if toml() {
        #[derive(Serialize)]
        struct Month {
            month: String,
            net: Vec<koyomi_core::report::Row>,
        }
        emit(&toml::to_string(&Month { month: first.format("%Y-%m").to_string(), net: rows }).unwrap_or_default());
        return 0;
    }
    out(&first.format("%Y-%m").to_string(), false);
    for l in crate::report::lines(&t, &rows) {
        out(&l, false);
    }
    0
}

fn log(args: &[String]) -> i32 {
    let [run, job] = args else { return usage("log takes a run and a job") };
    // The engine reads it from archive/ once the sweep took it; with no engine to ask, read it here.
    let text = match client::call(&Request::Log { run: run.clone(), job: job.clone() }) {
        Ok(text) => text,
        Err(e) => match archive::log_text(run, job) {
            Ok(text) => text,
            Err(_) => return fail(&e),
        },
    };
    if toml() {
        let log = Log { run: run.clone(), job: job.clone(), text };
        emit(&toml::to_string(&log).unwrap_or_default());
    } else {
        out(text.trim_end_matches('\n'), false);
    }
    0
}

/// `koyomi compare <run-a> <run-b>`: the two runs, their jobs side by side and the logs' difference.
fn compare(args: &[String]) -> i32 {
    let [a, b] = args else { return usage("compare takes two runs") };
    let t = table();
    let (ra, rb) = match (find_run(&t, a), find_run(&t, b)) {
        (Ok(x), Ok(y)) => (x, y),
        (Err(e), _) | (_, Err(e)) => return fail(&e),
    };
    let logs: Vec<(String, Option<String>, Option<String>)> = crate::compare::jobs(&ra, &rb)
        .into_iter()
        .filter(|(_, x, y)| x.is_some_and(crate::compare::has_log) && y.is_some_and(crate::compare::has_log))
        .map(|(id, _, _)| {
            let read = |run: &str| client::call(&Request::Log { run: run.to_owned(), job: id.to_owned() }).ok();
            (id.to_owned(), read(&ra.id), read(&rb.id))
        })
        .collect();
    if toml() {
        emit(&toml::to_string(&crate::compare::Compare::new(&ra, &rb, &logs)).unwrap_or_default());
        return 0;
    }
    for l in crate::compare::lines(&t, &ra, &rb, &logs) {
        out(&l, false);
    }
    0
}

/// The run `id`, from the history of the day its id names (`YYYYMMDD-…`), else from the runs going now.
fn find_run(t: &ito_i18n::Table, id: &str) -> Result<Run, String> {
    let day = id.get(..8).and_then(|d| NaiveDate::parse_from_str(d, "%Y%m%d").ok());
    let mut found = Vec::new();
    if let Some(d) = day {
        found = client::call(&Request::History { from: d.to_string(), to: d.to_string() }).and_then(|s| Runs::parse(&s)).map(|r| r.runs)?;
    }
    if !found.iter().any(|r| r.id == id) {
        found.extend(runs()?);
    }
    found.into_iter().find(|r| r.id == id).ok_or_else(|| t.trf("compare.missing", &[id]))
}

/// `koyomi log --toml`: the run, the job and the tail.
#[derive(Serialize)]
struct Log {
    run: String,
    job: String,
    text: String,
}

/// `koyomi new <group/name>`: the template as `jobnets/<group>/<name>.toml`. Nothing is asked of
/// the engine: the file is like one written by hand, and stays unconfirmed until approved.
fn new_cmd(args: &[String]) -> i32 {
    let [id] = args else { return usage("new takes one group/name") };
    let id = id.replace('\\', "/");
    let Some((group, name)) = id.split_once('/').filter(|_| koyomi_core::jobnet::is_id(&id)) else {
        return fail(&format!("グループ/名前 の形で書く: {id}"));
    };
    let paths = match engine::Paths::find() {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    let path = paths.jobnets.join(group).join(format!("{name}.toml"));
    if path.exists() {
        return fail(&format!("もうある: {}", path.display()));
    }
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            return fail(&format!("{} を作れない: {e}", dir.display()));
        }
    }
    if let Err(e) = ito_common::write_atomic(&path, koyomi_core::write::template(name)) {
        return fail(&e);
    }
    out(&format!("書いた: {}（未確認。窓か koyomi approve で確かめてから流す）", path.display()), false);
    0
}

/// `koyomi versions <group/name>`: the versions the engine kept, newest first.
fn versions(args: &[String]) -> i32 {
    let [id] = args else { return usage("versions takes one group/name") };
    let id = id.replace('\\', "/");
    let text = match client::call(&Request::Versions { id: id.clone() }) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    if toml() {
        emit(&text);
        return 0;
    }
    let v = match proto::Versions::parse(&text) {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    if v.versions.is_empty() {
        out(&format!("版はまだ無い: {id}"), false);
    } else {
        out(&v.versions.join("\n"), false);
    }
    0
}

/// `koyomi restore <group/name> <version>`.
fn restore(args: &[String]) -> i32 {
    match args {
        [id, version] => ask_and_say(&Request::Restore { id: id.replace('\\', "/"), version: version.clone() }),
        _ => usage("restore takes a group/name and a version"),
    }
}

fn rename(args: &[String]) -> i32 {
    let fix = args.iter().any(|a| a == "--fix");
    let rest: Vec<&String> = args.iter().filter(|a| *a != "--fix").collect();
    match rest.as_slice() {
        [id, to] => ask_and_say(&Request::Rename { id: id.replace('\\', "/"), to: to.replace('\\', "/"), fix }),
        _ => usage("rename takes a group/name, the new group/name and --fix"),
    }
}

fn copy(args: &[String]) -> i32 {
    match args {
        [id, to] => ask_and_say(&Request::Copy { id: id.replace('\\', "/"), to: to.replace('\\', "/"), title: None }),
        _ => usage("copy takes a group/name and the new group/name"),
    }
}

/// `koyomi replace <from> <to> [--apply]`.
fn replace(args: &[String]) -> i32 {
    let apply = args.iter().any(|a| a == "--apply");
    let rest: Vec<&String> = args.iter().filter(|a| *a != "--apply").collect();
    match rest.as_slice() {
        [from, to] => ask_and_say(&Request::Replace { from: (*from).clone(), to: (*to).clone(), apply }),
        _ => usage("replace takes the text to find, the text to put, and optionally --apply"),
    }
}

/// `koyomi delete <id>`: what uses it first (the engine refuses then anyway), then `y`.
fn delete(args: &[String]) -> i32 {
    let [id] = args else { return usage("delete takes one group/name") };
    let id = id.replace('\\', "/");
    if let Ok(used) = client::call(&Request::Uses { target: id.clone() }) {
        if !used.is_empty() {
            out(&used, true);
        }
    }
    say(&format!("{id} を消すなら y を入力（前の形は版に残る）:"), false);
    let mut answer = String::new();
    let _ = std::io::stdin().read_line(&mut answer);
    if !answer.trim().eq_ignore_ascii_case("y") {
        return fail(&format!("{id}: 消さなかった"));
    }
    ask_and_say(&Request::Delete { id })
}

/// `koyomi stage <group/name> <file> --from YYYY-MM-DD`, or `koyomi stage <group/name> --cancel`.
fn stage(args: &[String]) -> i32 {
    let (mut from, mut cancel, mut rest) = (None, false, Vec::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--from" => match it.next() {
                Some(d) => from = Some(d.clone()),
                None => return usage("--from takes a day (YYYY-MM-DD)"),
            },
            "--cancel" => cancel = true,
            _ => rest.push(a.replace('\\', "/")),
        }
    }
    match (rest.as_slice(), from, cancel) {
        ([id], None, true) => ask_and_say(&Request::Unstage { id: id.clone() }),
        ([id, file], Some(from), false) => stage_file(id, file, &from),
        _ => usage("stage takes a group/name, a file and --from YYYY-MM-DD, or a group/name and --cancel"),
    }
}

/// The change from the net's file now to `file`, then `y` stages it for `from` (approved).
fn stage_file(id: &str, file: &str, from: &str) -> i32 {
    let text = match std::fs::read(file)
        .map_err(|e| e.to_string())
        .and_then(|b| String::from_utf8(b).map_err(|_| "UTF-8 の文字として読めない".to_owned()))
    {
        Ok(t) => t,
        Err(e) => return fail(&format!("{file} が読めない: {e}")),
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned();
    let now: Def = match client::call(&Request::Show { id: id.to_owned() }).and_then(|s| toml::from_str(&s).map_err(|e| e.to_string())) {
        Ok(d) => d,
        Err(e) => return fail(&format!("koyomi: {e}")),
    };
    if now.text == text {
        return fail(&format!("今の定義と同じ中身: {id}"));
    }
    say(&format!("{id} を {from} からこの形にする（- 今の定義 / + 次の定義）:"), false);
    for l in review::diff_text(&ito_diff::diff(&now.text, &text, 3)) {
        say(&l, false);
    }
    say("確かめたなら y を入力（承認して置く。それ以外は置かない）:", false);
    let mut answer = String::new();
    let _ = std::io::stdin().read_line(&mut answer);
    if !answer.trim().eq_ignore_ascii_case("y") {
        return fail(&format!("{id}: 置かなかった"));
    }
    ask_and_say(&Request::Stage { id: id.to_owned(), text, from: from.to_owned() })
}

/// `koyomi export -o <file> …`: reads the settings folder itself (the engine need not be up)
/// and writes the bundle. An existing file is not written over.
fn export(args: &[String]) -> i32 {
    let mut pick = koyomi_core::bundle::Pick::default();
    let mut to: Option<String> = None;
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "-o" | "--out" => match rest.next() {
                Some(f) => to = Some(f.clone()),
                None => return usage("-o takes a file"),
            },
            "-n" | "--net" => match rest.next() {
                Some(id) => pick.ids.push(id.replace('\\', "/")),
                None => return usage("-n takes a group/name"),
            },
            "--group" => match rest.next() {
                Some(g) => pick.groups.push(g.clone()),
                None => return usage("--group takes a group"),
            },
            "--calendars" => pick.calendars = true,
            "--settings" => pick.settings = true,
            "--all" => pick.all = true,
            other => return usage(&format!("export does not take {other}")),
        }
    }
    let Some(to) = to else { return usage("export takes -o <file>") };
    let path = std::path::Path::new(&to);
    if path.exists() {
        return fail(&format!("もうある: {}", path.display()));
    }
    let paths = match engine::Paths::find() {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    let src = koyomi_core::bundle::Source::read(&paths.app);
    let bundle = match crate::notify::export(&src, &pick) {
        Ok(b) => b,
        Err(e) => return fail(&e),
    };
    if bundle.is_empty() {
        return fail("書き出すものがない");
    }
    if let Err(e) = ito_common::write_atomic(path, bundle.to_toml()) {
        return fail(&e);
    }
    out(
        &format!(
            "書いた: {}（ジョブネット {}、グループ {}、カレンダー {}{}）",
            path.display(),
            bundle.jobnets.len(),
            bundle.groups.len(),
            bundle.calendars.len(),
            if bundle.settings.is_some() { "、設定（webhook の宛先は *** にした）" } else { "" }
        ),
        false,
    );
    0
}

/// `koyomi import <file> [--dry-run] [--on-conflict …]`: the engine plans and does it.
/// Exit 1 when it was not done: a choice is needed, something is wrong, or `--dry-run`
/// found something to do.
/// A file to import as text: UTF-8 or UTF-16, or a task's XML in the console code page
/// (what `schtasks /query /xml > file` writes from cmd).
pub(crate) fn port_text(bytes: &[u8]) -> Option<String> {
    koyomi_core::task::decode(bytes).or_else(|| Some(ito_proc::Encoding::Console.decode(bytes)).filter(|t| koyomi_core::task::is_xml(t)))
}

fn import(args: &[String]) -> i32 {
    let mut file: Option<String> = None;
    let (mut dry_run, mut on_conflict) = (false, None);
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--dry-run" => dry_run = true,
            "--on-conflict" => match rest.next() {
                Some(c) => {
                    if let Err(e) = koyomi_core::bundle::Conflict::parse(c) {
                        return usage(&e);
                    }
                    on_conflict = Some(c.clone());
                }
                None => return usage("--on-conflict takes overwrite, rename or skip"),
            },
            other if file.is_none() && !other.starts_with('-') => file = Some(other.to_owned()),
            other => return usage(&format!("import does not take {other}")),
        }
    }
    let Some(file) = file else { return usage("import takes one file") };
    let text = match std::fs::read(&file) {
        Ok(b) => match port_text(&b) {
            Some(t) => t,
            None => return fail(&format!("{file} を読めない: UTF-8 でも UTF-16 でもない")),
        },
        Err(e) => return fail(&format!("{file} を読めない: {e}")),
    };
    let answer = match client::call(&Request::Import { text, dry_run, on_conflict }) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    if toml() {
        emit(&answer);
        return 0;
    }
    let plan = match proto::Plan::parse(&answer) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    out(&plan.lines.join("\n"), false);
    if plan.applied {
        out("取り込んだ（ジョブネットは未確認。窓か koyomi approve で確かめてから流す）", false);
        return 0;
    }
    if plan.problems > 0 {
        return fail(&format!("問題が {} 件あるので取り込まない", plan.problems));
    }
    if plan.undecided {
        return fail(&format!(
            "今あるものと違うものが {} 件ある: --on-conflict overwrite（上書き。前の中身は versions/ に残る）、rename（名前を変えて足す）か skip（飛ばす）で決める",
            plan.conflicts
        ));
    }
    if !plan.writes {
        out("書くものはない（全部同じか、飛ばす）", false);
        return 0;
    }
    out("まだ取り込んでいない（--dry-run）", false);
    1
}

/// Sends `request` and prints the answer: the commands with nothing to read back.
fn ask_and_say(request: &Request) -> i32 {
    match client::call(request) {
        Ok(text) => {
            out(&text, false);
            0
        }
        Err(e) => fail(&e),
    }
}

fn id_and_job(args: &[String], name: &str, make: fn(String, Option<String>) -> Request) -> i32 {
    match args {
        [id] => ask_and_say(&make(id.clone(), None)),
        [id, job] => ask_and_say(&make(id.clone(), Some(job.clone()))),
        _ => usage(&format!("{name} takes a group/name and maybe a job")),
    }
}

fn plain(args: &[String], name: &str, request: Request) -> i32 {
    if args.is_empty() {
        ask_and_say(&request)
    } else {
        usage(&format!("{name} takes nothing"))
    }
}

fn approve_step(args: &[String]) -> i32 {
    let yes = !args.iter().any(|a| a == "--no");
    let rest: Vec<&String> = args.iter().filter(|a| *a != "--no").collect();
    match rest.as_slice() {
        [run, job] => ask_and_say(&Request::Answer { run: (*run).clone(), job: (*job).clone(), yes }),
        _ => usage("approve-step takes a run and a job"),
    }
}

fn skip(args: &[String]) -> i32 {
    match args {
        [run, job] => ask_and_say(&Request::Skip { run: run.clone(), job: job.clone() }),
        _ => usage("skip takes a run and a job"),
    }
}

fn shift(args: &[String]) -> i32 {
    let at = |t: &str| {
        if t.contains(' ') {
            t.to_owned()
        } else {
            format!("{} {t}", chrono::Local::now().format("%Y-%m-%d"))
        }
    };
    match args {
        [id, from, to] if to == "--cancel" => ask_and_say(&Request::Shift { id: id.clone(), from: at(from), to: None }),
        [id, from, to] => ask_and_say(&Request::Shift { id: id.clone(), from: at(from), to: Some(at(to)) }),
        _ => usage("shift takes a group/name, its time and the new time (or --cancel)"),
    }
}

fn skip_next(args: &[String]) -> i32 {
    match args {
        [id] => ask_and_say(&Request::SkipNext { id: id.replace(char::from(92), "/"), cancel: false }),
        [id, c] if c == "--cancel" => ask_and_say(&Request::SkipNext { id: id.replace(char::from(92), "/"), cancel: true }),
        _ => usage("skip-next takes a group/name (and --cancel to take the skips back)"),
    }
}

fn uses(args: &[String]) -> i32 {
    let [target] = args else { return usage("uses takes one calendar, group/name or program") };
    match client::call(&Request::Uses { target: target.replace(char::from(92), "/") }) {
        Ok(text) if text.is_empty() => {
            out("(not used)", false);
            0
        }
        Ok(text) => {
            out(&text, false);
            0
        }
        Err(e) => fail(&e),
    }
}

fn ops(args: &[String]) -> i32 {
    let (mut from, mut to) = (None, None);
    let day = |d: Option<&String>| d.and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--from" => match day(it.next()) {
                Some(d) => from = Some(d),
                None => return usage("--from takes YYYY-MM-DD"),
            },
            "--to" => match day(it.next()) {
                Some(d) => to = Some(d),
                None => return usage("--to takes YYYY-MM-DD"),
            },
            _ => return usage("ops takes --from and --to"),
        }
    }
    let to = to.unwrap_or_else(|| chrono::Local::now().date_naive());
    let from = from.unwrap_or(to);
    match client::call(&Request::Ops { from: from.to_string(), to: to.to_string() }) {
        Ok(text) if text.is_empty() => {
            out("(none)", false);
            0
        }
        Ok(text) => {
            out(&text, false);
            0
        }
        Err(e) => fail(&e),
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
            out(&text, false);
            0
        }
        Err(e) => fail(&e),
    }
}

fn engine_cmd(args: &[String]) -> i32 {
    match args {
        [] => engine::run(true),
        [a] if a == "--no-tray" => engine::run(false),
        [a] if a == "status" => match ask(&proto::door(), &Request::Status) {
            Ok(text) => {
                out(&format!("{text}\n{}", startup_line()), false);
                0
            }
            Err(_) => {
                out("koyomi: エンジンは動いていない", true);
                1
            }
        },
        [a] if a == "stop" => match ask(&proto::door(), &Request::Stop) {
            Ok(_) => {
                out("koyomi: エンジンを止めた", false);
                0
            }
            Err(_) => {
                out("koyomi: エンジンは動いていない", true);
                1
            }
        },
        _ => usage("engine takes status, stop or --no-tray"),
    }
}

/// Whether the engine starts at sign-in, in a line.
fn startup_line() -> String {
    match ito_common::startup::enabled("koyomi") {
        true if crate::service::state().is_some() => "サインイン時の起動: あり（トレイだけ。エンジンはサービス）".to_owned(),
        true => "サインイン時の起動: あり".to_owned(),
        false => "サインイン時の起動: なし".to_owned(),
    }
}

/// `koyomi secret set|delete|check <name>`: the values a job's `env` reads as `secret:<name>`.
/// `set` reads the value from stdin (a line), so it stays out of the shell history.
fn secret_cmd(args: &[String]) -> i32 {
    let [op, name] = args else { return usage("secret takes set, delete or check, and a name") };
    let res = match op.as_str() {
        "set" => {
            let mut value = String::new();
            if let Err(e) = std::io::stdin().read_line(&mut value) {
                out(&format!("koyomi: {e}"), true);
                return 1;
            }
            let value = value.trim_end_matches(['\r', '\n']);
            if value.is_empty() {
                return usage("secret set reads the value from stdin; it was empty");
            }
            ito_secret::set("koyomi", name, value).map(|()| format!("koyomi: 秘密 {name} を保存した"))
        }
        "delete" => ito_secret::delete("koyomi", name).map(|()| format!("koyomi: 秘密 {name} を消した")),
        "check" => ito_secret::get("koyomi", name).map(|v| match v {
            Some(_) => format!("koyomi: 秘密 {name} はある"),
            None => format!("koyomi: 秘密 {name} はない"),
        }),
        _ => return usage("secret takes set, delete or check, and a name"),
    };
    match res {
        Ok(text) => {
            out(&text, false);
            0
        }
        Err(e) => {
            out(&format!("koyomi: {e}"), true);
            1
        }
    }
}

fn startup_cmd(args: &[String]) -> i32 {
    let on = match args {
        [] => {
            out(&startup_line(), false);
            return 0;
        }
        [a] if a == "on" => true,
        [a] if a == "off" => false,
        _ => return usage("startup takes on or off"),
    };
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            out(&format!("koyomi: {e}"), true);
            return 1;
        }
    };
    if on {
        if let Err(e) = refuse_temp(&exe) {
            out(&format!("koyomi: {e}"), true);
            return 1;
        }
    }
    match ito_common::startup::set("koyomi", &exe, &startup_args(), on) {
        Ok(()) => {
            out(
                if on && crate::service::state().is_some() {
                    "koyomi: サインイン時にトレイを起動する（エンジンはサービス）"
                } else if on {
                    "koyomi: サインイン時にエンジンを起動する"
                } else {
                    "koyomi: サインイン時には起動しない"
                },
                false,
            );
            0
        }
        Err(e) => {
            out(&format!("koyomi: {e}"), true);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    /// `koyomi status` shows the engine's HTTP line when it listens.
    #[test]
    fn status_picks_the_http_line() {
        let text = "koyomi engine pid 1: 定義 0 件\nHTTP: 127.0.0.1:8790 で待ち受けている\nx";
        assert_eq!(super::http_line(text), Some("HTTP: 127.0.0.1:8790 で待ち受けている"));
        assert_eq!(super::http_line("koyomi engine pid 1"), None);
    }

    use super::day_range;

    /// an exe in Temp is refused with where to put it instead, unless KOYOMI_ALLOW_TEMP lets a test through.
    #[test]
    fn an_exe_in_temp_is_refused_unless_allowed() {
        let exe = std::path::Path::new(r"C:\Users\a\AppData\Local\Temp\x\koyomi.com");
        let e = super::temp_words(exe, true, false).expect_err("Temp is refused");
        assert!(e.contains("Temp") && e.contains("Program Files") && e.contains(super::ALLOW_TEMP));
        assert_eq!(super::temp_words(exe, true, true), Ok(()));
        assert_eq!(super::temp_words(exe, false, false), Ok(()));
    }

    /// `check` tells what a group of PCs would stop on before it runs.
    #[test]
    fn a_missing_group_or_a_pc_without_a_key_is_told() {
        let hk = "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU";
        let config = koyomi_core::config::Config::parse(&format!(
            "[groups]\nweb = ['ssh://ops@web1', {{ on = 'ssh://ops@web2', host_key = '{hk}' }}]\napp = ['winrm://ops@app01']\n"
        ))
        .unwrap();
        let ssh = |id: &str, on: &str, more: &str| format!("[[job]]\nid = '{id}'\nkind = 'ssh'\non = '{on}'\ncommand = 'x'\n{more}");
        let net = koyomi_core::jobnet::parse(&format!(
            "format = 1\n{}{}{}{}[[job]]\nid = 'e'\nkind = 'winrm'\non = 'group:web'\ncommand = 'x'\npassword = 'secret:p'\n",
            ssh("a", "group:web", ""),
            ssh("b", "group:db", ""),
            ssh("c", "group:web", &format!("host_key = '{hk}'\n")),
            ssh("d", "ssh://ops@web9", &format!("host_key = '{hk}'\n")),
        ))
        .unwrap();
        let w = super::group_warnings(&config, &net);
        assert_eq!(w.len(), 3, "{w:?}");
        assert_eq!(w[0], "a: 組 web の ssh://ops@web1 に host_key がなく、ジョブにもない");
        assert!(w[1].starts_with("b: 組 db が koyomi.toml の [groups] にない"), "{}", w[1]);
        assert!(w[2].starts_with("e: 組 web の PC は ssh:// で書いてある"), "{}", w[2]);
    }

    #[test]
    fn a_date_range_lists_both_ends() {
        let d = day_range("2026-02-27..2026-03-02").unwrap();
        let t: Vec<String> = d.iter().map(|x| x.to_string()).collect();
        assert_eq!(t, ["2026-02-27", "2026-02-28", "2026-03-01", "2026-03-02"]);
        assert_eq!(day_range("2026-05-01..2026-05-01").unwrap().len(), 1);
    }

    #[test]
    fn a_bad_date_range_is_refused() {
        assert!(day_range("2026-05-02..2026-05-01").is_err());
        assert!(day_range("x..2026-05-01").is_err());
        assert!(day_range("2020-01-01..2026-01-01").is_err());
    }
}
