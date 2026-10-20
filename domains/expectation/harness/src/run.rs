//! Running plans against a driver.

use crate::driver::{Driver, Need, Progress};
use crate::service::Services;
use expectation_model::{Suite, Trigger};
use expectation_story::{Check, CheckResult, Plan, PlanResult, Report, Status, StepResult};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct RunOptions {
    /// Screenshots go to `<docs_dir>/screenshots/<plan>/<step>.<ext>`; `None` takes none.
    pub docs_dir: Option<PathBuf>,
    /// How many times to [`Driver::advance`] while waiting for a step's checks to hold.
    pub patience: u32,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions { docs_dir: None, patience: 100 }
    }
}

pub fn run(suite: &Suite, plans: &[Plan], driver: &mut dyn Driver, opts: &RunOptions) -> Report {
    Report { results: plans.iter().map(|p| run_plan(suite, p, driver, opts)).collect() }
}

/// As [`run`], with what the driver needs from outside started before each plan: a service that
/// is up stays, one that died is started again, and when one cannot be had the plan is not
/// supported yet, with what to do about it.
pub fn run_with(suite: &Suite, plans: &[Plan], driver: &mut dyn Driver, opts: &RunOptions, services: &mut Services) -> Report {
    let needs = driver.needs();
    let names: Vec<&str> = needs.iter().map(String::as_str).collect();
    let fresh = driver.fresh();
    let fresh: Vec<&str> = fresh.iter().map(String::as_str).collect();
    let results = plans
        .iter()
        .map(|plan| match unsupported(suite, plan, driver).map_or_else(|| services.restart(&fresh).and_then(|()| services.start(&names)), |_| Ok(())) {
            Ok(()) => run_plan(suite, plan, driver, opts),
            Err(why) => {
                let mut r = PlanResult::with(Status::Unsupported, plan);
                r.start.error = Some(format!("It needs {why}."));
                r
            }
        })
        .collect();
    Report { results }
}

fn needs<'a>(suite: &'a Suite, plan: &Plan) -> Vec<Need<'a>> {
    let v = &suite.vocabulary;
    let mut out: Vec<Need> = plan.start.vars().map(|x| Need::Arrange(v.var(x))).collect();
    let observe = |c: &Check| Need::Observe(v.var(c.var));
    out.extend(plan.start_checks.iter().map(observe));
    for step in &plan.steps {
        out.extend(step.action.changes.vars().map(|x| Need::Set(v.var(x))));
        out.extend(step.action.event.map(|e| Need::Perform(v.event(e))));
        out.extend(step.checks.iter().map(observe));
    }
    out
}

/// A plan the driver cannot run yet is not run and not failed: the result says what it lacks.
fn unsupported(suite: &Suite, plan: &Plan, driver: &dyn Driver) -> Option<PlanResult> {
    let mut missing: Vec<String> =
        needs(suite, plan).into_iter().filter(|n| !driver.supports(*n)).map(|n| n.to_string()).collect();
    for (x, c) in plan.start.iter() {
        let var = suite.vocabulary.var(x);
        let value = var.witness(c).expect("plans only arrange satisfiable states");
        if !driver.can_arrange(var, &value) {
            missing.push(format!("arrange {} as {value}", var.subject));
        }
    }
    missing.sort();
    missing.dedup();
    if !missing.is_empty() {
        // Not a failure: the system (or its driver) just doesn't do this yet.
        let mut r = PlanResult::with(Status::Unsupported, plan);
        // A few of them, so the reason stays readable: another system's tests lack dozens.
        let mut shown: Vec<String> = missing.iter().take(3).cloned().collect();
        if missing.len() > 3 {
            shown.push(format!("{} more things", missing.len() - 3));
        }
        r.start.error = Some(format!("The driver can't {} yet.", expectation_model::text::join_and(&shown)));
        return Some(r);
    }
    None
}

