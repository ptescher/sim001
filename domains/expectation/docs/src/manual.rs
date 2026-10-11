//! What the manual says, worked out once for the HTML and the Markdown alike: each section's
//! "Before you begin", each tutorial's numbered steps, and the notes on exceptions. The
//! renderers only lay these out.

use crate::{Docs, Facts, voice};
use expectation_graph::ConflictKind;
use expectation_model::text::{join_and, paragraphs};
use expectation_model::{ExpId, FeatureId, Said, SectionId, StepText, Suite, VarId};
use expectation_story::{Origin, Plan, Status};

/// A section's "Before you begin".
pub struct Before {
    /// Start from this section's setup first.
    pub parent: Option<SectionId>,
    /// What to have in place, one sentence each.
    pub items: Vec<String>,
    /// How to get here from the parent's setup, one sentence per step.
    pub route: Vec<String>,
    /// The route is detail: the items already say how, in the author's words.
    pub route_is_detail: bool,
}

/// The steps from a section's parent's setup to its own, in the reader's words.
pub fn route(d: &Docs, id: SectionId) -> Vec<String> {
    let s = d.suite;
    let Some(route) = d.analysis.section_routes[id.0 as usize].as_ref() else { return vec![] };
    // Step through it, so each outcome can say how things are afterwards ("UPLOAD 3"), not
    // how they changed ("1 more item").
    let prec = &d.analysis.precedence;
    let scope = expectation_graph::flow::in_scope(s, id);
    let (mut state, _) = expectation_graph::semantics::settle(s, prec, &route.start, &[]);
    route
        .steps
        .iter()
        .map(|&e| {
            let x = s.exp(e);
            let candidates = s.transitions().filter(|t| scope[t.id.0 as usize]);
            state = expectation_graph::semantics::perform_among(s, prec, &state, &x.trigger, candidates).after;
            let [_, whens, _] = voice::parts(&x.steps);
            let seen = shown_after(s, &[e], &state);
            let doing = voice::instruction(s, &whens);
            if seen.is_empty() { doing } else { format!("{doing} {seen}") }
        })
        .collect()
}

/// What the Thens of `fired` show, with each state as it is in `after`: "the UPLOAD button
/// reads UPLOAD 3", not "the plan has 1 more item".
fn shown_after(suite: &Suite, fired: &[ExpId], after: &expectation_model::Condition) -> String {
    let mut clauses: Vec<String> = vec![];
    let mut seen: Vec<VarId> = vec![];
    for &e in fired {
        let [_, _, thens] = voice::parts(&suite.exp(e).steps);
        for t in thens {
            let var = match t.meaning {
                Some(Said::Fact(v, _) | Said::Shift(v, _)) => Some(v),
                _ => None,
            };
            if let Some(v) = var {
                if seen.contains(&v) {
                    continue;
                }
                seen.push(v);
            }
            let clause = match var.and_then(|v| after.get(v).map(|c| (v, c))) {
                Some((v, c)) => voice::show(suite.vocabulary.var(v), c),
                None => voice::seen(suite, t),
            };
            if !clauses.contains(&clause) {
                clauses.push(clause);
            }
        }
    }
    voice::sentences(&clauses)
}

pub fn before(d: &Docs, id: SectionId) -> Option<Before> {
    let s = d.suite;
    let section = s.section(id);
    let route = route(d, id);
    if section.setup_steps.is_empty() && route.is_empty() {
        return None;
    }
    // The author's own words for the setup come first. Without them, the route there is the
    // instructions, and without a route, the setup is what to have in place.
    let said = section
        .setup_steps
        .iter()
        .any(|t| matches!(t.meaning, Some(Said::Phrase(p)) if s.vocabulary.phrase(p).say.is_some()));
    let items = if said || route.is_empty() {
        section.setup_steps.iter().map(|t| voice::setup(s, t)).collect()
    } else {
        vec![]
    };
    let detail = said && !route.is_empty();
    Some(Before {
        parent: section.parent.filter(|&p| !s.section(p).start.is_unconstrained()),
        items,
        route,
        route_is_detail: detail,
    })
}

