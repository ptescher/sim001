//! The primitives' guidance: the lateral acceleration, the height and the airspeed that each one
//! asks for. The envelope (in `lib.rs`) comes after, whatever these ask.
//!
//! A strategy of the league (domains/cognition/league) may change this file.

use crate::math::*;
// Pursue sits beyond the separation zone: it reads the envelope's numbers, and may not change them.
use crate::safety::{SEP_BUFFER, SEP_BUFFER_MIN};
use crate::{Ctx, Lims, Other, rotate, wind};

/// Roll for a lateral acceleration (m/s², positive to the right), within the bank limit.
pub(crate) fn roll_for(a: f32, l: &Lims) -> f32 {
    atan(a / G).within(-l.bank, l.bank)
}

/// L1 guidance to a point: the lateral acceleration (positive right).
pub(crate) fn l1_to(c: &Ctx, l: &Lims, target: [f32; 2]) -> f32 {
    let d = sub2(target, c.pos);
    let dist = norm2(d);
    if dist < 0.1 {
        return 0.0;
    }
    let eta = wrap_pi(atan2(d[1], d[0]) - c.course);
    let l1 = (l.t.l1_period * l.t.l1_damping * c.v / PI).min(dist).max(l.t.l1_min);
    let k = 2.0 * c.v * c.v / l1;
    if eta.abs() > HALF_PI { k.copysign(eta) } else { k * sin(eta) }
}

/// Course hold: the lateral acceleration towards course `chi`.
pub(crate) fn course_to(c: &Ctx, l: &Lims, chi: f32) -> f32 {
    l.t.k_course * c.v * wrap_pi(chi - c.course)
}

/// A vector field round a circle: the lateral acceleration to join it and stay on it.
pub(crate) fn circle(c: &Ctx, l: &Lims, centre: [f32; 2], radius: f32, dir: f32) -> f32 {
    let radius = radius.max(1.0);
    let r = sub2(c.pos, centre);
    let d = norm2(r);
    // At the centre, the bearing from it is taken as if the plane were already going round.
    let phi = if d > 1.0 { atan2(r[1], r[0]) } else { c.course - dir * HALF_PI };
    let chi = phi + dir * (HALF_PI + atan(l.t.k_circle * (d - radius) / radius));
    let near = (1.0 - (d - radius).abs() / radius).within(0.0, 1.0);
    dir * c.v * c.v / radius * near + course_to(c, l, chi)
}

/// Which way round a circle the plane is already going (+1 clockwise), from where it is and its velocity.
pub(crate) fn natural_dir(c: &Ctx, centre: [f32; 2]) -> f32 {
    if cross2(sub2(c.pos, centre), c.vel) >= 0.0 { 1.0 } else { -1.0 }
}

/// Band: the radius wandered round its point.
pub(crate) fn band_radius(l: &Lims) -> f32 {
    let r = l.loiter_radius();
    if l.fenced() { r.max(l.t.band_wander * l.fence_radius) } else { r }
}

/// Pursue: lead pursuit to where the target will be when the plane gets there (along the arc it
/// is turning on), from behind it (beyond the separation zone, when
/// separation is on), at its airspeed plus a closing speed; at its height.
pub(crate) fn pursue(c: &Ctx, t: &Other, l: &Lims) -> (f32, f32, f32) {
    let turn = t.turn;
    let rel = sub2(t.pos, c.pos);
    let range = norm2(rel).max(0.1);
    let lead = (range / c.v.max(5.0)).within(l.t.lead_min, l.t.lead_max);
    let standoff = if l.separation > 0.0 {
        l.separation + (SEP_BUFFER * l.separation).max(SEP_BUFFER_MIN) + l.t.standoff_extra
    } else {
        0.0
    };
    let vt = norm2(t.vel);
    let dir = if vt > 1.0 { [t.vel[0] / vt, t.vel[1] / vt] } else { [rel[0] / range, rel[1] / range] };
    // The chord of the arc: the velocity turned half way, for the lead time.
    let chord = rotate(t.vel, 0.5 * turn * lead);
    let back = rotate(dir, turn * lead);
    let aim = [t.pos[0] + chord[0] * lead - back[0] * standoff, t.pos[1] + chord[1] * lead - back[1] * standoff];
    let accel = l1_to(c, l, aim);

    // How far behind the target the plane is, along the target's track, past the standoff.
    let behind = dot2(rel, dir) - standoff;
    let t_air = norm2(sub2(t.vel, wind(c)));
    let speed = t_air + (l.t.k_close * behind).within(-l.t.open_max, l.t.close_max);
    (accel, t.h, speed)
}

/// Evade: away from the threat, bent back into the arena near its edge; well above or below it;
/// fast, but less so near the edge, where a wide turn would take it out.
pub(crate) fn evade(c: &Ctx, t: &Other, l: &Lims) -> (f32, f32, f32) {
    let away = sub2(c.pos, t.pos);
    let n = norm2(away);
    let mut dir = if n > 0.5 { [away[0] / n, away[1] / n] } else { [cos(c.course), sin(c.course)] };
    // 0 well inside the arena, rising to 1 at its edge.
    let mut edge = 0.0;
    if l.fenced() {
        let to_c = sub2(l.fence_centre, c.pos);
        let d = norm2(to_c);
        edge = ((d / l.fence_radius - l.t.evade_edge) / (1.0 - l.t.evade_edge)).within(0.0, 1.0);
        if d > 0.5 {
            dir = [dir[0] + 2.0 * to_c[0] / d * edge, dir[1] + 2.0 * to_c[1] / d * edge];
        }
    }
    let accel = course_to(c, l, atan2(dir[1], dir[0]));
    let height = if t.h >= c.h { t.h - l.t.evade_height } else { t.h + l.t.evade_height };
    // If the floor or the ceiling stops that, go the other way.
    let safe = l.safe_height(c, height);
    let height = if (safe - t.h).abs() < 0.5 * l.t.evade_height {
        if height < t.h { t.h + l.t.evade_height } else { t.h - l.t.evade_height }
    } else {
        height
    };
    (accel, height, lerp(l.v_cruise, l.v_max, l.t.evade_speed * (1.0 - 0.5 * edge)))
}
