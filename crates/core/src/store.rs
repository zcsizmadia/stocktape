//! The quote store: latest quote per symbol plus a short price history.

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use jiff::Timestamp;

use crate::{Quote, Symbol};

/// What the store knows about one symbol.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreEntry {
    quote: Quote,
    received: Duration,
    history: VecDeque<(Timestamp, f64)>,
}

impl StoreEntry {
    /// The latest quote.
    #[must_use]
    pub fn quote(&self) -> &Quote {
        &self.quote
    }

    /// Monotonic time (from the [`Clock`](crate::Clock)) when the latest quote
    /// arrived.
    #[must_use]
    pub fn received(&self) -> Duration {
        self.received
    }

    /// Recent prices, oldest first, for sparklines.
    pub fn history(&self) -> impl ExactSizeIterator<Item = (Timestamp, f64)> + '_ {
        self.history.iter().copied()
    }

    /// Whether the latest quote is older than `max_age` at monotonic time `now`.
    #[must_use]
    pub fn is_stale(&self, now: Duration, max_age: Duration) -> bool {
        now.saturating_sub(self.received) > max_age
    }
}

/// The latest quote for every symbol, with a bounded price history each.
///
/// The store is pure: callers pass the monotonic time a quote arrived, so it
/// behaves the same under a [`ManualClock`](crate::ManualClock). Iteration is
/// in symbol order, so output is deterministic.
///
/// ```
/// use std::time::Duration;
/// use stocktape_core::{Quote, QuoteStore, Timestamp};
///
/// let mut store = QuoteStore::new(32);
/// let aapl = "AAPL".parse().unwrap();
/// store.apply(Quote::new(aapl, 238.41, Timestamp::UNIX_EPOCH), Duration::ZERO);
/// let entry = store.get(&"AAPL".parse().unwrap()).unwrap();
/// assert_eq!(entry.quote().last, 238.41);
/// assert!(entry.is_stale(Duration::from_secs(61), Duration::from_secs(60)));
/// ```
#[derive(Debug, Clone)]
pub struct QuoteStore {
    entries: BTreeMap<Symbol, StoreEntry>,
    history_capacity: usize,
}

impl QuoteStore {
    /// Creates an empty store that keeps up to `history_capacity` prices per
    /// symbol. A capacity of zero keeps no history.
    #[must_use]
    pub fn new(history_capacity: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            history_capacity,
        }
    }

    /// Records a quote that arrived at monotonic time `received`.
    ///
    /// A quote older (by its own `time`) than the one already stored is
    /// ignored, so out-of-order delivery from several feeds cannot move a
    /// price backwards. Returns whether the quote was accepted.
    pub fn apply(&mut self, quote: Quote, received: Duration) -> bool {
        if !quote.last.is_finite() {
            return false;
        }
        let capacity = self.history_capacity;
        match self.entries.get_mut(&quote.symbol) {
            Some(entry) => {
                if quote.time < entry.quote.time {
                    return false;
                }
                push_history(&mut entry.history, capacity, quote.time, quote.last);
                entry.quote = quote;
                entry.received = received;
            }
            None => {
                let mut history = VecDeque::with_capacity(capacity.min(1024));
                push_history(&mut history, capacity, quote.time, quote.last);
                self.entries.insert(
                    quote.symbol.clone(),
                    StoreEntry {
                        quote,
                        received,
                        history,
                    },
                );
            }
        }
        true
    }

    /// The entry for `symbol`, if any quote has been seen.
    #[must_use]
    pub fn get(&self, symbol: &Symbol) -> Option<&StoreEntry> {
        self.entries.get(symbol)
    }

    /// All entries, in symbol order.
    pub fn iter(&self) -> impl Iterator<Item = (&Symbol, &StoreEntry)> {
        self.entries.iter()
    }

    /// Number of symbols with a quote.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no quote has been seen yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Removes a symbol and returns its entry.
    pub fn remove(&mut self, symbol: &Symbol) -> Option<StoreEntry> {
        self.entries.remove(symbol)
    }
}

