// SPDX-License-Identifier: Apache-2.0
//! `BulkOut`: a bulk OUT pipe with a ring of queued URBs.
//!
//! Bulk with a NULL callback returns immediately on this io-usb and
//! completes in the background (measured: `usbd_io` returns `EOK` with
//! the URB still `INPROG`; the transfer lands later). A client that frees
//! the URB or buffer after that return corrupts in-flight state. So the
//! port uses the async form: every submission carries a completion
//! callback, and nothing is freed until every callback has run.
//!
//! Ownership (contract item 9): the URBs, the `usbd_alloc` buffers, the
//! bookkeeping mutex and the condvar live in one `Arc<RingState>`. Each
//! `usbd_io` passes `Arc::into_raw(clone)` as the handle; the callback
//! does `Arc::from_raw`, records the slot's status and length, frees the
//! slot, signals the condvar regardless of status (`ABORTED` is a
//! completion too) and releases its clone. `RingState::drop` frees the
//! URBs and buffers, and runs only on the caller's thread, only after
//! every callback has released its clone, so a callback can never touch
//! freed memory and nothing is ever freed on libusbdi's event thread.
//! The callback runs with thread cancellation disabled (`cancel` module
//! doc): `usbd_disconnect`'s `pthread_cancel` of the event thread must
//! not end it inside the `Book` mutex or the stderr lock.
//!
//! The drain condition (in-flight == 0) is set INSIDE the callback and
//! so precedes libusbdi's own accounting by the callback's return: the
//! event thread decrements the per-pipe counter and then the per-device
//! counter only after the client callback returned, and until then
//! `usbd_close_pipe` and `usbd_detach` answer `EBUSY` and do nothing
//! \[RE: SDP `libusbdi.so.2` `usbdi_deliver_event` @0x8004-0x8014 calls
//! the callback, @0x8018-0x8020 `atomic_sub(pipe+12, 1)`, @0x8024-0x8030
//! `usbdi_synchronise(dev, 1, -1)`; `usbd_close_pipe` @0x5800-0x580c;
//! `usbd_detach` @0x5e78-0x5e84\]. Teardown: `usbd_abort_pipe`; wait for
//! in-flight == 0 bounded by the last write timeout; `usbd_close_pipe`
//! retried on `EBUSY` (`EOK` proves every callback has returned, so
//! this side's reference is the last one, which is checked before the
//! ring is dropped); drop the ring, on this thread, before the device
//! is detached and the connection closed. On a drain timeout, or when
//! `close_pipe` stays `EBUSY` at its bound, log it, keep this side's
//! reference (the ring is leaked; a straggling callback can only
//! decrement the count, and libusbdi's memchunk destroy at
//! `usbd_disconnect` reclaims the URBs and buffers) and leave the pipe
//! to `usbd_detach`, which closes leftover pipes itself. A ring must
//! never be released to the event thread: `usbd_disconnect` destroys
//! the pools `usbd_free` / `usbd_free_urb` free into before it cancels
//! that thread \[RE: @0x7b4c-0x7b58, then @0x7bf0-0x7bfc\].
//! `usbd_abort_pipe` returning does not mean the callbacks have run:
//! they run on libusbdi's event thread, so the drain is mandatory.
//!
//! A stop request ([`crate::stop`], what a `SIGTERM` handler makes) ends
//! the two waits a caller can sit in, `write`'s wait for a free slot and
//! `flush`, with [`Error::Interrupted`] within [`STOP_SLICE_MS`]; the
//! teardown drain is not interrupted but capped to
//! `DEFAULT_DRAIN_TIMEOUT_MS` once a stop is requested, so the whole
//! drain is bounded during shutdown. A ring with pending callbacks is leaked:
//! a URB the HCD never completes still cannot be freed.
//!
//! io-usb queues client URBs per endpoint and hands the HCD one at a
//! time in submission order (measured for bulk IN on dwcotg with 8 URBs;
//! not verified for EHCI), so several writes may be outstanding
//! and their order on the wire is their `write` order. Ordering holds
//! within this one pipe only; a control transfer that must follow queued
//! bulk data needs a `flush` first.
//!
//! The bookkeeping (`Book`) is pure and unit-tested on the host: slot
//! choice, first-failure capture, the `CMP`-and-length judgement, the
//! statistics. The URBs themselves exist only on the target.
//!
//! Copyright 2026. Licensed under the Apache License, Version 2.0.

