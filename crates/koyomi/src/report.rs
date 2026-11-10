//! `koyomi report`: a month's success rate, usual time and worst jobs per net, as lines for the
//! terminal and the window. The figures come from [`koyomi_core::report`].

use ito_i18n::Table;
use koyomi_core::report::Row;

use crate::show;

/// `92%`, or `-` when no run passed or failed.
pub fn rate(r: &Row) -> String {
    r.rate.map_or_else(|| "-".to_owned(), |p| format!("{p}%"))
}

/// `1分30秒`, or `-` when no run ended 正常.
pub fn average(t: &Table, r: &Row) -> String {
    r.average.map_or_else(|| "-".to_owned(), |s| show::duration(t, chrono::Duration::seconds(s)))
}

/// `a×3 b×1`: the jobs that failed most; empty when none failed.
pub fn worst(r: &Row) -> String {
    r.worst.iter().map(|f| format!("{}×{}", f.id, f.times)).collect::<Vec<_>>().join(" ")
}

/// One line a net: `group/name  runs 12  rate 92%  usual 1 min 30 s  failing: a×3`.
pub fn line(t: &Table, r: &Row) -> String {
    let mut s = t.trf("report.line", &[&r.jobnet, &r.runs.to_string(), &rate(r), &average(t, r)]);
    if !r.worst.is_empty() {
        s.push_str(&format!("  {}", t.trf("report.worst", &[&worst(r)])));
    }
    s
}

/// The lines of a report; one saying there is nothing when no run is over.
pub fn lines(t: &Table, rows: &[Row]) -> Vec<String> {
    if rows.is_empty() {
        return vec![t.tr("report.none").to_owned()];
    }
    rows.iter().map(|r| line(t, r)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use koyomi_core::report::Failing;

    #[test]
    fn a_line_names_the_rate_the_usual_time_and_the_jobs_that_failed() {
        let (t, _) = crate::i18n::table("ja");
        let mut r = Row {
            jobnet: "g/n".into(),
            title: "n".into(),
            runs: 12,
            passed: 11,
            failed: 1,
            other: 0,
            rate: Some(92),
            average: Some(90),
            worst: vec![Failing { id: "a".into(), times: 3 }],
        };
        assert_eq!(line(&t, &r), "g/n  12 回  成功 92%  いつもは 1分30秒  異常が多いジョブ: a×3");
        r.rate = None;
        r.average = None;
        r.worst.clear();
        assert_eq!(line(&t, &r), "g/n  12 回  成功 -  いつもは -");
        assert_eq!(lines(&t, &[]), ["この期間に終了した実行はありません。"]);
    }
}
