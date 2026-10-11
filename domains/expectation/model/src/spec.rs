//! Expectations, features and stories: what a suite says, after its text has been resolved
//! against the vocabulary.

use crate::condition::{Condition, Constraint, Interval};
use crate::diagnostic::{Sources, Span};
use crate::vocabulary::{EventId, VarId, Variable, Vocabulary};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExpId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FeatureId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoryId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectionId(pub u32);

/// A Then that's relative to what was there before: "the plan goes up by 1 item", "the
/// selected item stays the same".
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shift {
    By(f64),
    Same,
}

impl Shift {
    /// What `var` can be afterwards, given what it could be before.
    pub fn apply(self, var: &Variable, before: Option<&Constraint>) -> Constraint {
        let before = before.copied().unwrap_or_else(|| var.full());
        match (self, before, var.full()) {
            (Shift::By(d), Constraint::Range(r), Constraint::Range(full)) => {
                Constraint::Range(Interval { lo: r.lo + d, hi: r.hi + d, ..r }.intersect(&full))
            }
            _ => before,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keyword {
    Given,
    When,
    Then,
    And,
    But,
}

impl Keyword {
    pub fn as_str(self) -> &'static str {
        match self {
            Keyword::Given => "Given",
            Keyword::When => "When",
            Keyword::Then => "Then",
            Keyword::And => "And",
            Keyword::But => "But",
        }
    }
}

/// A step as written, kept for docs and generated tests, with what it turned out to mean
/// (none if it didn't resolve) so docs can say it in the reader's words.
#[derive(Clone, Debug)]
pub struct StepText {
    pub keyword: Keyword,
    pub text: String,
    pub span: Span,
    pub meaning: Option<Said>,
}

/// What a step says, once resolved.
#[derive(Clone, Copy, Debug)]
pub enum Said {
    Fact(VarId, Constraint),
    Event(EventId),
    Phrase(PhraseId),
    Shift(VarId, Shift),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhraseId(pub u32);

/// A step of your own, built from others: a higher-level way to say a setup or a sequence of
/// actions, like "the operator plans a circuit" for +TKOF, three clicks on the map and +RTL.
/// Specs read better and stay put when the details change; docs say it with `say`.
#[derive(Clone, Debug)]
pub struct Phrase {
    pub id: PhraseId,
    pub name: String,
    pub key: String,
    /// How docs say it: an instruction for actions ("press +TKOF, click three waypoints on the
    /// map, then press +RTL"), a clause for a setup ("a mission uploaded").
    pub say: Option<String>,
    pub span: Span,
    pub body: PhraseBody,
}

#[derive(Clone, Debug)]
pub enum PhraseBody {
    /// Facts, for Givens (and Thens).
    Setup(Vec<Fact>),
    /// Actions in order, for Whens: one for an expectation, any number in a story.
    Actions(Vec<Trigger>),
}

/// One resolved step: "the throttle is above 55 %".
#[derive(Clone, Debug)]
pub struct Fact {
    pub var: VarId,
    pub constraint: Constraint,
    pub span: Span,
}

/// What sets an expectation off: an event, a change the outside world makes to some states,
/// or both at once.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Trigger {
    pub event: Option<EventId>,
    pub changes: Condition,
}

impl Trigger {
    /// No event and no change: the expectation is an invariant that holds whenever its Given does.
    pub fn is_none(&self) -> bool {
        self.event.is_none() && self.changes.is_unconstrained()
    }

    /// Could a single action set off both?
    pub fn compatible(&self, o: &Trigger) -> bool {
        self.event == o.event && self.changes.overlaps(&o.changes)
    }

    /// Does performing this action certainly set off an expectation triggered by `o`?
    pub fn fires(&self, o: &Trigger) -> bool {
        !o.is_none() && self.event == o.event && self.changes.implies(&o.changes)
    }

    /// Might performing this action set off `o`, depending on which value it picks? Only if
    /// it changes every state `o` is set off by.
    pub fn may_fire(&self, o: &Trigger) -> bool {
        !o.is_none() && self.compatible(o) && o.changes.vars().all(|v| self.changes.get(v).is_some())
    }
}

/// Given some state, when something happens, then the system ends up in some state.
#[derive(Clone, Debug)]
pub struct Expectation {
    pub id: ExpId,
    pub feature: FeatureId,
    pub name: String,
    pub slug: String,
    pub description: Vec<String>,
    pub span: Span,
    pub given: Condition,
    pub trigger: Trigger,
    pub then: Condition,
    /// Relative Thens, applied to the state before.
    pub shifts: Vec<(VarId, Shift)>,
    /// Expectations this one is an exception to: where both apply, this one wins.
    pub overrides: Vec<ExpId>,
    pub steps: Vec<StepText>,
    /// `Docs: hidden`: tested, but left out of the reading pages (it's only in the test report).
    pub hidden: bool,
}

impl Expectation {
    pub fn is_invariant(&self) -> bool {
        self.trigger.is_none()
    }

    /// What its Then says the states it mentions become, from the state `before`.
    pub fn effect(&self, vocab: &Vocabulary, before: &Condition) -> Condition {
        let mut out = self.then.clone();
        for &(v, shift) in &self.shifts {
            let var = vocab.var(v);
            out.set(var, shift.apply(var, before.get(v)));
        }
        out
    }

    /// The most general state it leaves the system in: its Given, with the trigger's changes
    /// and then its Then written over the top. For an invariant, its Given and Then together.
    pub fn outcome(&self, vocab: &Vocabulary) -> Condition {
        if self.is_invariant() {
            self.given.and(&self.then)
        } else {
            let before = self.given.overlay(&self.trigger.changes);
            before.overlay(&self.effect(vocab, &before))
        }
    }

    /// The states its Then mentions.
    pub fn sets(&self) -> impl Iterator<Item = VarId> + '_ {
        self.then.vars().chain(self.shifts.iter().map(|(v, _)| *v))
    }

    pub fn when_steps(&self) -> impl Iterator<Item = &StepText> {
        let mut section = Keyword::Given;
        self.steps.iter().filter(move |s| {
            if !matches!(s.keyword, Keyword::And | Keyword::But) {
                section = s.keyword;
            }
            section == Keyword::When
        })
    }
}

/// A group of features and stories that share a setup, nested like a book's chapters: one per
/// directory of the suite. Everything in a section starts from its `start`.
#[derive(Clone, Debug)]
pub struct Section {
    pub id: SectionId,
    pub parent: Option<SectionId>,
    pub name: String,
    /// Unique in the suite: its ancestors' slugs and its own, joined by "--". Empty for the root.
    pub slug: String,
    /// Its directory, relative to the suite's. Empty for the root.
    pub path: String,
    pub description: Vec<String>,
    /// What its Background adds to its parent's.
    pub setup: Condition,
    pub setup_steps: Vec<StepText>,
    /// The state everything in it starts from: its parent's start with `setup` written over it.
    pub start: Condition,
    pub span: Option<Span>,
    pub children: Vec<SectionId>,
    pub features: Vec<FeatureId>,
    pub stories: Vec<StoryId>,
}

#[derive(Clone, Debug)]
pub struct Feature {
    pub id: FeatureId,
    pub section: SectionId,
    pub name: String,
    pub slug: String,
    pub description: Vec<String>,
    pub span: Span,
    pub expectations: Vec<ExpId>,
}

/// "As a pilot, I want …, so that …", told through one or more scenarios.
#[derive(Clone, Debug)]
pub struct Story {
    pub id: StoryId,
    pub section: SectionId,
    pub name: String,
    pub slug: String,
    /// The user story: its "As a …", "I want …" and "So that …" lines.
    pub narrative: Vec<String>,
    /// The rest of the story's text: its introduction in the docs.
    pub description: Vec<String>,
    pub span: Span,
    pub scenarios: Vec<Scenario>,
}

#[derive(Clone, Debug)]
pub struct Scenario {
    pub name: String,
    pub slug: String,
    pub description: Vec<String>,
    pub span: Span,
    /// Arranged on top of the Background before the first step.
    pub given: Condition,
    pub given_steps: Vec<StepText>,
    pub steps: Vec<StoryStep>,
}

/// One "When …" of a story and the "Then …"s the story checks after it.
#[derive(Clone, Debug)]
pub struct StoryStep {
    pub action: Trigger,
    pub checks: Vec<Fact>,
    pub steps: Vec<StepText>,
    pub span: Span,
    /// Which written step it came from: a phrase of several actions makes several steps of
    /// one group, which docs show as one step.
    pub group: usize,
    /// The group's lines as written (When, And, Then), on its first step only. `steps` has
    /// what each step does, phrases spelt out, for the test report.
    pub written: Vec<StepText>,
}

/// A whole suite: the vocabulary, its sections (the first is the root), and the expectations
/// and stories written against them.
#[derive(Clone, Debug, Default)]
pub struct Suite {
    pub name: String,
    pub vocabulary: Vocabulary,
    /// The root section's start, which every section builds on.
    pub background: Condition,
    pub background_steps: Vec<StepText>,
    pub sections: Vec<Section>,
    pub features: Vec<Feature>,
    pub expectations: Vec<Expectation>,
    pub stories: Vec<Story>,
    /// `Term:`s: the glossary's definitions.
    pub glossary: Vec<Term>,
    pub sources: Sources,
}

/// `Term: <name>` and the text under it: a definition for the glossary.
#[derive(Clone, Debug)]
pub struct Term {
    pub name: String,
    pub slug: String,
    pub description: Vec<String>,
    pub span: Span,
}

impl Suite {
    pub fn exp(&self, id: ExpId) -> &Expectation {
        &self.expectations[id.0 as usize]
    }

