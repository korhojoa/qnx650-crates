// SPDX-License-Identifier: Apache-2.0
//! The `libusbdi.so.2` calls, each wrapped once, on the QNX target.
//!
//! Objects retain the storage that native interfaces can reference:
//! `HostSys` owns the connection plus the ident, funcs and path records
//! `usbd_connect` was given (boxed, so their addresses stay valid for the
//! connection's life in case the library keeps the pointers). `DeviceSys` owns the attached device, its
//! control pipe and the boxed device descriptor the pipe was opened on;
//! `PipeSys` owns one non-control pipe and the boxed endpoint descriptor
//! it was opened on; `SlotSys` owns one URB and one `usbd_alloc` buffer.
//! Every drop is the matching release call: the bulk pipe is closed
//! before its URBs and buffers are freed (contract item 9: `EOK` from
//! `usbd_close_pipe` is the proof that every completion callback has
//! returned), then the control pipe, then detach, then disconnect.
//! `usbd_close_pipe` and `usbd_detach` answer `EBUSY`, and do nothing,
//! while libusbdi's own per-pipe / per-device in-flight counters are
//! non-zero, which they stay until the event thread has come back from
//! the last callback; both are therefore retried on `EBUSY`, bounded.
//!
//! Copyright 2026. Licensed under the Apache License, Version 2.0.

use core::ffi::{c_int, c_void};
use core::ptr;
use std::ffi::CString;
use std::time::Instant;

use super::{ControlDone, ControlSetup};
use crate::clock;
use crate::desc::Endpoint;
use crate::events;
use crate::ffi::{
    self, UsbdConnectParm, UsbdConnection, UsbdDescNode, UsbdDevice, UsbdDeviceDescriptor,
    UsbdDeviceIdent, UsbdDeviceInstance, UsbdEndpointDescriptor, UsbdFuncs, UsbdIoCallback,
    UsbdPipe, UsbdUrb, EBUSY, EINVAL, EIO, ENOMEM, EOK, URB_DIR_IN, URB_DIR_OUT, URB_SHORT_XFER_OK,
    USBDI_NFUNCS, USBD_VERSION, USB_VERSION,
};
use crate::host::Ident;
use crate::{Error, Result};

/// `EOK` becomes `Ok`, anything else the errno io-usb returned.
fn check(rc: c_int) -> Result<()> {
    if rc == EOK {
        Ok(())
    } else {
        Err(Error::Errno(rc))
    }
}

/// One `usbd_connect`.
pub(crate) struct HostSys {
    conn: *mut UsbdConnection,
    _path: CString,
    _ident: Box<UsbdDeviceIdent>,
    _funcs: Box<UsbdFuncs>,
}

impl HostSys {
    pub(crate) fn connect(path: &str, ident: Ident, connect_wait_s: u16) -> Result<HostSys> {
        let cpath = CString::new(path).map_err(|_| Error::Connect {
            path: path.to_owned(),
            errno: EINVAL,
        })?;
        let mut ident_box = Box::new(ident.to_raw());
        let mut funcs = Box::new(UsbdFuncs {
            nentries: USBDI_NFUNCS,
            insertion: Some(events::insertion_callback),
            removal: Some(events::removal_callback),
            event: None,
        });
        let mut parm = UsbdConnectParm {
            path: cpath.as_ptr(),
            vusb: USB_VERSION,
            vusbd: USBD_VERSION,
            flags: 0,
            argc: 0,
            argv: ptr::null_mut(),
            evtbufsz: 0,
            ident: ptr::from_mut(ident_box.as_mut()),
            funcs: ptr::from_mut(funcs.as_mut()),
            connect_wait: connect_wait_s,
            evtprio: 0,
            reserved: [0; 13],
        };
        let mut conn: *mut UsbdConnection = ptr::null_mut();
        let started = Instant::now();
        // SAFETY: parm and everything it points to (the C string, the
        // boxed ident and funcs) are live for the call and, boxed, for the
        // connection's whole life; conn receives the handle.
        let rc = unsafe { ffi::usbd_connect(&raw mut parm, &raw mut conn) };
        let ms = clock::elapsed_ms(started);
        if rc != EOK || conn.is_null() {
            let errno = if rc == EOK { EIO } else { rc };
            crate::log!(
                "usbdi: usbd_connect({path}) rc {rc} conn {conn:?} after {ms} ms (connect_wait {connect_wait_s} s)"
            );
            return Err(Error::Connect {
                path: path.to_owned(),
                errno,
            });
        }
        crate::vlog!("usbdi: usbd_connect({path}) ok in {ms} ms");
        Ok(HostSys {
            conn,
            _path: cpath,
            _ident: ident_box,
            _funcs: funcs,
        })
    }

