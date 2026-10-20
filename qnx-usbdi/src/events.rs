// SPDX-License-Identifier: Apache-2.0
//! Device instances, the insertion/removal queue, and the removal flag.
//!
//! The `usbd_funcs_t` table carries no user pointer, so the two
//! callbacks io-usb invokes have nowhere to find their state except a
//! process-global. That is why there is one [`crate::Host`] per process:
//! every insertion and removal lands in the one queue here, and
//! `Host::poll` drains it.
//!
//! The insertion callback runs on libusbdi's event thread, the same
//! thread that delivers URB completions (`dwcotg/STATUS.md`, "THE usbdi
//! LESSON": blocking there waits for a completion that only this thread
//! can deliver). The removal callback runs there only when no transfer
//! is outstanding on the attached device when the event is processed;
//! otherwise libusbdi defers it and invokes it from whichever thread
//! brings the device's in-flight count to zero: the event thread's URB
//! completion, or the CALLER's thread inside `usbd_io` (a synchronous
//! control transfer, `DeviceSys::control`, or a refused bulk submit),
//! `usbd_descriptor` or `usbd_reset_device` [RE: SDP `libusbdi.so.2`
//! armle-v7 `usbdi_synchronise` @0x5940: mode 0 (the event thread's
//! removal path, @0x7f90) does `atomic_set(dev+20, 1)` and returns
//! `EBUSY` when `dev+24 != 0`, on which the event thread skips the
//! callback (@0x7f94-0x7f98); the deferred invoke is @0x5a00-0x5a3c,
//! `blx [[conn+0x1c]+8](conn, dev+28)` when a decrement reaches zero
//! with bit 0 set; the decrementers are `usbd_io` @0x8f40 (control) and
//! @0x8f74 (refused submit), `usbd_reset_device` @0x8dd8, `usbdi_sync_io`
//! @0x9614 (`usbd_descriptor`) and the event-thread completion @0x8030;
//! the head unit's copy is identical in shape: `usbdi_synchronise`
//! @0x5a0c, `blx` @0x5b08, `usbd_io` decrements @0x914c / @0x9180]. So
//! both callbacks do the least possible: copy the 36-byte instance, push
//! an event, and on removal set the `gone` flag of every device whose
//! `(path, devno, iface)` matches, taking only the queue's and the
//! registry's locks, which no caller holds across a `usbd_*` call.
//! Attach, open and every transfer happen on the caller's thread.
//!
//! Both callbacks run their whole body with thread cancellation
//! disabled (`cancel::CancelGuard`, their first local): `usbd_disconnect`
//! ends the event thread with `pthread_cancel`, which on QNX is a
//! `pthread_exit` with no unwinding, and a cancel landing inside the
//! `write(2)` of a log line would leave the stderr lock held for ever
//! (`cancel` module doc).
//!
//! Copyright 2026. Licensed under the Apache License, Version 2.0.

use core::fmt;
use core::ptr;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, Weak};
use std::time::Duration;

use crate::cancel::CancelGuard;
use crate::ffi::{UsbdConnection, UsbdDeviceInstance};

/// The most insertions/removals held between two `poll`s. The GIP shim
/// sizes its queue at four per pad slot because a burst of interface
/// matches arrives at daemon start; 64 covers any hub of devices this
/// crate will see, and a drop is logged, never silent.
pub const QUEUE_CAPACITY: usize = 64;

/// A newtype over the raw 36-byte `usbd_device_instance_t`. `Copy`; it is
/// never rebuilt from parts, because `usbd_attach` takes the whole
/// struct and io-usb writes it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instance {
    raw: UsbdDeviceInstance,
}

impl Instance {
    /// Wrap a raw instance (what the callbacks receive, or a test builds).
    #[must_use]
    pub const fn from_raw(raw: UsbdDeviceInstance) -> Self {
        Instance { raw }
    }

    /// The raw struct, for `usbd_attach`.
    #[must_use]
    pub const fn raw(&self) -> UsbdDeviceInstance {
        self.raw
    }

