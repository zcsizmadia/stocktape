//! Request pacing for rate-limited APIs.

use std::time::Duration;

/// A token bucket: up to `per_minute` requests in any minute, spread evenly.
///
/// Time is passed in (monotonic), so the limiter is deterministic in tests.
///
/// ```
/// use std::time::Duration;
/// use stocktape_sources::limiter::RateLimiter;
///
/// let mut limiter = RateLimiter::per_minute(5);
/// let t0 = Duration::ZERO;
/// // A full bucket allows a short burst...
/// for _ in 0..5 {
///     assert_eq!(limiter.wait(t0), Duration::ZERO);
///     limiter.take(t0);
/// }
/// // ...then requests come every 12 s.
/// assert_eq!(limiter.wait(t0), Duration::from_secs(12));
/// ```
#[derive(Debug, Clone)]
pub struct RateLimiter {
    capacity: f64,
    tokens: f64,
    /// Seconds per token.
    interval: f64,
    last: Duration,
}

impl RateLimiter {
    /// At most `n` requests per minute (at least 1).
    #[must_use]
    pub fn per_minute(n: u32) -> Self {
        let n = f64::from(n.max(1));
        Self {
            capacity: n,
            tokens: n,
            interval: 60.0 / n,
            last: Duration::ZERO,
        }
    }

    fn refill(&mut self, now: Duration) {
        let elapsed = now.saturating_sub(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed / self.interval).min(self.capacity);
        self.last = now.max(self.last);
    }

    /// How long to wait at `now` before the next request may go.
    pub fn wait(&mut self, now: Duration) -> Duration {
        self.refill(now);
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64((1.0 - self.tokens) * self.interval)
        }
    }

    /// Records a request made at `now` (call after waiting).
    pub fn take(&mut self, now: Duration) {
        self.refill(now);
        self.tokens = (self.tokens - 1.0).max(-self.capacity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn refills_over_time() {
        let mut l = RateLimiter::per_minute(60);
        for _ in 0..60 {
            l.take(Duration::ZERO);
        }
        assert_eq!(l.wait(Duration::ZERO), Duration::from_secs(1));
        assert_eq!(l.wait(Duration::from_secs(1)), Duration::ZERO);
        assert_eq!(l.wait(Duration::from_secs(600)), Duration::ZERO);
    }

    proptest! {
        /// Whatever the timing, following `wait` never allows more than the
        /// limit (plus the initial burst) within any minute.
        #[test]
        fn never_exceeds_the_limit(n in 1u32..30, gaps in proptest::collection::vec(0u64..20_000, 1..200)) {
            let mut l = RateLimiter::per_minute(n);
            let mut now = Duration::ZERO;
            let mut times = Vec::new();
            for gap in gaps {
                now += Duration::from_millis(gap);
                now += l.wait(now);
                l.take(now);
                times.push(now);
            }
            for (i, t) in times.iter().enumerate() {
                let in_minute = times[i..].iter().take_while(|u| u.saturating_sub(*t) < Duration::from_secs(60)).count();
                prop_assert!(in_minute <= 2 * n as usize, "{in_minute} requests in a minute (limit {n})");
            }
        }
    }
}
