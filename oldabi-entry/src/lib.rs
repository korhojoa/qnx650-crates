// SPDX-License-Identifier: MIT OR Apache-2.0
//! The edge of a QNX 6.5 ARM library that vendor code calls into: the
//! export trampolines that realign the stack for old-ABI callers
//! ([`entry!`]), and the two `no_std` panic handlers a final artifact
//! installs ([`fault_panic_handler!`], [`abort_panic_handler!`]).
//!
//! # Why the trampolines
//!
//! Vendor code on the MIB2 and the feature libraries built by the stock
//! gcc 4.4.2 use the old ARM ABI (APCS-GNU, ELF `e_flags` 0x602). It keeps
//! SP only 4-byte aligned. Rust code is AAPCS and assumes SP is 8-byte
//! aligned at every call, and LLVM uses that: it forms the address of a
//! field of an 8-aligned stack object with ORR instead of ADD (for example
//! `orr r1, r6, #4` for `&tm.tm_min`). Entered with SP = 4 (mod 8), that
//! ORR does nothing and the field aliases its neighbour, so the body reads
//! or writes the wrong stack slot. For an AAPCS caller the realignment
//! does nothing.
//!
//! # The trampoline
//!
//! Save r4-r6 and lr, keep the entry SP in r4, move SP down to an 8-byte
//! boundary with room for the stack-passed argument words, copy those
//! words, call the body, restore SP, return. r0-r3 (the register arguments)
//! and r0/r1 (the result) pass through unchanged. The body is an ordinary
//! `extern "C" fn` (or an `extern "C"` declaration). It must not unwind
//! through the trampoline: an `extern "C"` body already stops a panic at
//! its own boundary.
//!
//! The emitted code, `N` stack words (`B` = `4 * N` rounded up to 8):
//!
//! ```text
//! push {r4, r5, r6, lr}
//! mov  r4, sp
//! sub  r12, sp, #B
//! bic  r12, r12, #7
//! mov  sp, r12
//! ldr  r5, [r4, #16]     ; once per stack word k:
//! str  r5, [sp, #0]      ; [r4, #16 + 4k] -> [sp, #4k]
//! bl   body
//! mov  sp, r4
//! pop  {r4, r5, r6, pc}
//! ```
//!
//! The short form, for a body with no stack-passed arguments, replaces
//! `sub` and `bic` with one `bic r12, sp, #7`. Both forms are correct for
//! zero stack words. Each user keeps the form its build audit checks for.
//!
//! # Stack words
//!
//! The count is the number of argument words after r0-r3 in the AAPCS
//! layout: a 64-bit argument takes an even register pair (r2:r3 after
//! `this`), and the words that do not fit go on the stack. For a varargs
//! body give the most words any caller passes. A caller that passes fewer
//! leaves junk in the copied tail, which a varargs body never reads.
//!
//! # The panic handlers
//!
//! A `#[panic_handler]` must be defined in exactly one crate of the final
//! artifact (the `.so` or the executable). So this crate defines none. It
//! gives each handler as a macro that the artifact crate invokes once:
//!
//! - [`fault_panic_handler!`] formats the panic message into a fixed
//!   256-byte buffer, hands it to the callback registered in a
//!   [`FaultSink`], and spins if the callback returns. For a library that
//!   must never `abort()` its host process.
//! - [`abort_panic_handler!`] calls libc's `abort()`.
//!
//! A cargo feature was not used for this. Cargo unifies features across a
//! build, so a library that turned the handler on would put it into every
//! artifact and test build that links the library. A macro keeps the
//! choice, and its `cfg` (for example `#[cfg(not(test))]`), in the crate
//! that owns the artifact.

#![no_std]

use core::ffi::c_void;
use core::fmt;
use core::sync::atomic::{AtomicPtr, Ordering};

/// Defines a stack-realigning ARM trampoline for an `extern "C"` body.
///
/// Forms:
///
/// ```text
/// entry!("export_name" => path::to::body);        // exported, short form
/// entry!("export_name" => path::to::body, N);     // exported, N stack words
/// entry!(fn name => path::to::body);              // not exported, short form
/// entry!(pub fn name => path::to::body, N);       // not exported, N words
/// ```
///
/// `N` is a literal from 0 to 12. The exported forms give the trampoline
/// the export name (`#[export_name]`). The `fn` forms define a function
/// `name` with the signature `unsafe extern "C" fn()`, for a callback
/// pointer that the caller converts to the body's real type.
///
/// Expands only in an ARM build (it is ARM assembly). See the crate
/// documentation for the code and the reasons.
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

/// The trampoline body with a stack frame for `$words` stack words.
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

/// The trampoline body with no stack-passed arguments, one instruction
/// shorter than `__entry_frame!` with 0 words.
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

/// The realigning instruction of the frame form.
#[cfg(not(feature = "no_realign"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __entry_align {
    () => {
        "bic r12, r12, #7"
    };
}

/// The negative control: no realignment.
#[cfg(feature = "no_realign")]
#[doc(hidden)]
#[macro_export]
macro_rules! __entry_align {
    () => {
        "nop"
    };
}

/// The realigning instruction of the short form.
#[cfg(not(feature = "no_realign"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __entry_align_short {
    () => {
        "bic r12, sp, #7"
    };
}

