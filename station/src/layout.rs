//! Where the three station panes go: the sim's outside view, the sensor feed and the GCS
//! console. One display gets tiled; with more, the panes spread across them.
//!
//! All rectangles are window frames (title bar included) in logical points, in the global
//! desktop space with the origin at the top-left of the main display and y down.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn cx(&self) -> f32 {
        self.x + self.w * 0.5
    }

    /// `x,y,w,h`, the form `t1s_sim --window-rect` takes.
    pub fn to_arg(&self) -> String {
        format!("{:.0},{:.0},{:.0},{:.0}", self.x, self.y, self.w, self.h)
    }

    pub fn parse(s: &str) -> Option<Rect> {
        let v: Vec<f32> = s.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
        match v.as_slice() {
            [x, y, w, h] if *w > 0.0 && *h > 0.0 => Some(Rect::new(*x, *y, *w, *h)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    /// The sim's own window: line-of-sight / chase view of the aircraft.
    Sim,
    /// The aircraft's camera, streamed from the sim, with symbology.
    Feed,
    /// The ground control console.
    Console,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub pane: Pane,
    /// Index into the `displays` slice given to [`plan`].
    pub display: usize,
    pub rect: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Tile on one display, or spread over several if there are.
    Auto,
    /// Tile everything on the main display even if there are others.
    Single,
}

#[derive(Clone, Copy, Debug)]
pub struct Display {
    /// Usable area (below the menu bar).
    pub rect: Rect,
    pub main: bool,
}

/// Fraction of a single display's width given to the console.
const CONSOLE_FRAC: f32 = 0.58;

pub fn plan(displays: &[Display], mode: Mode) -> Vec<Placement> {
    if displays.is_empty() {
        return vec![];
    }
    let main = displays.iter().position(|d| d.main).unwrap_or(0);
    let others: Vec<usize> = (0..displays.len()).filter(|&i| i != main).collect();
    if mode == Mode::Single || others.is_empty() {
        return tile_all(main, displays[main].rect);
    }
    let m = displays[main].rect;
    let mut out = vec![Placement { pane: Pane::Console, display: main, rect: m }];
    if others.len() == 1 {
        // Console on the main display, sensor feed and outside view share the other.
        let i = others[0];
        let r = displays[i].rect;
        let (a, b) = if r.h > r.w { split_v(r, 0.5) } else { split_h(r, 0.5) };
        out.push(Placement { pane: Pane::Feed, display: i, rect: a });
        out.push(Placement { pane: Pane::Sim, display: i, rect: b });
        return out;
    }
    // Three or more: each pane gets a display. The feed goes on the nearest display to the
    // left of the console (pilot's video on the left, as on a UAS ground station), the sim
    // on the nearest remaining one.
    let dist = |i: usize| (displays[i].rect.cx() - m.cx()).abs();
    let mut left: Vec<usize> = others.iter().copied().filter(|&i| displays[i].rect.cx() < m.cx()).collect();
    left.sort_by(|&a, &b| dist(a).total_cmp(&dist(b)));
    let mut rest = others.clone();
    rest.sort_by(|&a, &b| dist(a).total_cmp(&dist(b)));
    let feed = left.first().copied().unwrap_or(rest[0]);
    let sim = rest.iter().copied().find(|&i| i != feed).unwrap();
    out.push(Placement { pane: Pane::Feed, display: feed, rect: displays[feed].rect });
    out.push(Placement { pane: Pane::Sim, display: sim, rect: displays[sim].rect });
    out
}

/// Console on the left; on the right the feed (sized for a 16:9 picture) above the sim.
fn tile_all(display: usize, r: Rect) -> Vec<Placement> {
    let (console, right) = split_h(r, CONSOLE_FRAC);
    let feed_h = (right.w * 9.0 / 16.0 + 28.0).min(right.h * 0.55);
    let (feed, sim) = split_v(right, feed_h / right.h);
    vec![
        Placement { pane: Pane::Console, display, rect: console },
        Placement { pane: Pane::Feed, display, rect: feed },
        Placement { pane: Pane::Sim, display, rect: sim },
    ]
}

fn split_h(r: Rect, frac: f32) -> (Rect, Rect) {
    let w = (r.w * frac).round();
    (Rect::new(r.x, r.y, w, r.h), Rect::new(r.x + w, r.y, r.w - w, r.h))
}

fn split_v(r: Rect, frac: f32) -> (Rect, Rect) {
    let h = (r.h * frac).round();
    (Rect::new(r.x, r.y, r.w, h), Rect::new(r.x, r.y + h, r.w, r.h - h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(x: f32, w: f32, h: f32, main: bool) -> Display {
        Display { rect: Rect::new(x, 25.0, w, h), main }
    }

    fn find(p: &[Placement], pane: Pane) -> Placement {
        *p.iter().find(|q| q.pane == pane).unwrap()
    }

    #[test]
    fn single_display_tiles_without_overlap() {
        let p = plan(&[d(0.0, 2560.0, 1415.0, true)], Mode::Auto);
        assert_eq!(p.len(), 3);
        let (c, f, s) = (find(&p, Pane::Console), find(&p, Pane::Feed), find(&p, Pane::Sim));
        assert_eq!(c.rect.x + c.rect.w, f.rect.x);
        assert_eq!(f.rect.x, s.rect.x);
        assert_eq!(f.rect.y + f.rect.h, s.rect.y);
        assert_eq!(s.rect.y + s.rect.h, 25.0 + 1415.0);
        assert_eq!(c.rect.w + f.rect.w, 2560.0);
    }

    #[test]
    fn two_displays_console_on_main() {
        let p = plan(&[d(-1920.0, 1920.0, 1055.0, false), d(0.0, 2560.0, 1415.0, true)], Mode::Auto);
        assert_eq!(find(&p, Pane::Console).display, 1);
        assert_eq!(find(&p, Pane::Feed).display, 0);
        assert_eq!(find(&p, Pane::Sim).display, 0);
        assert_eq!(find(&p, Pane::Feed).rect.w + find(&p, Pane::Sim).rect.w, 1920.0);
    }

    #[test]
    fn three_displays_feed_left_sim_right() {
        let ds = [d(2560.0, 1920.0, 1055.0, false), d(0.0, 2560.0, 1415.0, true), d(-1920.0, 1920.0, 1055.0, false)];
        let p = plan(&ds, Mode::Auto);
        assert_eq!(find(&p, Pane::Console).display, 1);
        assert_eq!(find(&p, Pane::Feed).display, 2);
        assert_eq!(find(&p, Pane::Sim).display, 0);
    }

    #[test]
    fn single_mode_ignores_extra_displays() {
        let p = plan(&[d(0.0, 2560.0, 1415.0, true), d(2560.0, 1920.0, 1055.0, false)], Mode::Single);
        assert!(p.iter().all(|q| q.display == 0));
    }

    #[test]
    fn rect_arg_roundtrip() {
        let r = Rect::new(-1920.0, 25.0, 960.0, 540.0);
        assert_eq!(Rect::parse(&r.to_arg()), Some(r));
        assert_eq!(Rect::parse("1,2,0,4"), None);
    }
}
