//! Copyright 2026. Licensed under the Apache License, Version 2.0.
#[cfg(target_os = "nto")]
use core::ffi::c_int;
#[must_use = "the cancel state is restored when the guard is dropped"]
pub(crate) struct CancelGuard {
    #[cfg(target_os = "nto")]
    old: c_int,
}
impl CancelGuard {
    pub(crate) fn new() -> CancelGuard {
        #[cfg(target_os = "nto")]
        {
            let mut old: c_int = crate::ffi::PTHREAD_CANCEL_ENABLE;
            // SAFETY: a plain libc call on the calling thread. `old` is a
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
            // SAFETY: The libc call acts on the caller thread.
            // The call accepts a null `oldstate` pointer.
            let _ = unsafe { crate::ffi::pthread_setcancelstate(self.old, core::ptr::null_mut()) };
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guards_nest_and_release_in_reverse_order() {
        let outer = CancelGuard::new();
        {
            let _inner = CancelGuard::new();
        }
        drop(outer);
    }
}
