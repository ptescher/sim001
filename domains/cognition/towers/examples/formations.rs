//! Plays the allocations of drones to towers against each other, each pair on both sides of
//! every seed: `cargo run --release -p cognition_towers --example formations [SEEDS]`.
//! The cell is the row's mean score over the column's (positive: the row wins).

use cognition_towers::{Captain, Formation, Persist, Post, Prepared, compare_captains};
use recreation_towers::{Commander, Rules};

type Make = Box<dyn Fn() -> Box<dyn Captain>>;

fn formation(name: &'static str, posts: [Post; 5]) -> (&'static str, Make) {
    let rules = Rules::default();
    (name, Box::new(move || Box::new(Prepared { captain: Formation::new(name, posts, rules), autonomy: Persist })))
}

fn main() -> Result<(), String> {
    let n: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(40);
    let seeds: Vec<u64> = (1..=n).collect();
    use Post::*;
    let rules = Rules::default();
    let entries: Vec<(&str, Make)> = vec![
        ("commander", Box::new(move || Box::new(Prepared { captain: Commander::new(rules), autonomy: Persist }))),
        formation("centre rush", [Centre; 5]),
        formation("wings", [West, West, East, East, Centre]),
        formation("balanced", [West, East, Centre, Centre, Centre]),
        formation("no centre", [West, West, East, East, East]),
        formation("raid", [West, East, EnemyHome, Centre, EnemyHome]),
        formation("turtle", [OwnHome, West, East, OwnHome, West]),
        formation("heavy centre", [West, East, West, Centre, East]),
    ];
    print!("{:<14}", "row vs column");
    for (name, _) in &entries {
        print!("{:>13.12}", name);
    }
    println!("{:>9}", "mean");
    for (a, make_a) in &entries {
        print!("{a:<14}");
        let mut sum = 0.0;
        for (b, make_b) in &entries {
            if a == b {
                print!("{:>13}", "-");
                continue;
            }
            let c = compare_captains(rules, &|| make_a(), &|| make_b(), &seeds)?;
            sum += c.margin;
            print!("{:>+13.2}", c.margin);
        }
        println!("{:>+9.2}", sum / (entries.len() - 1) as f32);
    }
    Ok(())
}
