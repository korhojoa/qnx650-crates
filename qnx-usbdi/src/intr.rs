// SPDX-License-Identifier: Apache-2.0
//! `IntrIn`: an interrupt IN pipe with a ring of armed URBs.
//!
//! The mirror image of [`crate::bulk`]: every slot is queued on the pipe
//! at open (`URB_DIR_IN | URB_SHORT_XFER_OK`, no io-usb timeout), the completion callback records the slot's
//! status and length on a done queue and signals the condvar, and
//! [`IntrIn::read`] pops the oldest completion, copies the report out of
//! the slot's `usbd_alloc` buffer and re-arms the slot. A slot whose
//! completion was not `CMP` stays disarmed until [`IntrIn::rearm`], so a
//! `STALL` or a removal never spins the event thread.
//!
//! Ownership, the callback's cancellation guard, the drain at teardown
//! and the leak path are the bulk module's (its doc is the reference):
//! the URBs and buffers live in one `Arc<InState>`, each submission
//! carries `Arc::into_raw(clone)`, the callback reclaims it, and the
//! state is freed only on the caller's thread after `usbd_close_pipe`
//! answered `EOK` and the strong count was seen to be one. A slot's
//! buffer is written by io-usb only while the slot is armed and read by
//! `read` only after its completion was popped and before it is re-armed,
//! so the two never overlap.
//!
//! Copyright 2026. Licensed under the Apache License, Version 2.0.

use core::ffi::c_void;
use core::mem::ManuallyDrop;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

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

/// Ring depth when the caller has no opinion: two URBs queued so a
/// report that lands while the previous one is being copied out is not
/// refused (`NAK`) for a poll interval.
pub const DEFAULT_SLOTS: usize = 2;

/// Timeout of the `CLEAR_FEATURE(ENDPOINT_HALT)` fallback in
/// [`IntrIn::reset`].
pub const CLEAR_HALT_TIMEOUT_MS: u32 = 1000;

/// Maximum teardown drain time for completion callbacks on the event thread.
const DRAIN_TIMEOUT_MS: u32 = 1000;

/// Counters kept since the pipe was opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IntrStats {
    /// URBs `usbd_io` accepted.
    pub urbs: u64,
    /// Bytes the completed URBs carried.
    pub bytes: u64,
    /// Completions that were not `CMP`, plus submissions `usbd_io`
    /// refused.
    pub failures: u64,
    /// Longest arm-to-completion time seen, in milliseconds (the device's
    /// idle time counts, so this is not a latency figure for a busy
    /// device).
    pub max_latency_ms: u64,
    /// URBs armed and not yet completed.
    pub in_flight: usize,
    /// Completions popped by `read`.
    pub reports: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct SlotBook {
    armed: bool,
    submitted_ms: u64,
}

/// One completion, in the order the event thread delivered them.
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
    stats: IntrStats,
}

