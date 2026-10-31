//! Who may do what over HTTP (TODO 83, DECISIONS「複数の PC・Web UI・利用者とロール・HA」).
//!
//! Three roles, each holding the one below: `view` reads, `run` also starts and stops runs,
//! `edit` also sends definitions (which still wait for the first-time confirmation in that PC's
//! window). A path this table does not know needs `edit`, so a new path is closed until it is
//! placed here.

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

/// The role a request needs. Reads need `view`; starting, stopping, rerunning and skipping runs and
/// holding nets need `run`; anything else, including what this table does not know yet, needs `edit`.
pub fn needs(method: &str, path: &str) -> Role {
    let path = path.trim_end_matches('/');
    let run_part: Option<Vec<&str>> = path.strip_prefix("/runs/").map(|rest| rest.split('/').collect());
    match method {
        "GET" | "HEAD" => Role::View,
        "POST" if matches!(path, "/runs" | "/nets/hold" | "/nets/release") => Role::Run,
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
        assert_eq!(needs("POST", "/nets/release/"), Role::Run);
        assert_eq!(needs("POST", "/nets"), Role::Edit);
        assert_eq!(needs("PUT", "/nets/g/a"), Role::Edit);
        assert_eq!(needs("DELETE", "/runs"), Role::Edit);
        assert_eq!(needs("POST", "/something-new"), Role::Edit);
    }
}
