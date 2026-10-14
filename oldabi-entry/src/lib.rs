// SPDX-License-Identifier: MIT OR Apache-2.0
//! The edge of a QNX 6.5 ARM library that vendor code calls into: the
//! export trampolines that realign the stack for old-ABI callers
//! ([`entry!`]), and the two `no_std` panic handlers a final artifact
//! installs ([`fault_panic_handler!`], [`abort_panic_handler!`]).
#![no_std]
use core::ffi::c_void;
use core::fmt;
use core::sync::atomic::{AtomicPtr, Ordering};
#[macro_export]
macro_rules! entry {
    ($export:literal => $body:path) => {
        const _: () = {
            #[unsafe(naked)]
            #[export_name = $export]
            unsafe extern "C" fn trampoline() {
                $crate::__entry_short!($body)
            }
        };
    };
    ($export:literal => $body:path, $words:tt) => {
        const _: () = {
            #[unsafe(naked)]
            #[export_name = $export]
            unsafe extern "C" fn trampoline() {
                $crate::__entry_frame!($body, $words)
            }
        };
    };
    ($vis:vis fn $name:ident => $body:path) => {
        #[unsafe(naked)]
        $vis unsafe extern "C" fn $name() {
            $crate::__entry_short!($body)
        }
    };
    ($vis:vis fn $name:ident => $body:path, $words:tt) => {
        #[unsafe(naked)]
        $vis unsafe extern "C" fn $name() {
            $crate::__entry_frame!($body, $words)
        }
    };
}
#[doc(hidden)]
#[macro_export]
macro_rules! __entry_frame {
    ($body:path, $words:tt) => {
        ::core::arch::naked_asm!(
            "push {{r4, r5, r6, lr}}",
            "mov r4, sp",
            "sub r12, sp, #{bytes}",
            $crate::__entry_align!(),
            "mov sp, r12",
            $crate::__entry_copy!($words),
            "bl {body}",
            "mov sp, r4",
            "pop {{r4, r5, r6, pc}}",
            bytes = const $crate::frame_bytes($words),
            body = sym $body,
        )
    };
}
#[doc(hidden)]
#[macro_export]
macro_rules! __entry_short {
    ($body:path) => {
        ::core::arch::naked_asm!(
            "push {{r4, r5, r6, lr}}",
            "mov r4, sp",
            $crate::__entry_align_short!(),
            "mov sp, r12",
            "bl {body}",
            "mov sp, r4",
            "pop {{r4, r5, r6, pc}}",
            body = sym $body,
        )
    };
}
#[cfg(not(feature = "no_realign"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __entry_align {
    () => {
        "bic r12, r12, #7"
    };
}
#[cfg(feature = "no_realign")]
#[doc(hidden)]
#[macro_export]
macro_rules! __entry_align {
    () => {
        "nop"
    };
}
#[cfg(not(feature = "no_realign"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __entry_align_short {
    () => {
        "bic r12, sp, #7"
    };
}
#[cfg(feature = "no_realign")]
#[doc(hidden)]
#[macro_export]
macro_rules! __entry_align_short {
    () => {
        "mov r12, sp"
    };
}
#[doc(hidden)]
#[macro_export]
macro_rules! __entry_copy {
    (0) => {
        ""
    };
    (1) => {
        "ldr r5, [r4, #16]\nstr r5, [sp, #0]"
    };
    (2) => {
        concat!(
            $crate::__entry_copy!(1),
            "\nldr r5, [r4, #20]\nstr r5, [sp, #4]"
        )
    };
    (3) => {
        concat!(
            $crate::__entry_copy!(2),
            "\nldr r5, [r4, #24]\nstr r5, [sp, #8]"
        )
    };
    (4) => {
        concat!(
            $crate::__entry_copy!(3),
            "\nldr r5, [r4, #28]\nstr r5, [sp, #12]"
        )
    };
    (5) => {
        concat!(
            $crate::__entry_copy!(4),
            "\nldr r5, [r4, #32]\nstr r5, [sp, #16]"
        )
    };
    (6) => {
        concat!(
            $crate::__entry_copy!(5),
            "\nldr r5, [r4, #36]\nstr r5, [sp, #20]"
        )
    };
    (7) => {
        concat!(
            $crate::__entry_copy!(6),
            "\nldr r5, [r4, #40]\nstr r5, [sp, #24]"
        )
    };
    (8) => {
        concat!(
            $crate::__entry_copy!(7),
            "\nldr r5, [r4, #44]\nstr r5, [sp, #28]"
        )
    };
    (9) => {
        concat!(
            $crate::__entry_copy!(8),
            "\nldr r5, [r4, #48]\nstr r5, [sp, #32]"
        )
    };
    (10) => {
        concat!(
            $crate::__entry_copy!(9),
            "\nldr r5, [r4, #52]\nstr r5, [sp, #36]"
        )
    };
    (11) => {
        concat!(
            $crate::__entry_copy!(10),
            "\nldr r5, [r4, #56]\nstr r5, [sp, #40]"
        )
    };
    (12) => {
        concat!(
            $crate::__entry_copy!(11),
            "\nldr r5, [r4, #60]\nstr r5, [sp, #44]"
        )
    };
    ($other:tt) => {
        compile_error!("oldabi_entry::entry!: the stack word count must be a literal 0 to 12")
    };
}
#[doc(hidden)]
#[must_use]
pub const fn frame_bytes(words: u32) -> u32 {
    (words * 4 + 7) & !7
}
pub type FaultFn = extern "C" fn(user: *mut c_void, msg: *const u8, len: u32);
pub struct FaultSink {
    user: AtomicPtr<c_void>,
    func: AtomicPtr<c_void>,
}
impl FaultSink {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            user: AtomicPtr::new(core::ptr::null_mut()),
            func: AtomicPtr::new(core::ptr::null_mut()),
        }
    }
    #[inline]
    pub fn set(&self, f: Option<FaultFn>, user: *mut c_void) {
        self.user.store(user, Ordering::Relaxed);
        let fp = f.map_or(core::ptr::null_mut(), |func| func as *mut c_void);
        self.func.store(fp, Ordering::Release);
    }
    #[inline]
    pub fn report(&self, info: &core::panic::PanicInfo) {
        let mut text = PanicText::new();
        let _ = fmt::write(&mut text, format_args!("{info}"));
        self.deliver(&text);
    }
    #[inline]
    pub fn report_args(&self, args: fmt::Arguments<'_>) {
        let mut text = PanicText::new();
        let _ = fmt::write(&mut text, args);
        self.deliver(&text);
    }
    #[inline]
    fn deliver(&self, text: &PanicText) {
        let fp = self.func.load(Ordering::Acquire);
        if !fp.is_null() {
            // SAFETY: Each non-null `func` value comes from a `FaultFn` in `set`.
            // On this target, code pointers and data pointers have the same width.
            let f: FaultFn = unsafe { core::mem::transmute::<*mut c_void, FaultFn>(fp) };
            let user = self.user.load(Ordering::Relaxed);
            f(user, text.b.as_ptr(), text.len_u32());
        }
    }
}
impl Default for FaultSink {
    fn default() -> Self {
        Self::new()
    }
}
pub struct PanicText {
    b: [u8; PanicText::CAPACITY],
    n: usize,
}
impl PanicText {
    pub const CAPACITY: usize = 256;
    #[must_use]
    pub const fn new() -> Self {
        Self {
            b: [0; Self::CAPACITY],
            n: 0,
        }
    }
    #[must_use]
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.b.get(..self.n).unwrap_or(&[])
    }
    #[must_use]
    #[inline]
    pub fn len_u32(&self) -> u32 {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "n <= 256 by construction of write_str"
        )]
        let n = self.n as u32;
        n
    }
}
impl Default for PanicText {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Write for PanicText {
    #[inline]
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &c in s.as_bytes() {
            if let Some(slot) = self.b.get_mut(self.n) {
                *slot = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}
#[inline]
pub fn spin() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
#[inline]
pub fn abort() -> ! {
    extern "C" {
        fn abort() -> !;
    }
    // SAFETY: libc's abort takes nothing and does not return.
    unsafe { abort() }
}
#[macro_export]
macro_rules! fault_panic_handler {
    ($sink:path) => {
        #[panic_handler]
        fn oldabi_entry_on_panic(info: &::core::panic::PanicInfo) -> ! {
            $sink.report(info);
            $crate::spin()
        }
    };
}
#[macro_export]
macro_rules! abort_panic_handler {
    () => {
        #[panic_handler]
        fn oldabi_entry_on_panic(_info: &::core::panic::PanicInfo) -> ! {
            $crate::abort()
        }
    };
}
#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use core::fmt::Write;
    use std::sync::Mutex;
    use std::vec::Vec;
    #[test]
    fn frame_bytes_keep_sp_aligned() {
        assert_eq!(frame_bytes(0), 0);
        assert_eq!(frame_bytes(1), 8);
        assert_eq!(frame_bytes(2), 8);
        assert_eq!(frame_bytes(3), 16);
        assert_eq!(frame_bytes(4), 16);
        assert_eq!(frame_bytes(12), 48);
        for w in 0..=12 {
            assert_eq!(frame_bytes(w) % 8, 0);
            assert!(frame_bytes(w) >= w * 4);
            assert!(frame_bytes(w) < w * 4 + 8);
        }
    }
    #[test]
    fn copy_text_moves_each_word() {
        assert_eq!(crate::__entry_copy!(0), "");
        assert_eq!(
            crate::__entry_copy!(2),
            "ldr r5, [r4, #16]\nstr r5, [sp, #0]\nldr r5, [r4, #20]\nstr r5, [sp, #4]"
        );
        let texts = [
            crate::__entry_copy!(1),
            crate::__entry_copy!(2),
            crate::__entry_copy!(3),
            crate::__entry_copy!(4),
            crate::__entry_copy!(5),
            crate::__entry_copy!(6),
            crate::__entry_copy!(7),
            crate::__entry_copy!(8),
            crate::__entry_copy!(9),
            crate::__entry_copy!(10),
            crate::__entry_copy!(11),
            crate::__entry_copy!(12),
        ];
        for (i, text) in texts.iter().enumerate() {
            let lines: Vec<&str> = text.split('\n').collect();
            assert_eq!(lines.len(), 2 * (i + 1));
            for (k, pair) in lines.chunks(2).enumerate() {
                assert_eq!(pair[0], std::format!("ldr r5, [r4, #{}]", 16 + 4 * k));
                assert_eq!(pair[1], std::format!("str r5, [sp, #{}]", 4 * k));
            }
        }
    }
    #[test]
    fn panic_text_truncates_at_capacity() {
        let mut t = PanicText::new();
        assert_eq!(t.len_u32(), 0);
        for _ in 0..30 {
            t.write_str("0123456789").unwrap();
        }
        assert_eq!(t.as_bytes().len(), PanicText::CAPACITY);
        assert_eq!(t.len_u32(), 256);
        assert_eq!(&t.as_bytes()[..12], b"012345678901");
    }
    static SEEN: Mutex<Vec<(usize, Vec<u8>)>> = Mutex::new(Vec::new());
    extern "C" fn record(user: *mut c_void, msg: *const u8, len: u32) {
        // SAFETY: report_args passes the PanicText buffer and its length.
        let bytes = unsafe { core::slice::from_raw_parts(msg, len as usize) };
        SEEN.lock().unwrap().push((user as usize, bytes.to_vec()));
    }
    #[test]
    fn sink_calls_the_registered_callback() {
        let sink = FaultSink::new();
        sink.report_args(format_args!("lost {}", 1));
        sink.set(Some(record), 0x1234 as *mut c_void);
        sink.report_args(format_args!("at {}:{}", "x.rs", 7));
        sink.set(None, core::ptr::null_mut());
        sink.report_args(format_args!("lost {}", 2));
        let seen = SEEN.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, 0x1234);
        assert_eq!(seen[0].1, b"at x.rs:7");
    }
}
