//! Where banners go: mapping one belt onto edges of one or more displays.
//!
//! A banner is one [`Belt`](crate::Belt). [`plan`] turns a [`Placement`] and
//! the current displays into [`Segment`]s, each a rectangle on one display that
//! shows one stretch of that belt. Segment 0 holds the belt's leading end (where
//! items leave), the last segment its trailing end (where new items enter), so
//! content flows seamlessly from one segment into the next: across monitors,
//! or around the corners of a screen.
//!
//! Everything here is pure: displays are plain data, so layouts are tested
//! against made-up monitor arrangements.

use std::fmt;

/// A rectangle in physical pixels, in virtual-desktop coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

impl Rect {
    /// A rectangle.
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// One past the right edge.
    #[must_use]
    pub fn right(&self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    /// One past the bottom edge.
    #[must_use]
    pub fn bottom(&self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }

    /// Whether two rectangles share any pixel.
    #[must_use]
    pub fn intersects(&self, other: &Rect) -> bool {
        i64::from(self.x) < other.right()
            && i64::from(other.x) < self.right()
            && i64::from(self.y) < other.bottom()
            && i64::from(other.y) < self.bottom()
    }

    /// Whether `other` lies entirely inside `self`.
    #[must_use]
    pub fn contains(&self, other: &Rect) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }
}

/// A display (monitor), as the platform reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    /// The platform's name for it, e.g. `\\.\DISPLAY2` or `DP-1`.
    pub name: String,
    /// Position and size in physical pixels.
    pub rect: Rect,
    /// Logical-to-physical scale factor (1.0, 1.25, 2.0, ...).
    pub scale: f64,
    /// Whether this is the primary display.
    pub primary: bool,
}

/// Which displays a banner uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplaySelector {
    /// The primary display (or the first one if none is marked primary).
    Primary,
    /// Every display.
    All,
    /// Displays whose name contains this text (case-insensitive). Names are
    /// stable across reboots and re-plugging, unlike positions in a list.
    Name(String),
    /// A display by its position in the platform's list (0-based).
    Index(usize),
    /// Displays at least as wide as they are tall.
    Landscape,
    /// Displays taller than they are wide.
    Portrait,
    /// Every display matched by any of these.
    AnyOf(Vec<DisplaySelector>),
    /// Displays matched by the first selector but not the second, e.g. all
    /// displays except the portrait ones.
    Except(Box<DisplaySelector>, Box<DisplaySelector>),
}

impl Display {
    /// Whether the display is at least as wide as it is tall.
    #[must_use]
    pub fn is_landscape(&self) -> bool {
        self.rect.width >= self.rect.height
    }
}

/// A screen edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edge {
    /// Top edge.
    Top,
    /// Bottom edge.
    Bottom,
    /// Left edge.
    Left,
    /// Right edge.
    Right,
}

/// Where a banner goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Placement {
    /// Along one edge. When several displays are selected, one belt spans
    /// them (left to right for top/bottom, top to bottom for left/right).
    Edge {
        /// Which displays.
        displays: DisplaySelector,
        /// Which edge.
        edge: Edge,
    },
    /// Around the edges: of one display, or of the whole desktop when several
    /// are selected (displays in one row, side by side).
    Around {
        /// Which displays.
        displays: DisplaySelector,
    },
}

/// Fine-tuning shared by all placements.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementOptions {
    /// Banner thickness in logical pixels (usually the skin's height).
    pub thickness: f64,
    /// Distance from the screen edge in logical pixels, e.g. to stack a
    /// second banner above the first.
    pub offset: f64,
    /// Space left free at the start and end of an edge, in logical pixels
    /// (single-edge and spanning placements only).
    pub inset: (f64, f64),
    /// Reverse the flow: rightward on top/bottom edges, downward on
    /// left/right edges, clockwise around.
    pub reverse: bool,
}

impl Default for PlacementOptions {
    fn default() -> Self {
        Self {
            thickness: 56.0,
            offset: 0.0,
            inset: (0.0, 0.0),
            reverse: false,
        }
    }
}

/// The direction content moves across a segment on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flow {
    /// Right to left; items leave at the left end.
    Left,
    /// Left to right; items leave at the right end.
    Right,
    /// Bottom to top; items leave at the top.
    Up,
    /// Top to bottom; items leave at the bottom.
    Down,
}

