// SPDX-License-Identifier: Apache-2.0
//! The QNX 6.5 system page, read safely.
//!
//! The kernel maps the system page read-only into every process before
//! `main` and never moves it; libc.so.3 exports its address as the data
//! symbol `_syspage_ptr`. [`Syspage::get`] turns that into a byte slice of
//! the page's own `total_size`, and every reader below is a bounds-checked
//! read from that slice: a section offset or a string offset that points
//! outside the page gives `None`, never a read past it. The layout
//! (`sys/syspage.h`) is in [`layout`] and `LAYOUT.md`, measured with the
//! SDP compiler.
//!
//! What it reads:
//!
//! * [`Syspage::num_cpu`]: the number of CPUs the kernel runs on.
//! * [`Syspage::cpu`] / [`Syspage::cpus`]: the `cpuinfo` entries: the CPU
//!   id (`cpu`, the MIDR on ARM), the speed in MHz, the flags, the model
//!   name (`pidin info` prints the same: `Cortex A9 MPCore 900MHz`).
//! * [`Syspage::asinfo`]: the address-space entries (`ram`, `sysram`,
//!   `device` ...), and [`Syspage::total_ram_bytes`], the sum of the
//!   `ram` ranges.
//!
//! Host builds have no system page: [`Syspage::get`] is `None` there, and
//! tests build a page in memory with [`Syspage::from_bytes`].
//!
//! Reads CPU identification and memory ranges from the system page.

#![no_std]

pub mod layout;

use layout::{
    AsinfoEntry, CpuinfoEntry, ASINFO_ENTRY_SIZE, CPUINFO_ENTRY_SIZE, INFO_ENTRY_OFF,
    INFO_ENTRY_SIZE, SYSPAGE_ASINFO, SYSPAGE_CPUINFO, SYSPAGE_HEADER, SYSPAGE_NUM_CPU,
    SYSPAGE_STRINGS,
};

#[cfg(target_os = "nto")]
extern "C" {
    /// The process-global, read-only system page (libc.so.3 data symbol).
    static _syspage_ptr: *const u8;
}

/// The system page as a byte slice.
#[derive(Clone, Copy, Debug)]
pub struct Syspage<'a> {
    bytes: &'a [u8],
}

/// One `cpuinfo` entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuInfo<'a> {
    /// `cpu`: the CPU id; on ARM the kernel stores the MIDR here.
    pub cpu: u32,
    /// `speed`: the clock in MHz.
    pub speed_mhz: u32,
    /// `flags`: `CPU_FLAG_*`.
    pub flags: u32,
    /// `name`: the model name from the strings section, if it has one.
    pub name: Option<&'a [u8]>,
    /// `ins_cache`: index of the first instruction cache entry.
    pub ins_cache: u8,
    /// `data_cache`: index of the first data cache entry.
    pub data_cache: u8,
}

/// One `asinfo` entry: a range of the physical address space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AsInfo<'a> {
    /// First address of the range.
    pub start: u64,
    /// Last address of the range (inclusive).
    pub end: u64,
    /// Offset of the enclosing entry, or [`layout::AS_NULL_OFF`].
    pub owner: u16,
    /// The range's name from the strings section (`ram`, `sysram` ...).
    pub name: Option<&'a [u8]>,
    /// `AS_ATTR_*`.
    pub attr: u16,
    /// `AS_PRIORITY_*`.
    pub priority: u16,
}

impl AsInfo<'_> {
    /// Bytes in the range (`end - start + 1`, saturating).
    #[must_use]
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start).saturating_add(1)
    }

    /// True for an inverted range (`end < start`), which holds no byte.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.end < self.start
    }
}

fn u16_at(bytes: &[u8], off: usize) -> Option<u16> {
    let b = bytes.get(off..off.checked_add(2)?)?;
    Some(u16::from_ne_bytes([b[0], b[1]]))
}

