# Shared Rust crates for QNX 6.5

This workspace contains 17 Rust crates for QNX 6.5.
The QNX target is `armv7-unknown-nto-qnx650`.
The source is work in progress. APIs can change.
Each crate has a separate version and license declaration.
The workspace contains no QNX SDK or runtime libraries.

| Crate | Function |
| --- | --- |
| `hidkbm-ring` | Shared HID event rings and PPS notifications |
| `joined-worker` | Worker shutdown, wake paths, and thread joins |
| `oldabi-entry` | ARM stack alignment and panic handlers at C interfaces |
| `qnx-dl` | Dynamic library loading and symbol lookup |
| `qnx-errno` | Error number access without the Rust standard library |
| `qnx-mmio` | Device memory mappings and register access |
| `qnx-msg` | Neutrino messages, pulses, channels, and interrupt events |
| `qnx-procfs` | Process information from the QNX process file system |
| `qnx-qsa` | QNX Sound Architecture playback and mixer access |
| `qnx-resmgr` | QNX resource manager interfaces and message layouts |
| `qnx-shm` | POSIX shared memory mappings |
| `qnx-slog` | Slog logging for libraries and programs |
| `qnx-stop` | Process shutdown flags for SIGINT and SIGTERM |
| `qnx-syslog` | Program logging through syslog and standard error |
| `qnx-syspage` | CPU and memory information from the QNX system page |
| `qnx-time` | QNX clocks and time conversions |
| `qnx-usbdi` | USB connection, control, bulk, and interrupt transfers |

## Use a crate

The workspace supports path dependencies from a Git submodule.
For example, a consumer can use this dependency:

```toml
qnx-usbdi = { path = "third_party/qnx650-crates/qnx-usbdi" }
```

The consumer repository pins the submodule commit.
The published `ghcr.io/korhojoa/qnx650-compiler` container builds all 17 crates.
No separate QNX SDK installation is needed for these builds.
Host builds use test implementations for the QNX interfaces.
Host tests do not execute those hardware or operating system interfaces.

## Checks

The following host commands require Rust 1.99.0.
The container procedure below supplies the QNX compiler and target libraries.

1. Check formatting.

   ```sh
   cargo fmt --all --check
   ```

2. Run the host tests.

   ```sh
   cargo test --offline --locked --workspace
   ```

3. Run Clippy.

   ```sh
   cargo clippy --offline --locked --workspace --all-targets -- -D warnings -W clippy::pedantic
   ```

4. Create the container output directory.

   ```sh
   mkdir -p target-qnx
   ```

5. Run the QNX checks in the published container.

   ```sh
   podman run --rm \
       -v "$PWD:/src:ro" \
       -v "$PWD/target-qnx:/target" \
       -w /src \
       -e CARGO_TARGET_DIR=/target \
       ghcr.io/korhojoa/qnx650-compiler@sha256:85f95b995d4b5dc2c323a6842c29b5c08652f34cc2288f6cc745534f5b2ecc7a \
       bash -euo pipefail -c '
           cargo clippy --offline --locked --target armv7-unknown-nto-qnx650 --workspace --all-targets -- -D warnings -W clippy::pedantic
           cargo build --offline --release --locked --target armv7-unknown-nto-qnx650 --workspace
       '
   ```

The container reads the source without write access.
It writes build output to `target-qnx`.

## Features

- `hidkbm-ring/shm` enables the shared memory producer and reader. It is enabled by default.
- `qnx-slog/static-slogf` selects the direct `/dev/slog` writer. It does not link a static logging library.
- `qnx-dl/alloc` enables owned linker error strings.
- `qnx-slog/alloc` enables the owned error re-export and `qnx-dl/alloc`.
- `oldabi-entry/no_realign` disables ARM stack alignment for negative tests.

Do not enable `oldabi-entry/no_realign` in production builds.
Do not use `--all-features` for production builds of this workspace.

## Error text and register access

`qnx-dl::dl_error()` returns a complete owned `String` when the `alloc` feature is enabled.
The text remains valid after later dynamic linker calls.
The wrapper imposes no message length limit.

An allocating consumer can use this dependency:

```toml
qnx-dl = { path = "third_party/qnx650-crates/qnx-dl", features = ["alloc"] }
```

Allocation-free consumers use `dl_error_into(&mut buffer)`.
It returns the complete error as a `CStr` that borrows the caller's buffer.
Insufficient capacity returns `ErrorBufferTooSmall` with the required byte count, including the NUL terminator.
The buffer remains unchanged on failure. The wrapper never truncates the message.
The call consumes the linker error even when capacity is insufficient.
A later call with a larger buffer cannot retrieve that same error.

`DeviceMapping::len()` returns the requested byte count.
Register accesses must stay within that range and have four-byte alignment.
The constructor rejects an unaligned physical address.
`write()` returns false for a read-only mapping.

## License

Each Cargo manifest specifies Apache-2.0 or MIT OR Apache-2.0.
The workspace contains [Apache-2.0](LICENSE-APACHE) and [MIT](LICENSE-MIT) license texts.
The source files retain their existing copyright notices.
