//! How far the picture of the battlefield trails the camera (plans/battlefield-latency.md, step 0).
//!
//! The app asks for a picture each time the camera changes, each request with the next `seq`, and
//! each picture comes back with the `seq` it was rendered for. While the camera moves, each paint
//! of a picture of the motion says how old it is: how long ago the app asked for the camera it
//! shows. The picture from before the motion is not counted; the wait for the motion's first one
//! is kept apart. A motion ends when the camera has not changed for
//! [`STILL`]; its numbers are kept until the next one starts.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// The camera has stopped when it has not changed for this long.
pub const STILL: Duration = Duration::from_millis(250);
/// How many requests are remembered (more than a second of them at 60 a second).
const KEPT: usize = 240;

/// The numbers of one motion of the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    /// How long the motion lasted (s): from its first request to its last.
    pub secs: f32,
    /// The pictures that came while it lasted.
    pub pictures: usize,
    pub pictures_per_s: f32,
    /// How far the picture on screen trailed the camera, over the paints of the motion (ms):
    /// the median and the 90th percentile.
    pub lag_ms: f32,
    pub lag_p90_ms: f32,
    /// From the motion's first request to the first picture of it on screen (ms), if one came.
    pub first_ms: Option<f32>,
}

#[derive(Default)]
struct Run {
    /// The requests in it: one alone (the camera placed, a click on the minimap) is a jump, not a
    /// motion, and is not measured.
    requests: usize,
    from: Option<Instant>,
    to: Option<Instant>,
    pictures: usize,
    lags: Vec<f32>,
    first_ms: Option<f32>,
}

impl Run {
    fn motion(&self) -> Option<Motion> {
        if self.requests < 2 {
            return None;
        }
        let (from, to) = (self.from?, self.to?);
        // A motion of one request still lasts a frame.
        let secs = (to - from).as_secs_f32().max(1.0 / 60.0);
        let mut lags = self.lags.clone();
        lags.sort_by(f32::total_cmp);
        let at = |q: f32| lags.get(((lags.len() as f32 - 1.0) * q).round() as usize).copied().unwrap_or(0.0);
        Some(Motion {
            secs,
            pictures: self.pictures,
            pictures_per_s: self.pictures as f32 / secs,
            lag_ms: at(0.5),
            lag_p90_ms: at(0.9),
            first_ms: self.first_ms,
        })
    }
}

/// How far behind the camera the picture is, motion by motion.
#[derive(Default)]
pub struct Lag {
    asked: VecDeque<(u32, Instant)>,
    run: Run,
    last: Option<Motion>,
}

impl Lag {
    /// The app asked for camera `seq` at `now`.
    pub fn asked(&mut self, seq: u32, now: Instant) {
        self.finish_if_still(now);
        self.asked.push_back((seq, now));
        if self.asked.len() > KEPT {
            self.asked.pop_front();
        }
        self.run.requests += 1;
        self.run.from.get_or_insert(now);
        self.run.to = Some(now);
    }

    /// A picture of camera `seq` came at `now`.
    pub fn arrived(&mut self, _seq: u32, now: Instant) {
        if self.moving(now) {
            self.run.pictures += 1;
        }
    }

    /// The picture of camera `seq` is on screen at `now`. Returns the motion that has just ended,
    /// if one has, so that the app can log it.
    pub fn painted(&mut self, seq: u32, now: Instant) -> Option<Motion> {
        let ended = self.finish_if_still(now);
        if self.moving(now) {
            // As old as the camera it shows, if that camera is one of this motion's.
            let from = self.run.from;
            let shown = self.asked.iter().find(|(s, at)| *s == seq && from.is_some_and(|f| *at >= f));
            if let (Some(&(_, at)), Some(from)) = (shown, from) {
                self.run.lags.push(ms(now - at));
                self.run.first_ms.get_or_insert(ms(now - from));
            }
        }
        ended
    }

    /// The motion now, or the last one if the camera is still.
    pub fn motion(&mut self, now: Instant) -> Option<Motion> {
        self.finish_if_still(now);
        self.run.motion().or(self.last)
    }

    fn moving(&self, now: Instant) -> bool {
        self.run.to.is_some_and(|to| now.saturating_duration_since(to) < STILL)
    }

    fn finish_if_still(&mut self, now: Instant) -> Option<Motion> {
        if self.run.to.is_none() || self.moving(now) {
            return None;
        }
        let done = std::mem::take(&mut self.run).motion();
        self.last = done.or(self.last);
        done
    }
}

