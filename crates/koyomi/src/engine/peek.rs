//! `HostPeek`: the engine connects to a host and says what it shows (the SSH host key or the WinRM
//! certificate's fingerprint), for the settings' host form. A peek takes up to about 20 s, so it runs on a
//! thread of its own and the loop only starts it (`start`) and collects what finished (`drain`).
//!
//! At most [`SLOTS`] run at once; a slot is reserved before its thread is spawned. The replies wait here, not
//! in the threads: when the engine stops, the loop returns and drops them, the door then tells the caller the
//! request was not done, and a thread that ends later sends into a closed channel and its error is ignored.
//! Nothing is reported as done after Stop.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::Arc;

use ito_ipc::door::Reply;
use koyomi_core::msg;

use crate::proto::{self, PeekKind};

/// How many peeks may run at once.
pub const SLOTS: usize = 4;

/// What a peek does: the key or fingerprint `host:port` shows, or why not. Injected so tests never connect.
pub type PeekFn = Arc<dyn Fn(PeekKind, &str, u16) -> Result<String, String> + Send + Sync>;

/// The peeks as they really connect.
pub fn real() -> PeekFn {
    Arc::new(|kind, host, port| match kind {
        PeekKind::Ssh => crate::sftp::peek_host_key(host, port),
        PeekKind::Winrm => crate::winrm::peek_cert(host, port),
    })
}

/// A peek that was started and has not been drained yet.
struct Pending {
    reply: SyncSender<Reply>,
    expired: Arc<AtomicBool>,
    target: String,
    by: String,
}

/// Sends `(id, result)` exactly once when it is dropped: the real result, or an internal error when the peek
/// ended without one (it panicked).
struct Guard {
    id: u64,
    tx: Sender<(u64, Result<String, String>)>,
    result: Option<Result<String, String>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let result = self.result.take().unwrap_or_else(|| Err(msg::line(msg::HOST_PEEK_STOPPED, &[])));
        // The manager may be gone (the engine stopped): nobody is waiting.
        let _ = self.tx.send((self.id, result));
    }
}

pub struct Peeks {
    peek: PeekFn,
    tx: Sender<(u64, Result<String, String>)>,
    rx: Receiver<(u64, Result<String, String>)>,
    pending: HashMap<u64, Pending>,
    next: u64,
    /// The threads started, kept so a test can join them.
    #[cfg(test)]
    handles: Vec<std::thread::JoinHandle<()>>,
}

