//! The camera of each of the team's drones: the sensor feed of the sim (`teleoperation_shared::feed`),
//! one connection for each drone, to the game's 3D view (`towers_view`, drone `i` on port
//! `first_port + i`). The pictures arrive on threads of their own; the window takes the newest.

use gpui::RenderImage;
use recreation_towers::Team;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use teleoperation_shared::feed::read_frame;

/// The default port of the first drone's feed (Blue's first).
pub const FIRST_PORT: u16 = 5601;
/// A picture older than this is no picture (s).
const STALE: f32 = 2.0;

struct Latest {
    image: Arc<RenderImage>,
    at: Instant,
}

/// The pictures of a team's three drones.
pub struct Cameras {
    drones: Vec<Arc<Mutex<Option<Latest>>>>,
}

impl Cameras {
    /// Connects to the feeds of `team`'s `n` drones at `host`; each connects again when it is lost.
    pub fn connect(host: &str, first_port: u16, team: Team, n: usize) -> Arc<Cameras> {
        let drones: Vec<Arc<Mutex<Option<Latest>>>> = (0..n).map(|_| Arc::new(Mutex::new(None))).collect();
        for (k, slot) in drones.iter().enumerate() {
            let (slot, addr) = (slot.clone(), format!("{host}:{}", first_port + (team.index() * n + k) as u16));
            std::thread::Builder::new()
                .name(format!("camera-{k}"))
                .spawn(move || receive(&addr, &slot))
                .expect("spawn a camera thread");
        }
        Arc::new(Cameras { drones })
    }

    /// No cameras (the game has no 3D view).
    pub fn none() -> Arc<Cameras> {
        Arc::new(Cameras { drones: vec![] })
    }

    /// The newest picture of the team's drone `k`, if there is a fresh one.
    pub fn image(&self, k: usize) -> Option<Arc<RenderImage>> {
        let slot = self.drones.get(k)?.lock().unwrap();
        slot.as_ref().filter(|l| l.at.elapsed().as_secs_f32() < STALE).map(|l| l.image.clone())
    }

    /// How many drones have a fresh picture.
    pub fn live(&self) -> usize {
        (0..self.drones.len()).filter(|&k| self.image(k).is_some()).count()
    }
}

fn receive(addr: &str, slot: &Mutex<Option<Latest>>) {
    loop {
        let Ok(mut stream) = TcpStream::connect(addr) else {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        stream.set_nodelay(true).ok();
        let mut buf = Vec::new();
        while let Ok(h) = read_frame(&mut stream, &mut buf) {
            let (w, ht) = (h.width as u32, h.height as u32);
            let row = w as usize * 4;
            // GPUI takes tightly packed BGRA, which is what the feed sends (minus row padding).
            let pixels = if h.stride as usize == row {
                std::mem::take(&mut buf)
            } else {
                buf.chunks(h.stride as usize).flat_map(|r| &r[..row]).copied().collect()
            };
            let Some(img) = image::RgbaImage::from_raw(w, ht, pixels) else { continue };
            let image = Arc::new(RenderImage::new(vec![image::Frame::new(img)]));
            *slot.lock().unwrap() = Some(Latest { image, at: Instant::now() });
        }
        *slot.lock().unwrap() = None;
        std::thread::sleep(Duration::from_millis(500));
    }
}
