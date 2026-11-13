//! `DirList` and `DirPlaces`: the engine lists a folder of its own PC for the window's picker (参照… when the window is
//! on another PC's engine). Nothing is written, and a listing is not written to the operations record.
//!
//! A listing blocks on a disk, so it runs on a slot's thread (`slots`) and the read itself on an inner thread, so a slow
//! disk cannot hold the slot longer than [`SLOW`]. At most [`SLOTS`] listings are being answered at once; counting the
//! reads a slow disk still holds after their answer went out, at most [`LINGER_MAX`] reads exist. The count is reserved on
//! the engine loop (`Linger::try_take`) before any thread exists, and every path gives it back by dropping its [`Hold`].
//!
//! What is refused: in the engine loop, lexically (`db_test::path_ok`, no disk access): a relative path, `\\server\share`,
//! `\\?\` and `\\.\`. Then ito refuses on the read thread (`ListOptions::strict`): device names, bad characters, a drive
//! mapped to a share, a `subst` drive, and a link in the path; a link entry in a listing is shown but not followed.
//! Not guarded (ito's own doc): cloud placeholders (OneDrive), a network file system mounted on Linux, and someone
//! swapping a folder for a link while the checks run.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use koyomi_core::msg::{self, line};

use super::db_test;
use super::slots;
use crate::proto::PlacesReply;

/// How many listings are answered at once.
pub const SLOTS: usize = 2;

/// The most entries one listing carries (the picker is told when there are more).
pub const MAX_ENTRIES: usize = 2000;

/// How long a listing is waited for before it is answered "slow". A target, not a promise: the slot's result reaches the
/// door only at the next `tick_peeks`, after the loop's other work, and the door gives up at `ito_ipc::door::ANSWER_WAIT`
/// (5 s); 3 s leaves room for a busy loop. When the door gives up first, the window gets the door's own error.
pub const SLOW: Duration = Duration::from_secs(3);

/// The most reads that exist at once, counting the ones a slow disk still holds after their answer went out.
pub const LINGER_MAX: usize = 4;

/// The longest path quoted in a message, in characters.
const SHORT_MAX: usize = 200;

/// The longest path a listing takes, in characters.
const PATH_MAX: usize = 4096;

/// How a folder is listed: the path as the engine reads it and the listing, or the engine's words for why not. Injected
/// so tests never read a real disk.
pub type Lister = Arc<dyn Fn(&str) -> Result<ito_filepick::Listing, String> + Send + Sync>;

/// The lister as it really reads this PC's disk.
pub fn real() -> Lister {
    Arc::new(|path| {
        ito_filepick::list_local_with(path, &ito_filepick::ListOptions::strict(MAX_ENTRIES)).map_err(|e| e.words(&msg::language()))
    })
}

/// What the operations record would keep of a result (it never does for a listing, but `Slots::new` needs one): the ID of
/// an error, or `ok`.
pub fn detail(result: &Result<String, String>) -> String {
    db_test::detail(result)
}

/// How many reads exist, as a count the engine loop reserves from.
#[derive(Clone, Default)]
pub struct Linger(Arc<AtomicUsize>);

/// One reserved read. Dropping it counts the read down; it is moved into the work and from there into the inner thread, so
/// whichever way the read ends (it returns, it panics, the slot refuses it, a spawn fails) the count is given back.
pub struct Hold(Arc<AtomicUsize>);

