// SPDX-License-Identifier: Apache-2.0
//! Thread cancellation deferred across an io-usb callback.
//!
//! `usbd_disconnect` ends libusbdi's event thread with `pthread_cancel`
//! and `pthread_join`. Cancellation can occur while a callback is
//! executing. QNX cancellation is `pthread_exit` with no unwinding
//! (`pthread.h` lines 254-255: `pthread_testcancel` is `if (flags ==
//! ENABLE | PENDING) pthread_exit(PTHREAD_CANCELED)`), so no Rust
//! destructor runs on the cancelled thread. Every callback logs, and
//! `clock::log` holds the process-wide stderr lock across a `write(2)`,
//! a cancellation point: a callback cancelled there leaves that lock
//! held for ever, and the caller's very next log line (`HostSys::drop`'s
//! own `usbd_disconnect rc` line, on the thread that called
//! `usbd_disconnect`) blocks, so the process never exits on its own.
//! The bulk completion callback holds the ring's `Book` mutex across its
//! log line as well.
//!
//! So each callback runs its whole body with cancellation DISABLED
//! (contract item 4): [`CancelGuard`] is its FIRST local, before the
//! null checks and before any `log!`, `vlog!` or `lock()`, and Rust
//! drops locals in reverse declaration order, so the state is restored
//! only after every lock guard and `Arc` the body took is gone. A cancel
//! that lands meanwhile stays pending and is acted on at the event
//! thread's next cancellation point, its next message to io-usb, once
//! the callback has returned. Restoring the state does not itself exit
//! the thread: with the default deferred cancel type
//! `pthread_setcancelstate(ENABLE)` only clears the state bit [RE: SDP
//! `libc.so.3` `pthread_setcancelstate` @0x1fff0-0x20048; the
//! pending-cancel exit @0x20080-0x20090 is reached only when the old
//! flags carried the asynchronous-type bit 0x20 as well].
//!
//! The removal callback may also run nested on the caller's thread
//! (`events` module doc); there the guard disables and restores the
//! caller's own state, which changes nothing the caller can observe. On
//! a non-QNX target the guard is a no-op. Nothing here logs: the guard
//! is what makes logging in a callback safe and must not depend on it.
//!
//! Copyright 2026. Licensed under the Apache License, Version 2.0.

#[cfg(target_os = "nto")]
use core::ffi::c_int;

/// Cancellation disabled for the calling thread from construction to
/// drop. Declare it as the first local of a callback so it is dropped
/// last.
#[must_use = "the cancel state is restored when the guard is dropped"]
pub(crate) struct CancelGuard {
    /// The state the thread had before, restored on drop.
    #[cfg(target_os = "nto")]
    old: c_int,
}

impl CancelGuard {
    /// Disable cancellation for the calling thread and remember the
    /// previous state. The call cannot fail for a valid state
    /// (`ffi::pthread_setcancelstate`), so `old` is always what the
    /// thread had.
    pub(crate) fn new() -> CancelGuard {
        #[cfg(target_os = "nto")]
        {
            let mut old: c_int = crate::ffi::PTHREAD_CANCEL_ENABLE;
            // SAFETY: a plain libc call on the calling thread; `old` is a
            // live local the callee writes the previous state through.
            let _ = unsafe {
                crate::ffi::pthread_setcancelstate(crate::ffi::PTHREAD_CANCEL_DISABLE, &raw mut old)
            };
            CancelGuard { old }
        }
        #[cfg(not(target_os = "nto"))]
        {
            CancelGuard {}
        }
    }
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "nto")]
        {
            // SAFETY: a plain libc call on the calling thread; a null
            // `oldstate` is accepted [RE: SDP libc.so.3 @0x20034-0x20048].
            let _ = unsafe { crate::ffi::pthread_setcancelstate(self.old, core::ptr::null_mut()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard nests and is released in reverse order, the shape every
    /// callback relies on. On the host both calls are no-ops; the QNX
    /// Clippy and build steps of the gate compile the real ones.
    #[test]
    fn guards_nest_and_release_in_reverse_order() {
        let outer = CancelGuard::new();
        {
            let _inner = CancelGuard::new();
        }
        drop(outer);
    }
}
