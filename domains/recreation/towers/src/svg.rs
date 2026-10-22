//! A team's map as a picture (SVG), drawn as the commander app draws it: the fog, what the team
//! sees, the towers with their circles and control bars, the drones and their orders, and the
//! clock and the scores along the top. The tests with no window put it in the docs.

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

/// `view` as `view.team` sees it, the whole field fitted under the top bar, north up.
pub fn map_svg(view: &TeamView, rules: &Rules) -> String {
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
    eyes.extend(view.drones.iter().filter(|d| d.phase != Phase::Down).map(|d| (d.at, rules.drone_vision)));
    eyes.extend(view.towers.iter().filter(|t| t.holder == Some(view.team)).map(|t| (t.at, rules.tower_vision)));
    for (p, range) in eyes {
        let (x, y) = at(p);
        let _ = write!(s, r##"<circle cx="{x:.1}" cy="{y:.1}" r="{:.1}" fill="{FIELD}"/>"##, range * scale);
    }
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
        s += &drone(x, y, d.vel, colour, fill, 1.0);
        // Drones on top of each other (at the base) get their labels one under the other.
        let below = view.drones[..j].iter().filter(|o| (at(o.at).0 - x).hypot(at(o.at).1 - y) < 16.0).count();
        let _ = write!(
            s,
            r##"<text x="{:.1}" y="{:.1}" fill="{colour}">{}{}</text>"##,
            x + 12.0,
            y - 8.0 + 11.0 * below as f32,
            letter(view.team),
            d.k + 1
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
    fn the_map_shows_the_clock_the_scores_and_the_five_towers() {
        let g = Game::new(Rules::default());
        let svg = map_svg(&g.view(Team::Blue), &g.rules);
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
        assert!(svg.contains("BLUE MAP") && svg.contains("8:00 LEFT"), "{svg}");
        assert!(svg.contains("BLUE 0") && svg.contains("RED 0"));
        for name in ["WEST", "CENTRE", "EAST", "BLUE HOME", "RED HOME"] {
            assert!(svg.contains(name), "no {name}");
        }
        assert_eq!(svg.matches("<polygon").count(), 3, "Blue's three drones, and no Red drone it hasn't seen");
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
