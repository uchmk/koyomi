//! Search and replace across the definitions: `koyomi replace <from> <to>` shows every line of
//! every net that holds `from` and, with `--apply`, writes `to` in its place. The files are
//! changed as text, so their comments and layout stay.

use crate::jobnet;

/// One line that holds the searched text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// 1-based.
    pub line: usize,
    pub before: String,
    pub after: String,
}

/// Every line of `text` that holds `from`, as it is and as it would read with `to`.
pub fn hits(text: &str, from: &str, to: &str) -> Vec<Hit> {
    if from.is_empty() {
        return Vec::new();
    }
    text.lines()
        .enumerate()
        .filter(|(_, l)| l.contains(from))
        .map(|(i, l)| Hit { line: i + 1, before: l.to_owned(), after: l.replace(from, to) })
        .collect()
}

/// `text` with `from` written as `to` everywhere. An error when the result no longer reads as a
/// net: such a file is left as it is for a person to change.
pub fn apply(text: &str, from: &str, to: &str) -> Result<String, String> {
    let out = text.replace(from, to);
    match jobnet::parse(&out) {
        Ok(_) => Ok(out),
        Err(p) => Err(p.first().map(|p| p.message.clone()).unwrap_or_default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET: &str = "format = 1\nname = \"b\"\n[[job]]\nid = \"a\"\nrun = [\"C:/old/tool.exe\"]\n# C:/old/tool.exe is gone\n";

    #[test]
    fn every_line_with_the_text_is_listed_with_its_new_form() {
        let h = hits(NET, "C:/old", "D:/new");
        assert_eq!(h.len(), 2);
        assert_eq!((h[0].line, h[0].after.as_str()), (5, "run = [\"D:/new/tool.exe\"]"));
        assert_eq!(h[1].line, 6);
        assert!(hits(NET, "", "x").is_empty());
        assert!(hits(NET, "nowhere", "x").is_empty());
    }

    #[test]
    fn a_replace_that_breaks_the_file_is_refused() {
        assert!(apply(NET, "\"a\"", "a").is_err());
        let ok = apply(NET, "C:/old", "D:/new").unwrap();
        assert!(ok.contains("run = [\"D:/new/tool.exe\"]") && ok.contains("# D:/new/tool.exe is gone"));
    }
}
