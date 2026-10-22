//! On-screen RadioMaster TX15 diagram: both gimbals with live stick positions, the arm switch
//! and the six positions of the mode channel, with the one the radio is on lit.
//!
//! Shown automatically when a radio becomes the active controller, hidden once the plane is
//! thrown, toggled with H / F1 (the pad overlay's keys; the radio has no HOME button).

use crate::SharedSim;
use crate::input::{Action, PilotInput, Profile};
use crate::theme::{P, alpha, color};
use bevy::prelude::*;
use simulation_physics::Phase;

const W: f32 = 760.0;
const H: f32 = 300.0;
const WELL: f32 = 120.0;
const KNOB: f32 = 30.0;

const BODY: Color = alpha(P.bezel, 0.95);
const PART: Color = color(P.bezel_edge);
const LIT: Color = color(P.caution);
const TEXT: Color = color(P.text);
const MUTED: Color = color(P.muted);

#[derive(Component)]
pub struct RadioOverlay;
#[derive(Component)]
pub struct RadioHint;
/// A gimbal's knob (0 = left, 1 = right).
#[derive(Component)]
pub struct Gimbal(u8);
/// One of the six mode positions.
#[derive(Component)]
pub struct ModeSlot(usize);
#[derive(Component)]
pub struct ArmBadge;
#[derive(Component)]
pub struct GimbalLabel(u8);
/// The parts that only a plane with a flight controller has: the mode channel and the arm switch.
#[derive(Component)]
pub struct FcPart;

#[derive(Resource, Default)]
pub struct RadioOverlayState {
    visible: bool,
    was_radio: bool,
    last_phase: Option<Phase>,
}

