//! Plays one game of Towers by the default (v2) rules in the fast arena and writes a page that
//! replays it: `cargo run --release -p cognition_towers --example replay [SEED] [BLUE] [RED] [OUT]`.
//! BLUE and RED are what a drone does when a radio shadow cuts it off: `failsafe`, `persist` or
//! `surface`. OUT is the page (default `towers-replay.html`).

use cognition_towers::{Autonomy, Failsafe, Match, Persist, Prepared, Surface};
use recreation_towers::{Commander, Kind, Order, Phase, Rules, Team};
use serde_json::{Value, json};

fn policy(name: &str, rules: Rules) -> Result<Box<dyn Autonomy>, String> {
    Ok(match name {
        "failsafe" => Box::new(Failsafe),
        "persist" => Box::new(Persist),
        "surface" => Box::new(Surface { after: 5.0, rules }),
        other => return Err(format!("{other}: failsafe, persist or surface")),
    })
}

fn kind(k: Kind) -> &'static str {
    match k {
        Kind::Fort => "fort",
        Kind::Spire => "spire",
        Kind::Depot => "depot",
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize, default: &str| args.get(i).cloned().unwrap_or(default.to_string());
    let seed: u64 = arg(0, "7").parse().map_err(|_| "the seed is a number")?;
    let rules = Rules::default();
    let (blue_name, red_name) = (arg(1, "persist"), arg(2, "failsafe"));
    let out = arg(3, "towers-replay.html");
    let (blue_policy, red_policy) = (policy(&blue_name, rules)?, policy(&red_name, rules)?);
    let mut blue = Prepared { captain: Commander::new(rules), autonomy: blue_policy.as_ref() };
    let mut red = Prepared { captain: Commander::new(rules), autonomy: red_policy.as_ref() };

    let mut m = Match::with_rules(rules, seed);
    let tower_names: Vec<String> = m.game.towers.iter().map(|t| t.name.clone()).collect();
    let mut frames: Vec<Value> = vec![];
    let mut next = 0.0f32;
    while m.result().is_none() {
        if m.game.time >= next {
            next += 1.0;
            let g = &m.game;
            let drones: Vec<Value> = g
                .drones
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    let order = match m.flyers[i].order {
                        None => "-".to_string(),
                        Some(Order::Take(t)) => format!("take {}", tower_names[t]),
                        Some(Order::Goto(_)) => "go to a point".into(),
                        Some(Order::Hold) => "hold".into(),
                        Some(Order::Home) => "home".into(),
                    };
                    json!({
                        "x": d.at.y, "y": d.at.x, "alt": d.alt,
                        "team": if d.team == Team::Blue { 0 } else { 1 },
                        "class": d.class.name(),
                        "phase": match d.phase { Phase::Flying => "flying", Phase::Ground => "ground", Phase::Down => "down" },
                        "bat": g.battery[i], "cap": g.rules.battery(d.class),
                        "cut": g.jammed[i], "order": order,
                    })
                })
                .collect();
            let towers: Vec<Value> = g
                .towers
                .iter()
                .map(|t| {
                    json!({
                        "c": t.control,
                        "h": t.holder.map(|h| if h == Team::Blue { 0 } else { 1 }),
                        "fight": t.contested,
                    })
                })
                .collect();
            frames.push(json!({ "t": g.time, "score": g.score, "towers": towers, "drones": drones }));
        }
        m.step(&mut blue, &mut red)?;
    }
    let result = m.result().unwrap();
    let g = &m.game;
    let data = json!({
        "title": format!("Towers v2, seed {seed}: Blue {blue_name}, Red {red_name}"),
        "field": g.rules.field_radius,
        "circle": g.rules.circle_radius,
        "win": g.rules.win_score,
        "duration": g.rules.duration,
        "lake": { "x": g.map.lake.at.y, "y": g.map.lake.at.x, "r": g.map.lake.radius },
        "lanes": g.map.lanes.iter().map(|l| l.points.iter().map(|p| json!([p.y, p.x])).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "bases": g.map.bases.iter().map(|p| json!([p.y, p.x])).collect::<Vec<_>>(),
        "towers": g.towers.iter().map(|t| json!({
            "name": t.name, "kind": kind(t.kind), "x": t.at.y, "y": t.at.x,
            "shadow": g.rules.shadow(t.kind), "points": g.rules.points(t.kind),
        })).collect::<Vec<_>>(),
        "rules": {
            "mesh": g.rules.mesh_range, "timeout": g.rules.link_timeout, "contest": g.rules.contest_drain,
            "decay": g.rules.decay_time, "respawn": g.rules.respawn_time,
        },
        "result": {
            "outcome": format!("{:?}", result.outcome), "score": result.score, "length": result.length,
            "downs": result.downs, "dark": result.dark,
        },
        "frames": frames,
    });
    let page = include_str!("replay.html").replace("__DATA__", &data.to_string());
    std::fs::write(&out, page).map_err(|e| format!("{out}: {e}"))?;
    println!(
        "{out}: {} s, {:?}, score {:.0} to {:.0}, drones down {:?}",
        result.length, result.outcome, result.score[0], result.score[1], result.downs
    );
    Ok(())
}
