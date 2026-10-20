// SPDX-License-Identifier: Apache-2.0
//! Link `libusbdi.so.2` on the QNX target only.
//!
//! The io-usb client library ships with the SDP; inside the build
//! container the SDP sits at `/opt/qnx650` (the `qsaplay` crate links
//! `libasound` the same way). On the host nothing is linked: the crate
//! compiles to stubs there (see `src/lib.rs`), so the host gate can lint
//! and test it without a QNX toolchain.
//!
//! Copyright 2026. Licensed under the Apache License, Version 2.0.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("nto-qnx") {
        return;
    }
    println!("cargo:rustc-link-search=native=/opt/qnx650/target/qnx6/armle-v7/lib");
    println!("cargo:rustc-link-lib=dylib=usbdi");
}
