// SPDX-License-Identifier: Apache-2.0
//! Offsets and sizes of `sys/syspage.h` (SDP 6.5.0), measured with the
//! SDP compiler for ARM (`LAYOUT.md`). All fields are fixed-width
//! (`_Uint16t`, `_Uint32t`, `_Uint64t`); the one pointer, `asinfo_entry`'s
//! `alloc_checker`, is 32 bits on both QNX 6.5 targets.

/// `sizeof(syspage_entry_info)`: `{ _Uint16t entry_off, entry_size }`.
pub const INFO_SIZE: usize = 4;
/// `offsetof(syspage_entry_info, entry_off)`: offset of the section from
/// the start of the page.
pub const INFO_ENTRY_OFF: usize = 0;
/// `offsetof(syspage_entry_info, entry_size)`: size of the section.
pub const INFO_ENTRY_SIZE: usize = 2;

/// `offsetof(struct syspage_entry, total_size)`, a `_Uint16t`: the size
/// of the whole page.
pub const SYSPAGE_TOTAL_SIZE: usize = 2;
/// `offsetof(struct syspage_entry, num_cpu)`, a `_Uint16t`.
pub const SYSPAGE_NUM_CPU: usize = 6;
/// `offsetof(struct syspage_entry, asinfo)`, a `syspage_entry_info`.
pub const SYSPAGE_ASINFO: usize = 12;
/// `offsetof(struct syspage_entry, cpuinfo)`, a `syspage_entry_info`.
pub const SYSPAGE_CPUINFO: usize = 24;
/// `offsetof(struct syspage_entry, strings)`, a `syspage_entry_info`.
pub const SYSPAGE_STRINGS: usize = 48;
/// The bytes of the page header this crate reads (up to the end of
/// `strings`).
pub const SYSPAGE_HEADER: usize = SYSPAGE_STRINGS + INFO_SIZE;

/// `sizeof(struct cpuinfo_entry)`.
pub const CPUINFO_ENTRY_SIZE: usize = 32;
/// `sizeof(struct asinfo_entry)` on the 32-bit targets.
pub const ASINFO_ENTRY_SIZE: usize = 32;

/// `AS_NULL_OFF`: an `asinfo` entry without an owner.
pub const AS_NULL_OFF: u16 = 0xFFFF;

/// Field offsets of `struct cpuinfo_entry`.
pub struct CpuinfoEntry;

impl CpuinfoEntry {
    /// `cpu`, a `_Uint32t` (the MIDR on ARM).
    pub const CPU: usize = 0;
    /// `speed`, a `_Uint32t` (MHz).
    pub const SPEED: usize = 4;
    /// `flags`, a `_Uint32t`.
    pub const FLAGS: usize = 8;
    /// `name`, a `_Uint16t` offset into the strings section.
    pub const NAME: usize = 28;
    /// `ins_cache`, a `_Uint8t`.
    pub const INS_CACHE: usize = 30;
    /// `data_cache`, a `_Uint8t`.
    pub const DATA_CACHE: usize = 31;
}

/// Field offsets of `struct asinfo_entry`.
pub struct AsinfoEntry;

impl AsinfoEntry {
    /// `start`, a `_Uint64t`.
    pub const START: usize = 0;
    /// `end`, a `_Uint64t` (inclusive).
    pub const END: usize = 8;
    /// `owner`, a `_Uint16t`.
    pub const OWNER: usize = 16;
    /// `name`, a `_Uint16t` offset into the strings section.
    pub const NAME: usize = 18;
    /// `attr`, a `_Uint16t`.
    pub const ATTR: usize = 20;
    /// `priority`, a `_Uint16t`.
    pub const PRIORITY: usize = 22;
    /// `alloc_checker`, a function pointer (32 bits).
    pub const ALLOC_CHECKER: usize = 24;
}

/// `offsetof(struct syspage_entry, cpuinfo) + offsetof(syspage_entry_info,
/// entry_off)`: where the offset of the cpuinfo array is (the name
/// qnx-rpi used).
pub const SYSPAGE_CPUINFO_ENTRY_OFF: usize = SYSPAGE_CPUINFO + INFO_ENTRY_OFF;
/// `offsetof(struct cpuinfo_entry, cpu)` (the name qnx-rpi used).
pub const CPUINFO_CPU: usize = CpuinfoEntry::CPU;

const _: () = assert!(CpuinfoEntry::DATA_CACHE + 1 == CPUINFO_ENTRY_SIZE);
const _: () = assert!(AsinfoEntry::ALLOC_CHECKER + 8 == ASINFO_ENTRY_SIZE);
