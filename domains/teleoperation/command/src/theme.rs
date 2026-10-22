//! The app's colours and corners: the console's look (the shared theme's Night variant), with a
//! colour for each team.

use gpui::{Hsla, Rgba, rgb};
use illustration_theme::{NIGHT as P, geometry};

pub const FONT: &str = illustration_theme::typography::UI;

pub const GAP: f32 = geometry::GAP;
pub const PANEL_RADIUS: f32 = geometry::PANEL_RADIUS;
pub const BEZEL_PAD: f32 = geometry::BEZEL_PAD;
pub const SCREEN_RADIUS: f32 = geometry::SCREEN_RADIUS;
pub const CONTROL_RADIUS: f32 = geometry::CONTROL_RADIUS;

pub const CONSOLE: u32 = P.canvas.0;
pub const BEZEL: u32 = P.bezel.0;
pub const BEZEL_EDGE: u32 = P.bezel_edge.0;
pub const TITLE_BG: u32 = P.title.0;
pub const SCREEN: u32 = P.screen.0;

pub const GREEN: u32 = P.ok.0;
pub const GREEN_DIM: u32 = P.ok_dim.0;
pub const AMBER: u32 = P.caution.0;
pub const WHITE: u32 = P.text.0;
pub const GREY: u32 = P.muted.0;
pub const DARK: u32 = P.faint.0;

pub const CONTROL: u32 = P.control.0;
pub const CONTROL_HOVER: u32 = P.control_hover.0;
pub const ACTIVE_FILL: u32 = P.ok_fill.0;

/// The teams: Blue in the theme's info colour, Red in its warning colour.
pub const BLUE: u32 = P.info.0;
pub const RED: u32 = P.warning.0;
/// What a team sees, and the fog over the rest.
pub const FIELD: u32 = 0x1d2a1f;
pub const FOG: u32 = 0x07090a;
/// The ground: the lanes cut through the forest, and the lake.
pub const LANE: u32 = 0x33463a;
pub const LAKE: u32 = 0x1d3b52;

pub fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// Colour with alpha in 0..1.
pub fn ca(hex: u32, a: f32) -> Hsla {
    let mut r: Rgba = rgb(hex);
    r.a = a;
    r.into()
}

pub fn team(t: recreation_towers::Team) -> u32 {
    match t {
        recreation_towers::Team::Blue => BLUE,
        recreation_towers::Team::Red => RED,
    }
}
