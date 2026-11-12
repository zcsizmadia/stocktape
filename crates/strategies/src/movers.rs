//! Big movers, picked more often the more they moved.

use stocktape_core::Symbol;

use crate::{Context, Item, Rng, Strategy};

/// Picks a symbol at random, with probability proportional to the size of
/// its percent move. Symbols that moved less than the threshold are left out.
#[derive(Debug, Clone)]
pub struct Movers {
    symbols: Option<Vec<Symbol>>,
    min_move: f64,
    rng: Rng,
}

impl Movers {
    /// Considers every symbol in the store; `seed` makes picks repeatable.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            symbols: None,
            min_move: 0.0,
            rng: Rng::new(seed),
        }
    }

    /// Only considers these symbols.
    #[must_use]
    pub fn among(mut self, symbols: impl IntoIterator<Item = Symbol>) -> Self {
        self.symbols = Some(symbols.into_iter().collect());
        self
    }

    /// Leaves out symbols that moved less than `percent` (absolute).
    #[must_use]
    pub fn min_move(mut self, percent: f64) -> Self {
        self.min_move = percent.abs();
        self
    }
}

impl Strategy for Movers {
    fn next(&mut self, ctx: &Context<'_>) -> Option<Item> {
        let candidates: Vec<(&Symbol, f64)> = ctx
            .store
            .iter()
            .filter(|(s, _)| self.symbols.as_ref().is_none_or(|list| list.contains(s)))
            .filter_map(|(s, e)| {
                let size = e.quote().change_percent()?.abs();
                (size > 0.0 && size >= self.min_move).then_some((s, size))
            })
            .collect();
        let total: f64 = candidates.iter().map(|c| c.1).sum();
        if candidates.is_empty() || !total.is_finite() {
            return None;
        }
        let mut pick = self.rng.unit() * total;
        for (symbol, weight) in &candidates {
            if pick < *weight {
                return Some(Item::quote((*symbol).clone()));
            }
            pick -= weight;
        }
        candidates.last().map(|(s, _)| Item::quote((*s).clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{ctx, store, sym, take};
    use std::collections::HashMap;
    use std::time::Duration;

    #[test]
    fn bigger_moves_come_up_more_often() {
        // +10%, -5%, +1%
        let store = store(&[
            ("BIG", 110.0, 100.0),
            ("MID", 95.0, 100.0),
            ("SML", 101.0, 100.0),
        ]);
        let ctx = ctx(&store, Duration::ZERO);
        let mut movers = Movers::new(1);
        let mut counts: HashMap<String, usize> = HashMap::new();
        for name in take(&mut movers, &ctx, 16_000) {
            *counts.entry(name).or_default() += 1;
        }
        let big = counts["BIG"] as f64;
        let mid = counts["MID"] as f64;
        let small = counts["SML"] as f64;
        assert!((big / mid - 2.0).abs() < 0.2, "{counts:?}");
        assert!((mid / small - 5.0).abs() < 1.0, "{counts:?}");
    }

    #[test]
    fn threshold_and_filter() {
        let store = store(&[
            ("BIG", 110.0, 100.0),
            ("SML", 101.0, 100.0),
            ("FLAT", 1.0, 1.0),
        ]);
        let ctx = ctx(&store, Duration::ZERO);
        let mut movers = Movers::new(2).min_move(2.0);
        assert!(take(&mut movers, &ctx, 50).iter().all(|n| n == "BIG"));
        let mut only = Movers::new(3).among([sym("SML"), sym("FLAT")]);
        assert!(take(&mut only, &ctx, 50).iter().all(|n| n == "SML"));
        let mut none = Movers::new(4).min_move(50.0);
        assert_eq!(none.next(&ctx), None);
    }

    #[test]
    fn same_seed_same_picks() {
        let store = store(&[("A", 110.0, 100.0), ("B", 95.0, 100.0), ("C", 101.0, 100.0)]);
        let ctx = ctx(&store, Duration::ZERO);
        let a = take(&mut Movers::new(9), &ctx, 100);
        let b = take(&mut Movers::new(9), &ctx, 100);
        assert_eq!(a, b);
    }
}
