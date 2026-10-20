// SPDX-License-Identifier: Apache-2.0
//! The byte-wise configuration descriptor walk, string decoding, and the
//! configuration-selection rule.
//!
//! Why byte-wise: this kernel runs with `SCTLR.A = 1` and cannot be
//! relaxed, so a pointer into the raw block cast to a descriptor type
//! (`&*p.cast()` or `ptr::read(p.cast())`) can cause SIGBUS
//! whenever an endpoint descriptor sits at an odd offset, which it does
//! after any second endpoint or any 9-byte class descriptor. An x86 host
//! test cannot see that fault. So every field is read as a `u8` index
//! and `wMaxPacketSize` as `u16::from_le_bytes`. The result is an owned,
//! aligned [`Endpoint`] built field by field.
//! Its address is what `usbd_open_pipe` receives.
//!
//! The raw block itself comes from `usbd_descriptor(dev, 0,
//! USB_DESC_CONFIGURATION, ...)` over EP0 (`device::Device::raw_config`):
//! this io-usb caches only the device descriptor, so
//! `usbd_parse_descriptors` cannot supply the interfaces and endpoints.
//!
//! The descriptor parser and its fixtures run on the host without a device.
//!
//! Copyright 2026. Licensed under the Apache License, Version 2.0.

use crate::ffi::{
    UsbdEndpointDescriptor, USB_ATTRIB_BULK, USB_ATTRIB_INTERRUPT, USB_ATTRIB_TYPE_MASK,
    USB_DESC_CONFIGURATION, USB_DESC_ENDPOINT, USB_DESC_INTERFACE, USB_DESC_STRING,
    USB_ENDPOINT_IN, USB_ENDPOINT_MASK,
};
use crate::{Error, Result};

/// Size of the EP0 staging buffer for a configuration descriptor block.
/// A longer block is truncated to this size.
pub const CONFIG_BUF_SIZE: usize = 1024;

/// The class triple of a `DisplayLink` display interface (udlfb's
/// `id_table`: vendor specific, no subclass, no protocol).
pub const VENDOR_CLASS_TRIPLE: (u8, u8, u8) = (0xFF, 0x00, 0x00);

/// An endpoint descriptor the port owns: an aligned `#[repr(C)]` value
/// built from the raw block, never a pointer into it. `Copy`, 8 bytes,
/// so it is passed by value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Endpoint {
    raw: UsbdEndpointDescriptor,
}

impl Endpoint {
    /// Build one from its fields (the wire `bLength` is kept so
    /// `usbd_open_pipe` sees what the device sent).
    #[must_use]
    pub const fn new(
        b_length: u8,
        address: u8,
        attributes: u8,
        max_packet: u16,
        interval: u8,
    ) -> Self {
        Endpoint {
            raw: UsbdEndpointDescriptor {
                b_length,
                b_descriptor_type: USB_DESC_ENDPOINT,
                b_endpoint_address: address,
                bm_attributes: attributes,
                w_max_packet_size: max_packet,
                b_interval: interval,
            },
        }
    }

    /// `bEndpointAddress`: number in bits 0..3, direction in bit 7.
    #[must_use]
    pub const fn address(self) -> u8 {
        self.raw.b_endpoint_address
    }

    /// The endpoint number (0..15).
    #[must_use]
    pub const fn number(self) -> u8 {
        self.raw.b_endpoint_address & USB_ENDPOINT_MASK
    }

    /// `bmAttributes`.
    #[must_use]
    pub const fn attributes(self) -> u8 {
        self.raw.bm_attributes
    }

    /// `wMaxPacketSize`.
    #[must_use]
    pub const fn max_packet(self) -> u16 {
        self.raw.w_max_packet_size
    }

    /// `bInterval`.
    #[must_use]
    pub const fn interval(self) -> u8 {
        self.raw.b_interval
    }

    /// Transfer type bits say bulk.
    #[must_use]
    pub const fn is_bulk(self) -> bool {
        self.raw.bm_attributes & USB_ATTRIB_TYPE_MASK == USB_ATTRIB_BULK
    }