/// A note on an expectation about an exception: one to it, or the one it is.
pub enum Note {
    /// "Except when the pitch stick is forward: see …". `when` is empty if it's the same situation.
    Except { when: String, see: ExpId },
    /// "An exception to …".
    ExceptionTo(ExpId),
    /// An invariant's exception: "… can leave the plan with no items".
    CanLeave { by: ExpId, what: String },
}

/// The exceptions to `e` and the ones it makes. Each says only what's different about the
/// exception's situation: "when the pitch stick is forward", not the aircraft being in hand
/// that both of them need.
pub fn notes(d: &Docs, e: ExpId) -> Vec<Note> {
    let s = d.suite;
    let mut out = vec![];
    for c in d.analysis.exceptions() {
        let (Some(winner), Some(loser)) = (c.winner, c.loser()) else { continue };
        if loser != e && winner != e {
            continue;
        }
        let known = &s.exp(e).given;
        let differs: Vec<String> = c
            .situation
            .iter()
            .filter(|(var, k)| known.get(*var) != Some(*k))
            .map(|(var, k)| s.vocabulary.var(var).describe(k))
            .collect();
        out.push(match c.kind {
            _ if winner == e => Note::ExceptionTo(loser),
            ConflictKind::Outcome => Note::Except { when: join_and(&differs), see: winner },
            ConflictKind::Invariant => Note::CanLeave { by: winner, what: s.describe(&c.situation) },
        });
    }
    out
}

/// One numbered step of a tutorial.
pub struct Step {
    /// What to do: "Press PLAN."
    pub doing: String,
    /// What you'll see: "The map tool is PLAN."
    pub seen: String,
    pub shot: Option<String>,
    pub status: Status,
}

/// A story as a tutorial.
pub struct Tutorial {
    /// The story's name, when the page is one of its scenarios.
    pub kicker: Option<String>,
    pub title: String,
    pub intro: Vec<String>,
    /// What to have in place, one sentence each.
    pub before: Vec<String>,
    /// A section whose page has the setup step by step, when `before` only summarises it.
    pub details: Option<SectionId>,
    pub steps: Vec<Step>,
    /// The features its steps use, in order.
    pub related: Vec<FeatureId>,
    pub narrative: Option<String>,
    /// The scenario as written, for "How this is tested".
    pub written: Vec<StepText>,
}

/// A test's name for the navigation: its story's, unless the story has more scenarios.
pub fn short_title(suite: &Suite, plan: &Plan) -> String {
    match plan.origin {
        Origin::Story { story, scenario } => {
            let st = suite.story(story);
            if st.scenarios.len() == 1 { st.name.clone() } else { st.scenarios[scenario].name.clone() }
        }
        Origin::Coverage { .. } => plan.title.clone(),
    }
}

/// A plan's steps, grouped as written: a phrase of several actions is one step in the docs.
pub fn groups(plan: &Plan) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = vec![];
    let mut last = None;
    for (n, step) in plan.steps.iter().enumerate() {
        match groups.last_mut() {
            Some(g) if last == Some(step.group) => g.push(n),
            _ => groups.push(vec![n]),
        }
        last = Some(step.group);
    }
    groups
}

/// What the reader should see after a group of steps: the story's Thens, or else each state
/// the expectations that fire change, as it is afterwards ("the plan has 3 items" after two
/// clicks, not "the plan has 1 more item" twice).
fn outcome(suite: &Suite, plan: &Plan, group: &[usize]) -> String {
    let first = &plan.steps[group[0]];
    let [_, _, thens] = voice::parts(&first.written);
    if !thens.is_empty() {
        return voice::outcome(suite, &thens);
    }
    let after = &plan.steps[*group.last().unwrap()].state;
    let fired: Vec<ExpId> = group.iter().flat_map(|&n| plan.steps[n].fired.iter().copied()).collect();
    shown_after(suite, &fired, after)
}

