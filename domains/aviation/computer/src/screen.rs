//! The screen of the TX15 as a picture: the flight display that the widget `WIDGETS/T1SFC` draws
//! (480 x 272): lamps along the top, bezelled monitors for the air data, the horizon and the nav
//! data, and ArduPilot's messages below, in the console's palettes. The same layout, in Lua, is in `domains/teleoperation/edgetx/WIDGETS/T1SFC/main.lua`; change them together.

use crate::handset::Screen;
use illustration_theme::{NIGHT, Palette, Rgb};
use std::fmt::Write;

const W: f32 = 480.0;
const H: f32 = 272.0;

impl Screen {
    /// The height above home, in metres.
    pub fn height(&self) -> f32 {
        self.alt - self.home.map_or(self.alt, |h| h.2)
    }

    /// How far and which way home is (metres, degrees).
    pub fn home_distance(&self) -> (f32, f32) {
        let (Some((hlat, hlon, _)), Some(g)) = (self.home, &self.gps) else { return (0.0, 0.0) };
        let north = (g.lat - hlat) * 111_320.0;
        let east = (g.lon - hlon) * 111_320.0 * hlat.to_radians().cos();
        ((north * north + east * east).sqrt() as f32, (east.atan2(north).to_degrees().rem_euclid(360.0)) as f32)
    }

    /// The screen as an SVG picture, in the console's night palette.
    pub fn to_svg(&self) -> String {
        self.to_svg_in(&NIGHT)
    }