    /// Transfer type bits say interrupt.
    #[must_use]
    pub const fn is_interrupt(self) -> bool {
        self.raw.bm_attributes & USB_ATTRIB_TYPE_MASK == USB_ATTRIB_INTERRUPT
    }

    /// Direction bit says device to host.
    #[must_use]
    pub const fn is_in(self) -> bool {
        self.raw.b_endpoint_address & USB_ENDPOINT_IN != 0
    }

    /// Direction bit says host to device.
    #[must_use]
    pub const fn is_out(self) -> bool {
        !self.is_in()
    }

    /// The raw C struct, for `usbd_open_pipe`.
    #[must_use]
    pub const fn raw(self) -> UsbdEndpointDescriptor {
        self.raw
    }
}

/// One interface descriptor of the block with the endpoints that follow
/// it (up to the next interface descriptor).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    /// `bInterfaceNumber`.
    pub number: u8,
    /// `bAlternateSetting`.
    pub alternate: u8,
    /// `bNumEndpoints` as declared (the walk may find fewer in a
    /// truncated block).
    pub num_endpoints: u8,
    /// `bInterfaceClass`.
    pub class: u8,
    /// `bInterfaceSubClass`.
    pub subclass: u8,
    /// `bInterfaceProtocol`.
    pub protocol: u8,
    /// The endpoints found after this descriptor, in block order.
    pub endpoints: Vec<Endpoint>,
}

impl Interface {
    /// The class triple, in the order udlfb's `id_table` states it.
    #[must_use]
    pub const fn class_triple(&self) -> (u8, u8, u8) {
        (self.class, self.subclass, self.protocol)
    }

    /// The first bulk OUT endpoint, the one a `DisplayLink` command
    /// stream is written to.
    #[must_use]
    pub fn first_bulk_out(&self) -> Option<Endpoint> {
        self.endpoints
            .iter()
            .copied()
            .find(|ep| ep.is_bulk() && ep.is_out())
    }
}

/// Check the block starts with a configuration descriptor and return its
/// usable length: `wTotalLength` clamped to the bytes actually present.
///
/// # Errors
/// [`Error::BadDescriptor`] when the block is shorter than 9 bytes, does
/// not begin with `bDescriptorType == 2`, or claims fewer than 9 bytes.
pub fn total_length(block: &[u8]) -> Result<usize> {
    if block.len() < 9 || block[1] != USB_DESC_CONFIGURATION || block[0] < 9 {
        return Err(Error::BadDescriptor);
    }
    let claimed = usize::from(u16::from_le_bytes([block[2], block[3]]));
    if claimed < 9 {
        return Err(Error::BadDescriptor);
    }
    Ok(claimed.min(block.len()))
}

/// `bNumInterfaces`: byte 4 of the block.
///
/// # Errors
/// [`Error::BadDescriptor`] as [`total_length`].
pub fn num_interfaces(block: &[u8]) -> Result<u8> {
    total_length(block)?;
    Ok(block[4])
}

/// Walk the whole block: every interface descriptor with the endpoints
/// that follow it. Class-specific and unknown descriptors are skipped by
/// their own `bLength`. A `bLength < 2` ends the walk.
///
/// # Errors
/// [`Error::BadDescriptor`] as [`total_length`].
pub fn walk(block: &[u8]) -> Result<Vec<Interface>> {
    let total = total_length(block)?;
    let mut out: Vec<Interface> = Vec::new();
    let mut off = 0usize;
    while off + 2 <= total {
        let blen = usize::from(block[off]);
        let btype = block[off + 1];
        if blen < 2 {
            break;
        }
        if btype == USB_DESC_INTERFACE && off + 9 <= total {
            out.push(Interface {
                number: block[off + 2],
                alternate: block[off + 3],
                num_endpoints: block[off + 4],
                class: block[off + 5],
                subclass: block[off + 6],
                protocol: block[off + 7],
                endpoints: Vec::new(),
            });
        } else if btype == USB_DESC_ENDPOINT && off + 7 <= total {
            if let Some(cur) = out.last_mut() {
                cur.endpoints.push(Endpoint::new(
                    block[off],
                    block[off + 2],
                    block[off + 3],
                    u16::from_le_bytes([block[off + 4], block[off + 5]]),
                    block[off + 6],
                ));
            }
        }
        off += blen;
    }
    Ok(out)
}

