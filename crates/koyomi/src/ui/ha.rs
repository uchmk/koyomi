//! The status bar's HA words: with `[standby]` or `[primary]` in koyomi.toml, which PC runs the
//! schedules, in the words of the Web UI's `#ha` chip, and when the standby last took them over
//! or gave them back. The thread asks the engine (`Handover { release: false }`, the `"ha"` of
//! `GET /health`) every `HA_POLL`; without either section the engine says nothing and so does the
//! bar.

/// How the words are coloured: the Web UI's `run`, `paused` and `ok`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// The standby runs the schedules.
    Run,
    /// Watching, or the primary leaves them to the standby or asks it.
    Wait,
    /// The primary runs them and the standby waits.
    Calm,
}

impl Tone {
    pub fn color(self) -> egui::Color32 {
        let c = ito_theme::colors();
        match self {
            Tone::Run => c.run,
            Tone::Wait => c.wait,
            Tone::Calm => c.dim,
        }
    }
}

/// The words for the engine's `"ha"`, and their tone. `None` for a role it does not know.
pub fn words(t: &ito_i18n::Table, ha: &serde_json::Value) -> Option<(String, Tone)> {
    let s = |k: &str| ha[k].as_str().unwrap_or_default().to_owned();
    // The time after the words, when the engine gives one.
    let at = |line: String, key: &str, field: &str| match ha[field].as_str() {
        Some(at) => format!("{line}{}", t.trf(key, &[at])),
        None => line,
    };
    match ha["role"].as_str()? {
        "standby" if ha["active"].as_bool() == Some(true) => {
            Some((at(t.trf("status.ha_took", &[&s("watch")]), "status.ha_since", "since"), Tone::Run))
        }
        "standby" => Some((at(t.trf("status.ha_watch", &[&s("watch")]), "status.ha_back", "back"), Tone::Wait)),
        "primary" => Some(match ha["hold"].as_str() {
            Some("held") => (t.trf("status.ha_held", &[&s("standby")]), Tone::Wait),
            Some("asking") => (t.trf("status.ha_asking", &[&s("standby")]), Tone::Wait),
            _ => (t.trf("status.ha_standing", &[&s("standby")]), Tone::Calm),
        }),
        _ => None,
    }
}

impl super::Browser {
    /// The status bar's HA words, while the engine answers and has `[standby]` or `[primary]`.
    pub fn ha_fact(&self, t: &ito_i18n::Table) -> Option<(String, Tone)> {
        if self.down.is_some() || self.starting {
            return None;
        }
        words(t, self.ha.as_ref()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ja(ha: serde_json::Value) -> Option<(String, Tone)> {
        words(&crate::i18n::table("ja").0, &ha)
    }

    /// The same words as the Web UI's chip, with the time the engine gives after them.
    #[test]
    fn each_role_says_who_runs_the_schedules() {
        let took = json!({ "role": "standby", "watch": "main-pc", "active": true, "ready": false, "since": "2026-11-01 09:30" });
        assert_eq!(ja(took), Some(("main-pc から引き継いで実行しています（2026-11-01 09:30 から）".to_owned(), Tone::Run)));
        let watching = json!({ "role": "standby", "watch": "main-pc", "active": false, "ready": false });
        assert_eq!(ja(watching), Some(("待機中: main-pc を監視しています".to_owned(), Tone::Wait)));
        let back = json!({ "role": "standby", "watch": "main-pc", "active": false, "back": "2026-10-31 18:05" });
        assert_eq!(ja(back), Some(("待機中: main-pc を監視しています（2026-10-31 18:05 に戻しました）".to_owned(), Tone::Wait)));
        let held = json!({ "role": "primary", "standby": "sub-pc", "hold": "held" });
        assert_eq!(ja(held), Some(("sub-pc が引き継いでいます".to_owned(), Tone::Wait)));
        let asking = json!({ "role": "primary", "standby": "sub-pc", "hold": "asking" });
        assert_eq!(ja(asking), Some(("sub-pc に確認しています".to_owned(), Tone::Wait)));
        let running = json!({ "role": "primary", "standby": "sub-pc", "hold": "running" });
        assert_eq!(ja(running), Some(("sub-pc が待機しています".to_owned(), Tone::Calm)));
        assert_eq!(ja(json!({ "role": "other" })), None);
        assert_eq!(ja(json!({})), None);
        let (en, _) = crate::i18n::table("en");
        assert_eq!(words(&en, &json!({ "role": "primary", "standby": "sub-pc" })).map(|w| w.0).as_deref(), Some("sub-pc is standing by"));
    }
}
