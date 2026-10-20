// SPDX-License-Identifier: Apache-2.0
//! The shared-memory side (feature `shm`, the default): [`Producer`] maps
//! and fills a ring, [`Reader`] maps and drains one, and [`wait_any`] sleeps
//! on the rings' PPS objects. These need libc (`shm_open` and `mmap`
//! through `qnx-shm`, `open`, `read` and `poll` through `src/sys.rs`);
//! producers that map their ring themselves use only [`crate::Wake`] and
//! build without this feature.

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{fence, Ordering};
use std::time::{Duration, Instant};

use qnx_shm::{
    close, ftruncate, mmap, munmap, shm_open, shm_unlink, OffT, MAP_SHARED, MODE_ALL_RW, O_CREAT,
    O_RDONLY, O_RDWR, PROT_READ, PROT_WRITE,
};

use crate::sys::{self, NfdsT, PollFd, EINTR, O_NONBLOCK, POLLIN};
use crate::{
    pps_object, pps_root, Event, Shm, Wake, ABSENT_INTERVAL, FALLBACK_INTERVAL, MAGIC, NO_PPS,
    RING, RING_U32, SHM_BYTES,
};

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn c_path(path: &Path) -> Result<CString, String> {
    CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| format!("{} contains a NUL byte", path.display()))
}

/// Map `name` (size [`SHM_BYTES`]); `create` makes and sizes it.
fn map(name: &str, create: bool) -> Result<*mut Shm, String> {
    let cname = CString::new(name).map_err(|_| format!("bad ring name {name:?}"))?;
    let (oflag, prot) = if create {
        (O_RDWR | O_CREAT, PROT_READ | PROT_WRITE)
    } else {
        (O_RDONLY, PROT_READ)
    };
    // SAFETY: a NUL-terminated name; every return is checked before the
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
        // MAP_FAILED is (void *)-1.
        if p.addr() == usize::MAX || p.is_null() {
            return Err(format!("mmap({name}): errno {e}"));
        }
        Ok(p.cast::<Shm>())
    }
}

/// The producer side of one ring.
pub struct Producer {
    shm: *mut Shm,
    name: String,
    pushed: u64,
    wake: Wake,
    // Made by `open_fresh`: the object is unlinked again on drop.
    fresh: bool,
}

fn unlink(name: &str) {
    if let Ok(c) = CString::new(name) {
        // SAFETY: a NUL-terminated name; a missing object is fine.
        unsafe { shm_unlink(c.as_ptr()) };
    }
}

impl Producer {
    /// As [`Self::open`], on a NEW object: an object left by an earlier
    /// producer is unlinked first, and this one is unlinked again on drop.
    /// A reader that compares the object's identity (the SDL module) then
    /// sees the old producer gone and a fresh object. Other producers can
    /// reuse an existing object instead.
    ///
    /// # Errors
    ///
    /// As [`Self::open`].
    pub fn open_fresh(name: &str) -> Result<Producer, String> {
        unlink(name);
        let mut producer = Self::open(name)?;
        producer.fresh = true;
        Ok(producer)
    }

    /// Create or reopen the ring `name` (`O_CREAT`, not `O_EXCL`, so a reader
    /// that mapped an earlier producer's object keeps a valid mapping), reset
    /// it for this producer, and open its PPS object.
    ///
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

    /// The ring's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Events pushed since open.
    #[must_use]
    pub fn pushed(&self) -> u64 {
        self.pushed
    }

    /// The PPS object that wakes readers, if it could be opened.
    #[must_use]
    pub fn wake_path(&self) -> Option<&Path> {
        self.wake.path()
    }

    /// Why readers are not woken, if they are not.
    #[must_use]
    pub fn wake_error(&self) -> Option<&str> {
        self.wake.error()
    }

    fn shm(&mut self) -> &mut Shm {
        // SAFETY: a live mapping of one Shm that only this process writes.
        unsafe { &mut *self.shm }
    }

    /// Append one event (the slot, a barrier, then the head). Readers see
    /// it at once; they are woken by the next [`Self::publish`].
    pub fn push(&mut self, ev: Event) {
        self.shm().push(ev);
        self.pushed += 1;
    }

    /// Set the live device counts (informational).
    pub fn set_present(&mut self, kbd: u8, mouse: u8) {
        self.shm().set_present(kbd, mouse);
    }

    /// Wake the readers if events were pushed since the last call: one PPS
    /// write of the head. Call it once per batch (one input report).
    pub fn publish(&mut self) {
        let head = self.shm().head();
        self.wake.publish(head);
    }
}