/// The interface descriptor with `bInterfaceNumber == iface` and
/// `bAlternateSetting == alt`, the one io-usb's instance names.
///
/// # Errors
/// [`Error::BadDescriptor`] when the block is malformed or holds no such
/// interface: the instance io-usb delivered and the block the device
/// returned disagree, which is a descriptor problem, not a missing
/// endpoint.
pub fn interface(block: &[u8], iface: u8, alt: u8) -> Result<Interface> {
    walk(block)?
        .into_iter()
        .find(|i| i.number == iface && i.alternate == alt)
        .ok_or(Error::BadDescriptor)
}

/// The endpoints of one interface, in block order.
///
/// # Errors
/// As [`interface`].
pub fn endpoints(block: &[u8], iface: u8, alt: u8) -> Result<Vec<Endpoint>> {
    Ok(interface(block, iface, alt)?.endpoints)
}

/// The class triple of one interface.
///
/// # Errors
/// As [`interface`].
pub fn interface_class(block: &[u8], iface: u8, alt: u8) -> Result<(u8, u8, u8)> {
    Ok(interface(block, iface, alt)?.class_triple())
}

/// Whether a class triple is the vendor-specific one a `DisplayLink`
/// display interface carries. The caller (the `dlo` claim helper)
/// applies this before claiming; the walk itself does not refuse other
/// classes, so a probe can still list them.
#[must_use]
pub fn is_vendor_class(triple: (u8, u8, u8)) -> bool {
    triple == VENDOR_CLASS_TRIPLE
}

/// The first bulk OUT endpoint of one interface. Logs a warning when it
/// is not endpoint 1 (every `DisplayLink` driver writes to endpoint 1;
/// a different number is worth a line in the record, not a refusal).
///
/// # Errors
/// [`Error::NoEndpoint`] when the interface has no bulk OUT endpoint;
/// [`Error::BadDescriptor`] as [`interface`].
pub fn find_bulk_out(block: &[u8], iface: u8, alt: u8) -> Result<Endpoint> {
    let intf = interface(block, iface, alt)?;
    let Some(ep) = intf.first_bulk_out() else {
        crate::log!(
            "usbdi: interface {iface} alt {alt} (class {:02x}/{:02x}/{:02x}) has no bulk OUT endpoint among {}",
            intf.class,
            intf.subclass,
            intf.protocol,
            intf.endpoints.len()
        );
        return Err(Error::NoEndpoint);
    };
    if ep.number() != 1 {
        crate::log!(
            "usbdi: warning: first bulk OUT of interface {iface} alt {alt} is endpoint {} (address {:#04x}), not 1",
            ep.number(),
            ep.address()
        );
    }
    Ok(ep)
}

/// Where the display interface lives: which configuration
/// (`bConfigurationValue`), which interface and alternate, and its bulk
/// OUT endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayInterface {
    /// `bConfigurationValue` of the configuration holding it.
    pub config: u8,
    /// Descriptor index of that configuration (`0..bNumConfigurations`).
    pub config_index: u8,
    /// `bInterfaceNumber`.
    pub iface: u8,
    /// `bAlternateSetting`.
    pub alternate: u8,
    /// The command-stream endpoint.
    pub bulk_out: Endpoint,
}

/// Find the display interface across a device's configuration blocks
/// (in descriptor-index order): the first interface with the vendor
/// class triple that has a bulk OUT endpoint.
///
/// A `DisplayLink` adapter (`17e9:02a7`) can expose two
/// configurations and lists the mass-storage "virtual CD" one first
/// (`bConfigurationValue` 2), the display one second
/// (`bConfigurationValue` 1). io-usb enumerates the first and matches
/// clients against ITS interfaces, so a class-filtered connect never
/// hears about the device, and a claim that trusted the instance's
/// configuration would open a pipe on the SCSI interface.
#[must_use]
pub fn find_display_interface(blocks: &[Vec<u8>]) -> Option<DisplayInterface> {
    for (index, block) in blocks.iter().enumerate() {
        let Ok(config_index) = u8::try_from(index) else {
            return None;
        };
        let Ok(interfaces) = walk(block) else {
            continue;
        };
        for intf in interfaces {
            if !is_vendor_class(intf.class_triple()) {
                continue;
            }
            if let Some(bulk_out) = intf.first_bulk_out() {
                return Some(DisplayInterface {
                    config: block[5],
                    config_index,
                    iface: intf.number,
                    alternate: intf.alternate,
                    bulk_out,
                });
            }
        }
    }
    None
}

