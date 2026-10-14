// SPDX-License-Identifier: Apache-2.0
//! Joinable workers with an explicit wake path, and C-callback containment.
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
pub const JOIN_DEADLINE: Duration = Duration::from_secs(5);
#[derive(Clone)]
pub struct Shutdown(Arc<AtomicBool>);
impl Shutdown {
    #[must_use]
    pub fn new() -> Shutdown {
        Shutdown(Arc::new(AtomicBool::new(false)))
    }
    pub fn signal(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
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
pub struct WakePath {
    primitive: &'static str,
    wake: Option<Box<dyn Fn() + Send + Sync>>,
}
impl WakePath {
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
    #[must_use]
    pub fn polling() -> WakePath {
        WakePath {
            primitive: "polling (no blocking wait)",
            wake: None,
        }
    }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    Cleanly { panicked: bool, waited: Duration },
    LateJoin { panicked: bool, waited: Duration },
}
impl Stopped {
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
    #[must_use]
    pub fn waited(self) -> Duration {
        match self {
            Stopped::Cleanly { waited, .. } | Stopped::LateJoin { waited, .. } => waited,
        }
    }
}
pub struct Worker {
    handle: Option<JoinHandle<()>>,
    shutdown: Shutdown,
    wake: WakePath,
    exited: Option<Receiver<()>>,
    name: &'static str,
}
impl Worker {
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
    #[must_use]
    pub fn primitive(&self) -> &'static str {
        self.wake.primitive()
    }
    #[must_use]
    pub fn stop(mut self) -> bool {
        self.stop_inner(JOIN_DEADLINE).is_clean()
    }
    #[must_use]
    pub fn stop_within(mut self, deadline: Duration) -> Stopped {
        self.stop_inner(deadline)
    }
    fn stop_inner(&mut self, deadline: Duration) -> Stopped {
        self.shutdown.signal();
        let started = Instant::now();
        self.wake.wake();
        let in_time = match self.exited.take() {
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
        let _ = self.stop_inner(JOIN_DEADLINE);
    }
}
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
        }
        assert!(flag.load(Ordering::SeqCst), "drop signalled AND waited");
    }
    #[test]
    fn wakes_a_worker_blocked_in_recv() {
        let (tx, rx) = channel::<u32>();
        let woke = Arc::new(AtomicBool::new(false));
        let w2 = woke.clone();
        let w = Worker::spawn(
            "recv-blocked",
            WakePath::interrupt("mpsc::recv", move || {
                let _ = tx.send(0);
            }),
            move |sd| {
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
    #[test]
    fn a_wake_that_does_not_reach_the_worker_is_reported() {
        let (tx, rx) = channel::<u32>();
        let w = Worker::spawn("unreachable-wake", WakePath::polling(), move |_sd| {
            let _ = rx.recv();
        })
        .unwrap();
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
