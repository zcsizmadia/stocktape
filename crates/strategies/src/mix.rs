//! Interleaving strategies by weight.

use crate::{Context, Item, Strategy};

struct Part {
    strategy: Box<dyn Strategy>,
    weight: i64,
    current: i64,
}

/// Interleaves strategies in proportion to their weights, spreading them out
/// evenly (smooth weighted round-robin): weights 3 and 1 give `A A B A A A B A`
/// rather than `A A A B` bursts. When the chosen strategy has nothing, the
/// next one is tried.
#[derive(Default)]
pub struct Mix {
    parts: Vec<Part>,
}

impl std::fmt::Debug for Mix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mix")
            .field(
                "weights",
                &self.parts.iter().map(|p| p.weight).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Mix {
    /// An empty mix.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a strategy with a weight (zero leaves it out).
    #[must_use]
    pub fn with(mut self, strategy: impl Strategy + 'static, weight: u32) -> Self {
        if weight > 0 {
            self.parts.push(Part {
                strategy: Box::new(strategy),
                weight: i64::from(weight),
                current: 0,
            });
        }
        self
    }

    fn pick(&mut self) -> Option<usize> {
        let total: i64 = self.parts.iter().map(|p| p.weight).sum();
        for part in &mut self.parts {
            part.current += part.weight;
        }
        let (index, _) = self
            .parts
            .iter()
            .enumerate()
            .max_by_key(|(i, p)| (p.current, std::cmp::Reverse(*i)))?;
        self.parts[index].current -= total;
        Some(index)
    }
}

impl Strategy for Mix {
    fn next(&mut self, ctx: &Context<'_>) -> Option<Item> {
        let chosen = self.pick()?;
        if let Some(item) = self.parts[chosen].strategy.next(ctx) {
            return Some(item);
        }
        // The chosen part has nothing right now: try the others, most overdue
        // first, without disturbing the rotation.
        let mut others: Vec<usize> = (0..self.parts.len()).filter(|i| *i != chosen).collect();
        others.sort_by_key(|i| std::cmp::Reverse(self.parts[*i].current));
        others
            .into_iter()
            .find_map(|i| self.parts[i].strategy.next(ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Watchlist;
    use crate::test_util::{ctx, store, sym, take};
    use std::time::Duration;

    #[test]
    fn spreads_by_weight() {
        let store = store(&[("A", 1.0, 1.0), ("B", 1.0, 1.0)]);
        let ctx = ctx(&store, Duration::ZERO);
        let mut mix = Mix::new()
            .with(Watchlist::new([sym("A")]), 3)
            .with(Watchlist::new([sym("B")]), 1);
        assert_eq!(
            take(&mut mix, &ctx, 8),
            ["A", "A", "B", "A", "A", "A", "B", "A"]
        );
    }

    #[test]
    fn falls_through_when_a_part_is_empty() {
        let store = store(&[("A", 1.0, 1.0)]);
        let ctx = ctx(&store, Duration::ZERO);
        let mut mix = Mix::new()
            .with(Watchlist::new([sym("MISSING")]), 5)
            .with(Watchlist::new([sym("A")]), 1)
            .with(Watchlist::new([sym("ZERO")]), 0);
        assert_eq!(take(&mut mix, &ctx, 3), ["A", "A", "A"]);
        assert_eq!(Mix::new().next(&ctx), None);
    }
}
