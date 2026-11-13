//! Work that takes long and must not run on the engine loop (the peek at a host, the test of a database): each ask runs
//! on a thread of its own, and the loop only starts it (`start`) and collects what finished (`drain`).
//!
//! The contract, in order:
//! - An ask whose caller has already given up (`expired`) starts nothing and takes no slot.
//! - At most `cap` run at once; a slot is reserved before its thread is spawned, so asking fast cannot pass the cap.
//! - A thread that cannot be spawned frees its reservation and answers the manager's `stopped`.
//! - A thread that ends without a result (it panicked) answers `stopped` too, through a guard.
//! - The replies wait here, not in the threads. When the engine stops (`stop`), what already finished is answered with its
//!   real result, and every ask still running is answered `stopped` and recorded at once; a work that ends later sends
//!   into the manager's channel and is never delivered (its id is gone). Nothing is reported as done after Stop.
//! - A caller that gave up midway gets no reply, but its slot stays held until its work returns, and the op is still
//!   recorded.
//! - Dropping the manager sets `cancel` too, which the work looks at (work that does not look at it just ends later).
//!   Work that blocks (a credential store read, a file check) cannot be cancelled: its thread ends later on its own.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::Arc;

use ito_ipc::door::Reply;
use koyomi_core::msg;

/// The work of one ask: its result is the reply. It gets the cancel flag of the manager.
pub type Work = Box<dyn FnOnce(&AtomicBool) -> Result<String, String> + Send>;

/// What the operations record keeps of a result.
pub type Detail = fn(&Result<String, String>) -> String;

/// An ask that was started and has not been drained yet.
struct Pending {
    reply: SyncSender<Reply>,
    expired: Arc<AtomicBool>,
    target: String,
    by: String,
}

/// Sends `(id, result)` exactly once when it is dropped: the real result, or the manager's `stopped` error when the work
/// ended without one (it panicked).
struct Guard {
    id: u64,
    tx: Sender<(u64, Result<String, String>)>,
    stopped: msg::Id,
    result: Option<Result<String, String>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let result = self.result.take().unwrap_or_else(|| Err(msg::line(self.stopped, &[])));
        // The manager may be gone (the engine stopped): nobody is waiting.
        let _ = self.tx.send((self.id, result));
    }
}

pub struct Slots {
    cap: usize,
    /// The op name in the operations record.
    op: &'static str,
    thread: &'static str,
    busy: msg::Id,
    stopped: msg::Id,
    detail: Detail,
    /// Set when the engine stops (`stop`) or the manager is dropped.
    cancel: Arc<AtomicBool>,
    tx: Sender<(u64, Result<String, String>)>,
    rx: Receiver<(u64, Result<String, String>)>,
    pending: HashMap<u64, Pending>,
    next: u64,
    /// The threads started, kept so a test can join them.
    #[cfg(test)]
    handles: Vec<std::thread::JoinHandle<()>>,
    /// A test makes the next spawns fail.
    #[cfg(test)]
    pub fail_spawn: bool,
}

