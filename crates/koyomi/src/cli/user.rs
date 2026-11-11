//! `koyomi user list|add|role|reset|remove`: the `[[http.user]]` of koyomi.toml, through the engine's door.
//!
//! The engine does the reading and the writing (an admin over HTTPS asks for the same things); this
//! only asks, and says what came back. A change is made against the hash of koyomi.toml that the
//! `Users` reply carries, so a file changed in between is refused rather than overwritten.

use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};

use koyomi_core::msg;
use koyomi_core::role::Role;
use koyomi_core::users::{Listed, Listing, Token};

use super::{emit, fail, out, toml as want_toml, usage};
use crate::client;
use crate::proto::Request;

pub fn cmd(args: &[String]) -> i32 {
    match args.split_first() {
        None => list(),
        Some((op, rest)) => match op.as_str() {
            "list" if rest.is_empty() => list(),
            "list" => usage("user list takes nothing"),
            "add" => add(rest),
            "role" => role(rest),
            "reset" => reset(rest),
            "remove" => remove(rest),
            _ => usage("user takes list, add, role <name> <role>, reset <name> or remove <name>"),
        },
    }
}

/// What the engine says the users are, with the hash a change is made against.
fn read() -> Result<(String, Listing), String> {
    let text = client::call(&Request::Users)?;
    let listing = Listing::parse(&text)?;
    Ok((text, listing))
}

fn nets_word(nets: &Option<Vec<String>>) -> String {
    match nets {
        Some(n) if !n.is_empty() => n.join(", "),
        _ => msg::note(msg::N_USER_NETS_ALL, &[]),
    }
}

fn roles_word(roles: &Option<BTreeMap<String, String>>) -> String {
    match roles {
        Some(r) if !r.is_empty() => r.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(", "),
        _ => msg::note(msg::N_CFG_NONE, &[]),
    }
}

fn token_word(l: &Listed) -> String {
    let secret = l.user.secret.as_deref().unwrap_or_default();
    match l.token {
        Token::Ok => msg::note(msg::N_USER_TOKEN_OK, &[]),
        Token::Hash => msg::note(msg::N_USER_TOKEN_HASH, &[]),
        Token::Missing => msg::note(msg::N_USER_TOKEN_MISSING, &[secret]),
        Token::Unreadable => msg::note(msg::N_USER_TOKEN_UNREADABLE, &[secret]),
        Token::None => msg::note(msg::N_USER_TOKEN_NONE, &[]),
    }
}

/// How many users have a token the HTTPS listener cannot use.
fn unusable(listing: &Listing) -> usize {
    listing.user.iter().filter(|l| matches!(l.token, Token::Missing | Token::Unreadable)).count()
}

fn list() -> i32 {
    let (text, listing) = match read() {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };
    let bad = unusable(&listing);
    if want_toml() {
        emit(&text);
        return i32::from(bad > 0);
    }
    if listing.user.is_empty() {
        out(&msg::note(msg::N_USER_NONE, &[]), false);
        return 0;
    }
    // One block per user and no column padding: a CJK character takes two terminal columns.
    let labels =
        [msg::N_USER_LABEL_ROLE, msg::N_USER_LABEL_NETS, msg::N_USER_LABEL_ROLES, msg::N_USER_LABEL_TOKEN].map(|l| msg::note(l, &[]));
    for (i, l) in listing.user.iter().enumerate() {
        if i > 0 {
            out("", false);
        }
        out(&l.user.name, false);
        let values = [l.user.role.clone(), nets_word(&l.user.nets), roles_word(&l.user.roles), token_word(l)];
        for (label, value) in labels.iter().zip(values) {
            out(format!("  {label}: {value}").trim_end(), false);
        }
    }
    if bad > 0 {
        out(&msg::note(msg::N_USER_TOKEN_BAD, &[&bad.to_string()]), true);
        return 1;
    }
    0
}

/// The user a person has described, by flags or by answers.
#[derive(Debug, PartialEq)]
struct NewUser {
    name: String,
    role: String,
    nets: Option<Vec<String>>,
    roles: Option<BTreeMap<String, String>>,
}

/// What `UserAdd` and `UserReset` answer.
#[derive(serde::Deserialize)]
struct TokenReply {
    token: String,
}

