//! A small camera's sensor, after the lens: the shutter and the noise.
//!
//! - **The shutter** is open for part of each frame, so what moves across the picture smears
//!   along its path: the trees as the drone banks, the ground rushing under it. A still scene
//!   stays sharp. The smear follows the motion vectors; they are read where the main pass drew
//!   them, which with the ray tracer's upscaler is a corner of the textures. (Bevy's own motion
//!   blur reads them as if they filled the picture.)
//! - **The grain** moves every frame: faint in the sky and in sunlit grass, plain in the
//!   shadows. It is two noises added: the read noise of the sensor's electronics, the same at
//!   every brightness, and the shot noise of the light itself, which grows with the square root
//!   of the light. Relative to the picture, both are strongest in the dark.
//!
//! One pass after the tone mapping does both, to the picture of a camera that has a [`Sensor`].

use bevy::prelude::*;

/// Put it on a camera to give it a small camera's shutter and grain.
#[derive(Component, Clone, Copy, Debug)]
pub struct Sensor {
    /// The read noise: its standard deviation, in the picture's (linear, 0 to 1) units.
    pub read: f32,
    /// The shot noise: its variance per unit of light.
    pub shot: f32,
    /// How much of the noise is colour, not brightness (0 to 1).
    pub chroma: f32,
    /// Counts the frames, so the grain moves.
    pub frame: u32,
    /// How long the shutter is open (s).
    pub exposure: f32,
    /// The smear this frame, as a part of the pixel's motion since the last frame (0 to 1): the
    /// exposure over the frame's time.
    pub blur: f32,
    /// Samples of the smear on each side of a pixel.
    pub samples: u32,
    /// Where the motion vectors are: the main pass's size over the picture's.
    pub motion_scale: Vec2,
}

impl Sensor {
    /// A small FPV camera in daylight.
    pub fn fpv() -> Sensor {
        Sensor {
            read: 0.0,
            shot: 0.0,
            chroma: 0.0,
            frame: 0,
            exposure: 0.0,
            blur: 0.0,
            samples: 0,
            motion_scale: Vec2::ZERO,
        }
    }

    /// The noise's standard deviation where the picture is `v` (linear, 0 to 1).
    pub fn sigma(&self, v: f32) -> f32 {
        let _ = v;
        0.0
    }

    /// The smear of a frame that took `dt` seconds.
    pub fn blur(&self, dt: f32) -> f32 {
        let _ = dt;
        0.0
    }

    /// Where the motion vectors of a picture `full` big are, when the main pass drew `render`.
    pub fn motion_scale(full: UVec2, render: Option<UVec2>) -> Vec2 {
        let _ = (full, render);
        Vec2::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MID_GREY: f32 = 0.18;

    fn relative(v: f32) -> f32 {
        Sensor::fpv().sigma(v) / v
    }

    /// In sunlit grass the grain is there if you look for it, not a snowstorm.
    #[test]
    fn the_grain_is_faint_in_the_light() {
        let r = relative(MID_GREY);
        assert!((0.015..=0.05).contains(&r), "{:.1}% at mid grey", r * 100.0);
        assert!(relative(0.8) < r, "brighter is cleaner: {:.1}% at 0.8", relative(0.8) * 100.0);
    }

    /// Under the trees and in the shade of a building the picture gets noisy.
    #[test]
    fn the_grain_is_plain_in_the_shadows() {
        let shade = relative(0.02);
        assert!(shade > 3.0 * relative(MID_GREY), "{:.1}% in the shade", shade * 100.0);
        assert!(shade < 0.4, "the shade is still a picture ({:.0}%)", shade * 100.0);
    }

    /// More light is more noise, though less of the picture: the shot noise.
    #[test]
    fn the_noise_grows_with_the_light() {
        let s = Sensor::fpv();
        assert!(s.sigma(0.8) > 1.5 * s.sigma(0.02));
        assert!(s.chroma > 0.0 && s.chroma < 0.5, "mostly brightness, a little colour");
    }

    /// The shutter is open for the same time whatever the frame rate: a slow frame sees a smaller
    /// part of the motion since the last one, and never more than all of it.
    #[test]
    fn the_smear_is_the_same_at_any_frame_rate() {
        let s = Sensor { exposure: 1.0 / 120.0, ..Sensor::fpv() };
        assert!((s.blur(1.0 / 60.0) - 0.5).abs() < 1e-6);
        assert!((s.blur(1.0 / 10.0) - 1.0 / 12.0).abs() < 1e-6);
        assert_eq!(s.blur(1.0 / 240.0), 1.0);
        assert_eq!(s.blur(0.0), 0.0);
    }

    /// With the upscaler, the main pass draws a smaller picture into a corner of the textures:
    /// the smear reads each pixel's motion there, not off to the side.
    #[test]
    fn the_smear_reads_the_motion_where_the_main_pass_drew_it() {
        let full = UVec2::new(3200, 1800);
        assert_eq!(Sensor::motion_scale(full, Some(UVec2::new(1600, 900))), Vec2::splat(0.5));
        assert_eq!(Sensor::motion_scale(full, None), Vec2::ONE);
    }
}
