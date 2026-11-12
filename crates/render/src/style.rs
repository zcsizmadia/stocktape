//! Visual style of a banner: the renderer's view of a resolved skin.

use stocktape_skin::{Color, Skin, SkinLibrary, TextSpec};

/// A straight (not premultiplied) RGBA color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha; 255 is opaque.
    pub a: u8,
}

impl Rgba {
    /// An opaque color from a `0xRRGGBB` value.
    #[must_use]
    pub const fn hex(rgb: u32) -> Self {
        Self {
            r: (rgb >> 16) as u8,
            g: (rgb >> 8) as u8,
            b: rgb as u8,
            a: 255,
        }
    }

    /// The same color with a different alpha.
    #[must_use]
    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }
}

impl From<Color> for Rgba {
    fn from(c: Color) -> Self {
        Self {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        }
    }
}

/// Font, size and color for one text element.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    /// Font family; the bundled ones are `Inter` and `JetBrains Mono`.
    pub family: String,
    /// Font size in logical pixels.
    pub size: f32,
    /// Weight: 400 regular, 700 bold, 800 extra bold.
    pub weight: u16,
    /// Text color.
    pub color: Rgba,
}

impl From<&TextSpec> for TextStyle {
    fn from(t: &TextSpec) -> Self {
        Self {
            family: t.family.clone(),
            size: t.size,
            weight: t.weight,
            color: t.color.into(),
        }
    }
}

/// Pill behind the percent change: `(background, text)` per direction.
#[derive(Debug, Clone, PartialEq)]
pub struct BadgeStyle {
    /// Colors for rising prices.
    pub up: (Rgba, Rgba),
    /// Colors for falling prices.
    pub down: (Rgba, Rgba),
    /// Colors for unchanged prices.
    pub flat: (Rgba, Rgba),
    /// Corner radius; clamped to half the pill height.
    pub radius: f32,
    /// Horizontal padding.
    pub padding_x: f32,
    /// Vertical padding.
    pub padding_y: f32,
}

/// Intraday price line after the price.
#[derive(Debug, Clone, PartialEq)]
pub struct SparklineStyle {
    /// Width in logical pixels.
    pub width: f32,
    /// Height in logical pixels.
    pub height: f32,
    /// Line thickness in logical pixels.
    pub stroke: f32,
}

/// Everything the renderer needs to draw quote segments.
#[derive(Debug, Clone, PartialEq)]
pub struct BannerStyle {
    /// Banner thickness in logical pixels.
    pub height: f32,
    /// Background behind the segments; may be translucent.
    pub background: Rgba,
    /// Space before the first and after the last element of a segment.
    pub padding: f32,
    /// Space between elements inside a segment.
    pub spacing: f32,
    /// Where the text baseline sits, as a fraction of the height from the top.
    pub baseline: f32,
    /// The ticker symbol.
    pub symbol: TextStyle,
    /// The price.
    pub price: TextStyle,
    /// The absolute change, if shown; colored by direction.
    pub change: Option<TextStyle>,
    /// Whether an arrow precedes the change.
    pub arrow: bool,
    /// The percent change, if shown; colored by direction (or the badge).
    pub percent: Option<TextStyle>,
    /// Color for rising prices.
    pub up: Rgba,
    /// Color for falling prices.
    pub down: Rgba,
    /// Color for unchanged prices.
    pub flat: Rgba,
    /// Thin divider drawn at the end of each segment.
    pub separator: Rgba,
    /// Divider width in logical pixels; zero for none.
    pub separator_width: f32,
    /// Pill behind the percent change.
    pub badge: Option<BadgeStyle>,
    /// Intraday price line.
    pub sparkline: Option<SparklineStyle>,
}

impl From<&Skin> for BannerStyle {
    fn from(s: &Skin) -> Self {
        Self {
            height: s.height,
            background: s.background.into(),
            padding: s.padding,
            spacing: s.spacing,
            baseline: s.baseline,
            symbol: (&s.symbol).into(),
            price: (&s.price).into(),
            change: s.change.as_ref().map(Into::into),
            arrow: s.arrow,
            percent: s.percent.as_ref().map(Into::into),
            up: s.up.into(),
            down: s.down.into(),
            flat: s.flat.into(),
            separator: s.separator.into(),
            separator_width: s.separator_width,
            badge: s.badge.as_ref().map(|b| BadgeStyle {
                up: (b.up.0.into(), b.up.1.into()),
                down: (b.down.0.into(), b.down.1.into()),
                flat: (b.flat.0.into(), b.flat.1.into()),
                radius: b.radius,
                padding_x: b.padding_x,
                padding_y: b.padding_y,
            }),
            sparkline: s.sparkline.as_ref().map(|p| SparklineStyle {
                width: p.width,
                height: p.height,
                stroke: p.stroke,
            }),
        }
    }
}

impl Default for BannerStyle {
    /// The built-in default skin (Classic Dark).
    fn default() -> Self {
        let skin = SkinLibrary::builtin()
            .resolve(SkinLibrary::DEFAULT)
            .expect("the built-in default skin is valid");
        Self::from(&skin)
    }
}
