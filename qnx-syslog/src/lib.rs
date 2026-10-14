// SPDX-License-Identifier: Apache-2.0
//! Log lines of a stand-alone QNX 6.5 executable: each line goes to the
//! system log through `syslog()` and to stderr with a `[ssss.mmm]` stamp.
use std::ffi::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Instant;
pub const LOG_ERR: c_int = 3;
pub const LOG_WARNING: c_int = 4;
pub const LOG_INFO: c_int = 6;
pub const LOG_USER: c_int = 1 << 3;
#[cfg(target_os = "nto")]
extern "C" {
    fn syslog(priority: c_int, fmt: *const u8, ...);
}
/// # Safety
/// The function has the C signature.
/// The function does nothing.
#[cfg(not(target_os = "nto"))]
unsafe fn syslog(_priority: c_int, _fmt: *const u8, _arg: *const u8) {}
static TRACE: AtomicBool = AtomicBool::new(false);
static ORIGIN: OnceLock<Instant> = OnceLock::new();
#[must_use]
pub fn elapsed_ms() -> u128 {
    ORIGIN.get_or_init(Instant::now).elapsed().as_millis()
}
fn stamp() -> String {
    let ms = elapsed_ms();
    format!("[{:04}.{:03}]", ms / 1000, ms % 1000)
}
pub fn set_trace(on: bool) {
    TRACE.store(on, Ordering::Relaxed);
}
pub fn trace_enabled() -> bool {
    TRACE.load(Ordering::Relaxed)
}
fn c_line(msg: &str) -> Vec<u8> {
    let mut buf: Vec<u8> = msg
        .as_bytes()
        .iter()
        .map(|&b| if b == 0 { b'?' } else { b })
        .collect();
    buf.push(0);
    buf
}
fn emit(level: c_int, msg: &str) {
    let buf = c_line(msg);
    // SAFETY: `buf` is NUL-terminated and stays live until the call returns.
    // The literal format string has one `%s` for one pointer.
    // A `%` in the message is not a format directive.
    unsafe {
        syslog(LOG_USER | level, c"%s".as_ptr().cast::<u8>(), buf.as_ptr());
    }
    eprintln!("{} {msg}", stamp());
}
pub fn info(msg: &str) {
    emit(LOG_INFO, msg);
}
pub fn warn(msg: &str) {
    emit(LOG_WARNING, msg);
}
pub fn error(msg: &str) {
    emit(LOG_ERR, msg);
}
pub fn trace(msg: &str) {
    if trace_enabled() {
        emit(LOG_INFO, msg);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_syslog_copy_is_terminated_and_keeps_no_interior_nul() {
        assert_eq!(c_line("a\0b%s"), b"a?b%s\0");
        assert_eq!((LOG_USER | LOG_ERR, LOG_WARNING, LOG_INFO), (11, 4, 6));
    }
    #[test]
    fn the_stamp_is_seconds_and_milliseconds_since_the_first_line() {
        let s = stamp();
        assert_eq!(s.len(), "[0000.000]".len(), "{s}");
        assert!(s.starts_with("[0000."), "{s}");
        set_trace(true);
        assert!(trace_enabled());
        set_trace(false);
        assert!(!trace_enabled());
    }
}
