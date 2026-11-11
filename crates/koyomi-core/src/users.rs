//! The `[[http.user]]` of koyomi.toml as `koyomi user` and the door's user requests change them.
//!
//! Pure functions on the text of koyomi.toml: nothing is read or written here. A change keeps the
//! comments, the places and the text of every value it does not change (`ito_common::set_tables`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::{Config, HttpUser};
use crate::role::Role;
use crate::{approved, msg};

/// One `[[http.user]]`, without its token: only the name the token is kept under.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub name: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nets: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale: Option<String>,
    /// The name after `secret:`; never a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

/// How the engine sees a user's token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Token {
    /// The credential store holds a value.
    Ok,
    /// The credential store holds none.
    Missing,
    /// The credential store could not be read.
    Unreadable,
    /// The user has no token (Tailscale only).
    None,
    /// The token is kept only as its hash, in koyomi.toml: nothing to check in a store.
    Hash,
}

/// A [`User`] and the state of its token, as the reply to `Users` lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed {
    #[serde(flatten)]
    pub user: User,
    pub token: Token,
}

/// The reply to `Users`: what koyomi.toml hashes to, and the users.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    /// A change is made against this hash ([`sha256`]) and refused when the file is not that any more.
    pub base: String,
    #[serde(default)]
    pub user: Vec<Listed>,
}

