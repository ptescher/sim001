//! The picture of the battlefield: the valley from the commander's camera, rendered by the game's
//! 3D view (`towers_view`) with no drones in it (`teleoperation_shared::battlefield`). The app sends
//! the camera and the size of the battlefield; the pictures come back on a thread of their own,
//! each with the eye that it was rendered from, and the window takes the newest. Each change of
//! the camera is a request with the next number; the picture comes back with it, and [`Lag`]
//! measures how far the picture trails the camera.

use crate::lag::{Lag, Motion};
use gpui::RenderImage;
use std::io::Write;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use teleoperation_shared::battlefield::{self, Eye, Request, read_frame};

/// A picture older than this is no picture (s).
const STALE: f32 = 2.0;
/// The app asks again this often, though nothing changed: a 3D view that restarted hears it.
const RESEND: Duration = Duration::from_millis(500);

struct Latest {
    image: Arc<RenderImage>,
    eye: Eye,
    seq: u32,
    at: Instant,
}

#[derive(Default)]
struct Link {
    latest: Mutex<Option<Latest>>,
    /// The newest request, and whether it has been sent.
    request: Mutex<Option<(Request, bool)>>,
    lag: Mutex<Lag>,
    /// The draws since the last log line, and when that was.
    draws: Mutex<Option<(Instant, u32)>>,
}

/// How often the app logs how many times a second it draws.
const LOG_DRAWS_EVERY: Duration = Duration::from_secs(5);

/// The battlefield on a 3D view, or none (there is no 3D view).
pub struct Battlefield {
    link: Option<Arc<Link>>,
}

impl Battlefield {
    /// Connects to battlefield `slot` at `host` (see `teleoperation_shared::battlefield::port`); it
    /// connects again when it is lost.
    pub fn connect(host: &str, first_feed_port: u16, slot: usize) -> Arc<Battlefield> {
        let link = Arc::new(Link::default());
        let addr = format!("{host}:{}", battlefield::port(first_feed_port, slot));
        let l = link.clone();
        std::thread::Builder::new()
            .name("battlefield".into())
            .spawn(move || receive(&addr, &l))
            .expect("spawn the battlefield's thread");
        Arc::new(Battlefield { link: Some(link) })
    }

    pub fn none() -> Arc<Battlefield> {
        Arc::new(Battlefield { link: None })
    }

    /// Asks for a picture `w` by `h` from `eye` (sent only when it changed, with the next number).
    pub fn ask(&self, eye: Eye, w: u16, h: u16) {
        let Some(link) = &self.link else { return };
        let mut req = link.request.lock().unwrap();
        let same = req.is_some_and(|(old, _)| old.eye == eye && old.width == w && old.height == h);
        if !same {
            let seq = req.map_or(1, |(old, _)| old.seq.wrapping_add(1));
            *req = Some((Request { eye, width: w, height: h, seq }, false));
            link.lag.lock().unwrap().asked(seq, Instant::now());
        }
    }

    /// The newest picture, the eye it is from and the number of its request, if there is a fresh one.
    pub fn picture(&self) -> Option<(Arc<RenderImage>, Eye, u32)> {
        let latest = self.link.as_ref()?.latest.lock().unwrap();
        latest.as_ref().filter(|l| l.at.elapsed().as_secs_f32() < STALE).map(|l| (l.image.clone(), l.eye, l.seq))
    }

    /// The picture of request `seq` is on the battlefield now (`None`: no picture). The end of a
    /// motion of the camera is logged with its numbers.
    pub fn painted(&self, seq: Option<u32>) {
        let Some(link) = &self.link else { return };
        {
            let mut draws = link.draws.lock().unwrap();
            let (since, n) = draws.get_or_insert((Instant::now(), 0));
            *n += 1;
            let secs = since.elapsed();
            if secs >= LOG_DRAWS_EVERY {
                println!("battlefield: the app draws {:.0} times a second", *n as f32 / secs.as_secs_f32());
                *draws = Some((Instant::now(), 0));
            }
        }
        // With no picture, the plain map is drawn with the camera now: not behind.
        let seq = seq.unwrap_or_else(|| link.request.lock().unwrap().map_or(0, |(r, _)| r.seq));
        if let Some(m) = link.lag.lock().unwrap().painted(seq, Instant::now()) {
            println!(
                "battlefield: the camera moved {:.1} s; {} pictures ({:.0} a second); behind by {:.0} ms (median), {:.0} ms (90 %)",
                m.secs, m.pictures, m.pictures_per_s, m.lag_ms, m.lag_p90_ms
            );
        }
    }

    /// The numbers of the camera's motion now, or of its last one.
    pub fn motion(&self) -> Option<Motion> {
        self.link.as_ref()?.lag.lock().unwrap().motion(Instant::now())
    }
}

fn receive(addr: &str, link: &Arc<Link>) {
    loop {
        let Ok(mut stream) = TcpStream::connect(addr) else {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        stream.set_nodelay(true).ok();
        // The requests go out on a thread of their own.
        if let Ok(mut out) = stream.try_clone() {
            let link = link.clone();
            std::thread::spawn(move || {
                let mut last = Instant::now();
                loop {
                    let due = {
                        let mut req = link.request.lock().unwrap();
                        match req.as_mut() {
                            Some((r, sent)) if !*sent || last.elapsed() > RESEND => {
                                *sent = true;
                                Some(*r)
                            }
                            _ => None,
                        }
                    };
                    if let Some(r) = due {
                        if out.write_all(&r.encode()).is_err() {
                            return;
                        }
                        last = Instant::now();
                    }
                    std::thread::sleep(Duration::from_millis(8));
                }
            });
        }
        let mut buf = Vec::new();
        while let Ok(f) = read_frame(&mut stream, &mut buf) {
            let (w, h) = (f.width as u32, f.height as u32);
            let row = w as usize * 4;
            // GPUI takes tightly packed BGRA, which is what the view sends (minus row padding).
            let pixels = if f.stride as usize == row {
                std::mem::take(&mut buf)
            } else {
                buf.chunks(f.stride as usize).flat_map(|r| &r[..row]).copied().collect()
            };
            let Some(img) = image::RgbaImage::from_raw(w, h, pixels) else { continue };
            let image = Arc::new(RenderImage::new(vec![image::Frame::new(img)]));
            link.lag.lock().unwrap().arrived(f.seq, Instant::now());
            *link.latest.lock().unwrap() = Some(Latest { image, eye: f.eye, seq: f.seq, at: Instant::now() });
        }
        *link.latest.lock().unwrap() = None;
        // The next connection sends the request again.
        if let Some((_, sent)) = link.request.lock().unwrap().as_mut() {
            *sent = false;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}
