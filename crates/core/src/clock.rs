//! Time sources.
//!
//! stocktape needs two kinds of time:
//!
//! - **Monotonic** time drives animation (scroll position, flashes, fades) and
//!   staleness. It never jumps backwards.
//! - **Wall** time (UTC) drives schedules such as market sessions.
//!
//! Code never reads the system clock directly; it asks a [`Clock`].

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use jiff::{SignedDuration, Timestamp};

/// A source of monotonic and wall-clock time.
pub trait Clock: Send + Sync {
    /// Time elapsed since the clock's own origin. Never decreases.
    fn monotonic(&self) -> Duration;

    /// The current wall-clock time in UTC.
    fn wall(&self) -> Timestamp;
}

/// The real clock, backed by [`Instant`] and the system time.
#[derive(Debug, Clone)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    /// Creates a clock whose monotonic origin is now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn monotonic(&self) -> Duration {
        self.origin.elapsed()
    }

    fn wall(&self) -> Timestamp {
        Timestamp::now()
    }
}

#[derive(Debug)]
struct ManualState {
    monotonic: Duration,
    wall: Timestamp,
}

/// A clock that only moves when told to, for deterministic tests.
///
/// Clones share the same time, so a test can keep one handle and give another
/// to the code under test.
///
/// ```
/// use std::time::Duration;
/// use stocktape_core::{Clock, ManualClock, Timestamp};
///
/// let clock = ManualClock::new(Timestamp::UNIX_EPOCH);
/// let handle = clock.clone();
/// clock.advance(Duration::from_millis(250));
/// assert_eq!(handle.monotonic(), Duration::from_millis(250));
/// ```
#[derive(Debug, Clone)]
pub struct ManualClock {
    state: Arc<Mutex<ManualState>>,
}

impl ManualClock {
    /// Creates a clock at monotonic zero and the given wall time.
    #[must_use]
    pub fn new(wall: Timestamp) -> Self {
        Self {
            state: Arc::new(Mutex::new(ManualState {
                monotonic: Duration::ZERO,
                wall,
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, ManualState> {
        // A panic while holding this lock cannot leave the state inconsistent.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Moves both monotonic and wall time forward by `step`.
    ///
    /// # Panics
    ///
    /// Panics if the wall time would overflow the supported range.
    pub fn advance(&self, step: Duration) {
        let mut state = self.lock();
        state.monotonic += step;
        let step = SignedDuration::try_from(step).expect("step fits in a SignedDuration");
        state.wall = state
            .wall
            .checked_add(step)
            .expect("wall time stays within range");
    }

    /// Sets the wall time without touching monotonic time, as when the system
    /// clock is adjusted.
    pub fn set_wall(&self, wall: Timestamp) {
        self.lock().wall = wall;
    }
}

impl Clock for ManualClock {
    fn monotonic(&self) -> Duration {
        self.lock().monotonic
    }

    fn wall(&self) -> Timestamp {
        self.lock().wall
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_starts_at_zero() {
        let clock = ManualClock::new(Timestamp::UNIX_EPOCH);
        assert_eq!(clock.monotonic(), Duration::ZERO);
        assert_eq!(clock.wall(), Timestamp::UNIX_EPOCH);
    }

    #[test]
    fn advance_moves_both_times() {
        let clock = ManualClock::new(Timestamp::UNIX_EPOCH);
        clock.advance(Duration::from_secs(90));
        assert_eq!(clock.monotonic(), Duration::from_secs(90));
        assert_eq!(clock.wall(), Timestamp::from_second(90).unwrap());
    }

    #[test]
    fn set_wall_leaves_monotonic_alone() {
        let clock = ManualClock::new(Timestamp::UNIX_EPOCH);
        clock.advance(Duration::from_secs(5));
        let later = Timestamp::from_second(1_000_000).unwrap();
        clock.set_wall(later);
        assert_eq!(clock.monotonic(), Duration::from_secs(5));
        assert_eq!(clock.wall(), later);
    }

    #[test]
    fn clones_share_time() {
        let clock = ManualClock::new(Timestamp::UNIX_EPOCH);
        let other = clock.clone();
        other.advance(Duration::from_millis(16));
        assert_eq!(clock.monotonic(), Duration::from_millis(16));
    }

    #[test]
    fn system_clock_is_monotonic() {
        let clock = SystemClock::new();
        let a = clock.monotonic();
        let b = clock.monotonic();
        assert!(b >= a);
    }

    #[test]
    fn clock_is_object_safe() {
        let clock: Box<dyn Clock> = Box::new(ManualClock::new(Timestamp::UNIX_EPOCH));
        assert_eq!(clock.monotonic(), Duration::ZERO);
    }
}
