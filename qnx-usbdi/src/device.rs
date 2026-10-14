//! Copyright 2026. Licensed under the Apache License, Version 2.0.
use crate::bulk::{BulkOut, CLEAR_HALT_TIMEOUT_MS};
use crate::desc::{self, Endpoint, CONFIG_BUF_SIZE};
use crate::events::{GoneFlag, Instance};
use crate::ffi::{
    UsbdDeviceDescriptor, USBD_STATUS_CMP, USBD_URB_STATUS_MASK, USB_CLEAR_FEATURE,
    USB_DESC_CONFIGURATION, USB_DESC_STRING, USB_FEATURE_EPT_HALT, USB_GET_DESCRIPTOR,
    USB_RECIPIENT_DEVICE, USB_RECIPIENT_ENDPOINT, USB_TYPE_STANDARD,
};
use crate::host::HostShared;
use crate::intr::IntrIn;
use crate::intr_out::IntrOut;
use crate::sys::{self, ControlSetup};
use crate::{Error, Result};
use std::cell::RefCell;
use std::rc::Rc;
pub const DESCRIPTOR_TIMEOUT_MS: u32 = 5000;
pub(crate) const fn descriptor_wvalue(dtype: u8, index: u8) -> u16 {
    ((dtype as u16) << 8) | (index as u16)
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceDescriptor {
    pub vendor: u16,
    pub product: u16,
    pub bcd_usb: u16,
    pub bcd_device: u16,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub max_packet0: u8,
    pub i_manufacturer: u8,
    pub i_product: u8,
    pub i_serial: u8,
    pub num_configurations: u8,
}
impl DeviceDescriptor {
    #[must_use]
    pub const fn from_raw(raw: &UsbdDeviceDescriptor) -> Self {
        DeviceDescriptor {
            vendor: raw.id_vendor,
            product: raw.id_product,
            bcd_usb: raw.bcd_usb,
            bcd_device: raw.bcd_device,
            class: raw.b_device_class,
            subclass: raw.b_device_sub_class,
            protocol: raw.b_device_protocol,
            max_packet0: raw.b_max_packet_size0,
            i_manufacturer: raw.i_manufacturer,
            i_product: raw.i_product,
            i_serial: raw.i_serial_number,
            num_configurations: raw.b_num_configurations,
        }
    }
}
pub(crate) struct DeviceShared {
    pub(crate) inst: Instance,
    pub(crate) desc: DeviceDescriptor,
    pub(crate) gone: GoneFlag,
    pub(crate) config: RefCell<Option<Vec<u8>>>,
    pub(crate) sys: sys::DeviceSys,
    pub(crate) _host: Rc<HostShared>,
}
impl DeviceShared {
    pub(crate) fn control(
        &self,
        setup: ControlSetup,
        data: &mut [u8],
        timeout_ms: u32,
    ) -> Result<usize> {
        let dir = if setup.dir_in { "IN" } else { "OUT" };
        let len = data.len();
        if self.gone.is_set() {
            crate::log!(
                "usbdi: control {dir} req {:#04x} len {len} refused: device removed",
                setup.request
            );
            return Err(Error::Gone);
        }
        let done = self.sys.control(setup, data, timeout_ms)?;
        let class = crate::ffi::urb_status_name(done.status);
        let usb = crate::ffi::usb_status_name(done.status);
        if len == 0 {
            crate::vlog!(
                "usbdi: control {dir} req {:#04x} rtype {:#04x} value {:#06x} index {:#06x} len 0: usbd_io EOK, raw status {:#010x} ({class}/{usb}) alen {} in {} ms (timeout {timeout_ms} ms); judged by rc alone",
                setup.request,
                setup.rtype,
                setup.value,
                setup.index,
                done.status,
                done.alen,
                done.elapsed_ms
            );
            return Ok(0);
        }
        if done.status & USBD_URB_STATUS_MASK != USBD_STATUS_CMP {
            crate::log!(
                "usbdi: control {dir} req {:#04x} rtype {:#04x} value {:#06x} index {:#06x} len {len}: status {:#010x} ({class}/{usb}) alen {} in {} ms (timeout {timeout_ms} ms)",
                setup.request,
                setup.rtype,
                setup.value,
                setup.index,
                done.status,
                done.alen,
                done.elapsed_ms
            );
            return Err(Error::Status(done.status));
        }
        let got = usize::try_from(done.alen).unwrap_or(usize::MAX).min(len);
        crate::vlog!(
            "usbdi: control {dir} req {:#04x} rtype {:#04x} value {:#06x} index {:#06x} len {len}: CMP alen {} in {} ms (timeout {timeout_ms} ms)",
            setup.request,
            setup.rtype,
            setup.value,
            setup.index,
            done.alen,
            done.elapsed_ms
        );
        Ok(if setup.dir_in { got } else { len })
    }
    /// # Errors
    /// [`Error::Gone`] when the device is gone.
    /// The fallback error when the two methods fail.
    pub(crate) fn reset_pipe(&self, pipe: &sys::PipeSys, ep: Endpoint) -> Result<()> {
        if self.gone.is_set() {
            return Err(Error::Gone);
        }
        let Err(e) = pipe.reset() else {
            crate::vlog!("usbdi: reset_pipe(endpoint {:#04x}) ok", ep.address());
            return Ok(());
        };
        crate::log!(
            "usbdi: reset_pipe(endpoint {:#04x}) failed: {e}; trying CLEAR_FEATURE(ENDPOINT_HALT)",
            ep.address()
        );
        let setup = ControlSetup {
            dir_in: false,
            request: USB_CLEAR_FEATURE,
            rtype: USB_TYPE_STANDARD | USB_RECIPIENT_ENDPOINT,
            value: USB_FEATURE_EPT_HALT,
            index: u16::from(ep.address()),
        };
        match self.control(setup, &mut [], CLEAR_HALT_TIMEOUT_MS) {
            Ok(_) => {
                crate::log!("usbdi: clear_halt(endpoint {:#04x}) ok", ep.address());
                Ok(())
            }
            Err(e2) => {
                crate::log!(
                    "usbdi: clear_halt(endpoint {:#04x}) failed: {e2}",
                    ep.address()
                );
                Err(e2)
            }
        }
    }
}
impl Drop for DeviceShared {
    fn drop(&mut self) {
        crate::log!("usbdi: releasing {}", self.inst);
    }
}
pub struct Device {
    shared: Rc<DeviceShared>,
}
impl Device {
    pub(crate) fn from_shared(shared: Rc<DeviceShared>) -> Device {
        Device { shared }
    }
    #[must_use]
    pub fn instance(&self) -> Instance {
        self.shared.inst
    }
    #[must_use]
    pub fn descriptor(&self) -> DeviceDescriptor {
        self.shared.desc
    }
    #[must_use]
    pub fn gone(&self) -> bool {
        self.shared.gone.is_set()
    }
    #[must_use]
    pub fn gone_flag(&self) -> GoneFlag {
        self.shared.gone.clone()
    }
    #[must_use]
    pub fn string(&self, index: u8) -> Option<String> {
        if index == 0 {
            return None;
        }
        if self.gone() {
            crate::log!("usbdi: string {index} refused: device removed");
            return None;
        }
        let rtype = u16::from(USB_TYPE_STANDARD | USB_RECIPIENT_DEVICE);
        let mut buf = [0u8; 255];
        if let Err(e) = self
            .shared
            .sys
            .descriptor(USB_DESC_STRING, rtype, 0, 0, &mut buf)
        {
            crate::log!("usbdi: string descriptor 0 failed: {e}");
            return None;
        }
        let Some(lang) = desc::langid(&buf) else {
            crate::log!(
                "usbdi: string descriptor 0 malformed (bLength {} type {})",
                buf[0],
                buf[1]
            );
            return None;
        };
        buf = [0u8; 255];
        if let Err(e) = self
            .shared
            .sys
            .descriptor(USB_DESC_STRING, rtype, index, lang, &mut buf)
        {
            crate::log!("usbdi: string descriptor {index} (langid {lang:#06x}) failed: {e}");
            return None;
        }
        let text = desc::decode_string(&buf);
        if text.is_none() {
            crate::log!(
                "usbdi: string descriptor {index} malformed (bLength {} type {})",
                buf[0],
                buf[1]
            );
        }
        text
    }
    /// # Errors
    /// [`Error::Gone`]. [`Error::Errno`] from the transfer.
    /// [`Error::BadDescriptor`] when the block does not start with a
    /// configuration descriptor.
    pub fn raw_config(&self) -> Result<Vec<u8>> {
        if let Some(block) = self.shared.config.borrow().as_ref() {
            return Ok(block.clone());
        }
        let buf = self.raw_config_at(0)?;
        *self.shared.config.borrow_mut() = Some(buf.clone());
        Ok(buf)
    }
    /// # Errors
    /// [`Error::Gone`]. [`Error::Errno`] from the transfer.
    /// [`Error::BadDescriptor`] when the block does not start with a
    /// configuration descriptor.
    pub fn raw_config_at(&self, index: u8) -> Result<Vec<u8>> {
        if self.gone() {
            return Err(Error::Gone);
        }
        let mut buf = vec![0u8; CONFIG_BUF_SIZE];
        self.shared.sys.descriptor(
            USB_DESC_CONFIGURATION,
            u16::from(USB_TYPE_STANDARD | USB_RECIPIENT_DEVICE),
            index,
            0,
            &mut buf,
        )?;
        let total = match desc::total_length(&buf) {
            Ok(n) => n,
            Err(e) => {
                crate::log!(
                    "usbdi: configuration block {index} malformed: bLength {} type {} wTotalLength {}",
                    buf[0],
                    buf[1],
                    u16::from_le_bytes([buf[2], buf[3]])
                );
                return Err(e);
            }
        };
        buf.truncate(total);
        crate::vlog!(
            "usbdi: configuration block {index}: {total} bytes, {} interface(s), bConfigurationValue {}",
            buf[4],
            buf[5]
        );
        Ok(buf)
    }
    pub fn set_config_block(&self, block: Vec<u8>) {
        *self.shared.config.borrow_mut() = Some(block);
    }
    /// # Errors
    /// As [`Device::raw_config`].
    pub fn num_interfaces(&self) -> Result<u8> {
        desc::num_interfaces(&self.raw_config()?)
    }
    /// # Errors
    /// As [`Device::raw_config`]. [`Error::BadDescriptor`] when the
    /// block has no such interface.
    pub fn endpoints(&self, iface: u8, alt: u8) -> Result<Vec<Endpoint>> {
        desc::endpoints(&self.raw_config()?, iface, alt)
    }
    /// # Errors
    /// As [`Device::endpoints`].
    pub fn interface_class(&self, iface: u8, alt: u8) -> Result<(u8, u8, u8)> {
        desc::interface_class(&self.raw_config()?, iface, alt)
    }
    /// # Errors
    /// As [`Device::endpoints`]. [`Error::NoEndpoint`] when there is none.
    pub fn find_bulk_out(&self, iface: u8, alt: u8) -> Result<Endpoint> {
        desc::find_bulk_out(&self.raw_config()?, iface, alt)
    }
    /// # Errors
    /// [`Error::Gone`]. [`Error::Errno`] from io-usb (logged with the
    /// instance's config and, when the block has been read, its
    /// `bNumInterfaces`).
    pub fn select_config(&self, cfg: u8) -> Result<()> {
        if self.gone() {
            return Err(Error::Gone);
        }
        let num_if = self.shared.config.borrow().as_ref().map(|b| b[4]);
        match self.shared.sys.select_config(cfg) {
            Ok(()) => {
                crate::vlog!("usbdi: select_config({cfg}) ok");
                Ok(())
            }
            Err(e) => {
                crate::log!(
                    "usbdi: select_config({cfg}) failed: {e} (instance config {}, bNumInterfaces {num_if:?})",
                    self.shared.inst.config()
                );
                Err(e)
            }
        }
    }
    /// # Errors
    /// [`Error::Gone`]. [`Error::Errno`] from io-usb (logged with the
    /// instance's config and, when the block has been read, its
    /// `bNumInterfaces`).
    pub fn select_interface(&self, iface: u8, alt: u8) -> Result<()> {
        if self.gone() {
            return Err(Error::Gone);
        }
        let num_if = self.shared.config.borrow().as_ref().map(|b| b[4]);
        match self.shared.sys.select_interface(iface, alt) {
            Ok(()) => {
                crate::vlog!("usbdi: select_interface({iface}, {alt}) ok");
                Ok(())
            }
            Err(e) => {
                crate::log!(
                    "usbdi: select_interface({iface}, {alt}) failed: {e} (instance config {}, bNumInterfaces {num_if:?})",
                    self.shared.inst.config()
                );
                Err(e)
            }
        }
    }
    /// # Errors
    /// [`Error::Gone`]. [`Error::Errno`] when `usbd_io` refuses.
    /// [`Error::Status`] when the URB did not complete `CMP` (a `STALL`
    /// or a timed-out stage is reported this way).
    pub fn control_in(
        &self,
        req: u8,
        rtype: u8,
        value: u16,
        index: u16,
        buf: &mut [u8],
        timeout_ms: u32,
    ) -> Result<usize> {
        let setup = ControlSetup {
            dir_in: true,
            request: req,
            rtype,
            value,
            index,
        };
        self.shared.control(setup, buf, timeout_ms)
    }
    /// # Errors
    /// As [`Device::control_in`].
    pub fn control_out(
        &self,
        req: u8,
        rtype: u8,
        value: u16,
        index: u16,
        data: &[u8],
        timeout_ms: u32,
    ) -> Result<()> {
        let setup = ControlSetup {
            dir_in: false,
            request: req,
            rtype,
            value,
            index,
        };
        let mut copy = data.to_vec();
        self.shared.control(setup, &mut copy, timeout_ms).map(drop)
    }
    /// # Errors
    /// As [`Device::control_in`].
    pub fn get_descriptor(
        &self,
        dtype: u8,
        index: u8,
        langid: u16,
        buf: &mut [u8],
    ) -> Result<usize> {
        self.control_in(
            USB_GET_DESCRIPTOR,
            USB_TYPE_STANDARD | USB_RECIPIENT_DEVICE,
            descriptor_wvalue(dtype, index),
            langid,
            buf,
            DESCRIPTOR_TIMEOUT_MS,
        )
    }
    /// # Errors
    /// [`Error::NoEndpoint`] unless `ep` is bulk OUT. [`Error::Errno`]
    /// (`EINVAL`) for zero slots or bytes. [`Error::Gone`].
    /// [`Error::Errno`] from `usbd_open_pipe` or the allocations.
    pub fn open_bulk_out(&self, ep: Endpoint, slots: usize, slot_bytes: usize) -> Result<BulkOut> {
        BulkOut::open(Rc::clone(&self.shared), ep, slots, slot_bytes)
    }
    /// # Errors
    /// [`Error::NoEndpoint`] unless `ep` is bulk IN.
    /// [`Error::Errno`] (`EINVAL`) for zero slots, zero bytes, or no armed URB.
    /// [`Error::Gone`] when the device is gone.
    /// [`Error::Errno`] from `usbd_open_pipe` or allocation.
    pub fn open_bulk_in(
        &self,
        ep: Endpoint,
        slots: usize,
        slot_bytes: usize,
    ) -> Result<crate::bulk_in::BulkIn> {
        crate::bulk_in::BulkIn::open(Rc::clone(&self.shared), ep, slots, slot_bytes)
    }
    /// # Errors
    /// [`Error::NoEndpoint`] unless `ep` is interrupt IN.
    /// [`Error::Errno`] (`EINVAL`) for zero slots, zero packet size, or no armed URB.
    /// [`Error::Gone`] when the device is gone.
    /// [`Error::Errno`] from `usbd_open_pipe` or allocation.
    pub fn open_intr_in(&self, ep: Endpoint, slots: usize) -> Result<IntrIn> {
        IntrIn::open(Rc::clone(&self.shared), ep, slots)
    }
    /// # Errors
    /// [`Error::NoEndpoint`] unless `ep` is interrupt OUT.
    /// [`Error::Errno`] (`EINVAL`) for a zero packet size. [`Error::Gone`].
    /// [`Error::Errno`] from `usbd_open_pipe` or the allocations.
    pub fn open_intr_out(&self, ep: Endpoint) -> Result<IntrOut> {
        IntrOut::open(Rc::clone(&self.shared), ep)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_descriptor_decodes_every_field() {
        let raw = UsbdDeviceDescriptor {
            b_length: 18,
            b_descriptor_type: 1,
            bcd_usb: 0x0200,
            b_device_class: 0,
            b_device_sub_class: 0,
            b_device_protocol: 0,
            b_max_packet_size0: 64,
            id_vendor: 0x17E9,
            id_product: 0x0198,
            bcd_device: 0x0100,
            i_manufacturer: 1,
            i_product: 2,
            i_serial_number: 3,
            b_num_configurations: 1,
        };
        let d = DeviceDescriptor::from_raw(&raw);
        assert_eq!((d.vendor, d.product), (0x17E9, 0x0198));
        assert_eq!((d.bcd_usb, d.bcd_device), (0x0200, 0x0100));
        assert_eq!((d.class, d.subclass, d.protocol), (0, 0, 0));
        assert_eq!(d.max_packet0, 64);
        assert_eq!((d.i_manufacturer, d.i_product, d.i_serial), (1, 2, 3));
        assert_eq!(d.num_configurations, 1);
    }
    #[test]
    fn get_descriptor_packs_wvalue_as_type_then_index() {
        assert_eq!(descriptor_wvalue(0x5F, 0), 0x5F00);
        assert_eq!(descriptor_wvalue(0x03, 2), 0x0302);
        assert_eq!(descriptor_wvalue(0x01, 0), 0x0100);
        assert_eq!(DESCRIPTOR_TIMEOUT_MS, 5000);
    }
}
