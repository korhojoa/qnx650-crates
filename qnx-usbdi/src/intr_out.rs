// SPDX-License-Identifier: Apache-2.0
//! `IntrOut`: an interrupt OUT pipe, one synchronous transfer at a time.
//!
//! Unlike the rings in [`crate::bulk`] and [`crate::intr`], a write here
//! does not queue a URB with a completion callback. It sets up the one
//! URB of the pipe and calls `usbd_io` with a NULL callback.
//! That call can return `EOK` with the URB still `INPROG`.
//! The transfer completes in the background. The write polls the URB
//! status until it leaves `INPROG`, bounded by the timeout.
//! The buffer remains allocated and unchanged until completion.
//! Interrupt OUT carries short commands (a game pad's LED, rumble and
//! handshake messages), so one transfer in flight is enough.
//!
//! A write is judged by its URB status (`CMP`). The transferred length
//! is logged, not judged, as for a control OUT.
//!
//! When `usbd_io` fails, or the transfer is still `INPROG` at the
//! timeout, the pipe is aborted before the call returns, so the URB is
//! not left queued on the host controller when the next write sets it up
//! again.
//!
//! Copyright 2026. Licensed under the Apache License, Version 2.0.

use core::mem::ManuallyDrop;
use std::rc::Rc;
use std::time::Instant;

use crate::clock;
use crate::desc::Endpoint;
use crate::device::DeviceShared;
use crate::ffi::{EINVAL, USBD_STATUS_CMP, USBD_STATUS_INPROG, USBD_URB_STATUS_MASK};
use crate::sys;
use crate::{Error, Result};

/// Default timeout for one interrupt OUT transfer, in milliseconds.
pub const DEFAULT_TIMEOUT_MS: u32 = 1000;

/// Counters kept since the pipe was opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IntrOutStats {
    /// Writes that completed `CMP`.
    pub writes: u64,
    /// Bytes of those writes.
    pub bytes: u64,
    /// Writes that failed (refused, timed out, or not `CMP`).
    pub failures: u64,
    /// The longest time one write spent in `usbd_io`.
    pub max_ms: u64,
}

impl IntrOutStats {
    fn record(&mut self, result: &Result<()>, len: usize, ms: u64) {
        match result {
            Ok(()) => {
                self.writes += 1;
                self.bytes += u64::try_from(len).unwrap_or(u64::MAX);
            }
            Err(_) => self.failures += 1,
        }
        self.max_ms = self.max_ms.max(ms);
    }
}

/// Judge one completed interrupt OUT transfer by its status class.
///
/// # Errors
/// [`Error::Status`] unless the status is `CMP`.
pub fn judge(status: u32) -> Result<()> {
    if status & USBD_URB_STATUS_MASK == USBD_STATUS_CMP {
        Ok(())
    } else {
        Err(Error::Status(status))
    }
}

/// Whether `ep` is an interrupt OUT endpoint.
fn is_interrupt_out(ep: Endpoint) -> bool {
    ep.is_interrupt() && !ep.is_in()
}

/// Check a write's length against the slot before anything is sent.
fn check_len(len: usize, slot_bytes: usize) -> Result<u32> {
    if len == 0 || len > slot_bytes {
        return Err(Error::Errno(EINVAL));
    }
    u32::try_from(len).map_err(|_| Error::Errno(EINVAL))
}

/// An interrupt OUT pipe with its one URB. `!Send`/`!Sync` (it shares the
/// device's `Rc`).
pub struct IntrOut {
    slot: ManuallyDrop<sys::SlotSys>,
    pipe: sys::PipeSys,
    dev: Rc<DeviceShared>,
    ep: Endpoint,
    slot_bytes: usize,
    stats: IntrOutStats,
}

