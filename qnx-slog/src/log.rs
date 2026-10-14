// SPDX-License-Identifier: MIT OR Apache-2.0
//! Log lines. Every line is built in a fixed stack buffer (no allocation, so
//! it is safe on a vendor process's reader and controller threads) and handed
//! to the sink as one "%s".
use crate::{sink, sys};
use core::ffi::c_int;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicU64, Ordering};
pub struct Line<const N: usize> {
    buf: [u8; N],
    len: usize,
}
impl<const N: usize> Line<N> {
    #[must_use]
    pub const fn new() -> Self {
        Line {
            buf: [0; N],
            len: 0,
        }
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        let b = &self.buf[..self.len];
        match core::str::from_utf8(b) {
            Ok(s) => s,
            Err(e) => core::str::from_utf8(&b[..e.valid_up_to()]).unwrap_or(""),
        }
    }
    pub fn c_ptr(&mut self) -> *const core::ffi::c_char {
        self.as_cstr().as_ptr()
    }
    pub fn as_cstr(&mut self) -> &core::ffi::CStr {
        let end = if self.len < N { self.len } else { N - 1 };
        self.buf[end] = 0;
        core::ffi::CStr::from_bytes_until_nul(&self.buf[..=end]).unwrap_or(c"")
    }
}
impl<const N: usize> Default for Line<N> {
    fn default() -> Self {
        Self::new()
    }
}
impl<const N: usize> Write for Line<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = (N - 1).saturating_sub(self.len);
        let n = s.len().min(room);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}
