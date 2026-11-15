//! A computer commander for one team of Towers, as a seat of its own:
//! `towers_commander --team blue|red [--host 127.0.0.1] [--port 5955] [--idle]`. It takes the
//! team's seat, flies its drones from the map its drones' telemetry makes, and gives them orders
//! once a second. If a person holds the seat it waits for it to be free; if a person joins the
//! team, it leaves. `--idle` sits in the seat and gives no orders. Ends when the game is over.

use recreation_towers::Team;
use recreation_towers_ground::commander::{self, Brain};
use std::sync::atomic::AtomicBool;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let team = match arg("--team").as_deref() {
        Some("blue") => Team::Blue,
        Some("red") => Team::Red,
        _ => {
            eprintln!("usage: towers_commander --team blue|red [--host H] [--port P] [--idle]");
            std::process::exit(2);
        }
    };
    let host = arg("--host").unwrap_or_else(|| "127.0.0.1".into());
    let port: u16 = arg("--port").and_then(|p| p.parse().ok()).unwrap_or(recreation_towers::seat::PORT);
    let brain = if args.iter().any(|a| a == "--idle") { Brain::Idle } else { Brain::Play };
    commander::run(&format!("{host}:{port}"), team, brain, None, &AtomicBool::new(false));
}
