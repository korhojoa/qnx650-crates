# qnx-qsa: layouts and values

Every size, offset and value that `src/lib.rs` pins, with the SDP 6.5.0
header line it comes from. Header paths are relative to
`target/qnx6/usr/include` of the SDP. `sys/asound.h` has no packing
pragma, so every structure has the default ARM EABI layout (64-bit
members 8-byte aligned).

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

The probe includes `sys/asoundlib.h`. Each line is
`const int NAME = (int)(EXPR);`, for example
`const int par_frag_size = (int)offsetof(snd_pcm_channel_params_t, buf.block.frag_size);`.
Measured with the target compiler. The const asserts in `src/lib.rs` repeat the numbers
below; the asserts of `snd_mixer_group_t` and `snd_mixer_groups_t` (which
hold pointers) apply to 32-bit builds only.

An earlier check of the same three PCM structures: the object
`vchiq/deva-vcaud/libqsashim.a` of the Raspberry-QNX repository (compiled
C) clears 752, 728 and 184 bytes for them and uses the offsets 4, 24, 28,
32, 36, 472, 476, 488, 492, 496 (parameters), 16, 456 (setup) and 8
(status). The measurement below agrees.

## Structs

| struct | header | size | offsets |
|---|---|---|---|
| `snd_pcm_sync_t` (union) | `sys/asound.h` 1208-1214 | 16, align 8 | |
| `snd_pcm_digital_t` (`PcmDigital`) | `sys/asound.h` 1216-1223 | 304 | dig_subframe 172, reserved 176 |
| `snd_pcm_format_t` (`PcmFormat`) | `sys/asound.h` 1256-1264 | 144 | format 4, rate 8, voices 12, special 16, reserved 20 |
| `snd_pcm_channel_params_t` (`ChannelParams`) | `sys/asound.h` 1273-1305 | 752 | mode 4, sync 8, format 24, digital 168, start_mode 472, stop_mode 476, why_failed 484, buf 488 (block: frag_size 488, frags_min 492, frags_max 496), sw_mixer_subchn_name 624, reserved 656 |
| `snd_pcm_channel_setup_t` (`ChannelSetup`) | `sys/asound.h` 1308-1341 | 728 | format 8, digital 152, buf.block: frag_size 456, frags 460, frags_min 464, frags_max 468, max_frag_size 472; msbits_per_sample 600, mixer_device 604, mixer_eid 608, mixer_gid 612, mixer_card 620, reserved 624 |
| `snd_pcm_channel_status_t` (`ChannelStatus`) | `sys/asound.h` 1343-1359 | 184 | status 8, scount 12, stime 16, ust_stime 24, frag 32, count 36, free 40, underrun 44, overrun 48, overrange 52, subbuffered 56, reserved 60 |
| `snd_mixer_gid_t` (`RawGroupId`) | `sys/asound.h` 863-870 | 168 | name 4, index 36, reserved 40, weight 164 |
| `snd_mixer_group_t` (`RawGroup`) | `sys/asound.h` 872-902 | 496 | caps 168, channels 172, min 176, max 180, volume 184 (union of 152 bytes), mute 336, capture 340, capture_group 344, elements_size 348, pelements 360, change_duration 364, min_dB 368, max_dB 372, reserved 376 |
| `snd_mixer_groups_t` (`RawGroups`) | `sys/asound.h` 929-936 | 152 | groups_size 4, groups_over 8, pgroups 16, pzero 20, reserved 24 |

Notes:

- The buffer union of the parameters is 136 bytes, not the 128 of its
  `reserved` member: its `block` member is three `int32_t` and 124 bytes.
  The setup's union is 144 bytes for the same reason (five words and 124
  bytes). The crate carries the `block` members.
- The volume union of a group is 152 bytes: the `names` member (six
  words and 128 bytes) is larger than `values[32]` (128 bytes). The crate
  carries `values` and 24 bytes after it.
- The two pointers of the setup (`mixer_eid`, `mixer_gid`) are carried
  as `u32` and never followed, so that structure has the same layout on a
  64-bit host. The pointers of the mixer structures are real pointers.

## Values

| name | header | value |
|---|---|---|
| `SND_PCM_OPEN_PLAYBACK` | `sys/asoundlib.h` 187 | 1 |
| `SND_PCM_CHANNEL_PLAYBACK` | `sys/asound.h` 1010 | 0 |
| `SND_PCM_SFMT_S16_LE` | `sys/asound.h` 1018 | 4 |
| `SND_PCM_MODE_BLOCK` | `sys/asound.h` 1122 | 1 |
| `SND_PCM_STATUS_NOTREADY` ... `PAUSED` | `sys/asound.h` 1160-1166 | 0, 1, 2, 3, 4, 5, 10 |
| `SND_PCM_STATUS_ERROR`, `SND_PCM_STATUS_CHANGE` | `sys/asound.h` 1167-1168 | 10000, 10001 |
| `SND_PCM_START_FULL` | `sys/asound.h` 1172 | 2 |
| `SND_PCM_STOP_STOP` | `sys/asound.h` 1176 | 1 |
| `SND_MIXER_CHN_FRONT_LEFT`, `SND_MIXER_CHN_FRONT_RIGHT` | `sys/asound.h` 826-827 | 0, 1 |
| `SND_MIXER_GRPCAP_VOLUME` | `sys/asound.h` 851 | 1 |
| `SND_MIXER_GRP_MAX_VOICES` | `sys/asound.h` 861 | 32 |

## Calls

Prototypes in `sys/asoundlib.h`: `snd_strerror` 53, `snd_mixer_open`
141, `snd_mixer_close` 143, `snd_mixer_groups` 148,
`snd_mixer_group_read` 149, `snd_mixer_group_write` 150,
`snd_pcm_open_preferred` 201, `snd_pcm_close` 202,
`snd_pcm_plugin_params` 268, `snd_pcm_plugin_setup` 269,
`snd_pcm_plugin_status` 270, `snd_pcm_plugin_prepare` 271,
`snd_pcm_plugin_playback_drain` 272, `snd_pcm_plugin_flush` 273,
`snd_pcm_plugin_write` 275 (returns `ssize_t`).

All 15 calls are in `libasound.so.2`: the target test binary
(`cargo test --no-run --target armv7-unknown-nto-qnx650 -p qnx-qsa`, whose
`links_every_call` test takes each address) links, its `NEEDED` entries
are `libasound.so.2`, `libc.so.3` and `libsocket.so.3`, and
`readelf --dyn-syms` lists each of the 15 as an undefined import.
