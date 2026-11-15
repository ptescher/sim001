//! A scripted pilot for one drone of Towers, with no person at the sticks:
//! `towers_pilot --team blue|red --drone N [--host 127.0.0.1] [--port 5955] [--after S] [--for S]
//! [--roll R]`. It takes the seat of drone `N` of the team (from 1: Blue2 is `--team blue --drone 2`),
//! waits `--after` seconds of wall time before it does (0), flies a gentle bank (`--roll`, 0.3) for
//! `--for` seconds (30) and lets go: the drone is its computer's again. It prints what its seat
//! showed it. See `recreation_towers_ground::pilot`.

use recreation_towers::Team;
use recreation_towers_ground::pilot::{self, Script};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let secs = |name: &str, default: f32| arg(name).and_then(|s| s.parse().ok()).unwrap_or(default);
    let team = match arg("--team").as_deref() {
        Some("blue") => Team::Blue,
        Some("red") => Team::Red,
        _ => usage(),
    };
    let Some(drone) = arg("--drone").and_then(|d| d.parse::<usize>().ok()).filter(|&d| d >= 1) else { usage() };
    let host = arg("--host").unwrap_or_else(|| "127.0.0.1".into());
    let port: u16 = arg("--port").and_then(|p| p.parse().ok()).unwrap_or(recreation_towers::seat::PORT);
    let script = Script { roll: secs("--roll", 0.3), ..Script::bank() };
    std::thread::sleep(Duration::from_secs_f32(secs("--after", 0.0)));
    let flown = pilot::fly(
        &format!("{host}:{port}"),
        team,
        drone - 1,
        &script,
        Duration::from_secs_f32(secs("--for", 30.0)),
        &AtomicBool::new(false),
    );
    println!("{flown:#?}");
}

fn usage() -> ! {
    eprintln!("usage: towers_pilot --team blue|red --drone N [--host H] [--port P] [--after S] [--for S] [--roll R]");
    std::process::exit(2);
}
