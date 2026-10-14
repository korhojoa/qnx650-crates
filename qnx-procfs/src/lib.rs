// SPDX-License-Identifier: MIT OR Apache-2.0
//! Process information from the QNX 6.5 `/proc` file system: open
//! `/proc/<pid>/as` and ask it for `DCMD_PROC_INFO`, the
//! `debug_process_t` of `sys/debug.h` (`procfs_info` in `sys/procfs.h`).
//! That holds the process's ids, thread, channel and fd counts, and its
//! user and system running time in nanoseconds.
#![no_std]
use core::ffi::{c_char, c_int, c_void};
use core::mem::{offset_of, size_of};
pub use qnx_errno::errno;
const O_RDONLY: c_int = 0;
pub const DCMD_PROC_INFO: c_int = 0x4110_0801;
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: i32,
    pub parent: i32,
    pub flags: u32,
    pub umask: u32,
    pub child: i32,
    pub sibling: i32,
    pub pgrp: i32,
    pub sid: i32,
    pub base_address: u64,
    pub initial_stack: u64,
    pub uid: i32,
    pub gid: i32,
    pub euid: i32,
    pub egid: i32,
    pub suid: i32,
    pub sgid: i32,
    pub sig_ignore: [u32; 2],
    pub sig_queue: [u32; 2],
    pub sig_pending: [u32; 2],
    pub num_chancons: u32,
    pub num_fdcons: u32,
    pub num_threads: u32,
    pub num_timers: u32,
    pub start_time: u64,
    pub utime: u64,
    pub stime: u64,
    pub cutime: u64,
    pub cstime: u64,
    pub priority: u8,
    reserved2: [u8; 7],
    pub extsched: [u8; 8],
    pub pls: u64,
    pub sigstub: u64,
    pub canstub: u64,
    reserved: [u64; 10],
}
const _: () = {
    assert!(size_of::<ProcessInfo>() == 272);
    assert!(core::mem::align_of::<ProcessInfo>() == 8);
    assert!(offset_of!(ProcessInfo, parent) == 4);
    assert!(offset_of!(ProcessInfo, flags) == 8);
    assert!(offset_of!(ProcessInfo, child) == 16);
    assert!(offset_of!(ProcessInfo, sid) == 28);
    assert!(offset_of!(ProcessInfo, base_address) == 32);
    assert!(offset_of!(ProcessInfo, initial_stack) == 40);
    assert!(offset_of!(ProcessInfo, uid) == 48);
    assert!(offset_of!(ProcessInfo, sgid) == 68);
    assert!(offset_of!(ProcessInfo, sig_ignore) == 72);
    assert!(offset_of!(ProcessInfo, sig_queue) == 80);
    assert!(offset_of!(ProcessInfo, sig_pending) == 88);
    assert!(offset_of!(ProcessInfo, num_chancons) == 96);
    assert!(offset_of!(ProcessInfo, num_threads) == 104);
    assert!(offset_of!(ProcessInfo, num_timers) == 108);
    assert!(offset_of!(ProcessInfo, start_time) == 112);
    assert!(offset_of!(ProcessInfo, utime) == 120);
    assert!(offset_of!(ProcessInfo, stime) == 128);
    assert!(offset_of!(ProcessInfo, cutime) == 136);
    assert!(offset_of!(ProcessInfo, cstime) == 144);
    assert!(offset_of!(ProcessInfo, priority) == 152);
    assert!(offset_of!(ProcessInfo, extsched) == 160);
    assert!(offset_of!(ProcessInfo, pls) == 168);
    assert!(offset_of!(ProcessInfo, sigstub) == 176);
    assert!(offset_of!(ProcessInfo, canstub) == 184);
    assert!(offset_of!(ProcessInfo, reserved) == 192);
    assert!((DCMD_PROC_INFO >> 16) & 0x3fff == 272);
};
impl ProcessInfo {
    #[must_use]
    pub const fn zeroed() -> Self {
        Self {
            pid: 0,
            parent: 0,
            flags: 0,
            umask: 0,
            child: 0,
            sibling: 0,
            pgrp: 0,
            sid: 0,
            base_address: 0,
            initial_stack: 0,
            uid: 0,
            gid: 0,
            euid: 0,
            egid: 0,
            suid: 0,
            sgid: 0,
            sig_ignore: [0; 2],
            sig_queue: [0; 2],
            sig_pending: [0; 2],
            num_chancons: 0,
            num_fdcons: 0,
            num_threads: 0,
            num_timers: 0,
            start_time: 0,
            utime: 0,
            stime: 0,
            cutime: 0,
            cstime: 0,
            priority: 0,
            reserved2: [0; 7],
            extsched: [0; 8],
            pls: 0,
            sigstub: 0,
            canstub: 0,
            reserved: [0; 10],
        }
    }
    #[must_use]
    pub const fn running_ns(&self) -> u64 {
        self.utime.saturating_add(self.stime)
    }
}
extern "C" {
    fn open(path: *const c_char, oflag: c_int, ...) -> c_int;
    fn close(fd: c_int) -> c_int;
}
#[cfg(target_os = "nto")]
extern "C" {
    fn devctl(fd: c_int, dcmd: c_int, data: *mut c_void, nbytes: usize, info: *mut c_int) -> c_int;
}
#[cfg(not(target_os = "nto"))]
pub const HOST_ENOSYS: c_int = 38;
#[cfg(not(target_os = "nto"))]
#[allow(clippy::unnecessary_wraps)]
unsafe fn devctl(
    _fd: c_int,
    _dcmd: c_int,
    _data: *mut c_void,
    _nbytes: usize,
    _info: *mut c_int,
) -> c_int {
    HOST_ENOSYS
}
fn as_path(pid: u32, buf: &mut [u8; 24]) -> &[u8] {
    const PREFIX: &[u8] = b"/proc/";
    const SUFFIX: &[u8] = b"/as\0";
    buf[..PREFIX.len()].copy_from_slice(PREFIX);
    let mut digits = [0u8; 10];
    let mut n = pid;
    let mut count = 0;
    loop {
        #[allow(clippy::cast_possible_truncation)]
        let digit = (n % 10) as u8;
        digits[count] = b'0' + digit;
        count += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    let mut at = PREFIX.len();
    for &digit in digits[..count].iter().rev() {
        buf[at] = digit;
        at += 1;
    }
    buf[at..at + SUFFIX.len()].copy_from_slice(SUFFIX);
    &buf[..at + SUFFIX.len()]
}
fn last_error() -> c_int {
    match errno() {
        0 => -1,
        e => e,
    }
}
/// # Errors
/// The errno of the failed `open` (`ENOENT` when the process does not
/// exist, `EACCES` or `EPERM` without the rights), or the error number
/// `devctl` returned (`ENOSYS` on a host).
pub fn process_info(pid: u32) -> Result<ProcessInfo, c_int> {
    let mut buf = [0u8; 24];
    let path = as_path(pid, &mut buf);
    // SAFETY: a NUL-terminated path.
    let fd = unsafe { open(path.as_ptr().cast(), O_RDONLY) };
    if fd < 0 {
        return Err(last_error());
    }
    let mut info = ProcessInfo::zeroed();
    // SAFETY: an open descriptor and a buffer of the size the command
    // carries. No info out-parameter.
    let code = unsafe {
        devctl(
            fd,
            DCMD_PROC_INFO,
            (&raw mut info).cast(),
            size_of::<ProcessInfo>(),
            core::ptr::null_mut(),
        )
    };
    // SAFETY: the descriptor opened above, closed once.
    unsafe { close(fd) };
    if code == 0 {
        Ok(info)
    } else {
        Err(code)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths() {
        let mut buf = [0u8; 24];
        assert_eq!(as_path(0, &mut buf), b"/proc/0/as\0");
        assert_eq!(as_path(1234, &mut buf), b"/proc/1234/as\0");
        assert_eq!(as_path(u32::MAX, &mut buf), b"/proc/4294967295/as\0");
    }
    #[test]
    fn running_time_adds_user_and_system() {
        let mut info = ProcessInfo::zeroed();
        info.utime = 3;
        info.stime = 4;
        assert_eq!(info.running_ns(), 7);
        info.stime = u64::MAX;
        assert_eq!(info.running_ns(), u64::MAX);
    }
    #[cfg(target_os = "nto")]
    #[test]
    fn reads_own_process() {
        // SAFETY: getpid has no preconditions.
        let pid = unsafe { getpid() };
        let info = process_info(pid.unsigned_abs()).expect("DCMD_PROC_INFO");
        assert_eq!(info.pid, pid);
        assert!(info.num_threads >= 1);
    }
    #[cfg(target_os = "nto")]
    extern "C" {
        fn getpid() -> c_int;
    }
    #[cfg(not(target_os = "nto"))]
    #[test]
    fn host_has_no_proc_info() {
        assert!(process_info(1).is_err());
        assert!(process_info(u32::MAX).is_err());
    }
}
