//! User stories to acceptance-test plans.
//!
//! A story says what the user does ("When the throttle is 80 %") and, optionally, what they
//! see ("Then the aircraft is flying"). Planning walks the story through the expectations:
//! each step works out which expectations fire, and their Thens become the checks to run.
//! So a story only has to tell the journey; the expectations supply what must be true at each
//! step. Planning also refuses stories that rely on behaviour nobody has specified, or that
//! contradict the expectations.

use expectation_graph::semantics::perform_among;
use expectation_graph::{Analysis, Precedence};
use expectation_model::text;
use expectation_model::{
    Condition, Constraint, Diagnostic, ExpId, Fact, Keyword, SectionId, Span, StepText, StoryId, Suite, Trigger, VarId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A scenario someone wrote.
    Story { story: StoryId, scenario: usize },
    /// Generated to exercise an expectation no story reaches.
    Coverage { target: ExpId },
}

/// Why a check is there.
#[derive(Clone, Debug, PartialEq)]
pub enum Because {
    /// The plan arranged it before the first step.
    Arranged,
    /// An expectation's Then (or an invariant) promises it.
    Expectation(ExpId),
    /// The story says so in a Then.
    Story(Span),
}

#[derive(Clone, Debug)]
pub struct Check {
    pub var: VarId,
    pub expect: Constraint,
    pub because: Because,
}

#[derive(Clone, Debug)]
pub struct PlanStep {
    pub action: Trigger,
    pub text: Vec<StepText>,
    /// The written step it belongs to: docs show a group as one step.
    pub group: usize,
    /// The group's lines as written, on its first step.
    pub written: Vec<StepText>,
    /// Transitions that fire.
    pub fired: Vec<ExpId>,
    /// Invariants in force afterwards.
    pub invariants: Vec<ExpId>,
    pub checks: Vec<Check>,
    /// The predicted state afterwards.
    pub state: Condition,
}

impl PlanStep {
    pub fn exercises(&self) -> impl Iterator<Item = ExpId> + '_ {
        self.fired.iter().chain(&self.invariants).copied()
    }
}

/// One acceptance test: arrange `start`, check it, then perform each step and check it.
#[derive(Clone, Debug)]
pub struct Plan {
    pub origin: Origin,
    /// The section it belongs to, whose setup it starts from.
    pub section: SectionId,
    pub title: String,
    pub slug: String,
    pub start: Condition,
    pub given: Vec<StepText>,
    pub start_checks: Vec<Check>,
    /// Invariants in force at the start.
    pub start_invariants: Vec<ExpId>,
    pub steps: Vec<PlanStep>,
}

pub struct Planned {
    pub plans: Vec<Plan>,
    pub diagnostics: Vec<Diagnostic>,
}

/// One input step: what to do, what the story checks afterwards, and how it was written.
struct Input<'a> {
    action: Trigger,
    checks: &'a [Fact],
    text: Vec<StepText>,
    span: Span,
    group: usize,
    written: Vec<StepText>,
}

/// A plan for every scenario of every story.
pub fn plan_stories(suite: &Suite, analysis: &Analysis) -> Planned {
    let mut plans = vec![];
    let mut diagnostics = vec![];
    for story in &suite.stories {
        for (i, sc) in story.scenarios.iter().enumerate() {
            let title = if story.scenarios.len() == 1 && sc.name == story.name {
                story.name.clone()
            } else {
                format!("{} › {}", story.name, sc.name)
            };
            let slug = if title == story.name { story.slug.clone() } else { format!("{}--{}", story.slug, sc.slug) };
            let section = suite.section(story.section);
            let slug = prefixed(&section.slug, &slug);
            let inputs = sc.steps.iter().map(|s| Input {
                action: s.action.clone(),
                checks: &s.checks,
                text: s.steps.clone(),
                span: s.span.clone(),
                group: s.group,
                written: s.written.clone(),
            });
            let start = section.start.overlay(&sc.given);
            let mut given: Vec<StepText> =
                suite.chain(section.id).iter().flat_map(|s| s.setup_steps.iter().cloned()).collect();
            given.extend(sc.given_steps.iter().cloned());
            let origin = Origin::Story { story: story.id, scenario: i };
            let head = Head { origin, section: section.id, title, slug, start, given };
            plans.push(build(suite, &analysis.precedence, head, inputs, &mut diagnostics));
        }
    }
    Planned { plans, diagnostics }
}

/// `slug` under a section: "gcs--planning--first-mission".
fn prefixed(section: &str, slug: &str) -> String {
    if section.is_empty() { slug.to_string() } else { format!("{section}--{slug}") }
}

/// What a plan is, before its steps are worked out.
struct Head {
    origin: Origin,
    section: SectionId,
    title: String,
    slug: String,
    start: Condition,
    given: Vec<StepText>,
}

