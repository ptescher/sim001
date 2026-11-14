//! What advice is worth, and what lateness costs: a commander with the lookahead as its advisor,
//! at several delays, against a commander with no advisor, on paired seeds, both sides:
//! `cargo run --release -p cognition_towers --example advice [SEEDS] [DELAY...]`.
//! The table in `plans/actors.md` (step 4) is read off its output.

use cognition_towers::{Captain, Comparison, compare_captains};
use recreation_towers::{Commander, Lookahead, Rules};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let n: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(100);
    let mut delays: Vec<f32> = args.filter_map(|s| s.parse().ok()).collect();
    if delays.is_empty() {
        delays = vec![0.0, 2.0, 5.0, 10.0, 30.0];
    }
    let rules = Rules::default();
    let seeds: Vec<u64> = (1..=n).collect();
    println!("advice: the lookahead as an advisor against no advisor, {n} seeds ({} games each)\n", 2 * n);
    println!("| delay | margin | won | lost | drawn |\n|---|---|---|---|---|");
    for delay in delays {
        let advised = || -> Box<dyn Captain> {
            Box::new(Commander::new(rules).with_lookahead(false).with_advisor(Lookahead::new(rules), delay))
        };
        let alone = || -> Box<dyn Captain> { Box::new(Commander::new(rules).with_lookahead(false)) };
        let Comparison { margin, error, won, lost, drawn, .. } = compare_captains(rules, &advised, &alone, &seeds)?;
        println!("| {delay} s | {margin:+.2} ± {error:.2} | {won} | {lost} | {drawn} |");
    }
    Ok(())
}
