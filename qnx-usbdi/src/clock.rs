// SPDX-License-Identifier: Apache-2.0
//! The one clock every log line and every deadline in the port reads.
//!
//! `T0` is set once by `init()` (the first statement of `dlotest::main`)
//! or lazily by the first `now_ms()`, so a library caller that never calls
//! `init()` still gets stamps that count from its first log line rather
//! than a panic. `Instant` on the nto target is `CLOCK_MONOTONIC`
//! (`time.h` line 105: `CLOCK_MONOTONIC 2`; NOT 4, which is
//! `CLOCK_THREAD_CPUTIME_ID` and is what `qnx_mbox::now_ms` reads), and
//! std's `Condvar` there is created with
//! `pthread_condattr_setclock(CLOCK_MONOTONIC)`, so the bulk ring's
//! condvar deadlines and these stamps share one clock: a logged
//! completion and the timeout that would have fired can be lined up.
//!
//! `usbd_io`'s own `timeout_ms` is io-usb's timer in another process. It
//! is never reported as a duration; every transfer logs the
//! `Instant`-measured elapsed time next to the timeout it passed.
//!
//! Verbosity is a process-wide level (`set_verbosity`); `vlog!` lines
//! (one per transfer, per slot transition) print at level 1 and above,
//! `log!` lines (state changes, every failure) always. Both carry the
//! `[ssss.mmm]` stamp; nothing in the crate prints a line without one.
//!
//! `retry_while_busy` is the one bounded retry the teardown uses
//! (contract item 9): libusbdi answers `EBUSY` from `usbd_close_pipe`
//! and `usbd_detach` until its event thread, back from the last
//! completion callback, has decremented its own counters.
//!
//! Copyright 2026. Licensed under the Apache License, Version 2.0.

use core::fmt;
use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::ffi::EBUSY;
use crate::{Error, Result};

static T0: OnceLock<Instant> = OnceLock::new();
static LEVEL: AtomicU8 = AtomicU8::new(0);

/// Bound on the wait for libusbdi's own in-flight accounting to catch
/// up with a drained ring. The per-pipe and per-device counters are
/// decremented by the event thread a few instructions after the client
/// callback returned, so the window is microseconds unless that thread
/// is descheduled; one second is generous and keeps a stuck teardown
/// bounded.
pub(crate) const SETTLE_TIMEOUT_MS: u32 = 1000;

/// Pause between two attempts of [`retry_while_busy`].
pub(crate) const SETTLE_STEP_MS: u64 = 1;

/// Call `op` until it stops failing with `EBUSY` or `timeout_ms` has
/// passed, sleeping [`SETTLE_STEP_MS`] between attempts; any other
/// outcome is returned at once. A call that needed retries is logged
/// at level 0 with their count and the measured wait, so the diagnostic
/// record shows every time the window was hit. `timeout_ms == 0` makes
/// exactly one attempt.
///
/// # Errors
/// The last error `op` returned: `Error::Errno(EBUSY)` when the bound
/// expired, or whatever else `op` failed with first.
pub(crate) fn retry_while_busy<T>(
    what: &str,
    timeout_ms: u32,
    mut op: impl FnMut() -> Result<T>,
) -> Result<T> {
    let started = Instant::now();
    let deadline = started + Duration::from_millis(u64::from(timeout_ms));
    let mut retries: u32 = 0;
    loop {
        let result = op();
        let busy = matches!(result, Err(Error::Errno(EBUSY)));
        if busy && Instant::now() < deadline {
            retries += 1;
            std::thread::sleep(Duration::from_millis(SETTLE_STEP_MS));
            continue;
        }
        if retries > 0 {
            crate::log!(
                "usbdi: {what}: {} after {retries} EBUSY retry(ies), {} ms (bound {timeout_ms} ms)",
                if busy { "still EBUSY" } else { "settled" },
                elapsed_ms(started)
            );
        }
        return result;
    }
}

/// Pin the clock origin to now. A second call is a no-op: the origin is
/// the first one, so stamps never jump backwards.
pub fn init() {
    let _ = T0.set(Instant::now());
}

