//! Plays the allocations of drones to towers against each other, each pair on both sides of
//! every seed: `cargo run --release -p cognition_towers --example formations [SEEDS]`.
//! The cell is the row's mean score over the column's (positive: the row wins). After the games,
//! `spire_score=1 contest_drain=3 spire_jam_radius=120 tower_score=1 layout=1` set those rules
//! (`layout=1` is the chain, with eleven towers and formations that name the outposts).
//! `cognition_league nash` plays the same table and gives its equilibrium.

use cognition_towers::{compare_captains, formations, set_rule};
use recreation_towers::Rules;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n: u64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(40);
    let seeds: Vec<u64> = (1..=n).collect();
    let mut rules = Rules::default();
    for a in args.iter().skip(1) {
        set_rule(&mut rules, a)?;
    }
    let entries = formations(rules);
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
