//! `--test-radio`: a TX15 that the ground station's acceptance tests move.
//!
//! The station's runner sends the stand-in radio's sticks and switches over the sensor
//! feed link (`ClientMsg::Radio`). Here they become the events a USB joystick sends, through
//! Bevy's raw gamepad events, so they take the same path as the real radio's: the device is
//! found by its name, the axes are mapped, the throttle is scaled, and the arm and mode channels are read as the radio sends them. Only the USB hardware and the OS's HID driver are left out.

use crate::SharedSim;
use crate::input::{AxisMap, PilotInput};
use bevy::input::InputSystems;
use bevy::input::gamepad::{
    GamepadAxis, GamepadConnection, GamepadConnectionEvent, RawGamepadAxisChangedEvent, RawGamepadEvent,
};
use bevy::prelude::*;
use simulation_aircraft::rc_mode_pwm;
use teleoperation_shared::feed::RadioState;

/// Channels 5 and 8 of a TX15 in joystick mode (see controls.cfg.example).
const ARM_AXIS: GamepadAxis = GamepadAxis::Other(0x0001_0034);
const MODE_AXIS: GamepadAxis = GamepadAxis::Other(0x0001_0037);

pub fn plugin(app: &mut App) {
    app.init_resource::<Radio>().add_systems(Startup, setup).add_systems(PreUpdate, drive.before(InputSystems));
}

#[derive(Resource, Default)]
struct Radio {
    pad: Option<Entity>,
    plugged_in: bool,
    reset_presses: u8,
}

/// The pilot has given two channels to the autopilot, as the calibration wizard (F2) does.
fn setup(mut commands: Commands, mut radio: ResMut<Radio>, mut input: ResMut<PilotInput>) {
    input.radio.arm = Some(AxisMap { axis: ARM_AXIS, center: 0.0, scale: 1.0 });
    input.radio.mode = Some(AxisMap { axis: MODE_AXIS, center: 0.0, scale: 1.0 });
    // The device exists from the start. A test must not pick up a real radio plugged into the
    // computer, so this is the only one the sim listens to.
    let pad = commands.spawn_empty().id();
    radio.pad = Some(pad);
    input.only_pad = Some(pad);
}

fn drive(
    sim: Res<SharedSim>,
    mut radio: ResMut<Radio>,
    mut events: MessageWriter<RawGamepadEvent>,
    mut connections: MessageWriter<GamepadConnectionEvent>,
) {
    let Some(pad) = radio.pad else { return };
    // Until the station says otherwise, the sticks are centred and the throttle is down, and the
    // radio isn't plugged in.
    let idle = RadioState {
        connected: false,
        roll: 0.0,
        pitch: 0.0,
        yaw: 0.0,
        throttle: 0.0,
        arm: false,
        mode: 0,
        reset_presses: 0,
    };
    let s = sim.0.lock().unwrap().test_radio.unwrap_or(idle);
    // A fresh plane, as the R key gives (whether or not the radio is plugged in).
    if s.reset_presses != radio.reset_presses {
        radio.reset_presses = s.reset_presses;
        sim.0.lock().unwrap().reset();
    }
    // Plug the radio in, or take it out. Bevy makes the device from this event (the gilrs plugin
    // sends it, too).
    if s.connected != radio.plugged_in {
        radio.plugged_in = s.connected;
        let connection = if s.connected {
            GamepadConnection::Connected { name: "TX15 Joystick (test)".into(), vendor_id: None, product_id: None }
        } else {
            GamepadConnection::Disconnected
        };
        connections.write(GamepadConnectionEvent::new(pad, connection));
        return;
    }
    if !radio.plugged_in {
        return;
    }

    // The axes as the radio reports them (see `RadioConfig::default`): roll and pitch on the
    // left stick's two axes, yaw on its own, and the throttle from -1 (down) to 1 (up).
    let axis = |a: GamepadAxis, v: f32| RawGamepadEvent::Axis(RawGamepadAxisChangedEvent::new(pad, a, v));
    events.write(axis(GamepadAxis::LeftStickX, s.roll));
    // Radio stick forward is nose down.
    events.write(axis(GamepadAxis::LeftStickY, -s.pitch));
    events.write(axis(GamepadAxis::Other(0x0001_0033), s.yaw));
    events.write(axis(GamepadAxis::RightStickX, 2.0 * s.throttle - 1.0));

    // The switches are channels: the arm switch is low or high, and the mode channel sits in
    // the middle of its slot's band.
    events.write(axis(ARM_AXIS, if s.arm { 1.0 } else { -1.0 }));
    events.write(axis(MODE_AXIS, (rc_mode_pwm(s.mode as usize) as f32 - 1500.0) / 500.0));
}