use core::ffi::c_void;
use core::mem::ManuallyDrop;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

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

/// Ring depth when the caller has no opinion (`dlotest --slots`).
pub const DEFAULT_SLOTS: usize = 4;

/// Slot size when the caller has no opinion: one libdlo command buffer
/// (`dlotest --urb-bytes`).
pub const DEFAULT_SLOT_BYTES: usize = 65_536;

/// Timeout of the `CLEAR_FEATURE(ENDPOINT_HALT)` fallback (the GIP
/// shim's 1000 ms).
pub const CLEAR_HALT_TIMEOUT_MS: u32 = 1000;

/// Drain bound at teardown before any `write` has set one, and the
/// bound the teardown drain is capped to once a stop is requested
/// (the `EBUSY` retries of `close_pipe` and `detach`,
/// [`clock::SETTLE_TIMEOUT_MS`] each, cost nothing on a clean drain and
/// are spent only while libusbdi's counters lag the last callback).
const DEFAULT_DRAIN_TIMEOUT_MS: u32 = 1000;

/// Longest one condvar wait lasts inside an interruptible wait
/// (`write`'s slot wait, `flush`): how late a stop request is seen at
/// worst when the signal was delivered to a thread other than the
/// waiting one (which thread QNX picks for a process-directed signal is
/// unverified; on the waiting thread the wait returns at once).
pub const STOP_SLICE_MS: u64 = 250;

/// Counters kept since the pipe was opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BulkStats {
    /// URBs `usbd_io` accepted.
    pub urbs: u64,
    /// Bytes requested across them.
    pub bytes: u64,
    /// Completions that were not `CMP` with the full length, plus
    /// submissions `usbd_io` refused.
    pub failures: u64,
    /// Longest submit-to-completion time seen, in milliseconds.
    pub max_latency_ms: u64,
    /// URBs submitted and not yet completed.
    pub in_flight: usize,
}

/// Judge one completion: `CMP` and every requested byte moved.
///
/// # Errors
/// [`Error::Status`] for any other completion class; [`Error::Short`]
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

/// What a completion changed, for the log line.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Completion {
    requested: u32,
    latency_ms: u64,
    timeout_ms: u32,
    result: Result<()>,
}

/// The ring's state behind the mutex: which slots are in flight, the
/// first failure not yet reported, and the counters.
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

    /// `usbd_io` refused the URB: the slot never went out. The failure
    /// is counted and the URB and its bytes come back out of the
    /// accepted totals (`stats.urbs`, `stats.bytes`), which `submit`
    /// added before the call. The error is the caller's return value,
    /// not a recorded failure: `write` already returned it, and
    /// recording it here would report it a second time from the next
    /// `write` or `flush`.
    fn unsubmit(&mut self, idx: usize) {
        if let Some(slot) = self.slots.get_mut(idx) {
            slot.busy = false;
            self.stats.in_flight = self.stats.in_flight.saturating_sub(1);
            self.stats.urbs = self.stats.urbs.saturating_sub(1);
            self.stats.bytes = self.stats.bytes.saturating_sub(u64::from(slot.requested));
            self.stats.failures += 1;
        }
    }

    /// Record a completion (any status word; `usbd_urb_status` always
    /// yields one, see `sys::urb_status`) and free the slot.
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

/// Everything the completion callback may touch. Its `Drop` frees the
/// URBs and buffers, so it runs only on the caller's thread, after
/// `usbd_close_pipe` proved every callback has returned and released
/// its reference; the leak path never runs it.
pub(crate) struct RingState {
    sys: Vec<sys::SlotSys>,
    book: Mutex<Book>,
    cv: Condvar,
    slot_bytes: usize,
}

