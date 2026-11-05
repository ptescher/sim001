//! A team's map as a picture (SVG), drawn as the commander app draws it: the fog, what the team
//! sees, the towers with their circles and control bars, the drones and their orders, and the
//! clock and the scores along the top. The tests with no window put it in the docs.

use crate::map::{LANE_HALF_WIDTH, MOUNTAIN_HEIGHT, Map};
use crate::{Order, Outcome, Phase, Rules, Team, TeamView};
use glam::Vec2;
use illustration_theme::NIGHT as P;
use std::fmt::Write;

const W: f32 = 640.0;
const H: f32 = 520.0;
/// The top bar: the clock and the scores.
const BAR: f32 = 36.0;
/// What a team sees, and the fog over the rest (the commander app's colours).
const FIELD: &str = "#1d2a1f";
const FOG: &str = "#07090a";

fn hex(c: illustration_theme::Rgb) -> String {
    format!("#{:06x}", c.0)
}

fn team_colour(t: Team) -> String {
    match t {
        Team::Blue => hex(P.info),
        Team::Red => hex(P.warning),
    }
}

fn team_name(t: Team) -> &'static str {
    match t {
        Team::Blue => "BLUE",
        Team::Red => "RED",
    }
}

const LANE: &str = "#33463a";
const LAKE: &str = "#1d3b52";
const ROCK: &str = "#6b7078";
/// The mountains are drawn in cells this big (m).
const ROCK_CELL: f32 = 15.0;

/// The valley under the fog: the lake, the lanes and the clearings. A team knows the ground, so the
/// map shows it everywhere, dim in the fog.
fn valley_svg(map: &Map, scale: f32, at: &dyn Fn(Vec2) -> (f32, f32)) -> String {
    let mut s = mountains_svg(map, scale, at);
    let (lx, ly) = at(map.lake.at);
    let _ = write!(
        s,
        r##"<circle cx="{lx:.1}" cy="{ly:.1}" r="{:.1}" fill="{LAKE}" fill-opacity="0.8"/>"##,
        map.lake.radius * scale
    );
    for lane in &map.lanes {
        let pts =
            lane.points.iter().map(|&p| at(p)).map(|(x, y)| format!("{x:.1},{y:.1}")).collect::<Vec<_>>().join(" ");
        let _ = write!(
            s,
            r##"<polyline points="{pts}" fill="none" stroke="{LANE}" stroke-opacity="0.75" stroke-width="{:.1}" stroke-linejoin="round" stroke-linecap="round"/>"##,
            2.0 * LANE_HALF_WIDTH * scale
        );
    }
    for c in &map.clearings {
        let (x, y) = at(c.at);
        let _ = write!(
            s,
            r##"<circle cx="{x:.1}" cy="{y:.1}" r="{:.1}" fill="{LANE}" fill-opacity="0.75"/>"##,
            c.radius * scale
        );
    }
    s
}

/// The mountains: grey, the darker the taller. Runs of cells along a row are one rectangle.
fn mountains_svg(map: &Map, scale: f32, at: &dyn Fn(Vec2) -> (f32, f32)) -> String {
    let mut s = String::new();
    let n = (map.field_radius * 1.1 / ROCK_CELL).ceil() as i32;
    // Four steps of height: 1/8, 1/4, 1/2 and 3/4 of the tallest.
    let level = |p: Vec2| (map.mountain_height(p) / MOUNTAIN_HEIGHT * 8.0).log2().floor().clamp(-1.0, 2.0) as i32;
    for row in -n..=n {
        let north = row as f32 * ROCK_CELL;
        let mut run: Option<(i32, i32, i32)> = None;
        for col in -n..=n + 1 {
            let l = if col <= n { level(Vec2::new(north, col as f32 * ROCK_CELL)) } else { -1 };
            if let Some((from, to, l0)) = run
                && (l != l0 || col > n)
            {
                if l0 >= 0 {
                    let (x, y) = at(Vec2::new(north + ROCK_CELL * 0.5, (from as f32 - 0.5) * ROCK_CELL));
                    let _ = write!(
                        s,
                        r##"<rect x="{x:.1}" y="{y:.1}" width="{:.1}" height="{:.1}" fill="{ROCK}" fill-opacity="{:.2}"/>"##,
                        (to - from + 1) as f32 * ROCK_CELL * scale,
                        ROCK_CELL * scale + 0.5,
                        0.3 + 0.2 * l0 as f32
                    );
                }
                run = None;
            }
            match run {
                Some((from, _, l0)) => run = Some((from, col, l0)),
                None if l >= 0 => run = Some((col, col, l)),
                None => {}
            }
        }
    }
    s
}

