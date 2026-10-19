//! One instance's side of the lockstep: repeat the answer to a frame SITL re-sent, notice a
//! restart, and turn the frame rate into a time step. What the physics does with a fresh frame
//! is the caller's.

use crate::ServoPacket;
use std::net::UdpSocket;
use std::time::Duration;

/// How a packet relates to the one before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The first, or the next frame: the physics steps.
    Fresh,
    /// SITL sent the same frame again (it timed out): the last answer is repeated.
    Repeat,
    /// The frame counter went backwards: SITL was restarted.
    Restarted,
}

/// The time step for SITL's frame rate (s), kept within what the physics can take.
pub fn dt_for(frame_rate: u16) -> f32 {
    if frame_rate > 0 { (1.0 / frame_rate as f32).clamp(1.0 / 4000.0, 1.0 / 50.0) } else { 1.0 / 1200.0 }
}

#[derive(Default)]
pub struct Lockstep {
    last_frame: Option<u32>,
    last_reply: String,
}

impl Lockstep {
    /// Answers `pkt`. `step` runs the physics for a fresh frame (or after a restart) and returns
    /// the JSON reply; a repeated frame gets the last reply without running it.
    pub fn answer(&mut self, pkt: &ServoPacket, step: impl FnOnce(Step, &ServoPacket, f32) -> String) -> &str {
        let kind = match self.last_frame {
            Some(f) if f == pkt.frame_count && !self.last_reply.is_empty() => Step::Repeat,
            Some(f) if pkt.frame_count < f => Step::Restarted,
            _ => Step::Fresh,
        };
        if kind != Step::Repeat {
            self.last_reply = step(kind, pkt, dt_for(pkt.frame_rate));
        }
        self.last_frame = Some(pkt.frame_count);
        &self.last_reply
    }

    /// Serves one socket for ever: `step` for each frame, and `idle` after every 200 ms without one.
    pub fn serve(
        mut self,
        sock: UdpSocket,
        mut step: impl FnMut(Step, &ServoPacket, f32) -> String,
        mut idle: impl FnMut(),
    ) {
        sock.set_read_timeout(Some(Duration::from_millis(200))).ok();
        let mut buf = [0u8; 256];
        loop {
            match sock.recv_from(&mut buf) {
                Ok((n, src)) => {
                    let Some(pkt) = crate::parse_servo_packet(&buf[..n]) else { continue };
                    let reply = self.answer(&pkt, &mut step).to_owned();
                    if let Err(e) = sock.send_to(reply.as_bytes(), src) {
                        eprintln!("JSON reply to {src} failed: {e}");
                    }
                }
                Err(_) => idle(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(frame_count: u32) -> ServoPacket {
        ServoPacket { frame_rate: 1200, frame_count, pwm: [1500; 16] }
    }

    #[test]
    fn a_repeated_frame_gets_the_same_answer_without_stepping() {
        let mut l = Lockstep::default();
        let mut steps = 0;
        let mut run = |l: &mut Lockstep, f| l.answer(&pkt(f), |_, _, _| { steps += 1; format!("r{steps}") }).to_owned();
        assert_eq!(run(&mut l, 1), "r1");
        assert_eq!(run(&mut l, 1), "r1");
        assert_eq!(run(&mut l, 2), "r2");
    }

    #[test]
    fn a_counter_that_goes_backwards_is_a_restart() {
        let mut l = Lockstep::default();
        let mut seen = vec![];
        for f in [5, 6, 1] {
            l.answer(&pkt(f), |k, _, _| { seen.push(k); "x".into() });
        }
        assert_eq!(seen, [Step::Fresh, Step::Fresh, Step::Restarted]);
    }

    #[test]
    fn the_time_step_follows_the_frame_rate() {
        assert!((dt_for(1200) - 1.0 / 1200.0).abs() < 1e-9);
        assert_eq!(dt_for(0), 1.0 / 1200.0);
        assert_eq!(dt_for(10), 1.0 / 50.0);
    }
}
