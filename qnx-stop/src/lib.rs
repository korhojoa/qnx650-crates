// SPDX-License-Identifier: MIT OR Apache-2.0
//! A process-wide stop flag that `SIGINT` and `SIGTERM` set.
//!
//! A signal handler may only do async-signal-safe work, so the handler
//! [`install`] puts on both signals stores the signal number in one atomic
//! and does nothing else. The program's loops read [`requested`] (or
//! [`signal`], to log which one) and end in order: a daemon releases its
//! devices and shared memory instead of being killed with them held.
//!
//! One flag per process, never cleared: a stop is the end of the run.
//! `#![no_std]`, no allocation. Signal numbers are from the SDP
//! `signal.h` (lines 97 and 112).

#![no_std]

use core::ffi::c_int;
use core::sync::atomic::{AtomicI32, Ordering};

/// `SIGINT`: interrupt (Ctrl-C).
pub const SIGINT: c_int = 2;
/// `SIGTERM`: termination request (`kill`, `slay`, a deadline script).
pub const SIGTERM: c_int = 15;

/// 0 = no stop requested, else the signal number (or -1 for [`request`]).
static SIGNAL: AtomicI32 = AtomicI32::new(0);

extern "C" {
    /// `signal(3)`: the previous handler as an address; `SIG_ERR` is -1.
    #[link_name = "signal"]
    fn c_signal(sig: c_int, handler: extern "C" fn(c_int)) -> usize;
}

extern "C" fn on_signal(sig: c_int) {
    // The first signal is kept; a second one changes nothing.
    let _ = SIGNAL.compare_exchange(0, sig, Ordering::SeqCst, Ordering::SeqCst);
}

/// Route `SIGINT` and `SIGTERM` to the flag.
///
/// # Errors
/// The number of the signal whose `signal()` call failed.
pub fn install() -> Result<(), c_int> {
    for sig in [SIGTERM, SIGINT] {
        // SAFETY: `on_signal` only does an atomic compare-exchange, which is
        // async-signal-safe; `signal` takes and returns plain values.
        if unsafe { c_signal(sig, on_signal) } == usize::MAX {
            return Err(sig);
        }
    }
    Ok(())
}

/// Request a stop from the program itself. One atomic store, safe from a
/// signal handler.
pub fn request() {
    let _ = SIGNAL.compare_exchange(0, -1, Ordering::SeqCst, Ordering::SeqCst);
}

/// Whether a stop has been requested.
#[must_use]
pub fn requested() -> bool {
    SIGNAL.load(Ordering::SeqCst) != 0
}

/// The signal that requested the stop: `Some(SIGTERM)`, `Some(SIGINT)`,
/// `Some(-1)` for [`request`], `None` while no stop is requested.
#[must_use]
pub fn signal() -> Option<c_int> {
    match SIGNAL.load(Ordering::SeqCst) {
        0 => None,
        s => Some(s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // One process-wide flag, so one test covers the whole sequence.
    #[test]
    fn a_signal_sets_the_flag_once() {
        install().expect("signal()");
        assert!(!requested());
        assert_eq!(signal(), None);
        on_signal(SIGTERM);
        assert!(requested());
        on_signal(SIGINT);
        request();
        assert_eq!(signal(), Some(SIGTERM));
    }
}