fn run_plan(suite: &Suite, plan: &Plan, driver: &mut dyn Driver, opts: &RunOptions) -> PlanResult {
    let failed = |error: String| StepResult {
        status: Status::Failed,
        checks: vec![],
        screenshot: None,
        error: Some(error),
        polls: 0,
    };
    let skip_all = |start: StepResult| PlanResult {
        status: Status::Failed,
        start,
        steps: plan.steps.iter().map(|_| StepResult::skipped()).collect(),
    };

    if let Some(r) = unsupported(suite, plan, driver) {
        return r;
    }

    let shots = opts.docs_dir.as_ref().map(|d| {
        let dir = d.join("screenshots").join(&plan.slug);
        let _ = std::fs::remove_dir_all(&dir);
        dir
    });
    let shoot = |driver: &mut dyn Driver, n: usize| -> Option<String> {
        let dir = shots.as_ref()?;
        let shot = driver.screenshot().ok()??;
        let name = format!("{n:02}.{}", shot.extension);
        std::fs::create_dir_all(dir).ok()?;
        std::fs::write(dir.join(&name), &shot.bytes).ok()?;
        Some(format!("screenshots/{}/{name}", plan.slug))
    };

    if let Err(e) = driver.reset() {
        return skip_all(failed(format!("couldn't reset: {e}")));
    }
    for (x, c) in plan.start.iter() {
        let var = suite.vocabulary.var(x);
        let value = var.witness(c).expect("plans only arrange satisfiable states");
        if let Err(e) = driver.arrange(var, &value) {
            return skip_all(failed(format!("couldn't arrange {}: {e}", var.describe(c))));
        }
    }
    driver.settle();
    let mut start = observe(suite, driver, &plan.start_checks, opts.patience);
    start.screenshot = shoot(driver, 0);

    let mut ok = start.status == Status::Passed;
    let mut steps = vec![];
    for (n, step) in plan.steps.iter().enumerate() {
        if !ok {
            steps.push(StepResult::skipped());
            continue;
        }
        let mut r = match act(suite, driver, &step.action) {
            Ok(()) => {
                driver.settle();
                observe(suite, driver, &step.checks, opts.patience)
            }
            Err(e) => failed(e),
        };
        r.screenshot = shoot(driver, n + 1);
        ok = r.status == Status::Passed;
        steps.push(r);
    }
    PlanResult { status: if ok { Status::Passed } else { Status::Failed }, start, steps }
}

fn act(suite: &Suite, driver: &mut dyn Driver, action: &Trigger) -> Result<(), String> {
    for (x, c) in action.changes.iter() {
        let var = suite.vocabulary.var(x);
        let value = var.witness(c).ok_or_else(|| format!("no value satisfies {}", var.describe(c)))?;
        driver.set(var, &value).map_err(|e| format!("couldn't set {}: {e}", var.describe(c)))?;
    }
    if let Some(e) = action.event {
        let event = suite.vocabulary.event(e);
        driver.perform(event).map_err(|err| format!("couldn't make it happen that {}: {err}", event.text))?;
    }
    Ok(())
}

/// Observe until every check holds at once, or patience runs out.
fn observe(suite: &Suite, driver: &mut dyn Driver, checks: &[Check], patience: u32) -> StepResult {
    let mut polls = 0;
    loop {
        polls += 1;
        let results: Vec<CheckResult> = checks
            .iter()
            .map(|c| {
                let var = suite.vocabulary.var(c.var);
                match driver.observe(var) {
                    Ok(v) => CheckResult { passed: var.admits(&c.expect, &v), observed: Some(v), error: None },
                    Err(e) => CheckResult { observed: None, passed: false, error: Some(e) },
                }
            })
            .collect();
        let passed = results.iter().all(|r| r.passed);
        if passed || polls > patience || driver.advance() == Progress::Static {
            let status = if passed { Status::Passed } else { Status::Failed };
            return StepResult { status, checks: results, screenshot: None, error: None, polls };
        }
    }
}
