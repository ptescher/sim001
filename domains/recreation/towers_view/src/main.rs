//! towers_view: the 3D view of a game of Towers, and the camera of each drone.
//!
//!   cargo run --release -p recreation_towers_view -- [--server ADDR] [--feed-port N] [--feed-size WxH]
//!                                                    [--stop-with-stdin]
//!   cargo run --release -p recreation_towers_view -- --export-map DIR
//!
//! The view takes the seat of a spectator on the game server (default 127.0.0.1:5955), draws the
//! valley (forest, lanes, lake) with the six drones where the game has them, and sends the picture
//! of each drone's camera on the sensor feed of the sim (`teleoperation_shared::feed`): drone `i`
//! (Blue's three, then Red's) on TCP port `--feed-port + i` (default 5601). A camera renders only
//! while someone watches its feed. With `--stop-with-stdin` the view quits when its input closes
//! (the commander app starts it so, and it stops with the app).
//!
//! `--export-map DIR` writes the top-down map of the valley (`terrain_map.png`, `terrain_height.bin`
//! and `terrain_map.txt`, as `sim001 --export-map` writes the sim's) and exits. It is the ground of
//! this view, drawn from the same trees and lake, in metres north and east of the field's centre
//! (SITL's home), so the commander app and the ground station draw it where the game has it.

mod feeds;
mod valley;

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution};
use recreation_towers::seat::{self, Join, ToSeat, ToServer, Truth};
use recreation_towers::{Class, Phase};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use visualization_engine::{EnginePlugin, EngineSet, Pose, Scene, camera};

/// The latest whole game from the server.
#[derive(Resource, Clone, Default)]
pub struct Game(pub Arc<Mutex<Option<Truth>>>);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!(
            "{}",
            include_str!("main.rs")
                .lines()
                .take_while(|l| l.starts_with("//!"))
                .map(|l| l.trim_start_matches("//!").trim_start_matches(' '))
                .collect::<Vec<_>>()
                .join("\n")
        );
        return;
    }
    let server = arg("--server").unwrap_or_else(|| format!("127.0.0.1:{}", seat::PORT));
    let first_port = arg("--feed-port").and_then(|p| p.parse().ok()).unwrap_or(feeds::FIRST_PORT);
    let size = arg("--feed-size")
        .and_then(|s| s.split_once('x').map(|(w, h)| (w.parse().ok(), h.parse().ok())))
        .and_then(|(w, h)| Some(UVec2::new(w?, h?)))
        .unwrap_or(UVec2::new(320, 240));

    if let Err(e) = valley::set(&recreation_towers::Rules::default()) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    if let Some(dir) = arg("--export-map") {
        visualization_engine::mapgen::export(&dir);
        return;
    }
    if args.iter().any(|a| a == "--stop-with-stdin") {
        std::thread::spawn(|| {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
            std::process::exit(0);
        });
    }
    let game = Game::default();
    spawn_spectator(server, game.0.clone());
    let links = feeds::serve(first_port);

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: format!("SIM001 Towers - {}", visualization_world::terrain::site_title()),
            resolution: WindowResolution::new(1280, 720),
            present_mode: PresentMode::AutoVsync,
            ..default()
        }),
        ..default()
    }))
    // The pictures of the cameras do not need the window on screen: the UI is drawn offscreen.
    .insert_resource(camera::UiOffscreen(UVec2::new(1280, 720)))
    .add_plugins(EnginePlugin {
        raytracing: false,
        liveries: (0..6)
            .map(|i| if i < 3 { Color::srgb(0.10, 0.42, 1.0) } else { Color::srgb(1.0, 0.18, 0.12) })
            .collect(),
        tips: vec!["Waiting for the valley. The drones appear when the game server sends them."],
    })
    .insert_resource(game)
    .insert_resource(links)
    .add_systems(Update, publish.before(EngineSet::Draw));
    feeds::plugin(&mut app, size);
    app.run();
}

/// A spectator seat on the game server: reads the whole game, and reconnects when it is lost.
fn spawn_spectator(addr: String, game: Arc<Mutex<Option<Truth>>>) {
    std::thread::spawn(move || {
        loop {
            if let Ok(stream) = TcpStream::connect(&addr)
                && let Ok(mut out) = stream.try_clone()
                && writeln!(out, "{}", seat::encode(&ToServer::Join(Join::Spectate))).is_ok()
            {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    if let Ok(ToSeat::Truth(t)) = seat::decode::<ToSeat>(&line) {
                        *game.lock().unwrap() = Some(t);
                    }
                }
            }
            *game.lock().unwrap() = None;
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

/// Where drone `i` is in the view's world (x east, y up, z south), and how it sits.
pub fn pose_of(t: &Truth, i: usize) -> Option<Pose> {
    let d = t.drones.get(i)?;
    if d.at.length() > 5_000.0 {
        return None;
    }
    let rot = t.attitudes.get(i).map_or(Quat::IDENTITY, |q| Quat::from_xyzw(q[0], q[1], q[2], q[3]));
    // Motors up on the ground and in a hover, forward in flight.
    let up = d.phase != Phase::Flying || d.vel.length() < 8.0;
    let tilt = if up { std::f32::consts::FRAC_PI_2 } else { 0.0 };
    let spool = if d.phase == Phase::Flying { 0.6 } else { 0.0 };
    // The light drone is a quadcopter: four motors that always point up.
    let (spool, tilt) = match d.class {
        Class::Light => (vec![spool; 4], vec![]),
        _ => (vec![spool; 3], vec![tilt, tilt, 0.0]),
    };
    Some(Pose {
        pos: Vec3::new(d.at.y, d.alt, -d.at.x),
        rot,
        spool,
        tilt,
        crashed: d.phase == Phase::Down,
        ..default()
    })
}

fn publish(game: Res<Game>, mut scene: ResMut<Scene>) {
    let truth = game.0.lock().unwrap();
    for i in 0..scene.vehicles.len() {
        scene.vehicles[i] = truth.as_ref().and_then(|t| pose_of(t, i));
    }
}
