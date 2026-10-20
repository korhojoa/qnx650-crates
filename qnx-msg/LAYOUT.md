# qnx-msg: layouts and values

Every size, offset and value that `src/lib.rs` pins, with the SDP 6.5.0
header line it comes from. Header paths are relative to
`target/qnx6/usr/include` of the SDP.

## How the numbers were measured

A C probe that stores each `sizeof`, `offsetof` and macro value in a
`const int`, compiled to assembly (no linking) with the target compiler
of the compiler image and the SDP headers, then read back from the
`.word` lines:

```sh
podman run --rm -v "$PWD:/w" -v "$SDP/target/qnx6/usr/include:/sdp:ro" -w /w \
    ghcr.io/korhojoa/qnx650-compiler@sha256:85f95b995d4b5dc2c323a6842c29b5c08652f34cc2288f6cc745534f5b2ecc7a \
    /opt/gcc11/bin/arm-unknown-nto-qnx6.5.0eabi-gcc -isystem /sdp -D_QNX_SOURCE \
    -S -O0 -o layout.s layout.c
```

Each probe line is `const int NAME = (int)(EXPR);`, for example
`const int sz_msg_info = (int)sizeof(struct _msg_info);`. The compiler
defines `__QNXNTO__`, `__LITTLEENDIAN__` and `__ARM_EABI__`. Measured with the target compiler. The const asserts in `src/lib.rs` repeat every number below
for 32-bit builds, so a target build fails if a struct drifts.

## Structs

| struct | header | size | offsets |
|---|---|---|---|
| `struct _msg_info` (`MsgInfo`) | `sys/neutrino.h` 113-127 | 48 | pid 8, scoid 20, priority 40, flags 42, reserved 44 |
| `struct _pulse` (`Pulse`) | `sys/neutrino.h` 226-233 | 16 | code 4, value 8, scoid 12 |
| `struct sigevent` (`Sigevent`) | `sys/siginfo.h` 88-130 | 16 | sigev_coid 4, sigev_value 8, sigev_code 12, sigev_priority 14 |
| `iov_t` (`Iov`) | `sys/target_nto.h` 154-158 | 8 | iov_len 4 |
| `name_attach_t` (`NameAttach`) | `sys/dispatch.h` 117-122 | 20 | chid 4, mntid 8 |

Notes:

- `_msg_info` is 48 bytes, not 52: `priority` and `flags` are two
  `_Int16t` in one word. (An earlier hand copy said 52 in a comment; its
  Rust struct was 48 and correct.)
- `union sigval` is 4 bytes on the target (an `int` or a 32-bit
  pointer), so `Pulse::value` and `Sigevent::value` are `c_int`.
- The second union of `struct sigevent` is the `short` pair
  `sigev_code`, `sigev_priority` or the `pthread_attr_t *`
  `sigev_notify_attributes`; the crate carries the pair.
- The headers include `_pack64.h`, which is `#pragma pack()` (the
  default packing) for GCC, so there is no extra packing.

## Values

| name | header | value |
|---|---|---|
| `SIGEV_PULSE` | `sys/siginfo.h` 76 | 4 |
| `SIGEV_INTR` | `sys/siginfo.h` 78 | 6 |
| `_NTO_SIDE_CHANNEL` | `sys/neutrino.h` 370 | 0x40000000 |
| `_NTO_TIMEOUT_RECEIVE` | `sys/neutrino.h` 101, `STATE_RECEIVE` = 5 in `sys/states.h` 45 | 0x20 |
| `_NTO_TIMEOUT_INTR` | `sys/neutrino.h` 110, `STATE_INTR` = 16 in `sys/states.h` 58 | 0x10000 |
| `_NTO_INTR_FLAGS_TRK_MSK` | `sys/neutrino.h` 388 | 0x08 |
| `_PULSE_CODE_UNBLOCK` | `sys/neutrino.h` 215 | -32 |
| `_PULSE_CODE_DISCONNECT` | `sys/neutrino.h` 216 | -33 |
| `_NOTIFY_COND_INPUT` | `sys/iomgr.h` 56 | 0x10000000 |
| `_NOTIFY_COND_OUTPUT` | `sys/iomgr.h` 55 | 0x20000000 |
| `_NOTIFY_ACTION_POLL` | `sys/iomgr.h` 118 | 0x2 |
| `_NOTIFY_ACTION_POLLARM` | `sys/iomgr.h` 119 | 0x3 |

## Calls

Prototypes in `sys/neutrino.h`: `ChannelCreate` 447, `ChannelDestroy`
450, `ConnectAttach` 452, `ConnectDetach` 455, `MsgSend` 466, `MsgSendv`
478, `MsgReceive` 482, `MsgReceivePulse` 486, `MsgReply` 490, `MsgReplyv`
492, `MsgSendPulse` 504, `MsgError` 514, `InterruptAttachEvent` 553,
`InterruptDetach` 557, `InterruptWait` 559, `TimerTimeout` 583,
`InterruptUnmask` 642. `ionotify`: `sys/iomgr.h` 151. `name_attach`,
`name_detach`, `name_open`, `name_close`: `sys/dispatch.h` 124-128.

`clockid_t` is `int` (`sys/target_nto.h` 139). The byte counts of
`MsgSend`, `MsgReceive` and `MsgReply` are `int` in the headers.

All 22 calls are in `libc.so.3`: the target test binary
(`cargo test --no-run --target armv7-unknown-nto-qnx650 -p qnx-msg`)
links, its only `NEEDED` entries are `libc.so.3` and `libsocket.so.3`, and
`readelf --dyn-syms` lists each of the 22 as an undefined import.
