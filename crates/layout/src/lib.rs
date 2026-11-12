//! Banner layout for stocktape: the scrolling belt, and (later) how banners
//! map onto display edges.
//!
//! Part of [stocktape](https://github.com/zcsizmadia/stocktape). See
//! `docs/design-notes.md` for the design.

pub mod belt;

pub use belt::{Belt, BeltConfig, BeltItem, BeltSource, Snap, VisibleItem};
