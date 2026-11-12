//! The ticker without windows: a shared market (feed and quote store) and any
//! number of banners, each with its own strategy, skin and belt.
//!
//! Everything is driven by a [`Clock`], so frames can be produced in tests
//! with a [`ManualClock`](stocktape_core::ManualClock).

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use stocktape_core::format::format_fixed;
use stocktape_core::{Clock, Headline, QuoteStore, Symbol, Timestamp};
use stocktape_layout::{Belt, BeltConfig, BeltItem, Flow, Segment, Snap};
use stocktape_render::frame::compose;
use stocktape_render::{BannerStyle, QuoteSegment, SegmentImage, SegmentRenderer};
use stocktape_sources::failover::Arbiter;
use stocktape_sources::mock::MockFeed;
use stocktape_sources::{FeedEvent, FeedHandle, Update, finnhub, massive, news};
use stocktape_strategies::{
    Caps, Context, Group, Headlines, HighlightRules, Highlights, Item, Messages, Mix, Movers,
    Strategy, Watchlist,
};
use tiny_skia::Pixmap;

use stocktape_config::{Content, FeedConfig, Label, NewsConfig, Provider, StrategyKind};

/// How often the mock feed produces new prices.
const FEED_INTERVAL: Duration = Duration::from_secs(1);
/// Price points kept per symbol.
const HISTORY: usize = 120;
/// A backup feed takes over a symbol after the primary is quiet this long.
const FAILOVER_AFTER: Duration = Duration::from_secs(30);
/// Live quotes older than this are drawn faded. Snapshots refresh every
/// 5 minutes even without trades, so this only trips when a feed is down.
const STALE_AFTER: Duration = Duration::from_secs(15 * 60);

enum Source {
    Mock {
        feed: MockFeed,
        last: Option<Duration>,
    },
    Live {
        events: Receiver<FeedEvent>,
        arbiter: Arbiter,
        // Held so the feed threads keep running; dropping them stops them.
        _feeds: Vec<FeedHandle>,
        last_status: HashMap<usize, String>,
    },
}

/// The feeds and the latest quotes, shared by every banner.
pub struct Market {
    source: Source,
    store: QuoteStore,
    stale_after: Option<Duration>,
    news: Option<News>,
    headlines: Vec<Headline>,
    labels: HashMap<Symbol, Label>,
}

/// Running news feeds.
struct News {
    events: Receiver<FeedEvent>,
    // Held so the feed threads keep running; dropping them stops them.
    _feeds: Vec<FeedHandle>,
    by_source: HashMap<String, Vec<Headline>>,
    last_status: HashMap<usize, String>,
}

/// Headlines older than this are dropped.
const HEADLINE_MAX_AGE: Duration = Duration::from_secs(24 * 3600);
/// Most headlines kept across all sources.
const HEADLINE_MAX: usize = 100;

/// Merges every source's headlines: newest first (undated ones after dated
/// ones), duplicates removed, old ones dropped, at most `HEADLINE_MAX`.
fn merge_headlines(by_source: &HashMap<String, Vec<Headline>>, now: Timestamp) -> Vec<Headline> {
    let cutoff = now
        .checked_sub(jiff::SignedDuration::try_from(HEADLINE_MAX_AGE).unwrap_or_default())
        .ok();
    let mut all: Vec<Headline> = by_source
        .values()
        .flatten()
        .filter(|h| match (h.published, cutoff) {
            (Some(p), Some(c)) => p >= c,
            _ => true,
        })
        .cloned()
        .collect();
    all.sort_by(|a, b| {
        b.published
            .cmp(&a.published)
            .then_with(|| a.title.cmp(&b.title))
    });
    let mut seen = HashSet::new();
    all.retain(|h| seen.insert(h.title.to_lowercase()));
    all.truncate(HEADLINE_MAX);
    all
}

impl Market {
    /// The demo market: sample prices from the mock feed.
    pub fn demo(seed: u64) -> Self {
        Self {
            source: Source::Mock {
                feed: MockFeed::demo(seed),
                last: None,
            },
            store: QuoteStore::new(HISTORY),
            stale_after: None,
            news: None,
            headlines: Vec::new(),
            labels: HashMap::new(),
        }
    }

    /// Sets how particular symbols are shown (name, unit, decimals).
    #[must_use]
    pub fn with_labels(mut self, labels: &[(Symbol, Label)]) -> Self {
        self.labels = labels.iter().cloned().collect();
        self
    }

    /// Starts news feeds for headline lines.
    #[must_use]
    pub fn with_news(mut self, sources: &[NewsConfig]) -> Self {
        if sources.is_empty() {
            return self;
        }
        let (tx, events) = channel();
        let feeds = sources
            .iter()
            .enumerate()
            .map(|(i, n)| {
                news::spawn(
                    news::NewsConfig {
                        url: n.url.clone(),
                        name: n.name.clone(),
                        refresh: n.refresh,
                    },
                    i,
                    tx.clone(),
                )
            })
            .collect();
        self.news = Some(News {
            events,
            _feeds: feeds,
            by_source: HashMap::new(),
            last_status: HashMap::new(),
        });
        self
    }

