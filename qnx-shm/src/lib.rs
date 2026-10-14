// SPDX-License-Identifier: MIT OR Apache-2.0
//! POSIX shared memory on QNX 6.5: create or open a named object and map
//! it. This is how the daemons on the unit publish their rings and state
//! blocks (`/hidkbm-*`, `/btgamepad`, `/usbgamepad`, `/bnep-iface` ...).
#![no_std]
use core::ffi::{c_char, c_int, c_uint, c_void, CStr};
use core::ptr::NonNull;
pub use qnx_errno::errno;
pub const O_RDONLY: c_int = 0;
pub const O_RDWR: c_int = 0o2;
#[cfg(target_os = "nto")]
pub const O_CREAT: c_int = 0o400;
#[cfg(not(target_os = "nto"))]
pub const O_CREAT: c_int = 0o100;
pub const S_IRUSR: c_uint = 0o400;
pub const S_IWUSR: c_uint = 0o200;
pub const S_IRGRP: c_uint = 0o40;
pub const S_IWGRP: c_uint = 0o20;
pub const S_IROTH: c_uint = 0o4;
pub const S_IWOTH: c_uint = 0o2;
pub const MODE_ALL_RW: c_uint = S_IRUSR | S_IWUSR | S_IRGRP | S_IWGRP | S_IROTH | S_IWOTH;
#[cfg(target_os = "nto")]
pub const PROT_READ: c_int = 0x100;
#[cfg(target_os = "nto")]
pub const PROT_WRITE: c_int = 0x200;
#[cfg(not(target_os = "nto"))]
pub const PROT_READ: c_int = 0x1;
#[cfg(not(target_os = "nto"))]
pub const PROT_WRITE: c_int = 0x2;
pub const MAP_SHARED: c_int = 0x1;
#[cfg(target_os = "nto")]
pub type OffT = i32;
#[cfg(not(target_os = "nto"))]
pub type OffT = i64;
extern "C" {
    pub fn shm_open(name: *const c_char, oflag: c_int, mode: c_uint) -> c_int;
    pub fn shm_unlink(name: *const c_char) -> c_int;
    pub fn ftruncate(fd: c_int, length: OffT) -> c_int;
    pub fn fchmod(fd: c_int, mode: c_uint) -> c_int;
    pub fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        off: OffT,
    ) -> *mut c_void;
    pub fn munmap(addr: *mut c_void, len: usize) -> c_int;
    pub fn close(fd: c_int) -> c_int;
}
const MAP_FAILED: *mut c_void = usize::MAX as *mut c_void;
pub struct Mapping {
    ptr: NonNull<u8>,
    len: usize,
}
fn last_error() -> c_int {
    match errno() {
        0 => -1,
        e => e,
    }
}
fn map_and_close(fd: c_int, len: usize, prot: c_int) -> Result<Mapping, c_int> {
    // SAFETY: fd is an open shm descriptor. A new mapping of len bytes.
    let p = unsafe { mmap(core::ptr::null_mut(), len, prot, MAP_SHARED, fd, 0) };
    let err = if p == MAP_FAILED { last_error() } else { 0 };
    // SAFETY: fd is ours. The mapping (if any) does not need it.
    unsafe { close(fd) };
    match NonNull::new(p.cast::<u8>()) {
        Some(ptr) if err == 0 => Ok(Mapping { ptr, len }),
        _ => Err(if err == 0 { -1 } else { err }),
    }
}
impl Mapping {
    /// # Errors
    /// The errno of the failed call (`shm_open`, `ftruncate`, `fchmod`,
    /// `mmap`). -1 when `len` does not fit `off_t`.
    pub fn create(name: &CStr, len: usize, mode: c_uint) -> Result<Mapping, c_int> {
        let size = OffT::try_from(len).map_err(|_| -1)?;
        // SAFETY: a NUL-terminated name.
        let fd = unsafe { shm_open(name.as_ptr(), O_RDWR | O_CREAT, mode) };
        if fd < 0 {
            return Err(last_error());
        }
        // SAFETY: fd is the descriptor just opened.
        let failed = unsafe { ftruncate(fd, size) != 0 || fchmod(fd, mode) != 0 };
        if failed {
            let e = last_error();
            // SAFETY: fd is ours.
            unsafe { close(fd) };
            return Err(e);
        }
        map_and_close(fd, len, PROT_READ | PROT_WRITE)
    }
    /// # Errors
    /// The errno of the failed `shm_open` or `mmap` (`ENOENT` when the
    /// producer has not created it yet).
    pub fn open(name: &CStr, len: usize, writable: bool) -> Result<Mapping, c_int> {
        let (oflag, prot) = if writable {
            (O_RDWR, PROT_READ | PROT_WRITE)
        } else {
            (O_RDONLY, PROT_READ)
        };
        // SAFETY: a NUL-terminated name.
        let fd = unsafe { shm_open(name.as_ptr(), oflag, 0) };
        if fd < 0 {
            return Err(last_error());
        }
        map_and_close(fd, len, prot)
    }
    #[must_use]
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    #[must_use]
    pub fn leak(self) -> (*mut u8, usize) {
        let out = (self.ptr.as_ptr(), self.len);
        core::mem::forget(self);
        out
    }
}
impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: the mapping this value owns, unmapped exactly once.
        unsafe { munmap(self.ptr.as_ptr().cast(), self.len) };
    }
}
/// # Errors
/// The errno of `shm_unlink`.
pub fn unlink(name: &CStr) -> Result<(), c_int> {
    // SAFETY: a NUL-terminated name.
    if unsafe { shm_unlink(name.as_ptr()) } == 0 {
        Ok(())
    } else {
        Err(last_error())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qnx_values_match_the_sdp_headers() {
        assert_eq!((O_RDWR, 0o400, S_IRUSR, S_IWUSR), (0x2, 0x100, 0x100, 0x80));
        assert_eq!((S_IRGRP, S_IWGRP, S_IROTH, S_IWOTH), (0x20, 0x10, 0x4, 0x2));
        assert_eq!(MODE_ALL_RW, 0o666);
    }
    #[test]
    fn create_write_open_read_unlink() {
        let name = c"/qnx-shm-test-create-open";
        let _ = unlink(name);
        let a = Mapping::create(name, 64, MODE_ALL_RW).expect("create");
        assert_eq!(a.len(), 64);
        // SAFETY: inside the 64-byte mapping.
        unsafe { a.as_ptr().add(3).write(0x5a) };
        let b = Mapping::open(name, 64, false).expect("open");
        // SAFETY: inside the 64-byte mapping.
        assert_eq!(unsafe { b.as_ptr().add(3).read() }, 0x5a);
        unlink(name).expect("unlink");
        assert!(Mapping::open(name, 64, false).is_err());
    }
}