/// Prints the new token as the CLI's one printout of it: on its own line, then the note.
fn show_token(reply: &str, name: &str) -> Result<(), String> {
    // The reply holds the token, so neither it nor the parser's words (which quote it) are shown.
    let r: TokenReply = toml::from_str(reply).map_err(|_| msg::line(msg::USER_REPLY_BAD, &[name]))?;
    out(&r.token, false);
    out(&msg::note(msg::N_USER_TOKEN_ONCE, &[name]), false);
    Ok(())
}

/// What is said when a change of user `name` fails with `e`. The engine's own refusals and the client's
/// failures are coded; a plain text is a broken connection, after which the request may have been done.
fn call_failed(name: &str, e: &str) -> String {
    if msg::is_coded(e) {
        e.to_owned()
    } else {
        msg::line(msg::USER_UNSURE, &[name, e])
    }
}

/// Sends a `UserAdd` against the hash of the file as it is now and says what came of it.
fn send_add(u: &NewUser) -> i32 {
    let (_, listing) = match read() {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };
    let request =
        Request::UserAdd { name: u.name.clone(), role: u.role.clone(), nets: u.nets.clone(), roles: u.roles.clone(), base: listing.base };
    let reply = match client::call(&request) {
        Ok(r) => r,
        Err(e) => return fail(&call_failed(&u.name, &e)),
    };
    if want_toml() {
        emit(&reply);
        return 0;
    }
    out(&msg::note(msg::N_USER_ADD_DONE, &[&u.name, &u.role, &nets_word(&u.nets), &roles_word(&u.roles)]), false);
    match show_token(&reply, &u.name) {
        Ok(()) => 0,
        Err(e) => fail(&e),
    }
}

/// `user add <name> --role <r> [--nets <pattern>]... [--roles <pattern>=<role>]...`, or with no arguments
/// on a terminal, the same by questions.
fn add(args: &[String]) -> i32 {
    const WHAT: &str = "user add takes a name, --role <view|run|edit|admin>, and --nets or --roles";
    if args.is_empty() {
        if !std::io::stdin().is_terminal() {
            return usage(WHAT);
        }
        let taken = match read() {
            Ok((_, l)) => l.user.into_iter().map(|l| l.user.name).collect::<Vec<_>>(),
            Err(e) => return fail(&e),
        };
        return interactive(&mut std::io::stdin().lock(), &mut std::io::stderr(), &taken, send_add);
    }
    let [name, flags @ ..] = args else { return usage(WHAT) };
    let (mut role, mut nets, mut roles) = (None, Vec::new(), BTreeMap::new());
    let mut it = flags.iter();
    while let Some(f) = it.next() {
        match f.as_str() {
            "--role" => match it.next() {
                Some(r) if Role::parse(r.trim()).is_ok() => role = Some(r.trim().to_owned()),
                _ => return usage("--role takes view, run, edit or admin"),
            },
            "--nets" => match it.next() {
                Some(p) if !p.trim().is_empty() => nets.push(p.trim().to_owned()),
                _ => return usage("--nets takes a jobnet pattern"),
            },
            "--roles" => match it.next().and_then(|p| p.split_once('=')) {
                Some((pat, r)) if !pat.trim().is_empty() && Role::parse(r.trim()).is_ok() => {
                    roles.insert(pat.trim().to_owned(), r.trim().to_owned());
                }
                _ => return usage("--roles takes <pattern>=<view|run|edit|admin>"),
            },
            _ => return usage(WHAT),
        }
    }
    let Some(role) = role else { return usage(WHAT) };
    let user = NewUser {
        name: name.trim().to_owned(),
        role,
        nets: Some(nets).filter(|n| !n.is_empty()),
        roles: Some(roles).filter(|r| !r.is_empty()),
    };
    send_add(&user)
}

/// Why the questions stopped before there was an answer.
enum Stop {
    Ended,
    Failed(String),
}

/// One line from `input`, trimmed. End of input is [`Stop::Ended`], never an empty answer.
fn line_of(input: &mut impl BufRead) -> Result<String, Stop> {
    let mut line = String::new();
    match input.read_line(&mut line) {
        Ok(0) => Err(Stop::Ended),
        Ok(_) => Ok(line.trim().to_owned()),
        Err(e) => Err(Stop::Failed(e.to_string())),
    }
}

fn ask(input: &mut impl BufRead, output: &mut impl Write, prompt: &str) -> Result<String, Stop> {
    let _ = write!(output, "{prompt} ");
    let _ = output.flush();
    line_of(input)
}

fn yes(answer: &str) -> bool {
    matches!(answer.to_lowercase().as_str(), "y" | "yes")
}

