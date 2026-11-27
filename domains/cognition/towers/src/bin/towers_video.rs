//! Makes a video of a recorded game of Towers (`plans/game-tooling.md`, step 4):
//!
//!   cargo run --release -p cognition_towers --bin towers_video -- RECORD [--out FILE.mp4]
//!       [--speed 10] [--fps 30] [--from S] [--to S] [--view all|blue|red]
//!
//! The field from above, as the replay page draws it, with overlays: the drones with their names,
//! trails and orders, a ring round each drone the other team sees, the towers with their control,
//! the clock and the score, the latest events, and each team's latest board entries. Each frame is
//! drawn as SVG, rasterised with resvg and piped to `ffmpeg` (H.264). `--speed` is game seconds a
//! second of video; `--view` shows one team's view (the enemy only where that team sees it).

use recreation_towers::record::{self, Header, Line, Tick};
use recreation_towers::{EntryKind, Order, Phase, Team};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const W: u32 = 1280;
const H: u32 = 720;
const BG: &str = "#11151a";
const PANEL: &str = "#1a2027";
const FIELD: &str = "#17231c";
const LANE: &str = "#2f4034";
const LAKE: &str = "#1d3447";
const TEXT: &str = "#e6ebf0";
const MUTED: &str = "#94a0ad";
const GREY: &str = "#8b95a1";
const TAKE: &str = "#e0b341";
const COL: [&str; 2] = ["#4f9cf9", "#f0625f"];

fn main() {
    if let Err(e) = run() {
        eprintln!("towers_video: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let num = |name: &str, d: f32| -> Result<f32, String> {
        arg(name).map_or(Ok(d), |v| v.parse().map_err(|_| format!("{name} {v}: a number")))
    };
    let input =
        args.first().filter(|a| !a.starts_with("--")).ok_or("usage: towers_video RECORD [--out FILE.mp4] ...")?;
    let out = arg("--out").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(input).with_extension("mp4"));
    let (speed, fps) = (num("--speed", 10.0)?, num("--fps", 30.0)?);
    let view = match arg("--view").as_deref() {
        None | Some("all") => None,
        Some("blue") => Some(0),
        Some("red") => Some(1),
        Some(v) => return Err(format!("--view {v}: all, blue or red")),
    };
    let text = std::fs::read_to_string(input).map_err(|e| format!("{input}: {e}"))?;
    let game = Game::read(&text)?;
    let last = game.ticks.last().map_or(0.0, |t| t.t);
    let (from, to) = (num("--from", 0.0)?.max(0.0), num("--to", last)?.min(last));

    let mut fonts = resvg::usvg::fontdb::Database::new();
    fonts.load_system_fonts();
    let opts = resvg::usvg::Options { fontdb: std::sync::Arc::new(fonts), ..Default::default() };
    let mut ffmpeg = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{W}x{H}")])
        .args(["-r", &fps.to_string(), "-i", "-", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "20"])
        .arg(&out)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e} (is it installed?)"))?;
    let mut pipe = ffmpeg.stdin.take().ok_or("ffmpeg's input")?;
    let ground = game.ground();
    let mut frames = 0;
    let mut t = from;
    while t <= to + 1e-3 {
        let svg = game.frame(t, view, &ground);
        let tree = resvg::usvg::Tree::from_str(&svg, &opts).map_err(|e| format!("frame at {t:.1} s: {e}"))?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(W, H).ok_or("a frame")?;
        resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pixmap.as_mut());
        pipe.write_all(pixmap.data()).map_err(|e| format!("ffmpeg: {e}"))?;
        frames += 1;
        t += speed / fps;
    }
    drop(pipe);
    let status = ffmpeg.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("ffmpeg failed ({status})"));
    }
    println!("{} ({frames} frames, {:.0} s of game at {speed}x)", out.display(), to - from);
    Ok(())
}

/// A record, read for drawing.
struct Game {
    header: Header,
    ticks: Vec<Tick>,
    /// The record's lines after the header, in order.
    lines: Vec<Line>,
}

impl Game {
    fn read(text: &str) -> Result<Game, String> {
        let mut lines = record::read(text)?;
        let Some(Line::Header(header)) = lines.first().cloned() else { return Err("no header".into()) };
        lines.remove(0);
        let ticks: Vec<Tick> =
            lines.iter().filter_map(|l| if let Line::Tick(t) = l { Some(t.clone()) } else { None }).collect();
        if ticks.is_empty() {
            return Err("no ticks".into());
        }
        Ok(Game { header: *header, ticks, lines })
    }

