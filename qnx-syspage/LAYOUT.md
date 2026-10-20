# qnx-syspage: layouts

Every offset and size that `src/layout.rs` pins, from `sys/syspage.h` of
SDP 6.5.0 (`target/qnx6/usr/include/sys/syspage.h`).

## How the numbers were measured

A C probe that stores each `sizeof`, `offsetof` and macro value in a
`const int`, compiled to assembly (no linking) with the target compiler
of the compiler image and the SDP headers, then read back from the
`.word` lines (a zero value is a `.space 4` line):

```sh
podman run --rm -v "$PWD:/w" -v "$SDP/target/qnx6/usr/include:/sdp:ro" -w /w \
    ghcr.io/korhojoa/qnx650-compiler@sha256:85f95b995d4b5dc2c323a6842c29b5c08652f34cc2288f6cc745534f5b2ecc7a \
    /opt/gcc11/bin/arm-unknown-nto-qnx6.5.0eabi-gcc -isystem /sdp -D_QNX_SOURCE \
    -S -O0 -o layout.s layout.c
```

Each probe line is `const int NAME = (int)(EXPR);`, for example
`const int off_ci_name = (int)offsetof(struct cpuinfo_entry, name);`.
Measured for ARM. qnx-rpi's `abi-check.c` held `num_cpu`,
`cpuinfo.entry_off` and `cpuinfo_entry.cpu` against the ARM and x86
headers with the same results. Every field is a fixed-width type, and the
one pointer (`alloc_checker`) is 32 bits on both QNX 6.5 targets.

## Structs

| struct | header lines | size | offsets |
|---|---|---|---|
| `syspage_entry_info` | 69-72 | 4 | entry_off 0, entry_size 2 (both `_Uint16t`) |
| `struct syspage_entry` (the header) | 453-494 | (read up to 52) | size 0, total_size 2, type 4, num_cpu 6 (`_Uint16t`), system_private 8, asinfo 12, meminfo 16, hwinfo 20, cpuinfo 24, cacheattr 28, qtime 32, typed_strings 44, strings 48 |
| `struct cpuinfo_entry` | 318-326 | 32 | cpu 0, speed 4, flags 8, spare 12, name 28 (`_Uint16t`), ins_cache 30, data_cache 31 |
| `struct asinfo_entry` | 251-264 | 32 | start 0, end 8 (`_Uint64t`), owner 16, name 18, attr 20, priority 22 (`_Uint16t`), alloc_checker 24, spare 28 |

## Values

| name | value |
|---|---|
| `AS_NULL_OFF` | 0xffff |
| `CPU_FLAG_MMU` | 0x40000000 |
| `CPU_FLAG_FPU` | 0x80000000 |

## Notes

- `cpuinfo_entry.name` and `asinfo_entry.name` are offsets into the
  `strings` section (not `typed_strings`).
- The libc fork that qnx-apps `rustfetch` reads the page through names
  the 6.5 fields `old_asinfo` and `old_cpuinfo`; they are the header's
  `asinfo` (12) and `cpuinfo` (24).
- `_syspage_ptr` is a data symbol of `libc.so.3` (checked with `nm -D`
  on the SDP's `armle-v7/lib/libc.so.3`). The header's
  `__SLIB_DATA_INDIRECT` form (`__get_syspage_ptr()`) is for static
  library code and is not used.
