// SPDX-License-Identifier: Apache-2.0
//! The libc calls the `shm` feature needs besides shared memory: `open`,
//! `read` and `poll` on the PPS objects. Shared memory (`shm_open`,
//! `ftruncate`, `mmap`, `munmap`, `close`, `shm_unlink`) comes from
//! `qnx-shm`.
//!
//! The values are QNX 6.5's (`fcntl.h`, `poll.h`, `errno.h`; measured with
//! the SDP compiler, see `LAYOUT.md`). A host build (tests) uses the Linux
//! values. Until 0.2.0 these came from the `libc` crate, patched to the
//! QNX 6.5 fork by each consumer; the values here are the fork's.

#[cfg(target_os = "nto")]
use core::ffi::c_uint;
use core::ffi::{c_char, c_int, c_short, c_void};

/// `open` flag: do not block (QNX `O_NONBLOCK`, 0o200).
#[cfg(target_os = "nto")]
pub const O_NONBLOCK: c_int = 0o200;
/// `open` flag: do not block (Linux value, host builds).
#[cfg(not(target_os = "nto"))]
pub const O_NONBLOCK: c_int = 0o4000;

/// `poll` event: data to read (QNX: `POLLRDNORM | POLLRDBAND`).
#[cfg(target_os = "nto")]
pub const POLLIN: c_short = 0x0001 | 0x0004;
/// `poll` event: data to read (Linux value, host builds).
#[cfg(not(target_os = "nto"))]
pub const POLLIN: c_short = 0x0001;

/// `EINTR`: the call was interrupted by a signal (4 on QNX and Linux).
pub const EINTR: c_int = 4;

/// `nfds_t`: `unsigned int` on QNX 6.5.
#[cfg(target_os = "nto")]
pub type NfdsT = c_uint;
/// `nfds_t`: `unsigned long` on Linux (host builds).
#[cfg(not(target_os = "nto"))]
pub type NfdsT = core::ffi::c_ulong;

/// `struct pollfd`: 8 bytes on QNX 6.5 and Linux.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PollFd {
    /// The descriptor.
    pub fd: c_int,
    /// The events to wait for.
    pub events: c_short,
    /// The events that happened.
    pub revents: c_short,
}

const _: () = assert!(core::mem::size_of::<PollFd>() == 8);
const _: () = assert!(core::mem::offset_of!(PollFd, revents) == 6);

extern "C" {
    pub fn open(path: *const c_char, oflag: c_int, ...) -> c_int;
    pub fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize;
    pub fn poll(fds: *mut PollFd, nfds: NfdsT, timeout: c_int) -> c_int;
}
