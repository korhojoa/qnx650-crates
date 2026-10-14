//! Copyright 2026. Licensed under the Apache License, Version 2.0.
use crate::bulk::STOP_SLICE_MS;
use crate::stop;
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};
static COUNT: Mutex<u64> = Mutex::new(0);
static MOVED: Condvar = Condvar::new();
pub fn signal() {
    let mut count = COUNT.lock().unwrap_or_else(PoisonError::into_inner);
    *count = count.wrapping_add(1);
    MOVED.notify_all();
}
#[must_use]
pub fn seen() -> u64 {
    *COUNT.lock().unwrap_or_else(PoisonError::into_inner)
}
#[must_use]
pub fn wait(seen: u64, timeout: Option<Duration>) -> u64 {
    let deadline = timeout.map(|t| Instant::now() + t);
    let mut count = COUNT.lock().unwrap_or_else(PoisonError::into_inner);
    while *count == seen && !stop::requested() {
        let slice = Duration::from_millis(STOP_SLICE_MS);
        let step = match deadline {
            Some(d) => {
                let left = d.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                left.min(slice)
            }
            None => slice,
        };
        count = MOVED
            .wait_timeout(count, step)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
    *count
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_signal_from_another_thread_ends_the_wait_and_a_bound_ends_it_too() {
        let started = Instant::now();
        let _ = wait(seen(), Some(Duration::from_millis(30)));
        assert!(started.elapsed() < Duration::from_millis(1000));
        let before = seen();
        let signaller = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(40));
            signal();
        });
        let started = Instant::now();
        let after = wait(before, None);
        assert!(started.elapsed() < Duration::from_millis(1000));
        signaller.join().unwrap();
        if stop::requested() {
            return;
        }
        assert_ne!(after, before);
        let started = Instant::now();
        assert_ne!(wait(before, Some(Duration::from_secs(5))), before);
        assert!(started.elapsed() < Duration::from_millis(5));
    }
}