fn build<'a>(
    suite: &Suite,
    prec: &Precedence,
    head: Head,
    inputs: impl Iterator<Item = Input<'a>>,
    diags: &mut Vec<Diagnostic>,
) -> Plan {
    let Head { origin, section, title, slug, start, given } = head;
    let v = &suite.vocabulary;
    let (mut state, start_invariants) = expectation_graph::semantics::settle(suite, prec, &start, &[]);
    let mut start_checks: Vec<Check> =
        start.iter().map(|(var, c)| Check { var, expect: *c, because: Because::Arranged }).collect();
    for &inv in &start_invariants {
        promise(&mut start_checks, suite, inv, &start);
    }

    // A top-level section is a system of its own: only its expectations apply to its stories.
    let scope = expectation_graph::flow::in_scope(suite, section);
    let mut steps = vec![];
    for input in inputs {
        let errors = diags.iter().filter(|d| d.is_error()).count();
        let candidates = suite.transitions().filter(|e| scope[e.id.0 as usize]);
        let out = perform_among(suite, prec, &state, &input.action, candidates);
        if let Some(&(a, b, var)) = out.clashes.first() {
            let (a, b) = (suite.exp(a), suite.exp(b));
            diags.push(
                Diagnostic::error(
                    "clash",
                    format!("«{}» and «{}» both apply here and disagree about {}", a.name, b.name, v.var(var).subject),
                )
                .at(&input.span, "")
                .help("settle the conflict between them first"),
            );
            break;
        }
        for &u in &out.uncertain {
            let e = suite.exp(u);
            let mut d = Diagnostic::error("unclear-step", format!("it isn't clear whether «{}» applies here", e.name));
            let given = expectation_graph::semantics::seen_by(&state, &input.action, &e.trigger).unproven(&e.given);
            if !given.is_unconstrained() {
                d = d.at(&input.span, format!("nothing establishes that {}", v.describe(&given)));
            }
            let trig = input.action.changes.unproven(&e.trigger.changes);
            if !trig.is_unconstrained() {
                d = d.at(&input.span, format!("the step doesn't say whether {}", v.describe(&trig)));
            }
            diags.push(
                d.at(&e.span, "").help("pin it down: arrange it with a `Given`, or give the When an exact value"),
            );
        }
        if let (true, true, Some(e)) = (out.fired.is_empty(), out.uncertain.is_empty(), input.action.event) {
            let event = v.event(e);
            let line = input.text.iter().find(|t| text::key(&t.text) == event.key).map_or(&input.span, |t| &t.span);
            diags.push(
                Diagnostic::error("unspecified", format!("nothing is expected to happen when {}", event.text))
                    .at(line, format!("at this point {}", v.describe(&state)))
                    .help("no expectation covers this; write one, or the story relies on unspecified behaviour"),
            );
        }

        let mut checks = vec![];
        let before = state.overlay(&input.action.changes);
        for &f in out.fired.iter().chain(&out.invariants) {
            promise(&mut checks, suite, f, &before);
        }
        let mut after = out.after;
        for fact in input.checks {
            let mut want = Condition::new();
            want.constrain(v.var(fact.var), fact.constraint);
            let stated = v.var(fact.var).describe(&fact.constraint);
            if !after.overlaps(&want) {
                let said = v.describe(&after.restrict([fact.var]));
                let mut d = Diagnostic::error(
                    "contradicts",
                    format!("the story expects {stated}, but the expectations say {said}"),
                )
                .at(&fact.span, "");
                if let Some(&by) = out.fired.iter().find(|&&f| suite.exp(f).then.get(fact.var).is_some()) {
                    d = d.at(&suite.exp(by).span, format!("«{}» says so", suite.exp(by).name));
                }
                diags.push(d);
                continue;
            }
            if !after.implies(&want) {
                diags.push(
                    Diagnostic::warning("unpromised", format!("no expectation promises that {stated}"))
                        .at(&fact.span, "")
                        .help("it's checked anyway; write an expectation so it's specified"),
                );
            }
            after = after.and(&want);
            if !checks.iter().any(|c: &Check| c.var == fact.var && c.expect == fact.constraint) {
                checks.push(Check {
                    var: fact.var,
                    expect: fact.constraint,
                    because: Because::Story(fact.span.clone()),
                });
            }
        }
        steps.push(PlanStep {
            action: input.action,
            text: input.text,
            group: input.group,
            written: input.written,
            fired: out.fired,
            invariants: out.invariants,
            checks,
            state: after.clone(),
        });
        state = after;
        // Stop at a scenario's first error: later steps would be judged from a state that's wrong.
        if diags.iter().filter(|d| d.is_error()).count() > errors {
            break;
        }
    }
    Plan { origin, section, title, slug, start, given, start_checks, start_invariants, steps }
}

