//! Instrument symbols.
//!
//! The canonical text form is `TICKER` for instruments without an exchange,
//! or `EXCHANGE:TICKER` (for example `XLON:VOD`, `XTKS:7203`). A colon is used
//! rather than a dot suffix because tickers such as `BRK.B` already contain
//! dots. Provider-specific forms (`VOD.L`, `7203.T`) are mapped by the feed
//! adapters.

use std::fmt;
use std::str::FromStr;

/// Maximum length of a ticker, in bytes.
const MAX_TICKER_LEN: usize = 32;
/// Maximum length of an exchange code, in bytes.
const MAX_EXCHANGE_LEN: usize = 8;

/// Why a symbol or exchange code was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolError {
    /// The ticker was empty.
    EmptyTicker,
    /// The ticker was longer than allowed.
    TickerTooLong,
    /// The ticker contained a character other than `A-Z`, `0-9`, `.`, `-`,
    /// `_`, `^`, `=` or `/`.
    InvalidTickerChar(char),
    /// The exchange code was empty, too long, or not `A-Z`/`0-9`.
    InvalidExchange(String),
}

impl fmt::Display for SymbolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTicker => f.write_str("ticker is empty"),
            Self::TickerTooLong => write!(f, "ticker is longer than {MAX_TICKER_LEN} characters"),
            Self::InvalidTickerChar(c) => write!(f, "ticker contains invalid character {c:?}"),
            Self::InvalidExchange(code) => write!(f, "invalid exchange code {code:?}"),
        }
    }
}

impl std::error::Error for SymbolError {}

/// An exchange identifier, such as an ISO 10383 MIC (`XNYS`, `XLON`, `XTKS`).
///
/// Stored in upper case.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExchangeCode(String);

impl ExchangeCode {
    /// Parses and normalizes an exchange code (upper case, `A-Z0-9`, 1–8 characters).
    ///
    /// # Errors
    ///
    /// Returns [`SymbolError::InvalidExchange`] if the code is empty, too long
    /// or contains other characters.
    pub fn new(code: &str) -> Result<Self, SymbolError> {
        let upper = code.trim().to_ascii_uppercase();
        let valid = !upper.is_empty()
            && upper.len() <= MAX_EXCHANGE_LEN
            && upper
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        if valid {
            Ok(Self(upper))
        } else {
            Err(SymbolError::InvalidExchange(code.to_owned()))
        }
    }

    /// The code as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExchangeCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A tradable instrument: a ticker, optionally qualified by an exchange.
///
/// ```
/// use stocktape_core::Symbol;
///
/// let vod: Symbol = "xlon:vod".parse().unwrap();
/// assert_eq!(vod.to_string(), "XLON:VOD");
/// assert_eq!(vod.ticker(), "VOD");
/// assert_eq!(vod.exchange().unwrap().as_str(), "XLON");
///
/// let brk: Symbol = "BRK.B".parse().unwrap();
/// assert!(brk.exchange().is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Symbol {
    exchange: Option<ExchangeCode>,
    ticker: String,
}

impl Symbol {
    /// Creates a symbol from a ticker and an optional exchange.
    ///
    /// The ticker is trimmed and upper-cased.
    ///
    /// # Errors
    ///
    /// Returns an error if the ticker is empty, too long or contains invalid
    /// characters.
    pub fn new(ticker: &str, exchange: Option<ExchangeCode>) -> Result<Self, SymbolError> {
        let ticker = ticker.trim().to_ascii_uppercase();
        if ticker.is_empty() {
            return Err(SymbolError::EmptyTicker);
        }
        if ticker.len() > MAX_TICKER_LEN {
            return Err(SymbolError::TickerTooLong);
        }
        if let Some(c) = ticker
            .chars()
            .find(|c| !(c.is_ascii_uppercase() || c.is_ascii_digit() || ".-_^=/".contains(*c)))
        {
            return Err(SymbolError::InvalidTickerChar(c));
        }
        Ok(Self { exchange, ticker })
    }

    /// The ticker, upper case.
    #[must_use]
    pub fn ticker(&self) -> &str {
        &self.ticker
    }

    /// The exchange, if the symbol is qualified with one.
    #[must_use]
    pub fn exchange(&self) -> Option<&ExchangeCode> {
        self.exchange.as_ref()
    }
}

impl FromStr for Symbol {
    type Err = SymbolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split_once(':') {
            Some((exchange, ticker)) => Self::new(ticker, Some(ExchangeCode::new(exchange)?)),
            None => Self::new(s, None),
        }
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.exchange {
            Some(exchange) => write!(f, "{exchange}:{}", self.ticker),
            None => f.write_str(&self.ticker),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn parses_plain_ticker() {
        let s: Symbol = " aapl ".parse().unwrap();
        assert_eq!(s.ticker(), "AAPL");
        assert!(s.exchange().is_none());
        assert_eq!(s.to_string(), "AAPL");
    }

    #[test]
    fn parses_exchange_qualified() {
        let s: Symbol = "XTKS:7203".parse().unwrap();
        assert_eq!(s.ticker(), "7203");
        assert_eq!(s.exchange().map(ExchangeCode::as_str), Some("XTKS"));
    }

    #[test]
    fn keeps_dots_and_index_carets() {
        assert_eq!("brk.b".parse::<Symbol>().unwrap().ticker(), "BRK.B");
        assert_eq!("^GSPC".parse::<Symbol>().unwrap().ticker(), "^GSPC");
        assert_eq!("BTC-USD".parse::<Symbol>().unwrap().ticker(), "BTC-USD");
        assert_eq!("EUR/USD".parse::<Symbol>().unwrap().ticker(), "EUR/USD");
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!("".parse::<Symbol>(), Err(SymbolError::EmptyTicker));
        assert_eq!("XLON:".parse::<Symbol>(), Err(SymbolError::EmptyTicker));
        assert!(matches!(
            ":VOD".parse::<Symbol>(),
            Err(SymbolError::InvalidExchange(_))
        ));
        assert_eq!(
            "AA PL".parse::<Symbol>(),
            Err(SymbolError::InvalidTickerChar(' '))
        );
        assert!(matches!(
            "X-LON:VOD".parse::<Symbol>(),
            Err(SymbolError::InvalidExchange(_))
        ));
        assert_eq!(
            "A".repeat(33).parse::<Symbol>(),
            Err(SymbolError::TickerTooLong)
        );
    }

    #[test]
    fn ordering_is_stable() {
        let mut v: Vec<Symbol> = ["MSFT", "AAPL", "XLON:VOD"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        v.sort();
        let names: Vec<String> = v.iter().map(ToString::to_string).collect();
        assert_eq!(names, ["AAPL", "MSFT", "XLON:VOD"]);
    }

    proptest! {
        #[test]
        fn display_round_trips(
            exchange in proptest::option::of("[A-Z0-9]{1,8}"),
            ticker in "[A-Z0-9][A-Z0-9.^=/_-]{0,31}",
        ) {
            let exchange = exchange.map(|e| ExchangeCode::new(&e).unwrap());
            let symbol = Symbol::new(&ticker, exchange).unwrap();
            let parsed: Symbol = symbol.to_string().parse().unwrap();
            prop_assert_eq!(parsed, symbol);
        }
    }
}
