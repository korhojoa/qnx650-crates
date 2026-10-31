// SPDX-License-Identifier: Apache-2.0
//! Test-only native substitutes for the public transfer methods.
//! Normal host builds still use the uninhabited types in host.rs.
// The substitutes must keep the native method signatures even when a
// scenario returns a fixed result or does not use the handle.
#![allow(clippy::unnecessary_wraps, clippy::unused_self)]
use super::{ControlDone, ControlSetup};
use crate::desc::Endpoint;
use crate::ffi::{
    UsbdDeviceDescriptor, UsbdDeviceInstance, UsbdIoCallback, UsbdUrb, EBUSY, EIO, USBD_STATUS_CMP,
    USBD_STATUS_INPROG,
};
use crate::host::Ident;
use crate::{Error, Result};
use core::ffi::c_void;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
pub(crate) enum Scenario {
    Input { refuse_second: bool },
    Output { send_error: bool },
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Snapshot {
    pub fills: usize,
    pub submissions: usize,
    pub resets: usize,
    pub aborts: usize,
    pub closes: usize,
    pub freed_slots: usize,
    pub bytes: Vec<u8>,
}
struct State {
    scenario: Scenario,
    allocated: usize,
    snapshot: Snapshot,
}
type Shared = Arc<Mutex<State>>;
thread_local! { static ACTIVE: RefCell<Option<Shared>> = const { RefCell::new(None) }; }
fn active() -> Result<Shared> {
    ACTIVE
        .with(|value| value.borrow().clone())
        .ok_or(Error::Unsupported)
}
pub(crate) struct Fixture {
    state: Shared,
    previous: Option<Shared>,
}
impl Fixture {
    pub(crate) fn new(scenario: Scenario) -> Self {
        let state = Arc::new(Mutex::new(State {
            scenario,
            allocated: 0,
            snapshot: Snapshot::default(),
        }));
        let previous = ACTIVE.with(|value| value.replace(Some(state.clone())));
        Self { state, previous }
    }
    pub(crate) fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap().snapshot.clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        ACTIVE.with(|value| value.replace(self.previous.take()));
    }
}