/// Add an expectation's Then to `checks`, skipping what's already checked.
fn promise(checks: &mut Vec<Check>, suite: &Suite, e: ExpId, before: &Condition) {
    for (var, c) in suite.exp(e).effect(&suite.vocabulary, before).iter() {
        if !checks.iter().any(|k| k.var == var && k.expect == *c) {
            checks.push(Check { var, expect: *c, because: Because::Expectation(e) });
        }
    }
}

/// For each expectation, the (plan, step) pairs that exercise it; step `None` is the start.
pub fn coverage(suite: &Suite, plans: &[Plan]) -> Vec<Vec<(usize, Option<usize>)>> {
    let mut out = vec![vec![]; suite.expectations.len()];
    for (p, plan) in plans.iter().enumerate() {
        for &inv in &plan.start_invariants {
            out[inv.0 as usize].push((p, None));
        }
        for (s, step) in plan.steps.iter().enumerate() {
            for e in step.exercises() {
                out[e.0 as usize].push((p, Some(s)));
            }
        }
    }
    out
}

/// Warnings for expectations no plan exercises, each with a route that would.
pub fn untested(suite: &Suite, analysis: &Analysis, plans: &[Plan]) -> Vec<Diagnostic> {
    let cov = coverage(suite, plans);
    let mut diags = vec![];
    for e in &suite.expectations {
        if !cov[e.id.0 as usize].is_empty() {
            continue;
        }
        let Some(route) = &analysis.flow.routes[e.id.0 as usize] else { continue };
        let mut lines: Vec<String> = route
            .start
            .iter()
            .filter(|(v, c)| suite.background.get(*v) != Some(c))
            .map(|(v, c)| format!("Given {}", suite.vocabulary.var(v).describe(c)))
            .collect();
        lines.extend(route.steps.iter().map(|s| format!("When {}", suite.describe_trigger(&suite.exp(*s).trigger))));
        diags.push(Diagnostic::warning("untested", format!("no story exercises «{}»", e.name)).at(&e.span, "").help(
            if lines.is_empty() {
                "it holds from the start; any story checks it".into()
            } else {
                format!("a story that would: {}", lines.join(" / "))
            },
        ));
    }
    diags
}

/// Generated plans for expectations no plan in `plans` exercises, following each one's
/// shortest route from the flow graph. Routes that can't be made into a clean plan are skipped.
pub fn coverage_plans(suite: &Suite, analysis: &Analysis, plans: &[Plan]) -> Vec<Plan> {
    let cov = coverage(suite, plans);
    let mut covered: Vec<bool> = cov.iter().map(|c| !c.is_empty()).collect();
    let mut out = vec![];
    // Routes from each section's own start, so a generated test starts where the section does.
    let mut flows: Vec<Option<expectation_graph::Flow>> = suite.sections.iter().map(|_| None).collect();
    for e in &suite.expectations {
        if covered[e.id.0 as usize] {
            continue;
        }
        let section = suite.section_of(e.id);
        let flow = flows[section.id.0 as usize].get_or_insert_with(|| {
            let roots = [(section.start.clone(), section.id)];
            expectation_graph::flow::explore(suite, &analysis.precedence, &roots, expectation_graph::Limits::default())
        });
        let Some(route) = flow.routes[e.id.0 as usize].as_ref().or(analysis.flow.routes[e.id.0 as usize].as_ref())
        else {
            continue;
        };
        if route.steps.is_empty() {
            continue;
        }
        let inputs = route.steps.iter().enumerate().map(|(n, &s)| {
            let x = suite.exp(s);
            let text: Vec<StepText> = x
                .when_steps()
                .cloned()
                .enumerate()
                .map(|(i, mut t)| {
                    t.keyword = if i == 0 { Keyword::When } else { Keyword::And };
                    t
                })
                .collect();
            Input {
                action: x.trigger.clone(),
                checks: &[],
                written: text.clone(),
                text,
                span: x.span.clone(),
                group: n + 1,
            }
        });
        let mut given: Vec<StepText> =
            suite.chain(section.id).iter().flat_map(|s| s.setup_steps.iter().cloned()).collect();
        for (var, c) in route.start.iter().filter(|(v, c)| section.start.get(*v) != Some(c)) {
            let span = e.span.clone();
            let (text, meaning) = (suite.vocabulary.var(var).describe(c), Some(expectation_model::Said::Fact(var, *c)));
            given.push(StepText { keyword: Keyword::Given, text, span, meaning });
        }
        let mut diags = vec![];
        let title = format!("Reaching «{}»", e.name);
        let head = Head {
            origin: Origin::Coverage { target: e.id },
            section: section.id,
            title,
            slug: prefixed(&section.slug, &format!("coverage--{}", text::slug(&e.name))),
            start: route.start.clone(),
            given,
        };
        let plan = build(suite, &analysis.precedence, head, inputs, &mut diags);
        if diags.iter().any(|d| d.is_error()) {
            continue;
        }
        for step in &plan.steps {
            for x in step.exercises() {
                covered[x.0 as usize] = true;
            }
        }
        out.push(plan);
    }
    out
}