pub fn tutorial(d: &Docs, f: &Facts, i: usize) -> Option<Tutorial> {
    let s = d.suite;
    let plan = &d.plans[i];
    let Origin::Story { story, scenario } = plan.origin else { return None };
    let st = s.story(story);
    let sc = &st.scenarios[scenario];
    let result = d.report.and_then(|r| r.results.get(i));
    let mut intro = if scenario == 0 { paragraphs(&st.description) } else { vec![] };
    intro.extend(paragraphs(&sc.description));

    let mut steps = vec![];
    let mut related: Vec<FeatureId> = vec![];
    let mut written: Vec<StepText> = sc.given_steps.clone();
    for group in groups(plan) {
        let first = &plan.steps[group[0]];
        let lines = if first.written.is_empty() { &first.text } else { &first.written };
        written.extend(lines.iter().cloned());
        let [_, whens, _] = voice::parts(lines);
        let last = result.and_then(|r| r.steps.get(*group.last().unwrap()));
        steps.push(Step {
            doing: voice::instruction(s, &whens),
            seen: outcome(s, plan, &group),
            shot: last.and_then(|r| r.screenshot.clone()),
            status: last.map_or(f.status[i], |r| r.status),
        });
        for &n in &group {
            for e in plan.steps[n].exercises() {
                let feature = s.exp(e).feature;
                if !related.contains(&feature) {
                    related.push(feature);
                }
            }
        }
    }
    // Each section's setup on the way down, in the author's words if it has them, or else the
    // steps that get there from the section above; then what the scenario adds.
    let mut before: Vec<String> = vec![];
    let mut details = None;
    for section in s.chain(plan.section) {
        match self::before(d, section.id) {
            Some(b) if b.route_is_detail || b.route.is_empty() => {
                before.extend(b.items);
                if b.route_is_detail {
                    details = Some(section.id);
                }
            }
            Some(b) => before.extend(b.route),
            None => {}
        }
    }
    before.extend(sc.given_steps.iter().map(|t| voice::setup(s, t)));
    Some(Tutorial {
        kicker: (st.scenarios.len() > 1).then(|| st.name.clone()),
        title: short_title(s, plan),
        intro,
        before,
        details,
        steps,
        related,
        narrative: (!st.narrative.is_empty()).then(|| crate::narrative(&st.narrative)),
        written,
    })
}

/// The tutorials that show `e`, each once.
pub fn shown_in(d: &Docs, f: &Facts, e: ExpId) -> Vec<usize> {
    f.tested_by(e).into_iter().filter(|&p| matches!(d.plans[p].origin, Origin::Story { .. })).collect()
}

/// The top-level section a declaration's file is in (`None` for the suite's own files), so
/// the glossary can list each system's terms together.
pub fn declared_in(suite: &Suite, span: &expectation_model::Span) -> Option<SectionId> {
    let dir = span.file.rsplit_once('/').map_or("", |(d, _)| d);
    suite
        .sections
        .iter()
        .filter(|s| s.parent == Some(SectionId(0)))
        .find(|s| dir == s.path || dir.starts_with(&format!("{}/", s.path)))
        .map(|s| s.id)
}

/// The glossary's groups: the top-level sections in order, then the suite's own terms.
pub fn glossary_groups(suite: &Suite) -> Vec<Option<SectionId>> {
    let mut groups: Vec<Option<SectionId>> = suite.section(SectionId(0)).children.iter().map(|&c| Some(c)).collect();
    groups.push(None);
    groups
}

/// Something prose can link to with `[[its name]]`.
#[derive(Clone, Copy, PartialEq)]
pub enum Ref {
    Section(SectionId),
    Feature(FeatureId),
    Exp(ExpId),
    /// A story's first tutorial.
    Tutorial(usize),
    /// A glossary term.
    Term(usize),
}

