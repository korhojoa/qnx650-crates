// SPDX-License-Identifier: MIT OR Apache-2.0
//! The QNX 6.5 resource manager framework: `dispatch_create`,
//! `resmgr_attach` and the `resmgr_block`/`resmgr_handler` loop, the
//! `iofunc_*` defaults, and the layouts a handler touches
//! (`resmgr_context_t`, the connect and I/O function tables,
//! `iofunc_attr_t`, and the `_IO_CONNECT`, `_IO_READ` and `_IO_DEVCTL`
//! messages).
//!
//! The raw calls keep their C names (`sys/dispatch.h`, `sys/iofunc.h`).
//! The framework is in `libc.so.3` on the unit, so nothing extra is
//! linked. The message and OCB pointers that the C prototypes type as
//! `io_read_t *`, `iofunc_ocb_t *` and so on are `*mut c_void` here; a
//! handler casts `msg` to the message struct it expects ([`IoRead`],
//! [`IoDevctl`], [`IoConnect`]).
//!
//! Every size and offset is the SDP 6.5.0 headers' for the 32-bit ARM
//! target, measured with the target compiler; `LAYOUT.md` lists them with
//! their header lines, and the const asserts below pin them for every
//! 32-bit build. A host build gets stand-ins that set `errno` to `ENOSYS`
//! and fail (-1 or null).
//!
//! `#![no_std]`, no allocation.

#![no_std]
#![allow(non_snake_case, reason = "the framework calls keep their C names")]

use core::ffi::{c_char, c_int, c_uint, c_void};

pub use qnx_msg::{Iov, MsgInfo};

// ---- constants -----------------------------------------------------------

/// `_IO_READ` (`sys/iomsg.h` line 92): the type in the first `u16` of a
/// read message.
pub const IO_READ: u16 = 0x101;
/// `_IO_WRITE` (`sys/iomsg.h` line 93).
pub const IO_WRITE: u16 = 0x102;
/// `_IO_STAT` (`sys/iomsg.h` line 95).
pub const IO_STAT: u16 = 0x104;
/// `_IO_NOTIFY` (`sys/iomsg.h` line 96).
pub const IO_NOTIFY: u16 = 0x105;
/// `_IO_DEVCTL` (`sys/iomsg.h` line 97).
pub const IO_DEVCTL: u16 = 0x106;
/// `_IO_LSEEK` (`sys/iomsg.h` line 100).
pub const IO_LSEEK: u16 = 0x109;
/// `_IO_MSG` (`sys/iomsg.h` line 110).
pub const IO_MSG: u16 = 0x113;

/// `_IO_XTYPE_NONE` (`sys/iomsg.h` line 122): a plain read or write.
pub const IO_XTYPE_NONE: u32 = 0;
/// `_IO_XTYPE_MASK` (`sys/iomsg.h` line 130): the xtype bits of `xtype`.
pub const IO_XTYPE_MASK: u32 = 0xff;

/// `_RESMGR_DEFAULT` (`sys/resmgr.h` line 176): "do the default action",
/// what `iofunc_devctl_default` returns for a command it does not know.
#[allow(
    clippy::cast_possible_wrap,
    reason = "the header's value is the unsigned 0x80000001 stored in an int"
)]
pub const RESMGR_DEFAULT: c_int = 0x8000_0001_u32 as c_int;

/// `_RESMGR_FLAG_BEFORE` (`sys/resmgr.h` line 240): resolve this path
/// ahead of others attached at the same mount point.
pub const RESMGR_FLAG_BEFORE: c_uint = 0x0001;

/// `_FTYPE_ANY` (`sys/ftype.h` line 31).
pub const FTYPE_ANY: c_int = 0;

/// `_RESMGR_CONNECT_NFUNCS` (`sys/resmgr.h` line 138): the pointers in
/// [`ConnectFuncs`].
pub const RESMGR_CONNECT_NFUNCS: c_uint = 8;
/// `_RESMGR_IO_NFUNCS` (`sys/resmgr.h` line 173): the pointers in
/// [`IoFuncs`].
pub const RESMGR_IO_NFUNCS: c_uint = 26;

/// The `_io_devctl` header that comes before the data of a devctl request
/// and of its reply (`struct _io_devctl` and `struct _io_devctl_reply`
/// are both 16 bytes).
pub const DEVCTL_HDR: usize = 16;

// ---- structs -------------------------------------------------------------

/// `dispatch_t` (`sys/resmgr.h` line 57): opaque.
#[repr(C)]
pub struct Dispatch {
    _opaque: [u8; 0],
}

