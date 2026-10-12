//! Simulation core shared between the render thread and the ArduPilot bridge thread.

use simulation_aircraft::AircraftParams;
use simulation_physics::{self as physics, ActuatorCommands, Environment, FlightState, Obstacle, Phase};
use crate::terrain;
use bevy::math::Vec3;
use std::f32::consts::PI;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlightMode {
    /// Direct stick-to-surface, like the stock PNP plane on a plain receiver.
    Manual,
    /// Self-levelling: sticks command bank and pitch angles.
    Angle,
    /// Rate mode: sticks command roll/pitch rates, attitude is held when centred.
    Acro,
}

impl FlightMode {
    pub fn next(self) -> Self {
        match self {
            FlightMode::Manual => FlightMode::Angle,
            FlightMode::Angle => FlightMode::Acro,
            FlightMode::Acro => FlightMode::Manual,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            FlightMode::Manual => "MANUAL",
            FlightMode::Angle => "ANGLE",
            FlightMode::Acro => "ACRO",
        }
    }
}

/// ArduPlane modes on the FLTMODE channel (ch8), with the PWM that selects each slot.
pub const AP_MODES: [(&str, u16); 6] = [
    ("MANUAL", 1165),
    ("FBWA", 1295),
    ("CRUISE", 1425),
    ("AUTO", 1555),
    ("RTL", 1685),
    ("LOITER", 1815),
];

/// Pilot stick positions, normalized.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sticks {
    /// Right positive.
    pub roll: f32,
    /// Stick back (nose up) positive.
    pub pitch: f32,
    /// Right positive.
    pub yaw: f32,
    /// 0..1.
    pub throttle: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct WindSettings {
    pub speed: f32,
    /// Direction the wind blows FROM, radians clockwise from north.
    pub from_dir: f32,
    pub turbulence: f32,
}

#[derive(Default, Clone, Copy)]
struct PidState {
    roll_i: f32,
    pitch_i: f32,
}

pub struct ArduPilotLink {
    pub enabled: bool,
    pub connected: bool,
    pub last_packet: Option<Instant>,
    pub frame_rate: u16,
    pub frame_count: u32,
    pub packets: u64,
    pub pwm: [u16; 16],
    pub mode_index: usize,
    pub arm_switch: bool,
    pub port: u16,
    pub status: String,
    /// Armed state and mode as the ground station sees them over MAVLink (sent on the sensor
    /// feed link), with when they arrived. JSON SITL doesn't tell the sim either.
    pub gcs: Option<(Instant, bool, String)>,
}

impl ArduPilotLink {
    /// Armed and mode from the ground station, if it reported in the last 3 s.
    pub fn vehicle(&self) -> Option<(bool, &str)> {
        self.gcs.as_ref().filter(|g| g.0.elapsed().as_secs_f32() < 3.0).map(|g| (g.1, g.2.as_str()))
    }
}

pub struct SimCore {
    pub params: AircraftParams,
    pub state: FlightState,
    pub obstacles: Vec<Obstacle>,
    pub sticks: Sticks,
    pub mode: FlightMode,
    pub high_rates: bool,
    pub diff_thrust: bool,
    pub wind: WindSettings,
    pub paused: bool,
    pub last_cmd: ActuatorCommands,
    pub last_wind: Vec3,
    pub ap: ArduPilotLink,
    pid: PidState,
    launch_hold: f32,
    /// Times the aircraft has been reset (sent to the ground station with each video frame).
    pub resets: u16,
    /// With ArduPilot: fly with the sticks through the sim's own modes instead. Physics still
    /// runs in lockstep with SITL, so ArduPilot keeps estimating (and the ground station keeps
    /// its telemetry); its servo outputs are just ignored.
    pub direct: bool,
}

fn expo(x: f32, e: f32) -> f32 {
    e * x * x * x + (1.0 - e) * x
}