    fn tick_at(&self, t: f32) -> usize {
        self.ticks.partition_point(|k| k.t <= t).saturating_sub(1)
    }

    /// The map's scale and where a point (north, east) is in the frame.
    fn at(&self, p: [f32; 2]) -> (f32, f32) {
        let s = H as f32 / (2.0 * self.header.field.radius * 1.06);
        (H as f32 / 2.0 + p[1] * s, H as f32 / 2.0 - p[0] * s)
    }

    fn scale(&self) -> f32 {
        H as f32 / (2.0 * self.header.field.radius * 1.06)
    }

    /// The ground: drawn once, the same in every frame.
    fn ground(&self) -> String {
        let f = &self.header.field;
        let mut s = String::new();
        let (cx, cy) = self.at([0.0, 0.0]);
        let _ = write!(s, r#"<circle cx="{cx}" cy="{cy}" r="{}" fill="{FIELD}"/>"#, f.radius * self.scale());
        let pts = |v: &[[f32; 2]]| {
            v.iter().map(|&p| self.at(p)).map(|(x, y)| format!("{x:.1},{y:.1}")).collect::<Vec<_>>().join(" ")
        };
        for lake in &f.lakes {
            let _ = write!(s, r#"<polygon points="{}" fill="{LAKE}"/>"#, pts(lake));
        }
        for lane in &f.lanes {
            let _ = write!(
                s,
                r#"<polyline points="{}" fill="none" stroke="{LANE}" stroke-width="{:.1}" stroke-linecap="round" stroke-linejoin="round"/>"#,
                pts(lane),
                2.0 * f.lane_half_width * self.scale()
            );
        }
        for (at, r) in &f.clearings {
            let (x, y) = self.at(*at);
            let _ = write!(s, r#"<circle cx="{x:.1}" cy="{y:.1}" r="{:.1}" fill="{LANE}"/>"#, r * self.scale());
        }
        s
    }

    /// Which enemy drones each team sees at time `t`.
    fn seen(&self, t: f32) -> [Vec<bool>; 2] {
        let n = self.header.drones.len();
        let mut seen = [vec![false; n], vec![false; n]];
        for l in &self.lines {
            match l {
                Line::Spotted { t: at, by, drone } if *at <= t => seen[by.index()][*drone] = true,
                Line::Lost { t: at, by, drone } if *at <= t => seen[by.index()][*drone] = false,
                _ => {}
            }
        }
        seen
    }

    fn drone_name(&self, i: usize) -> String {
        let team = self.header.drones[i].0;
        let k = self.header.drones[..i].iter().filter(|d| d.0 == team).count() + 1;
        format!("{}{k}", if team == Team::Blue { "B" } else { "R" })
    }

    fn order_target(&self, o: &Option<Order>) -> Option<[f32; 2]> {
        match o {
            Some(Order::Take(t)) => self.header.towers.get(*t).map(|t| t.at),
            Some(Order::Goto(p)) | Some(Order::Circle(p, _)) => Some([p.x, p.y]),
            _ => None,
        }
    }

    fn frame(&self, t: f32, view: Option<usize>, ground: &str) -> String {
        let i = self.tick_at(t);
        let f = &self.ticks[i];
        let seen = self.seen(t);
        let scale = self.scale();
        let mut s = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" font-family="Helvetica, Arial, sans-serif"><rect width="{W}" height="{H}" fill="{BG}"/>{ground}"#
        );
        // Towers.
        for (k, tw) in self.header.towers.iter().enumerate() {
            let (control, holder) = f.towers[k];
            let (x, y) = self.at(tw.at);
            let c = holder.map_or(GREY, |h| COL[h.index()]);
            let r = self.header.rules.circle_radius * scale;
            let half = if tw.kind == "spire" { 6.0 } else { 4.5 };
            let bar = control * 20.0;
            let _ = write!(
                s,
                r#"<circle cx="{x:.1}" cy="{y:.1}" r="{r:.1}" fill="none" stroke="{c}" stroke-width="1.5" stroke-dasharray="4 4"/><rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{c}"/><rect x="{:.1}" y="{:.1}" width="40" height="5" fill="{BG}"/><rect x="{:.1}" y="{:.1}" width="{:.1}" height="5" fill="{}"/><text x="{x:.1}" y="{:.1}" font-size="11" fill="{MUTED}" text-anchor="middle">{}</text>"#,
                x - half,
                y - half,
                2.0 * half,
                2.0 * half,
                x - 20.0,
                y + 10.0,
                x + bar.min(0.0),
                y + 10.0,
                bar.abs(),
                COL[usize::from(control < 0.0)],
                y - 12.0,
                tw.name
            );
        }
        let shown = |d: usize| view.is_none_or(|v| self.header.drones[d].0.index() == v || seen[v][d]);
        // Trails: the last 30 s.
        let from = self.tick_at(t - 30.0);
        for d in (0..self.header.drones.len()).filter(|&d| shown(d)) {
            let mut pts = String::new();
            for k in &self.ticks[from..=i] {
                if k.drones[d].phase == Phase::Flying {
                    let (x, y) = self.at(k.drones[d].at);
                    let _ = write!(pts, "{x:.1},{y:.1} ");
                }
            }
            let _ = write!(
                s,
                r#"<polyline points="{pts}" fill="none" stroke="{}" stroke-opacity="0.35" stroke-width="1.5"/>"#,
                COL[self.header.drones[d].0.index()]
            );
        }
        for (d, x) in f.drones.iter().enumerate().filter(|(d, _)| shown(*d)) {
            let (team, class) = &self.header.drones[d];
            let ti = team.index();
            let (px, py) = self.at(x.at);
            let r = match class.as_str() {
                "heavy" => 8.0,
                "light" => 4.5,
                _ => 6.0,
            };
            if x.phase == Phase::Flying
                && let Some(g) = self.order_target(&x.order)
            {
                let (gx, gy) = self.at(g);
                let _ = write!(
                    s,
                    r#"<line x1="{px:.1}" y1="{py:.1}" x2="{gx:.1}" y2="{gy:.1}" stroke="{}" stroke-opacity="0.25"/>"#,
                    COL[ti]
                );
            }
            if seen[1 - ti][d] {
                let _ = write!(
                    s,
                    r#"<circle cx="{px:.1}" cy="{py:.1}" r="{:.1}" fill="none" stroke="{}" stroke-width="2"/>"#,
                    r + 5.0,
                    COL[1 - ti]
                );
            }
            let opacity = if x.phase == Phase::Flying { 1.0 } else { 0.35 };
            let _ =
                write!(s, r#"<circle cx="{px:.1}" cy="{py:.1}" r="{r}" fill="{}" fill-opacity="{opacity}"/>"#, COL[ti]);
            if !x.linked {
                let _ = write!(
                    s,
                    r#"<circle cx="{px:.1}" cy="{py:.1}" r="{:.1}" fill="none" stroke="{TEXT}" stroke-dasharray="2 2"/>"#,
                    r + 2.0
                );
            }
            let _ = write!(
                s,
                r#"<text x="{px:.1}" y="{:.1}" font-size="10" fill="{TEXT}" text-anchor="middle">{}</text>"#,
                py - r - 3.0,
                self.drone_name(d)
            );
        }
        self.panel(&mut s, t, f, view);
        s.push_str("</svg>");
        s
    }

    /// The panel on the right: the clock and score, the events, the boards.
    fn panel(&self, s: &mut String, t: f32, f: &Tick, view: Option<usize>) {
        let x0 = H as f32 + 16.0;
        let w = W as f32 - x0 - 16.0;
        let win = self.header.rules.win_score.max(1.0);
        let _ = write!(
            s,
            r#"<rect x="{H}" y="0" width="{}" height="{H}" fill="{PANEL}"/><text x="{x0}" y="34" font-size="22" font-weight="bold" fill="{TEXT}">{:.0} s</text><text x="{}" y="34" font-size="13" fill="{MUTED}" text-anchor="end">seed {}{}</text>"#,
            W - H,
            f.t,
            x0 + w,
            self.header.seed,
            match view {
                Some(0) => ", Blue's view",
                Some(_) => ", Red's view",
                None => "",
            }
        );
        for (k, team) in Team::BOTH.iter().enumerate() {
            let y = 58.0 + 26.0 * k as f32;
            let _ = write!(
                s,
                r#"<text x="{x0}" y="{}" font-size="13" fill="{}">{:?}</text><rect x="{}" y="{}" width="{}" height="8" rx="4" fill="{BG}"/><rect x="{}" y="{}" width="{:.1}" height="8" rx="4" fill="{}"/><text x="{}" y="{}" font-size="13" fill="{TEXT}" text-anchor="end">{:.0}</text>"#,
                y + 4.0,
                COL[k],
                team,
                x0 + 46.0,
                y - 4.0,
                w - 90.0,
                x0 + 46.0,
                y - 4.0,
                ((w - 90.0) * f.score[k] / win).clamp(0.0, w - 90.0),
                COL[k],
                x0 + w,
                y + 4.0,
                f.score[k]
            );
        }
        let side = |k: usize| esc(&self.header.sides[k]);
        let _ = write!(
            s,
            r#"<text x="{x0}" y="124" font-size="11" fill="{}">Blue: {}</text><text x="{x0}" y="140" font-size="11" fill="{}">Red: {}</text>"#,
            COL[0],
            side(0),
            COL[1],
            side(1)
        );
        // Events: the last few.
        let mut y = 172.0;
        let _ = write!(s, r#"<text x="{x0}" y="{y}" font-size="13" font-weight="bold" fill="{MUTED}">Events</text>"#);
        let events: Vec<String> = self
            .lines
            .iter()
            .filter_map(|l| match l {
                Line::Spotted { t: at, by, drone } if *at <= t => {
                    Some(format!("{at:.0} s  {by:?} spotted {}", self.drone_name(*drone)))
                }
                Line::Taken { t: at, tower, by } if *at <= t => Some(match by {
                    Some(b) => format!("{at:.0} s  {b:?} took the {}", self.header.towers[*tower].name),
                    None => format!("{at:.0} s  the {} is neutral", self.header.towers[*tower].name),
                }),
                Line::Down { t: at, drone } if *at <= t => {
                    Some(format!("{at:.0} s  {} is down", self.drone_name(*drone)))
                }
                _ => None,
            })
            .collect();
        for e in events.iter().rev().take(6) {
            y += 17.0;
            let _ = write!(s, r#"<text x="{x0}" y="{y}" font-size="12" fill="{MUTED}">{}</text>"#, esc(e));
        }
        // Each team's last board entries.
        for (k, team) in Team::BOTH.iter().enumerate() {
            y += 30.0;
            let _ = write!(
                s,
                r#"<text x="{x0}" y="{y}" font-size="13" font-weight="bold" fill="{}">{team:?}'s board</text>"#,
                COL[k]
            );
            let took: Vec<u64> = self
                .lines
                .iter()
                .filter_map(|l| match l {
                    Line::Took { t: at, team: tm, entry } if *at <= t && tm == team => Some(*entry),
                    _ => None,
                })
                .collect();
            let entries: Vec<(f32, &recreation_towers::Entry)> = self
                .lines
                .iter()
                .filter_map(|l| match l {
                    Line::Board { t: at, team: tm, entry } if *at <= t && tm == team => Some((*at, entry)),
                    _ => None,
                })
                .collect();
            for (at, e) in entries.iter().rev().take(3) {
                let said = match &e.kind {
                    EntryKind::Proposal(p) => p.why.clone(),
                    EntryKind::Intent(i) => format!("Intent: {}", i.why),
                };
                let mark = if took.contains(&e.id) { "taken · " } else { "" };
                y += 17.0;
                let _ = write!(
                    s,
                    r#"<text x="{x0}" y="{y}" font-size="11" fill="{}">{at:.0} s · {}{}</text>"#,
                    if mark.is_empty() { MUTED } else { TAKE },
                    mark,
                    esc(&e.author)
                );
                for line in wrap(&said, 62).iter().take(2) {
                    y += 15.0;
                    let _ = write!(s, r#"<text x="{x0}" y="{y}" font-size="12" fill="{TEXT}">{}</text>"#, esc(line));
                }
            }
        }
    }
}

/// Splits `text` into lines of at most `n` characters, at spaces.
fn wrap(text: &str, n: usize) -> Vec<String> {
    let mut out = vec![String::new()];
    for word in text.split_whitespace() {
        let line = out.last_mut().unwrap();
        if !line.is_empty() && line.len() + 1 + word.len() > n {
            out.push(word.to_string());
        } else {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
