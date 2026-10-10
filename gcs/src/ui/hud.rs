//! Primary flight display: synthetic horizon with HUD-style symbology.

use super::draw::*;
use super::theme::*;
use crate::geo::{angle_diff, wrap360};
use crate::vehicle::Vehicle;
use gpui::{App, Bounds, ContentMask, Pixels, Window};

/// Everything the PFD needs, copied out of the shared vehicle state.
#[derive(Clone)]
pub struct HudData {
    pub alive: bool,
    pub roll: f32,
    pub pitch: f32,
    pub heading: f32,
    pub track: f32,
    pub airspeed: f32,
    pub groundspeed: f32,
    pub climb: f32,
    pub alt: f32,
    pub agl: Option<f32>,
    pub throttle: u16,
    pub mode: &'static str,
    pub armed: bool,
    pub nav: Option<Nav>,
    pub gps: (u8, u8),
    /// (volts, percent, volts per cell)
    pub batt: (f32, i8, Option<f32>),
    pub wind: Option<(f32, f32)>,
    pub flight_secs: f32,
}

#[derive(Clone)]
pub struct Nav {
    pub target_bearing: f32,
    pub wp_dist: f32,
    pub label: String,
    pub target_alt: f32,
    pub target_aspd: f32,
}

/// Modes in which ArduPlane is navigating to a target.
pub fn navigating(mode: &str) -> bool {
    matches!(mode, "AUTO" | "RTL" | "LOITER" | "GUIDED" | "CIRCLE" | "TAKEOFF" | "CRUISE" | "FBWB")
}

impl HudData {
    pub fn from(v: &Vehicle, agl: Option<f32>) -> Self {
        let mode = v.mode();
        let nav = (v.nav_valid && navigating(mode)).then(|| Nav {
            target_bearing: v.target_bearing,
            wp_dist: v.wp_dist,
            label: match mode {
                "GUIDED" => "TGT".to_string(),
                "RTL" => "HOME".to_string(),
                "AUTO" => format!("WP{}", v.mission_seq),
                _ => "NAV".to_string(),
            },
            target_alt: v.alt_rel + v.alt_error,
            target_aspd: v.airspeed + v.aspd_error,
        });
        let track = if v.groundspeed > 1.0 { wrap360(v.ve.atan2(v.vn).to_degrees()) } else { v.heading };
        HudData {
            alive: v.link.alive(),
            roll: v.roll,
            pitch: v.pitch.to_degrees(),
            heading: wrap360(v.yaw.to_degrees()),
            track,
            airspeed: v.airspeed,
            groundspeed: v.groundspeed,
            climb: v.climb,
            alt: v.alt_rel,
            agl,
            throttle: v.throttle,
            mode,
            armed: v.armed,
            nav,
            gps: (v.gps_fix, v.sats),
            batt: (v.batt_v, v.batt_pct, v.cell_v()),
            wind: v.wind_valid.then_some((v.wind_dir, v.wind_speed)),
            flight_secs: v.flight_secs(),
        }
    }
}

/// How the symbology is drawn.
#[derive(Clone, Copy, Default)]
pub struct HudOpts {
    /// Over camera video: no synthetic sky/ground, and pitch scaled to the camera so the
    /// horizon and flight path marker are conformal with the picture.
    pub video_vfov: Option<f32>,
}

pub fn paint(bounds: Bounds<Pixels>, d: &HudData, opts: HudOpts, window: &mut Window, cx: &mut App) {
    window.with_content_mask(Some(ContentMask { bounds }), |window| paint_inner(bounds, d, opts, window, cx));
}

