//! Tactical situation display: the sim's terrain orthophoto with home, mission, flight plan,
//! guided target, ground track and the aircraft, in metres north/east of the origin.

use super::draw::*;
use super::theme::*;
use crate::geo::{Origin, TerrainMap, wrap360};
use crate::vehicle::{Vehicle, Waypoint, WpKind};
use gpui::{App, Bounds, ContentMask, Corners, Pixels, RenderImage, Window};
use std::sync::Arc;

/// ArduPlane's default WP_LOITER_RAD.
pub const LOITER_RADIUS: f32 = 60.0;

#[derive(Clone, Copy, PartialEq)]
pub enum Tool {
    Pan,
    Goto,
    Plan,
    /// Point the aircraft camera at a ground point and keep it there.
    Look,
}

pub struct MapState {
    pub centre: (f32, f32),
    /// Metres per screen pixel.
    pub mpp: f32,
    pub follow: bool,
    pub tool: Tool,
    pub plan: Vec<Waypoint>,
    pub plan_alt: f32,
    pub drag: Option<((f32, f32), (f32, f32))>,
    pub cursor: Option<(f32, f32)>,
    pub bounds: Bounds<Pixels>,
}

impl MapState {
    pub fn new() -> Self {
        Self {
            centre: (60.0, 0.0),
            mpp: 0.9,
            follow: true,
            tool: Tool::Pan,
            plan: vec![],
            plan_alt: 40.0,
            drag: None,
            cursor: None,
            bounds: Bounds::default(),
        }
    }

    pub fn screen_to_ne(&self, sx: f32, sy: f32) -> (f32, f32) {
        let (x, y, w, h) = rect(self.bounds);
        let e = self.centre.1 + (sx - (x + w * 0.5)) * self.mpp;
        let n = self.centre.0 - (sy - (y + h * 0.5)) * self.mpp;
        (n, e)
    }

    pub fn zoom_at(&mut self, sx: f32, sy: f32, factor: f32) {
        let before = self.screen_to_ne(sx, sy);
        self.mpp = (self.mpp * factor).clamp(0.08, 12.0);
        let after = self.screen_to_ne(sx, sy);
        self.centre.0 += before.0 - after.0;
        self.centre.1 += before.1 - after.1;
    }
}

