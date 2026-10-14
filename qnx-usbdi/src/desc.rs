//! Copyright 2026. Licensed under the Apache License, Version 2.0.
use crate::ffi::{
    UsbdEndpointDescriptor, USB_ATTRIB_BULK, USB_ATTRIB_INTERRUPT, USB_ATTRIB_TYPE_MASK,
    USB_DESC_CONFIGURATION, USB_DESC_ENDPOINT, USB_DESC_INTERFACE, USB_DESC_STRING,
    USB_ENDPOINT_IN, USB_ENDPOINT_MASK,
};
use crate::{Error, Result};
pub const CONFIG_BUF_SIZE: usize = 1024;
pub const VENDOR_CLASS_TRIPLE: (u8, u8, u8) = (0xFF, 0x00, 0x00);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Endpoint {
    raw: UsbdEndpointDescriptor,
}
impl Endpoint {
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
    #[must_use]
    pub const fn address(self) -> u8 {
        self.raw.b_endpoint_address
    }
    #[must_use]
    pub const fn number(self) -> u8 {
        self.raw.b_endpoint_address & USB_ENDPOINT_MASK
    }
    #[must_use]
    pub const fn attributes(self) -> u8 {
        self.raw.bm_attributes
    }
    #[must_use]
    pub const fn max_packet(self) -> u16 {
        self.raw.w_max_packet_size
    }
    #[must_use]
    pub const fn interval(self) -> u8 {
        self.raw.b_interval
    }
    #[must_use]
    pub const fn is_bulk(self) -> bool {
        self.raw.bm_attributes & USB_ATTRIB_TYPE_MASK == USB_ATTRIB_BULK
    }
    #[must_use]
    pub const fn is_interrupt(self) -> bool {
        self.raw.bm_attributes & USB_ATTRIB_TYPE_MASK == USB_ATTRIB_INTERRUPT
    }
    #[must_use]
    pub const fn is_in(self) -> bool {
        self.raw.b_endpoint_address & USB_ENDPOINT_IN != 0
    }
    #[must_use]
    pub const fn is_out(self) -> bool {
        !self.is_in()
    }
    #[must_use]
    pub const fn raw(self) -> UsbdEndpointDescriptor {
        self.raw
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub number: u8,
    pub alternate: u8,
    pub num_endpoints: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub endpoints: Vec<Endpoint>,
}
impl Interface {
    #[must_use]
    pub const fn class_triple(&self) -> (u8, u8, u8) {
        (self.class, self.subclass, self.protocol)
    }
    #[must_use]
    pub fn first_bulk_out(&self) -> Option<Endpoint> {
        self.endpoints
            .iter()
            .copied()
            .find(|ep| ep.is_bulk() && ep.is_out())
    }
}
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
/// # Errors
/// [`Error::BadDescriptor`] as [`total_length`].
pub fn num_interfaces(block: &[u8]) -> Result<u8> {
    total_length(block)?;
    Ok(block[4])
}
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
/// # Errors
/// [`Error::BadDescriptor`] when the block is invalid or does not contain the specified interface.
/// The supplied instance and descriptor block must agree.
pub fn interface(block: &[u8], iface: u8, alt: u8) -> Result<Interface> {
    walk(block)?
        .into_iter()
        .find(|i| i.number == iface && i.alternate == alt)
        .ok_or(Error::BadDescriptor)
}
/// # Errors
/// As [`interface`].
pub fn endpoints(block: &[u8], iface: u8, alt: u8) -> Result<Vec<Endpoint>> {
    Ok(interface(block, iface, alt)?.endpoints)
}
/// # Errors
/// As [`interface`].
pub fn interface_class(block: &[u8], iface: u8, alt: u8) -> Result<(u8, u8, u8)> {
    Ok(interface(block, iface, alt)?.class_triple())
}
#[must_use]
pub fn is_vendor_class(triple: (u8, u8, u8)) -> bool {
    triple == VENDOR_CLASS_TRIPLE
}
/// # Errors
/// [`Error::NoEndpoint`] when the interface has no bulk OUT endpoint.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayInterface {
    pub config: u8,
    pub config_index: u8,
    pub iface: u8,
    pub alternate: u8,
    pub bulk_out: Endpoint,
}
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
#[must_use]
pub fn config_to_select(num_interfaces: u8, instance_config: u32) -> Option<u8> {
    if num_interfaces == 1 || instance_config == 0 {
        let cfg = u8::try_from(instance_config).unwrap_or(0);
        Some(if cfg == 0 { 1 } else { cfg })
    } else {
        None
    }
}
pub const USB_DESC_HID: u8 = 0x21;
pub const USB_DESC_HID_REPORT: u8 = 0x22;
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
#[must_use]
pub fn langid(desc0: &[u8]) -> Option<u16> {
    if desc0.len() < 4 || desc0[0] < 4 || desc0[1] != USB_DESC_STRING {
        return None;
    }
    Some(u16::from_le_bytes([desc0[2], desc0[3]]))
}
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
    Some(text.trim_end_matches('\0').to_owned())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hid_report_length_comes_from_the_asked_interface_only() {
        let block = [
            9, 2, 52, 0, 2, 1, 0, 0xa0, 22, 9, 4, 0, 0, 1, 3, 0, 0, 0, 9, 0x21, 0x10, 0x01, 0, 1,
            0x22, 0x41, 0x00, 7, 5, 0x81, 3, 8, 0, 10, 9, 4, 1, 0, 1, 3, 0, 0, 0, 9, 0x21, 0x10,
            0x01, 0, 1, 0x22, 0x02, 0x01,
        ];
        assert_eq!(hid_report_len(&block, 0), Some(65));
        assert_eq!(hid_report_len(&block, 1), Some(258));
        assert_eq!(hid_report_len(&block, 2), None);
        assert_eq!(hid_report_len(&block[..20], 0), None);
    }
    const DL1XX: [u8; 44] = [
        9, 2, 44, 0, 1, 1, 0, 0x80, 0xFA, 9, 4, 0, 0, 2, 0xFF, 0x00, 0x00, 0, 12, 0x5F, 0x01, 0x00,
        10, 0x00, 0x02, 0x04, 0x00, 0x00, 0x20, 0x00, 7, 5, 0x01, 0x02, 0x00, 0x02, 0, 7, 5, 0x81,
        0x02, 0x00, 0x02, 0,
    ];
    const TWO_EPS: [u8; 32] = [
        9, 2, 32, 0, 1, 1, 0, 0x80, 0x32, 9, 4, 0, 0, 2, 0xFF, 0x00, 0x00, 0, 7, 5, 0x01, 0x02,
        0x00, 0x02, 0, 7, 5, 0x82, 0x02, 0x00, 0x02, 0,
    ];
    const COMPOSITE: [u8; 48] = [
        9, 2, 48, 0, 2, 1, 0, 0x80, 0xFA, 9, 4, 0, 0, 1, 0xFF, 0x00, 0x00, 0, 7, 5, 0x01, 0x02,
        0x00, 0x02, 0, 9, 4, 1, 0, 2, 0x08, 0x06, 0x50, 0, 7, 5, 0x81, 0x02, 0x00, 0x02, 0, 7, 5,
        0x02, 0x02, 0x00, 0x02, 0,
    ];
    const BENCH_CFG_MSC: [u8; 32] = [
        9, 2, 32, 0, 1, 2, 0, 0xC0, 0xFA, 9, 4, 0, 0, 2, 0x08, 0x06, 0x50, 0, 7, 5, 0x03, 0x02,
        0x00, 0x02, 0, 7, 5, 0x84, 0x02, 0x00, 0x02, 0,
    ];
    const BENCH_CFG_DISPLAY: [u8; 32] = [
        9, 2, 32, 0, 1, 1, 0, 0xC0, 0xFA, 9, 4, 0, 0, 2, 0xFF, 0x00, 0x00, 0, 7, 5, 0x01, 0x02,
        0x00, 0x02, 0, 7, 5, 0x82, 0x03, 0x08, 0x00, 1,
    ];
    #[test]
    fn the_display_interface_is_found_in_the_second_configuration() {
        let blocks = vec![BENCH_CFG_MSC.to_vec(), BENCH_CFG_DISPLAY.to_vec()];
        let d = find_display_interface(&blocks).expect("display interface");
        assert_eq!(d.config, 1);
        assert_eq!(d.config_index, 1);
        assert_eq!((d.iface, d.alternate), (0, 0));
        assert_eq!(d.bulk_out.address(), 0x01);
        assert_eq!(d.bulk_out.max_packet(), 512);
        assert_eq!(find_display_interface(&blocks[..1]), None);
        let one = find_display_interface(&[DL1XX.to_vec()]).unwrap();
        assert_eq!((one.config, one.config_index), (1, 0));
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
        assert_eq!(find_bulk_out(&COMPOSITE, 1, 0).unwrap().address(), 0x02);
        assert_eq!(config_to_select(2, 1), None);
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
        let mut short = DL1XX;
        short[2] = 0xFF;
        short[3] = 0x03;
        assert_eq!(total_length(&short).unwrap(), 44);
        assert_eq!(interface(&DL1XX, 1, 0), Err(Error::BadDescriptor));
        assert_eq!(interface(&DL1XX, 0, 1), Err(Error::BadDescriptor));
        let mut no_out = TWO_EPS;
        no_out[20] = 0x81;
        assert_eq!(find_bulk_out(&no_out, 0, 0), Err(Error::NoEndpoint));
        let mut stuck = TWO_EPS;
        stuck[18] = 0;
        assert_eq!(endpoints(&stuck, 0, 0).unwrap().len(), 0);
    }
    #[test]
    fn strings_decode_as_ascii_with_question_marks() {
        assert_eq!(langid(&[6, 3, 0x09, 0x04, 0x07, 0x04]), Some(0x0409));
        assert_eq!(langid(&[2, 3]), None);
        assert_eq!(langid(&[4, 2, 0x09, 0x04]), None);
        assert_eq!(langid(&[2, 3, 0x09, 0x04]), None);
        assert_eq!(langid(&[3, 3, 0x09, 0x04]), None);
        assert_eq!(langid(&[4, 3, 0x09, 0x04]), Some(0x0409));
        let desc = [10, 3, b'A', 0, b'B', 0, 0xE4, 0x00, b'1', 0, 0xFF, 0xFF];
        assert_eq!(decode_string(&desc).as_deref(), Some("AB?1"));
        assert_eq!(decode_string(&[12, 3, b'A', 0]), None);
        assert_eq!(decode_string(&[4, 4, b'A', 0]), None);
        assert_eq!(decode_string(&[2, 3]).as_deref(), Some(""));
        assert_eq!(decode_string(&[5, 3, b'Z', 0, b'Q']).as_deref(), Some("Z"));
        let padded = [
            14, 3, b'2', 0, b'3', 0, b'2', 0, b'8', 0, b'5', 0, b'8', 0, 0, 0, 0, 0,
        ];
        assert_eq!(decode_string(&padded[..14]).as_deref(), Some("232858"));
        let mut long = vec![18, 3];
        long.extend_from_slice(&padded[2..]);
        assert_eq!(decode_string(&long).as_deref(), Some("232858"));
    }
    #[test]
    fn endpoint_is_an_owned_aligned_c_struct() {
        let ep = Endpoint::new(7, 0x01, 0x02, 512, 0);
        let raw = ep.raw();
        assert_eq!(core::mem::size_of_val(&raw), 8);
        assert_eq!(core::mem::align_of_val(&raw), 2);
        assert_eq!(raw.w_max_packet_size, 512);
        assert_eq!(raw.b_interval, 0);
    }
}
