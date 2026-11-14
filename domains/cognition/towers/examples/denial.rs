//! Compares what a drone does when a radio shadow cuts it off, against the failsafe, on paired
//! seeds: `cargo run --release -p cognition_towers --example denial [GAMES]`.

use cognition_towers::{After, Autonomy, Chain, Failsafe, Persist, Timed, compare};
use recreation_towers::Order;

fn main() -> Result<(), String> {
    let n: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(100);
    let seeds: Vec<u64> = (1..=n).collect();
    let policies: Vec<Box<dyn Autonomy>> = vec![
        Box::new(Persist),
        Box::new(Chain),
        Box::new(Timed { next: 20.0, home: 60.0 }),
        Box::new(Timed { next: 30.0, home: 90.0 }),
        Box::new(After { after: 5.0, then: Order::Hold }),
        Box::new(After { after: 5.0, then: Order::Home }),
        Box::new(After { after: 30.0, then: Order::Home }),
    ];
    println!(
        "{:<22} {:>9} {:>11} {:>5} {:>5} {:>5} {:>8} {:>8}",
        "against failsafe", "margin", "± error", "won", "lost", "draw", "dark", "dark(f)"
    );
    for p in &policies {
        let c = compare(p.as_ref(), &Failsafe, &seeds)?;
        println!(
            "{:<22} {:>+9.3} {:>11.3} {:>5} {:>5} {:>5} {:>7.0}s {:>7.0}s",
            p.name(),
            c.margin,
            c.error,
            c.won,
            c.lost,
            c.drawn,
            c.dark[0],
            c.dark[1]
        );
    }
    // The queue policies against persisting: the same orders, with a queue beyond the first one.
    println!("\n{:<22} {:>9} {:>11} {:>5} {:>5} {:>5}", "against persist", "margin", "± error", "won", "lost", "draw");
    let queued: Vec<Box<dyn Autonomy>> =
        vec![Box::new(Chain), Box::new(Timed { next: 20.0, home: 60.0 }), Box::new(Timed { next: 30.0, home: 90.0 })];
    for p in &queued {
        let c = compare(p.as_ref(), &Persist, &seeds)?;
        println!("{:<22} {:>+9.3} {:>11.3} {:>5} {:>5} {:>5}", p.name(), c.margin, c.error, c.won, c.lost, c.drawn);
    }
    Ok(())
}
