// SPDX-License-Identifier: Apache-2.0
//! Joinable workers with an explicit wake path, and C-callback containment.
//!
//! Nothing here is QNX-specific except the log line of a late join, which
//! goes through `qnx-syslog` (system log and stamped stderr).
//!
//! **Workers.** Detached threads can outlive a shared library and execute
//! unmapped code. A `Worker` cannot be detached: it carries its `JoinHandle`.
//! Its `Drop` signals and joins, and the shutdown flag is part of the type rather
//! than a convention.
//!
//! **A flag is not a shutdown.** Setting an `AtomicBool` before `join()`
//! is sufficient only for a worker that polls. A thread parked in `MsgReceive`, `recv`,
//! `read`, `sem_wait`, `usbd_io` or a firmware round trip never reaches
//! the top of its loop to look at the flag, so signal-then-join converts
//! "thread outlives the DLL" into "unload hangs forever". Trading a
//! use-after-unmap for a permanent hang is a better failure, not a fixed
//! one.
//!
//! So the wake path is part of the constructor. [`Worker::spawn`] takes a
//! [`WakePath`] and there is no overload without one; a worker that
//! genuinely only polls must say so with [`WakePath::polling`], which is
//! a claim someone writes on purpose rather than the default you get by
//! not thinking about it.
//!
//! Shutdown runs in the order the wake path requires:
//!
//! 1. **Seal**: [`Shutdown::signal`] is set first, so a worker that
//!    wakes for any other reason sees "stop" before it looks for work.
//! 2. **Signal**: the same store; sealing and signalling are one act
//!    here because there is one flag.
//! 3. **Wake**: [`WakePath`] runs: close the socket, pulse the channel,
//!    post the semaphore, cancel the URB.
//! 4. **Join**: unconditionally, and never with a detach as the escape.
//!
//! **What bounds the join.** The wake operation does, not a timer. There
//! is no way in Rust (or in POSIX, safely) to abort a thread that will
//! not come back, so a deadline here cannot be a kill: a deadline that
//! "expires" by abandoning the thread would reintroduce exactly the
//! use-after-unmap this type exists to prevent. The deadline is therefore
//! a **detector**: [`Worker::stop_within`] reports whether the join
//! landed inside it and logs the worker and its primitive when it did
//! not, and then joins anyway. A `LateJoin` in a log is a missing or
//! wrong wake path, and is a bug in the caller.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long [`Worker::stop`] expects a woken worker to take to exit.
///
/// Generous on purpose: it is a "something is wrong" threshold, not a
/// latency budget. A correctly woken worker returns in microseconds.
pub const JOIN_DEADLINE: Duration = Duration::from_secs(5);

/// A shutdown signal shared with a worker.
///
/// Setting it seals new work and asks the worker to stop. It does **not**
/// by itself get a blocked worker moving; see [`WakePath`].
#[derive(Clone)]
pub struct Shutdown(Arc<AtomicBool>);

impl Shutdown {
    #[must_use]
    pub fn new() -> Shutdown {
        Shutdown(Arc::new(AtomicBool::new(false)))
    }
    /// Ask the worker to stop. Idempotent.
    pub fn signal(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    /// Has stopping been asked for?
    #[must_use]
    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

/// The operation that gets a *blocked* worker moving again.
///
/// One per worker, named after the primitive it interrupts so a late
/// join says which wait is stuck rather than just which thread.
pub struct WakePath {
    primitive: &'static str,
    wake: Option<Box<dyn Fn() + Send + Sync>>,
}

impl WakePath {
    /// The worker blocks in `primitive`, and `wake` interrupts it.
    ///
    /// `wake` runs on the stopping thread, after the shutdown flag is
    /// set, and must be safe to call when the worker has already exited
    /// (shutdown races with a worker finishing on its own).
    #[must_use]
    pub fn interrupt<F>(primitive: &'static str, wake: F) -> WakePath
    where
        F: Fn() + Send + Sync + 'static,
    {
        WakePath {
            primitive,
            wake: Some(Box::new(wake)),
        }
    }

    /// The worker never blocks: it polls [`Shutdown::is_set`] and every
    /// wait it performs is bounded and short.
    ///
    /// This is an assertion about the worker body, and the only way to
    /// build a worker with no wake operation. If the body later grows a
    /// blocking call, this becomes wrong and shows up as a `LateJoin`.
    #[must_use]
    pub fn polling() -> WakePath {
        WakePath {
            primitive: "polling (no blocking wait)",
            wake: None,
        }
    }

    /// Which wait this path interrupts, for diagnostics.
    #[must_use]
    pub fn primitive(&self) -> &'static str {
        self.primitive
    }

    fn wake(&self) {
        if let Some(f) = &self.wake {
            f();
        }
    }
}

/// What a stop actually did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// Joined inside the deadline. `panicked` is whether the worker body
    /// unwound (only observable in unwinding builds; the release profile
    /// aborts).
    Cleanly { panicked: bool, waited: Duration },
    /// The wake did not get the worker out in time. It was joined anyway
    /// (the thread is never abandoned), but the wake path is wrong or
    /// missing and the caller has a bug.
    LateJoin { panicked: bool, waited: Duration },
}

impl Stopped {
    /// Joined inside the deadline without panicking.
    #[must_use]
    pub fn is_clean(self) -> bool {
        matches!(
            self,
            Stopped::Cleanly {
                panicked: false,
                ..
            }
        )
    }

