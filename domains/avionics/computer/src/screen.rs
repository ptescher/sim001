//! The screen of the TX15 as a picture: the flight display that the widget `WIDGETS/T1SFC` draws
//! (480 x 272), in the layout of Yaapu's ArduPilot telemetry script. The same layout, in Lua,
//! is in `domains/teleoperation/edgetx/WIDGETS/T1SFC/main.lua`; change them together.

use crate::handset::Screen;
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

    /// The screen as an SVG picture.
    pub fn to_svg(&self) -> String {
        let live = self.last_frame.is_some();
        let (white, grey) = ("#ffffff", "#96a0aa");
        let armed = if self.armed { "#3cc85a" } else { "#e6463c" };
        let mut s = String::new();
        let _ = write!(
            s,
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" font-family="Helvetica, Arial, sans-serif">
<rect width="{W}" height="{H}" fill="#0c1016"/>
<rect width="{W}" height="26" fill="#181e28"/>
<text x="6" y="22" font-size="26" font-weight="bold" fill="{white}">{mode}</text>
<text x="{armed_x}" y="21" font-size="18" fill="{armed}">{state}</text>
<text x="{right}" y="17" font-size="12" text-anchor="end" fill="{grey}">GPS {sats}  LINK {link}%  {volts:.1}V</text>
"##,
            mode = if live { escape(&self.mode) } else { "NO LINK".into() },
            armed_x = W * 0.30,
            state = if !live { "" } else if self.armed { "ARMED" } else { "DISARMED" },
            right = W - 6.0,
            sats = self.sats,
            link = self.quality,
            volts = self.volts,
        );
        // The horizon.
        let (hud_w, hud_h) = (W * 0.52, H - 30.0 - 44.0);
        let (hx, hy) = ((W - hud_w) / 2.0, 30.0);
        let (cx, cy) = (hx + hud_w / 2.0, hy + hud_h / 2.0);
        let k = hud_h / 55.0;
        let _ = write!(
            s,
            r##"<clipPath id="hud"><rect x="{hx}" y="{hy}" width="{hud_w}" height="{hud_h}"/></clipPath>
<g clip-path="url(#hud)"><rect x="{hx}" y="{hy}" width="{hud_w}" height="{hud_h}" fill="#286ec8"/>
<g transform="translate({cx} {cy}) rotate({rot})"><g transform="translate(0 {down})">
<rect x="-400" y="0" width="800" height="400" fill="#78501e"/><line x1="-400" y1="0" x2="400" y2="0" stroke="{white}" stroke-width="2"/>
"##,
            rot = -self.roll,
            down = self.pitch * k,
        );
        for deg in [-20i32, -10, 10, 20] {
            let y = -(deg as f32) * k;
            let _ = write!(
                s,
                r##"<line x1="-22" y1="{y}" x2="22" y2="{y}" stroke="{white}" stroke-width="1.5"/><text x="26" y="{ty}" font-size="10" fill="{white}">{n}</text>
"##,
                ty = y + 3.0,
                n = deg.abs()
            );
        }
        let r = hud_h / 2.0 - 6.0;
        let _ = write!(s, "</g></g>");
        for deg in [-30.0f32, -15.0, 0.0, 15.0, 30.0] {
            let a = deg.to_radians();
            let _ = write!(
                s,
                r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{white}"/>"##,
                cx + a.sin() * r,
                cy - a.cos() * r,
                cx + a.sin() * (r - 6.0),
                cy - a.cos() * (r - 6.0)
            );
        }
        let a = -self.roll.to_radians();
        let _ = write!(
            s,
            r##"<polygon points="{},{} {},{} {},{}" fill="#ffd200"/>
<g stroke="#ffd200" stroke-width="2" fill="none"><line x1="{}" y1="{cy}" x2="{}" y2="{cy}"/><line x1="{}" y1="{cy}" x2="{}" y2="{cy}"/><polyline points="{},{cy} {cx},{} {},{cy}"/></g></g>
<rect x="{hx}" y="{hy}" width="{hud_w}" height="{hud_h}" fill="none" stroke="{white}"/>
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
        );
        let (dist, _) = self.home_distance();
        let mut label = |x: f32, y: f32, name: &str, value: String, unit: &str| {
            let _ = write!(
                s,
                r##"<text x="{x}" y="{}" font-size="10" fill="{grey}">{name}</text><text x="{}" y="{}" font-size="22" font-weight="bold" fill="{white}">{value}</text><text x="{}" y="{}" font-size="10" fill="{grey}">{unit}</text>
"##,
                y + 8.0,
                x + 36.0,
                y + 12.0,
                x + 36.0,
                y + 26.0
            );
        };
        label(6.0, 38.0, "AS", format!("{:.0}", self.airspeed), "m/s");
        label(6.0, 86.0, "GS", format!("{:.0}", self.ground_speed), "m/s");
        label(6.0, 134.0, "HDG", format!("{:03.0}", self.heading), "");
        let rx = hx + hud_w + 8.0;
        label(rx, 38.0, "ALT", format!("{:.0}", self.height()), "m");
        label(rx, 86.0, "VS", format!("{:+.1}", self.climb), "m/s");
        label(rx, 134.0, "HOME", format!("{dist:.0}"), "m");
        // The battery.
        let (bx, by) = (W - 74.0, 30.0 + hud_h - 26.0);
        let pct = self.percent.min(100) as f32;
        let _ = write!(
            s,
            r##"<text x="{bx}" y="{}" font-size="10" fill="{white}">{:.1}A  {}mAh</text><rect x="{bx}" y="{}" width="68" height="12" fill="none" stroke="{grey}"/><rect x="{}" y="{}" width="{}" height="10" fill="{}"/>
"##,
            by,
            self.amps,
            self.used_mah,
            by + 8.0,
            bx + 1.0,
            by + 9.0,
            66.0 * pct / 100.0,
            if pct > 30.0 { "#3cc85a" } else { "#e6463c" }
        );
        // ArduPilot's messages.
        let _ = write!(s, r##"<rect y="{}" width="{W}" height="42" fill="#181e28"/>"##, H - 42.0);
        let first = self.messages.len().saturating_sub(3);
        for (i, m) in self.messages[first..].iter().enumerate() {
            let c = if m.severity <= 3 { "#e6463c" } else if m.severity == 4 { "#f0be3c" } else { white };
            let _ = write!(s, r##"<text x="6" y="{}" font-size="11" fill="{c}">{}</text>"##, H - 30.0 + i as f32 * 13.0, escape(&m.text));
        }
        s.push_str("</svg>");
        s
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
        assert!(svg.contains("#e6463c\">PreArm: bad") && svg.contains("#ffffff\">Mode QHOVER"));
    }

    #[test]
    fn height_and_home_are_measured_from_where_it_stood_disarmed() {
        let g = |lat, alt| avionics_crsf::Gps { lat, lon: 149.0, speed: 0.0, heading: 0.0, alt, sats: 10 };
        let s = Screen { home: Some((-35.0, 149.0, 584.0)), gps: Some(g(-35.0 + 100.0 / 111_320.0, 624.0)), alt: 624.0, ..Default::default() };
        assert_eq!(s.height().round(), 40.0);
        let (d, bearing) = s.home_distance();
        assert!((d - 100.0).abs() < 1.0 && (bearing - 0.0).abs() < 1.0, "{d} {bearing}");
    }
}