    pub fn feature(&self, id: FeatureId) -> &Feature {
        &self.features[id.0 as usize]
    }

    pub fn story(&self, id: StoryId) -> &Story {
        &self.stories[id.0 as usize]
    }

    pub fn section(&self, id: SectionId) -> &Section {
        &self.sections[id.0 as usize]
    }

    /// The root first, down to `id`.
    pub fn chain(&self, id: SectionId) -> Vec<&Section> {
        let mut out = vec![self.section(id)];
        while let Some(p) = out.last().unwrap().parent {
            out.push(self.section(p));
        }
        out.reverse();
        out
    }

    /// The section an expectation belongs to (its feature's).
    pub fn section_of(&self, e: ExpId) -> &Section {
        self.section(self.feature(self.exp(e).feature).section)
    }

    /// `id` and every section below it.
    pub fn subtree(&self, id: SectionId) -> Vec<SectionId> {
        let mut out = vec![id];
        let mut i = 0;
        while i < out.len() {
            out.extend(self.section(out[i]).children.iter().copied());
            i += 1;
        }
        out
    }

    pub fn describe(&self, c: &Condition) -> String {
        self.vocabulary.describe(c)
    }

    /// "the pilot presses reset", "the throttle is above 55 %", or both.
    pub fn describe_trigger(&self, t: &Trigger) -> String {
        let mut parts = vec![];
        if !t.changes.is_unconstrained() {
            parts.push(self.vocabulary.describe(&t.changes));
        }
        if let Some(e) = t.event {
            parts.push(self.vocabulary.event(e).text.clone());
        }
        if parts.is_empty() { "always".into() } else { parts.join(" and ") }
    }

    /// Transitions (expectations with a trigger), in declaration order.
    pub fn transitions(&self) -> impl Iterator<Item = &Expectation> {
        self.expectations.iter().filter(|e| !e.is_invariant())
    }

    pub fn invariants(&self) -> impl Iterator<Item = &Expectation> {
        self.expectations.iter().filter(|e| e.is_invariant())
    }
}