    pub(crate) fn attach(&self, raw: &mut UsbdDeviceInstance) -> Result<DeviceSys> {
        let mut dev: *mut UsbdDevice = ptr::null_mut();
        let started = Instant::now();
        // SAFETY: conn is the live connection; raw is the caller's local
        // copy, which io-usb writes back; extra = 0 asks for no per-device
        // allocation; dev receives the handle.
        let rc = unsafe { ffi::usbd_attach(self.conn, ptr::from_mut(raw), 0, &raw mut dev) };
        if rc != EOK || dev.is_null() {
            crate::log!(
                "usbdi: usbd_attach(path {} devno {} if {}) rc {rc} after {} ms",
                raw.path,
                raw.devno,
                raw.iface,
                clock::elapsed_ms(started)
            );
            return Err(Error::Errno(if rc == EOK { EIO } else { rc }));
        }
        let mut node: *mut UsbdDescNode = ptr::null_mut();
        // SAFETY: dev is attached; node receives io-usb's cache node,
        // which the port never uses.
        let ddp = unsafe { ffi::usbd_device_descriptor(dev, &raw mut node) };
        if ddp.is_null() {
            // SAFETY: dev is the handle attached above and nothing else refers to it.
            let rc_detach = unsafe { ffi::usbd_detach(dev) };
            crate::log!(
                "usbdi: usbd_device_descriptor returned NULL after attach; usbd_detach rc {rc_detach}"
            );
            return Err(Error::BadDescriptor);
        }
        // SAFETY: ddp points into io-usb's descriptor cache, valid while
        // attached; an unaligned read copies it out without assuming the
        // cache's alignment.
        let mut dd = Box::new(unsafe { ptr::read_unaligned(ddp) });
        let mut ctrl: *mut UsbdPipe = ptr::null_mut();
        // SAFETY: the boxed descriptor has a stable address for the
        // device's life;
        // opening a pipe on a DEVICE descriptor yields the control pipe.
        let rc = unsafe {
            ffi::usbd_open_pipe(
                dev,
                ptr::from_mut(dd.as_mut()).cast::<c_void>(),
                &raw mut ctrl,
            )
        };
        if rc != EOK || ctrl.is_null() {
            // SAFETY: as above.
            let rc_detach = unsafe { ffi::usbd_detach(dev) };
            crate::log!("usbdi: usbd_open_pipe(control) rc {rc}; usbd_detach rc {rc_detach}");
            return Err(Error::Errno(if rc == EOK { EIO } else { rc }));
        }
        crate::vlog!(
            "usbdi: attached path {} devno {} if {} in {} ms",
            raw.path,
            raw.devno,
            raw.iface,
            clock::elapsed_ms(started)
        );
        Ok(DeviceSys { dev, ctrl, dd })
    }
}

impl Drop for HostSys {
    fn drop(&mut self) {
        let started = Instant::now();
        // SAFETY: conn is the live connection; every device attached on
        // it has been detached (each Device holds an Rc to the owner of
        // this struct, so this drop runs last).
        let rc = unsafe { ffi::usbd_disconnect(self.conn) };
        crate::log!(
            "usbdi: usbd_disconnect rc {rc} in {} ms",
            clock::elapsed_ms(started)
        );
    }
}

/// One `usbd_attach` with its control pipe.
pub(crate) struct DeviceSys {
    dev: *mut UsbdDevice,
    ctrl: *mut UsbdPipe,
    dd: Box<UsbdDeviceDescriptor>,
}

