//! Quote segments: the text of one quote, measured and drawn on the CPU.
//!
//! Each segment is drawn once into an RGBA image, which the GPU compositor
//! then only has to move. Drawing on the CPU with bundled fonts makes the
//! output identical on every machine, so it can be checked against reference
//! images in tests.

use cosmic_text::fontdb;
use cosmic_text::{
    Attrs, Buffer, Family, FeatureTag, FontFeatures, FontSystem, Metrics, Shaping, SwashCache,
    Weight,
};
use stocktape_core::format::{format_change, format_percent, format_price};
use stocktape_core::{Direction, Quote};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

use crate::style::{BannerStyle, Rgba, TextStyle};

const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_BOLD: &[u8] = include_bytes!("../assets/fonts/Inter-Bold.ttf");
const INTER_EXTRA_BOLD: &[u8] = include_bytes!("../assets/fonts/Inter-ExtraBold.ttf");

/// The family name of the bundled UI font.
pub const FONT_FAMILY: &str = "Inter";

/// The display text of one quote.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
        }
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

/// A laid-out segment, in physical pixels.
struct Layout {
    /// Text pieces with their left edge.
    pieces: Vec<(f32, Shaped)>,
    /// Arrow left edge and size, if the segment shows one.
    arrow: Option<(f32, f32)>,
    /// Total width.
    width: f32,
}

/// Arrow size relative to the change text size.
const ARROW_SCALE: f32 = 0.55;
/// Space between the arrow and the change text, relative to the change size.
const ARROW_GAP: f32 = 0.3;
/// Space between change and percent, relative to the spacing.
const PERCENT_GAP: f32 = 0.6;

impl SegmentRenderer {
    /// Creates a renderer that uses only the bundled fonts, never system
    /// fonts, so output is the same everywhere.
    #[must_use]
    pub fn new(style: BannerStyle) -> Self {
        let mut db = fontdb::Database::new();
        for font in [INTER_REGULAR, INTER_BOLD, INTER_EXTRA_BOLD] {
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
            .family(Family::Name(FONT_FAMILY))
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

    /// Lays out a segment at `scale` and returns its pieces and total width in
    /// physical pixels.
    fn layout(&mut self, segment: &QuoteSegment, scale: f32) -> Layout {
        let style = self.style.clone();
        let accent = self.direction_color(segment.direction);
        let mut pieces = Vec::new();
        let mut x = style.padding * scale;

        let symbol = self.shape(&segment.symbol, &style.symbol, style.symbol.color, scale);
        let w = symbol.width;
        pieces.push((x, symbol));
        x += w + style.spacing * scale;

        let price = self.shape(&segment.price, &style.price, style.price.color, scale);
        let w = price.width;
        pieces.push((x, price));
        x += w;

        let mut arrow = None;
        if !segment.change.is_empty() {
            x += style.spacing * scale;
            if segment.direction != Direction::Flat {
                let size = style.change.size * ARROW_SCALE * scale;
                arrow = Some((x, size));
                x += size + style.change.size * ARROW_GAP * scale;
            }
            let change = self.shape(&segment.change, &style.change, accent, scale);
            let w = change.width;
            pieces.push((x, change));
            x += w;
        }
        if !segment.percent.is_empty() {
            x += style.spacing * PERCENT_GAP * scale;
            let percent = self.shape(&segment.percent, &style.percent, accent, scale);
            let w = percent.width;
            pieces.push((x, percent));
            x += w;
        }
        x += style.padding * scale;
        Layout {
            pieces,
            arrow,
            width: x,
        }
    }

    /// Width of a segment in logical pixels at scale factor `scale`, rounded up
    /// to whole physical pixels.
    pub fn measure(&mut self, segment: &QuoteSegment, scale: f32) -> f64 {
        let width = self.layout(segment, scale).width;
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
        let Layout {
            pieces,
            arrow,
            width,
        } = self.layout(segment, scale);
        let style = self.style.clone();
        let px_width = width.ceil().max(1.0);
        let px_height = (style.height * scale).ceil().max(1.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let mut pixmap =
            Pixmap::new(px_width as u32, px_height as u32).expect("segment size is non-zero");
        pixmap.fill(to_skia(style.background));

        let baseline = style.height * style.baseline * scale;
        for (x, mut piece) in pieces {
            let top = baseline - piece.baseline;
            let color = cosmic_text::Color::rgba(
                piece.color.r,
                piece.color.g,
                piece.color.b,
                piece.color.a,
            );
            let (w, h) = (pixmap.width(), pixmap.height());
            let data = pixmap.data_mut();
            piece.buffer.draw(
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

        if let Some((x, size)) = arrow {
            let color = self.direction_color(segment.direction);
            // Center the arrow on the lowercase x-height of the change text.
            let center_y = baseline - style.change.size * 0.36 * scale;
            draw_arrow(&mut pixmap, x, center_y, size, segment.direction, color);
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