impl SimCore {
    pub fn new(params: AircraftParams, obstacles: Vec<Obstacle>, ap_enabled: bool, ap_port: u16) -> Self {
        let state = FlightState::new(&params, &terrain::hand());
        Self {
            params,
            state,
            obstacles,
            sticks: Sticks::default(),
            mode: FlightMode::Manual,
            high_rates: true,
            diff_thrust: true,
            wind: WindSettings { speed: 3.0, from_dir: 20.0f32.to_radians(), turbulence: 0.3 },
            paused: false,
            last_cmd: ActuatorCommands::default(),
            last_wind: Vec3::ZERO,
            ap: ArduPilotLink {
                enabled: ap_enabled,
                connected: false,
                last_packet: None,
                frame_rate: 0,
                frame_count: 0,
                packets: 0,
                pwm: [0; 16],
                mode_index: 1,
                arm_switch: false,
                port: ap_port,
                status: String::from("waiting for SITL"),
                gcs: None,
            },
            pid: PidState::default(),
            launch_hold: 0.0,
            resets: 0,
            direct: false,
        }
    }

    pub fn reset(&mut self) {
        self.state.reset_to_hand(&terrain::hand());
        // With a ground station attached it restarts ArduPilot on a reset (see the feed), so
        // snap straight back to the hand. Otherwise the plane is carried back, which keeps
        // ArduPilot's estimators consistent without a restart.
        if self.ap.enabled && self.ap.vehicle().is_some() {
            self.state.snap_to_hand();
        }
        self.resets = self.resets.wrapping_add(1);
        self.pid = PidState::default();
        self.launch_hold = 0.0;
    }

    /// Wind at a point: mean wind with a boundary-layer profile plus turbulence.
    pub fn wind_at(&self, pos: Vec3, t: f64) -> Vec3 {
        let w = &self.wind;
        let to_dir = -Vec3::new(w.from_dir.sin(), 0.0, -w.from_dir.cos());
        let agl = (pos.y - terrain::ground_height(pos.x, pos.z)).max(0.3);
        let profile = (agl / 10.0).powf(0.16).min(1.4);
        let mean = to_dir * w.speed * profile;
        let t = t as f32;
        let amp = w.turbulence * (0.6 + 0.25 * w.speed);
        let s = pos * 0.02;
        let turb = Vec3::new(
            (t * 0.71 + s.x * 1.3).sin() * 0.6 + (t * 1.93 + s.z * 2.1 + 1.0).sin() * 0.4,
            ((t * 1.17 + s.z * 1.7 + 2.0).sin() * 0.6 + (t * 2.71 + s.x * 2.3).sin() * 0.4) * 0.5,
            (t * 0.83 + s.z * 1.1 + 4.0).sin() * 0.6 + (t * 2.29 + s.x * 1.9 + 3.0).sin() * 0.4,
        ) * amp;
        mean + turb
    }