/// Milliseconds since the origin (lazily set if `init()` never ran).
#[must_use]
pub fn now_ms() -> u64 {
    let elapsed = T0.get_or_init(Instant::now).elapsed();
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

/// Milliseconds elapsed since `since`, on the same clock as `now_ms()`.
#[must_use]
pub fn elapsed_ms(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The `[ssss.mmm]` prefix for the current instant.
#[must_use]
pub fn stamp() -> String {
    format_stamp(now_ms())
}

/// The prefix for a given millisecond count; separated so the format is
/// testable without a clock.
#[must_use]
pub fn format_stamp(ms: u64) -> String {
    format!("[{:04}.{:03}]", ms / 1000, ms % 1000)
}

/// Write one stamped line to stderr. A closed stderr is not a reason to
/// panic inside a USB completion callback, so the write error is dropped.
pub fn log(args: fmt::Arguments<'_>) {
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "{} {args}", stamp());
}

/// Set the verbosity level: 0 prints state changes and failures, 1 and
/// above also print one line per transfer and per ring slot transition.
pub fn set_verbosity(level: u8) {
    LEVEL.store(level, Ordering::Relaxed);
}

/// The current verbosity level.
#[must_use]
pub fn verbosity() -> u8 {
    LEVEL.load(Ordering::Relaxed)
}

/// Write a stamped line only at verbosity 1 and above.
pub fn verbose(args: fmt::Arguments<'_>) {
    if verbosity() >= 1 {
        log(args);
    }
}

/// A stamped line, always printed.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::clock::log(format_args!($($arg)*))
    };
}

/// A stamped line, printed at verbosity 1 and above. The arguments are
/// not formatted when the level is 0.
#[macro_export]
macro_rules! vlog {
    ($($arg:tt)*) => {
        if $crate::clock::verbosity() >= 1 {
            $crate::clock::log(format_args!($($arg)*));
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_format_is_seconds_dot_millis() {
        assert_eq!(format_stamp(0), "[0000.000]");
        assert_eq!(format_stamp(1_234), "[0001.234]");
        assert_eq!(format_stamp(65_001), "[0065.001]");
        assert_eq!(format_stamp(12_345_678), "[12345.678]");
    }

    #[test]
    fn clock_is_monotonic_and_init_is_idempotent() {
        init();
        let a = now_ms();
        init();
        let b = now_ms();
        assert!(b >= a);
        assert!(stamp().starts_with('['));
    }

    #[test]
    fn verbosity_round_trips() {
        let before = verbosity();
        set_verbosity(2);
        assert_eq!(verbosity(), 2);
        set_verbosity(before);
    }

    /// The teardown retry: `EBUSY` is retried within the bound, any
    /// other outcome ends it at once, a zero bound is one attempt.
    #[test]
    fn retry_while_busy_retries_only_ebusy_within_the_bound() {
        let mut calls = 0;
        let settled = retry_while_busy("test", 1000, || {
            calls += 1;
            if calls < 3 {
                Err(Error::Errno(EBUSY))
            } else {
                Ok(calls)
            }
        });
        assert_eq!(settled, Ok(3));

        let mut calls = 0;
        let expired: Result<()> = retry_while_busy("test", 50, || {
            calls += 1;
            Err(Error::Errno(EBUSY))
        });
        assert_eq!(expired, Err(Error::Errno(EBUSY)));
        assert!(calls > 1, "{calls}");

        let mut calls = 0;
        let other: Result<()> = retry_while_busy("test", 1000, || {
            calls += 1;
            Err(Error::Errno(crate::ffi::EINVAL))
        });
        assert_eq!(other, Err(Error::Errno(crate::ffi::EINVAL)));
        assert_eq!(calls, 1);

        let mut calls = 0;
        let once: Result<()> = retry_while_busy("test", 0, || {
            calls += 1;
            Err(Error::Errno(EBUSY))
        });
        assert_eq!(once, Err(Error::Errno(EBUSY)));
        assert_eq!(calls, 1);
        assert_eq!(SETTLE_TIMEOUT_MS, 1000);
    }
}