    /// The screen as an SVG picture in a palette of the console (the widget's Day option is `DAY`).
    pub fn to_svg_in(&self, p: &Palette) -> String {
        let live = self.last_frame.is_some();
        let c = |x: Rgb| x.css();
        let (white, black) = ("#ffffff", "#000000");
        let (gap, pad, panel_r, screen_r, control_r) = (4.0f32, 5.0f32, 11.0f32, 6.0f32, 4.0f32);
        let mut s = String::new();
        let _ = write!(
            s,
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" font-family="Helvetica, Arial, sans-serif" font-size="12">
<rect width="{W}" height="{H}" fill="{}"/>
"##,
            c(p.canvas)
        );
        // A box with a one-pixel border, both rounded: the console's panels, buttons and lamps.
        let boxed = |s: &mut String, x: f32, y: f32, w: f32, h: f32, r: f32, edge: Rgb, inner: Rgb| {
            let _ = write!(
                s,
                r##"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{}"/><rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="{}"/>
"##,
                c(edge),
                x + 1.0,
                y + 1.0,
                w - 2.0,
                h - 2.0,
                (r - 1.0).max(1.0),
                c(inner)
            );
        };
        let label = |s: &mut String, x: f32, y: f32, text: &str, fill: &str, anchor: &str| {
            let _ = write!(s, r##"<text x="{x}" y="{}" text-anchor="{anchor}" fill="{fill}">{}</text>
"##, y + 11.0, escape(text));
        };
        // A bezelled monitor with a title strip. Returns where its screen is.
        let monitor = |s: &mut String, x: f32, y: f32, w: f32, h: f32, title: &str| {
            boxed(s, x, y, w, h, panel_r, p.bezel_edge, p.bezel);
            label(s, x + pad + 3.0, y + 3.0, title, &c(p.muted), "start");
            (x + pad, y + 19.0, w - 2.0 * pad, h - 19.0 - pad)
        };
        // An annunciator lamp: "off", "ok", "active", "caution" or "warn".
        let lamp = |s: &mut String, x: f32, y: f32, w: f32, h: f32, name: &str, state: &str| {
            let (bg, fg, edge) = match state {
                "ok" => (c(p.lamp_ok), c(p.ok), c(p.ok_dim)),
                "active" => (c(p.ok_fill), c(p.ok), c(p.ok_dim)),
                "caution" => (c(p.caution), black.to_string(), c(p.caution)),
                "warn" => (c(p.lamp_warning), white.to_string(), c(p.warning)),
                _ => (c(p.off_fill), c(p.faint), c(p.off_edge)),
            };
            let _ = write!(
                s,
                r##"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{control_r}" fill="{edge}"/><rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="{bg}"/>
"##,
                x + 1.0,
                y + 1.0,
                w - 2.0,
                h - 2.0,
                control_r - 1.0
            );
            label(s, x + w / 2.0, y + (h - 14.0) / 2.0, name, &fg, "middle");
        };
        // The console's SYSTEMS list: a muted name on the left, the value on the right.
        let systems = |s: &mut String, x: f32, y: f32, w: f32, h: f32, rows: &[(&str, String, Option<Rgb>)]| {
            boxed(s, x, y, w, h, screen_r, Rgb(0), p.screen);
            let row_h = (h - 6.0) / rows.len() as f32;
            for (i, (name, value, color)) in rows.iter().enumerate() {
                let ry = y + 3.0 + i as f32 * row_h + (row_h - 14.0) / 2.0;
                label(s, x + 8.0, ry, name, &c(p.muted), "start");
                let fill = if live { c(color.unwrap_or(p.ok)) } else { c(p.faint) };
                label(s, x + w - 8.0, ry, value, &fill, "end");
            }
        };

        let (x0, w0) = (gap, W - 2.0 * gap);
        // The lamps along the top: the mode, armed, GPS, link and battery.
        let (ly, lh) = (gap, 30.0);
        let mode_w = w0 * 0.34;
        let small = (w0 - mode_w - 4.0 * gap) / 4.0;
        let low = self.percent < 20;
        let lamp_state = |good: bool, bad: &'static str| if !live { "off" } else if good { "ok" } else { bad };
        lamp(&mut s, x0, ly, mode_w, lh, &if live { self.mode.clone() } else { "NO LINK".into() }, if live { "active" } else { "warn" });
        let armed_label = if live && !self.armed { "DISARMED" } else { "ARMED" };
        lamp(&mut s, x0 + mode_w + gap, ly, small, lh, armed_label, if live && self.armed { "caution" } else { "off" });
        lamp(&mut s, x0 + mode_w + gap * 2.0 + small, ly, small, lh, &format!("GPS {}", self.sats), lamp_state(self.sats >= 6, "caution"));
        lamp(&mut s, x0 + mode_w + gap * 3.0 + small * 2.0, ly, small, lh, &format!("LINK {}%", self.quality), lamp_state(self.quality >= 50, "caution"));
        lamp(&mut s, x0 + mode_w + gap * 4.0 + small * 3.0, ly, small, lh, &format!("{:.1}V", self.volts), lamp_state(!low, "warn"));

        // The middle: air data on the left, the horizon, the nav data on the right.
        let mh = 52.0;
        let (py, ph) = (ly + lh + gap, H - lh - mh - 4.0 * gap);
        let side_w = w0 * 0.25;
        let mid_w = w0 - 2.0 * side_w - 2.0 * gap;
        let (sx, sy, sw, sh) = monitor(&mut s, x0, py, side_w, ph, "AIR DATA");
        let climb_color = if self.climb < -3.0 { p.caution } else { p.ok };
        systems(&mut s, sx, sy, sw, sh, &[
            ("IAS", format!("{:.0} m/s", self.airspeed), None),
            ("GS", format!("{:.0} m/s", self.ground_speed), None),
            ("HDG", format!("{:03}", self.heading.floor() as i32), None),
            ("VS", format!("{:+.1} m/s", self.climb), Some(climb_color)),
        ]);
        let (hx, hy, hw, hh) = monitor(&mut s, x0 + side_w + gap, py, mid_w, ph, "ATTITUDE");
        self.horizon(&mut s, p, hx, hy, hw, hh);
        let (dist, _) = self.home_distance();
        let (rx, ry, rw, rh) = monitor(&mut s, x0 + side_w + mid_w + 2.0 * gap, py, side_w, ph, "NAV");
        systems(&mut s, rx, ry, rw, rh, &[
            ("ALT", format!("{:.0} m", self.height()), None),
            ("HOME", format!("{dist:.0} m"), None),
            ("BATT", format!("{}%", self.percent), Some(if low { p.warning } else { p.ok })),
            ("CURR", format!("{:.1} A", self.amps), None),
        ]);

        // ArduPilot's messages along the bottom, in a monitor of their own.
        let (mx, my, mw, mhh) = monitor(&mut s, x0, py + ph + gap, w0, mh, "MESSAGES");
        boxed(&mut s, mx, my, mw, mhh, screen_r, Rgb(0), p.screen);
        if self.messages.is_empty() && live {
            let hint = if self.armed { "Armed. ArduPilot's messages show here." } else { "Disarmed: waiting for ArduPilot's pre-arm checks." };
            label(&mut s, mx + 8.0, my + 3.0, hint, &c(p.muted), "start");
        }
        let first = self.messages.len().saturating_sub(2);
        for (i, m) in self.messages[first..].iter().enumerate() {
            let fill = if m.severity <= 3 { c(p.warning) } else if m.severity == 4 { c(p.caution) } else { c(p.text) };
            let shown: String = m.text.chars().take(60).collect();
            label(&mut s, mx + 8.0, my + 3.0 + i as f32 * 13.0, &shown, &fill, "start");
        }
        s.push_str("</svg>");
        s
    }

    /// The artificial horizon, on a screen with the console's black edge.
    fn horizon(&self, s: &mut String, p: &Palette, x: f32, y: f32, w: f32, h: f32) {
        let (white, yellow) = ("#ffffff", "#ffd200");
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let k = h / 55.0;
        // (The widget draws the pitch the other way round from the frame: the radio showed it inverted.)
        let down = -self.pitch * k;
        let _ = write!(
            s,
            r##"<clipPath id="hud"><rect x="{x}" y="{y}" width="{w}" height="{h}" rx="6"/></clipPath>
<g clip-path="url(#hud)"><rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{}"/>
<g transform="translate({cx} {cy}) rotate({})"><g transform="translate(0 {down})">
<rect x="-400" y="0" width="800" height="400" fill="{}"/><line x1="-400" y1="0" x2="400" y2="0" stroke="{white}" stroke-width="2"/>
"##,
            p.sky.css(),
            -self.roll,
            p.ground.css()
        );
        let roll = self.roll.to_radians();
        let (nx, ny) = (roll.sin(), roll.cos());
        for deg in [-20i32, -10, 10, 20] {
            let ly = -(deg as f32) * k;
            // The widget leaves out a rung that is not near the middle of the screen.
            let (px, py) = (cx + nx * down - nx * deg as f32 * k, cy + ny * down - ny * deg as f32 * k);
            if px > x + 30.0 && px < x + w - 30.0 && py > y + 14.0 && py < y + h - 14.0 {
                let _ = write!(
                    s,
                    r##"<line x1="-22" y1="{ly}" x2="22" y2="{ly}" stroke="{white}" stroke-width="2"/><text x="26" y="{}" font-size="12" fill="{white}">{}</text>
"##,
                    ly + 4.0,
                    deg.abs()
                );
            }
        }
        let _ = write!(s, "</g></g>");
        // The roll arc across the top, and its pointer.
        let r = h / 2.0 - 6.0;
        for deg in [-30.0f32, -15.0, 0.0, 15.0, 30.0] {
            let a = deg.to_radians();
            let _ = write!(
                s,
                r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{white}" stroke-width="2"/>"##,
                cx + a.sin() * r,
                cy - a.cos() * r,
                cx + a.sin() * (r - 8.0),
                cy - a.cos() * (r - 8.0)
            );
        }
        let a = -roll;
        let _ = write!(
            s,
            r##"<polygon points="{},{} {},{} {},{}" fill="{yellow}"/>
<g stroke="{yellow}" stroke-width="2" fill="none"><line x1="{}" y1="{cy}" x2="{}" y2="{cy}"/><line x1="{}" y1="{cy}" x2="{}" y2="{cy}"/><polyline points="{},{cy} {cx},{} {},{cy}"/></g></g>
<rect x="{}" y="{}" width="{}" height="{}" rx="5" fill="none" stroke="#000000" stroke-width="2"/>
"##,
            cx + a.sin() * (r - 8.0),
            cy - a.cos() * (r - 8.0),
            cx + (a - 0.07).sin() * (r - 18.0),
            cy - (a - 0.07).cos() * (r - 18.0),
            cx + (a + 0.07).sin() * (r - 18.0),
            cy - (a + 0.07).cos() * (r - 18.0),
            cx - 34.0,
            cx - 10.0,
            cx + 10.0,
            cx + 34.0,
            cx - 10.0,
            cy + 8.0,
            cx + 10.0,
            x + 1.0,
            y + 1.0,
            w - 2.0,
            h - 2.0
        );
    }
}

fn escape(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_shows_the_mode_and_whether_it_is_armed() {
        let mut s = Screen { mode: "QHOVER".into(), armed: true, last_frame: Some(0.0), ..Default::default() };
        let svg = s.to_svg();
        assert!(svg.contains(">QHOVER<") && svg.contains(">ARMED<"));
        s.armed = false;
        assert!(s.to_svg().contains(">DISARMED<"));
        s.last_frame = None;
        assert!(s.to_svg().contains("NO LINK"));
    }

    #[test]
    fn the_horizon_banks_and_pitches_with_the_plane() {
        let svg = Screen { roll: 30.0, pitch: 10.0, last_frame: Some(0.0), ..Default::default() }.to_svg();
        assert!(svg.contains("rotate(-30)"), "a right bank turns the horizon counter-clockwise");
        assert!(!Screen::default().to_svg().contains("rotate(-30)"));
    }

    #[test]
    fn messages_are_coloured_by_how_bad_they_are() {
        let s = Screen {
            messages: vec![
                crate::Message { severity: 2, text: "PreArm: bad".into() },
                crate::Message { severity: 6, text: "Mode QHOVER".into() },
            ],
            ..Default::default()
        };
        let svg = s.to_svg();
        assert!(svg.contains("#ff5a4f\">PreArm: bad") && svg.contains("#dfe8df\">Mode QHOVER"));
    }

    #[test]
    fn height_and_home_are_measured_from_where_it_stood_disarmed() {
        let g = |lat, alt| aviation_crsf::Gps { lat, lon: 149.0, speed: 0.0, heading: 0.0, alt, sats: 10 };
        let s = Screen { home: Some((-35.0, 149.0, 584.0)), gps: Some(g(-35.0 + 100.0 / 111_320.0, 624.0)), alt: 624.0, ..Default::default() };
        assert_eq!(s.height().round(), 40.0);
        let (d, bearing) = s.home_distance();
        assert!((d - 100.0).abs() < 1.0 && (bearing - 0.0).abs() < 1.0, "{d} {bearing}");
    }
}