    /// Compute actuator commands from the sticks via the onboard (non-ArduPilot) flight modes.
    fn stick_commands(&mut self, dt: f32) -> ActuatorCommands {
        let s = self.sticks;
        let st = &self.state;
        let rate = if self.high_rates { 1.0 } else { 0.65 };
        let (roll, pitch, _) = physics::euler(st.rot);
        let (p, q, _r) = physics::body_rates(st.omega);
        let v = st.airspeed.max(6.0);
        // Gain scheduling: surface effectiveness grows with dynamic pressure.
        let gs = (13.0 / v).powi(2).clamp(0.25, 2.5);

        let (ail, ele) = match self.mode {
            FlightMode::Manual => (expo(s.roll, 0.3) * rate, expo(s.pitch, 0.3) * rate),
            FlightMode::Angle => {
                let tr = s.roll * 55f32.to_radians();
                // Climb bias at high throttle, so a hands-off launch climbs away.
                let tp = s.pitch * 35f32.to_radians() + (3.0 + 16.0 * (s.throttle - 0.5).max(0.0)).to_radians();
                let er = tr - roll;
                let ep = tp - pitch;
                self.pid.pitch_i = (self.pid.pitch_i + ep * dt * 0.8).clamp(-0.3, 0.3);
                let ail = (1.6 * er - 0.18 * p) * gs;
                // Extra elevator in a bank to hold altitude.
                let coord = 0.25 * (1.0 / roll.cos().abs().max(0.4) - 1.0);
                let ele = (1.8 * ep - 0.15 * q + self.pid.pitch_i) * gs + coord;
                (ail, ele)
            }
            FlightMode::Acro => {
                let tp_rate = expo(s.roll, 0.4) * 5.5 * rate;
                let tq_rate = expo(s.pitch, 0.4) * 3.0 * rate;
                self.pid.roll_i = (self.pid.roll_i + (tp_rate - p) * dt).clamp(-1.0, 1.0);
                self.pid.pitch_i = (self.pid.pitch_i + (tq_rate - q) * dt).clamp(-1.0, 1.0);
                let ail = (0.09 * tp_rate + 0.12 * (tp_rate - p) + 0.25 * self.pid.roll_i) * gs;
                let ele = (0.12 * tq_rate + 0.20 * (tq_rate - q) + 0.35 * self.pid.pitch_i) * gs;
                (ail, ele)
            }
        };
        let rud = expo(s.yaw, 0.2) * rate;
        let thr = s.throttle.clamp(0.0, 1.0);
        let dt_mix = if self.diff_thrust && thr > 0.05 { 0.1 * rud } else { 0.0 };
        let thr_l = (thr + dt_mix).clamp(0.0, 1.0);
        let thr_r = (thr - dt_mix).clamp(0.0, 1.0);
        // V-tail rudder mix at 50%, as typically set up on the real plane.
        ActuatorCommands::from_controls(ail.clamp(-1.0, 1.0), ele.clamp(-1.0, 1.0), 0.5 * rud, [thr_l, thr_r])
    }

    /// Hand-launch automatically once throttle is held up, like a real throw.
    fn auto_launch(&mut self, throttle: f32, dt: f32) {
        if self.state.phase == Phase::Held {
            if throttle > 0.55 {
                self.launch_hold += dt;
                if self.launch_hold > 0.6 {
                    self.state.launch();
                    self.launch_hold = 0.0;
                }
            } else {
                self.launch_hold = 0.0;
            }
        }
    }

    pub fn launch_progress(&self) -> f32 {
        (self.launch_hold / 0.6).clamp(0.0, 1.0)
    }

    /// Standalone step (built-in flight modes).
    pub fn step_standalone(&mut self, dt: f32) {
        let cmd = self.stick_commands(dt);
        self.auto_launch(self.sticks.throttle, dt);
        self.step_physics(&cmd, dt);
    }

    /// Lockstep step driven by ArduPilot servo outputs.
    pub fn step_ardupilot(&mut self, pwm: &[u16; 16], dt: f32) {
        if self.direct {
            self.step_standalone(dt);
            return;
        }
        let ang = |v: u16| if v == 0 { 0.0 } else { ((v as f32 - 1500.0) / 500.0).clamp(-1.0, 1.0) };
        let rng = |v: u16| if v == 0 { 0.0 } else { ((v as f32 - 1000.0) / 1000.0).clamp(0.0, 1.0) };
        // SERVO1 aileron (4), SERVO2 V-tail left (79), SERVO3 throttle left (73),
        // SERVO4 V-tail right (80), SERVO5 throttle right (74). ArduPilot's V-tail mixer
        // drives non-reversed servos "trailing edge up" for positive output.
        let cmd = ActuatorCommands {
            aileron: ang(pwm[0]),
            vtail_left: -ang(pwm[1]),
            vtail_right: -ang(pwm[3]),
            throttle: [rng(pwm[2]), rng(pwm[4])],
        };
        self.auto_launch(0.5 * (cmd.throttle[0] + cmd.throttle[1]), dt);
        self.step_physics(&cmd, dt);
    }

