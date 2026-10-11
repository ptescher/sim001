//! Sensor window: the aircraft camera's video with symbology, and gimbal control.
//!
//! Looking straight ahead it shows the flight HUD, conformal with the picture. Slewed off the
//! boresight it shows sensor symbology: reticle, azimuth/elevation scales, line-of-sight
//! bearing and the sensor point of interest (SPI) on the sim's terrain.
//!
//! Mouse: drag to slew, scroll to zoom, click to track that ground point.

use super::draw::*;
use super::hud::{self, HudData, HudOpts};
use super::theme::*;
use crate::feed::{Sensor, Spi};
use crate::geo::{Origin, sensor_ray, wrap360};
use crate::link::Shared;
use gpui::{
    App, Bounds, ContentMask, Context, Corners, Div, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Render, RenderImage, ScrollDelta, ScrollWheelEvent, SharedString, Stateful, Window, canvas, div, prelude::*, px,
};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use t1s_station::feed::{Gimbal, HFOV_DEFAULT};

pub struct SensorView {
    sensor: Arc<Sensor>,
    vehicle: Shared,
    origin: Origin,
    shown: Option<Arc<RenderImage>>,
    video_rect: Rc<Cell<Bounds<Pixels>>>,
    drag: Option<((f32, f32), Gimbal, bool)>,
}

impl SensorView {
    pub fn new(sensor: Arc<Sensor>, vehicle: Shared, origin: Origin, cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self { sensor, vehicle, origin, shown: None, video_rect: Rc::new(Cell::new(Bounds::default())), drag: None }
    }

    fn commanded(&self) -> Gimbal {
        *self.sensor.commanded.lock().unwrap()
    }

    fn mouse_down(&mut self, ev: &MouseDownEvent) {
        let p = (f32::from(ev.position.x), f32::from(ev.position.y));
        self.drag = Some((p, self.commanded(), false));
    }

    fn mouse_move(&mut self, ev: &MouseMoveEvent) {
        let Some((start, g0, moved)) = self.drag else { return };
        if !ev.dragging() {
            self.drag = None;
            return;
        }
        let (_, _, vw, vh) = rect(self.video_rect.get());
        let (dx, dy) = (f32::from(ev.position.x) - start.0, f32::from(ev.position.y) - start.1);
        if !moved && dx.abs() + dy.abs() < 4.0 {
            return;
        }
        self.drag = Some((start, g0, true));
        // Grab-and-drag: the picture follows the mouse.
        let vfov = vfov(g0.hfov, vh / vw.max(1.0));
        self.sensor.slew(Gimbal { pan: g0.pan - dx / vw.max(1.0) * g0.hfov, tilt: g0.tilt + dy / vh.max(1.0) * vfov, hfov: g0.hfov });
    }

    fn mouse_up(&mut self, ev: &MouseUpEvent) {
        let Some((_, _, moved)) = self.drag.take() else { return };
        if moved {
            return;
        }
        // A click: track the ground point under the cursor.
        let (x, y, w, h) = rect(self.video_rect.get());
        let (u, v) = ((f32::from(ev.position.x) - x) / w * 2.0 - 1.0, (f32::from(ev.position.y) - y) / h * 2.0 - 1.0);
        let Some(g) = self.sensor.gimbal() else { return };
        let veh = self.vehicle.lock().unwrap();
        if let Some(spi) = self.sensor.ground_at(&veh, g, h / w, u, v) {
            drop(veh);
            self.sensor.track_point(spi.north, spi.east);
        }
    }

    fn scroll(&mut self, ev: &ScrollWheelEvent) {
        let dy = match ev.delta {
            ScrollDelta::Pixels(p) => f32::from(p.y),
            ScrollDelta::Lines(l) => l.y * 20.0,
        };
        let mut g = self.commanded();
        g.hfov *= (dy * 0.004).exp();
        self.sensor.command(g);
    }

