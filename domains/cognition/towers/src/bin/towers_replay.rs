//! Makes the replay page of a recorded game of Towers (`plans/game-tooling.md`, step 3):
//!
//!   cargo run --release -p cognition_towers --bin towers_replay -- RECORD... [--out DIR]
//!
//! For each record (`towers_games` or `towers_server --record` writes them) a page that stands
//! alone, `<record>.html` beside it or in DIR: the field with the drones, their trails and orders,
//! the towers with their control, who sees whom, each team's view or everything, the score over
//! time, a timeline to scrub and play, each team's board (what each advisor said, what the
//! commander took), and the game's events.

use recreation_towers::record;
use std::path::{Path, PathBuf};

const PAGE: &str = include_str!("towers_replay.html");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let records: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--out"))
        .map(|(_, a)| a)
        .collect();
    if records.is_empty() {
        eprintln!("usage: towers_replay RECORD... [--out DIR]");
        std::process::exit(2);
    }
    let mut failed = false;
    for r in records {
        match page(Path::new(r), out.as_deref()) {
            Ok(p) => println!("{}", p.display()),
            Err(e) => {
                eprintln!("towers_replay: {r}: {e}");
                failed = true;
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}

/// Writes the page of the record at `path`; returns where.
fn page(path: &Path, out: Option<&Path>) -> Result<PathBuf, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    // It must read as a record; the page gets the lines as they are.
    let lines = record::read(&text)?;
    if !matches!(lines.first(), Some(record::Line::Header(_))) {
        return Err("it does not start with a header".into());
    }
    if !lines.iter().any(|l| matches!(l, record::Line::Tick(_))) {
        return Err("it has no ticks".into());
    }
    let json: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let html = PAGE.replace("__RECORD__", &format!("[{}]", json.join(",\n")));
    let name = path.with_extension("html");
    let dest = match out {
        Some(dir) => {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            dir.join(name.file_name().unwrap_or_default())
        }
        None => name,
    };
    std::fs::write(&dest, html).map_err(|e| format!("{}: {e}", dest.display()))?;
    Ok(dest)
}
