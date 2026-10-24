//! `cargo run` from the root starts the launcher (domains/teleoperation/launcher), not the sim.
//! The launcher is a separate package (GPUI), so this only hands over to cargo.
//! The sim is `cargo run --release --bin sim001 -- ...`.

use std::process::{Command, exit};

fn main() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(env!("CARGO_MANIFEST_DIR")).args(["run", "-q", "--release"]);
    cmd.args(["-p", "teleoperation_launcher", "--"]).args(std::env::args().skip(1));
    match cmd.status() {
        Ok(s) => exit(s.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("can't run cargo: {e}");
            exit(1);
        }
    }
}
