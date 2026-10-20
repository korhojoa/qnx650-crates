# qnx-mmio: values and sizes

Every value and size that `src/lib.rs` pins, with the SDP 6.5.0 header it
comes from. Header paths are relative to `target/qnx6/usr/include` of the
SDP.

## How the numbers were measured

A C probe that stores each macro value and `sizeof` in a `const int`,
compiled to assembly (no linking) with the target compiler of the
compiler image and the SDP headers, then read back from the `.word` lines
(a zero value is a `.space 4` line):

```sh
podman run --rm -v "$PWD:/w" -v "$SDP/target/qnx6/usr/include:/sdp:ro" -w /w \
    ghcr.io/korhojoa/qnx650-compiler@sha256:85f95b995d4b5dc2c323a6842c29b5c08652f34cc2288f6cc745534f5b2ecc7a \
    /opt/gcc11/bin/arm-unknown-nto-qnx6.5.0eabi-gcc -isystem /sdp -D_QNX_SOURCE \
    -S -O0 -o layout.s layout.c
```

Each probe line is `const int NAME = (int)(EXPR);`, for example
`const int v_PROT_NOCACHE = (int)(PROT_NOCACHE);`. Measured with the target compiler.
The probe included `sys/mman.h` and `sys/neutrino.h`. qnx-rpi's
`abi-check.c` held the same values against the ARM and x86 headers; they
are the same on both targets.

## Values

| name | header | value |
|---|---|---|
| `PROT_READ` | `sys/mman.h` | 0x100 |
| `PROT_WRITE` | `sys/mman.h` | 0x200 |
| `PROT_NOCACHE` | `sys/mman.h` | 0x800 |
| `MAP_SHARED` | `sys/mman.h` | 0x1 |
| `MAP_PHYS` | `sys/mman.h` | 0x10000 |
| `MAP_NOX64K` | `sys/mman.h` | 0x20000 |
| `MAP_ANON` | `sys/mman.h` | 0x80000 |
| `MAP_FAILED` | `sys/mman.h` | -1 (`(void *)-1`) |
| `NOFD` | `sys/mman.h` | -1 |
| `_NTO_TCTL_IO` (`NTO_TCTL_IO`) | `sys/neutrino.h` | 1 |

## Sizes

| type | size | used by |
|---|---|---|
| `off_t` | 4 | why `mmap` is bound to `mmap64` |
| `off64_t` | 8 | the `off` argument of `mmap64`, the `offset` of `mem_offset64` |
| `size_t` | 4 | `len` arguments (`usize`) |

The header defines `PROT_NOCACHE` as 0x800. The value 0x1000 is invalid
and causes `EINVAL` on the target device.

The host build uses the same values. The host substitutes of the QNX calls
ignore their arguments and fail.
