//! Banner layout for stocktape: the scrolling belt, and how banners map onto
//! display edges.
//!
//! Part of [stocktape](https://github.com/zcsizmadia/stocktape). See
//! `docs/design-notes.md` for the design.

pub mod belt;
pub mod placement;

pub use belt::{Belt, BeltConfig, BeltItem, BeltSource, Snap, VisibleItem};
pub use placement::{
    BannerLayout, Display, DisplaySelector, Edge, Flow, LayoutError, Placement, PlacementOptions,
    Rect, Segment, plan, select,
};
