//! On-screen display: telemetry, warnings, stick gauges, help and controller debug.

use crate::theme::{P, alpha, color, geometry};
use crate::camera::CamState;
use crate::input::{Action, PilotInput, Profile};
use simulation_physics::{self as physics, Phase};
use crate::sim::wrap_deg;
use crate::terrain;
use crate::SharedSim;
use visualization_engine::RenderSettings;
use bevy::prelude::*;

#[derive(Component)]
pub struct OsdText;
#[derive(Component)]
pub struct CenterText;
#[derive(Component)]
pub struct HelpText;
#[derive(Component)]
pub struct RawText;
#[derive(Component)]
pub struct NoticeText;
#[derive(Component)]
pub struct StickDot(pub u8);

#[derive(Resource, Default)]
pub struct HudState {
    pub show_help: bool,
    notice: String,
    notice_timer: f32,
}

const HELP: &str = "\
CONTROLS (keyboard)
  W/S or Up/Down   pitch (S = nose up)
  A/D or Left/Right roll      Q/E  rudder
  Shift / Ctrl     throttle up / down   1-5 = 20..100%   0 = cut
  Space            hand launch (or hold throttle >55%)
  R                reset to hand        V  fresh battery
  M                flight mode (MANUAL / ANGLE / ACRO)
  L                low/high rates       X  differential thrust
  C                camera (LOS / chase / FPV)   Z  LOS auto-zoom
  G                pylon race   T  tag   (the same key again stops the game)
  [ ]  wind speed   , .  wind direction   /  turbulence
  F5 / F6 render resolution down / up (F7: automatic)
  P pause   F2 radio calibration   F3 raw controller axes   F4 radio/gamepad profile
ARDUPILOT MODE
  B  arm switch (RC7)   N  next mode on RC8 (MANUAL FBWA CRUISE AUTO RTL LOITER)
  J  control: ArduPilot / direct (sticks + sim modes, M cycles them)
SWITCH PRO CONTROLLER (mode 2)
  Left stick: rudder + throttle up/down   Right stick: roll + pitch
  ZL cut  ZR throttle up  B launch  + reset  A camera  X mode  Y rates
  L AP arm  R AP mode  - pause  D-up zoom  HOME controller diagram
Esc closes help / overlays / calibration. H / F1 toggles this help";

