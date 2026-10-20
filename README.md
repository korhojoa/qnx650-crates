# qnx650-crates

Rust interfaces for ARM QNX 6.5 applications.
Applications use the libraries through Cargo path dependencies.
The repository supplies no firmware or SDK headers.

See [Terms](docs/TERMS.md) for the technical names and
[Settings](docs/SETTINGS.md) for configuration files and values.

| Crate | Function |
| --- | --- |
| `hidkbm-ring` | Read keyboard and mouse ring events. |
| `joined-worker` | Stop and join an owned worker. |
| `oldabi-entry` | Align calls from the older ARM ABI. |
| `qnx-dl` | Load libraries and find symbols. |
| `qnx-errno` | Read and set the thread error number. |
| `qnx-mmio` | Map device memory. |
| `qnx-msg` | Use channels, connections, messages and pulses. |
| `qnx-procfs` | Read process information. |
| `qnx-qsa` | Use QNX Sound Architecture interfaces. |
| `qnx-resmgr` | Use resource manager and dispatch interfaces. |
| `qnx-shm` | Map POSIX shared memory. |
| `qnx-slog` | Write fixed-buffer log messages. |
| `qnx-stop` | Stop and wait for an owned process. |
| `qnx-syslog` | Write system log messages. |
| `qnx-syspage` | Read system page data. |
| `qnx-time` | Read monotonic and wall clocks. |
| `qnx-usbdi` | Use USB device interfaces. |

## Fetch and build

Install Git and Podman or Docker. Fetch the source:

```sh
git clone https://github.com/korhojoa/qnx650-crates.git
```

From the new source folder, run one build command:

```sh
./build.sh
```

The command uses the public compiler image pinned in `compiler-image.txt`.
It fetches locked dependencies, checks formatting and Clippy, runs host tests, and builds all ARM QNX libraries.
Set `CONTAINER_ENGINE=docker` to use Docker. The default engine is Podman.
Set `CARGO_TARGET_DIR` to reuse a separate target cache.
The workspace uses one root lock file and the normal Cargo cache.
The libraries have no installation folder or configuration file.
Applications supply all file paths and native interfaces.
Installation of the extenders is described in the [combined guide](https://github.com/korhojoa/mib2-extenders/blob/main/INSTALL.md).

## Status

The layout assertions check the native ABI sizes and offsets.
Host tests use substitutes when a native service is required.
The tests do not qualify native service behavior or a head-unit installation.

## License

`hidkbm-ring`, `joined-worker`, `qnx-mmio`, `qnx-syslog`, `qnx-syspage` and `qnx-usbdi` use `Apache-2.0`.
The other libraries use `MIT OR Apache-2.0`. You can choose either license for those libraries.
See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT).
