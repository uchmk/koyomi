//! What a person has stopped by hand: the nets on hold, and whether the whole
//! schedule is paused. Kept in `state_dir("koyomi")/held.toml`; the engine is
//! its one writer. A held net's times, and every time while paused, are written
//! down as ⏸ 保留 runs that did not start, so nothing is skipped in silence.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The contents of `held.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Held {
    /// `koyomi pause`: no time starts anything until `koyomi resume`. Runs by hand still go.
    #[serde(default)]
    pub paused: bool,
    /// The ids (`経理/月次締め`) on hold: their times do not run, and neither does `koyomi run`.
    #[serde(default)]
    pub held: Vec<String>,
    /// `koyomi shift`: times of nets that start at another time this once.
    #[serde(default, rename = "shift", skip_serializing_if = "Vec::is_empty")]
    pub shifts: Vec<Shift>,
}

/// One time moved for once: the net's time `from` starts at `to` instead (the definition is not changed).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shift {
    pub id: String,
    pub from: chrono::NaiveDateTime,
    pub to: chrono::NaiveDateTime,
}

impl Held {
    pub fn parse(text: &str) -> Result<Held, String> {
        toml::from_str(text).map_err(|e: toml::de::Error| e.message().lines().next().unwrap_or_default().to_owned())
    }

    /// What is kept at `path`: nothing held when there is no file yet. A file that does not
    /// read is an error: writing over it would drop every hold without a word.
    pub fn load(path: &Path) -> Result<Held, String> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Held::default()),
            Err(e) => return Err(e.to_string()),
        };
        let text = String::from_utf8(bytes).map_err(|_| "UTF-8 の文字として読めない".to_owned())?;
        Held::parse(text.strip_prefix('\u{feff}').unwrap_or(&text))
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    pub fn is_held(&self, id: &str) -> bool {
        self.held.iter().any(|h| h == id)
    }

    /// Puts `id` on hold; `false` when it already was.
    pub fn hold(&mut self, id: &str) -> bool {
        if self.is_held(id) {
            return false;
        }
        self.held.push(id.to_owned());
        self.held.sort();
        true
    }

    /// Moves the time `from` of `id` to `to`; an earlier move of the same time is replaced.
    pub fn shift(&mut self, id: &str, from: chrono::NaiveDateTime, to: chrono::NaiveDateTime) {
        self.unshift(id, from);
        self.shifts.push(Shift { id: id.to_owned(), from, to });
        self.shifts.sort_by(|a, b| (a.from, &a.id).cmp(&(b.from, &b.id)));
    }

    /// Puts the time `from` of `id` back; `false` when it was not moved.
    pub fn unshift(&mut self, id: &str, from: chrono::NaiveDateTime) -> bool {
        let before = self.shifts.len();
        self.shifts.retain(|s| !(s.id == id && s.from == from));
        self.shifts.len() != before
    }

    /// Whether the time `from` of `id` is moved.
    pub fn is_shifted(&self, id: &str, from: chrono::NaiveDateTime) -> bool {
        self.shifts.iter().any(|s| s.id == id && s.from == from)
    }

    /// Takes `id` off hold; `false` when it was not on it.
    pub fn release(&mut self, id: &str) -> bool {
        let before = self.held.len();
        self.held.retain(|h| h != id);
        self.held.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shift_is_kept_and_replaced_by_the_same_time() {
        let at = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        let mut h = Held::default();
        h.shift("g/a", at("2026-10-30 09:00"), at("2026-10-30 10:00"));
        h.shift("g/a", at("2026-10-30 09:00"), at("2026-10-30 11:00"));
        assert_eq!(h.shifts.len(), 1);
        assert_eq!(Held::parse(&h.to_toml()).unwrap(), h);
        assert!(h.is_shifted("g/a", at("2026-10-30 09:00")) && !h.is_shifted("g/b", at("2026-10-30 09:00")));
        assert!(h.unshift("g/a", at("2026-10-30 09:00")) && !h.unshift("g/a", at("2026-10-30 09:00")));
    }

    #[test]
    fn a_hold_round_trips_and_a_release_takes_it_off() {
        let mut h = Held::default();
        assert!(h.hold("g/b"));
        assert!(h.hold("g/a"));
        assert!(!h.hold("g/a"));
        h.paused = true;
        let back = Held::parse(&h.to_toml()).unwrap();
        assert_eq!(back, h);
        assert_eq!(back.held, vec!["g/a", "g/b"]);
        assert!(back.is_held("g/a") && back.paused);
        let mut h = back;
        assert!(h.release("g/a"));
        assert!(!h.release("g/a"));
        assert!(!h.is_held("g/a") && h.is_held("g/b"));
        assert!(Held::parse("held = 1").is_err());
        assert_eq!(Held::parse("").unwrap(), Held::default());
        let gone = std::env::temp_dir().join(format!("koyomi-held-none-{}", std::process::id()));
        assert_eq!(Held::load(&gone).unwrap(), Held::default());
    }
}
