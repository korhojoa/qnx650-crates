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

/// Capture host test messages without a device or a file.
/// This module is not part of a QNX build.
#[cfg(not(target_os = "nto"))]
pub mod host_test {
    extern crate std;

    use core::cell::Cell;
    use core::ffi::{c_int, CStr};

    /// Receive the original code, severity and complete message.
    pub type Capture = fn(c_int, c_int, &CStr) -> bool;

    std::thread_local! {
        static CAPTURE: Cell<Option<Capture>> = const { Cell::new(None) };
    }

    struct Restore(Option<Capture>);

    impl Drop for Restore {
        fn drop(&mut self) {
            CAPTURE.with(|slot| slot.set(self.0));
        }
    }

    /// Capture writes on this thread during `run`.
    /// Restore the previous capture on return or panic. Other threads keep
    /// their own sink. A false capture result does not call the native writer.
    pub fn with_capture<R>(capture: Capture, run: impl FnOnce() -> R) -> R {
        let _restore = Restore(CAPTURE.with(|slot| slot.replace(Some(capture))));
        run()
    }

    pub(super) fn write(code: c_int, sev: c_int, line: &CStr) -> Option<bool> {
        CAPTURE
            .with(|slot| slot.get())
            .map(|capture| capture(code, sev, line))
    }
}

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
    #[cfg(not(target_os = "nto"))]
    if let Some(result) = host_test::write(code, sev, line) {
        return result;
    }
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

#[cfg(all(test, not(target_os = "nto")))]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::vec::Vec;

    std::thread_local! {
        static LINES: RefCell<Vec<(c_int, c_int, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
    }

    fn capture(code: c_int, sev: c_int, line: &CStr) -> bool {
        LINES.with(|lines| {
            lines
                .borrow_mut()
                .push((code, sev, line.to_bytes().to_vec()));
        });
        true
    }

    fn refuse(_: c_int, _: c_int, _: &CStr) -> bool {
        false
    }

    #[test]
    fn host_capture_keeps_code_severity_and_message_bytes() {
        host_test::with_capture(capture, || {
            assert!(write_code(123, 3, c"literal %s\n\n"));
            assert!(write(5, c"without newline"));
        });
        LINES.with(|lines| {
            assert_eq!(
                *lines.borrow(),
                [
                    (123, 3, b"literal %s\n\n".to_vec()),
                    (0, 5, b"without newline".to_vec())
                ]
            );
        });
        assert!(host_test::write(0, 0, c"outside").is_none());
    }

    #[test]
    fn host_capture_restores_after_nested_calls_and_panic() {
        host_test::with_capture(capture, || {
            assert!(!host_test::with_capture(refuse, || write(2, c"refused")));
            assert!(std::panic::catch_unwind(|| {
                host_test::with_capture(refuse, || panic!("capture test"));
            })
            .is_err());
            assert!(write(2, c"restored"));
        });
        LINES.with(|lines| assert_eq!(*lines.borrow(), [(0, 2, b"restored".to_vec())]));
        assert!(host_test::write(0, 0, c"outside").is_none());
    }

    #[test]
    fn host_capture_does_not_change_another_thread() {
        host_test::with_capture(capture, || {
            std::thread::spawn(|| assert!(host_test::write(0, 0, c"other thread").is_none()))
                .join()
                .unwrap();
            assert!(write(4, c"own thread"));
        });
        LINES.with(|lines| assert_eq!(*lines.borrow(), [(0, 4, b"own thread".to_vec())]));
    }
}
