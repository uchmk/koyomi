//! PowerShell's CLIXML on a job's error output, read back as the lines a console would show.
//!
//! A job's script goes to Windows PowerShell as `-EncodedCommand` (`run::Job::command`). With its
//! error output not a console, PowerShell then writes every record but plain text there as CLIXML:
//! a `#< CLIXML` line, then, when it ends, one long `<Objs …>` line holding each error, warning,
//! verbose, debug, progress and information record. A person reading the log wants the text: an
//! error's lines as they are, a warning as `警告: …`, and no progress (`Preparing modules for first
//! use`) or information records (`Write-Host`, which are on the standard output as text too).
//! Text written straight to the error output (`[Console]::Error.WriteLine`) stays as it is.

const HEADER: &str = "#< CLIXML";

/// Reads one job's error output line by line: after the header, CLIXML becomes plain lines.
#[derive(Debug, Default)]
pub struct Clixml {
    on: bool,
}

impl Clixml {
    /// The lines to log for one line of error output: none for the header or a line of progress
    /// alone, the line as it is when it is not CLIXML.
    pub fn lines(&mut self, line: &str) -> Vec<String> {
        if line.trim() == HEADER {
            self.on = true;
            return Vec::new();
        }
        if !self.on {
            return vec![line.to_owned()];
        }
        let mut out = Vec::new();
        let mut plain = String::new();
        let mut rest = line;
        while let Some(at) = rest.find('<') {
            plain.push_str(&rest[..at]);
            rest = &rest[at..];
            if let Some((kind, after)) = rest.strip_prefix("<S S=\"").and_then(|a| a.split_once("\">")) {
                // A killed PowerShell leaves the line cut short: the text up to there still counts.
                let (text, next) = after.split_once("</S>").unwrap_or((after, ""));
                flush(&mut plain, &mut out);
                record(kind, text, &mut out);
                rest = next;
            } else if is_tag(rest, "<Objs") {
                rest = rest.split_once('>').map_or("", |(_, r)| r);
            } else if let Some(r) = rest.strip_prefix("</Objs>") {
                self.on = false;
                rest = r;
            } else if is_tag(rest, "<Obj") {
                rest = skip_obj(rest);
            } else {
                // A `<` in text written straight to the error output.
                plain.push('<');
                rest = &rest[1..];
            }
        }
        plain.push_str(rest);
        flush(&mut plain, &mut out);
        out
    }
}

/// Whether `s` opens the element `name` (`<Obj` and not `<Objs`).
fn is_tag(s: &str, name: &str) -> bool {
    s.strip_prefix(name).is_some_and(|r| matches!(r.chars().next(), Some(' ' | '>' | '/')))
}

/// What follows the `<Obj …>` element `s` starts with, other objects inside it and all.
fn skip_obj(mut s: &str) -> &str {
    let mut depth = 0u32;
    loop {
        let Some(at) = s.find('<') else { return "" };
        s = &s[at..];
        if is_tag(s, "<Obj") {
            let Some(end) = s.find('>') else { return "" };
            if !s[..end].ends_with('/') {
                depth += 1;
            }
            s = &s[end + 1..];
        } else if let Some(r) = s.strip_prefix("</Obj>") {
            depth = depth.saturating_sub(1);
            s = r;
        } else {
            s = &s[1..];
        }
        if depth == 0 {
            return s;
        }
    }
}

/// Text met outside any record, as lines.
fn flush(plain: &mut String, out: &mut Vec<String>) {
    out.extend(plain.split('\n').map(|l| l.trim_end_matches('\r')).filter(|l| !l.trim().is_empty()).map(str::to_owned));
    plain.clear();
}

/// One `<S S="kind">` record's lines, a warning's, verbose's or debug's first line marked as the
/// console marks it. An error's own text says it is one; the `! ` of the error output is enough.
fn record(kind: &str, text: &str, out: &mut Vec<String>) {
    let mark = match kind.to_ascii_lowercase().as_str() {
        "warning" => "警告: ",
        "verbose" => "詳細: ",
        "debug" => "デバッグ: ",
        _ => "",
    };
    let text = unescape(&entities(text));
    let mut lines = text.split('\n').map(|l| l.trim_end_matches('\r')).filter(|l| !l.trim().is_empty());
    if let Some(first) = lines.next() {
        out.push(format!("{mark}{first}"));
    }
    out.extend(lines.map(str::to_owned));
}

