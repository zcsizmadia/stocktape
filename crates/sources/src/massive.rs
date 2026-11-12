//! Massive (massive.com, formerly Polygon.io): batched REST snapshots.
//!
//! Massive's plans differ a lot, and its free plan allows only about five
//! requests a minute, so this feed batches and paces every request:
//!
//! - **Snapshot mode** (Stocks Starter and up): one "full market snapshot"
//!   request with `tickers=AAPL,MSFT,...` returns the last trade, today's
//!   change and the previous close for the whole watchlist (split into chunks
//!   for long lists). Starter and Developer data is 15 minutes delayed.
//! - **End-of-day mode** (Stocks Basic, free): the snapshot endpoint isn't
//!   included, so after a "not entitled" answer the feed switches to the
//!   previous-day bar, one ticker per request, paced by the rate limit.
//!
//! Every request goes through a [`RateLimiter`]. The key is sent in the
//! `Authorization` header, never in URLs, and never appears in messages.
//! WebSocket streaming needs the Developer or Advanced plan and isn't
//! implemented yet. Only US stocks are supported for now.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use serde::Deserialize;
use stocktape_core::{Currency, Quote, Symbol, Timestamp};

use crate::limiter::RateLimiter;
use crate::{FeedEvent, FeedHandle, Update, redact};

/// Settings for the Massive feed.
#[derive(Debug, Clone)]
pub struct MassiveConfig {
    /// Your API key.
    pub api_key: String,
    /// Symbols to follow, each with Massive's ticker.
    pub symbols: Vec<(Symbol, String)>,
    /// REST base URL.
    pub rest_url: String,
    /// Time between rounds of snapshots.
    pub refresh: Duration,
    /// Your plan's request limit.
    pub requests_per_minute: u32,
}

impl MassiveConfig {
    /// The public endpoint with free-plan limits: 5 requests a minute and a
    /// round every 60 s.
    #[must_use]
    pub fn new(api_key: String, symbols: Vec<(Symbol, String)>) -> Self {
        Self {
            api_key,
            symbols,
            rest_url: "https://api.massive.com".to_owned(),
            refresh: Duration::from_secs(60),
            requests_per_minute: 5,
        }
    }
}

/// Most tickers per snapshot request, to keep URLs reasonable.
pub const CHUNK: usize = 50;

/// A ticker's latest prices from a snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct TickerSnapshot {
    /// Massive's ticker.
    pub ticker: String,
    /// Last trade price (or today's close so far).
    pub last: f64,
    /// Previous day's close, if given.
    pub previous_close: Option<f64>,
    /// When the price was observed.
    pub time: Timestamp,
}

/// Interprets a Unix time whose unit isn't documented: seconds, milliseconds,
/// microseconds or nanoseconds, judged by its size. Massive's samples show
/// 19-digit (nanosecond) values in snapshots and milliseconds elsewhere.
#[must_use]
pub fn timestamp_any(value: i64) -> Option<Timestamp> {
    let v = i128::from(value);
    if v <= 0 {
        return None;
    }
    let nanos = if v > 100_000_000_000_000_000 {
        v
    } else if v > 100_000_000_000_000 {
        v * 1_000
    } else if v > 100_000_000_000 {
        v * 1_000_000
    } else {
        v * 1_000_000_000
    };
    Timestamp::from_nanosecond(nanos).ok()
}

#[derive(Deserialize)]
struct Bar {
    c: Option<f64>,
}

