//! The numbers that shape how the executor flies. Distances in m, speeds in m/s, angles in rad.
//!
//! The guidance and energy numbers are [`Tuning`]: each has its default (the
//! constant of the same name), its bounds, and may be set per executor
//! ([`Executor::with_tuning`](crate::Executor::with_tuning)). The safety envelope's numbers (the track and step checks, the
//! fence, the floor and ceiling, separation and the stall margin) are plain constants: nothing
//! that tunes the executor can trade them away.
//!
//! A strategy of the league (domains/cognition/league) may change this file: add a number, change
//! a default or its bounds. The envelope's numbers are in `crate::safety`, which it may not.

/// Declares the tunable numbers: for each, a constant (its default) and a field of [`Tuning`],
/// with its bounds (the least and the most a tuning may set).
macro_rules! tunable {
    ($($(#[$doc:meta])* $field:ident / $name:ident = $default:expr, $lo:expr, $hi:expr;)*) => {
        $($(#[$doc])* pub const $name: f32 = $default;)*

        /// The guidance and energy numbers of one executor (see the constants of the same
        /// names, in capitals, for what each does).
        #[repr(C)]
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Tuning {
            $(pub $field: f32,)*
        }

        impl Tuning {
            /// The constants: how the executor flies when nothing tunes it.
            pub const DEFAULT: Tuning = Tuning { $($field: $name,)* };
            /// How many numbers there are.
            pub const LEN: usize = [$(stringify!($field)),*].len();
            /// Their names, in the order of [`Tuning::to_array`].
            pub const NAMES: [&'static str; Tuning::LEN] = [$(stringify!($field)),*];
            /// Their bounds (least, most), in the same order.
            pub const BOUNDS: [(f32, f32); Tuning::LEN] = [$(($lo, $hi)),*];

            pub fn to_array(&self) -> [f32; Tuning::LEN] {
                [$(self.$field),*]
            }

            /// From numbers in the order of [`Tuning::NAMES`]. `None` unless each is finite
            /// and within its bounds.
            pub fn from_array(a: &[f32; Tuning::LEN]) -> Option<Tuning> {
                let ok = a.iter().zip(Tuning::BOUNDS.iter()).all(|(v, (lo, hi))| v.is_finite() && *v >= *lo && *v <= *hi);
                let mut it = a.iter().copied();
                let t = Tuning { $($field: it.next().unwrap_or($name),)* };
                ok.then_some(t)
            }
        }
    };
}

tunable! {
    /// L1 guidance: period (s) and damping, as ArduPilot's `NAVL1_PERIOD` / `NAVL1_DAMPING`. The
    /// L1 distance is `period * damping * v / π` (about 43 m at 18 m/s), or the distance to the
    /// point if that is shorter, but not under `L1_MIN`.
    l1_period / L1_PERIOD = 10.0, 4.0, 25.0;
    l1_damping / L1_DAMPING = 0.75, 0.6, 1.0;
    l1_min / L1_MIN = 8.0, 3.0, 20.0;
    /// Course-error gain (1/s): turn rate asked per radian of course error.
    k_course / K_COURSE = 0.9, 0.2, 2.5;
    /// Vector-field gain round a circle: how hard the course bends towards the circle.
    k_circle / K_CIRCLE = 2.0, 0.5, 5.0;
    /// Loiter radius: at least this, and at least `LOITER_TURN_MARGIN` turn radii at the cruise
    /// speed and `LOITER_BANK` of the bank limit.
    loiter_radius_min / LOITER_RADIUS_MIN = 25.0, 15.0, 60.0;
    loiter_bank / LOITER_BANK = 0.75, 0.4, 0.95;
    loiter_turn_margin / LOITER_TURN_MARGIN = 1.15, 1.0, 2.0;
    /// An orbit's radius is at least `ORBIT_TURN_MARGIN` turn radii at the cruise speed and
    /// `ORBIT_BANK` of the bank limit.
    orbit_bank / ORBIT_BANK = 0.85, 0.4, 0.95;
    orbit_turn_margin / ORBIT_TURN_MARGIN = 1.1, 1.0, 2.0;
    /// Goto: L1 to the point until within `GOTO_OVER` of it (or it is behind and within
    /// `GOTO_ARRIVED` loiter radii), then circle it at the loiter radius.
    goto_over / GOTO_OVER = 10.0, 3.0, 30.0;
    goto_arrived / GOTO_ARRIVED = 1.2, 1.0, 3.0;
    /// Band: wander round the point at this fraction of the fence radius (or the loiter radius).
    band_wander / BAND_WANDER = 0.45, 0.1, 0.7;

    /// Height: climb rate asked per metre of height error (1/s), and the most climb and sink.
    k_height / K_HEIGHT = 0.35, 0.1, 1.0;
    climb_max / CLIMB_MAX = 4.0, 1.0, 8.0;
    sink_max / SINK_MAX = 4.0, 1.0, 8.0;
    /// Pitch per m/s of climb-rate error, its integral, and the integrator's limit (rad).
    k_climb / K_CLIMB = 0.05, 0.0, 0.2;
    k_climb_i / K_CLIMB_I = 0.03, 0.0, 0.15;
    pitch_i_max / PITCH_I_MAX = 0.12, 0.0, 0.3;
    /// Throttle: trim, per m/s of airspeed error, per m/s of climb asked, the integral and its limit.
    thr_trim / THR_TRIM = 0.45, 0.2, 0.8;
    k_thr_speed / K_THR_SPEED = 0.1, 0.0, 0.4;
    k_thr_climb / K_THR_CLIMB = 0.06, 0.0, 0.2;
    k_thr_i / K_THR_I = 0.05, 0.0, 0.2;
    thr_i_max / THR_I_MAX = 0.4, 0.0, 0.6;
    /// Below the target airspeed by more than `UNDERSPEED`, lower the nose this much per m/s.
    underspeed / UNDERSPEED = 2.0, 0.5, 6.0;
    k_underspeed_pitch / K_UNDERSPEED_PITCH = 0.03, 0.0, 0.1;

    /// Pursue: airspeed added per metre the plane is behind the target (1/s), the most added and
    /// taken off, and the longest look-ahead to the intercept point (s; the look-ahead is the
    /// range over the plane's speed).
    k_close / K_CLOSE = 0.5, 0.05, 2.0;
    close_max / CLOSE_MAX = 5.0, 0.0, 12.0;
    open_max / OPEN_MAX = 4.0, 0.0, 10.0;
    lead_max / LEAD_MAX = 3.0, 0.5, 8.0;
    /// The shortest look-ahead (s): close behind, the plane follows the target's path to a point
    /// this far ahead of it, as L1 follows a track, instead of chasing a point it is on top of.
    lead_min / LEAD_MIN = 1.5, 0.0, 4.0;
    /// The target's turn rate is estimated from its velocity, filtered with this time constant
    /// (s), and believed up to `TURN_RATE_MAX` (rad/s).
    turn_filter / TURN_FILTER = 1.0, 0.1, 4.0;
    turn_rate_max / TURN_RATE_MAX = 1.0, 0.0, 1.5;
    /// With separation on, pursue sits this much beyond the separation zone, behind the target.
    standoff_extra / STANDOFF_EXTRA = 3.0, 0.0, 15.0;
    /// Evade: aim this far above or below the threat, at this fraction of the way from cruise to
    /// top speed; the arena edge starts to bend the escape inwards at `EVADE_EDGE` of the fence radius.
    evade_height / EVADE_HEIGHT = 20.0, 5.0, 40.0;
    evade_speed / EVADE_SPEED = 0.6, 0.0, 1.0;
    evade_edge / EVADE_EDGE = 0.5, 0.2, 0.9;
}

impl Default for Tuning {
    fn default() -> Tuning {
        Tuning::DEFAULT
    }
}

impl Tuning {
    /// A hash of the numbers (FNV-1a over their bits): what a report or a plane quotes to say
    /// which tuning flew.
    pub fn hash(&self) -> u32 {
        let mut h: u32 = 0x811c_9dc5;
        for v in self.to_array() {
            for b in v.to_bits().to_le_bytes() {
                h ^= b as u32;
                h = h.wrapping_mul(0x0100_0193);
            }
        }
        h
    }
}
