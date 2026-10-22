//! Sensor camera: a gimballed camera under the nose, rendered offscreen and streamed to the
//! ground station over TCP (see `teleoperation_shared::feed`) while a viewer is connected.
//!
//!   cargo run --release --bin sim001 -- --ardupilot --feed          serve on 127.0.0.1:5600
//!
//! The viewer steers the gimbal (pan / tilt / zoom); the gimbal slews at a finite rate, like a
//! real sensor ball. With no viewer the camera renders a tiny frame and nothing is read back.

use crate::SharedSim;
use crate::camera::{RenderScale, add_raytracing, scene_look};
use crate::sim::SimCore;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::{Extent3d, TextureFormat, TextureUsages};
use recreation_games::Kind;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use teleoperation_shared::feed::{
    Arena, ClientMsg, FLAG_DIRECT, FORMAT_BGRA8, FrameHeader, GameClock, GameKind, GameStatus, Gimbal, write_frame,
};
use visualization_engine::RenderSettings;

/// Camera position in the airframe (Bevy body axes: +X right, +Y up, -Z forward): the lens of the
/// gimbal hanging under the front of the pod, on the surface of its ball (see `optics`), so that
/// the camera looks out of the ball and never through the hull.
const MOUNT: Vec3 = visualization_engine::optics::SENSOR_MOUNT;
const IDLE_SIZE: UVec2 = UVec2::new(64, 36);
const MAX_FPS: f64 = 30.0;
/// Gimbal slew rate, degrees per second.
const SLEW: f32 = 120.0;

pub struct FeedConfig {
    pub port: u16,
    pub size: UVec2,
}

struct Frame {
    header: FrameHeader,
    pixels: Arc<Vec<u8>>,
}

#[derive(Default)]
struct Shared {
    clients: AtomicUsize,
    latest: Mutex<Option<Frame>>,
    new_frame: Condvar,
    commanded: Mutex<Option<Gimbal>>,
}

#[derive(Resource, Clone)]
struct FeedLink(Arc<Shared>);

#[derive(Component)]
struct FeedCamera;

#[derive(Resource)]
struct FeedState {
    image: Handle<Image>,
    size: UVec2,
    readback: Option<Entity>,
    gimbal: Gimbal,
    target: Gimbal,
    frame: u32,
    last_sent: Option<Instant>,
}

pub fn feed_plugin(app: &mut App, cfg: FeedConfig, core: Arc<Mutex<SimCore>>) {
    let shared = Arc::new(Shared::default());
    let server = shared.clone();
    let port = cfg.port;
    std::thread::Builder::new()
        .name("sensor-feed".into())
        .spawn(move || serve(server, core, port))
        .expect("spawn feed server");
    let size = cfg.size;
    app.insert_resource(FeedLink(shared))
        .add_systems(
            Startup,
            move |commands: Commands, images: ResMut<Assets<Image>>, settings: Res<RenderSettings>| {
                spawn_feed_camera(commands, images, settings, size)
            },
        )
        .add_systems(Update, update_feed.after(crate::SimSet::Logic));
}

fn spawn_feed_camera(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    settings: Res<RenderSettings>,
    size: UVec2,
) {
    // BGRA8 sRGB is what the ground station's renderer takes, so frames go out untouched.
    let mut target = Image::new_target_texture(IDLE_SIZE.x, IDLE_SIZE.y, TextureFormat::Bgra8UnormSrgb, None);
    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let image = images.add(target);
    let mut cam = commands.spawn((
        FeedCamera,
        Camera3d::default(),
        Camera { order: -1, ..default() },
        RenderTarget::Image(image.clone().into()),
        Projection::Perspective(PerspectiveProjection {
            fov: vfov(teleoperation_shared::feed::HFOV_DEFAULT, size).to_radians(),
            near: 0.02,
            ..default()
        }),
        Transform::default(),
        scene_look(),
    ));
    if settings.raytracing {
        add_raytracing(&mut cam);
        // Solari lights the whole scene with one sky cubemap, taken from the main camera, so
        // a second generated one would be pure cost (re-rendered and filtered every frame).
        cam.insert(crate::camera::feed_solari_settings()).remove::<bevy::light::AtmosphereEnvironmentMapLight>();
    }
    commands.insert_resource(FeedState {
        image,
        size,
        readback: None,
        gimbal: Gimbal::default(),
        target: Gimbal::default(),
        frame: 0,
        last_sent: None,
    });
}

