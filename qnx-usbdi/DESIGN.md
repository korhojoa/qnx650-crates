# qnx-usbdi: design and the io-usb contract

Safe Rust interfaces for `libusbdi.so.2`, the QNX 6.5 USB client library.
The `io-usb` service controls USB devices. This library retains transfer
storage until the service finishes using it.
Code references to "contract item N" identify the numbered rules below.

On a non-QNX host the crate compiles to stubs that return
`Error::Unsupported`, so its users build and test on the host unchanged.

## Clock (`src/clock.rs`, compiled everywhere)

`init()` sets a process-local monotonic clock origin in `T0`.
`now_ms()` returns the elapsed milliseconds. It initializes `T0` if necessary.
`stamp()` supplies a `[ssss.mmm]` prefix. `log(args)` writes that prefix and
the message to standard error.
On QNX, `Instant` and condition-variable deadlines use `CLOCK_MONOTONIC` (value 2).
The `usbd_io` timeout runs in the separate `io-usb` process.
Transfer logs include the requested timeout and the locally measured elapsed time.

## FFI layer (`src/ffi.rs`)

`#[repr(C)]` mirrors, compiled on every target: `UsbdDeviceDescriptor`
(18), `UsbdConfigurationDescriptor` (10: `w_total_length` at offset 2,
natural alignment, one byte of tail padding, 9 on the wire),
`UsbdInterfaceDescriptor` (9),
`UsbdEndpointDescriptor` (8: `w_max_packet_size` at offset 4,
`b_interval` at 6, natural alignment as in the header, NOT packed),
`UsbdDeviceIdent` (20), `UsbdDeviceInstance` (36: `path` 0, `devno` 1,
`generation` 2, `ident` 4, `config` 24, `iface` 28, `alternate` 32),
`UsbdFuncs` (16 on 32-bit), `UsbdConnectParm` (48 on 32-bit). Sizes and
the offsets of every field the code reads are pinned by
`const _: () = assert!(...)`; the pointer-free ones unconditionally (so
host `cargo test` evaluates them), the pointer-bearing `UsbdFuncs` /
`UsbdConnectParm` asserts under `#[cfg(target_pointer_width = "32")]`.
`usbd_descriptors_t` is a union in C; the port only ever passes an
endpoint descriptor or a device descriptor to `usbd_open_pipe`, so the
argument is `*mut c_void` and the address of the owned, aligned
descriptor struct supplies the argument.

`#[cfg(target_os = "nto")] extern "C"` declarations for exactly the
functions used: `usbd_connect`, `usbd_disconnect`, `usbd_attach`,
`usbd_detach`, `usbd_select_config`, `usbd_select_interface`,
`usbd_open_pipe`, `usbd_close_pipe`, `usbd_reset_pipe`,
`usbd_abort_pipe`, `usbd_alloc_urb`, `usbd_free_urb`, `usbd_alloc`,
`usbd_free`, `usbd_descriptor`, `usbd_setup_vendor`, `usbd_setup_bulk`,
`usbd_io`, `usbd_urb_status`, `usbd_device_descriptor`. NOT
`usbd_string` (see contract item 12). Link: `build.rs` emits
`cargo:rustc-link-search=native=/opt/qnx650/target/qnx6/armle-v7/lib`
and `cargo:rustc-link-lib=dylib=usbdi` for `nto-qnx` targets only (the
`qsaplay` pattern; `libusbdi.so` exists there and on the host at
the target library directory). Constants from `usbdi.h` /
`usb100.h` (`USBD_VERSION 0x0101`, `USB_VERSION 0x0110`,
`USBD_CONNECT_WILDCARD 0xffffffff`, `URB_DIR_IN 1`, `URB_DIR_OUT 2`,
`URB_SHORT_XFER_OK 4`, `USBD_URB_STATUS_MASK 0xFF000000`,
`USBD_STATUS_CMP 0x01000000`, `USBD_STATUS_ABORTED 0x08000000`,
`USB_TYPE_STANDARD 0`, `USB_TYPE_VENDOR 0x40`, `USB_RECIPIENT_DEVICE 0`,
`USB_RECIPIENT_ENDPOINT 2`, `USB_DESC_CONFIGURATION 2`,
`USB_DESC_STRING 3`, `USB_DESC_INTERFACE 4`, `USB_DESC_ENDPOINT 5`,
`USB_ATTRIB_BULK 2`, `USB_ENDPOINT_IN 0x80`) are declared verbatim with
the header line they came from in a comment.

