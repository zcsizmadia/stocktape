//! Skins for stocktape: how a banner looks.
//!
//! A skin is a small TOML file (see `builtin/classic-dark.toml` for every
//! setting, with comments). Skins are kept apart from layout, so any skin works
//! on any banner, and they are designed for iteration:
//!
//! - **Tokens:** named colors (`[tokens] navy = "#0E1A2F"`), used elsewhere as
//!   `"$navy"`.
//! - **Inheritance:** `extends = "classic-dark"` starts from another skin and
//!   overrides only what changes. Tokens are resolved after merging, so
//!   redefining a token restyles everything that uses it.
//! - **Versioned:** every file states `schema = 1`, so the format can evolve
//!   without breaking old skins.
//! - **Strict:** unknown keys are errors, so typos don't silently do nothing.
//!
//! [`SkinLibrary`] holds the built-in skins plus any loaded from a folder, and
//! [`SkinLibrary::resolve`] turns a skin name into a fully resolved [`Skin`].
//!
//! Part of [stocktape](https://github.com/zcsizmadia/stocktape). See
//! `docs/design-notes.md` for the design.

mod color;
mod library;
mod resolve;

pub use color::Color;
pub use library::{SkinLibrary, SkinSource};
pub use resolve::SkinError;

/// The newest skin file format this version understands.
pub const SCHEMA_VERSION: i64 = 1;

/// Font, size and color of a text element.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpec {
    /// Font family, e.g. `Inter` or `JetBrains Mono`.
    pub family: String,
    /// Size in logical pixels.
    pub size: f32,
    /// Weight: 400 regular, 700 bold, 800 extra bold.
    pub weight: u16,
    /// Color. For the change and percent elements the move colors (or the
    /// badge colors) are used instead.
    pub color: Color,
}

/// A pill drawn behind the percent change.
#[derive(Debug, Clone, PartialEq)]
pub struct BadgeSpec {
    /// Pill and text colors for rising prices.
    pub up: (Color, Color),
    /// Pill and text colors for falling prices.
    pub down: (Color, Color),
    /// Pill and text colors for unchanged prices.
    pub flat: (Color, Color),
    /// Corner radius; clamped to half the pill height.
    pub radius: f32,
    /// Horizontal padding inside the pill.
    pub padding_x: f32,
    /// Vertical padding inside the pill.
    pub padding_y: f32,
}

/// A small price line drawn after the price.
#[derive(Debug, Clone, PartialEq)]
pub struct SparklineSpec {
    /// Width in logical pixels.
    pub width: f32,
    /// Height in logical pixels.
    pub height: f32,
    /// Line thickness in logical pixels.
    pub stroke: f32,
}

/// A fully resolved skin: every value present, tokens substituted.
#[derive(Debug, Clone, PartialEq)]
pub struct Skin {
    /// Identifier (file stem), e.g. `classic-dark`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// One-line description.
    pub description: String,
    /// Banner thickness in logical pixels.
    pub height: f32,
    /// Banner background; may be translucent.
    pub background: Color,
    /// Space at both ends of each quote.
    pub padding: f32,
    /// Space between the parts of a quote.
    pub spacing: f32,
    /// Text baseline as a fraction of the height from the top.
    pub baseline: f32,
    /// Ticker symbol.
    pub symbol: TextSpec,
    /// Price.
    pub price: TextSpec,
    /// Absolute change, if shown.
    pub change: Option<TextSpec>,
    /// Whether an arrow precedes the change.
    pub arrow: bool,
    /// Percent change, if shown.
    pub percent: Option<TextSpec>,
    /// Color for rising prices.
    pub up: Color,
    /// Color for falling prices.
    pub down: Color,
    /// Color for unchanged prices.
    pub flat: Color,
    /// Divider at the end of each quote.
    pub separator: Color,
    /// Divider width; zero for none.
    pub separator_width: f32,
    /// Pill behind the percent change.
    pub badge: Option<BadgeSpec>,
    /// Intraday price line.
    pub sparkline: Option<SparklineSpec>,
}