impl Slots {
    pub fn new(cap: usize, op: &'static str, thread: &'static str, busy: msg::Id, stopped: msg::Id, detail: Detail) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            cap,
            op,
            thread,
            busy,
            stopped,
            detail,
            cancel: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
            pending: HashMap::new(),
            next: 0,
            #[cfg(test)]
            handles: Vec::new(),
            #[cfg(test)]
            fail_spawn: false,
        }
    }

    /// The threads started so far, handed over (a test joins them after the manager is gone).
    #[cfg(test)]
    pub fn take_handles(&mut self) -> Vec<std::thread::JoinHandle<()>> {
        std::mem::take(&mut self.handles)
    }

    /// How many asks run now.
    #[cfg(test)]
    pub fn running(&self) -> usize {
        self.pending.len()
    }

    /// Answers `reply` at once with `why`, unless the caller gave up.
    pub fn refuse(reply: &SyncSender<Reply>, expired: &AtomicBool, why: String) {
        if !expired.load(Ordering::Relaxed) {
            let _ = reply.send(Err(why));
        }
    }

    /// Starts `work` for `by` on a thread of its own; `target` is what the operations record names. It answers `reply`
    /// when the work is done (`drain`), unless `expired` is set by then. Refuses at once an ask over the cap.
    pub fn start(&mut self, by: &str, target: String, work: Work, reply: SyncSender<Reply>, expired: Arc<AtomicBool>) {
        if expired.load(Ordering::Relaxed) {
            return;
        }
        if self.pending.len() >= self.cap {
            return Self::refuse(&reply, &expired, msg::line(self.busy, &[]));
        }
        let id = self.next;
        self.next += 1;
        // Reserved before the thread exists: the cap cannot be passed by asking fast.
        self.pending.insert(id, Pending { reply: reply.clone(), expired: expired.clone(), target, by: by.to_owned() });
        let (tx, stopped, cancel) = (self.tx.clone(), self.stopped, self.cancel.clone());
        #[cfg(test)]
        let forced = self.fail_spawn;
        #[cfg(not(test))]
        let forced = false;
        let spawned = if forced {
            Err(std::io::Error::other("forced"))
        } else {
            std::thread::Builder::new().name(self.thread.into()).spawn(move || {
                let mut guard = Guard { id, tx, stopped, result: None };
                guard.result = Some(work(&cancel));
            })
        };
        match spawned {
            Ok(_handle) => {
                #[cfg(test)]
                self.handles.push(_handle);
            }
            Err(_) => {
                // The closure was dropped without running, so no guard was made and nothing was sent; were a stray
                // result ever to come, `drain` ignores an id it no longer holds.
                self.pending.remove(&id);
                Self::refuse(&reply, &expired, msg::line(self.stopped, &[]));
            }
        }
    }

    /// Collects the asks that finished: frees their slots, hands `record(op, target, detail, by)` each one (for both
    /// outcomes) and answers the caller that still waits. The record keeps only what `detail` makes of the result.
    pub fn drain(&mut self, mut record: impl FnMut(&str, &str, &str, &str)) {
        while let Ok((id, result)) = self.rx.try_recv() {
            let Some(p) = self.pending.remove(&id) else { continue };
            record(self.op, &p.target, &(self.detail)(&result), &p.by);
            if !p.expired.load(Ordering::Relaxed) {
                let _ = p.reply.send(result);
            }
        }
    }

    /// The engine stops: answers what already finished with its real result (`drain`), tells the running work to end,
    /// and answers every ask still running with the manager's `stopped` (unless its caller gave up), recording each one.
    /// The pending list is empty after; a work that ends later is never delivered.
    pub fn stop(&mut self, mut record: impl FnMut(&str, &str, &str, &str)) {
        self.drain(&mut record);
        self.cancel.store(true, Ordering::Relaxed);
        for (_, p) in std::mem::take(&mut self.pending) {
            let result = Err(msg::line(self.stopped, &[]));
            record(self.op, &p.target, &(self.detail)(&result), &p.by);
            if !p.expired.load(Ordering::Relaxed) {
                let _ = p.reply.send(result);
            }
        }
    }
}

impl Drop for Slots {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{sync_channel, RecvTimeoutError};
    use std::time::{Duration, Instant};

    fn plain(r: &Result<String, String>) -> String {
        match r {
            Ok(v) | Err(v) => v.clone(),
        }
    }

    fn slots(cap: usize) -> Slots {
        Slots::new(cap, "test-op", "koyomi-test-slot", msg::DB_TEST_BUSY, msg::DB_TEST_STOPPED, plain)
    }

    type Ops = Vec<(String, String, String, String)>;

