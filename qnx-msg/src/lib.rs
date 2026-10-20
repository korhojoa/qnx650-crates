// SPDX-License-Identifier: MIT OR Apache-2.0
//! QNX 6.5 Neutrino message passing: channels and connections, send,
//! receive and reply, pulses, `name_attach`/`name_open`, `struct sigevent`,
//! `iov_t`, `TimerTimeout`, interrupt events and `ionotify`.
//!
//! The raw calls keep their C names and signatures (`sys/neutrino.h`,
//! `sys/dispatch.h`, `sys/iomgr.h`). All of them are in `libc.so.3` on the
//! unit, so nothing extra is linked. The structs are laid out as the SDP
//! 6.5.0 headers lay them out for the 32-bit ARM target; `LAYOUT.md` has
//! every size and offset with the header line it comes from, measured with
//! the SDP headers and the target compiler, and the const asserts below
//! pin them for every 32-bit build.
//!
//! A host build (tests, `cargo doc`) has no Neutrino kernel: each call is a
//! stand-in that sets `errno` to `ENOSYS` and fails (-1, or null for
//! `name_attach`), so code above it compiles and its host tests run.
//!
//! The small safe helpers ([`receive_pulse`], [`send_pulse`],
//! [`timer_timeout`], [`notify`]) return the `errno` of a failed call.
//!
//! `#![no_std]`, no allocation.

#![no_std]
#![allow(non_snake_case, reason = "the kernel calls keep their C names")]

use core::ffi::{c_char, c_int, c_uint, c_void};

pub use qnx_errno::errno;

// ---- constants -----------------------------------------------------------

/// `SIGEV_PULSE` (`sys/siginfo.h` line 76): deliver a pulse.
pub const SIGEV_PULSE: c_int = 4;
/// `SIGEV_INTR` (`sys/siginfo.h` line 78): wake `InterruptWait`.
pub const SIGEV_INTR: c_int = 6;

/// `_NTO_SIDE_CHANNEL` (`sys/neutrino.h` line 370): the `index` for
/// `ConnectAttach` that gives a side-channel connection id, one that is
/// not a file descriptor.
pub const NTO_SIDE_CHANNEL: c_uint = 0x4000_0000;

/// `_NTO_TIMEOUT_RECEIVE` (`sys/neutrino.h` line 101): `1 << STATE_RECEIVE`,
/// and `STATE_RECEIVE` is 5 (`sys/states.h` line 45).
pub const NTO_TIMEOUT_RECEIVE: c_int = 1 << 5;
/// `_NTO_TIMEOUT_INTR` (`sys/neutrino.h` line 110): `1 << STATE_INTR`, and
/// `STATE_INTR` is 16 (`sys/states.h` line 58).
pub const NTO_TIMEOUT_INTR: c_int = 1 << 16;

/// `_NTO_INTR_FLAGS_TRK_MSK` (`sys/neutrino.h` line 388): the kernel
/// tracks the mask count of the interrupt, so `InterruptDetach` unmasks it.
pub const NTO_INTR_FLAGS_TRK_MSK: c_uint = 0x08;

/// `_PULSE_CODE_UNBLOCK` (`sys/neutrino.h` line 215): a client blocked in
/// `MsgSend` wants out; the pulse value is its receive id.
pub const PULSE_CODE_UNBLOCK: i8 = -32;
/// `_PULSE_CODE_DISCONNECT` (`sys/neutrino.h` line 216): a client's last
/// connection to the channel went away; the value is the server
/// connection id.
pub const PULSE_CODE_DISCONNECT: i8 = -33;

/// `_NOTIFY_COND_INPUT` (`sys/iomgr.h` line 56): data can be read.
pub const NOTIFY_COND_INPUT: c_int = 0x1000_0000;
/// `_NOTIFY_COND_OUTPUT` (`sys/iomgr.h` line 55): there is room to write.
pub const NOTIFY_COND_OUTPUT: c_int = 0x2000_0000;
/// `_NOTIFY_ACTION_POLL` (`sys/iomgr.h` line 118): return the conditions
/// that hold now, arm nothing.
pub const NOTIFY_ACTION_POLL: c_int = 0x2;
/// `_NOTIFY_ACTION_POLLARM` (`sys/iomgr.h` line 119): return the conditions
/// that hold now, and arm a one-shot event for those that do not.
pub const NOTIFY_ACTION_POLLARM: c_int = 0x3;

// ---- structs -------------------------------------------------------------

/// `struct _msg_info` (`sys/neutrino.h` lines 113-127): 48 bytes. What
/// `MsgReceive` reports about the sender.
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

/// `struct _pulse` (`sys/neutrino.h` lines 226-233): 16 bytes.
///
/// `value` is `union sigval`; on the 32-bit target its `int` and its
/// pointer are the same 4 bytes, so it is carried as the `int`.
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

