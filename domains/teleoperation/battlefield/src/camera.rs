//! The operator's camera over the battlefield and its controls, as in a real-time strategy game:
//! W, A, S, D or the arrow keys move it, and so does the mouse at an edge of the battlefield; Q and
//! E turn it by 45°; the wheel takes it nearer to the ground or further. The app adds its own: a
//! click on the minimap ([`Rig::look_at`]), and a key that looks at the selection ([`Rig::focus`]).

use crate::draw::rect;
use glam::Vec2;
use gpui::{Bounds, Modifiers, Pixels, ScrollDelta};
use std::collections::HashSet;
use std::time::Instant;
use teleoperation_shared::battlefield::Eye;

/// How the camera looks down, and how near and how far from the ground it goes (m).
pub const PITCH: f32 = 50.0;
pub const HEIGHT_MIN: f32 = 20.0;
pub const HEIGHT_MAX: f32 = 2500.0;
/// One step of the wheel takes the camera this much nearer.
pub const WHEEL_STEP: f32 = 0.8;
/// Q and E turn the camera this far (deg).
pub const TURN: f32 = 45.0;
/// The mouse this near an edge of the battlefield moves the camera (screen pixels).
pub const EDGE: f32 = 14.0;
/// From the camera to the point it looks at, at the start (m).
pub const START_DISTANCE: f32 = 450.0;

/// The camera and what moves it now.
pub struct Rig {
    pub eye: Eye,
    /// The camera has been put where it starts.
    placed: bool,
    /// The keys that move the camera, held down now.
    held: HashSet<String>,
    /// The mouse on the battlefield, for the edges (screen pixels).
    hover: Option<(f32, f32)>,
    last_tick: Instant,
    /// The camera looks no further than this from the middle of the field (m).
    pub limit: f32,
}

impl Default for Rig {
    fn default() -> Rig {
        Rig {
            eye: Eye { pitch: PITCH, ..Eye::default() },
            placed: false,
            held: HashSet::new(),
            hover: None,
            last_tick: Instant::now(),
            limit: 1000.0,
        }
    }
}

impl Rig {
    /// Puts the camera over `look`, looking towards `heading` (deg), if it is not placed yet.
    pub fn place(&mut self, look: Vec2, heading: f32) {
        if self.placed {
            return;
        }
        self.eye = Eye { look, heading, pitch: PITCH, distance: START_DISTANCE, ..Eye::default() };
        self.placed = true;
    }

    /// As on a fresh start: the camera is placed again, and nothing moves it.
    pub fn reset(&mut self) {
        self.placed = false;
        self.held.clear();
        self.hover = None;
    }

    /// A key went down: whether the camera took it (a key with Cmd or Ctrl is never the camera's).
    pub fn key_down(&mut self, key: &str, m: Modifiers) -> bool {
        if m.secondary() || m.control {
            return false;
        }
        match key {
            "w" | "a" | "s" | "d" | "up" | "down" | "left" | "right" => {
                self.held.insert(key.to_string());
            }
            "q" => self.turn(-TURN),
            "e" => self.turn(TURN),
            _ => return false,
        }
        true
    }

    pub fn key_up(&mut self, key: &str) {
        self.held.remove(key);
    }

    /// Where the mouse is, if it is on the battlefield.
    pub fn hover(&mut self, at: Option<(f32, f32)>) {
        self.hover = at;
    }

    /// Whether the keys held or the mouse at an edge of `bounds` move the camera.
    pub fn moving(&self, bounds: Bounds<Pixels>) -> bool {
        self.push(bounds) != (0.0, 0.0)
    }

    /// Moves the camera by the keys held and the mouse at an edge of `bounds` (the battlefield).
    pub fn tick(&mut self, bounds: Bounds<Pixels>) {
        let dt = self.last_tick.elapsed().as_secs_f32().min(0.1);
        self.last_tick = Instant::now();
        let (fwd, right) = self.push(bounds);
        if fwd == 0.0 && right == 0.0 {
            return;
        }
        let h = self.eye.heading.to_radians();
        let (ahead, side) = (Vec2::new(h.cos(), h.sin()), Vec2::new(-h.sin(), h.cos()));
        // Faster the higher the camera is, as in a strategy game.
        let speed = (self.eye.height() * 1.3).max(120.0);
        let to = self.eye.look + (ahead * fwd + side * right).normalize_or_zero() * speed * dt;
        self.look_at(to);
    }

    /// Which way the keys held and the mouse at an edge push the camera: (ahead, right).
    fn push(&self, bounds: Bounds<Pixels>) -> (f32, f32) {
        let held = |k: &[&str]| k.iter().any(|k| self.held.contains(*k));
        let (mut fwd, mut right) = (0.0f32, 0.0f32);
        if held(&["w", "up"]) {
            fwd += 1.0;
        }
        if held(&["s", "down"]) {
            fwd -= 1.0;
        }
        if held(&["d", "right"]) {
            right += 1.0;
        }
        if held(&["a", "left"]) {
            right -= 1.0;
        }
        if let Some((x, y)) = self.hover {
            let (bx, by, bw, bh) = rect(bounds);
            if x < bx + EDGE {
                right -= 1.0;
            }
            if x > bx + bw - EDGE {
                right += 1.0;
            }
            if y < by + EDGE {
                fwd += 1.0;
            }
            if y > by + bh - EDGE {
                fwd -= 1.0;
            }
        }
        (fwd, right)
    }

