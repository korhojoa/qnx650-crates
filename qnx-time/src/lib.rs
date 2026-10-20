// SPDX-License-Identifier: MIT OR Apache-2.0
//! QNX 6.5 clocks for code without std: the clock ids, `struct timespec`,
//! `struct tm`, the libc time calls, and millisecond readings of the
//! monotonic and wall clocks.
//!
//! The clock ids are QNX's (`time.h` lines 100-109): `CLOCK_MONOTONIC` is
//! 2, the clock Rust's `Instant` reads there, and 4 is
//! `CLOCK_THREAD_CPUTIME_ID`, not a monotonic clock. Sizes from the SDP
//! compiler: `struct timespec` 8, `struct tm` 44, `time_t` 4. A host build
//! (tests) uses the Linux clock ids.
//!
//! `#![no_std]`, no allocation.

#![no_std]

use core::ffi::{c_char, c_int, c_long};

/// The wall clock (1970 until the unit has GPS time).
pub const CLOCK_REALTIME: c_int = 0;
/// The monotonic clock.
#[cfg(target_os = "nto")]
pub const CLOCK_MONOTONIC: c_int = 2;
/// The monotonic clock (Linux value, host builds).
#[cfg(not(target_os = "nto"))]
pub const CLOCK_MONOTONIC: c_int = 1;
/// The calling thread's CPU time.
#[cfg(target_os = "nto")]
pub const CLOCK_THREAD_CPUTIME_ID: c_int = 4;
/// The calling thread's CPU time (Linux value, host builds).
#[cfg(not(target_os = "nto"))]
pub const CLOCK_THREAD_CPUTIME_ID: c_int = 3;

/// `struct timespec`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timespec {
    pub tv_sec: c_long,
    pub tv_nsec: c_long,
}

/// `struct tm`: nine ints, `tm_gmtoff`, `tm_zone` (44 bytes on QNX 6.5).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Tm {
    pub tm_sec: c_int,
    pub tm_min: c_int,
    pub tm_hour: c_int,
    pub tm_mday: c_int,
    pub tm_mon: c_int,
    pub tm_year: c_int,
    pub tm_wday: c_int,
    pub tm_yday: c_int,
    pub tm_isdst: c_int,
    pub tm_gmtoff: c_long,
    pub tm_zone: *const c_char,
}

impl Tm {
    /// All fields zero.
    #[must_use]
    pub const fn zeroed() -> Tm {
        Tm {
            tm_sec: 0,
            tm_min: 0,
            tm_hour: 0,
            tm_mday: 0,
            tm_mon: 0,
            tm_year: 0,
            tm_wday: 0,
            tm_yday: 0,
            tm_isdst: 0,
            tm_gmtoff: 0,
            tm_zone: core::ptr::null(),
        }
    }
}

#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::size_of::<Tm>() == 44 && core::mem::size_of::<Timespec>() == 8);

extern "C" {
    pub fn clock_gettime(clk: c_int, ts: *mut Timespec) -> c_int;
    pub fn localtime_r(t: *const c_long, out: *mut Tm) -> *mut Tm;
    pub fn gmtime_r(t: *const c_long, out: *mut Tm) -> *mut Tm;
    pub fn time(t: *mut c_long) -> c_long;
}

/// `clock_gettime(clk)`, or zero when the call fails.
#[must_use]
pub fn now(clk: c_int) -> Timespec {
    let mut ts = Timespec::default();
    // SAFETY: a valid out-pointer.
    if unsafe { clock_gettime(clk, &raw mut ts) } != 0 {
        return Timespec::default();
    }
    ts
}

/// `clk` in milliseconds. Neither the wall nor the monotonic clock is
/// before the epoch, so a negative field (a failed call) reads as 0.
#[must_use]
pub fn clock_ms(clk: c_int) -> u64 {
    let ts = now(clk);
    let secs = u64::try_from(ts.tv_sec).unwrap_or(0);
    let nanos = u64::try_from(ts.tv_nsec).unwrap_or(0);
    secs * 1000 + nanos / 1_000_000
}

/// Return monotonic milliseconds as a wrapping 32-bit counter.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    reason = "a wrapping 32-bit millisecond counter: the low bits are the value"
)]
pub fn now_ms() -> u32 {
    clock_ms(CLOCK_MONOTONIC) as u32
}

/// `CLOCK_MONOTONIC` in milliseconds, the clock Rust's `Instant` reads.
#[must_use]
pub fn mono_ms() -> u64 {
    clock_ms(CLOCK_MONOTONIC)
}

/// `CLOCK_REALTIME` in milliseconds since the epoch.
#[must_use]
pub fn wall_ms() -> u64 {
    clock_ms(CLOCK_REALTIME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_does_not_go_back_and_wall_is_after_2020() {
        let a = mono_ms();
        let b = mono_ms();
        assert!(b >= a);
        assert!(wall_ms() > 1_577_836_800_000);
    }

    #[test]
    fn gmtime_splits_a_known_second() {
        let t: c_long = 86_400 + 3_661;
        let mut tm = Tm::zeroed();
        // SAFETY: valid pointers.
        assert!(!unsafe { gmtime_r(&raw const t, &raw mut tm) }.is_null());
        assert_eq!(
            (tm.tm_year, tm.tm_mon, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec),
            (70, 0, 2, 1, 1, 1)
        );
    }
}