impl IntrOut {
    pub(crate) fn open(dev: Rc<DeviceShared>, ep: Endpoint) -> Result<IntrOut> {
        if !is_interrupt_out(ep) {
            crate::log!(
                "usbdi: open_intr_out refused: endpoint {:#04x} attr {:#04x} is not interrupt OUT",
                ep.address(),
                ep.attributes()
            );
            return Err(Error::NoEndpoint);
        }
        let slot_bytes = usize::from(ep.max_packet());
        if slot_bytes == 0 {
            crate::log!("usbdi: open_intr_out refused: wMaxPacketSize 0");
            return Err(Error::Errno(EINVAL));
        }
        if dev.gone.is_set() {
            return Err(Error::Gone);
        }
        let pipe = dev.sys.open_pipe(ep)?;
        let slot = match sys::SlotSys::alloc(slot_bytes) {
            Ok(s) => s,
            Err(e) => {
                if let Err(ce) = pipe.close() {
                    crate::log!("usbdi: close_pipe after a failed URB allocation: {ce}");
                }
                return Err(e);
            }
        };
        let mut out = IntrOut {
            slot: ManuallyDrop::new(slot),
            pipe,
            dev,
            ep,
            slot_bytes,
            stats: IntrOutStats::default(),
        };
        crate::log!(
            "usbdi: interrupt OUT endpoint {:#04x} (mps {}, interval {}) open",
            ep.address(),
            ep.max_packet(),
            ep.interval()
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

    /// The most bytes one write takes: the endpoint's `wMaxPacketSize`.
    #[must_use]
    pub fn slot_bytes(&self) -> usize {
        self.slot_bytes
    }

    /// Send `data` as one interrupt OUT transfer and wait for it, at most
    /// `timeout_ms` (the default is [`DEFAULT_TIMEOUT_MS`]).
    ///
    /// # Errors
    /// [`Error::Gone`]; [`Error::Errno`] (`EINVAL`) when `data` is empty
    /// or longer than [`IntrOut::slot_bytes`]; [`Error::Errno`] when
    /// `usbd_io` fails (the pipe is aborted first); [`Error::Timeout`] when
    /// the transfer is still in flight after `timeout_ms` (the pipe is
    /// aborted first); [`Error::Status`] when it completed other than `CMP`.
    pub fn write(&mut self, data: &[u8], timeout_ms: u32) -> Result<()> {
        if self.dev.gone.is_set() {
            return Err(Error::Gone);
        }
        let len32 = check_len(data.len(), self.slot_bytes)?;
        self.slot.fill(data);
        let started = Instant::now();
        let sent = self.pipe.send_intr_out(&self.slot, len32, timeout_ms);
        let ms = clock::elapsed_ms(started);
        let result = match sent {
            Ok((status, _)) if status & USBD_URB_STATUS_MASK == USBD_STATUS_INPROG => {
                // Still in flight at the timeout: end it before the slot is
                // filled again.
                let abort = self.pipe.abort();
                crate::log!(
                    "usbdi: intr OUT endpoint {:#04x} {len32} byte(s): still INPROG after {ms} ms (timeout {timeout_ms} ms); abort_pipe {abort:?}",
                    self.ep.address()
                );
                Err(Error::Timeout)
            }
            Ok((status, alen)) => {
                let r = judge(status);
                match &r {
                    Ok(()) => crate::vlog!(
                        "usbdi: intr OUT endpoint {:#04x} {len32} byte(s): CMP alen {alen} in {ms} ms",
                        self.ep.address()
                    ),
                    Err(e) => crate::log!(
                        "usbdi: intr OUT endpoint {:#04x} {len32} byte(s): {e}, alen {alen} in {ms} ms (timeout {timeout_ms} ms)",
                        self.ep.address()
                    ),
                }
                r
            }
            Err(e) => {
                let abort = self.pipe.abort();
                crate::log!(
                    "usbdi: intr OUT endpoint {:#04x} {len32} byte(s): usbd_io {e} after {ms} ms (timeout {timeout_ms} ms); abort_pipe {abort:?}",
                    self.ep.address()
                );
                Err(e)
            }
        };
        self.stats.record(&result, data.len(), ms);
        result
    }

    /// `usbd_reset_pipe`, then the `CLEAR_FEATURE(ENDPOINT_HALT)`
    /// fallback (as [`crate::bulk::BulkOut::reset`]): the data toggle
    /// cleanup at bring-up and after a stall.
    ///
    /// # Errors
    /// [`Error::Gone`]; the fallback's error when both fail.
    pub fn reset(&mut self) -> Result<()> {
        self.dev.reset_pipe(&self.pipe, self.ep)
    }

    /// The counters.
    #[must_use]
    pub fn stats(&self) -> IntrOutStats {
        self.stats
    }
}

impl Drop for IntrOut {
    fn drop(&mut self) {
        let started = Instant::now();
        // No URB is in flight: every write waited for its transfer or
        // aborted the pipe. The abort here is for a write that the
        // abort itself failed to clear.
        if let Err(e) = self.pipe.abort() {
            crate::log!(
                "usbdi: intr OUT teardown: abort_pipe(endpoint {:#04x}) failed: {e}",
                self.ep.address()
            );
        }
        let what = format!("usbd_close_pipe(endpoint {:#04x})", self.ep.address());
        match clock::retry_while_busy(&what, clock::SETTLE_TIMEOUT_MS, || self.pipe.close()) {
            Ok(()) => {
                crate::log!(
                    "usbdi: intr OUT teardown: closed in {} ms ({} write(s), {} bytes, {} failure(s), max {} ms)",
                    clock::elapsed_ms(started),
                    self.stats.writes,
                    self.stats.bytes,
                    self.stats.failures,
                    self.stats.max_ms
                );
                // SAFETY: the pipe is closed, so io-usb holds no reference
                // to the URB or its buffer; the slot is dropped exactly
                // once, here, before usbd_detach and usbd_disconnect.
                unsafe { ManuallyDrop::drop(&mut self.slot) };
            }
            Err(e) => crate::log!(
                "usbdi: intr OUT teardown: {what} failed: {e}; the URB is leaked (the memchunk destroy at usbd_disconnect reclaims it) and close_pipe is left to usbd_detach"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::{USBD_STATUS_ABORTED, USBD_STATUS_CMP_ERR, USBD_STATUS_STALL};

    #[test]
    fn judge_needs_cmp_only() {
        assert_eq!(judge(USBD_STATUS_CMP), Ok(()));
        let stall = USBD_STATUS_CMP_ERR | USBD_STATUS_STALL;
        assert_eq!(judge(stall), Err(Error::Status(stall)));
        assert_eq!(
            judge(USBD_STATUS_ABORTED),
            Err(Error::Status(USBD_STATUS_ABORTED))
        );
    }

    #[test]
    fn a_write_must_fit_the_packet() {
        assert_eq!(check_len(0, 64), Err(Error::Errno(EINVAL)));
        assert_eq!(check_len(1, 64), Ok(1));
        assert_eq!(check_len(64, 64), Ok(64));
        assert_eq!(check_len(65, 64), Err(Error::Errno(EINVAL)));
    }

    #[test]
    fn stats_count_writes_bytes_and_failures() {
        let mut s = IntrOutStats::default();
        s.record(&Ok(()), 4, 2);
        s.record(&Err(Error::Timeout), 9, 1000);
        s.record(&Ok(()), 13, 3);
        assert_eq!(
            s,
            IntrOutStats {
                writes: 2,
                bytes: 17,
                failures: 1,
                max_ms: 1000
            }
        );
    }

    #[test]
    fn open_needs_an_interrupt_out_endpoint() {
        let intr_in = Endpoint::new(7, 0x81, 0x03, 64, 4);
        let bulk_out = Endpoint::new(7, 0x01, 0x02, 64, 0);
        let intr_out = Endpoint::new(7, 0x01, 0x03, 64, 4);
        assert!(!is_interrupt_out(intr_in));
        assert!(!is_interrupt_out(bulk_out));
        assert!(is_interrupt_out(intr_out));
    }
}
