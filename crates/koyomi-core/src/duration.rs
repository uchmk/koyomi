//! A length of time as a definition writes it: `30m`, `1h30m`, `90s`.
//!
//! Hours, minutes and seconds, each at most once and in that order, every
//! number with its unit. A bare number is refused: `timeout = 30` could mean
//! seconds or minutes, and the wrong guess hangs a night job or cuts it off.

/// The seconds `text` says, or why it does not read.
pub fn parse(text: &str) -> Result<u64, String> {
    let bad = || format!("{text}: 30m・1h30m・90s のように書く（h・m・s、この順）");
    let t = text.trim();
    if t.is_empty() {
        return Err(bad());
    }
    let mut secs = 0u64;
    let mut number = String::new();
    // The unit that came last, as its rank: h 3, m 2, s 1.
    let mut last = 4;
    for ch in t.chars() {
        if ch.is_ascii_digit() {
            number.push(ch);
            continue;
        }
        let (rank, per) = match ch {
            'h' => (3, 3600),
            'm' => (2, 60),
            's' => (1, 1),
            _ => return Err(bad()),
        };
        if number.is_empty() || rank >= last {
            return Err(bad());
        }
        let n: u64 = number.parse().map_err(|_| bad())?;
        secs = n.checked_mul(per).and_then(|x| secs.checked_add(x)).ok_or_else(bad)?;
        number.clear();
        last = rank;
    }
    if !number.is_empty() {
        return Err(bad());
    }
    if secs == 0 {
        return Err(format!("{text}: 0 より長くする"));
    }
    Ok(secs)
}

/// The shortest text that [`parse`] reads back to `secs`: `30m`, `1h30m`, `90s` stays `1m30s`.
pub fn text(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let mut out = String::new();
    for (n, unit) in [(h, 'h'), (m, 'm'), (s, 's')] {
        if n > 0 {
            out.push_str(&format!("{n}{unit}"));
        }
    }
    if out.is_empty() {
        out.push_str("0s");
    }
    out
}

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
            assert!(parse(bad).unwrap_err().contains("30m・1h30m・90s のように書く"), "{bad}");
        }
        assert_eq!(parse("0m").unwrap_err(), "0m: 0 より長くする");
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
