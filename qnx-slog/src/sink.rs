// SPDX-License-Identifier: MIT OR Apache-2.0
//! Where a finished line goes: the system log.
use crate::{direct, sys};
use core::ffi::{c_char, c_int, c_void, CStr};
use core::sync::atomic::{AtomicPtr, AtomicU8, Ordering};
type SlogfFn = unsafe extern "C" fn(c_int, c_int, *const c_char, ...) -> c_int;
type VslogfFn = unsafe extern "C" fn(c_int, c_int, *const c_char, *const c_void) -> c_int;
static FUNC: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
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
#[must_use]
pub fn describe() -> &'static str {
    match resolve().0 {
        1 => "slogf (found in the process)",
        2 => "vslogf (found in the process; no slogf)",
        _ => "/dev/slog (direct writer)",
    }
}
#[must_use]
pub fn write(sev: c_int, line: &CStr) -> bool {
    write_code(sys::SLOGC_GENL, sev, line)
}
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