pub fn spawn_hud(commands: &mut Commands) {
    let font = |s: f32| TextFont { font_size: FontSize::Px(s), ..default() };
    // Screens over the 3D view, in the console's colours (see crate::theme).
    let panel = BackgroundColor(alpha(P.screen, 0.62));
    let edge = BorderColor::all(alpha(P.bezel_edge, 0.9));

    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(10.0),
            left: px(10.0),
            padding: px(8.0).all(),
            border: px(1.0).all(),
            border_radius: BorderRadius::all(px(geometry::SCREEN_RADIUS)),
            ..default()
        },
        panel,
        edge,
        children![(OsdText, Text::default(), font(14.0), TextColor(color(P.text)))],
    ));
    commands.spawn((
        CenterText,
        Text::default(),
        font(26.0),
        TextColor(color(P.caution)),
        TextLayout::justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            top: percent(30.0),
            left: percent(0.0),
            right: percent(0.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
    ));
    commands.spawn((
        NoticeText,
        Text::default(),
        font(16.0),
        TextColor(color(P.info)),
        Node { position_type: PositionType::Absolute, bottom: px(12.0), left: px(12.0), ..default() },
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: px(40.0),
            right: px(10.0),
            padding: px(8.0).all(),
            border: px(1.0).all(),
            border_radius: BorderRadius::all(px(geometry::SCREEN_RADIUS)),
            ..default()
        },
        panel,
        edge,
        HelpText,
        Visibility::Hidden,
        children![(Text::new(HELP), font(12.0), TextColor(color(P.text)))],
    ));
    commands.spawn((
        RawText,
        Text::default(),
        font(13.0),
        TextColor(color(P.text)),
        panel,
        Node { position_type: PositionType::Absolute, top: px(150.0), right: px(10.0), padding: px(6.0).all(), ..default() },
        Visibility::Hidden,
    ));

    // Stick gauges (transmitter view, mode 2: left = yaw/throttle, right = roll/pitch).
    for (i, right) in [(0u8, 120.0), (1u8, 10.0)] {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(10.0),
                right: px(right),
                width: px(100.0),
                height: px(100.0),
                border: px(2.0).all(),
                border_radius: BorderRadius::all(px(8.0)),
                ..default()
            },
            BorderColor::all(color(P.bezel_edge)),
            panel,
            children![(
                StickDot(i),
                Node {
                    position_type: PositionType::Absolute,
                    width: px(12.0),
                    height: px(12.0),
                    left: px(42.0),
                    top: px(42.0),
                    border_radius: BorderRadius::all(px(6.0)),
                    ..default()
                },
                BackgroundColor(color(P.caution)),
            )],
        ));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_hud(
    time: Res<Time>,
    sim: Res<SharedSim>,
    input: Res<PilotInput>,
    cam: Res<CamState>,
    settings: Res<RenderSettings>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    render_scale: Res<crate::camera::RenderScale>,
    mut hud: ResMut<HudState>,
    mut texts: ParamSet<(
        Single<&mut Text, With<OsdText>>,
        Single<&mut Text, With<CenterText>>,
        Single<&mut Text, With<NoticeText>>,
        Single<(&mut Text, &mut Visibility), With<RawText>>,
    )>,
    mut help: Single<&mut Visibility, (With<HelpText>, Without<RawText>)>,
    mut dots: Query<(&StickDot, &mut Node)>,
) {
    // With a gamepad or a radio active, HOME/H toggles the controller diagram instead.
    if input.has(Action::Help) && input.pad_view.is_none() && input.profile != Some(crate::input::Profile::Radio) {
        hud.show_help = !hud.show_help;
    }
    if input.has(Action::Dismiss) {
        hud.show_help = false;
    }
    **help = if hud.show_help { Visibility::Visible } else { Visibility::Hidden };

    let c = sim.0.lock().unwrap();
    let s = &c.state;
    let ground = terrain::ground_height(s.pos.x, s.pos.z);
    let agl = s.pos.y - ground;
    let (roll, pitch, heading) = physics::euler(s.rot);
    let gs = Vec3::new(s.vel.x, 0.0, s.vel.z).length();
    let dist = Vec3::new(s.pos.x - terrain::pilot_pos().x, 0.0, s.pos.z - terrain::pilot_pos().z).length();
    let soc = (1.0 - s.used_mah / c.params.battery_capacity_mah).max(0.0);
    let cell = s.voltage / c.params.battery_cells;
    let thr = if c.ap.enabled { c.last_cmd.forward_throttle(&c.params) } else { c.sticks.throttle };

    let mode_line = if c.ap.enabled && c.direct {
        format!(
            "DIRECT (J) | {} | {} rates | ArduPilot {} (watching only)",
            c.mode.label(),
            if c.high_rates { "high" } else { "low" },
            if c.ap.connected { "linked" } else { "waiting" }
        )
    } else if c.ap.enabled {
        // The real armed state and mode come from the ground station when it's connected;
        // the switches only say what the sticks are asking for.
        let actual = match c.ap.vehicle() {
            Some((armed, mode)) => format!("{mode} {} | ", if armed { "ARMED" } else { "disarmed" }),
            None => String::new(),
        };
        format!(
            "ARDUPILOT {} | {actual}mode sw (RC8): {} | {}",
            if c.ap.connected { "LINKED" } else { "WAITING" },
            c.mode_switch_name(),
            if c.arm_switch_is_on() { "ARM SW ON" } else { "arm sw off" }
        )
    } else {
        format!(
            "{} | {} rates | diff thrust {}",
            c.mode.label(),
            if c.high_rates { "high" } else { "low" },
            if c.diff_thrust { "on" } else { "off" }
        )
    };
    let input_line = match input.profile {
        Some(Profile::Radio) => format!("Radio: {}{}", input.source, if input.radio.calibrated { "" } else { " (uncalibrated: F2)" }),
        Some(Profile::Gamepad) => format!("Gamepad: {}", input.source),
        None => "Keyboard (connect TX15/gamepad any time)".into(),
    };
    let osd = format!(
        "{mode_line}\n{input_line}\nCAM {}{} | {}\n\
THR {:3.0}%   AS {:4.1} m/s   GS {:4.1} m/s\n\
ALT {:5.1} m  VS {:+4.1} m/s  DIST {:4.0} m\n\
ATT roll {:+4.0} deg  pitch {:+4.0} deg  hdg {:03.0} deg\n\
BAT {:5.2} V ({:.2} V/cell)  {:4.1} A  {:4.0} mAh  {:3.0}%\n\
WIND {:.1} m/s from {:03.0} deg  turb {:.0}%{}",
        cam.mode.label(),
        if cam.auto_zoom && cam.mode == crate::camera::CamMode::LineOfSight { " (auto-zoom)" } else { "" },
        format!(
            "{} {:.0}%{} | {:.0} fps",
            if settings.raytracing { "RAY TRACED (Solari)" } else { "raster" },
            render_scale.scale * 100.0,
            if render_scale.auto { " auto" } else { "" },
            diagnostics
                .get(&bevy::diagnostic::FrameTimeDiagnosticsPlugin::FPS)
                .and_then(|d| d.smoothed())
                .unwrap_or(0.0)
        ),
        thr * 100.0,
        s.airspeed,
        gs,
        agl,
        s.vel.y,
        dist,
        roll.to_degrees(),
        pitch.to_degrees(),
        wrap_deg(heading),
        s.voltage,
        cell,
        s.current,
        s.used_mah,
        soc * 100.0,
        c.wind.speed,
        wrap_deg(c.wind.from_dir),
        c.wind.turbulence * 100.0,
        if c.ap.enabled { format!("\nSITL: {} | frames {}", c.ap.status, c.ap.packets) } else { String::new() },
    );
    **texts.p0() = Text::new(osd);

    let mut center = String::new();
    let game_says = c.game.as_ref().and_then(|g| crate::games::centre_text(g, s.phase == Phase::Held));
    if let Some(w) = &input.wizard {
        center = w.message.clone();
    } else if c.paused {
        center = "PAUSED (P)".into();
    } else if let Some(said) = game_says {
        center = said;
    } else {
        match s.phase {
            Phase::Crashed => center = format!("CRASHED: {}\nPress R to reset", s.crash_reason),
            Phase::Held => {
                center = if c.ap.enabled && !c.ap.connected {
                    format!("Waiting for ArduPilot SITL on UDP {}...\n(sim_vehicle.py -v ArduPlane -f JSON)", c.ap.port)
                } else {
                    let prog = c.launch_progress();
                    // With ArduPilot the throw comes from its motor output, not the stick, and
                    // it won't arm with the throttle up.
                    let motor = c.last_cmd.forward_throttle(&c.params);
                    let stick_up = c.sticks.throttle > 0.1;
                    if s.carry.is_some() {
                        "Bringing it back to your hand...".into()
                    } else if prog > 0.0 {
                        format!("Launching... {}", "#".repeat((prog * 10.0) as usize))
                    } else if c.ap.enabled && !c.direct && motor < 0.15 {
                        // Armed comes from the ground station if it's reporting, else the switch.
                        let (armed, mode) = c.ap.vehicle().map_or((c.ap.arm_switch, None), |(a, m)| (a, Some(m)));
                        let waits_for_throw = matches!(mode, Some("AUTO" | "TAKEOFF"));
                        if !armed && stick_up {
                            "ArduPilot is DISARMED: throttle to zero, then L (or B, or ARM in the GCS)".into()
                        } else if !armed {
                            "Arm ArduPilot: L (or B, or ARM in the GCS), then throttle up to launch".into()
                        } else if waits_for_throw {
                            format!("{} takeoff: throw it (SPACE or B), the motors start after the throw", mode.unwrap_or("AUTO"))
                        } else if stick_up && mode.is_none() {
                            "No motor from ArduPilot. Arming is refused with throttle up:\nthrottle to zero, flip L off and on. (AUTO / TAKEOFF wait for a throw: SPACE)".into()
                        } else {
                            "Hold throttle above 55% (or press SPACE) to hand-launch".into()
                        }
                    } else {
                        "Hold throttle above 55% (or press SPACE) to hand-launch".into()
                    }
                };
            }
            _ => {
                let stalled = s.stalled.iter().take(4).filter(|&&x| x).count();
                if stalled >= 2 && s.airspeed > 1.0 {
                    center = "STALL".into();
                } else if stalled == 1 && s.airspeed > 1.0 {
                    center = "TIP STALL".into();
                } else if cell < 3.45 && s.current > 1.0 {
                    center = "LOW BATTERY".into();
                } else if agl > 120.0 {
                    center = "ABOVE 120 m".into();
                }
            }
        }
    }
    **texts.p1() = Text::new(center);

    if !input.notice.is_empty() && input.notice != hud.notice {
        hud.notice = input.notice.clone();
        hud.notice_timer = 5.0;
    }
    hud.notice_timer -= time.delta_secs();
    let notice = if hud.notice_timer > 0.0 { hud.notice.clone() } else { String::new() };
    **texts.p2() = Text::new(notice);

    {
        let mut raw = texts.p3();
        let (text, vis) = &mut *raw;
        **text = Text::new(input.raw_text.clone());
        **vis = if input.show_raw { Visibility::Visible } else { Visibility::Hidden };
    }

    // Stick gauges (always show transmitter mode 2 layout).
    let st = c.sticks;
    for (d, mut node) in &mut dots {
        let (x, y) = if d.0 == 0 { (st.yaw, st.throttle * 2.0 - 1.0) } else { (st.roll, -st.pitch) };
        node.left = px(42.0 + x.clamp(-1.0, 1.0) * 42.0);
        node.top = px(42.0 - y.clamp(-1.0, 1.0) * 42.0);
    }
}
