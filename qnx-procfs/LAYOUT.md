# qnx-procfs: layouts and values

Every size, offset and value that `src/lib.rs` pins, with the SDP 6.5.0
header line it comes from. Header paths are relative to
`target/qnx6/usr/include` of the SDP.

## Measure the layout

Use a layout probe that stores each `sizeof`, `offsetof` and macro value in a
`const int`, compiled to assembly (no linking) with the target compiler
of the compiler image and the SDP headers, then read back from the
`.word` lines (a zero value is a `.space 4` line):

```sh
podman run --rm -v "$PWD:/w" -v "$SDP/target/qnx6/usr/include:/sdp:ro" -w /w \
    ghcr.io/korhojoa/qnx650-compiler@sha256:85f95b995d4b5dc2c323a6842c29b5c08652f34cc2288f6cc745534f5b2ecc7a \
    /opt/gcc11/bin/arm-unknown-nto-qnx6.5.0eabi-gcc -isystem /sdp -D_QNX_SOURCE \
    -S -O0 -o layout.s layout.c
```

The probe includes `fcntl.h`, `devctl.h`, `sys/procfs.h` and
`sys/debug.h`. Each line is `const int NAME = (int)(EXPR);`, for example
`const int dp_utime = (int)offsetof(debug_process_t, utime);`. The const assertions in `src/lib.rs` repeat the numbers below
(the structure has no pointers, so they hold on a 64-bit host too).

## Struct

`procfs_info` is `debug_process_t` (`sys/procfs.h` 51). Both headers
include `_pack64.h` (`sys/debug.h` 130, `sys/procfs.h` 48), which is
`#pragma pack()` (the default packing) for GCC, so the 64-bit members
are 8-byte aligned.

| struct | header | size | offsets |
|---|---|---|---|
| `debug_process_t` (`ProcessInfo`) | `sys/debug.h` 168-204 | 272, align 8 | parent 4, flags 8, umask 12, child 16, sibling 20, pgrp 24, sid 28, base_address 32, initial_stack 40, uid 48, gid 52, euid 56, egid 60, suid 64, sgid 68, sig_ignore 72, sig_queue 80, sig_pending 88, num_chancons 96, num_fdcons 100, num_threads 104, num_timers 108, start_time 112, utime 120, stime 128, cutime 136, cstime 144, priority 152, extsched 160, pls 168, sigstub 176, canstub 184, reserved 192 |

`pid_t`, `uid_t` and `gid_t` are 4 bytes; `sigset_t` is 8 bytes
(`signal.h` 51), carried as `[u32; 2]`.

## Values

| name | header | value |
|---|---|---|
| `DCMD_PROC_INFO` | `sys/procfs.h` 120: `__DIOF(_DCMD_PROC, __PROC_SUBCMD_PROCFS + 1, procfs_info)` | 0x41100801 |
| `__DIOF` | `devctl.h` 131: `(sizeof(data)<<16) + ((class)<<8) + (cmd) + _POSIX_DEVDIR_FROM` | |
| `_POSIX_DEVDIR_FROM` | `devctl.h` 55 | 0x40000000 |
| `_DCMD_PROC` | `devctl.h` 120 | 0x08 |
| `__PROC_SUBCMD_PROCFS` | `sys/dcmd_proc.h` 53 | 0 |
| `O_RDONLY` | `fcntl.h` | 0 |

The measured 0x41100801 is 0x40000000 + (272 << 16) + (8 << 8) + 1.

## Calls

`devctl`: `devctl.h` 69. It returns `EOK` (0) or an error number, not -1
and `errno`. `open` and `close` are the POSIX calls of `fcntl.h` and
`unistd.h`. All are in `libc.so.3`: the target test binary
(`cargo test --no-run --target armv7-unknown-nto-qnx650 -p qnx-procfs`)
links, its only `NEEDED` entries are `libc.so.3` and `libsocket.so.3`,
and `readelf --dyn-syms` lists `open`, `close` and `devctl` as undefined
imports.