/// What `[[name]]` refers to: a section, feature, expectation, story or glossary term with
/// that name, ignoring case.
pub fn find(d: &Docs, name: &str) -> Option<Ref> {
    let s = d.suite;
    let is = |n: &str| n.eq_ignore_ascii_case(name.trim());
    if let Some(t) = s.glossary.iter().position(|t| is(&t.name)) {
        return Some(Ref::Term(t));
    }
    if let Some(x) = s.sections.iter().find(|x| is(&x.name)) {
        return Some(Ref::Section(x.id));
    }
    if let Some(x) = s.features.iter().find(|x| is(&x.name)) {
        return Some(Ref::Feature(x.id));
    }
    if let Some(i) = (0..d.plans.len()).find(|&i| match d.plans[i].origin {
        Origin::Story { story, .. } => is(&s.story(story).name) || is(&short_title(s, &d.plans[i])),
        Origin::Coverage { .. } => false,
    }) {
        return Some(Ref::Tutorial(i));
    }
    s.expectations.iter().find(|x| is(&x.name) && !x.hidden).map(|x| Ref::Exp(x.id))
}

/// A piece of prose: plain text, `code`, or a `[[link]]`.
pub enum Inline<'a> {
    Text(&'a str),
    Code(&'a str),
    Link(&'a str),
}

/// Prose split into its pieces. Unclosed markers are left as text.
pub fn inline(text: &str) -> Vec<Inline<'_>> {
    let mut out = vec![];
    let mut rest = text;
    loop {
        let code = rest.find('`');
        let link = rest.find("[[");
        let (at, is_code) = match (code, link) {
            (Some(c), Some(l)) if c < l => (c, true),
            (_, Some(l)) => (l, false),
            (Some(c), None) => (c, true),
            (None, None) => break,
        };
        let (open, close) = if is_code { ("`", "`") } else { ("[[", "]]") };
        let after = &rest[at + open.len()..];
        let Some(end) = after.find(close) else { break };
        if at > 0 {
            out.push(Inline::Text(&rest[..at]));
        }
        out.push(if is_code { Inline::Code(&after[..end]) } else { Inline::Link(&after[..end]) });
        rest = &after[end + close.len()..];
    }
    if !rest.is_empty() {
        out.push(Inline::Text(rest));
    }
    out
}

/// Prose without its markers, for titles and summaries that can't hold links.
pub fn plain(text: &str) -> String {
    inline(text)
        .into_iter()
        .map(|p| match p {
            Inline::Text(t) | Inline::Code(t) | Inline::Link(t) => t,
        })
        .collect()
}

/// The terms of the glossary in alphabetical order, as indexes into `suite.glossary`.
pub fn terms(suite: &Suite) -> Vec<usize> {
    let mut order: Vec<usize> = (0..suite.glossary.len()).collect();
    order.sort_by_key(|&i| suite.glossary[i].name.to_lowercase());
    order
}

/// Pages in reading order: each section, then its tutorials, its topics and its subsections.
#[derive(Clone, Copy, PartialEq)]
pub enum Reading {
    Section(SectionId),
    Tutorial(usize),
    Feature(FeatureId),
}

pub fn reading_order(d: &Docs) -> Vec<Reading> {
    fn walk(d: &Docs, id: SectionId, out: &mut Vec<Reading>) {
        let s = d.suite.section(id);
        out.push(Reading::Section(id));
        out.extend(
            (0..d.plans.len())
                .filter(|&i| d.plans[i].section == id && matches!(d.plans[i].origin, Origin::Story { .. }))
                .map(Reading::Tutorial),
        );
        out.extend(s.features.iter().map(|&f| Reading::Feature(f)));
        for &c in &s.children {
            walk(d, c, out);
        }
    }
    let mut out = vec![];
    walk(d, SectionId(0), &mut out);
    out
}

/// The tutorials in reading order.
pub fn tutorials(d: &Docs) -> Vec<usize> {
    reading_order(d)
        .into_iter()
        .filter_map(|r| match r {
            Reading::Tutorial(i) => Some(i),
            _ => None,
        })
        .collect()
}

/// Why a reading page has no screenshot, in one wording.
pub fn missing(status: Status) -> &'static str {
    match status {
        Status::Unsupported => "Screenshot to come: the tests cannot do this step yet.",
        _ => "Screenshot to come.",
    }
}