    /// Bus (host controller) index.
    #[must_use]
    pub const fn path(&self) -> u8 {
        self.raw.path
    }

    /// Device address on that bus.
    #[must_use]
    pub const fn devno(&self) -> u8 {
        self.raw.devno
    }

    /// io-usb's insertion generation, bumped on every re-enumeration.
    #[must_use]
    pub const fn generation(&self) -> u16 {
        self.raw.generation
    }

    /// `ident.vendor` (the VID as io-usb matched it; `WILDCARD` when the
    /// connect ident did not name one).
    #[must_use]
    pub const fn vendor(&self) -> u32 {
        self.raw.ident.vendor
    }

    /// `ident.device`.
    #[must_use]
    pub const fn product(&self) -> u32 {
        self.raw.ident.device
    }

    /// `ident.dclass`.
    #[must_use]
    pub const fn class(&self) -> u32 {
        self.raw.ident.dclass
    }

    /// `ident.subclass`.
    #[must_use]
    pub const fn subclass(&self) -> u32 {
        self.raw.ident.subclass
    }

    /// `ident.protocol`.
    #[must_use]
    pub const fn protocol(&self) -> u32 {
        self.raw.ident.protocol
    }

    /// The configuration io-usb selected at enumeration (0 = none).
    #[must_use]
    pub const fn config(&self) -> u32 {
        self.raw.config
    }

    /// The interface this instance names.
    #[must_use]
    pub const fn iface(&self) -> u32 {
        self.raw.iface
    }

    /// The alternate setting of that interface.
    #[must_use]
    pub const fn alternate(&self) -> u32 {
        self.raw.alternate
    }

    /// Same physical device: `(path, devno)` equal. io-usb fires one
    /// insertion per matching INTERFACE, so two instances of one device
    /// arrive for a composite; this is the dedup key.
    #[must_use]
    pub const fn same_device(&self, other: &Instance) -> bool {
        self.raw.path == other.raw.path && self.raw.devno == other.raw.devno
    }

    /// A removal callback is for this instance: `(path, devno, iface)`
    /// equal.
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

/// What the event thread reported since the last poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// io-usb matched an interface against the connect ident.
    Inserted(Instance),
    /// The device (or its interface) went away.
    Removed(Instance),
}

static EVENTS: OnceLock<Mutex<VecDeque<Event>>> = OnceLock::new();

/// Signalled by [`push`] for a caller blocked in [`wait`].
static ARRIVED: Condvar = Condvar::new();

fn events() -> &'static Mutex<VecDeque<Event>> {
    EVENTS.get_or_init(|| Mutex::new(VecDeque::with_capacity(QUEUE_CAPACITY)))
}

/// Queue one event and wake a caller blocked in [`wait`]; a full queue
/// drops the event with a log line. The wake is a broadcast on a condition
/// variable, which does not block, so it keeps the callbacks' rule (module
/// doc) of never waiting on libusbdi's event thread.
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

/// Take every queued event, oldest first.
#[must_use]
pub fn drain() -> Vec<Event> {
    let mut q = events().lock().unwrap_or_else(PoisonError::into_inner);
    q.drain(..).collect()
}

/// Block until at least one event is queued, or until `timeout` has passed
/// (`None`: no bound), then take every queued event, oldest first. Empty
/// only when the timeout passed with nothing queued.
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

/// Events waiting, without taking them.
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

/// The removal flag of one attached instance. Shared between the
/// `Device`, its `BulkOut` and the removal callback; every transfer
/// checks it first. Registered in a process-global list keyed by the
/// instance, so the callback (which has only the raw instance) can find
/// it; entries whose flag has been dropped are pruned on the next
/// registration. `Host::attach` registers it from the PRE-attach
/// instance, before `usbd_attach`, so a removal delivered during the
/// attach's io-usb round trips finds the entry too: the match is on
/// `(path, devno, iface)`, which the write-back does not change.
#[derive(Clone, Debug)]
pub struct GoneFlag {
    flag: Arc<AtomicBool>,
}

impl GoneFlag {
    /// Register a flag for `inst`. The flag starts clear.
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

