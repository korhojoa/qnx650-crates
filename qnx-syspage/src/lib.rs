// SPDX-License-Identifier: Apache-2.0
//! The QNX 6.5 system page, read safely.
#![no_std]
pub mod layout;
use layout::{
    AsinfoEntry, CpuinfoEntry, ASINFO_ENTRY_SIZE, CPUINFO_ENTRY_SIZE, INFO_ENTRY_OFF,
    INFO_ENTRY_SIZE, SYSPAGE_ASINFO, SYSPAGE_CPUINFO, SYSPAGE_HEADER, SYSPAGE_NUM_CPU,
    SYSPAGE_STRINGS,
};
#[cfg(target_os = "nto")]
extern "C" {
    static _syspage_ptr: *const u8;
}
#[derive(Clone, Copy, Debug)]
pub struct Syspage<'a> {
    bytes: &'a [u8],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuInfo<'a> {
    pub cpu: u32,
    pub speed_mhz: u32,
    pub flags: u32,
    pub name: Option<&'a [u8]>,
    pub ins_cache: u8,
    pub data_cache: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AsInfo<'a> {
    pub start: u64,
    pub end: u64,
    pub owner: u16,
    pub name: Option<&'a [u8]>,
    pub attr: u16,
    pub priority: u16,
}
impl AsInfo<'_> {
    #[must_use]
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start).saturating_add(1)
    }
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
    #[cfg(target_os = "nto")]
    #[must_use]
    pub fn get() -> Option<Syspage<'static>> {
        // SAFETY: the kernel maps the system page read-only before `main`
        // and never moves or unmaps it. its first `total_size` bytes are
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
    #[cfg(not(target_os = "nto"))]
    #[must_use]
    pub fn get() -> Option<Syspage<'static>> {
        None
    }
}
impl<'a> Syspage<'a> {
    #[must_use]
    pub fn from_bytes(bytes: &'a [u8]) -> Option<Syspage<'a>> {
        (bytes.len() >= SYSPAGE_HEADER).then_some(Syspage { bytes })
    }
    #[must_use]
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
    #[must_use]
    pub fn num_cpu(&self) -> u32 {
        u16_at(self.bytes, SYSPAGE_NUM_CPU).map_or(0, u32::from)
    }
    fn section(&self, info: usize) -> Option<&'a [u8]> {
        let off = usize::from(u16_at(self.bytes, info + INFO_ENTRY_OFF)?);
        let size = usize::from(u16_at(self.bytes, info + INFO_ENTRY_SIZE)?);
        if off == 0 {
            return None;
        }
        self.bytes.get(off..off.checked_add(size)?)
    }
    #[must_use]
    pub fn string(&self, off: u16) -> Option<&'a [u8]> {
        let strings = self.section(SYSPAGE_STRINGS)?;
        let tail = strings.get(usize::from(off)..)?;
        let end = tail.iter().position(|&b| b == 0)?;
        Some(&tail[..end])
    }
    #[must_use]
    pub fn cpuinfo_count(&self) -> usize {
        self.section(SYSPAGE_CPUINFO)
            .map_or(0, |s| s.len() / CPUINFO_ENTRY_SIZE)
    }
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
    pub fn cpus(&self) -> impl Iterator<Item = CpuInfo<'a>> + '_ {
        (0..self.cpuinfo_count()).filter_map(|i| self.cpu(i))
    }
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
    #[must_use]
    pub fn total_ram_bytes(&self) -> Option<u64> {
        let total = self
            .asinfo()
            .filter(|a| a.name.is_some_and(|n| n.eq_ignore_ascii_case(b"ram")))
            .fold(0u64, |sum, a| sum.saturating_add(a.len()));
        (total > 0).then_some(total)
    }
}
#[must_use]
pub fn num_cpu() -> u32 {
    Syspage::get().map_or(0, |s| s.num_cpu())
}
#[must_use]
pub fn cpu0_id() -> Option<u32> {
    Syspage::get()?.cpu(0).map(|c| c.cpu)
}
#[cfg(test)]
mod tests {
    use super::*;
    use layout::SYSPAGE_TOTAL_SIZE;
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
        let strings = b"\0ram\0Cortex A7\0device\0";
        p[200..200 + strings.len()].copy_from_slice(strings);
        put16(&mut p, SYSPAGE_STRINGS, 200);
        put16(
            &mut p,
            SYSPAGE_STRINGS + 2,
            u16::try_from(strings.len()).unwrap(),
        );
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
        bytes[SYSPAGE_STRINGS + 2..SYSPAGE_STRINGS + 4].copy_from_slice(&400u16.to_ne_bytes());
        let sp = Syspage::from_bytes(&bytes).unwrap();
        assert_eq!(sp.cpu(0).unwrap().name, None);
        assert_eq!(sp.total_ram_bytes(), None);
        bytes[SYSPAGE_CPUINFO..SYSPAGE_CPUINFO + 2].copy_from_slice(&0u16.to_ne_bytes());
        let sp = Syspage::from_bytes(&bytes).unwrap();
        assert_eq!(sp.cpuinfo_count(), 0);
        assert!(sp.cpu(0).is_none());
        assert!(Syspage::from_bytes(&bytes[..SYSPAGE_HEADER - 1]).is_none());
    }
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