fn paint_inner(bounds: Bounds<Pixels>, d: &HudData, opts: HudOpts, window: &mut Window, cx: &mut App) {
    let (x, y, w, h) = rect(bounds);
    let video = opts.video_vfov.is_some();
    // Over video the boresight is the picture centre.
    let (cx0, cy0) = (x + w * 0.5, y + h * if video { 0.5 } else { 0.52 });
    // Pixels per degree of pitch.
    let ppd = opts.video_vfov.map_or(h / 48.0, |vfov| h / vfov);
    let roll = d.roll;
    let to_screen = |hx: f32, hy: f32| {
        let (rx, ry) = rot(hx, hy + d.pitch * ppd, -roll);
        (cx0 + rx, cy0 + ry)
    };
    let big = (w + h) * 2.0;

    // Sky and ground.
    let sky: Vec<_> = [(-big, -big * 2.0), (big, -big * 2.0), (big, 0.0), (-big, 0.0)].iter().map(|p| to_screen(p.0, p.1)).collect();
    let gnd: Vec<_> = [(-big, 0.0), (big, 0.0), (big, big * 2.0), (-big, big * 2.0)].iter().map(|p| to_screen(p.0, p.1)).collect();
    if !video {
        fill_poly(window, &sky, c(SKY));
        fill_poly(window, &gnd, c(GROUND));
    }
    line(window, to_screen(-big, 0.0), to_screen(big, 0.0), 2.0, c(GREEN));

    let g = c(GREEN);
    let fs = (h / 38.0).clamp(10.0, 15.0);

    // Pitch ladder, clipped to a central window so it doesn't collide with the tapes.
    let ladder = Bounds::from_corners(pt(cx0 - w * 0.27, y + h * 0.16), pt(cx0 + w * 0.27, y + h * 0.88));
    window.with_content_mask(Some(ContentMask { bounds: ladder }), |window| {
        let mut deg: i32 = -90;
        while deg <= 90 {
            if deg != 0 {
                let hy = -(deg as f32) * ppd;
                let major = deg % 10 == 0;
                let half = if major { w * 0.09 } else { w * 0.045 };
                let gap = w * 0.03;
                let tick = if deg > 0 { 8.0 } else { -8.0 };
                for side in [-1.0f32, 1.0] {
                    let a = to_screen(side * gap, hy);
                    let b = to_screen(side * (gap + half), hy);
                    if deg > 0 {
                        line(window, a, b, 1.5, g);
                    } else {
                        dashed(window, &[a, b], 1.5, 6.0, g);
                    }
                    if major {
                        line(window, b, to_screen(side * (gap + half), hy + tick), 1.5, g);
                        let (lx, ly) = to_screen(side * (gap + half + 16.0), hy);
                        text(window, cx, &format!("{}", deg.abs()), lx, ly, fs * 0.9, g, Align::Center);
                    }
                }
            }
            deg += 5;
        }

        // Flight path marker: where the aircraft is actually going.
        if d.groundspeed > 2.0 && d.alive {
            let gamma = d.climb.atan2(d.groundspeed).to_degrees();
            let drift = angle_diff(d.track, d.heading).clamp(-20.0, 20.0);
            let (fx, fy) = to_screen(drift * ppd, -gamma * ppd);
            circle(window, fx, fy, 7.0, 1.8, g);
            line(window, (fx - 17.0, fy), (fx - 7.0, fy), 1.8, g);
            line(window, (fx + 7.0, fy), (fx + 17.0, fy), 1.8, g);
            line(window, (fx, fy - 7.0), (fx, fy - 13.0), 1.8, g);
        }
    });

    // Roll scale (fixed) and sky pointer (moves with the horizon).
    let r = h * 0.34;
    let arc: Vec<_> = (-60..=60)
        .step_by(5)
        .map(|a| {
            let a = (a as f32).to_radians();
            (cx0 + r * a.sin(), cy0 - r * a.cos())
        })
        .collect();
    polyline(window, &arc, 1.5, g);
    for a in [-60, -45, -30, -20, -10, 10, 20, 30, 45, 60] {
        let len = if a % 30 == 0 { 14.0 } else { 8.0 };
        let ar = (a as f32).to_radians();
        let p0 = (cx0 + r * ar.sin(), cy0 - r * ar.cos());
        let p1 = (cx0 + (r + len) * ar.sin(), cy0 - (r + len) * ar.cos());
        line(window, p0, p1, 1.5, g);
    }
    fill_poly(window, &[(cx0, cy0 - r - 2.0), (cx0 - 7.0, cy0 - r - 14.0), (cx0 + 7.0, cy0 - r - 14.0)], g);
    let pr = |dx: f32, dy: f32| {
        let (rx, ry) = rot(dx, dy - r + 2.0, -roll);
        (cx0 + rx, cy0 + ry)
    };
    let bank_col = if roll.to_degrees().abs() > 45.0 { c(AMBER) } else { g };
    fill_poly(window, &[pr(0.0, 0.0), pr(-8.0, 13.0), pr(8.0, 13.0)], bank_col);

    // Fixed aircraft reference.
    let yel = c(AMBER);
    let s = w * 0.06;
    polyline(window, &[(cx0 - s * 2.0, cy0), (cx0 - s * 0.7, cy0), (cx0 - s * 0.35, cy0 + s * 0.4), (cx0, cy0), (cx0 + s * 0.35, cy0 + s * 0.4), (cx0 + s * 0.7, cy0), (cx0 + s * 2.0, cy0)], 3.0, yel);
    fill_circle(window, cx0, cy0, 2.5, yel);

    heading_tape(window, cx, d, x, y, w, fs);
    speed_tape(window, cx, d, x + 14.0, cy0, h * 0.56, fs);
    alt_tape(window, cx, d, x + w - 14.0, cy0, h * 0.56, fs);

    // Corner data blocks.
    let lh = fs * 1.45;
    let bl = y + h - 12.0 - lh * 2.0;
    text(window, cx, d.mode, x + 14.0, bl, fs * 1.35, c(WHITE), Align::Left);
    let (arm_txt, arm_col) = if d.armed { ("ARMED", RED) } else { ("DISARMED", GREEN) };
    text(window, cx, arm_txt, x + 14.0, bl + lh, fs, c(arm_col), Align::Left);
    text(window, cx, &format!("THR {:>3}%", d.throttle), x + 14.0, bl + lh * 2.0, fs, g, Align::Left);

    let br = x + w - 14.0;
    if let Some(n) = &d.nav {
        text(window, cx, &format!("{:<4} {:>5.0} m", n.label, n.wp_dist), br, bl, fs, c(MAGENTA), Align::Right);
        let ete = if d.groundspeed > 1.0 { n.wp_dist / d.groundspeed } else { 0.0 };
        text(window, cx, &format!("ETE {:>2}:{:02}", (ete / 60.0) as u32, (ete % 60.0) as u32), br, bl + lh, fs, c(MAGENTA), Align::Right);
    }
    let ft = d.flight_secs;
    text(window, cx, &format!("FLT {:02}:{:02}", (ft / 60.0) as u32, (ft % 60.0) as u32), br, bl + lh * 2.0, fs, g, Align::Right);

    let tl = y + 70.0;
    let gps_col = if d.gps.0 >= 3 { g } else { c(AMBER) };
    text(window, cx, &format!("GPS {}D {:>2}SV", d.gps.0.min(3), d.gps.1), x + 14.0, tl, fs * 0.9, gps_col, Align::Left);
    if let Some((dir, spd)) = d.wind {
        text(window, cx, &format!("W {:03.0}/{:>4.1}", dir, spd), x + 14.0, tl + lh * 0.9, fs * 0.9, g, Align::Left);
    }
    let batt_col = if d.batt.2.is_some_and(|cv| cv < 3.5) { c(AMBER) } else { g };
    text(window, cx, &format!("{:>5.2} V", d.batt.0), x + w - 14.0, tl, fs * 0.9, batt_col, Align::Right);
    if d.batt.1 >= 0 {
        text(window, cx, &format!("{:>3}%", d.batt.1), x + w - 14.0, tl + lh * 0.9, fs * 0.9, batt_col, Align::Right);
    }

    if !d.alive {
        let red = c(RED);
        line(window, (x + 20.0, y + 20.0), (x + w - 20.0, y + h - 20.0), 3.0, ca(RED, 0.6));
        line(window, (x + w - 20.0, y + 20.0), (x + 20.0, y + h - 20.0), 3.0, ca(RED, 0.6));
        boxed_text(window, cx, "NO TELEMETRY", cx0, cy0 - h * 0.15, fs * 1.6, red, Align::Center, 0.0);
    }
}

