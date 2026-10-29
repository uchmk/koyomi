//! One job net's TOML file: what it may say in this version, and where it is wrong.
//!
//! Every message names a line, so `koyomi check` and the tree can say
//! "月次締め.toml の 12 行目: 知らない欄 rnu". A field the design has but this
//! version does not run is an error too (never ignored): a `timeout` that
//! was written and not kept is how a night job hangs.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use toml::Spanned;

/// The `format` this version reads.
pub const FORMAT: i64 = 1;

/// A mistake in a file, at a line (1-based; 0 when it is not in the text).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub line: usize,
    pub message: String,
}

impl Problem {
    /// `月次締め.toml の 12 行目: 知らない欄 rnu`.
    pub fn in_file(&self, file: &str) -> String {
        match self.line {
            0 => format!("{file}: {}", self.message),
            n => format!("{file} の {n} 行目: {}", self.message),
        }
    }
}

/// A job net that reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Jobnet {
    /// `name`, when the file has one.
    pub name: Option<String>,
    pub jobs: Vec<Job>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub name: Option<String>,
    /// The executable first, then one argument per element. Never a shell line.
    pub run: Vec<String>,
    pub dir: Option<String>,
    pub after: Vec<String>,
    pub pos: Option<(f64, f64)>,
}

type Field = Option<Spanned<toml::Value>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNet {
    format: Field,
    name: Field,
    #[serde(default)]
    job: Vec<Spanned<RawJob>>,
    // Of the design, not run by this version.
    calendar: Field,
    missed: Field,
    overlap: Field,
    when: Field,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJob {
    id: Field,
    name: Field,
    run: Field,
    dir: Field,
    after: Field,
    pos: Field,
    // Of the design, not run by this version.
    kind: Field,
    #[serde(rename = "use")]
    use_: Field,
    to: Field,
    shell: Field,
    script: Field,
    timeout: Field,
    retry: Field,
    judge: Field,
}

/// The 1-based line of byte `offset` in `text`.
pub fn line_of(text: &str, offset: usize) -> usize {
    let end = offset.min(text.len());
    text.as_bytes()[..end].iter().filter(|&&b| b == b'\n').count() + 1
}

/// Reads `text`; every problem found, in line order, when it does not.
pub fn parse(text: &str) -> Result<Jobnet, Vec<Problem>> {
    let raw: RawNet = match toml::from_str(text) {
        Ok(r) => r,
        Err(e) => {
            let span = e.span();
            let line = span.as_ref().map_or(0, |s| line_of(text, s.start));
            let at = span.and_then(|s| text.get(s));
            return Err(vec![Problem { line, message: explain(e.message(), at) }]);
        }
    };
    let mut c = Checker { text, problems: Vec::new() };
    c.top(&raw);
    let jobs = c.jobs(&raw.job);
    if c.problems.is_empty() {
        Ok(Jobnet { name: c.string(&raw.name, "name"), jobs })
    } else {
        c.problems.sort_by_key(|p| p.line);
        Err(c.problems)
    }
}

/// The reader's words for the toml crate's.
fn explain(message: &str, at: Option<&str>) -> String {
    if let Some(name) = message.strip_prefix("unknown field `").and_then(|r| r.split('`').next()) {
        return format!("知らない欄 {name}");
    }
    if message.starts_with("duplicate key") {
        return format!("同じ欄が 2 つある {}", at.unwrap_or("").trim()).trim_end().to_string();
    }
    format!("TOML の書き方が違う: {message}")
}

struct Checker<'a> {
    text: &'a str,
    problems: Vec<Problem>,
}

