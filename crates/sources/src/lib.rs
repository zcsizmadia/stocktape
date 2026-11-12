//! Market-data sources (feeds) for stocktape.
//!
//! - [`mock`]: deterministic sample prices for demos and tests.
//! - [`finnhub`]: real quotes from Finnhub (REST snapshots plus a WebSocket
//!   trade stream), on a background thread.
//! - [`massive`]: Massive (formerly Polygon.io), batched and rate-limited
//!   REST snapshots.
//! - [`limiter`]: request pacing for rate-limited APIs.
//! - [`news`]: headlines from RSS and Atom feeds.
//! - [`failover`]: decides which of several feeds a symbol's quotes come from.
//!
//! Live feeds run on their own threads and send [`FeedEvent`]s over a
//! channel, so the app needs no async runtime. Parsing is done by pure
//! functions, tested against recorded responses; CI never calls live APIs.
//! API keys are never logged, and are removed from error messages.
//!
//! Part of [stocktape](https://github.com/zcsizmadia/stocktape). See
//! `docs/design-notes.md` for the design.

pub mod failover;
pub mod finnhub;
pub mod limiter;
pub mod massive;
pub mod mock;
pub mod news;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use stocktape_core::{Headline, Quote};

/// Something a feed reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    /// A new or updated quote.
    Quote(Quote),
    /// The latest headlines from one news source (replacing earlier ones).
    Headlines {
        /// The source's name.
        source: String,
        /// Its headlines, newest feed order.
        items: Vec<Headline>,
    },
    /// The feed connected, disconnected or hit a problem; for the user.
    Status {
        /// Whether quotes are flowing.
        connected: bool,
        /// What happened, without secrets.
        message: String,
    },
}

/// An update tagged with the feed it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedEvent {
    /// Index of the feed (its priority: 0 is the primary).
    pub feed: usize,
    /// What it reported.
    pub update: Update,
}

/// A running feed thread. Dropping it asks the thread to stop and waits.
#[derive(Debug)]
pub struct FeedHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FeedHandle {
    pub(crate) fn new(stop: Arc<AtomicBool>, thread: JoinHandle<()>) -> Self {
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for FeedHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Removes every occurrence of `secret` from `text`, e.g. an API key that a
/// library put into an error message along with a URL.
#[must_use]
pub fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_owned()
    } else {
        text.replace(secret, "****")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secrets() {
        assert_eq!(
            redact("GET https://x/quote?token=abc123 failed", "abc123"),
            "GET https://x/quote?token=**** failed"
        );
        assert_eq!(redact("nothing here", ""), "nothing here");
    }
}
