// SPDX-License-Identifier: Apache-2.0
//! Checked physical address and length types.
use core::fmt;
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Len(pub u32);
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysAddr(u32);
impl PhysAddr {
    #[must_use]
    pub const fn new(a: u32) -> Self {
        PhysAddr(a)
    }
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
    #[must_use]
    pub fn offset(self, n: u32) -> Option<Self> {
        self.0.checked_add(n).map(PhysAddr)
    }
}
impl fmt::Debug for PhysAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "phys:{:#010x}", self.0)
    }
}
pub const PAGE_SIZE: u32 = 4096;
const PAGE_MASK: u32 = PAGE_SIZE - 1;
#[must_use]
pub fn map_extent(addr: PhysAddr, len: Len) -> Option<(u32, u32)> {
    if len.0 == 0 {
        return None;
    }
    let off = addr.raw() & PAGE_MASK;
    let need = off.checked_add(len.0)?;
    let rounded = need.checked_add(PAGE_MASK)? & !PAGE_MASK;
    Some((off, rounded))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn map_extent_ordinary() {
        assert_eq!(map_extent(PhysAddr::new(0x1000), Len(64)), Some((0, 4096)));
        assert_eq!(
            map_extent(PhysAddr::new(0x1FF0), Len(64)),
            Some((0xFF0, 8192))
        );
        assert_eq!(
            map_extent(PhysAddr::new(0x1000), Len(4096)),
            Some((0, 4096))
        );
        assert_eq!(
            map_extent(PhysAddr::new(0x1001), Len(4096)),
            Some((1, 8192))
        );
    }
    #[test]
    fn map_extent_refuses_rather_than_wrapping() {
        assert_eq!(map_extent(PhysAddr::new(0x1000), Len(0)), None);
        assert_eq!(
            map_extent(PhysAddr::new(0x1800), Len(u32::MAX - 0x400)),
            None
        );
        assert_eq!(map_extent(PhysAddr::new(0x1000), Len(u32::MAX)), None);
        assert!(map_extent(PhysAddr::new(0x1000), Len(u32::MAX - PAGE_MASK)).is_some());
    }
    #[test]
    fn phys_offset_is_checked() {
        assert_eq!(PhysAddr::new(0x1000).offset(0x100).unwrap().raw(), 0x1100);
        assert!(PhysAddr::new(u32::MAX).offset(1).is_none());
    }
}
