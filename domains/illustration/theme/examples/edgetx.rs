//! Writes the EdgeTX radio themes (Night and Day): `cargo run -p illustration_theme --example edgetx -- domains/teleoperation/edgetx/THEMES`.
//! Copy the folders to the THEMES folder of the radio's SD card.

use illustration_theme::edgetx::folder;
use illustration_theme::{DAY, NIGHT};
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "domains/teleoperation/edgetx/THEMES".into()));
    for (p, name, info) in [(&NIGHT, "SIM001 Night", "The ground station console"), (&DAY, "SIM001 Day", "The console in sunlight")] {
        let dir = out.join(name);
        std::fs::create_dir_all(&dir).expect("create the theme folder");
        for (file, bytes) in folder(p, name, info) {
            std::fs::write(dir.join(&file), bytes).expect("write a theme file");
        }
        println!("wrote {}", dir.display());
    }
}
