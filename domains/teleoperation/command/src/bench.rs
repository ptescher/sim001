//! The commander app on the field platform (plans/unified-link.md): the same app and the same
//! suite, read as `field`, against a bench: the app, a real ground radio on a serial line and a real
//! flight controller behind it (or an ArduPlane SITL behind a real radio hop).
//!
//! The radio and the flight controller are services of the driver. Without the bench every test is
//! "not supported yet", which is a gap, not a pass; the exit code stays 0, since the bench is not
//! there to fail.
//!
//! `SIM001_FIELD_RADIO` is the serial device of the ground radio (`/dev/cu.usbserial-…`), and
//! `SIM001_FIELD_FC` the one of the flight controller on the bench (its own USB port, for the
//! rig to power-cycle and read). The field driver stages the world of a test by flying the drone
//! to where the step says; that is the bench's to write when it is built.

use expectation_harness::Acceptance;
use expectation_harness::service::{Present, Services};
use std::path::{Path, PathBuf};

fn device(var: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("/dev/unset-{var}")))
}

/// The services the bench needs.
pub fn services() -> Services {
    Services::new()
        .with(
            Present::new("ground-radio", &device("SIM001_FIELD_RADIO"))
                .hint("plug in the ground radio and set SIM001_FIELD_RADIO to its serial device"),
        )
        .with(
            Present::new("flight-controller", &device("SIM001_FIELD_FC"))
                .hint("put a flight controller on the bench and set SIM001_FIELD_FC to its serial device"),
        )
}

/// Runs the section's tests on the field platform: the exit code.
pub fn run(suite: &Path, only: Option<&str>) -> i32 {
    let mut acceptance = match Acceptance::load_for(suite, "field") {
        Ok(a) => a,
        Err(e) => {
            eprint!("{e}");
            return 1;
        }
    };
    let section = acceptance.suite.sections.iter().find(|s| s.slug == "towers").map(|s| s.id);
    let ours = section.map(|s| acceptance.suite.subtree(s)).unwrap_or_default();
    acceptance.plans.retain(|p| ours.contains(&p.section));
    if let Some(only) = only {
        let only = only.to_lowercase();
        acceptance.plans.retain(|p| p.title.to_lowercase().contains(&only));
    }
    let mut services = services();
    let missing = services.missing(&["ground-radio", "flight-controller"]);
    for why in &missing {
        println!("not supported yet: {why}");
    }
    if missing.is_empty() {
        println!("the bench is here, but the field driver cannot stage a game on it yet");
    }
    println!("expect: field: {} tests, 0 passed, {} not supported yet", acceptance.plans.len(), acceptance.plans.len());
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_the_bench_every_test_of_the_field_platform_is_not_supported_yet() {
        let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../expectations");
        assert_eq!(run(&suite, None), 0);
        assert!(!services().missing(&["ground-radio", "flight-controller"]).is_empty());
    }
}
