// SPDX-License-Identifier: MIT OR Apache-2.0
//! A process-wide stop flag that `SIGINT` and `SIGTERM` set.
#![no_std]
use core::ffi::c_int;
use core::sync::atomic::{AtomicI32, Ordering};
pub const SIGINT: c_int = 2;
pub const SIGTERM: c_int = 15;
static SIGNAL: AtomicI32 = AtomicI32::new(0);
extern "C" {
    #[link_name = "signal"]
    fn c_signal(sig: c_int, handler: extern "C" fn(c_int)) -> usize;
}
extern "C" fn on_signal(sig: c_int) {
    let _ = SIGNAL.compare_exchange(0, sig, Ordering::SeqCst, Ordering::SeqCst);
}
/// # Errors
/// The number of the signal whose `signal()` call failed.
pub fn install() -> Result<(), c_int> {
    for sig in [SIGTERM, SIGINT] {
        // SAFETY: `on_signal` only does an atomic compare-exchange, which is
        // async-signal-safe. `signal` takes and returns plain values.
        if unsafe { c_signal(sig, on_signal) } == usize::MAX {
            return Err(sig);
        }
    }
    Ok(())
}
pub fn request() {
    let _ = SIGNAL.compare_exchange(0, -1, Ordering::SeqCst, Ordering::SeqCst);
}
#[must_use]
pub fn requested() -> bool {
    SIGNAL.load(Ordering::SeqCst) != 0
}
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
