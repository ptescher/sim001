//! The camera of each of the team's drones: the sensor feed of the sim (`teleoperation_shared::feed`),
//! from the game's 3D view (`towers_view`, drone `i` on port `first_port + i`). The 3D view renders a
//! drone's camera only while a viewer is connected, so the app connects only to the drone it shows
//! ([`Cameras::watch`]). The pictures arrive on a thread of their own; the window takes the newest.

use gpui::RenderImage;
use recreation_towers::Team;
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// A drone's camera: its newest picture, whether the app watches it, and the connection while it does.
#[derive(Default)]
struct Feed {
    latest: Mutex<Option<Latest>>,
    watched: AtomicBool,
    stream: Mutex<Option<TcpStream>>,
}

/// The pictures of a team's drones.
pub struct Cameras {
    drones: Vec<Arc<Feed>>,
}

impl Cameras {
    /// The feeds of `team`'s `n` drones at `host`. A drone's feed is connected while it is watched,
    /// and again when it is lost.
    pub fn connect(host: &str, first_port: u16, team: Team, n: usize) -> Arc<Cameras> {
        let drones: Vec<Arc<Feed>> = (0..n).map(|_| Arc::new(Feed::default())).collect();
        for (k, feed) in drones.iter().enumerate() {
            let (feed, addr) = (feed.clone(), format!("{host}:{}", first_port + (team.index() * n + k) as u16));
            std::thread::Builder::new()
                .name(format!("camera-{k}"))
                .spawn(move || receive(&addr, &feed))
                .expect("spawn a camera thread");
        }
        Arc::new(Cameras { drones })
    }

    /// Watches the camera of drone `k` only, or none: the others are let go, and their pictures too.
    pub fn watch(&self, k: Option<usize>) {
        for (i, feed) in self.drones.iter().enumerate() {
            let on = k == Some(i);
            if feed.watched.swap(on, Ordering::SeqCst) && !on {
                if let Some(s) = feed.stream.lock().unwrap().take() {
                    let _ = s.shutdown(Shutdown::Both);
                }
                *feed.latest.lock().unwrap() = None;
            }
        }
    }

    /// No cameras (the game has no 3D view).
    pub fn none() -> Arc<Cameras> {
        Arc::new(Cameras { drones: vec![] })
    }

    /// The newest picture of the team's drone `k`, if there is a fresh one.
    pub fn image(&self, k: usize) -> Option<Arc<RenderImage>> {
        let slot = self.drones.get(k)?.latest.lock().unwrap();
        slot.as_ref().filter(|l| l.at.elapsed().as_secs_f32() < STALE).map(|l| l.image.clone())
    }

    /// How many drones the feeds are for (none with `none()`).
    pub fn len(&self) -> usize {
        self.drones.len()
    }

    /// How many drones have a fresh picture.
    pub fn live(&self) -> usize {
        (0..self.drones.len()).filter(|&k| self.image(k).is_some()).count()
    }
}

fn receive(addr: &str, feed: &Feed) {
    loop {
        if !feed.watched.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        let Ok(mut stream) = TcpStream::connect(addr) else {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        stream.set_nodelay(true).ok();
        *feed.stream.lock().unwrap() = stream.try_clone().ok();
        // Let go at once if it stopped being watched while this connected.
        if !feed.watched.load(Ordering::SeqCst) {
            let _ = stream.shutdown(Shutdown::Both);
        }
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
            if feed.watched.load(Ordering::SeqCst) {
                *feed.latest.lock().unwrap() = Some(Latest { image, at: Instant::now() });
            }
        }
        *feed.latest.lock().unwrap() = None;
        *feed.stream.lock().unwrap() = None;
        if feed.watched.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    /// Waits up to `secs` for the 3D view's end of the feed to see a viewer come.
    fn comes(l: &TcpListener, secs: f32) -> Option<TcpStream> {
        let began = Instant::now();
        while began.elapsed().as_secs_f32() < secs {
            if let Ok((s, _)) = l.accept() {
                return Some(s);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    /// The 3D view renders a drone's camera only while a viewer is connected, so the app connects
    /// only to the camera it shows.
    #[test]
    fn only_the_watched_drones_camera_is_connected() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.set_nonblocking(true).unwrap();
        let cams = Cameras::connect("127.0.0.1", l.local_addr().unwrap().port(), Team::Blue, 1);
        assert!(comes(&l, 1.5).is_none(), "connected though nothing is watched");
        cams.watch(Some(0));
        let mut view = comes(&l, 3.0).expect("the watched camera connects");
        cams.watch(None);
        view.set_nonblocking(false).unwrap();
        view.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut b = [0u8; 1];
        assert_eq!(view.read(&mut b).ok(), Some(0), "the viewer leaves when it is no longer watched");
    }
}