/// Vertical field of view for a horizontal one at the feed's aspect ratio.
fn vfov(hfov_deg: f32, size: UVec2) -> f32 {
    let aspect = size.y as f32 / size.x as f32;
    (2.0 * ((hfov_deg.to_radians() * 0.5).tan() * aspect).atan()).to_degrees()
}

fn update_feed(
    mut commands: Commands,
    time: Res<Time>,
    link: Res<FeedLink>,
    sim: Res<SharedSim>,
    render_scale: Res<RenderScale>,
    mut st: ResMut<FeedState>,
    mut images: ResMut<Assets<Image>>,
    mut cam: Query<(&mut Transform, &mut Projection), With<FeedCamera>>,
) {
    let viewers = link.0.clients.load(Ordering::Relaxed);
    let live = viewers > 0 && !render_scale.suspended;

    // Render full size only while someone is watching (deactivating the camera would upset
    // the atmosphere probe, so idle means tiny instead).
    let want = if live { st.size } else { IDLE_SIZE };
    let handle = st.image.clone();
    if images.get(&handle).is_some_and(|img| img.size() != want)
        && let Some(mut img) = images.get_mut(&handle)
    {
        img.resize(Extent3d { width: want.x, height: want.y, depth_or_array_layers: 1 });
    }
    match (live, st.readback) {
        (true, None) => {
            let e = commands.spawn(Readback::texture(handle)).observe(on_readback).id();
            st.readback = Some(e);
        }
        (false, Some(e)) => {
            commands.entity(e).despawn();
            st.readback = None;
        }
        _ => {}
    }

    // Slew the gimbal toward the commanded attitude.
    if let Some(cmd) = *link.0.commanded.lock().unwrap() {
        st.target = cmd;
    }
    let dt = time.delta_secs().min(0.1);
    let step = |cur: f32, tgt: f32| cur + (tgt - cur).clamp(-SLEW * dt, SLEW * dt);
    let g = st.gimbal;
    st.gimbal = Gimbal {
        pan: step(g.pan, st.target.pan),
        tilt: step(g.tilt, st.target.tilt),
        // Zoom eases exponentially, about 0.3 s to settle.
        hfov: g.hfov + (st.target.hfov - g.hfov) * (1.0 - (-dt * 10.0).exp()),
    };

    let (pos, rot) = {
        let c = sim.0.lock().unwrap();
        (c.state.pos, c.state.rot)
    };
    let Ok((mut t, mut proj)) = cam.single_mut() else { return };
    t.translation = pos + rot * MOUNT;
    // Pan about the body's up axis (+ = right), then tilt about the ball's own right axis.
    t.rotation =
        rot * Quat::from_rotation_y(-st.gimbal.pan.to_radians()) * Quat::from_rotation_x(st.gimbal.tilt.to_radians());
    if let Projection::Perspective(p) = &mut *proj {
        p.fov = vfov(st.gimbal.hfov, st.size).to_radians();
    }
}

