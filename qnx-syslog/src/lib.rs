// SPDX-License-Identifier: Apache-2.0
//! Log lines of a stand-alone QNX 6.5 executable: each line goes to the
//! system log through `syslog()` and to stderr with a `[ssss.mmm]` stamp.
//!
//! Why `syslog` and not `slogf`: `slogf` is not exported by `libc.so.3`
//! on this SDP; it exists only in the static `libc.a` and `libcS.a`. A
//! driver shared object gets away with calling it because the statically
//! linked process that loads it (`io-usb`, `io-audio`) resolves it. A
//! dynamically linked executable does not. Linking `libcS.a` to satisfy
//! it is worse than it looks: the archive also carries `dlopen`, the
//! linker pulls that in too, and a statically linked `dlopen` has no
//! loader state and can dereference a null pointer. `syslog` is exported
//! by `libc.so.3` and routes messages into the same slogger, so `sloginfo` still
//! shows the lines. (`qnx-slog` is the other route: `slogf` found at run
//! time in a host process, or the slogger protocol on `/dev/slog`, with no
//! allocation.)
//!
//! Every stderr line is stamped `[ssss.mmm]` since the process's first log
//! line, on the monotonic clock (`Instant`) the code's own timeouts use,
//! so a logged event and the timeout that fires can be lined up. The
//! system-log copy gets the slogger's own wall-clock stamp.
//!
//! [`trace`] is gated at run time ([`set_trace`]): per-operation tracing
//! is disabled by default to keep normal logs readable.
//!
//! Uses the shared system-log interface.

use std::ffi::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

// The syslog.h values of SDP 6.5.0, measured with the compiler
// image's arm-unknown-nto-qnx6.5.0eabi-gcc -S against the SDP headers
// (the probe method of qnx-mmio/LAYOUT.md): LOG_ERR 3, LOG_WARNING 4,
// LOG_INFO 6, LOG_USER 8. The same on x86 (qnx-rpi abi-check.c).

/// `LOG_ERR` (`syslog.h`).
pub const LOG_ERR: c_int = 3;
/// `LOG_WARNING` (`syslog.h`).
pub const LOG_WARNING: c_int = 4;
/// `LOG_INFO` (`syslog.h`).
pub const LOG_INFO: c_int = 6;
/// `LOG_USER` (`syslog.h`): `1 << 3`.
pub const LOG_USER: c_int = 1 << 3;

#[cfg(target_os = "nto")]
extern "C" {
    fn syslog(priority: c_int, fmt: *const u8, ...);
}

/// Host builds keep the stderr copy and skip the system log: `syslog` is
/// a real glibc symbol, and a stub of it would shadow libc for the whole
/// test process.
///
/// # Safety
/// Signature-compatible with the real one; does nothing.
#[cfg(not(target_os = "nto"))]
unsafe fn syslog(_priority: c_int, _fmt: *const u8, _arg: *const u8) {}

static TRACE: AtomicBool = AtomicBool::new(false);

/// The process's log clock origin: the first line's instant.
static ORIGIN: OnceLock<Instant> = OnceLock::new();

/// Milliseconds since the first log line of this process.
#[must_use]
pub fn elapsed_ms() -> u128 {
    ORIGIN.get_or_init(Instant::now).elapsed().as_millis()
}

/// `[ssss.mmm]`, the stamp every stderr line starts with.
fn stamp() -> String {
    let ms = elapsed_ms();
    format!("[{:04}.{:03}]", ms / 1000, ms % 1000)
}

/// Turn per-operation tracing on or off.
pub fn set_trace(on: bool) {
    TRACE.store(on, Ordering::Relaxed);
}

/// Whether [`trace`] lines are written.
pub fn trace_enabled() -> bool {
    TRACE.load(Ordering::Relaxed)
}

/// The NUL-terminated copy of `msg` that `syslog` gets: an interior NUL
/// would cut the line, so it becomes `?`.
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
    // SAFETY: buf is NUL-terminated and outlives the call; the format
    // string is a literal with exactly one %s consuming one pointer, so a
    // '%' in the message is not read as a directive.
    unsafe {
        syslog(LOG_USER | level, c"%s".as_ptr().cast::<u8>(), buf.as_ptr());
    }
    eprintln!("{} {msg}", stamp());
}

/// An informational line.
pub fn info(msg: &str) {
    emit(LOG_INFO, msg);
}

/// A warning line.
pub fn warn(msg: &str) {
    emit(LOG_WARNING, msg);
}

/// An error line.
pub fn error(msg: &str) {
    emit(LOG_ERR, msg);
}

/// A per-operation tracing line: written only after `set_trace(true)`.
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