/// `view` as `view.team` sees it, the whole field fitted under the top bar, north up.
pub fn map_svg(view: &TeamView, rules: &Rules) -> String {
    map_with_valley_svg(view, rules, &Map::new(rules))
}

/// The same, over `map`.
pub fn map_with_valley_svg(view: &TeamView, rules: &Rules, map: &Map) -> String {
    let scale = 0.5 * (H - BAR) / (rules.field_radius * 1.08);
    let (cx, cy) = (W * 0.5, BAR + (H - BAR) * 0.5);
    let at = |p: Vec2| (cx + p.y * scale, cy - p.x * scale);
    let (white, grey, amber) = (hex(P.text), hex(P.muted), hex(P.caution));
    let mut s = String::new();
    let _ = write!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" font-family="Menlo, ui-monospace, monospace" font-size="10"><rect width="{W}" height="{H}" fill="{FOG}"/>"##
    );
    // What the team sees: round its base, its drones and the towers it holds.
    let mut eyes = vec![(rules.base(view.team), rules.base_vision)];
    // A drone out of contact sends no picture.
    eyes.extend(view.drones.iter().filter(|d| d.phase != Phase::Down && d.linked).map(|d| (d.at, rules.drone_vision)));
    eyes.extend(view.towers.iter().filter(|t| t.holder == Some(view.team)).map(|t| (t.at, rules.vision(t.kind))));
    for (p, range) in eyes {
        let (x, y) = at(p);
        let _ = write!(s, r##"<circle cx="{x:.1}" cy="{y:.1}" r="{:.1}" fill="{FIELD}"/>"##, range * scale);
    }
    s += &valley_svg(map, scale, &at);
    let (fx, fy) = at(Vec2::ZERO);
    let _ = write!(
        s,
        r##"<circle cx="{fx:.1}" cy="{fy:.1}" r="{:.1}" fill="none" stroke="{grey}" stroke-opacity="0.6" stroke-dasharray="6 5"/>"##,
        rules.field_radius * scale
    );
    for team in Team::BOTH {
        let (x, y) = at(rules.base(team));
        let colour = team_colour(team);
        let _ = write!(
            s,
            r##"<rect x="{:.1}" y="{:.1}" width="20" height="10" fill="{colour}" fill-opacity="0.35"/><text x="{x:.1}" y="{:.1}" fill="{colour}" fill-opacity="0.8" text-anchor="middle">{} BASE</text>"##,
            x - 10.0,
            y - 5.0,
            y + 18.0,
            team_name(team)
        );
    }
    // The towers, as the team last saw them.
    for t in &view.towers {
        let (x, y) = at(t.at);
        let colour = t.holder.map_or(grey.clone(), team_colour);
        let alpha = if t.seen { 1.0 } else { 0.55 };
        let r = rules.circle_radius * scale;
        // The radio shadow of a tower that the enemy holds.
        if t.holder == Some(view.team.other()) {
            let _ = write!(
                s,
                r##"<circle cx="{x:.1}" cy="{y:.1}" r="{:.1}" fill="{}" fill-opacity="0.12" stroke="{}" stroke-opacity="{:.2}" stroke-dasharray="2 3"/>"##,
                rules.shadow(t.kind) * scale,
                team_colour(view.team.other()),
                team_colour(view.team.other()),
                0.6 * alpha
            );
        }
        if t.contested {
            let _ = write!(
                s,
                r##"<circle cx="{x:.1}" cy="{y:.1}" r="{r:.1}" fill="none" stroke="{amber}" stroke-width="2.5"/>"##
            );
        } else {
            let _ = write!(
                s,
                r##"<circle cx="{x:.1}" cy="{y:.1}" r="{r:.1}" fill="none" stroke="{colour}" stroke-opacity="{:.2}" stroke-dasharray="4 3"/>"##,
                0.6 * alpha
            );
        }
        let _ = write!(
            s,
            r##"<rect x="{:.1}" y="{:.1}" width="14" height="14" fill="{colour}" fill-opacity="{alpha}" stroke="{white}" stroke-opacity="{:.2}"/>"##,
            x - 7.0,
            y - 7.0,
            0.7 * alpha
        );
        // The control bar: right of the middle towards Blue, left towards Red.
        let (bw, by) = (44.0, y + 14.0);
        let filled = t.control.clamp(-1.0, 1.0) * bw * 0.5;
        let (from, bar) =
            if filled >= 0.0 { (x, team_colour(Team::Blue)) } else { (x + filled, team_colour(Team::Red)) };
        let _ = write!(
            s,
            r##"<rect x="{:.1}" y="{by:.1}" width="{bw}" height="5" fill="{}"/><rect x="{from:.1}" y="{by:.1}" width="{:.1}" height="5" fill="{bar}" fill-opacity="{alpha}"/><rect x="{:.1}" y="{:.1}" width="1" height="7" fill="{white}"/>"##,
            x - bw * 0.5,
            hex(P.faint),
            filled.abs(),
            x - 0.5,
            by - 1.0
        );
        let label = if t.seen { t.name.to_uppercase() } else { format!("{} ({:.0} s)", t.name.to_uppercase(), t.age) };
        let _ = write!(
            s,
            r##"<text x="{x:.1}" y="{:.1}" fill="{white}" fill-opacity="{alpha}" text-anchor="middle">{label}</text>"##,
            by + 17.0
        );
    }
    // The team's drones, and a dashed line to where each is sent.
    let tint = team_colour(view.team);
    for d in view.drones.iter().filter(|d| d.phase != Phase::Down) {
        let to = match d.order {
            Some(Order::Take(t)) => view.towers.get(t).map(|t| t.at),
            Some(Order::Goto(p)) => Some(p),
            Some(Order::Home) => Some(rules.base(view.team)),
            _ => None,
        };
        if let Some(to) = to {
            let ((x0, y0), (x1, y1)) = (at(d.at), at(to));
            let _ = write!(
                s,
                r##"<line x1="{x0:.1}" y1="{y0:.1}" x2="{x1:.1}" y2="{y1:.1}" stroke="{tint}" stroke-opacity="0.5" stroke-dasharray="5 4"/>"##
            );
        }
    }
    let letter = |t: Team| &team_name(t)[..1];
    for (j, d) in view.drones.iter().enumerate() {
        let (x, y) = at(d.at);
        let (colour, fill) =
            if d.phase == Phase::Down { (grey.as_str(), true) } else { (tint.as_str(), d.phase == Phase::Flying) };
        let alpha = if d.linked { 1.0 } else { 0.5 };
        s += &drone(x, y, d.vel, colour, fill, alpha);
        // Drones on top of each other (at the base) get their labels one under the other.
        let below = view.drones[..j].iter().filter(|o| (at(o.at).0 - x).hypot(at(o.at).1 - y) < 16.0).count();
        let _ = write!(
            s,
            r##"<text x="{:.1}" y="{:.1}" fill="{colour}" fill-opacity="{alpha}">{}{}{}</text>"##,
            x + 12.0,
            y - 8.0 + 11.0 * below as f32,
            letter(view.team),
            if d.low { format!("{} LOW", d.k + 1) } else { (d.k + 1).to_string() },
            if d.linked { String::new() } else { format!(" NO LINK {:.0} s", d.silent) }
        );
    }
    // The enemy drones the team sees, and where it last saw the others.
    let enemy = team_colour(view.team.other());
    for e in &view.enemies {
        let (x, y) = at(e.at);
        let label = format!("{}{}", letter(view.team.other()), e.k + 1);
        let (alpha, label) = if e.seen { (1.0, label) } else { (0.5, format!("{label} {:.0} s", e.age)) };
        s += &drone(x, y, e.vel, &enemy, e.seen, alpha);
        let _ = write!(
            s,
            r##"<text x="{:.1}" y="{:.1}" fill="{enemy}" fill-opacity="{alpha}">{label}</text>"##,
            x + 12.0,
            y - 8.0
        );
    }
    // The top bar.
    let left = view.time_left.max(0.0) as u32;
    let _ = write!(
        s,
        r##"<rect width="{W}" height="{BAR}" fill="{}"/><text x="12" y="23" font-size="14" fill="{tint}">{} MAP</text><text x="{:.0}" y="23" font-size="14" fill="{white}" text-anchor="middle">{}:{:02} LEFT</text><text x="{:.0}" y="23" font-size="14" fill="{}" text-anchor="end">BLUE {:.0}</text><text x="{:.0}" y="23" font-size="14" fill="{}" text-anchor="end">RED {:.0}</text>"##,
        hex(P.bezel),
        team_name(view.team),
        W * 0.5,
        left / 60,
        left % 60,
        W - 92.0,
        team_colour(Team::Blue),
        view.score[0].floor(),
        W - 12.0,
        team_colour(Team::Red),
        view.score[1].floor()
    );
    if let Some(outcome) = view.outcome {
        let banner = match outcome {
            Outcome::Won(t) => format!("{} WINS", team_name(t)),
            Outcome::Draw => "DRAW".into(),
        };
        let _ = write!(
            s,
            r##"<rect x="{:.0}" y="{:.0}" width="280" height="60" fill="#000" fill-opacity="0.8"/><text x="{:.0}" y="{:.0}" font-size="28" fill="{amber}" text-anchor="middle">{banner}</text>"##,
            W * 0.5 - 140.0,
            cy - 30.0,
            W * 0.5,
            cy + 10.0
        );
    }
    s += "</svg>";
    s
}

