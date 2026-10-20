// SPDX-License-Identifier: Apache-2.0
//! One process-wide signal that something happened on the USB stack: an
//! interrupt IN completion ([`crate::IntrIn`]), an insertion or a removal
//! ([`crate::Host`]). A driver that serves several pipes and the hot-plug
//! events on one thread sleeps in [`wait`] and then collects from each pipe
//! with a zero timeout, instead of polling them in turn.
//!
//! The signal is a counter under a mutex with a condition variable.
//! [`signal`] takes the mutex for one increment and a notify: it never
//! blocks on a transfer, so it is allowed on libusbdi's event thread (the
//! rule in [`crate::events`]). A caller remembers the counter it has seen
//! ([`seen`]) before it collects, and waits for it to move; nothing that
//! happens between the two is missed.
//!
//! The wait ends early, with the counter unchanged, when a stop is
//! requested ([`crate::stop`]): a signal handler can only set a flag, so
//! the wait looks at it every [`crate::bulk::STOP_SLICE_MS`], as every
//! interruptible wait of this crate does.
//!
//! Copyright 2026. Licensed under the Apache License, Version 2.0.

use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::bulk::STOP_SLICE_MS;
use crate::stop;

static COUNT: Mutex<u64> = Mutex::new(0);
static MOVED: Condvar = Condvar::new();

/// Something happened: wake every [`wait`].
pub fn signal() {
    let mut count = COUNT.lock().unwrap_or_else(PoisonError::into_inner);
    *count = count.wrapping_add(1);
    MOVED.notify_all();
}

/// The counter now. Read it before collecting, then [`wait`] for it to move.
#[must_use]
pub fn seen() -> u64 {
    *COUNT.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Sleep until the counter is no longer `seen`, `timeout` passes (`None`: no
/// bound) or a stop is requested. Returns the counter.
#[must_use]
pub fn wait(seen: u64, timeout: Option<Duration>) -> u64 {
    let deadline = timeout.map(|t| Instant::now() + t);
    let mut count = COUNT.lock().unwrap_or_else(PoisonError::into_inner);
    while *count == seen && !stop::requested() {
        let slice = Duration::from_millis(STOP_SLICE_MS);
        let step = match deadline {
            Some(d) => {
                let left = d.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                left.min(slice)
            }
            None => slice,
        };
        count = MOVED
            .wait_timeout(count, step)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
    *count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_from_another_thread_ends_the_wait_and_a_bound_ends_it_too() {
        // Other tests of the crate push events, which signal too: the
        // counter is only ever compared with what this test saw.
        let started = Instant::now();
        let _ = wait(seen(), Some(Duration::from_millis(30)));
        assert!(started.elapsed() < Duration::from_millis(1000));
        let before = seen();
        let signaller = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(40));
            signal();
        });
        let started = Instant::now();
        let after = wait(before, None);
        assert!(started.elapsed() < Duration::from_millis(1000));
        signaller.join().unwrap();
        if stop::requested() {
            // Another test of the crate set the stop flag: then the wait
            // returns at once by design, and the rest does not apply.
            return;
        }
        assert_ne!(after, before);
        // Already moved: no wait at all.
        let started = Instant::now();
        assert_ne!(wait(before, Some(Duration::from_secs(5))), before);
        assert!(started.elapsed() < Duration::from_millis(5));
    }
}
