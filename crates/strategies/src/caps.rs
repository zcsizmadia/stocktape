//! Frequency caps.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use stocktape_core::Symbol;

use crate::{Context, Item, Strategy};

/// How many candidates to try before giving up for this request.
const ATTEMPTS: usize = 32;

/// Wraps a strategy and skips symbols shown too recently: at least `min_gap`
/// between two appearances of the same symbol, and at most `n` appearances per
/// `window`. Headers pass through.
#[derive(Debug, Clone)]
pub struct Caps<S> {
    inner: S,
    min_gap: Duration,
    limit: Option<(usize, Duration)>,
    shown: HashMap<Symbol, VecDeque<Duration>>,
}

impl<S: Strategy> Caps<S> {
    /// Caps `inner` with a minimum gap per symbol.
    pub fn new(inner: S, min_gap: Duration) -> Self {
        Self {
            inner,
            min_gap,
            limit: None,
            shown: HashMap::new(),
        }
    }

    /// Also allows at most `count` appearances of a symbol per `window`.
    #[must_use]
    pub fn at_most(mut self, count: usize, window: Duration) -> Self {
        self.limit = Some((count, window));
        self
    }

    fn allowed(&mut self, symbol: &Symbol, now: Duration) -> bool {
        let horizon = self
            .limit
            .map_or(self.min_gap, |(_, w)| w.max(self.min_gap));
        let Some(times) = self.shown.get_mut(symbol) else {
            return true;
        };
        while times
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= horizon)
        {
            times.pop_front();
        }
        let gap_ok = times
            .back()
            .is_none_or(|t| now.saturating_sub(*t) >= self.min_gap);
        let count_ok = self.limit.is_none_or(|(n, window)| {
            times
                .iter()
                .filter(|t| now.saturating_sub(**t) < window)
                .count()
                < n
        });
        gap_ok && count_ok
    }
}

impl<S: Strategy> Strategy for Caps<S> {
    fn next(&mut self, ctx: &Context<'_>) -> Option<Item> {
        for _ in 0..ATTEMPTS {
            let item = self.inner.next(ctx)?;
            let Some(symbol) = item.symbol() else {
                return Some(item);
            };
            if self.allowed(symbol, ctx.now) {
                self.shown
                    .entry(symbol.clone())
                    .or_default()
                    .push_back(ctx.now);
                return Some(item);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Watchlist;
    use crate::test_util::{ctx, store, sym, take};

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn min_gap_skips_recent_symbols() {
        let store = store(&[("A", 1.0, 1.0), ("B", 1.0, 1.0)]);
        let mut caps = Caps::new(Watchlist::new(["A", "B"].map(sym)), secs(10));
        assert_eq!(take(&mut caps, &ctx(&store, secs(0)), 3), ["A", "B", "-"]);
        assert_eq!(take(&mut caps, &ctx(&store, secs(9)), 1), ["-"]);
        assert_eq!(take(&mut caps, &ctx(&store, secs(10)), 3), ["A", "B", "-"]);
    }

    #[test]
    fn count_per_window() {
        let store = store(&[("A", 1.0, 1.0)]);
        let mut caps = Caps::new(Watchlist::new([sym("A")]), secs(0)).at_most(2, secs(60));
        let shown: Vec<String> = (0..6)
            .flat_map(|i| take(&mut caps, &ctx(&store, secs(i * 10)), 1))
            .collect();
        assert_eq!(shown, ["A", "A", "-", "-", "-", "-"]);
        assert_eq!(take(&mut caps, &ctx(&store, secs(60)), 1), ["A"]);
    }

    #[test]
    fn headers_pass_through() {
        let store = store(&[("A", 1.0, 1.0)]);
        let list = Watchlist::grouped(vec![crate::Group::titled("T", [sym("A")])]);
        let mut caps = Caps::new(list, secs(100));
        assert_eq!(take(&mut caps, &ctx(&store, secs(0)), 3), ["#T", "A", "#T"]);
    }
}
