//! Copyright 2026. Licensed under the Apache License, Version 2.0.
use crate::cancel::CancelGuard;
use crate::ffi::{UsbdConnection, UsbdDeviceInstance};
use core::fmt;
use core::ptr;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, Weak};
use std::time::Duration;
pub const QUEUE_CAPACITY: usize = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instance {
    raw: UsbdDeviceInstance,
}
impl Instance {
    #[must_use]
    pub const fn from_raw(raw: UsbdDeviceInstance) -> Self {
        Instance { raw }
    }
    #[must_use]
    pub const fn raw(&self) -> UsbdDeviceInstance {
        self.raw
    }
    #[must_use]
    pub const fn path(&self) -> u8 {
        self.raw.path
    }
    #[must_use]
    pub const fn devno(&self) -> u8 {
        self.raw.devno
    }
    #[must_use]
    pub const fn generation(&self) -> u16 {
        self.raw.generation
    }
    #[must_use]
    pub const fn vendor(&self) -> u32 {
        self.raw.ident.vendor
    }
    #[must_use]
    pub const fn product(&self) -> u32 {
        self.raw.ident.device
    }
    #[must_use]
    pub const fn class(&self) -> u32 {
        self.raw.ident.dclass
    }
    #[must_use]
    pub const fn subclass(&self) -> u32 {
        self.raw.ident.subclass
    }
    #[must_use]
    pub const fn protocol(&self) -> u32 {
        self.raw.ident.protocol
    }
    #[must_use]
    pub const fn config(&self) -> u32 {
        self.raw.config
    }
    #[must_use]
    pub const fn iface(&self) -> u32 {
        self.raw.iface
    }
    #[must_use]
    pub const fn alternate(&self) -> u32 {
        self.raw.alternate
    }
    #[must_use]
    pub const fn same_device(&self, other: &Instance) -> bool {
        self.raw.path == other.raw.path && self.raw.devno == other.raw.devno
    }
    #[must_use]
    pub const fn matches_removal(&self, removed: &Instance) -> bool {
        self.same_device(removed) && self.raw.iface == removed.raw.iface
    }
}
impl fmt::Display for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "path {} devno {} gen {} if {} alt {} cfg {} ident {:04x}:{:04x} {:02x}/{:02x}/{:02x}",
            self.raw.path,
            self.raw.devno,
            self.raw.generation,
            self.raw.iface,
            self.raw.alternate,
            self.raw.config,
            self.raw.ident.vendor,
            self.raw.ident.device,
            self.raw.ident.dclass,
            self.raw.ident.subclass,
            self.raw.ident.protocol
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Inserted(Instance),
    Removed(Instance),
}
static EVENTS: OnceLock<Mutex<VecDeque<Event>>> = OnceLock::new();
static ARRIVED: Condvar = Condvar::new();
fn events() -> &'static Mutex<VecDeque<Event>> {
    EVENTS.get_or_init(|| Mutex::new(VecDeque::with_capacity(QUEUE_CAPACITY)))
}
pub fn push(event: Event) {
    let mut q = events().lock().unwrap_or_else(PoisonError::into_inner);
    if q.len() >= QUEUE_CAPACITY {
        let inst = match event {
            Event::Inserted(i) | Event::Removed(i) => i,
        };
        crate::log!("usbdi: event queue full ({QUEUE_CAPACITY}); dropping {inst}");
        return;
    }
    q.push_back(event);
    ARRIVED.notify_all();
    drop(q);
    crate::activity::signal();
}
#[must_use]
pub fn drain() -> Vec<Event> {
    let mut q = events().lock().unwrap_or_else(PoisonError::into_inner);
    q.drain(..).collect()
}
#[must_use]
pub fn wait(timeout: Option<Duration>) -> Vec<Event> {
    let mut q = events().lock().unwrap_or_else(PoisonError::into_inner);
    match timeout {
        None => {
            while q.is_empty() {
                q = ARRIVED.wait(q).unwrap_or_else(PoisonError::into_inner);
            }
        }
        Some(timeout) => {
            q = ARRIVED
                .wait_timeout_while(q, timeout, |q| q.is_empty())
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
    q.drain(..).collect()
}
#[must_use]
pub fn pending() -> usize {
    events()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len()
}
struct GoneEntry {
    inst: Instance,
    flag: Weak<AtomicBool>,
}
static GONE: OnceLock<Mutex<Vec<GoneEntry>>> = OnceLock::new();
fn gone_registry() -> &'static Mutex<Vec<GoneEntry>> {
    GONE.get_or_init(|| Mutex::new(Vec::new()))
}
#[derive(Clone, Debug)]
pub struct GoneFlag {
    flag: Arc<AtomicBool>,
}
impl GoneFlag {
    #[must_use]
    pub fn register(inst: &Instance) -> GoneFlag {
        let flag = Arc::new(AtomicBool::new(false));
        let mut reg = gone_registry()
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        reg.retain(|e| e.flag.strong_count() > 0);
        reg.push(GoneEntry {
            inst: *inst,
            flag: Arc::downgrade(&flag),
        });
        GoneFlag { flag }
    }
    #[must_use]
    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}
pub fn mark_removed(removed: &Instance) -> usize {
    let reg = gone_registry()
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut hits = 0;
    for entry in reg.iter().filter(|e| e.inst.matches_removal(removed)) {
        if let Some(flag) = entry.flag.upgrade() {
            flag.store(true, Ordering::Release);
            hits += 1;
        }
    }
    hits
}
/// # Safety
/// `inst` must be null or point to a readable `usbd_device_instance_t`
/// for the duration of the call, which is io-usb's contract for the
/// callback. `conn` is not dereferenced.
pub unsafe extern "C" fn insertion_callback(
    _conn: *mut UsbdConnection,
    inst: *mut UsbdDeviceInstance,
) {
    let _cancel = CancelGuard::new();
    if inst.is_null() {
        crate::log!("usbdi: insertion callback with a null instance; ignored");
        return;
    }
    // SAFETY: non-null and readable per the contract above. Unaligned
    // read so an unexpected alignment cannot fault under SCTLR.A = 1.
    let raw = unsafe { ptr::read_unaligned(inst) };
    let instance = Instance::from_raw(raw);
    crate::vlog!("usbdi: insertion: {instance}");
    push(Event::Inserted(instance));
}
/// # Safety
/// As [`insertion_callback`].
pub unsafe extern "C" fn removal_callback(
    _conn: *mut UsbdConnection,
    inst: *mut UsbdDeviceInstance,
) {
    let _cancel = CancelGuard::new();
    if inst.is_null() {
        crate::log!("usbdi: removal callback with a null instance; ignored");
        return;
    }
    // SAFETY: as in insertion_callback.
    let raw = unsafe { ptr::read_unaligned(inst) };
    let instance = Instance::from_raw(raw);
    let hits = mark_removed(&instance);
    crate::log!("usbdi: removal: {instance} ({hits} attached device(s) flagged)");
    push(Event::Removed(instance));
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::UsbdDeviceIdent;
    fn inst(path: u8, devno: u8, iface: u32) -> Instance {
        Instance::from_raw(UsbdDeviceInstance {
            path,
            devno,
            generation: 7,
            ident: UsbdDeviceIdent {
                vendor: 0x17E9,
                device: 0x0198,
                dclass: 0xFF,
                subclass: 0,
                protocol: 0,
            },
            config: 1,
            iface,
            alternate: 0,
        })
    }
    static SERIAL: Mutex<()> = Mutex::new(());
    #[test]
    fn accessors_read_the_raw_fields() {
        let i = inst(1, 3, 2);
        assert_eq!((i.path(), i.devno(), i.generation()), (1, 3, 7));
        assert_eq!((i.vendor(), i.product()), (0x17E9, 0x0198));
        assert_eq!((i.class(), i.subclass(), i.protocol()), (0xFF, 0, 0));
        assert_eq!((i.config(), i.iface(), i.alternate()), (1, 2, 0));
        assert_eq!(i.raw().iface, 2);
        let text = i.to_string();
        assert!(text.contains("path 1 devno 3"));
        assert!(text.contains("17e9:0198"));
    }
    #[test]
    fn same_device_ignores_the_interface_and_removal_does_not() {
        let a = inst(1, 3, 0);
        let b = inst(1, 3, 1);
        let c = inst(1, 4, 0);
        assert!(a.same_device(&b));
        assert!(!a.same_device(&c));
        assert!(!a.matches_removal(&b));
        assert!(a.matches_removal(&inst(1, 3, 0)));
    }
    #[test]
    fn callbacks_queue_in_order_and_flag_removals() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = drain();
        let attached = inst(2, 5, 0);
        let flag = GoneFlag::register(&attached);
        let other = GoneFlag::register(&inst(2, 6, 0));
        let mut raw_a = inst(2, 5, 0).raw();
        let mut raw_b = inst(2, 5, 1).raw();
        // SAFETY: the pointers name live locals for the duration of each call.
        unsafe {
            insertion_callback(ptr::null_mut(), &raw mut raw_a);
            insertion_callback(ptr::null_mut(), &raw mut raw_b);
            insertion_callback(ptr::null_mut(), ptr::null_mut());
            removal_callback(ptr::null_mut(), &raw mut raw_b);
        }
        assert!(!flag.is_set());
        // SAFETY: as above.
        unsafe { removal_callback(ptr::null_mut(), &raw mut raw_a) };
        assert!(flag.is_set());
        assert!(!other.is_set());
        let got = drain();
        assert_eq!(
            got,
            vec![
                Event::Inserted(inst(2, 5, 0)),
                Event::Inserted(inst(2, 5, 1)),
                Event::Removed(inst(2, 5, 1)),
                Event::Removed(inst(2, 5, 0)),
            ]
        );
        assert_eq!(drain(), [] as [Event; 0]);
    }
    #[test]
    fn a_removal_before_the_attach_completes_is_caught() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = drain();
        let pre_attach = inst(1, 3, 0);
        let flag = GoneFlag::register(&pre_attach);
        assert!(!flag.is_set());
        let mut removed = inst(1, 3, 0).raw();
        removed.generation = 8;
        // SAFETY: the pointer names a live local for the duration of the call.
        unsafe { removal_callback(ptr::null_mut(), &raw mut removed) };
        assert!(flag.is_set());
        assert_eq!(drain(), vec![Event::Removed(Instance::from_raw(removed))]);
    }
    #[test]
    fn wait_returns_at_the_push_and_empty_at_its_timeout() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = drain();
        let started = std::time::Instant::now();
        assert_eq!(wait(Some(Duration::from_millis(30))), [] as [Event; 0]);
        assert!(started.elapsed() >= Duration::from_millis(30));
        let pusher = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(50));
            push(Event::Inserted(inst(4, 2, 0)));
        });
        assert_eq!(wait(None), vec![Event::Inserted(inst(4, 2, 0))]);
        pusher.join().unwrap();
        push(Event::Removed(inst(4, 2, 0)));
        assert_eq!(
            wait(Some(Duration::from_secs(60))),
            vec![Event::Removed(inst(4, 2, 0))]
        );
    }
    #[test]
    fn a_full_queue_drops_and_a_dead_flag_is_pruned() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = drain();
        for n in 0..(QUEUE_CAPACITY + 3) {
            push(Event::Inserted(inst(9, u8::try_from(n % 250).unwrap(), 0)));
        }
        assert_eq!(pending(), QUEUE_CAPACITY);
        assert_eq!(drain().len(), QUEUE_CAPACITY);
        let dropped = GoneFlag::register(&inst(9, 1, 0));
        drop(dropped);
        assert_eq!(mark_removed(&inst(9, 1, 0)), 0);
        let live = GoneFlag::register(&inst(9, 1, 0));
        assert_eq!(mark_removed(&inst(9, 1, 0)), 1);
        assert!(live.is_set());
    }
}
