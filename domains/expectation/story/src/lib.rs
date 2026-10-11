//! User stories as acceptance tests: [`plan`] compiles stories (and generated coverage
//! routes) into test plans, and [`report`] holds what happened when they ran.

pub mod plan;
pub mod report;

pub use plan::{Because, Check, Origin, Plan, PlanStep, Planned, coverage, coverage_plans, plan_stories, untested};
pub use report::{CheckResult, PlanResult, Report, Status, StepResult, Verdict, load_results};

#[cfg(test)]
mod tests {
    use super::*;
    use expectation_model::Suite;

    const SUITE: &str = "
Vocabulary:
  State: the aircraft is in hand | flying | crashed
  State: the throttle is 0..100 %
  State: the help screen is shown | hidden
  Event: the pilot presses reset
  Event: the pilot presses help
  Event: the aircraft hits the ground

Background:
  Given the aircraft is in hand
  And the throttle is 0 %

Feature: Launch
  Expectation: Throttle up throws the plane
    Given the aircraft is in hand
    When the throttle is above 55 %
    Then the aircraft is flying

  Expectation: A crash ends the flight
    Given the aircraft is flying
    When the aircraft hits the ground
    Then the aircraft is crashed

  Expectation: Reset picks the plane up
    When the pilot presses reset
    Then the aircraft is in hand

  Expectation: Help opens the help screen
    Given the help screen is hidden
    When the pilot presses help
    Then the help screen is shown
";

    fn planned(stories: &str) -> (Suite, expectation_graph::Analysis, Planned) {
        let src = format!("{SUITE}{stories}");
        let l = expectation_language::load_sources("t", &[("t.expect", &src)]);
        assert!(!l.has_errors(), "{}", l.render_diagnostics());
        let a = expectation_graph::analyze(&l.suite);
        let p = plan_stories(&l.suite, &a);
        (l.suite, a, p)
    }

    fn codes(p: &Planned) -> Vec<&'static str> {
        p.diagnostics.iter().map(|d| d.code).collect()
    }

    #[test]
    fn checks_come_from_the_expectations() {
        let (s, _, p) = planned(
            "
Story: Throw and crash
  When the throttle is 80 %
  When the aircraft hits the ground
  Then the aircraft is crashed
",
        );
        assert!(codes(&p).is_empty(), "{:?}", codes(&p));
        let plan = &p.plans[0];
        assert_eq!(plan.start_checks.len(), 2);
        // The story never says the plane flies; the expectation does, so the test checks it.
        let first = &plan.steps[0];
        assert_eq!(first.fired.len(), 1);
        assert_eq!(s.vocabulary.describe(&first.state), "the aircraft is flying and the throttle is 80 %");
        assert!(matches!(first.checks[0].because, Because::Expectation(_)));
        assert_eq!(plan.steps[1].checks.len(), 1, "the story's Then duplicates the promise");
    }

    #[test]
    fn stories_cant_rely_on_unspecified_or_contradicted_behaviour() {
        let (_, _, p) = planned("Story: S\n  When the aircraft hits the ground\n");
        assert_eq!(codes(&p), vec!["unspecified"]);

        let (_, _, p) = planned("Story: S\n  When the throttle is 80 %\n  Then the aircraft is in hand\n");
        assert_eq!(codes(&p), vec!["contradicts"]);

        // 50..100 % might or might not be above 55 %.
        let (_, _, p) = planned("Story: S\n  When the throttle is at least 50 %\n");
        assert_eq!(codes(&p), vec!["unclear-step"]);

        // The Background doesn't say whether help is showing.
        let (_, _, p) = planned("Story: S\n  When the pilot presses help\n");
        assert_eq!(codes(&p), vec!["unclear-step"]);
        let (_, _, p) = planned(
            "Story: S\n  Given the help screen is hidden\n  When the pilot presses help\n  Then the throttle is 0 %\n",
        );
        assert!(codes(&p).is_empty());

        let (_, _, p) = planned("Story: S\n  When the throttle is 80 %\n  Then the help screen is hidden\n");
        assert_eq!(codes(&p), vec!["unpromised"]);
    }

    #[test]
    fn coverage_fills_the_gaps() {
        let (s, a, p) = planned("Story: S\n  When the throttle is 80 %\n");
        let cov = coverage(&s, &p.plans);
        assert_eq!(cov.iter().filter(|c| c.is_empty()).count(), 3);
        assert_eq!(untested(&s, &a, &p.plans).len(), 3);
        let generated = coverage_plans(&s, &a, &p.plans);
        let titles: Vec<&str> = generated.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(
            titles,
            vec![
                "Reaching «A crash ends the flight»",
                "Reaching «Reset picks the plane up»",
                "Reaching «Help opens the help screen»"
            ]
        );
        // To reach the crash, the generated test throws the plane first.
        assert_eq!(generated[0].steps.len(), 2);
        // Help needs the help screen hidden, which the Background leaves open: it's arranged.
        assert_eq!(generated[2].given.last().unwrap().text, "the help screen is hidden");
    }
}
