//! The camera of each drone, on the sensor feed of the sim (`teleoperation_shared::feed`): a
//! camera under the nose of the drone, rendered offscreen and sent as raw frames over TCP while a
//! viewer is connected. Drone `i` is on port `FIRST_PORT + i`.

use crate::Game;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::{Extent3d, TextureFormat, TextureUsages};
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use teleoperation_shared::feed::{ClientMsg, FORMAT_BGRA8, FrameHeader, Gimbal, write_frame};
use visualization_engine::Scene;
use visualization_engine::camera::{RenderScale, scene_look};
use visualization_engine::optics::SENSOR_MOUNT;

/// Drone `i`'s feed is on this port plus `i`.
pub const FIRST_PORT: u16 = 5601;
/// Blue's three drones, then Red's.
pub const DRONES: usize = 6;
/// A camera nobody watches renders this small.
const IDLE_SIZE: UVec2 = UVec2::new(64, 36);
const MAX_FPS: f64 = 30.0;
/// The camera looks a little down, so that a drone flying low sees the lane in front of it.
const TILT_DEG: f32 = -6.0;

struct Frame {
    header: FrameHeader,
    pixels: Arc<Vec<u8>>,
}

#[derive(Default)]
struct Shared {
    clients: AtomicUsize,
    latest: Mutex<Option<Frame>>,
    new_frame: Condvar,
}

/// The feeds' servers, one for each drone.
#[derive(Resource, Clone)]
pub struct Links(Vec<Arc<Shared>>);

/// Starts the servers.
pub fn serve(first_port: u16) -> Links {
    let links: Vec<Arc<Shared>> = (0..DRONES).map(|_| Arc::new(Shared::default())).collect();
    for (i, shared) in links.iter().enumerate() {
        let shared = shared.clone();
        let port = first_port + i as u16;
        std::thread::Builder::new()
            .name(format!("feed-{i}"))
            .spawn(move || listen(shared, port))
            .expect("spawn a feed server");
    }
    Links(links)
}

fn listen(shared: Arc<Shared>, port: u16) {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("drone feed: cannot listen on 127.0.0.1:{port}: {e}");
            return;
        }
    };
    println!("drone feed on tcp://127.0.0.1:{port}");
    for stream in listener.incoming().flatten() {
        let shared = shared.clone();
        std::thread::spawn(move || {
            shared.clients.fetch_add(1, Ordering::Relaxed);
            let err = viewer(&shared, stream);
            shared.clients.fetch_sub(1, Ordering::Relaxed);
            println!("drone feed on {port}: viewer left ({err})");
        });
    }
}

fn viewer(shared: &Arc<Shared>, mut stream: TcpStream) -> String {
    stream.set_nodelay(true).ok();
    // The viewer may send gimbal commands, its status and keys: this camera is fixed to the drone.
    if let Ok(mut rd) = stream.try_clone() {
        std::thread::spawn(move || {
            let mut b = [0u8; ClientMsg::SIZE];
            while rd.read_exact(&mut b).is_ok() {}
        });
    }
    let mut last = 0u32;
    loop {
        // Always send the newest frame; a slow viewer just skips frames.
        let (header, pixels) = {
            let mut latest = shared.latest.lock().unwrap();
            loop {
                if let Some(f) = latest.as_ref().filter(|f| f.header.frame != last) {
                    break (f.header, f.pixels.clone());
                }
                let (guard, _) = shared.new_frame.wait_timeout(latest, Duration::from_secs(1)).unwrap();
                latest = guard;
            }
        };
        last = header.frame;
        if let Err(e) = write_frame(&mut stream, &header, &pixels) {
            return e.to_string();
        }
    }
}

struct Cam {
    image: Handle<Image>,
    readback: Option<Entity>,
    frame: u32,
    last_sent: Option<Instant>,
}

#[derive(Resource)]
struct Cams {
    size: UVec2,
    cams: Vec<Cam>,
}

#[derive(Component)]
struct DroneCamera(usize);

/// The cameras, from the start.
pub fn plugin(app: &mut App, size: UVec2) {
    app.add_systems(Startup, move |commands: Commands, images: ResMut<Assets<Image>>| spawn(commands, images, size))
        .add_systems(Update, update.after(visualization_engine::EngineSet::Draw));
}

