//! `jobnets/<group>/_group.toml`: what the group's nets share. In this
//! version only `calendar`, which a net without its own takes.

use std::path::Path;

use serde::Deserialize;

/// The `format` this version reads.
pub const FORMAT: i64 = 1;

/// One group's `_group.toml`, read; the default when the file is not there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Group {
    /// `calendar = "会社"` for every net of the group that names none.
    pub calendar: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    format: Option<i64>,
    calendar: Option<String>,
}

/// Reads a `_group.toml`'s text.
pub fn parse(text: &str) -> Result<Group, String> {
    let raw: Raw = toml::from_str(text).map_err(|e| {
        let m = e.message();
        match m.strip_prefix("unknown field `").and_then(|r| r.split('`').next()) {
            Some(f) => format!("知らない欄 {f}"),
            None => format!("TOML の書き方が違う: {m}"),
        }
    })?;
    match raw.format {
        None => return Err(format!("format = {FORMAT} が要る")),
        Some(f) if f != FORMAT => return Err(format!("この版が読める format は {FORMAT} だけ")),
        Some(_) => {}
    }
    if let Some(c) = &raw.calendar {
        if !crate::calendar::valid_name(c) {
            return Err("calendar は calendars/<名前>.toml の <名前>（例: calendar = \"会社\"）".to_owned());
        }
    }
    Ok(Group { calendar: raw.calendar })
}

/// Reads `<group folder>/_group.toml`; no file is the default.
pub fn load(path: &Path) -> Result<Group, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Group::default()),
        Err(e) => Err(format!("読めない: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_file_names_the_calendar_or_nothing() {
        assert_eq!(parse("format = 1\ncalendar = \"会社\"\n").unwrap().calendar.as_deref(), Some("会社"));
        assert_eq!(parse("format = 1\n").unwrap(), Group::default());
        assert_eq!(parse("calendar = \"会社\"\n").unwrap_err(), "format = 1 が要る");
        assert_eq!(parse("format = 2\n").unwrap_err(), "この版が読める format は 1 だけ");
        assert_eq!(parse("format = 1\nx = 1\n").unwrap_err(), "知らない欄 x");
        assert!(parse("format = 1\ncalendar = 1\n").unwrap_err().starts_with("TOML の書き方が違う"));
        assert_eq!(
            parse("format = 1\ncalendar = \"a/b\"\n").unwrap_err(),
            "calendar は calendars/<名前>.toml の <名前>（例: calendar = \"会社\"）"
        );
        assert_eq!(load(Path::new("Z:/koyomi-no-such-dir/_group.toml")).unwrap(), Group::default());
    }
}
