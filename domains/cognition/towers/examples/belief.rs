//! How good Blue's belief about the Red drones is, against where they are: `cargo run --release
//! -p cognition_towers --example belief [GAMES] [layout=1]`.
//!
//! Each second of a game of the default commanders, for each Red drone in the air, the example
//! asks how far the belief's average is from the drone, how far its last sighting is, and how much
//! of the belief is within 100 m of the drone. A guess that knows nothing would put 3 % of its
//! weight there (the area of the circle over the field).

use cognition_towers::{Match, league_rules};
use recreation_towers::map::LayoutKind;
use recreation_towers::{Belief, Commander, Phase, Rules, Team};

#[derive(Default, Clone, Copy)]
struct Cell {
    n: f64,
    belief_err: f64,
    last_err: f64,
    within: f64,
    last_within: f64,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let games: u64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(20);
    let mut rules: Rules = league_rules();
    if args.iter().any(|a| a == "layout=1") {
        rules.layout = LayoutKind::Chain;
    }
    // Rows: seen now, unseen for 0 to 10 s, 10 to 30 s, 30 s or more, never seen.
    let names = ["seen now", "unseen 0-10 s", "unseen 10-30 s", "unseen 30+ s", "never seen"];
    let mut cells = [Cell::default(); 5];
    for seed in 1..=games {
        let mut m = Match::with_rules(rules, seed);
        let (mut blue, mut red) = (Commander::new(rules), Commander::new(rules));
        let mut belief = Belief::new(rules, Team::Blue);
        let mut next = 0.0;
        while m.result().is_none() {
            m.step(&mut blue, &mut red).expect("a step");
            if m.game.time >= next {
                next += 1.0;
                let view = m.game.view(Team::Blue);
                belief.update(&view);
                for k in 0..rules.drones {
                    let truth = &m.game.drones[m.game.drone(Team::Red, k)];
                    if truth.phase != Phase::Flying {
                        continue;
                    }
                    let seen = view.enemies.iter().find(|e| e.k == k);
                    let (row, last) = match seen {
                        Some(s) if s.seen => (0, s.at),
                        Some(s) => (1 + (s.age > 10.0) as usize + (s.age > 30.0) as usize, s.at),
                        None => (4, rules.base(Team::Red)),
                    };
                    let c = &mut cells[row];
                    c.n += 1.0;
                    c.belief_err += belief.mean(k).distance(truth.at) as f64;
                    c.last_err += last.distance(truth.at) as f64;
                    c.within += belief.chance_within(k, truth.at, 100.0) as f64;
                    c.last_within += (last.distance(truth.at) <= 100.0) as u8 as f64;
                }
            }
        }
    }
    println!("{games} games on the {:?}", rules.layout);
    println!(
        "{:<16} {:>8} {:>13} {:>14} {:>14} {:>14}",
        "", "samples", "belief error", "last-seen err", "belief <100 m", "last <100 m"
    );
    for (name, c) in names.iter().zip(cells) {
        if c.n == 0.0 {
            continue;
        }
        println!(
            "{name:<16} {:>8} {:>11.0} m {:>12.0} m {:>13.0} % {:>13.0} %",
            c.n,
            c.belief_err / c.n,
            c.last_err / c.n,
            100.0 * c.within / c.n,
            100.0 * c.last_within / c.n
        );
    }
}
