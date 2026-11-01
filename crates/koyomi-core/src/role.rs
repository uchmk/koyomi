//! Who may do what over HTTP (TODO 83, DECISIONS「複数の PC・Web UI・利用者とロール・HA」).
//!
//! Three roles, each holding the one below: `view` reads, `run` also starts and stops runs,
//! `edit` also sends definitions (which still wait for the first-time confirmation in that PC's
//! window). A path this table does not know needs `edit`, so a new path is closed until it is
//! placed here.
//!
//! A caller may also be narrowed to some nets ([`Nets`], TODO 94): the others are as if they
//! were not there. And a caller may hold a role per net ([`Access`], TODO 101), as JP1's resource
//! groups do: `run` on 経理's nets, `view` on 基盤's.

use std::collections::BTreeMap;
use std::fmt;

/// A caller's role. Ordered: `View < Run < Edit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    View,
    Run,
    Edit,
}

impl Role {
    /// The role as `koyomi.toml` writes it.
    pub fn parse(text: &str) -> Result<Role, String> {
        match text {
            "view" => Ok(Role::View),
            "run" => Ok(Role::Run),
            "edit" => Ok(Role::Edit),
            _ => Err(format!("role は view・run・edit のどれか: {text}")),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Role::View => "view",
            Role::Run => "run",
            Role::Edit => "edit",
        }
    }

    /// Whether this role may do what `needed` asks for.
    pub fn allows(self, needed: Role) -> bool {
        self >= needed
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The nets a caller reaches: `[[http.user]]`'s `nets` (TODO 94). Each pattern is
/// `<group>/<name>`, and `*` stands for any run of characters inside one of the two
/// (`経理/*` is every net of 経理, `*/日次` every 日次); a pattern without `*` is one net.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Nets(Vec<String>);

impl Nets {
    /// The patterns as `koyomi.toml` writes them. Empty is an error: no `nets` at all means
    /// every net.
    pub fn parse(patterns: &[String]) -> Result<Nets, String> {
        if patterns.is_empty() {
            return Err("nets を空にしない（全部のネットなら nets を書かない）".to_owned());
        }
        let mut out = Vec::new();
        for p in patterns {
            out.push(pattern(p).map_err(|e| format!("nets は {e}"))?.to_owned());
        }
        Ok(Nets(out))
    }

    /// Whether the net `id` (`<group>/<name>`) is one of these.
    pub fn reaches(&self, id: &str) -> bool {
        self.0.iter().any(|p| names(p, id))
    }

    pub fn patterns(&self) -> &[String] {
        &self.0
    }
}

/// One `<group>/<name>` pattern, trimmed; the error says how to write one.
fn pattern(p: &str) -> Result<&str, String> {
    let p = p.trim();
    let parts: Vec<&str> = p.split('/').collect();
    if parts.len() != 2 || parts.iter().any(|s| s.trim().is_empty()) {
        return Err(format!("<グループ>/<名前> で書く（グループの全部なら 経理/*）: {p}"));
    }
    Ok(p)
}

/// Whether the pattern names the net `id`.
fn names(pattern: &str, id: &str) -> bool {
    let Some((group, name)) = id.split_once('/').filter(|(_, n)| !n.contains('/')) else { return false };
    pattern.split_once('/').is_some_and(|(g, n)| glob(g, group) && glob(n, name))
}

/// What a caller may do over HTTP: one role, or one per net (TODO 101), and which nets they see
/// at all (TODO 94). A net they do not see is as if it were not there, whatever its role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    /// The role on a net no pattern in `roles` names, and on what is not one net's (pausing the
    /// schedule, the ops log, an HA primary taking the schedules back …).
    pub role: Role,
    /// The nets they see (`nets`); `None` is every net.
    pub nets: Option<Nets>,
    /// A role per pattern (`roles`): a net one or more of these name gets the strongest of those,
    /// even when that is less than `role` (`基盤/* = "view"` under `role = "run"`).
    pub roles: Vec<(String, Role)>,
}

impl Access {
    /// One role on everything: what a caller without `nets` or `roles` has.
    pub fn all(role: Role) -> Access {
        Access { role, nets: None, roles: Vec::new() }
    }

    /// Whether they see the net `id`.
    pub fn sees(&self, id: &str) -> bool {
        self.nets.as_ref().is_none_or(|n| n.reaches(id))
    }