## The io-usb contract the safe layer enforces

The rules describe transfer results, ownership and teardown.
[API] identifies a native interface definition. [MEASURED] identifies an
observed implementation behavior, not qualification of a released application.
[INFERRED] identifies a conclusion that needs target verification.
EHCI and dwcotg are USB host-controller interfaces. Their timeout and
completion behavior can differ.

1. Control transfers are synchronous only: `usbd_io(urb, ctrl_pipe,
   NULL, NULL, timeout_ms)`. A completion callback on EP0 never fires.
   [MEASURED, both HCDs] After `usbd_io` returns `EOK`, `usbd_urb_status`
   gives status and actual length; for a transfer with a data stage
   (`len >= 1`) `status & USBD_URB_STATUS_MASK` must be `USBD_STATUS_CMP`
   and, for IN, `alen` is the byte count returned. A control transfer
   with no data stage (`len == 0`, the `CLEAR_FEATURE(ENDPOINT_HALT)`
   fallback of item 8) is judged by `usbd_io == EOK` alone. Its
   raw status pair is logged but is not used to determine success. The
   `usbd_io` timeout is honoured by EHCI and ignored by dwcotg, where a
   control stage is bounded by the
   HCD's own 1000 ms stage timeout and NAK budget and a timed-out stage
   fails the transfer with `USBD_STATUS_NOT_ACCESSED`. [MEASURED]
   The status WORD is what is judged, never an errno: io-usb's transfer
   request handler replies status 0 (so `usbd_io` returns `EOK`) when
   the transfer function returns 0 or 5 (`EIO`) and puts the URB's own
   status and length in the reply [RE: io-usb @0x115844..0x115890,
   `cmp r0,#5; cmpne r0,#0; bne` then `MsgReply_r(rcvid, 0, ...)`], and
   `usbd_urb_status` stores both out-values first and then RETURNS an
   errno it derives from the status word (`INPROG` -> `EBUSY` 16,
   `TIMEOUT` -> `ETIMEDOUT` 260, `ABORTED` -> `EINTR` 4, `CMP_ERR` ->
   `ENODEV` 19 for low code `DEV_NOANSWER` else `EIO` 5, `CMP` and
   `DEVICE_RESET` -> `EOK`) [RE: `usbd_urb_status` @0x8d30 + mapper
   @0x8cec in the SDP `libusbdi.so.2`, identical @0x8f24 / @0x8ee0 in
   the head unit's copy]. `sys::urb_status` reads the out-values and ignores the return value. Gating on it would
   discard the status and length of every non-`CMP` completion.
2. Bulk with a NULL callback returns immediately and completes in the
   background: the URB and buffer must outlive the transfer. [MEASURED]
   The port uses the async form (a completion callback that signals a
   condvar) with a ring of `usbd_alloc`'d buffers so several writes are
   queued and every completion is checked (status `CMP` and length ==
   requested). io-usb queues client URBs per endpoint and hands the HCD
   one at a time, in submission order (`URB_ENQUEUE_TAIL` is the default
   and the port never sets `ENQUEUE_HEAD`) [API; MEASURED for bulk-IN on
   dwcotg with 8 URBs; INFERRED for EHCI, unmeasured there].
   The ring's default depth is 4.
   Ordering applies within one pipe only. Before a control transfer that
   must follow queued bulk data, the application calls `BulkOut::flush`.
   `Device` does not automatically flush a separate bulk pipe.
   Interrupt OUT (`IntrOut`, `src/intr_out.rs`) uses one transfer at a time.
   `usbd_io` receives a NULL callback and can return before completion.
   The write polls the URB status until completion or the timeout (default 1000 ms).
   Status `CMP` indicates completion. The transferred length is logged.
   On submission failure or timeout, the pipe is aborted before buffer reuse.
3. Transfer buffers come from `usbd_alloc` (DMA-safe), never from the
   Rust heap. Copy in, submit, copy out. A control transfer stages
   through `usbd_alloc(max(len, 1))` so an empty slice's dangling pointer
   is never handed to io-usb (the dvbtuner shape). The largest OUT URB
   with a logged size on dwcotg is the NIC's 4096-byte TX; a 64 KiB OUT
   is one 128-packet hardware transfer DMA'd straight from the
   `usbd_alloc` block, so its completion and the physical contiguity of
   a 64 KiB `usbd_alloc` are [UNMEASURED on dwcotg]. `slot_bytes` must be
   >= the command buffer's capacity because one flush is copied into one
   slot. Applications must select a transfer size supported by the controller.
   Logs include the timestamp, requested bytes, status, actual length and completion latency.
4. The insertion callback runs on libusbdi's event thread. The removal
   callback runs there only when no transfer is outstanding on the
   attached device when the event is processed; otherwise libusbdi
   defers it and invokes it from whichever thread brings the device's
   in-flight count to zero: the event thread's URB completion, or the
   CALLER's thread inside `usbd_io` (a synchronous control transfer,
   `DeviceSys::control`, or a refused bulk submit), `usbd_descriptor`
   or `usbd_reset_device` [RE: SDP `libusbdi.so.2` armle-v7
   `usbdi_synchronise` @0x5940: mode 0 (the event thread's removal path
   @0x7f7c-0x7fb0, call @0x7f90) does `atomic_set(dev+20, 1)` and
   returns `EBUSY` when `dev+24 != 0`, on which the event thread skips
   the callback (@0x7f94-0x7f98); the deferred invoke is @0x5a00-0x5a3c,
   `blx [[conn+0x1c]+8](conn, dev+28)` when a decrement reaches zero
   with bit 0 set; decrementers: `usbd_io` @0x8f40 (control) and @0x8f74
   (refused submit), `usbd_reset_device` @0x8dd8, `usbdi_sync_io`
   @0x9614 (`usbd_descriptor`), event-thread completion @0x8030. The
   head unit's copy is identical in shape: `usbdi_synchronise` @0x5a0c,
   `blx` @0x5b08, `usbd_io` decrements @0x914c / @0x9180. ]
   Both callbacks only push the raw `UsbdDeviceInstance` onto a
   mutex-protected queue reached through a process-global `OnceLock`
   (the funcs table carries no user pointer); the removal callback also
   sets the per-device `gone` flag of every registered instance whose
   `(path, devno, iface)` matches. Because it may run nested inside any
   `usbd_*` call on the caller's thread, it takes no lock a caller can
   hold across such a call (only the queue's and the registry's; the
   bulk ring's mutex is released before `usbd_io`), never blocks and
   never calls back into libusbdi. All attach/open/transfer work happens
   on the caller's thread. The flag is registered from the PRE-attach
   instance, before `usbd_attach`. The library
   delivers a removal for an attached device with no URB in flight at
   once, so one that lands during the attach's three io-usb round trips
   (attach, device descriptor, control pipe) must already find the
   entry; every transfer checks the flag. Whether io-usb's write-back
   of the instance leaves `(path, devno, iface)` alone is unverified;
   `Host::attach` logs a difference.
   Every callback io-usb invokes (insertion, removal, the bulk
   completion of item 9) runs its WHOLE body with thread cancellation
   disabled: `cancel::CancelGuard` is its first local, before the null
   checks and before any log line or lock, and is dropped last.
   `usbd_disconnect` ends the event thread with `pthread_cancel` +
   `pthread_join` [RE: SDP @0x7be4-0x7bfc] and the cancel can land
   mid-callback; QNX cancellation is `pthread_exit` with no unwinding
   (`pthread.h` lines 254-255), and the `write(2)` inside `clock::log`
   is a cancellation point, so a callback cancelled there would leave
   the process-wide stderr lock (or the ring's mutex, held across the
   completion log line) locked for ever and the next log line, the
   `usbd_disconnect rc` line of `HostSys::drop` on the disconnecting
   thread, would block. With the state disabled the cancel stays
   pending until the callback has returned and is acted on at the event
   thread's next cancellation point (its next message to io-usb); with
   the default deferred cancel type, re-enabling does not itself exit
   the thread [RE: SDP `libc.so.3` `pthread_setcancelstate`
   @0x1fff0-0x20048]. A future callback that logs or locks inherits the
   rule: the guard is its first statement. On a non-QNX target the
   guard is a no-op.
5. `usbd_connect` parameters: `vusb = USB_VERSION` (0x0110, the USB spec
   version, NOT `USBD_VERSION`), `vusbd = USBD_VERSION`, `connect_wait =
   2` seconds (a failed connection reports the path in the error text),
   `funcs.nentries = 3` (`_USBDI_NFUNCS`), and `event = None`.
   The application supplies `ident` through `Host::connect`.
   A wildcard field accepts any value for that field.
   `io-usb` compares class fields against interfaces in its selected configuration.
   It sends one insertion event per matching interface.
   The library filters repeated events by `(path, devno)` and logs a second
   matching interface on the same device.
6. The raw configuration descriptor is fetched over EP0 with
   `usbd_descriptor(dev, 0, USB_DESC_CONFIGURATION, USB_TYPE_STANDARD |
   USB_RECIPIENT_DEVICE, 0, 0, buf, 1024)`; this io-usb caches only the
   device descriptor. The block is parsed BYTE-WISE (`bLength`,
   `bDescriptorType`, `bEndpointAddress`, `bmAttributes`, `bInterval` as
   `u8` indexes. `wMaxPacketSize` uses `u16::from_le_bytes`). No
   pointer into the block is ever cast to a descriptor type, neither
   `&*p.cast()` nor `ptr::read(p.cast())`: this kernel runs with
   `SCTLR.A = 1` and cannot be relaxed, so an endpoint descriptor at an
   odd offset (after any second endpoint or a 9-byte class descriptor)
   causes SIGBUS on the target device; x86 host tests cannot detect this.
   `Endpoint` is
   an owned `#[repr(C)]` value built field by field; its address is what
   `usbd_open_pipe` receives. `GET_DESCRIPTOR(CONFIGURATION, index)`
   addresses the device's LIST of configurations, not the active one, so
   the claim reads every block (`index` in `0..bNumConfigurations`,
   `Device::raw_config_at`) and `desc::find_display_interface` picks
   the first interface with class triple `0xFF/0x00/0x00` and a bulk
   OUT endpoint, remembering its `bConfigurationValue` (byte 5),
   interface and alternate. That block becomes the device's working
   block (`Device::set_config_block`) for every later walk (warn if the
   endpoint is not 1; refuse the device, with a log line, if no
   configuration has such an interface). `bNumInterfaces` is byte 4 of
   the block.
7. Configuration: if io-usb selected a configuration other than the
   display one (`inst.config != display.config`), the port issues
   `usbd_select_config(dev, display.config)` and then listens a bounded
   `reconfigure_settle_ms` (500) for io-usb's reaction: a removal plus
   insertion of the same `(path, devno)` means re-attach through the new
   instance; no event means the attached handle stays valid. [MEASURED on
   a target USB adapter: `select_config(1)` from configuration
   2 returned ok, no event followed, the handle stayed valid, and the
   next `usbd_connect` reported the instance with `cfg 1` and the
   `FF/00/00` ident: the switch persists across clients.] If io-usb had
   already selected the display configuration, `usbd_select_config` is
   issued only when `bNumInterfaces == 1` or `inst.config == 0` (device
   arrived unconfigured), with `cfg = if inst.config != 0 { inst.config }
   else { 1 }` (`usbhidd.c`). On a composite device that is already
   configured it is skipped: io-usb configured the device at enumeration
   and a SET_CONFIGURATION would reset the endpoint toggles of every
   other interface (a devb-umass "virtual CD", an io-hid dock button);
   libdlo could call `usb_set_configuration(1)` only because it first
   detached every kernel driver from every interface. `usbd_select_interface
   (dev, display.iface, display.alternate)` is always issued. A failure
   of either call is logged with `bNumInterfaces` and `inst.config`, not
   fatal.
8. Pipe hygiene at claim: `usbd_reset_pipe` on the bulk OUT pipe, with
   the `CLEAR_FEATURE(ENDPOINT_HALT)` fallback, so a
   previous unclean exit cannot leave the device's data toggle out of
   step. This also restores this port's own toggle state, which is why
   skipping SET_CONFIGURATION on a composite loses nothing.
9. Completion ownership and teardown: the ring state (in-flight counter,
   `Mutex` + `Condvar`, the URB pointers and `usbd_alloc` buffers) lives
   in one `Arc<RingState>`; every `usbd_io` submission passes
   `Arc::into_raw(clone)` as the handle and the callback does
   `Arc::from_raw`, records the slot's status/length, decrements the
   in-flight count and signals the condvar regardless of status
   (`ABORTED` counts as a completion), then releases its clone. URBs
   and buffers are freed in `RingState::drop`, which runs only on the
   caller's thread, only after `usbd_close_pipe` returned `EOK` and the
   caller's `Arc` was seen to be the last reference: the callback can
   never touch freed memory, and nothing is ever freed on libusbdi's
   event thread. The drain condition (in-flight == 0) is set INSIDE the
   callback and therefore precedes libusbdi's own accounting by the
   callback's return: the event thread decrements the per-pipe counter
   (`pipe+12`) and then the per-device counter (`dev+24`) only after
   the client callback returned [RE: SDP `libusbdi.so.2` armle-v7
   `usbdi_deliver_event` @0x8004-0x8014 `blx [urb+0x1c]`, then
   `atomic_sub(pipe+12, 1)` @0x8018-0x8020, then `usbdi_synchronise(dev,
   1, -1)` @0x8024-0x8030; head unit copy @0x7ff4 / @0x7ff8-0x8004 /
   @0x8008-0x8014]. `usbd_close_pipe` returns `EBUSY` (16) and does
   nothing while `pipe+12 != 0` [@0x5800-0x580c; head unit
   @0x58c0-0x58cc]; `usbd_detach` returns `EBUSY` and does nothing while
   `dev+24 != 0` [@0x5e78-0x5e84; head unit @0x5f4c-0x5f58]; and
   `usbd_disconnect` stops its detach loop at the first non-`EOK`
   detach [@0x7b10-0x7b18], destroys the two memchunk pools that
   `usbd_free` (@0x8ff0) and `usbd_free_urb` (@0x901c) free into when
   the last connection goes [@0x7b4c-0x7b58], and only then cancels and
   joins the event thread [@0x7bf0-0x7bfc]. That is why `close_pipe`
   and `detach` are retried on `EBUSY` and why the ring is never
   released to the event thread. Teardown order: `usbd_abort_pipe`;
   wait on the condvar until in-flight == 0, bounded by the write
   timeout; `usbd_close_pipe`, retried every 1 ms on `EBUSY` for up to
   1000 ms (`EOK` proves every callback has returned and released its
   clone); wait, same bound, until the caller's `Arc` is the last
   reference (a check that holds whatever the memory ordering between
   libusbdi's counter and the `Arc`'s); drop the ring (URBs + buffers
   freed, on the caller's thread); `usbd_detach`, retried the same way
   on `EBUSY` (the per-device counter is decremented a few instructions
   after the per-pipe one); `usbd_disconnect`. On a drain timeout, or
   when `close_pipe` is still `EBUSY` at its bound, log it, keep this
   side's `Arc` (the ring is leaked: a straggling callback can only
   decrement the count, never free into pools `usbd_disconnect` may
   already have destroyed; the memchunk destroy at disconnect reclaims
   the URBs and buffers) and leave that pipe to `usbd_detach`, which
   closes leftover pipes itself, retrying every 100 ms [@0x5f04-0x5f38;
   head unit @0x5fd8-0x600c]. A `usbd_detach` still `EBUSY` at its
   bound is logged at level 0: io-usb never received the detach and
   `usbd_disconnect` destroys the pools with the device attached.
   `usbd_abort_pipe` returning does not imply the client callbacks have
   run (they run on libusbdi's event thread), so the drain is mandatory.
   [MEASURED on dwcotg: freeing a URB before its callback corrupts
   in-flight state; abort completes every queued URB through the
   callback.]
10. Every log line carries the clock stamp above.
11. `Instance` is a newtype over the raw 36-byte `UsbdDeviceInstance`
    (`Copy`), with accessors `path() -> u8`, `devno() -> u8`,
    `generation() -> u16`, `vendor() -> u32`, `product() -> u32`,
    `config() -> u32`, `iface() -> u32`, `alternate() -> u32`. It is never
    rebuilt from parts: `usbd_attach` takes the whole struct (non-const
    pointer) and io-usb writes it back; `Host::attach` copies it to a
    local `mut`, passes `&mut local`, `extra = 0`, and stores the
    post-attach copy in `Device` for the removal match and the config /
    interface selection. VID and PID for logging come from the device
    descriptor after attach (`usbd_device_descriptor`).
12. Strings: `usbd_string` is not used. Its langid-0 form reads io-usb's
    parsed-descriptor cache (which holds only the device descriptor on
    this io-usb) and returns NULL with ENOENT; with a langid it returns a
    process-global buffer that the next call from any thread overwrites,
    and a code unit `wctomb` cannot encode in the C locale fails the
    whole call (from a disassembly of `libusbdi.so.2`, `usbd_string` at
    0x5184, performed during the review). `Device::string(index)` does
    what libdlo's `usb_get_string_simple` does: string descriptor 0 via
    `usbd_descriptor(dev, 0, USB_DESC_STRING, STANDARD|DEVICE, 0, 0, buf,
    255)` (require `bLength >= 4`, type 3; langid = first LANGID entry),
    then `usbd_descriptor(..., index, langid, buf, 255)` (type 3,
    `bLength <= 255`), decode `buf[2..bLength]` as UTF-16LE with every
    code unit above 0x7F mapped to `?` so the serial is deterministic.
13. Deadlines: `Host::wait_events(settle_ms)` sleeps a bounded time and
    drains; nothing in the crate waits for a plug. The EDID read in `dlo`
    stops at the first failed transfer, non-CMP status or non-zero I2C
    status byte, has an overall budget (5000 ms) and logs measured
    per-transfer milliseconds.

## Safe API (`src/lib.rs`)

```
pub struct Ident { pub vendor: u32, pub device: u32, pub class: u32, pub subclass: u32, pub protocol: u32 }  // WILDCARD = 0xffff_ffff
pub struct Host                      // one usbd_connection; the event queue is process-global (one Host per process)
impl Host {
    pub fn connect(path: Option<&str>, ident: Ident) -> Result<Host>;
    pub fn poll(&self) -> Vec<Event>;                        // drains insertions/removals seen so far
    pub fn wait_events(&self, settle_ms: u32) -> Vec<Event>; // sleeps settle_ms then poll()
    pub fn attach(&self, inst: &Instance) -> Result<Device>;
}
pub enum Event { Inserted(Instance), Removed(Instance) }
pub struct Instance { raw: ffi::UsbdDeviceInstance }        // Copy; accessors listed in item 11
pub struct Endpoint { .. }                                  // owned repr(C) endpoint descriptor; address(), attributes(), max_packet(), interval()
pub struct Device                    // usbd_device + control pipe + descriptors
impl Device {
    pub fn instance(&self) -> Instance;                  // the post-attach copy
    pub fn descriptor(&self) -> DeviceDescriptor;        // vid, pid, bcd, string indices, num_configurations
    pub fn string(&self, index: u8) -> Option<String>;
    pub fn raw_config(&self) -> Result<Vec<u8>>;         // the configuration descriptor block (cached after first read)
    pub fn num_interfaces(&self) -> Result<u8>;
    pub fn endpoints(&self, iface: u8, alt: u8) -> Result<Vec<Endpoint>>;   // byte-wise walk
    pub fn interface_class(&self, iface: u8, alt: u8) -> Result<(u8, u8, u8)>;
    pub fn select_config(&self, cfg: u8) -> Result<()>;
    pub fn select_interface(&self, iface: u8, alt: u8) -> Result<()>;
    pub fn control_in(&self, req: u8, rtype: u8, value: u16, index: u16, buf: &mut [u8], timeout_ms: u32) -> Result<usize>;
    pub fn control_out(&self, req: u8, rtype: u8, value: u16, index: u16, data: &[u8], timeout_ms: u32) -> Result<()>;
    pub fn get_descriptor(&self, dtype: u8, index: u8, langid: u16, buf: &mut [u8]) -> Result<usize>;
    pub fn open_bulk_out(&self, ep: &Endpoint, slots: usize, slot_bytes: usize) -> Result<BulkOut>;
    pub fn open_intr_out(&self, ep: Endpoint) -> Result<IntrOut>;
    pub fn gone(&self) -> bool;
}
pub struct BulkOut                   // pipe + Arc<RingState>
impl BulkOut {
    pub fn write(&mut self, data: &[u8], timeout_ms: u32) -> Result<()>; // copies into a free slot, submits async; waits (bounded) for a slot if all are busy
    pub fn flush(&mut self, timeout_ms: u32) -> Result<()>;              // waits for every in-flight URB, reports the first failure
    pub fn reset(&mut self) -> Result<()>;                               // usbd_reset_pipe, then the CLEAR_FEATURE fallback
    pub fn stats(&self) -> BulkStats;                                    // urbs, bytes, failures, max latency ms
}
pub struct IntrOut                   // interrupt OUT pipe + its one URB, one synchronous usbd_io at a time (item 2)
impl IntrOut {
    pub fn write(&mut self, data: &[u8], timeout_ms: u32) -> Result<()>; // 1..=wMaxPacketSize bytes; waits for the transfer; aborts the pipe if usbd_io fails
    pub fn reset(&mut self) -> Result<()>;                               // usbd_reset_pipe, then the CLEAR_FEATURE fallback
    pub fn stats(&self) -> IntrOutStats;                                 // writes, bytes, failures, max usbd_io ms
}
pub enum Error { Errno(i32), Status(u32), Short { want: usize, got: usize }, Gone, Timeout, Interrupted, NoEndpoint, BadDescriptor, Unsupported, Connect { path: String, errno: i32 } }
pub mod stop { pub fn request(); pub fn requested() -> bool; }         // the process-wide stop flag the ring's waits poll
```

The rings `BulkIn` (`src/bulk_in.rs`) and `IntrIn` (`src/intr.rs`) are
documented in their modules.

`Device` and `BulkOut` do not implement `Send` or `Sync`.
The internal completion path releases submitted transfers.
`Drop` runs the teardown in item 9.

Applications can call `stop::request()` from a signal handler.
`BulkOut::write` and `flush` check the stop flag every 250 ms and return
`Error::Interrupted`. Teardown retains transfer storage until completion.
After a stop request, its drain timeout is capped at 1 second.
A synchronous control transfer or connection attempt can still delay shutdown.
Applications must account for these native call bounds when they select a shutdown timeout.