fn u32_at(bytes: &[u8], off: usize) -> Option<u32> {
    let b = bytes.get(off..off.checked_add(4)?)?;
    Some(u32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
}

fn u64_at(bytes: &[u8], off: usize) -> Option<u64> {
    let lo = u64::from(u32_at(bytes, off)?);
    let hi = u64::from(u32_at(bytes, off.checked_add(4)?)?);
    Some(if cfg!(target_endian = "little") {
        lo | hi << 32
    } else {
        hi | lo << 32
    })
}

impl Syspage<'static> {
    /// The process's system page, or `None` when there is none (a null
    /// `_syspage_ptr`, or a host build).
    #[cfg(target_os = "nto")]
    #[must_use]
    pub fn get() -> Option<Syspage<'static>> {
        // SAFETY: the kernel maps the system page read-only before `main`
        // and never moves or unmaps it; its first `total_size` bytes are
        // the page. `read_unaligned` makes no alignment assumption.
        unsafe {
            let base = _syspage_ptr;
            if base.is_null() {
                return None;
            }
            let total = base
                .add(layout::SYSPAGE_TOTAL_SIZE)
                .cast::<u16>()
                .read_unaligned();
            Syspage::from_bytes(core::slice::from_raw_parts(base, usize::from(total)))
        }
    }

    /// Host builds have no system page.
    #[cfg(not(target_os = "nto"))]
    #[must_use]
    pub fn get() -> Option<Syspage<'static>> {
        None
    }
}

impl<'a> Syspage<'a> {
    /// A system page held in `bytes` (the page, `total_size` long). `None`
    /// when `bytes` is shorter than the page's fixed header.
    #[must_use]
    pub fn from_bytes(bytes: &'a [u8]) -> Option<Syspage<'a>> {
        (bytes.len() >= SYSPAGE_HEADER).then_some(Syspage { bytes })
    }

    /// The page's bytes.
    #[must_use]
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// `num_cpu`: the number of CPUs the kernel runs on.
    #[must_use]
    pub fn num_cpu(&self) -> u32 {
        u16_at(self.bytes, SYSPAGE_NUM_CPU).map_or(0, u32::from)
    }

    /// The bytes of the section whose `syspage_entry_info` is at `info`,
    /// or `None` when it is absent (offset 0) or outside the page.
    fn section(&self, info: usize) -> Option<&'a [u8]> {
        let off = usize::from(u16_at(self.bytes, info + INFO_ENTRY_OFF)?);
        let size = usize::from(u16_at(self.bytes, info + INFO_ENTRY_SIZE)?);
        if off == 0 {
            return None;
        }
        self.bytes.get(off..off.checked_add(size)?)
    }

