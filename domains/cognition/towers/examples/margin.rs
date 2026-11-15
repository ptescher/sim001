//! Which margin the commander should ask of the lookahead's proposals, on seeds chosen in advance:
//! `cargo run --release -p cognition_towers --example margin [SEEDS] [OFFSET] [MARGIN...]`
//! plays seeds `OFFSET+1 ..= OFFSET+SEEDS` (default 400 from 0). Each margin is played against a
//! commander with no advisor, and against a commander with the lookahead at margin 2 (the head to
//! head), paired seeds, both sides. The margin of 5 was found on seeds 1..=400, so a test of it is
//! made on others (`margin 800 10000`).

use cognition_towers::{Captain, Comparison, compare_captains};
use recreation_towers::{Commander, Lookahead, Rules};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let n: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(400);
    let offset: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut margins: Vec<f32> = args.filter_map(|s| s.parse().ok()).collect();
    if margins.is_empty() {
        margins = vec![2.0, 3.0, 5.0, 7.0, 10.0];
    }
    let rules = Rules::default();
    let seeds: Vec<u64> = (offset + 1..=offset + n).collect();
    let with = |m: f32| -> Box<dyn Captain> {
        Box::new(
            Commander::new(rules).with_lookahead(false).with_advisor(Lookahead::new(rules), 0.0).with_advice_margin(m),
        )
    };
    let alone = || -> Box<dyn Captain> { Box::new(Commander::new(rules).with_lookahead(false)) };
    println!("margin: seeds {}..={} ({} games each)\n", offset + 1, offset + n, 2 * n);
    println!(
        "| margin | against no advisor | won | lost | against margin 2 | won | lost |\n|---|---|---|---|---|---|---|"
    );
    for m in margins {
        let a = compare_captains(rules, &|| with(m), &alone, &seeds)?;
        let b = compare_captains(rules, &|| with(m), &|| with(2.0), &seeds)?;
        let Comparison { margin, error, won, lost, .. } = a;
        println!(
            "| {m} | {margin:+.3} ± {error:.3} | {won} | {lost} | {:+.3} ± {:.3} (lower {:+.3}) | {} | {} |",
            b.margin,
            b.error,
            b.margin - 1.96 * b.error,
            b.won,
            b.lost
        );
    }
    Ok(())
}