/// `struct sigevent` (`sys/siginfo.h` lines 88-130): 16 bytes.
///
/// The two unions are carried as their members that this crate's users
/// set: `coid` is `sigev_coid` (also `sigev_signo` and `sigev_id`), and
/// `code`/`priority` are the short pair that shares its word with
/// `sigev_notify_attributes`. `value` is `union sigval`, as in [`Pulse`].
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
    /// `SIGEV_PULSE_INIT(e, coid, priority, code, value)`: send a pulse with
    /// `code` and `value` at `priority` on connection `coid`.
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

    /// `SIGEV_INTR_INIT(e)`: wake the thread blocked in `InterruptWait`.
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

/// `iov_t`, `struct iovec` (`sys/target_nto.h` lines 154-158): base, then
/// length. 8 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Iov {
    pub base: *mut c_void,
    pub len: usize,
}

impl Iov {
    /// `SETIOV(iov, base, len)`.
    #[must_use]
    pub const fn new(base: *mut c_void, len: usize) -> Iov {
        Iov { base, len }
    }

    /// An entry over `bytes`, for data the kernel reads (a send or a
    /// reply). The pointer is cast to `*mut` as `iov_t` wants; the kernel
    /// does not write through it.
    #[must_use]
    pub const fn from_slice(bytes: &[u8]) -> Iov {
        Iov {
            base: bytes.as_ptr().cast_mut().cast(),
            len: bytes.len(),
        }
    }

    /// An entry over `bytes`, for data the kernel writes (a receive or a
    /// reply buffer).
    #[must_use]
    pub fn from_mut_slice(bytes: &mut [u8]) -> Iov {
        Iov {
            base: bytes.as_mut_ptr().cast(),
            len: bytes.len(),
        }
    }
}

/// `name_attach_t` (`sys/dispatch.h` lines 117-122): 20 bytes. `chid` is
/// the channel `name_attach` created.
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

// ---- the calls -----------------------------------------------------------

/// `ENOSYS` on the host (Linux), the errno of every host stand-in.
#[cfg(not(target_os = "nto"))]
pub const HOST_ENOSYS: c_int = 38;

/// Set `errno` to [`HOST_ENOSYS`]: what every host stand-in does before it
/// fails.
#[cfg(not(target_os = "nto"))]
#[doc(hidden)]
pub fn host_enosys() {
    qnx_errno::set_errno(HOST_ENOSYS);
}

/// Declare C calls of the QNX libc: an `extern "C"` import on QNX, and on
/// the host a stand-in that sets `errno` to `ENOSYS` and returns the value
/// after `=`. The qnx-* crates that bind libc.so.3 calls with no host
/// equivalent use it.
///
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
            ///
            /// # Safety
            /// The host stand-in does not touch its arguments; the target
            /// call has the contract of its C function.
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
    /// `ChannelCreate(flags)`: a new channel id, or -1.
    pub fn ChannelCreate(flags: c_uint) -> c_int = -1;
    /// `ChannelDestroy(chid)`.
    pub fn ChannelDestroy(chid: c_int) -> c_int = -1;
    /// `ConnectAttach(nd, pid, chid, index, flags)`: a connection id, or -1.
    /// `nd` 0 and `pid` 0 mean this node and this process.
    pub fn ConnectAttach(nd: u32, pid: c_int, chid: c_int, index: c_uint, flags: c_int) -> c_int = -1;
    /// `ConnectDetach(coid)`.
    pub fn ConnectDetach(coid: c_int) -> c_int = -1;
    /// `MsgSend(coid, smsg, sbytes, rmsg, rbytes)`: the server's reply
    /// status, or -1.
    pub fn MsgSend(coid: c_int, smsg: *const c_void, sbytes: c_int, rmsg: *mut c_void, rbytes: c_int) -> c_int = -1;
    /// `MsgSendv(coid, siov, sparts, riov, rparts)`.
    pub fn MsgSendv(coid: c_int, siov: *const Iov, sparts: c_int, riov: *const Iov, rparts: c_int) -> c_int = -1;
    /// `MsgReceive(chid, msg, bytes, info)`: a receive id (> 0), 0 for a
    /// pulse, or -1.
    pub fn MsgReceive(chid: c_int, msg: *mut c_void, bytes: c_int, info: *mut MsgInfo) -> c_int = -1;
    /// `MsgReceivePulse(chid, pulse, bytes, info)`: 0, or -1.
    pub fn MsgReceivePulse(chid: c_int, pulse: *mut c_void, bytes: c_int, info: *mut MsgInfo) -> c_int = -1;
    /// `MsgReply(rcvid, status, msg, bytes)`.
    pub fn MsgReply(rcvid: c_int, status: c_int, msg: *const c_void, bytes: c_int) -> c_int = -1;
    /// `MsgReplyv(rcvid, status, iov, parts)`.
    pub fn MsgReplyv(rcvid: c_int, status: c_int, iov: *const Iov, parts: c_int) -> c_int = -1;
    /// `MsgError(rcvid, err)`: unblock the sender with `errno` `err`.
    pub fn MsgError(rcvid: c_int, err: c_int) -> c_int = -1;
    /// `MsgSendPulse(coid, priority, code, value)`.
    pub fn MsgSendPulse(coid: c_int, priority: c_int, code: c_int, value: c_int) -> c_int = -1;
    /// `TimerTimeout(id, flags, notify, ntime, otime)`: a kernel timeout
    /// for the next blocking call in one of the `flags` states. `id` is a
    /// clock id; a null `notify` means `SIGEV_UNBLOCK`.
    pub fn TimerTimeout(id: c_int, flags: c_int, notify: *const Sigevent, ntime: *const u64, otime: *mut u64) -> c_int = -1;
    /// `InterruptAttachEvent(intr, event, flags)`: an interrupt id, or -1.
    pub fn InterruptAttachEvent(intr: c_int, event: *const Sigevent, flags: c_uint) -> c_int = -1;
    /// `InterruptDetach(id)`.
    pub fn InterruptDetach(id: c_int) -> c_int = -1;
    /// `InterruptWait(flags, timeout)`: block until a `SIGEV_INTR` event.
    pub fn InterruptWait(flags: c_int, timeout: *const u64) -> c_int = -1;
    /// `InterruptUnmask(intr, id)`: the mask count left, or -1.
    pub fn InterruptUnmask(intr: c_int, id: c_int) -> c_int = -1;
    /// `ionotify(fd, action, flags, event)`: the conditions that hold now,
    /// or -1 (`sys/iomgr.h` line 151).
    pub fn ionotify(fd: c_int, action: c_int, flags: c_int, event: *const Sigevent) -> c_int = -1;
    /// `name_attach(dpp, path, flags)`: a channel registered under
    /// `/dev/name/local/<path>`, or null. `dpp` may be null.
    pub fn name_attach(dpp: *mut c_void, path: *const c_char, flags: c_uint) -> *mut NameAttach = core::ptr::null_mut();
    /// `name_detach(attach, flags)`.
    pub fn name_detach(attach: *mut NameAttach, flags: c_uint) -> c_int = -1;
    /// `name_open(name, flags)`: a connection id to the named channel, or
    /// -1.
    pub fn name_open(name: *const c_char, flags: c_int) -> c_int = -1;
    /// `name_close(coid)`.
    pub fn name_close(coid: c_int) -> c_int = -1;
}

