//! HEEWING T1-S Ranger flight simulator.
//!
//!   cargo run --release                 ray-traced (Bevy Solari), built-in flight modes
//!   cargo run --release -- --raster     classic rasterized lighting with shadow maps
//!   cargo run --release -- --ardupilot  physics backend for ArduPilot SITL (JSON, UDP 9002)
//!   cargo run --release -- --selftest   headless flight-model checks
//!   cargo run --release -- --export-map gcs/assets   map + heights for the ground station
//!   cargo run --release -- --ardupilot --feed  also stream the sensor camera (TCP 5600)
//!   --window-rect x,y,w,h [--title-bar N]       window frame in points (set by `t1s_gcs --station`)
//!   --frameless                                 transparent title bar, no title or buttons
//!   --test-radio                                a TX15 stand-in that the ground station's tests move
//!   --radio PORT [--radio-upstream ADDR] [--radio-rate B] [--home LAT,LON,ALT]
//!                                               the TX15 stand-in: the station connects to PORT, and gets the
//!                                               autopilot's MAVLink (at ADDR, default 127.0.0.1:5760) through a
//!                                               telemetry link, with the other planes the pilot's radios hear
//!   --fullscreen                                start borderless full-screen (F11 toggles); macOS Game Mode needs it
//!   --aircraft ID                               which plane to fly (default t1s-ranger)
//!   --map NAME                                  fly a real place from maps/NAME (see tools/mapbuild)
//!   --direct                                    with --ardupilot: start with the sticks flying (J toggles)
//!   --expect [suite] [--docs dir]               run the acceptance tests in expectations/ (and write the docs)

mod acceptance;
mod test_radio;
mod ardupilot;
mod camera;
mod gc_pad;
mod games;
mod foliage;
mod grass;
mod hud;
mod feed;
mod input;
mod loading;
mod mapgen;
mod placement;
mod power;
mod radio;
mod realsite;
mod mesh_util;
mod model;
mod pad_overlay;
mod radio_overlay;
mod scenery;
mod script;
mod selftest;
mod sim;
#[cfg(target_os = "macos")]
mod switch_hid;
mod terrain;
mod terrain_render;
mod theme;
mod treegen;
mod vegetation;
mod world;

use bevy::light::atmosphere::ScatteringMedium;
use bevy::prelude::*;
use bevy::solari::SolariPlugins;
use bevy::window::{PresentMode, WindowLevel, WindowResolution};
use input::{Action, PilotInput};
use sim::SimCore;
use std::sync::{Arc, Mutex};

/// Physics step for standalone mode (ArduPilot mode is paced by SITL's frame rate).
const PHYSICS_DT: f32 = 1.0 / 600.0;

#[derive(Resource, Clone)]
pub struct SharedSim(pub Arc<Mutex<SimCore>>);

#[derive(Resource, Clone)]
pub struct RenderSettings {
    pub raytracing: bool,
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SimSet {
    Input,
    Logic,
    Visual,
}

fn main() {
    install_controller_mappings();
    let args: Vec<String> = std::env::args().collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    if flag("--selftest") {
        selftest::run_all();
        return;
    }
    if let Some(dir) = args.iter().position(|a| a == "--export-map").map(|i| args.get(i + 1).map_or("gcs/assets", |s| s.as_str())) {
        mapgen::export(dir);
        return;
    }
    // A real place, from a map pack: must be loaded before anything asks about the ground.
    if let Some(name) = args.iter().position(|a| a == "--map").and_then(|i| args.get(i + 1)) {
        let maps = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("maps");
        if let Err(e) = terrain::load_site(&maps, name) {
            eprintln!("--map {name}: {e}\nBuild it with: cargo run --release --manifest-path tools/mapbuild/Cargo.toml -- {name}");
            std::process::exit(1);
        }
        let r = terrain::real().unwrap();
        println!(
            "map: {} (home {:.6}, {:.6}, {:.1} m AMSL, {} trees, {} buildings)",
            r.meta.title,
            r.meta.home_lat,
            r.meta.home_lon,
            r.meta.home_alt,
            r.trees.len(),
            r.buildings.len()
        );
    }
    // Which plane to fly: `--aircraft ID` (see simulation_aircraft).
    let aircraft = match args.iter().position(|a| a == "--aircraft").and_then(|i| args.get(i + 1)) {
        // The sim's own acceptance tests are about the plain T1-S, which the keyboard flies.
        None if flag("--expect") => simulation_aircraft::find("t1s-ranger").unwrap(),
        None => simulation_aircraft::default(),
        Some(id) => simulation_aircraft::find(id.as_str()).unwrap_or_else(|| {
            eprintln!("--aircraft {id}: no such plane. Available: {}", simulation_aircraft::ids().join(", "));
            std::process::exit(1);
        }),
    };
    println!("aircraft: {}", aircraft.title);
    let params = (aircraft.params)();
    let ap_port: u16 = args
        .iter()
        .position(|a| a == "--ap-port")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse().ok())
        .unwrap_or(9002);
    let ap_enabled = flag("--ardupilot");
    if let Some(why) = sim::cannot_fly(&params, aircraft.title, ap_enabled) {
        eprintln!("{why}");
        std::process::exit(1);
    }
    if ap_enabled && !params.flight_controller {
        eprintln!("--ardupilot: the {} has no flight controller (only a receiver and ESCs), so ArduPilot has nothing to fly", aircraft.title);
        std::process::exit(1);
    }
    let direct = flag("--direct");
    let raytracing = !flag("--raster");

