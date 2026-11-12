//! The battlefield (`teleoperation_shared::battlefield`): the field as an operator's camera sees
//! it, rendered offscreen and sent while the app that steers the camera is connected: the ground
//! station from the sim or from `sim001_view`, or each team's commander app from the game's
//! `towers_view`. Each picture goes back with the eye that it was rendered from, so the app draws
//! its map on it in the right place.
//!
//! With `hide_vehicles`, the picture has no planes in it: they are on a render layer of their own,
//! which the battlefield cameras do not see, so a game's picture shows nothing that a team's fog
//! hides. The app draws the vehicles itself.

use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};
use bevy::render::render_resource::{Extent3d, TextureFormat, TextureUsages};
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use teleoperation_shared::battlefield::{self, Eye, Frame, MAX_SIZE, Request, write_frame};
use visualization_engine::camera::scene_look;
use visualization_engine::loading::AppState;
use visualization_engine::model::PlaneRoot;

mod outbox;

use outbox::Outbox;

/// The layer of the planes, when they are hidden: every camera but the battlefields' sees it.
const DRONES: usize = 1;
const IDLE_SIZE: UVec2 = UVec2::new(64, 36);

#[derive(Default)]
struct Shared {
    clients: AtomicUsize,
    request: Mutex<Option<Request>>,
    latest: Mutex<Option<(Frame, Arc<Vec<u8>>)>>,
    new_frame: Condvar,
}

/// The battlefields: `slots` of them, each on `battlefield::port(first_feed_port, slot)` (a game
/// has one for each team, the ground station one).
pub struct BattlefieldPlugin {
    pub first_feed_port: u16,
    pub slots: usize,
    pub hide_vehicles: bool,
    /// Where the origin of the field's frame (north, east, up) is in the world: the game's field
    /// centre, or the ground station's home (the pilot box).
    pub origin: Vec3,
}

/// The world point of the field's origin.
#[derive(Resource, Clone, Copy)]
struct Origin(Vec3);

impl Plugin for BattlefieldPlugin {
    fn build(&self, app: &mut App) {
        let links = serve(self.first_feed_port, self.slots);
        let slots = self.slots;
        app.insert_resource(links)
            .insert_resource(Origin(self.origin))
            .add_systems(Startup, move |commands: Commands, images: ResMut<Assets<Image>>| {
                spawn(commands, images, slots)
            })
            .add_systems(Update, (update, log_frames).after(visualization_engine::EngineSet::Draw));
        if self.hide_vehicles {
            app.add_systems(Update, layers);
        }
    }
}

/// The servers of the battlefields, by slot.
#[derive(Resource, Clone)]
struct Links(Vec<Arc<Shared>>);

fn serve(first_feed_port: u16, slots: usize) -> Links {
    let links: Vec<Arc<Shared>> = (0..slots).map(|_| Arc::new(Shared::default())).collect();
    for (slot, shared) in links.iter().enumerate() {
        let (shared, port) = (shared.clone(), battlefield::port(first_feed_port, slot));
        std::thread::Builder::new()
            .name(format!("battlefield-{slot}"))
            .spawn(move || listen(shared, port))
            .expect("spawn a battlefield server");
    }
    Links(links)
}

fn listen(shared: Arc<Shared>, port: u16) {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("battlefield: cannot listen on 127.0.0.1:{port}: {e}");
            return;
        }
    };
    println!("battlefield on tcp://127.0.0.1:{port}");
    for stream in listener.incoming().flatten() {
        let shared = shared.clone();
        std::thread::spawn(move || {
            shared.clients.fetch_add(1, Ordering::Relaxed);
            let err = viewer(&shared, stream);
            shared.clients.fetch_sub(1, Ordering::Relaxed);
            println!("battlefield on {port}: the app left ({err})");
        });
    }
}

fn viewer(shared: &Arc<Shared>, mut stream: TcpStream) -> String {
    stream.set_nodelay(true).ok();
    // The app's requests: the newest one counts.
    if let Ok(mut rd) = stream.try_clone() {
        let shared = shared.clone();
        std::thread::spawn(move || {
            let mut b = [0u8; Request::SIZE];
            while rd.read_exact(&mut b).is_ok() {
                if let Some(r) = Request::decode(&b) {
                    *shared.request.lock().unwrap() = Some(r);
                }
            }
        });
    }
    let mut last = 0u32;
    loop {
        let (frame, pixels) = {
            let mut latest = shared.latest.lock().unwrap();
            loop {
                if let Some((f, p)) = latest.as_ref().filter(|(f, _)| f.frame != last) {
                    break (*f, p.clone());
                }
                let (guard, _) = shared.new_frame.wait_timeout(latest, Duration::from_secs(1)).unwrap();
                latest = guard;
            }
        };
        last = frame.frame;
        if let Err(e) = write_frame(&mut stream, &frame, &pixels) {
            return e.to_string();
        }
    }
}

#[derive(Component)]
struct BattlefieldCamera(usize);

/// A mesh whose render layers have been worked out.
#[derive(Component)]
struct Layered;

struct Cam {
    image: Handle<Image>,
    /// Which of its pictures go out.
    outbox: Outbox,
    /// Pictures sent since the last log line.
    sent: u32,
}

/// How often the view logs its frame rate while an app is connected.
const LOG_EVERY: Duration = Duration::from_secs(5);

/// The frames rendered since the last log line, and when that was.
#[derive(Resource)]
struct FrameLog {
    since: Instant,
    frames: u32,
}

#[derive(Resource)]
struct Cams(Vec<Cam>);

