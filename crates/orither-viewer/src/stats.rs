//! Numbers about a video stream, to find out where it lags. Silent unless the
//! environment sets `ORITHER_VIDEO_STATS`; then a line goes to stderr every few
//! seconds.

use std::{
    env,
    time::{Duration, Instant},
};

const EVERY: Duration = Duration::from_secs(2);

pub struct Stats {
    label: &'static str,
    on: bool,
    since: Instant,
    frames: u32,
    keys: u32,
    bytes: usize,
    decoding: Duration,
    slowest: Duration,
    /// When the first picture arrived, and the phone's time for it.
    first: Option<(Instant, u64)>,
    /// How much later than the phone's clock says pictures arrive, at best so far.
    best_late: Option<i64>,
    worst_late: i64,
    late: i64,
}

impl Stats {
    pub fn new(label: &'static str) -> Self {
        Self {
            label,
            on: env::var_os("ORITHER_VIDEO_STATS").is_some(),
            since: Instant::now(),
            frames: 0,
            keys: 0,
            bytes: 0,
            decoding: Duration::ZERO,
            slowest: Duration::ZERO,
            first: None,
            best_late: None,
            worst_late: 0,
            late: 0,
        }
    }

    /// A picture taken at `pts` (microseconds, the phone's clock) arrived now.
    pub fn arrived(&mut self, pts: u64, key: bool, bytes: usize) {
        if !self.on {
            return;
        }
        let now = Instant::now();
        let (start, start_pts) = *self.first.get_or_insert((now, pts));
        let elapsed = now.duration_since(start).as_micros() as i64;
        let late = elapsed - (pts as i64 - start_pts as i64);
        let best = *self.best_late.insert(self.best_late.map_or(late, |best| best.min(late)));
        self.late = late - best;
        self.worst_late = self.worst_late.max(self.late);
        self.frames += 1;
        self.keys += u32::from(key);
        self.bytes += bytes;
    }

    /// Says something that happened to the video, among the numbers.
    pub fn note(&self, what: &str) {
        if self.on {
            eprintln!("video {}: {what}", self.label);
        }
    }

    pub fn decoded(&mut self, took: Duration) {
        if !self.on {
            return;
        }
        self.decoding += took;
        self.slowest = self.slowest.max(took);
        let seconds = self.since.elapsed();
        if seconds < EVERY {
            return;
        }
        let per_second = |count: f64| count / seconds.as_secs_f64();
        let average = self.decoding.as_secs_f64() * 1000.0 / f64::from(self.frames.max(1));
        eprintln!(
            "video {}: {:.0} fps ({} key), {:.0} kbit/s, decoding {average:.1} ms (at most {:.1}), behind {} ms (at most {})",
            self.label,
            per_second(f64::from(self.frames)),
            self.keys,
            per_second(self.bytes as f64 * 8.0 / 1000.0),
            self.slowest.as_secs_f64() * 1000.0,
            self.late / 1000,
            self.worst_late / 1000,
        );
        (self.since, self.frames, self.keys, self.bytes, self.decoding, self.slowest, self.worst_late) =
            (Instant::now(), 0, 0, 0, Duration::ZERO, Duration::ZERO, 0);
    }
}

/// How often the window draws a new picture, and how long that takes.
pub struct Drawing {
    on: bool,
    since: Instant,
    frames: u32,
    busy: Duration,
    slowest: Duration,
}

impl Drawing {
    pub fn new() -> Self {
        let on = env::var_os("ORITHER_VIDEO_STATS").is_some();
        Self { on, since: Instant::now(), frames: 0, busy: Duration::ZERO, slowest: Duration::ZERO }
    }

    pub fn drew(&mut self, took: Duration) {
        if !self.on {
            return;
        }
        self.frames += 1;
        self.busy += took;
        self.slowest = self.slowest.max(took);
        let seconds = self.since.elapsed();
        if seconds < EVERY {
            return;
        }
        eprintln!(
            "video drawn: {:.0} fps, drawing {:.1} ms (at most {:.1})",
            f64::from(self.frames) / seconds.as_secs_f64(),
            self.busy.as_secs_f64() * 1000.0 / f64::from(self.frames),
            self.slowest.as_secs_f64() * 1000.0,
        );
        (self.since, self.frames, self.busy, self.slowest) = (Instant::now(), 0, Duration::ZERO, Duration::ZERO);
    }
}
