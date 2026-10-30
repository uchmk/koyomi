//! A net's new name, and the other nets that name it: `koyomi rename` moves a net and, when
//! asked, points every child net, `wait_for` and `after_jobnet` at the new name. The files are
//! changed as text, line by line, so their comments and layout stay.

use crate::jobnet::{self, Jobnet, Part};
use crate::scan::Def;
use crate::when::Kind;
use crate::write::q;

/// The keys whose value names another net.
const KEYS: [&str; 3] = ["use", "wait_for", "after_jobnet"];

/// Whether `net` names `id` as a child net, in `wait_for` or in `after_jobnet`.
pub fn names(net: &Jobnet, id: &str) -> bool {
    net.jobs.iter().any(|j| matches!(&j.part, Part::Jobnet { use_ } if use_ == id))
        || net.when.iter().any(|w| w.wait_for.as_deref() == Some(id) || matches!(&w.kind, Kind::AfterJobnet { jobnet, .. } if jobnet == id))
}

/// The ids of the nets other than `id` itself that name it, sorted.
pub fn referrers(defs: &[Def], id: &str) -> Vec<String> {
    let mut out: Vec<String> =
        defs.iter().filter(|d| d.id != id && d.net.as_ref().is_some_and(|n| names(n, id))).map(|d| d.id.clone()).collect();
    out.sort();
    out.dedup();
    out
}

/// `text` with every `use`, `wait_for` and `after_jobnet` that names `from` naming `to`. An
/// error when it does not read afterwards, or still names `from` (written in a way this does
/// not follow, such as a string with escapes): such a file is left for a person to change.
pub fn retarget(text: &str, from: &str, to: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        out.push_str(&retarget_line(line, from, to));
    }
    match jobnet::parse(&out) {
        Ok(net) if !names(&net, from) => Ok(out),
        Ok(_) => Err(format!("{from} の書き方を直せない")),
        Err(p) => Err(p.first().map(|p| p.message.clone()).unwrap_or_default()),
    }
}

/// `text` with the net's own `name` written as `to` when it is `from` (the old file name) or
/// empty, so a copy or a moved net is not still titled by the old name. A custom title stays.
/// Only the line changes, so comments and order stay; text without such a line is returned as is.
pub fn retitle(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut top = true;
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with('[') {
            top = false;
        }
        match top.then(|| retitle_line(line, from, to)).flatten() {
            Some(new) => out.push_str(&new),
            None => out.push_str(line),
        }
    }
    out
}

/// `text` with the net's own `name` set to `title` (trimmed): replaced where the file has one, added
/// after `format` where it has none, and dropped when `title` is empty, so the tree shows the file
/// name. Only the top-level lines change.
pub fn set_title(text: &str, title: &str) -> String {
    let title = title.trim();
    let is_name = |l: &str| l.trim_start().strip_prefix("name").is_some_and(|r| r.trim_start_matches([' ', '\t']).starts_with('='));
    let mut out = String::with_capacity(text.len() + title.len() + 12);
    let (mut top, mut done) = (true, false);
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with('[') {
            top = false;
        }
        if top && is_name(line) {
            if !title.is_empty() && !done {
                out.push_str(&format!(
                    "name = {}
",
                    crate::write::q(title)
                ));
            }
            done = true;
            continue;
        }
        out.push_str(line);
        if top && !done && !title.is_empty() && line.trim_start().starts_with("format") {
            if !line.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&format!(
                "name = {}
",
                crate::write::q(title)
            ));
            done = true;
        }
    }
    out
}

/// `name = "from"` (or `''`, or `""`) on this line written as `name = "to"`.
fn retitle_line(line: &str, from: &str, to: &str) -> Option<String> {
    let value = line.trim_start().strip_prefix("name")?.trim_start_matches([' ', '\t']).strip_prefix('=')?;
    let value = value.trim_start_matches([' ', '\t']);
    [q(from), format!("'{from}'"), q(""), "''".to_owned()].iter().find_map(|old| {
        let tail = value.strip_prefix(old.as_str())?;
        Some(format!("{}{}{tail}", &line[..line.len() - value.len()], q(to)))
    })
}

