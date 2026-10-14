//! Copyright 2026. Licensed under the Apache License, Version 2.0.
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
use core::ffi::{c_int, c_void};
use core::ptr;
use std::ffi::CString;
use std::time::Instant;
fn check(rc: c_int) -> Result<()> {
    if rc == EOK {
        Ok(())
    } else {
        Err(Error::Errno(rc))
    }
}
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
        // SAFETY: `parm`, the C string, and the boxed ident and funcs stay live until the call returns.
        // The boxed values stay live for the full connection lifetime.
        // `conn` receives the handle.
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
        // SAFETY: `conn` is the live connection.
        // `raw` is a local copy that io-usb updates.
        // The zero `extra` value selects no per-device allocation.
        // `dev` receives the handle.
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
        // SAFETY: dev is attached. node receives io-usb's cache node,
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
        // attached. An unaligned read copies it out without assuming the
        // cache's alignment.
        let mut dd = Box::new(unsafe { ptr::read_unaligned(ddp) });
        let mut ctrl: *mut UsbdPipe = ptr::null_mut();
        // SAFETY: The boxed descriptor has a stable address for the device lifetime.
        // Opening a pipe on a DEVICE descriptor gives the control pipe.
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
        // SAFETY: `conn` is the live connection.
        // Each attached device holds an `Rc` for the owner of this struct.
        // Each device detaches before the owner drops the connection.
        let rc = unsafe { ffi::usbd_disconnect(self.conn) };
        crate::log!(
            "usbdi: usbd_disconnect rc {rc} in {} ms",
            clock::elapsed_ms(started)
        );
    }
}
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
        // SAFETY: dev is attached. stage holds buf.len() bytes. The call
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
        // SAFETY: `urb` and `buf` are live libusbdi allocations.
        // The buffer holds at least `len` bytes.
        // For zero `len`, the buffer has one byte to keep the pointer valid.
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
        // SAFETY: Control transfers complete inside `usbd_io` on this io-usb implementation.
        // An EP0 callback does not run.
        // The caller can release the URB and buffer when `usbd_io` returns.
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
    pub(crate) fn open_pipe(&self, ep: Endpoint) -> Result<PipeSys> {
        let mut desc = Box::new(ep.raw());
        let mut pipe: *mut UsbdPipe = ptr::null_mut();
        // SAFETY: dev is attached. The boxed descriptor's address stays
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
        // SAFETY: `ctrl` and `dev` are the live handles that this struct owns.
        // Each bulk pipe belongs to a `BulkOut` that holds an `Rc` for the owner.
        // Before this operation, each bulk pipe closes or stays open after a drain timeout for `usbd_detach` to close.
        // The control URB decreases the per-pipe counter inside `usbd_io`.
        // The control pipe does not require an `EBUSY` retry.
        let rc_pipe = unsafe { ffi::usbd_close_pipe(self.ctrl) };
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
    /// # Errors
    /// [`Error::Errno`] with `EBUSY` until a completion callback returns.
    /// Other errno values from libusbdi.
    pub(crate) fn close(&self) -> Result<()> {
        // SAFETY: The pipe is open.
        // A close call that did not return `EOK` leaves the pipe open.
        // The owner does not call again after `EOK`.
        check(unsafe { ffi::usbd_close_pipe(self.pipe) })
    }
    pub(crate) fn submit_bulk_out(
        &self,
        slot: &SlotSys,
        len: u32,
        cb: UsbdIoCallback,
        handle: *mut c_void,
        timeout_ms: u32,
    ) -> Result<()> {
        // SAFETY: the slot's URB and buffer are live. The caller checked
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
    pub(crate) fn submit_intr_in(
        &self,
        slot: &SlotSys,
        len: u32,
        cb: UsbdIoCallback,
        handle: *mut c_void,
        timeout_ms: u32,
    ) -> Result<()> {
        // SAFETY: the slot's URB and buffer are live. The caller checked
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
    pub(crate) fn send_intr_out(
        &self,
        slot: &SlotSys,
        len: u32,
        timeout_ms: u32,
    ) -> Result<(u32, u32)> {
        // SAFETY: the slot's URB and buffer are live. The caller checked
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
        // SAFETY: The slot keeps the URB and buffer allocated until the pipe closes.
        // The wait, or the owner abort after a timeout, ends the transfer before the next buffer fill.
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
    pub(crate) fn submit_bulk_in(
        &self,
        slot: &SlotSys,
        len: u32,
        cb: UsbdIoCallback,
        handle: *mut c_void,
        timeout_ms: u32,
    ) -> Result<()> {
        // SAFETY: the slot's URB and buffer are live. The caller checked
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
    fn alloc_zeroed(len: usize) -> Result<DmaBuf> {
        let buf = DmaBuf::alloc(len)?;
        // SAFETY: ptr is a live usbd_alloc block of len bytes that only
        // this side holds until it is handed to libusbdi.
        unsafe { ptr::write_bytes(buf.ptr, 0, buf.len) };
        Ok(buf)
    }
    fn copy_from(&self, src: &[u8]) {
        let n = src.len().min(self.len);
        // SAFETY: The two ranges each contain n bytes inside separate live allocations.
        unsafe { ptr::copy_nonoverlapping(src.as_ptr(), self.ptr, n) };
    }
    fn copy_to(&self, dst: &mut [u8]) {
        let n = dst.len().min(self.len);
        // SAFETY: The two ranges each contain n bytes inside separate live allocations.
        unsafe { ptr::copy_nonoverlapping(self.ptr, dst.as_mut_ptr(), n) };
    }
}
impl Drop for DmaBuf {
    fn drop(&mut self) {
        // SAFETY: ptr came from usbd_alloc and is freed exactly once.
        unsafe { ffi::usbd_free(self.ptr.cast::<c_void>()) };
    }
}
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
        // SAFETY: `ptr` comes from `usbd_alloc_urb` and is not in use by a transfer.
        // The owner drains and closes the pipe before `Drop`.
        // The owner does not drop while the connection can be gone.
        // `usbd_free_urb` uses a pool that `usbd_disconnect` destroys.
        // The owner frees the URB once.
        let _ = unsafe { ffi::usbd_free_urb(self.ptr) };
    }
}
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
    pub(crate) fn fill(&self, data: &[u8]) {
        self.buf.copy_from(data);
    }
    pub(crate) fn read_to(&self, dst: &mut [u8]) {
        self.buf.copy_to(dst);
    }
    pub(crate) fn owns_urb(&self, urb: *mut UsbdUrb) -> bool {
        ptr::eq(self.urb.ptr, urb)
    }
}
pub(crate) fn urb_status(urb: *mut UsbdUrb) -> (u32, u32) {
    let mut status: u32 = 0;
    let mut alen: u32 = 0;
    // SAFETY: urb is a live URB. The out-pointers name locals.
    let rc = unsafe { ffi::usbd_urb_status(urb, &raw mut status, &raw mut alen) };
    if rc != EOK {
        crate::vlog!("usbdi: usbd_urb_status rc {rc} status {status:#010x} alen {alen}");
    }
    (status, alen)
}
