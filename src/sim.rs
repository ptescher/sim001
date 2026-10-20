//! Simulation core shared between the render thread and the ArduPilot bridge thread.

use recreation_games::{Field, Kind, PILOTS, Radio, Session, Surroundings};
use recreation_games::pilot::Fbwa;
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
    /// ArduPilot says its pre-arm checks pass (its SYS_STATUS), so that it will arm.
    pub prearm_ok: bool,
    /// What ArduPilot says and reports, for the tests: when one fails, this is what it saw.
    pub watch: ApWatch,
}

/// ArduPilot as the sim's own MAVLink link hears it: its recent messages and last readings.
#[derive(Clone, Debug, Default)]
pub struct ApWatch {
    /// The newest STATUSTEXTs, oldest first.
    pub messages: std::collections::VecDeque<String>,
    /// The estimator's status flags (EKF_STATUS_REPORT), once heard.
    pub ekf_flags: Option<u16>,
    /// The channels ArduPilot received (RC_CHANNELS), in µs.
    pub rc: Option<[u16; 8]>,
    /// The outputs it drives (SERVO_OUTPUT_RAW), in µs.
    pub servo: Option<[u16; 8]>,
    /// Height above home in m, climb in m/s and throttle in % (VFR_HUD).
    pub hud: Option<(f32, f32, u16)>,
}

impl ApWatch {
    /// The estimator can fly on: attitude, horizontal and vertical velocity, and relative and
    /// absolute horizontal position and vertical position are all good. ArduPilot may not send
    /// its estimator status; then there is nothing to wait for.
    pub fn estimator_ready(&self) -> bool {
        const NEEDED: u16 = 1 | 2 | 4 | 8 | 16 | 32;
        self.ekf_flags.is_none_or(|f| f & NEEDED == NEEDED)
    }

    pub fn say(&mut self, text: String) {
        if self.messages.len() >= 30 {
            self.messages.pop_front();
        }
        self.messages.push_back(text);
    }

    /// What ArduPilot was doing, in a few lines, for a test that did not get what it waited for.
    pub fn report(&self, mode: Option<(bool, &str)>) -> String {
        let mut o = String::from("ArduPilot: ");
        match mode {
            Some((armed, m)) => o.push_str(&format!("{m}, {}", if armed { "armed" } else { "disarmed" })),
            None => o.push_str("no heartbeat"),
        }
        match self.ekf_flags {
            Some(f) => o.push_str(&format!("; estimator flags {f:#06x}{}", if self.estimator_ready() { " (ready)" } else { " (not ready)" })),
            None => o.push_str("; no estimator status"),
        }
        if let Some(rc) = self.rc {
            o.push_str(&format!("; it receives {rc:?}"));
        }
        if let Some(s) = self.servo {
            o.push_str(&format!("; it drives {s:?}"));
        }
        if let Some((alt, climb, thr)) = self.hud {
            o.push_str(&format!("; {alt:.1} m, climb {climb:+.1} m/s, throttle {thr} %"));
        }
        let last: Vec<&str> = self.messages.iter().rev().take(8).map(String::as_str).collect();
        if !last.is_empty() {
            o.push_str(&format!("; it said: {}", last.into_iter().rev().collect::<Vec<_>>().join(" | ")));
        }
        o
    }
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
    pub test_radio: Option<teleoperation_shared::feed::RadioState>,
    /// The pilot flies with a radio (not the keyboard or a gamepad): its throttle stick is where
    /// the pilot holds it.
    pub pilot_radio: bool,
    /// The radio's arm switch, if the pilot gave it one: up is armed. It decides the arm signal
    /// whatever the keyboard's toggle says.
    pub radio_arm: Option<bool>,
    /// The radio's mode channel in µs, if the pilot gave it one: it goes to ArduPilot as it is.
    pub radio_mode: Option<u16>,
    /// With a flight computer on a UART (`--flight-computer`): the radio channels it gives
    /// ArduPilot, in µs. They replace the sticks, the keyboard and the USB controller.
    pub uart_rc: Option<[u16; 16]>,
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
    /// The game being played (G: a pylon race, T: tag), with its computer pilots.
    pub game: Option<Session>,
    /// With ArduPilot, the computer pilots' planes, each flown by an autopilot of its own (see
    /// `step_opponent`): they stand on the field beside the pilot box until a game takes them.
    pub parked: Vec<FlightState>,
    /// Each computer pilot's autopilot link, as `ap` is yours.
    pub others: Vec<OtherLink>,
    /// Something that moves the sticks as the plane flies, 60 times a second of sim time (the
    /// acceptance tests' stand-in pilot): it runs inside the physics loop, so it flies the same
    /// whether the sim runs at real time or faster.
    pub hook: Option<Hook>,
    hook_clock: f32,
    /// FBWA as the autopilots fly it: what the computer pilots' roll and pitch sticks ask for.
    pub fbwa: Fbwa,
}