impl Drop for Producer {
    fn drop(&mut self) {
        // The magic is cleared first so a reader sees the producer leave;
        // dropping the wake afterwards tells the PPS readers.
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

/// What [`Reader::wait`] ended on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waited {
    /// The producer published (or events are already waiting).
    Woken,
    /// The timeout passed.
    TimedOut,
}

/// The reader side of one ring. The producer may start before or after the
/// reader, and restart while it runs.
pub struct Reader {
    name: String,
    shm: *const Shm,
    tail: Option<u32>,
    wake: Option<i32>,
    wake_error: Option<String>,
    wake_path: Option<PathBuf>,
    // The PPS object last said `live::1`: the ring's producer publishes, so
    // waiting on the object is enough. A producer that does not publish
    // (one built before the PPS wake) leaves this false and is looked at.
    published: bool,
}

impl Reader {
    /// A reader of ring `name`. The ring is mapped when it exists (now or
    /// later); its PPS object is opened (and created, so the reader can wait
    /// for a producer that has not started yet).
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
                        // SAFETY: a NUL-terminated path; the result is checked.
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
                            // The first read of a delta-mode object is its
                            // whole state: whether a publishing producer is
                            // live now.
                            reader.drain();
                        }
                    }
                }
            }
        }
        reader.attach();
        reader
    }

    /// The ring's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The PPS object this reader waits on, if it could be opened.
    #[must_use]
    pub fn wake_path(&self) -> Option<&Path> {
        self.wake_path.as_deref()
    }

    /// Why this reader cannot wait for a wake, if it cannot (it then looks
    /// every [`FALLBACK_INTERVAL`]).
    #[must_use]
    pub fn wake_error(&self) -> Option<&str> {
        self.wake_error.as_deref()
    }

    /// True while the ring is mapped and its producer is live.
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

    /// The events written since the last call. The first call after the
    /// ring appears (or after its producer restarted) only takes the
    /// position: what was written before is not this reader's input.
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
        // Behind by more than the ring holds, or the producer restarted
        // (head went back): resync and drop the backlog.
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

    /// Sleep until the producer publishes, or `timeout` passes (`None`: no
    /// bound). Returns at once when events are already waiting.
    pub fn wait(&mut self, timeout: Option<Duration>) -> Waited {
        wait_any(&mut [self], timeout)
    }

    /// True when this reader sleeps on its PPS object alone: the object is
    /// open and says a producer that publishes is live.
    #[must_use]
    pub fn woken_by_producer(&self) -> bool {
        self.wake.is_some() && self.published
    }

    /// Read what the PPS object has (the changed attributes) so the next
    /// `poll` waits for the next change, and note the producer's `live::`.
    fn drain(&mut self) {
        let Some(fd) = self.wake else {
            return;
        };
        // One read is one change, a few short lines: far below the buffer.
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

    /// Take the producer's state from PPS text (`live::1` / `live::0`
    /// lines; the object's `@name` line and other attributes are ignored).
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

/// Sleep until any of `readers` is woken by its producer, or `timeout`
/// passes (`None`: no bound). Returns at once when one already has events
/// waiting. A reader without a PPS object is looked at every
/// [`FALLBACK_INTERVAL`] meanwhile.
pub fn wait_any(readers: &mut [&mut Reader], timeout: Option<Duration>) -> Waited {
    for reader in readers.iter_mut() {
        reader.attach();
        if reader.pending() {
            return Waited::Woken;
        }
    }
    let deadline = timeout.map(|t| Instant::now() + t);
    loop {
        // A ring whose producer does not publish is looked at: every
        // FALLBACK_INTERVAL while it is live, every ABSENT_INTERVAL while
        // it is missing (to notice such a producer start).
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
            // One pollfd per reader: far below any nfds_t limit.
            let nfds = NfdsT::try_from(fds.len()).unwrap_or(NfdsT::MAX);
            // SAFETY: a valid array of `fds.len()` pollfds.
            unsafe { sys::poll(fds.as_mut_ptr(), nfds, ms) }
        };
        if n < 0 {
            let e = errno();
            if e == EINTR {
                continue;
            }
            // The PPS objects failed: fall back to looking.
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
            // Woken, though the ring shows nothing new yet (the producer
            // started or left): the caller looks again.
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

    /// Whether the object `name` exists (opened read-only, no mapping).
    fn exists(name: &str) -> bool {
        let c = CString::new(name).unwrap();
        // SAFETY: a NUL-terminated name; the descriptor is closed at once.
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
        // The reader still maps the old, now unlinked, object.
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
        // A new producer on the same object: head starts at 0 again.
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
        // Nothing comes: the wait ends at its bound.
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
        // Already waiting: at once.
        producer.push(key(5));
        let started = Instant::now();
        assert_eq!(reader.wait(Some(Duration::from_secs(5))), Waited::Woken);
        assert!(started.elapsed() < Duration::from_millis(5));
        reader.take();
        // Pushed while waiting: seen at the next look (10 ms here).
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
        // The PPS server's delta text: the object line, then the changes.
        reader.note(b"@hidkbm-usb\nhead::0\nlive::1\n");
        assert!(reader.published);
        reader.note(b"@hidkbm-usb\nhead::7\n");
        assert!(reader.published);
        reader.note(b"@hidkbm-usb\nlive::0\n");
        assert!(!reader.published);
        // Waiting on a missing ring of a producer that does not publish
        // looks every ABSENT_INTERVAL.
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
        producer.publish(); // nothing new: no write
        let text = std::fs::read_to_string(&object).unwrap();
        // A plain directory stands in for PPS here: the writes append.
        assert_eq!(text, "head::0\nlive::1\nhead::2\n");
        drop(producer);
        assert!(std::fs::read_to_string(&object)
            .unwrap()
            .ends_with("live::0\n"));
        let _ = std::fs::remove_dir_all(&root);
        unlink(&name);
    }
}
