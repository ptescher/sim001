//! Gamepads via Apple's GameController framework (macOS).
//!
//! Switch Pro Controllers are switched into their full report mode by macOS, which raw-HID
//! readers (gilrs) then misparse into phantom button presses and stuck axes. The
//! GameController framework understands the controller natively, so gamepads are read
//! from here on macOS; gilrs is still used for RC radios (plain HID joysticks).

use bevy::ecs::system::NonSendMarker;
use bevy::input::gamepad::GamepadButton;
use bevy::prelude::*;

/// One frame of gamepad state, in Bevy's positional button naming.
#[derive(Clone, Debug, Default)]
pub struct PadSnapshot {
    pub name: String,
    pub left: Vec2,
    pub right: Vec2,
    pub pressed: Vec<GamepadButton>,
    pub just_pressed: Vec<GamepadButton>,
}

#[derive(Resource, Default)]
pub struct GcPads {
    pub pad: Option<PadSnapshot>,
    #[cfg(target_os = "macos")]
    prev: Vec<GamepadButton>,
    /// `--debug-input`: log every change to the terminal.
    pub debug: bool,
    #[cfg(target_os = "macos")]
    last_logged: Option<(Vec2, Vec2, usize)>,
    /// Raw-HID Switch Pro reader (preferred over GameController on macOS).
    #[cfg(target_os = "macos")]
    hid: Option<crate::switch_hid::SharedHidPad>,
}

#[cfg(target_os = "macos")]
pub fn poll(mut pads: ResMut<GcPads>, mut init: Local<bool>, _main_thread: NonSendMarker) {
    use objc2_game_controller::{GCController, GCControllerButtonInput, GCDevice};

    // SAFETY: GameController calls are made on the main thread (NonSendMarker), and all
    // returned objects are retained for the duration of their use.
    unsafe {
        if !*init {
            *init = true;
            // Keep reading the controller even when the sim window isn't focused.
            GCController::setShouldMonitorBackgroundEvents(true);
        }
        // Prefer the raw-HID Switch Pro reader when it has fresh data.
        let mut snap: Option<PadSnapshot> = pads
            .hid
            .as_ref()
            .and_then(|h| h.lock().unwrap().clone())
            .filter(|s| s.last_report.is_some_and(|t| t.elapsed().as_secs_f32() < 0.5))
            .map(|s| PadSnapshot {
                name: format!("Pro Controller (HID{})", if s.calibrated { "" } else { ", uncalibrated" }),
                left: s.left,
                right: s.right,
                pressed: s.pressed,
                just_pressed: Vec::new(),
            });
        let controllers = GCController::controllers();
        for c in controllers.iter().take(if snap.is_some() { 0 } else { usize::MAX }) {
            let Some(g) = c.extendedGamepad() else { continue };
            let name = c.vendorName().map(|n| n.to_string()).unwrap_or_else(|| c.productCategory().to_string());
            let stick = |d: objc2::rc::Retained<objc2_game_controller::GCControllerDirectionPad>| {
                Vec2::new(d.xAxis().value(), d.yAxis().value())
            };
            let mut pressed = Vec::new();
            let mut check = |b: Option<objc2::rc::Retained<GCControllerButtonInput>>, id: GamepadButton| {
                if b.is_some_and(|b| b.isPressed()) {
                    pressed.push(id);
                }
            };
            // GameController names face buttons by position (A = bottom), like Bevy.
            check(Some(g.buttonA()), GamepadButton::South);
            check(Some(g.buttonB()), GamepadButton::East);
            check(Some(g.buttonX()), GamepadButton::West);
            check(Some(g.buttonY()), GamepadButton::North);
            check(Some(g.leftShoulder()), GamepadButton::LeftTrigger);
            check(Some(g.rightShoulder()), GamepadButton::RightTrigger);
            check(Some(g.leftTrigger()), GamepadButton::LeftTrigger2);
            check(Some(g.rightTrigger()), GamepadButton::RightTrigger2);
            check(Some(g.buttonMenu()), GamepadButton::Start);
            check(g.buttonOptions(), GamepadButton::Select);
            check(g.buttonHome(), GamepadButton::Mode);
            let dpad = g.dpad();
            check(Some(dpad.up()), GamepadButton::DPadUp);
            check(Some(dpad.down()), GamepadButton::DPadDown);
            snap = Some(PadSnapshot {
                name,
                left: stick(g.leftThumbstick()),
                right: stick(g.rightThumbstick()),
                pressed,
                just_pressed: Vec::new(),
            });
            break;
        }
        if let Some(s) = &mut snap {
            s.just_pressed = s.pressed.iter().filter(|b| !pads.prev.contains(b)).copied().collect();
            pads.prev = s.pressed.clone();
        } else {
            pads.prev.clear();
        }
        if pads.debug {
            match &snap {
                Some(s) => {
                    let key = (s.left, s.right, s.pressed.len());
                    let changed = match pads.last_logged {
                        None => true,
                        Some((l, r, n)) => {
                            l.distance(s.left) > 0.05 || r.distance(s.right) > 0.05 || n != s.pressed.len()
                        }
                    };
                    if changed {
                        info!(
                            "gamepad {}: left ({:+.2}, {:+.2}) right ({:+.2}, {:+.2}) pressed {:?}",
                            s.name, s.left.x, s.left.y, s.right.x, s.right.y, s.pressed
                        );
                        pads.last_logged = Some(key);
                    }
                }
                None if pads.last_logged.is_some() => {
                    info!("gamepad: none visible");
                    pads.last_logged = None;
                }
                None => {}
            }
        }
        pads.pad = snap;
    }
}

#[cfg(not(target_os = "macos"))]
pub fn poll() {}

impl GcPads {
    pub fn new(debug: bool) -> Self {
        #[allow(unused_mut)]
        let mut s = Self { debug, ..Default::default() };
        #[cfg(target_os = "macos")]
        {
            s.hid = Some(crate::switch_hid::start(debug));
        }
        s
    }
}

/// Make the sim a regular foreground app and bring it to the front.
///
/// macOS only delivers keyboard and GameController input to the active application. A bare
/// binary started from a terminal (rather than a .app bundle) is not always activated by
/// clicking its window, which leaves both keyboard and controller dead.
#[cfg(target_os = "macos")]
pub fn activate_app(_main_thread: NonSendMarker, mut done: Local<u32>) {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_game_controller::GCController;
    // Retry for the first few frames: the window may not exist yet on frame 0.
    if *done > 30 {
        return;
    }
    *done += 1;
    let Some(mtm) = objc2::MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    if *done == 1 {
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        // SAFETY: plain property setter on the main thread.
        unsafe { GCController::setShouldMonitorBackgroundEvents(true) };
    }
    if !app.isActive() {
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn activate_app() {}

/// `--debug-input`: report app activation and window focus changes.
pub fn log_focus(
    _main_thread: NonSendMarker,
    pads: Res<GcPads>,
    windows: Query<&Window>,
    mut last: Local<Option<(bool, bool)>>,
) {
    if !pads.debug {
        return;
    }
    #[cfg(target_os = "macos")]
    let active = objc2::MainThreadMarker::new()
        .map(|mtm| objc2_app_kit::NSApplication::sharedApplication(mtm).isActive())
        .unwrap_or(false);
    #[cfg(not(target_os = "macos"))]
    let active = true;
    let focused = windows.iter().any(|w| w.focused);
    if *last != Some((active, focused)) {
        info!("app active: {active}, window focused: {focused}");
        *last = Some((active, focused));
    }
}