impl Checker<'_> {
    fn add(&mut self, offset: usize, message: impl Into<String>) {
        let line = line_of(self.text, offset);
        self.problems.push(Problem { line, message: message.into() });
    }

    fn not_yet(&mut self, field: &Field, name: &str) {
        if let Some(f) = field {
            self.add(f.span().start, format!("この版ではまだ使えない: {name}"));
        }
    }

    fn string(&mut self, field: &Field, name: &str) -> Option<String> {
        let f = field.as_ref()?;
        match f.get_ref() {
            toml::Value::String(s) => Some(s.clone()),
            _ => {
                self.add(f.span().start, format!("{name} は文字列で書く"));
                None
            }
        }
    }

    fn top(&mut self, raw: &RawNet) {
        match &raw.format {
            None => self.problems.push(Problem { line: 1, message: format!("format = {FORMAT} が要る") }),
            Some(f) if f.get_ref().as_integer() != Some(FORMAT) => {
                self.add(f.span().start, format!("この版が読める format は {FORMAT} だけ"));
            }
            Some(_) => {}
        }
        self.string(&raw.name, "name");
        for (field, name) in [(&raw.calendar, "calendar"), (&raw.missed, "missed"), (&raw.overlap, "overlap"), (&raw.when, "[[when]]")] {
            self.not_yet(field, name);
        }
    }

    fn jobs(&mut self, raw: &[Spanned<RawJob>]) -> Vec<Job> {
        let mut jobs = Vec::new();
        let mut seen = BTreeSet::new();
        for entry in raw {
            let start = entry.span().start;
            let j = entry.get_ref();
            for (field, name) in [
                (&j.kind, "kind"),
                (&j.use_, "use"),
                (&j.to, "to"),
                (&j.shell, "shell"),
                (&j.script, "script"),
                (&j.timeout, "timeout"),
                (&j.retry, "retry"),
                (&j.judge, "judge"),
            ] {
                self.not_yet(field, name);
            }
            let id = match &j.id {
                None => {
                    self.add(start, "id が無い");
                    String::new()
                }
                Some(f) => match f.get_ref() {
                    toml::Value::String(s) if !s.is_empty() => {
                        if !seen.insert(s.clone()) {
                            self.add(f.span().start, format!("同じ id が 2 つある: {s}"));
                        }
                        s.clone()
                    }
                    _ => {
                        self.add(f.span().start, "id は空でない文字列で書く");
                        String::new()
                    }
                },
            };
            let run = self.run(&j.run, start);
            let after = self.after(&j.after);
            let dir = self.string(&j.dir, "dir");
            let name = self.string(&j.name, "name");
            let pos = self.pos(&j.pos);
            jobs.push(Job { id, name, run, dir, after, pos });
        }
        for (job, entry) in jobs.iter().zip(raw) {
            let at = entry.get_ref().after.as_ref().map_or(entry.span().start, |f| f.span().start);
            for a in job.after.iter().filter(|a| !seen.contains(a.as_str())) {
                self.add(at, format!("after が無い id を指している: {a}"));
            }
        }
        if let Some(cycle) = find_cycle(&jobs) {
            let first = jobs.iter().position(|j| j.id == cycle[0]).unwrap_or(0);
            self.add(raw[first].span().start, format!("after が輪になっている: {}", cycle.join(" → ")));
        }
        jobs
    }

    fn run(&mut self, field: &Field, job_start: usize) -> Vec<String> {
        let Some(f) = field else {
            self.add(job_start, "run が無い");
            return Vec::new();
        };
        let at = f.span().start;
        let toml::Value::Array(items) = f.get_ref() else {
            self.add(at, "run は文字列の配列で書く（例: run = ['C:\\batch\\a.exe', '--x']）");
            return Vec::new();
        };
        let strings: Vec<String> = items.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        if strings.len() != items.len() {
            self.add(at, "run の要素は全部文字列で書く");
        } else if strings.is_empty() {
            self.add(at, "run が空");
        } else if strings[0].trim().is_empty() {
            self.add(at, "run の先頭（実行ファイル）が空");
        }
        strings
    }

    fn after(&mut self, field: &Field) -> Vec<String> {
        let Some(f) = field else { return Vec::new() };
        let at = f.span().start;
        let toml::Value::Array(items) = f.get_ref() else {
            self.add(at, "after は id の配列で書く（例: after = [\"extract\"]）");
            return Vec::new();
        };
        let mut out = Vec::new();
        for v in items {
            match v {
                toml::Value::String(s) => out.push(s.clone()),
                toml::Value::Table(_) => self.add(at, "この版ではまだ使えない: after の線の種類（{ job, on }）"),
                _ => self.add(at, "after の要素は id の文字列で書く"),
            }
        }
        out
    }

    fn pos(&mut self, field: &Field) -> Option<(f64, f64)> {
        let f = field.as_ref()?;
        let num = |v: &toml::Value| v.as_float().or_else(|| v.as_integer().map(|i| i as f64));
        if let toml::Value::Array(a) = f.get_ref() {
            if let [x, y] = a.as_slice() {
                if let (Some(x), Some(y)) = (num(x), num(y)) {
                    return Some((x, y));
                }
            }
        }
        self.add(f.span().start, "pos は [x, y] の数 2 つで書く");
        None
    }
}

