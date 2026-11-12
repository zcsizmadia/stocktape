//! Visual style of a banner.
//!
//! This is the renderer's resolved view of a skin. The skin crate (#5) will
//! load TOML skins and produce one of these; until then [`BannerStyle::default`]
//! is the Classic Dark look from the mockups.

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

/// Font, size and color for one text element.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    /// Font size in logical pixels.
    pub size: f32,
    /// CSS-style weight: 400 regular, 700 bold, 800 extra bold.
    pub weight: u16,
    /// Text color.
    pub color: Rgba,
}

/// Everything the renderer needs to draw quote segments.
#[derive(Debug, Clone, PartialEq)]
pub struct BannerStyle {
    /// Banner thickness in logical pixels.
    pub height: f32,
    /// Background behind the segments.
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
    /// The absolute change; its color comes from `up`/`down`/`flat`.
    pub change: TextStyle,
    /// The percent change; its color comes from `up`/`down`/`flat`.
    pub percent: TextStyle,
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
}

impl Default for BannerStyle {
    /// Classic Dark.
    fn default() -> Self {
        Self {
            height: 56.0,
            background: Rgba::hex(0x0E1A2F),
            padding: 22.0,
            spacing: 10.0,
            baseline: 0.64,
            symbol: TextStyle {
                size: 24.0,
                weight: 800,
                color: Rgba::hex(0xFFFFFF),
            },
            price: TextStyle {
                size: 24.0,
                weight: 400,
                color: Rgba::hex(0xDCE3EE),
            },
            change: TextStyle {
                size: 19.0,
                weight: 700,
                color: Rgba::hex(0x9AA4B5),
            },
            percent: TextStyle {
                size: 15.0,
                weight: 700,
                color: Rgba::hex(0x9AA4B5),
            },
            up: Rgba::hex(0x34D399),
            down: Rgba::hex(0xF87171),
            flat: Rgba::hex(0x9AA4B5),
            separator: Rgba::hex(0xFFFFFF).with_alpha(20),
            separator_width: 1.0,
        }
    }
}
