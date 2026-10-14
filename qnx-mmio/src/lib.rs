// SPDX-License-Identifier: Apache-2.0
//! Device memory on QNX 6.5: what a driver or a diagnostic tool needs to
//! touch a peripheral's registers or hand a physical buffer to hardware.
#![no_std]
mod addr;
mod mapping;
pub use addr::{map_extent, Len, PhysAddr, PAGE_SIZE};
use core::ffi::c_int;
#[cfg(target_os = "nto")]
use core::ffi::c_void;
use core::fmt;
pub use mapping::DeviceMapping;
pub type CInt = c_int;
pub type CUint = core::ffi::c_uint;
#[cfg(target_os = "nto")]
extern "C" {
    pub fn mmap_device_memory(
        addr: *mut c_void,
        len: usize,
        prot: CInt,
        flags: CInt,
        physical: u64,
    ) -> *mut c_void;
    pub fn munmap_device_memory(addr: *mut c_void, len: usize) -> CInt;
    pub fn ThreadCtl(cmd: CInt, data: *mut c_void) -> CInt;
    #[link_name = "mmap64"]
    pub fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: CInt,
        flags: CInt,
        fd: CInt,
        off: i64,
    ) -> *mut c_void;
    pub fn munmap(addr: *mut c_void, len: usize) -> CInt;
    pub fn mem_offset64(
        addr: *const c_void,
        fd: CInt,
        len: usize,
        offset: *mut i64,
        contig_len: *mut usize,
    ) -> CInt;
}
#[cfg(not(target_os = "nto"))]
mod host {
    use super::CInt;
    use core::ffi::c_void;
    /// # Safety
    /// The function has the QNX C signature.
    /// The function does not dereference pointers.
    pub unsafe fn mmap_device_memory(
        _addr: *mut c_void,
        _len: usize,
        _prot: CInt,
        _flags: CInt,
        _physical: u64,
    ) -> *mut c_void {
        usize::MAX as *mut c_void
    }
    /// # Safety
    /// As above.
    pub unsafe fn munmap_device_memory(_addr: *mut c_void, _len: usize) -> CInt {
        0
    }
    /// # Safety
    /// As above.
    #[allow(non_snake_case)]
    pub unsafe fn ThreadCtl(_cmd: CInt, _data: *mut c_void) -> CInt {
        -1
    }
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
    /// # Safety
    /// As above.
    pub unsafe fn munmap(_addr: *mut c_void, _len: usize) -> CInt {
        0
    }
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
pub const NOFD: CInt = -1;
pub const PROT_READ: CInt = 0x0000_0100;
pub const PROT_WRITE: CInt = 0x0000_0200;
pub const PROT_NOCACHE: CInt = 0x0000_0800;
pub const MAP_SHARED: CInt = 0x0000_0001;
pub const MAP_ANON: CInt = 0x0008_0000;
pub const MAP_PHYS: CInt = 0x0001_0000;
pub const MAP_NOX64K: CInt = 0x0002_0000;
pub const NTO_TCTL_IO: CInt = 1;
#[cfg(target_os = "nto")]
#[must_use]
pub fn errno() -> CInt {
    qnx_errno::errno()
}
#[cfg(not(target_os = "nto"))]
#[must_use]
pub fn errno() -> CInt {
    0
}
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct Error {
    pub errno: CInt,
    pub what: &'static str,
}
impl Error {
    #[must_use]
    pub fn last(what: &'static str) -> Error {
        Error {
            errno: errno(),
            what,
        }
    }
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
pub type Result<T> = core::result::Result<T, Error>;
/// # Errors
/// The function fails if the caller does not have the necessary I/O privilege.
#[must_use = "a tool that ignores this will map nothing and report success"]
pub fn request_io_privilege() -> Result<()> {
    // SAFETY: ThreadCtl with _NTO_TCTL_IO takes no data pointer. The null
    // is what the QNX documentation specifies for this command.
    let rc = unsafe { ThreadCtl(NTO_TCTL_IO, core::ptr::null_mut()) };
    if rc == -1 {
        return Err(Error::last("ThreadCtl(_NTO_TCTL_IO): run as root"));
    }
    Ok(())
}
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