fn heading_tape(window: &mut Window, cx: &mut App, d: &HudData, x: f32, y: f32, w: f32, fs: f32) {
    let g = c(GREEN);
    let cx0 = x + w * 0.5;
    let tw = w * 0.5;
    let base = y + 44.0;
    let hp = tw / 70.0;
    let tape = Bounds::from_corners(pt(cx0 - tw * 0.5, y), pt(cx0 + tw * 0.5, base + 4.0));
    window.with_content_mask(Some(ContentMask { bounds: tape }), |window| {
        line(window, (cx0 - tw * 0.5, base), (cx0 + tw * 0.5, base), 1.5, g);
        let start = (d.heading - 40.0).floor() as i32;
        for deg in start..=start + 80 {
            if deg.rem_euclid(5) != 0 {
                continue;
            }
            let px = cx0 + angle_diff(deg as f32, d.heading) * hp;
            let major = deg.rem_euclid(10) == 0;
            line(window, (px, base), (px, base - if major { 10.0 } else { 5.0 }), 1.5, g);
            if major {
                let v = deg.rem_euclid(360);
                let label = match v {
                    0 => "N".to_string(),
                    90 => "E".to_string(),
                    180 => "S".to_string(),
                    270 => "W".to_string(),
                    _ => format!("{:02}", v / 10),
                };
                text(window, cx, &label, px, base - 20.0, fs * 0.9, g, Align::Center);
            }
        }
        if let Some(n) = &d.nav {
            let px = cx0 + angle_diff(n.target_bearing, d.heading).clamp(-35.0, 35.0) * hp;
            fill_poly(window, &[(px, base + 1.0), (px - 7.0, base - 9.0), (px + 7.0, base - 9.0)], c(MAGENTA));
        }
        // Ground track caret.
        let tx = cx0 + angle_diff(d.track, d.heading).clamp(-35.0, 35.0) * hp;
        circle(window, tx, base + 0.0, 3.0, 1.5, c(CYAN));
    });
    boxed_text(window, cx, &format!("{:03.0}", wrap360(d.heading.round())), cx0, base + 16.0, fs * 1.15, g, Align::Center, 0.0);
}

