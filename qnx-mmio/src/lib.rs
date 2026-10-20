// SPDX-License-Identifier: Apache-2.0
//! Device memory on QNX 6.5: what a driver or a diagnostic tool needs to
//! touch a peripheral's registers or hand a physical buffer to hardware.
//!
//! * I/O privilege: [`request_io_privilege`] (`ThreadCtl(_NTO_TCTL_IO)`).
//! * Device mappings with RAII and volatile, bounds-checked 32-bit
//!   register access: [`DeviceMapping`] (`mmap_device_memory`).
//! * Physical memory: [`mmap`] (`mmap64`, with `MAP_PHYS | MAP_ANON` for
//!   contiguous buffers) and [`mem_offset64`] (the physical address of a
//!   mapping).
//! * Checked address types: [`PhysAddr`], [`Len`], [`map_extent`].
//! * Errors that keep the raw `errno`: [`Error`].
//!
//! The constants are QNX's, from the SDP headers (`sys/mman.h`,
//! `sys/neutrino.h`), measured with the SDP compiler (`LAYOUT.md`). They
//! are the same on ARM and x86.
//!
//! Host builds (tests) get substitutes for the QNX calls that always fail
//! and never touch memory: a host test that appears to map a peripheral is
//! worse than one that cannot run. The substitutes are Rust functions, not
//! C stubs: a C stub named `mmap64` or `munmap` shadows glibc for the
//! whole test process (an earlier version did that and the test binary
//! aborted while Rust set up a thread's signal stack).
//!
//! Provides typed physical addresses and mapped I/O access.

#![no_std]

mod addr;
mod mapping;

pub use addr::{map_extent, Len, PhysAddr, PAGE_SIZE};
pub use mapping::DeviceMapping;

use core::ffi::c_int;
#[cfg(target_os = "nto")]
use core::ffi::c_void;
use core::fmt;

/// `int` of the C calls.
pub type CInt = c_int;
/// `unsigned int` of the C calls.
pub type CUint = core::ffi::c_uint;

#[cfg(target_os = "nto")]
extern "C" {
    /// Map `len` bytes of device (physical) memory at `physical`.
    pub fn mmap_device_memory(
        addr: *mut c_void,
        len: usize,
        prot: CInt,
        flags: CInt,
        physical: u64,
    ) -> *mut c_void;

    /// Unmap a [`mmap_device_memory`] mapping.
    pub fn munmap_device_memory(addr: *mut c_void, len: usize) -> CInt;

    /// Thread control; `_NTO_TCTL_IO` asks for I/O privilege.
    pub fn ThreadCtl(cmd: CInt, data: *mut c_void) -> CInt;

    /// Ordinary `mmap`, used with `MAP_PHYS | MAP_ANON` for buffers that
    /// hardware reads. `mmap64` rather than `mmap` because `off_t` is 32
    /// bits here unless `_FILE_OFFSET_BITS=64`, and an ABI that changes
    /// with a C macro is not one to guess at.
    #[link_name = "mmap64"]
    pub fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: CInt,
        flags: CInt,
        fd: CInt,
        off: i64,
    ) -> *mut c_void;

    /// Unmap an [`mmap`] mapping.
    pub fn munmap(addr: *mut c_void, len: usize) -> CInt;

    /// The physical address of a mapping, and how much of it is
    /// contiguous.
    pub fn mem_offset64(
        addr: *const c_void,
        fd: CInt,
        len: usize,
        offset: *mut i64,
        contig_len: *mut usize,
    ) -> CInt;
}

/// Host substitutes, so pure-logic tests can link and run. Every one of
/// them fails.
#[cfg(not(target_os = "nto"))]
mod host {
    use super::CInt;
    use core::ffi::c_void;

    /// Fails with `MAP_FAILED`.
    ///
    /// # Safety
    /// Signature-compatible with the QNX original; never dereferences.
    pub unsafe fn mmap_device_memory(
        _addr: *mut c_void,
        _len: usize,
        _prot: CInt,
        _flags: CInt,
        _physical: u64,
    ) -> *mut c_void {
        usize::MAX as *mut c_void
    }

    /// Does nothing.
    ///
    /// # Safety
    /// As above.
    pub unsafe fn munmap_device_memory(_addr: *mut c_void, _len: usize) -> CInt {
        0
    }

    /// Fails with -1.
    ///
    /// # Safety
    /// As above.
    #[allow(non_snake_case)] // The QNX name, so callers read the same on both.
    pub unsafe fn ThreadCtl(_cmd: CInt, _data: *mut c_void) -> CInt {
        -1
    }

    /// Fails with `MAP_FAILED`.
    ///
    /// # Safety
    /// As above.
    pub unsafe fn mmap(
        _addr: *mut c_void,
        _len: usize,
        _prot: CInt,
        _flags: CInt,
        _fd: CInt,
        _off: i64,
    ) -> *mut c_void {
        usize::MAX as *mut c_void
    }

    /// Does nothing.
    ///
    /// # Safety
    /// As above.
    pub unsafe fn munmap(_addr: *mut c_void, _len: usize) -> CInt {
        0
    }

