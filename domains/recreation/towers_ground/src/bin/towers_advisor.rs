//! An advisor for Towers as a program of its own, for a station to start (`towers_commander
//! --advisor towers_advisor`, `towers_server --blue-advisor "towers_advisor --claude"`): it reads
//! one JSON line for each map on its input and writes one line of proposals on its output (see
//! `recreation_towers_ground::advisors::ToAdvisor`).
//!
//! `towers_advisor` serves the lookahead. `towers_advisor --claude [--model NAME] [--every S]`
//! asks Claude (`claude -p`, or the command in `SIM001_CLAUDE`) every `S` s of the game (default 8)
//! and costs money: it is not run by any test.

use recreation_towers::{Advise, Lookahead};
use recreation_towers_ground::advisors::serve;
use recreation_towers_ground::claude::ClaudeAdvisor;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!("usage: towers_advisor [--claude [--model NAME] [--every SECONDS]]");
        return;
    }
    let claude = args.iter().any(|a| a == "--claude");
    let (model, every) = (arg("--model"), arg("--every").and_then(|e| e.parse::<f32>().ok()));
    let stdin = std::io::stdin();
    let result = serve(stdin.lock(), std::io::stdout().lock(), |rules, _| -> Box<dyn Advise> {
        if claude {
            let mut a = ClaudeAdvisor::new();
            if let Some(m) = &model {
                a = a.with_model(m);
            }
            if let Some(e) = every {
                a.every = e;
            }
            Box::new(a)
        } else {
            Box::new(Lookahead::new(rules))
        }
    });
    if let Err(e) = result {
        eprintln!("towers_advisor: {e}");
        std::process::exit(1);
    }
}