impl Book {
    fn new(slots: usize) -> Book {
        Book {
            slots: vec![SlotBook::default(); slots],
            done: VecDeque::with_capacity(slots),
            stats: IntrStats::default(),
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

    /// `usbd_io` refused the URB: the slot never went out.
    fn unarm(&mut self, idx: usize) {
        if let Some(slot) = self.slots.get_mut(idx) {
            slot.armed = false;
            self.stats.in_flight = self.stats.in_flight.saturating_sub(1);
            self.stats.urbs = self.stats.urbs.saturating_sub(1);
            self.stats.failures += 1;
        }
    }

    /// Slots `rearm` may submit: not in flight, and no completion of theirs
    /// waiting in `done`. A queued slot is re-armed by the `read` that pops
    /// it; arming it here as well would submit its URB twice.
    fn disarmed(&self) -> Vec<usize> {
        self.slots
            .iter()
            .enumerate()
            .filter(|&(i, s)| !s.armed && !self.done.iter().any(|d| d.idx == i))
            .map(|(i, _)| i)
            .collect()
    }

    /// Record a completion and queue it for `read`.
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

/// Everything the completion callback may touch (see the bulk module's
/// `RingState`).
pub(crate) struct InState {
    sys: Vec<sys::SlotSys>,
    book: Mutex<Book>,
    cv: Condvar,
    slot_bytes: usize,
}

// SAFETY: as `bulk::RingState`: `sys` is never mutated after
// construction; a slot's buffer is written by io-usb only between its
// arm and its completion callback and read by `IntrIn::read` (`&mut
// self` on a !Send/!Sync type) only after that completion was popped
// from the done queue under the mutex and before the slot is re-armed;
// the callback touches only the `Book` under the mutex and compares URB
// pointers; the URBs and buffers are freed once, on the caller's thread,
// after `usbd_close_pipe` returned `EOK` and the strong count was seen
// to be one.
unsafe impl Send for InState {}
// SAFETY: as above.
unsafe impl Sync for InState {}

impl InState {
    fn new(slots: usize, slot_bytes: usize) -> Result<InState> {
        let mut sys_slots = Vec::with_capacity(slots);
        for i in 0..slots {
            match sys::SlotSys::alloc(slot_bytes) {
                Ok(s) => sys_slots.push(s),
                Err(e) => {
                    crate::log!("usbdi: intr slot {i} of {slots} ({slot_bytes} bytes): {e}");
                    return Err(e);
                }
            }
        }
        Ok(InState {
            sys: sys_slots,
            book: Mutex::new(Book::new(slots)),
            cv: Condvar::new(),
            slot_bytes,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wait until no URB is in flight or `timeout_ms` passes (the
    /// teardown drain; not interruptible, the state must not be freed
    /// under a URB).
    fn wait_idle(&self, timeout_ms: u32) -> Result<()> {
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
        let mut book = self.lock();
        while !book.idle() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }
            book = self
                .cv
                .wait_timeout(book, remaining)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        Ok(())
    }
}

impl Drop for InState {
    fn drop(&mut self) {
        crate::vlog!(
            "usbdi: intr ring of {} slot(s) x {} bytes freed",
            self.sys.len(),
            self.slot_bytes
        );
    }
}

/// The `usbd_io` completion callback: as `bulk::bulk_complete`, with the
/// completion queued for `read` instead of only counted.
///
/// # Safety
/// `handle` must be the pointer `Arc::into_raw` produced for this very
/// submission; it is reclaimed exactly once here. `urb` is the URB
/// io-usb completed.
unsafe extern "C" fn intr_complete(urb: *mut UsbdUrb, _pipe: *mut UsbdPipe, handle: *mut c_void) {
    let _cancel = CancelGuard::new();
    if handle.is_null() {
        crate::log!("usbdi: intr completion with a null handle; ignored");
        return;
    }
    // SAFETY: handle is the Arc::into_raw pointer the arm passed for this
    // URB (the contract above); this is its one matching from_raw.
    let ring: Arc<InState> = unsafe { Arc::from_raw(handle.cast_const().cast::<InState>()) };
    let now = clock::now_ms();
    let (status, alen) = sys::urb_status(urb);
    let idx = ring.sys.iter().position(|s| s.owns_urb(urb));
    let mut book = ring.lock();
    match idx {
        Some(i) => {
            let done = book.complete(i, (status, alen), now);
            if status & USBD_URB_STATUS_MASK == USBD_STATUS_CMP {
                crate::vlog!(
                    "usbdi: intr slot {i}: CMP {alen} bytes after {} ms, {} queued",
                    done.latency_ms,
                    book.done.len()
                );
            } else {
                crate::log!(
                    "usbdi: intr slot {i}: status {status:#010x} ({}/{}) alen {alen} after {} ms; slot disarmed",
                    urb_status_name(status),
                    usb_status_name(status),
                    done.latency_ms
                );
            }
        }
        None => crate::log!("usbdi: intr completion for a URB not in the ring ({urb:?}); ignored"),
    }
    ring.cv.notify_all();
    drop(book);
    drop(ring);
    crate::activity::signal();
}

/// An interrupt IN pipe with its ring of armed URBs. `!Send`/`!Sync`
/// (it shares the device's `Rc`); only the completion path crosses
/// threads, and that is internal.
pub struct IntrIn {
    ring: ManuallyDrop<Arc<InState>>,
    pipe: sys::PipeSys,
    dev: Rc<DeviceShared>,
    ep: Endpoint,
}

impl IntrIn {
    pub(crate) fn open(dev: Rc<DeviceShared>, ep: Endpoint, slots: usize) -> Result<IntrIn> {
        if !(ep.is_interrupt() && ep.is_in()) {
            crate::log!(
                "usbdi: open_intr_in refused: endpoint {:#04x} attr {:#04x} is not interrupt IN",
                ep.address(),
                ep.attributes()
            );
            return Err(Error::NoEndpoint);
        }
        let slot_bytes = usize::from(ep.max_packet());
        if slots == 0 || slot_bytes == 0 {
            crate::log!("usbdi: open_intr_in refused: {slots} slot(s) x {slot_bytes} bytes");
            return Err(Error::Errno(EINVAL));
        }
        if dev.gone.is_set() {
            return Err(Error::Gone);
        }
        let pipe = dev.sys.open_pipe(ep)?;
        let ring = match InState::new(slots, slot_bytes) {
            Ok(r) => r,
            Err(e) => {
                if let Err(ce) = pipe.close() {
                    crate::log!("usbdi: close_pipe after a failed ring allocation: {ce}");
                }
                return Err(e);
            }
        };
        let mut intr = IntrIn {
            ring: ManuallyDrop::new(Arc::new(ring)),
            pipe,
            dev,
            ep,
        };
        crate::log!(
            "usbdi: interrupt IN endpoint {:#04x} (mps {}, interval {}) open: {slots} slot(s)",
            ep.address(),
            ep.max_packet(),
            ep.interval()
        );
        if let Err(e) = intr.reset() {
            crate::log!(
                "usbdi: pipe hygiene on endpoint {:#04x} failed: {e}; continuing",
                ep.address()
            );
        }
        let armed = intr.rearm()?;
        if armed == 0 {
            return Err(Error::Errno(EINVAL));
        }
        Ok(intr)
    }

    /// The endpoint this pipe was opened on.
    #[must_use]
    pub fn endpoint(&self) -> Endpoint {
        self.ep
    }

    /// Bytes one slot holds: the endpoint's `wMaxPacketSize`, the most
    /// one `read` returns.
    #[must_use]
    pub fn slot_bytes(&self) -> usize {
        self.ring.slot_bytes
    }

    /// Queue one slot on the pipe. The slot must be disarmed.
    fn arm(&self, idx: usize) -> Result<()> {
        let ring: &InState = &self.ring;
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
        match self
            .pipe
            .submit_intr_in(slot, len32, Some(intr_complete), handle, USBD_TIME_INFINITY)
        {
            Ok(()) => Ok(()),
            Err(e) => {
                // SAFETY: usbd_io refused the URB, so the callback will
                // never run for it and this is the handle's only owner.
                drop(unsafe { Arc::from_raw(handle.cast_const().cast::<InState>()) });
                ring.lock().unarm(idx);
                crate::log!("usbdi: intr slot {idx}: usbd_io refused: {e}");
                Err(e)
            }
        }
    }

    /// Arm every disarmed slot (all of them at open; after a failed
    /// completion, the ones that failed). Returns how many are armed
    /// after the call; the first refusal is logged and returned only when
    /// nothing is armed at all.
    ///
    /// # Errors
    /// [`Error::Gone`]; [`Error::Errno`] when `usbd_io` refused and no
    /// slot is in flight.
    pub fn rearm(&mut self) -> Result<usize> {
        if self.dev.gone.is_set() {
            return Err(Error::Gone);
        }
        let mut first_err = None;
        // The guard must be gone before `arm` takes the lock again: a
        // `for` over `self.ring.lock().disarmed()` keeps the temporary
        // guard alive for the whole loop and deadlocks when `arm` locks it.
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

    /// Wait, bounded by `timeout_ms`, for the oldest completed report,
    /// copy it into `buf` (at most `buf.len()` bytes) and re-arm its
    /// slot. A completion that was not `CMP` is returned as
    /// [`Error::Status`] once and leaves its slot disarmed; the caller
    /// decides ([`IntrIn::reset`] then [`IntrIn::rearm`] for a stall,
    /// giving up for a removal). The wait ends early with
    /// [`Error::Interrupted`] when a stop is requested ([`crate::stop`])
    /// and with [`Error::Gone`] once the device is removed and nothing is
    /// in flight any more.
    ///
    /// # Errors
    /// As above, plus [`Error::Timeout`] when nothing completed in time.
    pub fn read(&mut self, buf: &mut [u8], timeout_ms: u32) -> Result<usize> {
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
        let ring: &InState = &self.ring;
        let done = {
            let mut book = ring.lock();
            loop {
                if let Some(d) = book.done.pop_front() {
                    book.stats.reports += 1;
                    break d;
                }
                if self.dev.gone.is_set() && book.idle() {
                    return Err(Error::Gone);
                }
                if stop::requested() {
                    return Err(Error::Interrupted);
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(Error::Timeout);
                }
                book = ring
                    .cv
                    .wait_timeout(book, remaining.min(Duration::from_millis(STOP_SLICE_MS)))
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
        };
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
                "usbdi: intr slot {}: not re-armed ({e}); rearm() when the device is back",
                done.idx
            );
        }
        Ok(n)
    }

    /// `usbd_reset_pipe`, then the `CLEAR_FEATURE(ENDPOINT_HALT)`
    /// fallback (as [`crate::bulk::BulkOut::reset`]). URBs in flight
    /// complete `ABORTED` and come back through `read` as failures.
    ///
    /// # Errors
    /// [`Error::Gone`]; the fallback's error when both fail.
    pub fn reset(&mut self) -> Result<()> {
        self.dev.reset_pipe(&self.pipe, self.ep)
    }

    /// The counters.
    #[must_use]
    pub fn stats(&self) -> IntrStats {
        self.ring.lock().stats
    }
}

impl Drop for IntrIn {
    fn drop(&mut self) {
        let started = Instant::now();
        let in_flight = self.ring.lock().stats.in_flight;
        match self.pipe.abort() {
            Ok(()) => crate::vlog!(
                "usbdi: intr teardown: abort_pipe(endpoint {:#04x}) ok, {in_flight} in flight",
                self.ep.address()
            ),
            Err(e) => crate::log!(
                "usbdi: intr teardown: abort_pipe(endpoint {:#04x}) failed: {e}, {in_flight} in flight",
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
                    crate::log!("usbdi: intr teardown: {what} failed: {e}");
                    false
                }
            };
        let last = closed && wait_last_reference(&self.ring, clock::SETTLE_TIMEOUT_MS);
        if last {
            let stats = self.ring.lock().stats;
            crate::log!(
                "usbdi: intr teardown: drained and closed in {} ms ({} URB(s), {} report(s), {} bytes, {} failure(s))",
                clock::elapsed_ms(started),
                stats.urbs,
                stats.reports,
                stats.bytes,
                stats.failures
            );
            // SAFETY: this side holds the only reference (checked just
            // above), no callback can take a new one (the pipe is closed
            // and no arm runs during drop), so the state is dropped
            // exactly once, here, on the caller's thread, before
            // usbd_detach and usbd_disconnect.
            unsafe { ManuallyDrop::drop(&mut self.ring) };
        } else {
            let left = self.ring.lock().stats.in_flight;
            let refs = Arc::strong_count(&self.ring);
            crate::log!(
                "usbdi: intr teardown: {} after {} ms, {left} URB(s) in flight, {refs} ring ref(s): the ring is leaked (the memchunk destroy at usbd_disconnect reclaims the URBs and buffers){}",
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
    use crate::ffi::{USBD_STATUS_ABORTED, USBD_STATUS_CMP_ERR, USBD_STATUS_STALL};

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
            (
                b.stats.in_flight,
                b.stats.bytes,
                b.stats.failures,
                b.stats.max_latency_ms
            ),
            (0, 5, 1, 20)
        );
        assert!(b.idle());
    }

    #[test]
    fn a_queued_completion_is_not_offered_for_rearm() {
        let mut b = Book::new(2);
        b.arm(0, 0);
        b.arm(1, 0);
        b.complete(0, (USBD_STATUS_CMP_ERR | USBD_STATUS_STALL, 0), 1);
        // Slot 0 is done but not read yet: rearm must leave it to `read`.
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
        // An abort at teardown is a failure too, and the slot is free.
        b.arm(0, 0);
        b.complete(0, (USBD_STATUS_ABORTED, 0), 1);
        assert_eq!(b.stats.failures, 2);
        assert!(b.idle());
    }

    #[test]
    fn open_needs_an_interrupt_in_endpoint() {
        // A bulk OUT or an interrupt OUT endpoint is refused before any
        // pipe is opened (so this runs on the host).
        let bulk = Endpoint::new(7, 0x01, 0x02, 512, 0);
        let intr_out = Endpoint::new(7, 0x02, 0x03, 8, 10);
        let intr_in = Endpoint::new(7, 0x81, 0x03, 8, 10);
        assert!(!(bulk.is_interrupt() && bulk.is_in()));
        assert!(!(intr_out.is_interrupt() && intr_out.is_in()));
        assert!(intr_in.is_interrupt() && intr_in.is_in());
        assert_eq!(DEFAULT_SLOTS, 2);
    }
}
