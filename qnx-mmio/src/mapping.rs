// SPDX-License-Identifier: Apache-2.0
//! Device mappings with RAII, and volatile MMIO access.
use crate::addr::{map_extent, Len, PhysAddr, PAGE_SIZE};
use crate::{
    mmap_device_memory, munmap_device_memory, Error, Result, PROT_NOCACHE, PROT_READ, PROT_WRITE,
};
pub struct DeviceMapping {
    base: *mut u32,
    len: usize,
    off: u32,
}
impl DeviceMapping {
    /// # Errors
    /// The function fails for an empty extent or extent arithmetic overflow.
    /// It also fails if `mmap_device_memory` fails.
    pub fn new(phys: PhysAddr, len: Len, writable: bool) -> Result<DeviceMapping> {
        let (off, window) = map_extent(phys, len)
            .ok_or_else(|| Error::new("mapping extent overflows or is empty", 0))?;
        let prot = PROT_READ | PROT_NOCACHE | if writable { PROT_WRITE } else { 0 };
        let page = phys.raw() & !(PAGE_SIZE - 1);
        // SAFETY: A null hint makes `mmap_device_memory` allocate a new window.
        // The calculated `window` length has page alignment.
        // The kernel maps at least `off + len` bytes from `page`.
        let p = unsafe {
            mmap_device_memory(
                core::ptr::null_mut(),
                window as usize,
                prot,
                0,
                u64::from(page),
            )
        };
        if p.is_null() || p.addr() == usize::MAX {
            return Err(Error::last("mmap_device_memory"));
        }
        Ok(DeviceMapping {
            base: p.cast::<u32>(),
            len: window as usize,
            off,
        })
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.len - self.off as usize
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn word_ptr(&self, byte_off: u32) -> Option<*mut u32> {
        if byte_off & 3 != 0 {
            return None;
        }
        let abs = self.off.checked_add(byte_off)? as usize;
        if abs.checked_add(4)? > self.len {
            return None;
        }
        // SAFETY: `abs` is inside the mapped window.
        // `mmap_device_memory` returns a page-aligned base address.
        // The offset must be a multiple of 4.
        // Byte arithmetic applies the page offset in bytes.
        #[allow(clippy::cast_ptr_alignment)]
        Some(unsafe { self.base.cast::<u8>().add(abs).cast::<u32>() })
    }
    #[must_use]
    pub fn read(&self, byte_off: u32) -> Option<u32> {
        let p = self.word_ptr(byte_off)?;
        // SAFETY: p is in-window and 4-byte aligned.
        Some(unsafe { core::ptr::read_volatile(p) })
    }
    pub fn write(&mut self, byte_off: u32, val: u32) -> bool {
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
        // SAFETY: base/len are exactly what the successful map returned.
        unsafe {
            munmap_device_memory(self.base.cast::<core::ffi::c_void>(), self.len);
        }
    }
}
// SAFETY: The mapping owns its window and can move between threads.
// The mapping is not `Sync`.
// The caller must supply the ordering for concurrent register access.
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
    fn register_access_is_bounded_and_aligned() {
        let mut words = [0u32; 2048];
        let mut m = DeviceMapping {
            base: words.as_mut_ptr(),
            len: 8192,
            off: 8,
        };
        assert_eq!(m.len(), 8184);
        assert!(m.write(0, 0xdead_beef));
        assert_eq!(m.read(0), Some(0xdead_beef));
        assert_eq!(m.read(2), None, "misaligned");
        assert_eq!(m.read(8180), Some(0), "the last whole word");
        assert_eq!(m.read(8184), None, "past the window");
        assert!(!m.write(8184, 1));
        core::mem::forget(m);
        assert_eq!(words[2], 0xdead_beef);
    }
}