/// The patterns in an answer, separated by spaces or commas.
fn patterns(answer: &str) -> Vec<String> {
    answer.split(|c: char| c.is_whitespace() || c == ',').filter(|p| !p.is_empty()).map(str::to_owned).collect()
}

/// The TOML block a new user will be written as, with a word where the hash will be.
fn preview(u: &NewUser) -> String {
    let q = |s: &str| toml::Value::String(s.to_owned()).to_string();
    let mut t = format!("[[http.user]]\nname = {}\nrole = {}\n", q(&u.name), q(&u.role));
    if let Some(n) = &u.nets {
        t.push_str(&format!("nets = [{}]\n", n.iter().map(|p| q(p)).collect::<Vec<_>>().join(", ")));
    }
    if let Some(r) = &u.roles {
        t.push_str(&format!("roles = {{ {} }}\n", r.iter().map(|(k, v)| format!("{} = {}", q(k), q(v))).collect::<Vec<_>>().join(", ")));
    }
    t.push_str(&format!("token = \"sha256:{}\"\n", msg::note(msg::N_USER_PREVIEW_TOKEN, &[])));
    t
}

enum Asked {
    Go(NewUser),
    /// Said no at the end.
    Declined,
}

fn questions(input: &mut impl BufRead, output: &mut impl Write, taken: &[String]) -> Result<Asked, Stop> {
    let name = loop {
        let n = ask(input, output, &msg::note(msg::N_USER_ASK_NAME, &[]))?;
        if n.is_empty() || n.contains(char::is_whitespace) || n == "default" {
            let _ = writeln!(output, "{}", msg::note(msg::N_USER_ASK_NAME_BAD, &[]));
        } else if taken.iter().any(|t| t.trim() == n) {
            let _ = writeln!(output, "{}", msg::note(msg::N_USER_ASK_NAME_EXISTS, &[&n]));
        } else {
            break n;
        }
    };
    let _ = writeln!(output, "{}", msg::note(msg::N_USER_ASK_ROLE, &[]));
    for l in [msg::N_USER_ROLE_VIEW, msg::N_USER_ROLE_RUN, msg::N_USER_ROLE_EDIT, msg::N_USER_ROLE_ADMIN] {
        let _ = writeln!(output, "  {}", msg::note(l, &[]));
    }
    let role = loop {
        let n = ask(input, output, ">")?;
        match n.as_str() {
            "1" => break "view",
            "2" => break "run",
            "3" => break "edit",
            "4" => break "admin",
            _ => {
                let _ = writeln!(output, "{}", msg::note(msg::N_USER_ASK_ROLE_BAD, &[]));
            }
        }
    };
    let mut nets = None;
    if role != "admin" && yes(&ask(input, output, &msg::note(msg::N_USER_ASK_NETS_YN, &[]))?) {
        nets = loop {
            let p = patterns(&ask(input, output, &msg::note(msg::N_USER_ASK_NETS, &[]))?);
            if !p.is_empty() {
                break Some(p);
            }
            let _ = writeln!(output, "{}", msg::note(msg::N_USER_ASK_NETS_EMPTY, &[]));
        };
    }
    let user = NewUser { name, role: role.to_owned(), nets, roles: None };
    let _ = writeln!(output, "{}\n{}", msg::note(msg::N_USER_PREVIEW_HEAD, &[]), preview(&user));
    if yes(&ask(input, output, &msg::note(msg::N_USER_ASK_CONFIRM, &[]))?) {
        Ok(Asked::Go(user))
    } else {
        Ok(Asked::Declined)
    }
}

/// Asks what `user add` needs and, only on a yes at the end, hands the user to `send` (which returns the
/// exit code). End of input or a no changes nothing and exits 0; a read error exits 1.
fn interactive(input: &mut impl BufRead, output: &mut impl Write, taken: &[String], send: impl FnOnce(&NewUser) -> i32) -> i32 {
    match questions(input, output, taken) {
        Ok(Asked::Go(u)) => send(&u),
        Ok(Asked::Declined) => {
            let _ = writeln!(output, "{}", msg::note(msg::N_USER_ADD_STOPPED, &[]));
            0
        }
        Err(Stop::Ended) => {
            let _ = writeln!(output, "{}", msg::note(msg::N_USER_INPUT_ENDED, &[]));
            0
        }
        Err(Stop::Failed(e)) => fail(&e),
    }
}

