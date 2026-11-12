//! A deterministic mock feed for demos and tests.
//!
//! Each instrument does a small random walk around its starting price. The
//! walk is driven by a seeded generator, so the same seed and the same call
//! sequence always produce the same quotes.

use stocktape_core::{Currency, Quote, Symbol, Timestamp};

/// One instrument in the mock feed.
#[derive(Debug, Clone)]
struct Instrument {
    symbol: Symbol,
    previous_close: f64,
    last: f64,
    volume: u64,
    currency: Currency,
    /// Typical move per tick, as a fraction of the price.
    volatility: f64,
}

/// A mock feed producing random-walk quotes.
///
/// ```
/// use stocktape_core::Timestamp;
/// use stocktape_sources::mock::MockFeed;
///
/// let mut a = MockFeed::demo(7);
/// let mut b = MockFeed::demo(7);
/// let t = Timestamp::UNIX_EPOCH;
/// assert_eq!(a.tick(t), b.tick(t));
/// ```
#[derive(Debug, Clone)]
pub struct MockFeed {
    instruments: Vec<Instrument>,
    rng: XorShift,
}

impl MockFeed {
    /// Creates an empty feed with the given seed.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            instruments: Vec::new(),
            rng: XorShift::new(seed),
        }
    }

    /// A feed with a demo watchlist of US stocks, indices and crypto.
    ///
    /// Prices are sample values for demos, not real market data.
    ///
    /// # Panics
    ///
    /// Never: the demo symbols are valid.
    #[must_use]
    pub fn demo(seed: u64) -> Self {
        let mut feed = Self::new(seed);
        for (symbol, close, volatility) in [
            ("AAPL", 236.24, 0.0008),
            ("MSFT", 515.52, 0.0008),
            ("NVDA", 182.07, 0.0015),
            ("AMZN", 223.13, 0.0010),
            ("GOOGL", 252.25, 0.0009),
            ("META", 733.29, 0.0012),
            ("TSLA", 443.86, 0.0020),
            ("JPM", 301.37, 0.0007),
            ("XOM", 113.84, 0.0007),
            ("BTC-USD", 99_372.0, 0.0018),
        ] {
            feed.add(
                symbol.parse().expect("demo symbols are valid"),
                close,
                Currency::USD,
                volatility,
            );
        }
        feed
    }

    /// Adds an instrument starting at `previous_close`.
    pub fn add(
        &mut self,
        symbol: Symbol,
        previous_close: f64,
        currency: Currency,
        volatility: f64,
    ) {
        let opening_move = 1.0 + self.rng.next_signed() * volatility * 10.0;
        self.instruments.push(Instrument {
            symbol,
            previous_close,
            last: previous_close * opening_move,
            volume: 0,
            currency,
            volatility,
        });
    }

    /// Adds sample instruments for any of `symbols` the feed doesn't have
    /// yet, so every configured symbol shows something without a real feed.
    /// Each gets a made-up but stable starting price derived from its name.
    pub fn add_samples<'a>(&mut self, symbols: impl IntoIterator<Item = &'a Symbol>) {
        for symbol in symbols {
            if self.instruments.iter().any(|i| &i.symbol == symbol) {
                continue;
            }
            // FNV-1a over the name: the same symbol always starts at the same
            // price, between 10 and 600.
            let hash = symbol
                .to_string()
                .bytes()
                .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
                });
            #[allow(clippy::cast_precision_loss)]
            let close = 10.0 + (hash % 59_000) as f64 / 100.0;
            self.add(symbol.clone(), close, Currency::USD, 0.001);
        }
    }

    /// The symbols in the feed, in the order they were added.
    pub fn symbols(&self) -> impl Iterator<Item = &Symbol> {
        self.instruments.iter().map(|i| &i.symbol)
    }

    /// Advances every instrument one step and returns the new quotes, stamped
    /// with `time`.
    pub fn tick(&mut self, time: Timestamp) -> Vec<Quote> {
        let rng = &mut self.rng;
        self.instruments
            .iter_mut()
            .map(|instrument| {
                let step = rng.next_signed() * instrument.volatility;
                // Gentle pull back towards the previous close keeps the walk bounded.
                let drift = (instrument.previous_close - instrument.last)
                    / instrument.previous_close
                    * instrument.volatility
                    * 0.05;
                instrument.last *= 1.0 + step + drift;
                instrument.volume += 100 + rng.next_u64() % 5_000;
                Quote::new(instrument.symbol.clone(), instrument.last, time)
                    .with_previous_close(instrument.previous_close)
                    .with_volume(instrument.volume)
                    .with_currency(instrument.currency)
            })
            .collect()
    }
}

/// xorshift64*: small, fast and deterministic; not for anything secure.
#[derive(Debug, Clone)]
struct XorShift(u64);

impl XorShift {
    fn new(seed: u64) -> Self {
        // Zero is a fixed point of xorshift; avoid it.
        Self(seed ^ 0x9E37_79B9_7F4A_7C15 | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [-1, 1).
    fn next_signed(&mut self) -> f64 {
        // 53 random bits mapped to [0, 1), then to [-1, 1).
        #[allow(clippy::cast_precision_loss)]
        let unit = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        unit * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_for_any_symbol() {
        let mut feed = MockFeed::demo(1);
        let extra: Vec<Symbol> = ["FANG", "QCOM", "AAPL"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        feed.add_samples(&extra);
        let symbols: Vec<String> = feed.symbols().map(ToString::to_string).collect();
        assert_eq!(symbols.len(), 12, "AAPL was already there: {symbols:?}");
        let quotes = feed.tick(Timestamp::UNIX_EPOCH);
        let fang = quotes.iter().find(|q| q.symbol.ticker() == "FANG").unwrap();
        assert!((10.0..=610.0).contains(&fang.last), "{fang:?}");
        // Stable: the same symbol starts at the same price in another feed.
        let mut again = MockFeed::new(99);
        again.add_samples(&extra[..1]);
        let first = again.tick(Timestamp::UNIX_EPOCH);
        assert_eq!(first[0].previous_close, fang.previous_close);
    }

    fn at(s: i64) -> Timestamp {
        Timestamp::from_second(s).unwrap()
    }

    #[test]
    fn same_seed_same_quotes() {
        let mut a = MockFeed::demo(42);
        let mut b = MockFeed::demo(42);
        for s in 0..100 {
            assert_eq!(a.tick(at(s)), b.tick(at(s)));
        }
    }

    #[test]
    fn different_seeds_differ() {
        let mut a = MockFeed::demo(1);
        let mut b = MockFeed::demo(2);
        assert_ne!(a.tick(at(0)), b.tick(at(0)));
    }

    #[test]
    fn quotes_are_complete_and_stay_near_close() {
        let mut feed = MockFeed::demo(3);
        let mut last = Vec::new();
        for s in 0..10_000 {
            last = feed.tick(at(s));
        }
        assert_eq!(last.len(), 10);
        for quote in &last {
            let close = quote.previous_close.unwrap();
            assert!(quote.last.is_finite());
            assert!((quote.last / close - 1.0).abs() < 0.5, "{quote:?}");
            assert_eq!(quote.time, at(9_999));
            assert!(quote.volume.unwrap() > 0);
        }
    }

    #[test]
    fn signed_values_cover_the_range() {
        let mut rng = XorShift::new(0);
        let values: Vec<f64> = (0..10_000).map(|_| rng.next_signed()).collect();
        assert!(values.iter().all(|v| (-1.0..1.0).contains(v)));
        assert!(values.iter().any(|v| *v < -0.9));
        assert!(values.iter().any(|v| *v > 0.9));
    }
}
