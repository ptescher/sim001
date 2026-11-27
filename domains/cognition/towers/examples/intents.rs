//! What a slow thinker's intents are worth (`plans/advisors.md`, step 2): a commander with the
//! lookahead as its fast advisor and the deep thinker beside it posting intents, against the same
//! commander with the lookahead alone, on paired seeds, both sides:
//! `cargo run --release -p cognition_towers --example intents [SEEDS] [OFFSET] [DELAY...]`.
//! The deep thinker's intents reach the board DELAY s of game time after the map they came from
//! (20 s, and 0 s for what the lateness costs, unless given). Seeds are OFFSET+1 to OFFSET+SEEDS.
//! `FOLLOW` and `LEAVE` (in the environment) set how much the lookahead follows the intent
//! (`Lookahead::following`; the defaults are `FOLLOW` and `LEAVE` of `recreation_towers::advice`).

use cognition_towers::{Captain, Comparison, compare_captains};
use recreation_towers::{Commander, Deep, Lookahead, Rules};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let n: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(100);
    let offset: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut delays: Vec<f32> = args.filter_map(|s| s.parse().ok()).collect();
    if delays.is_empty() {
        delays = vec![20.0, 0.0];
    }
    let rules = Rules::default();
    let seeds: Vec<u64> = (offset + 1..=offset + n).collect();
    println!(
        "intents: lookahead and the deep thinker against the lookahead alone, seeds {}..{} ({} games each)\n",
        offset + 1,
        offset + n,
        2 * n
    );
    println!("| deep thinker's delay | margin | won | lost | drawn |\n|---|---|---|---|---|");
    let env = |k: &str, d: f32| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (follow, leave) = (env("FOLLOW", recreation_towers::FOLLOW), env("LEAVE", recreation_towers::LEAVE));
    println!("the lookahead follows intents by {follow} points a unit of weight, and {leave} off a tower to leave\n");
    let fast = move || {
        Commander::new(rules).with_lookahead(false).with_advisor(Lookahead::new(rules).following(follow, leave), 0.0)
    };
    for delay in delays {
        let both = move || -> Box<dyn Captain> { Box::new(fast().with_advisor(Deep::new(rules), delay)) };
        let alone = move || -> Box<dyn Captain> { Box::new(fast()) };
        let Comparison { margin, error, won, lost, drawn, .. } = compare_captains(rules, &both, &alone, &seeds)?;
        println!("| {delay} s | {margin:+.2} ± {error:.2} | {won} | {lost} | {drawn} |");
    }
    Ok(())
}