    /// Whether a removal for this instance has been seen.
    #[must_use]
    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

/// Set the flag of every registered instance a removal matches; returns
/// how many were set.
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

/// The `insertion` entry of the funcs table. Runs on libusbdi's event
/// thread, with cancellation disabled for its whole body; copies the
/// instance and queues it, nothing else.
///
/// # Safety
/// `inst` must be null or point to a readable `usbd_device_instance_t`
/// for the duration of the call, which is io-usb's contract for the
/// callback. `conn` is not dereferenced.
pub unsafe extern "C" fn insertion_callback(
    _conn: *mut UsbdConnection,
    inst: *mut UsbdDeviceInstance,
) {
    // First, before any log line or lock: usbd_disconnect pthread_cancels
    // the event thread and the cancel can land mid-callback
    // during usbd_disconnect; QNX cancellation is
    // pthread_exit with no unwinding (pthread.h lines 254-255) and the
    // write(2) inside clock::log is a cancellation point, so a cancel
    // acted on there would leave the stderr lock held for ever. With the
    // state disabled until this guard drops (last, it is the first
    // local), the pending cancel is acted on at the event thread's next
    // MsgSendv_r to io-usb instead. Covers the log! in push() too.
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

/// The `removal` entry of the funcs table. Sets the matching `gone`
/// flags and queues the event. Runs on libusbdi's event thread when the
/// device has no transfer outstanding, otherwise on whichever thread
/// brings its in-flight count to zero, which can be the caller's own,
/// nested inside a `usbd_*` call (module doc).
///
/// Invariant: because it may be nested inside any `usbd_*` call on the
/// caller's thread, this callback must never take a lock a caller can
/// hold across such a call (today only `EVENTS` and `GONE`, which no
/// caller holds; the bulk ring's `Book` mutex is released before
/// `usbd_io`), must never block, and must never call back into
/// libusbdi. Its whole body runs with cancellation disabled, so a
/// `pthread_cancel` from `usbd_disconnect` cannot end the thread inside
/// one of those locks (`cancel` module doc).
///
/// # Safety
/// As [`insertion_callback`].
pub unsafe extern "C" fn removal_callback(
    _conn: *mut UsbdConnection,
    inst: *mut UsbdDeviceInstance,
) {
    // First, before any log line or lock: as in insertion_callback, the
    // cancel usbd_disconnect sends the event thread must not be acted on
    // inside the write(2) of the level-0 log line below or inside the
    // GONE / EVENTS locks; deferred until this guard drops (last), it is
    // taken at the thread's next MsgSendv_r to io-usb.
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

    /// The queue and the registry are process-global, so the tests that
    /// touch them run under one lock to keep their counts honest.
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

    /// `Host::attach` registers the flag from the pre-attach instance
    /// BEFORE `usbd_attach`, so a removal that lands during the attach's
    /// three io-usb round trips (attach, device descriptor, control
    /// pipe) is caught; the post-attach case is
    /// `callbacks_queue_in_order_and_flag_removals`. The match ignores
    /// the generation, which is what io-usb may rewrite.
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
        // Nothing queued: the bounded wait ends empty, after its bound.
        let started = std::time::Instant::now();
        assert_eq!(wait(Some(Duration::from_millis(30))), [] as [Event; 0]);
        assert!(started.elapsed() >= Duration::from_millis(30));
        // A push from another thread ends an unbounded wait.
        let pusher = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(50));
            push(Event::Inserted(inst(4, 2, 0)));
        });
        assert_eq!(wait(None), vec![Event::Inserted(inst(4, 2, 0))]);
        pusher.join().unwrap();
        // Already queued: no wait at all.
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
        // A flag whose holder is gone no longer counts as attached.
        let dropped = GoneFlag::register(&inst(9, 1, 0));
        drop(dropped);
        assert_eq!(mark_removed(&inst(9, 1, 0)), 0);
        let live = GoneFlag::register(&inst(9, 1, 0));
        assert_eq!(mark_removed(&inst(9, 1, 0)), 1);
        assert!(live.is_set());
    }
}
