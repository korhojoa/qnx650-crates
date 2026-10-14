//! Copyright 2026. Licensed under the Apache License, Version 2.0.
use crate::clock;
use crate::device::{Device, DeviceDescriptor, DeviceShared};
use crate::events::{self, Event, GoneFlag, Instance};
use crate::ffi::{UsbdDeviceIdent, EBUSY, USBD_CONNECT_WILDCARD};
use crate::sys;
use crate::{Error, Result};
use core::fmt;
use core::marker::PhantomData;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
pub const DEFAULT_PATH: &str = "/dev/io-usb/io-usb";
pub const CONNECT_WAIT_S: u16 = 2;
pub const WILDCARD: u32 = USBD_CONNECT_WILDCARD;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ident {
    pub vendor: u32,
    pub device: u32,
    pub class: u32,
    pub subclass: u32,
    pub protocol: u32,
}
impl Ident {
    pub const ANY: Ident = Ident {
        vendor: WILDCARD,
        device: WILDCARD,
        class: WILDCARD,
        subclass: WILDCARD,
        protocol: WILDCARD,
    };
    #[must_use]
    pub const fn to_raw(self) -> UsbdDeviceIdent {
        UsbdDeviceIdent {
            vendor: self.vendor,
            device: self.device,
            dclass: self.class,
            subclass: self.subclass,
            protocol: self.protocol,
        }
    }
}
impl Default for Ident {
    fn default() -> Self {
        Ident::ANY
    }
}
fn fmt_field(f: &mut fmt::Formatter<'_>, v: u32, width: usize) -> fmt::Result {
    if v == WILDCARD {
        f.write_str("*")
    } else {
        write!(f, "{v:0width$x}")
    }
}
impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("vendor ")?;
        fmt_field(f, self.vendor, 4)?;
        f.write_str(" device ")?;
        fmt_field(f, self.device, 4)?;
        f.write_str(" class ")?;
        fmt_field(f, self.class, 2)?;
        f.write_str("/")?;
        fmt_field(f, self.subclass, 2)?;
        f.write_str("/")?;
        fmt_field(f, self.protocol, 2)
    }
}
static HOST_LIVE: AtomicBool = AtomicBool::new(false);
pub(crate) fn claim_host_slot() -> bool {
    HOST_LIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}
