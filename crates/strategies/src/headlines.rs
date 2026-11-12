//! Headlines and text messages.

use std::collections::VecDeque;

use crate::{Context, Item, Strategy};

/// How many recently shown headlines are remembered, so a short list cycles
/// instead of repeating the newest one.
const MEMORY: usize = 64;

/// Shows the latest headlines, newest first. Once all have been shown it
/// starts over, and a new headline is picked up as soon as it arrives.
#[derive(Debug, Clone, Default)]
pub struct Headlines {
    shown: VecDeque<String>,
}

impl Headlines {
    /// A headline strategy.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Strategy for Headlines {
    fn next(&mut self, ctx: &Context<'_>) -> Option<Item> {
        let key = |h: &stocktape_core::Headline| format!("{}\u{1f}{}", h.source, h.title);
        let fresh = ctx.headlines.iter().find(|h| !self.shown.contains(&key(h)));
        let pick = match fresh {
            Some(h) => h,
            None => {
                // Everything was shown: start a new round.
                self.shown.clear();
                ctx.headlines.first()?
            }
        };
        self.shown.push_back(key(pick));
        if self.shown.len() > MEMORY {
            self.shown.pop_front();
        }
        Some(Item::Headline(pick.clone()))
    }
}

/// Cycles through fixed text messages.
#[derive(Debug, Clone)]
pub struct Messages {
    texts: Vec<String>,
    next: usize,
}

impl Messages {
    /// Messages to cycle through; empty ones are dropped.
    pub fn new(texts: impl IntoIterator<Item = String>) -> Self {
        Self {
            texts: texts.into_iter().filter(|t| !t.trim().is_empty()).collect(),
            next: 0,
        }
    }
}

impl Strategy for Messages {
    fn next(&mut self, _ctx: &Context<'_>) -> Option<Item> {
        let text = self.texts.get(self.next % self.texts.len().max(1))?.clone();
        self.next = (self.next + 1) % self.texts.len();
        Some(Item::Text(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{store, take};
    use crate::{Mix, Watchlist};
    use std::time::Duration;
    use stocktape_core::{Headline, Timestamp};

    fn headline(title: &str) -> Headline {
        Headline {
            source: "Desk".into(),
            title: title.into(),
            published: None,
            link: None,
        }
    }

    fn ctx<'a>(store: &'a stocktape_core::QuoteStore, headlines: &'a [Headline]) -> Context<'a> {
        Context {
            store,
            now: Duration::ZERO,
            wall: Timestamp::UNIX_EPOCH,
            headlines,
        }
    }

    #[test]
    fn cycles_newest_first_and_picks_up_new_ones() {
        let store = store(&[]);
        let mut list = vec![headline("B"), headline("A")];
        let mut h = Headlines::new();
        assert_eq!(take(&mut h, &ctx(&store, &list), 3), ["!B", "!A", "!B"]);
        list.insert(0, headline("C"));
        assert_eq!(take(&mut h, &ctx(&store, &list), 2), ["!C", "!A"]);
        assert_eq!(Headlines::new().next(&ctx(&store, &[])), None);
    }

    #[test]
    fn messages_cycle() {
        let store = store(&[]);
        let mut m = Messages::new(["One".to_owned(), " ".to_owned(), "Two".to_owned()]);
        assert_eq!(
            take(&mut m, &ctx(&store, &[]), 3),
            ["\"One\"", "\"Two\"", "\"One\""]
        );
        assert_eq!(Messages::new([]).next(&ctx(&store, &[])), None);
    }

    #[test]
    fn mixing_quotes_and_headlines() {
        let store = store(&[("AAPL", 1.0, 1.0)]);
        let list = [headline("News")];
        let mut mix = Mix::new()
            .with(Watchlist::new(["AAPL".parse().unwrap()]), 2)
            .with(Headlines::new(), 1);
        assert_eq!(
            take(&mut mix, &ctx(&store, &list), 6),
            ["AAPL", "!News", "AAPL", "AAPL", "!News", "AAPL"]
        );
        // Without headlines yet, the quotes fill in.
        assert_eq!(
            take(&mut mix, &ctx(&store, &[]), 3),
            ["AAPL", "AAPL", "AAPL"]
        );
    }
}