    fn step_physics(&mut self, cmd: &ActuatorCommands, dt: f32) {
        let wind = self.wind_at(self.state.pos, self.state.time);
        self.last_wind = wind;
        let env = Environment { wind };
        physics::step(&self.params, &mut self.state, cmd, &env, &terrain::WorldTerrain, &self.obstacles, dt);
        self.last_cmd = *cmd;
    }

    /// RC channel PWMs sent to ArduPilot (AETR + aux, ch8 = flight mode).
    pub fn rc_channels(&self) -> [u16; 8] {
        let s = self.sticks;
        let c = |x: f32| (1500.0 + 500.0 * x.clamp(-1.0, 1.0)).round() as u16;
        [
            c(s.roll),
            // ArduPlane: RC2 above trim = nose up.
            c(s.pitch),
            (1000.0 + 1000.0 * s.throttle.clamp(0.0, 1.0)).round() as u16,
            c(s.yaw),
            1500,
            1500,
            // RC7_OPTION = 153 (arm/disarm switch).
            if self.ap.arm_switch { 1900 } else { 1100 },
            AP_MODES[self.ap.mode_index].1,
        ]
    }

    /// Build the JSON sensor packet for ArduPilot's JSON SITL backend.
    pub fn sitl_json(&self) -> String {
        let s = &self.state;
        let home_ground = terrain::ground_height(terrain::pilot_pos().x, terrain::pilot_pos().z);
        // World (+X east, +Y up, -Z north) -> NED.
        let ned = |v: Vec3| [-v.z, v.x, -v.y];
        let rel = s.pos - Vec3::new(terrain::pilot_pos().x, home_ground, terrain::pilot_pos().z);
        let pos = ned(rel);
        let vel = ned(s.vel);
        let wind = ned(self.last_wind);
        // Body (Bevy: +X right, +Y up, -Z fwd) -> FRD.
        let frd = |v: Vec3| [-v.z, v.x, -v.y];
        let gyro = frd(s.omega);
        let acc = frd(s.specific_force);
        // Attitude: body FRD axes expressed in NED.
        let fwd = ned(s.rot * Vec3::NEG_Z);
        let right = ned(s.rot * Vec3::X);
        let down = ned(s.rot * Vec3::NEG_Y);
        let m = bevy::math::Mat3::from_cols(fwd.into(), right.into(), down.into());
        let q = bevy::math::Quat::from_mat3(&m).normalize();
        let rc = self.rc_channels();
        let mut rc_json = String::new();
        for (i, v) in rc.iter().enumerate() {
            if i > 0 {
                rc_json.push(',');
            }
            rc_json.push_str(&format!("\"rc_{}\":{}", i + 1, v));
        }
        format!(
            "\n{{\"timestamp\":{:.6},\"imu\":{{\"gyro\":[{:.6},{:.6},{:.6}],\"accel_body\":[{:.5},{:.5},{:.5}]}},\
\"position\":[{:.4},{:.4},{:.4}],\"quaternion\":[{:.7},{:.7},{:.7},{:.7}],\"velocity\":[{:.5},{:.5},{:.5}],\
\"airspeed\":{:.4},\"velocity_wind\":[{:.4},{:.4},{:.4}],\"battery\":{{\"voltage\":{:.3},\"current\":{:.3}}},\
\"rc\":{{{}}}}}\n",
            s.time,
            gyro[0], gyro[1], gyro[2],
            acc[0], acc[1], acc[2],
            pos[0], pos[1], pos[2],
            q.w, q.x, q.y, q.z,
            vel[0], vel[1], vel[2],
            s.airspeed,
            wind[0], wind[1], wind[2],
            s.voltage, s.current,
            rc_json
        )
    }
}

pub fn wrap_deg(rad: f32) -> f32 {
    let d = rad * 180.0 / PI;
    (d % 360.0 + 360.0) % 360.0
}