fn spawn(mut commands: Commands, mut images: ResMut<Assets<Image>>, slots: usize) {
    let mut cams = Vec::new();
    for i in 0..slots {
        let mut target = Image::new_target_texture(IDLE_SIZE.x, IDLE_SIZE.y, TextureFormat::Bgra8UnormSrgb, None);
        target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        let image = images.add(target);
        commands.spawn((
            BattlefieldCamera(i),
            Camera3d::default(),
            Camera { order: -20 - i as isize, ..default() },
            RenderTarget::Image(image.clone().into()),
            Projection::Perspective(PerspectiveProjection { near: 1.0, ..default() }),
            Transform::default(),
            RenderLayers::layer(0),
            scene_look(),
        ));
        cams.push(Cam { image, outbox: Outbox::default(), sent: 0 });
    }
    commands.insert_resource(Cams(cams));
    commands.insert_resource(FrameLog { since: Instant::now(), frames: 0 });
}

/// The planes go on a layer of their own, which every camera sees but the battlefields'.
#[allow(clippy::type_complexity)]
fn layers(
    mut commands: Commands,
    meshes: Query<Entity, (With<Mesh3d>, Without<Layered>)>,
    cameras: Query<Entity, (With<Camera3d>, Without<RenderLayers>, Without<BattlefieldCamera>)>,
    parents: Query<&ChildOf>,
    planes: Query<(), With<PlaneRoot>>,
) {
    for e in &meshes {
        let on_a_plane = parents.iter_ancestors(e).any(|a| planes.contains(a));
        let mut ec = commands.entity(e);
        ec.insert(Layered);
        if on_a_plane {
            ec.insert(RenderLayers::layer(DRONES));
        }
    }
    for e in &cameras {
        commands.entity(e).insert(RenderLayers::from_layers(&[0, DRONES]));
    }
}

/// The world frame of the view (x east, y up, z south) of a field point (north, east, up).
fn world(p: glam::Vec3) -> Vec3 {
    Vec3::new(p.y, p.z, -p.x)
}

fn place(eye: &Eye, origin: Vec3) -> Transform {
    let (forward, _, up) = eye.axes();
    Transform::from_translation(origin + world(eye.position())).looking_to(world(forward), world(up))
}

/// Each battlefield camera follows the app's requests, and renders full size only while the app
/// is connected.
fn update(
    mut commands: Commands,
    links: Res<Links>,
    mut cams: ResMut<Cams>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<(&BattlefieldCamera, &mut Transform, &mut Projection)>,
    state: Res<State<AppState>>,
    origin: Res<Origin>,
) {
    // Until the valley has loaded, the world is a placeholder: no picture of it goes out.
    let loaded = *state.get() == AppState::Running;
    for (BattlefieldCamera(i), mut t, mut projection) in &mut cameras {
        let shared = &links.0[*i];
        let request = *shared.request.lock().unwrap();
        let live = loaded && shared.clients.load(Ordering::Relaxed) > 0 && request.is_some();
        let cam = &mut cams.0[*i];
        let want = match request.filter(|_| live) {
            Some(r) => {
                *t = place(&r.eye, origin.0);
                if let Projection::Perspective(p) = projection.as_mut() {
                    p.fov = r.eye.vfov.to_radians();
                }
                UVec2::new(r.width.clamp(16, MAX_SIZE.0) as u32, r.height.clamp(16, MAX_SIZE.1) as u32)
            }
            None => IDLE_SIZE,
        };
        let handle = cam.image.clone();
        if images.get(&handle).is_some_and(|img| img.size() != want)
            && let Some(mut img) = images.get_mut(&handle)
        {
            img.resize(Extent3d { width: want.x, height: want.y, depth_or_array_layers: 1 });
        }
        // Each frame's picture is read back with the request it was rendered for: the readback is
        // taken with this frame, and copied after its render. So a picture's camera is known, and a
        // picture goes out while the camera moves.
        if let Some(r) = request.filter(|_| live) {
            let i = *i;
            commands.spawn(ReadbackOnce::texture(handle)).observe(
                move |ev: On<ReadbackComplete>, links: Res<Links>, mut cams: ResMut<Cams>, mut commands: Commands| {
                    on_readback(i, &r, &ev, &links, &mut cams);
                    // A one-shot readback keeps its entity: it goes with its picture.
                    commands.entity(ev.entity).despawn();
                },
            );
        }
    }
}

/// While an app is connected, logs the view's frame rate and the pictures it sent, every few seconds.
fn log_frames(links: Res<Links>, mut cams: ResMut<Cams>, mut log: ResMut<FrameLog>) {
    log.frames += 1;
    if log.since.elapsed() >= LOG_EVERY {
        let secs = log.since.elapsed().as_secs_f32();
        if links.0.iter().any(|s| s.clients.load(Ordering::Relaxed) > 0) {
            let sent: Vec<String> = cams.0.iter().map(|c| format!("{:.0}", c.sent as f32 / secs)).collect();
            println!(
                "battlefield: the 3D view renders {:.0} frames a second ({:.1} ms a frame); pictures sent a second, by slot: {}",
                log.frames as f32 / secs,
                1000.0 * secs / log.frames.max(1) as f32,
                sent.join(", ")
            );
        }
        for c in &mut cams.0 {
            c.sent = 0;
        }
        *log = FrameLog { since: Instant::now(), frames: 0 };
    }
}

/// The picture rendered for `r` is back: it goes out if the outbox says so.
fn on_readback(i: usize, r: &Request, ev: &ReadbackComplete, links: &Links, cams: &mut Cams) {
    let cam = &mut cams.0[i];
    let Some(frame) = cam.outbox.send(r, ev.data.len(), Instant::now()) else {
        return;
    };
    cam.sent += 1;
    let shared = &links.0[i];
    *shared.latest.lock().unwrap() = Some((frame, Arc::new(ev.data.clone())));
    shared.new_frame.notify_all();
}