/// `resmgr_context_t` (`sys/resmgr.h` lines 107-119): 84 bytes before the
/// flexible `iov[]`. The framework allocates the context with
/// `nparts_max` entries; this struct names the first one, so a handler
/// that replies with more parts must index past it through a pointer.
#[repr(C)]
pub struct ResmgrContext {
    pub rcvid: c_int,
    pub info: MsgInfo,
    /// `resmgr_iomsgs_t *`: the received message.
    pub msg: *mut c_void,
    pub dpp: *mut Dispatch,
    pub id: c_int,
    /// `struct _extended_context *`.
    pub extra: *mut c_void,
    pub msg_max_size: c_uint,
    pub status: c_int,
    pub offset: c_int,
    pub size: c_int,
    pub iov: [Iov; 1],
}

/// `resmgr_attr_t` (`sys/dispatch.h` lines 90-96): 32 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ResmgrAttr {
    pub flags: c_uint,
    pub nparts_max: c_uint,
    pub msg_max_size: c_uint,
    pub other_func: Option<unsafe extern "C" fn(*mut ResmgrContext, *mut c_void) -> c_int>,
    pub reserved: [c_uint; 4],
}

/// A connect handler: `(ctp, msg, handle, extra)`.
pub type ConnectFn = Option<
    unsafe extern "C" fn(*mut ResmgrContext, *mut c_void, *mut c_void, *mut c_void) -> c_int,
>;
/// An I/O handler: `(ctp, msg, ocb)`.
pub type IoFn = Option<unsafe extern "C" fn(*mut ResmgrContext, *mut c_void, *mut c_void) -> c_int>;

/// `resmgr_connect_funcs_t` (`sys/resmgr.h` lines 127-137): `nfuncs` and 8
/// handlers, 36 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ConnectFuncs {
    pub nfuncs: c_uint,
    pub open: ConnectFn,
    pub unlink: ConnectFn,
    pub rename: ConnectFn,
    pub mknod: ConnectFn,
    pub readlink: ConnectFn,
    pub link: ConnectFn,
    pub unblock: ConnectFn,
    pub mount: ConnectFn,
}

/// `resmgr_io_funcs_t` (`sys/resmgr.h` lines 144-172): `nfuncs` and 26
/// handlers in header order, 108 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct IoFuncs {
    pub nfuncs: c_uint,
    pub read: IoFn,
    pub write: IoFn,
    pub close_ocb: IoFn,
    pub stat: IoFn,
    pub notify: IoFn,
    pub devctl: IoFn,
    pub unblock: IoFn,
    pub pathconf: IoFn,
    pub lseek: IoFn,
    pub chmod: IoFn,
    pub chown: IoFn,
    pub utime: IoFn,
    pub openfd: IoFn,
    pub fdinfo: IoFn,
    pub lock: IoFn,
    pub space: IoFn,
    pub shutdown: IoFn,
    pub mmap: IoFn,
    pub msg: IoFn,
    pub reserved: IoFn,
    pub dup: IoFn,
    pub close_dup: IoFn,
    pub lock_ocb: IoFn,
    pub unlock_ocb: IoFn,
    pub sync: IoFn,
    pub power: IoFn,
}

impl IoFuncs {
    /// No handlers, `nfuncs` 0 (fill it with `iofunc_func_init`).
    pub const EMPTY: IoFuncs = IoFuncs {
        nfuncs: 0,
        read: None,
        write: None,
        close_ocb: None,
        stat: None,
        notify: None,
        devctl: None,
        unblock: None,
        pathconf: None,
        lseek: None,
        chmod: None,
        chown: None,
        utime: None,
        openfd: None,
        fdinfo: None,
        lock: None,
        space: None,
        shutdown: None,
        mmap: None,
        msg: None,
        reserved: None,
        dup: None,
        close_dup: None,
        lock_ocb: None,
        unlock_ocb: None,
        sync: None,
        power: None,
    };
}

/// `iofunc_attr_t` (`sys/iofunc.h` lines 102-165) with the default
/// 64-bit `nbytes` and `inode` (`off64_t`, `ino64_t`, lines 121-122):
/// 88 bytes, 8-aligned. `iofunc_attr_init` fills it.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IofuncAttr {
    /// `iofunc_mount_t *`.
    pub mount: *mut c_void,
    pub flags: u32,
    pub lock_tid: i32,
    pub lock_count: u16,
    pub count: u16,
    pub rcount: u16,
    pub wcount: u16,
    pub rlocks: u16,
    pub wlocks: u16,
    /// `struct _iofunc_mmap_list *`.
    pub mmap_list: *mut c_void,
    /// `struct _iofunc_lock_list *`.
    pub lock_list: *mut c_void,
    pub lockobj: *mut c_void,
    pub reserved: u32,
    pub nbytes: i64,
    pub inode: u64,
    pub uid: i32,
    pub gid: i32,
    pub mtime: u32,
    pub atime: u32,
    pub ctime: u32,
    pub mode: u32,
    pub nlink: u32,
    pub rdev: u32,
}

