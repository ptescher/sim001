//! Pilot input: keyboard, RC transmitters in USB-joystick mode (e.g. RadioMaster TX15 /
//! EdgeTX) and gamepads (e.g. Nintendo Switch Pro Controller).
//!
//! Radios are not in the SDL gamepad database, so their channels arrive as raw HID axes
//! with platform-specific names. The F2 wizard learns the mapping by watching the sticks
//! move, and stores it in `controls.cfg`.

use crate::gc_pad::{GcPads, PadSnapshot};
use crate::sim::Sticks;
use bevy::input::gamepad::{AxisSettings, GamepadAxis, GamepadButton, GamepadInput, GamepadSettings};
use bevy::prelude::*;
use std::collections::HashMap;
use std::fmt::Write as _;

pub const CONFIG_PATH: &str = "controls.cfg";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Launch,
    Reset,
    Camera,
    Mode,
    Rates,
    Pause,
    DiffThrust,
    Help,
    Calibrate,
    RawDebug,
    ApArm,
    ApMode,
    Recharge,
    Zoom,
    ToggleProfile,
    /// ArduPilot mode: switch between ArduPilot flying the plane and the sticks flying it
    /// directly through the sim's own flight modes.
    ControlSource,
    /// Close overlays / help / the calibration wizard.
    Dismiss,
    /// Start a pylon race, or stop the one being flown.
    Race,
    /// Start a game of tag, or stop the one being played.
    Tag,
}

