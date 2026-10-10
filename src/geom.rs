//! Shared airframe geometry for the HEEWING T1-S Ranger.
//!
//! Body frame (Bevy convention): +X right, +Y up, -Z forward (nose). Origin is the CG.
//! Published specs: 790 mm span, 715 mm length, twin 1406 motors, 3830 props, 4S,
//! 600-950 g takeoff weight. Chords, tail sizing and CG are estimates from photos.

use std::f32::consts::PI;

pub const DEG: f32 = PI / 180.0;

// Main wing
pub const WING_HALF_SPAN: f32 = 0.395;
pub const WING_ROOT_CHORD: f32 = 0.175;
pub const WING_TIP_CHORD: f32 = 0.115;
/// Leading edge Z at the root (CG sits ~30% MAC behind it).
pub const WING_LE_Z: f32 = -0.045;
/// Leading-edge sweep: how far back the tip LE sits relative to the root LE.
pub const WING_LE_SWEEP: f32 = 0.035;
pub const WING_Y: f32 = 0.022;
pub const WING_DIHEDRAL: f32 = 3.0 * DEG;
pub const WING_INCIDENCE: f32 = 1.0 * DEG;
pub const AILERON_START: f32 = 0.17;
pub const AILERON_CHORD_FRAC: f32 = 0.26;

// Motors (wing-mounted tractors, counter-rotating)
pub const MOTOR_X: f32 = 0.165;
pub const MOTOR_Y: f32 = 0.022;
pub const MOTOR_Z: f32 = -0.12;
pub const PROP_RADIUS: f32 = 0.0483; // 3.8 inch prop

// V-tail
pub const TAIL_LE_Z: f32 = 0.33;
pub const TAIL_LE_SWEEP: f32 = 0.035;
pub const TAIL_ROOT_CHORD: f32 = 0.115;
pub const TAIL_TIP_CHORD: f32 = 0.075;
pub const TAIL_SPAN: f32 = 0.175;
pub const TAIL_DIHEDRAL: f32 = 35.0 * DEG;
pub const TAIL_Y: f32 = 0.035;
pub const TAIL_ROOT_X: f32 = 0.01;
pub const TAIL_INCIDENCE: f32 = -2.6 * DEG;
pub const RUDDERVATOR_CHORD_FRAC: f32 = 0.35;

// Fuselage
pub const NOSE_Z: f32 = -0.27;
pub const TAIL_END_Z: f32 = 0.445;

pub fn wing_chord(s: f32) -> f32 {
    WING_ROOT_CHORD + (WING_TIP_CHORD - WING_ROOT_CHORD) * (s / WING_HALF_SPAN)
}

pub fn wing_le_z(s: f32) -> f32 {
    WING_LE_Z + WING_LE_SWEEP * (s / WING_HALF_SPAN)
}

pub fn tail_chord(s: f32) -> f32 {
    TAIL_ROOT_CHORD + (TAIL_TIP_CHORD - TAIL_ROOT_CHORD) * (s / TAIL_SPAN)
}

pub fn tail_le_z(s: f32) -> f32 {
    TAIL_LE_Z + TAIL_LE_SWEEP * (s / TAIL_SPAN)
}