/// Whether a reset goes on: `None` to go on, `Some(exit code)` to stop. `--yes` goes on; without it a
/// person is asked on a terminal (only `y`/`yes` goes on; no, an empty line or the end of input stops
/// with nothing changed), and a pipe is refused.
fn reset_gate(input: &mut impl BufRead, output: &mut impl Write, name: &str, yes_flag: bool, terminal: bool) -> Option<i32> {
    if yes_flag {
        return None;
    }
    if !terminal {
        return Some(fail(&msg::note(msg::N_USER_RESET_NEED_YES, &[name])));
    }
    match ask(input, output, &msg::note(msg::N_USER_RESET_ASK, &[name])) {
        Ok(a) if yes(&a) => None,
        Ok(_) => {
            let _ = writeln!(output, "{}", msg::note(msg::N_USER_ADD_STOPPED, &[]));
            Some(0)
        }
        Err(Stop::Ended) => {
            let _ = writeln!(output, "{}", msg::note(msg::N_USER_INPUT_ENDED, &[]));
            Some(0)
        }
        Err(Stop::Failed(e)) => Some(fail(&e)),
    }
}

/// `user reset <name> [--yes]`.
fn reset(args: &[String]) -> i32 {
    const WHAT: &str = "user reset takes a name and --yes";
    let (mut name, mut yes_flag) = (None, false);
    for a in args {
        match a.as_str() {
            "--yes" => yes_flag = true,
            n if name.is_none() && !n.starts_with("--") => name = Some(n.trim().to_owned()),
            _ => return usage(WHAT),
        }
    }
    let Some(name) = name else { return usage(WHAT) };
    let (_, listing) = match read() {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };
    let Some(current) = listing.user.iter().find(|l| l.user.name == name) else {
        return fail(&msg::line(msg::USER_NOT_IN_FILE, &[&name]));
    };
    let secret = current.user.secret.clone();
    if let Some(code) = reset_gate(&mut std::io::stdin().lock(), &mut std::io::stderr(), &name, yes_flag, std::io::stdin().is_terminal()) {
        return code;
    }
    let reply = match client::call(&Request::UserReset { name: name.clone(), base: listing.base.clone() }) {
        Ok(r) => r,
        Err(e) => return fail(&call_failed(&name, &e)),
    };
    if want_toml() {
        emit(&reply);
        return 0;
    }
    out(&msg::note(msg::N_USER_RESET_DONE, &[&name]), false);
    if let Err(e) = show_token(&reply, &name) {
        return fail(&e);
    }
    if let Some(secret) = secret {
        let id = if cfg!(windows) { msg::N_USER_SECRET_STAYS_WIN } else { msg::N_USER_SECRET_STAYS_LINUX };
        out(&msg::note(id, &[&secret]), false);
    }
    0
}

/// `user role <name> <role> [--nets <pattern>]... [--all-nets] [--roles <pattern>=<role>]... [--no-roles]`.
fn role(args: &[String]) -> i32 {
    const WHAT: &str = "user role takes a name, view, run, edit or admin, and --nets, --all-nets, --roles or --no-roles";
    let [name, new_role, flags @ ..] = args else { return usage(WHAT) };
    if Role::parse(new_role).is_err() {
        return usage(WHAT);
    }
    let (mut nets, mut all_nets) = (Vec::new(), false);
    let (mut roles, mut no_roles) = (BTreeMap::new(), false);
    let mut it = flags.iter();
    while let Some(f) = it.next() {
        match f.as_str() {
            "--nets" => match it.next() {
                Some(p) if !p.trim().is_empty() => nets.push(p.trim().to_owned()),
                _ => return usage("--nets takes a jobnet pattern"),
            },
            "--all-nets" => all_nets = true,
            "--roles" => match it.next().and_then(|p| p.split_once('=')) {
                Some((pat, r)) if !pat.trim().is_empty() && Role::parse(r.trim()).is_ok() => {
                    roles.insert(pat.trim().to_owned(), r.trim().to_owned());
                }
                _ => return usage("--roles takes <pattern>=<view|run|edit|admin>"),
            },
            "--no-roles" => no_roles = true,
            _ => return usage(WHAT),
        }
    }
    if (all_nets && !nets.is_empty()) || (no_roles && !roles.is_empty()) {
        return usage("--all-nets and --nets (or --no-roles and --roles) cannot go together");
    }
    let (_, listing) = match read() {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };
    let name = name.trim();
    let Some(current) = listing.user.iter().find(|l| l.user.name == name) else {
        return fail(&msg::line(msg::USER_NOT_IN_FILE, &[name]));
    };
    let admin = new_role == "admin";
    let new_nets = if admin || all_nets {
        None
    } else if nets.is_empty() {
        current.user.nets.clone()
    } else {
        Some(nets.clone())
    };
    let new_roles = if admin || no_roles {
        None
    } else if roles.is_empty() {
        current.user.roles.clone()
    } else {
        Some(roles.clone())
    };
    let cleared = admin && (current.user.nets.is_some() || current.user.roles.is_some());
    let request = Request::UserSet {
        name: name.to_owned(),
        role: new_role.clone(),
        nets: new_nets.clone(),
        roles: new_roles.clone(),
        base: listing.base.clone(),
    };
    if let Err(e) = client::call(&request) {
        return fail(&e);
    }
    if cleared {
        out(&msg::note(msg::N_USER_ADMIN_CLEARED, &[]), false);
    }
    out(&msg::note(msg::N_USER_ROLE_DONE, &[name, new_role, &nets_word(&new_nets), &roles_word(&new_roles)]), false);
    0
}

