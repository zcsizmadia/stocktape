//! Quote segments: the text of one quote, measured and drawn on the CPU.
//!
//! Each segment is drawn once into an RGBA image, which the compositor then
//! only has to move. Drawing on the CPU with bundled fonts makes the output
//! identical on every machine, so it can be checked against reference images
//! in tests.

use cosmic_text::fontdb;
use cosmic_text::{
    Attrs, Buffer, Family, FeatureTag, FontFeatures, FontSystem, Metrics, Shaping, SwashCache,
    Weight,
};
use stocktape_core::format::{format_change, format_percent, format_price};
use stocktape_core::{Direction, Quote};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

use crate::style::{BannerStyle, Rgba, TextStyle};

/// Fonts shipped with stocktape (SIL Open Font License; see
/// `assets/fonts/*-LICENSE.txt`).
const FONTS: [&[u8]; 5] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Bold.ttf"),
    include_bytes!("../assets/fonts/Inter-ExtraBold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
];

/// Price points kept for a sparkline.
const SPARK_POINTS: usize = 64;
/// Opacity of the move-color wash behind highlighted quotes.
const HIGHLIGHT_ALPHA: u8 = 40;

/// The display text of one quote.
#[derive(Debug, Clone, PartialEq)]
pub struct QuoteSegment {
    /// Ticker as shown, e.g. `AAPL`.
    pub symbol: String,
    /// Formatted price, e.g. `238.41`.
    pub price: String,
    /// Formatted change, e.g. `+2.17`; empty when unknown.
    pub change: String,
    /// Formatted percent change, e.g. `+0.92%`; empty when unknown.
    pub percent: String,
    /// Direction, for the arrow and colors.
    pub direction: Direction,
    /// Recent prices, oldest first, for a sparkline.
    pub history: Vec<f64>,
    /// Draw attention to this quote (a strategy flagged it).
    pub highlight: bool,
}

impl QuoteSegment {
    /// Builds the display text for a quote.
    #[must_use]
    pub fn from_quote(quote: &Quote) -> Self {
        Self {
            symbol: quote.symbol.ticker().to_owned(),
            price: format_price(quote.last),
            change: quote
                .change()
                .map(|c| format_change(c, quote.last))
                .unwrap_or_default(),
            percent: quote
                .change_percent()
                .map(format_percent)
                .unwrap_or_default(),
            direction: quote.direction(),
            history: Vec::new(),
            highlight: false,
        }
    }

    /// Marks the quote as highlighted (or not).
    #[must_use]
    pub fn with_highlight(mut self, highlight: bool) -> Self {
        self.highlight = highlight;
        self
    }

    /// Adds recent prices for a sparkline (only the last few dozen are kept).
    #[must_use]
    pub fn with_history<I: IntoIterator<Item = f64>>(mut self, prices: I) -> Self {
        self.history = prices.into_iter().filter(|p| p.is_finite()).collect();
        let excess = self.history.len().saturating_sub(SPARK_POINTS);
        self.history.drain(..excess);
        self
    }
}

/// A drawn segment.
#[derive(Debug, Clone)]
pub struct SegmentImage {
    /// The pixels, premultiplied RGBA, `width × height` physical pixels.
    pub pixmap: Pixmap,
    /// Width in logical pixels (physical width divided by the scale factor).
    pub logical_width: f64,
}

/// Measures and draws quote segments with the bundled fonts.
pub struct SegmentRenderer {
    fonts: FontSystem,
    glyphs: SwashCache,
    style: BannerStyle,
}

impl std::fmt::Debug for SegmentRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentRenderer")
            .field("style", &self.style)
            .finish_non_exhaustive()
    }
}

/// One shaped piece of text, ready to draw.
struct Shaped {
    buffer: Buffer,
    width: f32,
    baseline: f32,
    color: Rgba,
}

/// One drawable part of a segment, positioned in physical pixels.
enum Part {
    Text {
        x: f32,
        shaped: Shaped,
    },
    Arrow {
        x: f32,
        size: f32,
    },
    Pill {
        x: f32,
        width: f32,
        center_y: f32,
        height: f32,
        color: Rgba,
    },
    Spark {
        x: f32,
        width: f32,
        center_y: f32,
        height: f32,
    },
}

