//! On-screen Switch Pro Controller diagram: what every button and stick does, with live
//! button highlights, stick positions and current mode/rates/camera state.
//!
//! Shown automatically when a gamepad becomes the active controller, hidden on launch,
//! toggled with HOME (or H / F1).

use crate::theme::{P, alpha, color};
use crate::camera::CamState;
use crate::input::{Action, PilotInput, Profile};
use simulation_physics::Phase;
use crate::SharedSim;
use bevy::input::gamepad::GamepadButton;
use bevy::prelude::*;

const W: f32 = 900.0;
const H: f32 = 400.0;
const LEFT_COL: f32 = 235.0; // right edge of the left label column
const RIGHT_COL: f32 = 665.0; // left edge of the right label column

// The console's colours (see crate::theme): the pad as a bezel, pressed buttons lit amber.
const BODY: Color = alpha(P.bezel, 0.95);
const PART: Color = color(P.bezel_edge);
const LIT: Color = color(P.caution);
const LINE: Color = alpha(P.muted, 0.5);
const TEXT: Color = color(P.text);

#[derive(Component)]
pub struct PadOverlay;

#[derive(Component)]
pub struct PadHint;

/// A diagram part that lights up while its button is held.
#[derive(Component)]
pub struct ButtonVis(GamepadButton);

/// Stick knob that follows the stick (0 = left, 1 = right).
#[derive(Component)]
pub struct StickKnob(u8);

#[derive(Component, Clone, Copy)]
pub enum Label {
    ZL,
    L,
    Minus,
    LeftStick,
    DpadUp,
    ZR,
    R,
    Plus,
    X,
    A,
    B,
    Y,
    RightStick,
    Home,
}

#[derive(Resource, Default)]
pub struct PadOverlayState {
    pub visible: bool,
    was_gamepad: bool,
    last_phase: Option<Phase>,
}

fn abs_node(left: f32, top: f32, w: f32, h: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: px(left),
        top: px(top),
        width: px(w),
        height: px(h),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    }
}

fn round(r: f32) -> BorderRadius {
    BorderRadius::all(px(r))
}