fn tape_box(window: &mut Window, x0: f32, cy: f32, tw: f32, th: f32) {
    fill_rect(window, x0, cy - th * 0.5, tw, th, ca(0x000000, 0.45));
    stroke_rect(window, x0, cy - th * 0.5, tw, th, 1.0, ca(GREEN, 0.5));
}

fn speed_tape(window: &mut Window, cx: &mut App, d: &HudData, x: f32, cy: f32, th: f32, fs: f32) {
    let g = c(GREEN);
    let tw = 74.0;
    tape_box(window, x, cy, tw, th);
    let range = 15.0;
    let pp = th / (2.0 * range);
    let to_y = |v: f32| cy - (v - d.airspeed) * pp;
    let tape = Bounds::from_corners(pt(x, cy - th * 0.5), pt(x + tw, cy + th * 0.5));
    window.with_content_mask(Some(ContentMask { bounds: tape }), |window| {
        // Stall (red) and below-minimum (amber) bands, from the airframe params.
        let (stall, min) = (9.5, 11.0);
        fill_rect(window, x + tw - 6.0, to_y(stall), 6.0, (to_y(-10.0) - to_y(stall)).max(0.0), c(RED));
        fill_rect(window, x + tw - 6.0, to_y(min), 6.0, (to_y(stall) - to_y(min)).max(0.0), c(AMBER));
        let lo = (d.airspeed - range).floor().max(0.0) as i32;
        for v in lo..=(d.airspeed + range).ceil() as i32 {
            let yy = to_y(v as f32);
            let major = v % 5 == 0;
            line(window, (x + tw - if major { 16.0 } else { 9.0 }, yy), (x + tw - 6.0, yy), 1.5, g);
            if major {
                text(window, cx, &format!("{v}"), x + tw - 22.0, yy, fs * 0.9, g, Align::Right);
            }
        }
        if let Some(n) = &d.nav {
            let yy = to_y(n.target_aspd);
            fill_poly(window, &[(x + tw, yy), (x + tw + 8.0, yy - 7.0), (x + tw + 8.0, yy + 7.0)], c(MAGENTA));
            line(window, (x + tw - 6.0, yy), (x + tw, yy), 3.0, c(MAGENTA));
        }
    });
    let col = if d.airspeed < 9.5 && d.armed { c(RED) } else { g };
    boxed_text(window, cx, &format!("{:>4.1}", d.airspeed), x + tw * 0.45, cy, fs * 1.25, col, Align::Center, 44.0);
    text(window, cx, "IAS m/s", x + tw * 0.5, cy - th * 0.5 - 12.0, fs * 0.8, g, Align::Center);
    text(window, cx, &format!("GS {:>4.1}", d.groundspeed), x + tw * 0.5, cy + th * 0.5 + 13.0, fs * 0.9, g, Align::Center);
}

