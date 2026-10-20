// SPDX-License-Identifier: Apache-2.0
//! Device mappings with RAII, and volatile MMIO access.
//!
//! C tools mapped a peripheral and then either forgot to unmap it on some
//! path, or unmapped it with the wrong length. A [`DeviceMapping`] unmaps
//! itself on drop, with the length it was created with.
//!
//! What that does and does not cover: the drop runs on every normal path
//! (early returns, `?`, the end of a scope) and on an unwind in the
//! configurations that unwind. It does not run after an abort, and a
//! `panic = "abort"` release profile kills the process instead; the kernel
//! then reclaims its mappings. So the guarantee is "no leak across a live
//! process's control flow", not "cleanup on any termination".
//!
//! Register access goes through [`DeviceMapping::read`] and
//! [`DeviceMapping::write`], which are volatile and bounds-checked against
//! the requested register range. No Rust reference into memory a device mutates is
//! ever created: the type hands out values, never `&` or `&mut` into the
//! window.

use crate::addr::{map_extent, Len, PhysAddr, PAGE_SIZE};
use crate::{
    mmap_device_memory, munmap_device_memory, Error, Result, PROT_NOCACHE, PROT_READ, PROT_WRITE,
};

/// A mapped peripheral window. Unmaps on drop.
pub struct DeviceMapping {
    base: *mut u32,
    mapped_len: usize,
    len: usize,
    writable: bool,
    /// Bytes from the mapped page start to the address asked for.
    off: u32,
}

impl DeviceMapping {
    /// Map `len` bytes of device memory at `phys`, uncached.
    ///
    /// `writable` is explicit rather than defaulted: a read-only tool
    /// asking for a read-only mapping prevents register writes.
    ///
    /// # Errors
    /// Fails if the physical address is unaligned, the requested extent
    /// overflows or is empty, or if
    /// `mmap_device_memory` fails (typically no I/O privilege, or an
    /// address the kernel will not map).
    pub fn new(phys: PhysAddr, len: Len, writable: bool) -> Result<DeviceMapping> {
        if phys.raw() & 3 != 0 {
            return Err(Error::new("physical address is not 4-byte aligned", 0));
        }
        let (off, window) = map_extent(phys, len)
            .ok_or_else(|| Error::new("mapping extent overflows or is empty", 0))?;

        let prot = PROT_READ | PROT_NOCACHE | if writable { PROT_WRITE } else { 0 };
        let page = phys.raw() & !(PAGE_SIZE - 1);

        // SAFETY: mmap_device_memory with a null hint allocates a fresh
        // window; `window` is the page-rounded length computed above, so
        // the kernel maps at least `off + len` bytes from `page`.
        let p = unsafe {
            mmap_device_memory(
                core::ptr::null_mut(),
                window as usize,
                prot,
                0,
                u64::from(page),
            )
        };
        // MAP_FAILED is (void *)-1.
        if p.is_null() || p.addr() == usize::MAX {
            return Err(Error::last("mmap_device_memory"));
        }
        Ok(DeviceMapping {
            base: p.cast::<u32>(),
            mapped_len: window as usize,
            len: len.0 as usize,
            writable,
            off,
        })
    }