pub fn spawn_pad_overlay(commands: &mut Commands) {
    let small = TextFont { font_size: FontSize::Px(13.0), ..default() };
    let glyph = TextFont { font_size: FontSize::Px(12.0), ..default() };

    let root = commands
        .spawn((
            PadOverlay,
            Node {
                position_type: PositionType::Absolute,
                bottom: px(16.0),
                left: percent(50.0),
                margin: UiRect::left(px(-W / 2.0)),
                width: px(W),
                height: px(H),
                border_radius: round(14.0),
                ..default()
            },
            BackgroundColor(alpha(P.screen, 0.6)),
            Visibility::Hidden,
        ))
        .id();

    let mut parts: Vec<Entity> = Vec::new();
    let part = |commands: &mut Commands, node: Node, color: Color, r: f32, extra: Option<GamepadButton>, text: Option<&str>| -> Entity {
        let mut e = commands.spawn((Node { border_radius: round(r), ..node }, BackgroundColor(color)));
        if let Some(b) = extra {
            e.insert(ButtonVis(b));
        }
        if let Some(t) = text {
            e.with_child((Text::new(t), glyph.clone(), TextColor(TEXT)));
        }
        e.id()
    };

    // Shoulders (drawn first so the body overlaps them).
    parts.push(part(commands, abs_node(282.0, 46.0, 96.0, 34.0), PART, 12.0, Some(GamepadButton::LeftTrigger2), Some("ZL")));
    parts.push(part(commands, abs_node(522.0, 46.0, 96.0, 34.0), PART, 12.0, Some(GamepadButton::RightTrigger2), Some("ZR")));
    parts.push(part(commands, abs_node(272.0, 78.0, 116.0, 36.0), PART, 14.0, Some(GamepadButton::LeftTrigger), Some("L")));
    parts.push(part(commands, abs_node(512.0, 78.0, 116.0, 36.0), PART, 14.0, Some(GamepadButton::RightTrigger), Some("R")));
    // Grips and body.
    parts.push(part(commands, abs_node(255.0, 190.0, 130.0, 190.0), BODY, 64.0, None, None));
    parts.push(part(commands, abs_node(515.0, 190.0, 130.0, 190.0), BODY, 64.0, None, None));
    parts.push(part(commands, abs_node(250.0, 104.0, 400.0, 180.0), BODY, 70.0, None, None));

    // Left stick, D-pad, right stick (stick wells with moving knobs).
    for (i, (cx, cy)) in [(330.0, 175.0), (500.0, 250.0)].into_iter().enumerate() {
        let well = commands
            .spawn((Node { border_radius: round(34.0), ..abs_node(cx - 34.0, cy - 34.0, 68.0, 68.0) }, BackgroundColor(PART)))
            .id();
        let knob = commands
            .spawn((
                StickKnob(i as u8),
                Node { border_radius: round(14.0), ..abs_node(20.0, 20.0, 28.0, 28.0) },
                BackgroundColor(color(P.muted)),
            ))
            .id();
        commands.entity(well).add_child(knob);
        parts.push(well);
    }
    let (dx, dy) = (400.0, 250.0);
    for (b, ox, oy) in [
        (GamepadButton::DPadUp, 0.0, -20.0),
        (GamepadButton::DPadDown, 0.0, 20.0),
        (GamepadButton::DPadLeft, -20.0, 0.0),
        (GamepadButton::DPadRight, 20.0, 0.0),
    ] {
        parts.push(part(commands, abs_node(dx + ox - 10.0, dy + oy - 10.0, 20.0, 20.0), PART, 4.0, Some(b), None));
    }

    // Face buttons: Nintendo letters at their physical positions.
    let (fx, fy) = (572.0, 178.0);
    for (b, ox, oy, letter) in [
        (GamepadButton::North, 0.0, -32.0, "X"),
        (GamepadButton::East, 32.0, 0.0, "A"),
        (GamepadButton::South, 0.0, 32.0, "B"),
        (GamepadButton::West, -32.0, 0.0, "Y"),
    ] {
        parts.push(part(commands, abs_node(fx + ox - 15.0, fy + oy - 15.0, 30.0, 30.0), PART, 15.0, Some(b), Some(letter)));
    }
    // Minus, plus, home, capture.
    parts.push(part(commands, abs_node(392.0, 126.0, 22.0, 22.0), PART, 11.0, Some(GamepadButton::Select), Some("-")));
    parts.push(part(commands, abs_node(486.0, 126.0, 22.0, 22.0), PART, 11.0, Some(GamepadButton::Start), Some("+")));
    parts.push(part(commands, abs_node(464.0, 158.0, 22.0, 22.0), PART, 11.0, Some(GamepadButton::Mode), Some("H")));
    parts.push(part(commands, abs_node(414.0, 158.0, 20.0, 20.0), PART, 3.0, None, None));

    // Callouts: (label, label y, element x, element y, right column?)
    let callouts: [(Label, f32, f32, f32, bool); 13] = [
        (Label::ZL, 52.0, 282.0, 63.0, false),
        (Label::L, 92.0, 272.0, 96.0, false),
        (Label::Minus, 130.0, 392.0, 137.0, false),
        (Label::LeftStick, 175.0, 296.0, 175.0, false),
        (Label::DpadUp, 300.0, 400.0, 260.0, false),
        (Label::ZR, 52.0, 618.0, 63.0, true),
        (Label::R, 92.0, 628.0, 96.0, true),
        (Label::Plus, 122.0, 508.0, 137.0, true),
        (Label::X, 146.0, 587.0, 146.0, true),
        (Label::A, 178.0, 619.0, 178.0, true),
        (Label::B, 216.0, 587.0, 210.0, true),
        (Label::Y, 252.0, 540.0, 193.0, true),
        (Label::RightStick, 300.0, 500.0, 284.0, true),
    ];
    for (label, ly, ex, ey, right) in callouts {
        // Leader: horizontal from the label column to the element's x, then vertical to it.
        let (x0, x1) = if right { (ex.min(RIGHT_COL), RIGHT_COL) } else { (LEFT_COL, ex.max(LEFT_COL)) };
        parts.push(commands.spawn((abs_node(x0, ly, (x1 - x0).max(1.0), 1.0), BackgroundColor(LINE))).id());
        if (ey - ly).abs() > 2.0 {
            let (y0, y1) = if ey > ly { (ly, ey) } else { (ey, ly) };
            parts.push(commands.spawn((abs_node(ex, y0, 1.0, y1 - y0), BackgroundColor(LINE))).id());
        }
        let text_node = if right {
            Node { position_type: PositionType::Absolute, left: px(RIGHT_COL + 6.0), top: px(ly - 9.0), ..default() }
        } else {
            Node {
                position_type: PositionType::Absolute,
                right: px(W - LEFT_COL + 6.0),
                top: px(ly - 9.0),
                ..default()
            }
        };
        parts.push(
            commands
                .spawn((label, Text::default(), small.clone(), TextColor(TEXT), text_node))
                .id(),
        );
    }
    // Home: label centred at the top.
    parts.push(commands.spawn((abs_node(474.0, 26.0, 1.0, 132.0), BackgroundColor(LINE))).id());
    parts.push(
        commands
            .spawn((
                Label::Home,
                Text::default(),
                small,
                TextColor(color(P.caution)),
                Node { position_type: PositionType::Absolute, left: px(380.0), top: px(8.0), ..default() },
            ))
            .id(),
    );
    commands.entity(root).add_children(&parts);

    // Small reminder while the overlay is hidden.
    commands.spawn((
        PadHint,
        Text::new("HOME: controller layout"),
        TextFont { font_size: FontSize::Px(12.0), ..default() },
        TextColor(color(P.muted)),
        Node { position_type: PositionType::Absolute, bottom: px(12.0), right: px(12.0), ..default() },
        Visibility::Hidden,
    ));
}

