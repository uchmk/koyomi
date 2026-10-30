//! The words a beginner stumbles on, with a plain wording and a hover text
//! (`easy = true` in common.toml turns the plain wording on for every uchmk app).

use ito_glossary::{Glossary, Term};

pub const TERMS: &[Term] = &[
    Term {
        id: "jobnet",
        word: "jobnet",
        plain_en: "job flow",
        plain_ja: "仕事の流れ",
        tip_en: "Jobs and the order they run in, kept in one file. koyomi starts it at the times you set.",
        tip_ja: "仕事とその順番を 1 つのファイルにまとめたもの。決めた時刻に koyomi が流します。",
    },
    Term {
        id: "approve",
        word: "approve",
        plain_en: "check and allow",
        plain_ja: "確かめて許可",
        tip_en: "Read what the job net will run, then allow it. Nothing runs before this, and a changed file needs it again.",
        tip_ja: "流す中身を読んで許可すること。これまでは流れません。ファイルを変えるとまた必要です。",
    },
];

pub const GLOSSARY: Glossary = Glossary(TERMS);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_well_formed() {
        assert_eq!(ito_glossary::check(TERMS), Vec::<String>::new());
        assert!(GLOSSARY.find("jobnet").is_some() && GLOSSARY.find("approve").is_some());
    }

    #[test]
    fn the_label_is_the_usual_word_unless_easy() {
        assert_eq!(GLOSSARY.label("approve", "en", false, "Approve"), "Approve");
        assert_eq!(GLOSSARY.label("approve", "en", true, "Approve"), "check and allow (approve)");
        assert_eq!(GLOSSARY.label("approve", "ja", true, "承認"), "確かめて許可（approve）");
    }
}
