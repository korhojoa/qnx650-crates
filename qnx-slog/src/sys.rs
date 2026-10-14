// SPDX-License-Identifier: MIT OR Apache-2.0
use core::ffi::{c_char, c_int, c_void};
pub use qnx_dl::{
    dl_error, dlclose, dlerror, dlopen, dlsym, sym_default, Next, RTLD_DEFAULT, RTLD_GLOBAL,
    RTLD_LAZY, RTLD_LOCAL, RTLD_NEXT, RTLD_NODELETE, RTLD_NOW,
};
pub use qnx_time::{
    clock_gettime, gmtime_r, localtime_r, mono_ms, now_ms, time, wall_ms, Timespec, Tm,
    CLOCK_MONOTONIC, CLOCK_REALTIME,
};
pub const F_OK: c_int = 0;
pub const R_OK: c_int = 4;
pub const O_WRONLY: c_int = 1;
pub const PTHREAD_MUTEX_RECURSIVE: c_int = 2;
pub const SLOG_SHUTDOWN: c_int = 0;
pub const SLOG_CRITICAL: c_int = 1;
pub const SLOG_ERROR: c_int = 2;
pub const SLOG_WARNING: c_int = 3;
pub const SLOG_NOTICE: c_int = 4;
pub const SLOG_INFO: c_int = 5;
pub const SLOG_DEBUG1: c_int = 6;
pub const SLOG_DEBUG2: c_int = 7;
pub const SLOG_SEVMAXVAL: c_int = 7;
pub const SLOGC_GENL: c_int = 0;
pub const SLOGC_PRIVATE_START: c_int = 10000;
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
