//! Copyright 2026. Licensed under the Apache License, Version 2.0.
use crate::cancel::CancelGuard;
use crate::clock;
use crate::desc::Endpoint;
use crate::device::DeviceShared;
use crate::ffi::{
    urb_status_name, usb_status_name, UsbdPipe, UsbdUrb, EINVAL, USBD_STATUS_CMP,
    USBD_URB_STATUS_MASK,
};
use crate::sys;
use crate::{stop, Error, Result};
use core::ffi::c_void;
use core::mem::ManuallyDrop;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
pub const DEFAULT_SLOTS: usize = 4;
pub const DEFAULT_SLOT_BYTES: usize = 65_536;
pub const CLEAR_HALT_TIMEOUT_MS: u32 = 1000;
const DEFAULT_DRAIN_TIMEOUT_MS: u32 = 1000;
pub const STOP_SLICE_MS: u64 = 250;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BulkStats {
    pub urbs: u64,
    pub bytes: u64,
    pub failures: u64,
    pub max_latency_ms: u64,
    pub in_flight: usize,
}
/// # Errors
/// [`Error::Status`] for any other completion class. [`Error::Short`]
/// when the length differs.
pub fn judge(status: u32, alen: u32, requested: u32) -> Result<()> {
    if status & USBD_URB_STATUS_MASK != USBD_STATUS_CMP {
        return Err(Error::Status(status));
    }
    if alen != requested {
        return Err(Error::Short {
            want: usize::try_from(requested).unwrap_or(usize::MAX),
            got: usize::try_from(alen).unwrap_or(usize::MAX),
        });
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, Default)]
struct SlotBook {
    busy: bool,
    requested: u32,
    submitted_ms: u64,
    timeout_ms: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Completion {
    requested: u32,
    latency_ms: u64,
    timeout_ms: u32,
    result: Result<()>,
}
#[derive(Debug, Default)]
struct Book {
    slots: Vec<SlotBook>,
    first_failure: Option<Error>,
    stats: BulkStats,
}
impl Book {
    fn new(slots: usize) -> Book {
        Book {
            slots: vec![SlotBook::default(); slots],
            first_failure: None,
            stats: BulkStats::default(),
        }
    }
    fn free_slot(&self) -> Option<usize> {
        self.slots.iter().position(|s| !s.busy)
    }
    fn idle(&self) -> bool {
        self.stats.in_flight == 0
    }
    fn submit(&mut self, idx: usize, requested: u32, now_ms: u64, timeout_ms: u32) {
        if let Some(slot) = self.slots.get_mut(idx) {
            *slot = SlotBook {
                busy: true,
                requested,
                submitted_ms: now_ms,
                timeout_ms,
            };
            self.stats.in_flight += 1;
            self.stats.urbs += 1;
            self.stats.bytes += u64::from(requested);
        }
    }
    fn unsubmit(&mut self, idx: usize) {
        if let Some(slot) = self.slots.get_mut(idx) {
            slot.busy = false;
            self.stats.in_flight = self.stats.in_flight.saturating_sub(1);
            self.stats.urbs = self.stats.urbs.saturating_sub(1);
            self.stats.bytes = self.stats.bytes.saturating_sub(u64::from(slot.requested));
            self.stats.failures += 1;
        }
    }
    fn complete(&mut self, idx: usize, (status, alen): (u32, u32), now_ms: u64) -> Completion {
        let slot = self.slots.get(idx).copied().unwrap_or_default();
        if let Some(s) = self.slots.get_mut(idx) {
            s.busy = false;
        }
        self.stats.in_flight = self.stats.in_flight.saturating_sub(1);
        let latency_ms = now_ms.saturating_sub(slot.submitted_ms);
        self.stats.max_latency_ms = self.stats.max_latency_ms.max(latency_ms);
        let result = judge(status, alen, slot.requested);
        if let Err(e) = &result {
            self.stats.failures += 1;
            if self.first_failure.is_none() {
                self.first_failure = Some(e.clone());
            }
        }
        Completion {
            requested: slot.requested,
            latency_ms,
            timeout_ms: slot.timeout_ms,
            result,
        }
    }
    fn take_failure(&mut self) -> Option<Error> {
        self.first_failure.take()
    }
}
pub(crate) struct RingState {
    sys: Vec<sys::SlotSys>,
    book: Mutex<Book>,
    cv: Condvar,
    slot_bytes: usize,
}
// SAFETY: The ring shares state with the libusbdi event thread through the callback.
// The raw URB and buffer pointers require explicit `Send` and `Sync` implementations.
// - `sys` stays unchanged after construction. The callback compares URB pointers without the mutex.
// - `BulkOut::write` has exclusive access through `&mut self`. The method marks the slot busy before it writes the buffer.
// - io-usb reads the buffer between `usbd_io` and the callback. A rejected submission clears the busy flag without a buffer read.
// - The callback uses the mutex for `Book`. The callback compares URB pointers but does not access the buffer.
// - `Drop` frees URBs and buffers once on the caller thread. It requires `EOK` from `usbd_close_pipe` and a strong reference count of one.
// - The per-pipe counter becomes zero after the last callback returns and releases its clone.
// - The leak procedure frees nothing. No callback frees memory.
// - The ring stays live during transfers and while `write` holds `self.ring`.
unsafe impl Send for RingState {}
// SAFETY: The conditions are the same as above.
// The mutex protects `Book`.
// The raw pointers in `sys` are shared read-only data.
// The busy flag gives exclusive access for buffer writes.
unsafe impl Sync for RingState {}
impl RingState {
    fn new(slots: usize, slot_bytes: usize) -> Result<RingState> {
        let mut sys_slots = Vec::with_capacity(slots);
        for i in 0..slots {
            match sys::SlotSys::alloc(slot_bytes) {
                Ok(s) => sys_slots.push(s),
                Err(e) => {
                    crate::log!("usbdi: ring slot {i} of {slots} ({slot_bytes} bytes): {e}");
                    return Err(e);
                }
            }
        }
        Ok(RingState {
            sys: sys_slots,
            book: Mutex::new(Book::new(slots)),
            cv: Condvar::new(),
            slot_bytes,
        })
    }
    fn lock(&self) -> MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// # Errors
    /// [`Error::Timeout`] with URBs still in flight at the deadline.
    /// [`Error::Interrupted`] when interruptible and a stop is requested.
    fn wait_idle(&self, timeout_ms: u32, interruptible: bool) -> Result<()> {
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
        let mut book = self.lock();
        while !book.idle() {
            if interruptible && stop::requested() {
                return Err(Error::Interrupted);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }
            let slice = if interruptible {
                remaining.min(Duration::from_millis(STOP_SLICE_MS))
            } else {
                remaining
            };
            book = self
                .cv
                .wait_timeout(book, slice)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        Ok(())
    }
}
impl Drop for RingState {
    fn drop(&mut self) {
        crate::vlog!(
            "usbdi: ring of {} slot(s) x {} bytes freed",
            self.sys.len(),
            self.slot_bytes
        );
    }
}
/// # Safety
/// `handle` must be the pointer `Arc::into_raw` produced for this very
/// submission (what `BulkOut::write` passes). It is reclaimed exactly
/// once here. `urb` is the URB io-usb completed.
unsafe extern "C" fn bulk_complete(urb: *mut UsbdUrb, _pipe: *mut UsbdPipe, handle: *mut c_void) {
    let _cancel = CancelGuard::new();
    if handle.is_null() {
        crate::log!("usbdi: bulk completion with a null handle; ignored");
        return;
    }
    // SAFETY: handle is the Arc::into_raw pointer submit passed for this
    // URB (the contract above). This is its one matching from_raw.
    let ring: Arc<RingState> = unsafe { Arc::from_raw(handle.cast_const().cast::<RingState>()) };
    let now = clock::now_ms();
    let (status, alen) = sys::urb_status(urb);
    let idx = ring.sys.iter().position(|s| s.owns_urb(urb));
    let mut book = ring.lock();
    match idx {
        Some(i) => {
            let done = book.complete(i, (status, alen), now);
            let left = book.stats.in_flight;
            match &done.result {
                Ok(()) => crate::vlog!(
                    "usbdi: bulk slot {i}: CMP {} bytes in {} ms (timeout {} ms), {left} in flight",
                    done.requested,
                    done.latency_ms,
                    done.timeout_ms
                ),
                Err(e) => crate::log!(
                    "usbdi: bulk slot {i}: {e}: status {status:#010x} ({}/{}) alen {alen} of {} after {} ms (timeout {} ms), {left} in flight",
                    urb_status_name(status),
                    usb_status_name(status),
                    done.requested,
                    done.latency_ms,
                    done.timeout_ms
                ),
            }
        }
        None => crate::log!("usbdi: bulk completion for a URB not in the ring ({urb:?}); ignored"),
    }
    ring.cv.notify_all();
    drop(book);
    drop(ring);
}
pub struct BulkOut {
    ring: ManuallyDrop<Arc<RingState>>,
    pipe: sys::PipeSys,
    dev: Rc<DeviceShared>,
    ep: Endpoint,
    drain_timeout_ms: u32,
}
impl BulkOut {
    pub(crate) fn open(
        dev: Rc<DeviceShared>,
        ep: Endpoint,
        slots: usize,
        slot_bytes: usize,
    ) -> Result<BulkOut> {
        if !(ep.is_bulk() && ep.is_out()) {
            crate::log!(
                "usbdi: open_bulk_out refused: endpoint {:#04x} attr {:#04x} is not bulk OUT",
                ep.address(),
                ep.attributes()
            );
            return Err(Error::NoEndpoint);
        }
        if slots == 0 || slot_bytes == 0 || u32::try_from(slot_bytes).is_err() {
            crate::log!("usbdi: open_bulk_out refused: {slots} slot(s) x {slot_bytes} bytes");
            return Err(Error::Errno(EINVAL));
        }
        if dev.gone.is_set() {
            return Err(Error::Gone);
        }
        let pipe = dev.sys.open_pipe(ep)?;
        let ring = match RingState::new(slots, slot_bytes) {
            Ok(r) => r,
            Err(e) => {
                if let Err(ce) = pipe.close() {
                    crate::log!("usbdi: close_pipe after a failed ring allocation: {ce}");
                }
                return Err(e);
            }
        };
        let mut out = BulkOut {
            ring: ManuallyDrop::new(Arc::new(ring)),
            pipe,
            dev,
            ep,
            drain_timeout_ms: DEFAULT_DRAIN_TIMEOUT_MS,
        };
        crate::log!(
            "usbdi: bulk OUT endpoint {:#04x} (mps {}) open: {slots} slot(s) x {slot_bytes} bytes",
            ep.address(),
            ep.max_packet()
        );
        if let Err(e) = out.reset() {
            crate::log!(
                "usbdi: pipe hygiene on endpoint {:#04x} failed: {e}; continuing",
                ep.address()
            );
        }
        Ok(out)
    }
    #[must_use]
    pub fn endpoint(&self) -> Endpoint {
        self.ep
    }
    #[must_use]
    pub fn slot_bytes(&self) -> usize {
        self.ring.slot_bytes
    }
    #[must_use]
    pub fn slots(&self) -> usize {
        self.ring.sys.len()
    }
    /// # Errors
    /// [`Error::Gone`] when the device is gone.
    /// [`Error::Errno`] (`EINVAL`) when `data` is larger than a slot.
    /// [`Error::Timeout`] when no slot becomes free before the deadline.
    /// [`Error::Interrupted`] for a stop request during the wait.
    /// A previous completion error ([`Error::Status`] or [`Error::Short`]).
    /// [`Error::Errno`] when `usbd_io` rejects the URB.
    pub fn write(&mut self, data: &[u8], timeout_ms: u32) -> Result<()> {
        if self.dev.gone.is_set() {
            crate::log!(
                "usbdi: bulk write of {} bytes refused: device removed",
                data.len()
            );
            return Err(Error::Gone);
        }
        let len = data.len();
        let Ok(len32) = u32::try_from(len) else {
            return Err(Error::Errno(EINVAL));
        };
        if len > self.ring.slot_bytes {
            crate::log!(
                "usbdi: bulk write of {len} bytes refused: slot holds {}",
                self.ring.slot_bytes
            );
            return Err(Error::Errno(EINVAL));
        }
        self.drain_timeout_ms = timeout_ms;
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
        let ring: &RingState = &self.ring;
        let mut book = ring.lock();
        let idx = loop {
            if let Some(e) = book.take_failure() {
                crate::log!("usbdi: bulk write: reporting an earlier completion failure: {e}");
                return Err(e);
            }
            if let Some(i) = book.free_slot() {
                break i;
            }
            if stop::requested() {
                crate::log!(
                    "usbdi: bulk write: stop requested while waiting for a slot ({} in flight)",
                    book.stats.in_flight
                );
                return Err(Error::Interrupted);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                crate::log!(
                    "usbdi: bulk write: no free slot within {timeout_ms} ms ({} in flight)",
                    book.stats.in_flight
                );
                return Err(Error::Timeout);
            }
            book = ring
                .cv
                .wait_timeout(book, remaining.min(Duration::from_millis(STOP_SLICE_MS)))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        };
        book.submit(idx, len32, clock::now_ms(), timeout_ms);
        drop(book);
        let Some(slot) = ring.sys.get(idx) else {
            return Err(Error::Errno(EINVAL));
        };
        slot.fill(data);
        let handle = Arc::into_raw(Arc::clone(&self.ring))
            .cast_mut()
            .cast::<c_void>();
        match self
            .pipe
            .submit_bulk_out(slot, len32, Some(bulk_complete), handle, timeout_ms)
        {
            Ok(()) => {
                crate::vlog!(
                    "usbdi: bulk slot {idx}: {len} bytes queued (timeout {timeout_ms} ms)"
                );
                Ok(())
            }
            Err(e) => {
                // SAFETY: usbd_io refused the URB, so the callback will
                // never run for it and this is the handle's only owner.
                drop(unsafe { Arc::from_raw(handle.cast_const().cast::<RingState>()) });
                ring.lock().unsubmit(idx);
                crate::log!("usbdi: bulk slot {idx}: usbd_io refused {len} bytes: {e}");
                Err(e)
            }
        }
    }
    /// # Errors
    /// [`Error::Timeout`] when URBs stay in progress until the deadline.
    /// [`Error::Interrupted`] for a stop request while URBs are in progress.
    /// Otherwise, the first recorded completion error ([`Error::Status`] or [`Error::Short`]).
    /// `write` returns submission errors without a record for this operation.
    pub fn flush(&mut self, timeout_ms: u32) -> Result<()> {
        let started = Instant::now();
        if let Err(e) = self.ring.wait_idle(timeout_ms, true) {
            let left = self.ring.lock().stats.in_flight;
            match e {
                Error::Interrupted => crate::log!(
                    "usbdi: flush: stop requested with {left} URB(s) in flight after {} ms",
                    clock::elapsed_ms(started)
                ),
                _ => crate::log!(
                    "usbdi: flush: {left} URB(s) still in flight after {} ms (timeout {timeout_ms} ms)",
                    clock::elapsed_ms(started)
                ),
            }
            return Err(e);
        }
        let failure = self.ring.lock().take_failure();
        crate::vlog!(
            "usbdi: flush: drained in {} ms{}",
            clock::elapsed_ms(started),
            if failure.is_some() {
                ", with a failure"
            } else {
                ""
            }
        );
        failure.map_or(Ok(()), Err)
    }
    /// # Errors
    /// [`Error::Gone`] when the device is gone.
    /// The fallback error when the two methods fail.
    pub fn reset(&mut self) -> Result<()> {
        if self.dev.gone.is_set() {
            return Err(Error::Gone);
        }
        let in_flight = self.ring.lock().stats.in_flight;
        if in_flight > 0 {
            crate::log!(
                "usbdi: reset of endpoint {:#04x} with {in_flight} URB(s) in flight: they complete ABORTED",
                self.ep.address()
            );
        }
        self.dev.reset_pipe(&self.pipe, self.ep)
    }
    #[must_use]
    pub fn stats(&self) -> BulkStats {
        self.ring.lock().stats
    }
}
impl Drop for BulkOut {
    fn drop(&mut self) {
        let started = Instant::now();
        let in_flight = self.ring.lock().stats.in_flight;
        match self.pipe.abort() {
            Ok(()) => crate::vlog!(
                "usbdi: bulk teardown: abort_pipe(endpoint {:#04x}) ok, {in_flight} in flight",
                self.ep.address()
            ),
            Err(e) => crate::log!(
                "usbdi: bulk teardown: abort_pipe(endpoint {:#04x}) failed: {e}, {in_flight} in flight",
                self.ep.address()
            ),
        }
        let bound_ms = if stop::requested() {
            self.drain_timeout_ms.min(DEFAULT_DRAIN_TIMEOUT_MS)
        } else {
            self.drain_timeout_ms
        };
        let drained = self.ring.wait_idle(bound_ms, false).is_ok();
        let closed = drained && self.close_pipe_settled();
        let last = closed && wait_last_reference(&self.ring, clock::SETTLE_TIMEOUT_MS);
        if last {
            let stats = self.ring.lock().stats;
            crate::log!(
                "usbdi: bulk teardown: drained and closed in {} ms ({} URB(s), {} bytes, {} failure(s), max latency {} ms)",
                clock::elapsed_ms(started),
                stats.urbs,
                stats.bytes,
                stats.failures,
                stats.max_latency_ms
            );
            // SAFETY: This thread holds the only ring reference.
            // The pipe is closed and no write runs during `Drop`, so callbacks cannot take new references.
            // This thread drops the ring once before `usbd_detach` and `usbd_disconnect`.
            // No method uses `self` after that operation.
            unsafe { ManuallyDrop::drop(&mut self.ring) };
        } else {
            let left = self.ring.lock().stats.in_flight;
            let refs = Arc::strong_count(&self.ring);
            crate::log!(
                "usbdi: bulk teardown: {} after {} ms (bound {bound_ms} ms{}), {left} URB(s) in flight, {refs} ring ref(s): the ring is leaked (the memchunk destroy at usbd_disconnect reclaims the URBs and buffers){}",
                if !drained {
                    "drain timed out"
                } else if !closed {
                    "close_pipe stayed EBUSY"
                } else {
                    "a callback still holds the ring"
                },
                clock::elapsed_ms(started),
                if bound_ms == self.drain_timeout_ms {
                    ""
                } else {
                    ", capped after a stop request"
                },
                if closed {
                    ""
                } else {
                    "; close_pipe is left to usbd_detach"
                }
            );
        }
    }
}
impl BulkOut {
    fn close_pipe_settled(&self) -> bool {
        let what = format!("usbd_close_pipe(endpoint {:#04x})", self.ep.address());
        match clock::retry_while_busy(&what, clock::SETTLE_TIMEOUT_MS, || self.pipe.close()) {
            Ok(()) => {
                crate::vlog!("usbdi: bulk teardown: {what} ok");
                true
            }
            Err(e) => {
                crate::log!("usbdi: bulk teardown: {what} failed: {e}");
                false
            }
        }
    }
}
pub(crate) fn wait_last_reference<T>(arc: &Arc<T>, timeout_ms: u32) -> bool {
    let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
    loop {
        if Arc::strong_count(arc) == 1 {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(clock::SETTLE_STEP_MS));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::{
        USBD_STATUS_ABORTED, USBD_STATUS_CMP_ERR, USBD_STATUS_NOT_ACCESSED, USBD_STATUS_STALL,
        USBD_STATUS_TIMEOUT,
    };
    #[test]
    fn judge_requires_cmp_and_the_full_length() {
        assert_eq!(judge(USBD_STATUS_CMP, 512, 512), Ok(()));
        assert_eq!(
            judge(USBD_STATUS_CMP, 500, 512),
            Err(Error::Short {
                want: 512,
                got: 500
            })
        );
        assert_eq!(
            judge(USBD_STATUS_CMP, 513, 512),
            Err(Error::Short {
                want: 512,
                got: 513
            })
        );
        let stall = USBD_STATUS_CMP_ERR | USBD_STATUS_STALL;
        assert_eq!(judge(stall, 0, 512), Err(Error::Status(stall)));
        assert_eq!(
            judge(USBD_STATUS_ABORTED, 0, 512),
            Err(Error::Status(USBD_STATUS_ABORTED))
        );
        assert_eq!(judge(0, 0, 0), Err(Error::Status(0)));
    }
    #[test]
    fn slots_cycle_through_free_busy_free_in_order() {
        let mut b = Book::new(2);
        assert!(b.idle());
        assert_eq!(b.free_slot(), Some(0));
        b.submit(0, 100, 10, 1000);
        assert_eq!(b.free_slot(), Some(1));
        b.submit(1, 200, 12, 1000);
        assert_eq!(b.free_slot(), None);
        assert_eq!(b.stats.in_flight, 2);
        assert_eq!((b.stats.urbs, b.stats.bytes), (2, 300));
        let done = b.complete(0, (USBD_STATUS_CMP, 100), 15);
        assert_eq!(done.result, Ok(()));
        assert_eq!(
            (done.requested, done.latency_ms, done.timeout_ms),
            (100, 5, 1000)
        );
        assert_eq!(b.free_slot(), Some(0));
        assert!(!b.idle());
        let done = b.complete(1, (USBD_STATUS_CMP, 200), 40);
        assert_eq!(done.latency_ms, 28);
        assert!(b.idle());
        assert_eq!(b.stats.max_latency_ms, 28);
        assert_eq!(b.stats.failures, 0);
        assert_eq!(b.take_failure(), None);
    }
    #[test]
    fn the_first_failure_is_kept_until_taken_and_aborted_completes() {
        let mut b = Book::new(3);
        b.submit(0, 10, 0, 500);
        b.submit(1, 10, 0, 500);
        b.submit(2, 10, 0, 500);
        let short = b.complete(0, (USBD_STATUS_CMP, 9), 1);
        assert_eq!(short.result, Err(Error::Short { want: 10, got: 9 }));
        let aborted = b.complete(1, (USBD_STATUS_ABORTED, 0), 2);
        assert_eq!(aborted.result, Err(Error::Status(USBD_STATUS_ABORTED)));
        let timed_out = USBD_STATUS_TIMEOUT | USBD_STATUS_NOT_ACCESSED;
        let lost = b.complete(2, (timed_out, 0), 3);
        assert_eq!(lost.result, Err(Error::Status(0x0400_000F)));
        assert!(b.idle());
        assert_eq!(b.stats.failures, 3);
        assert_eq!(b.take_failure(), Some(Error::Short { want: 10, got: 9 }));
        assert_eq!(b.take_failure(), None);
        b.submit(0, 10, 5, 500);
        assert_eq!((b.stats.urbs, b.stats.bytes), (4, 40));
        b.unsubmit(0);
        assert!(b.idle());
        assert_eq!(b.free_slot(), Some(0));
        assert_eq!(b.stats.failures, 4);
        assert_eq!((b.stats.urbs, b.stats.bytes), (3, 30));
        assert_eq!(b.take_failure(), None);
        b.submit(0, 10, 6, 500);
        b.unsubmit(0);
        b.submit(0, 6, 7, 500);
        let ok = b.complete(0, (USBD_STATUS_CMP, 6), 9);
        assert_eq!(ok.result, Ok(()));
        assert_eq!(b.take_failure(), None);
        assert_eq!(b.stats.failures, 5);
        assert_eq!((b.stats.urbs, b.stats.bytes), (4, 36));
        b.submit(9, 1, 0, 1);
        assert!(b.idle());
        let none = b.complete(9, (USBD_STATUS_CMP, 0), 0);
        assert_eq!(none.result, Ok(()));
    }
    #[test]
    fn a_stalled_completion_is_judged_by_its_status_word() {
        let mut b = Book::new(1);
        b.submit(0, 4096, 0, 1000);
        let stall = USBD_STATUS_CMP_ERR | USBD_STATUS_STALL;
        let done = b.complete(0, (stall, 0), 3);
        assert_eq!(stall, 0x0200_0004);
        assert_eq!(done.result, Err(Error::Status(0x0200_0004)));
        assert_eq!(done.requested, 4096);
        assert!(b.idle());
        assert_eq!(b.stats.failures, 1);
        assert_eq!(b.take_failure(), Some(Error::Status(stall)));
    }
    #[test]
    fn defaults_match_the_design() {
        assert_eq!(DEFAULT_SLOTS, 4);
        assert_eq!(DEFAULT_SLOT_BYTES, 65_536);
        assert_eq!(CLEAR_HALT_TIMEOUT_MS, 1000);
        assert_eq!(DEFAULT_DRAIN_TIMEOUT_MS, 1000);
        assert_eq!(STOP_SLICE_MS, 250);
        assert_eq!(BulkStats::default().in_flight, 0);
    }
    #[test]
    fn the_ring_is_freed_only_once_this_side_is_the_last_reference() {
        let ring = Arc::new(());
        let callback = Arc::clone(&ring);
        assert_eq!(Arc::strong_count(&ring), 2);
        assert!(!wait_last_reference(&ring, 0));
        assert!(!wait_last_reference(&ring, 20));
        let event_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            drop(callback);
        });
        let started = Instant::now();
        assert!(wait_last_reference(&ring, 5000));
        assert!(clock::elapsed_ms(started) < 4000);
        event_thread.join().unwrap();
        assert_eq!(Arc::strong_count(&ring), 1);
        assert!(wait_last_reference(&ring, 0));
    }
    #[test]
    fn a_stop_request_ends_an_interruptible_wait() {
        let ring = RingState {
            sys: Vec::new(),
            book: Mutex::new(Book::new(1)),
            cv: Condvar::new(),
            slot_bytes: 0,
        };
        assert_eq!(ring.wait_idle(0, true), Ok(()));
        ring.lock().submit(0, 10, 0, 1000);
        assert!(!ring.lock().idle());
        assert!(!stop::requested());
        let started = Instant::now();
        assert_eq!(ring.wait_idle(60, true), Err(Error::Timeout));
        assert!(clock::elapsed_ms(started) >= 50);
        assert_eq!(ring.wait_idle(10, false), Err(Error::Timeout));
        let requester = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            stop::request();
        });
        let started = Instant::now();
        assert_eq!(ring.wait_idle(10_000, true), Err(Error::Interrupted));
        let took = clock::elapsed_ms(started);
        assert!((50..2000).contains(&took), "{took} ms");
        requester.join().unwrap();
        assert!(stop::requested());
        let started = Instant::now();
        assert_eq!(ring.wait_idle(10_000, true), Err(Error::Interrupted));
        assert!(clock::elapsed_ms(started) < STOP_SLICE_MS);
        let started = Instant::now();
        assert_eq!(ring.wait_idle(60, false), Err(Error::Timeout));
        assert!(clock::elapsed_ms(started) >= 50);
        let done = ring.lock().complete(0, (USBD_STATUS_CMP, 10), 5);
        assert_eq!(done.result, Ok(()));
        assert_eq!(ring.wait_idle(0, true), Ok(()));
        assert_eq!(ring.wait_idle(0, false), Ok(()));
    }
}