    // Trees/obstacles are generated with the rest of the world on the loading screen.
    let core = Arc::new(Mutex::new(SimCore::new(params.clone(), vec![], ap_enabled, ap_port)));
    if ap_enabled {
        core.lock().unwrap().direct = direct;
        ardupilot::spawn_bridge(core.clone(), ap_port);
    }

    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    let feed = flag("--feed").then(|| feed::FeedConfig {
        port: arg("--feed").and_then(|p| p.parse().ok()).unwrap_or(t1s_station::feed::DEFAULT_PORT),
        size: arg("--feed-size")
            .and_then(|s| s.split_once('x'))
            .and_then(|(w, h)| Some(UVec2::new(w.parse().ok()?, h.parse().ok()?)))
            .unwrap_or(UVec2::new(320, 240)),
    });
    let window_rect = arg("--window-rect").and_then(|s| t1s_station::layout::Rect::parse(s));
    let frameless = flag("--frameless");
    // A frameless window's content fills its frame; otherwise allow for the title bar.
    let title_bar: f32 = if frameless { 0.0 } else { arg("--title-bar").and_then(|s| s.parse().ok()).unwrap_or(28.0) };
    if flag("--exit-with-parent") {
        placement::exit_with_parent();
    }

    // `--expect [suite]` runs the suite (default expectations/); `--docs [dir]` writes the docs.
    let value = |name: &str| arg(name).filter(|v| !v.starts_with("--")).cloned();
    let expect = flag("--expect").then(|| {
        value("--expect").map_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("expectations"), Into::into)
    });
    let docs = flag("--docs").then(|| value("--docs").unwrap_or_else(|| "docs".into()).into());
    let shots = flag("--shots") || expect.is_some();
    // No sleep while the sim runs (and the OS's gaming mode, where it can be asked for).
    let _awake = power::keep_awake("T1-S sim is running");
    let mut app = App::new();
    app.add_plugins(bevy::diagnostic::FrameTimeDiagnosticsPlugin::default());
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            // Keep the window visible during scripted captures (occluded windows aren't presented).
            window_level: if shots { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal },
            title: format!("HEEWING T1-S Ranger Sim - {}", terrain::site_title()),
            resolution: WindowResolution::new(1600, 900),
            present_mode: PresentMode::AutoVsync,
            mode: if flag("--fullscreen") {
                bevy::window::WindowMode::BorderlessFullscreen(bevy::window::MonitorSelection::Current)
            } else {
                bevy::window::WindowMode::Windowed
            },
            // --frameless (set by the ground station): the view runs under a transparent title
            // bar with no title or buttons, like the station's other panes.
            titlebar_transparent: frameless,
            fullsize_content_view: frameless,
            titlebar_show_title: !frameless,
            titlebar_show_buttons: !frameless,
            ..default()
        }),
        ..default()
    }));
    if raytracing {
        app.add_plugins(SolariPlugins);
    }
    app.insert_resource(SharedSim(core.clone()))
        .insert_resource(RenderSettings { raytracing })
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(GlobalAmbientLight::NONE)
        .init_resource::<hud::HudState>()
        .init_resource::<pad_overlay::PadOverlayState>()
        .init_resource::<radio_overlay::RadioOverlayState>()
        .insert_resource(gc_pad::GcPads::new(flag("--debug-input")))
        .configure_sets(Update, (SimSet::Input, SimSet::Logic, SimSet::Visual).chain())
        .add_plugins(input::InputPlugin)
        .add_systems(Update, (quit_on_cmd_q, toggle_fullscreen))
        .add_systems(Startup, (setup, grass::setup_grass))
        .add_systems(Update, sim_update.in_set(SimSet::Logic).run_if(in_state(loading::AppState::Running)))
        .add_systems(
            Update,
            (model::sync_plane, world::update_windsock, scenery::animate_scenery, camera::update_camera, grass::stream_grass.run_if(in_state(loading::AppState::Running)), hud::update_hud, pad_overlay::update_pad_overlay, radio_overlay::update_radio_overlay)
                .chain()
                .in_set(SimSet::Visual),
        );
    camera::camera_plugin(&mut app);
    games::plugin(&mut app);
    if let Some(port) = arg("--radio").and_then(|p| p.parse::<u16>().ok()) {
        let home = arg("--home")
            .and_then(|s| {
                let v: Vec<f64> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                (v.len() >= 2).then(|| communication_traffic::Position::new(v[0], v[1], v.get(2).copied().unwrap_or(0.0) as f32))
            })
            // ardupilot/start.sh's default home.
            .unwrap_or(communication_traffic::Position::new(-35.363261, 149.165230, 584.0));
        radio::plugin(
            &mut app,
            radio::Config {
                port,
                upstream: arg("--radio-upstream").cloned().unwrap_or_else(|| "127.0.0.1:5760".into()),
                home,
                rate: arg("--radio-rate").and_then(|r| r.parse().ok()).unwrap_or(10_000.0),
            },
        );
    }
    if let Some(cfg) = feed {
        feed::feed_plugin(&mut app, cfg, core.clone());
    }
    if let Some(r) = window_rect {
        app.insert_resource(placement::WindowRect(r, title_bar)).add_systems(Update, placement::place_window);
    }
    loading::loading_plugin(&mut app);
    if let Some(dir) = args.iter().position(|a| a == "--shots").and_then(|i| args.get(i + 1)) {
        std::fs::create_dir_all(dir).ok();
        app.insert_resource(script::ShotScript { dir: dir.clone(), t: 0.0, done: vec![] })
            .add_systems(
                Update,
                script::run_script.after(SimSet::Input).before(SimSet::Logic).run_if(in_state(loading::AppState::Running)),
            )
            .add_systems(Update, script::log_state.after(SimSet::Visual))
            .add_systems(Update, script::shot_loading.run_if(in_state(loading::AppState::Loading)));
    }
    if flag("--test-radio") {
        app.add_plugins(test_radio::plugin);
    }
    if let Some(suite) = expect {
        acceptance::plugin(&mut app, suite, docs, params.flight_controller);
    }
    if matches!(app.run(), AppExit::Error(_)) {
        std::process::exit(1);
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut media: ResMut<Assets<ScatteringMedium>>,
    settings: Res<RenderSettings>,
) {
    world::spawn_world(&mut commands, &mut meshes, &mut materials, &mut images, &mut media, &settings);
    model::spawn_plane(&mut commands, &mut meshes, &mut materials, 0, Color::srgb(1.0, 0.36, 0.05));
    // The computer pilots' planes, in their own colours, for the games.
    for (i, (_, _, [r, g, b])) in recreation_games::PILOTS.iter().enumerate() {
        model::spawn_plane(&mut commands, &mut meshes, &mut materials, i + 1, Color::linear_rgb(*r, *g, *b));
    }
    camera::spawn_camera(&mut commands, &settings, &mut images);
    hud::spawn_hud(&mut commands);
    pad_overlay::spawn_pad_overlay(&mut commands);
    radio_overlay::spawn_radio_overlay(&mut commands);
}

