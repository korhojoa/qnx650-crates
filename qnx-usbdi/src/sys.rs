// SPDX-License-Identifier: Apache-2.0
//! The one place `libusbdi` is called.
//!
//! Every `unsafe` block of the crate that touches io-usb lives in
//! `sys/nto.rs`, behind four small owning types (`HostSys`, `DeviceSys`,
//! `PipeSys`, `SlotSys`). Their `Drop` implementations release resources
//! in dependency order. The safe modules above (`host`, `device`, `bulk`) hold the
//! policy: what to check before a call, what to log, what an outcome
//! means.
//!
//! On a normal build for any other target `sys/host.rs` supplies the same four types as
//! uninhabited enums and the same constructors returning
//! [`crate::Error::Unsupported`]. Nothing fakes success: a host build can
//! neither connect nor allocate, so no `Device` or `BulkOut` can exist
//! there, and every method on those types is compiled but unreachable.
//! Unit tests select a separate backend to exercise the public transfer
//! methods. Those synthetic completions do not qualify native USB behavior.
//!
//! Copyright 2026 korhojoa. Licensed under Apache-2.0.

/// One control transfer's setup packet, without the direction bit:
/// io-usb derives `bmRequestType` bit 7 from the URB flags
/// (`URB_DIR_IN` / `URB_DIR_OUT`). The setup type can thus be
/// `USB_TYPE_VENDOR | USB_RECIPIENT_DEVICE` (0x40) for either direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ControlSetup {
    /// Device to host.
    pub dir_in: bool,
    /// `bRequest`.
    pub request: u8,
    /// `bmRequestType` type and recipient bits, passed through.
    pub rtype: u8,
    /// `wValue`.
    pub value: u16,
    /// `wIndex`.
    pub index: u16,
}

/// What `usbd_io` and `usbd_urb_status` reported for a control transfer
/// that `usbd_io` accepted (`EOK`). Judging it is the caller's job:
/// a transfer without a data stage is judged by `usbd_io` alone and
/// its raw status pair only logged (contract item 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ControlDone {
    /// The raw URB status word.
    pub status: u32,
    /// Bytes io-usb reports as transferred.
    pub alen: u32,
    /// `Instant`-measured time inside `usbd_io`.
    pub elapsed_ms: u64,
}

#[cfg(target_os = "nto")]
mod nto;
#[cfg(target_os = "nto")]
pub(crate) use nto::{urb_status, DeviceSys, HostSys, PipeSys, SlotSys};

#[cfg(all(not(target_os = "nto"), not(test)))]
mod host;
#[cfg(all(not(target_os = "nto"), not(test)))]
pub(crate) use host::{urb_status, DeviceSys, HostSys, PipeSys, SlotSys};
#[cfg(all(not(target_os = "nto"), test))]
pub(crate) mod host_test;
#[cfg(all(not(target_os = "nto"), test))]
pub(crate) use host_test::{urb_status, DeviceSys, HostSys, PipeSys, SlotSys};
