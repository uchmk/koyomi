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
}