    /// Adds headlines from `source` directly (for tests and demos).
    #[cfg(test)]
    pub fn set_headlines(&mut self, source: &str, items: Vec<Headline>, now: Timestamp) {
        let mut by_source = HashMap::new();
        by_source.insert(source.to_owned(), items);
        self.headlines = merge_headlines(&by_source, now);
    }

    /// Starts the configured feeds (primary first) for the watchlist's
    /// symbols; with no live feeds, the demo market.
    pub fn from_config(feeds: &[FeedConfig], groups: &[Group], seed: u64) -> Self {
        let live: Vec<&FeedConfig> = feeds
            .iter()
            .filter(|f| f.provider != Provider::Mock)
            .collect();
        if live.is_empty() {
            return Self::demo(seed);
        }
        let mut symbols: Vec<Symbol> = Vec::new();
        for s in groups.iter().flat_map(|g| &g.symbols) {
            if !symbols.contains(s) {
                symbols.push(s.clone());
            }
        }
        let (tx, events) = channel();
        let handles = live
            .iter()
            .enumerate()
            .map(|(index, feed)| {
                // Provider names: the config's overrides, else each
                // provider's default mapping.
                let names = |default: &dyn Fn(&Symbol) -> String| -> Vec<(Symbol, String)> {
                    symbols
                        .iter()
                        .map(|s| {
                            let name = feed
                                .names
                                .iter()
                                .find(|(sym, _)| sym == s)
                                .map_or_else(|| default(s), |(_, n)| n.clone());
                            (s.clone(), name)
                        })
                        .collect()
                };
                match feed.provider {
                    Provider::Finnhub => {
                        let names = names(&finnhub::default_name);
                        finnhub::spawn(
                            finnhub::FinnhubConfig {
                                snapshot_every: feed.refresh,
                                stream: feed.stream,
                                ..finnhub::FinnhubConfig::new(feed.api_key.clone(), names)
                            },
                            index,
                            tx.clone(),
                        )
                    }
                    Provider::Massive => massive::spawn(
                        massive::MassiveConfig {
                            refresh: feed.refresh,
                            requests_per_minute: feed.requests_per_minute,
                            ..massive::MassiveConfig::new(
                                feed.api_key.clone(),
                                names(&|s: &Symbol| s.ticker().to_owned()),
                            )
                        },
                        index,
                        tx.clone(),
                    ),
                    Provider::Mock => unreachable!("mock feeds are filtered out"),
                }
            })
            .collect();
        Self {
            source: Source::Live {
                events,
                arbiter: Arbiter::new(FAILOVER_AFTER),
                _feeds: handles,
                last_status: HashMap::new(),
            },
            store: QuoteStore::new(HISTORY),
            stale_after: Some(STALE_AFTER),
            news: None,
            headlines: Vec::new(),
            labels: HashMap::new(),
        }
    }

    /// Whether `symbol`'s latest quote is too old to trust.
    pub fn is_stale(&self, symbol: &Symbol, now: Duration) -> bool {
        match (self.stale_after, self.store.get(symbol)) {
            (Some(after), Some(entry)) => entry.is_stale(now, after),
            _ => false,
        }
    }

