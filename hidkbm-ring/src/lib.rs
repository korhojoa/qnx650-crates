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
pub const MAGIC: u32 = 0x4b42_4d31;
pub const RING: usize = 256;
pub const NAME_USB: &str = "/hidkbm-usb";
pub const NAME_BT: &str = "/hidkbm-bt";
pub const NAME_GIP: &str = "/hidkbm-gip";
pub const NAME_TOUCH: &str = "/hidkbm-touch";
pub const NAME_TOUCH_USB0: &str = "/hidkbm-touch-usb0";
pub const EV_KEY: u8 = 1;
pub const EV_MBUTTON: u8 = 2;
pub const EV_MMOTION: u8 = 3;
pub const EV_MWHEEL: u8 = 4;
pub const EV_TOUCH: u8 = 5;
pub const TOUCH_UP: u8 = 0;
pub const TOUCH_DOWN: u8 = 1;
pub const TOUCH_MOVE: u8 = 2;
const NO_PPS: &str = "no PPS server (no /ramdisk/pps or /pps)";
const RING_U32: u32 = 256;
const _: () = assert!(RING_U32 as usize == RING);
pub const FALLBACK_INTERVAL: Duration = Duration::from_millis(10);
pub const ABSENT_INTERVAL: Duration = Duration::from_millis(250);
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
#[repr(C)]
pub struct Shm {
    pub(crate) magic: AtomicU32,
    pub(crate) head: AtomicU32,
    pub(crate) kbd_present: u8,
    pub(crate) mouse_present: u8,
    pub(crate) reserved: [u8; 6],
    pub(crate) ev: [Event; RING],
}
pub const SHM_BYTES: usize = std::mem::size_of::<Shm>();
const _: () = assert!(SHM_BYTES == 2064);
impl Shm {
    pub fn reset(&mut self) {
        self.head.store(0, Ordering::Relaxed);
        self.kbd_present = 0;
        self.mouse_present = 0;
        self.reserved = [0; 6];
        self.ev = [Event::default(); RING];
        fence(Ordering::SeqCst);
        self.magic.store(MAGIC, Ordering::SeqCst);
    }
    pub fn push(&mut self, ev: Event) {
        let head = self.head.load(Ordering::Relaxed);
        self.ev[usize::try_from(head).unwrap_or(0) % RING] = ev;
        fence(Ordering::SeqCst);
        self.head.store(head.wrapping_add(1), Ordering::SeqCst);
    }
    #[must_use]
    pub fn head(&self) -> u32 {
        self.head.load(Ordering::SeqCst)
    }
    pub fn set_present(&mut self, kbd: u8, mouse: u8) {
        self.kbd_present = kbd;
        self.mouse_present = mouse;
    }
    #[must_use]
    pub fn present(&self) -> (u8, u8) {
        (self.kbd_present, self.mouse_present)
    }
    #[must_use]
    pub fn live(&self) -> bool {
        self.magic.load(Ordering::Acquire) == MAGIC
    }
    #[must_use]
    pub fn event(&self, index: u32) -> Event {
        self.ev[usize::try_from(index).unwrap_or(0) % RING]
    }
    pub fn leave(&mut self) {
        self.magic.store(0, Ordering::SeqCst);
    }
}
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
#[must_use]
pub fn pps_object(root: &Path, name: &str) -> PathBuf {
    root.join("hidkbm").join(name.trim_start_matches('/'))
}
pub struct Wake {
    object: Option<(PathBuf, File)>,
    error: Option<String>,
    published: Option<u32>,
}
impl Wake {
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
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.object.as_ref().map(|(p, _)| p.as_path())
    }
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
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
        if let Err(e) = file.write_all(text.as_bytes()) {
            self.error = Some(format!("write({}): {e}", object.display()));
            self.object = None;
        }
    }
}
impl Drop for Wake {
    fn drop(&mut self) {
        self.write("live::0\n");
    }
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;
    pub(crate) static SERIAL: Mutex<()> = Mutex::new(());
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
        assert_eq!((shm.ev[43].dx, shm.ev[44].dx), (299, 44));
        shm.leave();
        assert_eq!(shm.magic.load(Ordering::Relaxed), 0);
    }
}
