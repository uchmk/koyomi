//! `lock`: whether a run may start while other runs hold the same lock name.
//!
//! - `lock = "販売DB"` is exclusive: the run takes one of the lock's places. koyomi.toml's
//!   `[locks]` says how many there are (`"API枠" = 3`); a name not listed has 1, so a plain
//!   `lock` keeps two nets from ever running together.
//! - `lock = { name = "販売DB", shared = true }` only reads: any number of shared holders go
//!   together, and they take no place. Shared and exclusive holders never go together.
//!
//! Nothing waits for a lock. A run that cannot take it is refused (by hand) or recorded as
//! 🔒 止めた (a scheduled time), and that is told like any other stop. So readers that keep
//! coming cannot starve an exclusive net in a queue nobody sees: each time it is kept out
//! shows, with the lock and who holds it.

use crate::msg;

/// A run that holds the lock now.
#[derive(Clone, Copy, Debug)]
pub struct Holder<'a> {
    /// What the screen calls its net.
    pub title: &'a str,
    /// It holds the lock shared (`shared = true`).
    pub shared: bool,
}

/// Whether a run may take the lock `name` (`shared` or exclusive) while `holders` hold it.
/// `count` is the lock's places for exclusive holders (`[locks]`, 1 when not listed).
/// `Err` says why not, naming the lock, its mode and who holds it.
pub fn admit(name: &str, shared: bool, count: usize, holders: &[Holder]) -> Result<(), String> {
    let count = count.max(1);
    let (readers, writers): (Vec<&Holder>, Vec<&Holder>) = holders.iter().partition(|h| h.shared);
    if shared {
        if writers.is_empty() {
            return Ok(());
        }
        return Err(msg::note(msg::N_LOCK_SHARED_HELD, &[name, &who(&writers)]));
    }
    if !readers.is_empty() {
        return Err(msg::note(msg::N_LOCK_EXCLUSIVE_BY_SHARED, &[name, &who(&readers)]));
    }
    if writers.len() < count {
        return Ok(());
    }
    if count == 1 {
        Err(msg::note(msg::N_LOCK_EXCLUSIVE_BY_ONE, &[name, &who(&writers)]))
    } else {
        Err(msg::note(msg::N_LOCK_EXCLUSIVE_FULL, &[name, &count.to_string(), &who(&writers)]))
    }
}

/// The holders' titles, at most three, then how many more.
fn who(holders: &[&Holder]) -> String {
    const SHOWN: usize = 3;
    let names: Vec<&str> = holders.iter().take(SHOWN).map(|h| h.title).collect();
    let mut s = names.join(&msg::note(msg::N_LIST_SEP, &[]));
    if holders.len() > SHOWN {
        s.push(' ');
        s.push_str(&msg::note(msg::N_LOCK_AND_MORE, &[&(holders.len() - SHOWN).to_string()]));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(title: &str) -> Holder<'_> {
        Holder { title, shared: false }
    }
    fn r(title: &str) -> Holder<'_> {
        Holder { title, shared: true }
    }

    #[test]
    fn a_plain_lock_is_one_at_a_time_as_before() {
        assert_eq!(admit("db", false, 1, &[]), Ok(()));
        assert_eq!(admit("db", false, 1, &[w("夜間集計")]), Err("排他ロック「db」を、夜間集計 が使用しています。".to_owned()));
        // A count of 0 cannot come from koyomi.toml; it still means one place, never none.
        assert_eq!(admit("db", false, 0, &[]), Ok(()));
        assert!(admit("db", false, 0, &[w("a")]).is_err());
    }

    #[test]
    fn readers_go_together_but_never_with_a_writer() {
        assert_eq!(admit("販売DB", true, 1, &[]), Ok(()));
        assert_eq!(admit("販売DB", true, 1, &[r("売上照会"), r("在庫照会")]), Ok(()));
        assert_eq!(admit("販売DB", true, 1, &[w("月次締め")]), Err("共有ロック「販売DB」は、月次締め が排他で使用しています。".to_owned()));
        assert_eq!(
            admit("販売DB", false, 1, &[r("売上照会")]),
            Err("排他ロック「販売DB」は、売上照会 が共有で使用しています。".to_owned())
        );
        // Readers take no place: a lock of 3 places still keeps a writer out of readers.
        assert!(admit("販売DB", false, 3, &[r("a")]).is_err());
        // And a writer in one place of 3 still keeps readers out.
        assert!(admit("販売DB", true, 3, &[w("a")]).is_err());
    }

    #[test]
    fn a_counted_lock_lets_that_many_writers_in() {
        assert_eq!(admit("API枠", false, 3, &[w("a"), w("b")]), Ok(()));
        assert_eq!(
            admit("API枠", false, 3, &[w("a"), w("b"), w("c")]),
            Err("排他ロック「API枠」は、同時に 3 本まで使用できます。a、b、c が使用しています。".to_owned())
        );
    }

    #[test]
    fn many_holders_are_named_three_and_counted() {
        let hs = [r("a"), r("b"), r("c"), r("d"), r("e")];
        assert_eq!(admit("x", false, 1, &hs), Err("排他ロック「x」は、a、b、c ほか 2 件 が共有で使用しています。".to_owned()));
    }
}
