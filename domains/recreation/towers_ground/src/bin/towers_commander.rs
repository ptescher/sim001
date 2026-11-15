//! A computer commander for one team of Towers, as a seat of its own:
//! `towers_commander --team blue|red [--host 127.0.0.1] [--port 5955] [--idle]
//! [--advisor lookahead|none|COMMAND]`. It takes the
//! team's seat, flies its drones from the map its drones' telemetry makes, and gives them orders
//! once a second. If a person holds the seat it waits for it to be free; if a person joins the
//! team, it leaves. `--idle` sits in the seat and gives no orders.
//! `--advisor` is the advisor beside the commander, on a thread of its own (it never waits for it):
//! `lookahead` (the default), `none`, or a command that serves an advisor, run with `sh -c`, such as
//! `towers_advisor` or `"towers_advisor --claude"` (which costs money: see `towers_advisor`). Ends when the game is over.

use recreation_towers::Team;
use recreation_towers_ground::advisors::AdvisorChoice;
use recreation_towers_ground::commander::{self, Brain};
use std::sync::atomic::AtomicBool;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let team = match arg("--team").as_deref() {
        Some("blue") => Team::Blue,
        Some("red") => Team::Red,
        _ => {
            eprintln!(
                "usage: towers_commander --team blue|red [--host H] [--port P] [--idle] [--advisor lookahead|none|COMMAND]"
            );
            std::process::exit(2);
        }
    };
    let host = arg("--host").unwrap_or_else(|| "127.0.0.1".into());
    let port: u16 = arg("--port").and_then(|p| p.parse().ok()).unwrap_or(recreation_towers::seat::PORT);
    let brain = if args.iter().any(|a| a == "--idle") { Brain::Idle } else { Brain::Play };
    let advisor = match arg("--advisor").map_or(Ok(AdvisorChoice::Lookahead), |a| a.parse()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("--advisor: {e}");
            std::process::exit(2);
        }
    };
    commander::run_with(&format!("{host}:{port}"), team, brain, None, &AtomicBool::new(false), &advisor);
}
