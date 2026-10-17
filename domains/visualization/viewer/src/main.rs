//! sim001_view: the simulator's 3D view of a real plane, from its MAVLink telemetry.
//!
//!   cargo run --release -p visualization_viewer -- [options]
//!
//!   --udp PORT          listen for MAVLink on this UDP port (default 14550, where ArduPilot and
//!                       MAVProxy send it)
//!   --tcp ADDR          connect to MAVLink on TCP instead (SITL's spare port is 127.0.0.1:5762)
//!   --home LAT,LON,ALT  where the pilot box is; default the map's home, or what the plane says
//!   --map NAME          the scenery of a real place, from maps/NAME (see domains/visualization/mapbuild)
//!   --raytrace          ray-traced lighting (Bevy Solari) instead of rasterized
//!   --capture DIR       save pictures of the scenery and exit; each view is one
//!   --at NAME=x,agl,z,tx,tagl,tz[,fov]   (repeat it). Heights are above the ground.
//!   --plane x,agl,z,heading,roll,tilt   hold a VTOL still there (degrees; tilt 0 forward, 90 up)
//!
//! C cycles the view (line of sight, chase, nose camera); Z switches the auto-zoom.

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution};
use std::io::Read;
use std::net::{TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use visualization_engine::camera::View;
use visualization_engine::{EnginePlugin, EngineSet, Pose, Scene, ViewInput, capture, terrain};
use visualization_telemetry::{Home, Watcher};

#[derive(Resource, Clone)]
struct Telemetry(Arc<Mutex<Watcher>>);

#[derive(Resource)]
struct Source(String);

/// A plane held still where `--plane` says.
#[derive(Resource)]
struct Parked(Option<Pose>);

#[derive(Component)]
struct Status;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    if flag("--help") || flag("-h") {
        println!("{}", include_str!("main.rs").lines().take_while(|l| l.starts_with("//!")).map(|l| l.trim_start_matches("//!").strip_prefix(' ').unwrap_or("")).collect::<Vec<_>>().join("\n"));
        return;
    }
    if let Some(name) = arg("--map") {
        let maps = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../maps");
        if let Err(e) = terrain::load_site(&maps, &name) {
            eprintln!("--map {name}: {e}");
            std::process::exit(1);
        }
    }
    // A real place's map pack knows where its pilot box is.
    let home = arg("--home").and_then(|s| Home::parse(&s)).or_else(|| {
        terrain::real().map(|r| Home { lat: r.meta.home_lat, lon: r.meta.home_lon, alt: r.meta.home_alt })
    });
    let watcher = Arc::new(Mutex::new(Watcher::new(home)));
    let source = match arg("--tcp") {
        Some(addr) => {
            spawn_tcp(addr.clone(), watcher.clone());
            format!("TCP {addr}")
        }
        None => {
            let port: u16 = arg("--udp").and_then(|p| p.parse().ok()).unwrap_or(14550);
            spawn_udp(port, watcher.clone());
            format!("UDP port {port}")
        }
    };

    let views: Vec<(String, View)> = args
        .iter()
        .enumerate()
        .filter(|(_, a)| *a == "--at")
        .filter_map(|(i, _)| args.get(i + 1))
        .map(|s| {
            let (name, v) = s.split_once('=').unwrap_or(("view", s));
            (name.to_string(), View::parse(v).unwrap_or_else(|| panic!("--at {s}: expected NAME=x,agl,z,tx,tagl,tz[,fov]")))
        })
        .collect();

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: format!("SIM001 view - {}", terrain::site_title()),
            resolution: WindowResolution::new(1600, 900),
            present_mode: PresentMode::AutoVsync,
            ..default()
        }),
        ..default()
    }))
    .add_plugins(EnginePlugin { raytracing: flag("--raytrace"), tips: vec!["Waiting for the scenery. The plane appears when its telemetry arrives."], ..default() })
    .insert_resource(Telemetry(watcher))
    .insert_resource(Source(source))
    .insert_resource(Parked(arg("--plane").map(|p| parked_pose(&p))))
    .add_systems(Startup, spawn_status)
    .add_systems(Update, (keys, publish).before(EngineSet::Draw))
    .add_systems(Update, update_status);
    if let Some(dir) = arg("--capture") {
        capture::plugin(&mut app, capture::Capture { dir: dir.into(), views, settle_secs: 4.0 });
    }
    app.run();
}

