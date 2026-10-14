//! Copyright 2026. Licensed under the Apache License, Version 2.0.
use core::ffi::{c_char, c_int, c_void};
use core::marker::{PhantomData, PhantomPinned};
pub const EOK: c_int = 0;
pub const EIO: c_int = 5;
pub const ENOMEM: c_int = 12;
pub const EBUSY: c_int = 16;
pub const EINVAL: c_int = 22;
pub const PTHREAD_CANCEL_ENABLE: c_int = 0x00;
pub const PTHREAD_CANCEL_DISABLE: c_int = 0x10;
pub const USBD_VERSION: u16 = 0x0101;
pub const USBD_CONNECT_WAIT: u16 = 60;
pub const USBD_CONNECT_WILDCARD: u32 = 0xffff_ffff;
pub const URB_DIR_IN: u32 = 0x0000_0001;
pub const URB_DIR_OUT: u32 = 0x0000_0002;
pub const URB_SHORT_XFER_OK: u32 = 0x0000_0004;
pub const USBD_URB_STATUS_MASK: u32 = 0xFF00_0000;
pub const USBD_STATUS_INPROG: u32 = 0x0000_0000;
pub const USBD_STATUS_CMP: u32 = 0x0100_0000;
pub const USBD_STATUS_CMP_ERR: u32 = 0x0200_0000;
pub const USBD_STATUS_TIMEOUT: u32 = 0x0400_0000;
pub const USBD_STATUS_ABORTED: u32 = 0x0800_0000;
pub const USBD_STATUS_DEVICE_RESET: u32 = 0x1000_0000;
pub const USBD_USB_STATUS_MASK: u32 = 0x00FF_FFFF;
pub const USBD_STATUS_CRC_ERR: u32 = 0x0000_0001;
pub const USBD_STATUS_BITSTUFFING: u32 = 0x0000_0002;
pub const USBD_STATUS_TOGGLE_MISMATCH: u32 = 0x0000_0003;
pub const USBD_STATUS_STALL: u32 = 0x0000_0004;
pub const USBD_STATUS_DEV_NOANSWER: u32 = 0x0000_0005;
pub const USBD_STATUS_DATA_OVERRUN: u32 = 0x0000_0008;
pub const USBD_STATUS_DATA_UNDERRUN: u32 = 0x0000_0009;
pub const USBD_STATUS_NOT_ACCESSED: u32 = 0x0000_000F;
pub const USBD_TIME_DEFAULT: u32 = 0x0000_0000;
pub const USBD_TIME_INFINITY: u32 = 0xFFFF_FFFF;
pub const USBDI_NFUNCS: u32 = 3;
pub const USB_VERSION: u16 = 0x0110;
pub const USB_DESC_DEVICE: u8 = 0x01;
pub const USB_DESC_CONFIGURATION: u8 = 0x02;
pub const USB_DESC_STRING: u8 = 0x03;
pub const USB_DESC_INTERFACE: u8 = 0x04;
pub const USB_DESC_ENDPOINT: u8 = 0x05;
pub const USB_ENDPOINT_MASK: u8 = 0x0F;
pub const USB_ENDPOINT_IN: u8 = 0x80;
pub const USB_ENDPOINT_OUT: u8 = 0x00;
pub const USB_ATTRIB_CONTROL: u8 = 0x00;
pub const USB_ATTRIB_ISOCHRONOUS: u8 = 0x01;
pub const USB_ATTRIB_BULK: u8 = 0x02;
pub const USB_ATTRIB_INTERRUPT: u8 = 0x03;
pub const USB_ATTRIB_TYPE_MASK: u8 = 0x03;
pub const USB_CLEAR_FEATURE: u8 = 1;
pub const USB_GET_DESCRIPTOR: u8 = 6;
pub const USB_FEATURE_EPT_HALT: u16 = 0;
pub const USB_RECIPIENT_DEVICE: u8 = 0;
pub const USB_RECIPIENT_INTERFACE: u8 = 1;
pub const USB_RECIPIENT_ENDPOINT: u8 = 2;
pub const USB_TYPE_STANDARD: u8 = 0;
pub const USB_TYPE_CLASS: u8 = 0x20;
pub const USB_TYPE_VENDOR: u8 = 0x40;
macro_rules! opaque {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[repr(C)]
        pub struct $name {
            _data: [u8; 0],
            _marker: PhantomData<(*mut u8, PhantomPinned)>,
        }
    };
}
opaque!(UsbdConnection);
opaque!(UsbdDescNode);
opaque!(UsbdDevice);
opaque!(UsbdPipe);
opaque!(UsbdUrb);
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsbdDeviceDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub bcd_usb: u16,
    pub b_device_class: u8,
    pub b_device_sub_class: u8,
    pub b_device_protocol: u8,
    pub b_max_packet_size0: u8,
    pub id_vendor: u16,
    pub id_product: u16,
    pub bcd_device: u16,
    pub i_manufacturer: u8,
    pub i_product: u8,
    pub i_serial_number: u8,
    pub b_num_configurations: u8,
}
const _: () = assert!(core::mem::size_of::<UsbdDeviceDescriptor>() == 18);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceDescriptor, bcd_usb) == 2);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceDescriptor, id_vendor) == 8);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceDescriptor, id_product) == 10);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceDescriptor, bcd_device) == 12);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceDescriptor, i_manufacturer) == 14);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceDescriptor, i_product) == 15);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceDescriptor, i_serial_number) == 16);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceDescriptor, b_num_configurations) == 17);
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsbdConfigurationDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub w_total_length: u16,
    pub b_num_interfaces: u8,
    pub b_configuration_value: u8,
    pub i_configuration: u8,
    pub bm_attributes: u8,
    pub max_power: u8,
}
const _: () = assert!(core::mem::size_of::<UsbdConfigurationDescriptor>() == 10);
const _: () = assert!(core::mem::offset_of!(UsbdConfigurationDescriptor, w_total_length) == 2);
const _: () = assert!(core::mem::offset_of!(UsbdConfigurationDescriptor, b_num_interfaces) == 4);
const _: () = assert!(core::mem::offset_of!(UsbdConfigurationDescriptor, max_power) == 8);
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsbdInterfaceDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub b_interface_number: u8,
    pub b_alternate_setting: u8,
    pub b_num_endpoints: u8,
    pub b_interface_class: u8,
    pub b_interface_sub_class: u8,
    pub b_interface_protocol: u8,
    pub i_interface: u8,
}
const _: () = assert!(core::mem::size_of::<UsbdInterfaceDescriptor>() == 9);
const _: () = assert!(core::mem::offset_of!(UsbdInterfaceDescriptor, b_interface_number) == 2);
const _: () = assert!(core::mem::offset_of!(UsbdInterfaceDescriptor, b_alternate_setting) == 3);
const _: () = assert!(core::mem::offset_of!(UsbdInterfaceDescriptor, b_interface_class) == 5);
const _: () = assert!(core::mem::offset_of!(UsbdInterfaceDescriptor, b_interface_protocol) == 7);
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsbdEndpointDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub b_endpoint_address: u8,
    pub bm_attributes: u8,
    pub w_max_packet_size: u16,
    pub b_interval: u8,
}
const _: () = assert!(core::mem::size_of::<UsbdEndpointDescriptor>() == 8);
const _: () = assert!(core::mem::offset_of!(UsbdEndpointDescriptor, b_endpoint_address) == 2);
const _: () = assert!(core::mem::offset_of!(UsbdEndpointDescriptor, bm_attributes) == 3);
const _: () = assert!(core::mem::offset_of!(UsbdEndpointDescriptor, w_max_packet_size) == 4);
const _: () = assert!(core::mem::offset_of!(UsbdEndpointDescriptor, b_interval) == 6);
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsbdDeviceIdent {
    pub vendor: u32,
    pub device: u32,
    pub dclass: u32,
    pub subclass: u32,
    pub protocol: u32,
}
const _: () = assert!(core::mem::size_of::<UsbdDeviceIdent>() == 20);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceIdent, device) == 4);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceIdent, protocol) == 16);
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsbdDeviceInstance {
    pub path: u8,
    pub devno: u8,
    pub generation: u16,
    pub ident: UsbdDeviceIdent,
    pub config: u32,
    pub iface: u32,
    pub alternate: u32,
}
const _: () = assert!(core::mem::size_of::<UsbdDeviceInstance>() == 36);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceInstance, path) == 0);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceInstance, devno) == 1);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceInstance, generation) == 2);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceInstance, ident) == 4);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceInstance, config) == 24);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceInstance, iface) == 28);
const _: () = assert!(core::mem::offset_of!(UsbdDeviceInstance, alternate) == 32);
pub type UsbdInstanceCallback =
    Option<unsafe extern "C" fn(*mut UsbdConnection, *mut UsbdDeviceInstance)>;