/// Arrow size relative to the change text size.
const ARROW_SCALE: f32 = 0.55;
/// Space between the arrow and the change text, relative to the change size.
const ARROW_GAP: f32 = 0.3;
/// Space between change and percent, relative to the spacing.
const PERCENT_GAP: f32 = 0.6;
/// Height of the lowercase x relative to the font size, for centering.
const X_HEIGHT: f32 = 0.36;

impl SegmentRenderer {
    /// Creates a renderer that uses only the bundled fonts, never system
    /// fonts, so output is the same everywhere.
    #[must_use]
    pub fn new(style: BannerStyle) -> Self {
        let mut db = fontdb::Database::new();
        for font in FONTS {
            db.load_font_data(font.to_vec());
        }
        Self {
            fonts: FontSystem::new_with_locale_and_db("en-US".to_owned(), db),
            glyphs: SwashCache::new(),
            style,
        }
    }

    /// The style in use.
    #[must_use]
    pub fn style(&self) -> &BannerStyle {
        &self.style
    }

    /// Replaces the style. Segments drawn earlier keep their old look.
    pub fn set_style(&mut self, style: BannerStyle) {
        self.style = style;
    }

    fn shape(&mut self, text: &str, style: &TextStyle, color: Rgba, scale: f32) -> Shaped {
        let size = style.size * scale;
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.25));
        let mut features = FontFeatures::new();
        // Tabular figures keep prices from wobbling as digits change.
        features.enable(FeatureTag::new(b"tnum"));
        let mut attrs = Attrs::new()
            .family(Family::Name(&style.family))
            .weight(Weight(style.weight));
        attrs.font_features = features;
        buffer.set_size(None, None);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let (width, baseline) = buffer.layout_runs().fold((0.0f32, 0.0f32), |(w, b), run| {
            (w.max(run.line_w), b.max(run.line_y))
        });
        Shaped {
            buffer,
            width,
            baseline,
            color,
        }
    }

    fn direction_color(&self, direction: Direction) -> Rgba {
        match direction {
            Direction::Up => self.style.up,
            Direction::Down => self.style.down,
            Direction::Flat => self.style.flat,
        }
    }

    /// Lays out a segment at `scale`: its parts and total width in physical
    /// pixels.
    fn layout(&mut self, segment: &QuoteSegment, scale: f32) -> (Vec<Part>, f32) {
        let style = self.style.clone();
        let accent = self.direction_color(segment.direction);
        let baseline = style.height * style.baseline * scale;
        let mut parts = Vec::new();
        let mut x = style.padding * scale;

        let symbol = self.shape(&segment.symbol, &style.symbol, style.symbol.color, scale);
        let w = symbol.width;
        parts.push(Part::Text { x, shaped: symbol });
        x += w + style.spacing * scale;

        let price = self.shape(&segment.price, &style.price, style.price.color, scale);
        let w = price.width;
        parts.push(Part::Text { x, shaped: price });
        x += w;

        if let Some(spark) = &style.sparkline {
            x += style.spacing * scale;
            let center_y = baseline - style.price.size * X_HEIGHT * scale;
            parts.push(Part::Spark {
                x,
                width: spark.width * scale,
                center_y,
                height: spark.height * scale,
            });
            x += spark.width * scale;
        }

        let mut shown_change = false;
        if let Some(change_style) = style.change.as_ref().filter(|_| !segment.change.is_empty()) {
            shown_change = true;
            x += style.spacing * scale;
            if style.arrow && segment.direction != Direction::Flat {
                let size = change_style.size * ARROW_SCALE * scale;
                parts.push(Part::Arrow { x, size });
                x += size + change_style.size * ARROW_GAP * scale;
            }
            let change = self.shape(&segment.change, change_style, accent, scale);
            let w = change.width;
            parts.push(Part::Text { x, shaped: change });
            x += w;
        }

        if let Some(percent_style) = style
            .percent
            .as_ref()
            .filter(|_| !segment.percent.is_empty())
        {
            x += if shown_change {
                style.spacing * PERCENT_GAP * scale
            } else {
                style.spacing * scale
            };
            if let Some(badge) = &style.badge {
                let (bg, fg) = match segment.direction {
                    Direction::Up => badge.up,
                    Direction::Down => badge.down,
                    Direction::Flat => badge.flat,
                };
                let text = self.shape(&segment.percent, percent_style, fg, scale);
                let pad_x = badge.padding_x * scale;
                let height = percent_style.size * scale + badge.padding_y * 2.0 * scale;
                let center_y = baseline - percent_style.size * X_HEIGHT * scale;
                let width = text.width + pad_x * 2.0;
                parts.push(Part::Pill {
                    x,
                    width,
                    center_y,
                    height,
                    color: bg,
                });
                parts.push(Part::Text {
                    x: x + pad_x,
                    shaped: text,
                });
                x += width;
            } else {
                let percent = self.shape(&segment.percent, percent_style, accent, scale);
                let w = percent.width;
                parts.push(Part::Text { x, shaped: percent });
                x += w;
            }
        }
        x += style.padding * scale;
        (parts, x)
    }

    /// Width of a segment in logical pixels at scale factor `scale`, rounded up
    /// to whole physical pixels.
    pub fn measure(&mut self, segment: &QuoteSegment, scale: f32) -> f64 {
        let (_, width) = self.layout(segment, scale);
        f64::from(width.ceil()) / f64::from(scale)
    }

    /// Draws a segment at scale factor `scale` (logical to physical pixels).
    ///
    /// # Panics
    ///
    /// Panics if `scale` is not finite and positive.
    pub fn draw(&mut self, segment: &QuoteSegment, scale: f32) -> SegmentImage {
        assert!(
            scale.is_finite() && scale > 0.0,
            "invalid scale factor {scale}"
        );
        let (parts, width) = self.layout(segment, scale);
        self.render(parts, width, segment, scale)
    }

    /// Draws a section header tile, e.g. `TECH`, in the symbol font and the
    /// skin's neutral color.
    ///
    /// # Panics
    ///
    /// Panics if `scale` is not finite and positive.
    pub fn draw_header(&mut self, title: &str, scale: f32) -> SegmentImage {
        assert!(
            scale.is_finite() && scale > 0.0,
            "invalid scale factor {scale}"
        );
        let style = self.style.clone();
        let shaped = self.shape(&title.to_uppercase(), &style.symbol, style.flat, scale);
        let x = style.padding * scale;
        let width = x * 2.0 + shaped.width;
        let blank = QuoteSegment {
            symbol: String::new(),
            price: String::new(),
            change: String::new(),
            percent: String::new(),
            direction: Direction::Flat,
            history: Vec::new(),
            highlight: false,
        };
        self.render(vec![Part::Text { x, shaped }], width, &blank, scale)
    }

    fn render(
        &mut self,
        parts: Vec<Part>,
        width: f32,
        segment: &QuoteSegment,
        scale: f32,
    ) -> SegmentImage {
        let style = self.style.clone();
        let accent = self.direction_color(segment.direction);
        let px_width = width.ceil().max(1.0);
        let px_height = (style.height * scale).ceil().max(1.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let mut pixmap =
            Pixmap::new(px_width as u32, px_height as u32).expect("segment size is non-zero");
        pixmap.fill(to_skia(style.background));
        if segment.highlight {
            // A soft wash of the move color marks highlighted quotes.
            if let Some(rect) = Rect::from_xywh(0.0, 0.0, px_width, px_height) {
                let mut paint = Paint::default();
                paint.set_color(to_skia(accent.with_alpha(HIGHLIGHT_ALPHA)));
                pixmap.fill_rect(rect, &paint, Transform::identity(), None);
            }
        }

        let baseline = style.height * style.baseline * scale;
        for part in parts {
            match part {
                Part::Text { x, mut shaped } => {
                    let top = baseline - shaped.baseline;
                    let c = shaped.color;
                    let color = cosmic_text::Color::rgba(c.r, c.g, c.b, c.a);
                    let (w, h) = (pixmap.width(), pixmap.height());
                    let data = pixmap.data_mut();
                    shaped.buffer.draw(
                        &mut self.fonts,
                        &mut self.glyphs,
                        color,
                        |gx, gy, gw, gh, c| {
                            #[allow(clippy::cast_possible_truncation)]
                            let ox = (x + gx as f32).round() as i32;
                            #[allow(clippy::cast_possible_truncation)]
                            let oy = (top + gy as f32).round() as i32;
                            blend_rect(data, w, h, ox, oy, gw, gh, c.as_rgba());
                        },
                    );
                }
                Part::Arrow { x, size } => {
                    let change_size = style.change.as_ref().map_or(0.0, |c| c.size);
                    let center_y = baseline - change_size * X_HEIGHT * scale;
                    draw_arrow(&mut pixmap, x, center_y, size, segment.direction, accent);
                }
                Part::Pill {
                    x,
                    width,
                    center_y,
                    height,
                    color,
                } => {
                    let radius = style
                        .badge
                        .as_ref()
                        .map_or(0.0, |b| b.radius * scale)
                        .min(height / 2.0);
                    fill_round_rect(
                        &mut pixmap,
                        x,
                        center_y - height / 2.0,
                        width,
                        height,
                        radius,
                        color,
                    );
                }
                Part::Spark {
                    x,
                    width,
                    center_y,
                    height,
                } => {
                    let stroke = style.sparkline.as_ref().map_or(1.0, |s| s.stroke * scale);
                    draw_sparkline(
                        &mut pixmap,
                        &segment.history,
                        x,
                        center_y,
                        width,
                        height,
                        stroke,
                        accent,
                    );
                }
            }
        }

        if style.separator_width > 0.0 {
            let sw = (style.separator_width * scale).max(1.0);
            if let Some(rect) = Rect::from_xywh(px_width - sw, 0.0, sw, px_height) {
                let mut paint = Paint::default();
                paint.set_color(to_skia(style.separator));
                pixmap.fill_rect(rect, &paint, Transform::identity(), None);
            }
        }

        SegmentImage {
            pixmap,
            logical_width: f64::from(px_width) / f64::from(scale),
        }
    }
}

