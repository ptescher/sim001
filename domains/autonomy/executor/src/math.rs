//! The little trigonometry the executor needs, for `core` alone (it has no `sqrt`, `sin`,
//! `atan2`...). Every build uses these, the on-board one and the training one alike, so both
//! compute the same numbers. Each is good to about 1e-5 (absolute, or relative for `sqrt`) and
//! returns something finite for any finite input.

pub const PI: f32 = core::f32::consts::PI;
pub const TAU: f32 = 2.0 * PI;
pub const HALF_PI: f32 = 0.5 * PI;
/// Standard gravity (m/s²).
pub const G: f32 = 9.80665;

/// Square root; 0 for zero, negative numbers and NaN.
pub fn sqrt(x: f32) -> f32 {
    if x.is_nan() || x <= 0.0 {
        return 0.0;
    }
    if x == f32::INFINITY {
        return x;
    }
    // A guess from halving the exponent, then Newton: three steps take it to full precision.
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1fbd_1df5);
    for _ in 0..3 {
        y = 0.5 * (y + x / y);
    }
    y
}

/// The angle in [-π, π]; 0 for a NaN, an infinity or an absurdly large angle.
pub fn wrap_pi(x: f32) -> f32 {
    if x.is_nan() || x.abs() >= 1.0e6 {
        return 0.0;
    }
    if (-PI..=PI).contains(&x) {
        return x;
    }
    let k = x / TAU;
    // Round to the nearest whole number of turns (`as` truncates towards zero).
    let k = (k + 0.5f32.copysign(k)) as i32 as f32;
    let y = x - k * TAU;
    y.within(-PI, PI)
}

/// Sine and cosine of `x` (radians).
pub fn sin_cos(x: f32) -> (f32, f32) {
    let x = wrap_pi(x);
    // Fold into [-π/2, π/2], where the polynomials are good: sin(π - x) = sin x, cos(π - x) = -cos x.
    let (y, c_sign) = if x > HALF_PI {
        (PI - x, -1.0)
    } else if x < -HALF_PI {
        (-PI - x, -1.0)
    } else {
        (x, 1.0)
    };
    let y2 = y * y;
    // Taylor series to the 11th / 12th power: error below 1e-7 on [-π/2, π/2].
    let s = y
        * (1.0
            + y2 * (-1.0 / 6.0
                + y2 * (1.0 / 120.0 + y2 * (-1.0 / 5040.0 + y2 * (1.0 / 362_880.0 + y2 * (-1.0 / 39_916_800.0))))));
    let c = 1.0
        + y2 * (-0.5
            + y2 * (1.0 / 24.0
                + y2 * (-1.0 / 720.0
                    + y2 * (1.0 / 40_320.0 + y2 * (-1.0 / 3_628_800.0 + y2 * (1.0 / 479_001_600.0))))));
    (s, c * c_sign)
}

pub fn sin(x: f32) -> f32 {
    sin_cos(x).0
}

pub fn cos(x: f32) -> f32 {
    sin_cos(x).1
}

/// Tangent, for |x| < π/2 (the bank angles it is used on); clipped to ±1e4 near ±π/2.
pub fn tan(x: f32) -> f32 {
    let (s, c) = sin_cos(x);
    if c.abs() < 1.0e-4 { (1.0e4f32).copysign(s) } else { s / c }
}

/// Arctangent of `z` for |z| <= 1 (a minimax polynomial, error about 1e-5).
fn atan_unit(z: f32) -> f32 {
    let z2 = z * z;
    z * (0.999_977_26
        + z2 * (-0.332_623_47 + z2 * (0.193_543_46 + z2 * (-0.116_432_87 + z2 * (0.052_653_32 + z2 * -0.011_721_2)))))
}

/// The angle of the vector (x, y) from the x axis, in [-π, π]; 0 for (0, 0) or anything not finite.
pub fn atan2(y: f32, x: f32) -> f32 {
    if !(y.is_finite() && x.is_finite()) || (x == 0.0 && y == 0.0) {
        return 0.0;
    }
    let (ax, ay) = (x.abs(), y.abs());
    let a = if ax >= ay { atan_unit(ay / ax) } else { HALF_PI - atan_unit(ax / ay) };
    let a = if x < 0.0 { PI - a } else { a };
    a.copysign(y)
}

