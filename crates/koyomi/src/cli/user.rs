//! `koyomi user list|role|remove`: the `[[http.user]]` of koyomi.toml, through the engine's door.
//!
//! The engine does the reading and the writing (an admin over HTTPS asks for the same things); this
//! only asks, and says what came back. A change is made against the hash of koyomi.toml that the
//! `Users` reply carries, so a file changed in between is refused rather than overwritten.

use std::collections::BTreeMap;

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
            "role" => role(rest),
            "remove" => remove(rest),
            _ => usage("user takes list, role <name> <role> or remove <name>"),
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
        _ => String::new(),
    }
}

fn token_word(l: &Listed) -> String {
    let secret = l.user.secret.as_deref().unwrap_or_default();
    match l.token {
        Token::Ok => msg::note(msg::N_USER_TOKEN_OK, &[]),
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
    let mut rows: Vec<Vec<String>> = vec![msg::note(msg::N_USER_HEAD, &[]).split('|').map(str::to_owned).collect()];
    for l in &listing.user {
        rows.push(vec![l.user.name.clone(), l.user.role.clone(), nets_word(&l.user.nets), roles_word(&l.user.roles), token_word(l)]);
    }
    let width = |c: usize| rows.iter().map(|r| r.get(c).map_or(0, |s| s.chars().count())).max().unwrap_or(0);
    let widths: Vec<usize> = (0..5).map(width).collect();
    for r in &rows {
        let cells: Vec<String> = r.iter().enumerate().map(|(i, s)| format!("{s:<w$}", w = widths[i])).collect();
        out(cells.join("  ").trim_end(), false);
    }
    if bad > 0 {
        out(&msg::note(msg::N_USER_TOKEN_BAD, &[&bad.to_string()]), true);
        return 1;
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
    let roles_text = match roles_word(&new_roles) {
        w if w.is_empty() => msg::note(msg::N_CFG_NONE, &[]),
        w => w,
    };
    out(&msg::note(msg::N_USER_ROLE_DONE, &[name, new_role, &nets_word(&new_nets), &roles_text]), false);
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
        assert_eq!(roles_word(&None), "");
    }

    #[test]
    fn the_words_for_a_missing_token_name_the_secret() {
        let l = listing(&[Token::Missing]);
        assert!(token_word(&l.user[0]).contains("s0"));
    }
}