/// The configuration-selection rule:
/// `SET_CONFIGURATION` is issued only on a single-interface device or
/// one that arrived unconfigured, and then with the instance's
/// configuration or 1. On an already-configured composite it is skipped,
/// because io-usb configured the device at enumeration and a second
/// `SET_CONFIGURATION` resets the endpoint toggles of every other
/// interface (a `devb-umass` virtual CD, an `io-hid` dock button).
#[must_use]
pub fn config_to_select(num_interfaces: u8, instance_config: u32) -> Option<u8> {
    if num_interfaces == 1 || instance_config == 0 {
        let cfg = u8::try_from(instance_config).unwrap_or(0);
        Some(if cfg == 0 { 1 } else { cfg })
    } else {
        None
    }
}

/// `USB_DESC_HID` (`hid.h`): the HID class descriptor that follows a HID
/// interface descriptor.
pub const USB_DESC_HID: u8 = 0x21;
/// `USB_DESC_HID_REPORT`: the HID report descriptor (`GET_DESCRIPTOR` with
/// the interface as recipient).
pub const USB_DESC_HID_REPORT: u8 = 0x22;

/// `wDescriptorLength` of the HID report descriptor of interface `iface`:
/// bytes 7..8 of the HID class descriptor that follows the interface's
/// descriptor in the configuration block.
/// `None` when the interface has no HID class descriptor or the block is
/// malformed.
#[must_use]
pub fn hid_report_len(block: &[u8], iface: u8) -> Option<u16> {
    let mut off = 0usize;
    let mut cur: Option<u8> = None;
    while off + 2 <= block.len() {
        let len = usize::from(block[off]);
        let kind = block[off + 1];
        if len < 2 || off + len > block.len() {
            return None;
        }
        if kind == USB_DESC_INTERFACE && len >= 9 {
            cur = Some(block[off + 2]);
        } else if kind == USB_DESC_HID && len >= 9 && cur == Some(iface) {
            return Some(u16::from(block[off + 7]) | (u16::from(block[off + 8]) << 8));
        }
        off += len;
    }
    None
}

/// The first LANGID of string descriptor 0 (`bLength >= 4`, type 3).
#[must_use]
pub fn langid(desc0: &[u8]) -> Option<u16> {
    if desc0.len() < 4 || desc0[0] < 4 || desc0[1] != USB_DESC_STRING {
        return None;
    }
    Some(u16::from_le_bytes([desc0[2], desc0[3]]))
}

