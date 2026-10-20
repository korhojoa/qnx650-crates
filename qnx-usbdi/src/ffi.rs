// SPDX-License-Identifier: Apache-2.0
//! `sys/usbdi.h` and `sys/usb100.h` (QNX 6.5.0 SDP, `lib/usbdi/public`),
//! transliterated.
//!
//! Every struct is `#[repr(C)]` and compiled on every target, so the
//! host `cargo test` evaluates the size and offset assertions of the
//! pointer-free ones; the two that carry pointers (`UsbdFuncs`,
//! `UsbdConnectParm`) are asserted under `target_pointer_width = "32"`,
//! which the QNX Clippy and build steps of the gate evaluate. Layouts are
//! computed for 32-bit ARM EABI: pointers 4 bytes, natural alignment, no
//! packing (the header has none). A `u16` field therefore aligns its
//! struct to 2, which is why the configuration descriptor is 10 bytes as
//! a C object though 9 on the wire, and the endpoint descriptor 8 though
//! 7. The port never reads either from a raw block through these types
//! (see `desc`); they exist because `usbd_open_pipe` takes the address
//! of one.
//!
//! Field names are the C names in snake case (`bLength` is `b_length`),
//! so a reader with the header open can match them line by line.
//!
//! `usbd_descriptors_t` is a union in C. The port only ever passes an
//! endpoint descriptor or a device descriptor to `usbd_open_pipe`, so
//! that argument is `*mut c_void` and the address of the owned, aligned
//! descriptor struct supplies the argument.
//!
//! The `extern "C"` block is declared for the nto target only. A host
//! build has no `libusbdi` to link, and the safe layer above compiles to
//! `Error::Unsupported` there (`sys/host.rs`).
//!
//! Copyright 2026. Licensed under the Apache License, Version 2.0.

use core::ffi::{c_char, c_int, c_void};
use core::marker::{PhantomData, PhantomPinned};

// ---- errno values the crate names (errno.h) ------------------------------

/// `errno.h` line 53: `EOK 0`.
pub const EOK: c_int = 0;
/// `errno.h` line 58: `EIO 5`.
pub const EIO: c_int = 5;
/// `errno.h` line 65: `ENOMEM 12`.
pub const ENOMEM: c_int = 12;
/// `errno.h` line 69: `EBUSY 16`.
pub const EBUSY: c_int = 16;
/// `errno.h` line 75: `EINVAL 22`.
pub const EINVAL: c_int = 22;

// ---- pthread.h constants (the callbacks' cancel state, see `cancel`) -----

/// `pthread.h` line 164: `PTHREAD_CANCEL_ENABLE 0x00`.
pub const PTHREAD_CANCEL_ENABLE: c_int = 0x00;
/// `pthread.h` line 165: `PTHREAD_CANCEL_DISABLE 0x10`.
pub const PTHREAD_CANCEL_DISABLE: c_int = 0x10;

// ---- sys/usbdi.h constants ---------------------------------------------

/// `usbdi.h` line 27: `USBD_VERSION 0x0101`.
pub const USBD_VERSION: u16 = 0x0101;
/// `usbdi.h` line 28: `USBD_CONNECT_WAIT 60`.
/// Connection setup uses [`crate::host::CONNECT_WAIT_S`] (2 s) to bound a
/// failed connection attempt.
pub const USBD_CONNECT_WAIT: u16 = 60;
/// `usbdi.h` line 30: `USBD_CONNECT_WILDCARD 0xffffffff`.
pub const USBD_CONNECT_WILDCARD: u32 = 0xffff_ffff;

/// `usbdi.h` line 32: `URB_DIR_IN 0x00000001`.
pub const URB_DIR_IN: u32 = 0x0000_0001;
/// `usbdi.h` line 33: `URB_DIR_OUT 0x00000002`.
pub const URB_DIR_OUT: u32 = 0x0000_0002;
/// `usbdi.h` line 35: `URB_SHORT_XFER_OK 0x00000004`.
pub const URB_SHORT_XFER_OK: u32 = 0x0000_0004;

