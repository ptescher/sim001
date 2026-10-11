//! Analysis of a suite as a graph of expectations:
//!
//! - [`precedence`]: the override DAG (exceptions and what they override),
//! - [`conflict`]: pairs of expectations that can't both hold, settled or not,
//! - [`flow`]: the DAG of what leads to what, from the Background,
//! - [`semantics`]: stepping a state through an action, shared with story planning.

pub mod conflict;
pub mod export;
pub mod flow;
pub mod precedence;
pub mod semantics;

pub use conflict::{Conflict, Kind as ConflictKind};
pub use flow::{Flow, Limits, Node, Route};
pub use precedence::Precedence;

use expectation_model::{Condition, Diagnostic, ExpId, FeatureId, SectionId, Suite};

pub struct Analysis {
    pub precedence: Precedence,
    pub conflicts: Vec<Conflict>,
    pub flow: Flow,
    /// Per section: the expectations that lead from its parent's start into its own.
    pub section_routes: Vec<Option<Route>>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Analysis {
    pub fn has_errors(&self) -> bool {
        expectation_model::diagnostic::has_errors(&self.diagnostics)
    }

    /// Conflicts an override settles: the suite's exceptions.
    pub fn exceptions(&self) -> impl Iterator<Item = &Conflict> {
        self.conflicts.iter().filter(|c| c.winner.is_some())
    }

    /// Features in dependency order: after the features their expectations follow on from
    /// (flow edges) or make exceptions to (overrides). Among those ready, the earliest in the
    /// flow first, then the most depended on, then declaration order. Cycles fall back to
    /// declaration order.
    pub fn feature_order(&self, suite: &Suite) -> Vec<FeatureId> {
        let n = suite.features.len();
        let feature = |e: ExpId| suite.exp(e).feature.0 as usize;
        let mut deps = vec![std::collections::BTreeSet::new(); n];
        for (p, c) in &self.flow.edges {
            if let Node::Exp(p) = p {
                deps[feature(*c)].insert(feature(*p));
            }
        }
        for (w, l) in &self.precedence.declared {
            deps[feature(*w)].insert(feature(*l));
        }
        for (f, d) in deps.iter_mut().enumerate() {
            d.remove(&f);
        }
        let dependents: Vec<usize> = (0..n).map(|f| deps.iter().filter(|d| d.contains(&f)).count()).collect();
        let earliest: Vec<u32> = suite
            .features
            .iter()
            .map(|f| f.expectations.iter().filter_map(|e| self.flow.level[e.0 as usize]).min().unwrap_or(u32::MAX))
            .collect();
        let key = |f: usize| (earliest[f], std::cmp::Reverse(dependents[f]), f);
        let mut placed = vec![false; n];
        let mut order = vec![];
        while order.len() < n {
            let ready = (0..n).filter(|&f| !placed[f] && deps[f].iter().all(|&d| placed[d])).min_by_key(|&f| key(f));
            let next = ready.unwrap_or_else(|| (0..n).find(|&f| !placed[f]).unwrap());
            placed[next] = true;
            order.push(FeatureId(next as u32));
        }
        order
    }
}

/// The states searches start from: the start of each section with features or stories in
/// it, and each scenario's arranged start. (A section that only groups others has nothing that
/// starts from it; searching from an empty root would mean searching every state there is.)
pub fn roots(suite: &Suite) -> Vec<(Condition, SectionId)> {
    let mut roots: Vec<(Condition, SectionId)> = vec![];
    let starts = suite
        .sections
        .iter()
        .filter(|s| !s.features.is_empty() || !s.stories.is_empty())
        .map(|s| (s.start.clone(), s.id));
    let scenarios = suite.stories.iter().flat_map(|story| {
        let start = &suite.section(story.section).start;
        story.scenarios.iter().map(move |sc| (start.overlay(&sc.given), story.section))
    });
    for r in starts.chain(scenarios) {
        if !roots.contains(&r) {
            roots.push(r);
        }
    }
    roots
}

/// For each section, how its setup is reached from its parent's start: the chain of
/// expectations that gets there. `None` for the root, for a section that adds no setup, and
/// for one nothing leads into.
pub fn section_routes(suite: &Suite, prec: &Precedence, limits: Limits) -> Vec<Option<Route>> {
    suite
        .sections
        .iter()
        .map(|s| {
            let parent = s.parent.map(|p| &suite.section(p).start)?;
            if s.setup.is_unconstrained() || parent.implies(&s.start) {
                return None;
            }
            flow::route_to(suite, prec, parent, &s.start, limits)
        })
        .collect()
}

pub fn analyze(suite: &Suite) -> Analysis {
    analyze_with(suite, Limits::default())
}

pub fn analyze_with(suite: &Suite, limits: Limits) -> Analysis {
    let (precedence, mut diagnostics) = precedence::build(suite);
    let conflicts = conflict::detect(suite, &precedence);
    diagnostics.extend(conflict::diagnose(suite, &precedence, &conflicts));
    let flow = flow::explore(suite, &precedence, &roots(suite), limits);
    if !flow.truncated {
        for e in suite.expectations.iter().filter(|e| flow.level[e.id.0 as usize].is_none()) {
            let needs = if e.is_invariant() {
                suite.describe(&e.given)
            } else {
                format!("{}, and then {}", suite.describe(&e.given), suite.describe_trigger(&e.trigger))
            };
            diagnostics.push(
                Diagnostic::warning("unreachable", format!("«{}» can never happen, starting from any section's setup", e.name))
                    .at(&e.span, format!("it needs {needs}"))
                    .help("no chain of expectations leads there; check the Background, or add the expectation that gets there"),
            );
        }
    }
    let section_routes = section_routes(suite, &precedence, limits);
    for (s, route) in suite.sections.iter().zip(&section_routes) {
        let Some(parent) = s.parent.map(|p| suite.section(p)) else { continue };
        // From a parent with no setup there's nothing to route from: getting there is starting
        // the program, which happens outside the suite.
        let from_somewhere = !parent.start.is_unconstrained();
        if from_somewhere && route.is_none() && !s.setup.is_unconstrained() && !parent.start.implies(&s.start) {
            let span = s.setup_steps.first().map(|t| &t.span).or(s.span.as_ref());
            let mut d =
                Diagnostic::warning("no-way-in", format!("nothing leads from «{}» into «{}»", parent.name, s.name))
                    .note_that(format!("«{}» starts with {}", s.name, suite.describe(&parent.start.unproven(&s.start))))
                    .help("add the expectation that gets there, so the docs can say how to get to this section");
            if let Some(span) = span {
                d = d.at(span, "");
            }
            diagnostics.push(d);
        }
    }
    Analysis { precedence, conflicts, flow, section_routes, diagnostics }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOCAB: &str = "
Vocabulary:
  State: the aircraft is in hand | flying | crashed
  State: the throttle is 0..100 %
  State: the calibration wizard is open | closed
  State: the help screen is shown | hidden
  Event: the pilot presses reset
  Event: the pilot presses help
  Event: the aircraft hits the ground

Background:
  Given the aircraft is in hand
  And the throttle is 0 %
  And the calibration wizard is closed
  And the help screen is hidden
";

    const LAUNCH: &str = "
Feature: Launch
  Expectation: Throttle up throws the plane
    Given the aircraft is in hand
    When the throttle is above 55 %
    Then the aircraft is flying

  Expectation: The wizard holds the plane
    Given the aircraft is in hand
    And the calibration wizard is open
    When the throttle is above 70 %
    Then the aircraft is in hand

  Expectation: A crash ends the flight
    Given the aircraft is flying
    When the aircraft hits the ground
    Then the aircraft is crashed

  Expectation: Reset picks the plane up
    When the pilot presses reset
    Then the aircraft is in hand
";

    fn suite(extra: &str) -> Suite {
        let src = format!("{VOCAB}{LAUNCH}{extra}");
        let l = expectation_language::load_sources("t", &[("t.expect", &src)]);
        assert!(!l.has_errors(), "{}", l.render_diagnostics());
        l.suite
    }

    fn named(s: &Suite, name: &str) -> ExpId {
        s.expectations.iter().find(|e| e.name == name).unwrap().id
    }

    #[test]
    fn finds_an_outcome_conflict_and_where_it_happens() {
        let s = suite("");
        let a = analyze(&s);
        let [c] = &a.conflicts[..] else { panic!("{:?}", a.conflicts) };
        assert_eq!((c.a, c.b), (named(&s, "Throttle up throws the plane"), named(&s, "The wizard holds the plane")));
        assert_eq!(c.winner, None);
        // The triggers overlap above 70 %, and both Givens hold with the wizard open.
        assert_eq!(s.describe_trigger(&c.trigger), "the throttle is above 70 %");
        assert!(s.describe(&c.situation).contains("the calibration wizard is open"));
        let d = a.diagnostics.iter().find(|d| d.code == "conflict").unwrap();
        assert!(d.help.as_ref().unwrap().contains("«The wizard holds the plane» is the more specific"), "{:?}", d.help);
    }

    #[test]
    fn an_override_settles_it() {
        let s = suite("");
        let src = format!("{VOCAB}{}", LAUNCH.replace("    Given the aircraft is in hand\n    And the calibration", "    Overrides: Throttle up throws the plane\n    Given the aircraft is in hand\n    And the calibration"));
        let l = expectation_language::load_sources("t", &[("t.expect", &src)]);
        let a = analyze(&l.suite);
        assert!(!a.has_errors(), "{}", expectation_model::diagnostic::render_all(&a.diagnostics, &l.suite.sources));
        assert_eq!(a.exceptions().count(), 1);
        assert_eq!(a.exceptions().next().unwrap().winner, Some(named(&s, "The wizard holds the plane")));
    }

    #[test]
    fn disjoint_givens_or_different_triggers_dont_conflict() {
        let s = suite(
            "
Feature: More
  Expectation: Help while flying
    Given the aircraft is flying
    When the pilot presses help
    Then the help screen is shown
  Expectation: Help is ignored in the hand
    Given the aircraft is in hand
    When the pilot presses help
    Then the help screen is hidden
",
        );
        let a = analyze(&s);
        assert_eq!(a.conflicts.len(), 1, "only the wizard conflict: {:?}", a.conflicts);
    }

    #[test]
    fn invariant_conflicts() {
        let s = suite(
            "
Feature: Help
  Expectation: Help toggles on
    Given the help screen is hidden
    When the pilot presses help
    Then the help screen is shown
  Expectation: No help in flight
    Given the aircraft is flying
    Then the help screen is hidden
",
        );
        let a = analyze(&s);
        let c = a.conflicts.iter().find(|c| c.kind == ConflictKind::Invariant).expect("help toggles while flying");
        assert_eq!((c.a, c.b), (named(&s, "Help toggles on"), named(&s, "No help in flight")));
    }

    #[test]
    fn override_cycles_are_errors() {
        let src = format!(
            "{VOCAB}
Feature: F
  Expectation: A
    Overrides: B
    When the pilot presses help
    Then the help screen is shown
  Expectation: B
    Overrides: A
    When the pilot presses help
    Then the help screen is hidden
"
        );
        let l = expectation_language::load_sources("t", &[("t.expect", &src)]);
        let a = analyze(&l.suite);
        assert!(a.diagnostics.iter().any(|d| d.code == "override-cycle"));
    }

    #[test]
    fn flow_levels_edges_and_routes() {
        let s = suite("");
        let a = analyze(&s);
        let throw = named(&s, "Throttle up throws the plane");
        let crash = named(&s, "A crash ends the flight");
        let reset = named(&s, "Reset picks the plane up");
        let wizard = named(&s, "The wizard holds the plane");
        let level = |e: ExpId| a.flow.level[e.0 as usize];
        assert_eq!(level(throw), Some(0));
        assert_eq!(level(reset), Some(0));
        assert_eq!(level(crash), Some(1));
        assert!(a.flow.edges.contains(&(Node::Start, throw)));
        assert!(a.flow.edges.contains(&(Node::Exp(throw), crash)));
        // Nothing opens the wizard, so its exception can't happen from the Background.
        assert_eq!(level(wizard), None);
        assert!(a.diagnostics.iter().any(|d| d.code == "unreachable"));
        // Levels grow along every edge: a DAG, even though reset loops back to the start.
        for (p, c) in &a.flow.edges {
            if let Node::Exp(p) = p {
                assert!(level(*p) < level(*c));
            }
        }
        let route = a.flow.routes[crash.0 as usize].as_ref().unwrap();
        assert_eq!(route.steps, vec![throw, crash]);
    }

    #[test]
    fn relative_thens_conflict_and_sections_have_a_way_in() {
        let root = "Vocabulary:\n State: the tool is none | PLAN\n State: the plan has 0..100 items\n Event: the operator presses PLAN\n Event: the operator clicks the map\nBackground:\n Given the tool is none\n And the plan has 0 items\nFeature: Tools\n Expectation: PLAN picks the planning tool\n  Given the tool is none\n  When the operator presses PLAN\n  Then the tool is PLAN\n";
        let planning = "Background:\n Given the tool is PLAN\nFeature: Waypoints\n Expectation: A click adds a waypoint\n  Given the tool is PLAN\n  When the operator clicks the map\n  Then the plan goes up by 1\n Expectation: A click adds two\n  Given the tool is PLAN\n  When the operator clicks the map\n  Then the plan goes up by 2\n";
        let l = expectation_language::load_sources("t", &[("root.expect", root), ("planning/w.expect", planning)]);
        assert!(!l.has_errors(), "{}", l.render_diagnostics());
        let a = analyze(&l.suite);
        let [c] = &a.conflicts[..] else { panic!("{:?}", a.conflicts) };
        assert_eq!((c.a, c.b), (ExpId(1), ExpId(2)));
        // From the root's start, pressing PLAN gets into the planning section.
        let route = a.section_routes[1].as_ref().expect("a way into planning");
        assert_eq!(route.steps, vec![ExpId(0)]);
        assert!(!a.diagnostics.iter().any(|d| d.code == "no-way-in"));
    }

    #[test]
    fn mermaid_mentions_everything() {
        let s = suite("");
        let a = analyze(&s);
        let m = export::mermaid(&s, &a, &|_| None);
        assert!(m.contains("start --> e0"));
        assert!(m.contains("<-. conflict .->"));
        assert!(m.contains("class e1 unreachable"));
        assert!(export::dot(&s, &a).contains("color=red"));
    }
}