pub(crate) fn release_host_slot() {
    HOST_LIVE.store(false, Ordering::Release);
}
pub(crate) struct HostShared {
    pub(crate) path: String,
    pub(crate) sys: sys::HostSys,
}
impl Drop for HostShared {
    fn drop(&mut self) {
        crate::log!("usbdi: disconnecting from {}", self.path);
        release_host_slot();
    }
}
pub struct Host {
    shared: Rc<HostShared>,
    _not_send: PhantomData<*mut ()>,
}
impl Host {
    /// # Errors
    /// [`Error::Connect`] with the path when `usbd_connect` fails or the path contains a NUL byte.
    /// [`Error::Connect`] (`EBUSY`) when this process has a live `Host`.
    /// [`Error::Unsupported`] on a non-QNX target.
    pub fn connect(path: Option<&str>, ident: Ident) -> Result<Host> {
        let path = path.unwrap_or(DEFAULT_PATH);
        if !claim_host_slot() {
            crate::log!("usbdi: connect({path}) refused: this process already holds a Host");
            return Err(Error::Connect {
                path: path.to_owned(),
                errno: EBUSY,
            });
        }
        match sys::HostSys::connect(path, ident, CONNECT_WAIT_S) {
            Ok(sys) => {
                crate::log!("usbdi: connected to {path}, matching {ident}");
                Ok(Host {
                    shared: Rc::new(HostShared {
                        path: path.to_owned(),
                        sys,
                    }),
                    _not_send: PhantomData,
                })
            }
            Err(e) => {
                release_host_slot();
                crate::log!("usbdi: connect failed: {e}");
                Err(e)
            }
        }
    }
    #[must_use]
    pub fn path(&self) -> &str {
        &self.shared.path
    }
    #[must_use]
    pub fn poll(&self) -> Vec<Event> {
        let got = events::drain();
        if !got.is_empty() {
            crate::vlog!(
                "usbdi: poll on {}: {} event(s)",
                self.shared.path,
                got.len()
            );
        }
        got
    }
    #[must_use]
    pub fn wait_events(&self, settle_ms: u32) -> Vec<Event> {
        let started = Instant::now();
        std::thread::sleep(Duration::from_millis(u64::from(settle_ms)));
        let got = self.poll();
        crate::vlog!(
            "usbdi: settled {} ms (asked {settle_ms}), {} event(s)",
            clock::elapsed_ms(started),
            got.len()
        );
        got
    }
    #[must_use]
    pub fn next_events(&self, timeout: Option<Duration>) -> Vec<Event> {
        let started = Instant::now();
        let got = events::wait(timeout);
        crate::vlog!(
            "usbdi: waited {} ms on {} (bound {timeout:?}), {} event(s)",
            clock::elapsed_ms(started),
            self.shared.path,
            got.len()
        );
        got
    }
    #[must_use]
    pub fn attached(&self) -> usize {
        Rc::strong_count(&self.shared) - 1
    }
    /// # Errors
    /// [`Error::Errno`] from `usbd_attach` or `usbd_open_pipe`.
    /// [`Error::BadDescriptor`] when io-usb has no device descriptor for
    /// it. [`Error::Unsupported`] on a non-QNX build.
    pub fn attach(&self, inst: &Instance) -> Result<Device> {
        let mut raw = inst.raw();
        let started = Instant::now();
        let gone = GoneFlag::register(inst);
        let sys = self.shared.sys.attach(&mut raw)?;
        let post = Instance::from_raw(raw);
        if !post.matches_removal(inst) {
            crate::log!("usbdi: attach rewrote (path, devno, iface): {inst} -> {post}");
        }
        let desc = DeviceDescriptor::from_raw(&sys.device_descriptor());
        crate::log!(
            "usbdi: attached {post}: {:04x}:{:04x} bcdDevice {:04x} in {} ms",
            desc.vendor,
            desc.product,
            desc.bcd_device,
            clock::elapsed_ms(started)
        );
        Ok(Device::from_shared(Rc::new(DeviceShared {
            inst: post,
            desc,
            gone,
            config: RefCell::new(None),
            sys,
            _host: Rc::clone(&self.shared),
        })))
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let live = self.attached();
        if live > 0 {
            crate::log!(
                "usbdi: Host dropped with {live} device(s) attached; usbd_disconnect deferred to the last detach"
            );
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, PoisonError};
    static SERIAL: Mutex<()> = Mutex::new(());
    #[test]
    fn ident_default_is_all_wildcards_and_displays_them() {
        let any = Ident::default();
        assert_eq!(any, Ident::ANY);
        assert_eq!(any.to_raw().vendor, WILDCARD);
        assert_eq!(any.to_string(), "vendor * device * class */*/*");
        let dl = Ident {
            vendor: 0x17E9,
            device: WILDCARD,
            class: 0xFF,
            subclass: 0,
            protocol: 0,
        };
        assert_eq!(dl.to_string(), "vendor 17e9 device * class ff/00/00");
        let raw = dl.to_raw();
        assert_eq!(
            (raw.vendor, raw.device, raw.dclass),
            (0x17E9, WILDCARD, 0xFF)
        );
    }
    #[test]
    fn the_host_slot_is_exclusive_and_released() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(claim_host_slot());
        assert!(!claim_host_slot());
        release_host_slot();
        assert!(claim_host_slot());
        release_host_slot();
    }
    #[test]
    fn connect_off_target_is_unsupported_and_frees_the_slot() {
        if cfg!(target_os = "nto") {
            return;
        }
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let Err(err) = Host::connect(Some("/dev/none"), Ident::ANY) else {
            panic!("connect succeeded without io-usb");
        };
        assert_eq!(err, Error::Unsupported);
        assert!(claim_host_slot());
        release_host_slot();
    }
}