impl DeviceSys {
    pub(crate) fn device_descriptor(&self) -> UsbdDeviceDescriptor {
        *self.dd
    }

    pub(crate) fn select_config(&self, cfg: u8) -> Result<()> {
        // SAFETY: dev is attached.
        check(unsafe { ffi::usbd_select_config(self.dev, cfg) })
    }

    pub(crate) fn select_interface(&self, iface: u8, alt: u8) -> Result<()> {
        // SAFETY: dev is attached.
        check(unsafe { ffi::usbd_select_interface(self.dev, iface, alt) })
    }

    /// A synchronous `GET_DESCRIPTOR` through `usbd_descriptor`, staged
    /// through a zeroed `usbd_alloc` buffer of `buf.len()` bytes. The
    /// call reports no length and libusbdi copies back only the
    /// transferred bytes [RE: SDP `libusbdi.so.2` `usbd_descriptor`
    /// @0x983c-0x9850, `memcpy(desc, stage, min(len, transferred))`], so
    /// `buf` receives the descriptor followed by zeros, never a previous
    /// transfer's staging bytes; the descriptor's own length field
    /// bounds what the caller parses.
    pub(crate) fn descriptor(
        &self,
        dtype: u8,
        rtype: u16,
        index: u8,
        langid: u16,
        buf: &mut [u8],
    ) -> Result<()> {
        let stage = DmaBuf::alloc_zeroed(buf.len().max(1))?;
        let started = Instant::now();
        // SAFETY: dev is attached; stage holds buf.len() bytes; the call
        // is synchronous, so the staging buffer outlives the transfer.
        let rc = unsafe {
            ffi::usbd_descriptor(
                self.dev,
                0,
                dtype,
                rtype,
                index,
                langid,
                stage.ptr,
                buf.len(),
            )
        };
        let ms = clock::elapsed_ms(started);
        if rc != EOK {
            crate::log!(
                "usbdi: usbd_descriptor(type {dtype:#04x} index {index} langid {langid:#06x} len {}) rc {rc} after {ms} ms",
                buf.len()
            );
            return Err(Error::Errno(rc));
        }
        crate::vlog!(
            "usbdi: usbd_descriptor(type {dtype:#04x} index {index} langid {langid:#06x} len {}) ok in {ms} ms",
            buf.len()
        );
        stage.copy_to(buf);
        Ok(())
    }

    /// A synchronous control transfer (contract item 1): NULL callback,
    /// `usbd_alloc(max(len, 1))` staging, `URB_SHORT_XFER_OK` for IN.
    pub(crate) fn control(
        &self,
        setup: ControlSetup,
        data: &mut [u8],
        timeout_ms: u32,
    ) -> Result<ControlDone> {
        let len = data.len();
        let len32 = u32::try_from(len).map_err(|_| Error::Errno(EINVAL))?;
        let buf = DmaBuf::alloc(len.max(1))?;
        let urb = Urb::alloc()?;
        if !setup.dir_in {
            buf.copy_from(data);
        }
        let flags = if setup.dir_in {
            URB_DIR_IN | URB_SHORT_XFER_OK
        } else {
            URB_DIR_OUT
        };
        // SAFETY: urb and buf are live libusbdi allocations; buf holds at
        // least len bytes (and one byte when len is 0, so io-usb never
        // sees a dangling pointer).
        let rc = unsafe {
            ffi::usbd_setup_vendor(
                urb.ptr,
                flags,
                u16::from(setup.request),
                u16::from(setup.rtype),
                setup.value,
                setup.index,
                buf.ptr.cast::<c_void>(),
                len32,
            )
        };
        check(rc)?;
        let started = Instant::now();
        // SAFETY: control transfers complete inside usbd_io on this
        // io-usb (a callback on EP0 never fires), so the URB and buffer
        // are free to release once it returns.
        let rc = unsafe { ffi::usbd_io(urb.ptr, self.ctrl, None, ptr::null_mut(), timeout_ms) };
        let elapsed_ms = clock::elapsed_ms(started);
        if rc != EOK {
            crate::log!(
                "usbdi: usbd_io(control {} req {:#04x} len {len}) rc {rc} after {elapsed_ms} ms (timeout {timeout_ms} ms)",
                if setup.dir_in { "IN" } else { "OUT" },
                setup.request
            );
            return Err(Error::Errno(rc));
        }
        let (status, alen) = urb_status(urb.ptr);
        if setup.dir_in {
            let got = usize::try_from(alen).unwrap_or(usize::MAX).min(len);
            buf.copy_to(&mut data[..got]);
        }
        Ok(ControlDone {
            status,
            alen,
            elapsed_ms,
        })
    }

