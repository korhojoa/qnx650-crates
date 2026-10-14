// SPDX-License-Identifier: MIT OR Apache-2.0
//! `errno` for QNX 6.5 code that cannot use `std::io::Error`: `#![no_std]`
//! libraries injected into a vendor process, and FFI shims.
#![no_std]
use core::ffi::c_int;
extern "C" {
    #[cfg_attr(target_os = "nto", link_name = "__get_errno_ptr")]
    #[cfg_attr(not(target_os = "nto"), link_name = "__errno_location")]
    fn errno_ptr() -> *mut c_int;
}
#[must_use]
pub fn errno() -> c_int {
    // SAFETY: the libc returns a valid pointer to this thread's errno.
    unsafe { *errno_ptr() }
}
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
