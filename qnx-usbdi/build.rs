// SPDX-License-Identifier: Apache-2.0
//! Link `libusbdi.so.2` on the QNX target only.
//!
//! The public compiler image supplies the target library in
//! `/opt/qnx650/target/qnx6/armle-v7/lib`.
//! This build script links it only for QNX targets.
//! Host builds use the unsupported-operation types in `src/sys/host.rs`.
//!
//! Copyright 2026 qnx650-crates contributors. Licensed under Apache-2.0.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("nto-qnx") {
        return;
    }
    println!("cargo:rustc-link-search=native=/opt/qnx650/target/qnx6/armle-v7/lib");
    println!("cargo:rustc-link-lib=dylib=usbdi");
}