/// `user remove <name>`.
fn remove(args: &[String]) -> i32 {
    let [name] = args else { return usage("user remove takes a name") };
    let (_, listing) = match read() {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };
    let name = name.trim();
    let Some(current) = listing.user.iter().find(|l| l.user.name == name) else {
        return fail(&msg::line(msg::USER_NOT_IN_FILE, &[name]));
    };
    let secret = current.user.secret.clone();
    if let Err(e) = client::call(&Request::UserRemove { name: name.to_owned(), base: listing.base.clone() }) {
        return fail(&e);
    }
    out(&msg::note(msg::N_USER_REMOVE_DONE, &[name]), false);
    if let Some(secret) = secret {
        let id = if cfg!(windows) { msg::N_USER_SECRET_STAYS_WIN } else { msg::N_USER_SECRET_STAYS_LINUX };
        out(&msg::note(id, &[&secret]), false);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(tokens: &[Token]) -> Listing {
        Listing {
            base: "x".to_owned(),
            user: tokens
                .iter()
                .enumerate()
                .map(|(i, t)| Listed {
                    user: koyomi_core::users::User {
                        name: format!("u{i}"),
                        role: "view".to_owned(),
                        nets: None,
                        roles: None,
                        tailscale: None,
                        secret: Some(format!("s{i}")),
                    },
                    token: *t,
                })
                .collect(),
        }
    }

    #[test]
    fn a_token_the_listener_cannot_use_is_counted() {
        assert_eq!(unusable(&listing(&[Token::Ok, Token::None])), 0);
        assert_eq!(unusable(&listing(&[Token::Ok, Token::Missing, Token::Unreadable])), 2);
    }

    #[test]
    fn the_words_for_nets_and_roles() {
        assert_eq!(nets_word(&Some(vec!["a/*".to_owned(), "b/x".to_owned()])), "a/*, b/x");
        assert_eq!(nets_word(&Some(Vec::new())), nets_word(&None));
        let r: BTreeMap<String, String> = [("a/*".to_owned(), "run".to_owned())].into();
        assert_eq!(roles_word(&Some(r)), "a/*=run");
        assert_eq!(roles_word(&None), msg::note(msg::N_CFG_NONE, &[]));
        assert_eq!(roles_word(&Some(BTreeMap::new())), roles_word(&None));
    }

    #[test]
    fn the_words_for_a_missing_token_name_the_secret() {
        let l = listing(&[Token::Missing]);
        assert!(token_word(&l.user[0]).contains("s0"));
    }

    fn scripted(answers: &str, taken: &[&str]) -> (i32, Vec<NewUser>, String) {
        let taken: Vec<String> = taken.iter().map(|t| (*t).to_owned()).collect();
        let mut sent = Vec::new();
        let mut shown = Vec::new();
        let code = interactive(&mut answers.as_bytes(), &mut shown, &taken, |u| {
            sent.push(NewUser { name: u.name.clone(), role: u.role.clone(), nets: u.nets.clone(), roles: u.roles.clone() });
            0
        });
        (code, sent, String::from_utf8(shown).unwrap())
    }

    #[test]
    fn a_run_user_narrowed_to_a_group_is_sent_after_a_yes() {
        let (code, sent, shown) = scripted("ann\n2\ny\n経理/*\ny\n", &["boss"]);
        assert_eq!(code, 0);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].name, "ann");
        assert_eq!(sent[0].role, "run");
        assert_eq!(sent[0].nets, Some(vec!["経理/*".to_owned()]));
        assert!(shown.contains("[[http.user]]") && shown.contains("nets = [\"経理/*\"]"), "{shown}");
    }

    #[test]
    fn a_taken_name_is_asked_again() {
        let (code, sent, shown) = scripted("boss\ndefault\nann\n1\nn\ny\n", &["boss"]);
        assert_eq!(code, 0);
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].name.as_str(), sent[0].role.as_str(), sent[0].nets.is_none()), ("ann", "view", true));
        assert!(shown.contains(&msg::note(msg::N_USER_ASK_NAME_EXISTS, &["boss"])), "{shown}");
    }

    #[test]
    fn anything_but_a_yes_at_the_end_sends_nothing() {
        let (code, sent, shown) = scripted("ann\n4\nn\n", &[]);
        assert_eq!(code, 0);
        assert!(sent.is_empty());
        assert!(shown.contains(&msg::note(msg::N_USER_ADD_STOPPED, &[])), "{shown}");
    }

    #[test]
    fn the_end_of_input_stops_at_once_and_sends_nothing() {
        for answers in ["", "ann\n", "ann\n9\n", "ann\n2\n", "ann\n2\ny\n", "ann\n2\nn\n"] {
            let (code, sent, shown) = scripted(answers, &[]);
            assert_eq!(code, 0, "{answers:?}");
            assert!(sent.is_empty(), "{answers:?}");
            assert!(shown.contains(&msg::note(msg::N_USER_INPUT_ENDED, &[])), "{answers:?}: {shown}");
        }
    }

    #[test]
    fn patterns_are_split_at_spaces_and_commas() {
        assert_eq!(patterns("経理/*, */日次  a/b"), vec!["経理/*", "*/日次", "a/b"]);
    }

    #[test]
    fn the_reset_prompt_goes_on_only_on_a_yes() {
        let gate = |input: &str, yes_flag: bool, terminal: bool| {
            let mut shown = Vec::new();
            let code = reset_gate(&mut input.as_bytes(), &mut shown, "ann", yes_flag, terminal);
            (code, String::from_utf8(shown).unwrap())
        };
        assert_eq!(gate("y\n", false, true).0, None);
        assert_eq!(gate("YES\n", false, true).0, None);
        for answer in ["n\n", "\n", "maybe\n"] {
            let (code, shown) = gate(answer, false, true);
            assert_eq!(code, Some(0), "{answer:?}");
            assert!(shown.contains(&msg::note(msg::N_USER_ADD_STOPPED, &[])), "{shown}");
        }
        let (code, shown) = gate("", false, true);
        assert_eq!(code, Some(0));
        assert!(shown.contains(&msg::note(msg::N_USER_INPUT_ENDED, &[])), "{shown}");
        // --yes needs no question; a pipe without it is refused, with it goes on.
        assert_eq!(gate("", true, false).0, None);
        assert_eq!(gate("y\n", false, false).0, Some(1));
    }

    #[test]
    fn a_read_error_at_the_prompt_exits_1() {
        struct Broken;
        impl std::io::Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("broken"))
            }
        }
        let mut input = std::io::BufReader::new(Broken);
        let mut shown = Vec::new();
        assert_eq!(reset_gate(&mut input, &mut shown, "ann", false, true), Some(1));
        let mut input = std::io::BufReader::new(Broken);
        let code = interactive(&mut input, &mut shown, &[], |_| panic!("nothing is sent"));
        assert_eq!(code, 1);
    }

    #[test]
    fn a_lost_reply_is_unsure_and_a_coded_one_is_as_it_is() {
        let coded = msg::line(msg::USER_EXISTS, &["ann"]);
        assert_eq!(call_failed("ann", &coded), coded);
        let plain = call_failed("ann", "connection reset");
        assert!(msg::USER_UNSURE.is_in(&plain) && plain.contains("ann") && plain.contains("connection reset"), "{plain}");
    }

    #[test]
    fn a_reply_that_does_not_read_is_not_shown() {
        let e = show_token("token = ", "ann").unwrap_err();
        assert!(msg::USER_REPLY_BAD.is_in(&e) && e.contains("ann"), "{e}");
        let e = show_token("name = \"ann\"\ntoken = 12\nSECRETVALUE", "ann").unwrap_err();
        assert!(!e.contains("SECRETVALUE"), "{e}");
    }
}
