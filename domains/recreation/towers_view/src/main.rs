//! towers_view: the 3D view of a game of Towers, and the camera of each drone.
//!
//!   cargo run --release -p recreation_towers_view -- [--server ADDR] [--feed-port N] [--feed-size WxH]
//!                                                    [--stop-with-stdin] [--hidden]
//!   cargo run --release -p recreation_towers_view -- --export-map DIR
//!   cargo run --release -p recreation_towers_view -- --capture DIR --at NAME=x,agl,z,tx,tagl,tz[,fov] ...
//!
//! The view takes the seat of a spectator on the game server (default 127.0.0.1:5955), draws the
//! valley (forest, lanes, lake) with the six drones where the game has them, and sends the picture
//! of each drone's camera on the sensor feed of the sim (`teleoperation_shared::feed`): drone `i`
//! (Blue's three, then Red's) on TCP port `--feed-port + i` (default 5601). A camera renders only
//! while someone watches its feed. The battlefield of each team, the valley from the camera of
//! its commander with no drones in it, is on `--feed-port + 20` (Blue) and `+ 21` (Red); see
//! `visualization_battlefield`. With `--stop-with-stdin` the view quits when its input closes
//! (the commander app starts it so, and it stops with the app). With `--hidden` its window is not
//! on screen (the commander app starts it so too: a window on top of the app's slows the app).
//!
//! `--export-map DIR` writes the top-down map of the valley (`terrain_map.png`, `terrain_height.bin`
//! and `terrain_map.txt`, as `sim001 --export-map` writes the sim's) and exits. It is the ground of
//! this view, drawn from the same trees and lake, in metres north and east of the field's centre
//! (SITL's home), so the commander app and the ground station draw it where the game has it.
//!
//! `--capture DIR` saves a picture of the valley from each `--at` view as `DIR/NAME.png` and
//! exits, as `sim001_view --capture` does (see the visualization domain's README).

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
    let hidden = args.iter().any(|a| a == "--hidden");
    app.add_plugins(DefaultPlugins.set(WindowPlugin { primary_window: Some(window(hidden)), ..default() }))
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
    park_if_hidden(&mut app, hidden);
    if let Some(dir) = arg("--capture") {
        visualization_engine::capture::plugin(
            &mut app,
            visualization_engine::capture::Capture {
                dir: dir.into(),
                views: visualization_engine::capture::views_from_args(&args),
                settle_secs: std::env::var("SIM001_SETTLE").ok().and_then(|v| v.parse().ok()).unwrap_or(4.0),
            },
        );
    }
    // The battlefield of each team, with no drones in it: the fog hides them from the commander.
    app.add_plugins(visualization_battlefield::BattlefieldPlugin {
        first_feed_port: first_port,
        slots: 2,
        hide_vehicles: true,
        origin: Vec3::ZERO,
    });
    app.run();
}

/// With `--hidden` nobody sees the view's own camera: it draws a tiny picture of the sky
/// (`camera::RenderScale::parked`), and the GPU is free for the pictures the app shows.
fn park_if_hidden(app: &mut App, hidden: bool) {
    if hidden {
        app.add_systems(PostStartup, |mut rs: ResMut<camera::RenderScale>| rs.parked = true);
    }
}

/// The view's window. With `--hidden` it is not on screen: the commander app starts the view so,
/// because the app is its screen, and a window on top of the app's makes macOS throttle the app's
/// drawing. The cameras draw into images either way.
fn window(hidden: bool) -> Window {
    Window {
        title: format!("SIM001 Towers - {}", visualization_world::terrain::site_title()),
        resolution: WindowResolution::new(1280, 720),
        present_mode: PresentMode::AutoVsync,
        visible: !hidden,
        ..default()
    }
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

#[cfg(test)]
mod window_tests {
    use super::*;

    #[test]
    fn with_hidden_the_view_has_no_window_on_screen() {
        assert!(!window(true).visible);
        assert!(window(false).visible);
    }

    #[test]
    fn with_hidden_the_main_camera_is_parked() {
        for hidden in [true, false] {
            let mut app = App::new();
            app.insert_resource(camera::RenderScale {
                scale: 1.0,
                auto: false,
                image: Handle::default(),
                suspended: false,
                upscale: false,
                parked: false,
            });
            park_if_hidden(&mut app, hidden);
            app.update();
            assert_eq!(app.world().resource::<camera::RenderScale>().parked, hidden);
        }
    }
}
