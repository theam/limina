//! A small, bounded failure limiter for single-node authentication surfaces.
//!
//! Each key (a client address, or `all-transports` for the global ceiling) keeps the times of its
//! recent failures. Once `limit` failures fall inside the sliding window, [`check`] refuses with
//! `rate_limited` and `retry_after_seconds`: the whole seconds until the oldest failure leaves the
//! window, rounded half-to-even and at least 1, exactly as the Python runtime computed it. At most
//! `max_keys` keys are tracked; the least recently failed key is forgotten first.
//!
//! [`check`]: FailureRateLimiter::check

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use limina_core::{LiminaError, Result};

use super::config::AuthConfigError;

/// Seconds on a monotonic clock. Injectable so tests can move time.
pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

/// The process monotonic clock.
pub fn monotonic_clock() -> Clock {
    let start = Instant::now();
    Arc::new(move || start.elapsed().as_secs_f64())
}

/// How many failures a key may accumulate inside a sliding window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitSettings {
    pub limit: i64,
    pub window_seconds: i64,
    pub max_keys: i64,
}

impl RateLimitSettings {
    pub const fn new(limit: i64, window_seconds: i64) -> Self {
        Self { limit, window_seconds, max_keys: 10_000 }
    }
}

pub struct FailureRateLimiter {
    limit: usize,
    window_seconds: f64,
    max_keys: usize,
    clock: Clock,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    items: HashMap<String, Failures>,
    /// Keys by last failure, oldest first, for eviction.
    recency: BTreeMap<u64, String>,
    next_stamp: u64,
}

struct Failures {
    timestamps: VecDeque<f64>,
    stamp: u64,
}

impl FailureRateLimiter {
    pub fn new(settings: RateLimitSettings) -> std::result::Result<Self, AuthConfigError> {
        Self::with_clock(settings, monotonic_clock())
    }

    pub fn with_clock(
        settings: RateLimitSettings,
        clock: Clock,
    ) -> std::result::Result<Self, AuthConfigError> {
        let positive = |value: i64| usize::try_from(value).ok().filter(|value| *value >= 1);
        let (Some(limit), Some(window), Some(max_keys)) = (
            positive(settings.limit),
            positive(settings.window_seconds),
            positive(settings.max_keys),
        ) else {
            return Err(AuthConfigError::new("Rate-limit settings must be positive."));
        };
        Ok(Self {
            limit,
            window_seconds: window as f64,
            max_keys,
            clock,
            state: Mutex::new(State::default()),
        })
    }

    /// Refuses with `rate_limited` once the key has `limit` failures inside the window.
    pub fn check(&self, key: &str) -> Result<()> {
        let mut state = self.lock();
        let Some(item) = state.items.get_mut(key) else {
            return Ok(());
        };
        let now = (self.clock)();
        prune(&mut item.timestamps, now - self.window_seconds);
        if item.timestamps.len() < self.limit {
            return Ok(());
        }
        let oldest = item.timestamps.front().copied().unwrap_or(now);
        // Python's round() is half-to-even; the result lies in 1..=window.
        let remaining = (oldest + self.window_seconds - now).round_ties_even().max(1.0);
        Err(LiminaError::rate_limited(remaining as u64))
    }

    /// Records one failed attempt for the key.
    pub fn failure(&self, key: &str) {
        let now = (self.clock)();
        let mut guard = self.lock();
        let state = &mut *guard;
        let stamp = state.next_stamp;
        state.next_stamp += 1;
        let item = state
            .items
            .entry(key.to_owned())
            .or_insert_with(|| Failures { timestamps: VecDeque::new(), stamp });
        prune(&mut item.timestamps, now - self.window_seconds);
        item.timestamps.push_back(now);
        state.recency.remove(&item.stamp);
        item.stamp = stamp;
        state.recency.insert(stamp, key.to_owned());
        while state.items.len() > self.max_keys {
            let Some((_, oldest)) = state.recency.pop_first() else { break };
            state.items.remove(&oldest);
        }
    }

    /// Forgets the key's failures (a successful sign-in).
    pub fn success(&self, key: &str) {
        let mut state = self.lock();
        if let Some(item) = state.items.remove(key) {
            state.recency.remove(&item.stamp);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn prune(timestamps: &mut VecDeque<f64>, cutoff: f64) {
    while timestamps.front().is_some_and(|first| *first <= cutoff) {
        timestamps.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn manual_clock() -> (Arc<AtomicU64>, Clock) {
        let now = Arc::new(AtomicU64::new(0));
        let reader = now.clone();
        let clock: Clock = Arc::new(move || reader.load(Ordering::SeqCst) as f64);
        (now, clock)
    }

    #[test]
    fn retry_after_counts_down_from_the_oldest_failure() {
        let (now, clock) = manual_clock();
        let limiter = FailureRateLimiter::with_clock(RateLimitSettings::new(2, 10), clock).unwrap();
        limiter.failure("client");
        now.store(4, Ordering::SeqCst);
        limiter.failure("client");
        let error = limiter.check("client").unwrap_err();
        assert_eq!(error.code(), "rate_limited");
        assert_eq!(error.details["retry_after_seconds"], 6);
        limiter.success("client");
        limiter.check("client").unwrap();
    }

    #[test]
    fn the_least_recently_failed_key_is_evicted_first() {
        let (_now, clock) = manual_clock();
        let settings = RateLimitSettings { limit: 1, window_seconds: 60, max_keys: 2 };
        let limiter = FailureRateLimiter::with_clock(settings, clock).unwrap();
        limiter.failure("a");
        limiter.failure("b");
        limiter.failure("a");
        limiter.failure("c");
        assert!(limiter.check("a").is_err());
        assert!(limiter.check("b").is_ok());
        assert!(limiter.check("c").is_err());
    }

    #[test]
    fn settings_must_be_positive() {
        for settings in [
            RateLimitSettings::new(0, 60),
            RateLimitSettings::new(10, 0),
            RateLimitSettings { limit: 10, window_seconds: 60, max_keys: 0 },
            RateLimitSettings::new(-1, 60),
        ] {
            assert!(FailureRateLimiter::new(settings).is_err());
        }
    }
}