fn spawn_udp(port: u16, watcher: Arc<Mutex<Watcher>>) {
    let socket = UdpSocket::bind(("0.0.0.0", port)).unwrap_or_else(|e| panic!("UDP port {port}: {e}"));
    std::thread::spawn(move || {
        let mut buf = [0u8; 2048];
        while let Ok(n) = socket.recv(&mut buf) {
            watcher.lock().unwrap().push(&buf[..n]);
        }
    });
}

/// Connects, and connects again whenever the link drops.
fn spawn_tcp(addr: String, watcher: Arc<Mutex<Watcher>>) {
    std::thread::spawn(move || {
        loop {
            if let Ok(mut s) = TcpStream::connect(&addr) {
                let mut buf = [0u8; 4096];
                while let Ok(n) = s.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    watcher.lock().unwrap().push(&buf[..n]);
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    });
}

/// The pilot box on the ground: where home is in the view.
fn pilot_box() -> Vec3 {
    let p = terrain::pilot_pos();
    Vec3::new(p.x, terrain::ground_height(p.x, p.z), p.z)
}

/// `x,agl,z,heading,roll`: east, height above the ground, south, degrees clockwise from north,
/// degrees of right bank.
fn parked_pose(s: &str) -> Pose {
    let n: Vec<f32> = s.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    assert!(n.len() >= 3, "--plane {s}: expected x,agl,z[,heading,roll,tilt]");
    let (heading, roll) = (n.get(3).copied().unwrap_or(0.0).to_radians(), n.get(4).copied().unwrap_or(0.0).to_radians());
    let tilt = n.get(5).copied().unwrap_or(0.0).to_radians();
    let ground = terrain::ground_height(n[0], n[2]);
    Pose { pos: Vec3::new(n[0], ground + n[1], n[2]), rot: Quat::from_rotation_y(-heading) * Quat::from_rotation_z(-roll), spool: vec![0.6, 0.6, 0.6], tilt: vec![tilt, tilt, 0.0], ..default() }
}

fn publish(telemetry: Res<Telemetry>, parked: Res<Parked>, mut scene: ResMut<Scene>) {
    if let Some(p) = &parked.0 {
        scene.vehicles[0] = Some(p.clone());
        return;
    }
    let placed = telemetry.0.lock().unwrap().placement();
    scene.vehicles[0] = placed.map(|p| Pose { pos: pilot_box() + p.offset, rot: p.rot, ..default() });
}

fn keys(keys: Res<ButtonInput<KeyCode>>, mut view: ResMut<ViewInput>, mut exit: MessageWriter<AppExit>) {
    view.cycle |= keys.just_pressed(KeyCode::KeyC);
    view.toggle_auto_zoom |= keys.just_pressed(KeyCode::KeyZ);
    if keys.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight, KeyCode::ControlLeft, KeyCode::ControlRight]) && keys.just_pressed(KeyCode::KeyQ) {
        exit.write(AppExit::Success);
    }
}

fn spawn_status(mut commands: Commands) {
    commands.spawn((
        Status,
        Text::new(""),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::WHITE),
        Node { position_type: PositionType::Absolute, left: px(12.0), bottom: px(10.0), ..default() },
    ));
}

fn update_status(telemetry: Res<Telemetry>, source: Res<Source>, cam: Res<visualization_engine::camera::CamState>, mut text: Query<&mut Text, With<Status>>) {
    let w = telemetry.0.lock().unwrap();
    let line = match w.placement() {
        None => format!("Waiting for telemetry on {}", source.0),
        Some(p) => format!("{}  |  {:.0} m north, {:.0} m east, {:.0} m up  |  C: view", cam.mode.label(), -p.offset.z, p.offset.x, p.offset.y),
    };
    for mut t in &mut text {
        if t.0 != line {
            t.0 = line.clone();
        }
    }
}
