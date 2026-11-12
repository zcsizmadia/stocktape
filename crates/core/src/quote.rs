//! Quotes.

use std::fmt;

use jiff::Timestamp;

use crate::Symbol;

/// Which way a price moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Higher than the reference price.
    Up,
    /// Lower than the reference price.
    Down,
    /// Unchanged, or no reference price.
    Flat,
}

/// Why a currency code was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencyError(String);

impl fmt::Display for CurrencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid currency code {:?}", self.0)
    }
}

impl std::error::Error for CurrencyError {}

/// An ISO 4217 currency code, such as `USD`, `GBP` or `JPY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Currency([u8; 3]);

impl Currency {
    /// US dollar.
    pub const USD: Self = Self(*b"USD");

    /// Parses a three-letter code; case-insensitive.
    ///
    /// # Errors
    ///
    /// Returns an error unless the code is exactly three ASCII letters.
    pub fn new(code: &str) -> Result<Self, CurrencyError> {
        match code.trim().as_bytes() {
            [a, b, c] if [a, b, c].iter().all(|x| x.is_ascii_alphabetic()) => Ok(Self([
                a.to_ascii_uppercase(),
                b.to_ascii_uppercase(),
                c.to_ascii_uppercase(),
            ])),
            _ => Err(CurrencyError(code.to_owned())),
        }
    }

    /// The code as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // Only ASCII letters are ever stored.
        std::str::from_utf8(&self.0).unwrap_or("???")
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The latest known state of one instrument.
#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    /// The instrument.
    pub symbol: Symbol,
    /// Last traded (or mid) price.
    pub last: f64,
    /// Reference price for the change, usually the previous session's close.
    pub previous_close: Option<f64>,
    /// Volume traded in the current session, if known.
    pub volume: Option<u64>,
    /// Currency the prices are quoted in, if known.
    pub currency: Option<Currency>,
    /// When the provider says this price was observed.
    pub time: Timestamp,
}

impl Quote {
    /// Creates a quote with only a price and time; other fields are empty.
    #[must_use]
    pub fn new(symbol: Symbol, last: f64, time: Timestamp) -> Self {
        Self {
            symbol,
            last,
            previous_close: None,
            volume: None,
            currency: None,
            time,
        }
    }

    /// Sets the previous close.
    #[must_use]
    pub fn with_previous_close(mut self, previous_close: f64) -> Self {
        self.previous_close = Some(previous_close);
        self
    }

    /// Sets the volume.
    #[must_use]
    pub fn with_volume(mut self, volume: u64) -> Self {
        self.volume = Some(volume);
        self
    }

    /// Sets the currency.
    #[must_use]
    pub fn with_currency(mut self, currency: Currency) -> Self {
        self.currency = Some(currency);
        self
    }

    /// Absolute change from the previous close.
    #[must_use]
    pub fn change(&self) -> Option<f64> {
        let previous = self.previous_close.filter(|p| p.is_finite())?;
        let change = self.last - previous;
        change.is_finite().then_some(change)
    }

    /// Percentage change from the previous close (`1.5` means +1.5 %).
    ///
    /// `None` when there is no usable previous close (missing, zero or not
    /// finite).
    #[must_use]
    pub fn change_percent(&self) -> Option<f64> {
        let previous = self.previous_close.filter(|p| p.is_finite() && *p != 0.0)?;
        let percent = (self.last - previous) / previous.abs() * 100.0;
        percent.is_finite().then_some(percent)
    }

    /// Direction of the move from the previous close.
    #[must_use]
    pub fn direction(&self) -> Direction {
        match self.change() {
            Some(c) if c > 0.0 => Direction::Up,
            Some(c) if c < 0.0 => Direction::Down,
            _ => Direction::Flat,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn quote(last: f64, previous: Option<f64>) -> Quote {
        let q = Quote::new("AAPL".parse().unwrap(), last, Timestamp::UNIX_EPOCH);
        match previous {
            Some(p) => q.with_previous_close(p),
            None => q,
        }
    }

    #[test]
    fn change_and_percent() {
        let q = quote(110.0, Some(100.0));
        assert_eq!(q.change(), Some(10.0));
        assert_eq!(q.change_percent(), Some(10.0));
        assert_eq!(q.direction(), Direction::Up);

        let q = quote(95.0, Some(100.0));
        assert_eq!(q.change(), Some(-5.0));
        assert_eq!(q.change_percent(), Some(-5.0));
        assert_eq!(q.direction(), Direction::Down);
    }

    #[test]
    fn no_reference_price_is_flat() {
        let q = quote(100.0, None);
        assert_eq!(q.change(), None);
        assert_eq!(q.change_percent(), None);
        assert_eq!(q.direction(), Direction::Flat);
    }

    #[test]
    fn zero_or_nan_reference_has_no_percent() {
        assert_eq!(quote(5.0, Some(0.0)).change_percent(), None);
        assert_eq!(quote(5.0, Some(f64::NAN)).change_percent(), None);
        assert_eq!(quote(5.0, Some(f64::NAN)).direction(), Direction::Flat);
    }

    #[test]
    fn negative_reference_uses_magnitude() {
        // Oil futures briefly traded below zero in 2020.
        let q = quote(-10.0, Some(-20.0));
        assert_eq!(q.change_percent(), Some(50.0));
        assert_eq!(q.direction(), Direction::Up);
    }

    #[test]
    fn builder_sets_fields() {
        let q = quote(1.0, None)
            .with_volume(42)
            .with_currency(Currency::new("gbp").unwrap());
        assert_eq!(q.volume, Some(42));
        assert_eq!(q.currency.unwrap().as_str(), "GBP");
    }

    #[test]
    fn currency_validation() {
        assert_eq!(Currency::new("usd"), Ok(Currency::USD));
        assert!(Currency::new("US").is_err());
        assert!(Currency::new("US1").is_err());
        assert!(Currency::new("€UR").is_err());
    }

    proptest! {
        #[test]
        fn direction_matches_change_sign(last in -1e9f64..1e9, previous in -1e9f64..1e9) {
            let q = quote(last, Some(previous));
            let expected = if last > previous {
                Direction::Up
            } else if last < previous {
                Direction::Down
            } else {
                Direction::Flat
            };
            prop_assert_eq!(q.direction(), expected);
        }

        #[test]
        fn percent_is_finite_when_present(last in any::<f64>(), previous in any::<f64>()) {
            if let Some(p) = quote(last, Some(previous)).change_percent() {
                prop_assert!(p.is_finite());
            }
        }
    }
}