/// Cmd-Q (macOS) / Ctrl-Q elsewhere ends the sim.
/// F11 switches between a window and borderless full screen.
fn toggle_fullscreen(keys: Res<ButtonInput<KeyCode>>, mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>) {
    use bevy::window::{MonitorSelection, WindowMode};
    if !keys.just_pressed(KeyCode::F11) {
        return;
    }
    for mut w in &mut windows {
        w.mode = match w.mode {
            WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            _ => WindowMode::Windowed,
        };
    }
}

fn quit_on_cmd_q(keys: Res<ButtonInput<KeyCode>>, mut exit: MessageWriter<AppExit>) {
    let modifier = keys.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight, KeyCode::ControlLeft, KeyCode::ControlRight]);
    if modifier && keys.just_pressed(KeyCode::KeyQ) {
        exit.write(AppExit::Success);
    }
}

fn sim_update(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut input: ResMut<PilotInput>,
    sim: Res<SharedSim>,
    mut acc: Local<f32>,
) {
    let mut c = sim.0.lock().unwrap();
    c.sticks = input.sticks;
    c.pilot_radio = input.profile == Some(input::Profile::Radio);
    c.radio_arm = input.radio_arm;
    c.radio_mode = input.radio_mode;

    for a in input.actions.clone() {
        match a {
            Action::Launch => c.launch(),
            Action::Race => c.toggle_game(recreation_games::Kind::Race),
            Action::Tag => c.toggle_game(recreation_games::Kind::Tag),
            Action::Reset => c.reset(),
            // Self-levelling and rate hold are a flight controller's: a plane without one stays in MANUAL.
            Action::Mode if c.params.flight_controller && (!c.ap.enabled || c.direct) => c.mode = c.mode.next(),
            Action::ControlSource if c.ap.enabled => {
                c.direct = !c.direct;
                input.notice = if c.direct {
                    format!("DIRECT: sticks fly the plane ({}); ArduPilot only watches", c.mode.label())
                } else {
                    "ARDUPILOT flies the plane".into()
                };
            }
            Action::Rates => c.high_rates = !c.high_rates,
            Action::Pause => c.paused = !c.paused,
            Action::DiffThrust => c.diff_thrust = !c.diff_thrust,
            Action::Recharge => {
                c.state.recharge();
                input.notice = "Fresh 4S 1800 mAh pack".into();
            }
            Action::ApArm => c.ap.arm_switch = !c.ap.arm_switch,
            Action::ApMode | Action::Mode => {
                c.ap.mode_index = (c.ap.mode_index + 1) % c.params.rc_modes.len().max(1);
            }
            _ => {}
        }
    }

    // Wind controls.
    if keys.just_pressed(KeyCode::BracketRight) {
        c.wind.speed = (c.wind.speed + 1.0).min(15.0);
    }
    if keys.just_pressed(KeyCode::BracketLeft) {
        c.wind.speed = (c.wind.speed - 1.0).max(0.0);
    }
    if keys.just_pressed(KeyCode::Period) {
        c.wind.from_dir += 15f32.to_radians();
    }
    if keys.just_pressed(KeyCode::Comma) {
        c.wind.from_dir -= 15f32.to_radians();
    }
    if keys.just_pressed(KeyCode::Slash) {
        c.wind.turbulence = if c.wind.turbulence >= 0.99 { 0.0 } else { (c.wind.turbulence + 0.25).min(1.0) };
    }

    if c.ap.enabled {
        // Physics is stepped in lockstep by the ArduPilot bridge thread.
        return;
    }
    if c.paused {
        *acc = 0.0;
        return;
    }
    *acc += time.delta_secs().min(0.1);
    while *acc >= PHYSICS_DT {
        c.step_standalone(PHYSICS_DT);
        *acc -= PHYSICS_DT;
    }
}

/// Extra SDL mappings for controllers whose exact USB version isn't in gilrs' bundled
/// database (e.g. Switch Pro Controllers reporting version 0x0200 on macOS).
fn install_controller_mappings() {
    const PRO: &str = "Nintendo Switch Pro Controller,a:b0,b:b1,back:b8,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b12,leftshoulder:b4,leftstick:b10,lefttrigger:b6,leftx:a0,lefty:a1,rightshoulder:b5,rightstick:b11,righttrigger:b7,rightx:a2,righty:a3,start:b9,x:b2,y:b3,platform:Mac OS X,";
    let mut mappings: Vec<String> = ["030000007e0500000920000000020000", "030000007e0500000920000001020000"]
        .iter()
        .map(|guid| format!("{guid},{PRO}"))
        .collect();
    if let Ok(existing) = std::env::var("SDL_GAMECONTROLLERCONFIG") {
        mappings.insert(0, existing);
    }
    // SAFETY: called first thing in main, before any other threads exist.
    unsafe { std::env::set_var("SDL_GAMECONTROLLERCONFIG", mappings.join("\n")) };
}
