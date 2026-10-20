# hidkbm-ring: layouts and values

Every size, offset and value that `src/lib.rs` and `src/sys.rs` pin, with
the header it comes from. SDP header paths are relative to
`target/qnx6/usr/include` of the SDP; `hidinput_shm.h` is the C producer's
header (`apps/usbhidd/hidinput_shm.h` in the tree that holds usbhidd).

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
`const int off_pollfd_revents = (int)offsetof(struct pollfd, revents);`.
Measured with the target compiler. The const asserts in `src/lib.rs` and `src/sys.rs`
repeat the struct numbers, so a build fails if a struct drifts.

## Structs

| struct | header | size | offsets |
|---|---|---|---|
| `hidinput_shm_t` (`Shm`) | `hidinput_shm.h` | 2064 | head 4, kbd_present 8, ev 16 |
| `hidinput_ev_t` (`Event`) | `hidinput_shm.h` | 8 | dx 4 |
| `struct pollfd` (`PollFd`) | `poll.h` 70-74 | 8 | events 4, revents 6 |

## Values

| name | header | QNX 6.5 | host (Linux) |
|---|---|---|---|
| `HIDINPUT_MAGIC` (`MAGIC`) | `hidinput_shm.h` | 0x4b424d31 | same |
| `HIDINPUT_RING` (`RING`) | `hidinput_shm.h` | 256 | same |
| `O_RDONLY` (from qnx-shm) | `fcntl.h` 76 | 0 | 0 |
| `O_RDWR` (from qnx-shm) | `fcntl.h` | 2 | 2 |
| `O_CREAT` (from qnx-shm) | `fcntl.h` 107 | 0o400 (256) | 0o100 |
| `O_NONBLOCK` | `fcntl.h` 90 | 0o200 (128) | 0o4000 |
| `POLLIN` | `poll.h` 83 | 5 (`POLLRDNORM \| POLLRDBAND`) | 1 |
| `EINTR` | `errno.h` 57 | 4 | 4 |
| `nfds_t` (`NfdsT`) | `poll.h` 68 | `unsigned int`, 4 bytes | `unsigned long` |

The QNX values are the same as those of the QNX 6.5 `libc` fork the
consumers patched in before 0.2.0 (`src/unix/nto/mod.rs`: `O_NONBLOCK`
0o200, `POLLIN` `POLLRDNORM | POLLRDBAND`, `EINTR` 4, `nfds_t` `c_uint`),
so the calls the crate makes did not change. The shared-memory calls,
their constants and the 32-bit `off_t` are `qnx-shm`'s.