#[derive(Deserialize)]
struct LastTrade {
    p: Option<f64>,
    t: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTicker {
    ticker: String,
    day: Option<Bar>,
    prev_day: Option<Bar>,
    last_trade: Option<LastTrade>,
    updated: Option<i64>,
}

#[derive(Deserialize)]
struct SnapshotResponse {
    #[serde(default)]
    tickers: Vec<RawTicker>,
}

/// Parses a full-market (multi-ticker) snapshot. Tickers without a usable
/// price or time are skipped.
///
/// # Errors
///
/// Returns a message for malformed JSON.
pub fn parse_snapshot(body: &str) -> Result<Vec<TickerSnapshot>, String> {
    let response: SnapshotResponse =
        serde_json::from_str(body).map_err(|e| format!("bad snapshot response: {e}"))?;
    Ok(response
        .tickers
        .into_iter()
        .filter_map(|t| {
            let trade_price = t.last_trade.as_ref().and_then(|l| l.p);
            let day_close = t.day.as_ref().and_then(|d| d.c);
            let last = trade_price
                .or(day_close)
                .filter(|p| p.is_finite() && *p > 0.0)?;
            let time = t
                .last_trade
                .as_ref()
                .and_then(|l| l.t)
                .or(t.updated)
                .and_then(timestamp_any)?;
            let previous_close = t
                .prev_day
                .and_then(|d| d.c)
                .filter(|p| p.is_finite() && *p > 0.0);
            Some(TickerSnapshot {
                ticker: t.ticker,
                last,
                previous_close,
                time,
            })
        })
        .collect())
}

#[derive(Deserialize)]
struct PrevResult {
    c: Option<f64>,
    t: Option<i64>,
}

#[derive(Deserialize)]
struct PrevResponse {
    #[serde(default)]
    results: Vec<PrevResult>,
}

/// Parses a previous-day bar: `(close, time)`, or `None` without data.
///
/// # Errors
///
/// Returns a message for malformed JSON.
pub fn parse_previous_day(body: &str) -> Result<Option<(f64, Timestamp)>, String> {
    let response: PrevResponse =
        serde_json::from_str(body).map_err(|e| format!("bad previous-day response: {e}"))?;
    Ok(response.results.first().and_then(|r| {
        let close = r.c.filter(|c| c.is_finite() && *c > 0.0)?;
        Some((close, timestamp_any(r.t?)?))
    }))
}

/// The snapshot URL for a batch of tickers.
#[must_use]
pub fn snapshot_url(base: &str, tickers: &[&str]) -> String {
    format!(
        "{base}/v2/snapshot/locale/us/markets/stocks/tickers?tickers={}",
        tickers.join(",")
    )
}

/// Starts the feed on a background thread.
#[must_use]
pub fn spawn(config: MassiveConfig, feed: usize, sink: Sender<FeedEvent>) -> FeedHandle {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let stop = Arc::new(AtomicBool::new(false));
    let worker = Worker {
        limiter: RateLimiter::per_minute(config.requests_per_minute),
        config,
        feed,
        sink,
        stop: stop.clone(),
        started: Instant::now(),
        end_of_day: false,
    };
    let thread = std::thread::Builder::new()
        .name("massive".to_owned())
        .spawn(move || worker.run())
        .expect("can spawn the feed thread");
    FeedHandle::new(stop, thread)
}

enum Fetch {
    Body(String),
    /// 401: the key is wrong.
    BadKey,
    /// 403: the plan doesn't include this endpoint.
    NotEntitled,
    /// 429: over the limit; back off.
    TooMany,
    Failed(String),
}

struct Worker {
    config: MassiveConfig,
    limiter: RateLimiter,
    feed: usize,
    sink: Sender<FeedEvent>,
    stop: Arc<AtomicBool>,
    started: Instant,
    end_of_day: bool,
}

impl Worker {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn send(&self, update: Update) -> bool {
        self.sink
            .send(FeedEvent {
                feed: self.feed,
                update,
            })
            .is_ok()
    }

    fn status(&self, connected: bool, message: &str) -> bool {
        self.send(Update::Status {
            connected,
            message: format!("massive: {}", redact(message, &self.config.api_key)),
        })
    }