    /// Takes in new prices: from the mock feed when due, or whatever the live
    /// feeds sent since the last call.
    pub fn poll(&mut self, clock: &dyn Clock) {
        let now = clock.monotonic();
        if let Some(news) = &mut self.news {
            let mut changed = false;
            while let Ok(event) = news.events.try_recv() {
                match event.update {
                    Update::Headlines { source, items } => {
                        news.by_source.insert(source, items);
                        changed = true;
                    }
                    Update::Status { message, .. } => {
                        if news.last_status.get(&event.feed) != Some(&message) {
                            eprintln!("{message}");
                            news.last_status.insert(event.feed, message);
                        }
                    }
                    Update::Quote(_) => {}
                }
            }
            if changed {
                self.headlines = merge_headlines(&news.by_source, clock.wall());
            }
        }
        match &mut self.source {
            Source::Mock { feed, last } => {
                if last.is_none_or(|l| now.saturating_sub(l) >= FEED_INTERVAL) {
                    for quote in feed.tick(clock.wall()) {
                        self.store.apply(quote, now);
                    }
                    *last = Some(now);
                }
            }
            Source::Live {
                events,
                arbiter,
                last_status,
                ..
            } => {
                while let Ok(event) = events.try_recv() {
                    match event.update {
                        Update::Quote(quote) => {
                            if arbiter.accept(event.feed, &quote.symbol, now) {
                                self.store.apply(quote, now);
                            }
                        }
                        Update::Headlines { .. } => {}
                        Update::Status { message, .. } => {
                            // Print each change once, not every retry tick.
                            if last_status.get(&event.feed) != Some(&message) {
                                eprintln!("{message}");
                                last_status.insert(event.feed, message);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn sym(s: &str) -> Symbol {
    s.parse().expect("demo symbols are valid")
}

/// The demo watchlist, used when the configuration has none.
pub fn demo_groups() -> Vec<Group> {
    vec![
        Group::titled(
            "Tech",
            ["AAPL", "MSFT", "NVDA", "GOOGL", "META", "AMZN", "TSLA"].map(sym),
        ),
        Group::titled("Banks & Energy", ["JPM", "XOM"].map(sym)),
        Group::titled("Crypto", ["BTC-USD"].map(sym)),
    ]
}

/// The part of the watchlist a line shows: its own symbols if it lists any,
/// else the groups it names (by title, case-insensitive), else everything.
pub fn line_groups(all: &[Group], names: &[String], symbols: &[Symbol]) -> Vec<Group> {
    if !symbols.is_empty() {
        return vec![Group {
            title: None,
            symbols: symbols.to_vec(),
        }];
    }
    if names.is_empty() {
        return all.to_vec();
    }
    all.iter()
        .filter(|g| {
            g.title
                .as_deref()
                .is_some_and(|t| names.iter().any(|n| n.eq_ignore_ascii_case(t)))
        })
        .cloned()
        .collect()
}

/// Builds a line's strategy: quotes chosen by `kind` over `groups`,
/// headlines, or both, with the line's own messages mixed in now and then.
fn strategy(
    kind: StrategyKind,
    content: Content,
    messages: &[String],
    seed: u64,
    groups: &[Group],
) -> Box<dyn Strategy> {
    let main: Box<dyn Strategy> = match content {
        Content::Quotes => quote_strategy(kind, seed, groups),
        Content::Headlines => Box::new(Headlines::new()),
        Content::Mixed => Box::new(
            Mix::new()
                .with(quote_strategy(kind, seed, groups), 4)
                .with(Headlines::new(), 1),
        ),
    };
    if messages.is_empty() {
        main
    } else {
        Box::new(
            Mix::new()
                .with(main, 6)
                .with(Messages::new(messages.to_vec()), 1),
        )
    }
}

/// Builds the quote strategy for `kind` over the watchlist `groups`.
fn quote_strategy(kind: StrategyKind, seed: u64, groups: &[Group]) -> Box<dyn Strategy> {
    let watchlist = || Watchlist::grouped(groups.to_vec());
    let symbols = || {
        groups
            .iter()
            .flat_map(|g| g.symbols.clone())
            .collect::<Vec<_>>()
    };
    match kind {
        // Groups with a mover after every three items, repeats kept 20 s
        // apart, notable moves highlighted.
        StrategyKind::Demo => {
            let mix = Mix::new()
                .with(watchlist(), 3)
                .with(Movers::new(seed).among(symbols()).min_move(0.5), 1);
            Box::new(Highlights::new(
                Caps::new(mix, Duration::from_secs(20)),
                HighlightRules::default(),
            ))
        }
        StrategyKind::Watchlist => {
            Box::new(Highlights::new(watchlist(), HighlightRules::default()))
        }
        StrategyKind::Movers => Box::new(Highlights::new(
            Caps::new(Movers::new(seed).among(symbols()), Duration::from_secs(8)),
            HighlightRules::default(),
        )),
    }
}

/// What a belt item shows; kept so it can be drawn at any scale.
enum Piece {
    Quote(QuoteSegment),
    Header(String),
    Headline { label: String, title: String },
    Text(String),
}

struct Entry {
    content: Piece,
    width: f64,
    /// Drawn images by scale factor (as `f32` bits).
    images: HashMap<u32, SegmentImage>,
}

/// One line of a banner: its strategy, look and belt.
pub struct LineRuntime {
    strategy: Box<dyn Strategy>,
    belt: Belt<u64>,
    renderer: SegmentRenderer,
    entries: HashMap<u64, Entry>,
    next_id: u64,
    /// Scale used to measure item widths (the largest of the banner's
    /// displays, so text is sized for the sharpest one).
    measure_scale: f32,
    /// For showing headline times in local time.
    local_tz: jiff::tz::TimeZone,
}

impl std::fmt::Debug for LineRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LineRuntime")
            .field("items", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl LineRuntime {
    /// Creates a line.
    pub fn new(spec: &LineSpec, seed: u64, groups: &[Group], clock: &dyn Clock) -> Self {
        let config = BeltConfig {
            speed: spec.speed,
            gap: 0.0,
            lookahead: 400.0,
            snap: Snap::None,
        };
        Self {
            strategy: strategy(
                spec.strategy,
                spec.content,
                &spec.messages,
                seed,
                &line_groups(groups, &spec.groups, &spec.symbols),
            ),
            belt: Belt::new(config, clock.monotonic()),
            renderer: SegmentRenderer::new(spec.style.clone()),
            local_tz: jiff::tz::TimeZone::system(),
            entries: HashMap::new(),
            next_id: 0,
            measure_scale: 1.0,
        }
    }

    /// The line's thickness in logical pixels.
    pub fn thickness(&self) -> f64 {
        f64::from(self.renderer.style().height)
    }

    /// Sets the scale used to measure new items (call when the layout
    /// changes).
    pub fn set_measure_scale(&mut self, scale: f64) {
        #[allow(clippy::cast_possible_truncation)]
        let scale = scale as f32;
        if scale.is_finite() && scale > 0.0 {
            self.measure_scale = scale;
            self.belt.set_snap(Snap::Pixels(f64::from(scale)));
        }
    }

    /// Switches to a new style; the belt restarts in the new look.
    pub fn set_style(&mut self, style: BannerStyle) {
        self.renderer.set_style(style);
        self.belt.clear();
        self.entries.clear();
    }

    /// Advances the belt to the clock's time for a belt `viewport` logical
    /// pixels long, pulling new items from the strategy.
    pub fn update(&mut self, market: &Market, clock: &dyn Clock, viewport: f64) {
        let Self {
            strategy,
            belt,
            renderer,
            entries,
            next_id,
            measure_scale,
            local_tz,
        } = self;
        let scale = *measure_scale;
        let wall = clock.wall();
        let store = &market.store;
        let mut source = |now| {
            let ctx = Context {
                store,
                now,
                wall,
                headlines: &market.headlines,
            };
            let content = match strategy.next(&ctx)? {
                Item::Header(title) => Piece::Header(title),
                Item::Headline(h) => Piece::Headline {
                    label: headline_label(&h, local_tz),
                    title: h.title,
                },
                Item::Text(text) => Piece::Text(text),
                Item::Quote { symbol, highlight } => {
                    let entry = store.get(&symbol)?;
                    let segment = QuoteSegment::from_quote(entry.quote())
                        .with_history(entry.history().map(|(_, price)| price))
                        .with_highlight(highlight.is_some())
                        .with_stale(market.is_stale(&symbol, now));
                    Piece::Quote(labelled(segment, entry.quote(), market.labels.get(&symbol)))
                }
            };
            let image = draw(renderer, &content, scale);
            let width = image.logical_width;
            let id = *next_id;
            *next_id += 1;
            entries.insert(
                id,
                Entry {
                    content,
                    width,
                    images: HashMap::from([(scale.to_bits(), image)]),
                },
            );
            Some(BeltItem::new(id, width))
        };
        belt.update(clock.monotonic(), viewport, &mut source);
        let live: HashSet<u64> = belt.items().copied().collect();
        entries.retain(|id, _| live.contains(id));
    }

    /// Draws this line's stretch of the belt for `segment` into `target`
    /// (the segment's length × this line's thickness, in physical pixels).
    fn render_line(&mut self, segment: &Segment, target: &mut Pixmap) {
        let len = target.width();
        #[allow(clippy::cast_possible_truncation)]
        let scale = segment.scale as f32;
        let viewport = segment.belt_start + segment.belt_len;
        // Draw any images this display's scale still needs.
        let wanted: Vec<u64> = self
            .belt
            .visible(viewport)
            .filter(|v| v.position + v.width > segment.belt_start)
            .map(|v| *v.content)
            .collect();
        for id in &wanted {
            if let Some(entry) = self.entries.get_mut(id)
                && !entry.images.contains_key(&scale.to_bits())
            {
                let image = draw(&mut self.renderer, &entry.content, scale);
                entry.images.insert(scale.to_bits(), image);
            }
        }
        let len_px = f64::from(len);
        let items = self
            .belt
            .visible(viewport)
            .filter(|v| v.position + v.width > segment.belt_start)
            .filter_map(|v| {
                let entry = self.entries.get(v.content)?;
                let image = entry.images.get(&scale.to_bits())?;
                let along = (v.position - segment.belt_start) * segment.scale;
                let x = if segment.flow == Flow::Right {
                    len_px - along - entry.width * segment.scale
                } else {
                    along
                };
                #[allow(clippy::cast_possible_truncation)]
                Some((&image.pixmap, x.round() as i32))
            });
        compose(target, self.renderer.style().background, items);
    }
}

/// One line's settings, for building a [`BannerRuntime`].
#[derive(Debug, Clone)]
pub struct LineSpec {
    /// How quotes are chosen.
    pub strategy: StrategyKind,
    /// Quotes, headlines, or both.
    pub content: Content,
    /// Text messages mixed in now and then.
    pub messages: Vec<String>,
    /// Only these watchlist groups (by title); empty means all.
    pub groups: Vec<String>,
    /// Only these symbols; empty means the groups.
    pub symbols: Vec<Symbol>,
    /// Scroll speed in logical pixels per second.
    pub speed: f64,
    /// The line's look; its height is the line's thickness.
    pub style: BannerStyle,
}

/// One banner: one or more lines stacked across its thickness, each with its
/// own belt, so each can scroll at its own speed. All lines share the
/// banner's segments, so they cross monitors and corners together.
pub struct BannerRuntime {
    lines: Vec<LineRuntime>,
    /// Per-line strips, reused between frames.
    line_strips: Vec<Option<Pixmap>>,
}

impl std::fmt::Debug for BannerRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BannerRuntime")
            .field("lines", &self.lines.len())
            .finish_non_exhaustive()
    }
}

impl BannerRuntime {
    /// Creates a banner with the given lines (top line first); `seed` varies
    /// random choices between lines.
    ///
    /// # Panics
    ///
    /// Panics if `lines` is empty.
    pub fn new(lines: &[LineSpec], groups: &[Group], seed: u64, clock: &dyn Clock) -> Self {
        assert!(!lines.is_empty(), "a banner needs at least one line");
        let lines: Vec<LineRuntime> = lines
            .iter()
            .enumerate()
            .map(|(i, l)| LineRuntime::new(l, seed.wrapping_add(i as u64 * 7919), groups, clock))
            .collect();
        let line_strips = lines.iter().map(|_| None).collect();
        Self { lines, line_strips }
    }

    /// A one-line banner.
    pub fn single(
        kind: StrategyKind,
        seed: u64,
        speed: f64,
        style: BannerStyle,
        clock: &dyn Clock,
    ) -> Self {
        Self::new(
            &[LineSpec {
                strategy: kind,
                content: Content::Quotes,
                messages: Vec::new(),
                groups: Vec::new(),
                symbols: Vec::new(),
                speed,
                style,
            }],
            &demo_groups(),
            seed,
            clock,
        )
    }

    /// The banner's thickness in logical pixels: all lines together.
    pub fn thickness(&self) -> f64 {
        self.lines.iter().map(LineRuntime::thickness).sum()
    }

    /// Sets the scale used to measure new items on every line.
    pub fn set_measure_scale(&mut self, scale: f64) {
        for line in &mut self.lines {
            line.set_measure_scale(scale);
        }
    }

    /// Restyles the lines (one style per line, top first; extra styles are
    /// ignored, missing ones leave a line as it is).
    pub fn set_styles(&mut self, styles: Vec<BannerStyle>) {
        for (line, style) in self.lines.iter_mut().zip(styles) {
            line.set_style(style);
        }
    }

    /// Advances every line to the clock's time.
    pub fn update(&mut self, market: &Market, clock: &dyn Clock, viewport: f64) {
        for line in &mut self.lines {
            line.update(market, clock, viewport);
        }
    }

    /// Physical thickness of each line on a segment: rounded per line, with
    /// the last line taking up any rounding difference so the lines fill the
    /// segment exactly.
    fn line_thicknesses(&self, segment: &Segment) -> Vec<u32> {
        let total = segment.physical_thickness().max(1);
        let mut used = 0u32;
        let n = self.lines.len();
        self.lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let t = if i + 1 == n {
                    total.saturating_sub(used)
                } else {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let t = (line.thickness() * segment.scale).round() as u32;
                    t.min(total.saturating_sub(used))
                };
                used += t;
                t
            })
            .collect()
    }

    /// Draws a segment's stretch of every line into `strip` (resized to the
    /// segment's length × thickness in physical pixels), top line first,
    /// ready for [`present_strip`](stocktape_render::frame::present_strip).
    pub fn render_segment(&mut self, segment: &Segment, strip: &mut Option<Pixmap>) {
        let (len, thick) = (
            segment.physical_len().max(1),
            segment.physical_thickness().max(1),
        );
        let mut target = match strip.take() {
            Some(p) if p.width() == len && p.height() == thick => p,
            _ => Pixmap::new(len, thick).expect("non-zero strip size"),
        };
        let row_bytes = len as usize * 4;
        let thicknesses = self.line_thicknesses(segment);
        let mut row = 0usize;
        for ((line, slot), t) in self
            .lines
            .iter_mut()
            .zip(&mut self.line_strips)
            .zip(thicknesses)
        {
            if t == 0 {
                continue;
            }
            let mut line_strip = match slot.take() {
                Some(p) if p.width() == len && p.height() == t => p,
                _ => Pixmap::new(len, t).expect("non-zero line size"),
            };
            line.render_line(segment, &mut line_strip);
            let bytes = line_strip.data();
            let start = row * row_bytes;
            target.data_mut()[start..start + bytes.len()].copy_from_slice(bytes);
            row += t as usize;
            *slot = Some(line_strip);
        }
        *strip = Some(target);
    }
}

/// Applies a symbol's label: a display name, fixed decimals and a unit.
fn labelled(
    mut segment: QuoteSegment,
    quote: &stocktape_core::Quote,
    label: Option<&Label>,
) -> QuoteSegment {
    let Some(label) = label else {
        return segment;
    };
    if let Some(name) = &label.name {
        segment.symbol.clone_from(name);
    }
    if let Some(d) = label.decimals {
        let d = usize::from(d);
        segment.price = format_fixed(quote.last, d, false);
        if let Some(change) = quote.change() {
            segment.change = format_fixed(change, d, true);
        }
    }
    if let Some(unit) = &label.unit {
        segment.price.push_str(unit);
    }
    segment
}

/// `SOURCE · 10:42` (local time) for a headline, or just the source.
fn headline_label(h: &Headline, tz: &jiff::tz::TimeZone) -> String {
    match h.published {
        Some(t) => {
            let local = t.to_zoned(tz.clone());
            format!("{} · {:02}:{:02}", h.source, local.hour(), local.minute())
        }
        None => h.source.clone(),
    }
}

fn draw(renderer: &mut SegmentRenderer, content: &Piece, scale: f32) -> SegmentImage {
    match content {
        Piece::Quote(segment) => renderer.draw(segment, scale),
        Piece::Header(title) => renderer.draw_header(title, scale),
        Piece::Headline { label, title } => renderer.draw_headline(label, title, scale),
        Piece::Text(text) => renderer.draw_text(text, scale),
    }
}

/// A fixed point in time for previews and tests (2026-11-12 15:30 UTC).
const PREVIEW_EPOCH: i64 = 1_794_497_400;

/// Renders a demo frame of `skin`, `width` pixels wide, as it looks once the
/// belt has filled. Always the same image for the same skin and width.
pub fn preview_frame(skin: &stocktape_skin::Skin, width: u32) -> Pixmap {
    let clock = stocktape_core::ManualClock::new(
        stocktape_core::Timestamp::from_second(PREVIEW_EPOCH).expect("valid preview time"),
    );
    let mut market = Market::demo(7);
    let mut banner = BannerRuntime::single(
        StrategyKind::Demo,
        7,
        240.0,
        BannerStyle::from(skin),
        &clock,
    );
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let height = banner.thickness().ceil() as u32;
    let segment = Segment {
        display: 0,
        rect: stocktape_layout::Rect::new(0, 0, width, height),
        scale: 1.0,
        flow: Flow::Left,
        belt_start: 0.0,
        belt_len: f64::from(width),
    };
    for _ in 0..(60 * 40) {
        clock.advance(Duration::from_millis(16));
        market.poll(&clock);
        banner.update(&market, &clock, segment.belt_len);
    }
    let mut strip = None;
    banner.render_segment(&segment, &mut strip);
    strip.expect("strip is drawn")
}

#[cfg(test)]
mod tests {
    use super::*;
    use stocktape_core::{ManualClock, Timestamp};
    use stocktape_layout::{
        Display, DisplaySelector, Edge, Placement, PlacementOptions, Rect, plan,
    };

    fn clock() -> ManualClock {
        ManualClock::new(Timestamp::from_second(PREVIEW_EPOCH).unwrap())
    }

    fn banner(speed: f64, clock: &dyn Clock) -> BannerRuntime {
        BannerRuntime::single(StrategyKind::Demo, 1, speed, BannerStyle::default(), clock)
    }

    fn run(
        market: &mut Market,
        b: &mut BannerRuntime,
        clock: &ManualClock,
        viewport: f64,
        frames: u32,
    ) {
        for _ in 0..frames {
            clock.advance(Duration::from_millis(16));
            market.poll(clock);
            b.update(market, clock, viewport);
        }
    }

    fn ink_columns(p: &Pixmap) -> usize {
        let bg = [0x0E, 0x1A, 0x2F, 0xFF];
        (0..p.width())
            .filter(|x| {
                (0..p.height()).any(|y| {
                    let i = ((y * p.width() + x) * 4) as usize;
                    p.data()[i..i + 4] != bg
                })
            })
            .count()
    }

    fn row(scales: [f64; 2]) -> Vec<Display> {
        vec![
            Display {
                name: "A".into(),
                rect: Rect::new(0, 0, 1000, 600),
                scale: scales[0],
                primary: true,
            },
            Display {
                name: "B".into(),
                rect: Rect::new(1000, 0, 1000, 600),
                scale: scales[1],
                primary: false,
            },
        ]
    }

    #[test]
    fn content_fills_every_segment_of_a_span() {
        let clock = clock();
        let mut market = Market::demo(1);
        let mut b = banner(600.0, &clock);
        let layout = plan(
            &row([1.0, 2.0]),
            &Placement::Edge {
                displays: DisplaySelector::All,
                edge: Edge::Bottom,
            },
            &PlacementOptions {
                thickness: b.thickness(),
                ..PlacementOptions::default()
            },
        )
        .unwrap();
        b.set_measure_scale(2.0);
        run(&mut market, &mut b, &clock, layout.belt_len(), 600);
        for segment in &layout.segments {
            let mut strip = None;
            b.render_segment(segment, &mut strip);
            let strip = strip.unwrap();
            assert_eq!(strip.width(), segment.physical_len());
            assert_eq!(strip.height(), segment.physical_thickness());
            assert!(
                ink_columns(&strip) * 2 > strip.width() as usize,
                "{segment:?}"
            );
        }
    }

    #[test]
    fn around_renders_vertical_segments() {
        let clock = clock();
        let mut market = Market::demo(2);
        let mut b = banner(900.0, &clock);
        let layout = plan(
            &row([1.0, 1.0]),
            &Placement::Around {
                displays: DisplaySelector::Primary,
            },
            &PlacementOptions {
                thickness: b.thickness(),
                ..PlacementOptions::default()
            },
        )
        .unwrap();
        run(&mut market, &mut b, &clock, layout.belt_len(), 400);
        let side = layout.segments.iter().find(|s| s.flow == Flow::Up).unwrap();
        let mut strip = None;
        b.render_segment(side, &mut strip);
        let strip = strip.unwrap();
        // The strip is drawn lengthwise; presenting it rotates it.
        assert_eq!(
            (strip.width(), strip.height()),
            (side.rect.height, side.rect.width)
        );
        assert!(ink_columns(&strip) > 100);
    }

    #[test]
    fn rightward_flow_mirrors_positions() {
        let clock = clock();
        let mut market = Market::demo(3);
        let mut b = banner(400.0, &clock);
        let mut seg = Segment {
            display: 0,
            rect: Rect::new(0, 0, 800, 56),
            scale: 1.0,
            flow: Flow::Left,
            belt_start: 0.0,
            belt_len: 800.0,
        };
        // After one second only the first ~400 px of belt hold content, at the
        // entry end: the right side for leftward flow, the left side for
        // rightward flow.
        run(&mut market, &mut b, &clock, 800.0, 60);
        let mut left = None;
        b.render_segment(&seg, &mut left);
        seg.flow = Flow::Right;
        let mut right = None;
        b.render_segment(&seg, &mut right);
        let (left, right) = (left.unwrap(), right.unwrap());
        let ink = |p: &Pixmap, xs: std::ops::Range<u32>| {
            xs.into_iter()
                .filter(|x| (0..56).any(|y| p.data()[((y * 800 + x) * 4) as usize] != 0x0E))
                .count()
        };
        assert!(ink(&left, 450..800) > 100 && ink(&left, 0..350) == 0);
        assert!(ink(&right, 0..350) > 100 && ink(&right, 450..800) == 0);
    }

    fn two_lines(clock: &dyn Clock) -> BannerRuntime {
        let skins = stocktape_skin::SkinLibrary::builtin();
        let style = |id: &str| BannerStyle::from(&skins.resolve(id).unwrap());
        BannerRuntime::new(
            &[
                LineSpec {
                    strategy: StrategyKind::Movers,
                    content: Content::Quotes,
                    messages: Vec::new(),
                    groups: Vec::new(),
                    symbols: Vec::new(),
                    speed: 300.0,
                    style: style("classic-dark"),
                },
                LineSpec {
                    strategy: StrategyKind::Watchlist,
                    content: Content::Quotes,
                    messages: Vec::new(),
                    groups: Vec::new(),
                    symbols: Vec::new(),
                    speed: 100.0,
                    style: style("daylight"),
                },
            ],
            &demo_groups(),
            5,
            clock,
        )
    }

    #[test]
    fn lines_scroll_at_their_own_speeds() {
        let clock = clock();
        let mut market = Market::demo(5);
        let mut b = two_lines(&clock);
        assert_eq!(b.thickness(), 56.0 + 52.0);
        run(&mut market, &mut b, &clock, 1000.0, 125);
        // After 2 s: 600 px of travel on the fast line, 200 px on the slow one.
        let offsets: Vec<f64> = b.lines.iter().map(|l| l.belt.offset()).collect();
        assert!((offsets[0] - 600.0).abs() < 1.0, "{offsets:?}");
        assert!((offsets[1] - 200.0).abs() < 1.0, "{offsets:?}");
    }

    #[test]
    fn lines_stack_in_the_strip() {
        let clock = clock();
        let mut market = Market::demo(6);
        let mut b = two_lines(&clock);
        run(&mut market, &mut b, &clock, 1000.0, 400);
        let segment = Segment {
            display: 0,
            rect: Rect::new(0, 0, 1000, 108),
            scale: 1.0,
            flow: Flow::Left,
            belt_start: 0.0,
            belt_len: 1000.0,
        };
        let mut strip = None;
        b.render_segment(&segment, &mut strip);
        let strip = strip.unwrap();
        assert_eq!((strip.width(), strip.height()), (1000, 108));
        let pixel = |x: u32, y: u32| {
            let i = ((y * 1000 + x) * 4) as usize;
            strip.data()[i..i + 4].to_vec()
        };
        // Top line has Classic Dark's navy background, the bottom line
        // Daylight's white, at least somewhere in their rows.
        let row_has = |y: u32, rgba: [u8; 4]| (0..1000).any(|x| pixel(x, y) == rgba);
        assert!(row_has(1, [0x0E, 0x1A, 0x2F, 0xFF]));
        assert!(row_has(106, [0xFF, 0xFF, 0xFF, 0xFF]));
        assert!(!row_has(106, [0x0E, 0x1A, 0x2F, 0xFF]));
    }

    #[test]
    fn line_thicknesses_fill_the_segment_exactly() {
        let clock = clock();
        let b = two_lines(&clock);
        let segment = |scale: f64, thick: u32| Segment {
            display: 0,
            rect: Rect::new(0, 0, thick, 500),
            scale,
            flow: Flow::Up,
            belt_start: 0.0,
            belt_len: 500.0,
        };
        assert_eq!(b.line_thicknesses(&segment(1.0, 108)), [56, 52]);
        // 1.25 × 108 = 135; 1.25 × 56 = 70 → 65 for the second line.
        assert_eq!(b.line_thicknesses(&segment(1.25, 135)), [70, 65]);
        assert_eq!(b.line_thicknesses(&segment(1.0, 30)), [30, 0]);
    }

    #[test]
    fn set_style_restarts_with_new_thickness() {
        let clock = clock();
        let mut market = Market::demo(1);
        let mut b = banner(400.0, &clock);
        run(&mut market, &mut b, &clock, 400.0, 120);
        let terminal = stocktape_skin::SkinLibrary::builtin()
            .resolve("terminal")
            .unwrap();
        b.set_styles(vec![BannerStyle::from(&terminal)]);
        assert_eq!(b.lines[0].entries.len(), 0);
        assert_eq!(b.thickness(), 44.0);
    }

    #[test]
    fn entries_are_released_after_scrolling_out() {
        let clock = clock();
        let mut market = Market::demo(1);
        let mut b = banner(2_000.0, &clock);
        run(&mut market, &mut b, &clock, 400.0, 600);
        assert!(
            b.lines[0].entries.len() < 12,
            "{}",
            b.lines[0].entries.len()
        );
    }

    fn headline(title: &str, published: Option<i64>) -> Headline {
        Headline {
            source: "Desk".into(),
            title: title.into(),
            published: published.map(|s| Timestamp::from_second(s).unwrap()),
            link: None,
        }
    }

    #[test]
    fn lines_draw_from_their_own_groups_or_symbols() {
        let all = demo_groups();
        let crypto = line_groups(&all, &["crypto".to_owned()], &[]);
        assert_eq!(crypto.len(), 1);
        assert_eq!(crypto[0].symbols[0].to_string(), "BTC-USD");
        let picked = line_groups(&all, &["Crypto".to_owned()], &[sym("AAPL")]);
        assert_eq!(picked[0].symbols, [sym("AAPL")], "symbols win over groups");
        assert_eq!(line_groups(&all, &[], &[]).len(), all.len());
        assert!(line_groups(&all, &["Nope".to_owned()], &[]).is_empty());

        // A line limited to Crypto only ever shows BTC-USD.
        let clock = clock();
        let mut market = Market::demo(8);
        let mut b = BannerRuntime::new(
            &[LineSpec {
                strategy: StrategyKind::Watchlist,
                content: Content::Quotes,
                messages: Vec::new(),
                groups: vec!["Crypto".to_owned()],
                symbols: Vec::new(),
                speed: 900.0,
                style: BannerStyle::default(),
            }],
            &all,
            1,
            &clock,
        );
        run(&mut market, &mut b, &clock, 2000.0, 120);
        let shown: HashSet<String> = b.lines[0]
            .entries
            .values()
            .filter_map(|e| match &e.content {
                Piece::Quote(q) => Some(q.symbol.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(shown, HashSet::from(["BTC-USD".to_owned()]));
    }

    #[test]
    fn labels_rename_and_reformat_quotes() {
        let quote = stocktape_core::Quote::new(
            "OANDA:EUR_USD".parse().unwrap(),
            1.08321,
            Timestamp::UNIX_EPOCH,
        )
        .with_previous_close(1.08);
        let plain = QuoteSegment::from_quote(&quote);
        assert_eq!(
            (plain.symbol.as_str(), plain.price.as_str()),
            ("EUR_USD", "1.08")
        );
        let fx = Label {
            name: Some("EUR/USD".into()),
            unit: None,
            decimals: Some(4),
        };
        let s = labelled(plain.clone(), &quote, Some(&fx));
        assert_eq!(
            (s.symbol.as_str(), s.price.as_str(), s.change.as_str()),
            ("EUR/USD", "1.0832", "+0.0032")
        );
        let gold = Label {
            name: Some("GOLD".into()),
            unit: Some("/oz".into()),
            decimals: None,
        };
        assert_eq!(
            labelled(plain.clone(), &quote, Some(&gold)).price,
            "1.08/oz"
        );
        assert_eq!(labelled(plain.clone(), &quote, None), plain);
    }

    #[test]
    fn headlines_merge_newest_first_without_duplicates_or_old_news() {
        let now = Timestamp::from_second(PREVIEW_EPOCH).unwrap();
        let mut by_source = HashMap::new();
        by_source.insert(
            "A".to_owned(),
            vec![
                headline("Older", Some(PREVIEW_EPOCH - 3600)),
                headline("Newest", Some(PREVIEW_EPOCH - 60)),
                headline("Ancient", Some(PREVIEW_EPOCH - 3 * 24 * 3600)),
                headline("Undated", None),
            ],
        );
        by_source.insert(
            "B".to_owned(),
            vec![headline("newest", Some(PREVIEW_EPOCH - 30))],
        );
        let merged = merge_headlines(&by_source, now);
        let titles: Vec<&str> = merged.iter().map(|h| h.title.as_str()).collect();
        // "newest" (B) is the same story as "Newest" (A): kept once, the newer.
        assert_eq!(titles, ["newest", "Older", "Undated"]);
    }

    #[test]
    fn a_headlines_line_shows_headlines() {
        let clock = clock();
        let mut market = Market::demo(4);
        market.set_headlines(
            "Desk",
            vec![
                headline("Stocks rise", Some(PREVIEW_EPOCH - 60)),
                headline("Oil slips", None),
            ],
            clock.wall(),
        );
        let style = BannerStyle::default();
        let mut b = BannerRuntime::new(
            &[LineSpec {
                strategy: StrategyKind::Demo,
                content: Content::Headlines,
                messages: vec!["Hello".to_owned()],
                groups: Vec::new(),
                symbols: Vec::new(),
                speed: 600.0,
                style,
            }],
            &demo_groups(),
            1,
            &clock,
        );
        run(&mut market, &mut b, &clock, 2000.0, 120);
        let kinds: Vec<&str> = b.lines[0]
            .entries
            .values()
            .map(|e| match &e.content {
                Piece::Headline { .. } => "headline",
                Piece::Text(_) => "text",
                Piece::Quote(_) => "quote",
                Piece::Header(_) => "header",
            })
            .collect();
        assert!(kinds.contains(&"headline"), "{kinds:?}");
        assert!(kinds.contains(&"text"), "{kinds:?}");
        assert!(!kinds.contains(&"quote"), "{kinds:?}");
    }

    #[test]
    fn headline_labels_show_source_and_local_time() {
        let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(-5));
        // 15:30 UTC is 10:30 at UTC-5.
        assert_eq!(
            headline_label(&headline("x", Some(PREVIEW_EPOCH)), &tz),
            "Desk · 10:30"
        );
        assert_eq!(headline_label(&headline("x", None), &tz), "Desk");
    }

    #[test]
    fn previews_are_repeatable() {
        let skin = stocktape_skin::SkinLibrary::builtin()
            .resolve("glass")
            .unwrap();
        let a = preview_frame(&skin, 300);
        let b = preview_frame(&skin, 300);
        assert_eq!(a.data(), b.data());
        assert_eq!((a.width(), a.height()), (300, 60));
    }
}
