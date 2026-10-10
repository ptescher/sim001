//! Console palette: tan/olive bezels around near-black screens with phosphor-green symbology,
//! amber cautions and red warnings, after a fixed-wing UAS ground control station.

use gpui::{Hsla, Rgba, rgb};

pub const FONT: &str = "Menlo";

// Corners are concentric with the window's: macOS 26+ rounds windows at about 17 pt (measured
// from a capture; there is no public API), and every inset steps the radius down by the inset.
pub const WINDOW_RADIUS: f32 = 17.0;
/// Margin between the window edge and the bezels, and between bezels.
pub const GAP: f32 = 6.0;
pub const PANEL_RADIUS: f32 = WINDOW_RADIUS - GAP;
/// Bezel padding around each screen.
pub const BEZEL_PAD: f32 = 5.0;
pub const SCREEN_RADIUS: f32 = PANEL_RADIUS - BEZEL_PAD;
/// Buttons and annunciator tiles.
pub const CONTROL_RADIUS: f32 = 4.0;

// Console furniture.
pub const CONSOLE: u32 = 0x1a1b17;
pub const BEZEL: u32 = 0x2b2d26;
pub const BEZEL_EDGE: u32 = 0x45483d;
pub const TITLE_BG: u32 = 0x22241e;
pub const SCREEN: u32 = 0x050806;

// Symbology.
pub const GREEN: u32 = 0x4cff7a;
pub const GREEN_DIM: u32 = 0x2a9447;
pub const AMBER: u32 = 0xffb21f;
pub const RED: u32 = 0xff3b30;
pub const CYAN: u32 = 0x45d4ff;
pub const MAGENTA: u32 = 0xff5ce1;
pub const WHITE: u32 = 0xdfe8df;
pub const GREY: u32 = 0x8a958a;
pub const DARK: u32 = 0x3a403a;

// Synthetic horizon.
pub const SKY: u32 = 0x0e2c4c;
pub const GROUND: u32 = 0x3b2a15;

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
