// SPDX-License-Identifier: MIT OR Apache-2.0
//! The QNX 6.5 libc surface these crates use, declared by hand (no libc crate
//! in the offline build). Constants and sizes are from the SDP headers in the
//! target compiler: pthread.h, unistd.h, and
//! `sizeof()` of `pthread_mutex_t` (8) and `pthread_mutexattr_t` (32) from
//! the gcc11 compiler. The dynamic linker calls come from qnx-dl and the
//! clocks from qnx-time.

use core::ffi::{c_char, c_int, c_void};

// The dynamic linker part lives in qnx-dl; re-exported so callers keep
// their `qnx_slog::sys::` paths.
#[cfg(feature = "alloc")]
pub use qnx_dl::dl_error;
pub use qnx_dl::{
    dl_error_into, dlclose, dlerror, dlopen, dlsym, sym_default, ErrorBufferTooSmall, Next,
    RTLD_DEFAULT, RTLD_GLOBAL, RTLD_LAZY, RTLD_LOCAL, RTLD_NEXT, RTLD_NODELETE, RTLD_NOW,
};

// The clocks live in qnx-time; re-exported for the same reason.
pub use qnx_time::{
    clock_gettime, gmtime_r, localtime_r, mono_ms, now_ms, time, wall_ms, Timespec, Tm,
    CLOCK_MONOTONIC, CLOCK_REALTIME,
};

pub const F_OK: c_int = 0;
pub const R_OK: c_int = 4;

/// fcntl.h: `O_WRONLY`, the mode slog.o opens /dev/slog with.
pub const O_WRONLY: c_int = 1;

pub const PTHREAD_MUTEX_RECURSIVE: c_int = 2;

// Slog severity values from sys/slog.h in the QNX 6.5 SDP.
// `severities_match_sys_slog_h` checks these values.
/// `_SLOG_SHUTDOWN`: shut down the system now.
pub const SLOG_SHUTDOWN: c_int = 0;
/// `_SLOG_CRITICAL`: unexpected unrecoverable error.
pub const SLOG_CRITICAL: c_int = 1;
/// `_SLOG_ERROR`: unexpected recoverable error.
pub const SLOG_ERROR: c_int = 2;
/// `_SLOG_WARNING`: expected error.
pub const SLOG_WARNING: c_int = 3;
/// `_SLOG_NOTICE`: a warning.
pub const SLOG_NOTICE: c_int = 4;
/// `_SLOG_INFO`: information.
pub const SLOG_INFO: c_int = 5;
/// `_SLOG_DEBUG1`: debug, normal detail.
pub const SLOG_DEBUG1: c_int = 6;
/// `_SLOG_DEBUG2`: debug, fine detail.
pub const SLOG_DEBUG2: c_int = 7;
/// `_SLOG_SEVMAXVAL`: the largest severity value.
pub const SLOG_SEVMAXVAL: c_int = 7;
/// Major code zero (`_SLOGC_GENL`) selects the general log category.
pub const SLOGC_GENL: c_int = 0;
/// _`SLOGC_PRIVATE_START`: the major-code range reserved for OEM applications
/// (sys/slogcodes.h). A code is `major | minor << 20` (_`SLOG_SETCODE`).
pub const SLOGC_PRIVATE_START: c_int = 10000;

/// _`SLOG_SETCODE(major`, minor).
#[must_use]
pub const fn slog_code(major: c_int, minor: c_int) -> c_int {
    major | (minor << 20)
}

extern "C" {
    pub fn access(path: *const c_char, mode: c_int) -> c_int;
    pub fn unlink(path: *const c_char) -> c_int;
    pub fn unsetenv(name: *const c_char) -> c_int;
    pub fn write(fd: c_int, buf: *const c_void, n: usize) -> isize;
    pub fn open(path: *const c_char, oflag: c_int, ...) -> c_int;
    pub fn close(fd: c_int) -> c_int;
    pub fn pthread_mutex_init(m: *mut c_void, attr: *const c_void) -> c_int;
    pub fn pthread_mutex_lock(m: *mut c_void) -> c_int;
    pub fn pthread_mutex_unlock(m: *mut c_void) -> c_int;
    pub fn pthread_mutexattr_init(a: *mut c_void) -> c_int;
    pub fn pthread_mutexattr_settype(a: *mut c_void, kind: c_int) -> c_int;
    pub fn pthread_mutexattr_destroy(a: *mut c_void) -> c_int;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every severity constant against the `#define` line of sys/slog.h
    /// (pristine QNX 6.5 SDP). A value here that drifts from the header is
    /// a line logged at the wrong level, which no build gate sees.
    #[test]
    fn severities_match_sys_slog_h() {
        let header: [(&str, c_int, c_int); 8] = [
            ("_SLOG_SHUTDOWN", 0, SLOG_SHUTDOWN),
            ("_SLOG_CRITICAL", 1, SLOG_CRITICAL),
            ("_SLOG_ERROR", 2, SLOG_ERROR),
            ("_SLOG_WARNING", 3, SLOG_WARNING),
            ("_SLOG_NOTICE", 4, SLOG_NOTICE),
            ("_SLOG_INFO", 5, SLOG_INFO),
            ("_SLOG_DEBUG1", 6, SLOG_DEBUG1),
            ("_SLOG_DEBUG2", 7, SLOG_DEBUG2),
        ];
        for (name, header_value, crate_value) in header {
            assert_eq!(crate_value, header_value, "{name}");
        }
        assert_eq!(SLOG_SEVMAXVAL, 0x7, "_SLOG_SEVMAXVAL");
        // The severity field is three bits (_SLOG_GETSEVERITY masks 0x7),
        // and the text marker direct.rs sets is the bit above it.
        for (_, v, _) in header {
            assert_eq!(v & 0x7, v);
            assert_eq!(v & 0x8, 0);
        }
    }

    #[test]
    fn slog_code_packs_major_and_minor() {
        assert_eq!(slog_code(SLOGC_PRIVATE_START, 1), 10_000 + (1 << 20));
        assert_eq!(slog_code(SLOGC_GENL, 0), 0);
    }
}
