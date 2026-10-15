//! The shared theme (domains/illustration/theme) as Bevy colours. The HUD and overlays use the
//! console's Night palette, the same as the ground station: they sit over a bright 3D view, so
//! each is a little screen of its own.

use bevy::color::Color;
use illustration_theme::Rgb;

pub use illustration_theme::NIGHT as P;
pub use illustration_theme::geometry;

pub const fn color(c: Rgb) -> Color {
    Color::srgb_u8(c.r(), c.g(), c.b())
}

/// With alpha in 0..1.
pub const fn alpha(c: Rgb, a: f32) -> Color {
    Color::srgba_u8(c.r(), c.g(), c.b(), (a * 255.0) as u8)
}
