# qnx-resmgr: layouts and values

Every size, offset and value that `src/lib.rs` pins, with the SDP 6.5.0
header line it comes from. Header paths are relative to
`target/qnx6/usr/include` of the SDP. The numbers were measured the same
way as for `qnx-msg` (see `../qnx-msg/LAYOUT.md`): a C probe of `sizeof`,
`offsetof` and macro values, compiled to assembly with the image's
`arm-unknown-nto-qnx6.5.0eabi-gcc` and the SDP headers, measured with the target compiler. The const asserts in `src/lib.rs` repeat them for 32-bit
builds.

## Structs

| struct | header | size | offsets |
|---|---|---|---|
| `resmgr_context_t` (`ResmgrContext`) | `sys/resmgr.h` 107-119 | 92 with one `iov` (84 before `iov[]`) | info 4, msg 52, dpp 56, id 60, extra 64, msg_max_size 68, status 72, offset 76, size 80, iov 84 |
| `resmgr_attr_t` (`ResmgrAttr`) | `sys/dispatch.h` 90-96 | 32 | other_func 12, reserved 16 |
| `resmgr_connect_funcs_t` (`ConnectFuncs`) | `sys/resmgr.h` 127-137 | 36 | open 4, mount 32; `_RESMGR_CONNECT_NFUNCS` = 8 (line 138) |
| `resmgr_io_funcs_t` (`IoFuncs`) | `sys/resmgr.h` 144-172 | 108 | read 4, devctl 24, lseek 36, msg 76, power 104; `_RESMGR_IO_NFUNCS` = 26 (line 173) |
| `iofunc_attr_t` (`IofuncAttr`) | `sys/iofunc.h` 102-165 | 88, align 8 | flags 4, lock_count 12, mmap_list 24, reserved 36, nbytes 40, inode 48, uid 56, mtime 64, mode 76, nlink 80, rdev 84 |
| `struct _io_connect` (`IoConnect`) | `sys/iomsg.h` 165-183 | 40 to `path[]` (C `sizeof` 44) | file_type 4, key 12, sflag 28, path_len 34, eflag 36, extra_len 38, path 40 |
| `struct _io_read` (`IoRead`) | `sys/iomsg.h` 420-426 | 16 | nbytes 4, xtype 8 |
| `struct _io_devctl` (`IoDevctl`) | `sys/iomsg.h` 498-505 | 16 | dcmd 4, nbytes 8 |
| `struct _io_devctl_reply` (`IoDevctlReply`) | `sys/iomsg.h` 507-513 | 16 | ret_val 4 |

Notes:

- `iofunc_attr_t`: with `_IOFUNC_OFFSET_BITS` and `_FILE_OFFSET_BITS`
  not defined (the default), `nbytes` is `off64_t` and `inode` is
  `ino64_t` (lines 116-123), so the struct is 8-aligned. `uid_t` and
  `gid_t` are `_INT32`, `time_t`, `mode_t`, `nlink_t` and `dev_t` are
  `_Uint32t` (`sys/target_nto.h` 46, 49, 121, 127, 130, 133). An earlier
  hand copy carried the attribute as an opaque 256-byte block; the real
  size is 88.
- `struct _io_connect`: the flexible `path` member starts at 40. The C
  `sizeof` is 44 because the header's `__FLEXARY` gives `path[1]` plus
  padding, so the path is at `size_of::<IoConnect>()` in Rust (40). An
  earlier hand copy's comment said 36; its Rust struct was 40 and
  correct.
- `resmgr_context_t` ends in the flexible `iov[]`, allocated by the
  framework with `nparts_max` entries; `ResmgrContext` names `iov[0]`.

## Values

| name | header | value |
|---|---|---|
| `_IO_READ` | `sys/iomsg.h` 92 | 0x101 |
| `_IO_WRITE` | `sys/iomsg.h` 93 | 0x102 |
| `_IO_STAT` | `sys/iomsg.h` 95 | 0x104 |
| `_IO_NOTIFY` | `sys/iomsg.h` 96 | 0x105 |
| `_IO_DEVCTL` | `sys/iomsg.h` 97 | 0x106 |
| `_IO_LSEEK` | `sys/iomsg.h` 100 | 0x109 |
| `_IO_MSG` | `sys/iomsg.h` 110 | 0x113 |
| `_IO_XTYPE_NONE` | `sys/iomsg.h` 122 | 0 |
| `_IO_XTYPE_MASK` | `sys/iomsg.h` 130 | 0xff |
| `_RESMGR_DEFAULT` | `sys/resmgr.h` 176 | 0x80000001 (-2147483647 as `int`) |
| `_RESMGR_NPARTS(n)` | `sys/resmgr.h` 177 | -n |
| `_RESMGR_PTR(ctp, p, n)` | `sys/resmgr.h` 179 | `SETIOV(ctp->iov, p, n)`, then -1 |
| `_RESMGR_FLAG_BEFORE` | `sys/resmgr.h` 240 | 0x0001 |
| `_FTYPE_ANY` | `sys/ftype.h` 31 | 0 |
| `S_IFCHR` | `sys/stat.h` | 0x2000 (measured; for `iofunc_attr_init`) |

## Calls

`sys/dispatch.h`: `dispatch_create` 217, `resmgr_attach` 98,
`resmgr_context_alloc` 102, `resmgr_block` 103, `resmgr_handler` 105.
`sys/iofunc.h`: `iofunc_func_init` 325, `iofunc_close_ocb_default` 331,
`iofunc_devctl_default` 332, `iofunc_open_default` 337,
`iofunc_attr_init` 365, `iofunc_read_verify` 377.

All 11 calls are in `libc.so.3`: the target test binary
(`cargo test --no-run --target armv7-unknown-nto-qnx650 -p qnx-resmgr`)
links, its only `NEEDED` entries are `libc.so.3` and `libsocket.so.3`, and
`readelf --dyn-syms` lists each of the 11 as an undefined import.
