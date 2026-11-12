//! The belt: a continuous strip of items that scrolls past a viewport.
//!
//! The belt has its own coordinate, measured in logical pixels from where it
//! started. Items are laid end to end along it, separated by a fixed gap. The
//! *offset* is the belt coordinate at the viewport's leading edge, and it grows
//! with time: `offset = speed × elapsed`. Position depends only on the clock,
//! never on how many frames were drawn, so a dropped frame doesn't slow the
//! crawl down.
//!
//! The belt keeps itself filled a little beyond the viewport by pulling items
//! from a [`BeltSource`]. The source decides what comes next (that's the
//! strategy's job) and how wide it is (that's the renderer's job, through text
//! measurement), so the belt itself needs no fonts or GPU and is fully
//! testable.

use std::collections::VecDeque;
use std::time::Duration;

/// One item offered to the belt, with its measured width in logical pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct BeltItem<T> {
    /// What to show; opaque to the belt.
    pub content: T,
    /// Width along the belt, in logical pixels. Non-finite or negative widths
    /// are treated as zero.
    pub width: f64,
}

impl<T> BeltItem<T> {
    /// Creates an item.
    pub fn new(content: T, width: f64) -> Self {
        Self { content, width }
    }
}

/// Supplies the next item when the belt needs more content.
///
/// Implemented for any `FnMut(Duration) -> Option<BeltItem<T>>`, where the
/// argument is the monotonic time of the request.
pub trait BeltSource<T> {
    /// Returns the next item, or `None` if there is nothing to show right now.
    fn next_item(&mut self, now: Duration) -> Option<BeltItem<T>>;
}

impl<T, F> BeltSource<T> for F
where
    F: FnMut(Duration) -> Option<BeltItem<T>>,
{
    fn next_item(&mut self, now: Duration) -> Option<BeltItem<T>> {
        self(now)
    }
}

/// How the offset is rounded before items are positioned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Snap {
    /// Sub-pixel positions: smoothest motion, slightly soft text.
    None,
    /// Round the offset to whole device pixels at this scale factor (logical
    /// to physical). All items move together, so spacing never jitters; give
    /// items whole-pixel widths for crisp text.
    Pixels(f64),
}

/// Settings for a [`Belt`].
#[derive(Debug, Clone, PartialEq)]
pub struct BeltConfig {
    /// Scroll speed in logical pixels per second. Zero pauses; negative values
    /// are treated as zero.
    pub speed: f64,
    /// Space between consecutive items, in logical pixels.
    pub gap: f64,
    /// How far beyond the trailing edge of the viewport to keep the belt
    /// filled, in logical pixels, so new items are ready before they appear.
    pub lookahead: f64,
    /// Offset rounding.
    pub snap: Snap,
}

impl Default for BeltConfig {
    fn default() -> Self {
        Self {
            speed: 120.0,
            gap: 0.0,
            lookahead: 256.0,
            snap: Snap::None,
        }
    }
}

/// An item placed on the belt.
#[derive(Debug, Clone, PartialEq)]
struct Placed<T> {
    content: T,
    start: f64,
    width: f64,
}

impl<T> Placed<T> {
    fn end(&self) -> f64 {
        self.start + self.width
    }
}

/// An item currently intersecting the viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisibleItem<'a, T> {
    /// The item's content.
    pub content: &'a T,
    /// Distance from the viewport's leading edge (where items leave) to the
    /// item's leading side, in logical pixels. Negative while the item is
    /// partly scrolled out.
    pub position: f64,
    /// The item's width.
    pub width: f64,
}

/// A scrolling strip of items, driven by monotonic time.
///
/// ```
/// use std::time::Duration;
/// use stocktape_layout::belt::{Belt, BeltConfig, BeltItem};
///
/// let config = BeltConfig { speed: 100.0, ..BeltConfig::default() };
/// let mut belt = Belt::new(config, Duration::ZERO);
/// let mut n = 0;
/// let mut source = |_now| {
///     n += 1;
///     Some(BeltItem::new(n, 50.0))
/// };
///
/// belt.update(Duration::ZERO, 200.0, &mut source);
/// // New content enters from the trailing edge of the viewport.
/// assert_eq!(belt.visible(200.0).count(), 0);
///
/// belt.update(Duration::from_secs(1), 200.0, &mut source);
/// let first = belt.visible(200.0).next().unwrap();
/// assert_eq!((*first.content, first.position), (1, 100.0));
/// ```
#[derive(Debug, Clone)]
pub struct Belt<T> {
    config: BeltConfig,
    items: VecDeque<Placed<T>>,
    /// Offset at `anchor_time`; offset grows linearly from there.
    anchor_offset: f64,
    anchor_time: Duration,
    /// Offset as of the last `update`.
    offset: f64,
    /// Where the next item may start (end of the last item plus the gap).
    next_start: f64,
    /// The source ran out (or nothing was placed yet), so the next item should
    /// scroll in from the trailing edge.
    dry: bool,
}