impl IofuncAttr {
    /// All fields zero (what a C `static iofunc_attr_t` starts as).
    #[must_use]
    pub const fn zeroed() -> IofuncAttr {
        IofuncAttr {
            mount: core::ptr::null_mut(),
            flags: 0,
            lock_tid: 0,
            lock_count: 0,
            count: 0,
            rcount: 0,
            wcount: 0,
            rlocks: 0,
            wlocks: 0,
            mmap_list: core::ptr::null_mut(),
            lock_list: core::ptr::null_mut(),
            lockobj: core::ptr::null_mut(),
            reserved: 0,
            nbytes: 0,
            inode: 0,
            uid: 0,
            gid: 0,
            mtime: 0,
            atime: 0,
            ctime: 0,
            mode: 0,
            nlink: 0,
            rdev: 0,
        }
    }
}

/// `struct _io_connect` (`sys/iomsg.h` lines 165-183) up to the flexible
/// `path[]`: 40 bytes, so the path starts at
/// `size_of::<IoConnect>()`. (The C `sizeof` is 44, because the header's
/// `__FLEXARY` is `path[1]` plus padding.)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IoConnect {
    pub r#type: u16,
    pub subtype: u16,
    pub file_type: u32,
    pub reply_max: u16,
    pub entry_max: u16,
    pub key: u32,
    pub handle: u32,
    pub ioflag: u32,
    pub mode: u32,
    pub sflag: u16,
    pub access: u16,
    pub zero: u16,
    pub path_len: u16,
    pub eflag: u8,
    pub extra_type: u8,
    pub extra_len: u16,
}

/// `struct _io_read` (`sys/iomsg.h` lines 420-426): 16 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IoRead {
    pub r#type: u16,
    pub combine_len: u16,
    pub nbytes: i32,
    pub xtype: u32,
    pub zero: u32,
}

/// `struct _io_devctl` (`sys/iomsg.h` lines 498-505): 16 bytes; the data
/// follows it.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IoDevctl {
    pub r#type: u16,
    pub combine_len: u16,
    pub dcmd: i32,
    pub nbytes: i32,
    pub zero: i32,
}

/// `struct _io_devctl_reply` (`sys/iomsg.h` lines 507-513): 16 bytes, the
/// same bytes as the request (`io_devctl_t` is their union); the reply
/// data follows it.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IoDevctlReply {
    pub zero: u32,
    pub ret_val: i32,
    pub nbytes: i32,
    pub zero2: i32,
}

