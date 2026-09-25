//! Time source (PHASE-0A.md §65).
//!
//! Domain types must not read the wall clock directly. The runtime
//! takes a [`Clock`] in its constructor; the production wiring
//! uses [`SystemClock`], tests can use a [`FixedClock`].

use std::sync::Arc;
use std::sync::Mutex;

use time::OffsetDateTime;

/// Wall-clock abstraction.
///
/// Implementations must be `Send + Sync` so the daemon can share a
/// single instance across tasks.
pub trait Clock: Send + Sync {
    /// Current UTC time.
    fn now_utc(&self) -> OffsetDateTime;
}

/// Reads the OS wall clock via [`OffsetDateTime::now_utc`].
///
/// Used in production; tests prefer [`FixedClock`].
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_utc(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

/// Test-only clock that returns a pre-set instant and never moves.
///
/// Shared between threads via [`Arc<Mutex<_>>`]; every call to
/// [`Clock::now_utc`] returns whatever the test most recently
/// installed via [`FixedClock::set`]. Lock poisoning is recovered
/// via [`Mutex::into_inner`] on the underlying guard so a panic
/// in one thread doesn't permanently break the clock.
#[derive(Debug, Clone)]
pub struct FixedClock {
    inner: Arc<Mutex<OffsetDateTime>>,
}

impl FixedClock {
    /// Build a clock pinned to `start`.
    #[must_use]
    pub fn new(start: OffsetDateTime) -> Self {
        Self {
            inner: Arc::new(Mutex::new(start)),
        }
    }

    /// Move the clock forward (or backward) to `now`.
    pub fn set(&self, now: OffsetDateTime) {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = now;
    }
}

impl Clock for FixedClock {
    fn now_utc(&self) -> OffsetDateTime {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard
    }
}

impl Default for FixedClock {
    fn default() -> Self {
        // Unix epoch as a deterministic, reproducible default. Tests
        // almost always call `set` before exercising the runtime.
        Self::new(OffsetDateTime::UNIX_EPOCH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_returns_recent_time() {
        let before = OffsetDateTime::now_utc();
        let clock = SystemClock;
        let sampled = clock.now_utc();
        let after = OffsetDateTime::now_utc();
        assert!(
            sampled >= before && sampled <= after,
            "system clock outside [before, after]: {sampled}"
        );
    }

    #[test]
    fn fixed_clock_returns_set_value() {
        let clock = FixedClock::new(OffsetDateTime::UNIX_EPOCH);
        assert_eq!(clock.now_utc(), OffsetDateTime::UNIX_EPOCH);
        let t = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        clock.set(t);
        assert_eq!(clock.now_utc(), t);
    }

    #[test]
    fn fixed_clock_shares_state_across_clones() {
        let a = FixedClock::new(OffsetDateTime::UNIX_EPOCH);
        let b = a.clone();
        let t = OffsetDateTime::from_unix_timestamp(42).unwrap();
        a.set(t);
        assert_eq!(b.now_utc(), t);
    }
}
