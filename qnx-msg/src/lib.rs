// SPDX-License-Identifier: MIT OR Apache-2.0
//! QNX 6.5 Neutrino message passing: channels and connections, send,
//! receive and reply, pulses, `name_attach`/`name_open`, `struct sigevent`,
//! `iov_t`, `TimerTimeout`, interrupt events and `ionotify`.
#![no_std]
#![allow(non_snake_case, reason = "the kernel calls keep their C names")]
use core::ffi::{c_char, c_int, c_uint, c_void};
pub use qnx_errno::errno;
pub const SIGEV_PULSE: c_int = 4;
pub const SIGEV_INTR: c_int = 6;
pub const NTO_SIDE_CHANNEL: c_uint = 0x4000_0000;
pub const NTO_TIMEOUT_RECEIVE: c_int = 1 << 5;
pub const NTO_TIMEOUT_INTR: c_int = 1 << 16;
pub const NTO_INTR_FLAGS_TRK_MSK: c_uint = 0x08;
pub const PULSE_CODE_UNBLOCK: i8 = -32;
pub const PULSE_CODE_DISCONNECT: i8 = -33;
pub const NOTIFY_COND_INPUT: c_int = 0x1000_0000;
pub const NOTIFY_COND_OUTPUT: c_int = 0x2000_0000;
pub const NOTIFY_ACTION_POLL: c_int = 0x2;
pub const NOTIFY_ACTION_POLLARM: c_int = 0x3;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MsgInfo {
    pub nd: u32,
    pub srcnd: u32,
    pub pid: i32,
    pub tid: i32,
    pub chid: i32,
    pub scoid: i32,
    pub coid: i32,
    pub msglen: i32,
    pub srcmsglen: i32,
    pub dstmsglen: i32,
    pub priority: i16,
    pub flags: i16,
    pub reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pulse {
    pub r#type: u16,
    pub subtype: u16,
    pub code: i8,
    pub zero: [u8; 3],
    pub value: c_int,
    pub scoid: i32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sigevent {
    pub notify: c_int,
    pub coid: c_int,
    pub value: c_int,
    pub code: i16,
    pub priority: i16,
}
impl Sigevent {
    #[must_use]
    pub const fn pulse(coid: c_int, priority: i16, code: i16, value: c_int) -> Sigevent {
        Sigevent {
            notify: SIGEV_PULSE,
            coid,
            value,
            code,
            priority,
        }
    }
    #[must_use]
    pub const fn intr() -> Sigevent {
        Sigevent {
            notify: SIGEV_INTR,
            coid: 0,
            value: 0,
            code: 0,
            priority: 0,
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Iov {
    pub base: *mut c_void,
    pub len: usize,
}
impl Iov {
    #[must_use]
    pub const fn new(base: *mut c_void, len: usize) -> Iov {
        Iov { base, len }
    }
    #[must_use]
    pub const fn from_slice(bytes: &[u8]) -> Iov {
        Iov {
            base: bytes.as_ptr().cast_mut().cast(),
            len: bytes.len(),
        }
    }
    #[must_use]
    pub fn from_mut_slice(bytes: &mut [u8]) -> Iov {
        Iov {
            base: bytes.as_mut_ptr().cast(),
            len: bytes.len(),
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NameAttach {
    pub dpp: *mut c_void,
    pub chid: c_int,
    pub mntid: c_int,
    pub zero: [c_int; 2],
}
#[cfg(target_pointer_width = "32")]
const _: () = {
    use core::mem::{offset_of, size_of};
    assert!(size_of::<MsgInfo>() == 48);
    assert!(offset_of!(MsgInfo, pid) == 8);
    assert!(offset_of!(MsgInfo, scoid) == 20);
    assert!(offset_of!(MsgInfo, priority) == 40);
    assert!(offset_of!(MsgInfo, reserved) == 44);
    assert!(size_of::<Pulse>() == 16);
    assert!(offset_of!(Pulse, code) == 4);
    assert!(offset_of!(Pulse, value) == 8);
    assert!(offset_of!(Pulse, scoid) == 12);
    assert!(size_of::<Sigevent>() == 16);
    assert!(offset_of!(Sigevent, coid) == 4);
    assert!(offset_of!(Sigevent, value) == 8);
    assert!(offset_of!(Sigevent, code) == 12);
    assert!(offset_of!(Sigevent, priority) == 14);
    assert!(size_of::<Iov>() == 8);
    assert!(offset_of!(Iov, len) == 4);
    assert!(size_of::<NameAttach>() == 20);
    assert!(offset_of!(NameAttach, chid) == 4);
    assert!(offset_of!(NameAttach, mntid) == 8);
};
#[cfg(not(target_os = "nto"))]
pub const HOST_ENOSYS: c_int = 38;
#[cfg(not(target_os = "nto"))]
#[doc(hidden)]
pub fn host_enosys() {
    qnx_errno::set_errno(HOST_ENOSYS);
}
/// ```
/// qnx_msg::qnx_calls! {
///     /// `ChannelCreate(flags)`.
///     pub fn MyChannelCreate(flags: core::ffi::c_uint) -> core::ffi::c_int = -1;
/// }
/// ```
#[macro_export]
macro_rules! qnx_calls {
    ($(
        $(#[$meta:meta])*
        pub fn $name:ident($($arg:ident: $ty:ty),* $(,)?) $(-> $ret:ty)? = $fail:expr;
    )*) => {
        #[cfg(target_os = "nto")]
        extern "C" {
            $(
                $(#[$meta])*
                pub fn $name($($arg: $ty),*) $(-> $ret)?;
            )*
        }
        $(
            $(#[$meta])*
                        /// # Safety
            /// The host substitute does not use its arguments.
            /// The target call has the conditions of its C function.
            #[cfg(not(target_os = "nto"))]
            pub unsafe fn $name($($arg: $ty),*) $(-> $ret)? {
                let _ = ($($arg,)*);
                $crate::host_enosys();
                $fail
            }
        )*
    };
}
qnx_calls! {
    pub fn ChannelCreate(flags: c_uint) -> c_int = -1;
    pub fn ChannelDestroy(chid: c_int) -> c_int = -1;
    pub fn ConnectAttach(nd: u32, pid: c_int, chid: c_int, index: c_uint, flags: c_int) -> c_int = -1;
    pub fn ConnectDetach(coid: c_int) -> c_int = -1;
    pub fn MsgSend(coid: c_int, smsg: *const c_void, sbytes: c_int, rmsg: *mut c_void, rbytes: c_int) -> c_int = -1;
    pub fn MsgSendv(coid: c_int, siov: *const Iov, sparts: c_int, riov: *const Iov, rparts: c_int) -> c_int = -1;
    pub fn MsgReceive(chid: c_int, msg: *mut c_void, bytes: c_int, info: *mut MsgInfo) -> c_int = -1;
    pub fn MsgReceivePulse(chid: c_int, pulse: *mut c_void, bytes: c_int, info: *mut MsgInfo) -> c_int = -1;
    pub fn MsgReply(rcvid: c_int, status: c_int, msg: *const c_void, bytes: c_int) -> c_int = -1;
    pub fn MsgReplyv(rcvid: c_int, status: c_int, iov: *const Iov, parts: c_int) -> c_int = -1;
    pub fn MsgError(rcvid: c_int, err: c_int) -> c_int = -1;
    pub fn MsgSendPulse(coid: c_int, priority: c_int, code: c_int, value: c_int) -> c_int = -1;
    pub fn TimerTimeout(id: c_int, flags: c_int, notify: *const Sigevent, ntime: *const u64, otime: *mut u64) -> c_int = -1;
    pub fn InterruptAttachEvent(intr: c_int, event: *const Sigevent, flags: c_uint) -> c_int = -1;
    pub fn InterruptDetach(id: c_int) -> c_int = -1;
    pub fn InterruptWait(flags: c_int, timeout: *const u64) -> c_int = -1;
    pub fn InterruptUnmask(intr: c_int, id: c_int) -> c_int = -1;
    pub fn ionotify(fd: c_int, action: c_int, flags: c_int, event: *const Sigevent) -> c_int = -1;
    pub fn name_attach(dpp: *mut c_void, path: *const c_char, flags: c_uint) -> *mut NameAttach = core::ptr::null_mut();
    pub fn name_detach(attach: *mut NameAttach, flags: c_uint) -> c_int = -1;
    pub fn name_open(name: *const c_char, flags: c_int) -> c_int = -1;
    pub fn name_close(coid: c_int) -> c_int = -1;
}
fn last_error() -> c_int {
    match errno() {
        0 => -1,
        e => e,
    }
}
/// # Errors
/// The errno (never 0) when `rc` is negative.
pub fn check(rc: c_int) -> Result<c_int, c_int> {
    if rc < 0 {
        Err(last_error())
    } else {
        Ok(rc)
    }
}
/// # Errors
/// The errno of `MsgReceivePulse`: `ETIMEDOUT` after a [`timer_timeout`]
/// with [`NTO_TIMEOUT_RECEIVE`], `EINTR` on a signal.
pub fn receive_pulse(chid: c_int) -> Result<Pulse, c_int> {
    let mut p = Pulse::default();
    let len = c_int::try_from(core::mem::size_of::<Pulse>()).unwrap_or(c_int::MAX);
    // SAFETY: a writable buffer of `len` bytes. No info requested.
    let rc = unsafe { MsgReceivePulse(chid, (&raw mut p).cast(), len, core::ptr::null_mut()) };
    check(rc).map(|_| p)
}
/// # Errors
/// The errno of `MsgSendPulse` (`EAGAIN` when the kernel has no pulse
/// buffer left, `ESRCH` when the channel is gone).
pub fn send_pulse(coid: c_int, priority: c_int, code: c_int, value: c_int) -> Result<(), c_int> {
    // SAFETY: integer arguments only.
    check(unsafe { MsgSendPulse(coid, priority, code, value) }).map(|_| ())
}
/// # Errors
/// The errno of `TimerTimeout`.
pub fn timer_timeout(clock: c_int, states: c_int, ns: u64) -> Result<(), c_int> {
    // SAFETY: a valid time. A null event is SIGEV_UNBLOCK. No old time.
    let rc = unsafe {
        TimerTimeout(
            clock,
            states,
            core::ptr::null(),
            &raw const ns,
            core::ptr::null_mut(),
        )
    };
    check(rc).map(|_| ())
}
/// # Errors
/// The errno of `ionotify`.
pub fn notify(
    fd: c_int,
    action: c_int,
    conditions: c_int,
    event: Option<&Sigevent>,
) -> Result<c_int, c_int> {
    let ev = event.map_or(core::ptr::null(), core::ptr::from_ref);
    // SAFETY: `ev` is null or a live sigevent the call only reads.
    check(unsafe { ionotify(fd, action, conditions, ev) })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn values_match_the_sdp_headers() {
        assert_eq!(NTO_SIDE_CHANNEL, 1_073_741_824);
        assert_eq!((NTO_TIMEOUT_RECEIVE, NTO_TIMEOUT_INTR), (32, 65_536));
        assert_eq!((SIGEV_PULSE, SIGEV_INTR), (4, 6));
        assert_eq!((PULSE_CODE_UNBLOCK, PULSE_CODE_DISCONNECT), (-32, -33));
        assert_eq!(NOTIFY_COND_INPUT | NOTIFY_COND_OUTPUT, 0x3000_0000);
        assert_eq!((NOTIFY_ACTION_POLL, NOTIFY_ACTION_POLLARM), (2, 3));
        assert_eq!(NTO_INTR_FLAGS_TRK_MSK, 8);
    }
    #[test]
    fn sigevent_initialisers_set_what_the_macros_set() {
        let e = Sigevent::pulse(7, 10, 1, -5);
        assert_eq!(
            (e.notify, e.coid, e.priority, e.code, e.value),
            (SIGEV_PULSE, 7, 10, 1, -5)
        );
        assert_eq!(
            Sigevent::intr(),
            Sigevent {
                notify: SIGEV_INTR,
                ..Sigevent::default()
            }
        );
    }
    #[test]
    fn iov_covers_the_slice() {
        let mut buf = [0u8; 12];
        let a = Iov::from_slice(&buf);
        assert_eq!(
            (a.base.cast_const().cast::<u8>(), a.len),
            (buf.as_ptr(), 12)
        );
        let b = Iov::from_mut_slice(&mut buf[4..]);
        assert_eq!(b.len, 8);
    }
    #[test]
    fn every_call_has_an_address() {
        let calls = [
            ChannelCreate as *const (),
            ChannelDestroy as *const (),
            ConnectAttach as *const (),
            ConnectDetach as *const (),
            MsgSend as *const (),
            MsgSendv as *const (),
            MsgReceive as *const (),
            MsgReceivePulse as *const (),
            MsgReply as *const (),
            MsgReplyv as *const (),
            MsgError as *const (),
            MsgSendPulse as *const (),
            TimerTimeout as *const (),
            InterruptAttachEvent as *const (),
            InterruptDetach as *const (),
            InterruptWait as *const (),
            InterruptUnmask as *const (),
            ionotify as *const (),
            name_attach as *const (),
            name_detach as *const (),
            name_open as *const (),
            name_close as *const (),
        ];
        assert!(core::hint::black_box(calls).iter().all(|p| !p.is_null()));
    }
    #[test]
    #[cfg(not(target_os = "nto"))]
    fn host_stand_ins_fail_with_enosys() {
        assert_eq!(receive_pulse(1), Err(HOST_ENOSYS));
        assert_eq!(send_pulse(1, 10, 1, 0), Err(HOST_ENOSYS));
        assert_eq!(timer_timeout(0, NTO_TIMEOUT_RECEIVE, 1), Err(HOST_ENOSYS));
        assert_eq!(
            notify(0, NOTIFY_ACTION_POLL, NOTIFY_COND_INPUT, None),
            Err(HOST_ENOSYS)
        );
        // SAFETY: host stand-ins.
        unsafe {
            assert!(name_attach(core::ptr::null_mut(), c"x".as_ptr(), 0).is_null());
            assert_eq!(ChannelCreate(0), -1);
        }
        assert_eq!(check(3), Ok(3));
    }
}
