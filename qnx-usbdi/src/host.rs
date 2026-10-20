// SPDX-License-Identifier: Apache-2.0
//! `Host`: one `usbd_connect`, the insertion/removal poll, and `attach`.
//!
//! One `Host` per process. The funcs table io-usb calls back through
//! carries no user pointer, so the event queue is process-global
//! (`events`), and a second connection would have no way to tell its
//! events from the first's. A second `connect` while one is live fails
//! with `EBUSY` and the path in the text.
//!
//! Connect parameters (contract item 5): `vusb = USB_VERSION` (0x0110,
//! the USB spec version, NOT `USBD_VERSION`), `vusbd = USBD_VERSION`,
//! `connect_wait = 2` seconds (the five C clients pass 60; a wrong path
//! should fail fast and the path is in the error), `funcs.nentries = 3`,
//! `event = None`.
//!
//! Teardown order: a `Device` holds a reference to the connection, so
//! dropping the `Host` while devices live only logs and defers the
//! `usbd_disconnect` to the last device's drop. Two ways to get events:
//! `wait_events` sleeps a bounded time and drains (a driver with its own
//! loop tick), `next_events` blocks until the event thread queues one (a
//! caller that waits for a plug).
//!
//! Copyright 2026. Licensed under the Apache License, Version 2.0.

use core::fmt;
use core::marker::PhantomData;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::clock;
use crate::device::{Device, DeviceDescriptor, DeviceShared};
use crate::events::{self, Event, GoneFlag, Instance};
use crate::ffi::{UsbdDeviceIdent, EBUSY, USBD_CONNECT_WILDCARD};
use crate::sys;
use crate::{Error, Result};

/// io-usb's default resource manager path.
pub const DEFAULT_PATH: &str = "/dev/io-usb/io-usb";

/// Seconds `usbd_connect` waits for the manager to appear. Deliberately
/// 2 rather than the header's 60, so a wrong path fails fast.
pub const CONNECT_WAIT_S: u16 = 2;

/// `USBD_CONNECT_WILDCARD`: match any value in an [`Ident`] field.
pub const WILDCARD: u32 = USBD_CONNECT_WILDCARD;

/// The match io-usb applies to every interface of every device: fields
/// are matched exactly unless [`WILDCARD`]. io-usb fires one insertion
/// per matching INTERFACE, so a composite device produces several
/// [`Instance`]s that share `(path, devno)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ident {
    /// `idVendor`, or [`WILDCARD`].
    pub vendor: u32,
    /// `idProduct`, or [`WILDCARD`].
    pub device: u32,
    /// `bInterfaceClass`, or [`WILDCARD`].
    pub class: u32,
    /// `bInterfaceSubClass`, or [`WILDCARD`].
    pub subclass: u32,
    /// `bInterfaceProtocol`, or [`WILDCARD`].
    pub protocol: u32,
}

impl Ident {
    /// Match everything.
    pub const ANY: Ident = Ident {
        vendor: WILDCARD,
        device: WILDCARD,
        class: WILDCARD,
        subclass: WILDCARD,
        protocol: WILDCARD,
    };

    /// The C struct `usbd_connect` reads.
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

/// Take the one-per-process slot; false when a connection is live.
pub(crate) fn claim_host_slot() -> bool {
    HOST_LIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

/// Give the slot back (the connection is gone).
pub(crate) fn release_host_slot() {
    HOST_LIVE.store(false, Ordering::Release);
}

/// The connection, shared by the `Host` and every `Device` attached on
/// it so that `usbd_disconnect` runs after the last `usbd_detach`. An
/// `Rc`, not an `Arc`: nothing on libusbdi's event thread touches it
/// (the callbacks reach only the global queue), and `Host` and `Device`
/// are single-threaded.
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

/// One io-usb connection. `!Send`: the queue behind it is global and
/// the devices attached on it are single-threaded.
pub struct Host {
    shared: Rc<HostShared>,
    _not_send: PhantomData<*mut ()>,
}

impl Host {
    /// Connect to io-usb at `path` (default [`DEFAULT_PATH`]) and ask
    /// for insertion/removal callbacks for interfaces matching `ident`.
    ///
    /// # Errors
    /// [`Error::Connect`] with the path when `usbd_connect` fails, when
    /// the path holds a NUL byte, or (`EBUSY`) when this process already
    /// has a live `Host`; [`Error::Unsupported`] on a non-QNX build.
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

    /// The path this connection was opened on.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.shared.path
    }

    /// Drain the insertions and removals seen so far, oldest first.
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

    /// Sleep `settle_ms` (bounded; it does not wait for a plug, see
    /// [`Host::next_events`] for that), then [`Host::poll`]. The measured
    /// sleep is logged.
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

    /// Block until io-usb reports an insertion or a removal, or until
    /// `timeout` has passed (`None`: no bound), and return everything
    /// queued, oldest first. For a caller that waits for a plug: it sleeps
    /// in the kernel until the event thread queues something, with no
    /// periodic wake. Empty only when the timeout passed.
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

    /// Devices attached on this connection and not yet dropped.
    #[must_use]
    pub fn attached(&self) -> usize {
        Rc::strong_count(&self.shared) - 1
    }

    /// `usbd_attach` the instance (contract item 11): a local mutable
    /// copy is passed, io-usb writes it back, and the post-attach copy is
    /// what the `Device` keeps for the config and interface selection.
    /// VID and PID come from the device descriptor after attach. The
    /// removal flag is registered from the PRE-attach instance, before
    /// `usbd_attach`: libusbdi delivers a removal for an
    /// attached device with no URB in flight at once, so one that lands
    /// during the attach's three io-usb round trips would otherwise find
    /// no entry and the `Device` would start with `gone == false` on a
    /// removed device. The match is on `(path, devno, iface)`; whether
    /// io-usb's write-back leaves those three alone is unverified, so a
    /// difference is logged. On an attach error the flag drops with the
    /// local and its entry is pruned by the next registration.
    ///
    /// # Errors
    /// [`Error::Errno`] from `usbd_attach` or `usbd_open_pipe`;
    /// [`Error::BadDescriptor`] when io-usb has no device descriptor for
    /// it; [`Error::Unsupported`] on a non-QNX build.
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

    /// The slot is process-global; the tests that take it run one at a
    /// time.
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
        // Only meaningful where io-usb is absent; on the target this test
        // would try a real connect, which the gate never does.
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