pub fn load_image(map: &TerrainMap) -> Option<Arc<RenderImage>> {
    let img = image::open(&map.png).ok()?;
    let mut buf = img.into_rgba8();
    // GPUI's image atlas is BGRA.
    for p in buf.pixels_mut() {
        p.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
}

/// Another aircraft, as the map draws it.
pub struct Marker {
    pub at: (f32, f32),
    pub heading: f32,
    pub velocity: (f32, f32),
    /// Not heard for a while: the marker is dim and stays where the aircraft was last heard.
    pub stale: bool,
    pub name: String,
    /// The sources and the height: `MAV RID 31 m`.
    pub detail: String,
}

/// Snapshot of what the map draws, so the vehicle lock isn't held while painting.
pub struct MapData {
    pub origin: Origin,
    pub alive: bool,
    pub aircraft: Option<(f32, f32)>,
    pub heading: f32,
    pub velocity: (f32, f32),
    pub home: Option<(f32, f32)>,
    pub mission: Vec<(WpKind, Option<(f32, f32)>, f32)>,
    pub mission_seq: u16,
    pub guided: Option<(f32, f32)>,
    pub trail: Vec<(f32, f32)>,
    pub wind: Option<(f32, f32)>,
    pub mode: &'static str,
    /// Where the camera's line of sight meets the ground, and the tracked point if any.
    pub spi: Option<(f32, f32)>,
    pub track: Option<(f32, f32)>,
    /// The cones that mark the arena of a game of tag.
    pub cones: Vec<(f32, f32)>,
    /// The other aircraft that the radio has told the station about.
    pub traffic: Vec<Marker>,
}

impl MapData {
    fn markers(v: &Vehicle, origin: Origin) -> Vec<Marker> {
        let now = v.start.elapsed().as_secs_f64();
        let home_alt = v.home.map(|h| h.2);
        v.other_aircraft()
            .iter()
            .map(|t| {
                let live = t.freshness == communication_traffic::Freshness::Live;
                // A live aircraft is drawn where it has probably got to since its last report.
                let p = if live { t.estimate(now) } else { t.position };
                let velocity = t.velocity.map_or((0.0, 0.0), |v| (v[0], v[1]));
                let heading = t.heading.unwrap_or_else(|| velocity.1.atan2(velocity.0).to_degrees());
                let height = match home_alt {
                    Some(h) if p.alt.is_finite() => format!(" {:.0} m", p.alt - h),
                    _ => String::new(),
                };
                Marker {
                    at: origin.to_ne(p.lat, p.lon),
                    heading,
                    velocity,
                    stale: !live,
                    name: t.name.clone(),
                    detail: format!("{}{height}", t.source_label()),
                }
            })
            .collect()
    }

    pub fn from(v: &Vehicle, origin: Origin) -> Self {
        let ne = |lat: f64, lon: f64| origin.to_ne(lat, lon);
        MapData {
            origin,
            alive: v.link.alive(),
            aircraft: v.have_position.then(|| ne(v.lat, v.lon)),
            heading: v.yaw.to_degrees(),
            velocity: (v.vn, v.ve),
            home: v.home.map(|h| ne(h.0, h.1)),
            mission: v.mission.iter().map(|w| (w.kind, w.has_position().then(|| ne(w.lat, w.lon)), w.alt)).collect(),
            mission_seq: v.mission_seq,
            guided: v.guided_target.map(|g| ne(g.0, g.1)),
            trail: v.trail.iter().map(|p| ne(p.0, p.1)).collect(),
            wind: v.wind_valid.then_some((v.wind_dir, v.wind_speed)),
            mode: v.mode(),
            spi: None,
            track: None,
            cones: vec![],
            traffic: Self::markers(v, origin),
        }
    }
}

pub fn paint(
    bounds: Bounds<Pixels>,
    st: &MapState,
    d: &MapData,
    terrain: Option<&TerrainMap>,
    image: Option<&Arc<RenderImage>>,
    window: &mut Window,
    cx: &mut App,
) {
    window.with_content_mask(Some(ContentMask { bounds }), |window| paint_inner(bounds, st, d, terrain, image, window, cx));
}

fn paint_inner(
    bounds: Bounds<Pixels>,
    st: &MapState,
    d: &MapData,
    terrain: Option<&TerrainMap>,
    image: Option<&Arc<RenderImage>>,
    window: &mut Window,
    cx: &mut App,
) {
    let (x, y, w, h) = rect(bounds);
    let (mx, my) = (x + w * 0.5, y + h * 0.5);
    let to_s = |n: f32, e: f32| (mx + (e - st.centre.1) / st.mpp, my - (n - st.centre.0) / st.mpp);
    let fs = 11.5;

    // Base layer.
    fill_rect(window, x, y, w, h, c(MAP_BG));
    match (terrain, image) {
        (Some(t), Some(img)) => {
            let half = t.size_m * 0.5;
            let (l, top) = to_s(t.centre_n + half, t.centre_e - half);
            let side = t.size_m / st.mpp;
            let b = Bounds { origin: pt(l, top), size: gpui::size(gpui::px(side), gpui::px(side)) };
            let _ = window.paint_image(b, Corners::default(), img.clone(), 0, false);
            fill_rect(window, x, y, w, h, ca(0x000000, 0.22));
        }
        _ => {
            // No exported map: a 100 m grid.
            let step = 100.0;
            let (n0, e0) = st.screen_to_ne(x, y + h);
            let (n1, e1) = st.screen_to_ne(x + w, y);
            let mut e = (e0 / step).floor() * step;
            while e <= e1 {
                let (sx, _) = to_s(0.0, e);
                line(window, (sx, y), (sx, y + h), 1.0, ca(GREEN_DIM, 0.35));
                e += step;
            }
            let mut n = (n0 / step).floor() * step;
            while n <= n1 {
                let (_, sy) = to_s(n, 0.0);
                line(window, (x, sy), (x + w, sy), 1.0, ca(GREEN_DIM, 0.35));
                n += step;
            }
        }
    }

    // Runway outline (also drawn into the orthophoto; this keeps it visible on the grid).
    if let Some(r) = terrain.and_then(|t| t.runway) {
        let (hl, hw) = (r.length * 0.5, r.width * 0.5);
        let hd = r.heading_deg.to_radians();
        let corner = |a: f32, c: f32| {
            let (s, co) = hd.sin_cos();
            to_s(r.north + a * co - c * s, r.east + a * s + c * co)
        };
        let pts = [corner(-hl, -hw), corner(hl, -hw), corner(hl, hw), corner(-hl, hw), corner(-hl, -hw)];
        polyline(window, &pts, 1.0, ca(WHITE, 0.5));
    }

    // Range rings around home.
    let home = d.home.unwrap_or((0.0, 0.0));
    let (hx, hy) = to_s(home.0, home.1);
    for r in [100.0, 250.0, 500.0, 1000.0] {
        let rp = r / st.mpp;
        if rp < 25.0 || rp > (w + h) * 2.0 {
            continue;
        }
        dashed_circle(window, hx, hy, rp, 1.0, 5.0, ca(GREEN, 0.45));
        text(window, cx, &format!("{r:.0}"), hx + rp * 0.7071 + 4.0, hy - rp * 0.7071 - 6.0, fs * 0.85, ca(GREEN, 0.75), Align::Left);
    }

    // Ground track.
    let trail: Vec<_> = d.trail.iter().map(|p| to_s(p.0, p.1)).collect();
    polyline(window, &trail, 2.0, ca(0xffffff, 0.55));

    // Vehicle mission. Item 0 is home on ArduPilot.
    let pts: Vec<(usize, (f32, f32))> = d
        .mission
        .iter()
        .enumerate()
        .skip(1)
        .filter_map(|(i, m)| m.1.map(|p| (i, to_s(p.0, p.1))))
        .collect();
    let path: Vec<_> = pts.iter().map(|p| p.1).collect();
    polyline(window, &path, 2.0, c(CYAN));
    for (i, (sx, sy)) in &pts {
        let (kind, _, alt) = d.mission[*i];
        let current = *i as u16 == d.mission_seq && matches!(d.mode, "AUTO");
        if matches!(kind, WpKind::LoiterUnlim | WpKind::LoiterTurns) {
            dashed_circle(window, *sx, *sy, LOITER_RADIUS / st.mpp, 1.2, 4.0, ca(CYAN, 0.7));
        }
        let col = if current { MAGENTA } else { CYAN };
        fill_circle(window, *sx, *sy, 9.0, c(0x000000));
        circle(window, *sx, *sy, 9.0, 2.0, c(col));
        text(window, cx, &format!("{i}"), *sx, *sy, fs * 0.9, c(col), Align::Center);
        text(window, cx, &format!("{} {:.0}m", kind.label(), alt), *sx + 13.0, *sy + 11.0, fs * 0.8, ca(col, 0.9), Align::Left);
    }

    // Flight plan being edited.
    let plan: Vec<_> = st.plan.iter().map(|wp| d.origin.to_ne(wp.lat, wp.lon)).map(|p| to_s(p.0, p.1)).collect();
    dashed(window, &plan, 2.0, 8.0, c(AMBER));
    for (i, (sx, sy)) in plan.iter().enumerate() {
        fill_rect(window, sx - 9.0, sy - 9.0, 18.0, 18.0, c(0x000000));
        stroke_rect(window, sx - 9.0, sy - 9.0, 18.0, 18.0, 2.0, c(AMBER));
        text(window, cx, &format!("{}", i + 1), *sx, *sy, fs * 0.9, c(AMBER), Align::Center);
        let wp = &st.plan[i];
        text(window, cx, &format!("{} {:.0}m", wp.kind.label(), wp.alt), sx + 13.0, sy + 11.0, fs * 0.8, c(AMBER), Align::Left);
    }

    // Home.
    fill_circle(window, hx, hy, 10.0, c(0x000000));
    circle(window, hx, hy, 10.0, 2.0, c(AMBER));
    text(window, cx, "H", hx, hy, fs, c(AMBER), Align::Center);

    // Guided target with its loiter circle.
    if let Some(g) = d.guided {
        let (gx, gy) = to_s(g.0, g.1);
        dashed_circle(window, gx, gy, LOITER_RADIUS / st.mpp, 1.5, 5.0, c(MAGENTA));
        line(window, (gx - 12.0, gy), (gx + 12.0, gy), 2.0, c(MAGENTA));
        line(window, (gx, gy - 12.0), (gx, gy + 12.0), 2.0, c(MAGENTA));
    }

    // The cones round the arena of a game of tag.
    for &(n, e) in &d.cones {
        let (cx0, cy0) = to_s(n, e);
        fill_poly(window, &[(cx0, cy0 - 6.0), (cx0 + 5.0, cy0 + 4.0), (cx0 - 5.0, cy0 + 4.0)], ca(AMBER, 0.95));
    }

    // The other aircraft: a small plane, a 10 s leader while it is live, its name and where we heard it.
    for m in &d.traffic {
        let (mx, my) = to_s(m.at.0, m.at.1);
        let col = if m.stale { c(GREY) } else { c(CYAN) };
        if !m.stale && (m.velocity.0 != 0.0 || m.velocity.1 != 0.0) {
            let lead = to_s(m.at.0 + m.velocity.0 * 10.0, m.at.1 + m.velocity.1 * 10.0);
            dashed(window, &[(mx, my), lead], 1.2, 4.0, ca(CYAN, 0.8));
        }
        let hd = m.heading.to_radians();
        let shape = [(0.0, -11.0), (2.0, -4.0), (10.0, 1.0), (10.0, 3.0), (2.0, 1.5), (1.5, 7.0), (4.0, 9.5), (4.0, 11.0), (0.0, 9.5), (-4.0, 11.0), (-4.0, 9.5), (-1.5, 7.0), (-2.0, 1.5), (-10.0, 3.0), (-10.0, 1.0), (-2.0, -4.0)];
        let pts: Vec<_> = shape
            .iter()
            .map(|&(px, py)| {
                let (rx, ry) = rot(px, py, hd);
                (mx + rx, my + ry)
            })
            .collect();
        let outline: Vec<_> = pts.iter().chain(std::iter::once(&pts[0])).copied().collect();
        polyline(window, &outline, 3.0, c(0x000000));
        fill_poly(window, &pts, col);
        text(window, cx, &m.name, mx + 14.0, my - 4.0, fs * 0.85, col, Align::Left);
        text(window, cx, &m.detail, mx + 14.0, my + 9.0, fs * 0.75, col, Align::Left);
    }

    // Camera line of sight to the sensor point of interest, and the tracked point.
    if let (Some(a), Some(p)) = (d.aircraft, d.spi) {
        let (ax, ay) = to_s(a.0, a.1);
        let (px0, py0) = to_s(p.0, p.1);
        dashed(window, &[(ax, ay), (px0, py0)], 1.5, 6.0, ca(SPI, 0.85));
        fill_poly(window, &[(px0, py0 - 8.0), (px0 + 8.0, py0), (px0, py0 + 8.0), (px0 - 8.0, py0)], ca(SPI, 0.9));
        text(window, cx, "SPI", px0 + 12.0, py0, fs * 0.85, c(SPI), Align::Left);
    }
    if let Some(t) = d.track {
        let (tx, ty) = to_s(t.0, t.1);
        stroke_rect(window, tx - 11.0, ty - 11.0, 22.0, 22.0, 2.0, c(AMBER));
    }

    // Aircraft with a 20 s velocity leader.
    if let Some(a) = d.aircraft {
        let (ax, ay) = to_s(a.0, a.1);
        let col = if d.alive { c(GREEN) } else { c(GREY) };
        let lead = to_s(a.0 + d.velocity.0 * 20.0, a.1 + d.velocity.1 * 20.0);
        dashed(window, &[(ax, ay), lead], 1.5, 5.0, col);
        if let Some(t) = d.mission.get(d.mission_seq as usize).and_then(|m| m.1).filter(|_| d.mode == "AUTO") {
            let (tx, ty) = to_s(t.0, t.1);
            dashed(window, &[(ax, ay), (tx, ty)], 1.0, 4.0, ca(MAGENTA, 0.7));
        }
        let hd = d.heading.to_radians();
        let shape = [(0.0, -16.0), (3.0, -6.0), (15.0, 1.0), (15.0, 4.0), (3.0, 2.0), (2.0, 10.0), (6.0, 14.0), (6.0, 16.0), (0.0, 14.0), (-6.0, 16.0), (-6.0, 14.0), (-2.0, 10.0), (-3.0, 2.0), (-15.0, 4.0), (-15.0, 1.0), (-3.0, -6.0)];
        let pts: Vec<_> = shape
            .iter()
            .map(|&(px, py)| {
                let (rx, ry) = rot(px, py, hd);
                (ax + rx, ay + ry)
            })
            .collect();
        let outline: Vec<_> = pts.iter().chain(std::iter::once(&pts[0])).copied().collect();
        polyline(window, &outline, 4.0, c(0x000000));
        fill_poly(window, &pts, col);
    }

    // North arrow.
    let (nx, ny) = (x + 26.0, y + 34.0);
    fill_poly(window, &[(nx, ny - 16.0), (nx - 7.0, ny + 6.0), (nx, ny + 1.0), (nx + 7.0, ny + 6.0)], c(WHITE));
    text(window, cx, "N", nx, ny - 26.0 + 0.0, fs, c(WHITE), Align::Center);

    // Wind.
    if let Some((dir, spd)) = d.wind {
        let (wx, wy) = (x + w - 40.0, y + 40.0);
        circle(window, wx, wy, 22.0, 1.0, ca(WHITE, 0.6));
        // Arrow points downwind (direction is where it blows from).
        let a = (dir + 180.0).to_radians();
        let tip = (wx + 18.0 * a.sin(), wy - 18.0 * a.cos());
        let tail = (wx - 18.0 * a.sin(), wy + 18.0 * a.cos());
        line(window, tail, tip, 2.0, c(CYAN));
        let (l1, l2) = (rot(-5.0, 8.0, a), rot(5.0, 8.0, a));
        fill_poly(window, &[tip, (tip.0 + l1.0, tip.1 + l1.1), (tip.0 + l2.0, tip.1 + l2.1)], c(CYAN));
        text(window, cx, &format!("{dir:03.0}/{spd:.1}"), wx, wy + 34.0, fs * 0.85, c(CYAN), Align::Center);
    }

    // Scale bar.
    let target_px = w * 0.18;
    let raw = target_px * st.mpp;
    let mag = 10f32.powf(raw.log10().floor());
    let nice = [1.0, 2.0, 5.0, 10.0].iter().map(|k| k * mag).find(|v| *v >= raw * 0.6).unwrap_or(mag);
    let bar = nice / st.mpp;
    let (sbx, sby) = (x + 16.0, y + h - 18.0);
    fill_rect(window, sbx - 6.0, sby - 22.0, bar + 12.0, 30.0, ca(0x000000, 0.55));
    polyline(window, &[(sbx, sby - 6.0), (sbx, sby), (sbx + bar, sby), (sbx + bar, sby - 6.0)], 2.0, c(WHITE));
    text(window, cx, &format!("{nice:.0} m"), sbx + bar * 0.5, sby - 12.0, fs * 0.9, c(WHITE), Align::Center);

    // Cursor readout.
    if let Some((n, e)) = st.cursor {
        let (lat, lon) = d.origin.to_ll(n, e);
        let elev = terrain.and_then(|t| t.height(n, e));
        let mut s = format!("{lat:.6} {lon:.6}  N{n:+.0} E{e:+.0}");
        if let Some(el) = elev {
            s.push_str(&format!("  GND {:+.0} m", el));
        }
        if let Some(a) = d.aircraft {
            let (dn, de) = (n - a.0, e - a.1);
            s.push_str(&format!("  {:.0} m @ {:03.0}", (dn * dn + de * de).sqrt(), wrap360(de.atan2(dn).to_degrees())));
        }
        let tw = s.len() as f32 * fs * 0.6 + 12.0;
        fill_rect(window, x + w - tw - 8.0, y + h - 26.0, tw, 20.0, ca(0x000000, 0.6));
        text(window, cx, &s, x + w - 14.0, y + h - 16.0, fs * 0.9, c(WHITE), Align::Right);
    }

    // Tool hint.
    let hint = match st.tool {
        Tool::Pan => None,
        Tool::Goto => Some(("GOTO: click to fly there (GUIDED)".to_string(), MAGENTA)),
        Tool::Plan => Some((format!("PLAN: click adds waypoint at {:.0} m, right-click removes last", st.plan_alt), AMBER)),
        Tool::Look => Some(("LOOK: click a point to keep the camera on it".to_string(), SPI)),
    };
    if let Some((s, col)) = hint {
        let tw = s.len() as f32 * fs * 0.62 + 16.0;
        fill_rect(window, x + w * 0.5 - tw * 0.5, y + 8.0, tw, 22.0, ca(0x000000, 0.7));
        text(window, cx, &s, x + w * 0.5, y + 19.0, fs, c(col), Align::Center);
    }
}