const RADIO_ACTIONS: [(Action, &str); 6] = [
    (Action::Launch, "LAUNCH"),
    (Action::Reset, "RESET"),
    (Action::Camera, "CAMERA"),
    (Action::ApArm, "ARDUPILOT ARM"),
    (Action::ApMode, "ARDUPILOT MODE"),
    (Action::ControlSource, "CONTROL: ARDUPILOT / DIRECT"),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisMap {
    pub axis: GamepadAxis,
    pub center: f32,
    /// Signed scale so that (raw - center) * scale is +1 at full right / up deflection.
    pub scale: f32,
}

impl AxisMap {
    fn read(&self, pad: &Gamepad) -> f32 {
        let raw = pad.get_unclamped(self.axis).unwrap_or(self.center);
        ((raw - self.center) * self.scale).clamp(-1.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThrottleMap {
    pub axis: GamepadAxis,
    pub low: f32,
    pub high: f32,
}

#[derive(Clone, Debug)]
pub struct RadioConfig {
    pub roll: AxisMap,
    /// +1 = elevator stick pushed forward.
    pub pitch: AxisMap,
    pub yaw: AxisMap,
    pub throttle: ThrottleMap,
    pub buttons: HashMap<Action, GamepadButton>,
    /// The switch the pilot gave to ArduPilot arm: a channel that reads up (+1) to arm.
    pub arm: Option<AxisMap>,
    /// The channel the pilot gave to ArduPilot mode: it goes to the autopilot as it is.
    pub mode: Option<AxisMap>,
    /// A knob for the line-of-sight camera's zoom, and one that picks the camera view.
    pub zoom: Option<AxisMap>,
    pub view: Option<AxisMap>,
    /// Substrings of device names that should be treated as a radio.
    pub names: Vec<String>,
    pub calibrated: bool,
}

impl Default for RadioConfig {
    fn default() -> Self {
        // EdgeTX "classic" joystick with AETR channel order, as seen through gilrs on macOS:
        // CH1 = HID X, CH2 = HID Y, CH3 = HID Z, CH4 = HID Rx.
        Self {
            roll: AxisMap { axis: GamepadAxis::LeftStickX, center: 0.0, scale: 1.0 },
            pitch: AxisMap { axis: GamepadAxis::LeftStickY, center: 0.0, scale: 1.0 },
            yaw: AxisMap { axis: GamepadAxis::Other(0x0001_0033), center: 0.0, scale: 1.0 },
            throttle: ThrottleMap { axis: GamepadAxis::RightStickX, low: -1.0, high: 1.0 },
            buttons: HashMap::new(),
            arm: None,
            mode: None,
            zoom: None,
            view: None,
            names: ["tx15", "edgetx", "radiomaster", "opentx", "frsky", "jumper", "tx16", "boxer", "zorro"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            calibrated: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GamepadConfig {
    /// Transmitter-style stick mode (2 = throttle on the left stick).
    pub stick_mode: u8,
    pub deadzone: f32,
    pub throttle_rate: f32,
}

impl Default for GamepadConfig {
    fn default() -> Self {
        Self { stick_mode: 2, deadzone: 0.07, throttle_rate: 0.7 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Radio,
    Gamepad,
}

#[derive(Clone, Debug)]
enum WizardStep {
    Baseline,
    Axis { which: &'static str, prompt: &'static str },
    Throttle,
    Button(usize),
    Done,
}

#[derive(Clone, Debug)]
pub struct Wizard {
    step: WizardStep,
    baseline: HashMap<GamepadAxis, f32>,
    hold: f32,
    candidate: Option<(GamepadAxis, f32)>,
    waiting_for_center: bool,
    result: RadioConfig,
    pub message: String,
}

#[derive(Resource)]
pub struct PilotInput {
    pub sticks: Sticks,
    pub actions: Vec<Action>,
    pub source: String,
    pub profile: Option<Profile>,
    pub active_pad: Option<Entity>,
    pub radio: RadioConfig,
    pub gamepad: GamepadConfig,
    pub wizard: Option<Wizard>,
    pub show_raw: bool,
    pub raw_text: String,
    pub profile_override: HashMap<String, Profile>,
    kb: Sticks,
    kb_throttle_used: bool,
    pub notice: String,
    /// Live gamepad state this frame (for the controller overlay).
    pub pad_view: Option<PadSnapshot>,
    /// Ignore every controller and fly with the keyboard, whatever is plugged in (`--expect`).
    pub keyboard_only: bool,
    /// Listen to this one controller only (the test radio, `--test-radio`).
    pub only_pad: Option<Entity>,
    /// What the radio's arm switch and mode channel read this frame, if the pilot gave it them.
    pub radio_arm: Option<bool>,
    pub radio_mode: Option<u16>,
    /// The radio's zoom and view knobs, 0 to 1, if the pilot gave it them.
    pub radio_zoom: Option<f32>,
    pub radio_view: Option<f32>,
    /// `--calibrate` (from the launcher): open the radio wizard as soon as a controller is there.
    pub calibrate_on_start: bool,
}

impl PilotInput {
    pub fn load() -> Self {
        let mut s = Self {
            sticks: Sticks::default(),
            actions: vec![],
            source: "keyboard".into(),
            profile: None,
            active_pad: None,
            radio: RadioConfig::default(),
            gamepad: GamepadConfig::default(),
            wizard: None,
            show_raw: false,
            raw_text: String::new(),
            profile_override: HashMap::new(),
            kb: Sticks::default(),
            kb_throttle_used: false,
            notice: String::new(),
            pad_view: None,
            keyboard_only: false,
            only_pad: None,
            radio_arm: None,
            radio_mode: None,
            radio_zoom: None,
            radio_view: None,
            calibrate_on_start: std::env::args().any(|a| a == "--calibrate"),
        };
        if let Ok(text) = std::fs::read_to_string(CONFIG_PATH) {
            parse_config(&text, &mut s.radio, &mut s.gamepad);
        }
        s
    }

    pub fn has(&self, a: Action) -> bool {
        self.actions.contains(&a)
    }
}

// ---------- config file ----------

fn axis_to_str(a: GamepadAxis) -> String {
    match a {
        GamepadAxis::LeftStickX => "LeftStickX".into(),
        GamepadAxis::LeftStickY => "LeftStickY".into(),
        GamepadAxis::LeftZ => "LeftZ".into(),
        GamepadAxis::RightStickX => "RightStickX".into(),
        GamepadAxis::RightStickY => "RightStickY".into(),
        GamepadAxis::RightZ => "RightZ".into(),
        GamepadAxis::Other(n) => format!("Other({n})"),
    }
}

fn axis_from_str(s: &str) -> Option<GamepadAxis> {
    Some(match s {
        "LeftStickX" => GamepadAxis::LeftStickX,
        "LeftStickY" => GamepadAxis::LeftStickY,
        "LeftZ" => GamepadAxis::LeftZ,
        "RightStickX" => GamepadAxis::RightStickX,
        "RightStickY" => GamepadAxis::RightStickY,
        "RightZ" => GamepadAxis::RightZ,
        _ => GamepadAxis::Other(s.strip_prefix("Other(")?.strip_suffix(')')?.parse().ok()?),
    })
}

const BUTTONS: [(GamepadButton, &str); 19] = [
    (GamepadButton::South, "South"),
    (GamepadButton::East, "East"),
    (GamepadButton::North, "North"),
    (GamepadButton::West, "West"),
    (GamepadButton::C, "C"),
    (GamepadButton::Z, "Z"),
    (GamepadButton::LeftTrigger, "LeftTrigger"),
    (GamepadButton::LeftTrigger2, "LeftTrigger2"),
    (GamepadButton::RightTrigger, "RightTrigger"),
    (GamepadButton::RightTrigger2, "RightTrigger2"),
    (GamepadButton::Select, "Select"),
    (GamepadButton::Start, "Start"),
    (GamepadButton::Mode, "Mode"),
    (GamepadButton::LeftThumb, "LeftThumb"),
    (GamepadButton::RightThumb, "RightThumb"),
    (GamepadButton::DPadUp, "DPadUp"),
    (GamepadButton::DPadDown, "DPadDown"),
    (GamepadButton::DPadLeft, "DPadLeft"),
    (GamepadButton::DPadRight, "DPadRight"),
];

fn button_to_str(b: GamepadButton) -> String {
    if let GamepadButton::Other(n) = b {
        return format!("Other({n})");
    }
    BUTTONS.iter().find(|(x, _)| *x == b).map(|(_, s)| s.to_string()).unwrap_or_default()
}

fn button_from_str(s: &str) -> Option<GamepadButton> {
    if let Some(n) = s.strip_prefix("Other(").and_then(|r| r.strip_suffix(')')) {
        return n.parse().ok().map(GamepadButton::Other);
    }
    BUTTONS.iter().find(|(_, n)| *n == s).map(|(b, _)| *b)
}

fn action_key(a: Action) -> &'static str {
    match a {
        Action::Launch => "launch",
        Action::Reset => "reset",
        Action::Camera => "camera",
        Action::ApArm => "ap_arm",
        Action::ApMode => "ap_mode",
        Action::ControlSource => "control_source",
        _ => "",
    }
}

fn parse_config(text: &str, radio: &mut RadioConfig, pad: &mut GamepadConfig) {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        let f = |v: &str| v.parse::<f32>().ok();
        let axis_map = |v: &str| -> Option<AxisMap> {
            let mut it = v.split(',').map(str::trim);
            Some(AxisMap { axis: axis_from_str(it.next()?)?, center: f(it.next()?)?, scale: f(it.next()?)? })
        };
        match k {
            "radio.roll" => { if let Some(m) = axis_map(v) { radio.roll = m; radio.calibrated = true; } }
            "radio.pitch" => { if let Some(m) = axis_map(v) { radio.pitch = m; } }
            "radio.yaw" => { if let Some(m) = axis_map(v) { radio.yaw = m; } }
            "radio.throttle" => {
                let mut it = v.split(',').map(str::trim);
                if let (Some(a), Some(lo), Some(hi)) = (it.next().and_then(axis_from_str), it.next().and_then(f), it.next().and_then(f)) {
                    radio.throttle = ThrottleMap { axis: a, low: lo, high: hi };
                }
            }
            "radio.arm" => radio.arm = axis_map(v),
            "radio.mode" => radio.mode = axis_map(v),
            "radio.zoom" => radio.zoom = axis_map(v),
            "radio.view" => radio.view = axis_map(v),
            "radio.names" => {
                radio.names = v.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
            }
            "gamepad.stick_mode" => pad.stick_mode = v.parse().unwrap_or(2),
            "gamepad.deadzone" => pad.deadzone = f(v).unwrap_or(0.07),
            "gamepad.throttle_rate" => pad.throttle_rate = f(v).unwrap_or(0.7),
            _ => {
                if let Some(name) = k.strip_prefix("radio.button.") {
                    if let (Some((a, _)), Some(b)) = (
                        RADIO_ACTIONS.iter().find(|(a, _)| action_key(*a) == name),
                        button_from_str(v),
                    ) {
                        radio.buttons.insert(*a, b);
                    }
                }
            }
        }
    }
}

/// The config file's text.
fn config_text(radio: &RadioConfig, pad: &GamepadConfig) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# SIM001 controls. Re-run the radio wizard with F2.");
    let _ = writeln!(s, "# axis = <axis>, <center>, <scale>   (scale sign = direction)");
    let am = |m: &AxisMap| format!("{}, {:.4}, {:.4}", axis_to_str(m.axis), m.center, m.scale);
    let _ = writeln!(s, "radio.roll = {}", am(&radio.roll));
    let _ = writeln!(s, "radio.pitch = {}", am(&radio.pitch));
    let _ = writeln!(s, "radio.yaw = {}", am(&radio.yaw));
    let _ = writeln!(
        s,
        "radio.throttle = {}, {:.4}, {:.4}",
        axis_to_str(radio.throttle.axis),
        radio.throttle.low,
        radio.throttle.high
    );
    if let Some(m) = &radio.arm {
        let _ = writeln!(s, "radio.arm = {}", am(m));
    }
    if let Some(m) = &radio.mode {
        let _ = writeln!(s, "radio.mode = {}", am(m));
    }
    if let Some(m) = &radio.zoom {
        let _ = writeln!(s, "radio.zoom = {}", am(m));
    }
    if let Some(m) = &radio.view {
        let _ = writeln!(s, "radio.view = {}", am(m));
    }
    let _ = writeln!(s, "radio.names = {}", radio.names.join(", "));
    for (a, b) in &radio.buttons {
        let _ = writeln!(s, "radio.button.{} = {}", action_key(*a), button_to_str(*b));
    }
    let _ = writeln!(s, "gamepad.stick_mode = {}", pad.stick_mode);
    let _ = writeln!(s, "gamepad.deadzone = {}", pad.deadzone);
    let _ = writeln!(s, "gamepad.throttle_rate = {}", pad.throttle_rate);
    s
}

fn save_config(radio: &RadioConfig, pad: &GamepadConfig) -> std::io::Result<()> {
    std::fs::write(CONFIG_PATH, config_text(radio, pad))
}

/// Is a switch on a channel up? (A three-position switch reads -1, 0 and +1.) Only up arms.
pub fn arm_switch_on(v: f32) -> bool {
    v > 0.5
}

/// A knob's -1 to +1 as 0 (all the way down) to 1.
pub fn knob(v: f32) -> f32 {
    ((v + 1.0) / 2.0).clamp(0.0, 1.0)
}

/// A channel's -1 to +1 as the µs ArduPilot reads.
pub fn channel_pwm(v: f32) -> u16 {
    (1500.0 + 500.0 * v.clamp(-1.0, 1.0)).round() as u16
}

// ---------- systems ----------

/// RC radios need the full, unfiltered axis range: no deadzone, no livezone clipping.
pub fn configure_new_gamepads(mut pads: Query<(&mut GamepadSettings, Option<&Name>), Added<Gamepad>>) {
    for (mut settings, name) in &mut pads {
        if let Ok(a) = AxisSettings::new(-1.0, -0.0, 0.0, 1.0, 0.0005) {
            settings.default_axis_settings = a;
        }
        info!("controller connected: {}", name.map(|n| n.as_str()).unwrap_or("<unnamed>"));
    }
}

fn is_radio_name(cfg: &RadioConfig, name: &str) -> bool {
    let n = name.to_lowercase();
    cfg.names.iter().any(|k| n.contains(k.as_str()))
}

fn smooth_key(cur: f32, neg: bool, pos: bool, dt: f32) -> f32 {
    let target = (pos as i32 - neg as i32) as f32;
    let rate = if target == 0.0 { 6.0 } else { 3.5 };
    cur + (target - cur).clamp(-rate * dt, rate * dt)
}

pub fn read_input(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    pads: Query<(Entity, &Gamepad, Option<&Name>)>,
    gc: Res<GcPads>,
    mut input: ResMut<PilotInput>,
    mut last_debug: Local<String>,
) {
    let dt = time.delta_secs().min(0.1);
    input.actions.clear();
    input.pad_view = None;
    let input = &mut *input;

    // Keyboard actions.
    let key_actions = [
        (KeyCode::Space, Action::Launch),
        (KeyCode::KeyR, Action::Reset),
        (KeyCode::KeyC, Action::Camera),
        (KeyCode::KeyM, Action::Mode),
        (KeyCode::KeyL, Action::Rates),
        (KeyCode::KeyP, Action::Pause),
        (KeyCode::KeyX, Action::DiffThrust),
        (KeyCode::F1, Action::Help),
        (KeyCode::KeyH, Action::Help),
        (KeyCode::F2, Action::Calibrate),
        (KeyCode::F3, Action::RawDebug),
        (KeyCode::F4, Action::ToggleProfile),
        (KeyCode::KeyB, Action::ApArm),
        (KeyCode::KeyN, Action::ApMode),
        (KeyCode::KeyJ, Action::ControlSource),
        (KeyCode::KeyV, Action::Recharge),
        (KeyCode::KeyZ, Action::Zoom),
        (KeyCode::Escape, Action::Dismiss),
        (KeyCode::KeyG, Action::Race),
        (KeyCode::KeyT, Action::Tag),
    ];
    let in_wizard = input.wizard.is_some();
    for (k, a) in key_actions {
        if keys.just_pressed(k) && !(in_wizard && a != Action::Calibrate && a != Action::Dismiss) {
            input.actions.push(a);
        }
    }

    // Keyboard sticks.
    let k = |c: KeyCode| keys.pressed(c);
    input.kb.roll = smooth_key(input.kb.roll, k(KeyCode::KeyA) || k(KeyCode::ArrowLeft), k(KeyCode::KeyD) || k(KeyCode::ArrowRight), dt);
    input.kb.pitch = smooth_key(input.kb.pitch, k(KeyCode::KeyW) || k(KeyCode::ArrowUp), k(KeyCode::KeyS) || k(KeyCode::ArrowDown), dt);
    input.kb.yaw = smooth_key(input.kb.yaw, k(KeyCode::KeyQ), k(KeyCode::KeyE), dt);
    let thr_up = k(KeyCode::ShiftLeft) || k(KeyCode::ShiftRight) || k(KeyCode::Equal);
    let thr_dn = k(KeyCode::ControlLeft) || k(KeyCode::ControlRight) || k(KeyCode::Minus);
    if thr_up {
        input.kb.throttle = (input.kb.throttle + 0.6 * dt).min(1.0);
        input.kb_throttle_used = true;
    }
    if thr_dn {
        input.kb.throttle = (input.kb.throttle - 0.6 * dt).max(0.0);
        input.kb_throttle_used = true;
    }
    if keys.just_pressed(KeyCode::Digit0) || keys.just_pressed(KeyCode::Backquote) {
        input.kb.throttle = 0.0;
    }
    for (i, key) in [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5].iter().enumerate() {
        if keys.just_pressed(*key) {
            input.kb.throttle = 0.2 * (i + 1) as f32;
            input.kb_throttle_used = true;
        }
    }

    // Pick a controller: prefer a radio, else the first gamepad. When the GameController
    // framework sees a gamepad, raw-HID (gilrs) gamepads are ignored: they are the same
    // physical device, and gilrs misreads some of them (Switch Pro on macOS).
    let gc_pad = gc.pad.clone().filter(|_| !input.keyboard_only && input.only_pad.is_none());
    let mut chosen: Option<(Entity, &Gamepad, String, Profile)> = None;
    for (e, pad, name) in pads.iter().filter(|_| !input.keyboard_only).filter(|(e, ..)| input.only_pad.is_none_or(|o| o == *e)) {
        let name = name.map(|n| n.as_str().to_string()).unwrap_or_else(|| "controller".into());
        let profile = input
            .profile_override
            .get(&name)
            .copied()
            .unwrap_or(if is_radio_name(&input.radio, &name) { Profile::Radio } else { Profile::Gamepad });
        if profile == Profile::Gamepad && (gc_pad.is_some() || handled_by_game_controller(&name)) {
            continue;
        }
        let better = match &chosen {
            None => true,
            Some((_, _, _, p)) => *p == Profile::Gamepad && profile == Profile::Radio,
        };
        if better {
            chosen = Some((e, pad, name, profile));
        }
    }

    if keys.just_pressed(KeyCode::F4) {
        if let Some((_, _, name, p)) = &chosen {
            let np = if *p == Profile::Radio { Profile::Gamepad } else { Profile::Radio };
            input.profile_override.insert(name.clone(), np);
            input.notice = format!("{name}: using {np:?} profile");
        }
    }

    // Raw axis readout (F3) — handy for checking what the radio actually sends.
    if let Some((_, pad, name, _)) = &chosen {
        let mut t = format!("{name}\n");
        let mut axes: Vec<_> = pad
            .get_analog_axes()
            .filter_map(|i| match i {
                GamepadInput::Axis(a) => Some(*a),
                _ => None,
            })
            .collect();
        axes.sort_by_key(|a| axis_to_str(*a));
        for a in axes {
            let _ = writeln!(t, "  {:<16} {:+.3}", axis_to_str(a), pad.get_unclamped(a).unwrap_or(0.0));
        }
        let pressed: Vec<String> = pad.get_pressed().map(|b| button_to_str(*b)).collect();
        let _ = write!(t, "  buttons: {}", pressed.join(" "));
        input.raw_text = t;
    } else {
        input.raw_text = "no controller connected".into();
    }

    // Calibration wizard.
    if input.calibrate_on_start && chosen.is_some() {
        input.calibrate_on_start = false;
        input.actions.push(Action::Calibrate);
    }
    if input.has(Action::Calibrate) || (input.has(Action::Dismiss) && input.wizard.is_some()) {
        if input.wizard.is_some() {
            input.wizard = None;
            input.notice = "Radio calibration cancelled".into();
        } else if let Some((_, _, name, _)) = &chosen {
            let mut result = input.radio.clone();
            let lname = name.to_lowercase();
            if !result.names.iter().any(|n| lname.contains(n.as_str())) {
                result.names.push(lname);
            }
            result.buttons.clear();
            input.wizard = Some(Wizard {
                step: WizardStep::Baseline,
                baseline: HashMap::new(),
                hold: 0.0,
                candidate: None,
                waiting_for_center: false,
                result,
                message: String::new(),
            });
        } else {
            input.notice = "Connect a radio (USB joystick mode) or gamepad first".into();
        }
    }
    if let (Some(w), Some((_, pad, name, _))) = (input.wizard.as_mut(), &chosen) {
        if let Some(done) = run_wizard(w, pad, &keys, dt) {
            input.radio = done;
            input.profile_override.insert(name.clone(), Profile::Radio);
            input.notice = match save_config(&input.radio, &input.gamepad) {
                Ok(()) => format!("Radio calibrated and saved to {CONFIG_PATH}"),
                Err(e) => format!("Radio calibrated (could not save: {e})"),
            };
            input.wizard = None;
        }
    }

    // Controller sticks and buttons.
    let mut sticks = input.kb;
    input.radio_arm = None;
    input.radio_mode = None;
    input.radio_zoom = None;
    input.radio_view = None;
    input.source = "keyboard".into();
    input.profile = None;
    input.active_pad = None;
    if let Some((e, pad, name, profile)) = chosen {
        input.active_pad = Some(e);
        input.profile = Some(profile);
        input.source = name;
        if input.wizard.is_none() {
            match profile {
                Profile::Radio => {
                    let r = &input.radio;
                    let t = &r.throttle;
                    let raw_t = pad.get_unclamped(t.axis).unwrap_or(t.low);
                    let thr = ((raw_t - t.low) / (t.high - t.low)).clamp(0.0, 1.0);
                    sticks.roll = (r.roll.read(pad) + input.kb.roll).clamp(-1.0, 1.0);
                    // Radio stick forward = nose down.
                    sticks.pitch = (-r.pitch.read(pad) + input.kb.pitch).clamp(-1.0, 1.0);
                    sticks.yaw = (r.yaw.read(pad) + input.kb.yaw).clamp(-1.0, 1.0);
                    sticks.throttle = if input.kb_throttle_used && thr < 0.02 { input.kb.throttle } else { thr };
                    let arm = r.arm.map(|m| arm_switch_on(m.read(pad)));
                    let mode = r.mode.map(|m| channel_pwm(m.read(pad)));
                    for (a, b) in &r.buttons {
                        if pad.just_pressed(*b) {
                            input.actions.push(*a);
                        }
                    }
                    input.radio_arm = arm;
                    input.radio_mode = mode;
                    input.radio_zoom = r.zoom.map(|m| knob(m.read(pad)));
                    input.radio_view = r.view.map(|m| knob(m.read(pad)));
                }
                Profile::Gamepad => {
                    let snap = PadSnapshot {
                        name: input.source.clone(),
                        left: pad.left_stick(),
                        right: pad.right_stick(),
                        pressed: pad.get_pressed().copied().collect(),
                        just_pressed: pad.get_just_pressed().copied().collect(),
                    };
                    apply_gamepad(input, &snap, dt, &mut sticks);
                }
            }
        }
    } else if let Some(snap) = &gc_pad {
        input.profile = Some(Profile::Gamepad);
        input.source = snap.name.clone();
        input.raw_text = format!(
            "{}\n  left {:+.3} {:+.3}  right {:+.3} {:+.3}\n  buttons: {}",
            snap.name,
            snap.left.x,
            snap.left.y,
            snap.right.x,
            snap.right.y,
            snap.pressed.iter().map(|b| button_to_str(*b)).collect::<Vec<_>>().join(" ")
        );
        if input.wizard.is_none() {
            apply_gamepad(input, snap, dt, &mut sticks);
        }
    }
    if gc.debug {
        let s = sticks;
        let key = format!("{:?} {} thr {:.2} roll {:+.2} pitch {:+.2} yaw {:+.2}", input.profile, input.source, s.throttle, s.roll, s.pitch, s.yaw);
        if *last_debug != key {
            info!("sim input: {key}");
            *last_debug = key;
        }
    }
    input.sticks = sticks;
    if input.has(Action::RawDebug) {
        input.show_raw = !input.show_raw;
    }
}

/// Advances the radio wizard. Returns the finished config when complete.
fn run_wizard(w: &mut Wizard, pad: &Gamepad, keys: &ButtonInput<KeyCode>, dt: f32) -> Option<RadioConfig> {
    let axes: Vec<GamepadAxis> = pad
        .get_analog_axes()
        .filter_map(|i| match i {
            GamepadInput::Axis(a) => Some(*a),
            _ => None,
        })
        .collect();
    let value = |a: GamepadAxis| pad.get_unclamped(a).unwrap_or(0.0);

    // Find the axis that moved furthest from its baseline.
    let biggest = || -> Option<(GamepadAxis, f32, f32)> {
        axes.iter()
            .map(|a| {
                let base = *w.baseline.get(a).unwrap_or(&0.0);
                (*a, value(*a), value(*a) - base)
            })
            .max_by(|x, y| x.2.abs().total_cmp(&y.2.abs()))
    };

    if w.waiting_for_center {
        w.message = "Return the stick to centre (throttle low)...".into();
        let still_moved = axes.iter().any(|a| (value(*a) - *w.baseline.get(a).unwrap_or(&0.0)).abs() > 0.25);
        if !still_moved {
            w.waiting_for_center = false;
            w.hold = 0.0;
            w.candidate = None;
        }
        return None;
    }

    match w.step.clone() {
        WizardStep::Baseline => {
            w.message = "RADIO SETUP: put the radio in USB Joystick mode.\nCentre roll/pitch/yaw, throttle fully LOW, then press ENTER.".into();
            if keys.just_pressed(KeyCode::Enter) {
                w.baseline = axes.iter().map(|a| (*a, value(*a))).collect();
                w.step = WizardStep::Axis { which: "roll", prompt: "Move the AILERON stick fully RIGHT and hold" };
            }
        }
        WizardStep::Axis { which, prompt } => {
            w.message = format!("{prompt}");
            if let Some((axis, raw, delta)) = biggest() {
                if delta.abs() > 0.5 {
                    match w.candidate {
                        Some((a, _)) if a == axis => w.hold += dt,
                        _ => {
                            w.candidate = Some((axis, raw));
                            w.hold = 0.0;
                        }
                    }
                    if let Some((_, ref mut best)) = w.candidate {
                        if (raw - w.baseline[&axis]).abs() > (*best - w.baseline[&axis]).abs() {
                            *best = raw;
                        }
                    }
                    if w.hold > 0.4 {
                        let center = w.baseline[&axis];
                        let extreme = w.candidate.unwrap().1;
                        let m = AxisMap { axis, center, scale: 1.0 / (extreme - center) };
                        let next = match which {
                            "roll" => {
                                w.result.roll = m;
                                WizardStep::Axis { which: "pitch", prompt: "Push the ELEVATOR stick fully UP (forward) and hold" }
                            }
                            "pitch" => {
                                w.result.pitch = m;
                                WizardStep::Axis { which: "yaw", prompt: "Move the RUDDER stick fully RIGHT and hold" }
                            }
                            _ => {
                                w.result.yaw = m;
                                WizardStep::Throttle
                            }
                        };
                        w.step = next;
                        w.waiting_for_center = true;
                    }
                } else {
                    w.hold = 0.0;
                    w.candidate = None;
                }
            }
        }
        WizardStep::Throttle => {
            w.message = "Push the THROTTLE fully UP and hold".into();
            if let Some((axis, raw, delta)) = biggest() {
                if delta.abs() > 0.5 {
                    w.hold += dt;
                    if w.hold > 0.5 {
                        w.result.throttle = ThrottleMap { axis, low: w.baseline[&axis], high: raw };
                        // Throttle stays up; treat its new position as the baseline.
                        w.baseline.insert(axis, raw);
                        w.step = WizardStep::Button(0);
                        w.hold = 0.0;
                    }
                } else {
                    w.hold = 0.0;
                }
            }
        }
        WizardStep::Button(i) => {
            if i >= RADIO_ACTIONS.len() {
                w.step = WizardStep::Done;
                return None;
            }
            w.message = format!(
                "Optional: flip a switch / press a button for {}\n(ENTER to skip). Throttle can go back down.",
                RADIO_ACTIONS[i].1
            );
            if keys.just_pressed(KeyCode::Enter) {
                w.step = WizardStep::Button(i + 1);
            } else if let Some(b) = pad.get_just_pressed().next() {
                w.result.buttons.insert(RADIO_ACTIONS[i].0, *b);
                w.step = WizardStep::Button(i + 1);
            }
        }
        WizardStep::Done => {
            let mut r = w.result.clone();
            r.calibrated = true;
            return Some(r);
        }
    }
    None
}

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PilotInput::load())
            .add_systems(PreUpdate, (configure_new_gamepads, crate::gc_pad::activate_app, crate::gc_pad::poll).chain())
            .add_systems(Update, crate::gc_pad::log_focus)
            .add_systems(Update, read_input.in_set(crate::SimSet::Input));
    }
}

/// Gamepad (spring-centred sticks) handling, shared by gilrs and GameController pads.
fn apply_gamepad(input: &mut PilotInput, pad: &PadSnapshot, dt: f32, sticks: &mut Sticks) {
    input.pad_view = Some(pad.clone());
    let g = &input.gamepad;
    let dz = |v: f32| if v.abs() < g.deadzone { 0.0 } else { (v - g.deadzone * v.signum()) / (1.0 - g.deadzone) };
    let (l, r) = (pad.left, pad.right);
    let (yaw, thr_axis, roll, pitch_up) = if g.stick_mode == 1 {
        (dz(l.x), dz(r.y), dz(r.x), dz(l.y))
    } else {
        (dz(l.x), dz(l.y), dz(r.x), dz(r.y))
    };
    let pressed = |b: GamepadButton| pad.pressed.contains(&b);
    // Throttle stick is spring-centred, so it nudges the throttle up/down.
    let mut thr = input.kb.throttle;
    thr = (thr + thr_axis * g.throttle_rate * dt).clamp(0.0, 1.0);
    if pressed(GamepadButton::LeftTrigger2) {
        thr = 0.0;
    }
    if pressed(GamepadButton::RightTrigger2) {
        thr = (thr + 1.5 * dt).min(1.0);
    }
    input.kb.throttle = thr;
    sticks.roll = (roll + input.kb.roll).clamp(-1.0, 1.0);
    // Stick pushed forward (up) = nose down.
    sticks.pitch = (-pitch_up + input.kb.pitch).clamp(-1.0, 1.0);
    sticks.yaw = (yaw + input.kb.yaw).clamp(-1.0, 1.0);
    sticks.throttle = thr;
    let map = [
        (GamepadButton::South, Action::Launch),
        (GamepadButton::Start, Action::Reset),
        (GamepadButton::East, Action::Camera),
        (GamepadButton::North, Action::Mode),
        (GamepadButton::West, Action::Rates),
        (GamepadButton::Select, Action::Pause),
        (GamepadButton::LeftTrigger, Action::ApArm),
        (GamepadButton::RightTrigger, Action::ApMode),
        (GamepadButton::DPadUp, Action::Zoom),
        (GamepadButton::DPadDown, Action::ControlSource),
        (GamepadButton::Mode, Action::Help),
    ];
    for (b, a) in map {
        if pad.just_pressed.contains(&b) {
            input.actions.push(a);
        }
    }
}

/// Controller families the macOS GameController framework supports natively. Their raw HID
/// reports are not reliable through gilrs, so they are only read via `gc_pad` on macOS.
fn handled_by_game_controller(name: &str) -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    let n = name.to_lowercase();
    ["pro controller", "nintendo", "joy-con", "xbox", "dualsense", "dualshock", "wireless controller"]
        .iter()
        .any(|k| n.contains(k))
}

#[cfg(test)]
mod switch_tests {
    use super::*;

    /// A three-position switch on a channel reads -1 (down), 0 (middle) and +1 (up). Up arms; the
    /// middle and down do not.
    #[test]
    fn an_arm_switch_is_on_only_when_it_is_up() {
        assert!(arm_switch_on(1.0));
        assert!(arm_switch_on(0.8));
        assert!(!arm_switch_on(0.0), "the middle disarms");
        assert!(!arm_switch_on(-1.0));
    }

    /// The mode channel reaches ArduPilot in µs: -1 is 1000, the middle 1500 and +1 is 2000.
    #[test]
    fn a_mode_channel_becomes_microseconds() {
        assert_eq!((channel_pwm(-1.0), channel_pwm(0.0), channel_pwm(1.0)), (1000, 1500, 2000));
        assert_eq!(channel_pwm(5.0), 2000, "clamped");
        assert_eq!(channel_pwm(-0.34), 1330);
    }

    /// The switches the pilot gave are in the config file, and read back.
    #[test]
    fn the_switches_are_saved_and_read_back() {
        let mut radio = RadioConfig::default();
        let mut pad = GamepadConfig::default();
        assert!(radio.arm.is_none() && radio.mode.is_none());
        parse_config("radio.arm = Other(65589), 0.0, 1.0\nradio.mode = Other(65590), 0.0, 1.0\n", &mut radio, &mut pad);
        let arm = radio.arm.expect("arm axis");
        assert_eq!((arm.center, arm.scale), (0.0, 1.0));
        assert!(radio.mode.is_some());
        let text = config_text(&radio, &pad);
        let mut again = RadioConfig::default();
        parse_config(&text, &mut again, &mut GamepadConfig::default());
        assert_eq!(again.arm, radio.arm);
        assert_eq!(again.mode, radio.mode);
    }

    /// The two knobs (zoom and camera view) are saved and read back too.
    #[test]
    fn the_knobs_are_saved_and_read_back() {
        let mut radio = RadioConfig::default();
        assert!(radio.zoom.is_none() && radio.view.is_none());
        parse_config("radio.zoom = Other(65594), 0.0, 1.0\nradio.view = Other(65595), 0.0, -1.0\n", &mut radio, &mut GamepadConfig::default());
        assert!(radio.zoom.is_some());
        assert_eq!(radio.view.expect("view").scale, -1.0);
        let text = config_text(&radio, &GamepadConfig::default());
        let mut again = RadioConfig::default();
        parse_config(&text, &mut again, &mut GamepadConfig::default());
        assert_eq!((again.zoom, again.view), (radio.zoom, radio.view));
        // A knob reads -1 to +1, and the camera wants 0 to 1.
        assert_eq!((knob(-1.0), knob(0.0), knob(1.0), knob(3.0)), (0.0, 0.5, 1.0, 1.0));
    }

    /// The example file in the repository is a valid config, with the switches in it.
    #[test]
    fn the_example_controls_file_parses() {
        let mut radio = RadioConfig::default();
        let mut pad = GamepadConfig::default();
        parse_config(include_str!("../../../../controls.cfg.example"), &mut radio, &mut pad);
        assert!(radio.calibrated, "the sticks are there");
        assert_eq!(radio.throttle.low, -1.0);
        assert!(radio.arm.is_some() && radio.mode.is_some(), "SE and the six buttons");
        assert!(radio.zoom.is_some() && radio.view.is_some(), "the two knobs");
        assert!(radio.names.iter().any(|n| n == "tx15"));
        assert_eq!(pad.stick_mode, 2);
    }
}
