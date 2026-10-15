//! Sensor feed client: video frames from the sim's gimballed camera, gimbal commands back, and
//! ground-point tracking (the GCS re-aims the body-fixed gimbal from live attitude, ~30 Hz).

use crate::geo::{Origin, TerrainMap, ground_hit, look_angles, view_ray};
use crate::link::{Cmd, Shared};
use crate::vehicle::Vehicle;
use gpui::RenderImage;
use std::io::Write;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use t1s_station::feed::{FLAG_DIRECT, FrameHeader, GameStatus, Gimbal, GimbalCmd, Key, KeyPress, RadioState, VehicleStatus, read_frame};

pub struct VideoFrame {
    pub header: FrameHeader,
    pub image: Arc<RenderImage>,
    pub at: Instant,
}

/// Sensor point of interest: where the line of sight meets the ground.
#[derive(Clone, Copy, Debug)]
pub struct Spi {
    pub north: f32,
    pub east: f32,
    /// Ground height above the origin.
    pub height: f32,
    pub range: f32,
}

pub struct Sensor {
    pub address: String,
    pub frame: Mutex<Option<VideoFrame>>,
    pub connected: AtomicBool,
    pub status: Mutex<String>,
    pub fps: Mutex<f32>,
    /// Last gimbal command sent (the sim slews toward it).
    pub commanded: Mutex<Gimbal>,
    /// Ground point (north, east, height) the gimbal is kept on.
    pub track: Mutex<Option<(f32, f32, f32)>>,
    writer: Mutex<Option<TcpStream>>,
    vehicle: Shared,
    /// MAVLink commands: a sim reset restarts the autopilot.
    tx: Mutex<Sender<Cmd>>,
    origin: Origin,
    terrain: Option<Arc<TerrainMap>>,
}

impl Sensor {
    /// The sim's sticks are flying the aircraft (ArduPilot only watching), per the newest frame.
    pub fn direct(&self) -> bool {
        self.frame.lock().unwrap().as_ref().is_some_and(|f| f.header.flags & FLAG_DIRECT != 0 && f.at.elapsed().as_secs_f32() < 2.0)
    }

    /// Gimbal state of the newest frame (what the picture actually shows).
    pub fn gimbal(&self) -> Option<Gimbal> {
        self.frame.lock().unwrap().as_ref().map(|f| f.header.gimbal)
    }

    /// Tell a sim started with `--test-radio` what the stand-in radio is doing.
    pub fn send_radio(&self, r: RadioState) {
        if let Some(w) = self.writer.lock().unwrap().as_mut() {
            let _ = w.write_all(&r.encode());
        }
    }

    /// Press one of the sim's keys (G starts a race, T a game of tag).
    pub fn press_key(&self, k: Key) {
        if let Some(w) = self.writer.lock().unwrap().as_mut() {
            let _ = w.write_all(&KeyPress(k).encode());
        }
    }

    /// The game the sim is playing, per the newest frame.
    pub fn game(&self) -> Option<GameStatus> {
        self.frame.lock().unwrap().as_ref().filter(|f| f.at.elapsed().as_secs_f32() < 2.0).and_then(|f| f.header.game)
    }

    pub fn command(&self, g: Gimbal) {
        let g = g.clamped();
        *self.commanded.lock().unwrap() = g;
        if let Some(w) = self.writer.lock().unwrap().as_mut() {
            let _ = w.write_all(&GimbalCmd(g).encode());
        }
    }

    /// Manual slew: drops any ground track.
    pub fn slew(&self, g: Gimbal) {
        *self.track.lock().unwrap() = None;
        self.command(g);
    }

    pub fn track_point(&self, north: f32, east: f32) {
        let h = self.terrain.as_ref().and_then(|t| t.height(north, east)).unwrap_or(0.0);
        *self.track.lock().unwrap() = Some((north, east, h));
    }

    /// Aircraft (north, east, height above origin ground).
    fn aircraft(&self, v: &Vehicle) -> Option<(f32, f32, f32)> {
        v.have_position.then(|| {
            let (n, e) = self.origin.to_ne(v.lat, v.lon);
            (n, e, v.alt_msl - self.origin.alt)
        })
    }

    /// Ground point along a direction given in the camera's own frame: `u`, `v` in -1..1 across
    /// the picture (right, down). (0, 0) is the boresight.
    pub fn ground_at(&self, v: &Vehicle, g: Gimbal, aspect: f32, u: f32, w: f32) -> Option<Spi> {
        let ac = self.aircraft(v)?;
        let th = (g.hfov.to_radians() * 0.5).tan();
        let dir = view_ray(v.roll, v.pitch, v.yaw, g.pan, g.tilt, u * th, w * th * aspect);
        let (n, e, h, r) = ground_hit(self.terrain.as_deref(), ac, dir)?;
        Some(Spi { north: n, east: e, height: h, range: r })
    }

    /// Height above the sim's terrain.
    pub fn agl(&self, v: &Vehicle) -> Option<f32> {
        let (n, e, h) = self.aircraft(v)?;
        Some(h - self.terrain.as_ref()?.height(n, e)?)
    }

