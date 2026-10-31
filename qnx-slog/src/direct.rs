// SPDX-License-Identifier: MIT OR Apache-2.0
//! The slogger's own protocol, written directly to `/dev/slog`.
//!
//! This writer sends an `_IO_WRITE` message to `/dev/slog`.
//! It does not link the static slog implementation.
//! The message payload contains these fields:
//!
//! ```text
//! int32  severity   (bit 3, value 8, set for text: what vslogf adds)
//! int32  code       (_SLOG_SETCODE(major, minor) = major | minor << 20)
//! int32  reserved   (libc leaves it uninitialised; sent as 0 here)
//! bytes  text, NUL included, at most 1016 bytes
//! ```
//!
//! libc builds that message by hand with `MsgSendv` to avoid a copy. A plain
//! `write()` of the same bytes on the same descriptor is the same message
//! (`_IO_WRITE`, xtype none), so this writer is `open` + `write`. On a
//! failed write it reopens once and retries, as libc does on EBADF.

use core::ffi::{c_int, CStr};
use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use crate::sys;

/// The text marker libc's vslogf sets in the severity word.
const SLOG_TEXT: c_int = 8;
/// libc's vslogf formats into 1016 bytes, NUL included.
const MAX_TEXT: usize = 1016;
const HEADER: usize = 12;

struct Writer {
    fd: AtomicI32,
    busy: AtomicBool,
}

struct Held<'a>(&'a AtomicBool);
impl Drop for Held<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

trait FileOps {
    fn open(&self) -> c_int;
    fn write(&self, fd: c_int, bytes: &[u8]) -> bool;
    fn close(&self, fd: c_int);
}

struct Native;
impl FileOps for Native {
    fn open(&self) -> c_int {
        // SAFETY: NUL-terminated path, write-only.
        unsafe { sys::open(c"/dev/slog".as_ptr(), sys::O_WRONLY) }
    }
    fn write(&self, fd: c_int, bytes: &[u8]) -> bool {
        // SAFETY: the writer holds its descriptor gate and the buffer is valid.
        // The slogger reply count need not equal the input length.
        unsafe { sys::write(fd, bytes.as_ptr().cast(), bytes.len()) >= 0 }
    }
    fn close(&self, fd: c_int) {
        // SAFETY: the writer opened this descriptor and excludes other writes.
        unsafe { sys::close(fd) };
    }
}

impl Writer {
    const fn new() -> Self {
        Self {
            fd: AtomicI32::new(-1),
            busy: AtomicBool::new(false),
        }
    }

    fn send(&self, bytes: &[u8], ops: &impl FileOps) -> bool {
        while self
            .busy
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        let _held = Held(&self.busy);
        let mut fd = self.fd.load(Ordering::Relaxed);
        for _ in 0..2 {
            if fd < 0 {
                fd = ops.open();
                self.fd.store(fd, Ordering::Relaxed);
            }
            if fd < 0 {
                return false;
            }
            if ops.write(fd, bytes) {
                return true;
            }
            ops.close(fd);
            fd = -1;
            self.fd.store(fd, Ordering::Relaxed);
        }
        false
    }
}

static WRITER: Writer = Writer::new();

/// Encode one text entry: the 12-byte header and the NUL-terminated text,
/// cut at libc's limit. Returns the number of bytes to send.
pub fn encode(buf: &mut [u8; HEADER + MAX_TEXT], code: c_int, sev: c_int, line: &CStr) -> usize {
    let text = line.to_bytes_with_nul();
    let n = if text.len() > MAX_TEXT {
        MAX_TEXT
    } else {
        text.len()
    };
    buf[0..4].copy_from_slice(&(sev | SLOG_TEXT).to_ne_bytes());
    buf[4..8].copy_from_slice(&code.to_ne_bytes());
    buf[8..12].copy_from_slice(&0i32.to_ne_bytes());
    buf[HEADER..HEADER + n].copy_from_slice(&text[..n]);
    buf[HEADER + n - 1] = 0;
    HEADER + n
}

/// Send one text entry with the given slog code and severity. Returns false
/// when `/dev/slog` cannot be opened or written. Calls are serialized through
/// the write and any close/reopen. Do not call from a signal handler.
#[must_use]
pub fn write_text(code: c_int, sev: c_int, line: &CStr) -> bool {
    let mut buf = [0u8; HEADER + MAX_TEXT];
    let n = encode(&mut buf, code, sev, line);
    WRITER.send(&buf[..n], &Native)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_writers_hold_the_descriptor_through_retry() {
        use std::sync::{Arc, Mutex};
        #[derive(Default)]
        struct State {
            next: i32,
            live: Option<i32>,
            writes: usize,
            closes: usize,
        }
        #[derive(Default)]
        struct Mock(Mutex<State>);
        impl FileOps for Mock {
            fn open(&self) -> i32 {
                let mut state = self.0.lock().unwrap();
                assert!(state.live.is_none(), "no overwritten descriptor");
                state.next += 1;
                state.live = Some(state.next);
                state.next
            }
            fn write(&self, fd: i32, _: &[u8]) -> bool {
                std::thread::yield_now();
                let mut state = self.0.lock().unwrap();
                assert_eq!(state.live, Some(fd), "no closed or reused descriptor");
                state.writes += 1;
                // Each message fails once and then succeeds after reopen.
                state.writes & 1 == 0
            }
            fn close(&self, fd: i32) {
                let mut state = self.0.lock().unwrap();
                assert_eq!(state.live.take(), Some(fd));
                state.closes += 1;
            }
        }
        let writer = Arc::new(Writer::new());
        let ops = Arc::new(Mock::default());
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let writer = Arc::clone(&writer);
                let ops = Arc::clone(&ops);
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        assert!(writer.send(b"entry", ops.as_ref()));
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let state = ops.0.lock().unwrap();
        assert_eq!(state.writes, 1600);
        assert_eq!(state.closes, 800);
        assert_eq!(state.next, 801);
    }

    #[test]
    fn header_then_text_with_nul() {
        let mut buf = [0u8; HEADER + MAX_TEXT];
        let code = crate::sys::slog_code(crate::sys::SLOGC_PRIVATE_START, 1);
        let n = encode(&mut buf, code, 5, c"hello");
        assert_eq!(n, 12 + 6);
        assert_eq!(&buf[0..4], &(0x5_i32 | 0x8).to_ne_bytes());
        assert_eq!(&buf[4..8], &(10_000_i32 + (1 << 20)).to_ne_bytes());
        assert_eq!(&buf[8..12], &[0, 0, 0, 0]);
        assert_eq!(&buf[12..18], b"hello\0");
    }

    #[test]
    fn long_text_is_cut_at_the_libc_limit() {
        let text = [b'x'; 2000];
        let owned = std::ffi::CString::new(&text[..]).unwrap();
        let mut buf = [0u8; HEADER + MAX_TEXT];
        let n = encode(&mut buf, 0, 6, &owned);
        assert_eq!(n, HEADER + MAX_TEXT);
        assert_eq!(buf[HEADER + MAX_TEXT - 1], 0);
        assert_eq!(buf[HEADER + MAX_TEXT - 2], b'x');
    }
}
