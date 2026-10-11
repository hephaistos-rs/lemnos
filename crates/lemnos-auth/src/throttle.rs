//! Slows down guessing of passwords and codes.
//!
//! The first few failures for a key (a username, say) are free. After that
//! every failure doubles how long the key is blocked, up to [`MAX_BLOCK`].
//! Someone can use this to lock a user out for that long by failing on
//! purpose; that is the price of per-account throttling, and passkey and SSO
//! sign-ins are not affected.
//!
//! Counts live in memory, so they reset on restart and are per instance.

use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

const FREE_FAILURES: u32 = 5;
// The maximum block time is 15 minutes, which is long enough to slow down guessing
// but short enough that a user can try again after a break.
const MAX_BLOCK: Duration = Duration::from_secs(60 * 15);
/// Above this many keys, stale ones are dropped, so guessing random
/// usernames can't grow the map without bound.
const PRUNE_ABOVE: usize = 10_000;

#[derive(Default)]
pub(crate) struct Throttle {
    entries: Mutex<HashMap<String, Entry>>,
}

struct Entry {
    failures: u32,
    blocked_until: Instant,
}

impl Throttle {
    /// `Err` with how long to wait if `key` is currently blocked.
    pub(crate) fn check(&self, key: &str) -> Result<(), Duration> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        match entries.get(key) {
            Some(entry) => match entry.blocked_until.checked_duration_since(Instant::now()) {
                Some(wait) if !wait.is_zero() => Err(wait),
                _ => Ok(()),
            },
            None => Ok(()),
        }
    }

    pub(crate) fn failure(&self, key: &str) {
        let now = Instant::now();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if entries.len() >= PRUNE_ABOVE {
            entries.retain(|_, entry| entry.blocked_until + MAX_BLOCK > now);
        }
        let entry = entries.entry(key.to_owned()).or_insert(Entry {
            failures: 0,
            blocked_until: now,
        });
        entry.failures += 1;
        entry.blocked_until = now + block_for(entry.failures);
    }

    pub(crate) fn success(&self, key: &str) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }
}

fn block_for(failures: u32) -> Duration {
    match failures.checked_sub(FREE_FAILURES) {
        None | Some(0) => Duration::ZERO,
        // 1s, 2s, 4s, ... The `min` keeps the shift from overflowing.
        Some(extra) => Duration::from_secs(1 << (extra - 1).min(20)).min(MAX_BLOCK),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_grow_and_are_capped() {
        assert_eq!(block_for(1), Duration::ZERO);
        assert_eq!(block_for(5), Duration::ZERO);
        assert_eq!(block_for(6), Duration::from_secs(1));
        assert_eq!(block_for(8), Duration::from_secs(4));
        assert_eq!(block_for(1000), MAX_BLOCK);
    }

    #[test]
    fn blocks_after_free_failures_and_resets_on_success() {
        let throttle = Throttle::default();
        for _ in 0..FREE_FAILURES {
            throttle.failure("alice");
        }
        assert!(throttle.check("alice").is_ok());
        throttle.failure("alice");
        assert!(throttle.check("alice").is_err());
        assert!(throttle.check("bob").is_ok());
        throttle.success("alice");
        assert!(throttle.check("alice").is_ok());
    }
}