impl Listing {
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Result<Listing, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

fn user_of(u: &HttpUser) -> User {
    User {
        name: u.name.trim().to_owned(),
        role: u.role.trim().to_owned(),
        nets: u.nets.clone(),
        roles: u.roles.clone(),
        tailscale: u.tailscale().map(str::to_owned),
        secret: Some(u.secret()).filter(|s| !s.is_empty()).map(str::to_owned),
    }
}

fn users_of(text: &str) -> Result<Vec<HttpUser>, String> {
    Ok(Config::parse(text)?.http.map(|h| h.user).unwrap_or_default())
}

/// The users of `text` with the state of each token: a `sha256:` token is [`Token::Hash`], a `secret:` one
/// is what `secret` says of its name, and one with neither is [`Token::None`].
pub fn listed(text: &str, secret: impl Fn(&str) -> Token) -> Result<Vec<Listed>, String> {
    Ok(users_of(text)?
        .iter()
        .map(|u| {
            let token = if u.hash().is_some() {
                Token::Hash
            } else if u.secret().is_empty() {
                Token::None
            } else {
                secret(u.secret())
            };
            Listed { user: user_of(u), token }
        })
        .collect())
}

/// The users of `text` (koyomi.toml); a file that does not read is an error, not a guess.
pub fn list(text: &str) -> Result<Vec<User>, String> {
    Ok(users_of(text)?.iter().map(user_of).collect())
}

/// The hex SHA-256 of `text`: the base a change is made against.
pub fn sha256(text: &str) -> String {
    approved::hash(text.as_bytes())
}

fn quote(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

/// A user as the keys of its `[[http.user]]` block, each value as TOML. A key the user does not have is not given.
fn block(u: &HttpUser) -> Vec<(&'static str, String)> {
    let mut b = vec![("name", quote(&u.name)), ("role", quote(&u.role))];
    if !u.token.is_empty() {
        b.push(("token", quote(&u.token)));
    }
    if let Some(t) = &u.tailscale {
        b.push(("tailscale", quote(t)));
    }
    if let Some(n) = &u.nets {
        b.push(("nets", toml::Value::Array(n.iter().cloned().map(toml::Value::String).collect()).to_string()));
    }
    if let Some(r) = &u.roles {
        let table: toml::Table = r.iter().map(|(k, v)| (k.clone(), toml::Value::String(v.clone()))).collect();
        b.push(("roles", toml::Value::Table(table).to_string()));
    }
    b
}

fn has_admin(users: &[HttpUser]) -> bool {
    users.iter().any(|u| u.role() == Role::Admin)
}

/// Writes `after` as the `[[http.user]]` of `text`, refusing to take the last admin away.
fn rewrite(text: &str, before: &[HttpUser], after: &[HttpUser]) -> Result<String, String> {
    if has_admin(before) && !has_admin(after) {
        return Err(msg::line(msg::USER_LAST_ADMIN, &[]));
    }
    let blocks: Vec<Vec<(&str, String)>> = after.iter().map(block).collect();
    let out = ito_common::set_tables(text, "http.user", &blocks)?;
    // The result must read as a koyomi.toml that holds exactly these users.
    let now = users_of(&out)?;
    if now != after {
        return Err(msg::line(msg::REWRITE_MISMATCH, &[]));
    }
    Ok(out)
}

fn missing(name: &str) -> String {
    msg::line(msg::USER_NOT_IN_FILE, &[name])
}

/// `text` with the user `name` given `role`, `nets` and `roles` (`None`: the key is taken out).
/// Everything else stays as it was. Refused: no such user, the last admin not being one any more,
/// and whatever [`Config::parse`] says of the result (an admin with `nets`, a role it does not know …).
pub fn set(
    text: &str,
    name: &str,
    role: &str,
    nets: Option<Vec<String>>,
    roles: Option<BTreeMap<String, String>>,
) -> Result<String, String> {
    let name = name.trim();
    let before = users_of(text)?;
    if !before.iter().any(|u| u.name.trim() == name) {
        return Err(missing(name));
    }
    Role::parse(role.trim()).map_err(|e| msg::line(msg::CFG_USER_FIELD, &[name, msg::uncoded(&e)]))?;
    let after: Vec<HttpUser> = before
        .iter()
        .map(|u| {
            if u.name.trim() == name {
                HttpUser { role: role.trim().to_owned(), nets: nets.clone(), roles: roles.clone(), ..u.clone() }
            } else {
                u.clone()
            }
        })
        .collect();
    rewrite(text, &before, &after)
}

/// `text` with a new user `name` after the others, whose token is kept as `hash` (`sha256:<hash>`). Text
/// with no `[http]` (a new, empty file too) is refused: without it the HTTPS door does not open. Refused
/// also: a name that is there already, and whatever [`Config::parse`] says of the result (an admin with `nets`, a name
/// that does not read, a role it does not know …).
pub fn add(
    text: &str,
    name: &str,
    role: &str,
    nets: Option<Vec<String>>,
    roles: Option<BTreeMap<String, String>>,
    hash: &str,
) -> Result<String, String> {
    let name = name.trim();
    let before = users_of(text)?;
    if before.iter().any(|u| u.name.trim() == name) {
        return Err(msg::line(msg::USER_EXISTS, &[name]));
    }
    let mut after = before.clone();
    after.push(HttpUser {
        name: name.to_owned(),
        role: role.trim().to_owned(),
        token: format!("sha256:{hash}"),
        tailscale: None,
        nets,
        roles,
    });
    if Config::parse(text)?.http.is_none() {
        return Err(msg::line(msg::USER_NO_HTTP, &[]));
    }
    rewrite(text, &before, &after)
}

/// `text` with the token of the user `name` made `sha256:<hash>` (a `secret:` token too, which is left
/// where it is kept). Refused: no such user, and whatever [`Config::parse`] says of the result.
pub fn reset(text: &str, name: &str, hash: &str) -> Result<String, String> {
    let name = name.trim();
    let before = users_of(text)?;
    if !before.iter().any(|u| u.name.trim() == name) {
        return Err(missing(name));
    }
    let after: Vec<HttpUser> = before
        .iter()
        .map(|u| if u.name.trim() == name { HttpUser { token: format!("sha256:{hash}"), ..u.clone() } } else { u.clone() })
        .collect();
    rewrite(text, &before, &after)
}

/// `text` with the user `name` taken out. Refused: no such user, and the last admin.
pub fn remove(text: &str, name: &str) -> Result<String, String> {
    let name = name.trim();
    let before = users_of(text)?;
    if !before.iter().any(|u| u.name.trim() == name) {
        return Err(missing(name));
    }
    let after: Vec<HttpUser> = before.iter().filter(|u| u.name.trim() != name).cloned().collect();
    rewrite(text, &before, &after)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "\
# koyomi settings
concurrency = 2

[http]
port = 8700  # the port
tailnet = 8701

# the boss
[[http.user]]
name = \"boss\"
role = \"admin\"  # all of it
token = \"secret:boss-token\"

[[http.user]]
name = \"sato\"
role = \"run\"
token = \"secret:sato-token\"  # sato's
nets = [\"経理/*\"]

[[http.user]]
name = \"ito\"
role = \"view\"
tailscale = \"ito@example.com\"
";

    fn roles(pairs: &[(&str, &str)]) -> Option<BTreeMap<String, String>> {
        Some(pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect())
    }

    #[test]
    fn list_reads_the_users_without_a_token_value() {
        let u = list(FILE).unwrap();
        assert_eq!(u.len(), 3);
        assert_eq!(u[0].secret.as_deref(), Some("boss-token"));
        assert_eq!(u[1].nets.as_deref(), Some(&["経理/*".to_owned()][..]));
        assert_eq!(u[2].secret, None);
        assert_eq!(u[2].tailscale.as_deref(), Some("ito@example.com"));
        assert!(list("").unwrap().is_empty());
    }

    #[test]
    fn an_invalid_file_is_an_error() {
        assert!(list("concurrency = [\n").is_err());
        assert!(set("concurrency = [\n", "a", "view", None, None).is_err());
        assert!(remove("concurrency = [\n", "a").is_err());
    }

    #[test]
    fn the_hash_is_the_files() {
        assert_eq!(sha256("a"), approved::hash(b"a"));
        assert_ne!(sha256("a"), sha256("b"));
    }

    #[test]
    fn set_keeps_comments_and_the_other_users_text() {
        let out = set(FILE, "sato", "edit", Some(vec!["経理/*".into(), "基盤/日次".into()]), None).unwrap();
        for kept in [
            "# koyomi settings",
            "port = 8700  # the port",
            "# the boss",
            "role = \"admin\"  # all of it",
            "token = \"secret:sato-token\"  # sato's",
        ] {
            assert!(out.contains(kept), "{kept}\n{out}");
        }
        let at = |t: &str| t.find("[[http.user]]\nname = \"ito\"").map(|i| t[i..].to_owned());
        assert_eq!(at(&out), at(FILE), "the next user's lines are byte for byte");
        let boss_end = |t: &str| t[..t.find("[[http.user]]\nname = \"sato\"").unwrap()].to_owned();
        assert_eq!(boss_end(&out), boss_end(FILE));
        let u = list(&out).unwrap();
        assert_eq!(u[1].role, "edit");
        assert_eq!(u[1].nets.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn set_with_none_takes_the_keys_out() {
        let out = set(FILE, "sato", "run", None, None).unwrap();
        assert!(list(&out).unwrap()[1].nets.is_none());
        assert!(!out.contains("経理"), "{out}");
        assert!(out.contains("token = \"secret:sato-token\"  # sato's"));
    }

    #[test]
    fn an_admin_with_nets_or_roles_is_refused_and_the_files_words_are_the_configs() {
        let e = set(FILE, "sato", "admin", Some(vec!["経理/*".into()]), None).unwrap_err();
        assert!(msg::CFG_USER_ADMIN_NARROWED.is_in(&e), "{e}");
        let e = set(FILE, "sato", "admin", None, roles(&[("経理/*", "run")])).unwrap_err();
        assert!(msg::CFG_USER_ADMIN_NARROWED.is_in(&e), "{e}");
        let out = set(FILE, "sato", "admin", None, None).unwrap();
        assert_eq!(list(&out).unwrap()[1].role, "admin");
        let e = set(FILE, "sato", "boss", None, None).unwrap_err();
        assert!(msg::CFG_USER_FIELD.is_in(&e), "{e}");
    }

    #[test]
    fn roles_round_trip_quoted_and_a_sub_table_becomes_inline() {
        let out = set(FILE, "sato", "view", None, roles(&[("経理/*", "run"), ("基盤/*", "edit")])).unwrap();
        assert!(out.contains("\"経理/*\" = \"run\"") || out.contains("'経理/*' = 'run'"), "{out}");
        let u = list(&out).unwrap();
        assert_eq!(u[1].roles, roles(&[("経理/*", "run"), ("基盤/*", "edit")]));

        // The sub-table form: after a change of the role it is the inline form (accepted), with the same pairs.
        let sub = "\
[http]
port = 8700
[[http.user]]
name = \"boss\"
role = \"admin\"
token = \"secret:b\"
[[http.user]]
name = \"sato\"
role = \"view\"
token = \"secret:s\"
[http.user.roles]
\"経理/*\" = \"run\"
";
        assert_eq!(list(sub).unwrap()[1].roles, roles(&[("経理/*", "run")]));
        let out = set(sub, "sato", "edit", None, roles(&[("経理/*", "run")])).unwrap();
        assert!(!out.contains("[http.user.roles]"), "{out}");
        let u = list(&out).unwrap();
        assert_eq!((u[1].role.as_str(), u[1].roles.clone()), ("edit", roles(&[("経理/*", "run")])));
    }

    #[test]
    fn a_comment_after_an_unchanged_value_stays() {
        let out = set(FILE, "boss", "admin", None, None).unwrap();
        assert!(out.contains("role = \"admin\"  # all of it"), "{out}");
        let out = set(FILE, "sato", "view", Some(vec!["経理/*".into()]), None).unwrap();
        assert!(out.contains("token = \"secret:sato-token\"  # sato's"), "{out}");
    }

    #[test]
    fn the_last_admin_cannot_be_removed_or_demoted() {
        let e = remove(FILE, "boss").unwrap_err();
        assert!(msg::USER_LAST_ADMIN.is_in(&e), "{e}");
        let e = set(FILE, "boss", "edit", None, None).unwrap_err();
        assert!(msg::USER_LAST_ADMIN.is_in(&e), "{e}");
        // With a second admin, either goes.
        let two = set(FILE, "sato", "admin", None, None).unwrap();
        assert!(remove(&two, "boss").is_ok());
        assert!(set(&two, "boss", "edit", None, None).is_ok());
        // No admin to begin with: nothing to protect.
        let none = set(&two, "sato", "run", None, None).and_then(|t| set(&t, "boss", "view", None, None));
        assert!(none.is_err(), "the last admin of two is still protected");
        let no_admin = FILE.replace("role = \"admin\"", "role = \"edit\"");
        assert!(remove(&no_admin, "boss").is_ok());
    }

    #[test]
    fn remove_takes_one_block_out_and_leaves_the_rest() {
        let out = remove(FILE, "sato").unwrap();
        let u = list(&out).unwrap();
        assert_eq!(u.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(), ["boss", "ito"]);
        assert!(!out.contains("sato"), "{out}");
        for kept in ["# koyomi settings", "# the boss", "role = \"admin\"  # all of it", "tailscale = \"ito@example.com\""] {
            assert!(out.contains(kept), "{kept}\n{out}");
        }
        let at = |t: &str| t.find("[[http.user]]\nname = \"ito\"").map(|i| t[i..].to_owned());
        assert_eq!(at(&out), at(FILE));
    }

    #[test]
    fn an_unknown_name_is_an_error() {
        assert!(msg::USER_NOT_IN_FILE.is_in(&set(FILE, "nobody", "view", None, None).unwrap_err()));
        assert!(msg::USER_NOT_IN_FILE.is_in(&remove(FILE, "nobody").unwrap_err()));
        assert!(msg::USER_NOT_IN_FILE.is_in(&remove("", "a").unwrap_err()));
    }

    #[test]
    fn the_listing_round_trips_and_holds_no_token_value() {
        let l = Listing {
            base: sha256(FILE),
            user: list(FILE)
                .unwrap()
                .into_iter()
                .zip([Token::Ok, Token::Missing, Token::None])
                .map(|(user, token)| Listed { user, token })
                .collect(),
        };
        let text = l.to_toml();
        assert!(text.contains("token = \"missing\""), "{text}");
        assert_eq!(Listing::parse(&text).unwrap(), l);
    }

    const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn a_list_mixing_all_four_token_states() {
        let text = format!("{FILE}\n[[http.user]]\nname = \"hashed\"\nrole = \"view\"\ntoken = \"sha256:{HASH_A}\"\n");
        let l = listed(&text, |name| if name == "boss-token" { Token::Ok } else { Token::Missing }).unwrap();
        let states: Vec<Token> = l.iter().map(|l| l.token).collect();
        assert_eq!(states, [Token::Ok, Token::Missing, Token::None, Token::Hash]);
        assert_eq!(l[3].user.secret, None);
        let text = Listing { base: String::new(), user: l }.to_toml();
        assert!(text.contains("token = \"hash\""), "{text}");
        assert!(!text.contains(HASH_A), "the hash is not in the listing");
    }

    #[test]
    fn add_to_a_file_with_http_and_no_user_makes_that_one_user() {
        let base = "[http]\nport = 8790\ntoken = \"secret:t\"\n";
        let out = add(base, "sato", "run", Some(vec!["経理/*".into()]), None, HASH_A).unwrap();
        let u = list(&out).unwrap();
        assert_eq!(u.len(), 1);
        assert_eq!((u[0].name.as_str(), u[0].role.as_str()), ("sato", "run"));
        assert_eq!(u[0].nets.as_deref(), Some(&["経理/*".to_owned()][..]));
        assert!(out.contains(&format!("token = \"sha256:{HASH_A}\"")), "{out}");
        assert_eq!(out.matches("[[http.user]]").count(), 1, "{out}");
        assert_eq!(Config::parse(&out).unwrap().http.unwrap().user[0].hash(), Some(HASH_A));
        assert!(out.starts_with(base), "{out}");
    }

    #[test]
    fn add_without_http_is_refused_and_nothing_is_made() {
        for text in ["", "# only a comment\n", "concurrency = 2\n"] {
            let e = add(text, "sato", "run", None, None, HASH_A).unwrap_err();
            assert!(msg::USER_NO_HTTP.is_in(&e), "{text:?}: {e}");
        }
    }

    #[test]
    fn add_with_empty_nets_is_refused() {
        let e = add(FILE, "kato", "view", Some(Vec::new()), None, HASH_A).unwrap_err();
        assert!(msg::CFG_USER_FIELD.is_in(&e) && e.contains("kato"), "{e}");
    }

    #[test]
    fn add_after_others_leaves_their_text_byte_for_byte() {
        let out = add(FILE, "kato", "view", None, None, HASH_B).unwrap();
        assert!(out.starts_with(FILE.trim_end()), "{out}");
        let u = list(&out).unwrap();
        assert_eq!(u.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(), ["boss", "sato", "ito", "kato"]);
        assert!(out.contains(&format!("token = \"sha256:{HASH_B}\"")), "{out}");
    }

    #[test]
    fn add_refuses_a_name_that_is_there_and_what_the_config_refuses() {
        let e = add(FILE, "sato", "view", None, None, HASH_A).unwrap_err();
        assert!(msg::USER_EXISTS.is_in(&e), "{e}");
        let e = add(FILE, "kato", "admin", Some(vec!["経理/*".into()]), None, HASH_A).unwrap_err();
        assert!(msg::CFG_USER_ADMIN_NARROWED.is_in(&e), "{e}");
        assert!(msg::CFG_USER_NAME.is_in(&add(FILE, "a b", "view", None, None, HASH_A).unwrap_err()));
        assert!(msg::CFG_USER_NAME.is_in(&add(FILE, "default", "view", None, None, HASH_A).unwrap_err()));
        assert!(msg::CFG_USER_FIELD.is_in(&add(FILE, "kato", "boss", None, None, HASH_A).unwrap_err()));
        assert!(msg::CFG_TOKEN_HASH_BAD.is_in(&add(FILE, "kato", "view", None, None, "abc").unwrap_err()));
    }

    #[test]
    fn reset_makes_the_token_a_hash_and_keeps_the_rest() {
        let out = reset(FILE, "sato", HASH_A).unwrap();
        assert!(out.contains(&format!("token = \"sha256:{HASH_A}\"")), "{out}");
        assert!(!out.contains("sato-token"), "{out}");
        for kept in ["# the boss", "token = \"secret:boss-token\"", "tailscale = \"ito@example.com\"", "nets = [\"経理/*\"]"] {
            assert!(out.contains(kept), "{kept}\n{out}");
        }
        let l = listed(&out, |_| Token::Ok).unwrap();
        assert_eq!(l[1].token, Token::Hash);
        assert_eq!(l[1].user.secret, None);
        // Every other line of the file is as it was, byte for byte.
        let mut before: Vec<&str> = FILE.lines().collect();
        before.retain(|l| !l.contains("sato-token"));
        let mut after: Vec<&str> = out.lines().collect();
        after.retain(|l| !l.contains(HASH_A));
        assert_eq!(before, after);
    }

    #[test]
    fn reset_gives_a_tailscale_only_user_a_token_and_refuses_an_unknown_name() {
        let out = reset(FILE, "ito", HASH_B).unwrap();
        let l = listed(&out, |_| Token::Ok).unwrap();
        assert_eq!((l[2].token, l[2].user.tailscale.as_deref()), (Token::Hash, Some("ito@example.com")));
        assert!(msg::USER_NOT_IN_FILE.is_in(&reset(FILE, "nobody", HASH_A).unwrap_err()));
        assert!(msg::USER_NOT_IN_FILE.is_in(&reset("", "a", HASH_A).unwrap_err()));
    }
}
