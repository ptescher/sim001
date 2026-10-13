//! Simulation core shared between the render thread and the ArduPilot bridge thread.

use simulation_aircraft::{ActuatorCommands, AircraftParams, t1s_ranger};
use simulation_physics::{self as physics, Environment, FlightState, Obstacle, Phase};
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
    /// The stand-in radio's last state, from the ground station (`--test-radio`).
    pub test_radio: Option<t1s_station::feed::RadioState>,
    /// The pilot flies with a radio (not the keyboard or a gamepad): its throttle stick is where
    /// the pilot holds it.
    pub pilot_radio: bool,
    /// The radio's arm switch, if the pilot gave it one: up is armed. It decides the arm signal
    /// whatever the keyboard's toggle says.
    pub radio_arm: Option<bool>,
    /// The radio's mode channel in µs, if the pilot gave it one: it goes to ArduPilot as it is.
    pub radio_mode: Option<u16>,
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
        let state = FlightState::new(&params, &terrain::start());
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
            test_radio: None,
            pilot_radio: false,
            radio_arm: None,
            radio_mode: None,
            last_wind: Vec3::ZERO,
            ap: ArduPilotLink {
                enabled: ap_enabled,
                connected: false,
                last_packet: None,
                frame_rate: 0,
                frame_count: 0,
                packets: 0,
                pwm: [0; 16],
                // The mode switch starts in its first position (for the VTOL, QHOVER).
                mode_index: 0,
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
        self.state.reset_to_hand(&self.params, &terrain::start());
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
        t1s_ranger::mix(ail.clamp(-1.0, 1.0), ele.clamp(-1.0, 1.0), 0.5 * rud, [thr_l, thr_r])
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
        // What each output does is in the plane's description (for the T1-S, SERVO1 to SERVO5
        // are the aileron, the V-tail panels and the throttles: see ardupilot/t1s.parm).
        let cmd = ActuatorCommands::from_pwm(&self.params, pwm);
        self.auto_launch(cmd.forward_throttle(&self.params), dt);
        self.step_physics(&cmd, dt);
    }

    fn step_physics(&mut self, cmd: &ActuatorCommands, dt: f32) {
        let wind = self.wind_at(self.state.pos, self.state.time);
        self.last_wind = wind;
        let env = Environment { wind };
        physics::step(&self.params, &mut self.state, cmd, &env, &terrain::WorldTerrain, &self.obstacles, dt);
        self.last_cmd = *cmd;
    }

    /// The name of the radio's mode switch position `i`, for the HUD.
    pub fn rc_mode_name(&self, i: usize) -> &'static str {
        self.params.rc_modes.get(i).copied().unwrap_or("-")
    }

    /// The mode the radio's mode channel is on, as ArduPilot reads it.
    pub fn mode_switch_name(&self) -> &'static str {
        self.rc_mode_name(simulation_aircraft::rc_mode_slot(self.rc_channels()[7]))
    }

    /// The arm signal being sent: the radio's switch if it has one, else the keyboard's toggle.
    pub fn arm_switch_is_on(&self) -> bool {
        self.radio_arm.unwrap_or(self.ap.arm_switch)
    }

    /// The throttle channel for a stick position. A VTOL in a Q mode (or GUIDED, which hovers)
    /// reads the stick as a climb rate with the middle holding height. A radio's stick goes as it
    /// is. The keyboard and gamepads idle with the throttle closed, so for them a closed throttle
    /// is sent as the middle and full throttle as a full climb. Everywhere else the stick is a
    /// plain throttle, and so it is if the mode isn't known (no console).
    fn throttle_pwm(&self, throttle: f32) -> u16 {
        let t = throttle.clamp(0.0, 1.0);
        let q_mode = self.ap.vehicle().is_some_and(|(_, m)| m.starts_with('Q') || m == "GUIDED");
        if q_mode && !self.pilot_radio && self.params.motors.iter().any(|m| m.lift) {
            (1500.0 + 500.0 * t).round() as u16
        } else {
            (1000.0 + 1000.0 * t).round() as u16
        }
    }

    /// RC channel PWMs sent to ArduPilot (AETR + aux, ch8 = flight mode).
    pub fn rc_channels(&self) -> [u16; 8] {
        let s = self.sticks;
        let c = |x: f32| (1500.0 + 500.0 * x.clamp(-1.0, 1.0)).round() as u16;
        [
            c(s.roll),
            // ArduPlane: RC2 above trim = nose up.
            c(s.pitch),
            self.throttle_pwm(s.throttle),
            c(s.yaw),
            // RC5_OPTION = 153 (arm/disarm switch), on channel 5 as ExpressLRS needs.
            if self.radio_arm.unwrap_or(self.ap.arm_switch) { 1900 } else { 1100 },
            1500,
            1500,
            self.radio_mode.unwrap_or_else(|| simulation_aircraft::rc_mode_pwm(self.ap.mode_index)),
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

/// Why a plane can't be flown as the sim was started, if it can't: a plane with a flight
/// controller is flown by ArduPilot (the sim's own stick modes are for the plane that hasn't one).
pub fn cannot_fly(params: &AircraftParams, title: &str, ardupilot: bool) -> Option<String> {
    (params.flight_controller && !ardupilot).then(|| {
        format!(
            "the {title} has a flight controller, and ArduPilot flies it. Start the whole station with `cd gcs && cargo run --release -- --station`, or fly the plain T1-S, which has none, with `cargo run --release -- --aircraft t1s-ranger`"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plane_with_a_flight_controller_needs_ardupilot() {
        let vtol = (simulation_aircraft::find("t1s-vtol").unwrap().params)();
        let plain = (simulation_aircraft::find("t1s-ranger").unwrap().params)();
        assert!(cannot_fly(&vtol, "VTOL", false).is_some_and(|m| m.contains("--station") && m.contains("t1s-ranger")));
        assert!(cannot_fly(&vtol, "VTOL", true).is_none());
        assert!(cannot_fly(&plain, "T1-S", false).is_none(), "the plain T1-S flies from the keyboard");
    }

    fn vtol() -> SimCore {
        SimCore::new((simulation_aircraft::find("t1s-vtol").unwrap().params)(), vec![], true, 9002)
    }

    fn in_mode(c: &mut SimCore, mode: &str) {
        c.ap.gcs = Some((Instant::now(), true, mode.into()));
    }

    /// In a Q mode the throttle stick sets the climb rate, with the middle holding height: a
    /// closed throttle (the sim's idle) has to read as the middle, or the VTOL sinks to the ground.
    #[test]
    fn a_vtol_in_a_q_mode_idles_with_the_throttle_stick_centred() {
        let mut c = vtol();
        c.sticks.throttle = 0.0;
        for mode in ["QHOVER", "QLOITER", "QSTABILIZE", "GUIDED"] {
            in_mode(&mut c, mode);
            assert_eq!(c.rc_channels()[2], 1500, "{mode}");
        }
        c.sticks.throttle = 1.0;
        in_mode(&mut c, "QLOITER");
        assert_eq!(c.rc_channels()[2], 2000, "full throttle climbs");
    }

    /// In the fixed-wing modes the throttle is a throttle: closed is closed.
    #[test]
    fn a_vtol_in_a_fixed_wing_mode_has_a_plain_throttle() {
        let mut c = vtol();
        c.sticks.throttle = 0.0;
        for mode in ["MANUAL", "CRUISE", "FBWA", "AUTO"] {
            in_mode(&mut c, mode);
            assert_eq!(c.rc_channels()[2], 1000, "{mode}");
        }
        // The mode isn't known (no console): the safe reading is a plain throttle.
        c.ap.gcs = None;
        assert_eq!(c.rc_channels()[2], 1000);
    }

    /// A real radio's throttle stick is where the pilot holds it: all the way down is all the way
    /// down (a descent in a Q mode), not the idle of the keyboard.
    #[test]
    fn a_radio_throttle_stick_goes_to_ardupilot_as_it_is() {
        let mut c = vtol();
        c.pilot_radio = true;
        in_mode(&mut c, "QLOITER");
        for (stick, pwm) in [(0.0, 1000), (0.5, 1500), (0.75, 1750), (1.0, 2000)] {
            c.sticks.throttle = stick;
            assert_eq!(c.rc_channels()[2], pwm, "stick at {stick}");
        }
    }

    /// The radio's mode channel starts in its first position, QHOVER.
    #[test]
    fn a_vtol_starts_with_the_mode_switch_on_qhover() {
        let c = vtol();
        assert_eq!(c.rc_mode_name(c.ap.mode_index), "QHOVER");
        assert_eq!(simulation_aircraft::rc_mode_slot(c.rc_channels()[7]), c.ap.mode_index);
    }

    /// A radio switch for arm sends its position, not a toggle: up is armed, anything else is not.
    #[test]
    fn a_radio_arm_switch_sends_its_position() {
        let mut c = vtol();
        c.ap.arm_switch = true; // the keyboard's toggle says armed
        c.radio_arm = Some(false);
        assert_eq!(c.rc_channels()[4], 1100, "the radio's switch is down: the toggle doesn't count");
        c.radio_arm = Some(true);
        assert_eq!(c.rc_channels()[4], 1900);
        c.radio_arm = None;
        assert_eq!(c.rc_channels()[4], 1900, "without a radio switch the toggle decides");
    }

    /// The radio's mode channel goes to ArduPilot as it is: ArduPilot picks the mode from the band.
    #[test]
    fn a_radio_mode_channel_goes_through_as_it_is() {
        let mut c = vtol();
        for pwm in [1000, 1263, 1500, 1750, 2000] {
            c.radio_mode = Some(pwm);
            assert_eq!(c.rc_channels()[7], pwm);
        }
        c.radio_mode = None;
        assert_eq!(c.rc_channels()[7], simulation_aircraft::rc_mode_pwm(c.ap.mode_index), "without one, the mode key's slot");
    }

    /// What the HUD shows of the switches is what ArduPilot is being sent, from the radio or the keys.
    #[test]
    fn the_hud_shows_the_switches_as_sent() {
        let mut c = vtol();
        assert_eq!((c.mode_switch_name(), c.arm_switch_is_on()), ("QHOVER", false));
        c.radio_mode = Some(1295);
        c.radio_arm = Some(true);
        assert_eq!((c.mode_switch_name(), c.arm_switch_is_on()), ("QLOITER", true));
        c.radio_mode = Some(2000);
        assert_eq!(c.mode_switch_name(), "QRTL");
    }

    /// The T1-S has no Q modes, whatever mode name it reports.
    #[test]
    fn the_plain_t1s_always_has_a_plain_throttle() {
        let mut c = SimCore::new((simulation_aircraft::find("t1s-ranger").unwrap().params)(), vec![], false, 9002);
        c.sticks.throttle = 0.0;
        in_mode(&mut c, "QHOVER");
        assert_eq!(c.rc_channels()[2], 1000);
    }
}
