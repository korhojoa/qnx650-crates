// SPDX-License-Identifier: MIT OR Apache-2.0
//! POSIX shared memory on QNX 6.5: create or open a named object and map
//! it. This is how the daemons on the unit publish their rings and state
//! blocks (`/hidkbm-*`, `/btgamepad`, `/usbgamepad`, `/bnep-iface` ...).
//!
//! The constants are QNX's, from the SDP headers (`fcntl.h`, `sys/mman.h`,
//! `sys/stat.h`; the headers write them in octal). They differ from
//! Linux: `O_CREAT` is 0x100, `PROT_READ` 0x100, `PROT_WRITE` 0x200. `off_t`
//! is 32 bits on QNX 6.5, so `ftruncate` and `mmap` take an `i32` there. A
//! host build (tests) uses the Linux values.
//!
//! `#![no_std]`, no allocation. Errors are the `errno` of the failed call.

#![no_std]

use core::ffi::{c_char, c_int, c_uint, c_void, CStr};
use core::ptr::NonNull;

pub use qnx_errno::errno;

/// `open` flag: read only.
pub const O_RDONLY: c_int = 0;
/// `open` flag: read and write.
pub const O_RDWR: c_int = 0o2;
/// `open` flag: create the object when it does not exist.
#[cfg(target_os = "nto")]
pub const O_CREAT: c_int = 0o400;
/// `open` flag: create the object (Linux value, host builds).
#[cfg(not(target_os = "nto"))]
pub const O_CREAT: c_int = 0o100;

/// Owner read permission.
pub const S_IRUSR: c_uint = 0o400;
/// Owner write permission.
pub const S_IWUSR: c_uint = 0o200;
/// Group read permission.
pub const S_IRGRP: c_uint = 0o40;
/// Group write permission.
pub const S_IWGRP: c_uint = 0o20;
/// Others read permission.
pub const S_IROTH: c_uint = 0o4;
/// Others write permission.
pub const S_IWOTH: c_uint = 0o2;
/// Read and write for everyone: what the unit's rings use, because the
/// producer and the consumers run as different users.
pub const MODE_ALL_RW: c_uint = S_IRUSR | S_IWUSR | S_IRGRP | S_IWGRP | S_IROTH | S_IWOTH;

/// `mmap` protection: readable.
#[cfg(target_os = "nto")]
pub const PROT_READ: c_int = 0x100;
/// `mmap` protection: writable.
#[cfg(target_os = "nto")]
pub const PROT_WRITE: c_int = 0x200;
/// `mmap` protection: readable (Linux value, host builds).
#[cfg(not(target_os = "nto"))]
pub const PROT_READ: c_int = 0x1;
/// `mmap` protection: writable (Linux value, host builds).
#[cfg(not(target_os = "nto"))]
pub const PROT_WRITE: c_int = 0x2;
/// `mmap` flag: changes are shared with every other mapping.
pub const MAP_SHARED: c_int = 0x1;

/// `off_t`: 32 bits on QNX 6.5.
#[cfg(target_os = "nto")]
pub type OffT = i32;
/// `off_t` (host builds).
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

/// `MAP_FAILED`, `(void *)-1`.
const MAP_FAILED: *mut c_void = usize::MAX as *mut c_void;

/// A mapped shared memory object. Unmapped on drop, unless [`leak`]ed.
///
/// [`leak`]: Mapping::leak
pub struct Mapping {
    ptr: NonNull<u8>,
    len: usize,
}

/// The errno of the call that just failed, never 0.
fn last_error() -> c_int {
    match errno() {
        0 => -1,
        e => e,
    }
}

/// Map `len` bytes of `fd` and close `fd` (the mapping keeps the object).
fn map_and_close(fd: c_int, len: usize, prot: c_int) -> Result<Mapping, c_int> {
    // SAFETY: fd is an open shm descriptor; a new mapping of len bytes.
    let p = unsafe { mmap(core::ptr::null_mut(), len, prot, MAP_SHARED, fd, 0) };
    let err = if p == MAP_FAILED { last_error() } else { 0 };
    // SAFETY: fd is ours; the mapping (if any) does not need it.
    unsafe { close(fd) };
    match NonNull::new(p.cast::<u8>()) {
        Some(ptr) if err == 0 => Ok(Mapping { ptr, len }),
        _ => Err(if err == 0 { -1 } else { err }),
    }
}

impl Mapping {
    /// Create (or reuse) the object `name`, size it to `len` bytes, set its
    /// permissions to `mode` (with `fchmod`, so the process umask does not
    /// narrow them) and map it read-write.
    ///
    /// # Errors
    /// The errno of the failed call (`shm_open`, `ftruncate`, `fchmod`,
    /// `mmap`); -1 when `len` does not fit `off_t`.
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

    /// Open the existing object `name` and map its first `len` bytes, read
    /// only or read-write.
    ///
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

    /// The start of the mapping.
    #[must_use]
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    /// The mapped length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the mapping is empty (never, for a successful map).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Keep the mapping for the life of the process and return its start
    /// and length.
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

/// Remove the name `name` (existing mappings stay valid).
///
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
        // Written as the headers write them (octal), checked against the
        // hex values the earlier hand copies used.
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
