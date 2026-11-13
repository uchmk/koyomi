//! What a person has stopped by hand: the nets and the jobs on hold, and whether the whole
//! schedule is paused. Kept in `state_dir("koyomi")/held.toml`; the engine is
//! its one writer. A held net's times, and every time while paused, are written
//! down as ⏸ 保留 runs that did not start, so nothing is skipped in silence.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::msg;

/// The contents of `held.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Held {
    /// `koyomi pause`: no time starts anything until `koyomi resume`. Runs by hand still go.
    #[serde(default)]
    pub paused: bool,
    /// The ids (`経理/月次締め`) on hold: their times do not run, and neither does `koyomi run`.
    #[serde(default)]
    pub held: Vec<String>,
    /// `koyomi hold <net> <job>`: a run that comes to the job waits there as ⏸ 保留 (not started,
    /// the jobs after it wait too) until the job is released, then goes on as the same run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jobs: Vec<HeldJob>,
    /// `koyomi shift`: times of nets that start at another time this once.
    #[serde(default, rename = "shift", skip_serializing_if = "Vec::is_empty")]
    pub shifts: Vec<Shift>,
    /// `koyomi skip-next`: times of nets that do not start this once (written down as skipped).
    #[serde(default, rename = "skip", skip_serializing_if = "Vec::is_empty")]
    pub skips: Vec<Skip>,
}

/// One job on hold: `job` of the net `net`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HeldJob {
    pub net: String,
    pub job: String,
}

/// One time moved for once: the net's time `from` starts at `to` instead (the definition is not changed).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shift {
    pub id: String,
    pub from: chrono::NaiveDateTime,
    pub to: chrono::NaiveDateTime,
}

/// One time skipped for once: the net's time `at` does not start (the definition is not changed).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skip {
    pub id: String,
    pub at: chrono::NaiveDateTime,
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
            Err(e) => return Err(msg::io_words(&e)),
        };
        let text = String::from_utf8(bytes).map_err(|_| msg::line(msg::STATE_NOT_UTF8, &[&path.display().to_string()]))?;
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

    /// Whether the job `job` of `net` is on hold, or the child jobnet's station it is in
    /// (`sub` for `sub/x`): holding a `kind = "jobnet"` part holds the whole call.
    pub fn is_job_held(&self, net: &str, job: &str) -> bool {
        self.job_held_by(net, job).is_some()
    }

    /// The held job that holds `job` of `net`: itself, or the child jobnet's station it is in.
    pub fn job_held_by(&self, net: &str, job: &str) -> Option<&str> {
        self.jobs
            .iter()
            .find(|h| h.net == net && (h.job == job || job.strip_prefix(h.job.as_str()).is_some_and(|r| r.starts_with('/'))))
            .map(|h| h.job.as_str())
    }

    /// The jobs of `net` on hold.
    pub fn jobs_of<'a>(&'a self, net: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.jobs.iter().filter(move |h| h.net == net).map(|h| h.job.as_str())
    }

    /// Puts the job `job` of `net` on hold; `false` when it already was.
    pub fn hold_job(&mut self, net: &str, job: &str) -> bool {
        let h = HeldJob { net: net.to_owned(), job: job.to_owned() };
        if self.jobs.contains(&h) {
            return false;
        }
        self.jobs.push(h);
        self.jobs.sort();
        true
    }

    /// Takes the job `job` of `net` off hold; `false` when it was not on it.
    pub fn release_job(&mut self, net: &str, job: &str) -> bool {
        let before = self.jobs.len();
        self.jobs.retain(|h| !(h.net == net && h.job == job));
        self.jobs.len() != before
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

    /// Skips the time `at` of `id` this once; `false` when it already was.
    pub fn skip(&mut self, id: &str, at: chrono::NaiveDateTime) -> bool {
        if self.is_skipped(id, at) {
            return false;
        }
        self.skips.push(Skip { id: id.to_owned(), at });
        self.skips.sort_by(|a, b| (a.at, &a.id).cmp(&(b.at, &b.id)));
        true
    }

    /// Whether the time `at` of `id` is skipped.
    pub fn is_skipped(&self, id: &str, at: chrono::NaiveDateTime) -> bool {
        self.skips.iter().any(|s| s.id == id && s.at == at)
    }

    /// The times of `id` that are skipped, earliest first.
    pub fn skips_of<'a>(&'a self, id: &'a str) -> impl Iterator<Item = chrono::NaiveDateTime> + 'a {
        self.skips.iter().filter(move |s| s.id == id).map(|s| s.at)
    }

    /// Uses up the skip of the time `at` of `id`, with any of its skips at or before `at` (they
    /// are past); `true` when `at` itself was skipped.
    pub fn take_skip(&mut self, id: &str, at: chrono::NaiveDateTime) -> bool {
        let hit = self.is_skipped(id, at);
        self.skips.retain(|s| !(s.id == id && s.at <= at));
        hit
    }

    /// Puts the skipped times of `id` back; `false` when there were none.
    pub fn unskip(&mut self, id: &str) -> bool {
        let before = self.skips.len();
        self.skips.retain(|s| s.id != id);
        self.skips.len() != before
    }

    /// Takes `id` off hold; `false` when it was not on it.
    pub fn release(&mut self, id: &str) -> bool {
        let before = self.held.len();
        self.held.retain(|h| h != id);
        self.held.len() != before
    }

    /// Carries the hold, the held jobs and the moved and skipped times of `from` over to `to` (the net's new name), or drops
    /// them when `to` is `None` (the net is gone); `false` when there was nothing of `from`.
    pub fn carry(&mut self, from: &str, to: Option<&str>) -> bool {
        let held = self.release(from);
        if let (true, Some(to)) = (held, to) {
            self.hold(to);
        }
        let before = self.shifts.len();
        match to {
            Some(to) => self.shifts.iter_mut().filter(|s| s.id == from).for_each(|s| s.id = to.to_owned()),
            None => self.shifts.retain(|s| s.id != from),
        }
        let moved = self.shifts.iter().any(|s| to.is_some_and(|t| s.id == t));
        let skipped = self.skips.iter().any(|s| s.id == from);
        match to {
            Some(to) => self.skips.iter_mut().filter(|s| s.id == from).for_each(|s| s.id = to.to_owned()),
            None => self.skips.retain(|s| s.id != from),
        }
        let jobs = self.jobs.iter().any(|h| h.net == from);
        match to {
            Some(to) => self.jobs.iter_mut().filter(|h| h.net == from).for_each(|h| h.net = to.to_owned()),
            None => self.jobs.retain(|h| h.net != from),
        }
        self.jobs.sort();
        held || moved || skipped || jobs || self.shifts.len() != before
    }
}

