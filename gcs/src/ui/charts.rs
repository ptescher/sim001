//! Telemetry strip charts: altitude, speeds and power over the last couple of minutes.

use super::draw::*;
use super::theme::*;
use crate::vehicle::Sample;
use gpui::{App, Bounds, ContentMask, Pixels, Window};

pub const WINDOW_S: f32 = 120.0;

struct Series<'a> {
    label: &'a str,
    color: u32,
    get: fn(&Sample) -> f32,
}

pub fn paint(bounds: Bounds<Pixels>, samples: &[Sample], now: f32, window: &mut Window, cx: &mut App) {
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        let (x, y, w, h) = rect(bounds);
        let lanes: [(&str, &str, Vec<Series>); 3] = [
            ("ALT", "m", vec![Series { label: "REL", color: GREEN, get: |s| s.alt }]),
            (
                "SPD",
                "m/s",
                vec![
                    Series { label: "IAS", color: GREEN, get: |s| s.airspeed },
                    Series { label: "GS", color: CYAN, get: |s| s.groundspeed },
                ],
            ),
            (
                "PWR",
                "V / %",
                vec![
                    Series { label: "V", color: AMBER, get: |s| s.voltage },
                    Series { label: "THR/10", color: MAGENTA, get: |s| s.throttle / 10.0 },
                ],
            ),
        ];
        let lane_w = (w - 16.0) / lanes.len() as f32;
        for (i, (name, unit, series)) in lanes.iter().enumerate() {
            let lx = x + 8.0 + i as f32 * lane_w;
            lane(window, cx, (lx, y + 6.0, lane_w - 10.0, h - 12.0), name, unit, series, samples, now);
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn lane(window: &mut Window, cx: &mut App, r: (f32, f32, f32, f32), name: &str, unit: &str, series: &[Series], samples: &[Sample], now: f32) {
    let (x, y, w, h) = r;
    let fs = 10.5;
    let plot = (x + 38.0, y + 16.0, w - 42.0, h - 30.0);
    let t0 = now - WINDOW_S;
    let visible: Vec<&Sample> = samples.iter().filter(|s| s.t >= t0).collect();

    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for s in &visible {
        for se in series {
            let v = (se.get)(s);
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if lo > hi {
        (lo, hi) = (0.0, 1.0);
    }
    let pad = ((hi - lo) * 0.15).max(1.0);
    let (lo, hi) = (lo - pad, hi + pad);

    stroke_rect(window, plot.0, plot.1, plot.2, plot.3, 1.0, c(DARK));
    for k in 1..4 {
        let gy = plot.1 + plot.3 * k as f32 / 4.0;
        line(window, (plot.0, gy), (plot.0 + plot.2, gy), 1.0, ca(DARK, 0.6));
    }
    for k in 1..4 {
        let gx = plot.0 + plot.2 * k as f32 / 4.0;
        line(window, (gx, plot.1), (gx, plot.1 + plot.3), 1.0, ca(DARK, 0.6));
    }
    text(window, cx, &format!("{hi:.0}"), plot.0 - 4.0, plot.1 + 4.0, fs * 0.9, c(GREY), Align::Right);
    text(window, cx, &format!("{lo:.0}"), plot.0 - 4.0, plot.1 + plot.3 - 4.0, fs * 0.9, c(GREY), Align::Right);
    text(window, cx, &format!("{name} {unit}"), x, y + 4.0, fs, c(WHITE), Align::Left);
    text(window, cx, "-2 min", plot.0, plot.1 + plot.3 + 9.0, fs * 0.85, c(GREY), Align::Left);
    text(window, cx, "now", plot.0 + plot.2, plot.1 + plot.3 + 9.0, fs * 0.85, c(GREY), Align::Right);

    let mut lx = x + w;
    for se in series.iter().rev() {
        let cur = visible.last().map(|s| (se.get)(s));
        let label = match cur {
            Some(v) => format!("{} {:.1}", se.label, v),
            None => se.label.to_string(),
        };
        let tw = text(window, cx, &label, lx, y + 4.0, fs, c(se.color), Align::Right);
        lx -= tw + 12.0;
        let pts: Vec<_> = visible
            .iter()
            .map(|s| {
                let px = plot.0 + (s.t - t0) / WINDOW_S * plot.2;
                let py = plot.1 + plot.3 - ((se.get)(s) - lo) / (hi - lo) * plot.3;
                (px, py)
            })
            .collect();
        polyline(window, &pts, 1.6, c(se.color));
    }
}
