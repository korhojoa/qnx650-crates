// SPDX-License-Identifier: MIT OR Apache-2.0
//! Where a finished line goes: the system log.
//!
//! slogf is NOT in the SDK's libc.so.3: it is slog.o in the static libc.a,
//! linked into each executable that calls it, and those copies are only
//! visible to other objects when something exports one. Two cases:
//!
//! - A library injected into a vendor process (`dio_manager`, gal): look slogf
//!   up in the process's default scope at run time, then vslogf (`dio_manager`
//!   imports vslogf).
//!   Those are the functions the vendor code logs with, so the lines land
//!   exactly as the vendor's do. When neither exists, fall back to the
//!   direct writer.
//! - A stand-alone program: the `static-slogf` feature selects the direct
//!   writer ([`crate::direct`]) only. It speaks the slogger's protocol on
//!   `/dev/slog` itself, so no program links anything out of the SDP.

use core::ffi::{c_char, c_int, c_void, CStr};
use core::sync::atomic::{AtomicPtr, AtomicU8, Ordering};

use crate::{direct, sys};

type SlogfFn = unsafe extern "C" fn(c_int, c_int, *const c_char, ...) -> c_int;
type VslogfFn = unsafe extern "C" fn(c_int, c_int, *const c_char, *const c_void) -> c_int;

static FUNC: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
/// 0 = not found yet, 1 = slogf, 2 = vslogf, 3 = the direct writer.
static KIND: AtomicU8 = AtomicU8::new(0);

fn resolve() -> (u8, *mut c_void) {
    if cfg!(feature = "static-slogf") {
        return (3, core::ptr::null_mut());
    }
    let k = KIND.load(Ordering::Acquire);
    if k != 0 {
        return (k, FUNC.load(Ordering::Acquire));
    }
    for (kind, name) in [(1u8, c"slogf"), (2u8, c"vslogf")] {
        let p = sys::sym_default(name);
        if !p.is_null() {
            FUNC.store(p, Ordering::Release);
            KIND.store(kind, Ordering::Release);
            return (kind, p);
        }
    }
    (3, core::ptr::null_mut())
}

/// Which sink lines go to, for a startup log line.
#[must_use]
pub fn describe() -> &'static str {
    match resolve().0 {
        1 => "slogf (found in the process)",
        2 => "vslogf (found in the process; no slogf)",
        _ => "/dev/slog (direct writer)",
    }
}

/// Send one line to slog as `"%s"` with major code 0. Returns false when
/// there is no sink. The caller adds any "\n".
#[must_use]
pub fn write(sev: c_int, line: &CStr) -> bool {
    write_code(sys::SLOGC_GENL, sev, line)
}

/// [`write()`] with the caller's slog code (`major | minor << 20`, e.g.
/// [`sys::SLOGC_PRIVATE_START`] for an application).
#[must_use]
pub fn write_code(code: c_int, sev: c_int, line: &CStr) -> bool {
    let text = line.as_ptr();
    match resolve() {
        (1, p) => {
            // SAFETY: the process's slogf.
            let f = unsafe { core::mem::transmute::<*mut c_void, SlogfFn>(p) };
            // SAFETY: "%s" with one NUL-terminated string.
            unsafe { f(code, sev, c"%s".as_ptr(), text) };
            true
        }
        (2, p) if cfg!(target_arch = "arm") => {
            // ARM EABI va_list: one word pointing at the next argument.
            let args: [*const c_char; 1] = [text];
            // SAFETY: the process's vslogf(int, int, const char*, va_list).
            let f = unsafe { core::mem::transmute::<*mut c_void, VslogfFn>(p) };
            // SAFETY: "%s" with the one argument in `args`.
            unsafe { f(code, sev, c"%s".as_ptr(), args.as_ptr().cast()) };
            true
        }
        _ => direct::write_text(code, sev, line),
    }
}
