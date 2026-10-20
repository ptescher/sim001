//! What a test needs from outside (a flight controller, a database) is started by the harness,
//! and when it cannot be had the tests that need it are "not supported yet" with how to get it.

#[path = "../examples/hand_launch/world.rs"]
mod world;

use expectation_harness::service::{Running, Service, Services};
use expectation_harness::{Acceptance, RunOptions};
use expectation_story::Status;
use std::sync::Mutex;
use std::path::Path;
use std::sync::Arc;

fn suite() -> Acceptance {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/hand_launch");
    Acceptance::load(dir).unwrap_or_else(|e| panic!("{e}")).with_coverage()
}

struct Database {
    up: Arc<Mutex<bool>>,
    starts: Arc<Mutex<u32>>,
    missing: bool,
}

struct DatabaseRunning(Arc<Mutex<bool>>);

impl Service for Database {
    fn name(&self) -> &str {
        "database"
    }
    fn unavailable(&self) -> Option<String> {
        self.missing.then(|| "install PostgreSQL with `brew install postgresql`".to_string())
    }
    fn start(&self) -> Result<Box<dyn Running>, String> {
        *self.up.lock().unwrap() = true;
        *self.starts.lock().unwrap() += 1;
        Ok(Box::new(DatabaseRunning(self.up.clone())))
    }
}

impl Running for DatabaseRunning {}

impl Drop for DatabaseRunning {
    fn drop(&mut self) {
        *self.0.lock().unwrap() = false;
    }
}

#[test]
fn tests_that_need_what_is_missing_are_not_supported_yet_and_say_how_to_get_it() {
    let a = suite();
    let up = Arc::new(Mutex::new(false));
    let mut services = Services::new();
    services.add(Database { up: up.clone(), starts: Arc::default(), missing: true });
    let mut driver = world::driver(false).needs(&["database"]);
    let report = a.run_with(&mut driver, &RunOptions::default(), &mut services);
    assert!(report.results.iter().all(|r| r.status == Status::Unsupported), "{:?}", report.results.iter().map(|r| r.status).collect::<Vec<_>>());
    let why = report.results[0].start.error.clone().unwrap();
    assert!(why.contains("database") && why.contains("brew install postgresql"), "{why}");
    assert!(report.passed(), "a test that cannot run is not a failed test");
    assert!(!*up.lock().unwrap(), "nothing was started");
}

#[test]
fn what_is_available_is_started_once_for_all_the_tests_and_stopped_after() {
    let a = suite();
    let (up, starts) = (Arc::new(Mutex::new(false)), Arc::new(Mutex::new(0)));
    let mut services = Services::new();
    services.add(Database { up: up.clone(), starts: starts.clone(), missing: false });
    let mut driver = world::driver(false).needs(&["database"]);
    let report = a.run_with(&mut driver, &RunOptions::default(), &mut services);
    assert!(report.results.iter().any(|r| r.status == Status::Passed), "the tests ran");
    assert!(report.passed());
    assert_eq!(*starts.lock().unwrap(), 1);
    assert!(*up.lock().unwrap(), "still up until the run's services are dropped");
    drop(services);
    assert!(!*up.lock().unwrap(), "stopped with the run");
}

#[test]
fn a_driver_that_needs_nothing_runs_as_before() {
    let a = suite();
    let mut services = Services::new();
    let report = a.run_with(&mut world::driver(false), &RunOptions::default(), &mut services);
    assert!(report.passed());
    assert!(report.results.iter().any(|r| r.status == Status::Passed));
}

#[test]
fn a_service_the_driver_wants_fresh_is_started_afresh_for_each_test() {
    let a = suite();
    let (up, starts) = (Arc::new(Mutex::new(false)), Arc::new(Mutex::new(0)));
    let mut services = Services::new();
    services.add(Database { up, starts: starts.clone(), missing: false });
    let mut driver = world::driver(false).needs(&["database"]).fresh(&["database"]);
    let report = a.run_with(&mut driver, &RunOptions::default(), &mut services);
    let ran = report.results.iter().filter(|r| r.status != Status::Unsupported).count() as u32;
    assert!(ran > 3);
    assert_eq!(*starts.lock().unwrap(), ran, "one fresh start for every test that ran");
}