    /// How long the join took, measured from the wake.
    #[must_use]
    pub fn waited(self) -> Duration {
        match self {
            Stopped::Cleanly { waited, .. } | Stopped::LateJoin { waited, .. } => waited,
        }
    }
}

/// A worker thread that is always joined, and always has a way to be
/// woken out of whatever it blocks in.
///
/// There is deliberately no `detach()`. The C code this replaces could
/// detach, and did, and that is what made its shutdown impossible.
pub struct Worker {
    handle: Option<JoinHandle<()>>,
    shutdown: Shutdown,
    wake: WakePath,
    /// Disconnects when the worker's stack unwinds or returns, which is
    /// how the deadline is observed without polling the `JoinHandle`.
    exited: Option<Receiver<()>>,
    name: &'static str,
}

impl Worker {
    /// Spawn `f`, handing it the shutdown signal to poll, with `wake` as
    /// the operation that interrupts whatever `f` blocks in.
    ///
    /// # Errors
    /// Propagates the thread-spawn failure: out of memory, or a
    /// per-process thread limit.
    pub fn spawn<F>(name: &'static str, wake: WakePath, f: F) -> std::io::Result<Worker>
    where
        F: FnOnce(Shutdown) + Send + 'static,
    {
        let shutdown = Shutdown::new();
        let s = shutdown.clone();
        let (tx, exited) = channel::<()>();
        let handle = std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                // Owned by the worker's frame, so it drops (and
                // disconnects `exited`) on return AND on unwind.
                let _alive: Sender<()> = tx;
                f(s);
            })?;
        Ok(Worker {
            handle: Some(handle),
            shutdown,
            wake,
            exited: Some(exited),
            name,
        })
    }

    #[must_use]
    pub fn shutdown_handle(&self) -> Shutdown {
        self.shutdown.clone()
    }

    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The wait this worker's wake path interrupts.
    #[must_use]
    pub fn primitive(&self) -> &'static str {
        self.wake.primitive()
    }

    /// Seal, signal, wake, join, with [`JOIN_DEADLINE`] as the
    /// late-join detector. Returns whether the worker exited cleanly and
    /// in time.
    #[must_use]
    pub fn stop(mut self) -> bool {
        self.stop_inner(JOIN_DEADLINE).is_clean()
    }

    /// As [`Worker::stop`], with an explicit detection deadline, and
    /// reporting what happened.
    ///
    /// The deadline does not abort the thread: exceeding it logs and is
    /// reported, and the join still runs to completion.
    #[must_use]
    pub fn stop_within(mut self, deadline: Duration) -> Stopped {
        self.stop_inner(deadline)
    }

    fn stop_inner(&mut self, deadline: Duration) -> Stopped {
        // 1+2. Seal new work and signal, before the wake: a worker that
        //      wakes for an unrelated reason must see "stop", not spin
        //      back into the wait we are about to interrupt.
        self.shutdown.signal();

        // 3. Actively interrupt the wait. Nothing below this line can
        //    make a blocked worker return.
        let started = Instant::now();
        self.wake.wake();

        // 4. Observe, then join unconditionally.
        let in_time = match self.exited.take() {
            // Disconnected == the worker's frame is gone. A Timeout is
            // the interesting case: still blocked after the wake.
            Some(rx) => matches!(
                rx.recv_timeout(deadline),
                Err(RecvTimeoutError::Disconnected)
            ),
            None => true,
        };
        if !in_time {
            qnx_syslog::error(&format!(
                "worker {} still blocked in {} {:?} after wake; joining anyway \
                 (its wake path is missing or wrong)",
                self.name,
                self.wake.primitive(),
                deadline
            ));
        }
        let panicked = match self.handle.take() {
            Some(h) => h.join().is_err(),
            None => false,
        };
        let waited = started.elapsed();
        if in_time {
            Stopped::Cleanly { panicked, waited }
        } else {
            Stopped::LateJoin { panicked, waited }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Signalled, woken and joined even on an early return or an
        // unwind, so a worker can never outlive the state it borrows.
        let _ = self.stop_inner(JOIN_DEADLINE);
    }
}

