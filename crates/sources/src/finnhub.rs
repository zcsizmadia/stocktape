//! Finnhub: REST snapshots plus a WebSocket trade stream.
//!
//! - At start, and every few minutes after, each symbol's REST `quote` gives
//!   the last price and previous close (the reference for change and percent).
//! - The WebSocket streams trades; each trade updates the last price.
//! - On a dropped connection the thread backs off (1 s, 2 s, 4 s ... 60 s)
//!   and reconnects.
//!
//! Free Finnhub accounts allow 60 REST calls a minute and 50 WebSocket
//! symbols; snapshots are spaced to stay under the limit. You need your own
//! API key; it is never logged, and is removed from error messages.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use serde::Deserialize;
use stocktape_core::{Currency, Quote, Symbol, Timestamp};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::{FeedEvent, FeedHandle, Update, redact};

/// Settings for the Finnhub feed.
#[derive(Debug, Clone)]
pub struct FinnhubConfig {
    /// Your API key.
    pub api_key: String,
    /// Symbols to follow, each with Finnhub's name for it.
    pub symbols: Vec<(Symbol, String)>,
    /// REST base URL.
    pub rest_url: String,
    /// WebSocket URL.
    pub ws_url: String,
    /// How often to refresh REST snapshots.
    pub snapshot_every: Duration,
    /// Stream live trades over the WebSocket. Without it, prices update
    /// only from snapshots, every `snapshot_every`.
    pub stream: bool,
}

impl FinnhubConfig {
    /// The public Finnhub endpoints.
    #[must_use]
    pub fn new(api_key: String, symbols: Vec<(Symbol, String)>) -> Self {
        Self {
            api_key,
            symbols,
            rest_url: "https://finnhub.io/api/v1".to_owned(),
            ws_url: "wss://ws.finnhub.io".to_owned(),
            snapshot_every: Duration::from_secs(300),
            stream: true,
        }
    }
}

/// Finnhub's name for a symbol: crypto pairs like `BTC-USD` map to Binance's
/// `BINANCE:BTCUSDT`, everything else is the ticker. Override per symbol in
/// the configuration when this guess is wrong.
#[must_use]
pub fn default_name(symbol: &Symbol) -> String {
    let ticker = symbol.ticker();
    match ticker.strip_suffix("-USD") {
        Some(base) if symbol.exchange().is_none() => format!("BINANCE:{base}USDT"),
        _ => ticker.to_owned(),
    }
}

/// A REST quote snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapshot {
    /// Last price.
    pub last: f64,
    /// Previous session's close.
    pub previous_close: f64,
    /// When the price was observed.
    pub time: Timestamp,
}

#[derive(Deserialize)]
struct RestQuote {
    c: Option<f64>,
    pc: Option<f64>,
    t: Option<i64>,
}

/// Parses a REST `quote` response. `Ok(None)` means Finnhub has no data for
/// the symbol (it answers unknown symbols with zeros).
///
/// # Errors
///
/// Returns a message for malformed JSON.
pub fn parse_snapshot(body: &str) -> Result<Option<Snapshot>, String> {
    let q: RestQuote =
        serde_json::from_str(body).map_err(|e| format!("bad quote response: {e}"))?;
    let (Some(last), Some(previous_close), Some(t)) = (q.c, q.pc, q.t) else {
        return Ok(None);
    };
    if t <= 0 || !last.is_finite() || last == 0.0 {
        return Ok(None);
    }
    let time = Timestamp::from_second(t).map_err(|e| format!("bad quote time: {e}"))?;
    Ok(Some(Snapshot {
        last,
        previous_close,
        time,
    }))
}

/// One trade from the WebSocket.
#[derive(Debug, Clone, PartialEq)]
pub struct Trade {
    /// Finnhub's symbol name.
    pub symbol: String,
    /// Trade price.
    pub price: f64,
    /// Trade time.
    pub time: Timestamp,
}

/// A WebSocket message.
#[derive(Debug, Clone, PartialEq)]
pub enum WsMessage {
    /// One or more trades.
    Trades(Vec<Trade>),
    /// Keep-alive.
    Ping,
    /// An error report, e.g. an invalid key or symbol.
    Error(String),
    /// Anything else (news, unknown types); ignored.
    Other,
}

