// SPDX-License-Identifier: MIT OR Apache-2.0
//! The QNX 6.5 dynamic linker calls, for code that looks symbols up at run
//! time: libraries injected into a vendor process (interposers that call
//! the real function through `RTLD_NEXT`), and programs that load a vendor
//! library with `dlopen`.
//!
//! The `RTLD_*` values are QNX's, from the SDP `dlfcn.h`: `RTLD_DEFAULT` is
//! `(void *)-2` and `RTLD_NEXT` is `(void *)-3`, not the glibc values. On a
//! host build (tests) the two handles take the glibc values so the same
//! code runs there.
//!
//! A NULL handle is NOT a handle on QNX 6.5: `dlsym(NULL, name)` returns
//! NULL with `dlerror()` "Invalid handle" and searches nothing (`dlsym(NULL, "printf")` fails where
//! `dlsym(RTLD_DEFAULT, "printf")` finds it). Use [`RTLD_DEFAULT`].
//!
//! `#![no_std]` with no dependencies.
//! The optional `alloc` feature enables owned error strings.

#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::ffi::{c_char, c_int, c_void, CStr};
use core::fmt;
use core::marker::PhantomData;
use core::mem::size_of;
use core::sync::atomic::{AtomicPtr, Ordering};

/// `dlopen` mode: resolve symbols when first used.
pub const RTLD_LAZY: c_int = 0x0001;
/// `dlopen` mode: resolve every symbol at load.
pub const RTLD_NOW: c_int = 0x0002;
/// `dlopen` mode: only return a handle when the object is already loaded.
pub const RTLD_NOLOAD: c_int = 0x0004;
/// `dlopen` mode: the object's symbols serve later loads.
pub const RTLD_GLOBAL: c_int = 0x0100;
/// `dlopen` mode: the object's symbols serve only this handle.
pub const RTLD_LOCAL: c_int = 0x0200;
/// `dlopen` mode: never unload the object.
pub const RTLD_NODELETE: c_int = 0x1000;

/// The handle that searches the default scope (the process's objects in
/// load order).
#[cfg(target_os = "nto")]
pub const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
/// The handle that searches the default scope (glibc value, host builds).
#[cfg(not(target_os = "nto"))]
pub const RTLD_DEFAULT: *mut c_void = core::ptr::null_mut();
/// The handle that searches the objects loaded after the caller's, which
/// is how an interposer finds the function it replaces.
#[cfg(target_os = "nto")]
pub const RTLD_NEXT: *mut c_void = -3isize as *mut c_void;
/// The `RTLD_NEXT` handle (glibc value, host builds).
#[cfg(not(target_os = "nto"))]
pub const RTLD_NEXT: *mut c_void = -1isize as *mut c_void;

extern "C" {
    pub fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    pub fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    pub fn dlclose(handle: *mut c_void) -> c_int;
    pub fn dlerror() -> *const c_char;
}

/// `dlopen(path, mode)`, or `None` (see [`dl_error_into`] for the reason).
#[must_use]
pub fn open(path: &CStr, mode: c_int) -> Option<*mut c_void> {
    // SAFETY: dlopen with a NUL-terminated path.
    let h = unsafe { dlopen(path.as_ptr(), mode) };
    (!h.is_null()).then_some(h)
}

/// `dlsym(handle, name)`, or null.
///
/// # Safety
/// `handle` is [`RTLD_DEFAULT`], [`RTLD_NEXT`] or a handle from [`open`]
/// that has not been closed.
#[must_use]
pub unsafe fn sym(handle: *mut c_void, name: &CStr) -> *mut c_void {
    // SAFETY: a valid handle (the caller's contract) and a NUL-terminated
    // name.
    unsafe { dlsym(handle, name.as_ptr()) }
}

/// `dlsym(RTLD_DEFAULT, name)`, or null.
#[must_use]
pub fn sym_default(name: &CStr) -> *mut c_void {
    // SAFETY: RTLD_DEFAULT is always a valid handle.
    unsafe { sym(RTLD_DEFAULT, name) }
}

/// `dlsym(RTLD_NEXT, name)`, or null.
#[must_use]
pub fn sym_next(name: &CStr) -> *mut c_void {
    // SAFETY: RTLD_NEXT is always a valid handle.
    unsafe { sym(RTLD_NEXT, name) }
}

