//! Copyright 2026. Licensed under the Apache License, Version 2.0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ControlSetup {
    pub dir_in: bool,
    pub request: u8,
    pub rtype: u8,
    pub value: u16,
    pub index: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ControlDone {
    pub status: u32,
    pub alen: u32,
    pub elapsed_ms: u64,
}
#[cfg(target_os = "nto")]
mod nto;
#[cfg(target_os = "nto")]
pub(crate) use nto::{urb_status, DeviceSys, HostSys, PipeSys, SlotSys};
#[cfg(not(target_os = "nto"))]
mod host;
#[cfg(not(target_os = "nto"))]
pub(crate) use host::{urb_status, DeviceSys, HostSys, PipeSys, SlotSys};