impl Peeks {
    pub fn new(peek: PeekFn) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            peek,
            tx,
            rx,
            pending: HashMap::new(),
            next: 0,
            #[cfg(test)]
            handles: Vec::new(),
        }
    }

    /// The threads started so far, handed over (a test joins them after the manager is gone).
    #[cfg(test)]
    pub fn take_handles(&mut self) -> Vec<std::thread::JoinHandle<()>> {
        std::mem::take(&mut self.handles)
    }

    /// How many peeks run now.
    #[cfg(test)]
    pub fn running(&self) -> usize {
        self.pending.len()
    }

    /// Starts a peek of `host:port` for `by`, and answers `reply` when it is done (`drain`), unless `expired`
    /// is set by then. Refuses at once a host or port the rule (`proto::peek_target_ok`) does not take, and a
    /// fifth peek.
    pub fn start(&mut self, by: &str, kind: PeekKind, host: &str, port: u16, reply: SyncSender<Reply>, expired: Arc<AtomicBool>) {
        if expired.load(Ordering::Relaxed) {
            return;
        }
        let refuse = |why: String| {
            if !expired.load(Ordering::Relaxed) {
                let _ = reply.send(Err(why));
            }
        };
        // Before anything else, and without quoting what was sent.
        if !proto::peek_target_ok(host, port) {
            return refuse(msg::line(msg::HOST_PEEK_BAD, &[]));
        }
        if self.pending.len() >= SLOTS {
            return refuse(msg::line(msg::HOST_PEEK_BUSY, &[]));
        }
        let id = self.next;
        self.next += 1;
        // Reserved before the thread exists: the cap cannot be passed by asking fast.
        self.pending
            .insert(id, Pending { reply: reply.clone(), expired: expired.clone(), target: format!("{host}:{port}"), by: by.to_owned() });
        let (peek, tx, host) = (self.peek.clone(), self.tx.clone(), host.to_owned());
        let spawned = std::thread::Builder::new().name("koyomi-host-peek".into()).spawn(move || {
            let mut guard = Guard { id, tx, result: None };
            guard.result = Some(peek(kind, &host, port));
        });
        match spawned {
            Ok(_handle) => {
                #[cfg(test)]
                self.handles.push(_handle);
            }
            Err(_) => {
                // The closure was dropped without running, so no guard was made and nothing was sent; were a stray
                // result ever to come, `drain` ignores an id it no longer holds.
                self.pending.remove(&id);
                refuse(msg::line(msg::HOST_PEEK_STOPPED, &[]));
            }
        }
    }

    /// Collects the peeks that finished: frees their slots, hands `record(op, target, detail, by)` each one
    /// (for both outcomes) and answers the caller that still waits.
    pub fn drain(&mut self, mut record: impl FnMut(&str, &str, &str, &str)) {
        while let Ok((id, result)) = self.rx.try_recv() {
            let Some(p) = self.pending.remove(&id) else { continue };
            let first = match &result {
                Ok(v) | Err(v) => v.lines().next().unwrap_or_default().chars().take(120).collect::<String>(),
            };
            record("host-peek", &p.target, &first, &p.by);
            if !p.expired.load(Ordering::Relaxed) {
                let _ = p.reply.send(result);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{sync_channel, RecvTimeoutError};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    fn fake(f: impl Fn(PeekKind, &str, u16) -> Result<String, String> + Send + Sync + 'static) -> Peeks {
        Peeks::new(Arc::new(f))
    }

    type Ops = Vec<(String, String, String, String)>;

    /// Drains until `n` peeks have finished (or 5 s pass); the ops recorded so far.
    fn drain_until(p: &mut Peeks, ops: &mut Ops, left: usize) {
        let end = Instant::now() + Duration::from_secs(5);
        while p.running() > left && Instant::now() < end {
            p.drain(|a, b, c, d| ops.push((a.into(), b.into(), c.into(), d.into())));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(p.running(), left, "peeks did not finish");
    }

    fn ask(p: &mut Peeks, kind: PeekKind, host: &str, port: u16) -> (Receiver<Reply>, Arc<AtomicBool>) {
        let (tx, rx) = sync_channel(1);
        let expired = Arc::new(AtomicBool::new(false));
        p.start("ann", kind, host, port, tx, expired.clone());
        (rx, expired)
    }

    #[test]
    fn an_ok_peek_answers_with_the_key_and_is_recorded() {
        let mut p = fake(|kind, host, port| Ok(format!("SHA256:{kind:?}-{host}-{port}\nmore")));
        let (rx, _) = ask(&mut p, PeekKind::Ssh, "h.example", 22);
        let mut ops = Ops::new();
        drain_until(&mut p, &mut ops, 0);
        assert_eq!(rx.try_recv(), Ok(Ok("SHA256:Ssh-h.example-22\nmore".to_owned())));
        assert_eq!(ops, vec![("host-peek".into(), "h.example:22".into(), "SHA256:Ssh-h.example-22".into(), "ann".into())]);
    }

    #[test]
    fn an_err_peek_answers_with_the_error_and_is_recorded_cut_to_120_characters() {
        let long = format!("h:22: {}", "x".repeat(300));
        let said = long.clone();
        let mut p = fake(move |_, _, _| Err(said.clone()));
        let (rx, _) = ask(&mut p, PeekKind::Winrm, "h", 22);
        let mut ops = Ops::new();
        drain_until(&mut p, &mut ops, 0);
        assert_eq!(rx.try_recv(), Ok(Err(long.clone())));
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].2.chars().count(), 120);
        assert!(long.starts_with(&ops[0].2));
    }

    #[test]
    fn a_panicking_peek_answers_with_an_internal_error_and_frees_its_slot() {
        let mut p = fake(|_, _, _| panic!("boom"));
        let (rx, _) = ask(&mut p, PeekKind::Ssh, "h", 22);
        let mut ops = Ops::new();
        drain_until(&mut p, &mut ops, 0);
        let Ok(Err(e)) = rx.try_recv() else { panic!("no error answer") };
        assert!(msg::HOST_PEEK_STOPPED.is_in(&e), "{e}");
        assert_eq!(ops.len(), 1);
        assert!(msg::HOST_PEEK_STOPPED.is_in(&ops[0].2), "{:?}", ops[0]);
    }

    /// A peek that waits until the test lets it go.
    fn held() -> (Peeks, Sender<()>) {
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let go_rx = Mutex::new(go_rx);
        let p = fake(move |_, host, _| {
            let _ = go_rx.lock().unwrap().recv();
            Ok(format!("SHA256:{host}"))
        });
        (p, go_tx)
    }

    #[test]
    fn a_fifth_peek_is_refused_at_once_and_a_finished_one_frees_its_slot() {
        let (mut p, go) = held();
        let waiting: Vec<_> = (0..SLOTS).map(|i| ask(&mut p, PeekKind::Ssh, &format!("h{i}"), 22).0).collect();
        assert_eq!(p.running(), SLOTS);
        let (rx, _) = ask(&mut p, PeekKind::Ssh, "h9", 22);
        let Ok(Err(e)) = rx.try_recv() else { panic!("the fifth was not refused at once") };
        assert!(msg::HOST_PEEK_BUSY.is_in(&e), "{e}");
        assert_eq!(p.running(), SLOTS, "a refused peek holds no slot");
        // One goes; its slot is free again.
        go.send(()).unwrap();
        let mut ops = Ops::new();
        drain_until(&mut p, &mut ops, SLOTS - 1);
        let (rx, _) = ask(&mut p, PeekKind::Ssh, "h10", 22);
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)), "the new peek should run");
        assert_eq!(p.running(), SLOTS);
        for _ in 0..SLOTS {
            go.send(()).unwrap();
        }
        drain_until(&mut p, &mut ops, 0);
        assert_eq!(ops.len(), SLOTS + 1);
        drop(waiting);
    }

    #[test]
    fn an_invalid_host_or_port_is_refused_without_quoting_it_and_never_connects() {
        let called = Arc::new(AtomicBool::new(false));
        let seen = called.clone();
        let mut p = fake(move |_, _, _| {
            seen.store(true, Ordering::Relaxed);
            Ok(String::new())
        });
        for (host, port) in [("user@secret-host", 22), ("::1", 22), ("h x", 22), ("", 22), ("host", 0)] {
            let (rx, _) = ask(&mut p, PeekKind::Ssh, host, port);
            let Ok(Err(e)) = rx.try_recv() else { panic!("{host:?} was not refused at once") };
            assert!(msg::HOST_PEEK_BAD.is_in(&e), "{e}");
            assert!(host.is_empty() || !e.contains(host), "{e} quotes {host}");
        }
        assert_eq!(p.running(), 0);
        assert!(!called.load(Ordering::Relaxed));
    }

    #[test]
    fn a_caller_that_stopped_waiting_before_the_start_starts_nothing() {
        let called = Arc::new(AtomicBool::new(false));
        let seen = called.clone();
        let mut p = fake(move |_, _, _| {
            seen.store(true, Ordering::Relaxed);
            Ok("SHA256:x".into())
        });
        let (tx, rx) = sync_channel(1);
        p.start("ann", PeekKind::Ssh, "h", 22, tx, Arc::new(AtomicBool::new(true)));
        assert_eq!(p.running(), 0);
        assert!(matches!(rx.recv_timeout(Duration::from_millis(50)), Err(RecvTimeoutError::Disconnected | RecvTimeoutError::Timeout)));
        std::thread::sleep(Duration::from_millis(50));
        assert!(!called.load(Ordering::Relaxed));
    }

    #[test]
    fn a_caller_that_gave_up_midway_gets_no_reply_but_the_op_is_still_recorded() {
        let (mut p, go) = held();
        let (rx, expired) = ask(&mut p, PeekKind::Ssh, "h", 22);
        expired.store(true, Ordering::Relaxed);
        go.send(()).unwrap();
        let mut ops = Ops::new();
        drain_until(&mut p, &mut ops, 0);
        assert!(rx.try_recv().is_err(), "an expired caller must get no reply");
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].1, "h:22");
    }

    #[test]
    fn a_manager_dropped_after_the_start_does_not_panic_the_thread() {
        let (mut p, go) = held();
        let (rx, _) = ask(&mut p, PeekKind::Ssh, "h", 22);
        // The engine stopped: the pending reply goes with the manager, and the door tells the caller.
        let handles = p.take_handles();
        drop(p);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Err(RecvTimeoutError::Disconnected));
        // The peek ends later and sends into a closed channel: its thread must end without a panic.
        go.send(()).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        for h in handles {
            while !h.is_finished() {
                assert!(Instant::now() < end, "the peek thread did not end");
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(h.join().is_ok(), "the peek thread panicked");
        }
    }
}