/// `name` in `handle` as a function pointer of type `F`, or `None`.
///
/// # Safety
/// `handle` as for [`sym`]. `F` must be the `extern "C"` function pointer
/// type of the symbol's real signature. Its size is checked against a
/// pointer at compile time.
#[must_use]
pub unsafe fn symbol<F: Copy>(handle: *mut c_void, name: &CStr) -> Option<F> {
    const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
    // SAFETY: the caller's contract.
    let p = unsafe { sym(handle, name) };
    if p.is_null() {
        return None;
    }
    // SAFETY: F is a pointer-sized function pointer type (checked above)
    // that the caller says matches the symbol.
    Some(unsafe { core::mem::transmute_copy::<*mut c_void, F>(&p) })
}

/// The destination cannot hold the complete error and its NUL terminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorBufferTooSmall {
    /// Required capacity in bytes, including the NUL terminator.
    pub required: usize,
}

impl fmt::Display for ErrorBufferTooSmall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "linker error buffer requires {} bytes",
            self.required
        )
    }
}

impl core::error::Error for ErrorBufferTooSmall {}

fn with_error_text<T>(copy: impl FnOnce(&CStr) -> T) -> T {
    // SAFETY: The linker supplies a terminated string until the next linker call.
    // Both callers copy the text without any subsequent linker call.
    let pointer = unsafe { dlerror() };
    let text = if pointer.is_null() {
        c""
    } else {
        // SAFETY: The non-null pointer identifies the current linker error.
        unsafe { CStr::from_ptr(pointer) }
    };
    copy(text)
}

fn copy_error<'a>(text: &CStr, buffer: &'a mut [u8]) -> Result<&'a CStr, ErrorBufferTooSmall> {
    let bytes = text.to_bytes_with_nul();
    if buffer.len() < bytes.len() {
        return Err(ErrorBufferTooSmall {
            required: bytes.len(),
        });
    }
    let copied = &mut buffer[..bytes.len()];
    copied.copy_from_slice(bytes);
    // SAFETY: The complete bytes and terminator came from a valid C string.
    Ok(unsafe { CStr::from_bytes_with_nul_unchecked(copied) })
}

/// Copy the complete linker error into caller-owned storage without allocation.
/// The returned C string preserves all bytes, including non-UTF-8 text.
/// This call consumes the linker error even when the destination is too small.
/// A larger buffer cannot retrieve the same error through a later call.
///
/// # Errors
/// The error reports required capacity, including the NUL terminator.
/// The destination remains unchanged when capacity is insufficient.
pub fn dl_error_into(buffer: &mut [u8]) -> Result<&CStr, ErrorBufferTooSmall> {
    with_error_text(|text| copy_error(text, buffer))
}

#[cfg(feature = "alloc")]
fn own_error(text: &CStr) -> alloc::string::String {
    text.to_string_lossy().into_owned()
}

/// Return the complete linker error as an owned string without a fixed limit.
/// Invalid UTF-8 bytes become replacement characters.
/// This function requires the `alloc` feature and consumes the linker error.
#[cfg(feature = "alloc")]
#[must_use]
pub fn dl_error() -> alloc::string::String {
    with_error_text(own_error)
}

/// A symbol resolved once through `dlsym(RTLD_NEXT, name)` and cached: the
/// real function an interposer calls on. A failed lookup is not cached and
/// is retried on the next call.
pub struct Next {
    name: &'static CStr,
    p: AtomicPtr<c_void>,
}

impl Next {
    #[must_use]
    pub const fn new(name: &'static CStr) -> Self {
        Next {
            name,
            p: AtomicPtr::new(core::ptr::null_mut()),
        }
    }

    /// The address, or null when the lookup fails.
    pub fn get(&self) -> *mut c_void {
        let p = self.p.load(Ordering::Acquire);
        if !p.is_null() {
            return p;
        }
        let p = sym_next(self.name);
        if !p.is_null() {
            self.p.store(p, Ordering::Release);
        }
        p
    }

    /// The symbol's name.
    #[must_use]
    pub fn name(&self) -> &'static CStr {
        self.name
    }
}

/// [`Next`] with the function pointer type: `get` returns `Some(F)`.
pub struct NextFn<F> {
    next: Next,
    _f: PhantomData<F>,
}

impl<F: Copy> NextFn<F> {
    /// # Safety
    /// `F` must be the `extern "C"` function pointer type of the symbol's
    /// real signature (its size is checked at compile time).
    #[must_use]
    pub const unsafe fn new(name: &'static CStr) -> Self {
        const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
        NextFn {
            next: Next::new(name),
            _f: PhantomData,
        }
    }

    /// The real function, or `None` when the lookup fails.
    pub fn get(&self) -> Option<F> {
        let p = self.next.get();
        if p.is_null() {
            return None;
        }
        // SAFETY: F matches the symbol (the contract of `new`) and is
        // pointer-sized.
        Some(unsafe { core::mem::transmute_copy::<*mut c_void, F>(&p) })
    }

