//! The picture of the battlefield: the valley from the commander's camera, rendered by the game's
//! 3D view (`towers_view`) with no drones in it (`teleoperation_shared::battlefield`). The app sends
//! the camera and the size of the battlefield; the pictures come back on a thread of their own,
//! each with the eye that it was rendered from, and the window takes the newest.

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
    at: Instant,
}

#[derive(Default)]
struct Link {
    latest: Mutex<Option<Latest>>,
    /// The newest request, and whether it has been sent.
    request: Mutex<Option<(Request, bool)>>,
}

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

    /// Asks for a picture `w` by `h` from `eye` (sent only when it changed).
    pub fn ask(&self, eye: Eye, w: u16, h: u16) {
        let Some(link) = &self.link else { return };
        let r = Request { eye, width: w, height: h };
        let mut req = link.request.lock().unwrap();
        if req.is_none_or(|(old, _)| old != r) {
            *req = Some((r, false));
        }
    }

    /// The newest picture, and the eye it is from, if there is a fresh one.
    pub fn picture(&self) -> Option<(Arc<RenderImage>, Eye)> {
        let latest = self.link.as_ref()?.latest.lock().unwrap();
        latest.as_ref().filter(|l| l.at.elapsed().as_secs_f32() < STALE).map(|l| (l.image.clone(), l.eye))
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
            *link.latest.lock().unwrap() = Some(Latest { image, eye: f.eye, at: Instant::now() });
        }
        *link.latest.lock().unwrap() = None;
        // The next connection sends the request again.
        if let Some((_, sent)) = link.request.lock().unwrap().as_mut() {
            *sent = false;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}
