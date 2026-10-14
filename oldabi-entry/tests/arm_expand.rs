// SPDX-License-Identifier: MIT OR Apache-2.0
//! Expands every form of `entry!` in an ARM build, so the target build and
//! the target clippy run check the generated assembly. A host build has
//! nothing to check here: the trampolines are ARM assembly.
#![cfg(target_arch = "arm")]
extern "C" fn body() -> i32 {
    7
}
oldabi_entry::entry!("oae_test_short" => body);
oldabi_entry::entry!("oae_test_0" => body, 0);
oldabi_entry::entry!("oae_test_1" => body, 1);
oldabi_entry::entry!("oae_test_2" => body, 2);
oldabi_entry::entry!("oae_test_3" => body, 3);
oldabi_entry::entry!("oae_test_4" => body, 4);
oldabi_entry::entry!("oae_test_5" => body, 5);
oldabi_entry::entry!("oae_test_6" => body, 6);
oldabi_entry::entry!("oae_test_7" => body, 7);
oldabi_entry::entry!("oae_test_8" => body, 8);
oldabi_entry::entry!("oae_test_9" => body, 9);
oldabi_entry::entry!("oae_test_10" => body, 10);
oldabi_entry::entry!("oae_test_11" => body, 11);
oldabi_entry::entry!("oae_test_12" => body, 12);
oldabi_entry::entry!(fn local_short => body);
oldabi_entry::entry!(pub(crate) fn local_frame => body, 2);
#[test]
fn trampolines_exist() {
    let short = core::hint::black_box(local_short as unsafe extern "C" fn());
    let frame = core::hint::black_box(local_frame as unsafe extern "C" fn());
    assert_ne!(short as usize, 0);
    assert_ne!(frame as usize, 0);
}