#[cfg(target_pointer_width = "32")]
const _: () = {
    use core::mem::{align_of, offset_of, size_of};
    assert!(size_of::<ResmgrContext>() == 92);
    assert!(offset_of!(ResmgrContext, info) == 4);
    assert!(offset_of!(ResmgrContext, msg) == 52);
    assert!(offset_of!(ResmgrContext, id) == 60);
    assert!(offset_of!(ResmgrContext, msg_max_size) == 68);
    assert!(offset_of!(ResmgrContext, status) == 72);
    assert!(offset_of!(ResmgrContext, size) == 80);
    assert!(offset_of!(ResmgrContext, iov) == 84);
    assert!(size_of::<ResmgrAttr>() == 32);
    assert!(offset_of!(ResmgrAttr, other_func) == 12);
    assert!(size_of::<ConnectFuncs>() == 36);
    assert!(offset_of!(ConnectFuncs, open) == 4);
    assert!(offset_of!(ConnectFuncs, mount) == 32);
    assert!(size_of::<IoFuncs>() == 108);
    assert!(offset_of!(IoFuncs, read) == 4);
    assert!(offset_of!(IoFuncs, devctl) == 24);
    assert!(offset_of!(IoFuncs, lseek) == 36);
    assert!(offset_of!(IoFuncs, msg) == 76);
    assert!(offset_of!(IoFuncs, power) == 104);
    assert!(size_of::<IofuncAttr>() == 88 && align_of::<IofuncAttr>() == 8);
    assert!(offset_of!(IofuncAttr, flags) == 4);
    assert!(offset_of!(IofuncAttr, lock_count) == 12);
    assert!(offset_of!(IofuncAttr, mmap_list) == 24);
    assert!(offset_of!(IofuncAttr, reserved) == 36);
    assert!(offset_of!(IofuncAttr, nbytes) == 40);
    assert!(offset_of!(IofuncAttr, inode) == 48);
    assert!(offset_of!(IofuncAttr, uid) == 56);
    assert!(offset_of!(IofuncAttr, mtime) == 64);
    assert!(offset_of!(IofuncAttr, mode) == 76);
    assert!(offset_of!(IofuncAttr, nlink) == 80);
    assert!(offset_of!(IofuncAttr, rdev) == 84);
    assert!(size_of::<IoConnect>() == 40);
    assert!(offset_of!(IoConnect, file_type) == 4);
    assert!(offset_of!(IoConnect, key) == 12);
    assert!(offset_of!(IoConnect, sflag) == 28);
    assert!(offset_of!(IoConnect, path_len) == 34);
    assert!(offset_of!(IoConnect, eflag) == 36);
    assert!(offset_of!(IoConnect, extra_len) == 38);
    assert!(size_of::<IoRead>() == 16);
    assert!(offset_of!(IoRead, nbytes) == 4);
    assert!(offset_of!(IoRead, xtype) == 8);
    assert!(size_of::<IoDevctl>() == DEVCTL_HDR);
    assert!(offset_of!(IoDevctl, dcmd) == 4);
    assert!(offset_of!(IoDevctl, nbytes) == 8);
    assert!(size_of::<IoDevctlReply>() == DEVCTL_HDR);
    assert!(offset_of!(IoDevctlReply, ret_val) == 4);
};

// ---- reply helpers -------------------------------------------------------

/// `_RESMGR_NPARTS(n)` (`sys/resmgr.h` line 177): "reply with the first
/// `n` entries of `ctp->iov`".
#[must_use]
pub const fn nparts(n: c_int) -> c_int {
    -n
}

/// `_RESMGR_PTR(ctp, p, n)` (`sys/resmgr.h` line 179): point `iov[0]` at
/// `n` bytes at `p` and reply with that one part.
///
/// # Safety
/// `ctp` is the live context handed to the handler, and the `n` bytes at
/// `p` stay valid until the framework has replied.
pub unsafe fn reply_ptr(ctp: *mut ResmgrContext, p: *const u8, n: usize) -> c_int {
    // SAFETY: the caller's contract; iov[0] always exists.
    unsafe {
        (*ctp).iov[0] = Iov::new(p.cast_mut().cast(), n);
    }
    nparts(1)
}

// ---- the calls -----------------------------------------------------------