    /// Open a non-control pipe on an owned endpoint descriptor.
    pub(crate) fn open_pipe(&self, ep: Endpoint) -> Result<PipeSys> {
        let mut desc = Box::new(ep.raw());
        let mut pipe: *mut UsbdPipe = ptr::null_mut();
        // SAFETY: dev is attached; the boxed descriptor's address stays
        // valid for the pipe's life (PipeSys keeps the box).
        let rc = unsafe {
            ffi::usbd_open_pipe(
                self.dev,
                ptr::from_mut(desc.as_mut()).cast::<c_void>(),
                &raw mut pipe,
            )
        };
        if rc != EOK || pipe.is_null() {
            crate::log!(
                "usbdi: usbd_open_pipe(ep {:#04x} attr {:#04x} mps {}) rc {rc}",
                ep.address(),
                ep.attributes(),
                ep.max_packet()
            );
            return Err(Error::Errno(if rc == EOK { EIO } else { rc }));
        }
        crate::vlog!("usbdi: usbd_open_pipe(ep {:#04x}) ok", ep.address());
        Ok(PipeSys { pipe, _ep: desc })
    }
}

impl Drop for DeviceSys {
    fn drop(&mut self) {
        // SAFETY: ctrl and dev are the live handles this struct owns; any
        // bulk pipe on dev belongs to a BulkOut that holds an Rc to the
        // owner of this struct, so it was closed (or deliberately left
        // open after a drain timeout, for usbd_detach to close) before
        // this runs. The control pipe needs no EBUSY retry: a control
        // URB decrements the per-pipe counter synchronously inside
        // usbd_io [RE: SDP libusbdi.so.2 usbdi_sendcmd @0x5cc8-0x5cdc].
        let rc_pipe = unsafe { ffi::usbd_close_pipe(self.ctrl) };
        // usbd_detach returns EBUSY and does nothing while libusbdi's
        // per-device in-flight count is non-zero [RE: @0x5e78-0x5e84];
        // that count is decremented by the event thread only after the
        // last completion callback returned, a few instructions after
        // the per-pipe count a successful bulk close_pipe proved zero
        // [RE: @0x8018-0x8030], so a detach issued right after it can
        // still see the old value. Retried, bounded (contract item 9).
        let rc_detach =
            match clock::retry_while_busy("usbd_detach", clock::SETTLE_TIMEOUT_MS, || {
                // SAFETY: dev is the live handle this struct owns, and an
                // EBUSY return leaves it attached and untouched.
                check(unsafe { ffi::usbd_detach(self.dev) })
            }) {
                Ok(()) => EOK,
                Err(Error::Errno(rc)) => rc,
                Err(_) => EIO,
            };
        if rc_pipe == EOK && rc_detach == EOK {
            crate::vlog!("usbdi: close_pipe(control) rc {rc_pipe}, usbd_detach rc {rc_detach}");
        } else {
            crate::log!(
                "usbdi: close_pipe(control) rc {rc_pipe}, usbd_detach rc {rc_detach}{}",
                if rc_detach == EBUSY {
                    " (EBUSY: URB(s) still in flight; io-usb never received the detach, the device record is leaked and usbd_disconnect will stop at it)"
                } else {
                    ""
                }
            );
        }
    }
}

/// One non-control pipe. No `Drop`: closing is a decision of the owner
/// (skipped after a drain timeout and left to `usbd_detach`, which
/// closes leftover pipes itself, retrying every 100 ms [RE: SDP
/// `libusbdi.so.2` `usbd_detach` @0x5f04-0x5f38]; contract item 9).
pub(crate) struct PipeSys {
    pipe: *mut UsbdPipe,
    _ep: Box<UsbdEndpointDescriptor>,
}

