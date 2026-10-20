// SPDX-License-Identifier: MIT OR Apache-2.0
//! The slogger's own protocol, written directly to `/dev/slog`.
//!
//! The QNX 6.5 libc keeps slogf/vslogf/slogb in the static libc.a only, so a
//! stand-alone program used to link slog.o out of the SDP. This module needs
//! nothing from it. What libc's slogb/vslogf send (read from the SDP's
//! slog.o) is one `_IO_WRITE` message on the `/dev/slog` file
//! descriptor whose payload is:
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
use core::sync::atomic::{AtomicI32, Ordering};

use crate::sys;

/// The text marker libc's vslogf sets in the severity word.
const SLOG_TEXT: c_int = 8;
/// libc's vslogf formats into 1016 bytes, NUL included.
const MAX_TEXT: usize = 1016;
const HEADER: usize = 12;

/// The `/dev/slog` descriptor, -1 while closed.
static FD: AtomicI32 = AtomicI32::new(-1);

fn open_fd() -> c_int {
    let fd = FD.load(Ordering::Acquire);
    if fd >= 0 {
        return fd;
    }
    // SAFETY: NUL-terminated path, write-only.
    let fd = unsafe { sys::open(c"/dev/slog".as_ptr(), sys::O_WRONLY) };
    if fd >= 0 {
        FD.store(fd, Ordering::Release);
    }
    fd
}

fn reopen() -> c_int {
    let old = FD.swap(-1, Ordering::AcqRel);
    if old >= 0 {
        // SAFETY: a descriptor this module opened.
        unsafe { sys::close(old) };
    }
    open_fd()
}

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

fn send(bytes: &[u8]) -> bool {
    let mut fd = open_fd();
    if fd < 0 {
        return false;
    }
    for _ in 0..2 {
        // SAFETY: an open descriptor and a valid buffer of this length.
        let r = unsafe { sys::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        // The slogger's reply count is not the buffer length; only -1 is a
        // failure, as in libc's slogsend. Measured on the Pi: checking for
        // the full length retried and logged every entry twice.
        if r >= 0 {
            return true;
        }
        fd = reopen();
        if fd < 0 {
            return false;
        }
    }
    false
}

/// Send one text entry with the given slog code and severity. Returns false
/// when `/dev/slog` cannot be opened or written.
#[must_use]
pub fn write_text(code: c_int, sev: c_int, line: &CStr) -> bool {
    let mut buf = [0u8; HEADER + MAX_TEXT];
    let n = encode(&mut buf, code, sev, line);
    send(&buf[..n])
}

#[cfg(test)]
mod tests {
    use super::*;

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