/// One line with each `key = "from"` (or `'from'`) of [`KEYS`] naming `to`; inside a string
/// and after a comment's `#` nothing is changed.
fn retarget_line(line: &str, from: &str, to: &str) -> String {
    let quoted = [q(from), format!("'{from}'")];
    let mut out = String::new();
    let mut rest = line;
    // The quote of the string the scan is in.
    let mut inside: Option<char> = None;
    'scan: while !rest.is_empty() {
        for key in KEYS {
            if inside.is_some() {
                break;
            }
            let Some(after_key) = rest.strip_prefix(key) else { continue };
            // A whole key: what came before is the line's start, a space, `{` or `,`.
            let before = out.chars().next_back();
            if before.is_some_and(|c| !(c.is_whitespace() || c == '{' || c == ',')) {
                continue;
            }
            let eq = after_key.trim_start_matches([' ', '\t']);
            let Some(value) = eq.strip_prefix('=') else { continue };
            let value = value.trim_start_matches([' ', '\t']);
            for s in &quoted {
                if let Some(tail) = value.strip_prefix(s.as_str()) {
                    out.push_str(&rest[..rest.len() - value.len()]);
                    out.push_str(&q(to));
                    rest = tail;
                    continue 'scan;
                }
            }
        }
        let c = rest.chars().next().expect("not empty");
        match (inside, c) {
            (None, '#') => {
                out.push_str(rest);
                break;
            }
            (None, '"' | '\'') => inside = Some(c),
            (Some('"'), '\\') => {
                // An escape: the next character does not end the string.
                out.push(c);
                rest = &rest[1..];
                let Some(n) = rest.chars().next() else { break };
                out.push(n);
                rest = &rest[n.len_utf8()..];
                continue;
            }
            (Some(open), _) if c == open => inside = None,
            _ => {}
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET: &str = "format = 1\nname = \"b\" # uses = \"g/a\"\n\
                       [[job]]\nid = \"c\"\nkind = \"jobnet\"\nuse   =  \"g/a\"\n\
                       [[job]]\nid = \"d\"\nrun = [\"echo\", \"g/a\"]\n\
                       [[when]]\nafter_jobnet = 'g/a' # the end of a\non = \"ok\"\n\
                       [[when]]\nevery = \"day\"\nat = [\"02:00\"]\nwait_for = \"g/a\"\n";

    #[test]
    fn set_title_replaces_adds_or_drops_the_net_name() {
        let t = "format = 1
name = \"ab\"
[[job]]
id = \"j\"
name = \"x\"
";
        assert_eq!(
            set_title(t, " 月次 "),
            "format = 1
name = \"月次\"
[[job]]
id = \"j\"
name = \"x\"
"
        );
        assert_eq!(
            set_title(t, ""),
            "format = 1
[[job]]
id = \"j\"
name = \"x\"
"
        );
        let bare = "format = 1
[[job]]
id = \"j\"
";
        assert_eq!(
            set_title(bare, "n"),
            "format = 1
name = \"n\"
[[job]]
id = \"j\"
"
        );
        assert_eq!(set_title(bare, ""), bare);
    }

    #[test]
    fn a_title_that_was_the_old_file_name_follows_the_new_one_and_a_custom_title_stays() {
        let t = "# keep
format = 1
name = \"ab\" # the title
[[job]]
id = \"j\"
name = \"ab\"
";
        assert_eq!(
            retitle(t, "ab", "ab-2"),
            "# keep
format = 1
name = \"ab-2\" # the title
[[job]]
id = \"j\"
name = \"ab\"
"
        );
        assert_eq!(
            retitle(
                "name = \"Month end\"
",
                "ab",
                "ab-2"
            ),
            "name = \"Month end\"
"
        );
        assert_eq!(
            retitle(
                "name = \"\"
",
                "ab",
                "ab-2"
            ),
            "name = \"ab-2\"
"
        );
        assert_eq!(
            retitle(
                "format = 1
",
                "ab",
                "ab-2"
            ),
            "format = 1
"
        );
    }

    #[test]
    fn the_three_keys_are_pointed_at_the_new_name_and_nothing_else_changes() {
        let out = retarget(NET, "g/a", "h/新").unwrap();
        assert!(out.contains("use   =  \"h/新\"\n"));
        assert!(out.contains("after_jobnet = \"h/新\" # the end of a\n"));
        assert!(out.contains("wait_for = \"h/新\"\n"));
        // A job's argument and a comment that only look like it stay.
        assert!(out.contains("run = [\"echo\", \"g/a\"]"));
        assert!(out.contains("# uses = \"g/a\""));
        let net = jobnet::parse(&out).unwrap();
        assert!(names(&net, "h/新") && !names(&net, "g/a"));
    }

    #[test]
    fn a_spelling_it_does_not_follow_is_refused() {
        let text = "format = 1\nname = \"b\"\n[[job]]\nid = \"c\"\nkind = \"jobnet\"\nuse = \"g\\u002Fa\"\n";
        assert!(names(&jobnet::parse(text).unwrap(), "g/a"));
        assert!(retarget(text, "g/a", "g/b").is_err());
    }

    #[test]
    fn an_inline_table_is_followed() {
        assert_eq!(retarget_line("x = { use = \"g/a\", wait_for=\"g/a\" }\n", "g/a", "g/b"), "x = { use = \"g/b\", wait_for=\"g/b\" }\n");
        assert_eq!(retarget_line("reuse = \"g/a\"\n", "g/a", "g/b"), "reuse = \"g/a\"\n");
        assert_eq!(retarget_line("x = 1 # use = \"g/a\"\n", "g/a", "g/b"), "x = 1 # use = \"g/a\"\n");
        assert_eq!(retarget_line("s = \"a \\\" use = 'g/a'\"\n", "g/a", "g/b"), "s = \"a \\\" use = 'g/a'\"\n");
    }
}
