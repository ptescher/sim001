//! How far the picture of the battlefield trails the camera (plans/battlefield-latency.md, step 0).
//!
//! The app asks for a picture each time the camera changes, each request with the next `seq`, and
//! each picture comes back with the `seq` it was rendered for. While the camera moves, each paint
//! says how long the picture on screen has been out of date: since the app asked for the first
//! camera that the picture does not show. A motion ends when the camera has not changed for
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
            // Out of date since the first camera after the one it shows.
            let behind = self.asked.iter().find(|(s, _)| *s > seq).map_or(0.0, |(_, at)| ms(now - *at));
            self.run.lags.push(behind);
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
    fn a_picture_of_the_newest_camera_is_not_behind() {
        let t0 = Instant::now();
        let mut lag = Lag::default();
        lag.asked(1, t0);
        lag.asked(2, t0 + ms(16));
        lag.painted(2, t0 + ms(30));
        assert_eq!(lag.motion(t0 + ms(30)).unwrap().lag_ms, 0.0);
    }

    #[test]
    fn a_picture_is_behind_since_the_first_camera_it_does_not_show() {
        let t0 = Instant::now();
        let mut lag = Lag::default();
        lag.asked(1, t0);
        lag.asked(2, t0 + ms(16));
        lag.asked(3, t0 + ms(33));
        // Still the picture of camera 1: it has been out of date since camera 2 was asked for.
        lag.painted(1, t0 + ms(50));
        assert_eq!(lag.motion(t0 + ms(50)).unwrap().lag_ms, 34.0);
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
        assert_eq!(done.lag_ms, 20.0);
        // A new motion: its own numbers only.
        lag.asked(3, t0 + ms(2000));
        lag.asked(4, t0 + ms(2016));
        lag.painted(4, t0 + ms(2030));
        assert_eq!(lag.motion(t0 + ms(2030)).unwrap().lag_ms, 0.0);
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