impl Flow {
    /// Whether the segment is horizontal.
    #[must_use]
    pub fn is_horizontal(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

/// One rectangle of a banner on one display.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Index of the display in the list given to [`plan`].
    pub display: usize,
    /// The window rectangle, in physical pixels.
    pub rect: Rect,
    /// The display's scale factor.
    pub scale: f64,
    /// How content moves across it.
    pub flow: Flow,
    /// Where this segment's stretch starts, as a belt position (logical
    /// pixels from the belt's leading end).
    pub belt_start: f64,
    /// Length of the stretch in logical pixels.
    pub belt_len: f64,
}

impl Segment {
    /// Length along the flow in physical pixels.
    #[must_use]
    pub fn physical_len(&self) -> u32 {
        if self.flow.is_horizontal() {
            self.rect.width
        } else {
            self.rect.height
        }
    }

    /// Thickness across the flow in physical pixels.
    #[must_use]
    pub fn physical_thickness(&self) -> u32 {
        if self.flow.is_horizontal() {
            self.rect.height
        } else {
            self.rect.width
        }
    }
}

/// A planned banner.
#[derive(Debug, Clone, PartialEq)]
pub struct BannerLayout {
    /// Segments from the belt's leading end to its trailing end.
    pub segments: Vec<Segment>,
}

impl BannerLayout {
    /// Total belt length in logical pixels: the belt's viewport.
    #[must_use]
    pub fn belt_len(&self) -> f64 {
        self.segments.iter().map(|s| s.belt_len).sum()
    }
}

/// Why a placement couldn't be planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    /// No display matched the selector.
    NoDisplay(String),
    /// Around the desktop needs displays side by side in one row.
    NotInOneRow,
    /// The banner would be thicker than the display or have no length left.
    TooSmall,
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDisplay(sel) => write!(f, "no display matches {sel}"),
            Self::NotInOneRow => {
                f.write_str("going around several displays needs them side by side in one row")
            }
            Self::TooSmall => f.write_str("the banner doesn't fit on the display"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// Picks the displays a selector refers to, as indexes into `displays`,
/// in ascending order without duplicates.
#[must_use]
pub fn select(displays: &[Display], selector: &DisplaySelector) -> Vec<usize> {
    let mut picked = select_raw(displays, selector);
    picked.sort_unstable();
    picked.dedup();
    picked
}

fn select_raw(displays: &[Display], selector: &DisplaySelector) -> Vec<usize> {
    let matching = |keep: &dyn Fn(&Display) -> bool| -> Vec<usize> {
        displays
            .iter()
            .enumerate()
            .filter(|(_, d)| keep(d))
            .map(|(i, _)| i)
            .collect()
    };
    match selector {
        DisplaySelector::Landscape => matching(&Display::is_landscape),
        DisplaySelector::Portrait => matching(&|d| !d.is_landscape()),
        DisplaySelector::AnyOf(list) => list.iter().flat_map(|s| select_raw(displays, s)).collect(),
        DisplaySelector::Except(keep, drop) => {
            let dropped = select_raw(displays, drop);
            select_raw(displays, keep)
                .into_iter()
                .filter(|i| !dropped.contains(i))
                .collect()
        }
        DisplaySelector::Primary => displays
            .iter()
            .position(|d| d.primary)
            .or(if displays.is_empty() { None } else { Some(0) })
            .into_iter()
            .collect(),
        DisplaySelector::All => (0..displays.len()).collect(),
        DisplaySelector::Index(i) => (*i < displays.len()).then_some(*i).into_iter().collect(),
        DisplaySelector::Name(text) => {
            let text = text.to_lowercase();
            displays
                .iter()
                .enumerate()
                .filter(|(_, d)| d.name.to_lowercase().contains(&text))
                .map(|(i, _)| i)
                .collect()
        }
    }
}

fn px(logical: f64, scale: f64) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let v = (logical.max(0.0) * scale).round() as u32;
    v
}

/// A segment along `edge` of display `index`, `along` = (start, end) physical
/// pixels trimmed from the edge's two ends.
fn edge_segment(
    displays: &[Display],
    index: usize,
    edge: Edge,
    flow: Flow,
    options: &PlacementOptions,
    trim: (u32, u32),
) -> Result<Segment, LayoutError> {
    let d = &displays[index];
    let thick = px(options.thickness, d.scale).max(1);
    let offset = px(options.offset, d.scale);
    let r = d.rect;
    let (span, cross) = match edge {
        Edge::Top | Edge::Bottom => (r.width, r.height),
        Edge::Left | Edge::Right => (r.height, r.width),
    };
    let len = span
        .checked_sub(trim.0 + trim.1)
        .filter(|l| *l > 0)
        .ok_or(LayoutError::TooSmall)?;
    if thick + offset > cross {
        return Err(LayoutError::TooSmall);
    }
    let start = i32::try_from(trim.0).map_err(|_| LayoutError::TooSmall)?;
    let off = i32::try_from(offset).map_err(|_| LayoutError::TooSmall)?;
    let far =
        |extent: u32| i32::try_from(extent - thick - offset).map_err(|_| LayoutError::TooSmall);
    let rect = match edge {
        Edge::Top => Rect::new(r.x + start, r.y + off, len, thick),
        Edge::Bottom => Rect::new(r.x + start, r.y + far(r.height)?, len, thick),
        Edge::Left => Rect::new(r.x + off, r.y + start, thick, len),
        Edge::Right => Rect::new(r.x + far(r.width)?, r.y + start, thick, len),
    };
    Ok(Segment {
        display: index,
        rect,
        scale: d.scale,
        flow,
        belt_start: 0.0,
        belt_len: f64::from(len) / d.scale,
    })
}

/// Fills in belt positions for segments listed in travel order (entry first)
/// and returns them leading end first.
fn chain(mut travel: Vec<Segment>) -> BannerLayout {
    travel.reverse();
    let mut start = 0.0;
    for s in &mut travel {
        s.belt_start = start;
        start += s.belt_len;
    }
    BannerLayout { segments: travel }
}

/// Plans a banner on the given displays.
///
/// # Errors
///
/// Returns an error if no display matches, the banner doesn't fit, or a
/// desktop-wide `Around` placement has displays that aren't in one row.
pub fn plan(
    displays: &[Display],
    placement: &Placement,
    options: &PlacementOptions,
) -> Result<BannerLayout, LayoutError> {
    let selector = match placement {
        Placement::Edge { displays, .. } | Placement::Around { displays } => displays,
    };
    let mut chosen = select(displays, selector);
    if chosen.is_empty() {
        return Err(LayoutError::NoDisplay(format!("{selector:?}")));
    }
    match placement {
        Placement::Edge { edge, .. } => {
            let horizontal = matches!(edge, Edge::Top | Edge::Bottom);
            if horizontal {
                chosen.sort_by_key(|i| (displays[*i].rect.x, displays[*i].rect.y));
            } else {
                chosen.sort_by_key(|i| (displays[*i].rect.y, displays[*i].rect.x));
            }
            // Travel order: entry first. Default flow is leftward (enter on the
            // right) or upward (enter at the bottom).
            let forward = !options.reverse;
            let flow = match (horizontal, forward) {
                (true, true) => Flow::Left,
                (true, false) => Flow::Right,
                (false, true) => Flow::Up,
                (false, false) => Flow::Down,
            };
            if forward {
                chosen.reverse();
            }
            let last = chosen.len() - 1;
            let mut travel = Vec::with_capacity(chosen.len());
            for (n, i) in chosen.iter().enumerate() {
                let scale = displays[*i].scale;
                // Insets apply at the two outer ends of the whole banner.
                let (lead, tail) = (px(options.inset.0, scale), px(options.inset.1, scale));
                let mut trim = (0, 0);
                // `start` of an edge is its left/top end.
                let first_on_screen = if forward { n == last } else { n == 0 };
                let last_on_screen = if forward { n == 0 } else { n == last };
                if first_on_screen {
                    trim.0 = lead;
                }
                if last_on_screen {
                    trim.1 = tail;
                }
                travel.push(edge_segment(displays, *i, *edge, flow, options, trim)?);
            }
            Ok(chain(travel))
        }
        Placement::Around { .. } => {
            chosen.sort_by_key(|i| (displays[*i].rect.x, displays[*i].rect.y));
            for pair in chosen.windows(2) {
                let (a, b) = (&displays[pair[0]].rect, &displays[pair[1]].rect);
                if i64::from(b.x) < a.right() {
                    return Err(LayoutError::NotInOneRow);
                }
            }
            let thick = |i: usize| px(options.thickness, displays[i].scale).max(1);
            let (first, last) = (chosen[0], chosen[chosen.len() - 1]);
            // Counter-clockwise travel (leftward along the top), entering at the
            // top-right corner: top edges right to left, the left edge down,
            // bottom edges left to right, the right edge up. Horizontal
            // segments own the corners; side segments are trimmed.
            let mut travel = Vec::new();
            for i in chosen.iter().rev() {
                travel.push(edge_segment(
                    displays,
                    *i,
                    Edge::Top,
                    Flow::Left,
                    options,
                    (0, 0),
                )?);
            }
            travel.push(edge_segment(
                displays,
                first,
                Edge::Left,
                Flow::Down,
                options,
                (thick(first), thick(first)),
            )?);
            for i in &chosen {
                travel.push(edge_segment(
                    displays,
                    *i,
                    Edge::Bottom,
                    Flow::Right,
                    options,
                    (0, 0),
                )?);
            }
            travel.push(edge_segment(
                displays,
                last,
                Edge::Right,
                Flow::Up,
                options,
                (thick(last), thick(last)),
            )?);
            if options.reverse {
                travel.reverse();
                for s in &mut travel {
                    s.flow = match s.flow {
                        Flow::Left => Flow::Right,
                        Flow::Right => Flow::Left,
                        Flow::Up => Flow::Down,
                        Flow::Down => Flow::Up,
                    };
                }
            }
            Ok(chain(travel))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn display(name: &str, x: i32, y: i32, w: u32, h: u32, scale: f64, primary: bool) -> Display {
        Display {
            name: name.to_owned(),
            rect: Rect::new(x, y, w, h),
            scale,
            primary,
        }
    }

    fn three() -> Vec<Display> {
        vec![
            display("DP-1", 0, 0, 2560, 1440, 1.5, true),
            display("HDMI-1", -1920, 360, 1920, 1080, 1.0, false),
            display("DP-2", 2560, 0, 3840, 2160, 2.0, false),
        ]
    }

    fn opts() -> PlacementOptions {
        PlacementOptions {
            thickness: 40.0,
            ..PlacementOptions::default()
        }
    }

    fn edge(displays: DisplaySelector, edge: Edge) -> Placement {
        Placement::Edge { displays, edge }
    }

    #[test]
    fn selectors() {
        let d = three();
        assert_eq!(select(&d, &DisplaySelector::Primary), [0]);
        assert_eq!(select(&d, &DisplaySelector::All), [0, 1, 2]);
        assert_eq!(select(&d, &DisplaySelector::Index(2)), [2]);
        assert!(select(&d, &DisplaySelector::Index(3)).is_empty());
        assert_eq!(select(&d, &DisplaySelector::Name("dp".into())), [0, 2]);
        let no_primary: Vec<Display> = d
            .iter()
            .cloned()
            .map(|mut x| {
                x.primary = false;
                x
            })
            .collect();
        assert_eq!(select(&no_primary, &DisplaySelector::Primary), [0]);
        assert!(select(&[], &DisplaySelector::Primary).is_empty());
    }

    /// Landscape left and center, portrait on the right (the motivating
    /// setup for choosing monitors).
    fn with_portrait() -> Vec<Display> {
        vec![
            display(r"\\.\DISPLAY1", 0, 0, 1920, 1080, 1.0, true),
            display(r"\\.\DISPLAY2", -1920, 0, 1920, 1080, 1.0, false),
            display(r"\\.\DISPLAY3", 1920, -251, 1050, 1680, 1.0, false),
        ]
    }

    #[test]
    fn lists_orientation_and_exclusions() {
        let d = with_portrait();
        assert_eq!(select(&d, &DisplaySelector::Landscape), [0, 1]);
        assert_eq!(select(&d, &DisplaySelector::Portrait), [2]);
        let list = DisplaySelector::AnyOf(vec![
            DisplaySelector::Name("display2".into()),
            DisplaySelector::Index(0),
            DisplaySelector::Index(0),
            DisplaySelector::Name("missing".into()),
        ]);
        assert_eq!(select(&d, &list), [0, 1]);
        let except = DisplaySelector::Except(
            Box::new(DisplaySelector::All),
            Box::new(DisplaySelector::Name("DISPLAY3".into())),
        );
        assert_eq!(select(&d, &except), [0, 1]);
    }

    #[test]
    fn span_skips_the_portrait_monitor() {
        let layout = plan(
            &with_portrait(),
            &edge(DisplaySelector::Landscape, Edge::Bottom),
            &opts(),
        )
        .unwrap();
        let used: Vec<usize> = layout.segments.iter().map(|s| s.display).collect();
        // Left display first: it holds the belt's leading end.
        assert_eq!(used, [1, 0]);
        assert_eq!(layout.belt_len(), 3840.0);
    }

    #[test]
    fn bottom_of_primary() {
        let layout = plan(
            &three(),
            &edge(DisplaySelector::Primary, Edge::Bottom),
            &opts(),
        )
        .unwrap();
        assert_eq!(layout.segments.len(), 1);
        let s = &layout.segments[0];
        assert_eq!(s.rect, Rect::new(0, 1380, 2560, 60));
        assert_eq!(s.flow, Flow::Left);
        assert!((layout.belt_len() - 2560.0 / 1.5).abs() < 1e-9);
    }

    #[test]
    fn offset_and_insets() {
        let o = PlacementOptions {
            offset: 40.0,
            inset: (100.0, 20.0),
            ..opts()
        };
        let layout = plan(&three(), &edge(DisplaySelector::Primary, Edge::Top), &o).unwrap();
        assert_eq!(
            layout.segments[0].rect,
            Rect::new(150, 60, 2560 - 150 - 30, 60)
        );
    }

    #[test]
    fn span_runs_left_to_right_and_exits_on_the_left() {
        let layout = plan(&three(), &edge(DisplaySelector::All, Edge::Bottom), &opts()).unwrap();
        let names: Vec<usize> = layout.segments.iter().map(|s| s.display).collect();
        // Leading end (exit) is the leftmost display.
        assert_eq!(names, [1, 0, 2]);
        assert_eq!(layout.segments[0].belt_start, 0.0);
        assert_eq!(layout.segments[1].belt_start, 1920.0);
        assert!((layout.segments[2].belt_start - (1920.0 + 2560.0 / 1.5)).abs() < 1e-9);
        assert!(layout.segments.iter().all(|s| s.flow == Flow::Left));
        // Each display gets its own physical thickness.
        let thick: Vec<u32> = layout
            .segments
            .iter()
            .map(Segment::physical_thickness)
            .collect();
        assert_eq!(thick, [40, 60, 80]);
    }

    #[test]
    fn reversed_span_exits_on_the_right() {
        let o = PlacementOptions {
            reverse: true,
            ..opts()
        };
        let layout = plan(&three(), &edge(DisplaySelector::All, Edge::Top), &o).unwrap();
        let order: Vec<usize> = layout.segments.iter().map(|s| s.display).collect();
        assert_eq!(order, [2, 0, 1]);
        assert!(layout.segments.iter().all(|s| s.flow == Flow::Right));
    }

    #[test]
    fn vertical_edges() {
        let layout = plan(
            &three(),
            &edge(DisplaySelector::Index(2), Edge::Right),
            &opts(),
        )
        .unwrap();
        let s = &layout.segments[0];
        assert_eq!(s.rect, Rect::new(2560 + 3840 - 80, 0, 80, 2160));
        assert_eq!(s.flow, Flow::Up);
    }

    #[test]
    fn around_one_display() {
        let layout = plan(
            &three(),
            &Placement::Around {
                displays: DisplaySelector::Index(1),
            },
            &opts(),
        )
        .unwrap();
        let flows: Vec<Flow> = layout.segments.iter().map(|s| s.flow).collect();
        // Leading end first: the right edge going up ends at the entry corner.
        assert_eq!(flows, [Flow::Up, Flow::Right, Flow::Down, Flow::Left]);
        let total = layout.belt_len();
        assert!((total - (2.0 * 1920.0 + 2.0 * (1080.0 - 80.0))).abs() < 1e-9);
        for (i, a) in layout.segments.iter().enumerate() {
            for b in &layout.segments[i + 1..] {
                assert!(!a.rect.intersects(&b.rect), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn around_the_desktop() {
        let layout = plan(
            &three(),
            &Placement::Around {
                displays: DisplaySelector::All,
            },
            &opts(),
        )
        .unwrap();
        assert_eq!(layout.segments.len(), 3 + 1 + 3 + 1);
        let o = PlacementOptions {
            reverse: true,
            ..opts()
        };
        let reversed = plan(
            &three(),
            &Placement::Around {
                displays: DisplaySelector::All,
            },
            &o,
        )
        .unwrap();
        assert_eq!(reversed.segments.len(), 8);
        assert_eq!(
            reversed.segments[0].flow,
            Flow::Right,
            "clockwise exits along the top"
        );
    }

    #[test]
    fn errors() {
        let d = three();
        assert!(matches!(
            plan(
                &d,
                &edge(DisplaySelector::Name("nope".into()), Edge::Top),
                &opts()
            ),
            Err(LayoutError::NoDisplay(_))
        ));
        let stacked = vec![
            display("A", 0, 0, 1920, 1080, 1.0, true),
            display("B", 0, 1080, 1920, 1080, 1.0, false),
        ];
        assert_eq!(
            plan(
                &stacked,
                &Placement::Around {
                    displays: DisplaySelector::All
                },
                &opts()
            ),
            Err(LayoutError::NotInOneRow)
        );
        let huge = PlacementOptions {
            thickness: 5000.0,
            ..opts()
        };
        assert_eq!(
            plan(&d, &edge(DisplaySelector::Primary, Edge::Top), &huge),
            Err(LayoutError::TooSmall)
        );
        let insets = PlacementOptions {
            inset: (2000.0, 2000.0),
            ..opts()
        };
        assert_eq!(
            plan(&d, &edge(DisplaySelector::Primary, Edge::Top), &insets),
            Err(LayoutError::TooSmall)
        );
    }

    fn row() -> impl Strategy<Value = Vec<Display>> {
        proptest::collection::vec(
            (
                800u32..4000,
                600u32..2400,
                0u32..200,
                -300i32..300,
                0usize..4,
            ),
            1..5,
        )
        .prop_map(|specs| {
            let mut x = -2000i32;
            specs
                .into_iter()
                .enumerate()
                .map(|(i, (w, h, gap, y, s))| {
                    let d = display(
                        &format!("D{i}"),
                        x,
                        y,
                        w,
                        h,
                        [1.0, 1.25, 1.5, 2.0][s],
                        i == 0,
                    );
                    x += i32::try_from(w + gap).unwrap();
                    d
                })
                .collect()
        })
    }

    fn placements() -> impl Strategy<Value = (Placement, bool)> {
        let edge = prop_oneof![
            Just(Edge::Top),
            Just(Edge::Bottom),
            Just(Edge::Left),
            Just(Edge::Right)
        ];
        let sel = prop_oneof![Just(DisplaySelector::Primary), Just(DisplaySelector::All)];
        (
            prop_oneof![
                (sel.clone(), edge).prop_map(|(displays, edge)| Placement::Edge { displays, edge }),
                sel.prop_map(|displays| Placement::Around { displays }),
            ],
            any::<bool>(),
        )
    }

    proptest! {
        /// Segments stay on their display, never overlap, and form one
        /// contiguous belt with matching lengths.
        #[test]
        fn layouts_are_consistent(displays in row(), (placement, reverse) in placements(), thickness in 8.0f64..120.0) {
            let o = PlacementOptions { thickness, reverse, ..PlacementOptions::default() };
            let Ok(layout) = plan(&displays, &placement, &o) else { return Ok(()); };
            prop_assert!(!layout.segments.is_empty());
            let mut expected_start = 0.0;
            for (i, s) in layout.segments.iter().enumerate() {
                prop_assert!(displays[s.display].rect.contains(&s.rect), "{s:?}");
                prop_assert!((s.belt_start - expected_start).abs() < 1e-6);
                prop_assert!((s.belt_len * s.scale - f64::from(s.physical_len())).abs() < 1e-6);
                expected_start += s.belt_len;
                for other in &layout.segments[i + 1..] {
                    prop_assert!(!s.rect.intersects(&other.rect), "{s:?} / {other:?}");
                }
            }
        }
    }
}
