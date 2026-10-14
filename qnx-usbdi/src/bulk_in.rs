//! Copyright 2026. Licensed under the Apache License, Version 2.0.
use crate::bulk::{judge, wait_last_reference, STOP_SLICE_MS};
use crate::cancel::CancelGuard;
use crate::clock;
use crate::desc::Endpoint;
use crate::device::DeviceShared;
use crate::ffi::{
    urb_status_name, usb_status_name, UsbdPipe, UsbdUrb, EINVAL, USBD_STATUS_CMP,
    USBD_TIME_INFINITY, USBD_URB_STATUS_MASK,
};
use crate::sys;
use crate::{stop, Error, Result};
use core::ffi::c_void;
use core::fmt;
use core::mem::ManuallyDrop;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
pub const DEFAULT_SLOTS: usize = 4;
pub const DEFAULT_SLOT_BYTES: usize = 16 * 1024;
pub const CLEAR_HALT_TIMEOUT_MS: u32 = 1000;
const DRAIN_TIMEOUT_MS: u32 = 1000;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BulkInStats {
    pub urbs: u64,
    pub bytes: u64,
    pub failures: u64,
    pub max_latency_ms: u64,
    pub in_flight: usize,
    pub reports: u64,
    pub wakes: u64,
}
#[derive(Clone, Copy, Debug, Default)]
struct SlotBook {
    armed: bool,
    submitted_ms: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Done {
    idx: usize,
    status: u32,
    alen: u32,
    latency_ms: u64,
}
#[derive(Debug, Default)]
struct Book {
    slots: Vec<SlotBook>,
    done: VecDeque<Done>,
    stats: BulkInStats,
    wake: bool,
}
impl Book {
    fn new(slots: usize) -> Book {
        Book {
            slots: vec![SlotBook::default(); slots],
            done: VecDeque::with_capacity(slots),
            stats: BulkInStats::default(),
            wake: false,
        }
    }
    fn idle(&self) -> bool {
        self.stats.in_flight == 0
    }
    fn arm(&mut self, idx: usize, now_ms: u64) {
        if let Some(slot) = self.slots.get_mut(idx) {
            slot.armed = true;
            slot.submitted_ms = now_ms;
            self.stats.in_flight += 1;
            self.stats.urbs += 1;
        }
    }
    fn unarm(&mut self, idx: usize) {
        if let Some(slot) = self.slots.get_mut(idx) {
            slot.armed = false;
            self.stats.in_flight = self.stats.in_flight.saturating_sub(1);
            self.stats.urbs = self.stats.urbs.saturating_sub(1);
            self.stats.failures += 1;
        }
    }
    fn disarmed(&self) -> Vec<usize> {
        self.slots
            .iter()
            .enumerate()
            .filter(|&(i, s)| !s.armed && !self.done.iter().any(|d| d.idx == i))
            .map(|(i, _)| i)
            .collect()
    }
    fn complete(&mut self, idx: usize, (status, alen): (u32, u32), now_ms: u64) -> Done {
        let slot = self.slots.get(idx).copied().unwrap_or_default();
        if let Some(s) = self.slots.get_mut(idx) {
            s.armed = false;
        }
        self.stats.in_flight = self.stats.in_flight.saturating_sub(1);
        let latency_ms = now_ms.saturating_sub(slot.submitted_ms);
        self.stats.max_latency_ms = self.stats.max_latency_ms.max(latency_ms);
        if status & USBD_URB_STATUS_MASK == USBD_STATUS_CMP {
            self.stats.bytes += u64::from(alen);
        } else {
            self.stats.failures += 1;
        }
        let done = Done {
            idx,
            status,
            alen,
            latency_ms,
        };
        self.done.push_back(done);
        done
    }
}
struct RingWait {
    book: Mutex<Book>,
    cv: Condvar,
}
impl RingWait {
    fn new(slots: usize) -> RingWait {
        RingWait {
            book: Mutex::new(Book::new(slots)),
            cv: Condvar::new(),
        }
    }
    fn lock(&self) -> MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn wait_done(
        &self,
        timeout_ms: u32,
        gone: impl Fn() -> bool,
        stopped: impl Fn() -> bool,
    ) -> Result<Done> {
        let started = Instant::now();
        let deadline = started + Duration::from_millis(u64::from(timeout_ms));
        let mut book = self.lock();
        loop {
            if let Some(d) = book.done.pop_front() {
                book.stats.reports += 1;
                return Ok(d);
            }
            if gone() && book.idle() {
                return Err(Error::Gone);
            }
            if stopped() {
                return Err(Error::Interrupted);
            }
            if book.wake {
                book.wake = false;
                book.stats.wakes += 1;
                drop(book);
                crate::vlog!(
                    "usbdi: bulk-in read: ended by a wake after {} ms of {timeout_ms} ms, nothing completed",
                    clock::elapsed_ms(started)
                );
                return Err(Error::Timeout);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }
            book = self
                .cv
                .wait_timeout(book, remaining.min(Duration::from_millis(STOP_SLICE_MS)))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}
#[derive(Clone)]
pub struct BulkInWaker {
    wait: Arc<RingWait>,
}
impl BulkInWaker {
    pub fn wake(&self) {
        let mut book = self.wait.lock();
        book.wake = true;
        self.wait.cv.notify_all();
        drop(book);
    }
}
impl fmt::Debug for BulkInWaker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BulkInWaker").finish_non_exhaustive()
    }
}
pub(crate) struct BulkInState {
    sys: Vec<sys::SlotSys>,
    wait: Arc<RingWait>,
    slot_bytes: usize,
}
// SAFETY: The conditions for `sys` are the same as for `bulk::RingState` and `intr::InState`.
// - `sys` stays unchanged after construction.
// - io-usb writes a slot buffer between arm and completion.
// - `BulkIn::read` has exclusive access through `&mut self`. It reads after completion leaves the done queue and before the next arm.
// - The callback uses the mutex for `Book` and compares URB pointers.
// - The caller thread frees URBs and buffers once after `EOK` from `usbd_close_pipe` and a strong count of one.
// - `BulkInWaker` holds only `RingWait`. Its fields provide `Send + Sync` without an unsafe implementation.
// - The waker changes one flag in `Book` with the mutex. It cannot access `sys` or the reference count.
unsafe impl Send for BulkInState {}
// SAFETY: as above.
unsafe impl Sync for BulkInState {}
impl BulkInState {
    fn new(slots: usize, slot_bytes: usize) -> Result<BulkInState> {
        let mut sys_slots = Vec::with_capacity(slots);
        for i in 0..slots {
            match sys::SlotSys::alloc(slot_bytes) {
                Ok(s) => sys_slots.push(s),
                Err(e) => {
                    crate::log!("usbdi: bulk-in slot {i} of {slots} ({slot_bytes} bytes): {e}");
                    return Err(e);
                }
            }
        }
        Ok(BulkInState {
            sys: sys_slots,
            wait: Arc::new(RingWait::new(slots)),
            slot_bytes,
        })
    }
    fn lock(&self) -> MutexGuard<'_, Book> {
        self.wait.lock()
    }
    fn waker(&self) -> BulkInWaker {
        BulkInWaker {
            wait: Arc::clone(&self.wait),
        }
    }
    fn wait_idle(&self, timeout_ms: u32) -> Result<()> {
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
        let mut book = self.lock();
        while !book.idle() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }
            book = self
                .wait
                .cv
                .wait_timeout(book, remaining)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        Ok(())
    }
}
impl Drop for BulkInState {
    fn drop(&mut self) {
        crate::vlog!(
            "usbdi: bulk-in ring of {} slot(s) x {} bytes freed",
            self.sys.len(),
            self.slot_bytes
        );
    }
}
/// # Safety
/// `handle` must be the pointer `Arc::into_raw` produced for this very
/// submission. It is reclaimed exactly once here. `urb` is the URB
/// io-usb completed.
unsafe extern "C" fn bulk_in_complete(
    urb: *mut UsbdUrb,
    _pipe: *mut UsbdPipe,
    handle: *mut c_void,
) {
    let _cancel = CancelGuard::new();
    if handle.is_null() {
        crate::log!("usbdi: bulk-in completion with a null handle; ignored");
        return;
    }
    // SAFETY: handle is the Arc::into_raw pointer the arm passed for this
    // URB (the contract above). This is its one matching from_raw.
    let ring: Arc<BulkInState> =
        unsafe { Arc::from_raw(handle.cast_const().cast::<BulkInState>()) };
    let now = clock::now_ms();
    let (status, alen) = sys::urb_status(urb);
    let idx = ring.sys.iter().position(|s| s.owns_urb(urb));
    let mut book = ring.lock();
    match idx {
        Some(i) => {
            let done = book.complete(i, (status, alen), now);
            if status & USBD_URB_STATUS_MASK == USBD_STATUS_CMP {
                crate::vlog!(
                    "usbdi: bulk-in slot {i}: CMP {alen} bytes after {} ms, {} queued",
                    done.latency_ms,
                    book.done.len()
                );
            } else {
                crate::log!(
                    "usbdi: bulk-in slot {i}: status {status:#010x} ({}/{}) alen {alen} after {} ms; slot disarmed",
                    urb_status_name(status),
                    usb_status_name(status),
                    done.latency_ms
                );
            }
        }
        None => {
            crate::log!("usbdi: bulk-in completion for a URB not in the ring ({urb:?}); ignored");
        }
    }
    ring.wait.cv.notify_all();
    drop(book);
    drop(ring);
}
pub struct BulkIn {
    ring: ManuallyDrop<Arc<BulkInState>>,
    pipe: sys::PipeSys,
    dev: Rc<DeviceShared>,
    ep: Endpoint,
}
impl BulkIn {
    pub(crate) fn open(
        dev: Rc<DeviceShared>,
        ep: Endpoint,
        slots: usize,
        slot_bytes: usize,
    ) -> Result<BulkIn> {
        if !(ep.is_bulk() && ep.is_in()) {
            crate::log!(
                "usbdi: open_bulk_in refused: endpoint {:#04x} attr {:#04x} is not bulk IN",
                ep.address(),
                ep.attributes()
            );
            return Err(Error::NoEndpoint);
        }
        if slots == 0 || slot_bytes == 0 || u32::try_from(slot_bytes).is_err() {
            crate::log!("usbdi: open_bulk_in refused: {slots} slot(s) x {slot_bytes} bytes");
            return Err(Error::Errno(EINVAL));
        }
        if dev.gone.is_set() {
            return Err(Error::Gone);
        }
        let pipe = dev.sys.open_pipe(ep)?;
        let ring = match BulkInState::new(slots, slot_bytes) {
            Ok(r) => r,
            Err(e) => {
                if let Err(ce) = pipe.close() {
                    crate::log!("usbdi: close_pipe after a failed ring allocation: {ce}");
                }
                return Err(e);
            }
        };
        let mut bulk_in = BulkIn {
            ring: ManuallyDrop::new(Arc::new(ring)),
            pipe,
            dev,
            ep,
        };
        crate::log!(
            "usbdi: bulk IN endpoint {:#04x} (mps {}) open: {slots} slot(s) x {slot_bytes} bytes",
            ep.address(),
            ep.max_packet()
        );
        if let Err(e) = bulk_in.reset() {
            crate::log!(
                "usbdi: pipe hygiene on endpoint {:#04x} failed: {e}; continuing",
                ep.address()
            );
        }
        let armed = bulk_in.rearm()?;
        if armed == 0 {
            return Err(Error::Errno(EINVAL));
        }
        Ok(bulk_in)
    }
    #[must_use]
    pub fn endpoint(&self) -> Endpoint {
        self.ep
    }
    #[must_use]
    pub fn slot_bytes(&self) -> usize {
        self.ring.slot_bytes
    }
    fn arm(&self, idx: usize) -> Result<()> {
        let ring: &BulkInState = &self.ring;
        let Some(slot) = ring.sys.get(idx) else {
            return Err(Error::Errno(EINVAL));
        };
        let Ok(len32) = u32::try_from(ring.slot_bytes) else {
            return Err(Error::Errno(EINVAL));
        };
        ring.lock().arm(idx, clock::now_ms());
        let handle = Arc::into_raw(Arc::clone(&self.ring))
            .cast_mut()
            .cast::<c_void>();
        match self.pipe.submit_bulk_in(
            slot,
            len32,
            Some(bulk_in_complete),
            handle,
            USBD_TIME_INFINITY,
        ) {
            Ok(()) => Ok(()),
            Err(e) => {
                // SAFETY: usbd_io refused the URB, so the callback will
                // never run for it and this is the handle's only owner.
                drop(unsafe { Arc::from_raw(handle.cast_const().cast::<BulkInState>()) });
                ring.lock().unarm(idx);
                crate::log!("usbdi: bulk-in slot {idx}: usbd_io refused: {e}");
                Err(e)
            }
        }
    }
    /// # Errors
    /// [`Error::Gone`]. [`Error::Errno`] when `usbd_io` refused and no
    /// slot is in flight.
    pub fn rearm(&mut self) -> Result<usize> {
        if self.dev.gone.is_set() {
            return Err(Error::Gone);
        }
        let mut first_err = None;
        let idle = self.ring.lock().disarmed();
        for idx in idle {
            if let Err(e) = self.arm(idx) {
                first_err.get_or_insert(e);
            }
        }
        let armed = self.ring.lock().stats.in_flight;
        match first_err {
            Some(e) if armed == 0 => Err(e),
            _ => Ok(armed),
        }
    }
    /// # Errors
    /// As above, plus [`Error::Timeout`] when nothing completed in time or
    /// a wake ended the wait.
    pub fn read(&mut self, buf: &mut [u8], timeout_ms: u32) -> Result<usize> {
        let ring: &BulkInState = &self.ring;
        let gone = || self.dev.gone.is_set();
        let done = ring.wait.wait_done(timeout_ms, gone, stop::requested)?;
        judge(done.status, done.alen, done.alen)?;
        let n = usize::try_from(done.alen)
            .unwrap_or(usize::MAX)
            .min(buf.len())
            .min(ring.slot_bytes);
        if let (Some(slot), Some(dst)) = (ring.sys.get(done.idx), buf.get_mut(..n)) {
            slot.read_to(dst);
        }
        if let Err(e) = self.arm(done.idx) {
            crate::log!(
                "usbdi: bulk-in slot {}: not re-armed ({e}); rearm() when the device is back",
                done.idx
            );
        }
        Ok(n)
    }
    /// # Errors
    /// [`Error::Gone`] when the device is gone.
    /// The fallback error when the two methods fail.
    pub fn reset(&mut self) -> Result<()> {
        self.dev.reset_pipe(&self.pipe, self.ep)
    }
    #[must_use]
    pub fn stats(&self) -> BulkInStats {
        self.ring.lock().stats
    }
    #[must_use]
    pub fn waker(&self) -> BulkInWaker {
        self.ring.waker()
    }
}
impl Drop for BulkIn {
    fn drop(&mut self) {
        let started = Instant::now();
        let in_flight = self.ring.lock().stats.in_flight;
        match self.pipe.abort() {
            Ok(()) => crate::vlog!(
                "usbdi: bulk-in teardown: abort_pipe(endpoint {:#04x}) ok, {in_flight} in flight",
                self.ep.address()
            ),
            Err(e) => crate::log!(
                "usbdi: bulk-in teardown: abort_pipe(endpoint {:#04x}) failed: {e}, {in_flight} in flight",
                self.ep.address()
            ),
        }
        let drained = self.ring.wait_idle(DRAIN_TIMEOUT_MS).is_ok();
        let what = format!("usbd_close_pipe(endpoint {:#04x})", self.ep.address());
        let closed = drained
            && match clock::retry_while_busy(&what, clock::SETTLE_TIMEOUT_MS, || self.pipe.close())
            {
                Ok(()) => true,
                Err(e) => {
                    crate::log!("usbdi: bulk-in teardown: {what} failed: {e}");
                    false
                }
            };
        let last = closed && wait_last_reference(&self.ring, clock::SETTLE_TIMEOUT_MS);
        if last {
            let stats = self.ring.lock().stats;
            crate::log!(
                "usbdi: bulk-in teardown: drained and closed in {} ms ({} URB(s), {} report(s), {} bytes, {} failure(s))",
                clock::elapsed_ms(started),
                stats.urbs,
                stats.reports,
                stats.bytes,
                stats.failures
            );
            // SAFETY: This thread holds the only state reference.
            // The pipe is closed and no arm runs during `Drop`, so callbacks cannot take new references.
            // This thread drops the state once before `usbd_detach` and `usbd_disconnect`.
            unsafe { ManuallyDrop::drop(&mut self.ring) };
        } else {
            let left = self.ring.lock().stats.in_flight;
            let refs = Arc::strong_count(&self.ring);
            crate::log!(
                "usbdi: bulk-in teardown: {} after {} ms, {left} URB(s) in flight, {refs} ring ref(s): the ring is leaked (the memchunk destroy at usbd_disconnect reclaims the URBs and buffers){}",
                if !drained {
                    "drain timed out"
                } else if !closed {
                    "close_pipe stayed EBUSY"
                } else {
                    "a callback still holds the ring"
                },
                clock::elapsed_ms(started),
                if closed {
                    ""
                } else {
                    "; close_pipe is left to usbd_detach"
                }
            );
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::{USBD_STATUS_CMP_ERR, USBD_STATUS_STALL};
    #[test]
    fn completions_queue_in_order_and_disarm_their_slot() {
        let mut b = Book::new(2);
        b.arm(0, 10);
        b.arm(1, 10);
        assert_eq!(b.stats.in_flight, 2);
        assert_eq!(b.disarmed(), [] as [usize; 0]);
        let d1 = b.complete(1, (USBD_STATUS_CMP, 5), 25);
        let d0 = b.complete(0, (USBD_STATUS_CMP_ERR | USBD_STATUS_STALL, 0), 30);
        assert_eq!((d1.idx, d1.alen, d1.latency_ms), (1, 5, 15));
        assert_eq!(d0.idx, 0);
        assert_eq!(b.done.pop_front(), Some(d1));
        assert_eq!(b.done.pop_front(), Some(d0));
        assert_eq!(b.disarmed(), vec![0, 1]);
        assert_eq!(
            (b.stats.in_flight, b.stats.bytes, b.stats.failures),
            (0, 5, 1)
        );
        assert!(b.idle());
    }
    #[test]
    fn a_queued_completion_is_not_offered_for_rearm() {
        let mut b = Book::new(2);
        b.arm(0, 0);
        b.arm(1, 0);
        b.complete(0, (USBD_STATUS_CMP_ERR | USBD_STATUS_STALL, 0), 1);
        assert_eq!(b.disarmed(), [] as [usize; 0]);
        let d = b.done.pop_front().expect("queued completion");
        assert_eq!(d.idx, 0);
        assert_eq!(b.disarmed(), vec![0]);
    }
    #[test]
    fn a_refused_arm_comes_back_out_of_the_counters() {
        let mut b = Book::new(1);
        b.arm(0, 0);
        b.unarm(0);
        assert_eq!(
            (b.stats.in_flight, b.stats.urbs, b.stats.failures),
            (0, 0, 1)
        );
        assert_eq!(b.disarmed(), vec![0]);
    }
    #[test]
    fn open_needs_a_bulk_in_endpoint() {
        let bulk_out = Endpoint::new(7, 0x01, 0x02, 512, 0);
        let intr_in = Endpoint::new(7, 0x81, 0x03, 8, 10);
        let bulk_in = Endpoint::new(7, 0x81, 0x02, 512, 0);
        assert!(!(bulk_out.is_bulk() && bulk_out.is_in()));
        assert!(!(intr_in.is_bulk() && intr_in.is_in()));
        assert!(bulk_in.is_bulk() && bulk_in.is_in());
        assert_eq!(DEFAULT_SLOTS, 4);
        assert_eq!(DEFAULT_SLOT_BYTES, 16 * 1024);
    }
    fn ring(slots: usize) -> BulkInState {
        BulkInState {
            sys: Vec::new(),
            wait: Arc::new(RingWait::new(slots)),
            slot_bytes: 0,
        }
    }
    fn wait(ring: &BulkInState, timeout_ms: u32) -> Result<Done> {
        ring.wait.wait_done(timeout_ms, || false, || false)
    }
    fn complete(ring: &BulkInState, idx: usize, alen: u32) -> Done {
        let mut book = ring.lock();
        let done = book.complete(idx, (USBD_STATUS_CMP, alen), clock::now_ms());
        ring.wait.cv.notify_all();
        drop(book);
        done
    }
    #[test]
    fn the_waker_crosses_threads_and_clones() {
        fn send_sync_clone<T: Send + Sync + Clone>() {}
        send_sync_clone::<BulkInWaker>();
        let ring = ring(1);
        assert_eq!(format!("{:?}", ring.waker()), "BulkInWaker { .. }");
    }
    #[test]
    fn a_wake_ends_a_blocked_wait_promptly() {
        let ring = ring(1);
        let waker = ring.waker();
        let started = Instant::now();
        let other = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            let woke = Instant::now();
            waker.wake();
            woke
        });
        assert_eq!(wait(&ring, 10_000), Err(Error::Timeout));
        let returned = Instant::now();
        let woke = other.join().unwrap();
        let blocked_ms = clock::elapsed_ms(started);
        let wake_to_return = returned.saturating_duration_since(woke);
        assert!(blocked_ms >= 50, "returned after {blocked_ms} ms");
        assert!(
            wake_to_return < Duration::from_millis(STOP_SLICE_MS / 2),
            "{wake_to_return:?} from wake to return"
        );
        assert_eq!(ring.lock().stats.wakes, 1);
        assert!(!ring.lock().wake);
        let started = Instant::now();
        assert_eq!(wait(&ring, 60), Err(Error::Timeout));
        assert!(clock::elapsed_ms(started) >= 50);
        assert_eq!(ring.lock().stats.wakes, 1);
    }
    #[test]
    fn a_wake_before_a_wait_ends_the_next_wait_once_and_only_once() {
        let ring = ring(1);
        let waker = ring.waker();
        waker.wake();
        waker.clone().wake();
        ring.waker().wake();
        let started = Instant::now();
        assert_eq!(wait(&ring, 10_000), Err(Error::Timeout));
        assert!(clock::elapsed_ms(started) < 1000);
        assert_eq!(ring.lock().stats.wakes, 1);
        let started = Instant::now();
        assert_eq!(wait(&ring, 60), Err(Error::Timeout));
        assert!(clock::elapsed_ms(started) >= 50);
        assert_eq!(ring.lock().stats.wakes, 1);
        waker.wake();
        assert_eq!(wait(&ring, 0), Err(Error::Timeout));
        assert_eq!(ring.lock().stats.wakes, 2);
        assert_eq!(wait(&ring, 0), Err(Error::Timeout));
        assert_eq!(ring.lock().stats.wakes, 2);
        assert_eq!(ring.lock().stats.reports, 0);
    }
    #[test]
    fn a_completion_wins_over_a_pending_wake_and_the_wake_is_kept() {
        let ring = ring(2);
        let waker = ring.waker();
        ring.lock().arm(0, clock::now_ms());
        ring.lock().arm(1, clock::now_ms());
        let first = complete(&ring, 1, 7);
        waker.wake();
        let second = complete(&ring, 0, 9);
        assert_eq!(wait(&ring, 10_000), Ok(first));
        assert!(ring.lock().wake);
        assert_eq!(wait(&ring, 10_000), Ok(second));
        assert!(ring.lock().wake);
        assert_eq!(ring.lock().stats.wakes, 0);
        let started = Instant::now();
        assert_eq!(wait(&ring, 10_000), Err(Error::Timeout));
        assert!(clock::elapsed_ms(started) < 1000);
        let stats = ring.lock().stats;
        assert_eq!((stats.reports, stats.wakes, stats.bytes), (2, 1, 16));
        let started = Instant::now();
        assert_eq!(wait(&ring, 60), Err(Error::Timeout));
        assert!(clock::elapsed_ms(started) >= 50);
    }
    #[test]
    fn a_completion_ends_a_blocked_wait_with_a_waker_alive() {
        let ring = Arc::new(ring(1));
        let _waker = ring.waker();
        ring.lock().arm(0, clock::now_ms());
        let callback = Arc::clone(&ring);
        let event_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            complete(&callback, 0, 5)
        });
        let started = Instant::now();
        let got = wait(&ring, 10_000);
        let took = clock::elapsed_ms(started);
        assert_eq!(got, Ok(event_thread.join().unwrap()));
        assert!((50..5000).contains(&took), "{took} ms");
        assert_eq!(ring.lock().stats.wakes, 0);
    }
    #[test]
    fn gone_and_stop_are_reported_before_a_pending_wake() {
        let ring = ring(1);
        ring.waker().wake();
        assert_eq!(
            ring.wait.wait_done(10_000, || true, || false),
            Err(Error::Gone)
        );
        assert_eq!(
            ring.wait.wait_done(10_000, || false, || true),
            Err(Error::Interrupted)
        );
        assert!(ring.lock().wake);
        assert_eq!(ring.lock().stats.wakes, 0);
        ring.lock().arm(0, clock::now_ms());
        assert_eq!(
            ring.wait.wait_done(10_000, || true, || false),
            Err(Error::Timeout)
        );
        assert_eq!(ring.lock().stats.wakes, 1);
    }
    #[test]
    fn a_wake_after_the_ring_is_gone_is_a_no_op() {
        let ring = Arc::new(ring(1));
        let waker = ring.waker();
        let second = waker.clone();
        assert_eq!(Arc::strong_count(&ring), 1);
        assert_eq!(Arc::weak_count(&ring), 0);
        assert_eq!(Arc::strong_count(&waker.wait), 3);
        let observer = Arc::downgrade(&ring);
        drop(ring);
        assert!(observer.upgrade().is_none());
        assert_eq!(Arc::strong_count(&waker.wait), 2);
        waker.wake();
        second.wake();
        let elsewhere = std::thread::spawn(move || {
            second.wake();
            drop(second);
        });
        elsewhere.join().unwrap();
        waker.wake();
        assert_eq!(Arc::strong_count(&waker.wait), 1);
    }
    #[test]
    fn the_teardown_gates_pass_with_wakers_alive_and_waking() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let ring = Arc::new(ring(1));
        let kept = ring.waker();
        let busy = ring.waker();
        let quit = Arc::new(AtomicBool::new(false));
        let waking = {
            let quit = Arc::clone(&quit);
            std::thread::spawn(move || {
                let mut calls: u64 = 0;
                while !quit.load(Ordering::Acquire) {
                    busy.wake();
                    calls += 1;
                    std::thread::yield_now();
                }
                calls
            })
        };
        ring.lock().arm(0, clock::now_ms());
        let started = Instant::now();
        assert_eq!(ring.wait_idle(60), Err(Error::Timeout));
        assert!(clock::elapsed_ms(started) >= 50);
        let callback = Arc::clone(&ring);
        let event_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            let mut book = callback.lock();
            book.complete(0, (crate::ffi::USBD_STATUS_ABORTED, 0), clock::now_ms());
            callback.wait.cv.notify_all();
            drop(book);
            std::thread::sleep(Duration::from_millis(30));
            drop(callback);
        });
        assert_eq!(ring.wait_idle(DRAIN_TIMEOUT_MS), Ok(()));
        assert!(wait_last_reference(&ring, clock::SETTLE_TIMEOUT_MS));
        assert_eq!(Arc::strong_count(&ring), 1);
        let stats = ring.lock().stats;
        assert_eq!((stats.in_flight, stats.failures), (0, 1));
        let observer = Arc::downgrade(&ring);
        drop(ring);
        assert!(observer.upgrade().is_none());
        event_thread.join().unwrap();
        kept.wake();
        quit.store(true, Ordering::Release);
        assert!(waking.join().unwrap() > 0);
        kept.wake();
        assert_eq!(Arc::strong_count(&kept.wait), 1);
    }
}