fn abs(left: f32, top: f32, w: f32, h: f32) -> Node {
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

pub fn spawn_radio_overlay(commands: &mut Commands) {
    let small = TextFont { font_size: FontSize::Px(13.0), ..default() };
    let root = commands
        .spawn((
            RadioOverlay,
            Node {
                position_type: PositionType::Absolute,
                bottom: px(16.0),
                left: percent(50.0),
                margin: UiRect::left(px(-W / 2.0)),
                width: px(W),
                height: px(H),
                border_radius: BorderRadius::all(px(14.0)),
                ..default()
            },
            BackgroundColor(alpha(P.screen, 0.6)),
            Visibility::Hidden,
        ))
        .id();
    let mut parts: Vec<Entity> = Vec::new();
    // The body, and the two gimbals with their knobs.
    parts.push(
        commands
            .spawn((
                Node { border_radius: BorderRadius::all(px(60.0)), ..abs(20.0, 20.0, W - 40.0, H - 40.0) },
                BackgroundColor(BODY),
            ))
            .id(),
    );
    for (i, cx) in [150.0, W - 150.0].into_iter().enumerate() {
        let well = commands
            .spawn((
                Node { border_radius: BorderRadius::all(px(WELL / 2.0)), ..abs(cx - WELL / 2.0, 120.0, WELL, WELL) },
                BackgroundColor(PART),
            ))
            .id();
        let knob = commands
            .spawn((
                Gimbal(i as u8),
                Node {
                    border_radius: BorderRadius::all(px(KNOB / 2.0)),
                    ..abs((WELL - KNOB) / 2.0, (WELL - KNOB) / 2.0, KNOB, KNOB)
                },
                BackgroundColor(MUTED),
            ))
            .id();
        commands.entity(well).add_child(knob);
        parts.push(well);
        parts.push(
            commands
                .spawn((
                    GimbalLabel(i as u8),
                    Text::default(),
                    small.clone(),
                    TextColor(TEXT),
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(cx - 130.0),
                        top: px(250.0),
                        width: px(260.0),
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                ))
                .id(),
        );
    }
    // The six positions of the mode channel, left to right.
    let (slot_w, gap) = (74.0, 6.0);
    let x0 = (W - 6.0 * slot_w - 5.0 * gap) / 2.0;
    parts.push(
        commands
            .spawn((
                FcPart,
                Text::new("MODE (CH8: SA + SB)"),
                small.clone(),
                TextColor(MUTED),
                Node { position_type: PositionType::Absolute, left: px(x0), top: px(38.0), ..default() },
            ))
            .id(),
    );
    for i in 0..6 {
        let slot = commands
            .spawn((
                FcPart,
                ModeSlot(i),
                Node {
                    border_radius: BorderRadius::all(px(8.0)),
                    ..abs(x0 + i as f32 * (slot_w + gap), 62.0, slot_w, 30.0)
                },
                BackgroundColor(PART),
            ))
            .id();
        commands.entity(slot).with_child((Text::default(), small.clone(), TextColor(TEXT)));
        parts.push(slot);
    }
    // The arm switch.
    parts.push(
        commands
            .spawn((
                ArmBadge,
                FcPart,
                Node { border_radius: BorderRadius::all(px(8.0)), ..abs((W - 150.0) / 2.0, 120.0, 150.0, 40.0) },
                BackgroundColor(PART),
            ))
            .with_child((Text::default(), small.clone(), TextColor(TEXT)))
            .id(),
    );
    parts.push(
        commands
            .spawn((
                FcPart,
                Text::new("ARM (CH5: SE)"),
                small,
                TextColor(MUTED),
                Node {
                    position_type: PositionType::Absolute,
                    left: px((W - 150.0) / 2.0),
                    top: px(100.0),
                    ..default()
                },
            ))
            .id(),
    );
    commands.entity(root).add_children(&parts);

    commands.spawn((
        RadioHint,
        Text::new("H: radio layout"),
        TextFont { font_size: FontSize::Px(12.0), ..default() },
        TextColor(MUTED),
        Node { position_type: PositionType::Absolute, bottom: px(12.0), right: px(12.0), ..default() },
        Visibility::Hidden,
    ));
}

#[allow(clippy::too_many_arguments)]
pub fn update_radio_overlay(
    input: Res<PilotInput>,
    sim: Res<SharedSim>,
    mut state: ResMut<RadioOverlayState>,
    mut overlay: Single<&mut Visibility, (With<RadioOverlay>, Without<RadioHint>)>,
    mut hint: Single<&mut Visibility, (With<RadioHint>, Without<RadioOverlay>)>,
    mut knobs: Query<(&Gimbal, &mut Node), (Without<ModeSlot>, Without<ArmBadge>)>,
    mut slots: Query<(&ModeSlot, &mut BackgroundColor, &Children), Without<ArmBadge>>,
    mut badge: Query<(&mut BackgroundColor, &Children), (With<ArmBadge>, Without<ModeSlot>)>,
    mut texts: Query<&mut Text, (Without<GimbalLabel>, Without<RadioHint>)>,
    mut fc_parts: Query<&mut Visibility, (With<FcPart>, Without<RadioOverlay>, Without<RadioHint>)>,
    mut labels: Query<(&GimbalLabel, &mut Text), Without<RadioHint>>,
) {
    let is_radio = input.profile == Some(Profile::Radio);
    let (phase, armed_switch, slot, names) = {
        let c = sim.0.lock().unwrap();
        let slot = simulation_aircraft::rc_mode_slot(c.rc_channels()[7]);
        let names: Vec<&'static str> = (0..6).map(|i| c.rc_mode_name(i)).collect();
        (c.state.phase, c.arm_switch_is_on(), slot, names)
    };
    if is_radio && !state.was_radio {
        state.visible = true;
    }
    state.was_radio = is_radio;
    if state.last_phase == Some(Phase::Held) && phase != Phase::Held {
        state.visible = false;
    }
    state.last_phase = Some(phase);
    if is_radio && input.has(Action::Help) {
        state.visible = !state.visible;
    }
    if input.has(Action::Dismiss) {
        state.visible = false;
    }
    **overlay = if is_radio && state.visible { Visibility::Visible } else { Visibility::Hidden };
    **hint = if is_radio && !state.visible { Visibility::Visible } else { Visibility::Hidden };
    if !(is_radio && state.visible) {
        return;
    }

    // Mode 2: the left gimbal is throttle and rudder, the right one elevator and aileron.
    let s = input.sticks;
    let pos = [(s.yaw, s.throttle * 2.0 - 1.0), (s.roll, -s.pitch)];
    let reach = (WELL - KNOB) / 2.0 - 4.0;
    for (g, mut node) in &mut knobs {
        let (x, y) = pos[g.0 as usize];
        node.left = px((WELL - KNOB) / 2.0 + x.clamp(-1.0, 1.0) * reach);
        node.top = px((WELL - KNOB) / 2.0 - y.clamp(-1.0, 1.0) * reach);
    }
    let has_fc = names[0] != "-";
    for mut v in &mut fc_parts {
        *v = if has_fc { Visibility::Inherited } else { Visibility::Hidden };
    }
    for (g, mut t) in &mut labels {
        let want = if g.0 == 0 { "Throttle (up/down), rudder" } else { "Pitch (up/down), roll" };
        if t.0 != want {
            t.0 = want.into();
        }
    }
    for (m, mut bg, children) in &mut slots {
        *bg = BackgroundColor(if m.0 == slot { LIT } else { PART });
        if let Some(mut t) = children.first().and_then(|c| texts.get_mut(*c).ok()) {
            t.0 = names[m.0].to_string();
        }
    }
    for (mut bg, children) in &mut badge {
        *bg = BackgroundColor(if armed_switch { LIT } else { PART });
        if let Some(mut t) = children.first().and_then(|c| texts.get_mut(*c).ok()) {
            t.0 = if armed_switch { "ARM: ON".into() } else { "arm: off".into() };
        }
    }
}
