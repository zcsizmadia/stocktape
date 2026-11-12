//! The ticker: feed → store → belt → frame, independent of any window.
//!
//! Everything here is driven by a [`Clock`], so a whole frame can be produced
//! in a test with a [`ManualClock`](stocktape_core::ManualClock) and compared
//! with a reference image.

use std::collections::HashMap;
use std::time::Duration;

use stocktape_core::{Clock, QuoteStore, Symbol};
use stocktape_layout::{Belt, BeltConfig, BeltItem, Snap};
use stocktape_render::frame::compose;
use stocktape_render::{BannerStyle, QuoteSegment, SegmentImage, SegmentRenderer};
use stocktape_sources::mock::MockFeed;
use stocktape_strategies::{
    Caps, Context, Group, HighlightRules, Highlights, Item, Mix, Movers, Strategy, Watchlist,
};
use tiny_skia::Pixmap;

/// How often the mock feed produces new prices.
const FEED_INTERVAL: Duration = Duration::from_secs(1);
/// Price points kept per symbol.
const HISTORY: usize = 120;

/// The demo's strategy: the watchlist in sector groups, with a big mover
/// slotted in after every three items, repeats kept 20 s apart, and big moves
/// and new highs/lows highlighted.
fn demo_strategy(seed: u64) -> Box<dyn Strategy> {
    let sym = |s: &str| -> Symbol { s.parse().expect("demo symbols are valid") };
    let watchlist = Watchlist::grouped(vec![
        Group::titled(
            "Tech",
            ["AAPL", "MSFT", "NVDA", "GOOGL", "META", "AMZN", "TSLA"].map(sym),
        ),
        Group::titled("Banks & Energy", ["JPM", "XOM"].map(sym)),
        Group::titled("Crypto", ["BTC-USD"].map(sym)),
    ]);
    let mix = Mix::new()
        .with(watchlist, 3)
        .with(Movers::new(seed).min_move(0.5), 1);
    Box::new(Highlights::new(
        Caps::new(mix, Duration::from_secs(20)),
        HighlightRules::default(),
    ))
}

/// A running banner: feed, quote store, belt and drawn segments.
pub struct Ticker {
    feed: MockFeed,
    store: QuoteStore,
    strategy: Box<dyn Strategy>,
    belt: Belt<u64>,
    renderer: SegmentRenderer,
    images: HashMap<u64, SegmentImage>,
    next_id: u64,
    last_feed: Option<Duration>,
    scale: f32,
    frame: Option<Pixmap>,
}

impl std::fmt::Debug for Ticker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ticker")
            .field("symbols", &self.store.len())
            .field("segments", &self.images.len())
            .field("scale", &self.scale)
            .finish_non_exhaustive()
    }
}

impl Ticker {
    /// Creates a ticker with the demo mock feed.
    pub fn demo(seed: u64, speed: f64, scale: f32, style: BannerStyle, clock: &dyn Clock) -> Self {
        let feed = MockFeed::demo(seed);
        let config = BeltConfig {
            speed,
            gap: 0.0,
            lookahead: 400.0,
            snap: Snap::Pixels(f64::from(scale)),
        };
        Self {
            feed,
            store: QuoteStore::new(HISTORY),
            strategy: demo_strategy(seed),
            belt: Belt::new(config, clock.monotonic()),
            renderer: SegmentRenderer::new(style),
            images: HashMap::new(),
            next_id: 0,
            last_feed: None,
            scale,
            frame: None,
        }
    }

    /// Banner height in physical pixels at the current scale.
    pub fn height_px(&self) -> u32 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let h = (self.renderer.style().height * self.scale).ceil() as u32;
        h.max(1)
    }

    /// Changes the scale factor (e.g. the window moved to another monitor).
    /// Segments already on the belt keep their old resolution until they
    /// scroll out.
    pub fn set_scale(&mut self, scale: f32) {
        if scale.is_finite() && scale > 0.0 && scale != self.scale {
            self.scale = scale;
            self.belt.set_snap(Snap::Pixels(f64::from(scale)));
        }
    }

    /// Switches to a new style. The belt restarts so every quote on screen
    /// uses the new look; the banner height may change.
    pub fn set_style(&mut self, style: BannerStyle) {
        self.renderer.set_style(style);
        self.belt.clear();
        self.images.clear();
    }

    /// Pulls new prices from the feed when they are due.
    fn poll_feed(&mut self, clock: &dyn Clock) {
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

    /// Produces the frame for the current time, `width` physical pixels wide.
    ///
    /// # Panics
    ///
    /// Panics if `width` is zero.
    pub fn frame(&mut self, clock: &dyn Clock, width: u32) -> &Pixmap {
        assert!(width > 0, "frame width must be non-zero");
        self.poll_feed(clock);

        let viewport = f64::from(width) / f64::from(self.scale);
        let Self {
            store,
            strategy,
            belt,
            renderer,
            images,
            next_id,
            scale,
            ..
        } = self;
        let scale = *scale;
        let wall = clock.wall();

        let mut source = |now| {
            let ctx = Context { store, now, wall };
            let image = match strategy.next(&ctx)? {
                Item::Header(title) => renderer.draw_header(&title, scale),
                Item::Quote { symbol, highlight } => {
                    let entry = store.get(&symbol)?;
                    let segment = QuoteSegment::from_quote(entry.quote())
                        .with_history(entry.history().map(|(_, price)| price))
                        .with_highlight(highlight.is_some());
                    renderer.draw(&segment, scale)
                }
            };
            let width = image.logical_width;
            let id = *next_id;
            *next_id += 1;
            images.insert(id, image);
            Some(BeltItem::new(id, width))
        };
        belt.update(clock.monotonic(), viewport, &mut source);

        // Forget images that have scrolled out.
        let live: std::collections::HashSet<u64> = belt.items().copied().collect();
        images.retain(|id, _| live.contains(id));

        let height = self.height_px();
        let frame = match self.frame.take() {
            Some(f) if f.width() == width && f.height() == height => f,
            _ => Pixmap::new(width, height).expect("non-zero frame size"),
        };
        let mut frame = frame;
        let background = self.renderer.style().background;
        let scale = f64::from(self.scale);
        let items = self.belt.visible(viewport).filter_map(|item| {
            let image = self.images.get(item.content)?;
            #[allow(clippy::cast_possible_truncation)]
            let x = (item.position * scale).round() as i32;
            Some((&image.pixmap, x))
        });
        compose(&mut frame, background, items);
        self.frame.insert(frame)
    }
}

