// SPDX-License-Identifier: Apache-2.0
//! `qnx-usbdi`: the QNX 6.5 io-usb client API (`libusbdi.so.2`) in Rust.
//!
//! A safe layer over the calls a user-space USB client needs: connect to
//! io-usb, learn of insertions and removals, attach a device, read its
//! descriptors, issue control transfers, stream bulk OUT through a ring
//! of queued URBs, read interrupt IN reports from a ring of armed ones,
//! and send interrupt OUT reports one synchronous transfer at a time.
//! The contract this crate enforces, item by item, is in `DESIGN.md` next
//! to this crate's `Cargo.toml`; the module docs say which item each
//! mechanism answers.
//!
//! | module | holds |
//! |---|---|
//! | [`clock`] | the monotonic origin every stamp and deadline reads; the log |
//! | [`ffi`] | the `usbdi.h` / `usb100.h` mirror, sizes and offsets pinned |
//! | [`desc`] | the byte-wise configuration walk, strings, the config rule |
//! | [`events`] | `Instance`, the insertion/removal queue, the removal flag |
//! | [`host`] | `Host`: `usbd_connect`, poll, attach |
//! | [`device`] | `Device`: control transfers, descriptors, selection |
//! | [`bulk`] | `BulkOut`: the queued-URB ring and its teardown |
//! | [`bulk_in`] | `BulkIn`: the armed-URB ring a bulk IN endpoint's transfers come out of; `BulkInWaker`, which ends its `read` wait from another thread |
//! | [`intr`] | `IntrIn`: the armed-URB ring an interrupt endpoint's reports come out of |
//! | [`intr_out`] | `IntrOut`: one synchronous transfer at a time on an interrupt OUT endpoint |
//! | [`stop`] | the process-wide stop flag the ring's waits consult, and the signal handlers that set it |
//! | [`activity`] | one signal for every interrupt IN completion and hot-plug event, for a driver that serves several pipes on one thread |
//! | `cancel` | the guard every io-usb callback runs under (cancellation deferred) |
//!
//! On a non-QNX target the crate compiles and its pure parts run and are
//! tested; `Host::connect` returns [`Error::Unsupported`] and no
//! `Device` or `BulkOut` can exist. One `Host` per process: the
//! callbacks io-usb invokes carry no user pointer, so the event queue is
//! a process-global.
//!
//! Copyright 2026. Licensed under the Apache License, Version 2.0.

use core::fmt;

pub mod activity;
pub mod bulk;
pub mod bulk_in;
mod cancel;
pub mod clock;
pub mod desc;
pub mod device;
pub mod events;
pub mod ffi;
pub mod host;
pub mod intr;
pub mod intr_out;
pub mod stop;
mod sys;

pub use bulk::{BulkOut, BulkStats};
pub use bulk_in::{BulkIn, BulkInStats, BulkInWaker};
pub use desc::{Endpoint, Interface};
pub use device::{Device, DeviceDescriptor};
pub use events::{Event, GoneFlag, Instance};
pub use host::{Host, Ident, DEFAULT_PATH, WILDCARD};
pub use intr::{IntrIn, IntrStats};
pub use intr_out::{IntrOut, IntrOutStats};

/// Everything this crate can fail with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A `libusbdi` call returned this errno.
    Errno(i32),
    /// A transfer completed with this URB status word, which is not
    /// `USBD_STATUS_CMP`.
    Status(u32),
    /// A transfer completed `CMP` but moved fewer bytes than asked.
    Short {
        /// Bytes requested.
        want: usize,
        /// Bytes io-usb reports.
        got: usize,
    },
    /// The removal callback has fired for this device.
    Gone,
    /// A bounded wait (for a ring slot, a drain, a flush) expired, or a
    /// [`BulkIn::read`] wait was ended by its [`BulkInWaker`] before
    /// anything completed.
    Timeout,
    /// A stop was requested ([`stop::request`]) while waiting on the
    /// ring.
    Interrupted,
    /// The interface has no endpoint of the kind asked for, or the
    /// endpoint given is not of that kind.
    NoEndpoint,
    /// A descriptor block is malformed or names a different device than
    /// the instance io-usb delivered.
    BadDescriptor,
    /// io-usb is not available on this target.
    Unsupported,
    /// `usbd_connect` failed on `path` with `errno` (`EBUSY` when this
    /// process already holds a `Host`).
    Connect {
        /// The io-usb path that was tried.
        path: String,
        /// The errno io-usb returned.
        errno: i32,
    },
}

/// `Result` with this crate's [`Error`].
pub type Result<T> = core::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Errno(e) => write!(f, "{}", std::io::Error::from_raw_os_error(*e)),
            Error::Status(s) => {
                let usb = ffi::usb_status_name(*s);
                if usb.is_empty() {
                    write!(f, "URB status {s:#010x} ({})", ffi::urb_status_name(*s))
                } else {
                    write!(
                        f,
                        "URB status {s:#010x} ({}/{usb})",
                        ffi::urb_status_name(*s)
                    )
                }
            }
            Error::Short { want, got } => write!(f, "short transfer: {got} of {want} bytes"),
            Error::Gone => f.write_str("device removed"),
            Error::Timeout => f.write_str("timed out"),
            Error::Interrupted => f.write_str("interrupted by a stop request"),
            Error::NoEndpoint => f.write_str("no matching endpoint"),
            Error::BadDescriptor => f.write_str("malformed or unexpected descriptor"),
            Error::Unsupported => f.write_str("io-usb is not available on this target"),
            Error::Connect { path, errno } => write!(
                f,
                "usbd_connect({path}) failed: {}",
                std::io::Error::from_raw_os_error(*errno)
            ),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_display_their_payloads() {
        let busy = Error::Errno(ffi::EBUSY).to_string();
        assert!(busy.contains("16"), "{busy}");
        let conn = Error::Connect {
            path: "/dev/io-usb/none".to_owned(),
            errno: 2,
        }
        .to_string();
        assert!(conn.contains("/dev/io-usb/none"), "{conn}");
        assert!(conn.contains('2'), "{conn}");
        let stall = Error::Status(ffi::USBD_STATUS_CMP_ERR | ffi::USBD_STATUS_STALL).to_string();
        assert_eq!(stall, "URB status 0x02000004 (CMP_ERR/STALL)");
        assert_eq!(
            Error::Status(ffi::USBD_STATUS_ABORTED).to_string(),
            "URB status 0x08000000 (ABORTED)"
        );
        assert_eq!(
            Error::Short { want: 512, got: 64 }.to_string(),
            "short transfer: 64 of 512 bytes"
        );
        assert_eq!(Error::Gone.to_string(), "device removed");
        assert_eq!(Error::Timeout.to_string(), "timed out");
        assert_eq!(
            Error::Interrupted.to_string(),
            "interrupted by a stop request"
        );
        assert_eq!(Error::NoEndpoint.to_string(), "no matching endpoint");
        assert!(Error::BadDescriptor.to_string().contains("descriptor"));
        assert!(Error::Unsupported.to_string().contains("not available"));
    }
}