impl PipeSys {
    pub(crate) fn reset(&self) -> Result<()> {
        // SAFETY: pipe is open.
        check(unsafe { ffi::usbd_reset_pipe(self.pipe) })
    }

    pub(crate) fn abort(&self) -> Result<()> {
        // SAFETY: pipe is open.
        check(unsafe { ffi::usbd_abort_pipe(self.pipe) })
    }

    /// `usbd_close_pipe`. `EBUSY` means libusbdi's per-pipe in-flight
    /// count is still non-zero and nothing was done, so the owner may
    /// call again; `EOK` means the pipe is unlinked and freed.
    ///
    /// # Errors
    /// [`Error::Errno`] with `EBUSY` while a completion callback has not
    /// yet returned; any other errno libusbdi reports.
    pub(crate) fn close(&self) -> Result<()> {
        // SAFETY: pipe is open (every call before the one that returned
        // EOK left it untouched [RE: SDP libusbdi.so.2 @0x5800-0x580c]),
        // and the owner never calls again after EOK.
        check(unsafe { ffi::usbd_close_pipe(self.pipe) })
    }

    /// Queue `len` bytes of the slot's buffer on this pipe with a
    /// completion callback. `handle` reaches the callback verbatim.
    pub(crate) fn submit_bulk_out(
        &self,
        slot: &SlotSys,
        len: u32,
        cb: UsbdIoCallback,
        handle: *mut c_void,
        timeout_ms: u32,
    ) -> Result<()> {
        // SAFETY: the slot's URB and buffer are live; the caller checked
        // len against the buffer size.
        let rc = unsafe {
            ffi::usbd_setup_bulk(
                slot.urb.ptr,
                URB_DIR_OUT,
                slot.buf.ptr.cast::<c_void>(),
                len,
            )
        };
        check(rc)?;
        // SAFETY: the URB and buffer stay allocated until the ring is
        // dropped, which happens only on the caller's thread after
        // usbd_close_pipe returned EOK, i.e. after every completion
        // callback has returned and released its reference (contract
        // item 9), so io-usb never touches freed memory.
        check(unsafe { ffi::usbd_io(slot.urb.ptr, self.pipe, cb, handle, timeout_ms) })
    }

    /// Arm the slot's buffer for `len` bytes from this interrupt IN pipe
    /// (`URB_SHORT_XFER_OK`: a report shorter than the buffer completes
    /// `CMP` with its own length). `handle`
    /// reaches the callback verbatim.
    pub(crate) fn submit_intr_in(
        &self,
        slot: &SlotSys,
        len: u32,
        cb: UsbdIoCallback,
        handle: *mut c_void,
        timeout_ms: u32,
    ) -> Result<()> {
        // SAFETY: the slot's URB and buffer are live; the caller checked
        // len against the buffer size.
        let rc = unsafe {
            ffi::usbd_setup_interrupt(
                slot.urb.ptr,
                URB_DIR_IN | URB_SHORT_XFER_OK,
                slot.buf.ptr.cast::<c_void>(),
                len,
            )
        };
        check(rc)?;
        // SAFETY: as submit_bulk_out: the URB and buffer outlive every
        // completion (contract item 9).
        check(unsafe { ffi::usbd_io(slot.urb.ptr, self.pipe, cb, handle, timeout_ms) })
    }

