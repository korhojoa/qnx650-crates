// SPDX-License-Identifier: Apache-2.0
//! The process-wide stop request the bulk ring's waits consult.
//!
//! A signal handler may do nothing but store a flag, and a flag that is
//! only read between phases is not seen by a wait that is already inside
//! one: `BulkOut::write` can wait for a free slot and `BulkOut::flush`
//! can wait for the ring to drain. The flag lives in the crate that owns
//! the waits: [`request`] is one atomic store (async-signal-safe, what a
//! handler calls), [`requested`] one load. The interruptible waits
//! (`write`'s slot wait, `flush`) poll it in slices of
//! [`crate::bulk::STOP_SLICE_MS`] and return
//! [`crate::Error::Interrupted`] once it is set; the teardown drain in
//! `BulkOut::drop` is never interrupted (the ring must not be freed
//! under a URB) but is bounded to `DEFAULT_DRAIN_TIMEOUT_MS` once a stop
//! is requested, so abort, drain, `close_pipe`, `detach` and
//! `disconnect` can proceed after the bounded drain.
//!
//! One flag per process, never cleared: a stop is the end of the run,
//! and the crate's tests set it in one place
//! (`bulk::tests::a_stop_request_ends_an_interruptible_wait`).
//!
//! [`install_handlers`] routes `SIGTERM` and `SIGINT` to [`request`]
//! (QNX only): what every tool and daemon on this crate does at start,
//! so a termination signal tears down in order (ring drain,
//! `close_pipe`, `detach`, `disconnect`) instead of leaving URBs in
//! flight. What cannot be interrupted: a synchronous control `usbd_io`
//! and `usbd_connect`'s wait; a signal inside one of those may still be
//! escalated.
//!
//! Copyright 2026. Licensed under the Apache License, Version 2.0.

//! The flag itself is `qnx-stop`'s, so a program that also reads it
//! directly sees the same stop.

pub use qnx_stop::{request, requested, SIGINT, SIGTERM};

/// Install the handler for `SIGTERM` and `SIGINT` (QNX only; off target
/// the flag exists and is never set by a signal). `who` names the caller
/// in the log line of a failed install.
pub fn install_handlers(who: &str) {
    #[cfg(target_os = "nto")]
    match qnx_stop::install() {
        Ok(()) => {
            crate::vlog!("{who}: SIGTERM ({SIGTERM}) and SIGINT ({SIGINT}) handlers installed");
        }
        Err(sig) => {
            crate::log!("{who}: signal({sig}) failed; a deadline will not tear down");
        }
    }
    #[cfg(not(target_os = "nto"))]
    {
        crate::vlog!("{who}: off target: no handler for SIGTERM ({SIGTERM}) or SIGINT ({SIGINT})");
    }
}