/// Decode a string descriptor (`bLength`, type 3, UTF-16LE payload) as
/// libdlo's `usb_get_string_simple` does: every code unit above `0x7F`
/// becomes `?`, so a serial number is deterministic on every locale and
/// never fails on a character the C locale cannot encode (which is what
/// `usbd_string` does, and why it is not used).
#[must_use]
pub fn decode_string(desc: &[u8]) -> Option<String> {
    if desc.len() < 2 || desc[1] != USB_DESC_STRING {
        return None;
    }
    let blen = usize::from(desc[0]);
    if blen < 2 || blen > desc.len() {
        return None;
    }
    let payload = &desc[2..blen];
    let (pairs, _odd_tail) = payload.as_chunks::<2>();
    let text: String = pairs
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .map(|unit| {
            u8::try_from(unit)
                .ok()
                .filter(u8::is_ascii)
                .map_or('?', char::from)
        })
        .collect();
    // Trailing NUL code units are padding, not part of the serial.
    Some(text.trim_end_matches('\0').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hid_report_length_comes_from_the_asked_interface_only() {
        // config(9) if0(9) hid(9, report 0x0041) ep(7) if1(9) hid(9, 0x0102)
        let block = [
            9, 2, 52, 0, 2, 1, 0, 0xa0, 22, // configuration
            9, 4, 0, 0, 1, 3, 0, 0, 0, // interface 0
            9, 0x21, 0x10, 0x01, 0, 1, 0x22, 0x41, 0x00, // HID, report 65 bytes
            7, 5, 0x81, 3, 8, 0, 10, // endpoint
            9, 4, 1, 0, 1, 3, 0, 0, 0, // interface 1
            9, 0x21, 0x10, 0x01, 0, 1, 0x22, 0x02, 0x01, // HID, report 258 bytes
        ];
        assert_eq!(hid_report_len(&block, 0), Some(65));
        assert_eq!(hid_report_len(&block, 1), Some(258));
        assert_eq!(hid_report_len(&block, 2), None);
        assert_eq!(hid_report_len(&block[..20], 0), None);
    }

    /// (a) A block shaped like a DL-1xx adapter's: one vendor-specific
    /// interface with a class-specific `0x5F` block (one `max_area`
    /// record, 2048x1024) between the interface and its endpoints, so
    /// the second endpoint sits at an odd offset (37). Hand-built in the
    /// DL-1xx style.
    const DL1XX: [u8; 44] = [
        // configuration: 9 bytes, total 44, 1 interface, value 1, bus powered 500 mA
        9, 2, 44, 0, 1, 1, 0, 0x80, 0xFA, //
        // interface 0 alt 0: 2 endpoints, class FF/00/00
        9, 4, 0, 0, 2, 0xFF, 0x00, 0x00, 0, //
        // vendor descriptor 0x5F: 12 bytes, version 1, byte 4 = total - 2
        // (udlfb's "length after type"), key 0x0200 len 4 value 0x0020_0000
        12, 0x5F, 0x01, 0x00, 10, 0x00, 0x02, 0x04, 0x00, 0x00, 0x20, 0x00, //
        // endpoint 0x01 bulk OUT 512 (offset 30)
        7, 5, 0x01, 0x02, 0x00, 0x02, 0, //
        // endpoint 0x81 bulk IN 512 (offset 37)
        7, 5, 0x81, 0x02, 0x00, 0x02, 0, //
    ];

    /// (b) config(9) + interface(9) + endpoint(7) + endpoint(7): the
    /// second endpoint starts at offset 25.
    const TWO_EPS: [u8; 32] = [
        9, 2, 32, 0, 1, 1, 0, 0x80, 0x32, //
        9, 4, 0, 0, 2, 0xFF, 0x00, 0x00, 0, //
        7, 5, 0x01, 0x02, 0x00, 0x02, 0, //
        7, 5, 0x82, 0x02, 0x00, 0x02, 0, //
    ];

    /// (c) A composite: vendor interface 0 with bulk OUT 0x01, then a
    /// mass-storage interface 1 (08/06/50) with bulk IN 0x81 and OUT 0x02.
    const COMPOSITE: [u8; 48] = [
        9, 2, 48, 0, 2, 1, 0, 0x80, 0xFA, //
        9, 4, 0, 0, 1, 0xFF, 0x00, 0x00, 0, //
        7, 5, 0x01, 0x02, 0x00, 0x02, 0, //
        9, 4, 1, 0, 2, 0x08, 0x06, 0x50, 0, //
        7, 5, 0x81, 0x02, 0x00, 0x02, 0, //
        7, 5, 0x02, 0x02, 0x00, 0x02, 0, //
    ];

    /// (d) A dual-configuration display adapter (`17e9:02a7`): two
    /// configurations, the mass-storage "virtual CD" one FIRST
    /// (`bConfigurationValue` 2, interface 0 class 08/06/50, bulk OUT 3
    /// and IN 4), the display one second (`bConfigurationValue` 1,
    /// interface 0 class FF/00/00, bulk OUT 1 and interrupt IN 2, both
    /// self-powered 500 mA). Endpoint details and class-specific blocks
    /// beyond what `usb -vvv` prints are not reproduced.
    const MASS_STORAGE_CONFIG: [u8; 32] = [
        9, 2, 32, 0, 1, 2, 0, 0xC0, 0xFA, //
        9, 4, 0, 0, 2, 0x08, 0x06, 0x50, 0, //
        7, 5, 0x03, 0x02, 0x00, 0x02, 0, //
        7, 5, 0x84, 0x02, 0x00, 0x02, 0, //
    ];
    const DISPLAY_CONFIG: [u8; 32] = [
        9, 2, 32, 0, 1, 1, 0, 0xC0, 0xFA, //
        9, 4, 0, 0, 2, 0xFF, 0x00, 0x00, 0, //
        7, 5, 0x01, 0x02, 0x00, 0x02, 0, //
        7, 5, 0x82, 0x03, 0x08, 0x00, 1, //
    ];

    #[test]
    fn the_display_interface_is_found_in_the_second_configuration() {
        let blocks = vec![MASS_STORAGE_CONFIG.to_vec(), DISPLAY_CONFIG.to_vec()];
        let d = find_display_interface(&blocks).expect("display interface");
        assert_eq!(d.config, 1);
        assert_eq!(d.config_index, 1);
        assert_eq!((d.iface, d.alternate), (0, 0));
        assert_eq!(d.bulk_out.address(), 0x01);
        assert_eq!(d.bulk_out.max_packet(), 512);
        // The MSC configuration alone has no display interface, even
        // though it has a bulk OUT endpoint.
        assert_eq!(find_display_interface(&blocks[..1]), None);
        // A single-configuration DL-1xx block is found at index 0.
        let one = find_display_interface(&[DL1XX.to_vec()]).unwrap();
        assert_eq!((one.config, one.config_index), (1, 0));
        // A composite with the display interface first: interface 0.
        let c = find_display_interface(&[COMPOSITE.to_vec()]).unwrap();
        assert_eq!((c.config, c.iface, c.bulk_out.address()), (1, 0, 0x01));
        assert_eq!(find_display_interface(&[]), None);
    }

    #[test]
    fn dl1xx_block_yields_endpoint_1_out_past_a_class_block() {
        assert_eq!(DL1XX[30], 7);
        assert_eq!(DL1XX[37], 7);
        assert_eq!(num_interfaces(&DL1XX).unwrap(), 1);
        let ifs = walk(&DL1XX).unwrap();
        assert_eq!(ifs.len(), 1);
        assert_eq!(ifs[0].class_triple(), VENDOR_CLASS_TRIPLE);
        assert_eq!(ifs[0].endpoints.len(), 2);
        let ep = find_bulk_out(&DL1XX, 0, 0).unwrap();
        assert_eq!(ep.address(), 0x01);
        assert_eq!(ep.number(), 1);
        assert!(ep.is_bulk() && ep.is_out());
        assert_eq!(ep.max_packet(), 512);
        assert_eq!(ep.interval(), 0);
        assert_eq!(ep.raw().b_length, 7);
        assert_eq!(ep.raw().b_descriptor_type, 5);
        assert!(is_vendor_class(interface_class(&DL1XX, 0, 0).unwrap()));
        assert_eq!(config_to_select(1, 1), Some(1));
    }

    #[test]
    fn second_endpoint_at_offset_25_is_read_intact() {
        assert_eq!(TWO_EPS[25], 7);
        let eps = endpoints(&TWO_EPS, 0, 0).unwrap();
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].address(), 0x01);
        assert_eq!(eps[0].max_packet(), 512);
        assert_eq!(eps[1].address(), 0x82);
        assert_eq!(eps[1].max_packet(), 512);
        assert!(eps[1].is_in());
    }

    #[test]
    fn composite_selects_interface_0_and_keeps_1_apart() {
        assert_eq!(num_interfaces(&COMPOSITE).unwrap(), 2);
        let ep = find_bulk_out(&COMPOSITE, 0, 0).unwrap();
        assert_eq!(ep.address(), 0x01);
        let msc = interface_class(&COMPOSITE, 1, 0).unwrap();
        assert_eq!(msc, (0x08, 0x06, 0x50));
        assert!(!is_vendor_class(msc));
        // The MSC interface has its own bulk OUT; the caller refuses it
        // on the class triple, never by mistaking it for interface 0.
        assert_eq!(find_bulk_out(&COMPOSITE, 1, 0).unwrap().address(), 0x02);
        // Already configured composite: no SET_CONFIGURATION.
        assert_eq!(config_to_select(2, 1), None);
        // Arrived unconfigured: configure to 1.
        assert_eq!(config_to_select(2, 0), Some(1));
        assert_eq!(config_to_select(1, 0), Some(1));
        assert_eq!(config_to_select(1, 2), Some(2));
    }

    #[test]
    fn malformed_blocks_are_refused_not_walked() {
        assert_eq!(total_length(&[]), Err(Error::BadDescriptor));
        assert_eq!(
            total_length(&[9, 4, 0, 0, 0, 0, 0, 0, 0]),
            Err(Error::BadDescriptor)
        );
        assert_eq!(
            total_length(&[9, 2, 3, 0, 1, 1, 0, 0, 0]),
            Err(Error::BadDescriptor)
        );
        // A claimed total longer than the block is clamped, not trusted.
        let mut short = DL1XX;
        short[2] = 0xFF;
        short[3] = 0x03;
        assert_eq!(total_length(&short).unwrap(), 44);
        // No such interface in the block.
        assert_eq!(interface(&DL1XX, 1, 0), Err(Error::BadDescriptor));
        assert_eq!(interface(&DL1XX, 0, 1), Err(Error::BadDescriptor));
        // An interface without a bulk OUT.
        let mut no_out = TWO_EPS;
        no_out[20] = 0x81;
        assert_eq!(find_bulk_out(&no_out, 0, 0), Err(Error::NoEndpoint));
        // A zero bLength stops the walk instead of looping.
        let mut stuck = TWO_EPS;
        stuck[18] = 0;
        assert_eq!(endpoints(&stuck, 0, 0).unwrap().len(), 0);
    }

    #[test]
    fn strings_decode_as_ascii_with_question_marks() {
        // Descriptor 0: two LANGIDs, first 0x0409.
        assert_eq!(langid(&[6, 3, 0x09, 0x04, 0x07, 0x04]), Some(0x0409));
        assert_eq!(langid(&[2, 3]), None);
        assert_eq!(langid(&[4, 2, 0x09, 0x04]), None);
        // Contract item 12: bLength >= 4. A descriptor 0 that claims 2
        // or 3 bytes followed by stale buffer bytes (the 255-byte buffer
        // is zeroed before the call) must not yield a LANGID from them.
        assert_eq!(langid(&[2, 3, 0x09, 0x04]), None);
        assert_eq!(langid(&[3, 3, 0x09, 0x04]), None);
        assert_eq!(langid(&[4, 3, 0x09, 0x04]), Some(0x0409));
        // "AB" then U+00E4 then "1".
        let desc = [10, 3, b'A', 0, b'B', 0, 0xE4, 0x00, b'1', 0, 0xFF, 0xFF];
        assert_eq!(decode_string(&desc).as_deref(), Some("AB?1"));
        // bLength longer than the buffer, or wrong type, is refused.
        assert_eq!(decode_string(&[12, 3, b'A', 0]), None);
        assert_eq!(decode_string(&[4, 4, b'A', 0]), None);
        assert_eq!(decode_string(&[2, 3]).as_deref(), Some(""));
        // An odd payload drops the trailing half unit.
        assert_eq!(decode_string(&[5, 3, b'Z', 0, b'Q']).as_deref(), Some("Z"));
        // A fictional serial number followed by NUL padding units.
        let padded = [
            14, 3, b'1', 0, b'2', 0, b'3', 0, b'4', 0, b'5', 0, b'6', 0, 0, 0, 0, 0,
        ];
        assert_eq!(decode_string(&padded[..14]).as_deref(), Some("123456"));
        let mut long = vec![18, 3];
        long.extend_from_slice(&padded[2..]);
        assert_eq!(decode_string(&long).as_deref(), Some("123456"));
    }

    #[test]
    fn endpoint_is_owned_and_aligned() {
        let ep = Endpoint::new(7, 0x01, 0x02, 512, 0);
        let raw = ep.raw();
        assert_eq!(core::mem::size_of_val(&raw), 8);
        assert_eq!(core::mem::align_of_val(&raw), 2);
        assert_eq!(raw.w_max_packet_size, 512);
        assert_eq!(raw.b_interval, 0);
    }
}