/// XML's character references: `&lt;`, `&amp;`, `&#10;`, `&#x0A;` …
fn entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let named = tail.find(';').filter(|e| *e <= 10).and_then(|e| {
            let name = &tail[1..e];
            let c = match name {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => if let Some(hex) = name.strip_prefix("#x") {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    name.strip_prefix('#').and_then(|d| d.parse().ok())
                }
                .and_then(char::from_u32),
            };
            c.map(|c| (c, e + 1))
        });
        match named {
            Some((c, len)) => {
                out.push(c);
                rest = &tail[len..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// PowerShell's `_xHHHH_`: one UTF-16 unit (a line break, a control character, half of an
/// emoji), and `_x005F_` for a `_` that would otherwise start one.
fn unescape(s: &str) -> String {
    let mut units: Vec<u16> = Vec::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find("_x") {
        units.extend(rest[..at].encode_utf16());
        let tail = &rest[at..];
        let unit = tail
            .get(2..6)
            .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()) && tail.as_bytes().get(6) == Some(&b'_'))
            .and_then(|h| u16::from_str_radix(h, 16).ok());
        match unit {
            Some(u) => {
                units.push(u);
                rest = &tail[7..];
            }
            None => {
                units.extend("_x".encode_utf16());
                rest = &tail[2..];
            }
        }
    }
    units.extend(rest.encode_utf16());
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBJS: &str = r#"<Objs Version="1.1.0.1" xmlns="http://schemas.microsoft.com/powershell/2004/04">"#;

    fn read(lines: &[&str]) -> Vec<String> {
        let mut c = Clixml::default();
        lines.iter().flat_map(|l| c.lines(l)).collect()
    }

    #[test]
    fn output_that_is_not_clixml_passes_as_it_is() {
        assert_eq!(read(&["error: <none> & _x000D_", ""]), ["error: <none> & _x000D_", ""]);
    }

    #[test]
    fn errors_become_their_lines_and_progress_goes() {
        let objs = format!(
            "{OBJS}{}{}{}</Objs>",
            r#"<Obj S="progress" RefId="0"><TN RefId="0"><T>System.Management.Automation.PSCustomObject</T></TN><MS><PR N="Record"><AV>Preparing modules for first use.</AV><Nil /></PR></MS></Obj>"#,
            r#"<S S="Error">Get-Item : Cannot find path 'C:\nope' because it does not exist._x000D__x000A_</S><S S="Error">At line:2 char:1_x000D__x000A_</S>"#,
            r#"<S S="Error"> _x000D__x000A_</S>"#,
        );
        assert_eq!(read(&["#< CLIXML", &objs]), ["Get-Item : Cannot find path 'C:\\nope' because it does not exist.", "At line:2 char:1"]);
    }

    #[test]
    fn warnings_verbose_and_debug_are_marked_as_a_console_marks_them() {
        let objs = format!(
            "{OBJS}{}</Objs>",
            r#"<S S="warning">careful &lt;a&gt; &amp; _x005F_x_ b</S><S S="verbose">said_x000A_more</S><S S="debug">dbg</S>"#
        );
        assert_eq!(read(&["#< CLIXML", &objs]), ["警告: careful <a> & _x_ b", "詳細: said", "more", "デバッグ: dbg"]);
    }

    #[test]
    fn information_records_go_nested_objects_and_all() {
        // Write-Host: the text is on the standard output too.
        let objs = format!(
            "{OBJS}{}{}</Objs>",
            r#"<Obj S="information" RefId="1"><TNRef RefId="1" /><ToString>host-line</ToString><Props><Obj N="MessageData" RefId="2"><S N="Message">host-line</S></Obj><Obj N="Tags" RefId="5"><TNRef RefId="3" /><LST /></Obj><S N="User">pc\me</S></Props></Obj>"#,
            r#"<S S="Error">after_x000D__x000A_</S>"#
        );
        assert_eq!(read(&["#< CLIXML", &objs]), ["after"]);
    }

    #[test]
    fn text_written_straight_to_the_error_output_stays() {
        let objs = format!(r#"{OBJS}<S S="Error">boom</S></Objs>"#);
        assert_eq!(read(&["#< CLIXML", "raw-err", "a < b", &objs, "after the end"]), ["raw-err", "a < b", "boom", "after the end"]);
    }

    #[test]
    fn a_line_cut_short_by_a_kill_still_gives_what_it_has() {
        let objs = format!(r#"{OBJS}<S S="Error">first_x000D__x000A_</S><S S="Error">half a li"#);
        assert_eq!(read(&["#< CLIXML", &objs]), ["first", "half a li"]);
        let objs = format!(r#"{OBJS}<S S="Error">first</S><Obj S="progress" RefId="0"><TN"#);
        assert_eq!(read(&["#< CLIXML", &objs]), ["first"]);
    }

    #[test]
    fn escapes_cover_numeric_references_and_surrogate_pairs() {
        assert_eq!(unescape(&entities("&#65;&#x42;_xD83D__xDE00_ _x12_ x_x")), "AB😀 _x12_ x_x");
        assert_eq!(entities("a & b &bogus; &lt"), "a & b &bogus; &lt");
    }
}