// ---- safe helpers --------------------------------------------------------

/// The errno of the call that just failed, never 0.
fn last_error() -> c_int {
    match errno() {
        0 => -1,
        e => e,
    }
}

/// `rc` when it is not negative, else the errno of the call that set it.
///
/// # Errors
/// The errno (never 0) when `rc` is negative.
pub fn check(rc: c_int) -> Result<c_int, c_int> {
    if rc < 0 {
        Err(last_error())
    } else {
        Ok(rc)
    }
}

/// Block on channel `chid` until a pulse arrives (`MsgReceivePulse`).
///
/// # Errors
/// The errno of `MsgReceivePulse`: `ETIMEDOUT` after a [`timer_timeout`]
/// with [`NTO_TIMEOUT_RECEIVE`], `EINTR` on a signal.
pub fn receive_pulse(chid: c_int) -> Result<Pulse, c_int> {
    let mut p = Pulse::default();
    let len = c_int::try_from(core::mem::size_of::<Pulse>()).unwrap_or(c_int::MAX);
    // SAFETY: a writable buffer of `len` bytes; no info requested.
    let rc = unsafe { MsgReceivePulse(chid, (&raw mut p).cast(), len, core::ptr::null_mut()) };
    check(rc).map(|_| p)
}

/// Send a pulse with `code` and `value` at `priority` on connection `coid`
/// (`MsgSendPulse`). It does not block.
///
/// # Errors
/// The errno of `MsgSendPulse` (`EAGAIN` when the kernel has no pulse
/// buffer left, `ESRCH` when the channel is gone).
pub fn send_pulse(coid: c_int, priority: c_int, code: c_int, value: c_int) -> Result<(), c_int> {
    // SAFETY: integer arguments only.
    check(unsafe { MsgSendPulse(coid, priority, code, value) }).map(|_| ())
}

/// Arm a kernel timeout of `ns` nanoseconds on clock `clock` for the next
/// blocking call of this thread in one of the `states` (for example
/// [`NTO_TIMEOUT_RECEIVE`]). That call then fails with `ETIMEDOUT`. 0
/// makes it a poll.
///
/// # Errors
/// The errno of `TimerTimeout`.
pub fn timer_timeout(clock: c_int, states: c_int, ns: u64) -> Result<(), c_int> {
    // SAFETY: a valid time; a null event is SIGEV_UNBLOCK; no old time.
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

/// `ionotify(fd, action, conditions, event)`: the conditions of
/// `conditions` that hold now. With [`NOTIFY_ACTION_POLLARM`] and an
/// event, the event is armed (one-shot) for the ones that do not.
///
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
        // Measured with the SDP 6.5.0 headers and the target compiler
        // (LAYOUT.md).
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

    /// Takes the address of every call, so a target test binary imports
    /// each symbol and the link proves libc.so.3 exports it.
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
        // black_box: the optimiser would otherwise fold the check away and
        // drop the references.
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