pub fn atan(x: f32) -> f32 {
    atan2(x, 1.0)
}

/// `clamp` without its panic (on bounds the wrong way round or NaN): `x` kept between `lo` and
/// `hi` (`lo` if it is NaN), `hi` winning if they cross.
pub trait Within {
    fn within(self, lo: f32, hi: f32) -> f32;
}

impl Within for f32 {
    fn within(self, lo: f32, hi: f32) -> f32 {
        self.max(lo).min(hi)
    }
}

/// `x` if it is finite, else `or`.
pub fn finite_or(x: f32, or: f32) -> f32 {
    if x.is_finite() { x } else { or }
}

/// Linear blend: `a` at t = 0, `b` at t = 1.
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Length of the 2-vector (north, east).
pub fn norm2(v: [f32; 2]) -> f32 {
    sqrt(v[0] * v[0] + v[1] * v[1])
}

pub fn sub2(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

pub fn dot2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

/// The z of a × b, for north-east vectors: positive when b is clockwise of a (to its right, seen from above).
pub fn cross2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_roots_match_std() {
        let mut x = 1.0e-6f32;
        while x < 1.0e7 {
            let (a, b) = (sqrt(x), x.sqrt());
            assert!((a - b).abs() <= 1.0e-6 * b, "{x}: {a} vs {b}");
            x *= 1.37;
        }
        assert_eq!(sqrt(0.0), 0.0);
        assert_eq!(sqrt(-4.0), 0.0);
        assert_eq!(sqrt(f32::NAN), 0.0);
        assert_eq!(sqrt(f32::INFINITY), f32::INFINITY);
    }

    #[test]
    fn sines_cosines_and_tangents_match_std() {
        let mut x = -50.0f32;
        while x < 50.0 {
            let (s, c) = sin_cos(x);
            assert!((s - x.sin()).abs() < 2.0e-5, "sin {x}: {s} vs {}", x.sin());
            assert!((c - x.cos()).abs() < 2.0e-5, "cos {x}: {c} vs {}", x.cos());
            x += 0.0137;
        }
        let mut x = -1.5f32;
        while x < 1.5 {
            assert!((tan(x) - x.tan()).abs() < 1.0e-4 * (1.0 + x.tan().abs()), "tan {x}");
            x += 0.01;
        }
        assert_eq!(sin_cos(f32::NAN), (0.0, 1.0));
    }

    #[test]
    fn arctangents_match_std() {
        for i in 0..2000 {
            let a = i as f32 * 0.00314159 * 2.0 - PI;
            for r in [1.0e-3f32, 1.0, 37.0, 1.0e5] {
                let (y, x) = (r * a.sin(), r * a.cos());
                let (mine, std) = (atan2(y, x), y.atan2(x));
                let d = wrap_pi(mine - std).abs();
                assert!(d < 2.0e-5, "atan2({y}, {x}): {mine} vs {std}");
            }
        }
        assert_eq!(atan2(0.0, 0.0), 0.0);
        assert_eq!(atan2(f32::NAN, 1.0), 0.0);
        assert!((atan(1.0) - PI / 4.0).abs() < 1.0e-5);
    }

    #[test]
    fn angles_wrap_into_a_half_turn_either_way() {
        for x in [-1000.0f32, -7.0, -PI, -1.0, 0.0, 2.0, PI, 3.5, 6.3, 99.9, 12345.6] {
            let w = wrap_pi(x);
            assert!((-PI..=PI).contains(&w), "{x} -> {w}");
            assert!((w.sin() - x.sin()).abs() < 1.0e-3 && (w.cos() - x.cos()).abs() < 1.0e-3, "{x} -> {w}");
        }
        assert_eq!(wrap_pi(f32::NAN), 0.0);
        assert_eq!(wrap_pi(f32::INFINITY), 0.0);
    }
}