/// Run `f`, containing any panic so it cannot unwind into a C caller.
///
/// Returns `None` if `f` panicked. With `panic = "abort"` this never
/// returns `None` in release builds (the process aborts first), but it
/// documents the boundary and holds for test and debug builds where
/// unwinding is enabled.
pub fn contain<F, T>(what: &str, f: F) -> Option<T>
where
    F: FnOnce() -> T,
{
    let Ok(v) = catch_unwind(AssertUnwindSafe(f)) else {
        qnx_syslog::error(&format!("panic contained at the C boundary in {what}"));
        return None;
    };
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;
    use std::sync::{Condvar, Mutex};

    /// The polling case the original design assumed was the only case.
    #[test]
    fn polling_worker_stops_when_asked() {
        let ticks = Arc::new(AtomicU32::new(0));
        let t = ticks.clone();
        let w = Worker::spawn("ticker", WakePath::polling(), move |sd| {
            while !sd.is_set() {
                t.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(1));
            }
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        assert!(w.stop(), "worker joined cleanly");
        let seen = ticks.load(Ordering::SeqCst);
        assert!(seen > 0, "worker actually ran");
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(ticks.load(Ordering::SeqCst), seen, "and really stopped");
    }

    #[test]
    fn dropping_a_worker_joins_it() {
        let flag = Arc::new(AtomicBool::new(false));
        let f = flag.clone();
        {
            let _w = Worker::spawn("dropper", WakePath::polling(), move |sd| {
                while !sd.is_set() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                f.store(true, Ordering::SeqCst);
            })
            .unwrap();
        } // drop joins
        assert!(flag.load(Ordering::SeqCst), "drop signalled AND waited");
    }

    /// Blocked in a channel receive: the stand-in for `MsgReceive` and
    /// for any queue-driven driver thread. The wake is a sentinel send.
    ///
    /// Without the wake this worker never observes the flag at all: the
    /// test would hang in `stop`, which is the defect the finding names.
    #[test]
    fn wakes_a_worker_blocked_in_recv() {
        let (tx, rx) = channel::<u32>();
        let woke = Arc::new(AtomicBool::new(false));
        let w2 = woke.clone();
        let w = Worker::spawn(
            "recv-blocked",
            WakePath::interrupt("mpsc::recv", move || {
                // Safe after the worker has already exited: a send on a
                // disconnected channel is an Err, not a panic.
                let _ = tx.send(0);
            }),
            move |sd| {
                // Blocks indefinitely. It cannot poll `sd` from here.
                while let Ok(v) = rx.recv() {
                    if sd.is_set() {
                        w2.store(true, Ordering::SeqCst);
                        return;
                    }
                    assert_ne!(v, 0, "sentinel only arrives at shutdown");
                }
            },
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));

        let r = w.stop_within(Duration::from_secs(2));
        assert!(r.is_clean(), "woken and joined in time: {r:?}");
        assert!(woke.load(Ordering::SeqCst), "worker saw the shutdown flag");
    }

    /// Blocked on a condvar: the stand-in for `sem_wait` and for the
    /// firmware round trips. The wake is a notify, and the predicate is
    /// the shutdown flag, so a spurious wake cannot re-park it forever.
    #[test]
    fn wakes_a_worker_blocked_on_a_condvar() {
        let pair = Arc::new((Mutex::new(false), Condvar::new()));
        let p = pair.clone();
        let waker = pair.clone();
        let w = Worker::spawn(
            "condvar-blocked",
            WakePath::interrupt("Condvar::wait", move || {
                let (m, cv) = &*waker;
                *m.lock().unwrap() = true;
                cv.notify_all();
            }),
            move |sd| {
                let (m, cv) = &*p;
                let mut g = m.lock().unwrap();
                while !*g && !sd.is_set() {
                    g = cv.wait(g).unwrap();
                }
            },
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));

        let r = w.stop_within(Duration::from_secs(2));
        assert!(r.is_clean(), "notify released the wait: {r:?}");
    }

    /// The failure the deadline exists to make visible: a worker that
    /// really is blocked, whose wake path does not reach it.
    ///
    /// The join is still completed (the thread is not abandoned), so the
    /// test releases the worker from a second thread and checks that the
    /// stop reported `LateJoin` rather than silently passing.
    #[test]
    fn a_wake_that_does_not_reach_the_worker_is_reported() {
        let (tx, rx) = channel::<u32>();
        let w = Worker::spawn(
            "unreachable-wake",
            // Deliberately wrong: claims to poll while the body blocks.
            WakePath::polling(),
            move |_sd| {
                let _ = rx.recv();
            },
        )
        .unwrap();

        // Releases the worker well after the detection deadline, so the
        // join completes and the test cannot hang.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            drop(tx);
        });

        let r = w.stop_within(Duration::from_millis(50));
        assert!(
            matches!(
                r,
                Stopped::LateJoin {
                    panicked: false,
                    ..
                }
            ),
            "a missing wake path must be reported, not swallowed: {r:?}"
        );
        assert!(
            r.waited() >= Duration::from_millis(50),
            "and the join was actually waited out, not abandoned: {r:?}"
        );
    }

    #[test]
    fn contain_catches_a_panic() {
        assert_eq!(contain("ok", || 7), Some(7));
        let r: Option<()> = contain("boom", || panic!("from a callback"));
        assert!(r.is_none(), "the panic did not escape");
    }
}