// SAFETY: the ring is shared with libusbdi's event thread through the
// completion callback, and the raw URB and buffer pointers in `sys` are
// what make it !Send/!Sync by default. They are sound to share because:
// (1) `sys` is never mutated after construction, so the callback's
// owns_urb pointer compare reads it without the mutex; (2) a slot's
// buffer is written only by `BulkOut::write` (`&mut self` on a
// !Send/!Sync type), after it has marked the slot busy under the mutex
// (`Book::submit`) and before `usbd_io`, so the write's exclusivity
// comes from the busy flag, not from holding the mutex during the copy;
// io-usb reads the buffer only between `usbd_io` and the completion
// callback, and a refused `usbd_io` clears busy under the mutex
// (`unsubmit`) without io-usb ever having read it; (3) the callback
// touches only the `Book` under the mutex and compares URB pointers,
// never a buffer; (4) the URBs and buffers are freed once, on the
// caller's thread, in `Drop for BulkOut`, after `usbd_close_pipe`
// returned `EOK` (libusbdi's per-pipe counter reaches zero only after
// the last callback returned, i.e. after it released its clone) and
// the strong count was seen to be one; the leak path never frees, so
// no callback ever frees, and nothing is freed while a URB is in
// flight or while `write` holds `self.ring`.
unsafe impl Send for RingState {}
// SAFETY: as above; the mutable state is the `Book` behind the mutex,
// the raw pointers in `sys` are read-only shared data, and the buffer
// write is exclusive through the busy flag.
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

    /// Wait until no URB is in flight or `timeout_ms` passes. An
    /// `interruptible` wait also returns once a stop is requested
    /// ([`crate::stop`]), polling the flag every [`STOP_SLICE_MS`] so
    /// the flag is seen whichever thread the signal landed on (a handler
    /// stores it without a notify); the teardown drain waits the plain
    /// way, the ring must not be freed under a URB.
    ///
    /// # Errors
    /// [`Error::Timeout`] with URBs still in flight at the deadline;
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

/// The `usbd_io` completion callback. Runs on libusbdi's event thread,
/// with cancellation disabled for its whole body; takes the mutex
/// briefly, never blocks on anything else.
///
/// # Safety
/// `handle` must be the pointer `Arc::into_raw` produced for this very
/// submission (what `BulkOut::write` passes); it is reclaimed exactly
/// once here. `urb` is the URB io-usb completed.
unsafe extern "C" fn bulk_complete(urb: *mut UsbdUrb, _pipe: *mut UsbdPipe, handle: *mut c_void) {
    // First, before the null check, any log line and the Book lock:
    // usbd_disconnect pthread_cancels the event thread and the cancel can
    // land mid-callback, including a completion after a leaked ring.
    // QNX cancellation is
    // pthread_exit with no unwinding (pthread.h lines 254-255) and the
    // write(2) inside clock::log is a cancellation point, so a cancel
    // acted on there would leave the stderr lock, and the Book mutex held
    // across the log line, locked for ever. Disabled until this guard
    // drops, which is after the explicit drop(book) and drop(ring) at the
    // end, the pending cancel is acted on at the thread's next MsgSendv_r
    // to io-usb instead. Covers the vlog! in sys::urb_status too.
    let _cancel = CancelGuard::new();
    if handle.is_null() {
        crate::log!("usbdi: bulk completion with a null handle; ignored");
        return;
    }
    // SAFETY: handle is the Arc::into_raw pointer submit passed for this
    // URB (the contract above); this is its one matching from_raw.
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
    // Explicit, so the cancel state (restored when `_cancel` drops, last)
    // is provably re-enabled only after the mutex and the ring reference
    // are gone.
    drop(book);
    drop(ring);
}

