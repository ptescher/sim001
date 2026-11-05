//! Compares on-board policies against "fly the order on" on paired seeds, each played on both
//! sides: `cargo run --release -p cognition_towers --example edge [GAMES]`.

use cognition_towers::{Edge, FinishThenReturn, Keep, Outnumbered, compare_edge};
use recreation_towers::Rules;

fn main() -> Result<(), String> {
    let n: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(100);
    let seeds: Vec<u64> = (1..=n).collect();
    let policies: Vec<Box<dyn Edge>> = vec![
        Box::new(Keep),
        Box::new(Outnumbered { n: 1 }),
        Box::new(Outnumbered { n: 2 }),
        Box::new(Outnumbered { n: 3 }),
        Box::new(FinishThenReturn),
    ];
    println!(
        "{:<34} {:>8} {:>8} {:>5} {:>5} {:>5} {:>8} {:>8} {:>6}",
        "against persist", "margin", "± err", "won", "lost", "draw", "dark", "dark(p)", "downs"
    );
    for p in &policies {
        let c = compare_edge(Rules::default(), p.as_ref(), &seeds)?;
        println!(
            "{:<34} {:>+8.3} {:>8.3} {:>5} {:>5} {:>5} {:>7.0}s {:>7.0}s {:>6.2}",
            p.name(),
            c.margin,
            c.error,
            c.won,
            c.lost,
            c.drawn,
            c.dark[0],
            c.dark[1],
            c.downs[0]
        );
    }
    Ok(())
}