impl<T> Belt<T> {
    /// Creates an empty belt whose offset is zero at monotonic time `now`.
    #[must_use]
    pub fn new(config: BeltConfig, now: Duration) -> Self {
        Self {
            config,
            items: VecDeque::new(),
            anchor_offset: 0.0,
            anchor_time: now,
            offset: 0.0,
            next_start: 0.0,
            dry: true,
        }
    }

    /// The current settings.
    #[must_use]
    pub fn config(&self) -> &BeltConfig {
        &self.config
    }

    fn speed(&self) -> f64 {
        let s = self.config.speed;
        if s.is_finite() && s > 0.0 { s } else { 0.0 }
    }

    fn offset_at(&self, now: Duration) -> f64 {
        let elapsed = now.saturating_sub(self.anchor_time).as_secs_f64();
        self.anchor_offset + self.speed() * elapsed
    }

    /// Changes the speed at time `now` without making the belt jump.
    pub fn set_speed(&mut self, speed: f64, now: Duration) {
        self.anchor_offset = self.offset_at(now).max(self.anchor_offset);
        self.anchor_time = now.max(self.anchor_time);
        self.config.speed = speed;
    }

    /// Changes the gap used for items placed from now on.
    pub fn set_gap(&mut self, gap: f64) {
        self.config.gap = gap;
    }

    /// Changes the snapping mode.
    pub fn set_snap(&mut self, snap: Snap) {
        self.config.snap = snap;
    }

    /// The belt coordinate at the viewport's leading edge, after snapping, as
    /// of the last [`update`](Self::update).
    #[must_use]
    pub fn offset(&self) -> f64 {
        match self.config.snap {
            Snap::Pixels(scale) if scale.is_finite() && scale > 0.0 => {
                (self.offset * scale).round() / scale
            }
            _ => self.offset,
        }
    }

    /// Advances to monotonic time `now`: drops items that have scrolled out and
    /// pulls new ones from `source` until the belt reaches `viewport_len +
    /// lookahead` past the leading edge.
    ///
    /// Two cases decide where a new item starts:
    ///
    /// - After the source ran dry (or at the very start), the new item starts
    ///   at the trailing edge of the viewport and scrolls in, rather than
    ///   popping up in the middle.
    /// - When updates simply came late (a long frame, or the machine slept),
    ///   the belt continues seamlessly, skipping only the stretch that has
    ///   already scrolled past, so the viewport is full again at once.
    pub fn update<S: BeltSource<T> + ?Sized>(
        &mut self,
        now: Duration,
        viewport_len: f64,
        source: &mut S,
    ) {
        // Time never runs backwards for the belt.
        self.offset = self.offset_at(now).max(self.offset);
        let viewport_len = sanitize(viewport_len);
        let leading = self.offset();

        while self.items.front().is_some_and(|item| item.end() <= leading) {
            self.items.pop_front();
        }

        let trailing = leading + viewport_len;
        let fill_to = trailing + sanitize(self.config.lookahead);
        while self.next_start < fill_to {
            let Some(item) = source.next_item(now) else {
                // Dry only if nothing queued is still waiting to scroll in.
                self.dry = self.next_start <= trailing;
                break;
            };
            let start = if self.dry {
                self.next_start.max(trailing)
            } else {
                self.next_start.max(leading)
            };
            self.dry = false;
            let width = sanitize(item.width);
            self.next_start = start + width + sanitize(self.config.gap);
            self.items.push_back(Placed {
                content: item.content,
                start,
                width,
            });
            if width == 0.0 && sanitize(self.config.gap) == 0.0 {
                // Zero-width items with no gap would never fill the belt.
                break;
            }
        }
    }

    /// Items that intersect a viewport of `viewport_len`, from the leading edge
    /// backwards.
    pub fn visible(&self, viewport_len: f64) -> impl Iterator<Item = VisibleItem<'_, T>> {
        let leading = self.offset();
        let trailing = leading + sanitize(viewport_len);
        self.items
            .iter()
            .take_while(move |item| item.start < trailing)
            .filter(move |item| item.end() > leading)
            .map(move |item| VisibleItem {
                content: &item.content,
                position: item.start - leading,
                width: item.width,
            })
    }

    /// Every item on the belt, visible or queued, in order.
    pub fn items(&self) -> impl ExactSizeIterator<Item = &T> {
        self.items.iter().map(|item| &item.content)
    }

    /// Removes all items; new content enters from the trailing edge.
    pub fn clear(&mut self) {
        self.items.clear();
        self.next_start = self.offset;
        self.dry = true;
    }
}

