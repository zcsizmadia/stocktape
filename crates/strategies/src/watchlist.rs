//! Round-robin through a watchlist.

use stocktape_core::Symbol;

use crate::{Context, Item, Strategy};

/// A named section of a watchlist, e.g. `TECH` or `EUROPE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// Header tile text; `None` for no header.
    pub title: Option<String>,
    /// Symbols in display order.
    pub symbols: Vec<Symbol>,
}

impl Group {
    /// A group with a header.
    pub fn titled(title: &str, symbols: impl IntoIterator<Item = Symbol>) -> Self {
        Self {
            title: Some(title.to_owned()),
            symbols: symbols.into_iter().collect(),
        }
    }
}

/// Cycles through symbols in order, group by group, skipping symbols that have
/// no quote yet. A header tile opens each titled group (only if the group has
/// something to show).
///
/// ```
/// use std::time::Duration;
/// use stocktape_core::{Quote, QuoteStore, Timestamp};
/// use stocktape_strategies::{Context, Item, Strategy, Watchlist};
///
/// let mut store = QuoteStore::new(8);
/// for s in ["AAPL", "MSFT"] {
///     store.apply(Quote::new(s.parse().unwrap(), 1.0, Timestamp::UNIX_EPOCH), Duration::ZERO);
/// }
/// let ctx = Context { store: &store, now: Duration::ZERO, wall: Timestamp::UNIX_EPOCH, headlines: &[] };
/// let mut list = Watchlist::new(["AAPL", "MSFT"].map(|s| s.parse().unwrap()));
/// assert_eq!(list.next(&ctx), Some(Item::quote("AAPL".parse().unwrap())));
/// assert_eq!(list.next(&ctx), Some(Item::quote("MSFT".parse().unwrap())));
/// assert_eq!(list.next(&ctx), Some(Item::quote("AAPL".parse().unwrap())));
/// ```
#[derive(Debug, Clone)]
pub struct Watchlist {
    groups: Vec<Group>,
    group: usize,
    index: usize,
    header_due: bool,
}

impl Watchlist {
    /// One untitled group.
    pub fn new(symbols: impl IntoIterator<Item = Symbol>) -> Self {
        Self::grouped(vec![Group {
            title: None,
            symbols: symbols.into_iter().collect(),
        }])
    }

    /// Several groups, each opened by its header tile when titled.
    #[must_use]
    pub fn grouped(groups: Vec<Group>) -> Self {
        Self {
            groups,
            group: 0,
            index: 0,
            header_due: true,
        }
    }

    fn advance_group(&mut self) {
        self.group = (self.group + 1) % self.groups.len().max(1);
        self.index = 0;
        self.header_due = true;
    }
}

impl Strategy for Watchlist {
    fn next(&mut self, ctx: &Context<'_>) -> Option<Item> {
        let steps: usize = self.groups.iter().map(|g| g.symbols.len() + 1).sum();
        for _ in 0..=steps {
            let group = self.groups.get(self.group)?;
            if self.index >= group.symbols.len() {
                self.advance_group();
                continue;
            }
            if self.header_due {
                self.header_due = false;
                if let Some(title) = &group.title {
                    let has_quotes = group.symbols.iter().any(|s| ctx.store.get(s).is_some());
                    if has_quotes {
                        return Some(Item::Header(title.clone()));
                    }
                    self.advance_group();
                    continue;
                }
            }
            let symbol = &group.symbols[self.index];
            self.index += 1;
            if ctx.store.get(symbol).is_some() {
                return Some(Item::quote(symbol.clone()));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{ctx, store, sym, take};
    use std::time::Duration;

    #[test]
    fn cycles_and_skips_unquoted() {
        let store = store(&[("A", 1.0, 1.0), ("C", 1.0, 1.0)]);
        let ctx = ctx(&store, Duration::ZERO);
        let mut list = Watchlist::new(["A", "B", "C"].map(sym));
        assert_eq!(take(&mut list, &ctx, 5), ["A", "C", "A", "C", "A"]);
    }

    #[test]
    fn groups_get_headers() {
        let store = store(&[("A", 1.0, 1.0), ("B", 1.0, 1.0), ("X", 1.0, 1.0)]);
        let ctx = ctx(&store, Duration::ZERO);
        let mut list = Watchlist::grouped(vec![
            Group::titled("TECH", ["A", "B"].map(sym)),
            Group::titled("EMPTY", ["Q"].map(sym)),
            Group::titled("ENERGY", ["X"].map(sym)),
        ]);
        assert_eq!(
            take(&mut list, &ctx, 7),
            ["#TECH", "A", "B", "#ENERGY", "X", "#TECH", "A"]
        );
    }

    #[test]
    fn nothing_quoted_gives_none() {
        let store = store(&[]);
        let ctx = ctx(&store, Duration::ZERO);
        let mut list = Watchlist::grouped(vec![Group::titled("T", ["A"].map(sym))]);
        assert_eq!(list.next(&ctx), None);
        assert_eq!(Watchlist::new([]).next(&ctx), None);
        assert_eq!(Watchlist::grouped(vec![]).next(&ctx), None);
    }
}
