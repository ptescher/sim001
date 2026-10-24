//! The time one acceptance suite may take. Every runner starts a [`Budget`] and asks it before each
//! test: a suite that is out of time marks the tests it did not reach as failed (so the manual says
//! so) and still writes the results of the ones it ran. `tasks docs` runs the suites side by side on
//! a big box and gives each the same budget, plus a short grace for writing its results.

use expectation_harness::{Driver, Need};
use expectation_model::Suite;
use expectation_story::{Check, Plan, PlanResult, Status};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

static BEGAN: OnceLock<Instant> = OnceLock::new();

/// Starts the clock: a program that runs a suite calls this first thing in `main`, so that loading
/// the window and starting the servers count against the suite's time, as the `tasks` kill does.
/// Without it the clock starts at the first [`Budget`].
pub fn begin() {
    BEGAN.get_or_init(Instant::now);
}

/// How long a suite may run, when `SIM001_SUITE_BUDGET` does not say.
pub const DEFAULT_SECS: u64 = 120;

#[derive(Clone, Copy, Debug)]
pub struct Budget {
    began: Instant,
    limit: Duration,
}

impl Budget {
    /// Starts counting now, for `SIM001_SUITE_BUDGET` seconds (default 120).
    pub fn from_env() -> Self {
        let secs = std::env::var("SIM001_SUITE_BUDGET").ok().and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_SECS);
        Budget::new(Duration::from_secs(secs))
    }

    pub fn new(limit: Duration) -> Self {
        Budget { began: *BEGAN.get_or_init(Instant::now), limit }
    }

    pub fn limit(&self) -> Duration {
        self.limit
    }

    pub fn remaining(&self) -> Duration {
        self.limit.saturating_sub(self.began.elapsed())
    }

    pub fn spent(&self) -> bool {
        self.remaining().is_zero()
    }

    /// The most a test may take: its own limit, or what is left of the suite's time.
    pub fn cap(&self, test: Duration) -> Duration {
        test.min(self.remaining())
    }

    /// What a test that was not reached gets: failed, with the reason, unless the driver could not
    /// have run it anyway (that is "not supported yet", as it is when there is time).
    pub fn ran_out(&self, suite: &Suite, plan: &Plan, driver: &dyn Driver) -> PlanResult {
        let v = &suite.vocabulary;
        let observe = |c: &Check| Need::Observe(v.var(c.var));
        let mut needs: Vec<Need> = plan.start.vars().map(|x| Need::Arrange(v.var(x))).collect();
        needs.extend(plan.start_checks.iter().map(observe));
        for step in &plan.steps {
            needs.extend(step.action.changes.vars().map(|x| Need::Set(v.var(x))));
            needs.extend(step.action.event.map(|e| Need::Perform(v.event(e))));
            needs.extend(step.checks.iter().map(observe));
        }
        if needs.iter().any(|n| !driver.supports(*n)) {
            let mut r = PlanResult::with(Status::Unsupported, plan);
            r.start.error = Some("The driver can't do all of this yet.".into());
            return r;
        }
        let mut r = PlanResult::with(Status::Failed, plan);
        r.start.error = Some(format!("The suite's {} s ran out before this test started.", self.limit.as_secs()));
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_runs_down_and_caps_a_test() {
        begin();
        let b = Budget::new(Duration::from_millis(50));
        assert!(!b.spent());
        assert!(b.cap(Duration::from_secs(40)) <= Duration::from_millis(50));
        std::thread::sleep(Duration::from_millis(60));
        assert!(b.spent());
        assert_eq!(b.cap(Duration::from_secs(40)), Duration::ZERO);
    }
}
