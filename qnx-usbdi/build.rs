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
