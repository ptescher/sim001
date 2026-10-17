//! The `.expect` file format: a Gherkin-flavoured language for a suite's vocabulary,
//! expectations and user stories.
//!
//! ```text
//! Suite: SIM001
//!
//! Vocabulary:
//!   State: the aircraft is in hand | flying | crashed
//!   State: the throttle is 0..100 %
//!   Event: the pilot presses reset
//!
//! Background:
//!   Given the aircraft is in hand
//!
//! Feature: Hand launch
//!   Expectation: Throttle up throws the plane
//!     Given the aircraft is in hand
//!     When the throttle is above 55 %
//!     Then the aircraft is flying
//!
//! Story: First flight
//!   As a new pilot
//!   I want to throw the plane by opening the throttle
//!
//!   Scenario: Throw it
//!     When the throttle is 80 %
//!     Then the aircraft is flying
//! ```
//!
//! [`load_dir`] reads every `*.expect` file under a directory into one suite;
//! [`load_sources`] does the same from strings.

mod parse;
mod resolve;
pub mod step;

use expectation_model::{Diagnostic, Suite};
use std::path::{Path, PathBuf};

pub const EXTENSION: &str = "expect";

/// A suite and everything wrong with it. Check [`Loaded::has_errors`] before using the suite:
/// steps that didn't resolve are left out of it.
pub struct Loaded {
    pub suite: Suite,
    pub diagnostics: Vec<Diagnostic>,
}

impl Loaded {
    pub fn has_errors(&self) -> bool {
        expectation_model::diagnostic::has_errors(&self.diagnostics)
    }

    pub fn render_diagnostics(&self) -> String {
        expectation_model::diagnostic::render_all(&self.diagnostics, &self.suite.sources)
    }
}

/// Parse named sources, in order, as one suite.
pub fn load_sources(name: &str, sources: &[(&str, &str)]) -> Loaded {
    let mut diags = vec![];
    let mut suite = Suite { name: name.to_string(), ..Suite::default() };
    let mut files = vec![];
    for (file, text) in sources {
        let mut raw = parse::parse(file, text, &mut diags);
        raw.dir = file.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
        files.push(raw);
        suite.sources.insert((*file).into(), text.to_string());
    }
    resolve::resolve(&mut suite, &files, &mut diags);
    Loaded { suite, diagnostics: diags }
}

/// Every `*.expect` file under `dir` (recursively, in path order) as one suite, named by its
/// `Suite:` line or else after the directory. File names in diagnostics are relative to `dir`.
pub fn load_dir(dir: &Path) -> std::io::Result<Loaded> {
    let mut paths = vec![];
    collect(dir, &mut paths)?;
    paths.sort();
    let mut texts = vec![];
    for p in &paths {
        let rel = p.strip_prefix(dir).unwrap_or(p).to_string_lossy().replace('\\', "/");
        texts.push((rel, std::fs::read_to_string(p)?));
    }
    let refs: Vec<(&str, &str)> = texts.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let mut loaded = load_sources(&suite_name(dir), &refs);
    if paths.is_empty() {
        loaded
            .diagnostics
            .push(Diagnostic::error("empty-suite", format!("no .{EXTENSION} files in {}", dir.display())));
    }
    Ok(loaded)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        if p.is_dir() {
            collect(&p, out)?;
        } else if p.extension().is_some_and(|e| e == EXTENSION) {
            out.push(p);
        }
    }
    Ok(())
}

/// "hand_launch" -> "Hand launch".
fn suite_name(dir: &Path) -> String {
    let base = dir.canonicalize().ok().and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()));
    let base = base.unwrap_or_else(|| "Expectations".into()).replace(['_', '-'], " ");
    let mut c = base.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use expectation_model::{Constraint, ExpId, Keyword};

    const SUITE: &str = "
Suite: Hand launch basics

Vocabulary:
  State: the aircraft is in hand | flying | crashed
  State: the throttle is 0..100 %
  State: the calibration wizard is open | closed
  Event: the pilot presses reset

Background:
  Given the aircraft is in hand
  And the throttle is 0 %

Feature: Hand launch
  The plane starts in your hand.

  Expectation: Throttle up throws the plane
    Given the aircraft is in hand
    When the throttle is above 55 %
    Then the aircraft is flying

  Expectation: The wizard holds the plane
    Overrides: Throttle up throws the plane
    Given the aircraft is in hand
    And the calibration wizard is open
    When the throttle is above 55 %
    Then the aircraft is in hand

  Expectation: Never crash while held
    Given the aircraft is in hand
    Then the aircraft is not crashed

Story: First flight
  As a new pilot
  I want to throw the plane

  Given the throttle is 0 %
  When the throttle is 80 %
  Then the aircraft is flying
  When the throttle is 0 %
  And the pilot presses reset
  Then the aircraft is in hand
