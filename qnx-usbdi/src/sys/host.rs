// SPDX-License-Identifier: Apache-2.0
//! Non-QNX stand-ins: the same types and constructors as `nto.rs`, none
//! of which can produce a value. `HostSys::connect` and `SlotSys::alloc`
//! return [`Error::Unsupported`]; everything else is a method on an
//! uninhabited type and is unreachable by construction, except
//! `urb_status`, a free function the completion callback calls, which
//! is unreachable because no `SlotSys` exists to submit a URB.
//!
//! Copyright 2026. Licensed under the Apache License, Version 2.0.

use core::ffi::c_void;

use super::{ControlDone, ControlSetup};
use crate::desc::Endpoint;
use crate::ffi::{UsbdDeviceDescriptor, UsbdDeviceInstance, UsbdIoCallback, UsbdUrb};
use crate::host::Ident;
use crate::{Error, Result};

/// A connection that cannot exist here.
pub(crate) enum HostSys {}

impl HostSys {
    pub(crate) fn connect(_path: &str, _ident: Ident, _connect_wait_s: u16) -> Result<HostSys> {
        Err(Error::Unsupported)
    }

    pub(crate) fn attach(&self, _raw: &mut UsbdDeviceInstance) -> Result<DeviceSys> {
        match *self {}
    }
}

/// An attached device that cannot exist here.
pub(crate) enum DeviceSys {}

impl DeviceSys {
    pub(crate) fn device_descriptor(&self) -> UsbdDeviceDescriptor {
        match *self {}
    }

    pub(crate) fn select_config(&self, _cfg: u8) -> Result<()> {
        match *self {}
    }

    pub(crate) fn select_interface(&self, _iface: u8, _alt: u8) -> Result<()> {
        match *self {}
    }

    pub(crate) fn descriptor(
        &self,
        _dtype: u8,
        _rtype: u16,
        _index: u8,
        _langid: u16,
        _buf: &mut [u8],
    ) -> Result<()> {
        match *self {}
    }

    pub(crate) fn control(
        &self,
        _setup: ControlSetup,
        _data: &mut [u8],
        _timeout_ms: u32,
    ) -> Result<ControlDone> {
        match *self {}
    }

    pub(crate) fn open_pipe(&self, _ep: Endpoint) -> Result<PipeSys> {
        match *self {}
    }
}

/// An open pipe that cannot exist here.
pub(crate) enum PipeSys {}

impl PipeSys {
    pub(crate) fn reset(&self) -> Result<()> {
        match *self {}
    }

    pub(crate) fn abort(&self) -> Result<()> {
        match *self {}
    }

    pub(crate) fn close(&self) -> Result<()> {
        match *self {}
    }

    pub(crate) fn submit_bulk_out(
        &self,
        _slot: &SlotSys,
        _len: u32,
        _cb: UsbdIoCallback,
        _handle: *mut c_void,
        _timeout_ms: u32,
    ) -> Result<()> {
        match *self {}
    }

    pub(crate) fn submit_intr_in(
        &self,
        _slot: &SlotSys,
        _len: u32,
        _cb: UsbdIoCallback,
        _handle: *mut c_void,
        _timeout_ms: u32,
    ) -> Result<()> {
        match *self {}
    }

    pub(crate) fn send_intr_out(
        &self,
        _slot: &SlotSys,
        _len: u32,
        _timeout_ms: u32,
    ) -> Result<(u32, u32)> {
        match *self {}
    }

    pub(crate) fn submit_bulk_in(
        &self,
        _slot: &SlotSys,
        _len: u32,
        _cb: UsbdIoCallback,
        _handle: *mut c_void,
        _timeout_ms: u32,
    ) -> Result<()> {
        match *self {}
    }
}

/// A URB plus DMA buffer pair that cannot exist here.
pub(crate) enum SlotSys {}

impl SlotSys {
    pub(crate) fn alloc(_bytes: usize) -> Result<SlotSys> {
        Err(Error::Unsupported)
    }

    pub(crate) fn fill(&self, _data: &[u8]) {
        match *self {}
    }

    pub(crate) fn read_to(&self, _dst: &mut [u8]) {
        match *self {}
    }

    pub(crate) fn owns_urb(&self, _urb: *mut UsbdUrb) -> bool {
        match *self {}
    }
}

/// No URB can be in flight here, so there is no status to read.
///
/// # Panics
/// Always: the only caller is the bulk completion callback, and no
/// `SlotSys` exists to submit a URB that could complete.
pub(crate) fn urb_status(_urb: *mut UsbdUrb) -> (u32, u32) {
    unreachable!("usbdi: a URB completion on a host build")
}
