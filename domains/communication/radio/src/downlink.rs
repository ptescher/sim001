//! The telemetry link from the plane, through the TX15, to the station.

use std::collections::VecDeque;

/// Who has the right of way on the link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    /// Replies and anything else that must arrive (commands, missions, parameters): it is sent
    /// first, whatever the rate, and it is never replaced or dropped.
    Reliable,
    /// The plane's own telemetry: what the pilot flies by.
    Own,
    /// What the plane passes on about other aircraft.
    Traffic,
}

struct Queued {
    key: u64,
    time: f64,
    bytes: Vec<u8>,
}

/// A narrow link. Every frame has a `key` that says what it is about (a message of a system).
/// A new frame with the key of a frame that is still waiting takes its place, because an old
/// position is no use once there is a new one. A frame that waits too long is dropped. The
/// plane's own frames go first.
pub struct Downlink {
    rate: f32,
    tokens: f32,
    up: bool,
    reliable: VecDeque<Queued>,
    own: VecDeque<Queued>,
    traffic: VecDeque<Queued>,
    sent: u64,
    dropped: u64,
}

impl Downlink {
    /// What the link carries, in bytes a second (an assumption to tune: a telemetry link on
    /// the radio of a small plane carries from a few hundred bytes to a few thousand).
    pub const DEFAULT_RATE: f32 = 2600.0;
    /// A frame about other aircraft that waits this long (s) is dropped.
    pub const TRAFFIC_MAX_AGE: f64 = 3.0;
    /// A frame of the plane's own that waits this long (s) is dropped.
    pub const OWN_MAX_AGE: f64 = 5.0;

    pub fn new(rate: f32) -> Downlink {
        Downlink {
            rate,
            tokens: rate,
            up: true,
            reliable: VecDeque::new(),
            own: VecDeque::new(),
            traffic: VecDeque::new(),
            sent: 0,
            dropped: 0,
        }
    }

    pub fn is_up(&self) -> bool {
        self.up
    }

    /// A lost link carries nothing, and what waited for it is gone.
    pub fn set_up(&mut self, up: bool) {
        self.up = up;
        if !up {
            self.reliable.clear();
            self.own.clear();
            self.traffic.clear();
        }
    }

    /// Bytes sent, and frames dropped, so far.
    pub fn counts(&self) -> (u64, u64) {
        (self.sent, self.dropped)
    }

    pub fn submit(&mut self, priority: Priority, key: u64, now: f64, bytes: Vec<u8>) {
        if !self.up {
            self.dropped += 1;
            return;
        }
        if priority == Priority::Reliable {
            self.reliable.push_back(Queued { key, time: now, bytes });
            return;
        }
        let queue = match priority {
            Priority::Reliable => unreachable!(),
            Priority::Own => &mut self.own,
            Priority::Traffic => &mut self.traffic,
        };
        if let Some(old) = queue.iter_mut().find(|q| q.key == key) {
            self.dropped += 1;
            *old = Queued { key, time: now, bytes };
        } else {
            queue.push_back(Queued { key, time: now, bytes });
        }
    }

