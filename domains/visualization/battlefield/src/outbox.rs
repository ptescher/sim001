//! Which pictures of a battlefield go out to the app (plans/battlefield-latency.md, step 1).
//!
//! Each frame the view reads back the picture it rendered, with the request it rendered it for:
//! the camera of a picture is known, not guessed, so a picture goes out while the camera moves.

use glam::UVec2;
use std::time::Instant;
use teleoperation_shared::battlefield::{Frame, MAX_SIZE, Request};

/// The size a battlefield renders at: the one the app asks for (up to the largest), and none (it
/// does not render) while no app asks. Each camera costs its own shadows, sky and exposure
/// whatever its size, so an idle one is switched off rather than made small.
pub fn render_size(request: Option<&Request>) -> Option<UVec2> {
    request.map(|r| UVec2::new(r.width.clamp(16, MAX_SIZE.0) as u32, r.height.clamp(16, MAX_SIZE.1) as u32))
}

/// The most pictures a second that go out to an app.
pub const MAX_FPS: f64 = 60.0;

/// The pictures that went out of one battlefield.
#[derive(Default)]
pub struct Outbox {
    /// The newest request that a picture went out for, and when.
    sent: Option<(u32, Instant)>,
    /// The number of the last picture that went out.
    frame: u32,
}

impl Outbox {
    /// The picture rendered for `request` came back as `bytes` bytes at `now`: the header it goes
    /// out with, or `None` if it does not go out (another size than asked for, older than one
    /// sent, or too soon after the last).
    pub fn send(&mut self, request: &Request, bytes: usize, now: Instant) -> Option<Frame> {
        let (w, h) = (request.width.clamp(16, MAX_SIZE.0), request.height.clamp(16, MAX_SIZE.1));
        // Texture readbacks pad each row to 256 bytes.
        let stride = (w as u32 * 4).div_ceil(256) * 256;
        if bytes != (stride * h as u32) as usize {
            return None;
        }
        if let Some((seq, at)) = self.sent {
            // An older camera than one sent (the numbers wrap round).
            if (request.seq.wrapping_sub(seq) as i32) < 0 {
                return None;
            }
            if now.saturating_duration_since(at).as_secs_f64() < 1.0 / MAX_FPS - 0.004 {
                return None;
            }
        }
        self.sent = Some((request.seq, now));
        self.frame = self.frame.wrapping_add(1);
        Some(Frame { width: w, height: h, stride, frame: self.frame, seq: request.seq, eye: request.eye })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use std::time::{Duration, Instant};
    use teleoperation_shared::battlefield::{Eye, Request};

    fn req(seq: u32) -> Request {
        Request { eye: Eye { look: Vec2::new(seq as f32, 0.0), ..Eye::default() }, width: 300, height: 200, seq }
    }

    /// The bytes of a readback of a picture `w` by `h`: rows padded to 256 bytes.
    fn bytes(w: u32, h: u32) -> usize {
        ((w * 4).div_ceil(256) * 256 * h) as usize
    }

    #[test]
    fn a_battlefield_that_no_app_asks_for_does_not_render() {
        assert_eq!(render_size(None), None);
    }

    #[test]
    fn a_battlefield_renders_at_the_size_asked_for_up_to_the_largest() {
        assert_eq!(render_size(Some(&req(1))), Some(glam::UVec2::new(300, 200)));
        let huge = Request { width: MAX_SIZE.0 + 500, ..req(1) };
        assert_eq!(render_size(Some(&huge)), Some(glam::UVec2::new(MAX_SIZE.0 as u32, 200)));
    }

    #[test]
    fn while_the_camera_moves_each_picture_goes_out_with_its_own_camera() {
        let t0 = Instant::now();
        let mut out = Outbox::default();
        for k in 1..=10u32 {
            // A new camera every frame, 60 frames a second.
            let at = t0 + Duration::from_millis(17 * k as u64);
            let f = out.send(&req(k), bytes(300, 200), at).expect("a picture each frame");
            assert_eq!((f.seq, f.eye, f.width, f.height), (k, req(k).eye, 300, 200));
        }
    }

    #[test]
    fn a_picture_older_than_one_sent_does_not_go_out() {
        let t0 = Instant::now();
        let mut out = Outbox::default();
        assert!(out.send(&req(5), bytes(300, 200), t0).is_some());
        assert!(out.send(&req(4), bytes(300, 200), t0 + Duration::from_millis(20)).is_none());
        assert!(out.send(&req(6), bytes(300, 200), t0 + Duration::from_millis(40)).is_some());
    }

    #[test]
    fn a_picture_of_another_size_than_its_request_does_not_go_out() {
        // The battlefield was resized and the texture had not caught up yet.
        let mut out = Outbox::default();
        assert!(out.send(&req(1), bytes(640, 200), Instant::now()).is_none());
    }

    #[test]
    fn at_most_max_fps_pictures_go_out() {
        let t0 = Instant::now();
        let mut out = Outbox::default();
        assert!(out.send(&req(1), bytes(300, 200), t0).is_some());
        assert!(out.send(&req(2), bytes(300, 200), t0 + Duration::from_millis(3)).is_none(), "too soon");
        assert!(out.send(&req(3), bytes(300, 200), t0 + Duration::from_secs_f64(1.0 / MAX_FPS)).is_some());
    }

    #[test]
    fn a_picture_of_the_same_camera_goes_out_again_after_a_while() {
        // The app connected again, or a picture was lost: a still camera still gets pictures.
        let t0 = Instant::now();
        let mut out = Outbox::default();
        assert!(out.send(&req(1), bytes(300, 200), t0).is_some());
        assert!(out.send(&req(1), bytes(300, 200), t0 + Duration::from_millis(20)).is_some());
    }

    #[test]
    fn a_battlefield_bigger_than_the_largest_picture_gets_the_largest_picture() {
        let mut out = Outbox::default();
        let huge = Request { width: MAX_SIZE.0 + 500, ..req(1) };
        let f = out.send(&huge, bytes(MAX_SIZE.0 as u32, 200), Instant::now()).expect("the largest picture");
        assert_eq!(f.width, MAX_SIZE.0);
    }
}