    fn toolbar(&self, status: String, cx: &mut Context<Self>) -> Div {
        let g = self.commanded();
        let tracking = self.sensor.track.lock().unwrap().is_some();
        let fov_btn = |id: &'static str, label: &'static str, hfov: f32| {
            sbutton(id, label, (g.hfov - hfov).abs() < 1.0).on_click(cx.listener(move |this, _, _, _| {
                let mut g = this.commanded();
                g.hfov = hfov;
                this.sensor.command(g);
            }))
        };
        let point_btn = |id: &'static str, label: &'static str, pan: f32, tilt: f32| {
            let active = !tracking && (g.pan - pan).abs() < 0.5 && (g.tilt - tilt).abs() < 0.5;
            sbutton(id, label, active).on_click(cx.listener(move |this, _, _, _| {
                let hfov = this.commanded().hfov;
                this.sensor.slew(Gimbal { pan, tilt, hfov });
            }))
        };
        div()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .h(px(30.0))
            .bg(c(TITLE_BG))
            .child(div().text_color(c(GREY)).mr_2().child("SENSOR  EO").on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move()))
            .child(point_btn("fwd", "FWD", 0.0, 0.0))
            .child(point_btn("down", "DOWN 30", 0.0, -30.0))
            .child(point_btn("left", "LEFT 60", -60.0, -25.0))
            .child(point_btn("right", "RIGHT 60", 60.0, -25.0))
            .child(point_btn("nadir", "NADIR", 0.0, -90.0))
            .child(div().w(px(10.0)))
            .child(fov_btn("wfov", "WFOV", HFOV_DEFAULT))
            .child(fov_btn("mfov", "MFOV", 30.0))
            .child(fov_btn("nfov", "NFOV", 10.0))
            .child(div().w(px(10.0)))
            .child(sbutton("track", "TRACK SPI", tracking).on_click(cx.listener(|this, _, _, _| {
                let spi = {
                    let v = this.vehicle.lock().unwrap();
                    this.sensor.spi(&v)
                };
                if let Some(s) = spi {
                    this.sensor.track_point(s.north, s.east);
                }
            })))
            .child(sbutton("untrack", "STOP", false).on_click(cx.listener(|this, _, _, _| {
                *this.sensor.track.lock().unwrap() = None;
            })))
            // Empty space and the status read-out double as the window's drag handle.
            .child(div().flex_1().h_full().on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move()))
            .child(div().text_color(c(GREEN)).whitespace_nowrap().child(status).on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move()))
    }
}

impl Render for SensorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Swap in the newest frame, freeing the previous one from the GPU atlas.
        let (frame, stale) = {
            let f = self.sensor.frame.lock().unwrap();
            (f.as_ref().map(|f| (f.header, f.image.clone())), f.as_ref().is_none_or(|f| f.at.elapsed().as_secs_f32() > 1.5))
        };
        if let Some((_, img)) = &frame {
            if self.shown.as_ref().is_none_or(|s| !Arc::ptr_eq(s, img)) {
                if let Some(old) = self.shown.replace(img.clone()) {
                    let _ = window.drop_image(old);
                }
            }
        }
        let overlay = {
            let v = self.vehicle.lock().unwrap();
            let spi = self.sensor.spi(&v);
            let los_brg = frame.as_ref().map(|(h, _)| {
                let d = sensor_ray(v.roll, v.pitch, v.yaw, h.gimbal.pan, h.gimbal.tilt);
                wrap360(d[1].atan2(d[0]).to_degrees())
            });
            Overlay {
                hud: HudData::from(&v, self.sensor.agl(&v)),
                spi,
                spi_ll: spi.map(|s| self.origin.to_ll(s.north, s.east)),
                los_brg,
                tracking: self.sensor.track.lock().unwrap().is_some(),
                fps: *self.sensor.fps.lock().unwrap(),
                status: self.sensor.status.lock().unwrap().clone(),
                stale,
            }
        };
        let status = match &frame {
            Some((h, _)) if !overlay.stale => {
                let g = h.gimbal;
                let name = if g.hfov > 50.0 { "WFOV" } else if g.hfov > 18.0 { "MFOV" } else { "NFOV" };
                format!("{name} {:.1}°  {:.0} FPS  F{:06}  T {:.1}", g.hfov, overlay.fps, h.frame % 1_000_000, h.sim_time)
            }
            _ => self.sensor.status.lock().unwrap().clone(),
        };
        let rect_cell = self.video_rect.clone();
        let video = canvas(|_, _, _| {}, move |b, _, window, cx| paint_video(b, frame, &overlay, &rect_cell, window, cx)).size_full();
        let toolbar = self.toolbar(status, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(c(0x000000))
            .font_family(FONT)
            .text_size(px(12.0))
            .text_color(c(WHITE))
            .child(toolbar)
            .child(
                div()
                    .id("video")
                    .flex_1()
                    .min_h(px(0.0))
                    .child(video)
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev, _, _| this.mouse_down(ev)))
                    .on_mouse_move(cx.listener(|this, ev, _, _| this.mouse_move(ev)))
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, ev, _, _| this.mouse_up(ev)))
                    .on_scroll_wheel(cx.listener(|this, ev, _, _| this.scroll(ev))),
            )
    }
}

