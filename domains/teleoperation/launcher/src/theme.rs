//! The launcher's colours, type and corners come from the shared theme (Night variant), like the
//! console's; this puts them in the shape GPUI wants.

use gpui::{Hsla, rgb};
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
pub const SCREEN: u32 = P.screen.0;
pub const GREEN: u32 = P.ok.0;
pub const GREEN_DIM: u32 = P.ok_dim.0;
pub const AMBER: u32 = P.caution.0;
pub const RED: u32 = P.warning.0;
pub const WHITE: u32 = P.text.0;
pub const GREY: u32 = P.muted.0;
pub const DARK: u32 = P.faint.0;
pub const CONTROL: u32 = P.control.0;
pub const CONTROL_HOVER: u32 = P.control_hover.0;
pub const ACTIVE_FILL: u32 = P.ok_fill.0;
pub const CAUTION_FILL: u32 = P.caution_fill.0;
pub const DANGER_FILL: u32 = P.warning_fill.0;
pub const ON_DANGER: u32 = P.on_warning.0;

pub fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}
