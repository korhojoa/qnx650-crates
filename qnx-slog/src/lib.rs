// SPDX-License-Identifier: MIT OR Apache-2.0
//! System-log output for QNX 6.5 applications.
//!
//! - [`sink`] selects a runtime slog function or the direct writer.
//!   The `static-slogf` feature selects only the direct writer.
//! - [`direct`] sends the slog protocol to `/dev/slog` with `open` and `write`.
//! - [`log`] formats messages in fixed stack buffers without allocation.
//!   Loader macros supply wall-clock timestamps. `slog_raw!` supplies only text.
//!   `ms_log!` sends monotonic and wall-clock timestamps to stdout and slog.
//!   `app_line` writes once to file descriptor 2, then sends the line to slog.
//! - [`sys`] declares the native interfaces and re-exports linker and clock functions.

#![cfg_attr(not(test), no_std)]

pub mod direct;
pub mod log;
pub mod sink;
pub mod sys;
