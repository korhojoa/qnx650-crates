// SPDX-License-Identifier: Apache-2.0
//! HID event rings for USB, Bluetooth, gamepads and touch input.
//! Each producer uses one shared-memory object with this 2064-byte layout:
//!
//! ```text
//! u32 magic      "KBM1" = 0x4b424d31 while the producer lives, 0 after
//! u32 head       next write index, free-running (mod 2^32)
//! u8  kbd_present, u8 mouse_present, u8 reserved[6]
//! ev[256] of { u8 kind, u8 code, u8 down, u8 src, i16 dx, i16 dy }
//! ```
//!
//! Single producer, any number of readers, lock-free: the producer writes
//! `ev[head % 256]`, a barrier, then `head + 1`; a reader keeps its own tail
//! and drops the backlog when it falls more than 256 behind.
//!
//! Waking readers: the ring itself has no notification. Next to it the
//! producer keeps a PPS object, `<pps root>/hidkbm/<ring name>`, and writes
//! `head::<n>` to it after each batch of events ([`Wake::publish`]; the
//! `shm` feature's `Producer::publish` does it for its ring). A reader opens
//! that object in delta mode and sleeps in `poll()` until it changes
//! (`Reader`, `wait_any`). The PPS root is `HIDKBM_PPS_ROOT`, else
//! `/ramdisk/pps` (the MIB2 unit), else `/pps` (the alternate QNX PPS root).
//! The producer writes `live::1` when it starts and `live::0` when it leaves.
//! A reader trusts the object only while it says `live::1`: without a PPS
//! server, or with a producer that does not publish, the ring works as
//! before and the reader looks every [`FALLBACK_INTERVAL`] (every
//! [`ABSENT_INTERVAL`] while the ring is missing).
//!
//! Without the default `shm` feature the crate is only this: the layout
//! ([`Shm`], [`Event`], the constants), the PPS paths and [`Wake`], with no
//! dependencies. That is what producers that map their ring themselves
//! use. The `shm` feature adds `qnx-shm` and
//! its own `open`/`read`/`poll` declarations (`LAYOUT.md`).
//!
//! Readers that are driven by their own frame loop (the SDL input module)
//! need no wake and keep reading the ring as they do; the layout does not
//! change.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{fence, AtomicU32, Ordering};
use std::time::Duration;

#[cfg(feature = "shm")]
mod shm;
#[cfg(feature = "shm")]
mod sys;
#[cfg(feature = "shm")]
pub use shm::{wait_any, Producer, Reader, Waited};

/// `HIDINPUT_MAGIC`.
pub const MAGIC: u32 = 0x4b42_4d31;
/// `HIDINPUT_RING`: events in a ring.
pub const RING: usize = 256;

/// Ring names.
pub const NAME_USB: &str = "/hidkbm-usb";
pub const NAME_BT: &str = "/hidkbm-bt";
pub const NAME_GIP: &str = "/hidkbm-gip";
pub const NAME_TOUCH: &str = "/hidkbm-touch";
pub const NAME_TOUCH_USB0: &str = "/hidkbm-touch-usb0";

/// `HIDINPUT_EV_*`.
pub const EV_KEY: u8 = 1;
pub const EV_MBUTTON: u8 = 2;
pub const EV_MMOTION: u8 = 3;
pub const EV_MWHEEL: u8 = 4;
pub const EV_TOUCH: u8 = 5;
/// `HIDINPUT_TOUCH_*`.
pub const TOUCH_UP: u8 = 0;
pub const TOUCH_DOWN: u8 = 1;
pub const TOUCH_MOVE: u8 = 2;

const NO_PPS: &str = "no PPS server (no /ramdisk/pps or /pps)";
/// [`RING`] as the type of the head.
const RING_U32: u32 = 256;
const _: () = assert!(RING_U32 as usize == RING);

/// How often a reader looks at a live ring whose producer does not publish
/// (no PPS server, or a producer built before the PPS wake).
pub const FALLBACK_INTERVAL: Duration = Duration::from_millis(10);
/// How often a reader looks for a missing ring whose PPS object does not
/// say a publishing producer is live, to notice a producer that does not
/// publish when it starts.
pub const ABSENT_INTERVAL: Duration = Duration::from_millis(250);

