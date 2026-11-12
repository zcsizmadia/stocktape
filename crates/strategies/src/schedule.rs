//! Switching strategy by local time of day.

use jiff::civil::{Time, Weekday};
use jiff::tz::TimeZone;

use crate::{Context, Item, Strategy};

/// A time window and the strategy used during it.
pub struct Slot {
    start: Time,
    end: Time,
    weekdays: Option<Vec<Weekday>>,
    strategy: Box<dyn Strategy>,
}

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slot")
            .field("start", &self.start)
            .field("end", &self.end)
            .field("weekdays", &self.weekdays)
            .finish_non_exhaustive()
    }
}

impl Slot {
    /// Active from `start` (inclusive) to `end` (exclusive), local time. If
    /// `end` is before `start` the window runs past midnight.
    pub fn new(start: Time, end: Time, strategy: impl Strategy + 'static) -> Self {
        Self {
            start,
            end,
            weekdays: None,
            strategy: Box::new(strategy),
        }
    }

    /// Only on these days (by the day the window starts... simply the local
    /// weekday at the moment of asking).
    #[must_use]
    pub fn on(mut self, weekdays: impl IntoIterator<Item = Weekday>) -> Self {
        self.weekdays = Some(weekdays.into_iter().collect());
        self
    }

    fn contains(&self, time: Time, weekday: Weekday) -> bool {
        let in_window = if self.start <= self.end {
            self.start <= time && time < self.end
        } else {
            time >= self.start || time < self.end
        };
        in_window
            && self
                .weekdays
                .as_ref()
                .is_none_or(|days| days.contains(&weekday))
    }
}

/// Uses the first [`Slot`] whose window contains the current local time, or a
/// fallback outside all slots. Typical use: futures and movers pre-market, the
/// watchlist during the session, earnings after hours.
pub struct Schedule {
    tz: TimeZone,
    slots: Vec<Slot>,
    fallback: Option<Box<dyn Strategy>>,
}

impl std::fmt::Debug for Schedule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Schedule")
            .field("tz", &self.tz)
            .field("slots", &self.slots)
            .field("fallback", &self.fallback.is_some())
            .finish()
    }
}

impl Schedule {
    /// A schedule in time zone `tz` (e.g. `America/New_York`).
    #[must_use]
    pub fn new(tz: TimeZone) -> Self {
        Self {
            tz,
            slots: Vec::new(),
            fallback: None,
        }
    }

    /// Adds a slot; earlier slots win when windows overlap.
    #[must_use]
    pub fn slot(mut self, slot: Slot) -> Self {
        self.slots.push(slot);
        self
    }

    /// The strategy used outside every slot. Without one, nothing is shown
    /// then.
    #[must_use]
    pub fn otherwise(mut self, strategy: impl Strategy + 'static) -> Self {
        self.fallback = Some(Box::new(strategy));
        self
    }
}

impl Strategy for Schedule {
    fn next(&mut self, ctx: &Context<'_>) -> Option<Item> {
        let local = ctx.wall.to_zoned(self.tz.clone());
        let (time, weekday) = (local.time(), local.weekday());
        match self.slots.iter_mut().find(|s| s.contains(time, weekday)) {
            Some(slot) => slot.strategy.next(ctx),
            None => self.fallback.as_mut()?.next(ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{store, sym};
    use crate::{Context, Watchlist};
    use jiff::civil::time;
    use jiff::tz::offset;
    use std::time::Duration;
    use stocktape_core::Timestamp;

    /// 2026-11-12 (a Thursday) at `h:m` in UTC-5.
    fn at(h: i64, m: i64) -> Timestamp {
        Timestamp::from_second(1_794_459_600 + h * 3600 + m * 60).unwrap()
    }

    fn name(s: &mut Schedule, wall: Timestamp) -> Option<String> {
        let store = store(&[
            ("PRE", 1.0, 1.0),
            ("DAY", 1.0, 1.0),
            ("OFF", 1.0, 1.0),
            ("WKD", 1.0, 1.0),
        ]);
        let ctx = Context {
            store: &store,
            now: Duration::ZERO,
            wall,
            headlines: &[],
        };
        s.next(&ctx)
            .and_then(|i| i.symbol().map(ToString::to_string))
    }

    fn schedule() -> Schedule {
        Schedule::new(TimeZone::fixed(offset(-5)))
            .slot(Slot::new(
                time(4, 0, 0, 0),
                time(9, 30, 0, 0),
                Watchlist::new([sym("PRE")]),
            ))
            .slot(
                Slot::new(
                    time(9, 30, 0, 0),
                    time(16, 0, 0, 0),
                    Watchlist::new([sym("DAY")]),
                )
                .on([
                    Weekday::Monday,
                    Weekday::Tuesday,
                    Weekday::Wednesday,
                    Weekday::Thursday,
                    Weekday::Friday,
                ]),
            )
            .otherwise(Watchlist::new([sym("OFF")]))
    }

    #[test]
    fn local_time_picks_the_slot() {
        let mut s = schedule();
        assert_eq!(name(&mut s, at(0, 0)).unwrap(), "OFF");
        assert_eq!(name(&mut s, at(4, 0)).unwrap(), "PRE");
        assert_eq!(name(&mut s, at(9, 29)).unwrap(), "PRE");
        assert_eq!(name(&mut s, at(9, 30)).unwrap(), "DAY");
        assert_eq!(name(&mut s, at(15, 59)).unwrap(), "DAY");
        assert_eq!(name(&mut s, at(16, 0)).unwrap(), "OFF");
    }

    #[test]
    fn weekdays_and_overnight_windows() {
        let mut s = schedule();
        // Saturday 10:00 local: the session slot is weekdays only.
        assert_eq!(name(&mut s, at(48 + 10, 0)).unwrap(), "OFF");
        let mut night = Schedule::new(TimeZone::fixed(offset(-5))).slot(Slot::new(
            time(20, 0, 0, 0),
            time(4, 0, 0, 0),
            Watchlist::new([sym("WKD")]),
        ));
        assert_eq!(name(&mut night, at(23, 0)).as_deref(), Some("WKD"));
        assert_eq!(name(&mut night, at(24 + 3, 0)).as_deref(), Some("WKD"));
        assert_eq!(name(&mut night, at(12, 0)), None);
    }
}