fn fill_round_rect(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, color: Rgba) {
    let r = r.max(0.0).min(w / 2.0).min(h / 2.0);
    // Cubic approximation of a quarter circle.
    let k = r * 0.552_284_8;
    let mut p = PathBuilder::new();
    p.move_to(x + r, y);
    p.line_to(x + w - r, y);
    p.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    p.line_to(x + w, y + h - r);
    p.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    p.line_to(x + r, y + h);
    p.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    p.line_to(x, y + r);
    p.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    p.close();
    if let Some(path) = p.finish() {
        let mut paint = Paint::default();
        paint.set_color(to_skia(color));
        paint.anti_alias = true;
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_sparkline(
    pixmap: &mut Pixmap,
    prices: &[f64],
    x: f32,
    center_y: f32,
    width: f32,
    height: f32,
    stroke: f32,
    color: Rgba,
) {
    if prices.len() < 2 {
        return;
    }
    let (lo, hi) = prices
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(*p), hi.max(*p))
        });
    let range = hi - lo;
    let inset = stroke / 2.0;
    let usable = (height - stroke).max(0.0);
    #[allow(clippy::cast_precision_loss)]
    let step = (width - stroke) / (prices.len() - 1) as f32;
    let mut path = PathBuilder::new();
    for (i, p) in prices.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let t = if range > 0.0 {
            ((p - lo) / range) as f32
        } else {
            0.5
        };
        #[allow(clippy::cast_precision_loss)]
        let px = x + inset + step * i as f32;
        let py = center_y + usable / 2.0 - t * usable;
        if i == 0 {
            path.move_to(px, py);
        } else {
            path.line_to(px, py);
        }
    }
    if let Some(path) = path.finish() {
        let mut paint = Paint::default();
        paint.set_color(to_skia(color));
        paint.anti_alias = true;
        let stroke = Stroke {
            width: stroke,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

fn to_skia(c: Rgba) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

fn draw_arrow(
    pixmap: &mut Pixmap,
    x: f32,
    center_y: f32,
    size: f32,
    direction: Direction,
    color: Rgba,
) {
    let half = size / 2.0;
    let mut path = PathBuilder::new();
    match direction {
        Direction::Up => {
            path.move_to(x, center_y + half);
            path.line_to(x + size, center_y + half);
            path.line_to(x + half, center_y - half);
        }
        Direction::Down => {
            path.move_to(x, center_y - half);
            path.line_to(x + size, center_y - half);
            path.line_to(x + half, center_y + half);
        }
        Direction::Flat => return,
    }
    path.close();
    if let Some(path) = path.finish() {
        let mut paint = Paint::default();
        paint.set_color(to_skia(color));
        paint.anti_alias = true;
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// Source-over blends a solid color (straight alpha) into a premultiplied
/// RGBA buffer, clipped to the buffer.
#[allow(clippy::too_many_arguments)]
fn blend_rect(
    data: &mut [u8],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    rgba: [u8; 4],
) {
    let [r, g, b, a] = rgba.map(u32::from);
    if a == 0 {
        return;
    }
    let (width, height) = (i64::from(width), i64::from(height));
    let x0 = i64::from(x).max(0);
    let y0 = i64::from(y).max(0);
    let x1 = (i64::from(x) + i64::from(w)).min(width);
    let y1 = (i64::from(y) + i64::from(h)).min(height);
    let (sr, sg, sb) = (r * a / 255, g * a / 255, b * a / 255);
    let inv = 255 - a;
    for py in y0..y1 {
        for px in x0..x1 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let i = ((py * width + px) * 4) as usize;
            let d = &mut data[i..i + 4];
            #[allow(clippy::cast_possible_truncation)]
            {
                d[0] = (sr + u32::from(d[0]) * inv / 255) as u8;
                d[1] = (sg + u32::from(d[1]) * inv / 255) as u8;
                d[2] = (sb + u32::from(d[2]) * inv / 255) as u8;
                d[3] = (a + u32::from(d[3]) * inv / 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use stocktape_core::Timestamp;

    fn sample(symbol: &str, last: f64, close: f64) -> QuoteSegment {
        let quote = Quote::new(symbol.parse().unwrap(), last, Timestamp::UNIX_EPOCH)
            .with_previous_close(close);
        QuoteSegment::from_quote(&quote)
    }

    #[test]
    fn segment_text_from_quote() {
        let s = sample("AAPL", 238.41, 236.24);
        assert_eq!(s.symbol, "AAPL");
        assert_eq!(s.price, "238.41");
        assert_eq!(s.change, "+2.17");
        assert_eq!(s.percent, "+0.92%");
        assert_eq!(s.direction, Direction::Up);

        let q = Quote::new("XLON:VOD".parse().unwrap(), 71.5, Timestamp::UNIX_EPOCH);
        let s = QuoteSegment::from_quote(&q);
        assert_eq!(s.symbol, "VOD");
        assert!(s.change.is_empty() && s.percent.is_empty());
    }

    #[test]
    fn measure_matches_draw_and_scales() {
        let mut r = SegmentRenderer::new(BannerStyle::default());
        let s = sample("NVDA", 186.92, 182.07);
        let w1 = r.measure(&s, 1.0);
        let image = r.draw(&s, 1.0);
        assert_eq!(image.logical_width, w1);
        assert_eq!(f64::from(image.pixmap.width()), w1);
        assert_eq!(image.pixmap.height(), 56);

        let image2 = r.draw(&s, 2.0);
        assert_eq!(image2.pixmap.height(), 112);
        assert!((image2.logical_width - w1).abs() <= 1.0);
    }

    #[test]
    fn longer_text_is_wider() {
        let mut r = SegmentRenderer::new(BannerStyle::default());
        let short = r.measure(&sample("F", 11.2, 11.0), 1.0);
        let long = r.measure(&sample("GOOGL", 251.37, 252.25), 1.0);
        assert!(long > short);
    }

    #[test]
    fn tabular_figures_keep_width_stable() {
        let mut r = SegmentRenderer::new(BannerStyle::default());
        let a = r.measure(&sample("AAPL", 111.11, 100.0), 1.0);
        let b = r.measure(&sample("AAPL", 188.88, 170.0), 1.0);
        assert_eq!(a, b);
    }

    #[test]
    fn blend_clips_to_bounds() {
        let mut data = vec![0u8; 4 * 4 * 4];
        blend_rect(&mut data, 4, 4, -2, -2, 10, 10, [255, 0, 0, 255]);
        assert!(data.chunks(4).all(|p| p == [255, 0, 0, 255]));
        let mut data = vec![0u8; 16];
        blend_rect(&mut data, 2, 2, 5, 5, 3, 3, [255, 0, 0, 255]);
        assert!(data.iter().all(|b| *b == 0));
    }

    #[test]
    fn blend_is_source_over() {
        let mut data = vec![0, 0, 255, 255];
        blend_rect(&mut data, 1, 1, 0, 0, 1, 1, [255, 0, 0, 128]);
        assert_eq!(data, [128, 0, 127, 255]);
    }

    // Reference-image tests. Set STOCKTAPE_BLESS=1 to (re)write the images.

    fn golden_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
    }

    fn check_golden(name: &str, image: &Pixmap) {
        let path = golden_dir().join(format!("{name}.png"));
        if std::env::var_os("STOCKTAPE_BLESS").is_some() {
            std::fs::create_dir_all(golden_dir()).unwrap();
            image.save_png(&path).unwrap();
            return;
        }
        let expected = Pixmap::load_png(&path).unwrap_or_else(|e| {
            panic!(
                "missing reference {}: {e}; run with STOCKTAPE_BLESS=1",
                path.display()
            )
        });
        assert_eq!(
            (image.width(), image.height()),
            (expected.width(), expected.height()),
            "{name}: size"
        );
        // Allow tiny anti-aliasing differences between platforms.
        let mut differing = 0usize;
        for (a, b) in image.data().chunks(4).zip(expected.data().chunks(4)) {
            if a.iter().zip(b).any(|(x, y)| x.abs_diff(*y) > 24) {
                differing += 1;
            }
        }
        let total = (image.width() * image.height()) as usize;
        if differing * 200 > total {
            let actual = path.with_extension("new.png");
            image.save_png(&actual).unwrap();
            panic!(
                "{name}: {differing} of {total} pixels differ; wrote {}",
                actual.display()
            );
        }
    }

    fn history() -> Vec<f64> {
        (0..48)
            .map(|i| 236.0 + f64::from(i) * 0.05 + (f64::from(i) * 0.7).sin() * 0.6)
            .collect()
    }

    #[test]
    fn golden_builtin_skins() {
        let library = stocktape_skin::SkinLibrary::builtin();
        for id in library.ids() {
            let skin = library.resolve(id).unwrap();
            let mut r = SegmentRenderer::new(BannerStyle::from(&skin));
            let up = sample("AAPL", 238.41, 236.24).with_history(history());
            let down = sample("TSLA", 431.22, 443.86).with_history(history().into_iter().rev());
            check_golden(&format!("skin_{id}_up"), &r.draw(&up, 1.0).pixmap);
            check_golden(&format!("skin_{id}_down"), &r.draw(&down, 1.0).pixmap);
        }
    }

    #[test]
    fn hidden_parts_shrink_the_segment() {
        let mut full = SegmentRenderer::new(BannerStyle::default());
        let style = BannerStyle {
            change: None,
            percent: None,
            ..BannerStyle::default()
        };
        let mut bare = SegmentRenderer::new(style);
        let s = sample("AAPL", 238.41, 236.24);
        assert!(bare.measure(&s, 1.0) < full.measure(&s, 1.0) - 40.0);
    }

    #[test]
    fn history_is_trimmed_and_cleaned() {
        let s = sample("AAPL", 1.0, 1.0).with_history((0..100).map(f64::from).chain([f64::NAN]));
        assert_eq!(s.history.len(), 64);
        assert_eq!(s.history[0], 36.0);
    }

    #[test]
    fn golden_header_and_highlight() {
        let mut r = SegmentRenderer::new(BannerStyle::default());
        check_golden("header_1x", &r.draw_header("Tech", 1.0).pixmap);
        let hot = sample("NVDA", 191.0, 182.07).with_highlight(true);
        check_golden("highlight_up_1x", &r.draw(&hot, 1.0).pixmap);
    }

    #[test]
    fn golden_segments() {
        let mut r = SegmentRenderer::new(BannerStyle::default());
        check_golden(
            "up_1x",
            &r.draw(&sample("AAPL", 238.41, 236.24), 1.0).pixmap,
        );
        check_golden(
            "down_1x",
            &r.draw(&sample("TSLA", 431.22, 443.86), 1.0).pixmap,
        );
        check_golden(
            "flat_1x",
            &r.draw(&sample("JPM", 301.37, 301.37), 1.0).pixmap,
        );
        check_golden(
            "crypto_2x",
            &r.draw(&sample("BTC-USD", 101_284.0, 99_372.0), 2.0).pixmap,
        );
    }
}