/// `usbdi.h` line 40: `USBD_URB_STATUS_MASK 0xFF000000`.
pub const USBD_URB_STATUS_MASK: u32 = 0xFF00_0000;
/// `usbdi.h` line 41: `USBD_STATUS_INPROG 0x00000000`.
pub const USBD_STATUS_INPROG: u32 = 0x0000_0000;
/// `usbdi.h` line 42: `USBD_STATUS_CMP 0x01000000`.
pub const USBD_STATUS_CMP: u32 = 0x0100_0000;
/// `usbdi.h` line 43: `USBD_STATUS_CMP_ERR 0x02000000`.
pub const USBD_STATUS_CMP_ERR: u32 = 0x0200_0000;
/// `usbdi.h` line 44: `USBD_STATUS_TIMEOUT 0x04000000`.
pub const USBD_STATUS_TIMEOUT: u32 = 0x0400_0000;
/// `usbdi.h` line 45: `USBD_STATUS_ABORTED 0x08000000`.
pub const USBD_STATUS_ABORTED: u32 = 0x0800_0000;
/// `usbdi.h` line 46: `USBD_STATUS_DEVICE_RESET 0x10000000`.
pub const USBD_STATUS_DEVICE_RESET: u32 = 0x1000_0000;

/// `usbdi.h` line 48: `USBD_USB_STATUS_MASK 0x00FFFFFF`.
pub const USBD_USB_STATUS_MASK: u32 = 0x00FF_FFFF;
/// `usbdi.h` line 49: `USBD_STATUS_CRC_ERR 0x00000001`.
pub const USBD_STATUS_CRC_ERR: u32 = 0x0000_0001;
/// `usbdi.h` line 50: `USBD_STATUS_BITSTUFFING 0x00000002`.
pub const USBD_STATUS_BITSTUFFING: u32 = 0x0000_0002;
/// `usbdi.h` line 51: `USBD_STATUS_TOGGLE_MISMATCH 0x00000003`.
pub const USBD_STATUS_TOGGLE_MISMATCH: u32 = 0x0000_0003;
/// `usbdi.h` line 52: `USBD_STATUS_STALL 0x00000004`.
pub const USBD_STATUS_STALL: u32 = 0x0000_0004;
/// `usbdi.h` line 53: `USBD_STATUS_DEV_NOANSWER 0x00000005`.
pub const USBD_STATUS_DEV_NOANSWER: u32 = 0x0000_0005;
/// `usbdi.h` line 56: `USBD_STATUS_DATA_OVERRUN 0x00000008`.
pub const USBD_STATUS_DATA_OVERRUN: u32 = 0x0000_0008;
/// `usbdi.h` line 57: `USBD_STATUS_DATA_UNDERRUN 0x00000009`.
pub const USBD_STATUS_DATA_UNDERRUN: u32 = 0x0000_0009;
/// `usbdi.h` line 60: `USBD_STATUS_NOT_ACCESSED 0x0000000F`.
pub const USBD_STATUS_NOT_ACCESSED: u32 = 0x0000_000F;

/// `usbdi.h` line 65: `USBD_TIME_DEFAULT 0x00000000`.
pub const USBD_TIME_DEFAULT: u32 = 0x0000_0000;
/// `usbdi.h` line 66: `USBD_TIME_INFINITY 0xFFFFFFFF`.
pub const USBD_TIME_INFINITY: u32 = 0xFFFF_FFFF;

/// `usbdi.h` line 214: `_USBDI_NFUNCS` = `(sizeof(usbd_funcs_t) -
/// sizeof(_Uint32t)) / sizeof(void *)` = 3 on every target.
pub const USBDI_NFUNCS: u32 = 3;

// ---- sys/usb100.h constants --------------------------------------------

/// `usb100.h` line 25: `USB_VERSION 0x0110`.
pub const USB_VERSION: u16 = 0x0110;