fn ms(d: Duration) -> f32 {
    (d.as_secs_f64() * 1000.0).round() as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_picture_is_as_old_as_the_camera_it_shows() {
        let t0 = Instant::now();
        let mut lag = Lag::default();
        lag.asked(1, t0);
        lag.asked(2, t0 + ms(16));
        lag.asked(3, t0 + ms(33));
        // The picture of camera 1, asked for 50 ms ago.
        lag.painted(1, t0 + ms(50));
        assert_eq!(lag.motion(t0 + ms(50)).unwrap().lag_ms, 50.0);
    }

    #[test]
    fn a_steady_delay_reads_the_same_on_a_short_pan_and_a_long_one() {
        // Each picture is painted 500 ms after its camera was asked for, the app asking 30 times
        // a second and painting each time it asks.
        let pan = |secs: u64| {
            let t0 = Instant::now();
            let mut lag = Lag::default();
            let n = (secs * 30 / 1000) as u32;
            for k in 0..n {
                let now = t0 + ms(33 * k as u64);
                lag.asked(k + 1, now);
                // The newest picture painted: of the camera asked for 500 ms ago (15 asks).
                if k >= 15 {
                    lag.painted(k + 1 - 15, now);
                }
            }
            lag.motion(t0 + ms(33 * n as u64)).unwrap().lag_ms
        };
        let (short, long) = (pan(700), pan(3000));
        assert!((short - 495.0).abs() < 10.0, "short pan: {short}");
        assert!((long - 495.0).abs() < 10.0, "long pan: {long}");
    }

    #[test]
    fn the_picture_from_before_the_motion_is_not_counted_and_the_wait_for_the_first_new_one_is() {
        let t0 = Instant::now();
        let mut lag = Lag::default();
        // The camera placed; its picture on screen.
        lag.asked(1, t0);
        lag.painted(1, t0 + ms(100));
        // A pan starts a second later; the old picture stays on screen for a while.
        for k in 0..10u32 {
            let now = t0 + ms(1000 + 33 * k as u64);
            lag.asked(2 + k, now);
            lag.painted(1, now);
        }
        // The first picture of the pan, 300 ms after it began.
        lag.arrived(2, t0 + ms(1299));
        lag.painted(2, t0 + ms(1300));
        let m = lag.motion(t0 + ms(1300)).unwrap();
        assert_eq!(m.lag_ms, 300.0, "only the pan's own picture counts");
        assert_eq!(m.first_ms, Some(300.0));
    }

    #[test]
    fn a_motion_counts_the_new_pictures_a_second() {
        let t0 = Instant::now();
        let mut lag = Lag::default();
        for k in 0..=30u32 {
            lag.asked(k + 1, t0 + ms(33 * k as u64));
            if k % 2 == 0 {
                lag.arrived(k + 1, t0 + ms(33 * k as u64 + 5));
            }
        }
        let m = lag.motion(t0 + ms(1000)).unwrap();
        // 16 pictures in about a second.
        assert!((m.pictures_per_s - 16.0).abs() < 1.0, "{}", m.pictures_per_s);
    }

    #[test]
    fn a_camera_that_stops_ends_the_motion_and_the_next_one_starts_afresh() {
        let t0 = Instant::now();
        let mut lag = Lag::default();
        lag.asked(1, t0);
        lag.asked(2, t0 + ms(20));
        lag.painted(1, t0 + ms(40));
        // Still for a second: the motion is over, and is what motion() says.
        let done = lag.motion(t0 + ms(1040)).unwrap();
        assert_eq!(done.lag_ms, 40.0);
        // A new motion: its own numbers only.
        lag.asked(3, t0 + ms(2000));
        lag.asked(4, t0 + ms(2016));
        lag.painted(4, t0 + ms(2030));
        assert_eq!(lag.motion(t0 + ms(2030)).unwrap().lag_ms, 14.0);
    }

    #[test]
    fn a_single_change_of_the_camera_is_not_a_motion() {
        let t0 = Instant::now();
        let mut lag = Lag::default();
        // The camera placed at the start: one request, and its picture.
        lag.asked(1, t0);
        lag.arrived(1, t0 + ms(40));
        lag.painted(1, t0 + ms(41));
        assert!(lag.motion(t0 + ms(2000)).is_none(), "one request is not a motion");
        // A real one after it is.
        for k in 0..20u32 {
            lag.asked(2 + k, t0 + ms(3000 + 16 * k as u64));
        }
        assert!(lag.motion(t0 + ms(3400)).is_some());
    }

    #[test]
    fn with_no_camera_asked_for_there_is_no_motion() {
        assert!(Lag::default().motion(Instant::now()).is_none());
    }
}