    /// Lets `dt` seconds pass at time `now`, and returns the bytes that came out of the radio.
    pub fn step(&mut self, dt: f64, now: f64) -> Vec<u8> {
        self.tokens = (self.tokens + self.rate * dt as f32).min(self.rate);
        let mut out = vec![];
        // What must arrive goes whatever the rate. It still uses the budget, so the rest waits.
        while let Some(q) = self.reliable.pop_front() {
            self.tokens = (self.tokens - q.bytes.len() as f32).max(-self.rate);
            out.extend_from_slice(&q.bytes);
        }
        if self.tokens < 0.0 {
            self.sent += out.len() as u64;
            return out;
        }
        let (own_age, traffic_age) = (Self::OWN_MAX_AGE, Self::TRAFFIC_MAX_AGE);
        for (queue, max_age) in [(&mut self.own, own_age), (&mut self.traffic, traffic_age)] {
            let before = queue.len();
            queue.retain(|q| now - q.time <= max_age);
            self.dropped += (before - queue.len()) as u64;
            while let Some(front) = queue.front() {
                if front.bytes.len() as f32 > self.tokens {
                    break;
                }
                self.tokens -= front.bytes.len() as f32;
                out.extend_from_slice(&queue.pop_front().unwrap().bytes);
            }
        }
        self.sent += out.len() as u64;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(link: &mut Downlink, seconds: f64, mut each: impl FnMut(&mut Downlink, f64)) -> usize {
        let mut sent = 0;
        let mut now = 0.0;
        while now < seconds {
            each(link, now);
            sent += link.step(0.05, now).len();
            now += 0.05;
        }
        sent
    }

    #[test]
    fn the_link_carries_no_more_than_its_rate() {
        let mut link = Downlink::new(1000.0);
        let sent = run(&mut link, 10.0, |l, now| {
            for k in 0..10 {
                l.submit(Priority::Own, k, now, vec![0; 40]);
            }
        });
        assert!((9_500..=11_100).contains(&sent), "{sent} bytes in 10 s");
    }

    #[test]
    fn your_own_telemetry_goes_first() {
        let mut link = Downlink::new(1000.0);
        let (mut own, mut traffic) = (0, 0);
        let mut now = 0.0;
        while now < 10.0 {
            for k in 0..5 {
                link.submit(Priority::Own, k, now, vec![1; 40]);
            }
            for k in 100..140 {
                link.submit(Priority::Traffic, k, now, vec![2; 40]);
            }
            for b in link.step(0.05, now) {
                if b == 1 {
                    own += 1;
                } else {
                    traffic += 1;
                }
            }
            now += 0.05;
        }
        // The plane asks for 200 bytes every 0.05 s: 4000 bytes a second, more than the link. It still gets what it asked for.
        assert!(own >= 40 * 5 * 18, "{own} own bytes");
        assert!(traffic < own, "{traffic} traffic bytes");
    }

    #[test]
    fn a_newer_frame_takes_the_place_of_an_older_one_that_waits() {
        let mut link = Downlink::new(100.0);
        link.step(1.0, 0.0);
        link.submit(Priority::Traffic, 7, 0.0, vec![1; 90]);
        link.submit(Priority::Traffic, 7, 0.1, vec![2; 90]);
        let out = link.step(0.1, 0.1);
        assert_eq!(out, vec![2; 90]);
        assert_eq!(link.counts().1, 1);
    }

    #[test]
    fn a_frame_that_waits_too_long_is_dropped() {
        let mut link = Downlink::new(10.0);
        link.step(1.0, 0.0);
        link.submit(Priority::Traffic, 1, 0.0, vec![9; 100]);
        for i in 1..=100 {
            assert!(link.step(0.05, i as f64 * 0.05).is_empty(), "{i}");
        }
        assert_eq!(link.counts().1, 1);
    }

    #[test]
    fn what_must_arrive_arrives_whatever_the_rate_and_slows_the_rest() {
        let mut link = Downlink::new(100.0);
        link.step(1.0, 0.0);
        for i in 0..10 {
            link.submit(Priority::Reliable, i, 0.0, vec![5; 50]);
        }
        link.submit(Priority::Own, 99, 0.0, vec![1; 40]);
        let out = link.step(0.05, 0.05);
        assert_eq!(out.iter().filter(|b| **b == 5).count(), 500, "all ten, at once");
        assert!(!out.contains(&1), "the telemetry waits while the budget is spent");
        let mut later = vec![];
        for i in 1..=60 {
            later.extend(link.step(0.05, 0.05 + i as f64 * 0.05));
        }
        assert!(later.contains(&1), "and goes when the budget is back");
    }

    #[test]
    fn a_lost_link_carries_nothing_and_forgets_what_waited() {
        let mut link = Downlink::new(1000.0);
        link.submit(Priority::Own, 1, 0.0, vec![1; 10]);
        link.set_up(false);
        link.submit(Priority::Own, 2, 0.0, vec![1; 10]);
        assert!(link.step(0.5, 0.5).is_empty());
        link.set_up(true);
        assert!(link.step(0.5, 1.0).is_empty(), "the old frames are gone");
        link.submit(Priority::Own, 3, 1.0, vec![1; 10]);
        assert_eq!(link.step(0.1, 1.1).len(), 10);
    }
}