/// `usb100.h` line 27: `USB_DESC_DEVICE 0x01`.
pub const USB_DESC_DEVICE: u8 = 0x01;
/// `usb100.h` line 28: `USB_DESC_CONFIGURATION 0x02`.
pub const USB_DESC_CONFIGURATION: u8 = 0x02;
/// `usb100.h` line 29: `USB_DESC_STRING 0x03`.
pub const USB_DESC_STRING: u8 = 0x03;
/// `usb100.h` line 30: `USB_DESC_INTERFACE 0x04`.
pub const USB_DESC_INTERFACE: u8 = 0x04;
/// `usb100.h` line 31: `USB_DESC_ENDPOINT 0x05`.
pub const USB_DESC_ENDPOINT: u8 = 0x05;

/// `usb100.h` line 56: `USB_ENDPOINT_MASK 0x0F`.
pub const USB_ENDPOINT_MASK: u8 = 0x0F;
/// `usb100.h` line 57: `USB_ENDPOINT_IN 0x80`.
pub const USB_ENDPOINT_IN: u8 = 0x80;
/// `usb100.h` line 58: `USB_ENDPOINT_OUT 0x00`.
pub const USB_ENDPOINT_OUT: u8 = 0x00;

/// `usb100.h` line 60: `USB_ATTRIB_CONTROL 0x00`.
pub const USB_ATTRIB_CONTROL: u8 = 0x00;
/// `usb100.h` line 61: `USB_ATTRIB_ISOCHRONOUS 0x01`.
pub const USB_ATTRIB_ISOCHRONOUS: u8 = 0x01;
/// `usb100.h` line 62: `USB_ATTRIB_BULK 0x02`.
pub const USB_ATTRIB_BULK: u8 = 0x02;
/// `usb100.h` line 63: `USB_ATTRIB_INTERRUPT 0x03`.
pub const USB_ATTRIB_INTERRUPT: u8 = 0x03;
/// The transfer-type bits of `bmAttributes` (USB 2.0 table 9-13).
pub const USB_ATTRIB_TYPE_MASK: u8 = 0x03;

/// `usb100.h` line 66: `USB_CLEAR_FEATURE 1`.
pub const USB_CLEAR_FEATURE: u8 = 1;
/// `usb100.h` line 69: `USB_GET_DESCRIPTOR 6`.
pub const USB_GET_DESCRIPTOR: u8 = 6;
/// `usb100.h` line 77: `USB_FEATURE_EPT_HALT 0`.
pub const USB_FEATURE_EPT_HALT: u16 = 0;

/// `usb100.h` line 80: `USB_RECIPIENT_DEVICE (0 << 0)`.
pub const USB_RECIPIENT_DEVICE: u8 = 0;
/// `usb100.h` line 81: `USB_RECIPIENT_INTERFACE (1 << 0)`.
pub const USB_RECIPIENT_INTERFACE: u8 = 1;
/// `usb100.h` line 82: `USB_RECIPIENT_ENDPOINT (2 << 0)`.
pub const USB_RECIPIENT_ENDPOINT: u8 = 2;
/// `usb100.h` line 84: `USB_TYPE_STANDARD (0 << 5)`.
pub const USB_TYPE_STANDARD: u8 = 0;
/// `usb100.h` line 85: `USB_TYPE_CLASS (1 << 5)`.
pub const USB_TYPE_CLASS: u8 = 0x20;
/// `usb100.h` line 86: `USB_TYPE_VENDOR (2 << 5)`.
pub const USB_TYPE_VENDOR: u8 = 0x40;

// ---- opaque handles ------------------------------------------------------

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

opaque!(
    /// `struct usbd_connection` (`usbdi.h` line 84): one `usbd_connect`.
    UsbdConnection
);
opaque!(
    /// `struct usbd_desc_node` (`usbdi.h` line 85): io-usb's parsed
    /// descriptor cache; only ever passed back to the library.
    UsbdDescNode
);
opaque!(
    /// `struct usbd_device` (`usbdi.h` line 86): one `usbd_attach`.
    UsbdDevice
);
opaque!(
    /// `struct usbd_pipe` (`usbdi.h` line 87): one `usbd_open_pipe`.
    UsbdPipe
);
opaque!(
    /// `struct usbd_urb` (`usbdi.h` line 88): one `usbd_alloc_urb`.
    UsbdUrb
);

