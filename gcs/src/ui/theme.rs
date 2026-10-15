//! Console palette: tan/olive bezels around near-black screens with phosphor-green symbology,
//! amber cautions and red warnings, after a fixed-wing UAS ground control station. The colours,
//! type and corners come from the shared theme (domains/illustration/theme, its Night variant), which
//! the sim's HUD and the docs use too; this puts them in the shape GPUI wants.

use illustration_theme::{NIGHT as P, geometry};
use gpui::{Hsla, Rgba, rgb};

pub const FONT: &str = illustration_theme::typography::UI;

// Corners are concentric with the window's (see illustration_theme::geometry).
/// Margin between the window edge and the bezels, and between bezels.
pub const GAP: f32 = geometry::GAP;
pub const PANEL_RADIUS: f32 = geometry::PANEL_RADIUS;
/// Bezel padding around each screen.
pub const BEZEL_PAD: f32 = geometry::BEZEL_PAD;
pub const SCREEN_RADIUS: f32 = geometry::SCREEN_RADIUS;
/// Buttons and annunciator tiles.
pub const CONTROL_RADIUS: f32 = geometry::CONTROL_RADIUS;

// Console furniture.
pub const CONSOLE: u32 = P.canvas.0;
pub const BEZEL: u32 = P.bezel.0;
pub const BEZEL_EDGE: u32 = P.bezel_edge.0;
pub const TITLE_BG: u32 = P.title.0;
pub const SCREEN: u32 = P.screen.0;
pub const MAP_BG: u32 = P.map.0;

// Symbology.
pub const GREEN: u32 = P.ok.0;
pub const GREEN_DIM: u32 = P.ok_dim.0;
pub const AMBER: u32 = P.caution.0;
pub const RED: u32 = P.warning.0;
pub const CYAN: u32 = P.info.0;
pub const MAGENTA: u32 = P.highlight.0;
/// The sensor point of interest.
pub const SPI: u32 = P.marker.0;
pub const WHITE: u32 = P.text.0;
pub const GREY: u32 = P.muted.0;
pub const DARK: u32 = P.faint.0;

// Buttons and annunciator lamps.
pub const CONTROL: u32 = P.control.0;
pub const CONTROL_HOVER: u32 = P.control_hover.0;
pub const ACTIVE_FILL: u32 = P.ok_fill.0;
pub const CAUTION_FILL: u32 = P.caution_fill.0;
pub const DANGER_FILL: u32 = P.warning_fill.0;
pub const ON_DANGER: u32 = P.on_warning.0;
pub const LAMP_OFF: u32 = P.off_fill.0;
pub const LAMP_OFF_EDGE: u32 = P.off_edge.0;
pub const LAMP_OK: u32 = P.lamp_ok.0;
pub const LAMP_WARN: u32 = P.lamp_warning.0;

// Synthetic horizon.
pub const SKY: u32 = P.sky.0;
pub const GROUND: u32 = P.ground.0;

pub fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// Colour with alpha in 0..1.
pub fn ca(hex: u32, a: f32) -> Hsla {
    let mut r: Rgba = rgb(hex);
    r.a = a;
    r.into()
}

/// Message colour for a MAV_SEVERITY.
pub fn severity(sev: u8) -> u32 {
    match sev {
        0..=3 => RED,
        4 => AMBER,
        5 => WHITE,
        _ => GREY,
    }
}
