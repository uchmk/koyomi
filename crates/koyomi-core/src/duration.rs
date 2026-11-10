//! A length of time as a definition writes it: `30m`, `1h30m`, `90s`.
//!
//! Hours, minutes and seconds, each at most once and in that order, every
//! number with its unit. A bare number is refused: `timeout = 30` could mean
//! seconds or minutes, and the wrong guess hangs a night job or cuts it off.

use crate::msg;

/// The seconds `text` says, or why it does not read. Zero is refused here:
/// every length a definition writes (`timeout`, `wait`, `settle`) must pass.
pub fn parse(text: &str) -> Result<u64, String> {
    // ito's reason is plain Japanese whatever the language; the catalogue's action already names the form.
    match ito_common::text::parse_duration(text).map_err(|_| msg::line(msg::DURATION_UNREADABLE, &[text]))? {
        0 => Err(msg::line(msg::DURATION_ZERO, &[text])),
        secs => Ok(secs),
    }
}

/// The shortest text that [`parse`] reads back to `secs`: `30m`, `1h30m`, `90s` stays `1m30s`.
pub use ito_common::text::duration_text as text;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hours_minutes_and_seconds_read_in_order() {
        assert_eq!(parse("30m"), Ok(1800));
        assert_eq!(parse("1h30m"), Ok(5400));
        assert_eq!(parse("90s"), Ok(90));
        assert_eq!(parse("1h30m10s"), Ok(5410));
        assert_eq!(parse(" 2h "), Ok(7200));
    }

    #[test]
    fn a_bare_number_a_wrong_order_and_zero_are_refused() {
        for bad in ["30", "", "m", "30x", "30m1h", "5m5m", "1.5h", "30 m"] {
            let e = parse(bad).unwrap_err();
            assert!(msg::DURATION_UNREADABLE.is_in(&e) && e.contains("30m、1h30m、90s のように"), "{bad}: {e}");
        }
        let zero = parse("0m").unwrap_err();
        assert!(msg::DURATION_ZERO.is_in(&zero) && zero.contains("0 より長い時間を指定してください"), "{zero}");
    }

    #[test]
    fn the_text_reads_back_to_the_same_seconds() {
        for secs in [1, 59, 60, 90, 1800, 3600, 5400, 5410, 90000] {
            assert_eq!(parse(&text(secs)), Ok(secs), "{secs}");
        }
        assert_eq!(text(1800), "30m");
        assert_eq!(text(5400), "1h30m");
    }
}
