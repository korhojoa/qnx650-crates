// SPDX-License-Identifier: Apache-2.0
//! Checked physical address and length types.
//!
//! Distinct types rather than interchangeable integers, because defects of
//! exactly that kind were found in the C drivers these replace:
//! `disp_freemem(surf->paddr, surf->stride)` passed a physical address
//! where a pointer belonged and a stride where a length belonged, and a
//! page-rounded length wrapped and produced a mapping window shorter than
//! the transfer it was for. Here an address and a length cannot be
//! swapped at a call site, and rounding refuses rather than wraps.
//!
//! Bus addresses (a physical address seen through a GPU's or a DMA
//! engine's alias) are board-specific and live in the board crate.

use core::fmt;

/// A length in bytes, kept separate from an address so the two cannot be
/// swapped at a call site.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Len(pub u32);

/// A physical address (32 bits: the QNX 6.5 ARM and x86 targets).
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysAddr(u32);

impl PhysAddr {
    /// The physical address `a`.
    #[must_use]
    pub const fn new(a: u32) -> Self {
        PhysAddr(a)
    }

    /// The address as a number.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Offset by `n` bytes, `None` on overflow.
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

/// System page size on these targets.
pub const PAGE_SIZE: u32 = 4096;
const PAGE_MASK: u32 = PAGE_SIZE - 1;

/// The offset of `addr` within its page, and the page-rounded window
/// length needed to cover `len` bytes from it.
///
/// Returns `None` rather than wrapping. The C original was
/// `(len + off + 0xfff) & ~0xfff`, which for a large `len` produces a
/// window shorter than the transfer: mapping less than you then read or
/// write.
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
        // The case the C version got wrong.
        assert_eq!(
            map_extent(PhysAddr::new(0x1800), Len(u32::MAX - 0x400)),
            None
        );
        assert_eq!(map_extent(PhysAddr::new(0x1000), Len(u32::MAX)), None);
        // The largest length that still rounds safely.
        assert!(map_extent(PhysAddr::new(0x1000), Len(u32::MAX - PAGE_MASK)).is_some());
    }

    #[test]
    fn phys_offset_is_checked() {
        assert_eq!(PhysAddr::new(0x1000).offset(0x100).unwrap().raw(), 0x1100);
        assert!(PhysAddr::new(u32::MAX).offset(1).is_none());
    }
}