struct Overlay {
    hud: HudData,
    spi: Option<Spi>,
    spi_ll: Option<(f64, f64)>,
    los_brg: Option<f32>,
    tracking: bool,
    fps: f32,
    status: String,
    stale: bool,
}

/// Vertical FOV for a horizontal one at an aspect ratio (height / width).
fn vfov(hfov: f32, aspect: f32) -> f32 {
    (2.0 * ((hfov.to_radians() * 0.5).tan() * aspect).atan()).to_degrees()
}

fn paint_video(
    bounds: Bounds<Pixels>,
    frame: Option<(t1s_station::feed::FrameHeader, Arc<RenderImage>)>,
    o: &Overlay,
    rect_cell: &Rc<Cell<Bounds<Pixels>>>,
    window: &mut Window,
    cx: &mut App,
) {
    let (x, y, w, h) = rect(bounds);
    fill_rect(window, x, y, w, h, c(0x000000));
    let aspect = frame.as_ref().map_or(9.0 / 16.0, |(hd, _)| hd.height as f32 / hd.width.max(1) as f32);
    // Letterbox the picture.
    let (vw, vh) = if h / w > aspect { (w, w * aspect) } else { (h / aspect, h) };
    let vb = Bounds { origin: pt(x + (w - vw) * 0.5, y + (h - vh) * 0.5), size: gpui::size(px(vw), px(vh)) };
    rect_cell.set(vb);
    let (vx, vy, _, _) = rect(vb);
    let fs = (vh / 50.0).clamp(10.0, 15.0);

    let Some((hd, img)) = frame else {
        boxed_text(window, cx, "NO VIDEO", vx + vw * 0.5, vy + vh * 0.45, fs * 1.6, c(AMBER), Align::Center, 0.0);
        text(window, cx, &o.status, vx + vw * 0.5, vy + vh * 0.45 + fs * 3.0, fs, c(GREY), Align::Center);
        return;
    };
    let _ = window.paint_image(vb, Corners::default(), img, 0, false);

    window.with_content_mask(Some(ContentMask { bounds: vb }), |window| {
        let g = hd.gimbal;
        let vf = vfov(g.hfov, aspect);
        if g.is_boresight() && !o.tracking {
            hud::paint(vb, &o.hud, HudOpts { video_vfov: Some(vf) }, window, cx);
        } else {
            sensor_symbology(window, cx, (vx, vy, vw, vh), fs, g, o);
        }

        if o.stale {
            boxed_text(window, cx, "VIDEO LOST", vx + vw * 0.5, vy + vh * 0.3, fs * 1.6, c(RED), Align::Center, 0.0);
        }
    });
}