fn push_history(
    history: &mut VecDeque<(Timestamp, f64)>,
    capacity: usize,
    time: Timestamp,
    price: f64,
) {
    if capacity == 0 {
        return;
    }
    // Same-timestamp updates replace the last point instead of adding one.
    if let Some(last) = history.back_mut()
        && last.0 == time
    {
        last.1 = price;
        return;
    }
    if history.len() == capacity {
        history.pop_front();
    }
    history.push_back((time, price));
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_second(seconds).unwrap()
    }

    fn q(symbol: &str, last: f64, seconds: i64) -> Quote {
        Quote::new(symbol.parse().unwrap(), last, at(seconds))
    }

    fn sym(s: &str) -> Symbol {
        s.parse().unwrap()
    }

    #[test]
    fn keeps_latest_quote() {
        let mut store = QuoteStore::new(8);
        assert!(store.is_empty());
        assert!(store.apply(q("AAPL", 1.0, 1), Duration::from_secs(1)));
        assert!(store.apply(q("AAPL", 2.0, 2), Duration::from_secs(2)));
        let entry = store.get(&sym("AAPL")).unwrap();
        assert_eq!(entry.quote().last, 2.0);
        assert_eq!(entry.received(), Duration::from_secs(2));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn ignores_out_of_order_quotes() {
        let mut store = QuoteStore::new(8);
        store.apply(q("AAPL", 2.0, 10), Duration::from_secs(1));
        assert!(!store.apply(q("AAPL", 1.0, 5), Duration::from_secs(2)));
        let entry = store.get(&sym("AAPL")).unwrap();
        assert_eq!(entry.quote().last, 2.0);
        assert_eq!(entry.received(), Duration::from_secs(1));
    }

    #[test]
    fn rejects_non_finite_prices() {
        let mut store = QuoteStore::new(8);
        assert!(!store.apply(q("AAPL", f64::NAN, 1), Duration::ZERO));
        assert!(!store.apply(q("AAPL", f64::INFINITY, 1), Duration::ZERO));
        assert!(store.is_empty());
    }

    #[test]
    fn history_is_bounded_and_ordered() {
        let mut store = QuoteStore::new(3);
        for i in 0..5 {
            store.apply(q("AAPL", f64::from(i), i64::from(i)), Duration::ZERO);
        }
        let prices: Vec<f64> = store
            .get(&sym("AAPL"))
            .unwrap()
            .history()
            .map(|p| p.1)
            .collect();
        assert_eq!(prices, [2.0, 3.0, 4.0]);
    }

    #[test]
    fn same_timestamp_replaces_last_history_point() {
        let mut store = QuoteStore::new(8);
        store.apply(q("AAPL", 1.0, 1), Duration::ZERO);
        store.apply(q("AAPL", 1.5, 1), Duration::ZERO);
        let history: Vec<_> = store.get(&sym("AAPL")).unwrap().history().collect();
        assert_eq!(history, [(at(1), 1.5)]);
    }

    #[test]
    fn zero_capacity_keeps_no_history() {
        let mut store = QuoteStore::new(0);
        store.apply(q("AAPL", 1.0, 1), Duration::ZERO);
        assert_eq!(store.get(&sym("AAPL")).unwrap().history().len(), 0);
    }

    #[test]
    fn staleness() {
        let mut store = QuoteStore::new(1);
        store.apply(q("AAPL", 1.0, 1), Duration::from_secs(10));
        let entry = store.get(&sym("AAPL")).unwrap();
        let max_age = Duration::from_secs(60);
        assert!(!entry.is_stale(Duration::from_secs(70), max_age));
        assert!(entry.is_stale(Duration::from_secs(71), max_age));
        // A `now` before arrival is never stale.
        assert!(!entry.is_stale(Duration::ZERO, max_age));
    }

    #[test]
    fn iterates_in_symbol_order_and_removes() {
        let mut store = QuoteStore::new(1);
        for s in ["MSFT", "AAPL", "XLON:VOD"] {
            store.apply(q(s, 1.0, 1), Duration::ZERO);
        }
        let order: Vec<String> = store.iter().map(|(s, _)| s.to_string()).collect();
        assert_eq!(order, ["AAPL", "MSFT", "XLON:VOD"]);
        assert!(store.remove(&sym("MSFT")).is_some());
        assert_eq!(store.len(), 2);
    }

    proptest! {
        #[test]
        fn history_never_exceeds_capacity(
            capacity in 0usize..16,
            prices in proptest::collection::vec((0i64..1000, -1e6f64..1e6), 0..64),
        ) {
            let mut store = QuoteStore::new(capacity);
            for (t, p) in prices {
                store.apply(q("AAPL", p, t), Duration::ZERO);
            }
            if let Some(entry) = store.get(&sym("AAPL")) {
                prop_assert!(entry.history().len() <= capacity);
                let times: Vec<_> = entry.history().map(|h| h.0).collect();
                prop_assert!(times.windows(2).all(|w| w[0] < w[1]));
                if let Some(last) = entry.history().last() {
                    prop_assert_eq!(last.1, entry.quote().last);
                }
            }
        }
    }
}
