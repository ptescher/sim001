//! Plays the allocations of drones to towers against each other, each pair on both sides of
//! every seed: `cargo run --release -p cognition_towers --example formations [SEEDS]`.
//! The cell is the row's mean score over the column's (positive: the row wins). After the games,
//! `spire_score=1 contest_drain=3 spire_jam_radius=120 tower_score=1` set those rules.

use cognition_towers::{Captain, Formation, Persist, Post, Prepared, compare_captains};
use recreation_towers::{Commander, Rules};

type Make = Box<dyn Fn() -> Box<dyn Captain>>;

fn formation(rules: Rules, name: &'static str, posts: [Post; 5]) -> (&'static str, Make) {
    (name, Box::new(move || Box::new(Prepared { captain: Formation::new(name, posts, rules), autonomy: Persist })))
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n: u64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(40);
    let seeds: Vec<u64> = (1..=n).collect();
    use Post::*;
    let mut rules = Rules::default();
    for a in args.iter().skip(1) {
        let (k, v) = a.split_once('=').ok_or(format!("{a}: want name=number"))?;
        let v: f32 = v.parse().map_err(|_| format!("{a}: not a number"))?;
        match k {
            "spire_score" => rules.spire_score = v,
            "tower_score" => rules.tower_score = v,
            "contest_drain" => rules.contest_drain = v,
            "spire_jam_radius" => rules.spire_jam_radius = v,
            "win_score" => rules.win_score = v,
            _ => return Err(format!("no rule called {k}")),
        }
    }
    let entries: Vec<(&str, Make)> = vec![
        ("commander", Box::new(move || Box::new(Prepared { captain: Commander::new(rules), autonomy: Persist }))),
        formation(rules, "centre rush", [Centre; 5]),
        formation(rules, "wings", [West, West, East, East, Centre]),
        formation(rules, "balanced", [West, East, Centre, Centre, Centre]),
        formation(rules, "no centre", [West, West, East, East, East]),
        formation(rules, "raid", [West, East, EnemyHome, Centre, EnemyHome]),
        formation(rules, "turtle", [OwnHome, West, East, OwnHome, West]),
        formation(rules, "heavy centre", [West, East, West, Centre, East]),
    ];
    print!("{:<14}", "row vs column");
    for (name, _) in &entries {
        print!("{:>13.12}", name);
    }
    println!("{:>9}", "mean");
    let mut beats_all = vec![];
    for (a, make_a) in &entries {
        print!("{a:<14}");
        let (mut sum, mut worst) = (0.0, f32::MAX);
        for (b, make_b) in &entries {
            if a == b {
                print!("{:>13}", "-");
                continue;
            }
            let c = compare_captains(rules, &|| make_a(), &|| make_b(), &seeds)?;
            sum += c.margin;
            worst = worst.min(c.margin - 2.0 * c.error);
            print!("{:>+13.2}", c.margin);
        }
        println!("{:>+9.2}", sum / (entries.len() - 1) as f32);
        if worst > 0.0 {
            beats_all.push(*a);
        }
    }
    println!("beats every other at 2 standard errors: {beats_all:?}");
    Ok(())
}
