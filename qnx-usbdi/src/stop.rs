//! Copyright 2026. Licensed under the Apache License, Version 2.0.
pub use qnx_stop::{request, requested, SIGINT, SIGTERM};
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
