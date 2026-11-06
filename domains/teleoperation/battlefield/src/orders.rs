//! The orders that both apps give on the battlefield, the ground station and the commander app
//! alike: besides a right-click (go there) and Shift with a right-click (the next point of a route),
//! CIRCLE: press C or the button, press the mouse on a point, drag out to the distance, and let go.

use glam::Vec2;

/// The closest a circle goes round its point (m).
pub const CIRCLE_MIN: f32 = 30.0;

/// CIRCLE: armed (the next press on the battlefield is its centre), or being dragged.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CircleTool {
    armed: bool,
    drag: Option<(Vec2, f32)>,
}

impl CircleTool {
    /// CIRCLE (or C): the next press on the battlefield is the centre. Again: no circle after all.
    pub fn arm(&mut self) {
        if self.armed || self.drag.is_some() {
            self.cancel();
        } else {
            self.armed = true;
        }
    }

    pub fn armed(&self) -> bool {
        self.armed || self.drag.is_some()
    }

    /// A press on the ground at `at`: the centre, if CIRCLE is armed (and then the press is the tool's).
    pub fn press(&mut self, at: Vec2) -> bool {
        if !self.armed {
            return false;
        }
        self.armed = false;
        self.drag = Some((at, 0.0));
        true
    }

    /// The mouse is over the ground at `at` while the button is down.
    pub fn drag(&mut self, at: Vec2) {
        if let Some((centre, r)) = &mut self.drag {
            *r = centre.distance(at);
        }
    }

    /// The circle being dragged: its centre and its distance.
    pub fn preview(&self) -> Option<(Vec2, f32)> {
        self.drag
    }

    /// The button went up: the circle, its distance kept from `CIRCLE_MIN` to `max`.
    pub fn release(&mut self, max: f32) -> Option<(Vec2, f32)> {
        let (centre, r) = self.drag.take()?;
        Some((centre, r.clamp(CIRCLE_MIN, max)))
    }

    /// Esc: no circle.
    pub fn cancel(&mut self) {
        *self = CircleTool::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_does_nothing_until_circle_is_armed() {
        let mut t = CircleTool::default();
        assert!(!t.press(Vec2::ZERO));
        assert_eq!(t.preview(), None);
    }

    #[test]
    fn press_drag_and_let_go_give_the_centre_and_the_distance() {
        let mut t = CircleTool::default();
        t.arm();
        assert!(t.armed());
        assert!(t.press(Vec2::new(100.0, 0.0)));
        t.drag(Vec2::new(100.0, 120.0));
        assert_eq!(t.preview(), Some((Vec2::new(100.0, 0.0), 120.0)));
        assert_eq!(t.release(300.0), Some((Vec2::new(100.0, 0.0), 120.0)));
        // It is done: the next press is the app's own again.
        assert!(!t.armed() && !t.press(Vec2::ZERO));
    }

    #[test]
    fn the_distance_is_kept_between_the_least_and_the_most() {
        let mut t = CircleTool::default();
        t.arm();
        t.press(Vec2::ZERO);
        t.drag(Vec2::new(5.0, 0.0));
        assert_eq!(t.release(300.0), Some((Vec2::ZERO, CIRCLE_MIN)));
        t.arm();
        t.press(Vec2::ZERO);
        t.drag(Vec2::new(900.0, 0.0));
        assert_eq!(t.release(300.0), Some((Vec2::ZERO, 300.0)));
    }

    #[test]
    fn arming_again_or_escape_lets_go() {
        let mut t = CircleTool::default();
        t.arm();
        t.arm();
        assert!(!t.armed());
        t.arm();
        t.press(Vec2::ZERO);
        t.cancel();
        assert_eq!((t.armed(), t.preview(), t.release(300.0)), (false, None, None));
    }
}
