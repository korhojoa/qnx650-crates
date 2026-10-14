//! Copyright 2026. Licensed under the Apache License, Version 2.0.
use crate::ffi::EBUSY;
use crate::{Error, Result};
use core::fmt;
use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
static T0: OnceLock<Instant> = OnceLock::new();
static LEVEL: AtomicU8 = AtomicU8::new(0);
pub(crate) const SETTLE_TIMEOUT_MS: u32 = 1000;
pub(crate) const SETTLE_STEP_MS: u64 = 1;
/// # Errors
/// `Error::Errno(EBUSY)` when the time limit expired.
/// Otherwise, the first different error from `op`.
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
pub fn init() {
    let _ = T0.set(Instant::now());
}
#[must_use]
pub fn now_ms() -> u64 {
    let elapsed = T0.get_or_init(Instant::now).elapsed();
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}
#[must_use]
pub fn elapsed_ms(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}
#[must_use]
pub fn stamp() -> String {
    format_stamp(now_ms())
}
#[must_use]
pub fn format_stamp(ms: u64) -> String {
    format!("[{:04}.{:03}]", ms / 1000, ms % 1000)
}
pub fn log(args: fmt::Arguments<'_>) {
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "{} {args}", stamp());
}
pub fn set_verbosity(level: u8) {
    LEVEL.store(level, Ordering::Relaxed);
}
#[must_use]
pub fn verbosity() -> u8 {
    LEVEL.load(Ordering::Relaxed)
}
pub fn verbose(args: fmt::Arguments<'_>) {
    if verbosity() >= 1 {
        log(args);
    }
}
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::clock::log(format_args!($($arg)*))
    };
}
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