impl Linger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserves a read, atomically: `None` when [`LINGER_MAX`] reads exist.
    pub fn try_take(&self) -> Option<Hold> {
        self.0.try_update(Ordering::AcqRel, Ordering::Acquire, |n| (n < LINGER_MAX).then_some(n + 1)).ok()?;
        Some(Hold(self.0.clone()))
    }

    /// How many reads exist now.
    #[cfg(test)]
    pub fn count(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The engine's path style and its places (drives and home), with ito's strict options. Instant: it reads the drive
/// names (`QueryDosDeviceW`) and checks the home folder's path; it does not open a drive.
pub fn places() -> Result<String, String> {
    let reply =
        PlacesReply { style: ito_filepick::local_style(), places: ito_filepick::local_places_with(&ito_filepick::ListOptions::strict(0)) };
    serde_json::to_string(&reply).map_err(|e| e.to_string())
}

/// Checks the path of a listing and gives it back exactly as sent (no trim, no `canonicalize`), or a coded line.
pub fn prepare(path: &str) -> Result<String, String> {
    if path.is_empty() || path.chars().count() > PATH_MAX || path.contains('\0') || !db_test::path_ok(path) {
        return Err(line(msg::DIR_LIST_PATH_BAD, &[]));
    }
    Ok(path.to_owned())
}

/// The first characters of a path, for a message or the record.
pub fn short(path: &str) -> String {
    path.chars().take(SHORT_MAX).collect()
}

/// The work of one listing, run on a slot's thread. The read runs on an inner thread that owns `hold`; if it takes longer
/// than `slow` the work answers `DIR_LIST_SLOW` and lets the read go on alone (its late result is lost). The cancel flag is
/// ignored: a listing cannot be cancelled, and the slot is free at `slow` at the latest.
pub fn work(path: String, lister: Lister, hold: Hold, slow: Duration) -> slots::Work {
    Box::new(move |_cancel: &AtomicBool| {
        let (tx, rx) = mpsc::channel();
        let shown = short(&path);
        let spawned = std::thread::Builder::new().name("koyomi-dir-list-io".into()).spawn(move || {
            let result = lister(&path);
            // Given back as soon as the read ends, before the answer is sent.
            drop(hold);
            let _ = tx.send(result);
        });
        if spawned.is_err() {
            return Err(line(msg::DIR_LIST_STOPPED, &[]));
        }
        match rx.recv_timeout(slow) {
            Ok(Ok(listing)) => serde_json::to_string(&listing).map_err(|e| line(msg::DIR_LIST_FAILED, &[&shown, &e.to_string()])),
            Ok(Err(words)) => Err(line(msg::DIR_LIST_FAILED, &[&shown, &words])),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(line(msg::DIR_LIST_SLOW, &[&shown])),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(line(msg::DIR_LIST_STOPPED, &[])),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ito_filepick::{Entry, Listing};

    fn lister_of(entries: Vec<Entry>) -> Lister {
        Arc::new(move |_| Ok(Listing::new(entries.clone())))
    }

    fn run(work: slots::Work) -> Result<String, String> {
        work(&AtomicBool::new(false))
    }

    #[test]
    fn a_path_is_taken_as_sent_and_only_a_complete_one() {
        let good = if cfg!(windows) { r"C:\data" } else { "/srv/data" };
        assert_eq!(prepare(good), Ok(good.to_owned()));
        // No trimming: a leading space makes it a relative path.
        let spaced = format!(" {good}");
        assert!(prepare(&spaced).is_err());
        let long = format!("{good}{}", "a".repeat(PATH_MAX));
        for bad in ["", "relative", "a\0b", long.as_str()] {
            let e = prepare(bad).unwrap_err();
            assert!(msg::DIR_LIST_PATH_BAD.is_in(&e), "{bad:?}: {e}");
        }
        if cfg!(windows) {
            for bad in [r"\\server\share", r"\\server\share\x", "//server/share", r"/\server\share", r"\\?\C:\x", r"\\.\C:", r"\x", "C:foo"]
            {
                assert!(prepare(bad).is_err(), "{bad}");
            }
            assert!(prepare("C:/data").is_ok());
        }
    }

    #[test]
    fn a_reservation_is_atomic_and_every_drop_gives_it_back() {
        let linger = Linger::new();
        let holds: Vec<Hold> = (0..LINGER_MAX).map(|_| linger.try_take().expect("under the cap")).collect();
        assert_eq!(linger.count(), LINGER_MAX);
        assert!(linger.try_take().is_none());
        assert_eq!(linger.count(), LINGER_MAX, "a refused reservation counts nothing");
        drop(holds);
        assert_eq!(linger.count(), 0);
        // One below the cap lets exactly one through.
        let three: Vec<Hold> = (0..LINGER_MAX - 1).map(|_| linger.try_take().unwrap()).collect();
        let last = linger.try_take();
        assert!(last.is_some() && linger.try_take().is_none());
        drop((three, last));
        assert_eq!(linger.count(), 0);
    }

    #[test]
    fn a_listing_answers_the_listing_json_and_gives_its_count_back() {
        let linger = Linger::new();
        let hold = linger.try_take().unwrap();
        let out = run(work("p".into(), lister_of(vec![Entry::dir("a"), Entry::file("b").size(3)]), hold, Duration::from_secs(5)));
        let listing: Listing = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(listing.entries.len(), 2);
        assert_eq!(linger.count(), 0);
    }

    #[test]
    fn a_failed_listing_carries_the_path_and_the_engines_words() {
        let linger = Linger::new();
        let lister: Lister = Arc::new(|_| Err("no such folder".into()));
        let long = format!("/{}", "x".repeat(300));
        let e = run(work(long, lister, linger.try_take().unwrap(), Duration::from_secs(5))).unwrap_err();
        assert!(msg::DIR_LIST_FAILED.is_in(&e) && e.contains("no such folder"), "{e}");
        assert!(!e.contains(&"x".repeat(201)), "the path is cut at {SHORT_MAX} characters");
        assert_eq!(linger.count(), 0);
    }

    #[test]
    fn a_read_past_the_slow_time_answers_slow_and_keeps_its_count_until_it_ends() {
        let linger = Linger::new();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let go_rx = std::sync::Mutex::new(go_rx);
        let lister: Lister = Arc::new(move |_| {
            let _ = go_rx.lock().unwrap().recv();
            Ok(Listing::new(vec![]))
        });
        let e = run(work("p".into(), lister, linger.try_take().unwrap(), Duration::from_millis(20))).unwrap_err();
        assert!(msg::DIR_LIST_SLOW.is_in(&e), "{e}");
        assert_eq!(linger.count(), 1, "the read goes on alone");
        go_tx.send(()).unwrap();
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while linger.count() > 0 {
            assert!(std::time::Instant::now() < end, "the read did not give its count back");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_read_that_panics_answers_stopped_and_gives_its_count_back() {
        let linger = Linger::new();
        let lister: Lister = Arc::new(|_| panic!("boom"));
        let e = run(work("p".into(), lister, linger.try_take().unwrap(), Duration::from_secs(5))).unwrap_err();
        assert!(msg::DIR_LIST_STOPPED.is_in(&e), "{e}");
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while linger.count() > 0 {
            assert!(std::time::Instant::now() < end, "the count was not given back");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_work_that_never_runs_gives_its_count_back() {
        let linger = Linger::new();
        let w = work("p".into(), lister_of(vec![]), linger.try_take().unwrap(), SLOW);
        assert_eq!(linger.count(), 1);
        drop(w);
        assert_eq!(linger.count(), 0);
    }

    #[test]
    fn the_places_reply_reads_back_with_this_pcs_style() {
        let reply: PlacesReply = serde_json::from_str(&places().unwrap()).unwrap();
        assert_eq!(reply.style, ito_filepick::local_style());
    }

    #[test]
    fn the_real_lister_refuses_a_relative_path_without_reading_anything() {
        let list = real();
        assert!(list("relative/dir").is_err());
        assert!(list("").is_err());
    }

    /// A junction (made with `mklink /J`, which needs no administrator) in a temp folder is listed as a link and not
    /// followed, and a listing through it is refused with ito's link reason.
    #[cfg(windows)]
    #[test]
    fn a_junction_is_listed_as_a_link_and_a_listing_through_it_is_refused() {
        let base = std::env::temp_dir().join(format!("koyomi-dir-list-junction-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let target = base.join("target");
        std::fs::create_dir_all(target.join("inner")).unwrap();
        let junction = base.join("jn");
        let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&junction).arg(&target).output();
        if !made.is_ok_and(|o| o.status.success()) {
            eprintln!("skipped: a junction could not be made here");
            let _ = std::fs::remove_dir_all(&base);
            return;
        }
        // Every read first and the folder removed before any assert, so a failing assert leaves nothing behind.
        let list = real();
        let top = list(base.to_str().unwrap());
        let through = list(junction.to_str().unwrap());
        // The same options straight through ito, to see which refusal it is (not any error that quotes the path).
        let direct = ito_filepick::list_local_with(junction.to_str().unwrap(), &ito_filepick::ListOptions::strict(MAX_ENTRIES));
        let _ = std::fs::remove_dir_all(&junction);
        let _ = std::fs::remove_dir_all(&base);
        let top: Listing = top.expect("the temp folder lists");
        let link = top.entries.iter().find(|e| e.name == "jn").expect("the junction is listed");
        assert!(link.link, "{link:?}");
        let Err(refusal) = direct else { panic!("ito refuses a listing through a link") };
        assert!(matches!(refusal, ito_filepick::ListError::Link { .. }), "{refusal:?}");
        assert_eq!(through.err(), Some(refusal.words(&msg::language())), "the engine passes ito's words on");
    }
}
