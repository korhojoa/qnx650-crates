// SPDX-License-Identifier: Apache-2.0
//! The one clock every log line and every deadline in the port reads.
//!
//! `init()` sets `T0` once. The first `now_ms()` call also initializes it.
//! A caller can thus use timestamps without an explicit initialization call.
//! QNX `Instant` uses `CLOCK_MONOTONIC` (2).
//! `CLOCK_THREAD_CPUTIME_ID` (4) measures processor time for one thread.
//! Condition-variable deadlines also use `CLOCK_MONOTONIC`.
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
//! Copyright 2026 qnx650-crates contributors. Licensed under Apache-2.0.

use core::fmt;
#[cfg(not(target_os = "nto"))]
use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::ffi::EBUSY;
use crate::{Error, Result};

static T0: OnceLock<Instant> = OnceLock::new();
static LEVEL: AtomicU8 = AtomicU8::new(0);
static LOG_SINK: OnceLock<fn(&str)> = OnceLock::new();

/// Add a process log callback before USB workers start.
/// The callback receives the stamped line after the default output.
/// It must not call this logger recursively.
///
/// # Errors
/// Returns the supplied callback if one is already installed.
pub fn set_log_sink(sink: fn(&str)) -> core::result::Result<(), fn(&str)> {
    LOG_SINK.set(sink)
}

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

/// On QNX, write a stamped line to file descriptor 2 and slog.
/// On other targets, write the line through the standard-error lock.
/// Also pass it to the optional process callback. Output failure does not
/// panic inside a USB completion callback.
pub fn log(args: fmt::Arguments<'_>) {
    let line = format!("{} {args}", stamp());
    #[cfg(target_os = "nto")]
    qnx_slog::log::app_line(
        qnx_slog::sys::SLOGC_PRIVATE_START,
        qnx_slog::sys::SLOG_INFO,
        format_args!("{line}"),
    );
    #[cfg(not(target_os = "nto"))]
    {
        let mut err = std::io::stderr().lock();
        let _ = writeln!(err, "{line}");
    }
    if let Some(sink) = LOG_SINK.get() {
        sink(&line);
    }
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
    fn installed_sink_receives_stamped_lines_and_cannot_be_replaced() {
        static RECORDS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
        fn capture(line: &str) {
            RECORDS.lock().unwrap().push(line.to_owned());
        }
        assert!(set_log_sink(capture).is_ok());
        log(format_args!("owned callback marker"));
        assert_eq!(
            RECORDS
                .lock()
                .unwrap()
                .iter()
                .filter(|line| {
                    line.starts_with('[') && line.ends_with("] owned callback marker")
                })
                .count(),
            1
        );
        assert!(set_log_sink(capture).is_err());
    }

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