    /// The NUL-terminated string at `off` in the strings section, without
    /// the NUL. `None` when the section is absent or the string does not
    /// end inside it.
    #[must_use]
    pub fn string(&self, off: u16) -> Option<&'a [u8]> {
        let strings = self.section(SYSPAGE_STRINGS)?;
        let tail = strings.get(usize::from(off)..)?;
        let end = tail.iter().position(|&b| b == 0)?;
        Some(&tail[..end])
    }

    /// The number of `cpuinfo` entries.
    #[must_use]
    pub fn cpuinfo_count(&self) -> usize {
        self.section(SYSPAGE_CPUINFO)
            .map_or(0, |s| s.len() / CPUINFO_ENTRY_SIZE)
    }

    /// The `cpuinfo` entry of CPU `index`.
    #[must_use]
    pub fn cpu(&self, index: usize) -> Option<CpuInfo<'a>> {
        let section = self.section(SYSPAGE_CPUINFO)?;
        let at = index.checked_mul(CPUINFO_ENTRY_SIZE)?;
        let e = section.get(at..at.checked_add(CPUINFO_ENTRY_SIZE)?)?;
        let name_off = u16_at(e, CpuinfoEntry::NAME)?;
        Some(CpuInfo {
            cpu: u32_at(e, CpuinfoEntry::CPU)?,
            speed_mhz: u32_at(e, CpuinfoEntry::SPEED)?,
            flags: u32_at(e, CpuinfoEntry::FLAGS)?,
            name: self.string(name_off).filter(|n| !n.is_empty()),
            ins_cache: e[CpuinfoEntry::INS_CACHE],
            data_cache: e[CpuinfoEntry::DATA_CACHE],
        })
    }

    /// Every `cpuinfo` entry, in CPU order.
    pub fn cpus(&self) -> impl Iterator<Item = CpuInfo<'a>> + '_ {
        (0..self.cpuinfo_count()).filter_map(|i| self.cpu(i))
    }

    /// Every `asinfo` entry, in page order.
    pub fn asinfo(&self) -> impl Iterator<Item = AsInfo<'a>> + '_ {
        let section = self.section(SYSPAGE_ASINFO).unwrap_or(&[]);
        let (entries, _) = section.as_chunks::<ASINFO_ENTRY_SIZE>();
        entries.iter().filter_map(|e| {
            Some(AsInfo {
                start: u64_at(e, AsinfoEntry::START)?,
                end: u64_at(e, AsinfoEntry::END)?,
                owner: u16_at(e, AsinfoEntry::OWNER)?,
                name: self.string(u16_at(e, AsinfoEntry::NAME)?),
                attr: u16_at(e, AsinfoEntry::ATTR)?,
                priority: u16_at(e, AsinfoEntry::PRIORITY)?,
            })
        })
    }

    /// The system RAM in bytes: the sum of the `asinfo` ranges named `ram`
    /// (any case), or `None` when there are none.
    #[must_use]
    pub fn total_ram_bytes(&self) -> Option<u64> {
        let total = self
            .asinfo()
            .filter(|a| a.name.is_some_and(|n| n.eq_ignore_ascii_case(b"ram")))
            .fold(0u64, |sum, a| sum.saturating_add(a.len()));
        (total > 0).then_some(total)
    }
}

/// The number of CPUs the kernel runs on, or 0 without a system page.
#[must_use]
pub fn num_cpu() -> u32 {
    Syspage::get().map_or(0, |s| s.num_cpu())
}

