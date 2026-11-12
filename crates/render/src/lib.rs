//! Rendering for stocktape banners.
//!
//! - [`segment`] draws each quote into an image on the CPU, with bundled
//!   fonts, so output is identical everywhere and testable against reference
//!   images.
//! - [`frame`] composes those images at belt positions into a banner frame,
//!   also on the CPU, so stocktape runs on machines without a GPU. A GPU
//!   backend is planned on top of this.
//!
//! Part of [stocktape](https://github.com/zcsizmadia/stocktape). See
//! `docs/design-notes.md` for the design.

pub mod frame;
pub mod segment;
pub mod style;

pub use segment::{QuoteSegment, SegmentImage, SegmentRenderer};
pub use style::{BannerStyle, Rgba, TextStyle};
