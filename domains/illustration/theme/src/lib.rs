//! The T1-S look, shared by everything with a screen: the ground station (GPUI), the sim's HUD
//! and overlays (Bevy) and the docs site (CSS).
//!
//! It's the ground station's console: near-black screens in olive bezels, phosphor-green
//! symbology, amber cautions, red warnings, cyan for information and magenta for what the
//! autopilot is steering to. [`NIGHT`] is that console. [`DAY`] is the same instrument in
//! sunlight, with darker inks on light panels, for reading the docs in a light browser.
//!
//! Colours are roles, not hues. What a colour is for, such as a caution or the screen behind
//! the symbology, is the same in both variants, so anything written against the roles works in
//! either. Consumers convert [`Rgb`] to their own colour type.

pub mod css;
pub mod edgetx;

/// A colour as 0xRRGGBB, in sRGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u32);

impl Rgb {
    pub const fn r(self) -> u8 {
        (self.0 >> 16) as u8
    }

    pub const fn g(self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub const fn b(self) -> u8 {
        self.0 as u8
    }

    /// Components in 0..1, still sRGB-encoded.
    pub fn srgb(self) -> [f32; 3] {
        [self.r(), self.g(), self.b()].map(|c| c as f32 / 255.0)
    }

    /// "#4cff7a".
    pub fn css(self) -> String {
        format!("#{:06x}", self.0)
    }

    /// WCAG relative luminance.
    pub fn luminance(self) -> f64 {
        let lin = |c: u8| {
            let c = c as f64 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(self.r()) + 0.7152 * lin(self.g()) + 0.0722 * lin(self.b())
    }

    /// WCAG contrast ratio, 1 to 21.
    pub fn contrast(self, other: Rgb) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}

/// Every colour the UI uses, by what it's for.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub name: &'static str,

    // Furniture.
    /// Behind everything: the console, the docs page.
    pub canvas: Rgb,
    /// Panels and cards.
    pub bezel: Rgb,
    pub bezel_edge: Rgb,
    /// Title strips, the docs sidebar.
    pub title: Rgb,
    /// Behind symbology: displays, code, figures.
    pub screen: Rgb,
    /// The tactical map where there's no photo.
    pub map: Rgb,

    // Text.
    pub text: Rgb,
    pub muted: Rgb,
    /// Switched-off labels. Not for anything that has to be read.
    pub faint: Rgb,

    // Signals.
    pub ok: Rgb,
    pub ok_dim: Rgb,
    pub caution: Rgb,
    pub warning: Rgb,
    pub info: Rgb,
    /// What the autopilot is steering to: the active waypoint, target bugs.
    pub highlight: Rgb,
    /// A point the operator marked.
    pub marker: Rgb,

    // Fills behind signals: buttons, badges, annunciator lamps.
    pub control: Rgb,
    pub control_hover: Rgb,
    pub ok_fill: Rgb,
    pub caution_fill: Rgb,
    pub warning_fill: Rgb,
    /// Text on a warning fill.
    pub on_warning: Rgb,
    pub off_fill: Rgb,
    pub off_edge: Rgb,
    pub lamp_ok: Rgb,
    pub lamp_warning: Rgb,

    // The attitude indicator.
    pub sky: Rgb,
    pub ground: Rgb,
}

/// The console, after a fixed-wing UAS ground control station.
pub const NIGHT: Palette = Palette {
    name: "night",
    canvas: Rgb(0x1a1b17),
    bezel: Rgb(0x2b2d26),
    bezel_edge: Rgb(0x45483d),
    title: Rgb(0x22241e),
    screen: Rgb(0x050806),
    map: Rgb(0x0b100c),
    text: Rgb(0xdfe8df),
    muted: Rgb(0x919c91),
    faint: Rgb(0x3a403a),
    ok: Rgb(0x4cff7a),
    ok_dim: Rgb(0x2a9447),
    caution: Rgb(0xffb21f),
    warning: Rgb(0xff5a4f),
    info: Rgb(0x45d4ff),
    highlight: Rgb(0xff5ce1),
    marker: Rgb(0xffe14d),
    control: Rgb(0x161a16),
    control_hover: Rgb(0x2a302a),
    ok_fill: Rgb(0x14361f),
    caution_fill: Rgb(0x3a2c08),
    warning_fill: Rgb(0x4a0f0c),
    on_warning: Rgb(0xffd0cc),
    off_fill: Rgb(0x0c0f0c),
    off_edge: Rgb(0x1e221e),
    lamp_ok: Rgb(0x0c1f12),
    lamp_warning: Rgb(0x5a0d0a),
    sky: Rgb(0x0e2c4c),
    ground: Rgb(0x3b2a15),
};

/// The same console in sunlight: light tan panels, the signals as dark inks of the same hues.
pub const DAY: Palette = Palette {
    name: "day",
    canvas: Rgb(0xebe8dd),
    bezel: Rgb(0xf6f4ed),
    bezel_edge: Rgb(0xcfcab6),
    title: Rgb(0xe2dfd0),
    screen: Rgb(0xfbfaf6),
    map: Rgb(0xeef0e6),
    text: Rgb(0x1a1b17),
    muted: Rgb(0x545a4f),
    faint: Rgb(0xa6aa9a),
    ok: Rgb(0x136a32),
    ok_dim: Rgb(0x5a9a6a),
    caution: Rgb(0x7a4c00),
    warning: Rgb(0xa82219),
    info: Rgb(0x085d7b),
    highlight: Rgb(0x8c267e),
    marker: Rgb(0x8a6d00),
    control: Rgb(0xf1efe7),
    control_hover: Rgb(0xe4e1d3),
    ok_fill: Rgb(0xdcefe0),
    caution_fill: Rgb(0xf5e5c2),
    warning_fill: Rgb(0xf7dad6),
    on_warning: Rgb(0x7a120c),
    off_fill: Rgb(0xeceae1),
    off_edge: Rgb(0xd8d4c4),
    lamp_ok: Rgb(0xe3f2e6),
    lamp_warning: Rgb(0xf3cfca),
    sky: Rgb(0x6f9fc8),
    ground: Rgb(0x9a7a52),
};

impl Palette {
    /// Every role with its name, for exporting and for checking.
    pub fn roles(&self) -> [(&'static str, Rgb); 28] {
        [
            ("canvas", self.canvas),
            ("bezel", self.bezel),
            ("bezel-edge", self.bezel_edge),
            ("title", self.title),
            ("screen", self.screen),
            ("map", self.map),
            ("text", self.text),
            ("muted", self.muted),
            ("faint", self.faint),
            ("ok", self.ok),
            ("ok-dim", self.ok_dim),
            ("caution", self.caution),
            ("warning", self.warning),
            ("info", self.info),
            ("highlight", self.highlight),
            ("marker", self.marker),
            ("control", self.control),
            ("control-hover", self.control_hover),
            ("ok-fill", self.ok_fill),
            ("caution-fill", self.caution_fill),
            ("warning-fill", self.warning_fill),
            ("on-warning", self.on_warning),
            ("off-fill", self.off_fill),
            ("off-edge", self.off_edge),
            ("lamp-ok", self.lamp_ok),
            ("lamp-warning", self.lamp_warning),
            ("sky", self.sky),
            ("ground", self.ground),
        ]
    }
}

/// Corners and spacing. Corners are concentric: each inset steps the radius down by the inset,
/// starting from the window's own (macOS 26+ rounds windows at about 17 pt; there's no public
/// API for it, so it's measured from a capture).
pub mod geometry {
    pub const WINDOW_RADIUS: f32 = 17.0;
    /// Margin between the window edge and the bezels, and between bezels.
    pub const GAP: f32 = 6.0;
    pub const PANEL_RADIUS: f32 = WINDOW_RADIUS - GAP;
    /// Bezel padding around each screen.
    pub const BEZEL_PAD: f32 = 5.0;
    pub const SCREEN_RADIUS: f32 = PANEL_RADIUS - BEZEL_PAD;
    /// Buttons, badges and annunciator tiles.
    pub const CONTROL_RADIUS: f32 = 4.0;
}

/// Type. The instruments are monospaced throughout. Long reading (the docs) uses the
/// system's sans for the prose and keeps the mono for headings, labels and code.
pub mod typography {
    /// The UI font, by name (GPUI).
    pub const UI: &str = "Menlo";
    /// CSS stack for the mono, starting with the UI font.
    pub const MONO: &str = r#"Menlo, ui-monospace, "SF Mono", "Fira Mono", Consolas, monospace"#;
    /// CSS stack for prose.
    pub const SANS: &str = r#"system-ui, -apple-system, "Segoe UI", Roboto, sans-serif"#;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text roles have to read (WCAG AA, 4.5:1) on every surface text sits on.
    #[test]
    fn text_reads_on_every_surface() {
        for p in [NIGHT, DAY] {
            let inks = [
                ("text", p.text),
                ("muted", p.muted),
                ("ok", p.ok),
                ("caution", p.caution),
                ("warning", p.warning),
                ("info", p.info),
                ("highlight", p.highlight),
            ];
            let surfaces = [
                ("canvas", p.canvas),
                ("bezel", p.bezel),
                ("title", p.title),
                ("screen", p.screen),
                ("control", p.control),
            ];
            for (ink, i) in inks {
                for (surface, s) in surfaces {
                    let c = i.contrast(s);
                    assert!(c >= 4.5, "{}: {ink} on {surface} is {c:.2}:1", p.name);
                }
            }
        }
    }

    /// Badges and lamps: each signal on its own fill.
    #[test]
    fn signals_read_on_their_fills() {
        for p in [NIGHT, DAY] {
            for (what, ink, fill) in [
                ("ok", p.ok, p.ok_fill),
                ("caution", p.caution, p.caution_fill),
                ("warning", p.warning, p.warning_fill),
                ("on-warning", p.on_warning, p.warning_fill),
                ("muted", p.muted, p.off_fill),
            ] {
                let c = ink.contrast(fill);
                assert!(c >= 4.5, "{}: {what} on its fill is {c:.2}:1", p.name);
            }
        }
    }

    #[test]
    fn both_variants_have_every_role() {
        let names = |p: Palette| p.roles().map(|(n, _)| n);
        assert_eq!(names(NIGHT), names(DAY));
        assert_eq!(Rgb(0x4cff7a).css(), "#4cff7a");
        assert_eq!(Rgb(0x4cff7a).srgb()[1], 1.0);
    }
}