/// The CPU id of CPU 0 (the MIDR on ARM), or `None` without a system page
/// or a `cpuinfo` entry.
#[must_use]
pub fn cpu0_id() -> Option<u32> {
    Syspage::get()?.cpu(0).map(|c| c.cpu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use layout::SYSPAGE_TOTAL_SIZE;

    /// A page laid out like the kernel's: header, then strings, cpuinfo
    /// and asinfo sections.
    fn page() -> [u8; 512] {
        let mut p = [0u8; 512];
        let put16 = |p: &mut [u8; 512], off: usize, v: u16| {
            p[off..off + 2].copy_from_slice(&v.to_ne_bytes());
        };
        let put32 = |p: &mut [u8; 512], off: usize, v: u32| {
            p[off..off + 4].copy_from_slice(&v.to_ne_bytes());
        };
        let put64 = |p: &mut [u8; 512], off: usize, v: u64| {
            p[off..off + 8].copy_from_slice(&v.to_ne_bytes());
        };
        put16(&mut p, SYSPAGE_TOTAL_SIZE, 512);
        put16(&mut p, SYSPAGE_NUM_CPU, 2);
        // strings at 200: "\0ram\0Cortex A7\0device\0"
        let strings = b"\0ram\0Cortex A7\0device\0";
        p[200..200 + strings.len()].copy_from_slice(strings);
        put16(&mut p, SYSPAGE_STRINGS, 200);
        put16(
            &mut p,
            SYSPAGE_STRINGS + 2,
            u16::try_from(strings.len()).unwrap(),
        );
        // cpuinfo at 256: two entries
        put16(&mut p, SYSPAGE_CPUINFO, 256);
        put16(&mut p, SYSPAGE_CPUINFO + 2, 64);
        for (i, midr) in [0x410F_C075u32, 0x410F_C075].into_iter().enumerate() {
            let e = 256 + i * 32;
            put32(&mut p, e, midr);
            put32(&mut p, e + 4, 900);
            put32(&mut p, e + 8, 0xC000_0000);
            put16(&mut p, e + 28, 5);
            p[e + 30] = 1;
            p[e + 31] = 2;
        }
        // asinfo at 320: ram 0..0x0FFF_FFFF, device, RAM 0x2000_0000..+1 MB
        put16(&mut p, SYSPAGE_ASINFO, 320);
        put16(&mut p, SYSPAGE_ASINFO + 2, 96);
        for (i, (start, end, name)) in [
            (0u64, 0x0FFF_FFFFu64, 1u16),
            (0x3F00_0000, 0x3FFF_FFFF, 15),
            (0x2000_0000, 0x200F_FFFF, 1),
        ]
        .into_iter()
        .enumerate()
        {
            let e = 320 + i * 32;
            put64(&mut p, e, start);
            put64(&mut p, e + 8, end);
            put16(&mut p, e + 16, 0xFFFF);
            put16(&mut p, e + 18, name);
        }
        p
    }

    #[test]
    fn reads_the_cpus_the_ranges_and_the_strings() {
        let bytes = page();
        let sp = Syspage::from_bytes(&bytes).unwrap();
        assert_eq!(sp.num_cpu(), 2);
        assert_eq!(sp.cpuinfo_count(), 2);
        let c = sp.cpu(1).unwrap();
        assert_eq!(
            (c.cpu, c.speed_mhz, c.flags),
            (0x410F_C075, 900, 0xC000_0000)
        );
        assert_eq!(c.name, Some(&b"Cortex A7"[..]));
        assert_eq!((c.ins_cache, c.data_cache), (1, 2));
        assert!(sp.cpu(2).is_none());
        assert_eq!(sp.cpus().count(), 2);
        let ranges: [Option<&[u8]>; 3] = {
            let mut it = sp.asinfo().map(|a| a.name);
            [
                it.next().flatten(),
                it.next().flatten(),
                it.next().flatten(),
            ]
        };
        assert_eq!(
            ranges,
            [Some(&b"ram"[..]), Some(&b"device"[..]), Some(&b"ram"[..])]
        );
        assert_eq!(sp.total_ram_bytes(), Some(0x1000_0000 + 0x10_0000));
    }

    #[test]
    fn offsets_outside_the_page_read_as_absent() {
        let mut bytes = page();
        // The strings section runs past the page: no names.
        bytes[SYSPAGE_STRINGS + 2..SYSPAGE_STRINGS + 4].copy_from_slice(&400u16.to_ne_bytes());
        let sp = Syspage::from_bytes(&bytes).unwrap();
        assert_eq!(sp.cpu(0).unwrap().name, None);
        assert_eq!(sp.total_ram_bytes(), None);
        // No cpuinfo section at all.
        bytes[SYSPAGE_CPUINFO..SYSPAGE_CPUINFO + 2].copy_from_slice(&0u16.to_ne_bytes());
        let sp = Syspage::from_bytes(&bytes).unwrap();
        assert_eq!(sp.cpuinfo_count(), 0);
        assert!(sp.cpu(0).is_none());
        // Too short for the header.
        assert!(Syspage::from_bytes(&bytes[..SYSPAGE_HEADER - 1]).is_none());
    }

    /// On the unit: the kernel's page reads as at least one CPU with an id.
    #[cfg(target_os = "nto")]
    #[test]
    fn the_system_page_names_the_cpus() {
        let sp = Syspage::get().unwrap();
        assert!(sp.num_cpu() >= 1);
        assert!(sp.cpuinfo_count() >= 1);
        assert!(cpu0_id().is_some_and(|id| id != 0));
        assert!(sp.cpu(0).unwrap().speed_mhz > 0);
    }

    #[cfg(not(target_os = "nto"))]
    #[test]
    fn the_host_has_no_system_page() {
        assert!(Syspage::get().is_none());
        assert_eq!(num_cpu(), 0);
        assert_eq!(cpu0_id(), None);
    }
}