";

    #[test]
    fn parses_a_suite() {
        let l = load_sources("t", &[("t.expect", SUITE)]);
        assert!(!l.has_errors(), "{}", l.render_diagnostics());
        let s = &l.suite;
        assert_eq!(s.name, "Hand launch basics");
        assert_eq!(s.vocabulary.variables.len(), 3);
        assert_eq!(s.background.vars().count(), 2);
        assert_eq!(s.features[0].description, vec!["The plane starts in your hand."]);
        let [throw, wizard, never] = &s.expectations[..] else { panic!() };
        assert!(throw.trigger.event.is_none() && throw.trigger.changes.vars().count() == 1);
        assert_eq!(wizard.overrides, vec![throw.id]);
        assert!(never.is_invariant());
        assert_eq!(never.then.get(expectation_model::VarId(0)), Some(&Constraint::OneOf(0b011)));

        let story = &s.stories[0];
        assert_eq!(story.narrative.len(), 2);
        let sc = &story.scenarios[0];
        assert_eq!(sc.name, "First flight");
        assert_eq!(sc.steps.len(), 2);
        assert!(sc.steps[1].action.event.is_some() && sc.steps[1].action.changes.vars().count() == 1);
        assert_eq!(sc.steps[1].steps[1].keyword, Keyword::And);
    }

    #[test]
    fn sections_nest_by_directory_and_layer_their_setup() {
        let vocab = "Suite: Station\nVocabulary:\n State: the tool is none | PLAN\n State: the plan has 0..100 items\n Event: the operator presses PLAN\n Event: the operator clicks the map\nBackground:\n Given the tool is none\n And the plan has 0 items\n";
        let tools = "Feature: Tools\n Expectation: PLAN picks the planning tool\n  Given the tool is none\n  When the operator presses PLAN\n  Then the tool is PLAN\n";
        let planning = "Section: Planning a mission\n Everything here starts on the planning tool.\nBackground:\n Given the tool is PLAN\nFeature: Waypoints\n Expectation: Clicking adds a waypoint\n  Given the tool is PLAN\n  When the operator clicks the map\n  Then the plan goes up by 1\n";
        let l = load_sources(
            "x",
            &[("root.expect", vocab), ("tools.expect", tools), ("03-planning/waypoints.expect", planning)],
        );
        assert!(!l.has_errors(), "{}", l.render_diagnostics());
        let s = &l.suite;
        assert_eq!(s.sections.len(), 2);
        let (root, planning) = (&s.sections[0], &s.sections[1]);
        assert_eq!((root.name.as_str(), planning.name.as_str()), ("Station", "Planning a mission"));
        assert_eq!(planning.slug, "planning");
        assert_eq!(planning.parent, Some(root.id));
        assert_eq!(planning.description, vec!["Everything here starts on the planning tool."]);
        // The planning section starts from the root's start with its own setup on top.
        assert_eq!(s.describe(&planning.start), "the tool is PLAN and the plan has 0 items");
        assert_eq!(s.feature(s.exp(ExpId(1)).feature).section, planning.id);
        assert_eq!(s.exp(ExpId(1)).shifts, vec![(expectation_model::VarId(1), expectation_model::Shift::By(1.0))]);
    }

    #[test]
    fn phrases_say_more_with_less() {
        let src = "Vocabulary:
 State: the tool is pan | PLAN
   Set: pick the {value} tool
 State: the plan has 0..100 items
 Event: the operator presses PLAN
   Say: press PLAN
 Event: the operator clicks the map
 Phrase: the operator picks the planning tool
   Say: press PLAN
   When the operator presses PLAN
 Phrase: the operator plans two waypoints
   Say: click the map twice
   When the operator clicks the map
   When the operator clicks the map
 Phrase: planning has started
   Given the tool is PLAN
   And the plan has 0 items
Feature: F
 Expectation: A click adds a waypoint
  Given planning has started
  When the operator clicks the map
  Then the plan has 1 item
Story: S
 As a pilot
 I want to plan
 Planning starts on the map.
 When the operator picks the planning tool
 When the operator plans two waypoints
 Then the plan has 2 items
";
        let l = load_sources("t", &[("t.expect", src)]);
        assert!(!l.has_errors(), "{}", l.render_diagnostics());
        let s = &l.suite;
        assert_eq!(s.vocabulary.events[0].say.as_deref(), Some("press PLAN"));
        assert_eq!(s.vocabulary.variables[0].set.as_deref(), Some("pick the {value} tool"));
        assert_eq!(s.vocabulary.phrases.len(), 3);
        // A setup phrase in a Given is its facts.
        assert_eq!(s.exp(ExpId(0)).given.vars().count(), 2);
        let story = &s.stories[0];
        assert_eq!(story.narrative.len(), 2);
        assert_eq!(story.description, vec!["Planning starts on the map."]);
        // Two written steps make three actions: the two-click phrase is one group of two.
        let steps = &story.scenarios[0].steps;
        assert_eq!(steps.iter().map(|st| st.group).collect::<Vec<_>>(), vec![1, 2, 2]);
        assert_eq!(steps[1].written.len(), 2, "the When and the Then, as written");
        assert!(steps[2].written.is_empty());
        assert_eq!(steps[2].checks.len(), 1, "the Then checks after the last click");
        assert!(matches!(steps[1].written[0].meaning, Some(expectation_model::Said::Phrase(_))));
    }

    #[test]
    fn docs_lines_say_what_the_reader_sees() {
        let src = "Vocabulary:
 State: the aircraft is in hand | crashed
   Shows: crashed: the HUD says CRASHED
 State: the plan has 0..650 items
   Shows: the UPLOAD button reads UPLOAD {value}
 State: the battery is 0..100 %
   Shows: at least 99 %: the battery is full
 Event: the pilot throws the plane
Term: HUD
  The text on the view.
Feature: F
  When the plane is in hand, Space throws it.
  Expectation: E
    Docs: hidden
    Given the aircraft is in hand
    When the pilot throws the plane
    Then the aircraft is crashed
";
        let l = load_sources("t", &[("t.expect", src)]);
        assert!(!l.has_errors(), "{}", l.render_diagnostics());
        let s = &l.suite;
        let v = &s.vocabulary;
        let shown = |step: &str| match step::resolve(step, &expectation_model::Span::new("t", 1), v) {
            Ok(step::Meaning::Fact(f)) => v.var(f.var).shown(&f.constraint),
            _ => panic!("{step} isn't a state"),
        };
        assert_eq!(shown("the aircraft is crashed").as_deref(), Some("the HUD says CRASHED"));
        assert_eq!(shown("the aircraft is in hand"), None);
        assert_eq!(shown("the plan has 3 items").as_deref(), Some("the UPLOAD button reads UPLOAD 3"));
        assert_eq!(shown("the battery is at least 99 %").as_deref(), Some("the battery is full"));
        assert_eq!(s.glossary[0].name, "HUD");
        // A "When" line in a feature's description is prose: features hold no steps.
        assert_eq!(s.features[0].description, vec!["When the plane is in hand, Space throws it."]);
        assert!(s.expectations[0].hidden);
    }

    fn errors(src: &str) -> Vec<&'static str> {
        let l = load_sources("t", &[("t.expect", src)]);
        l.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.code).collect()
    }

    #[test]
    fn reports_mistakes() {
        let vocab = "Vocabulary:\n State: the aircraft is in hand | flying\n Event: the pilot presses reset\n";
        let with = |rest: &str| format!("{vocab}{rest}");
        assert_eq!(errors(&with("Feature: F\n Expectation: E\n  Given the aircraft is flying\n")), vec!["no-then"]);
        assert_eq!(errors(&with("Feature: F\n Expectation: E\n  Then the aircraft is landed\n")), vec!["bad-value"]);
        assert_eq!(errors(&with("Feature: F\n Expectation: E\n  Then the plane is flying\n")), vec!["unknown-step"]);
        assert_eq!(
            errors(&with(
                "Feature: F\n Expectation: E\n  Given the aircraft is flying\n  And the aircraft is in hand\n  Then the aircraft is flying\n"
            )),
            vec!["contradiction"]
        );
        assert_eq!(
            errors(&with("Feature: F\n Expectation: E\n  Then the pilot presses reset\n")),
            vec!["misplaced-event"]
        );
        assert_eq!(
            errors(&with("Feature: F\n Expectation: E\n  Overrides: Nope\n  Then the aircraft is flying\n")),
            vec!["bad-override"]
        );
        assert_eq!(errors(&with("Expectation: E\n")), vec!["syntax"]);
        assert_eq!(errors(&with("Story: S\n Then the aircraft is flying\n")), vec!["syntax", "empty-story"]);
        let vacuous = load_sources(
            "t",
            &[(
                "t.expect",
                &with(
                    "Feature: F\n Expectation: E\n  When the aircraft is in hand or flying\n  Then the aircraft is flying\n",
                ),
            )],
        );
        assert_eq!(vacuous.diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(), vec!["says-nothing"]);
        let typo =
            load_sources("t", &[("t.expect", &with("Feature: F\n Expectation: E\n  Gvien the aircraft is flying\n"))]);
        let codes: Vec<_> = typo.diagnostics.iter().map(|d| d.code).collect();
        assert_eq!(codes, vec!["maybe-step", "no-then"]);
    }
}