// ---- descriptors (usbdi.h lines 90-143) ----------------------------------

/// `usbd_device_descriptor_t`, `usbdi.h` lines 90-105. 18 bytes, align 2.
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

/// `usbd_configuration_descriptor_t`, `usbdi.h` lines 107-116. 9 bytes
/// on the wire; 10 as a C object because `wTotalLength` aligns it to 2.
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

/// `usbd_interface_descriptor_t`, `usbdi.h` lines 124-134. 9 bytes.
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

/// `usbd_endpoint_descriptor_t`, `usbdi.h` lines 136-143. 7 bytes on the
/// wire; 8 as a C object (`wMaxPacketSize` at 4, `bInterval` at 6, one
/// byte of tail padding). NOT packed, exactly as the header.
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

// ---- connection records (usbdi.h lines 189-281) --------------------------

/// `usbd_device_ident_t`, `usbdi.h` lines 189-195. 20 bytes.
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

/// `usbd_device_instance_t`, `usbdi.h` lines 197-205. 36 bytes. io-usb
/// fills it for the insertion callback and writes it back through
/// `usbd_attach`, which is why the port never rebuilds one from parts.
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

/// The insertion/removal entry of `usbd_funcs_t` (`usbdi.h` lines
/// 209-210). No user pointer: the callbacks reach their state through a
/// process-global (see `events`).
pub type UsbdInstanceCallback =
    Option<unsafe extern "C" fn(*mut UsbdConnection, *mut UsbdDeviceInstance)>;

/// The `event` entry of `usbd_funcs_t` (`usbdi.h` line 211).
pub type UsbdEventCallback =
    Option<unsafe extern "C" fn(*mut UsbdConnection, *mut UsbdDeviceInstance, u16)>;

/// The completion callback of `usbd_io` (`usbdi.h` line 342).
pub type UsbdIoCallback = Option<unsafe extern "C" fn(*mut UsbdUrb, *mut UsbdPipe, *mut c_void)>;

/// `usbd_funcs_t`, `usbdi.h` lines 207-212. 16 bytes on a 32-bit target.
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

/// `usbd_connect_parm_t`, `usbdi.h` lines 268-281. 48 bytes on a 32-bit
/// target (`connect_wait` at 32, `evtprio` at 34, 13 reserved bytes).
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
    /// `_reserved` in the header; zeroed by every client.
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

// ---- the library entry points the port uses (usbdi.h lines 295-359) ------

