//! The expectation domain's core model: the vocabulary a suite is written in, the conditions
//! built from it, and the expectations and stories that use them.
//!
//! An *expectation* says how the system behaves: given some state, when something happens,
//! then the system ends up in some state. A *story* is a user's journey through those
//! behaviours, which becomes an acceptance test. Everything here is plain data plus the
//! condition algebra (intersection, implication, overlay) that conflict detection, the flow
//! graph and story planning are built on.

pub mod condition;
pub mod diagnostic;
pub mod spec;
pub mod text;
pub mod vocabulary;

pub use condition::{Condition, Constraint, Interval};
pub use diagnostic::{Diagnostic, Label, Severity, Sources, Span};
pub use spec::{
    ExpId, Expectation, Fact, Feature, FeatureId, Keyword, Phrase, PhraseBody, PhraseId, Said, Scenario, Section,
    SectionId, Shift, StepText, Story, StoryId, StoryStep, Suite, Term, Trigger,
};
pub use vocabulary::{Event, EventId, Value, VarId, VarKind, Variable, Vocabulary};