fn sanitize(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use stocktape_core::{Clock, ManualClock, Timestamp};

    fn counter(width: f64) -> impl FnMut(Duration) -> Option<BeltItem<u32>> {
        let mut n = 0;
        move |_| {
            n += 1;
            Some(BeltItem::new(n, width))
        }
    }

    fn config(speed: f64, gap: f64) -> BeltConfig {
        BeltConfig {
            speed,
            gap,
            lookahead: 0.0,
            snap: Snap::None,
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn position_follows_time_not_frames() {
        let mut a = Belt::new(config(120.0, 0.0), Duration::ZERO);
        let mut b = Belt::new(config(120.0, 0.0), Duration::ZERO);
        let mut sa = counter(40.0);
        let mut sb = counter(40.0);
        // `a` renders at 60 fps, `b` drops to one frame at the end.
        for frame in 0..=60 {
            a.update(
                Duration::from_secs_f64(f64::from(frame) / 60.0),
                300.0,
                &mut sa,
            );
        }
        b.update(Duration::from_secs(1), 300.0, &mut sb);
        assert!((a.offset() - 120.0).abs() < 1e-9);
        assert!((b.offset() - 120.0).abs() < 1e-9);
    }

    #[test]
    fn works_with_manual_clock() {
        let clock = ManualClock::new(Timestamp::UNIX_EPOCH);
        let mut belt = Belt::new(config(60.0, 0.0), clock.monotonic());
        let mut source = counter(30.0);
        clock.advance(ms(500));
        belt.update(clock.monotonic(), 100.0, &mut source);
        assert_eq!(belt.offset(), 30.0);
    }

    #[test]
    fn items_are_laid_end_to_end_with_gap() {
        let mut belt = Belt::new(config(100.0, 10.0), Duration::ZERO);
        let mut source = counter(50.0);
        for step in 0..=20 {
            belt.update(ms(step * 100), 100.0, &mut source);
        }
        let visible: Vec<_> = belt
            .visible(100.0)
            .map(|v| (*v.content, v.position))
            .collect();
        // Item 1 starts at belt 100 (the trailing edge at t=0); offset is 200.
        assert_eq!(visible, [(2, -40.0), (3, 20.0), (4, 80.0)]);
    }

    #[test]
    fn items_leave_once_fully_scrolled_out() {
        let mut belt = Belt::new(config(100.0, 0.0), Duration::ZERO);
        let mut source = counter(50.0);
        for step in 0..=100 {
            belt.update(ms(step * 100), 100.0, &mut source);
        }
        // Offset 1000: items 1..=18 (belt 100..1000) have scrolled out.
        assert!(belt.items().all(|&n| n > 18));
        assert!(belt.items().len() <= 3);
        assert!(belt.visible(100.0).all(|v| v.position + v.width > 0.0));
    }

    #[test]
    fn speed_change_is_continuous() {
        let mut belt = Belt::new(config(100.0, 0.0), Duration::ZERO);
        let mut source = counter(50.0);
        belt.update(Duration::from_secs(1), 100.0, &mut source);
        assert_eq!(belt.offset(), 100.0);
        belt.set_speed(10.0, Duration::from_secs(1));
        belt.update(Duration::from_secs(1), 100.0, &mut source);
        assert_eq!(belt.offset(), 100.0);
        belt.update(Duration::from_secs(3), 100.0, &mut source);
        assert_eq!(belt.offset(), 120.0);
    }

    #[test]
    fn zero_speed_pauses() {
        let mut belt = Belt::new(config(0.0, 0.0), Duration::ZERO);
        let mut source = counter(50.0);
        belt.update(Duration::from_secs(60), 100.0, &mut source);
        assert_eq!(belt.offset(), 0.0);
        belt.set_speed(f64::NAN, Duration::from_secs(60));
        belt.update(Duration::from_secs(61), 100.0, &mut source);
        assert_eq!(belt.offset(), 0.0);
    }

    #[test]
    fn time_going_backwards_does_not_rewind() {
        let mut belt = Belt::new(config(100.0, 0.0), Duration::from_secs(5));
        let mut source = counter(50.0);
        belt.update(Duration::from_secs(6), 100.0, &mut source);
        belt.update(Duration::from_secs(1), 100.0, &mut source);
        assert_eq!(belt.offset(), 100.0);
    }

    #[test]
    fn late_content_enters_from_trailing_edge() {
        let mut belt = Belt::new(config(100.0, 0.0), Duration::ZERO);
        let mut empty = |_| None::<BeltItem<u32>>;
        belt.update(Duration::from_secs(5), 300.0, &mut empty);
        assert_eq!(belt.items().len(), 0);
        let mut source = counter(50.0);
        belt.update(Duration::from_secs(5), 300.0, &mut source);
        assert_eq!(belt.visible(300.0).count(), 0);
        belt.update(Duration::from_millis(5_500), 300.0, &mut source);
        let first = belt.visible(300.0).next().unwrap();
        assert_eq!((*first.content, first.position), (1, 250.0));
    }

    #[test]
    fn late_update_refills_viewport_at_once() {
        let mut belt = Belt::new(config(100.0, 0.0), Duration::ZERO);
        let mut source = counter(50.0);
        belt.update(Duration::ZERO, 300.0, &mut source);
        belt.update(ms(100), 300.0, &mut source);
        // The machine sleeps for an hour.
        belt.update(Duration::from_secs(3_600), 300.0, &mut source);
        let visible: Vec<_> = belt.visible(300.0).map(|v| v.position).collect();
        assert_eq!(visible.first().copied(), Some(0.0));
        assert_eq!(visible.len(), 6);
        // Only a handful of items were pulled, not one per skipped position.
        assert!(belt.items().all(|&n| n < 20));
    }

    #[test]
    fn lookahead_prefetches() {
        let cfg = BeltConfig {
            lookahead: 200.0,
            ..config(100.0, 0.0)
        };
        let mut belt = Belt::new(cfg, Duration::ZERO);
        let mut source = counter(50.0);
        belt.update(Duration::ZERO, 100.0, &mut source);
        // Items placed from the trailing edge (100) out to 300.
        assert_eq!(belt.items().len(), 4);
    }

    #[test]
    fn pixel_snapping_rounds_the_offset() {
        let cfg = BeltConfig {
            snap: Snap::Pixels(2.0),
            ..config(1.0, 0.0)
        };
        let mut belt = Belt::new(cfg, Duration::ZERO);
        let mut source = counter(50.0);
        belt.update(ms(1_200), 100.0, &mut source);
        assert_eq!(belt.offset(), 1.0);
        belt.update(ms(1_300), 100.0, &mut source);
        assert_eq!(belt.offset(), 1.5);
    }

    #[test]
    fn zero_width_items_do_not_spin() {
        let mut belt = Belt::new(config(100.0, 0.0), Duration::ZERO);
        let mut source = counter(0.0);
        belt.update(Duration::ZERO, 100.0, &mut source);
        assert!(belt.items().len() <= 1);
        let mut bad = counter(f64::NAN);
        belt.update(Duration::ZERO, 100.0, &mut bad);
    }

    #[test]
    fn clear_restarts_from_trailing_edge() {
        let mut belt = Belt::new(config(100.0, 0.0), Duration::ZERO);
        let mut source = counter(50.0);
        belt.update(Duration::from_secs(2), 100.0, &mut source);
        belt.clear();
        belt.update(Duration::from_secs(2), 100.0, &mut source);
        assert_eq!(belt.visible(100.0).count(), 0);
    }

    proptest! {
        /// With an endless source, the visible area is always covered by
        /// ordered, non-overlapping items separated by exactly the gap, after
        /// the first item has scrolled in.
        #[test]
        fn belt_invariants(
            speed in 1.0f64..2_000.0,
            gap in 0.0f64..40.0,
            widths in proptest::collection::vec(1.0f64..400.0, 1..20),
            steps in proptest::collection::vec(0u64..100, 1..200),
            viewport in 50.0f64..4_000.0,
        ) {
            let mut belt = Belt::new(config(speed, gap), Duration::ZERO);
            let mut i = 0usize;
            let mut source = |_| {
                let w = widths[i % widths.len()];
                i += 1;
                Some(BeltItem::new(i, w))
            };
            let mut now = Duration::ZERO;
            for step in steps {
                now += ms(step);
                belt.update(now, viewport, &mut source);
                let visible: Vec<_> = belt.visible(viewport).collect();
                for pair in visible.windows(2) {
                    let expected = pair[0].position + pair[0].width + gap;
                    prop_assert!((pair[1].position - expected).abs() < 1e-6);
                    prop_assert!(pair[0].content < pair[1].content);
                }
                if let Some(last) = visible.last() {
                    prop_assert!(last.position < viewport);
                    // Content reaches the trailing edge once the belt is primed.
                    if belt.offset() >= viewport + 400.0 + gap {
                        prop_assert!(last.position + last.width + gap >= viewport - 1e-6);
                    }
                }
                if let Some(first) = visible.first() {
                    prop_assert!(first.position + first.width > 0.0);
                }
            }
        }
    }
}