pub type UsbdEventCallback =
    Option<unsafe extern "C" fn(*mut UsbdConnection, *mut UsbdDeviceInstance, u16)>;
pub type UsbdIoCallback = Option<unsafe extern "C" fn(*mut UsbdUrb, *mut UsbdPipe, *mut c_void)>;
#[repr(C)]
pub struct UsbdFuncs {
    pub nentries: u32,
    pub insertion: UsbdInstanceCallback,
    pub removal: UsbdInstanceCallback,
    pub event: UsbdEventCallback,
}
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::size_of::<UsbdFuncs>() == 16);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::offset_of!(UsbdFuncs, insertion) == 4);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::offset_of!(UsbdFuncs, removal) == 8);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::offset_of!(UsbdFuncs, event) == 12);
#[repr(C)]
pub struct UsbdConnectParm {
    pub path: *const c_char,
    pub vusb: u16,
    pub vusbd: u16,
    pub flags: u32,
    pub argc: i32,
    pub argv: *mut *mut c_char,
    pub evtbufsz: u32,
    pub ident: *mut UsbdDeviceIdent,
    pub funcs: *mut UsbdFuncs,
    pub connect_wait: u16,
    pub evtprio: u8,
    pub reserved: [u8; 13],
}
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::size_of::<UsbdConnectParm>() == 48);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::offset_of!(UsbdConnectParm, vusb) == 4);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::offset_of!(UsbdConnectParm, vusbd) == 6);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::offset_of!(UsbdConnectParm, ident) == 24);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::offset_of!(UsbdConnectParm, funcs) == 28);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::offset_of!(UsbdConnectParm, connect_wait) == 32);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::offset_of!(UsbdConnectParm, evtprio) == 34);
#[cfg(target_os = "nto")]
extern "C" {
    pub fn usbd_connect(parm: *mut UsbdConnectParm, connection: *mut *mut UsbdConnection) -> c_int;
    pub fn usbd_disconnect(connection: *mut UsbdConnection) -> c_int;
    pub fn usbd_attach(
        connection: *mut UsbdConnection,
        instance: *mut UsbdDeviceInstance,
        extra: usize,
        device: *mut *mut UsbdDevice,
    ) -> c_int;
    pub fn usbd_detach(device: *mut UsbdDevice) -> c_int;
    pub fn usbd_select_config(device: *mut UsbdDevice, config: u8) -> c_int;
    pub fn usbd_select_interface(device: *mut UsbdDevice, iface: u8, alternate: u8) -> c_int;
    pub fn usbd_open_pipe(
        device: *mut UsbdDevice,
        desc: *mut c_void,
        pipe: *mut *mut UsbdPipe,
    ) -> c_int;
    pub fn usbd_close_pipe(pipe: *mut UsbdPipe) -> c_int;
    pub fn usbd_reset_pipe(pipe: *mut UsbdPipe) -> c_int;
    pub fn usbd_abort_pipe(pipe: *mut UsbdPipe) -> c_int;
    pub fn usbd_alloc_urb(link: *mut UsbdUrb) -> *mut UsbdUrb;
    pub fn usbd_free_urb(urb: *mut UsbdUrb) -> *mut UsbdUrb;
    pub fn usbd_alloc(size: usize) -> *mut c_void;
    pub fn usbd_free(ptr: *mut c_void);
    pub fn usbd_descriptor(
        device: *mut UsbdDevice,
        set: c_int,
        dtype: u8,
        rtype: u16,
        index: u8,
        langid: u16,
        desc: *mut u8,
        len: usize,
    ) -> c_int;
    pub fn usbd_setup_vendor(
        urb: *mut UsbdUrb,
        flags: u32,
        request: u16,
        rtype: u16,
        value: u16,
        index: u16,
        addr: *mut c_void,
        len: u32,
    ) -> c_int;
    pub fn usbd_setup_bulk(urb: *mut UsbdUrb, flags: u32, addr: *mut c_void, len: u32) -> c_int;
    pub fn usbd_setup_interrupt(
        urb: *mut UsbdUrb,
        flags: u32,
        addr: *mut c_void,
        len: u32,
    ) -> c_int;
    pub fn usbd_io(
        urb: *mut UsbdUrb,
        pipe: *mut UsbdPipe,
        func: UsbdIoCallback,
        handle: *mut c_void,
        timeout: u32,
    ) -> c_int;
    pub fn usbd_urb_status(urb: *mut UsbdUrb, status: *mut u32, length: *mut u32) -> c_int;
    pub fn usbd_device_descriptor(
        device: *mut UsbdDevice,
        node: *mut *mut UsbdDescNode,
    ) -> *mut UsbdDeviceDescriptor;
}
#[cfg(target_os = "nto")]
extern "C" {
    pub fn pthread_setcancelstate(state: c_int, oldstate: *mut c_int) -> c_int;
}
#[must_use]
pub fn urb_status_name(status: u32) -> &'static str {
    match status & USBD_URB_STATUS_MASK {
        USBD_STATUS_INPROG => "INPROG",
        USBD_STATUS_CMP => "CMP",
        USBD_STATUS_CMP_ERR => "CMP_ERR",
        USBD_STATUS_TIMEOUT => "TIMEOUT",
        USBD_STATUS_ABORTED => "ABORTED",
        USBD_STATUS_DEVICE_RESET => "DEVICE_RESET",
        _ => "?",
    }
}
#[must_use]
pub fn usb_status_name(status: u32) -> &'static str {
    match status & USBD_USB_STATUS_MASK {
        0 => "",
        USBD_STATUS_CRC_ERR => "CRC_ERR",
        USBD_STATUS_BITSTUFFING => "BITSTUFFING",
        USBD_STATUS_TOGGLE_MISMATCH => "TOGGLE_MISMATCH",
        USBD_STATUS_STALL => "STALL",
        USBD_STATUS_DEV_NOANSWER => "DEV_NOANSWER",
        USBD_STATUS_DATA_OVERRUN => "DATA_OVERRUN",
        USBD_STATUS_DATA_UNDERRUN => "DATA_UNDERRUN",
        USBD_STATUS_NOT_ACCESSED => "NOT_ACCESSED",
        _ => "?",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_names_decode_both_halves() {
        assert_eq!(urb_status_name(USBD_STATUS_CMP), "CMP");
        assert_eq!(
            urb_status_name(USBD_STATUS_CMP_ERR | USBD_STATUS_STALL),
            "CMP_ERR"
        );
        assert_eq!(
            usb_status_name(USBD_STATUS_CMP_ERR | USBD_STATUS_STALL),
            "STALL"
        );
        assert_eq!(usb_status_name(USBD_STATUS_CMP), "");
        assert_eq!(urb_status_name(0x8000_0000), "?");
        assert_eq!(urb_status_name(USBD_STATUS_ABORTED), "ABORTED");
        assert_eq!(usb_status_name(USBD_STATUS_NOT_ACCESSED), "NOT_ACCESSED");
    }
    #[test]
    fn ident_wildcard_and_versions_match_the_headers() {
        assert_eq!(USBD_CONNECT_WILDCARD, u32::MAX);
        assert_eq!(USB_VERSION, 0x0110);
        assert_eq!(USBD_VERSION, 0x0101);
        assert_eq!(USBDI_NFUNCS, 3);
        assert_eq!(USB_TYPE_VENDOR | USB_RECIPIENT_DEVICE, 0x40);
        assert_eq!(USB_TYPE_STANDARD | USB_RECIPIENT_ENDPOINT, 0x02);
        assert_eq!(PTHREAD_CANCEL_ENABLE, 0x00);
        assert_eq!(PTHREAD_CANCEL_DISABLE, 0x10);
    }
    #[test]
    fn instance_layout_pins_the_fields_the_code_reads() {
        let inst = UsbdDeviceInstance {
            path: 1,
            devno: 2,
            generation: 3,
            ident: UsbdDeviceIdent {
                vendor: 0x17E9,
                device: 0x0198,
                dclass: 0xFF,
                subclass: 0,
                protocol: 0,
            },
            config: 1,
            iface: 0,
            alternate: 0,
        };
        assert_eq!(core::mem::size_of_val(&inst), 36);
        assert_eq!(inst.ident.vendor, 0x17E9);
    }
}
