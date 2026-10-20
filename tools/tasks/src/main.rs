//! The project's chores, in Rust:
//!
//!   cargo run --release -p sim001_tasks -- docs [--no-station]   every acceptance runner, then the manual
//!   cargo run --release -p sim001_tasks -- pack-docs             the manual's results and screenshots, for Pages
//!
//! The runners ask for what they need from outside (ArduPilot, Docker) through the expectation
//! framework's services, so there is nothing to start here.

mod docs;
mod limit;
mod pack;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("the checkout");
    let code = match args.first().map(String::as_str) {
        Some("docs") => docs::run(&root, args.iter().any(|a| a == "--no-station")),
        Some("pack-docs") => pack::run(&root),
        _ => {
            eprintln!("usage: tasks docs [--no-station] | tasks pack-docs");
            2
        }
    };
    std::process::exit(code);
}
