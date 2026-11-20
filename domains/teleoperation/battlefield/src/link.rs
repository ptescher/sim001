//! The picture of the battlefield: the valley from the commander's camera, rendered by the game's
//! 3D view (`towers_view`) with no drones in it (`teleoperation_shared::battlefield`). The app sends
//! the camera and the size of the battlefield; the pictures come back on a thread of their own,
//! each with the eye that it was rendered from, and the window takes the newest. Each change of
//! the camera is a request with the next number; the picture comes back with it, and [`Lag`]
//! measures how far the picture trails the camera.

use crate::lag::{Lag, Motion};
use futures::StreamExt;
use futures::channel::mpsc;
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
    /// How far the battlefield as drawn (the picture moved to a camera, and the map) trails the camera.
    motion: Mutex<Lag>,
    /// Wakes the window when a picture comes.
    wake: Mutex<Option<mpsc::Sender<()>>>,
    /// The draws since the last log line, and when that was.
    draws: Mutex<Option<(Instant, u32)>>,
}

/// How often the app logs how many times a second it draws.
const LOG_DRAWS_EVERY: Duration = Duration::from_secs(5);

/// The pictures that come, for the window to draw each at once.
pub struct Arrivals(mpsc::Receiver<()>);

impl Arrivals {
    /// Waits for the next picture (several that came at once are one); `false` if no more come.
    pub async fn next(&mut self) -> bool {
        self.0.next().await.is_some()
    }
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

    /// The pictures that come from now on (none without a 3D view). One window takes them: a second
    /// call takes them from the first.
    pub fn arrivals(&self) -> Option<Arrivals> {
        let (tx, rx) = mpsc::channel(1);
        *self.link.as_ref()?.wake.lock().unwrap() = Some(tx);
        Some(Arrivals(rx))
    }

    /// Asks for a picture `w` by `h` from `eye` (sent only when it changed, with the next number).
    pub fn ask(&self, eye: Eye, w: u16, h: u16) {
        let Some(link) = &self.link else { return };
        let mut req = link.request.lock().unwrap();
        let same = req.is_some_and(|(old, _)| old.eye == eye && old.width == w && old.height == h);
        if !same {
            let seq = req.map_or(1, |(old, _)| old.seq.wrapping_add(1));
            *req = Some((Request { eye, width: w, height: h, seq }, false));
            let now = Instant::now();
            link.lag.lock().unwrap().asked(seq, now);
            link.motion.lock().unwrap().asked(seq, now);
        }
    }

    /// The newest picture, the eye it is from and the number of its request, if there is a fresh one.
    pub fn picture(&self) -> Option<(Arc<RenderImage>, Eye, u32)> {
        let latest = self.link.as_ref()?.latest.lock().unwrap();
        latest.as_ref().filter(|l| l.at.elapsed().as_secs_f32() < STALE).map(|l| (l.image.clone(), l.eye, l.seq))
    }

    /// The picture of request `seq` is on the battlefield now (`None`: no picture), `moved` to the
    /// camera now or not. The end of a motion of the camera is logged with its numbers.
    pub fn painted(&self, seq: Option<u32>, moved: bool) {
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
        let newest = link.request.lock().unwrap().map_or(0, |(r, _)| r.seq);
        let now = Instant::now();
        // With no picture, the plain map is drawn with the camera now: not behind.
        let drawn = drawn(seq, moved, newest);
        if let Some(m) = link.motion.lock().unwrap().painted(drawn, now) {
            println!(
                "battlefield: the camera moved {:.1} s; the battlefield as drawn {:.0} ms behind it (median)",
                m.secs, m.lag_ms
            );
        }
        if let Some(m) = link.lag.lock().unwrap().painted(seq.unwrap_or(newest), now) {
            println!(
                "battlefield: the camera moved {:.1} s; {} pictures ({:.0} a second); each as old as {:.0} ms (median), {:.0} ms (90 %); the first after {}",
                m.secs,
                m.pictures,
                m.pictures_per_s,
                m.lag_ms,
                m.lag_p90_ms,
                m.first_ms.map_or("none came".into(), |f| format!("{f:.0} ms"))
            );
        }
    }

    /// The numbers of the battlefield as drawn in the camera's motion now, or its last one: how far
    /// it trails the camera.
    pub fn drawn_motion(&self) -> Option<Motion> {
        self.link.as_ref()?.motion.lock().unwrap().motion(Instant::now())
    }

    /// The numbers of the camera's motion now, or of its last one.
    pub fn motion(&self) -> Option<Motion> {
        self.link.as_ref()?.lag.lock().unwrap().motion(Instant::now())
    }
}

/// The request whose camera the battlefield is drawn from: the newest when the picture was moved
/// to it, or there is no picture; else the picture's own.
fn drawn(picture: Option<u32>, moved: bool, newest: u32) -> u32 {
    match picture {
        Some(seq) if !moved => seq,
        _ => newest,
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
            if let Some(wake) = link.wake.lock().unwrap().as_mut() {
                // Full: the window has not drawn the last one yet, and will draw this one with it.
                let _ = wake.try_send(());
            }
        }
        *link.latest.lock().unwrap() = None;
        // The next connection sends the request again.
        if let Some((_, sent)) = link.request.lock().unwrap().as_mut() {
            *sent = false;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_that_comes_wakes_the_window() {
        use teleoperation_shared::battlefield::{Frame, write_frame};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let bf = Battlefield::connect("127.0.0.1", port - 20, 0);
        let mut arrivals = bf.arrivals().unwrap();
        let (mut view, _) = listener.accept().unwrap();
        let f = Frame { width: 2, height: 1, stride: 8, frame: 1, seq: 1, eye: Eye::default() };
        write_frame(&mut view, &f, &[0; 8]).unwrap();
        let woke = std::thread::spawn(move || futures::executor::block_on(arrivals.next()));
        let began = Instant::now();
        while !woke.is_finished() && began.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(woke.is_finished() && woke.join().unwrap(), "no wake within 5 s");
        assert!(bf.picture().is_some());
    }

    #[test]
    fn the_battlefield_is_drawn_from_the_camera_now_unless_the_picture_is_not_moved() {
        assert_eq!(drawn(Some(3), true, 7), 7);
        assert_eq!(drawn(Some(3), false, 7), 3, "a turn waits for its picture");
        assert_eq!(drawn(None, false, 7), 7, "the plain map");
    }
}