/// The negative control: no realignment.
#[cfg(feature = "no_realign")]
#[doc(hidden)]
#[macro_export]
macro_rules! __entry_align_short {
    () => {
        "mov r12, sp"
    };
}

/// The copy of `N` stack words, from above the saved registers (`r4` + 16)
/// to the new frame (`sp` + 0), as assembly text.
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

/// Bytes the frame form reserves for `words` stack words: `4 * words`
/// rounded up to a multiple of 8, so SP stays 8-aligned after the `bic`.
#[doc(hidden)]
#[must_use]
pub const fn frame_bytes(words: u32) -> u32 {
    (words * 4 + 7) & !7
}

/// The callback a [`FaultSink`] calls from the panic handler. `msg` is NOT
/// NUL-terminated, `len` gives its length. It must not return: it should log
/// the message and then park the thread. If it returns, the handler spins.
pub type FaultFn = extern "C" fn(user: *mut c_void, msg: *const u8, len: u32);

/// The registered fault callback and its user pointer, for
/// [`fault_panic_handler!`]. Put one in a `static` of the artifact crate.
///
/// [`FaultSink::set`] stores the user pointer first, then publishes the
/// callback with Release. The panic path's Acquire load of the callback
/// therefore sees the matching user pointer, with no lock and no data race.
pub struct FaultSink {
    user: AtomicPtr<c_void>,
    func: AtomicPtr<c_void>,
}

impl FaultSink {
    /// A sink with no callback.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            user: AtomicPtr::new(core::ptr::null_mut()),
            func: AtomicPtr::new(core::ptr::null_mut()),
        }
    }

    /// Registers `f` with `user`, or removes the callback when `f` is
    /// `None`. The last call wins.
    #[inline]
    pub fn set(&self, f: Option<FaultFn>, user: *mut c_void) {
        self.user.store(user, Ordering::Relaxed);
        let fp = f.map_or(core::ptr::null_mut(), |func| func as *mut c_void);
        self.func.store(fp, Ordering::Release);
    }

    /// Formats `info` into a [`PanicText`] and calls the callback, if one is
    /// registered. Returns if there is none or if the callback returns.
    #[inline]
    pub fn report(&self, info: &core::panic::PanicInfo) {
        let mut text = PanicText::new();
        let _ = fmt::write(&mut text, format_args!("{info}"));
        self.deliver(&text);
    }

    /// Formats `args` into a [`PanicText`] and calls the callback, if one is
    /// registered.
    #[inline]
    pub fn report_args(&self, args: fmt::Arguments<'_>) {
        let mut text = PanicText::new();
        let _ = fmt::write(&mut text, args);
        self.deliver(&text);
    }

    /// Calls the callback, if one is registered, with `text`.
    #[inline]
    fn deliver(&self, text: &PanicText) {
        let fp = self.func.load(Ordering::Acquire);
        if !fp.is_null() {
            // SAFETY: the only non-null values stored in `func` come from a
            // FaultFn converted to a data pointer in `set`; this target uses
            // same-width code and data pointers.
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

/// A fixed 256-byte text buffer for a panic message: no allocation, the
/// text past 256 bytes is dropped.
pub struct PanicText {
    b: [u8; PanicText::CAPACITY],
    n: usize,
}

impl PanicText {
    /// Bytes the buffer holds.
    pub const CAPACITY: usize = 256;

    /// An empty buffer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            b: [0; Self::CAPACITY],
            n: 0,
        }
    }

    /// The text written so far, at most [`PanicText::CAPACITY`] bytes. A
    /// cut can split a UTF-8 sequence, so this is bytes, not `str`.
    #[must_use]
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.b.get(..self.n).unwrap_or(&[])
    }

    /// The length as the `u32` of the [`FaultFn`] ABI.
    #[must_use]
    #[inline]
    pub fn len_u32(&self) -> u32 {
        // `write_str` advances `n` only after a successful `get_mut` into the
        // 256-byte array, so `n <= 256` and the narrowing is exact. A
        // `try_from` would add a branch to the panic path.
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

/// Spins forever. The end of [`fault_panic_handler!`] when the callback
/// returns: never `abort()`, which would end the host process.
#[inline]
pub fn spin() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

/// Calls libc's `abort()`. The body of [`abort_panic_handler!`].
#[inline]
pub fn abort() -> ! {
    extern "C" {
        fn abort() -> !;
    }
    // SAFETY: libc's abort takes nothing and does not return.
    unsafe { abort() }
}

/// Defines the artifact's `#[panic_handler]`: report the panic to the
/// [`FaultSink`] `$sink` (a `static`), then spin.
///
/// Invoke it once, in the crate that owns the final artifact, for example
/// `#[cfg(not(test))] oldabi_entry::fault_panic_handler!(FAULT);`.
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

/// Defines the artifact's `#[panic_handler]` as a call to libc's `abort()`.
///
/// Invoke it once, in the crate that owns the final artifact, for example
/// `#[cfg(not(feature = "std"))] oldabi_entry::abort_panic_handler!();`.
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
        // The sizes the hand-written trampolines used.
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
        // No callback: nothing is called.
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
