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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Errno(i32),
    Status(u32),
    Short { want: usize, got: usize },
    Gone,
    Timeout,
    Interrupted,
    NoEndpoint,
    BadDescriptor,
    Unsupported,
    Connect { path: String, errno: i32 },
}
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
