//! Choosing between several feeds, per symbol.

use std::collections::HashMap;
use std::time::Duration;

use stocktape_core::Symbol;

/// Decides whether to accept a quote, given which feed it came from.
///
/// Feeds are ranked by index (0 is the primary). A quote from a lower-ranked
/// feed is accepted only if no higher-ranked feed has delivered that symbol
/// within the failover window. So the backup takes over a symbol when the
/// primary goes quiet for it, and hands it back as soon as the primary
/// delivers again.
///
/// ```
/// use std::time::Duration;
/// use stocktape_sources::failover::Arbiter;
///
/// let mut arbiter = Arbiter::new(Duration::from_secs(15));
/// let aapl = "AAPL".parse().unwrap();
/// assert!(arbiter.accept(0, &aapl, Duration::from_secs(0)));
/// assert!(!arbiter.accept(1, &aapl, Duration::from_secs(5)));
/// assert!(arbiter.accept(1, &aapl, Duration::from_secs(16)));
/// ```
#[derive(Debug, Clone)]
pub struct Arbiter {
    window: Duration,
    /// Last time each feed delivered each symbol.
    seen: HashMap<Symbol, Vec<Option<Duration>>>,
}

impl Arbiter {
    /// Creates an arbiter that fails over after `window` of silence.
    #[must_use]
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            seen: HashMap::new(),
        }
    }

    /// Records a quote for `symbol` from `feed` at monotonic time `now` and
    /// says whether to use it.
    pub fn accept(&mut self, feed: usize, symbol: &Symbol, now: Duration) -> bool {
        let times = self.seen.entry(symbol.clone()).or_default();
        if times.len() <= feed {
            times.resize(feed + 1, None);
        }
        times[feed] = Some(now);
        let window = self.window;
        times[..feed]
            .iter()
            .flatten()
            .all(|t| now.saturating_sub(*t) > window)
    }

    /// Which feed currently serves `symbol`, if any delivered it.
    #[must_use]
    pub fn active(&self, symbol: &Symbol, now: Duration) -> Option<usize> {
        let times = self.seen.get(symbol)?;
        times
            .iter()
            .position(|t| t.is_some_and(|t| now.saturating_sub(t) <= self.window))
            .or_else(|| times.iter().rposition(Option::is_some))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(secs: u64) -> Duration {
        Duration::from_secs(secs)
    }

    #[test]
    fn primary_wins_backup_takes_over_and_hands_back() {
        let mut a = Arbiter::new(s(10));
        let sym: Symbol = "AAPL".parse().unwrap();
        assert!(a.accept(1, &sym, s(0)), "backup alone is fine");
        assert!(a.accept(0, &sym, s(1)));
        assert!(!a.accept(1, &sym, s(2)), "primary is live");
        assert_eq!(a.active(&sym, s(2)), Some(0));
        assert!(a.accept(1, &sym, s(12)), "primary silent for 11 s");
        assert_eq!(a.active(&sym, s(12)), Some(1));
        assert!(a.accept(0, &sym, s(13)), "primary back");
        assert!(!a.accept(1, &sym, s(14)));
    }

    #[test]
    fn symbols_fail_over_independently() {
        let mut a = Arbiter::new(s(10));
        let (x, y): (Symbol, Symbol) = ("X".parse().unwrap(), "Y".parse().unwrap());
        assert!(a.accept(0, &x, s(0)));
        assert!(a.accept(1, &y, s(1)), "primary never had Y");
        assert!(!a.accept(1, &x, s(2)));
        assert_eq!(a.active(&y, s(1)), Some(1));
        assert_eq!(a.active(&"Z".parse().unwrap(), s(1)), None);
    }
}