/// A drone: a triangle pointing the way it flies (north when it is still), filled when it flies.
fn drone(x: f32, y: f32, vel: Vec2, colour: &str, filled: bool, alpha: f32) -> String {
    let dir = if vel.length() > 1.0 { vel.normalize() } else { Vec2::X };
    // North is up the picture and east to the right.
    let (fx, fy) = (dir.y, -dir.x);
    let (sx, sy) = (-fy, fx);
    let p = [
        (x + fx * 9.0, y + fy * 9.0),
        (x - fx * 6.0 + sx * 6.0, y - fy * 6.0 + sy * 6.0),
        (x - fx * 6.0 - sx * 6.0, y - fy * 6.0 - sy * 6.0),
    ];
    let pts = p.iter().map(|(a, b)| format!("{a:.1},{b:.1}")).collect::<Vec<_>>().join(" ");
    if filled {
        format!(r##"<polygon points="{pts}" fill="{colour}" fill-opacity="{alpha}"/>"##)
    } else {
        format!(
            r##"<polygon points="{pts}" fill="none" stroke="{colour}" stroke-opacity="{alpha}" stroke-width="1.2"/>"##
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Game;

    #[test]
    fn the_map_shows_mountains_radio_shadows_and_a_drone_out_of_contact() {
        use crate::Game;
        let mut g = Game::new(Rules::default());
        let i = g.drone(Team::Blue, 1);
        g.drones[i].at = g.towers[4].at - Vec2::new(100.0, 0.0);
        g.drones[i].alt = 50.0;
        g.drones[i].phase = Phase::Flying;
        for _ in 0..70 {
            g.step(0.1);
        }
        let svg = map_svg(&g.view(Team::Blue), &g.rules);
        assert!(svg.contains(ROCK), "the mountains");
        assert!(
            svg.contains("B2 NO LINK 7 s"),
            "{}",
            svg.split("<text").filter(|t| t.contains("B2")).collect::<String>()
        );
        assert!(svg.contains("stroke-dasharray=\"2 3\""), "the radio shadow of the Red home tower");
    }

    #[test]
    fn the_map_shows_the_clock_the_scores_and_the_five_towers() {
        let g = Game::new(Rules::default());
        let svg = map_svg(&g.view(Team::Blue), &g.rules);
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
        assert!(svg.contains("BLUE MAP") && svg.contains("10:00 LEFT"), "{svg}");
        assert!(svg.contains("BLUE 0") && svg.contains("RED 0"));
        for name in ["WEST", "CENTRE", "EAST", "BLUE HOME", "RED HOME"] {
            assert!(svg.contains(name), "no {name}");
        }
        assert_eq!(svg.matches("<polygon").count(), g.rules.drones, "Blue's drones, and no Red drone it hasn't seen");
    }

    #[test]
    fn the_map_shows_the_lake_the_lanes_and_a_drone_that_flies_low() {
        let g = Game::new(Rules::default());
        let mut v = g.view(Team::Blue);
        let svg = map_svg(&v, &g.rules);
        assert!(svg.contains(LAKE) && svg.matches("<polyline").count() == 5, "{svg}");
        v.drones[1].low = true;
        assert!(map_svg(&v, &g.rules).contains(">B2 LOW<"));
    }

    #[test]
    fn a_tower_in_the_fog_shows_how_old_the_teams_news_is() {
        let mut g = Game::new(Rules::default());
        g.step(12.0);
        let svg = map_svg(&g.view(Team::Blue), &g.rules);
        assert!(svg.contains("EAST (12 s)"), "{svg}");
        assert!(svg.contains(">BLUE HOME<"), "a tower Blue holds is seen");
    }

    #[test]
    fn the_end_of_the_game_is_on_the_map() {
        let mut g = Game::new(Rules::default());
        g.outcome = Some(Outcome::Won(Team::Red));
        assert!(map_svg(&g.view(Team::Red), &g.rules).contains("RED WINS"));
        g.outcome = Some(Outcome::Draw);
        assert!(map_svg(&g.view(Team::Blue), &g.rules).contains(">DRAW<"));
    }
}