#[derive(Deserialize)]
struct RawTrade {
    s: String,
    p: f64,
    t: i64,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum RawMessage {
    Trade {
        data: Vec<RawTrade>,
    },
    Ping,
    Error {
        msg: String,
    },
    #[serde(other)]
    Other,
}

/// Parses a WebSocket text message. Trades with bad prices or times are
/// dropped.
///
/// # Errors
///
/// Returns a message for malformed JSON.
pub fn parse_ws(text: &str) -> Result<WsMessage, String> {
    let raw: RawMessage = serde_json::from_str(text).map_err(|e| format!("bad message: {e}"))?;
    Ok(match raw {
        RawMessage::Trade { data } => WsMessage::Trades(
            data.into_iter()
                .filter(|t| t.p.is_finite() && t.p > 0.0)
                .filter_map(|t| {
                    Some(Trade {
                        time: Timestamp::from_millisecond(t.t).ok()?,
                        symbol: t.s,
                        price: t.p,
                    })
                })
                .collect(),
        ),
        RawMessage::Ping => WsMessage::Ping,
        RawMessage::Error { msg } => WsMessage::Error(msg),
        RawMessage::Other => WsMessage::Other,
    })
}

/// The WebSocket URL with the token. A URL without a path gets `/`, because
/// `wss://host?token=...` would send the request line `GET ?token=...`,
/// which servers reject with 400.
#[must_use]
pub fn ws_request_url(base: &str, token: &str) -> String {
    let after_scheme = base.split_once("://").map_or(base, |(_, rest)| rest);
    let slash = if after_scheme.contains('/') { "" } else { "/" };
    format!("{base}{slash}?token={token}")
}

/// The subscribe request for one symbol.
#[must_use]
pub fn subscribe_message(name: &str) -> String {
    serde_json::json!({ "type": "subscribe", "symbol": name }).to_string()
}

/// Starts the feed on a background thread. Quotes and status go to `sink`
/// tagged with `feed` (its priority).
#[must_use]
pub fn spawn(config: FinnhubConfig, feed: usize, sink: Sender<FeedEvent>) -> FeedHandle {
    // Our TLS uses the `ring` provider; installing it twice is harmless.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let stop = Arc::new(AtomicBool::new(false));
    let worker = Worker {
        by_name: config
            .symbols
            .iter()
            .map(|(s, name)| (name.clone(), s.clone()))
            .collect(),
        previous_close: HashMap::new(),
        config,
        feed,
        sink,
        stop: stop.clone(),
    };
    let thread = std::thread::Builder::new()
        .name("finnhub".to_owned())
        .spawn(move || worker.run())
        .expect("can spawn the feed thread");
    FeedHandle::new(stop, thread)
}

/// How long to wait for a WebSocket message before checking for stop and
/// snapshot refreshes.
const READ_TIMEOUT: Duration = Duration::from_secs(1);
/// Spacing between REST calls (free tier: 60 per minute).
const REST_SPACING: Duration = Duration::from_millis(1_100);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

struct Worker {
    config: FinnhubConfig,
    by_name: HashMap<String, Symbol>,
    previous_close: HashMap<Symbol, f64>,
    feed: usize,
    sink: Sender<FeedEvent>,
    stop: Arc<AtomicBool>,
}

impl Worker {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Sends an update; false if the app has gone away.
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
            message: format!("finnhub: {}", redact(message, &self.config.api_key)),
        })
    }

    /// Sleeps in short steps so a stop request is noticed quickly.
    fn sleep(&self, total: Duration) {
        let end = Instant::now() + total;
        while !self.stopped() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(100).min(end - Instant::now()));
        }
    }

    fn quote(&self, symbol: &Symbol, last: f64, time: Timestamp) -> Quote {
        let mut q = Quote::new(symbol.clone(), last, time);
        if let Some(pc) = self.previous_close.get(symbol).filter(|p| **p > 0.0) {
            q = q.with_previous_close(*pc);
        }
        if symbol.exchange().is_none() {
            q = q.with_currency(Currency::USD);
        }
        q
    }

    /// Fetches REST snapshots for every symbol.
    fn snapshots(&mut self) -> bool {
        let symbols = self.config.symbols.clone();
        for (symbol, name) in &symbols {
            if self.stopped() {
                return false;
            }
            let url = format!(
                "{}/quote?symbol={name}&token={}",
                self.config.rest_url, self.config.api_key
            );
            let response = ureq::get(&url).call();
            if let Err(ureq::Error::StatusCode(code @ (401 | 403))) = response {
                // A rejected key won't fix itself; retrying would only hammer
                // the API. Say so once and stop this feed.
                self.status(
                    false,
                    &format!(
                        "the API key was rejected (HTTP {code}); check it and restart. \
                         This feed is stopped."
                    ),
                );
                self.stop.store(true, Ordering::Relaxed);
                return false;
            }
            let result = response
                .map_err(|e| e.to_string())
                .and_then(|mut r| r.body_mut().read_to_string().map_err(|e| e.to_string()))
                .and_then(|body| parse_snapshot(&body));
            match result {
                Ok(Some(s)) => {
                    self.previous_close.insert(symbol.clone(), s.previous_close);
                    let q = self.quote(symbol, s.last, s.time);
                    if !self.send(Update::Quote(q)) {
                        return false;
                    }
                }
                Ok(None) => {
                    if !self.status(true, &format!("no data for {name} (check the symbol)")) {
                        return false;
                    }
                }
                Err(e) => {
                    if !self.status(false, &format!("snapshot for {name} failed: {e}")) {
                        return false;
                    }
                }
            }
            self.sleep(REST_SPACING);
        }
        true
    }

    fn connect(&self) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
        let url = ws_request_url(&self.config.ws_url, &self.config.api_key);
        let (mut ws, _) = tungstenite::connect(url.as_str()).map_err(|e| e.to_string())?;
        let timeout = Some(READ_TIMEOUT);
        let set = match ws.get_mut() {
            MaybeTlsStream::Plain(s) => s.set_read_timeout(timeout),
            MaybeTlsStream::Rustls(s) => s.get_mut().set_read_timeout(timeout),
            _ => Ok(()),
        };
        set.map_err(|e| e.to_string())?;
        for (_, name) in &self.config.symbols {
            ws.send(Message::text(subscribe_message(name)))
                .map_err(|e| e.to_string())?;
        }
        Ok(ws)
    }

    /// Streams trades until the connection drops (returns the reason) or
    /// the feed is stopped (returns `None`).
    fn stream(&mut self, ws: &mut WebSocket<MaybeTlsStream<TcpStream>>) -> Option<String> {
        let mut next_snapshot = Instant::now() + self.config.snapshot_every;
        loop {
            if self.stopped() {
                let _ = ws.close(None);
                return None;
            }
            if Instant::now() >= next_snapshot {
                if !self.snapshots() {
                    return None;
                }
                next_snapshot = Instant::now() + self.config.snapshot_every;
            }
            match ws.read() {
                Ok(Message::Text(text)) => match parse_ws(text.as_str()) {
                    Ok(WsMessage::Trades(trades)) => {
                        // Only the latest trade per symbol in a batch matters.
                        let mut latest: HashMap<&str, &Trade> = HashMap::new();
                        for t in &trades {
                            latest
                                .entry(t.symbol.as_str())
                                .and_modify(|cur| {
                                    if t.time >= cur.time {
                                        *cur = t;
                                    }
                                })
                                .or_insert(t);
                        }
                        for (name, t) in latest {
                            if let Some(symbol) = self.by_name.get(name) {
                                let q = self.quote(symbol, t.price, t.time);
                                if !self.send(Update::Quote(q)) {
                                    return None;
                                }
                            }
                        }
                    }
                    Ok(WsMessage::Error(msg)) => {
                        if !self.status(true, &format!("server says: {msg}")) {
                            return None;
                        }
                    }
                    Ok(WsMessage::Ping | WsMessage::Other) => {}
                    Err(e) => {
                        if !self.status(true, &e) {
                            return None;
                        }
                    }
                },
                Ok(Message::Close(_)) => return Some("server closed the connection".to_owned()),
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(e) => return Some(e.to_string()),
            }
        }
    }

    fn run(mut self) {
        if self.config.api_key.is_empty() {
            self.status(
                false,
                "no API key; set it in the configuration or environment",
            );
            return;
        }
        if !self.config.stream {
            // Polling only: a full round of snapshots every `snapshot_every`.
            self.status(
                true,
                &format!(
                    "polling every {} s (no live stream)",
                    self.config.snapshot_every.as_secs()
                ),
            );
            while !self.stopped() && self.snapshots() {
                self.sleep(self.config.snapshot_every);
            }
            return;
        }
        let mut backoff = Duration::from_secs(1);
        while !self.stopped() {
            if !self.snapshots() {
                return;
            }
            match self.connect() {
                Ok(mut ws) => {
                    backoff = Duration::from_secs(1);
                    if !self.status(true, "connected") {
                        return;
                    }
                    match self.stream(&mut ws) {
                        None => return,
                        Some(reason) => {
                            if !self.status(false, &format!("disconnected: {reason}")) {
                                return;
                            }
                        }
                    }
                }
                Err(e) => {
                    if !self.status(false, &format!("can't connect: {e}")) {
                        return;
                    }
                }
            }
            if !self.status(false, &format!("retrying in {} s", backoff.as_secs())) {
                return;
            }
            self.sleep(backoff);
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // Recorded from Finnhub's documented response shapes.
    const QUOTE: &str = r#"{"c":238.41,"d":2.17,"dp":0.9186,"h":239.1,"l":235.9,"o":236.5,"pc":236.24,"t":1794497400}"#;
    const UNKNOWN: &str = r#"{"c":0,"d":null,"dp":null,"h":0,"l":0,"o":0,"pc":0,"t":0}"#;
    const TRADES: &str = r#"{"data":[{"c":null,"p":238.42,"s":"AAPL","t":1794497401123,"v":100},{"c":["1","12"],"p":7296.89,"s":"BINANCE:BTCUSDT","t":1794497401456,"v":0.011}],"type":"trade"}"#;

    #[test]
    fn snapshot() {
        let s = parse_snapshot(QUOTE).unwrap().unwrap();
        assert_eq!(s.last, 238.41);
        assert_eq!(s.previous_close, 236.24);
        assert_eq!(s.time, Timestamp::from_second(1_794_497_400).unwrap());
        assert_eq!(parse_snapshot(UNKNOWN).unwrap(), None);
        assert!(parse_snapshot("<html>").is_err());
    }

    #[test]
    fn trades_and_other_messages() {
        let WsMessage::Trades(trades) = parse_ws(TRADES).unwrap() else {
            panic!("expected trades");
        };
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].symbol, "AAPL");
        assert_eq!(trades[1].price, 7296.89);
        assert_eq!(
            trades[1].time,
            Timestamp::from_millisecond(1_794_497_401_456).unwrap()
        );
        assert_eq!(parse_ws(r#"{"type":"ping"}"#).unwrap(), WsMessage::Ping);
        assert_eq!(
            parse_ws(r#"{"type":"error","msg":"Invalid API key"}"#).unwrap(),
            WsMessage::Error("Invalid API key".into())
        );
        assert_eq!(
            parse_ws(r#"{"type":"news","data":[]}"#).unwrap(),
            WsMessage::Other
        );
        assert!(parse_ws("not json").is_err());
    }

    #[test]
    fn bad_trades_are_dropped() {
        let msg =
            r#"{"type":"trade","data":[{"s":"A","p":-1,"t":1},{"s":"B","p":5,"t":1794497401000}]}"#;
        let WsMessage::Trades(trades) = parse_ws(msg).unwrap() else {
            panic!("expected trades");
        };
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].symbol, "B");
    }

    #[test]
    fn websocket_url_always_has_a_path() {
        assert_eq!(
            ws_request_url("wss://ws.finnhub.io", "k"),
            "wss://ws.finnhub.io/?token=k"
        );
        assert_eq!(
            ws_request_url("wss://ws.finnhub.io/", "k"),
            "wss://ws.finnhub.io/?token=k"
        );
        assert_eq!(
            ws_request_url("ws://localhost:9/feed", "k"),
            "ws://localhost:9/feed?token=k"
        );
    }

    #[test]
    fn names() {
        let s = |t: &str| -> Symbol { t.parse().unwrap() };
        assert_eq!(default_name(&s("AAPL")), "AAPL");
        assert_eq!(default_name(&s("BTC-USD")), "BINANCE:BTCUSDT");
        assert_eq!(default_name(&s("BRK.B")), "BRK.B");
        assert_eq!(
            subscribe_message("AAPL"),
            r#"{"symbol":"AAPL","type":"subscribe"}"#
        );
    }

    proptest! {
        /// Parsers never panic, whatever arrives on the wire.
        #[test]
        fn parsers_never_panic(text in ".{0,300}") {
            let _ = parse_ws(&text);
            let _ = parse_snapshot(&text);
        }

        #[test]
        fn parsers_never_panic_on_json_shaped_input(
            kind in prop_oneof![Just("trade"), Just("ping"), Just("error"), Just("x")],
            p in any::<f64>(),
            t in any::<i64>(),
        ) {
            let msg = format!(r#"{{"type":"{kind}","data":[{{"s":"A","p":{p},"t":{t}}}],"msg":"m"}}"#);
            let _ = parse_ws(&msg);
            let quote = format!(r#"{{"c":{p},"pc":{p},"t":{t}}}"#);
            let _ = parse_snapshot(&quote);
        }
    }
}
