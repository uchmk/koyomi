//! The commands that print and exit: `check`, `list`, `approve`, `engine`.
//!
//! Exit codes: 0 done, 1 refused or not so (a problem in a file, not approved, an engine already up),
//! 2 used wrongly.

use ito_ipc::door::ask;
use koyomi_core::scan::{Def, Standing};

use crate::proto::{self, Request, Snapshot};
use crate::{client, engine, review, say};

pub const USAGE: &str = "\
usage: koyomi [--version | --help | env]
       koyomi check <file>           read a definition and say what is wrong with it (exit 1 if anything)
       koyomi list [--toml]          the definitions and where each stands
       koyomi approve <group/name>   show what it would run, and approve it on `y`
       koyomi engine [status|stop]   run the engine in the foreground / ask it / stop it
";

/// Words for the person at the console: common.toml's language.
fn table() -> ito_i18n::Table {
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
        "engine" => engine_cmd(rest),
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
        for line in review::tree(&table(), &snap.defs) {
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

fn engine_cmd(args: &[String]) -> i32 {
    match args {
        [] => engine::run(),
        [a] if a == "status" => match ask(&proto::door(), &Request::Status) {
            Ok(text) => {
                say(&text, false);
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
