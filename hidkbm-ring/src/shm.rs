use crate::sys::{self, NfdsT, PollFd, EINTR, O_NONBLOCK, POLLIN};
use crate::{
    pps_object, pps_root, Event, Shm, Wake, ABSENT_INTERVAL, FALLBACK_INTERVAL, MAGIC, NO_PPS,
    RING, RING_U32, SHM_BYTES,
};
use qnx_shm::{
    close, ftruncate, mmap, munmap, shm_open, shm_unlink, OffT, MAP_SHARED, MODE_ALL_RW, O_CREAT,
    O_RDONLY, O_RDWR, PROT_READ, PROT_WRITE,
};
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{fence, Ordering};
use std::time::{Duration, Instant};
fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}
fn c_path(path: &Path) -> Result<CString, String> {
    CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| format!("{} contains a NUL byte", path.display()))
}
fn map(name: &str, create: bool) -> Result<*mut Shm, String> {
    let cname = CString::new(name).map_err(|_| format!("bad ring name {name:?}"))?;
    let (oflag, prot) = if create {
        (O_RDWR | O_CREAT, PROT_READ | PROT_WRITE)
    } else {
        (O_RDONLY, PROT_READ)
    };
    // SAFETY: a NUL-terminated name. every return is checked before the
    // mapping is used.
    unsafe {
        let fd = shm_open(cname.as_ptr(), oflag, MODE_ALL_RW);
        if fd < 0 {
            return Err(format!("shm_open({name}): errno {}", errno()));
        }
        if create {
            let len = OffT::try_from(SHM_BYTES).unwrap_or(0);
            if ftruncate(fd, len) != 0 {
                let e = errno();
                close(fd);
                return Err(format!("ftruncate({name}, {SHM_BYTES}): errno {e}"));
            }
        }
        let p = mmap(std::ptr::null_mut(), SHM_BYTES, prot, MAP_SHARED, fd, 0);
        let e = errno();
        close(fd);
        if p.addr() == usize::MAX || p.is_null() {
            return Err(format!("mmap({name}): errno {e}"));
        }
        Ok(p.cast::<Shm>())
    }
}
pub struct Producer {
    shm: *mut Shm,
    name: String,
    pushed: u64,
    wake: Wake,
    fresh: bool,
}
fn unlink(name: &str) {
    if let Ok(c) = CString::new(name) {
        // SAFETY: a NUL-terminated name. A missing object is fine.
        unsafe { shm_unlink(c.as_ptr()) };
    }
}
impl Producer {
    /// # Errors
    ///
    /// As [`Self::open`].
    pub fn open_fresh(name: &str) -> Result<Producer, String> {
        unlink(name);
        let mut producer = Self::open(name)?;
        producer.fresh = true;
        Ok(producer)
    }
    /// # Errors
    ///
    /// The failing shared-memory call and its errno, as text. A missing PPS
    /// server is not an error: see [`Self::wake_error`].
    pub fn open(name: &str) -> Result<Producer, String> {
        let shm = map(name, true)?;
        // SAFETY: a fresh mapping of one Shm that only this process writes.
        unsafe { (*shm).reset() };
        Ok(Producer {
            shm,
            name: name.to_owned(),
            pushed: 0,
            wake: Wake::open(name),
            fresh: false,
        })
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn pushed(&self) -> u64 {
        self.pushed
    }
    #[must_use]
    pub fn wake_path(&self) -> Option<&Path> {
        self.wake.path()
    }
    #[must_use]
    pub fn wake_error(&self) -> Option<&str> {
        self.wake.error()
    }
    fn shm(&mut self) -> &mut Shm {
        // SAFETY: a live mapping of one Shm that only this process writes.
        unsafe { &mut *self.shm }
    }
    pub fn push(&mut self, ev: Event) {
        self.shm().push(ev);
        self.pushed += 1;
    }
    pub fn set_present(&mut self, kbd: u8, mouse: u8) {
        self.shm().set_present(kbd, mouse);
    }
    pub fn publish(&mut self) {
        let head = self.shm().head();
        self.wake.publish(head);
    }
}
impl Drop for Producer {
    fn drop(&mut self) {
        self.shm().leave();
        // SAFETY: the mapping made in `open`, unmapped once.
        unsafe { munmap(self.shm.cast(), SHM_BYTES) };
        if self.fresh {
            unlink(&self.name);
        }
    }
}
// SAFETY: the mapping is owned by the value and used from one thread at a
// time (`&mut self`).
unsafe impl Send for Producer {}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waited {
    Woken,
    TimedOut,
}
pub struct Reader {
    name: String,
    shm: *const Shm,
    tail: Option<u32>,
    wake: Option<i32>,
    wake_error: Option<String>,
    wake_path: Option<PathBuf>,
    published: bool,
}
impl Reader {
    #[must_use]
    pub fn new(name: &str) -> Reader {
        let mut reader = Reader {
            name: name.to_owned(),
            shm: std::ptr::null(),
            tail: None,
            wake: None,
            wake_error: None,
            wake_path: None,
            published: false,
        };
        match pps_root() {
            None => reader.wake_error = Some(String::from(NO_PPS)),
            Some(root) => {
                let object = pps_object(&root, name);
                if let Some(dir) = object.parent() {
                    let _ = std::fs::create_dir(dir);
                }
                let with_options = PathBuf::from(format!("{}?delta", object.display()));
                match c_path(&with_options) {
                    Err(e) => reader.wake_error = Some(e),
                    Ok(path) => {
                        // SAFETY: a NUL-terminated path. The result is checked.
                        let fd = unsafe {
                            sys::open(path.as_ptr(), O_RDONLY | O_CREAT | O_NONBLOCK, MODE_ALL_RW)
                        };
                        if fd < 0 {
                            reader.wake_error = Some(format!(
                                "open({}): errno {}",
                                with_options.display(),
                                errno()
                            ));
                        } else {
                            reader.wake = Some(fd);
                            reader.wake_path = Some(object);
                            reader.drain();
                        }
                    }
                }
            }
        }
        reader.attach();
        reader
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn wake_path(&self) -> Option<&Path> {
        self.wake_path.as_deref()
    }
    #[must_use]
    pub fn wake_error(&self) -> Option<&str> {
        self.wake_error.as_deref()
    }
    #[must_use]
    pub fn live(&self) -> bool {
        self.ring()
            .is_some_and(|s| s.magic.load(Ordering::Acquire) == MAGIC)
    }
    fn ring(&self) -> Option<&Shm> {
        // SAFETY: null or a live read-only mapping of one Shm.
        unsafe { self.shm.as_ref() }
    }
    fn attach(&mut self) {
        if self.shm.is_null() {
            if let Ok(shm) = map(&self.name, false) {
                self.shm = shm;
                self.tail = None;
            }
        }
    }
    pub fn take(&mut self) -> Vec<Event> {
        self.attach();
        let Some(shm) = self.ring() else {
            return Vec::new();
        };
        if shm.magic.load(Ordering::Acquire) != MAGIC {
            self.tail = None;
            return Vec::new();
        }
        let head = shm.head.load(Ordering::Acquire);
        let Some(mut from) = self.tail else {
            self.tail = Some(head);
            return Vec::new();
        };
        if head.wrapping_sub(from) > RING_U32 {
            from = head;
        }
        let mut out = Vec::new();
        while from != head {
            out.push(shm.ev[usize::try_from(from).unwrap_or(0) % RING]);
            from = from.wrapping_add(1);
        }
        fence(Ordering::Acquire);
        self.tail = Some(head);
        out
    }
    fn pending(&self) -> bool {
        match (self.ring(), self.tail) {
            (Some(shm), Some(tail)) => shm.head.load(Ordering::Acquire) != tail,
            _ => false,
        }
    }
    pub fn wait(&mut self, timeout: Option<Duration>) -> Waited {
        wait_any(&mut [self], timeout)
    }
    #[must_use]
    pub fn woken_by_producer(&self) -> bool {
        self.wake.is_some() && self.published
    }
    fn drain(&mut self) {
        let Some(fd) = self.wake else {
            return;
        };
        let mut buffer = [0u8; 1024];
        for _ in 0..16 {
            // SAFETY: an open descriptor and a writable buffer.
            let n = unsafe { sys::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
            let Ok(n) = usize::try_from(n) else {
                return;
            };
            self.note(&buffer[..n]);
            if n < buffer.len() {
                return;
            }
        }
    }
    fn note(&mut self, text: &[u8]) {
        for line in text.split(|&b| b == b'\n') {
            match line {
                b"live::1" => self.published = true,
                b"live::0" => self.published = false,
                _ => {}
            }
        }
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        if let Some(fd) = self.wake.take() {
            // SAFETY: our descriptor, closed once.
            unsafe { close(fd) };
        }
        if !self.shm.is_null() {
            // SAFETY: the mapping made in `attach`, unmapped once.
            unsafe { munmap(self.shm.cast_mut().cast(), SHM_BYTES) };
        }
    }
}
// SAFETY: the mapping is owned by the value and used from one thread at a
// time (`&mut self`).
unsafe impl Send for Reader {}
pub fn wait_any(readers: &mut [&mut Reader], timeout: Option<Duration>) -> Waited {
    for reader in readers.iter_mut() {
        reader.attach();
        if reader.pending() {
            return Waited::Woken;
        }
    }
    let deadline = timeout.map(|t| Instant::now() + t);
    loop {
        let look = readers
            .iter()
            .filter(|r| !r.woken_by_producer())
            .map(|r| {
                if r.live() {
                    FALLBACK_INTERVAL
                } else {
                    ABSENT_INTERVAL
                }
            })
            .min();
        let mut fds: Vec<PollFd> = readers
            .iter()
            .filter_map(|r| r.wake)
            .map(|fd| PollFd {
                fd,
                events: POLLIN,
                revents: 0,
            })
            .collect();
        let left = deadline.map(|d| d.saturating_duration_since(Instant::now()));
        let bound = match (left, look) {
            (Some(left), Some(look)) => Some(left.min(look)),
            (None, look) => look,
            (left, None) => left,
        };
        let ms = bound.map_or(-1, |b| i32::try_from(b.as_millis()).unwrap_or(i32::MAX));
        let n = if fds.is_empty() {
            std::thread::sleep(bound.unwrap_or(FALLBACK_INTERVAL));
            0
        } else {
            let nfds = NfdsT::try_from(fds.len()).unwrap_or(NfdsT::MAX);
            // SAFETY: a valid array of `fds.len()` pollfds.
            unsafe { sys::poll(fds.as_mut_ptr(), nfds, ms) }
        };
        if n < 0 {
            let e = errno();
            if e == EINTR {
                continue;
            }
            for reader in readers.iter_mut() {
                if let Some(fd) = reader.wake.take() {
                    reader.wake_error = Some(format!("poll on the PPS object: errno {e}"));
                    // SAFETY: the reader's descriptor, not used again.
                    unsafe { close(fd) };
                }
            }
            return Waited::TimedOut;
        }
        for reader in readers.iter_mut() {
            if fds
                .iter()
                .any(|p| p.revents != 0 && Some(p.fd) == reader.wake)
            {
                reader.drain();
            }
        }
        for reader in readers.iter_mut() {
            reader.attach();
            if reader.pending() {
                return Waited::Woken;
            }
        }
        if n > 0 {
            return Waited::Woken;
        }
        if left.is_some_and(|l| l <= bound.unwrap_or(Duration::ZERO)) {
            return Waited::TimedOut;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{key, SERIAL};
    fn unique(name: &str) -> String {
        format!("/hidkbm-test-{}-{name}", std::process::id())
    }
    fn exists(name: &str) -> bool {
        let c = CString::new(name).unwrap();
        // SAFETY: a NUL-terminated name. The descriptor is closed at once.
        unsafe {
            let fd = shm_open(c.as_ptr(), O_RDONLY, 0);
            if fd >= 0 {
                close(fd);
            }
            fd >= 0
        }
    }
    #[test]
    fn a_fresh_producer_replaces_the_object_and_removes_it_when_done() {
        let _g = serial();
        std::env::set_var("HIDKBM_PPS_ROOT", "/nonexistent-pps-root");
        let name = unique("fresh");
        let mut old = Producer::open(&name).unwrap();
        old.push(key(1));
        let mut reader = Reader::new(&name);
        reader.take();
        let mut fresh = Producer::open_fresh(&name).unwrap();
        fresh.push(key(2));
        assert_eq!(reader.take(), Vec::new());
        let mut late = Reader::new(&name);
        late.take();
        fresh.push(key(3));
        assert_eq!(late.take(), vec![key(3)]);
        drop(old);
        drop(fresh);
        assert!(!exists(&name), "a fresh producer unlinks its object");
    }
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    #[test]
    fn a_reader_sees_only_what_was_pushed_after_it_started() {
        let _g = serial();
        std::env::set_var("HIDKBM_PPS_ROOT", "/nonexistent-pps-root");
        let name = unique("order");
        let mut producer = Producer::open(&name).unwrap();
        producer.push(key(1));
        let mut reader = Reader::new(&name);
        assert!(reader.live());
        assert!(reader.take().is_empty(), "the backlog is not input");
        producer.push(key(2));
        producer.push(key(3));
        assert_eq!(reader.take(), vec![key(2), key(3)]);
        assert_eq!(reader.take(), Vec::new());
        drop(producer);
        assert!(!reader.live());
        unlink(&name);
    }
    #[test]
    fn a_reader_that_falls_behind_or_sees_a_restart_resyncs() {
        let _g = serial();
        std::env::set_var("HIDKBM_PPS_ROOT", "/nonexistent-pps-root");
        let name = unique("resync");
        let mut producer = Producer::open(&name).unwrap();
        let mut reader = Reader::new(&name);
        reader.take();
        for i in 0..300u32 {
            producer.push(key(u8::try_from(i % 256).unwrap()));
        }
        assert!(
            reader.take().is_empty(),
            "more than a ring behind: backlog dropped"
        );
        producer.push(key(7));
        assert_eq!(reader.take(), vec![key(7)]);
        drop(producer);
        let mut again = Producer::open(&name).unwrap();
        assert_eq!(reader.take(), Vec::new());
        again.push(key(9));
        reader.take();
        again.push(key(10));
        assert_eq!(reader.take(), vec![key(10)]);
        unlink(&name);
    }
    #[test]
    fn a_reader_started_before_its_producer_attaches_later() {
        let _g = serial();
        std::env::set_var("HIDKBM_PPS_ROOT", "/nonexistent-pps-root");
        let name = unique("late");
        unlink(&name);
        let mut reader = Reader::new(&name);
        assert!(!reader.live());
        assert_eq!(reader.take(), Vec::new());
        let mut producer = Producer::open(&name).unwrap();
        assert_eq!(reader.take(), Vec::new());
        producer.push(key(4));
        assert_eq!(reader.take(), vec![key(4)]);
        unlink(&name);
    }
    #[test]
    fn without_pps_wait_is_bounded_and_returns_at_once_when_events_wait() {
        let _g = serial();
        std::env::set_var("HIDKBM_PPS_ROOT", "/nonexistent-pps-root");
        let name = unique("nowake");
        let mut producer = Producer::open(&name).unwrap();
        assert!(producer.wake_error().is_some());
        let mut reader = Reader::new(&name);
        assert!(reader.wake_error().is_some());
        reader.take();
        let started = Instant::now();
        assert_eq!(
            reader.wait(Some(Duration::from_millis(100))),
            Waited::TimedOut
        );
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(100) && took < Duration::from_millis(300),
            "{took:?}"
        );
        producer.push(key(5));
        let started = Instant::now();
        assert_eq!(reader.wait(Some(Duration::from_secs(5))), Waited::Woken);
        assert!(started.elapsed() < Duration::from_millis(5));
        reader.take();
        let pusher = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            producer.push(key(6));
            producer
        });
        let started = Instant::now();
        assert_eq!(reader.wait(Some(Duration::from_secs(5))), Waited::Woken);
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(50) && took < Duration::from_millis(150),
            "{took:?}"
        );
        let producer = pusher.join().unwrap();
        assert_eq!(reader.take(), vec![key(6)]);
        drop(producer);
        unlink(&name);
    }
    #[test]
    fn only_a_producer_that_says_live_is_trusted_to_wake() {
        let _g = serial();
        std::env::set_var("HIDKBM_PPS_ROOT", "/nonexistent-pps-root");
        let mut reader = Reader::new(&unique("trust"));
        assert!(!reader.woken_by_producer(), "no PPS object");
        reader.note(b"@hidkbm-usb\nhead::0\nlive::1\n");
        assert!(reader.published);
        reader.note(b"@hidkbm-usb\nhead::7\n");
        assert!(reader.published);
        reader.note(b"@hidkbm-usb\nlive::0\n");
        assert!(!reader.published);
        let started = Instant::now();
        assert_eq!(
            reader.wait(Some(Duration::from_millis(600))),
            Waited::TimedOut
        );
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(600) && took < Duration::from_millis(900),
            "{took:?}"
        );
    }
    #[test]
    fn the_pps_object_gets_the_head_after_each_publish() {
        let _g = serial();
        let root = std::env::temp_dir().join(format!("hidkbm-pps-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::env::set_var("HIDKBM_PPS_ROOT", &root);
        let name = unique("publish");
        let mut producer = Producer::open(&name).unwrap();
        let object = producer.wake_path().unwrap().to_path_buf();
        assert_eq!(
            object,
            root.join("hidkbm").join(name.trim_start_matches('/'))
        );
        producer.push(key(1));
        producer.push(key(2));
        producer.publish();
        producer.publish();
        let text = std::fs::read_to_string(&object).unwrap();
        assert_eq!(text, "head::0\nlive::1\nhead::2\n");
        drop(producer);
        assert!(std::fs::read_to_string(&object)
            .unwrap()
            .ends_with("live::0\n"));
        let _ = std::fs::remove_dir_all(&root);
        unlink(&name);
    }
}