    /// Bytes addressable from the requested address.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when no byte is addressable (never, for a successful map).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn word_ptr(&self, byte_off: u32) -> Option<*mut u32> {
        // Registers are 32-bit and naturally aligned.
        if (byte_off as usize).checked_add(4)? > self.len {
            return None;
        }
        let abs = self.off.checked_add(byte_off)? as usize;
        if abs & 3 != 0 {
            return None;
        }
        // SAFETY: `abs` is within the mapped window, checked above.
        //
        // cast_ptr_alignment: the *mut u8 -> *mut u32 cast is sound
        // because `base` came from mmap_device_memory, which returns a
        // page-aligned address, and `abs` is rejected above unless it is
        // a multiple of 4. Byte arithmetic is used rather than
        // `base.add(abs / 4)` so that `off`, the offset of the requested
        // address within its page, is applied in bytes, the unit it is
        // measured in.
        #[allow(clippy::cast_ptr_alignment)]
        Some(unsafe { self.base.cast::<u8>().add(abs).cast::<u32>() })
    }

    /// Read a 32-bit register at `byte_off`, or `None` if that is outside
    /// the window or misaligned.
    ///
    /// Volatile: the compiler must not merge, reorder or elide device
    /// reads.
    #[must_use]
    pub fn read(&self, byte_off: u32) -> Option<u32> {
        let p = self.word_ptr(byte_off)?;
        // SAFETY: p is in-window and 4-byte aligned.
        Some(unsafe { core::ptr::read_volatile(p) })
    }

    /// Write a 32-bit register. Returns `false` for a read-only mapping,
    /// an offset outside the requested range, or an unaligned offset.
    ///
    /// Takes `&mut self`: a mapping shared immutably cannot be written
    /// through, so "this tool only reads" is expressible in the type.
    pub fn write(&mut self, byte_off: u32, val: u32) -> bool {
        if !self.writable {
            return false;
        }
        match self.word_ptr(byte_off) {
            Some(p) => {
                // SAFETY: p is in-window and 4-byte aligned.
                unsafe { core::ptr::write_volatile(p, val) };
                true
            }
            None => false,
        }
    }
}

impl Drop for DeviceMapping {
    fn drop(&mut self) {
        // SAFETY: The base and mapped length identify the complete mapping.
        unsafe {
            munmap_device_memory(self.base.cast::<core::ffi::c_void>(), self.mapped_len);
        }
    }
}

// SAFETY: a mapping owns its window and may move between threads. It is
// not Sync: concurrent access to device registers needs the caller's own
// ordering, which this crate does not presume to supply.
unsafe impl Send for DeviceMapping {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_extent_is_refused_before_any_call() {
        let e = DeviceMapping::new(PhysAddr::new(0x3F00_0000), Len(0), false)
            .err()
            .unwrap();
        assert_eq!(e.what, "mapping extent overflows or is empty");
    }

    #[cfg(not(target_os = "nto"))]
    #[test]
    fn the_host_refuses_to_map() {
        let e = DeviceMapping::new(PhysAddr::new(0x3F00_0000), Len(4), false)
            .err()
            .unwrap();
        assert_eq!(e.what, "mmap_device_memory");
    }

    #[test]
    fn unaligned_physical_addresses_are_refused_before_mapping() {
        for address in [0x1001, 0x1002, 0x1003] {
            let error = DeviceMapping::new(PhysAddr::new(address), Len(4), true)
                .err()
                .unwrap();
            assert_eq!(error.what, "physical address is not 4-byte aligned");
        }
    }

    #[test]
    fn readonly_mapping_and_page_padding_are_not_writable() {
        let mut words = [0u32; 1024];
        let mut mapping = core::mem::ManuallyDrop::new(DeviceMapping {
            base: words.as_mut_ptr(),
            mapped_len: 4096,
            len: 4,
            off: 0,
            writable: false,
        });
        assert_eq!(mapping.len(), 4);
        assert_eq!(mapping.read(0), Some(0));
        assert!(!mapping.write(0, 1));
        assert_eq!(words[0], 0);
        assert_eq!(mapping.read(4), None);
        assert_eq!(mapping.read(4092), None);
        assert!(!mapping.write(4092, 1));
        assert_eq!(mapping.read(u32::MAX), None);
        mapping.len = 3;
        assert_eq!(mapping.read(0), None);
    }

    #[test]
    fn register_access_is_bounded_and_aligned() {
        // A window over ordinary memory stands in for the device: 2 pages
        // mapped, the requested address 8 bytes into the first.
        let mut words = [0u32; 2048];
        let mut m = DeviceMapping {
            base: words.as_mut_ptr(),
            mapped_len: 8192,
            len: 12,
            writable: true,
            off: 8,
        };
        assert_eq!(m.len(), 12);
        assert!(m.write(0, 0xdead_beef));
        assert_eq!(m.read(0), Some(0xdead_beef));
        assert_eq!(m.read(2), None, "misaligned");
        assert_eq!(m.read(8), Some(0), "the last whole word");
        assert_eq!(m.read(12), None, "past the window");
        assert!(!m.write(12, 1));
        core::mem::forget(m);
        assert_eq!(words[2], 0xdead_beef);
    }
}
