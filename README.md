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
The QNX target requires a compiler with QNX 6.5 target support and a licensed QNX SDK.
Host builds use test implementations for the QNX interfaces.
Host tests do not execute those hardware or operating system interfaces.

## Checks

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

4. Check the QNX interfaces with the QNX compiler environment.

   ```sh
   cargo clippy --offline --locked --target armv7-unknown-nto-qnx650 --workspace --all-targets -- -D warnings -W clippy::pedantic
   cargo build --offline --release --locked --target armv7-unknown-nto-qnx650 --workspace
   ```

## License

Each Cargo manifest specifies Apache-2.0 or MIT OR Apache-2.0.
The workspace contains [Apache-2.0](LICENSE-APACHE) and [MIT](LICENSE-MIT) license texts.
The source files retain their existing copyright notices.