    fn sleep(&self, total: Duration) {
        let end = Instant::now() + total;
        while !self.stopped() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(100).min(end - Instant::now()));
        }
    }

    /// One request, paced by the rate limiter.
    fn get(&mut self, url: &str) -> Option<Fetch> {
        let wait = self.limiter.wait(self.started.elapsed());
        self.sleep(wait);
        if self.stopped() {
            return None;
        }
        self.limiter.take(self.started.elapsed());
        let response = ureq::get(url)
            .header("Authorization", &format!("Bearer {}", self.config.api_key))
            .call();
        Some(match response {
            Ok(mut r) => match r.body_mut().read_to_string() {
                Ok(body) => Fetch::Body(body),
                Err(e) => Fetch::Failed(e.to_string()),
            },
            Err(ureq::Error::StatusCode(401)) => Fetch::BadKey,
            Err(ureq::Error::StatusCode(403)) => Fetch::NotEntitled,
            Err(ureq::Error::StatusCode(429)) => Fetch::TooMany,
            Err(e) => Fetch::Failed(e.to_string()),
        })
    }

    fn quote(symbol: &Symbol, last: f64, previous_close: Option<f64>, time: Timestamp) -> Quote {
        let mut q = Quote::new(symbol.clone(), last, time).with_currency(Currency::USD);
        if let Some(pc) = previous_close {
            q = q.with_previous_close(pc);
        }
        q
    }

    /// Handles a failed request; returns false when the feed should stop.
    fn failure(&mut self, fetch: Fetch, what: &str) -> bool {
        match fetch {
            Fetch::BadKey => {
                self.status(false, "the API key was rejected (HTTP 401); check it and restart. This feed is stopped.");
                false
            }
            Fetch::TooMany => {
                self.status(
                    false,
                    &format!("{what}: over the plan's request limit; slowing down"),
                );
                self.sleep(Duration::from_secs(60));
                !self.stopped()
            }
            Fetch::Failed(e) => self.status(false, &format!("{what} failed: {e}")),
            Fetch::Body(_) | Fetch::NotEntitled => true,
        }
    }

    /// One round of batched snapshots. Returns false to stop.
    fn snapshot_round(&mut self) -> bool {
        let symbols = self.config.symbols.clone();
        for chunk in symbols.chunks(CHUNK) {
            let names: Vec<&str> = chunk.iter().map(|(_, n)| n.as_str()).collect();
            let url = snapshot_url(&self.config.rest_url, &names);
            match self.get(&url) {
                None => return false,
                Some(Fetch::Body(body)) => match parse_snapshot(&body) {
                    Ok(snapshots) => {
                        for s in snapshots {
                            if let Some((symbol, _)) = chunk.iter().find(|(_, n)| *n == s.ticker) {
                                let q = Self::quote(symbol, s.last, s.previous_close, s.time);
                                if !self.send(Update::Quote(q)) {
                                    return false;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if !self.status(true, &e) {
                            return false;
                        }
                    }
                },
                Some(Fetch::NotEntitled) => {
                    self.end_of_day = true;
                    return self.status(
                        true,
                        "this plan doesn't include snapshots (Stocks Starter or higher); \
                         showing end-of-day prices instead",
                    );
                }
                Some(other) => {
                    if !self.failure(other, "snapshot") {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// One round of previous-day bars, one ticker per request.
    fn end_of_day_round(&mut self) -> bool {
        let symbols = self.config.symbols.clone();
        for (symbol, name) in &symbols {
            let url = format!("{}/v2/aggs/ticker/{name}/prev", self.config.rest_url);
            match self.get(&url) {
                None => return false,
                Some(Fetch::Body(body)) => match parse_previous_day(&body) {
                    Ok(Some((close, time))) => {
                        if !self.send(Update::Quote(Self::quote(symbol, close, None, time))) {
                            return false;
                        }
                    }
                    Ok(None) => {
                        if !self.status(true, &format!("no data for {name} (check the symbol)")) {
                            return false;
                        }
                    }
                    Err(e) => {
                        if !self.status(true, &e) {
                            return false;
                        }
                    }
                },
                Some(Fetch::NotEntitled) => {
                    self.status(
                        false,
                        &format!("this plan doesn't include {name}'s previous-day bar"),
                    );
                }
                Some(other) => {
                    if !self.failure(other, &format!("previous day for {name}")) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn run(mut self) {
        if self.config.api_key.is_empty() {
            self.status(
                false,
                "no API key; set it in the configuration or environment",
            );
            return;
        }
        self.status(
            true,
            &format!(
                "polling every {} s, at most {} requests a minute",
                self.config.refresh.as_secs(),
                self.config.requests_per_minute
            ),
        );
        while !self.stopped() {
            let ok = if self.end_of_day {
                self.end_of_day_round()
            } else {
                let ok = self.snapshot_round();
                // Switched to end-of-day mid-round: fetch those prices now.
                if ok && self.end_of_day {
                    self.end_of_day_round()
                } else {
                    ok
                }
            };
            if !ok {
                return;
            }
            self.sleep(self.config.refresh);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // Shapes from Massive's documentation (abridged).
    const SNAPSHOT: &str = r#"{"count":2,"status":"OK","tickers":[
        {"ticker":"AAPL","todaysChange":2.17,"todaysChangePerc":0.918,"updated":1794497400000000000,
         "day":{"c":238.4,"h":239.1,"l":235.9,"o":236.5,"v":1000,"vw":237.2},
         "prevDay":{"c":236.24,"h":237,"l":234,"o":235,"v":2000,"vw":236},
         "lastTrade":{"p":238.41,"s":100,"t":1794497400123456789,"x":4}},
        {"ticker":"MSFT","updated":1794497400000000000,"day":{"c":512.08},"prevDay":{"c":515.52}},
        {"ticker":"NODATA","day":{"c":0},"prevDay":{"c":10}}
    ]}"#;
    const PREV: &str = r#"{"adjusted":true,"queryCount":1,"request_id":"x","resultsCount":1,"status":"OK","ticker":"AAPL",
        "results":[{"T":"AAPL","c":236.24,"h":237,"l":234,"n":1,"o":235,"t":1794441600000,"v":2000,"vw":236}]}"#;

    #[test]
    fn snapshots() {
        let s = parse_snapshot(SNAPSHOT).unwrap();
        assert_eq!(s.len(), 2, "NODATA is skipped");
        assert_eq!(s[0].ticker, "AAPL");
        assert_eq!(s[0].last, 238.41);
        assert_eq!(s[0].previous_close, Some(236.24));
        assert_eq!(
            s[0].time,
            Timestamp::from_nanosecond(1_794_497_400_123_456_789).unwrap()
        );
        assert_eq!(s[1].last, 512.08, "falls back to today's close");
        assert_eq!(s[1].time, Timestamp::from_second(1_794_497_400).unwrap());
        assert_eq!(parse_snapshot(r#"{"status":"OK"}"#).unwrap(), []);
        assert!(parse_snapshot("<html>").is_err());
    }

    #[test]
    fn previous_day() {
        assert_eq!(
            parse_previous_day(PREV).unwrap(),
            Some((
                236.24,
                Timestamp::from_millisecond(1_794_441_600_000).unwrap()
            ))
        );
        assert_eq!(parse_previous_day(r#"{"results":[]}"#).unwrap(), None);
        assert_eq!(parse_previous_day(r#"{"status":"OK"}"#).unwrap(), None);
    }

    #[test]
    fn timestamps_in_any_unit() {
        let t = Timestamp::from_second(1_794_497_400).unwrap();
        assert_eq!(timestamp_any(1_794_497_400), Some(t));
        assert_eq!(timestamp_any(1_794_497_400_000), Some(t));
        assert_eq!(timestamp_any(1_794_497_400_000_000), Some(t));
        assert_eq!(timestamp_any(1_794_497_400_000_000_000), Some(t));
        assert_eq!(timestamp_any(0), None);
        assert_eq!(timestamp_any(-5), None);
    }

    #[test]
    fn urls_never_hold_the_key() {
        let url = snapshot_url("https://api.massive.com", &["AAPL", "MSFT"]);
        assert_eq!(
            url,
            "https://api.massive.com/v2/snapshot/locale/us/markets/stocks/tickers?tickers=AAPL,MSFT"
        );
        assert!(!url.contains("apiKey"));
    }

    proptest! {
        #[test]
        fn parsers_never_panic(text in ".{0,300}") {
            let _ = parse_snapshot(&text);
            let _ = parse_previous_day(&text);
        }

        #[test]
        fn parsers_never_panic_on_json_shaped_input(p in any::<f64>(), t in any::<i64>()) {
            let snap = format!(r#"{{"tickers":[{{"ticker":"A","lastTrade":{{"p":{p},"t":{t}}},"prevDay":{{"c":{p}}},"updated":{t}}}]}}"#);
            let _ = parse_snapshot(&snap);
            let prev = format!(r#"{{"results":[{{"c":{p},"t":{t}}}]}}"#);
            let _ = parse_previous_day(&prev);
            let _ = timestamp_any(t);
        }
    }
}
