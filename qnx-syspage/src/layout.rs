// SPDX-License-Identifier: Apache-2.0
//! Offsets and sizes of `sys/syspage.h` (SDP 6.5.0), measured with the
//! SDP compiler for ARM (`LAYOUT.md`). All fields are fixed-width
//! (`_Uint16t`, `_Uint32t`, `_Uint64t`); the one pointer, `asinfo_entry`'s
//! `alloc_checker`, is 32 bits on both QNX 6.5 targets.
pub const INFO_SIZE: usize = 4;
pub const INFO_ENTRY_OFF: usize = 0;
pub const INFO_ENTRY_SIZE: usize = 2;
pub const SYSPAGE_TOTAL_SIZE: usize = 2;
pub const SYSPAGE_NUM_CPU: usize = 6;
pub const SYSPAGE_ASINFO: usize = 12;
pub const SYSPAGE_CPUINFO: usize = 24;
pub const SYSPAGE_STRINGS: usize = 48;
pub const SYSPAGE_HEADER: usize = SYSPAGE_STRINGS + INFO_SIZE;
pub const CPUINFO_ENTRY_SIZE: usize = 32;
pub const ASINFO_ENTRY_SIZE: usize = 32;
pub const AS_NULL_OFF: u16 = 0xFFFF;
pub struct CpuinfoEntry;
impl CpuinfoEntry {
    pub const CPU: usize = 0;
    pub const SPEED: usize = 4;
    pub const FLAGS: usize = 8;
    pub const NAME: usize = 28;
    pub const INS_CACHE: usize = 30;
    pub const DATA_CACHE: usize = 31;
}
pub struct AsinfoEntry;
impl AsinfoEntry {
    pub const START: usize = 0;
    pub const END: usize = 8;
    pub const OWNER: usize = 16;
    pub const NAME: usize = 18;
    pub const ATTR: usize = 20;
    pub const PRIORITY: usize = 22;
    pub const ALLOC_CHECKER: usize = 24;
}
pub const SYSPAGE_CPUINFO_ENTRY_OFF: usize = SYSPAGE_CPUINFO + INFO_ENTRY_OFF;
pub const CPUINFO_CPU: usize = CpuinfoEntry::CPU;
const _: () = assert!(CpuinfoEntry::DATA_CACHE + 1 == CPUINFO_ENTRY_SIZE);
const _: () = assert!(AsinfoEntry::ALLOC_CHECKER + 8 == ASINFO_ENTRY_SIZE);