/// `hidinput_ev_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub kind: u8,
    pub code: u8,
    pub down: u8,
    pub src: u8,
    pub dx: i16,
    pub dy: i16,
}

const _: () = assert!(std::mem::size_of::<Event>() == 8);

/// `hidinput_shm_t`: the ring as it lies in the shared-memory object. The
/// `shm` feature's `Producer` and `Reader` map it; a producer that maps the
/// object itself casts its mapping of
/// [`SHM_BYTES`] to this type and uses these methods.
#[repr(C)]
pub struct Shm {
    pub(crate) magic: AtomicU32,
    pub(crate) head: AtomicU32,
    pub(crate) kbd_present: u8,
    pub(crate) mouse_present: u8,
    pub(crate) reserved: [u8; 6],
    pub(crate) ev: [Event; RING],
}

/// The size of the shared-memory object.
pub const SHM_BYTES: usize = std::mem::size_of::<Shm>();
const _: () = assert!(SHM_BYTES == 2064);

impl Shm {
    /// Reset for a new producer: clear everything, then set the magic last.
    pub fn reset(&mut self) {
        self.head.store(0, Ordering::Relaxed);
        self.kbd_present = 0;
        self.mouse_present = 0;
        self.reserved = [0; 6];
        self.ev = [Event::default(); RING];
        fence(Ordering::SeqCst);
        self.magic.store(MAGIC, Ordering::SeqCst);
    }

    /// Append one event: the slot, a barrier, then the head.
    pub fn push(&mut self, ev: Event) {
        let head = self.head.load(Ordering::Relaxed);
        self.ev[usize::try_from(head).unwrap_or(0) % RING] = ev;
        fence(Ordering::SeqCst);
        self.head.store(head.wrapping_add(1), Ordering::SeqCst);
    }

    /// The next write index (what [`Wake::publish`] takes).
    #[must_use]
    pub fn head(&self) -> u32 {
        self.head.load(Ordering::SeqCst)
    }

    /// Set the live device counts (informational).
    pub fn set_present(&mut self, kbd: u8, mouse: u8) {
        self.kbd_present = kbd;
        self.mouse_present = mouse;
    }

    /// The live device counts: `(kbd_present, mouse_present)`.
    #[must_use]
    pub fn present(&self) -> (u8, u8) {
        (self.kbd_present, self.mouse_present)
    }

    /// True while the magic says a producer is live.
    #[must_use]
    pub fn live(&self) -> bool {
        self.magic.load(Ordering::Acquire) == MAGIC
    }

    /// The event in the slot of write index `index` (`index % RING`).
    #[must_use]
    pub fn event(&self, index: u32) -> Event {
        self.ev[usize::try_from(index).unwrap_or(0) % RING]
    }

    /// Clear the magic: readers see the producer leave.
    pub fn leave(&mut self) {
        self.magic.store(0, Ordering::SeqCst);
    }
}

/// The PPS root to use: `HIDKBM_PPS_ROOT`, else the first of
/// `/ramdisk/pps` and `/pps` that is a directory.
#[must_use]
pub fn pps_root() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("HIDKBM_PPS_ROOT") {
        return Some(PathBuf::from(root));
    }
    ["/ramdisk/pps", "/pps"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_dir())
}

/// The PPS object of ring `name` under `root`.
#[must_use]
pub fn pps_object(root: &Path, name: &str) -> PathBuf {
    root.join("hidkbm").join(name.trim_start_matches('/'))
}

/// The PPS object a producer writes to wake the readers of its ring. A
/// producer that maps its ring itself holds one of these
/// and calls [`Wake::publish`] after its pushes.
pub struct Wake {
    object: Option<(PathBuf, File)>,
    error: Option<String>,
    published: Option<u32>,
}

