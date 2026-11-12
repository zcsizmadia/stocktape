//! Event highlights.

use crate::{Context, Highlight, Item, Strategy};

/// When to highlight a quote.
#[derive(Debug, Clone, PartialEq)]
pub struct HighlightRules {
    /// Highlight moves of at least this many percent (absolute).
    pub big_move: Option<f64>,
    /// Highlight a price at a new high or low of the recent history.
    pub new_high_low: bool,
    /// History points needed before highs and lows count.
    pub min_history: usize,
}

impl Default for HighlightRules {
    fn default() -> Self {
        Self {
            big_move: Some(2.0),
            new_high_low: true,
            min_history: 10,
        }
    }
}

/// Wraps a strategy and marks items that deserve attention. Big moves win
/// over new highs and lows.
#[derive(Debug, Clone)]
pub struct Highlights<S> {
    inner: S,
    rules: HighlightRules,
}

impl<S: Strategy> Highlights<S> {
    /// Highlights `inner`'s items by `rules`.
    pub fn new(inner: S, rules: HighlightRules) -> Self {
        Self { inner, rules }
    }

    fn classify(&self, ctx: &Context<'_>, item: &Item) -> Option<Highlight> {
        let entry = ctx.store.get(item.symbol()?)?;
        let quote = entry.quote();
        if let (Some(threshold), Some(pct)) = (self.rules.big_move, quote.change_percent())
            && pct.abs() >= threshold
        {
            return Some(Highlight::BigMove);
        }
        if self.rules.new_high_low && entry.history().len() >= self.rules.min_history.max(2) {
            let prices: Vec<f64> = entry.history().map(|(_, p)| p).collect();
            let (last, earlier) = prices.split_last()?;
            if earlier.iter().all(|p| last > p) {
                return Some(Highlight::NewHigh);
            }
            if earlier.iter().all(|p| last < p) {
                return Some(Highlight::NewLow);
            }
        }
        None
    }
}

impl<S: Strategy> Strategy for Highlights<S> {
    fn next(&mut self, ctx: &Context<'_>) -> Option<Item> {
        let item = self.inner.next(ctx)?;
        let highlight = self.classify(ctx, &item);
        Some(match item {
            Item::Quote { symbol, .. } => Item::Quote { symbol, highlight },
            header => header,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Watchlist;
    use crate::test_util::{ctx, store, sym};
    use std::time::Duration;
    use stocktape_core::{Quote, QuoteStore, Timestamp};

    fn highlight_of(store: &QuoteStore, s: &str, rules: HighlightRules) -> Option<Highlight> {
        let mut h = Highlights::new(Watchlist::new([sym(s)]), rules);
        match h.next(&ctx(store, Duration::ZERO)) {
            Some(Item::Quote { highlight, .. }) => highlight,
            other => panic!("{other:?}"),
        }
    }

    fn walk(prices: &[f64]) -> QuoteStore {
        let mut store = QuoteStore::new(64);
        for (i, p) in prices.iter().enumerate() {
            let t = Timestamp::from_second(i64::try_from(i).unwrap()).unwrap();
            store.apply(
                Quote::new(sym("W"), *p, t).with_previous_close(100.0),
                Duration::ZERO,
            );
        }
        store
    }

    #[test]
    fn big_moves() {
        let store = store(&[
            ("UP", 103.0, 100.0),
            ("DN", 97.0, 100.0),
            ("SM", 101.0, 100.0),
        ]);
        let rules = HighlightRules::default();
        assert_eq!(
            highlight_of(&store, "UP", rules.clone()),
            Some(Highlight::BigMove)
        );
        assert_eq!(
            highlight_of(&store, "DN", rules.clone()),
            Some(Highlight::BigMove)
        );
        assert_eq!(highlight_of(&store, "SM", rules), None);
    }

    #[test]
    fn new_highs_and_lows() {
        let rules = HighlightRules {
            big_move: None,
            ..HighlightRules::default()
        };
        let rising: Vec<f64> = (0..12).map(|i| 100.0 + f64::from(i) * 0.1).collect();
        assert_eq!(
            highlight_of(&walk(&rising), "W", rules.clone()),
            Some(Highlight::NewHigh)
        );
        let falling: Vec<f64> = rising.iter().rev().copied().collect();
        assert_eq!(
            highlight_of(&walk(&falling), "W", rules.clone()),
            Some(Highlight::NewLow)
        );
        let choppy = [
            100.0, 100.5, 100.2, 100.4, 100.1, 100.3, 100.2, 100.4, 100.1, 100.3, 100.2,
        ];
        assert_eq!(highlight_of(&walk(&choppy), "W", rules.clone()), None);
        assert_eq!(
            highlight_of(&walk(&rising[..5]), "W", rules),
            None,
            "too little history"
        );
    }
}