/// A bulk OUT pipe with its ring. `!Send`/`!Sync` (it shares the
/// device's `Rc`); only the completion path crosses threads, and that is
/// internal.
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

    /// The endpoint this pipe was opened on.
    #[must_use]
    pub fn endpoint(&self) -> Endpoint {
        self.ep
    }

    /// Bytes one slot holds: the largest `write`.
    #[must_use]
    pub fn slot_bytes(&self) -> usize {
        self.ring.slot_bytes
    }

    /// Ring depth.
    #[must_use]
    pub fn slots(&self) -> usize {
        self.ring.sys.len()
    }

    /// Copy `data` into a free slot and queue it. Waits, bounded by
    /// `timeout_ms`, for a slot when all are in flight; `timeout_ms` is
    /// also io-usb's timeout for the URB and the drain bound at teardown.
    /// A failure recorded by an earlier completion is returned first,
    /// once. The slot wait ends early with [`Error::Interrupted`] when a
    /// stop is requested ([`crate::stop`]); a write that finds a free
    /// slot is queued regardless.
    ///
    /// # Errors
    /// [`Error::Gone`]; [`Error::Errno`] (`EINVAL`) when `data` exceeds a
    /// slot; [`Error::Timeout`] when no slot frees in time;
    /// [`Error::Interrupted`] when a stop is requested while waiting for
    /// one; a previous completion's [`Error::Status`] / [`Error::Short`];
    /// [`Error::Errno`] when `usbd_io` refuses the URB.
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
                // Frees the slot and counts the failure; the error is
                // reported once, here, never again by a later write or
                // flush.
                ring.lock().unsubmit(idx);
                crate::log!("usbdi: bulk slot {idx}: usbd_io refused {len} bytes: {e}");
                Err(e)
            }
        }
    }

    /// Wait until every queued URB has completed, bounded by
    /// `timeout_ms`, and report the first failure recorded since the last
    /// report. The wait ends early with [`Error::Interrupted`] when a
    /// stop is requested ([`crate::stop`]); an already idle ring is not
    /// interrupted.
    ///
    /// # Errors
    /// [`Error::Timeout`] with URBs still in flight; [`Error::Interrupted`]
    /// when a stop is requested while URBs are in flight; otherwise the
    /// first recorded [`Error::Status`] / [`Error::Short`] of a
    /// completion (a submit `usbd_io` refused was returned by that
    /// `write` and is not recorded).
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

    /// `usbd_reset_pipe`, then the `CLEAR_FEATURE(ENDPOINT_HALT)`
    /// fallback the GIP shim uses when it fails. Restores both sides'
    /// data toggle after an unclean previous exit. Meant for an idle
    /// pipe: URBs in flight complete `ABORTED` and are reported by the
    /// next `flush`.
    ///
    /// # Errors
    /// [`Error::Gone`]; the fallback's error when both fail.
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

    /// The counters.
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
        // After a stop request the drain is capped. It remains
        // uninterruptible: the ring cannot be freed under a URB.
        let bound_ms = if stop::requested() {
            self.drain_timeout_ms.min(DEFAULT_DRAIN_TIMEOUT_MS)
        } else {
            self.drain_timeout_ms
        };
        let drained = self.ring.wait_idle(bound_ms, false).is_ok();
        // in_flight == 0 is set inside the callback, before it returns;
        // libusbdi decrements the per-pipe counter usbd_close_pipe
        // checks only after the callback returned (module doc), so EOK
        // from close_pipe proves every callback has returned and, with
        // it, released its clone of the ring. The strong count is then
        // checked as well, so the drop below is provably the last
        // reference whatever the memory ordering between libusbdi's
        // counter and the Arc's.
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
            // SAFETY: this side holds the only reference (checked just
            // above), no callback can take a new one (the pipe is closed
            // and no `write` runs during drop), so the ring is dropped
            // exactly once, here, on the caller's thread, before
            // usbd_detach and usbd_disconnect; no method of self runs
            // afterwards.
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
            // The ring is deliberately kept: `self.ring` is never
            // dropped, so a straggling callback can only decrement the
            // count and never frees the URBs and buffers into pools that
            // usbd_disconnect may already have destroyed (module doc).
        }
    }
}