fn on_readback(ev: On<ReadbackComplete>, link: Res<FeedLink>, sim: Res<SharedSim>, mut st: ResMut<FeedState>) {
    if st.last_sent.is_some_and(|t| t.elapsed().as_secs_f64() < 1.0 / MAX_FPS - 0.004) {
        return;
    }
    let (w, h) = (st.size.x, st.size.y);
    // Texture readbacks pad each row to 256 bytes.
    let stride = (w * 4).div_ceil(256) * 256;
    if ev.data.len() != (stride * h) as usize {
        return; // a frame from before a resize
    }
    st.last_sent = Some(Instant::now());
    st.frame = st.frame.wrapping_add(1);
    let (sim_time, resets, flags, arena, game) = {
        let c = sim.0.lock().unwrap();
        let game = c.game.as_ref().map(|g| GameStatus {
            kind: if g.kind == Kind::Tag { GameKind::Tag } else { GameKind::Race },
            clock: match g.clock {
                recreation_games::Clock::Waiting => GameClock::Waiting,
                recreation_games::Clock::CountingDown { .. } => GameClock::CountingDown,
                recreation_games::Clock::Running { .. } => GameClock::Running,
                recreation_games::Clock::Stopped { .. } => GameClock::Stopped,
            },
        });
        // The sim's x is east and -z north.
        let arena = c.game.as_ref().filter(|g| g.kind == Kind::Tag).map(|g| Arena {
            north: -g.arena.centre.z,
            east: g.arena.centre.x,
            radius: g.arena.radius,
        });
        (c.state.time as f32, c.resets, if c.ap.enabled && c.direct { FLAG_DIRECT } else { 0 }, arena, game)
    };
    let header = FrameHeader {
        format: FORMAT_BGRA8,
        width: w as u16,
        height: h as u16,
        resets,
        stride,
        frame: st.frame,
        sim_time,
        gimbal: st.gimbal,
        flags,
        arena,
        game,
    };
    *link.0.latest.lock().unwrap() = Some(Frame { header, pixels: Arc::new(ev.data.clone()) });
    link.0.new_frame.notify_all();
}

fn serve(shared: Arc<Shared>, core: Arc<Mutex<SimCore>>, port: u16) {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("sensor feed: cannot listen on 127.0.0.1:{port}: {e}");
            return;
        }
    };
    println!("sensor feed on tcp://127.0.0.1:{port}");
    for stream in listener.incoming().flatten() {
        let shared = shared.clone();
        let core = core.clone();
        std::thread::spawn(move || {
            let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
            println!("sensor feed: viewer {peer} connected");
            shared.clients.fetch_add(1, Ordering::Relaxed);
            let err = viewer(&shared, core, stream);
            shared.clients.fetch_sub(1, Ordering::Relaxed);
            println!("sensor feed: viewer {peer} left ({err})");
        });
    }
}

fn viewer(shared: &Arc<Shared>, core: Arc<Mutex<SimCore>>, mut stream: TcpStream) -> String {
    stream.set_nodelay(true).ok();
    // Gimbal commands and the autopilot's status arrive on the same socket.
    if let Ok(mut rd) = stream.try_clone() {
        let shared = shared.clone();
        std::thread::spawn(move || {
            let mut b = [0u8; ClientMsg::SIZE];
            while rd.read_exact(&mut b).is_ok() {
                match ClientMsg::decode(&b) {
                    Ok(ClientMsg::Gimbal(g)) => *shared.commanded.lock().unwrap() = Some(g),
                    Ok(ClientMsg::Status(st)) => {
                        core.lock().unwrap().ap.gcs = Some((Instant::now(), st.armed, st.mode))
                    }
                    Ok(ClientMsg::Radio(r)) => core.lock().unwrap().test_radio = Some(r),
                    Ok(ClientMsg::Key(teleoperation_shared::feed::Key::Reset)) => core.lock().unwrap().reset(),
                    Ok(ClientMsg::Key(k)) => core.lock().unwrap().toggle_game(match k {
                        teleoperation_shared::feed::Key::Race => Kind::Race,
                        teleoperation_shared::feed::Key::Tag => Kind::Tag,
                        teleoperation_shared::feed::Key::Reset => unreachable!("handled above"),
                    }),
                    Err(_) => {}
                }
            }
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