    fn drain_until(s: &mut Slots, ops: &mut Ops, left: usize) {
        let end = Instant::now() + Duration::from_secs(5);
        while s.running() > left && Instant::now() < end {
            s.drain(|a, b, c, d| ops.push((a.into(), b.into(), c.into(), d.into())));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(s.running(), left, "work did not finish");
    }

    fn ask(s: &mut Slots, target: &str, work: Work) -> (Receiver<Reply>, Arc<AtomicBool>) {
        let (tx, rx) = sync_channel(1);
        let expired = Arc::new(AtomicBool::new(false));
        s.start("ann", target.into(), work, tx, expired.clone());
        (rx, expired)
    }

    /// Work that waits until the test lets it go.
    fn held() -> (Work, Sender<()>) {
        let (go_tx, go_rx) = mpsc::channel::<()>();
        (
            Box::new(move |_| {
                let _ = go_rx.recv();
                Ok("done".into())
            }),
            go_tx,
        )
    }

    #[test]
    fn the_result_answers_and_the_record_keeps_what_the_detail_function_makes_of_it() {
        fn id_only(r: &Result<String, String>) -> String {
            match r {
                Ok(_) => "ok".into(),
                Err(e) => e.chars().take(12).collect(),
            }
        }
        let mut s = Slots::new(2, "db-test", "t", msg::DB_TEST_BUSY, msg::DB_TEST_STOPPED, id_only);
        let (rx, _) = ask(&mut s, "h:5432/d", Box::new(|_| Err("KOYOMI-E2267 server said secret words".into())));
        let mut ops = Ops::new();
        drain_until(&mut s, &mut ops, 0);
        assert_eq!(rx.try_recv(), Ok(Err("KOYOMI-E2267 server said secret words".to_owned())));
        assert_eq!(ops, vec![("db-test".into(), "h:5432/d".into(), "KOYOMI-E2267".into(), "ann".into())]);
        let (rx, _) = ask(&mut s, "x", Box::new(|_| Ok("anything".into())));
        drain_until(&mut s, &mut ops, 0);
        assert_eq!(rx.try_recv(), Ok(Ok("anything".to_owned())));
        assert_eq!(ops[1].2, "ok");
    }

    #[test]
    fn a_panicking_work_answers_with_the_managers_stopped_and_frees_its_slot() {
        let mut s = slots(2);
        let (rx, _) = ask(&mut s, "t", Box::new(|_| panic!("boom")));
        let mut ops = Ops::new();
        drain_until(&mut s, &mut ops, 0);
        let Ok(Err(e)) = rx.try_recv() else { panic!("no error answer") };
        assert!(msg::DB_TEST_STOPPED.is_in(&e), "{e}");
        assert_eq!(ops.len(), 1);
    }

    #[test]
    fn the_cap_is_per_manager_and_a_refused_ask_holds_no_slot() {
        let (mut a, mut b) = (slots(4), slots(2));
        let mut gos = Vec::new();
        let mut waiting = Vec::new();
        for _ in 0..4 {
            let (w, go) = held();
            waiting.push(ask(&mut a, "t", w).0);
            gos.push(go);
        }
        for _ in 0..2 {
            let (w, go) = held();
            waiting.push(ask(&mut b, "t", w).0);
            gos.push(go);
        }
        assert_eq!((a.running(), b.running()), (4, 2));
        let (w, _go) = held();
        let (rx, _) = ask(&mut b, "t", w);
        let Ok(Err(e)) = rx.try_recv() else { panic!("the third was not refused at once") };
        assert!(msg::DB_TEST_BUSY.is_in(&e), "{e}");
        assert_eq!(b.running(), 2);
        // One of `b` goes; its slot is free again, and `a` is still full.
        gos[4].send(()).unwrap();
        let mut ops = Ops::new();
        drain_until(&mut b, &mut ops, 1);
        assert_eq!(a.running(), 4);
        let (w, go) = held();
        let (rx, _) = ask(&mut b, "t", w);
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)), "the new ask should run");
        assert_eq!(b.running(), 2);
        for g in &gos {
            let _ = g.send(());
        }
        let _ = go.send(());
        drain_until(&mut a, &mut ops, 0);
        drain_until(&mut b, &mut ops, 0);
    }

    #[test]
    fn a_caller_that_stopped_waiting_before_the_start_starts_nothing_and_takes_no_slot() {
        let mut s = slots(1);
        let called = Arc::new(AtomicBool::new(false));
        let seen = called.clone();
        let (tx, rx) = sync_channel(1);
        s.start(
            "ann",
            "t".into(),
            Box::new(move |_| {
                seen.store(true, Ordering::Relaxed);
                Ok(String::new())
            }),
            tx,
            Arc::new(AtomicBool::new(true)),
        );
        assert_eq!(s.running(), 0);
        assert!(matches!(rx.recv_timeout(Duration::from_millis(50)), Err(RecvTimeoutError::Disconnected | RecvTimeoutError::Timeout)));
        std::thread::sleep(Duration::from_millis(50));
        assert!(!called.load(Ordering::Relaxed));
        // Over the cap, an expired caller is not answered either.
        let (w, _go) = held();
        let _keep = ask(&mut s, "t", w);
        let (tx, rx) = sync_channel(1);
        s.start("ann", "t".into(), Box::new(|_| Ok(String::new())), tx, Arc::new(AtomicBool::new(true)));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_caller_that_gave_up_midway_gets_no_reply_keeps_its_slot_and_the_op_is_still_recorded() {
        let mut s = slots(1);
        let (w, go) = held();
        let (rx, expired) = ask(&mut s, "h:22", w);
        expired.store(true, Ordering::Relaxed);
        // The slot is held until the work ends, even though nobody waits.
        let (w2, _go2) = held();
        let (rx2, _) = ask(&mut s, "t", w2);
        assert!(msg::DB_TEST_BUSY.is_in(&rx2.try_recv().unwrap().unwrap_err()));
        go.send(()).unwrap();
        let mut ops = Ops::new();
        drain_until(&mut s, &mut ops, 0);
        assert!(rx.try_recv().is_err(), "an expired caller must get no reply");
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].1, "h:22");
    }

    #[test]
    fn a_spawn_that_fails_frees_the_reservation_and_answers_stopped() {
        let mut s = slots(1);
        s.fail_spawn = true;
        let (rx, _) = ask(&mut s, "t", Box::new(|_| Ok(String::new())));
        let Ok(Err(e)) = rx.try_recv() else { panic!("not answered") };
        assert!(msg::DB_TEST_STOPPED.is_in(&e), "{e}");
        assert_eq!(s.running(), 0);
        s.fail_spawn = false;
        let (rx, _) = ask(&mut s, "t", Box::new(|_| Ok("fine".into())));
        let mut ops = Ops::new();
        drain_until(&mut s, &mut ops, 0);
        assert_eq!(rx.try_recv(), Ok(Ok("fine".to_owned())));
    }

    #[test]
    fn a_manager_dropped_sets_cancel_sends_no_answer_and_the_thread_ends_without_a_panic() {
        let mut s = slots(1);
        let (seen_tx, seen_rx) = mpsc::channel::<bool>();
        let work: Work = Box::new(move |cancel| {
            let end = Instant::now() + Duration::from_secs(5);
            while !cancel.load(Ordering::Relaxed) && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = seen_tx.send(cancel.load(Ordering::Relaxed));
            Ok("late".into())
        });
        let (rx, _) = ask(&mut s, "t", work);
        let handles = s.take_handles();
        drop(s);
        // The pending reply went with the manager: the door tells the caller, and no answer comes after.
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Err(RecvTimeoutError::Disconnected));
        assert_eq!(seen_rx.recv_timeout(Duration::from_secs(5)), Ok(true), "the work did not see cancel");
        let end = Instant::now() + Duration::from_secs(5);
        for h in handles {
            while !h.is_finished() {
                assert!(Instant::now() < end, "the thread did not end");
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(h.join().is_ok(), "the thread panicked");
        }
    }

    #[test]
    fn stop_answers_the_finished_with_their_result_and_the_running_with_stopped_and_records_each_once() {
        let mut s = slots(4);
        let flag = s.cancel.clone();
        // One finished but not drained; two running; one running whose caller gave up.
        let (rx_done, _) = ask(&mut s, "done", Box::new(|_| Ok("real".into())));
        let end = Instant::now() + Duration::from_secs(5);
        // Wait until the first work has sent its result, and put it back for `stop` to collect.
        while s.rx.try_recv().map(|m| s.tx.send(m).unwrap()).is_err() {
            assert!(Instant::now() < end, "work did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
        let (w1, go1) = held();
        let (rx1, _) = ask(&mut s, "r1", w1);
        let (w2, go2) = held();
        let (rx2, _) = ask(&mut s, "r2", w2);
        let (w3, go3) = held();
        let (rx3, expired) = ask(&mut s, "gone", w3);
        expired.store(true, Ordering::Relaxed);
        let mut ops = Ops::new();
        s.stop(|a, b, c, d| ops.push((a.into(), b.into(), c.into(), d.into())));
        assert_eq!(s.running(), 0);
        assert!(flag.load(Ordering::Relaxed));
        assert_eq!(rx_done.try_recv(), Ok(Ok("real".to_owned())));
        for rx in [&rx1, &rx2] {
            let Ok(Err(e)) = rx.try_recv() else { panic!("not told stopped") };
            assert!(msg::DB_TEST_STOPPED.is_in(&e), "{e}");
        }
        assert!(rx3.try_recv().is_err(), "an expired caller must get no reply");
        let mut targets: Vec<&str> = ops.iter().map(|o| o.1.as_str()).collect();
        targets.sort_unstable();
        assert_eq!(targets, ["done", "gone", "r1", "r2"]);
        assert!(ops.iter().filter(|o| o.1 == "r1").all(|o| o.2.contains("E2271") && o.3 == "ann"));
        // The work that ends after the stop delivers nothing more.
        let handles = s.take_handles();
        for g in [&go1, &go2, &go3] {
            let _ = g.send(());
        }
        let end = Instant::now() + Duration::from_secs(5);
        for h in handles {
            while !h.is_finished() {
                assert!(Instant::now() < end, "the thread did not end");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        s.drain(|_, _, _, _| panic!("a late result must not be recorded"));
        drop(s);
        for rx in [&rx_done, &rx1, &rx2, &rx3] {
            assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Err(RecvTimeoutError::Disconnected));
        }
    }
}