fn alt_tape(window: &mut Window, cx: &mut App, d: &HudData, right: f32, cy: f32, th: f32, fs: f32) {
    let g = c(GREEN);
    let tw = 84.0;
    let x = right - tw;
    tape_box(window, x, cy, tw, th);
    let range = 40.0;
    let pp = th / (2.0 * range);
    let to_y = |v: f32| cy - (v - d.alt) * pp;
    let tape = Bounds::from_corners(pt(x, cy - th * 0.5), pt(x + tw, cy + th * 0.5));
    window.with_content_mask(Some(ContentMask { bounds: tape }), |window| {
        if let Some(agl) = d.agl {
            // Terrain under the aircraft, from the sim's exported height grid.
            let gy = to_y(d.alt - agl);
            fill_rect(window, x, gy, tw, (cy + th).max(gy) - gy, ca(GROUND, 0.95));
            line(window, (x, gy), (x + tw, gy), 2.0, c(AMBER));
        }
        let lo = ((d.alt - range) / 2.0).floor() as i32 * 2;
        let mut v = lo;
        while v as f32 <= d.alt + range {
            let yy = to_y(v as f32);
            let major = v % 10 == 0;
            line(window, (x, yy), (x + if major { 12.0 } else { 6.0 }, yy), 1.5, g);
            if major {
                text(window, cx, &format!("{v}"), x + 18.0, yy, fs * 0.9, g, Align::Left);
            }
            v += 2;
        }
        if let Some(n) = &d.nav {
            let yy = to_y(n.target_alt);
            fill_poly(window, &[(x, yy), (x - 8.0, yy - 7.0), (x - 8.0, yy + 7.0)], c(MAGENTA));
            line(window, (x, yy), (x + 8.0, yy), 3.0, c(MAGENTA));
        }
    });
    boxed_text(window, cx, &format!("{:>5.1}", d.alt), x + tw * 0.55, cy, fs * 1.25, g, Align::Center, 52.0);
    text(window, cx, "ALT m", x + tw * 0.5, cy - th * 0.5 - 12.0, fs * 0.8, g, Align::Center);
    let vs_col = if d.climb < -3.0 { c(AMBER) } else { g };
    text(window, cx, &format!("VS {:>+4.1}", d.climb), x + tw * 0.5, cy + th * 0.5 + 13.0, fs * 0.9, vs_col, Align::Center);
    if let Some(agl) = d.agl {
        let col = if agl < 15.0 && d.armed { c(AMBER) } else { g };
        text(window, cx, &format!("AGL {:>4.0}", agl), x + tw * 0.5, cy + th * 0.5 + 13.0 + fs * 1.3, fs * 0.9, col, Align::Center);
    }
}