#[allow(clippy::too_many_arguments)]
pub fn update_pad_overlay(
    input: Res<PilotInput>,
    sim: Res<SharedSim>,
    cam: Res<CamState>,
    mut state: ResMut<PadOverlayState>,
    mut overlay: Single<&mut Visibility, (With<PadOverlay>, Without<PadHint>)>,
    mut hint: Single<&mut Visibility, (With<PadHint>, Without<PadOverlay>)>,
    mut buttons: Query<(&ButtonVis, &mut BackgroundColor), Without<StickKnob>>,
    mut knobs: Query<(&StickKnob, &mut Node)>,
    mut labels: Query<(&Label, &mut Text)>,
) {
    let is_gamepad = input.profile == Some(Profile::Gamepad) && input.pad_view.is_some();
    let (phase, mode, high_rates, ap_enabled, ap_arm, ap_mode, diff, direct) = {
        let c = sim.0.lock().unwrap();
        (c.state.phase, c.mode, c.high_rates, c.ap.enabled, c.ap.arm_switch, c.rc_mode_name(c.ap.mode_index), c.diff_thrust, c.direct)
    };

    // Auto-show when a gamepad takes over; auto-hide when the plane is thrown.
    if is_gamepad && !state.was_gamepad {
        state.visible = true;
    }
    state.was_gamepad = is_gamepad;
    if state.last_phase == Some(Phase::Held) && phase != Phase::Held {
        state.visible = false;
    }
    state.last_phase = Some(phase);
    if is_gamepad && input.has(Action::Help) {
        state.visible = !state.visible;
    }
    if input.has(Action::Dismiss) {
        state.visible = false;
    }

    **overlay = if is_gamepad && state.visible { Visibility::Visible } else { Visibility::Hidden };
    **hint = if is_gamepad && !state.visible { Visibility::Visible } else { Visibility::Hidden };
    if !(is_gamepad && state.visible) {
        return;
    }
    let pad = input.pad_view.as_ref().unwrap();

    for (b, mut bg) in &mut buttons {
        *bg = BackgroundColor(if pad.pressed.contains(&b.0) { LIT } else { PART });
    }
    for (k, mut node) in &mut knobs {
        let v = if k.0 == 0 { pad.left } else { pad.right };
        node.left = px(20.0 + v.x.clamp(-1.0, 1.0) * 18.0);
        node.top = px(20.0 - v.y.clamp(-1.0, 1.0) * 18.0);
    }

    let mode1 = input.gamepad.stick_mode == 1;
    let cam_label = cam.mode.label();
    for (label, mut text) in &mut labels {
        let s = match label {
            Label::ZL => "Throttle cut".to_string(),
            Label::ZR => "Throttle up (hold)".to_string(),
            Label::L => format!("ArduPilot arm switch: {}", if ap_arm { "ON" } else { "off" }),
            Label::R => format!("ArduPilot mode: {}", ap_mode),
            Label::Minus => "Pause".to_string(),
            Label::Plus => "Reset to hand".to_string(),
            Label::LeftStick => {
                if mode1 { "Pitch (up/down), rudder (left/right)".into() } else { "Throttle (up/down), rudder (left/right)".into() }
            }
            Label::RightStick => {
                if mode1 { "Throttle (up/down), roll (left/right)".into() } else { "Pitch (up/down), roll (left/right)".into() }
            }
            Label::DpadUp => {
                if ap_enabled {
                    format!("D-pad up: auto-zoom   down: control {}", if direct { "DIRECT" } else { "ARDUPILOT" })
                } else {
                    "D-pad up: line-of-sight auto-zoom".to_string()
                }
            }
            Label::X => {
                if ap_enabled && !direct { format!("Next ArduPilot mode ({})", ap_mode) } else { format!("Flight mode: {}", mode.label()) }
            }
            Label::A => format!("Camera: {cam_label}"),
            Label::B => {
                if phase == Phase::Held { "Hand launch (or throttle > 55%)".into() } else { "Hand launch".into() }
            }
            Label::Y => format!("Rates: {}  (diff thrust {})", if high_rates { "HIGH" } else { "LOW" }, if diff { "on" } else { "off" }),
            Label::Home => "HOME  show / hide this layout".to_string(),
        };
        if text.0 != s {
            text.0 = s;
        }
    }
}
