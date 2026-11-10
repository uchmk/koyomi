//! The app's own words, in English and Japanese. The language is the one in
//! common.toml (`auto`: the OS's), shared by every uchmk app; the settings
//! screen's own words are `ito_prefs::Words`.

use ito_i18n::Table;

const TABLES: &[(&str, &str)] = &[("en", include_str!("../lang/en.toml")), ("ja", include_str!("../lang/ja.toml"))];

/// The code to use: common.toml's `language`, else the OS's, else English.
pub fn resolve(common: Option<&str>) -> String {
    ito_i18n::resolve(None, common, ito_common::os_language().as_deref(), &["en", "ja"])
}

/// What the engine answers in: common.toml's language when one is named, otherwise Japanese.
/// A service often runs with `LANG=C`, and its refusals are read in a window elsewhere.
pub fn engine_language(common: Option<&str>) -> &'static str {
    match common {
        Some("en") => "en",
        _ => "ja",
    }
}

/// The table for `code`, and what in the files does not read.
pub fn table(code: &str) -> (Table, Vec<String>) {
    Table::new(TABLES, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both files read, and Japanese lays over English.
    #[test]
    fn the_tables_read() {
        let (en, errs) = table("en");
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(en.tr("app.title"), "koyomi");
        assert_eq!(en.tr("standing.unconfirmed"), "not approved");
        let (ja, errs) = table("ja");
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(ja.tr("app.title"), "koyomi");
        assert_eq!(ja.tr("standing.unconfirmed"), "未確認");
    }

    #[test]
    fn the_engine_answers_in_japanese_unless_common_names_english() {
        assert_eq!(engine_language(None), "ja");
        assert_eq!(engine_language(Some("auto")), "ja");
        assert_eq!(engine_language(Some("fr")), "ja");
        assert_eq!(engine_language(Some("ja")), "ja");
        assert_eq!(engine_language(Some("en")), "en");
    }
}