    /// Their role on the net `id`.
    pub fn role_on(&self, id: &str) -> Role {
        self.roles.iter().filter(|(p, _)| names(p, id)).map(|&(_, r)| r).max().unwrap_or(self.role)
    }

    /// The most they may do anywhere: what the gate in front of every path checks, before the
    /// net a request is about is known.
    pub fn top(&self) -> Role {
        self.roles.iter().map(|&(_, r)| r).fold(self.role, Role::max)
    }

    /// Whether anything is narrowed, so each request is checked against its net.
    pub fn narrowed(&self) -> bool {
        self.nets.is_some() || !self.roles.is_empty()
    }
}

/// `[[http.user]]`'s `roles` as `koyomi.toml` writes it (`{ "経理/*" = "run" }`). Empty is an
/// error: no `roles` at all means `role` on every net.
pub fn per_net(roles: &BTreeMap<String, String>) -> Result<Vec<(String, Role)>, String> {
    if roles.is_empty() {
        return Err("roles を空にしない（どのネットも role なら roles を書かない）".to_owned());
    }
    let mut out = Vec::new();
    for (key, role) in roles {
        let p = pattern(key).map_err(|e| format!("roles のキーは {e}"))?;
        let r = Role::parse(role.trim()).map_err(|e| format!("roles の {p} の {e}"))?;
        out.push((p.to_owned(), r));
    }
    Ok(out)
}

/// `*` is any run of characters (none too); everything else stands for itself.
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
    while j < t.len() {
        if i < p.len() && p[i] != '*' && p[i] == t[j] {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    p[i..].iter().all(|&c| c == '*')
}

/// The role a request needs. Reads need `view`, but a definition's text needs `edit` (it is what
/// would run); starting, stopping, rerunning and skipping runs and holding nets or skipping their
/// next run need `run`, as does an HA primary taking the schedules back (`POST /standby/release`);
/// anything else, including what this table does not know yet, needs `edit`.
pub fn needs(method: &str, path: &str) -> Role {
    let path = path.trim_end_matches('/');
    let run_part: Option<Vec<&str>> = path.strip_prefix("/runs/").map(|rest| rest.split('/').collect());
    match method {
        "GET" | "HEAD" if path == "/nets/text" => Role::Edit,
        "GET" | "HEAD" => Role::View,
        "POST" if matches!(path, "/runs" | "/nets/hold" | "/nets/release" | "/nets/skip-next" | "/standby/release") => Role::Run,
        "POST" => match run_part.as_deref() {
            Some([id, "stop" | "rerun" | "skip"]) if !id.is_empty() => Role::Run,
            _ => Role::Edit,
        },
        _ => Role::Edit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the roles read as written and hold the ones below them.
    #[test]
    fn the_roles_read_and_hold_the_ones_below() {
        for r in [Role::View, Role::Run, Role::Edit] {
            assert_eq!(Role::parse(r.name()), Ok(r));
        }
        assert!(Role::parse("admin").is_err());
        assert!(Role::Edit.allows(Role::Run) && Role::Run.allows(Role::View));
        assert!(!Role::View.allows(Role::Run) && !Role::Run.allows(Role::Edit));
    }

    /// reads need view, runs need run, and what the table does not know needs edit.
    #[test]
    fn each_request_needs_its_role() {
        assert_eq!(needs("GET", "/runs/20261031-090000-01/log"), Role::View);
        assert_eq!(needs("GET", "/nets/"), Role::View);
        assert_eq!(needs("POST", "/runs"), Role::Run);
        assert_eq!(needs("POST", "/runs/20261031-090000-01/stop"), Role::Run);
        assert_eq!(needs("POST", "/runs//stop"), Role::Edit);
        assert_eq!(needs("POST", "/runs/20261031-090000-01/rerun"), Role::Run);
        assert_eq!(needs("POST", "/nets/hold"), Role::Run);
        assert_eq!(needs("POST", "/nets/skip-next"), Role::Run);
        assert_eq!(needs("POST", "/nets/release/"), Role::Run);
        assert_eq!(needs("POST", "/nets"), Role::Edit);
        assert_eq!(needs("PUT", "/nets/g/a"), Role::Edit);
        assert_eq!(needs("DELETE", "/runs"), Role::Edit);
        assert_eq!(needs("POST", "/something-new"), Role::Edit);
        assert_eq!(needs("GET", "/nets/text/"), Role::Edit);
        assert_eq!(needs("POST", "/nets/propose"), Role::Edit);
        assert_eq!(needs("POST", "/standby/release"), Role::Run);
    }

    fn nets(patterns: &[&str]) -> Result<Nets, String> {
        Nets::parse(&patterns.iter().map(|p| p.to_string()).collect::<Vec<_>>())
    }

    /// `*` stands for any run inside the group or the name; a pattern without one is one net.
    #[test]
    fn nets_reach_what_their_patterns_name() {
        let n = nets(&["経理/*", "共通/日次", "*/月末*"]).unwrap();
        assert!(n.reaches("経理/月次") && n.reaches("経理/日次"));
        assert!(n.reaches("共通/日次") && !n.reaches("共通/日次2") && !n.reaches("共通/週次"));
        assert!(n.reaches("人事/月末締め") && n.reaches("人事/月末") && !n.reaches("人事/締め月末"));
        assert!(!n.reaches("経理") && !n.reaches("経理/a/b") && !n.reaches("経理2/月次"));
        let one = nets(&[" g/a* b "]).unwrap();
        assert!(one.reaches("g/a b") && one.reaches("g/ax b") && !one.reaches("g/a bc"));
        assert!(nets(&["*/*"]).unwrap().reaches("x/y"));
        assert_eq!(nets(&["*/*"]).unwrap().patterns(), ["*/*"]);
        assert!(!Nets::default().reaches("x/y"));
    }

    /// a pattern is `<group>/<name>`, and an empty list is not "every net".
    #[test]
    fn nets_that_do_not_read_are_said() {
        assert!(nets(&[]).unwrap_err().contains("空にしない"));
        for bad in ["経理", "経理/", "/日次", "a/b/c", " ", "経理/ "] {
            let e = nets(&["共通/日次", bad]).unwrap_err();
            assert!(e.starts_with("nets は <グループ>/<名前>"), "{bad}: {e}");
        }
    }

    fn per(pairs: &[(&str, &str)]) -> Result<Vec<(String, Role)>, String> {
        per_net(&pairs.iter().map(|&(p, r)| (p.to_owned(), r.to_owned())).collect())
    }

    /// a net a pattern names gets the strongest role of those naming it, even less than the base;
    /// the others, and what is no net's, get the base.
    #[test]
    fn a_role_per_net_overrides_the_one_role() {
        let a =
            Access {
                role: Role::Run, nets: None, roles: per(&[("経理/*", "edit"), ("経理/月次", "view"), ("基盤/*", "view")]).unwrap()
            };
        assert_eq!(a.role_on("経理/月次"), Role::Edit);
        assert_eq!(a.role_on("経理/日次"), Role::Edit);
        assert_eq!(a.role_on("基盤/監視"), Role::View);
        assert_eq!(a.role_on("人事/給与"), Role::Run);
        assert_eq!(a.top(), Role::Edit);
        assert!(a.narrowed() && a.sees("人事/給与"));
        let low = Access { role: Role::View, nets: None, roles: per(&[("経理/*", "run")]).unwrap() };
        assert_eq!((low.top(), low.role_on("経理/a"), low.role_on("人事/a")), (Role::Run, Role::Run, Role::View));
        let seen = Access { role: Role::Edit, nets: Some(nets(&["経理/*"]).unwrap()), roles: Vec::new() };
        assert!(seen.narrowed() && seen.sees("経理/a") && !seen.sees("人事/a"));
        assert_eq!((seen.top(), seen.role_on("人事/a")), (Role::Edit, Role::Edit));
        assert!(!Access::all(Role::View).narrowed() && Access::all(Role::View).sees("x/y"));
    }

    /// each key is a `<group>/<name>` pattern and each value a role; an empty table is not "every net".
    #[test]
    fn roles_per_net_that_do_not_read_are_said() {
        assert_eq!(per(&[(" 経理/* ", " run ")]).unwrap(), [("経理/*".to_owned(), Role::Run)]);
        assert!(per(&[]).unwrap_err().contains("空にしない"));
        assert!(per(&[("経理", "run")]).unwrap_err().starts_with("roles のキーは <グループ>/<名前>"));
        let e = per(&[("経理/*", "admin")]).unwrap_err();
        assert!(e.starts_with("roles の 経理/* の role は view・run・edit"), "{e}");
    }
}