fn spawn(mut commands: Commands, mut images: ResMut<Assets<Image>>, size: UVec2) {
    let mut cams = Vec::new();
    for i in 0..DRONES {
        // BGRA8 sRGB is what the viewers' renderers take, so frames go out untouched.
        let mut target = Image::new_target_texture(IDLE_SIZE.x, IDLE_SIZE.y, TextureFormat::Bgra8UnormSrgb, None);
        target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        let image = images.add(target);
        commands.spawn((
            DroneCamera(i),
            Camera3d::default(),
            Camera { order: -1 - i as isize, ..default() },
            RenderTarget::Image(image.clone().into()),
            Projection::Perspective(PerspectiveProjection {
                fov: vfov(teleoperation_shared::feed::HFOV_DEFAULT, size).to_radians(),
                near: 0.02,
                ..default()
            }),
            Transform::default(),
            scene_look(),
        ));
        cams.push(Cam { image, readback: None, frame: 0, last_sent: None });
    }
    commands.insert_resource(Cams { size, cams });
}

/// Vertical field of view for a horizontal one at the feed's aspect ratio.
fn vfov(hfov_deg: f32, size: UVec2) -> f32 {
    let aspect = size.y as f32 / size.x as f32;
    (2.0 * ((hfov_deg.to_radians() * 0.5).tan() * aspect).atan()).to_degrees()
}

/// Each camera follows its drone, and renders full size only while someone watches its feed.
fn update(
    mut commands: Commands,
    links: Res<Links>,
    scene: Res<Scene>,
    render_scale: Res<RenderScale>,
    mut cams: ResMut<Cams>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<(&DroneCamera, &mut Transform)>,
) {
    let size = cams.size;
    for (DroneCamera(i), mut t) in &mut cameras {
        if let Some(pose) = &scene.vehicles[*i] {
            t.translation = pose.pos + pose.rot * SENSOR_MOUNT;
            t.rotation = pose.rot * Quat::from_rotation_x(TILT_DEG.to_radians());
        }
        let live = links.0[*i].clients.load(Ordering::Relaxed) > 0 && !render_scale.suspended;
        let cam = &mut cams.cams[*i];
        let want = if live { size } else { IDLE_SIZE };
        let handle = cam.image.clone();
        if images.get(&handle).is_some_and(|img| img.size() != want)
            && let Some(mut img) = images.get_mut(&handle)
        {
            img.resize(Extent3d { width: want.x, height: want.y, depth_or_array_layers: 1 });
        }
        match (live, cam.readback) {
            (true, None) => {
                let i = *i;
                let e = commands
                    .spawn(Readback::texture(handle))
                    .observe(
                        move |ev: On<ReadbackComplete>, links: Res<Links>, game: Res<Game>, mut cams: ResMut<Cams>| {
                            on_readback(i, &ev, &links, &game, &mut cams)
                        },
                    )
                    .id();
                cam.readback = Some(e);
            }
            (false, Some(e)) => {
                commands.entity(e).despawn();
                cam.readback = None;
            }
            _ => {}
        }
    }
}

fn on_readback(i: usize, ev: &ReadbackComplete, links: &Links, game: &Game, cams: &mut Cams) {
    let size = cams.size;
    let cam = &mut cams.cams[i];
    if cam.last_sent.is_some_and(|t| t.elapsed().as_secs_f64() < 1.0 / MAX_FPS - 0.004) {
        return;
    }
    let (w, h) = (size.x, size.y);
    // Texture readbacks pad each row to 256 bytes.
    let stride = (w * 4).div_ceil(256) * 256;
    if ev.data.len() != (stride * h) as usize {
        return; // a frame from before a resize
    }
    cam.last_sent = Some(Instant::now());
    cam.frame = cam.frame.wrapping_add(1);
    let sim_time = game.0.lock().unwrap().as_ref().map_or(0.0, |t| t.time);
    let header = FrameHeader {
        format: FORMAT_BGRA8,
        width: w as u16,
        height: h as u16,
        resets: 0,
        stride,
        frame: cam.frame,
        sim_time,
        gimbal: Gimbal { pan: 0.0, tilt: TILT_DEG, hfov: teleoperation_shared::feed::HFOV_DEFAULT },
        flags: 0,
        arena: None,
        game: None,
    };
    let shared = &links.0[i];
    *shared.latest.lock().unwrap() = Some(Frame { header, pixels: Arc::new(ev.data.clone()) });
    shared.new_frame.notify_all();
}