/// The ids of a loop in `after` (`a → b → a`), if any.
fn find_cycle(jobs: &[Job]) -> Option<Vec<String>> {
    type After<'a> = BTreeMap<&'a str, &'a [String]>;
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Open,
        Done,
    }
    fn visit<'a>(id: &'a str, after: &After<'a>, marks: &mut BTreeMap<&'a str, Mark>, path: &mut Vec<&'a str>) -> Option<Vec<String>> {
        match marks.get(id) {
            Some(Mark::Done) => return None,
            Some(Mark::Open) => {
                let from = path.iter().position(|p| *p == id).unwrap_or(0);
                let mut cycle: Vec<String> = path[from..].iter().map(|s| s.to_string()).collect();
                cycle.push(id.to_string());
                return Some(cycle);
            }
            None => {}
        }
        marks.insert(id, Mark::Open);
        path.push(id);
        for next in after.get(id).copied().unwrap_or(&[]) {
            if after.contains_key(next.as_str()) {
                if let Some(c) = visit(next, after, marks, path) {
                    return Some(c);
                }
            }
        }
        path.pop();
        marks.insert(id, Mark::Done);
        None
    }
    let after: After = jobs.iter().map(|j| (j.id.as_str(), j.after.as_slice())).collect();
    let mut marks = BTreeMap::new();
    jobs.iter().find_map(|j| visit(&j.id, &after, &mut marks, &mut Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"format = 1
name = "月次締め"

[[job]]
id = "extract"
name = "売上の抽出"
run = ['C:\batch\extract.bat', '--month']
dir = 'C:\batch'
pos = [40, 80]

[[job]]
id = "zip"
run = ["zip.exe", "a b"]
after = ["extract"]
"#;

    fn problems(text: &str) -> Vec<String> {
        match parse(text) {
            Ok(_) => Vec::new(),
            Err(p) => p.iter().map(|p| p.in_file("a.toml")).collect(),
        }
    }

    #[test]
    fn a_good_file_reads() {
        let net = parse(GOOD).unwrap();
        assert_eq!(net.name.as_deref(), Some("月次締め"));
        assert_eq!(net.jobs.len(), 2);
        assert_eq!(net.jobs[0].run, [r"C:\batch\extract.bat", "--month"]);
        assert_eq!(net.jobs[0].pos, Some((40.0, 80.0)));
        assert_eq!(net.jobs[1].after, ["extract"]);
    }

    #[test]
    fn an_unknown_field_is_named_with_its_line() {
        let text = GOOD.replace("run = [\"zip.exe\", \"a b\"]", "rnu = [\"zip.exe\"]");
        assert_eq!(problems(&text), ["a.toml の 13 行目: 知らない欄 rnu"]);
    }

    #[test]
    fn toml_syntax_errors_name_a_line() {
        let p = problems("format = 1\nname = \n");
        assert_eq!(p.len(), 1);
        assert!(p[0].starts_with("a.toml の 2 行目: TOML の書き方が違う"), "{p:?}");
    }

    #[test]
    fn run_must_be_a_non_empty_array_of_strings() {
        let head = "format = 1\n[[job]]\nid = \"a\"\n";
        assert_eq!(problems(head), ["a.toml の 2 行目: run が無い"]);
        assert_eq!(problems(&format!("{head}run = []\n")), ["a.toml の 4 行目: run が空"]);
        assert!(problems(&format!("{head}run = \"x.exe --a\"\n"))[0].starts_with("a.toml の 4 行目: run は文字列の配列で書く"));
        assert_eq!(problems(&format!("{head}run = [\"\"]\n")), ["a.toml の 4 行目: run の先頭（実行ファイル）が空"]);
        assert_eq!(problems(&format!("{head}run = [\"x\", 1]\n")), ["a.toml の 4 行目: run の要素は全部文字列で書く"]);
    }

    #[test]
    fn after_must_point_at_an_id_and_not_loop() {
        let t = |after_a: &str| {
            format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n{after_a}\n[[job]]\nid = \"b\"\nrun = [\"x\"]\nafter = [\"a\"]\n")
        };
        assert!(problems(&t("")).is_empty());
        assert_eq!(problems(&t("after = [\"zzz\"]")), ["a.toml の 5 行目: after が無い id を指している: zzz"]);
        assert_eq!(problems(&t("after = [\"b\"]")), ["a.toml の 2 行目: after が輪になっている: a → b → a"]);
        assert_eq!(problems(&t("after = [\"a\"]")), ["a.toml の 2 行目: after が輪になっている: a → a"]);
    }

    #[test]
    fn a_repeated_id_is_named_at_its_second_use() {
        let text = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n";
        assert_eq!(problems(text), ["a.toml の 6 行目: 同じ id が 2 つある: a"]);
    }

    #[test]
    fn fields_that_do_not_run_yet_are_refused_not_ignored() {
        for f in [
            "timeout = \"5m\"",
            "retry = { count = 2, wait = \"5m\" }",
            "judge = { warn = \"1\" }",
            "kind = \"notify\"",
            "shell = \"powershell\"",
            "script = \"x\"",
            "use = \"a/b\"",
            "to = \"c\"",
        ] {
            let text = format!("format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\n{f}\n");
            let name = f.split(' ').next().unwrap();
            assert_eq!(problems(&text), [format!("a.toml の 5 行目: この版ではまだ使えない: {name}")], "{f}");
        }
        for f in ["calendar = \"会社\"", "missed = \"skip\"", "overlap = \"skip\""] {
            let text = format!("format = 1\n{f}\n");
            let name = f.split(' ').next().unwrap();
            assert_eq!(problems(&text), [format!("a.toml の 2 行目: この版ではまだ使えない: {name}")], "{f}");
        }
        let when = "format = 1\n[[when]]\nevery = \"day\"\n";
        assert_eq!(problems(when), ["a.toml の 2 行目: この版ではまだ使えない: [[when]]"]);
        let line = "format = 1\n[[job]]\nid = \"a\"\nrun = [\"x\"]\nafter = [{ job = \"b\", on = \"error\" }]\n";
        assert_eq!(problems(line), ["a.toml の 5 行目: この版ではまだ使えない: after の線の種類（{ job, on }）"]);
    }

    #[test]
    fn format_must_be_one() {
        assert_eq!(problems("name = \"x\"\n"), ["a.toml の 1 行目: format = 1 が要る"]);
        assert_eq!(problems("format = 2\n"), ["a.toml の 1 行目: この版が読める format は 1 だけ"]);
    }

    #[test]
    fn several_problems_come_in_line_order() {
        let text = "format = 1\n[[job]]\nid = \"a\"\n[[job]]\nid = \"a\"\nrun = []\n";
        assert_eq!(
            problems(text),
            ["a.toml の 2 行目: run が無い", "a.toml の 5 行目: 同じ id が 2 つある: a", "a.toml の 6 行目: run が空"]
        );
    }

    #[test]
    fn line_of_counts_from_one() {
        assert_eq!(line_of("a\nb\nc", 0), 1);
        assert_eq!(line_of("a\nb\nc", 2), 2);
        assert_eq!(line_of("a\nb\nc", 99), 3);
    }
}