/// A function called with the sim and the time since its last call.
pub type Hook = Box<dyn FnMut(&mut SimCore, f32) + Send>;

/// How often a hook is called (s of sim time).
const HOOK_DT: f32 = 1.0 / 60.0;

/// A computer pilot's autopilot, as the sim sees it.
#[derive(Clone, Debug, Default)]
pub struct OtherLink {
    pub connected: bool,
    pub last_packet: Option<Instant>,
}

/// FBWA's limits from the plane's ArduPilot parameter files, layered as ArduPilot layers them
/// (a later file wins), over ArduPlane's own defaults.
pub fn fbwa_from_parms(files: &[&str]) -> Fbwa {
    Fbwa::from_parms(files)
}

/// The T1-S's ArduPilot parameters, as ardupilot/start.sh gives them to SITL.
fn t1s_fbwa() -> Fbwa {
    fbwa_from_parms(&[include_str!("../ardupilot/t1s.parm"), include_str!("../ardupilot/t1s-vtol.parm")])
}

fn expo(x: f32, e: f32) -> f32 {
    e * x * x * x + (1.0 - e) * x
}

impl SimCore {
    pub fn new(params: AircraftParams, obstacles: Vec<Obstacle>, ap_enabled: bool, ap_port: u16) -> Self {
        let state = FlightState::new(&params, &terrain::start());
        let parked = if ap_enabled { (0..PILOTS.len()).map(|i| FlightState::new(&params, &Self::field().hand(i))).collect() } else { vec![] };
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
            uart_rc: None,
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
                prearm_ok: false,
                watch: ApWatch::default(),
            },
            pid: PidState::default(),
            launch_hold: 0.0,
            resets: 0,
            direct: false,
            game: None,
            parked,
            others: if ap_enabled { vec![OtherLink::default(); PILOTS.len()] } else { vec![] },
            hook: None,
            hook_clock: 0.0,
            fbwa: t1s_fbwa(),
        }
    }

    /// Where the games are played: the pilot box, facing down the field.
    pub fn field() -> Field {
        let p = terrain::pilot_pos();
        Field { pilot: Vec3::new(p.x, terrain::ground_height(p.x, p.z), p.z), heading: terrain::launch_heading() }
    }

    /// G or T: start that game, with the plane back in the hand on a fresh pack, or stop it if
    /// it is the one being played.
    pub fn toggle_game(&mut self, kind: Kind) {
        let same = self.game.as_ref().is_some_and(|g| g.kind == kind);
        if let Some(game) = self.game.take() {
            // The computer pilots' planes stay where they are, with their autopilots.
            if self.ap.enabled {
                self.parked = game.into_planes();
            }
        }
        if same {
            return;
        }
        if self.ap.enabled {
            // Nobody's autopilot restarts (it takes a minute): your plane is carried to the
            // pilot box, the computer pilots' to their places, and the countdown waits for them.
            self.state.reset_to_hand(&self.params, &terrain::start());
            self.state.recharge();
            let planes = std::mem::take(&mut self.parked);
            let session = match kind {
                Kind::Race => Session::race(Self::field()),
                Kind::Tag => Session::tag(Self::field()),
            };
            self.game = Some(session.with_autopilots(self.params.clone(), self.fbwa, planes));
            return;
        }
        self.reset();
        self.state.snap_to_hand();
        self.state.recharge();
        self.game = Some(match kind {
            Kind::Race => Session::race(Self::field()).after_takeoff(),
            Kind::Tag => Session::tag(Self::field()).after_takeoff(),
        });
    }

    /// A throw is allowed now: not during a game's countdown.
    pub fn may_launch(&self) -> bool {
        self.game.as_ref().is_none_or(|g| g.may_launch())
    }

    /// Space: throw the plane, if a throw is allowed.
    pub fn launch(&mut self) {
        if self.may_launch() {
            self.state.launch();
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
        wind_at(&self.wind, pos, t)
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
        let mut cmd = t1s_ranger::mix(ail.clamp(-1.0, 1.0), ele.clamp(-1.0, 1.0), 0.5 * rud, [thr_l, thr_r]);
        // A VTOL flown from the sticks is a plane: the wing motors point forward.
        for m in &self.params.motors {
            if let Some(t) = m.tilt {
                cmd.out[t.channel] = 1.0;
            }
        }
        cmd
    }

    /// Hand-launch automatically once throttle is held up, like a real throw.
    fn auto_launch(&mut self, throttle: f32, dt: f32) {
        if self.state.phase == Phase::Held && self.may_launch() {
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
        self.run_hook(dt);
        let cmd = self.stick_commands(dt);
        self.auto_launch(self.sticks.throttle, dt);
        self.step_physics(&cmd, dt);
    }

    fn run_hook(&mut self, dt: f32) {
        self.hook_clock += dt;
        if self.hook_clock < HOOK_DT || self.hook.is_none() {
            return;
        }
        let since = std::mem::take(&mut self.hook_clock);
        if let Some(mut hook) = self.hook.take() {
            hook(self, since);
            self.hook = Some(hook);
        }
    }

    /// Lockstep step driven by ArduPilot servo outputs.
    pub fn step_ardupilot(&mut self, pwm: &[u16; 16], dt: f32) {
        if self.direct {
            self.step_standalone(dt);
            return;
        }
        // The tests' stand-in pilot holds the sticks through the hook, as it does without ArduPilot.
        self.run_hook(dt);
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
        // The computer pilots fly in the same wind, over the same ground and trees.
        if let Some(game) = &mut self.game {
            let w = self.wind;
            let world = Surroundings { ground: &terrain::WorldTerrain, obstacles: &self.obstacles, wind: &move |p, t| wind_at(&w, p, t) };
            game.step(&self.state, &world, dt);
        }
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
    /// plain throttle, and so it is if the mode isn't known (no console) or the plane is still
    /// disarmed: ArduPilot arms only with the throttle stick at the bottom.
    fn throttle_pwm(&self, throttle: f32) -> u16 {
        let t = throttle.clamp(0.0, 1.0);
        let q_mode = self.ap.vehicle().is_some_and(|(armed, m)| armed && (m.starts_with('Q') || m == "GUIDED"));
        if q_mode && !self.pilot_radio && self.params.motors.iter().any(|m| m.lift) {
            (1500.0 + 500.0 * t).round() as u16
        } else {
            (1000.0 + 1000.0 * t).round() as u16
        }
    }

    /// RC channel PWMs sent to ArduPilot (AETR + aux, ch8 = flight mode). With a flight computer
    /// on a UART, they are what it reads from the TX15.
    pub fn rc_channels(&self) -> [u16; 8] {
        let mut rc = match self.uart_rc {
            Some(rc) => std::array::from_fn(|i| rc[i]),
            None => self.pilot_channels(),
        };
        // In a game with ArduPilot nobody arms before GO, whatever is carrying the radio.
        if self.ap.enabled && !self.may_launch() {
            rc[4] = rc[4].min(1100);
        }
        rc
    }

    /// The channels the pilot's own controls make: what a TX15 stand-in puts on the UART.
    pub fn pilot_channels(&self) -> [u16; 8] {
        let s = self.sticks;
        let c = |x: f32| (1500.0 + 500.0 * x.clamp(-1.0, 1.0)).round() as u16;
        [
            c(s.roll),
            // ArduPlane: RC2 above trim = nose up.
            c(s.pitch),
            self.throttle_pwm(s.throttle),
            c(s.yaw),
            // RC5_OPTION = 153 (arm/disarm switch), on channel 5 as ExpressLRS needs. In a game
            // with ArduPilot nobody arms before GO, so the switch is held off until then.
            if self.radio_arm.unwrap_or(self.ap.arm_switch) && (!self.ap.enabled || self.may_launch()) { 1900 } else { 1100 },
            1500,
            1500,
            self.radio_mode.unwrap_or_else(|| simulation_aircraft::rc_mode_pwm(self.ap.mode_index)),
        ]
    }

    /// One step of computer pilot `i`'s plane, in lockstep with its own autopilot (`pwm` is what
    /// that autopilot sends), and the sensor packet for it, with what the computer pilot's radio
    /// sends after the step.
    pub fn step_opponent(&mut self, i: usize, pwm: &[u16; 16], dt: f32) -> String {
        if let Some(link) = self.others.get_mut(i) {
            link.connected = true;
            link.last_packet = Some(Instant::now());
        }
        let cmd = ActuatorCommands::from_pwm(&self.params, pwm);
        let w = self.wind;
        let ground = &terrain::WorldTerrain;
        let (state, radio) = match &mut self.game {
            Some(game) if i < game.opponents.len() => {
                let s = &mut game.opponents[i].state;
                let wind = wind_at(&w, s.pos, s.time);
                physics::step(&self.params, s, &cmd, &Environment { wind }, ground, &self.obstacles, dt);
                let radio = game.radio(i, ground, dt);
                (&game.opponents[i].state, radio)
            }
            _ => {
                let Some(s) = self.parked.get_mut(i) else { return String::new() };
                let wind = wind_at(&w, s.pos, s.time);
                physics::step(&self.params, s, &cmd, &Environment { wind }, ground, &self.obstacles, dt);
                // Parked: switch off, throttle down, in the mode switch's first position.
                (&self.parked[i], Radio::default())
            }
        };
        let wind = wind_at(&w, state.pos, state.time);
        sitl_json(state, wind, &radio_channels(&radio))
    }

    /// Build the JSON sensor packet for ArduPilot's JSON SITL backend.
    pub fn sitl_json(&self) -> String {
        sitl_json(&self.state, self.last_wind, &self.rc_channels())
    }
}

/// The RC channels a computer pilot's radio sends (AETR, the arm switch on 5, the mode on 8).
fn radio_channels(r: &Radio) -> [u16; 8] {
    let c = |x: f32| (1500.0 + 500.0 * x.clamp(-1.0, 1.0)).round() as u16;
    let s = r.sticks;
    [
        c(s.aileron),
        c(s.elevator),
        (1000.0 + 1000.0 * s.throttle.clamp(0.0, 1.0)).round() as u16,
        c(s.rudder),
        if r.arm { 1900 } else { 1100 },
        1500,
        1500,
        simulation_aircraft::rc_mode_pwm(r.mode),
    ]
}

/// The JSON sensor packet for ArduPilot's JSON SITL backend: a plane's state, the wind it flies
/// in, and the RC channels its radio sends.
fn sitl_json(s: &FlightState, wind: Vec3, rc: &[u16; 8]) -> String {
    let p = terrain::pilot_pos();
    simulation_sitl::sensor_json(s, wind, rc, Vec3::new(p.x, terrain::ground_height(p.x, p.z), p.z))
}

/// Wind at a point: mean wind with a boundary-layer profile plus turbulence.
pub fn wind_at(w: &WindSettings, pos: Vec3, t: f64) -> Vec3 {
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

pub fn wrap_deg(rad: f32) -> f32 {
    let d = rad * 180.0 / PI;
    (d % 360.0 + 360.0) % 360.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use recreation_games::{Clock, Kind};

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

    /// A disarmed plane is armed with the throttle stick at the bottom (ArduPilot refuses with
    /// "Throttle (RC3) is not neutral" otherwise), so the idle of the keyboard is a closed throttle
    /// until the plane is armed, in a Q mode too.
    #[test]
    fn a_disarmed_vtol_in_a_q_mode_sends_the_throttle_closed() {
        let mut c = vtol();
        c.sticks.throttle = 0.0;
        for mode in ["QHOVER", "QLOITER", "QSTABILIZE"] {
            c.ap.gcs = Some((Instant::now(), false, mode.into()));
            assert_eq!(c.rc_channels()[2], 1000, "{mode}, disarmed");
            in_mode(&mut c, mode);
            assert_eq!(c.rc_channels()[2], 1500, "{mode}, armed");
        }
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

    /// The pulse width of each of the 16 outputs, all idle: what a disarmed autopilot sends.
    const IDLE: [u16; 16] = [1500, 1500, 1000, 1500, 1500, 1000, 1000, 1000, 1000, 1000, 1000, 1000, 1000, 1000, 1000, 1000];

    /// Steps your plane (as its autopilot's bridge does) and each computer pilot's (as theirs
    /// do), with every autopilot idle; returns the computer pilots' last JSON replies.
    fn run_ardupilot(c: &mut SimCore, seconds: f32) -> Vec<String> {
        let dt = 1.0 / 400.0;
        let mut replies = vec![String::new(); recreation_games::PILOTS.len()];
        for _ in 0..(seconds / dt) as usize {
            c.step_ardupilot(&IDLE, dt);
            for (i, r) in replies.iter_mut().enumerate() {
                *r = c.step_opponent(i, &IDLE, dt);
            }
        }
        replies
    }

    /// Channel `n` (1-based) of a JSON reply's RC block.
    fn rc_in(json: &str, n: usize) -> u16 {
        let key = format!("\"rc_{n}\":");
        let at = json.find(&key).unwrap() + key.len();
        json[at..].split(|c: char| !c.is_ascii_digit()).next().unwrap().parse().unwrap()
    }

    /// With ArduPilot, the computer pilots' planes are there before a game: on the field beside
    /// the pilot box, each answering its own autopilot, disarmed.
    #[test]
    fn with_ardupilot_the_computer_pilots_planes_answer_their_autopilots() {
        let mut c = vtol();
        let replies = run_ardupilot(&mut c, 1.0);
        for r in &replies {
            assert!(r.contains("\"position\""), "a sensor packet: {r}");
            assert_eq!(rc_in(r, 5), 1100, "arm switch off");
            assert_eq!(rc_in(r, 3), 1000, "throttle down");
        }
        assert_eq!(c.parked.len(), recreation_games::PILOTS.len());
    }

    /// A game with ArduPilot doesn't restart anyone's autopilot (a restart takes a minute): your
    /// plane is carried to the pilot box, and the computer pilots fly with their autopilots.
    #[test]
    fn with_ardupilot_a_game_starts_without_restarting_the_autopilot() {
        let mut c = vtol();
        run_ardupilot(&mut c, 0.5);
        let resets = c.resets;
        c.toggle_game(Kind::Race);
        assert_eq!(c.resets, resets, "no restart");
        let g = c.game.as_ref().unwrap();
        assert!(matches!(g.clock, Clock::CountingDown { .. }));
        assert!(g.opponent_params().flight_controller, "their planes are like yours, with a flight controller");
        assert!(c.parked.is_empty(), "the game has their planes");
        c.toggle_game(Kind::Race);
        assert_eq!(c.parked.len(), recreation_games::PILOTS.len(), "and gives them back");
    }

    /// Nobody arms before GO: your arm switch reaches ArduPilot only once the game is running.
    #[test]
    fn with_ardupilot_your_arm_switch_waits_for_go() {
        let mut c = vtol();
        run_ardupilot(&mut c, 0.5);
        c.toggle_game(Kind::Race);
        c.radio_arm = Some(true);
        assert_eq!(c.rc_channels()[4], 1100, "held off during the countdown");
        run_ardupilot(&mut c, recreation_games::COUNTDOWN + 3.0);
        assert!(matches!(c.game.as_ref().unwrap().clock, Clock::Running { .. }), "{:?}", c.game.as_ref().unwrap().clock);
        assert_eq!(c.rc_channels()[4], 1900, "at GO it goes through");
    }

    /// The same holds when the radio comes through the flight computer's UART.
    #[test]
    fn with_ardupilot_the_arm_switch_waits_for_go_on_the_uart_too() {
        let mut c = vtol();
        run_ardupilot(&mut c, 0.5);
        c.toggle_game(Kind::Race);
        let mut rc = [1500u16; 16];
        rc[4] = 1900;
        c.uart_rc = Some(rc);
        assert_eq!(c.rc_channels()[4], 1100, "held off during the countdown");
        run_ardupilot(&mut c, recreation_games::COUNTDOWN + 3.0);
        assert_eq!(c.rc_channels()[4], 1900, "at GO it goes through");
    }

    /// At GO the computer pilots put their arm switches on.
    #[test]
    fn with_ardupilot_the_computer_pilots_arm_at_go() {
        let mut c = vtol();
        run_ardupilot(&mut c, 0.5);
        c.toggle_game(Kind::Race);
        let before = run_ardupilot(&mut c, 1.0);
        assert!(before.iter().all(|r| rc_in(r, 5) == 1100));
        let after = run_ardupilot(&mut c, recreation_games::COUNTDOWN + 2.0);
        assert!(after.iter().all(|r| rc_in(r, 5) == 1900), "{after:?}");
    }

    /// FBWA's limits come from the plane's parameter files: later files win, as ArduPilot layers
    /// them, and ArduPlane's own defaults fill in the rest.
    #[test]
    fn fbwa_reads_its_limits_from_the_parameter_files() {
        let f = fbwa_from_parms(&["# a comment\nROLL_LIMIT_DEG 50\nPTCH_LIM_MAX_DEG 25", "PTCH_LIM_MAX_DEG 20\n"]);
        assert!((f.roll_limit.to_degrees() - 50.0).abs() < 1e-3);
        assert!((f.pitch_max.to_degrees() - 20.0).abs() < 1e-3, "the later file wins");
        assert!((f.pitch_min.to_degrees() + 25.0).abs() < 1e-3, "ArduPlane's default");
    }

    /// Flown from the sticks (`--direct`), a VTOL is a plane: its wing motors point forward and
    /// the lift rotor is off.
    #[test]
    fn a_vtol_flown_from_the_sticks_points_its_wing_motors_forward() {
        let mut c = vtol();
        c.sticks.throttle = 0.8;
        let cmd = c.stick_commands(0.01);
        for m in &c.params.motors {
            if let Some(t) = m.tilt {
                assert_eq!(cmd.out[t.channel], 1.0, "tilt output {} is full forward", t.channel);
            } else {
                assert_eq!(cmd.out[m.channel], 0.0, "the lift rotor is off");
            }
        }
        assert!(cmd.out[simulation_aircraft::t1s_vtol::MOTOR_LEFT] > 0.7, "the wing motors take the throttle");
    }

    fn plain() -> SimCore {
        SimCore::new((simulation_aircraft::find("t1s-ranger").unwrap().params)(), vec![], false, 9002)
    }

    fn run(c: &mut SimCore, seconds: f32) {
        for _ in 0..(seconds * 600.0) as usize {
            c.step_standalone(1.0 / 600.0);
        }
    }

    /// G and T start a game with the plane back in the hand, on a fresh pack.
    #[test]
    fn a_game_starts_with_the_plane_in_hand_on_a_fresh_pack() {
        let mut c = plain();
        c.state.phase = Phase::Flying;
        c.state.pos.y += 30.0;
        c.state.used_mah = 900.0;
        c.toggle_game(Kind::Race);
        assert!(c.game.as_ref().is_some_and(|g| g.kind == Kind::Race));
        assert_eq!(c.state.phase, Phase::Held);
        assert!(c.state.carry.is_none(), "straight back in the hand");
        assert_eq!(c.state.used_mah, 0.0);
    }

    /// The game waits for the takeoff, then counts down from the throw.
    #[test]
    fn the_game_starts_after_the_takeoff() {
        let mut c = plain();
        c.toggle_game(Kind::Race);
        run(&mut c, 8.0);
        assert_eq!(c.game.as_ref().unwrap().clock, Clock::Waiting, "waiting while the plane is in the hand");
        assert!(c.game.as_ref().unwrap().opponents.iter().all(|o| o.state.phase == Phase::Held));
        c.sticks.throttle = 0.9;
        run(&mut c, 1.5);
        assert_ne!(c.state.phase, Phase::Held, "the throw is allowed at once");
        assert!(matches!(c.game.as_ref().unwrap().clock, Clock::CountingDown { .. }));
        run(&mut c, 6.0);
        assert!(matches!(c.game.as_ref().unwrap().clock, Clock::Running { .. }));
    }

    /// The key of the game stops it; the other key switches to the other game.
    #[test]
    fn the_game_keys_start_switch_and_stop() {
        let mut c = plain();
        c.toggle_game(Kind::Race);
        c.toggle_game(Kind::Tag);
        assert!(c.game.as_ref().is_some_and(|g| g.kind == Kind::Tag));
        c.toggle_game(Kind::Tag);
        assert!(c.game.is_none());
        assert!(c.may_launch(), "free flight throws whenever");
    }

    /// The computer pilots fly on the same physics, stepped with the plane.
    #[test]
    fn the_opponents_fly_while_the_sim_runs() {
        let mut c = plain();
        c.toggle_game(Kind::Tag);
        c.sticks.throttle = 0.9;
        run(&mut c, 12.0);
        let g = c.game.as_ref().unwrap();
        assert!(g.opponents.iter().all(|o| o.state.phase == Phase::Flying), "{:?}", g.opponents.iter().map(|o| o.state.phase).collect::<Vec<_>>());
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