fn sensor_symbology(window: &mut Window, cx: &mut App, r: (f32, f32, f32, f32), fs: f32, g: Gimbal, o: &Overlay) {
    let (vx, vy, vw, vh) = r;
    let (cx0, cy0) = (vx + vw * 0.5, vy + vh * 0.5);
    let col = c(GREEN);
    let sh = ca(0x000000, 0.6);

    // Reticle: broken crosshair, with a track gate when locked on a ground point.
    let (gap, len) = (vh * 0.025, vh * 0.09);
    for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        let a = (cx0 + dx * gap, cy0 + dy * gap);
        let b = (cx0 + dx * (gap + len), cy0 + dy * (gap + len));
        line(window, (a.0 + 1.0, a.1 + 1.0), (b.0 + 1.0, b.1 + 1.0), 3.0, sh);
        line(window, a, b, 1.6, col);
    }
    if o.tracking {
        let s = vh * 0.06;
        stroke_rect(window, cx0 - s, cy0 - s, 2.0 * s, 2.0 * s, 1.8, c(AMBER));
    }
    // Corner brackets.
    let (bx, by, bl) = (vw * 0.18, vh * 0.18, vh * 0.04);
    for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let (px0, py0) = (cx0 + sx * bx, cy0 + sy * by);
        polyline(window, &[(px0 - sx * bl, py0), (px0, py0), (px0, py0 - sy * bl)], 1.5, col);
    }

    // Azimuth scale (bottom) and elevation scale (right) for the gimbal.
    let (ax0, ax1, ay) = (cx0 - vw * 0.25, cx0 + vw * 0.25, vy + vh - 46.0);
    line(window, (ax0, ay), (ax1, ay), 1.5, col);
    for a in (-180..=180).step_by(30) {
        let px0 = ax0 + (a as f32 + 180.0) / 360.0 * (ax1 - ax0);
        line(window, (px0, ay), (px0, ay - if a % 90 == 0 { 9.0 } else { 5.0 }), 1.5, col);
    }
    let pan_x = ax0 + (g.pan + 180.0) / 360.0 * (ax1 - ax0);
    fill_poly(window, &[(pan_x, ay + 2.0), (pan_x - 6.0, ay + 12.0), (pan_x + 6.0, ay + 12.0)], col);
    text(window, cx, &format!("AZ {:+04.0}", g.pan), ax1 + 12.0, ay, fs, col, Align::Left);

    let (ex, ey0, ey1) = (vx + vw - 40.0, cy0 - vh * 0.25, cy0 + vh * 0.25);
    line(window, (ex, ey0), (ex, ey1), 1.5, col);
    let el_y = |e: f32| ey0 + (20.0 - e) / 110.0 * (ey1 - ey0);
    for e in (-90..=20).step_by(10) {
        line(window, (ex, el_y(e as f32)), (ex - if e % 30 == 0 { 9.0 } else { 5.0 }, el_y(e as f32)), 1.5, col);
    }
    let ty = el_y(g.tilt);
    fill_poly(window, &[(ex + 2.0, ty), (ex + 12.0, ty - 6.0), (ex + 12.0, ty + 6.0)], col);
    text(window, cx, &format!("EL {:+03.0}", g.tilt), ex, ey1 + 16.0, fs, col, Align::Center);

    // Line-of-sight bearing with a north pointer.
    if let Some(brg) = o.los_brg {
        let (nx, ny) = (cx0, vy + 34.0);
        circle(window, nx, ny, 14.0, 1.2, col);
        let a = (-brg).to_radians();
        let tip = (nx + 12.0 * a.sin(), ny - 12.0 * a.cos());
        line(window, (nx, ny), tip, 2.0, col);
        text(window, cx, "N", tip.0 + 6.0 * a.sin(), tip.1 - 6.0 * a.cos(), fs * 0.8, col, Align::Center);
        text(window, cx, &format!("LOS {:03.0}°", brg), nx + 24.0, ny, fs, col, Align::Left);
    }

    // Aircraft (top left) and SPI (top right).
    let d = &o.hud;
    let lh = fs * 1.4;
    let left = [
        format!("{}  {}", d.mode, if d.armed { "ARMED" } else { "SAFE" }),
        format!("ALT {:>5.0} m  AGL {}", d.alt, d.agl.map_or("--".into(), |a| format!("{a:.0}"))),
        format!("HDG {:03.0}  IAS {:>4.1}", d.heading, d.airspeed),
    ];
    for (i, s) in left.iter().enumerate() {
        text(window, cx, s, vx + 12.0, vy + 18.0 + i as f32 * lh, fs, col, Align::Left);
    }
    if let (Some(s), Some((lat, lon))) = (o.spi, o.spi_ll) {
        let right = [
            format!("SPI {}", if o.tracking { "TRACK" } else { "" }),
            format!("{lat:.6} {lon:.6}"),
            format!("ELEV {:+.0} m  SLR {:.0} m", s.height, s.range),
        ];
        for (i, t) in right.iter().enumerate() {
            text(window, cx, t, vx + vw - 12.0, vy + 18.0 + i as f32 * lh, fs, if i == 0 && o.tracking { c(AMBER) } else { col }, Align::Right);
        }
    } else {
        text(window, cx, "SPI --  (above horizon)", vx + vw - 12.0, vy + 18.0, fs, col, Align::Right);
    }
}

fn sbutton(id: &'static str, label: &'static str, active: bool) -> Stateful<Div> {
    let (bg, fg, border) = if active { (ACTIVE_FILL, GREEN, GREEN_DIM) } else { (CONTROL, WHITE, BEZEL_EDGE) };
    div()
        .id(SharedString::from(id))
        .px_2()
        .py(px(2.0))
        .bg(c(bg))
        .text_color(c(fg))
        .border_1()
        .border_color(c(border))
        .rounded(px(CONTROL_RADIUS))
        .cursor_pointer()
        .hover(|s| s.bg(c(CONTROL_HOVER)))
        .active(|s| s.opacity(0.7))
        .child(label)
}