    /// The symbol's name.
    #[must_use]
    pub fn name(&self) -> &'static CStr {
        self.next.name()
    }
}

// SAFETY: the only state is an atomic pointer to code and a static name.
unsafe impl<F> Sync for NextFn<F> {}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    type StrlenFn = unsafe extern "C" fn(*const c_char) -> usize;

    #[test]
    fn a_libc_function_is_found_and_callable() {
        // SAFETY: strlen has this signature.
        let f = unsafe { symbol::<StrlenFn>(RTLD_DEFAULT, c"strlen") }.expect("strlen");
        // SAFETY: a NUL-terminated string.
        assert_eq!(unsafe { f(c"four".as_ptr()) }, 4);
    }

    #[test]
    fn a_missing_symbol_is_none_and_not_cached() {
        static N: Next = Next::new(c"qnx_dl_no_such_symbol");
        assert!(N.get().is_null());
        assert!(N.get().is_null());
        // SAFETY: never called.
        let f = unsafe { symbol::<StrlenFn>(RTLD_DEFAULT, c"qnx_dl_no_such_symbol") };
        assert!(f.is_none());
    }

    #[test]
    fn next_fn_finds_the_libc_definition() {
        // SAFETY: strlen has this signature.
        static STRLEN: NextFn<StrlenFn> = unsafe { NextFn::new(c"strlen") };
        let f = STRLEN.get().expect("strlen through RTLD_NEXT");
        // SAFETY: a NUL-terminated string.
        assert_eq!(unsafe { f(c"ab".as_ptr()) }, 2);
        assert_eq!(STRLEN.name(), c"strlen");
    }

    #[test]
    fn caller_buffer_survives_subsequent_linker_calls() {
        assert!(sym_default(c"qnx_dl_missing_first").is_null());
        let mut buffer = [0u8; 4096];
        let first = dl_error_into(&mut buffer).unwrap();
        assert!(!first.is_empty());
        let expected = first.to_bytes().to_vec();
        assert!(sym_default(c"qnx_dl_missing_second").is_null());
        let mut next = [0u8; 4096];
        let second = dl_error_into(&mut next).unwrap();
        assert!(!second.is_empty());
        assert_eq!(first.to_bytes(), expected);
    }

    #[test]
    fn caller_buffer_reports_complete_capacity_and_preserves_bytes() {
        let mut source = [b'x'; 8193];
        source[100] = 0xff;
        source[8192] = 0;
        let text = CStr::from_bytes_with_nul(&source).unwrap();
        let mut short = [b'!'; 1024];
        assert_eq!(
            copy_error(text, &mut short).unwrap_err().required,
            source.len()
        );
        assert!(short.iter().all(|byte| *byte == b'!'));
        let mut full = [0u8; 8193];
        let result = copy_error(text, &mut full).unwrap();
        assert_eq!(result.to_bytes_with_nul(), source);
        let mut empty = [];
        assert_eq!(copy_error(c"", &mut empty).unwrap_err().required, 1);
    }

    #[test]
    fn insufficient_capacity_still_consumes_the_linker_error() {
        assert!(sym_default(c"qnx_dl_missing_for_small_buffer").is_null());
        let mut short = [0u8; 1];
        assert!(dl_error_into(&mut short).unwrap_err().required > short.len());
        let mut next = [0u8; 4096];
        assert_eq!(dl_error_into(&mut next).unwrap(), c"");
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn owned_error_preserves_long_utf8_messages_without_a_cap() {
        let text =
            std::ffi::CString::new(std::string::String::from("\u{e4}").repeat(8192)).unwrap();
        let error = own_error(&text);
        assert_eq!(error, text.to_str().unwrap());
        assert_eq!(error.len(), 16384);
        let invalid = c"bad \xff byte";
        assert_eq!(own_error(invalid), "bad \u{fffd} byte");
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn owned_error_survives_subsequent_linker_calls() {
        assert!(sym_default(c"qnx_dl_missing_owned_first").is_null());
        let first = dl_error();
        assert_ne!(first, "");
        let expected = first.clone();
        assert!(sym_default(c"qnx_dl_missing_owned_second").is_null());
        assert_ne!(dl_error(), "");
        assert_eq!(first, expected);
    }

    #[test]
    fn qnx_mode_values_match_the_sdp_header() {
        assert_eq!(
            (
                RTLD_LAZY,
                RTLD_NOW,
                RTLD_NOLOAD,
                RTLD_GLOBAL,
                RTLD_LOCAL,
                RTLD_NODELETE
            ),
            (0x1, 0x2, 0x4, 0x100, 0x200, 0x1000)
        );
    }
}
