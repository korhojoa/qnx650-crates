// SPDX-License-Identifier: MIT OR Apache-2.0
//! `errno` for QNX 6.5 code that cannot use `std::io::Error`: `#![no_std]`
//! libraries injected into a vendor process, and FFI shims.
//!
//! On QNX 6.5 `errno` is `(*__get_errno_ptr())` (`errno.h` line 42), a
//! per-thread value. On a host build (tests) the glibc equivalent,
//! `__errno_location`, is used.

#![no_std]

use core::ffi::c_int;

extern "C" {
    #[cfg_attr(target_os = "nto", link_name = "__get_errno_ptr")]
    #[cfg_attr(not(target_os = "nto"), link_name = "__errno_location")]
    fn errno_ptr() -> *mut c_int;
}

/// The calling thread's `errno`.
#[must_use]
pub fn errno() -> c_int {
    // SAFETY: the libc returns a valid pointer to this thread's errno.
    unsafe { *errno_ptr() }
}

/// Set the calling thread's `errno` (to 0 before a call whose failure is
/// only visible through errno).
pub fn set_errno(value: c_int) {
    // SAFETY: as in `errno`.
    unsafe { *errno_ptr() = value };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_then_read_back() {
        set_errno(0);
        assert_eq!(errno(), 0);
        set_errno(22);
        assert_eq!(errno(), 22);
        set_errno(0);
    }
}