    /// Send `len` bytes of the slot's buffer on this interrupt OUT pipe
    /// and wait for the transfer. `usbd_io` with a NULL callback does NOT
    /// wait for an interrupt OUT on this io-usb: it returns `EOK` while the
    /// URB is still `INPROG` and the transfer completes in the background,
    /// as bulk does (contract item 2). This polls the URB status until
    /// it leaves `INPROG` or `timeout_ms` passes, and returns the status
    /// word and the transferred length; `INPROG` comes back on a timeout,
    /// and the owner then aborts the pipe before the slot is reused.
    pub(crate) fn send_intr_out(
        &self,
        slot: &SlotSys,
        len: u32,
        timeout_ms: u32,
    ) -> Result<(u32, u32)> {
        // SAFETY: the slot's URB and buffer are live; the caller checked
        // len against the buffer size.
        let rc = unsafe {
            ffi::usbd_setup_interrupt(
                slot.urb.ptr,
                URB_DIR_OUT,
                slot.buf.ptr.cast::<c_void>(),
                len,
            )
        };
        check(rc)?;
        // SAFETY: the URB and buffer stay allocated until the slot is
        // dropped, which happens only after the pipe is closed; the wait
        // below (or the owner's abort after a timeout) ends the transfer
        // before the slot is filled again.
        check(unsafe { ffi::usbd_io(slot.urb.ptr, self.pipe, None, ptr::null_mut(), timeout_ms) })?;
        let started = Instant::now();
        loop {
            let (status, alen) = urb_status(slot.urb.ptr);
            if status & ffi::USBD_URB_STATUS_MASK != ffi::USBD_STATUS_INPROG
                || clock::elapsed_ms(started) >= u64::from(timeout_ms)
            {
                return Ok((status, alen));
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Arm the slot's buffer for up to `len` bytes from this bulk IN pipe
    /// (`URB_SHORT_XFER_OK`: a transfer shorter than the buffer completes
    /// `CMP` with its own length). `handle` reaches the callback verbatim.
    pub(crate) fn submit_bulk_in(
        &self,
        slot: &SlotSys,
        len: u32,
        cb: UsbdIoCallback,
        handle: *mut c_void,
        timeout_ms: u32,
    ) -> Result<()> {
        // SAFETY: the slot's URB and buffer are live; the caller checked
        // len against the buffer size.
        let rc = unsafe {
            ffi::usbd_setup_bulk(
                slot.urb.ptr,
                URB_DIR_IN | URB_SHORT_XFER_OK,
                slot.buf.ptr.cast::<c_void>(),
                len,
            )
        };
        check(rc)?;
        // SAFETY: as submit_bulk_out: the URB and buffer outlive every
        // completion (contract item 9).
        check(unsafe { ffi::usbd_io(slot.urb.ptr, self.pipe, cb, handle, timeout_ms) })
    }
}

/// A `usbd_alloc` buffer.
struct DmaBuf {
    ptr: *mut u8,
    len: usize,
}

impl DmaBuf {
    fn alloc(len: usize) -> Result<DmaBuf> {
        // SAFETY: usbd_alloc is a plain allocator call.
        let ptr = unsafe { ffi::usbd_alloc(len) }.cast::<u8>();
        if ptr.is_null() {
            crate::log!("usbdi: usbd_alloc({len}) returned NULL");
            return Err(Error::Errno(ENOMEM));
        }
        Ok(DmaBuf { ptr, len })
    }

    /// `alloc`, then the block cleared. `usbd_alloc` recycles pool
    /// elements without clearing them [RE: SDP `libusbdi.so.2`
    /// `usbdi_memchunk_malloc` @0x69e8-0x69ec pops the free list through
    /// the helper @0x60b4-0x60e0, which writes only the list links; only
    /// a NEW chunk is `memset` @0x6a3c], so a block whose whole length is
    /// copied back to a caller (`DeviceSys::descriptor`) is cleared here
    /// to reproduce the caller's zero-fill. The control path and the
    /// bulk slots need no clear: they copy back only the transferred
    /// length, or are written before every submit.
    fn alloc_zeroed(len: usize) -> Result<DmaBuf> {
        let buf = DmaBuf::alloc(len)?;
        // SAFETY: ptr is a live usbd_alloc block of len bytes that only
        // this side holds until it is handed to libusbdi.
        unsafe { ptr::write_bytes(buf.ptr, 0, buf.len) };
        Ok(buf)
    }

    fn copy_from(&self, src: &[u8]) {
        let n = src.len().min(self.len);
        // SAFETY: both ranges are n bytes inside live, distinct allocations.
        unsafe { ptr::copy_nonoverlapping(src.as_ptr(), self.ptr, n) };
    }

    fn copy_to(&self, dst: &mut [u8]) {
        let n = dst.len().min(self.len);
        // SAFETY: both ranges are n bytes inside live, distinct allocations.
        unsafe { ptr::copy_nonoverlapping(self.ptr, dst.as_mut_ptr(), n) };
    }
}

impl Drop for DmaBuf {
    fn drop(&mut self) {
        // SAFETY: ptr came from usbd_alloc and is freed exactly once.
        unsafe { ffi::usbd_free(self.ptr.cast::<c_void>()) };
    }
}

/// A `usbd_alloc_urb` URB.
struct Urb {
    ptr: *mut UsbdUrb,
}

impl Urb {
    fn alloc() -> Result<Urb> {
        // SAFETY: usbd_alloc_urb with no link is a plain allocator call.
        let ptr = unsafe { ffi::usbd_alloc_urb(ptr::null_mut()) };
        if ptr.is_null() {
            crate::log!("usbdi: usbd_alloc_urb returned NULL");
            return Err(Error::Errno(ENOMEM));
        }
        Ok(Urb { ptr })
    }
}

impl Drop for Urb {
    fn drop(&mut self) {
        // SAFETY: ptr came from usbd_alloc_urb, is not in flight (the
        // owner drains and closes the pipe before dropping, and never
        // drops while the connection may be gone: usbd_free_urb frees
        // into a pool usbd_disconnect destroys), and is freed exactly
        // once.
        let _ = unsafe { ffi::usbd_free_urb(self.ptr) };
    }
}

/// One ring slot: a URB and the `usbd_alloc` buffer it sends from.
pub(crate) struct SlotSys {
    urb: Urb,
    buf: DmaBuf,
}

impl SlotSys {
    pub(crate) fn alloc(bytes: usize) -> Result<SlotSys> {
        let buf = DmaBuf::alloc(bytes)?;
        let urb = Urb::alloc()?;
        Ok(SlotSys { urb, buf })
    }

    /// Copy `data` (at most the buffer size, checked by the caller) into
    /// the DMA buffer.
    pub(crate) fn fill(&self, data: &[u8]) {
        self.buf.copy_from(data);
    }

    /// Copy the DMA buffer (at most `dst.len()`, checked by the caller
    /// against the completed length) out to `dst`.
    pub(crate) fn read_to(&self, dst: &mut [u8]) {
        self.buf.copy_to(dst);
    }

    /// Whether a completion's URB pointer is this slot's.
    pub(crate) fn owns_urb(&self, urb: *mut UsbdUrb) -> bool {
        ptr::eq(self.urb.ptr, urb)
    }
}

/// `usbd_urb_status`: the raw status word and the transferred length.
///
/// The call's return value does not determine transfer success.
/// This function reads the two output values and checks the status word.
/// The library stores both
/// out-values first, unconditionally, and then returns an errno it
/// derives from the status word: `INPROG` -> `EBUSY` (16), `TIMEOUT` ->
/// `ETIMEDOUT` (260), `ABORTED` -> `EINTR` (4), `CMP_ERR` -> `ENODEV`
/// (19) when the low code is `DEV_NOANSWER` (5), else `EIO` (5); only
/// `CMP` and `DEVICE_RESET` return `EOK`. [RE: `usbd_urb_status` @0x8d30
/// with the mapper @0x8cec in the SDP `libusbdi.so.2`; the head unit's
/// copy has the same code @0x8f24 / @0x8ee0.] Gating on that return
/// would discard the status and length of every non-`CMP` completion
/// (every `ABORTED` one at teardown, every `STALL`, every io-usb
/// timeout) and make [`Error::Status`] unreachable on the target.
/// The diagnostic log includes errno.
pub(crate) fn urb_status(urb: *mut UsbdUrb) -> (u32, u32) {
    let mut status: u32 = 0;
    let mut alen: u32 = 0;
    // SAFETY: urb is a live URB; the out-pointers name locals.
    let rc = unsafe { ffi::usbd_urb_status(urb, &raw mut status, &raw mut alen) };
    if rc != EOK {
        crate::vlog!("usbdi: usbd_urb_status rc {rc} status {status:#010x} alen {alen}");
    }
    (status, alen)
}