    pub fn spi(&self, v: &Vehicle) -> Option<Spi> {
        let g = self.gimbal()?;
        self.ground_at(v, g, 9.0 / 16.0, 0.0, 0.0)
    }
}

pub fn spawn(address: String, vehicle: Shared, tx: Sender<Cmd>, origin: Origin, terrain: Option<Arc<TerrainMap>>) -> Arc<Sensor> {
    let sensor = Arc::new(Sensor {
        address: address.clone(),
        frame: Mutex::new(None),
        connected: AtomicBool::new(false),
        status: Mutex::new("waiting for sim".into()),
        fps: Mutex::new(0.0),
        commanded: Mutex::new(Gimbal::default()),
        track: Mutex::new(None),
        writer: Mutex::new(None),
        vehicle,
        tx: Mutex::new(tx),
        origin,
        terrain,
    });
    let s = sensor.clone();
    std::thread::Builder::new().name("sensor-feed".into()).spawn(move || receive(s)).expect("spawn feed thread");
    let s = sensor.clone();
    std::thread::Builder::new().name("sensor-track".into()).spawn(move || track(s)).expect("spawn track thread");
    sensor
}

fn receive(s: Arc<Sensor>) {
    loop {
        let mut stream = match TcpStream::connect(&s.address) {
            Ok(st) => st,
            Err(_) => {
                *s.status.lock().unwrap() = format!("no feed at {} (sim running with --feed?)", s.address);
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        stream.set_nodelay(true).ok();
        *s.writer.lock().unwrap() = stream.try_clone().ok();
        s.connected.store(true, Ordering::Relaxed);
        *s.status.lock().unwrap() = "connected".into();
        // Re-assert the gimbal after a (re)connect.
        let g = *s.commanded.lock().unwrap();
        s.command(g);

        let mut buf = Vec::new();
        let mut last = Instant::now();
        let mut resets: Option<u16> = None;
        let err = loop {
            let h = match read_frame(&mut stream, &mut buf) {
                Ok(h) => h,
                Err(e) => break e,
            };
            // The sim reset the aircraft (teleported it to the hand). Do what you'd do at the
            // field: disarm and power-cycle the autopilot, so its estimators start clean.
            if resets.is_some_and(|r| r != h.resets) {
                s.vehicle.lock().unwrap().push_log("GCS", 4, "Sim reset: restarting the autopilot for a clean start (~30 s)");
                let tx = s.tx.lock().unwrap();
                let _ = tx.send(Cmd::ForceDisarm);
                let _ = tx.send(Cmd::Reboot);
            }
            resets = Some(h.resets);
            let (w, ht) = (h.width as u32, h.height as u32);
            let row = w as usize * 4;
            // GPUI takes tightly packed BGRA, which is what the sim sends (minus row padding).
            let pixels = if h.stride as usize == row {
                std::mem::take(&mut buf)
            } else {
                buf.chunks(h.stride as usize).flat_map(|r| &r[..row]).copied().collect()
            };
            let Some(img) = image::RgbaImage::from_raw(w, ht, pixels) else { continue };
            let image = Arc::new(RenderImage::new(vec![image::Frame::new(img)]));
            let dt = last.elapsed().as_secs_f32().max(1e-3);
            last = Instant::now();
            {
                let mut fps = s.fps.lock().unwrap();
                *fps += (1.0 / dt - *fps) * 0.1;
            }
            *s.frame.lock().unwrap() = Some(VideoFrame { header: h, image, at: Instant::now() });
        };
        s.connected.store(false, Ordering::Relaxed);
        *s.writer.lock().unwrap() = None;
        *s.status.lock().unwrap() = format!("feed lost: {err}");
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn track(s: Arc<Sensor>) {
    let mut last_status = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(33));
        // Tell the sim the autopilot's real armed state and mode (~2 Hz): JSON SITL doesn't,
        // and its launch prompt depends on them.
        if last_status.elapsed() > Duration::from_millis(500) {
            last_status = Instant::now();
            let status = {
                let v = s.vehicle.lock().unwrap();
                (v.link.alive() && v.have_heartbeat).then(|| VehicleStatus { armed: v.armed, mode: v.mode().to_string() })
            };
            if let (Some(st), Some(w)) = (status, s.writer.lock().unwrap().as_mut()) {
                let _ = w.write_all(&st.encode());
            }
        }
        let Some(target) = *s.track.lock().unwrap() else { continue };
        let aim = {
            let v = s.vehicle.lock().unwrap();
            s.aircraft(&v).map(|ac| {
                let d = [target.0 - ac.0, target.1 - ac.1, -(target.2 - ac.2)];
                look_angles(v.roll, v.pitch, v.yaw, d)
            })
        };
        if let Some((pan, tilt)) = aim {
            let hfov = s.commanded.lock().unwrap().hfov;
            s.command(Gimbal { pan, tilt, hfov });
        }
    }
}
