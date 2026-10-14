#[cfg(target_os = "nto")]
use core::ffi::c_uint;
use core::ffi::{c_char, c_int, c_short, c_void};
#[cfg(target_os = "nto")]
pub const O_NONBLOCK: c_int = 0o200;
#[cfg(not(target_os = "nto"))]
pub const O_NONBLOCK: c_int = 0o4000;
#[cfg(target_os = "nto")]
pub const POLLIN: c_short = 0x0001 | 0x0004;
#[cfg(not(target_os = "nto"))]
pub const POLLIN: c_short = 0x0001;
pub const EINTR: c_int = 4;
#[cfg(target_os = "nto")]
pub type NfdsT = c_uint;
#[cfg(not(target_os = "nto"))]
pub type NfdsT = core::ffi::c_ulong;
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PollFd {
    pub fd: c_int,
    pub events: c_short,
    pub revents: c_short,
}
const _: () = assert!(core::mem::size_of::<PollFd>() == 8);
const _: () = assert!(core::mem::offset_of!(PollFd, revents) == 6);
extern "C" {
    pub fn open(path: *const c_char, oflag: c_int, ...) -> c_int;
    pub fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize;
    pub fn poll(fds: *mut PollFd, nfds: NfdsT, timeout: c_int) -> c_int;
}
