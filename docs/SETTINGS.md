<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Settings

These libraries have no runtime configuration file or fixed install folder.
The calling application supplies file paths, device handles and API options.
Use each crate's Rust API documentation for argument types and limits.
No loader key or default is defined by this repository.

## Build environment

| Variable | Type | Minimum | Maximum | Default | Meaning |
| --- | --- | --- | --- | --- | --- |
| `CONTAINER_ENGINE` | Executable name | Not applicable | Not applicable | `podman` | Select Podman or Docker for `build.sh`. |
| `CARGO_TARGET_DIR` | Folder path | Not applicable | Not applicable | `target` in this checkout | Select the generated build output folder. |

The compiler image is fixed in `compiler-image.txt`. These build options do
not change the unit's runtime configuration.
