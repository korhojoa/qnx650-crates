// SPDX-License-Identifier: MIT OR Apache-2.0
//! The slogger's own protocol, written directly to `/dev/slog`.
use crate::sys;
use core::ffi::{c_int, CStr};
use core::sync::atomic::{AtomicI32, Ordering};
const SLOG_TEXT: c_int = 8;
const MAX_TEXT: usize = 1016;
const HEADER: usize = 12;
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