impl BulkOut {
    /// `usbd_close_pipe` with the `EBUSY` retry of contract item 9; true
    /// when it returned `EOK`. An `EBUSY` return does nothing to the
    /// pipe [RE: SDP `libusbdi.so.2` @0x5800-0x580c], so retrying is
    /// safe.
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

/// Wait, bounded by `timeout_ms`, until `arc` is the only strong
/// reference (every completion callback has released its clone),
/// sleeping [`clock::SETTLE_STEP_MS`] between checks. True when reached.
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
        // Contract item 2 is length == requested, not <=: a longer
        // actual length is as wrong as a shorter one.
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
        // io-usb's URB timeout as dwcotg may report it (TIMEOUT above,
        // NOT_ACCESSED below): a status, never an errno, from a completion.
        let timed_out = USBD_STATUS_TIMEOUT | USBD_STATUS_NOT_ACCESSED;
        let lost = b.complete(2, (timed_out, 0), 3);
        assert_eq!(lost.result, Err(Error::Status(0x0400_000F)));
        assert!(b.idle());
        assert_eq!(b.stats.failures, 3);
        assert_eq!(b.take_failure(), Some(Error::Short { want: 10, got: 9 }));
        assert_eq!(b.take_failure(), None);
        // A refused submission counts as a failure, frees the slot and
        // takes its URB and bytes back out of the accepted totals, but is
        // not recorded: `write` already returned it.
        b.submit(0, 10, 5, 500);
        assert_eq!((b.stats.urbs, b.stats.bytes), (4, 40));
        b.unsubmit(0);
        assert!(b.idle());
        assert_eq!(b.free_slot(), Some(0));
        assert_eq!(b.stats.failures, 4);
        assert_eq!((b.stats.urbs, b.stats.bytes), (3, 30));
        assert_eq!(b.take_failure(), None);
        // A refused submit followed by a clean resubmission of the same
        // slot reports nothing.
        b.submit(0, 10, 6, 500);
        b.unsubmit(0);
        b.submit(0, 6, 7, 500);
        let ok = b.complete(0, (USBD_STATUS_CMP, 6), 9);
        assert_eq!(ok.result, Ok(()));
        assert_eq!(b.take_failure(), None);
        assert_eq!(b.stats.failures, 5);
        assert_eq!((b.stats.urbs, b.stats.bytes), (4, 36));
        // An out-of-range index is ignored, not a panic.
        b.submit(9, 1, 0, 1);
        assert!(b.idle());
        let none = b.complete(9, (USBD_STATUS_CMP, 0), 0);
        assert_eq!(none.result, Ok(()));
    }

    /// What the target hands `complete` for a stalled URB: the raw pair
    /// (libusbdi's status-derived errno is ignored, see
    /// `sys::nto::urb_status`), and the status word alone decides.
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

    /// The teardown's last gate before the ring is freed: a clone held
    /// by another thread (a completion callback between its
    /// `notify_all` and its `drop(ring)`) is waited for, bounded, and
    /// the wait returns at once when this side is already the last.
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

    /// The stop flag is process-global and never cleared, so this is the
    /// one test in the crate that sets it. The waits run on a ring with
    /// no slots allocated (a host build can allocate none) and one URB
    /// marked in flight by hand, which is all `wait_idle` reads.
    #[test]
    fn a_stop_request_ends_an_interruptible_wait() {
        let ring = RingState {
            sys: Vec::new(),
            book: Mutex::new(Book::new(1)),
            cv: Condvar::new(),
            slot_bytes: 0,
        };
        // Idle: nothing to wait for, whatever the flag says.
        assert_eq!(ring.wait_idle(0, true), Ok(()));
        ring.lock().submit(0, 10, 0, 1000);
        assert!(!ring.lock().idle());
        // Before any request a bounded wait runs to its deadline, both
        // ways.
        assert!(!stop::requested());
        let started = Instant::now();
        assert_eq!(ring.wait_idle(60, true), Err(Error::Timeout));
        assert!(clock::elapsed_ms(started) >= 50);
        assert_eq!(ring.wait_idle(10, false), Err(Error::Timeout));
        // A request from another thread with no condvar notify (a signal
        // handler on a thread other than the waiting one does exactly
        // this): the interruptible wait returns within one slice of it,
        // far short of its 10 s bound.
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
        // Once requested an interruptible wait returns at once...
        let started = Instant::now();
        assert_eq!(ring.wait_idle(10_000, true), Err(Error::Interrupted));
        assert!(clock::elapsed_ms(started) < STOP_SLICE_MS);
        // ...and the teardown's plain wait still runs to its bound.
        let started = Instant::now();
        assert_eq!(ring.wait_idle(60, false), Err(Error::Timeout));
        assert!(clock::elapsed_ms(started) >= 50);
        // A completion ends either wait as before.
        let done = ring.lock().complete(0, (USBD_STATUS_CMP, 10), 5);
        assert_eq!(done.result, Ok(()));
        assert_eq!(ring.wait_idle(0, true), Ok(()));
        assert_eq!(ring.wait_idle(0, false), Ok(()));
    }
}
