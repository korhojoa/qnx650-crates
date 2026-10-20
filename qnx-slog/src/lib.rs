// SPDX-License-Identifier: MIT OR Apache-2.0
//! System-log output for QNX 6.5 applications.
//!
//! - [`sink`]: where a line goes. slogf, else vslogf, found at run time in
//!   the process (libraries injected into `dio_manager` / gal), else the
//!   direct writer. The `static-slogf` feature selects the direct writer
//!   only (stand-alone programs; the unit's libc.so.3 exports neither
//!   slogf nor vslogf).
//! - [`direct`]: the slogger's protocol on `/dev/slog`, written with open
//!   and write, so nothing is linked out of the SDP's static libc.
//! - [`log`]: allocation-free lines in fixed buffers: the date-stamped
//!   `ldr_log!`/`ldr_loge!` form and `slog_raw!` (gal's loader), and
//!   `ms_log!`, the `mono_ms=.. wall_ms=.. tag: text` form stand-alone
//!   programs use, which also goes to stdout.
//! - [`sys`]: the QNX 6.5 libc declarations these need, plus dlsym helpers.
//!
//! Shared logging support for QNX applications;
//! relicensed MIT OR Apache-2.0 to match this repository.

#![cfg_attr(not(test), no_std)]

pub mod direct;
pub mod log;
pub mod sink;
pub mod sys;