impl Wake {
    /// Open (and create) the PPS object of ring `name`. Never fails: without
    /// a PPS server the ring works as before and [`Self::error`] says why
    /// readers are not woken.
    #[must_use]
    pub fn open(name: &str) -> Wake {
        let mut wake = Wake {
            object: None,
            error: None,
            published: None,
        };
        let Some(root) = pps_root() else {
            wake.error = Some(String::from(NO_PPS));
            return wake;
        };
        let object = pps_object(&root, name);
        if let Some(dir) = object.parent() {
            // PPS makes a directory on mkdir; an existing one is fine.
            let _ = std::fs::create_dir(dir);
        }
        match OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&object)
        {
            Ok(file) => {
                wake.object = Some((object, file));
                wake.write("head::0\nlive::1\n");
            }
            Err(e) => wake.error = Some(format!("open({}): {e}", object.display())),
        }
        wake
    }

    /// The PPS object, if it could be opened.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.object.as_ref().map(|(p, _)| p.as_path())
    }

    /// Why readers are not woken, if they are not.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Wake the readers if `head` (the ring's head after the producer's
    /// pushes) moved since the last call: one PPS write.
    pub fn publish(&mut self, head: u32) {
        if self.published != Some(head) {
            self.published = Some(head);
            self.write(&format!("head::{head}\n"));
        }
    }

    fn write(&mut self, text: &str) {
        let Some((object, file)) = self.object.as_mut() else {
            return;
        };
        // One write per update: PPS takes each write as one change.
        if let Err(e) = file.write_all(text.as_bytes()) {
            self.error = Some(format!("write({}): {e}", object.display()));
            self.object = None;
        }
    }
}

impl Drop for Wake {
    /// Tell the readers the producer left.
    fn drop(&mut self) {
        self.write("live::0\n");
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    // The tests share the process environment (HIDKBM_PPS_ROOT).
    pub(crate) static SERIAL: Mutex<()> = Mutex::new(());

    // Used by the shm module's tests.
    #[cfg(feature = "shm")]
    pub(crate) fn key(code: u8) -> Event {
        Event {
            kind: EV_KEY,
            code,
            down: 1,
            src: 0,
            dx: 0,
            dy: 0,
        }
    }

    #[test]
    fn the_pps_root_prefers_the_setting() {
        let _g = SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::env::set_var("HIDKBM_PPS_ROOT", "/x/pps");
        assert_eq!(pps_root(), Some(PathBuf::from("/x/pps")));
        assert_eq!(
            pps_object(Path::new("/pps"), NAME_USB),
            PathBuf::from("/pps/hidkbm/hidkbm-usb")
        );
    }

    #[test]
    fn a_wake_without_pps_says_why_and_a_wake_with_it_writes_each_new_head_once() {
        let _g = SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::env::set_var("HIDKBM_PPS_ROOT", "/nonexistent-pps-root/x");
        let wake = Wake::open(NAME_BT);
        assert!(wake.path().is_none());
        assert!(wake.error().is_some());
        let root = std::env::temp_dir().join(format!("hidkbm-wake-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::env::set_var("HIDKBM_PPS_ROOT", &root);
        let mut wake = Wake::open("/hidkbm-test-wake");
        let object = wake.path().unwrap().to_path_buf();
        wake.publish(3);
        wake.publish(3);
        wake.publish(4);
        drop(wake);
        // A plain directory stands in for PPS here: the writes append.
        assert_eq!(
            std::fs::read_to_string(&object).unwrap(),
            "head::0\nlive::1\nhead::3\nhead::4\nlive::0\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_layout_is_the_headers_and_push_writes_the_slot_before_the_head() {
        assert_eq!(std::mem::offset_of!(Shm, head), 4);
        assert_eq!(std::mem::offset_of!(Shm, kbd_present), 8);
        assert_eq!(std::mem::offset_of!(Shm, ev), 16);
        let mut shm = Shm {
            magic: AtomicU32::new(0),
            head: AtomicU32::new(7),
            kbd_present: 1,
            mouse_present: 1,
            reserved: [0; 6],
            ev: [Event::default(); RING],
        };
        shm.reset();
        assert_eq!((shm.magic.load(Ordering::Relaxed), shm.head()), (MAGIC, 0));
        for i in 0..300u16 {
            shm.push(Event {
                kind: EV_TOUCH,
                dx: i16::try_from(i).unwrap(),
                ..Event::default()
            });
        }
        assert_eq!(shm.head(), 300);
        // Slot 43 holds event 299 (299 % 256), slot 44 still event 44.
        assert_eq!((shm.ev[43].dx, shm.ev[44].dx), (299, 44));
        shm.leave();
        assert_eq!(shm.magic.load(Ordering::Relaxed), 0);
    }
}
