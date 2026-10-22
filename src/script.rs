//! `--shots <dir>`: scripted demo flight that saves screenshots, for visual regression checks.
//! With `--ardupilot` it follows a longer timeline and leaves the flying to ArduPilot.

use crate::SharedSim;
use crate::input::{Action, PilotInput};
use crate::sim::FlightMode;
use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

#[derive(Resource)]
pub struct ShotScript {
    pub dir: String,
    pub t: f32,
    pub done: Vec<bool>,
}

/// Standalone demo: (time, step). "shot:<name>" saves a screenshot.
const STEPS: &[(f32, &str)] = &[
    (7.0, "shot:01_los_held"),
    (7.5, "launch"),
    (16.0, "shot:02_los_climb"),
    (16.5, "camera"),
    (19.0, "shot:03_chase"),
    (19.5, "camera"),
    (22.0, "shot:04_fpv"),
    (22.5, "camera"),
    (26.0, "shot:05_los_bank"),
    (27.0, "exit"),
];

/// ArduPilot mode: the autopilot flies; sticks stay neutral (the test drives RC via MAVLink).
const AP_STEPS: &[(f32, &str)] = &[
    (20.0, "shot:ap_01_held"),
    (100.0, "shot:ap_02_fbwa"),
    (101.0, "camera"),
    (140.0, "shot:ap_03_chase_loiter"),
    (141.0, "camera"),
    (142.0, "camera"),
    (175.0, "shot:ap_04_los_loiter"),
    (200.0, "exit"),
];

pub fn run_script(
    mut commands: Commands,
    time: Res<Time>,
    mut script: ResMut<ShotScript>,
    mut input: ResMut<PilotInput>,
    sim: Res<SharedSim>,
    mut exit: MessageWriter<AppExit>,
) {
    script.t += time.delta_secs();
    let ap = sim.0.lock().unwrap().ap.enabled;
    let steps = if ap { AP_STEPS } else { STEPS };
    if script.done.is_empty() {
        script.done = vec![false; steps.len()];
        sim.0.lock().unwrap().mode = FlightMode::Angle;
    }
    if !ap {
        // Launch at 75% throttle, then a gentle climbing right bank.
        let t = script.t;
        input.sticks.throttle = if t > 7.5 { 0.75 } else { 0.0 };
        input.sticks.roll = if t > 23.0 { 0.5 } else { 0.0 };
        input.sticks.pitch = if t > 23.0 { 0.15 } else { 0.0 };
    }
    for (i, (at, what)) in steps.iter().enumerate() {
        if script.done[i] || script.t < *at {
            continue;
        }
        script.done[i] = true;
        if let Some(name) = what.strip_prefix("shot:") {
            let path = format!("{}/{}.png", script.dir, name);
            info!("screenshot {path}");
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
        } else {
            match *what {
                "launch" => input.actions.push(Action::Launch),
                "camera" => input.actions.push(Action::Camera),
                "exit" => {
                    exit.write(AppExit::Success);
                }
                _ => {}
            }
        }
    }
}

pub fn log_state(
    time: Res<Time>,
    sim: Res<SharedSim>,
    input: Res<PilotInput>,
    cam: Query<&Transform, With<crate::camera::MainCamera>>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    mut last: Local<f32>,
) {
    if time.elapsed_secs() - *last < 1.0 {
        return;
    }
    *last = time.elapsed_secs();
    info!("raw: {}", input.raw_text.replace('\n', " | "));
    if let Some(fps) = diagnostics.get(&bevy::diagnostic::FrameTimeDiagnosticsPlugin::FPS).and_then(|d| d.smoothed()) {
        info!("fps {fps:.1}");
    }
    let c = sim.0.lock().unwrap();
    let ct = cam.single().map(|t| (t.translation, t.rotation)).ok();
    info!(
        "t={:.0} phase {:?} {} pos {:.1?} vel {:.1?} rot {:.2?} thr {:.2} mode {:?} ap {} pwm {:?} cam {:?}",
        time.elapsed_secs(),
        c.state.phase,
        c.state.crash_reason,
        c.state.pos,
        c.state.vel,
        c.state.rot,
        c.sticks.throttle,
        c.mode,
        c.ap.status,
        &c.ap.pwm[..5],
        ct
    );
}

/// `--shots`: capture the loading screen and log main-thread frame times while loading.
pub fn shot_loading(
    mut commands: Commands,
    time: Res<Time>,
    script: Res<ShotScript>,
    mut taken: Local<bool>,
    mut worst: Local<f32>,
    mut elapsed: Local<f32>,
) {
    let dt = time.delta_secs();
    *elapsed += dt;
    if *elapsed > 0.5 {
        *worst = worst.max(dt);
    }
    if !*taken && *elapsed > 2.5 {
        *taken = true;
        let path = format!("{}/00_loading.png", script.dir);
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    }
    if (*elapsed % 1.0) < dt {
        info!("loading: t={:.1}s worst frame in the last second {:.0} ms", *elapsed, *worst * 1000.0);
        *worst = 0.0;
    }
}