    /// The camera looks at `at`, kept within `limit` of the middle of the field.
    pub fn look_at(&mut self, at: Vec2) {
        self.eye.look = at.clamp_length_max(self.limit);
    }

    /// The camera looks at the middle of `points`, if there are any.
    pub fn focus(&mut self, points: &[Vec2]) {
        if !points.is_empty() {
            self.look_at(points.iter().copied().sum::<Vec2>() / points.len() as f32);
        }
    }

    /// The camera turns `by` degrees, to a multiple of 45°.
    pub fn turn(&mut self, by: f32) {
        self.eye.heading = (((self.eye.heading + by) / TURN).round() * TURN).rem_euclid(360.0);
    }

    /// The camera goes nearer to the ground (`factor` < 1) or further.
    pub fn zoom(&mut self, factor: f32) {
        let s = self.eye.pitch.to_radians().sin();
        self.eye.distance = (self.eye.distance * factor).clamp(HEIGHT_MIN / s, HEIGHT_MAX / s);
    }

    /// The wheel: forward takes the camera nearer to the ground.
    pub fn scroll(&mut self, delta: ScrollDelta) {
        self.zoom(match delta {
            ScrollDelta::Pixels(p) => (-f32::from(p.y) * 0.004).exp(),
            ScrollDelta::Lines(l) => WHEEL_STEP.powf(l.y),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px, size};

    fn bounds() -> Bounds<Pixels> {
        Bounds { origin: point(px(0.0), px(0.0)), size: size(px(800.0), px(500.0)) }
    }

    #[test]
    fn it_starts_where_it_is_placed_and_only_once() {
        let mut rig = Rig::default();
        rig.place(Vec2::new(-500.0, 0.0), 0.0);
        rig.place(Vec2::new(500.0, 0.0), 180.0);
        assert_eq!((rig.eye.look, rig.eye.heading), (Vec2::new(-500.0, 0.0), 0.0));
        rig.reset();
        rig.place(Vec2::new(500.0, 0.0), 180.0);
        assert_eq!(rig.eye.heading, 180.0);
    }

    #[test]
    fn e_turns_right_and_q_left_by_45_degrees() {
        let mut rig = Rig::default();
        assert!(rig.key_down("e", Modifiers::default()));
        assert_eq!(rig.eye.heading, 45.0);
        rig.key_down("q", Modifiers::default());
        rig.key_down("q", Modifiers::default());
        assert_eq!(rig.eye.heading, 315.0);
    }

    #[test]
    fn a_key_with_cmd_or_ctrl_is_not_the_cameras() {
        let mut rig = Rig::default();
        assert!(!rig.key_down("a", Modifiers { control: true, ..Default::default() }));
    }

    #[test]
    fn w_held_moves_the_camera_the_way_it_looks() {
        let mut rig = Rig { eye: Eye { heading: 90.0, ..Eye::default() }, ..Rig::default() };
        rig.key_down("w", Modifiers::default());
        rig.last_tick = Instant::now() - std::time::Duration::from_millis(100);
        rig.tick(bounds());
        assert!(rig.eye.look.y > 10.0 && rig.eye.look.x.abs() < 1.0, "{}", rig.eye.look);
        rig.key_up("w");
        let at = rig.eye.look;
        rig.tick(bounds());
        assert_eq!(rig.eye.look, at);
    }

    #[test]
    fn the_camera_moves_while_a_key_is_held_or_the_mouse_is_at_an_edge() {
        let mut rig = Rig::default();
        assert!(!rig.moving(bounds()));
        rig.key_down("d", Modifiers::default());
        assert!(rig.moving(bounds()));
        rig.key_up("d");
        rig.hover(Some((400.0, 250.0)));
        assert!(!rig.moving(bounds()), "the mouse in the middle");
        rig.hover(Some((797.0, 250.0)));
        assert!(rig.moving(bounds()));
    }

    #[test]
    fn the_mouse_at_the_top_edge_moves_the_camera_ahead() {
        let mut rig = Rig::default();
        rig.hover(Some((400.0, 3.0)));
        rig.last_tick = Instant::now() - std::time::Duration::from_millis(100);
        rig.tick(bounds());
        assert!(rig.eye.look.x > 10.0);
    }

    #[test]
    fn one_step_of_the_wheel_takes_the_camera_a_fifth_nearer() {
        let mut rig = Rig::default();
        let h = rig.eye.height();
        rig.scroll(ScrollDelta::Lines(point(0.0, 1.0)));
        assert!((rig.eye.height() - h * 0.8).abs() < 0.01);
    }
}
