//! `koyomi uses <target>`: where a calendar, a net or a program is used, so a change
//! (or a stop) can be weighed before it is made. The target may be a calendar name,
//! a `グループ/名前` or the program a job runs (`robocopy`, `C:\tools\x.exe`).

use crate::jobnet::Part;
use crate::scan::Def;
use crate::when::Kind;

/// One line per use of `target` in `defs`; empty when nothing uses it.
pub fn find(defs: &[Def], target: &str) -> Vec<String> {
    let target = target.trim();
    let want = program(target);
    let mut out = Vec::new();
    for d in defs {
        if d.calendar.as_deref() == Some(target) {
            out.push(format!("calendar   {}", d.id));
        }
        let Some(net) = &d.net else { continue };
        for j in &net.jobs {
            if let Part::Jobnet { use_ } = &j.part {
                if use_ == target {
                    out.push(format!("child net  {} (job {})", d.id, j.id));
                }
            }
            if matches!(j.part, Part::Job) && j.run.first().is_some_and(|c| program(c) == want) {
                out.push(format!("program    {} (job {})", d.id, j.id));
            }
        }
        for w in &net.when {
            if w.wait_for.as_deref() == Some(target) {
                out.push(format!("waits for  {}", d.id));
            }
            if let Kind::AfterJobnet { jobnet, on } = &w.kind {
                if jobnet == target {
                    out.push(format!("follows    {} (on {on:?})", d.id));
                }
            }
        }
    }
    out
}

/// The program's file name without a folder or `.exe`, lower case: what two spellings of one program share.
fn program(command: &str) -> String {
    let name = command.rsplit([char::from(92), '/']).next().unwrap_or(command).to_lowercase();
    name.strip_suffix(".exe").map(str::to_owned).unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobnet;
    use crate::scan::Standing;

    fn def(id: &str, calendar: Option<&str>, text: &str) -> Def {
        Def {
            id: id.into(),
            group: "g".into(),
            name: id.into(),
            title: None,
            file: format!("{id}.toml"),
            standing: Standing::Confirmed,
            problems: Vec::new(),
            net: Some(jobnet::parse(text).unwrap()),
            sha256: String::new(),
            text: text.into(),
            previous: None,
            calendar: calendar.map(str::to_owned),
        }
    }

    #[test]
    fn calendars_nets_and_programs_are_found() {
        let a = def("g/a", Some("会社"), "format = 1\nname = \"a\"\n[[job]]\nid = \"j\"\nrun = [\"C:/tools/Robocopy.exe\", \"x\"]\n");
        let b = def(
            "g/b",
            None,
            "format = 1\nname = \"b\"\n[[job]]\nid = \"c\"\nkind = \"jobnet\"\nuse = \"g/a\"\n[[when]]\nafter_jobnet = \"g/a\"\non = \"ok\"\n",
        );
        let defs = [a, b];
        assert_eq!(find(&defs, "会社"), vec!["calendar   g/a"]);
        assert_eq!(find(&defs, "robocopy").len(), 1);
        assert_eq!(find(&defs, "C:/x/ROBOCOPY.EXE").len(), 1);
        let n = find(&defs, "g/a");
        assert!(n.iter().any(|l| l.starts_with("child net  g/b")) && n.iter().any(|l| l.starts_with("follows    g/b")), "{n:?}");
        assert!(find(&defs, "nothing").is_empty());
    }
}
