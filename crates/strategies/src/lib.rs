//! Strategies: what the belt shows next, and when.
//!
//! A [`Strategy`] is asked for one [`Item`] at a time, whenever the belt needs
//! more content. Strategies are small and composable:
//!
//! | Building block | What it does |
//! |---|---|
//! | [`Watchlist`] | Round-robin through symbols, optionally in groups with header tiles. |
//! | [`Movers`] | Picks symbols at random, weighted by how much they moved. |
//! | [`Mix`] | Interleaves strategies by weight (e.g. three watchlist items, then one mover). |
//! | [`Schedule`] | Switches strategy by local time of day (pre-market, session, after hours). |
//! | [`Caps`] | Keeps repeats apart: a minimum gap per symbol and a limit per time window. |
//! | [`Highlights`] | Marks items worth attention: big moves, new session highs and lows. |
//!
//! Everything is deterministic: time comes from the [`Context`], and random
//! choices use a seeded generator, so tests can replay exact sequences.
//!
//! Part of [stocktape](https://github.com/zcsizmadia/stocktape). See
//! `docs/design-notes.md` for the design.

mod caps;
mod highlights;
mod mix;
mod movers;
mod schedule;
mod watchlist;

use std::time::Duration;

use stocktape_core::{QuoteStore, Symbol, Timestamp};

pub use caps::Caps;
pub use highlights::{HighlightRules, Highlights};
pub use mix::Mix;
pub use movers::Movers;
pub use schedule::{Schedule, Slot};
pub use watchlist::{Group, Watchlist};

/// What a strategy can see when choosing.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// Latest quotes.
    pub store: &'a QuoteStore,
    /// Monotonic time, for gaps and staleness.
    pub now: Duration,
    /// Wall-clock time, for schedules.
    pub wall: Timestamp,
}

/// Why an item is highlighted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Highlight {
    /// The percent change crossed the configured threshold.
    BigMove,
    /// The price is at a new high of the recent history.
    NewHigh,
    /// The price is at a new low of the recent history.
    NewLow,
}

/// One thing to put on the belt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A quote for this symbol, optionally highlighted.
    Quote {
        /// The instrument.
        symbol: Symbol,
        /// Set by [`Highlights`].
        highlight: Option<Highlight>,
    },
    /// A section header tile, e.g. `TECH` or `EUROPE`.
    Header(String),
}

impl Item {
    /// A plain quote item.
    #[must_use]
    pub fn quote(symbol: Symbol) -> Self {
        Self::Quote {
            symbol,
            highlight: None,
        }
    }

    /// The symbol, for quote items.
    #[must_use]
    pub fn symbol(&self) -> Option<&Symbol> {
        match self {
            Self::Quote { symbol, .. } => Some(symbol),
            Self::Header(_) => None,
        }
    }
}

/// Chooses what the belt shows next.
pub trait Strategy: Send {
    /// The next item, or `None` when there is nothing to show right now (the
    /// belt then waits and asks again on a later frame).
    fn next(&mut self, ctx: &Context<'_>) -> Option<Item>;
}

impl<S: Strategy + ?Sized> Strategy for Box<S> {
    fn next(&mut self, ctx: &Context<'_>) -> Option<Item> {
        (**self).next(ctx)
    }
}

/// xorshift64*: small, fast and deterministic; not for anything secure.
#[derive(Debug, Clone)]
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Self((seed ^ 0x9E37_79B9_7F4A_7C15) | 1)
    }

    /// Uniform in [0, 1).
    pub(crate) fn unit(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        #[allow(clippy::cast_precision_loss)]
        let v = (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64;
        v
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;
    use stocktape_core::Quote;

    /// A store with `(symbol, last, previous_close)` quotes at time zero.
    pub(crate) fn store(quotes: &[(&str, f64, f64)]) -> QuoteStore {
        let mut store = QuoteStore::new(32);
        for (s, last, close) in quotes {
            store.apply(
                Quote::new(s.parse().unwrap(), *last, Timestamp::UNIX_EPOCH)
                    .with_previous_close(*close),
                Duration::ZERO,
            );
        }
        store
    }

    pub(crate) fn ctx(store: &QuoteStore, now: Duration) -> Context<'_> {
        Context {
            store,
            now,
            wall: Timestamp::UNIX_EPOCH,
        }
    }

    pub(crate) fn sym(s: &str) -> Symbol {
        s.parse().unwrap()
    }

    /// Names of the next `n` items (`#TITLE` for headers, `-` for none).
    pub(crate) fn take(strategy: &mut dyn Strategy, ctx: &Context<'_>, n: usize) -> Vec<String> {
        (0..n)
            .map(|_| match strategy.next(ctx) {
                Some(Item::Quote { symbol, .. }) => symbol.to_string(),
                Some(Item::Header(h)) => format!("#{h}"),
                None => "-".to_owned(),
            })
            .collect()
    }
}
