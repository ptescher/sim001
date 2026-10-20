//! `sim001_ios_expect [SUITE] [--docs DIR] [--only TEXT] [--device NAME] [--address ADDR]`:
//! runs the acceptance tests in `expectations/` against the iOS ground station in the Simulator.
//! It needs a vehicle: ArduPilot SITL on tcp:5760 (`ardupilot/run_sitl.sh`), which the Simulator
//! reaches on the Mac's localhost.

mod bridge;
mod driver;

use expectation_harness::{Acceptance, RunOptions};
use std::path::PathBuf;
use std::time::Instant;

const CONTROL_PORT: u16 = 5790;
const BRIDGE_PORT: u16 = 5791;

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let (mut suite, mut docs, mut only) = (root.join("expectations"), None, None);
    let mut device = "iPhone 17".to_string();
    let mut address = "tcpout:127.0.0.1:5760".to_string();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--docs" => docs = args.next().map(PathBuf::from),
            "--only" => only = args.next().map(|s| s.to_lowercase()),
            "--device" => device = args.next().unwrap_or(device),
            "--address" => address = args.next().unwrap_or(address),
            _ => suite = PathBuf::from(a),
        }
    }
    let mut acceptance = Acceptance::load_for(&suite, "iOS").unwrap_or_else(|e| {
        eprint!("{e}");
        std::process::exit(1)
    }).with_coverage();
    if let Some(only) = &only {
        acceptance.plans.retain(|p| p.title.to_lowercase().contains(only));
    }
    println!("expect: {} tests from {}", acceptance.plans.len(), suite.display());

    let app = root.join("domains/teleoperation/ios/App");
    println!("expect: building the app and starting it in the Simulator ({device}); the first time takes a few minutes");
    let bridge = bridge::Bridge::start(&app, &device, BRIDGE_PORT, CONTROL_PORT, &address).unwrap_or_else(|e| {
        eprintln!("expect: {e}");
        std::process::exit(2)
    });
    let mut driver = driver::IosDriver::new(bridge, CONTROL_PORT);
    // Screenshots go in this platform's tree of the manual.
    let opts = RunOptions { docs_dir: docs.as_ref().map(|d| acceptance.tree(d)), patience: 200 };
    let mut report = expectation_story::Report::default();
    for plan in &acceptance.plans {
        let began = Instant::now();
        let r = expectation_harness::run(&acceptance.suite, std::slice::from_ref(plan), &mut driver, &opts);
        let r = r.results.into_iter().next().unwrap();
        match r.status {
            expectation_story::Status::Passed => println!("expect: ✓ {} ({:.0} s)", plan.title, began.elapsed().as_secs_f32()),
            // Another system's test, or a feature the app doesn't have yet.
            expectation_story::Status::Unsupported => {}
            _ => println!("expect: ✗ {} ({:.0} s)", plan.title, began.elapsed().as_secs_f32()),
        }
        report.results.push(r);
    }
    if let Some(dir) = &docs {
        match acceptance.publish(dir, "ios", &report) {
            Ok(_) => println!("expect: wrote the docs to {}", dir.display()),
            Err(e) => eprintln!("expect: couldn't write the docs to {}: {e}", dir.display()),
        }
    }
    let unsupported = report.count(expectation_story::Status::Unsupported);
    if unsupported > 0 {
        println!("expect: {unsupported} tests need things the iOS app doesn't have yet");
    }
    drop(driver);
    if !report.passed() {
        eprint!("{}", report.failures(&acceptance.suite, &acceptance.plans));
        std::process::exit(1);
    }
}