fn emit<const N: usize>(sev: c_int, line: &mut Line<N>) -> bool {
    sink::write(sev, line.as_cstr())
}
pub fn write_wall_stamp<W: Write>(w: &mut W) {
    let mut ts = sys::Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let mut tm = sys::Tm {
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
    };
    // SAFETY: valid out-pointers.
    unsafe {
        sys::clock_gettime(sys::CLOCK_REALTIME, &raw mut ts);
        let secs: core::ffi::c_long = ts.tv_sec;
        sys::localtime_r(&raw const secs, &raw mut tm);
    }
    let _ = write!(
        w,
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} ",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        ts.tv_nsec / 1_000_000
    );
}
pub fn write_time_stamp<W: Write>(w: &mut W) {
    let mut tm = sys::Tm {
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
    };
    // SAFETY: time() with a null argument only returns. gmtime_r gets a valid
    // input and a valid output.
    let (t, ok) = unsafe {
        let t = sys::time(core::ptr::null_mut());
        (t, !sys::gmtime_r(&raw const t, &raw mut tm).is_null())
    };
    let _ = if ok {
        write!(
            w,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    } else {
        write!(w, "@{t}")
    };
}
pub fn app_line(code: c_int, sev: c_int, args: fmt::Arguments<'_>) {
    let mut line: Line<4096> = Line::new();
    let _ = line.write_str("[");
    write_time_stamp(&mut line);
    let _ = line.write_str("] ");
    let _ = line.write_fmt(args);
    let _ = line.write_str("\n");
    let s = line.as_str();
    // SAFETY: fd 2 and a valid buffer. A failed write (no stderr) is ignored.
    unsafe { sys::write(2, s.as_ptr().cast(), s.len()) };
    let _ = sink::write_code(code, sev, line.as_cstr());
}
pub fn stamped(sev: c_int, args: fmt::Arguments<'_>) {
    let mut msg: Line<400> = Line::new();
    let _ = msg.write_fmt(args);
    let mut line: Line<448> = Line::new();
    write_wall_stamp(&mut line);
    let _ = line.write_str(msg.as_str());
    let _ = line.write_str("\n");
    emit(sev, &mut line);
}
pub fn raw(sev: c_int, args: fmt::Arguments<'_>) {
    let mut line: Line<512> = Line::new();
    let _ = line.write_fmt(args);
    emit(sev, &mut line);
}
#[must_use]
pub fn text(sev: c_int, s: &str) -> bool {
    text_code(sys::SLOGC_GENL, sev, s)
}
#[must_use]
pub fn text_code(code: c_int, sev: c_int, s: &str) -> bool {
    let mut line: Line<1024> = Line::new();
    let _ = line.write_str(s);
    sink::write_code(code, sev, line.as_cstr())
}
static MS_T0: AtomicU64 = AtomicU64::new(0);
pub fn ms_start() {
    let now = sys::mono_ms().max(1);
    let _ = MS_T0.compare_exchange(0, now, Ordering::AcqRel, Ordering::Acquire);
}
pub fn ms(sev: c_int, tag: &str, args: fmt::Arguments<'_>) {
    ms_start();
    let mono = sys::mono_ms().saturating_sub(MS_T0.load(Ordering::Acquire));
    let mut line: Line<1024> = Line::new();
    let _ = write!(line, "mono_ms={mono} wall_ms={} {tag}: ", sys::wall_ms());
    let _ = line.write_fmt(args);
    let _ = line.write_str("\n");
    let s = line.as_str();
    // SAFETY: fd 1 and a valid buffer. A failed write (no stdout) is ignored.
    unsafe { sys::write(1, s.as_ptr().cast(), s.len()) };
    emit(sev, &mut line);
}
#[macro_export]
macro_rules! ldr_log {
    ($($a:tt)*) => { $crate::log::stamped($crate::sys::SLOG_INFO, format_args!($($a)*)) };
}
#[macro_export]
macro_rules! ldr_loge {
    ($($a:tt)*) => { $crate::log::stamped($crate::sys::SLOG_ERROR, format_args!($($a)*)) };
}
#[macro_export]
macro_rules! slog_raw {
    ($($a:tt)*) => { $crate::log::raw($crate::sys::SLOG_ERROR, format_args!($($a)*)) };
}
#[macro_export]
macro_rules! ms_log {
    ($tag:expr, $($a:tt)*) => {
        $crate::log::ms($crate::sys::SLOG_NOTICE, $tag, format_args!($($a)*))
    };
}
pub fn write_hex<W: Write>(w: &mut W, bytes: &[u8]) {
    for b in bytes {
        let _ = write!(w, "{b:02x} ");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn line_truncates_and_terminates() {
        let mut l: Line<8> = Line::new();
        let _ = l.write_str("abcdefghijk");
        assert_eq!(l.as_str(), "abcdefg");
        let p = l.c_ptr();
        // SAFETY: c_ptr returns a pointer into l's buffer, NUL at index 7.
        let s = unsafe { core::ffi::CStr::from_ptr(p) };
        assert_eq!(s.to_bytes(), b"abcdefg");
    }
    #[test]
    fn hex_matches_printf_form() {
        let mut l: Line<64> = Line::new();
        write_hex(&mut l, &[0x00, 0x01, 0xab, 0xff]);
        assert_eq!(l.as_str(), "00 01 ab ff ");
    }
    #[test]
    fn time_stamp_has_the_iso_shape_without_a_zone() {
        let mut l: Line<64> = Line::new();
        write_time_stamp(&mut l);
        let s = l.as_str().as_bytes();
        assert_eq!(s.len(), 19, "{}", l.as_str());
        assert_eq!(
            (s[4], s[7], s[10], s[13], s[16]),
            (b'-', b'-', b'T', b':', b':')
        );
    }
    #[test]
    fn ms_origin_is_set_once() {
        ms_start();
        let a = MS_T0.load(Ordering::Acquire);
        ms_start();
        assert_eq!(a, MS_T0.load(Ordering::Acquire));
        assert!(a > 0);
    }
    #[test]
    fn no_sink_on_the_host_is_not_an_error() {
        let _ = text(sys::SLOG_INFO, "hello");
        assert_ne!(sink::describe(), "");
    }
}