pub(crate) struct HostSys(Shared);
impl HostSys {
    pub(crate) fn connect(_: &str, _: Ident, _: u16) -> Result<Self> {
        Ok(Self(active()?))
    }
    pub(crate) fn attach(&self, _: &mut UsbdDeviceInstance) -> Result<DeviceSys> {
        Ok(DeviceSys(self.0.clone()))
    }
}
pub(crate) struct DeviceSys(Shared);
impl DeviceSys {
    pub(crate) fn reset(&self) -> Result<()> {
        Ok(())
    }
    pub(crate) fn device_descriptor(&self) -> UsbdDeviceDescriptor {
        UsbdDeviceDescriptor::default()
    }
    pub(crate) fn select_config(&self, _: u8) -> Result<()> {
        Ok(())
    }
    pub(crate) fn select_interface(&self, _: u8, _: u8) -> Result<()> {
        Ok(())
    }
    pub(crate) fn descriptor(&self, _: u8, _: u16, _: u8, _: u16, _: &mut [u8]) -> Result<()> {
        Err(Error::Unsupported)
    }
    pub(crate) fn control(&self, _: ControlSetup, _: &mut [u8], _: u32) -> Result<ControlDone> {
        Err(Error::Unsupported)
    }
    pub(crate) fn open_pipe(&self, _: Endpoint) -> Result<PipeSys> {
        Ok(PipeSys(self.0.clone()))
    }
}
pub(crate) struct PipeSys(Shared);
impl PipeSys {
    pub(crate) fn reset(&self) -> Result<()> {
        self.0.lock().unwrap().snapshot.resets += 1;
        Ok(())
    }
    pub(crate) fn abort(&self) -> Result<()> {
        let mut state = self.0.lock().unwrap();
        state.snapshot.aborts += 1;
        match state.scenario {
            Scenario::Input { .. } => Ok(()),
            Scenario::Output { .. } => Err(Error::Errno(EBUSY)),
        }
    }
    pub(crate) fn close(&self) -> Result<()> {
        self.0.lock().unwrap().snapshot.closes += 1;
        Ok(())
    }
    pub(crate) fn submit_bulk_out(
        &self,
        _: &SlotSys,
        _: u32,
        _: UsbdIoCallback,
        _: *mut c_void,
        _: u32,
    ) -> Result<()> {
        Err(Error::Unsupported)
    }
    fn submit_input(
        &self,
        slot: &SlotSys,
        callback: UsbdIoCallback,
        handle: *mut c_void,
    ) -> Result<()> {
        let scenario = {
            let mut state = self.0.lock().unwrap();
            state.snapshot.submissions += 1;
            state.scenario
        };
        if matches!(
            scenario,
            Scenario::Input {
                refuse_second: true
            }
        ) && slot.index == 1
        {
            return Err(Error::Errno(EIO));
        }
        assert!(matches!(scenario, Scenario::Input { .. }));
        let callback = callback.expect("input submission supplies its completion callback");
        // SAFETY: the real callback receives this slot's stable owned URB
        // identity and the exact Arc handle supplied by the real arm method.
        // No fixture lock is held. The completion runs before submit returns.
        unsafe { callback(slot.urb_ptr(), std::ptr::null_mut(), handle) };
        Ok(())
    }
    pub(crate) fn submit_intr_in(
        &self,
        slot: &SlotSys,
        _: u32,
        cb: UsbdIoCallback,
        handle: *mut c_void,
        _: u32,
    ) -> Result<()> {
        self.submit_input(slot, cb, handle)
    }
    pub(crate) fn submit_bulk_in(
        &self,
        slot: &SlotSys,
        _: u32,
        cb: UsbdIoCallback,
        handle: *mut c_void,
        _: u32,
    ) -> Result<()> {
        self.submit_input(slot, cb, handle)
    }
    pub(crate) fn send_intr_out(&self, _: &SlotSys, _: u32, _: u32) -> Result<(u32, u32)> {
        let mut state = self.0.lock().unwrap();
        state.snapshot.submissions += 1;
        match state.scenario {
            Scenario::Output { send_error: true } => Err(Error::Errno(EIO)),
            Scenario::Output { send_error: false } => Ok((USBD_STATUS_INPROG, 0)),
            Scenario::Input { .. } => Err(Error::Unsupported),
        }
    }
}
struct TestUrb {
    status: u32,
    length: u32,
}
pub(crate) struct SlotSys {
    state: Shared,
    index: usize,
    urb: Box<TestUrb>,
    bytes: Mutex<Vec<u8>>,
}
impl SlotSys {
    pub(crate) fn alloc(bytes: usize) -> Result<Self> {
        let state = active()?;
        let index = {
            let mut value = state.lock().unwrap();
            let index = value.allocated;
            value.allocated += 1;
            index
        };
        Ok(Self {
            state,
            index,
            urb: Box::new(TestUrb {
                status: USBD_STATUS_CMP,
                length: u32::try_from(bytes.min(4)).unwrap(),
            }),
            bytes: Mutex::new(vec![0x5a; bytes]),
        })
    }
    fn urb_ptr(&self) -> *mut UsbdUrb {
        std::ptr::from_ref(self.urb.as_ref()).cast_mut().cast()
    }
    pub(crate) fn fill(&self, data: &[u8]) {
        self.bytes.lock().unwrap()[..data.len()].copy_from_slice(data);
        let mut state = self.state.lock().unwrap();
        state.snapshot.fills += 1;
        state.snapshot.bytes = data.to_vec();
    }
    pub(crate) fn read_to(&self, dst: &mut [u8]) {
        let bytes = self.bytes.lock().unwrap();
        dst.copy_from_slice(&bytes[..dst.len()]);
    }
    pub(crate) fn owns_urb(&self, urb: *mut UsbdUrb) -> bool {
        self.urb_ptr() == urb
    }
}
impl Drop for SlotSys {
    fn drop(&mut self) {
        self.state.lock().unwrap().snapshot.freed_slots += 1;
    }
}
pub(crate) fn urb_status(urb: *mut UsbdUrb) -> (u32, u32) {
    // SAFETY: only submit_input invokes a callback in this backend. Its URB
    // pointer comes from a live TestUrb and its slot outlives the callback.
    let value = unsafe { &*urb.cast::<TestUrb>() };
    (value.status, value.length)
}

pub(crate) fn device() -> (crate::host::Host, crate::device::Device) {
    let host = crate::host::Host::connect(None, Ident::ANY).unwrap();
    let instance = crate::events::Instance::from_raw(UsbdDeviceInstance {
        path: 250,
        devno: 250,
        ..UsbdDeviceInstance::default()
    });
    let device = host.attach(&instance).unwrap();
    (host, device)
}