    /// Fails with -1.
    ///
    /// # Safety
    /// As above.
    pub unsafe fn mem_offset64(
        _addr: *const c_void,
        _fd: CInt,
        _len: usize,
        _offset: *mut i64,
        _contig_len: *mut usize,
    ) -> CInt {
        -1
    }
}

#[cfg(not(target_os = "nto"))]
pub use host::{mem_offset64, mmap, mmap_device_memory, munmap, munmap_device_memory, ThreadCtl};

/// `NOFD`: no file descriptor, for anonymous mappings.
pub const NOFD: CInt = -1;
/// `PROT_READ` (`sys/mman.h`).
pub const PROT_READ: CInt = 0x0000_0100;
/// `PROT_WRITE` (`sys/mman.h`).
pub const PROT_WRITE: CInt = 0x0000_0200;
/// `PROT_NOCACHE` (`sys/mman.h`): uncached, for device memory and buffers
/// that hardware also writes.
pub const PROT_NOCACHE: CInt = 0x0000_0800;
/// `MAP_SHARED` (`sys/mman.h`).
pub const MAP_SHARED: CInt = 0x0000_0001;
/// `MAP_ANON` (`sys/mman.h`).
pub const MAP_ANON: CInt = 0x0008_0000;
/// `MAP_PHYS` (`sys/mman.h`): physically contiguous memory.
pub const MAP_PHYS: CInt = 0x0001_0000;
/// `MAP_NOX64K` (`sys/mman.h`): do not cross a 64 KB boundary.
pub const MAP_NOX64K: CInt = 0x0002_0000;
/// `_NTO_TCTL_IO` (`sys/neutrino.h`).
pub const NTO_TCTL_IO: CInt = 1;

/// The calling thread's `errno`.
#[cfg(target_os = "nto")]
#[must_use]
pub fn errno() -> CInt {
    qnx_errno::errno()
}

/// Host builds: 0. No host substitute above sets `errno`, so a host
/// `errno` would be left over from an unrelated call.
#[cfg(not(target_os = "nto"))]
#[must_use]
pub fn errno() -> CInt {
    0
}

/// A QNX error, carrying the raw errno so nothing is lost in translation.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct Error {
    /// The `errno` of the failed call (0 when the failure has none).
    pub errno: CInt,
    /// What failed.
    pub what: &'static str,
}

impl Error {
    /// `what` failed; the errno is the current [`errno`].
    #[must_use]
    pub fn last(what: &'static str) -> Error {
        Error {
            errno: errno(),
            what,
        }
    }

    /// `what` failed with `errno`.
    #[must_use]
    pub fn new(what: &'static str, errno: CInt) -> Error {
        Error { errno, what }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (errno {})", self.what, self.errno)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A result with an [`Error`].
pub type Result<T> = core::result::Result<T, Error>;

/// Ask for I/O privilege.
///
/// C tools called `ThreadCtl(_NTO_TCTL_IO, 0)` and discarded the result,
/// so a non-root run carried on to a mapping that could not work. Here it
/// is a `Result` and cannot be ignored without saying so.
///
/// # Errors
/// Fails when the caller is not privileged; in practice, not running as
/// root.
#[must_use = "a tool that ignores this will map nothing and report success"]
pub fn request_io_privilege() -> Result<()> {
    // SAFETY: ThreadCtl with _NTO_TCTL_IO takes no data pointer; the null
    // is what the QNX documentation specifies for this command.
    let rc = unsafe { ThreadCtl(NTO_TCTL_IO, core::ptr::null_mut()) };
    if rc == -1 {
        return Err(Error::last("ThreadCtl(_NTO_TCTL_IO): run as root"));
    }
    Ok(())
}

// Both QNX 6.5 targets are 32-bit, and the structures shared with C
// assume it. Asserted only for QNX, so the tests run on a 64-bit host.
#[cfg(target_os = "nto")]
const _: () = assert!(core::mem::size_of::<usize>() == 4);
const _: () = assert!(core::mem::size_of::<CInt>() == 4);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_values_are_the_sdp_headers() {
        assert_eq!((PROT_READ, PROT_WRITE, PROT_NOCACHE), (256, 512, 2048));
        assert_eq!(
            (MAP_SHARED, MAP_ANON, MAP_PHYS, MAP_NOX64K),
            (1, 524_288, 65_536, 131_072)
        );
        assert_eq!((NOFD, NTO_TCTL_IO), (-1, 1));
    }

    #[cfg(not(target_os = "nto"))]
    #[test]
    fn the_host_has_no_io_privilege() {
        let e = request_io_privilege().unwrap_err();
        assert_eq!(e.errno, 0);
    }

    #[test]
    fn errors_say_what_failed() {
        let mut text = [0u8; 64];
        let mut w = Buf(&mut text, 0);
        core::fmt::write(&mut w, format_args!("{}", Error::new("mmap", 12))).unwrap();
        assert_eq!(&w.0[..w.1], b"mmap (errno 12)");
    }

    struct Buf<'a>(&'a mut [u8], usize);

    impl fmt::Write for Buf<'_> {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            let end = self.1 + s.len();
            self.0
                .get_mut(self.1..end)
                .ok_or(fmt::Error)?
                .copy_from_slice(s.as_bytes());
            self.1 = end;
            Ok(())
        }
    }
}
