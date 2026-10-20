<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Terms

- ARM is the processor architecture. QNX 6.5 is the operating system these interfaces target.
- An ABI, or application binary interface, defines function calls and memory layouts between compiled components.
- SDK means software development kit. This repository does not supply its headers or libraries.
- HID means Human Interface Device, such as a keyboard or mouse.
- A ring is a fixed-size queue whose read and write positions wrap around.
- POSIX defines operating-system interfaces, including shared memory.
- A resource manager supplies a QNX device interface. Dispatch processes its requests.
- MMIO means memory-mapped input/output, which exposes device registers through memory addresses.
- A system page contains operating-system and hardware information shared with applications.
- USB is the Universal Serial Bus. The `qnx-usbdi` library communicates with the QNX `io-usb` service.
- URB means USB Request Block, which stores one transfer request and its result.
- DMA means Direct Memory Access, which lets hardware transfer data without processor copying.
- EHCI and dwcotg identify USB host-controller interfaces with different transfer behavior.
- Cargo builds Rust packages. Clippy checks Rust code for common errors.


- MIB2 High is a Volkswagen Group infotainment platform that uses ARM QNX.

[README](../README.md)