/// A fixed point in time for previews and tests (2026-11-12 15:30 UTC).
const PREVIEW_EPOCH: i64 = 1_794_497_400;

/// Renders a demo frame of `skin`, `width` pixels wide, as it looks after the
/// belt has filled. Always the same image for the same skin and width.
pub fn preview_frame(skin: &stocktape_skin::Skin, width: u32) -> Pixmap {
    let clock = stocktape_core::ManualClock::new(
        stocktape_core::Timestamp::from_second(PREVIEW_EPOCH).expect("valid preview time"),
    );
    let mut ticker = Ticker::demo(7, 240.0, 1.0, BannerStyle::from(skin), &clock);
    // Run long enough for the sparklines to have history and the belt to fill.
    for _ in 0..(60 * 40) {
        clock.advance(Duration::from_millis(16));
        ticker.frame(&clock, width);
    }
    ticker.frame(&clock, width).clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use stocktape_core::{ManualClock, Timestamp};

    impl Ticker {
        fn test(seed: u64, speed: f64, scale: f32, clock: &dyn Clock) -> Self {
            Self::demo(seed, speed, scale, BannerStyle::default(), clock)
        }
    }

    #[test]
    fn set_style_restarts_with_new_height() {
        let clock = clock();
        let mut ticker = Ticker::test(1, 400.0, 1.0, &clock);
        for _ in 0..120 {
            clock.advance(Duration::from_millis(16));
            ticker.frame(&clock, 400);
        }
        let terminal = stocktape_skin::SkinLibrary::builtin()
            .resolve("terminal")
            .unwrap();
        ticker.set_style(BannerStyle::from(&terminal));
        assert_eq!(ticker.images.len(), 0);
        assert_eq!(ticker.frame(&clock, 400).height(), 44);
    }

    #[test]
    fn previews_are_repeatable() {
        let skin = stocktape_skin::SkinLibrary::builtin()
            .resolve("glass")
            .unwrap();
        let a = preview_frame(&skin, 300);
        let b = preview_frame(&skin, 300);
        assert_eq!(a.data(), b.data());
        assert_eq!(a.height(), 60);
    }

    fn clock() -> ManualClock {
        // 2026-11-12 15:30:00 UTC, a fixed time so tests are repeatable.
        ManualClock::new(Timestamp::from_second(1_794_497_400).unwrap())
    }

    #[test]
    fn frame_has_requested_size() {
        let clock = clock();
        let mut ticker = Ticker::test(1, 120.0, 1.0, &clock);
        let frame = ticker.frame(&clock, 800);
        assert_eq!((frame.width(), frame.height()), (800, 56));
        let mut hidpi = Ticker::test(1, 120.0, 2.0, &clock);
        assert_eq!(hidpi.frame(&clock, 800).height(), 112);
    }

    #[test]
    fn content_scrolls_in_and_fills_the_banner() {
        let clock = clock();
        let mut ticker = Ticker::test(1, 400.0, 1.0, &clock);
        let background = [0x0E, 0x1A, 0x2F, 0xFF];
        let first = ticker.frame(&clock, 800).clone();
        assert!(
            first.data().chunks(4).all(|p| p == background),
            "starts empty"
        );
        for _ in 0..240 {
            clock.advance(Duration::from_millis(16));
            ticker.frame(&clock, 800);
        }
        let later = ticker.frame(&clock, 800);
        // Text pixels appear across the whole width once the belt is full.
        let columns_with_ink = (0..800)
            .filter(|x| {
                (0..56).any(|y| {
                    let i = ((y * 800 + x) * 4) as usize;
                    later.data()[i..i + 4] != background
                })
            })
            .count();
        assert!(columns_with_ink > 400, "{columns_with_ink}");
    }

    #[test]
    fn images_are_released_after_scrolling_out() {
        let clock = clock();
        let mut ticker = Ticker::test(1, 2_000.0, 1.0, &clock);
        for _ in 0..600 {
            clock.advance(Duration::from_millis(16));
            ticker.frame(&clock, 400);
        }
        assert!(ticker.images.len() < 12, "{}", ticker.images.len());
    }

    #[test]
    fn same_clock_same_frame() {
        let run = || {
            let clock = clock();
            let mut ticker = Ticker::test(9, 150.0, 1.0, &clock);
            for _ in 0..300 {
                clock.advance(Duration::from_millis(16));
                ticker.frame(&clock, 640);
            }
            ticker.frame(&clock, 640).clone()
        };
        assert_eq!(run().data(), run().data());
    }
}
