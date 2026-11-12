//! Core types for stocktape: quotes, the quote store, symbols and clocks.
//!
//! Everything in this crate is pure: no I/O, no threads, and no hidden calls to
//! the system clock. Time always comes from a [`Clock`], so tests can use a
//! [`ManualClock`] and get the same result every run.
//!
//! Part of [stocktape](https://github.com/zcsizmadia/stocktape). See
//! `docs/design-notes.md` for the design.

mod clock;
pub mod format;
pub mod headline;
mod quote;
mod store;
mod symbol;

pub use clock::{Clock, ManualClock, SystemClock};
pub use headline::Headline;
pub use quote::{Currency, CurrencyError, Direction, Quote};
pub use store::{QuoteStore, StoreEntry};
pub use symbol::{ExchangeCode, Symbol, SymbolError};

/// Re-exported so downstream crates use the same wall-clock types.
pub use jiff::{SignedDuration, Timestamp};
