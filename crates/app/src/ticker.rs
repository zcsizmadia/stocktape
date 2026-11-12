//! The ticker without windows: a shared market (feed and quote store) and any
//! number of banners, each with its own strategy, skin and belt.
//!
//! Everything is driven by a [`Clock`], so frames can be produced in tests
//! with a [`ManualClock`](stocktape_core::ManualClock).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use stocktape_core::{Clock, QuoteStore, Symbol};
use stocktape_layout::{Belt, BeltConfig, BeltItem, Flow, Segment, Snap};
use stocktape_render::frame::compose;
use stocktape_render::{BannerStyle, QuoteSegment, SegmentImage, SegmentRenderer};
use stocktape_sources::mock::MockFeed;
use stocktape_strategies::{
    Caps, Context, Group, HighlightRules, Highlights, Item, Mix, Movers, Strategy, Watchlist,
};
use tiny_skia::Pixmap;

use crate::config::StrategyKind;

/// How often the mock feed produces new prices.
const FEED_INTERVAL: Duration = Duration::from_secs(1);
/// Price points kept per symbol.
const HISTORY: usize = 120;

/// The feed and the latest quotes, shared by every banner.
pub struct Market {
    feed: MockFeed,
    store: QuoteStore,
    last_feed: Option<Duration>,
}

impl Market {
    /// The demo market: sample prices from the mock feed.
    pub fn demo(seed: u64) -> Self {
        Self {
            feed: MockFeed::demo(seed),
            store: QuoteStore::new(HISTORY),
            last_feed: None,
        }
    }

    /// Pulls new prices from the feed when they are due.
    pub fn poll(&mut self, clock: &dyn Clock) {
        let now = clock.monotonic();
        let due = self
            .last_feed
            .is_none_or(|last| now.saturating_sub(last) >= FEED_INTERVAL);
        if due {
            for quote in self.feed.tick(clock.wall()) {
                self.store.apply(quote, now);
            }
            self.last_feed = Some(now);
        }
    }
}

fn sym(s: &str) -> Symbol {
    s.parse().expect("demo symbols are valid")
}

fn demo_watchlist() -> Watchlist {
    Watchlist::grouped(vec![
        Group::titled(
            "Tech",
            ["AAPL", "MSFT", "NVDA", "GOOGL", "META", "AMZN", "TSLA"].map(sym),
        ),
        Group::titled("Banks & Energy", ["JPM", "XOM"].map(sym)),
        Group::titled("Crypto", ["BTC-USD"].map(sym)),
    ])
}

/// Builds a banner's strategy.
fn strategy(kind: StrategyKind, seed: u64) -> Box<dyn Strategy> {
    match kind {
        // Sector groups with a mover after every three items, repeats kept
        // 20 s apart, notable moves highlighted.
        StrategyKind::Demo => {
            let mix = Mix::new()
                .with(demo_watchlist(), 3)
                .with(Movers::new(seed).min_move(0.5), 1);
            Box::new(Highlights::new(
                Caps::new(mix, Duration::from_secs(20)),
                HighlightRules::default(),
            ))
        }
        StrategyKind::Watchlist => {
            Box::new(Highlights::new(demo_watchlist(), HighlightRules::default()))
        }
        StrategyKind::Movers => Box::new(Highlights::new(
            Caps::new(Movers::new(seed), Duration::from_secs(8)),
            HighlightRules::default(),
        )),
    }
}

/// What a belt item shows; kept so it can be drawn at any scale.
enum Content {
    Quote(QuoteSegment),
    Header(String),
}

struct Entry {
    content: Content,
    width: f64,
    /// Drawn images by scale factor (as `f32` bits).
    images: HashMap<u32, SegmentImage>,
}

/// One banner: its strategy, look and belt.
pub struct BannerRuntime {
    strategy: Box<dyn Strategy>,
    belt: Belt<u64>,
    renderer: SegmentRenderer,
    entries: HashMap<u64, Entry>,
    next_id: u64,
    /// Scale used to measure item widths (the largest of the banner's
    /// displays, so text is sized for the sharpest one).
    measure_scale: f32,
}

impl std::fmt::Debug for BannerRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BannerRuntime")
            .field("items", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl BannerRuntime {
    /// Creates a banner.
    pub fn new(
        kind: StrategyKind,
        seed: u64,
        speed: f64,
        style: BannerStyle,
        clock: &dyn Clock,
    ) -> Self {
        let config = BeltConfig {
            speed,
            gap: 0.0,
            lookahead: 400.0,
            snap: Snap::None,
        };
        Self {
            strategy: strategy(kind, seed),
            belt: Belt::new(config, clock.monotonic()),
            renderer: SegmentRenderer::new(style),
            entries: HashMap::new(),
            next_id: 0,
            measure_scale: 1.0,
        }
    }

    /// The banner's thickness in logical pixels.
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
        } = self;
        let scale = *measure_scale;
        let wall = clock.wall();
        let store = &market.store;
        let mut source = |now| {
            let ctx = Context { store, now, wall };
            let content = match strategy.next(&ctx)? {
                Item::Header(title) => Content::Header(title),
                Item::Quote { symbol, highlight } => {
                    let entry = store.get(&symbol)?;
                    Content::Quote(
                        QuoteSegment::from_quote(entry.quote())
                            .with_history(entry.history().map(|(_, price)| price))
                            .with_highlight(highlight.is_some()),
                    )
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

    /// Draws a segment's stretch of the belt into `strip` (resized to the
    /// segment's length × thickness in physical pixels), ready for
    /// [`present_strip`](stocktape_render::frame::present_strip).
    pub fn render_segment(&mut self, segment: &Segment, strip: &mut Option<Pixmap>) {
        let (len, thick) = (
            segment.physical_len().max(1),
            segment.physical_thickness().max(1),
        );
        let mut target = match strip.take() {
            Some(p) if p.width() == len && p.height() == thick => p,
            _ => Pixmap::new(len, thick).expect("non-zero strip size"),
        };
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
        compose(&mut target, self.renderer.style().background, items);
        *strip = Some(target);
    }
}

fn draw(renderer: &mut SegmentRenderer, content: &Content, scale: f32) -> SegmentImage {
    match content {
        Content::Quote(segment) => renderer.draw(segment, scale),
        Content::Header(title) => renderer.draw_header(title, scale),
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
    let mut banner = BannerRuntime::new(
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
        BannerRuntime::new(StrategyKind::Demo, 1, speed, BannerStyle::default(), clock)
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

    #[test]
    fn set_style_restarts_with_new_thickness() {
        let clock = clock();
        let mut market = Market::demo(1);
        let mut b = banner(400.0, &clock);
        run(&mut market, &mut b, &clock, 400.0, 120);
        let terminal = stocktape_skin::SkinLibrary::builtin()
            .resolve("terminal")
            .unwrap();
        b.set_style(BannerStyle::from(&terminal));
        assert_eq!(b.entries.len(), 0);
        assert_eq!(b.thickness(), 44.0);
    }

    #[test]
    fn entries_are_released_after_scrolling_out() {
        let clock = clock();
        let mut market = Market::demo(1);
        let mut b = banner(2_000.0, &clock);
        run(&mut market, &mut b, &clock, 400.0, 600);
        assert!(b.entries.len() < 12, "{}", b.entries.len());
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