#[cfg(target_os = "nto")]
extern "C" {
    /// `usbdi.h` line 295.
    pub fn usbd_connect(parm: *mut UsbdConnectParm, connection: *mut *mut UsbdConnection) -> c_int;
    /// `usbdi.h` line 296.
    pub fn usbd_disconnect(connection: *mut UsbdConnection) -> c_int;
    /// `usbdi.h` line 302. `instance` is written back by io-usb.
    pub fn usbd_attach(
        connection: *mut UsbdConnection,
        instance: *mut UsbdDeviceInstance,
        extra: usize,
        device: *mut *mut UsbdDevice,
    ) -> c_int;
    /// `usbdi.h` line 303.
    pub fn usbd_detach(device: *mut UsbdDevice) -> c_int;
    /// `usbdi.h` line 304.
    pub fn usbd_select_config(device: *mut UsbdDevice, config: u8) -> c_int;
    /// `usbdi.h` line 305.
    pub fn usbd_select_interface(device: *mut UsbdDevice, iface: u8, alternate: u8) -> c_int;
    /// `usbdi.h` line 308. `desc` is `usbd_descriptors_t *` in C; the
    /// port passes the address of an owned endpoint or device descriptor.
    pub fn usbd_open_pipe(
        device: *mut UsbdDevice,
        desc: *mut c_void,
        pipe: *mut *mut UsbdPipe,
    ) -> c_int;
    /// `usbdi.h` line 309.
    pub fn usbd_close_pipe(pipe: *mut UsbdPipe) -> c_int;
    /// `usbdi.h` line 310.
    pub fn usbd_reset_pipe(pipe: *mut UsbdPipe) -> c_int;
    /// `usbdi.h` line 311.
    pub fn usbd_abort_pipe(pipe: *mut UsbdPipe) -> c_int;
    /// `usbdi.h` line 314.
    pub fn usbd_alloc_urb(link: *mut UsbdUrb) -> *mut UsbdUrb;
    /// `usbdi.h` line 318. Returns the link, which the port never uses.
    pub fn usbd_free_urb(urb: *mut UsbdUrb) -> *mut UsbdUrb;
    /// `usbdi.h` line 319. DMA-safe memory for transfer buffers.
    pub fn usbd_alloc(size: usize) -> *mut c_void;
    /// `usbdi.h` line 320.
    pub fn usbd_free(ptr: *mut c_void);
    /// `usbdi.h` line 332. A synchronous `GET_DESCRIPTOR` (or
    /// `SET_DESCRIPTOR`, `set != 0`) over EP0.
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
    /// `usbdi.h` line 336. A generic setup packet: standard requests go
    /// through it too (`CLEAR_FEATURE` in the GIP shim).
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
    /// `usbdi.h` line 338.
    pub fn usbd_setup_bulk(urb: *mut UsbdUrb, flags: u32, addr: *mut c_void, len: u32) -> c_int;
    /// `usbdi.h` line 339. Same shape as bulk; the pipe's endpoint type
    /// is what makes it an interrupt transfer.
    pub fn usbd_setup_interrupt(
        urb: *mut UsbdUrb,
        flags: u32,
        addr: *mut c_void,
        len: u32,
    ) -> c_int;
    /// `usbdi.h` line 342.
    pub fn usbd_io(
        urb: *mut UsbdUrb,
        pipe: *mut UsbdPipe,
        func: UsbdIoCallback,
        handle: *mut c_void,
        timeout: u32,
    ) -> c_int;
    /// `usbdi.h` line 345.
    pub fn usbd_urb_status(urb: *mut UsbdUrb, status: *mut u32, length: *mut u32) -> c_int;
    /// `usbdi.h` line 359. The cached device descriptor of an attached
    /// device (the only descriptor this io-usb caches).
    pub fn usbd_device_descriptor(
        device: *mut UsbdDevice,
        node: *mut *mut UsbdDescNode,
    ) -> *mut UsbdDeviceDescriptor;
}

// ---- the libc entry point the callbacks use (pthread.h line 238) ---------

#[cfg(target_os = "nto")]
extern "C" {
    /// `pthread.h` line 238. Sets the calling thread's cancel state
    /// ([`PTHREAD_CANCEL_ENABLE`] / [`PTHREAD_CANCEL_DISABLE`]) and, when
    /// `oldstate` is non-null, stores the previous one there. libc is
    /// always linked, so no link attribute is needed. Fails (`EINVAL`)
    /// only for a `state` that is neither value \[RE: SDP `libc.so.3`
    /// `pthread_setcancelstate` @0x1fff0-0x2007c\]. Used by
    /// `crate::cancel::CancelGuard`.
    pub fn pthread_setcancelstate(state: c_int, oldstate: *mut c_int) -> c_int;
}

/// The name of the io-usb completion class in the high byte of a URB
/// status word, for log lines and `Error::Status`.
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

/// The name of the USB-level code in the low 24 bits of a URB status
/// word (`USBD_STATUS_STALL` and friends); empty when the code is zero.
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
        // pthread.h lines 164-165: the state is bit 0x10 of the thread's
        // flags, ENABLE clear, DISABLE set.
        assert_eq!(PTHREAD_CANCEL_ENABLE, 0x00);
        assert_eq!(PTHREAD_CANCEL_DISABLE, 0x10);
    }

    #[test]
    fn instance_layout_pins_the_fields_the_code_reads() {
        // The const asserts above already fail the build on drift; this
        // keeps a runtime witness so `cargo test` lists the check.
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
