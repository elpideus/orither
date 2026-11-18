//! Fits the phone's video to the link while it plays. The phone's scrcpy server takes a
//! new bit rate on the fly (Orither's patch, android/scrcpy), so the window lowers it when
//! the phone had to drop pictures or pictures keep coming late, and raises it again, a step
//! at a time, while they come on time. A link that carries less then gets a softer picture
//! instead of a stuttering one.

use std::time::{Duration, Instant};

/// What a drop leaves of the bit rate.
const AFTER_DROP: f64 = 0.6;
/// What pictures that keep coming late leave of it.
const AFTER_LATE: f64 = 0.8;
/// How much it grows each time the pictures have come on time for a while.
const STEP_UP: f64 = 1.2;
/// Pictures this much later than the best seen count as late.
const LATE: Duration = Duration::from_millis(300);
/// How long pictures have to keep coming late before the bit rate comes down.
const LATE_FOR: Duration = Duration::from_secs(1);
/// How long between two cuts, so that one cut can take effect before the next.
const CUTS_APART: Duration = Duration::from_secs(1);
/// How long pictures have to come on time before the bit rate goes up a step.
const ON_TIME_FOR: Duration = Duration::from_secs(5);
/// How long after a drop the bit rate stays where the drop left it.
const AFTER_DROP_WAIT: Duration = Duration::from_secs(10);
/// The least it goes down to, unless the route started lower.
const FLOOR: u32 = 500_000;

pub struct BitRate {
    now: u32,
    top: u32,
    floor: u32,
    changed: Instant,
    dropped: Option<Instant>,
    late_since: Option<Instant>,
}

impl BitRate {
    /// Starts at `top`, the route's bit rate, which it never goes above. None when the
    /// phone cannot change it.
    pub fn new(top: u32, now: Instant) -> Option<Self> {
        (top > 0).then(|| Self { now: top, top, floor: FLOOR.min(top), changed: now, dropped: None, late_since: None })
    }

    #[cfg(test)]
    pub fn current(&self) -> u32 {
        self.now
    }

    /// The phone dropped pictures to keep up. Returns the bit rate to set, if it changes.
    pub fn dropped(&mut self, now: Instant) -> Option<u32> {
        self.dropped = Some(now);
        if now.duration_since(self.changed) < CUTS_APART {
            return None;
        }
        self.set(self.now as f64 * AFTER_DROP, now)
    }

    /// A picture came `late` after the best seen. Returns the bit rate to set, if it changes.
    pub fn arrived(&mut self, late: Duration, now: Instant) -> Option<u32> {
        if late > LATE {
            let since = *self.late_since.get_or_insert(now);
            if now.duration_since(since) >= LATE_FOR && now.duration_since(self.changed) >= CUTS_APART {
                self.late_since = None;
                return self.set(self.now as f64 * AFTER_LATE, now);
            }
            return None;
        }
        self.late_since = None;
        let calm = self.dropped.is_none_or(|dropped| now.duration_since(dropped) >= AFTER_DROP_WAIT);
        if calm && self.now < self.top && now.duration_since(self.changed) >= ON_TIME_FOR {
            return self.set(self.now as f64 * STEP_UP, now);
        }
        None
    }

    fn set(&mut self, wanted: f64, now: Instant) -> Option<u32> {
        let next = (wanted as u32).clamp(self.floor, self.top);
        self.changed = now;
        (next != self.now).then(|| {
            self.now = next;
            next
        })
    }
}

/// How much later than the best one so far a picture arrives, by the phone's clock.
#[derive(Default)]
pub struct Lateness {
    first: Option<(Instant, u64)>,
    best: Option<i64>,
    last_pts: u64,
}

impl Lateness {
    pub fn arrived(&mut self, pts: u64, now: Instant) -> Duration {
        // A clock that went back started again, and so does the count.
        if pts < self.last_pts {
            (self.first, self.best) = (None, None);
        }
        self.last_pts = pts;
        let (start, start_pts) = *self.first.get_or_insert((now, pts));
        let late = now.duration_since(start).as_micros() as i64 - (pts as i64 - start_pts as i64);
        let best = *self.best.insert(self.best.map_or(late, |best| best.min(late)));
        Duration::from_micros((late - best).max(0) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    #[test]
    fn a_phone_that_cannot_change_it_needs_nothing() {
        assert!(BitRate::new(0, Instant::now()).is_none());
    }

    #[test]
    fn drops_cut_the_bit_rate_once_a_second_down_to_a_floor() {
        let start = Instant::now();
        let mut rate = BitRate::new(6_000_000, start).unwrap();
        assert_eq!(rate.dropped(start + S), Some(3_600_000));
        assert_eq!(rate.dropped(start + S + S / 2), None);
        assert_eq!(rate.dropped(start + 2 * S), Some(2_160_000));
        for second in 3..20 {
            rate.dropped(start + second * S);
        }
        assert_eq!(rate.current(), FLOOR);
    }

    #[test]
    fn pictures_that_keep_coming_late_cut_it_too() {
        let start = Instant::now();
        let mut rate = BitRate::new(4_000_000, start).unwrap();
        let late = Duration::from_millis(500);
        assert_eq!(rate.arrived(late, start + 2 * S), None);
        assert_eq!(rate.arrived(late, start + 2 * S + S / 2), None);
        assert_eq!(rate.arrived(late, start + 3 * S), Some(3_200_000));
        // A late moment that passes cuts nothing.
        assert_eq!(rate.arrived(late, start + 5 * S), None);
        assert_eq!(rate.arrived(Duration::ZERO, start + 5 * S + S / 10), None);
        assert_eq!(rate.arrived(late, start + 6 * S), None);
    }

    #[test]
    fn it_climbs_back_while_pictures_come_on_time() {
        let start = Instant::now();
        let mut rate = BitRate::new(6_000_000, start).unwrap();
        rate.dropped(start + S);
        rate.dropped(start + 2 * S);
        let low = rate.current();
        // Not while the drop is recent.
        assert_eq!(rate.arrived(Duration::ZERO, start + 8 * S), None);
        let up = rate.arrived(Duration::ZERO, start + 12 * S).expect("a step up");
        assert_eq!(up, (low as f64 * STEP_UP) as u32);
        assert_eq!(rate.arrived(Duration::ZERO, start + 13 * S), None);
        let mut now = start + 13 * S;
        for _ in 0..30 {
            now += 5 * S;
            rate.arrived(Duration::ZERO, now);
        }
        assert_eq!(rate.current(), 6_000_000);
    }

    #[test]
    fn lateness_counts_from_the_best_picture() {
        let start = Instant::now();
        let mut lateness = Lateness::default();
        assert_eq!(lateness.arrived(0, start), Duration::ZERO);
        assert_eq!(lateness.arrived(16_000, start + Duration::from_millis(16)), Duration::ZERO);
        assert_eq!(lateness.arrived(32_000, start + Duration::from_millis(232)), Duration::from_millis(200));
        // The phone's clock started again.
        assert_eq!(lateness.arrived(5_000, start + Duration::from_millis(300)), Duration::ZERO);
    }
}