qnx_msg::qnx_calls! {
    /// `dispatch_create()`: a dispatch handle, or null.
    pub fn dispatch_create() -> *mut Dispatch = core::ptr::null_mut();
    /// `resmgr_attach(dpp, attr, path, file_type, flags, connect_funcs,
    /// io_funcs, handle)`: a link id, or -1. `handle` is usually the
    /// `iofunc_attr_t`.
    #[allow(clippy::too_many_arguments, reason = "the C signature")]
    pub fn resmgr_attach(
        dpp: *mut Dispatch,
        attr: *mut ResmgrAttr,
        path: *const c_char,
        file_type: c_int,
        flags: c_uint,
        connect_funcs: *const ConnectFuncs,
        io_funcs: *const IoFuncs,
        handle: *mut c_void,
    ) -> c_int = -1;
    /// `resmgr_context_alloc(dpp)`: a context for one receiving thread.
    pub fn resmgr_context_alloc(dpp: *mut Dispatch) -> *mut ResmgrContext = core::ptr::null_mut();
    /// `resmgr_block(ctp)`: receive the next message into `ctp`, or null.
    pub fn resmgr_block(ctp: *mut ResmgrContext) -> *mut ResmgrContext = core::ptr::null_mut();
    /// `resmgr_handler(ctp)`: run the handler for the received message and
    /// reply.
    pub fn resmgr_handler(ctp: *mut ResmgrContext) -> c_int = -1;
    /// `iofunc_func_init(nconnect, connect, nio, io)`: fill both tables with
    /// the `iofunc_*_default` handlers.
    pub fn iofunc_func_init(nconnect: c_uint, connect: *mut ConnectFuncs, nio: c_uint, io: *mut IoFuncs) = ();
    /// `iofunc_attr_init(attr, mode, dattr, info)`. `info` is a
    /// `struct _client_info *` and may be null.
    pub fn iofunc_attr_init(attr: *mut IofuncAttr, mode: u32, dattr: *mut IofuncAttr, info: *mut c_void) = ();
    /// `iofunc_open_default(ctp, msg, attr, extra)`.
    pub fn iofunc_open_default(ctp: *mut ResmgrContext, msg: *mut c_void, attr: *mut c_void, extra: *mut c_void) -> c_int = -1;
    /// `iofunc_close_ocb_default(ctp, reserved, ocb)`.
    pub fn iofunc_close_ocb_default(ctp: *mut ResmgrContext, reserved: *mut c_void, ocb: *mut c_void) -> c_int = -1;
    /// `iofunc_read_verify(ctp, msg, ocb, nonblock)`: 0 (`EOK`) or an
    /// errno.
    pub fn iofunc_read_verify(ctp: *mut ResmgrContext, msg: *mut c_void, ocb: *mut c_void, nonblock: *mut c_int) -> c_int = -1;
    /// `iofunc_devctl_default(ctp, msg, ocb)`: [`RESMGR_DEFAULT`] for a
    /// command it does not handle.
    pub fn iofunc_devctl_default(ctp: *mut ResmgrContext, msg: *mut c_void, ocb: *mut c_void) -> c_int = -1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_match_the_sdp_headers() {
        // Measured with the SDP 6.5.0 headers and the target compiler
        // (LAYOUT.md).
        assert_eq!(
            (IO_READ, IO_WRITE, IO_STAT, IO_NOTIFY, IO_DEVCTL, IO_LSEEK, IO_MSG),
            (257, 258, 260, 261, 262, 265, 275)
        );
        assert_eq!(RESMGR_DEFAULT, -2_147_483_647);
        assert_eq!((RESMGR_CONNECT_NFUNCS, RESMGR_IO_NFUNCS), (8, 26));
        assert_eq!((RESMGR_FLAG_BEFORE, FTYPE_ANY, IO_XTYPE_MASK), (1, 0, 0xff));
        assert_eq!(nparts(1), -1);
    }

    #[test]
    fn reply_ptr_points_iov0_at_the_buffer() {
        // SAFETY: an all-zero context is a valid value of this plain struct.
        let mut ctx: ResmgrContext = unsafe { core::mem::zeroed() };
        let buf = [1u8, 2, 3];
        // SAFETY: a live context and a buffer that outlives the check.
        let r = unsafe { reply_ptr(&raw mut ctx, buf.as_ptr(), 3) };
        assert_eq!(r, -1);
        assert_eq!(
            (ctx.iov[0].base.cast_const().cast::<u8>(), ctx.iov[0].len),
            (buf.as_ptr(), 3)
        );
    }

    #[test]
    fn io_funcs_table_has_26_handlers() {
        let ptr = core::mem::size_of::<IoFn>();
        assert_eq!(
            core::mem::size_of::<IoFuncs>(),
            core::mem::size_of::<c_uint>().next_multiple_of(ptr) + 26 * ptr
        );
        let conn = core::mem::size_of::<ConnectFn>();
        assert_eq!(
            core::mem::size_of::<ConnectFuncs>(),
            core::mem::size_of::<c_uint>().next_multiple_of(conn) + 8 * conn
        );
    }

    /// Takes the address of every call, so a target test binary imports
    /// each symbol and the link proves libc.so.3 exports it.
    #[test]
    fn every_call_has_an_address() {
        let calls = [
            dispatch_create as *const (),
            resmgr_attach as *const (),
            resmgr_context_alloc as *const (),
            resmgr_block as *const (),
            resmgr_handler as *const (),
            iofunc_func_init as *const (),
            iofunc_attr_init as *const (),
            iofunc_open_default as *const (),
            iofunc_close_ocb_default as *const (),
            iofunc_read_verify as *const (),
            iofunc_devctl_default as *const (),
        ];
        // black_box: the optimiser would otherwise fold the check away and
        // drop the references.
        assert!(core::hint::black_box(calls).iter().all(|p| !p.is_null()));
    }

    #[test]
    #[cfg(not(target_os = "nto"))]
    fn host_stand_ins_fail() {
        // SAFETY: host stand-ins.
        unsafe {
            assert!(dispatch_create().is_null());
            assert!(resmgr_block(core::ptr::null_mut()).is_null());
        }
        assert_eq!(qnx_errno::errno(), qnx_msg::HOST_ENOSYS);
    }
}