/// The next time of `whens` after `now` that `taken` does not claim (skipped or moved already):
/// what `koyomi skip-next` skips. `None` when there is no such time ahead.
pub fn next_open(
    whens: &[crate::when::When],
    cal: Option<&crate::calendar::Calendar>,
    now: chrono::NaiveDateTime,
    taken: impl Fn(chrono::NaiveDateTime) -> bool,
) -> Option<chrono::NaiveDateTime> {
    let mut cursor = now;
    for _ in 0..1000 {
        let t = *crate::when::next_times_in(whens, cursor, 1, cal).first()?;
        if !taken(t) {
            return Some(t);
        }
        cursor = t;
    }
    None
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
    fn a_skip_is_kept_used_up_and_carried() {
        let at = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        let mut h = Held::default();
        assert!(h.skip("g/a", at("2026-10-31 09:00")));
        assert!(!h.skip("g/a", at("2026-10-31 09:00")));
        assert!(h.skip("g/a", at("2026-11-01 09:00")));
        assert!(h.to_toml().contains("[[skip]]"), "{}", h.to_toml());
        assert_eq!(Held::parse(&h.to_toml()).unwrap(), h);
        assert!(!h.take_skip("g/b", at("2026-10-31 09:00")));
        assert!(!h.take_skip("g/a", at("2026-10-31 10:00")), "a time not skipped runs, and the past skip goes");
        assert_eq!(h.skips_of("g/a").collect::<Vec<_>>(), [at("2026-11-01 09:00")]);
        assert!(h.carry("g/a", Some("g/c")));
        assert!(h.take_skip("g/c", at("2026-11-01 09:00")) && h.skips.is_empty());
        h.skip("g/c", at("2026-11-02 09:00"));
        assert!(h.unskip("g/c") && !h.unskip("g/c"));
        assert!(!Held::default().to_toml().contains("skip"));
    }

    #[test]
    fn a_job_hold_round_trips_and_an_old_file_still_reads() {
        let old = Held::parse(
            "paused = false
held = [\"g/a\"]
",
        )
        .unwrap();
        assert!(old.jobs.is_empty() && old.is_held("g/a"));
        assert!(!old.to_toml().contains("jobs"), "nothing is written for no job: {}", old.to_toml());
        let mut h = old;
        assert!(h.hold_job("g/b", "y"));
        assert!(h.hold_job("g/b", "sub"));
        assert!(!h.hold_job("g/b", "y"));
        let back = Held::parse(&h.to_toml()).unwrap();
        assert_eq!(back, h);
        assert!(back.is_job_held("g/b", "y") && !back.is_job_held("g/a", "y") && !back.is_job_held("g/b", "yy"));
        assert!(back.is_job_held("g/b", "sub/x"), "a job in a held call is held");
        assert!(!back.is_job_held("g/b", "subway"));
        assert_eq!(back.jobs_of("g/b").collect::<Vec<_>>(), ["sub", "y"]);
        let mut h = back;
        assert!(h.carry("g/b", Some("g/c")));
        assert!(h.is_job_held("g/c", "y") && !h.is_job_held("g/b", "y"));
        assert!(h.release_job("g/c", "y") && !h.release_job("g/c", "y"));
        assert!(h.carry("g/c", None));
        assert!(h.jobs.is_empty());
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
