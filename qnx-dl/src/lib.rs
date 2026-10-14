// SPDX-License-Identifier: MIT OR Apache-2.0
//! The QNX 6.5 dynamic linker calls, for code that looks symbols up at run
//! time: libraries injected into a vendor process (interposers that call
//! the real function through `RTLD_NEXT`), and programs that load a vendor
//! library with `dlopen`.
#![no_std]
use core::ffi::{c_char, c_int, c_void, CStr};
use core::marker::PhantomData;
use core::mem::size_of;
use core::sync::atomic::{AtomicPtr, Ordering};
pub const RTLD_LAZY: c_int = 0x0001;
pub const RTLD_NOW: c_int = 0x0002;
pub const RTLD_NOLOAD: c_int = 0x0004;
pub const RTLD_GLOBAL: c_int = 0x0100;
pub const RTLD_LOCAL: c_int = 0x0200;
pub const RTLD_NODELETE: c_int = 0x1000;
#[cfg(target_os = "nto")]
pub const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
#[cfg(not(target_os = "nto"))]
pub const RTLD_DEFAULT: *mut c_void = core::ptr::null_mut();
#[cfg(target_os = "nto")]
pub const RTLD_NEXT: *mut c_void = -3isize as *mut c_void;
#[cfg(not(target_os = "nto"))]
pub const RTLD_NEXT: *mut c_void = -1isize as *mut c_void;
extern "C" {
    pub fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    pub fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    pub fn dlclose(handle: *mut c_void) -> c_int;
    pub fn dlerror() -> *const c_char;
}
#[must_use]
pub fn open(path: &CStr, mode: c_int) -> Option<*mut c_void> {
    // SAFETY: dlopen with a NUL-terminated path.
    let h = unsafe { dlopen(path.as_ptr(), mode) };
    (!h.is_null()).then_some(h)
}
/// # Safety
/// `handle` is [`RTLD_DEFAULT`], [`RTLD_NEXT`] or a handle from [`open`]
/// that has not been closed.
#[must_use]
pub unsafe fn sym(handle: *mut c_void, name: &CStr) -> *mut c_void {
    // SAFETY: a valid handle (the caller's contract) and a NUL-terminated
    // name.
    unsafe { dlsym(handle, name.as_ptr()) }
}
#[must_use]
pub fn sym_default(name: &CStr) -> *mut c_void {
    // SAFETY: RTLD_DEFAULT is always a valid handle.
    unsafe { sym(RTLD_DEFAULT, name) }
}
#[must_use]
pub fn sym_next(name: &CStr) -> *mut c_void {
    // SAFETY: RTLD_NEXT is always a valid handle.
    unsafe { sym(RTLD_NEXT, name) }
}
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
#[must_use]
pub fn dl_error() -> &'static str {
    // SAFETY: `dlerror` returns null or a NUL-terminated string.
    // The string stays valid until the next dl call on this thread.
    // This call reads the text before the next dl call.
    unsafe {
        let e = dlerror();
        if e.is_null() {
            ""
        } else {
            CStr::from_ptr(e).to_str().unwrap_or("(non-UTF-8 dlerror)")
        }
    }
}
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
    #[must_use]
    pub fn name(&self) -> &'static CStr {
        self.name
    }
}
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
    pub fn get(&self) -> Option<F> {
        let p = self.next.get();
        if p.is_null() {
            return None;
        }
        // SAFETY: F matches the symbol (the contract of `new`) and is
        // pointer-sized.
        Some(unsafe { core::mem::transmute_copy::<*mut c_void, F>(&p) })
    }
    #[must_use]
    pub fn name(&self) -> &'static CStr {
        self.next.name()
    }
}
// SAFETY: the only state is an atomic pointer to code and a static name.
unsafe impl<F> Sync for NextFn<F> {}
#[cfg(test)]
mod tests {
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
