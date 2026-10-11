//! Conflicts: two expectations that can both apply but promise things that can't both be true.
//!
//! - **Outcome**: the same action, Givens that overlap, Thens that contradict. In a state
//!   where both Givens hold, the system can't do both.
//! - **Invariant**: an expectation leaves the system in a state where an invariant's Given
//!   holds but its Then can't.
//!
//! An `Overrides:` between the two (directly or through a chain) settles a conflict: the
//! winner is an exception to the loser. Settled conflicts are kept, since docs show them.

use crate::precedence::Precedence;
use expectation_model::{Condition, Diagnostic, ExpId, Expectation, Shift, Suite, Trigger, VarId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Outcome,
    Invariant,
}

#[derive(Clone, Debug)]
pub struct Conflict {
    pub kind: Kind,
    /// For [`Kind::Invariant`], `a` breaks the invariant `b`.
    pub a: ExpId,
    pub b: ExpId,
    /// The states they disagree about.
    pub vars: Vec<VarId>,
    /// States where both apply (for an invariant conflict: the states `a` leaves behind).
    pub situation: Condition,
    /// The action that sets both off (outcome conflicts).
    pub trigger: Trigger,
    /// The exception that settles it.
    pub winner: Option<ExpId>,
}

impl Conflict {
    pub fn involves(&self, x: ExpId, y: ExpId) -> bool {
        (self.a == x && self.b == y) || (self.a == y && self.b == x)
    }

    pub fn loser(&self) -> Option<ExpId> {
        self.winner.map(|w| if w == self.a { self.b } else { self.a })
    }
}

pub fn detect(suite: &Suite, prec: &Precedence) -> Vec<Conflict> {
    let mut out = vec![];
    let winner = |a: ExpId, b: ExpId| {
        if prec.beats(a, b) {
            Some(a)
        } else if prec.beats(b, a) {
            Some(b)
        } else {
            None
        }
    };

    let transitions: Vec<&Expectation> = suite.transitions().collect();
    for (i, a) in transitions.iter().enumerate() {
        for b in &transitions[i + 1..] {
            if !a.trigger.compatible(&b.trigger) {
                continue;
            }
            let situation = a.given.and(&b.given);
            if !situation.is_satisfiable() {
                continue;
            }
            let trigger = Trigger { event: a.trigger.event, changes: a.trigger.changes.and(&b.trigger.changes) };
            let before = situation.overlay(&trigger.changes);
            let vars = disagree(suite, a, b, &before);
            if vars.is_empty() {
                continue;
            }
            out.push(Conflict {
                kind: Kind::Outcome,
                a: a.id,
                b: b.id,
                vars,
                situation,
                trigger,
                winner: winner(a.id, b.id),
            });
        }
    }

    for x in &suite.expectations {
        let after = x.outcome(&suite.vocabulary);
        for inv in suite.invariants() {
            if inv.id == x.id || out.iter().any(|c| c.kind == Kind::Invariant && c.involves(x.id, inv.id)) {
                continue;
            }
            let scope = after.and(&inv.given);
            if !scope.is_satisfiable() || scope.and(&inv.then).is_satisfiable() {
                continue;
            }
            // Only if `x` is what breaks it: something it changes is in the invariant's Given
            // or is what the invariant gets wrong. Otherwise the clash was in its Given already,
            // a state the invariant itself rules out.
            let vars = scope.contradictions(&inv.then);
            let changed: Vec<VarId> = x.sets().chain(x.trigger.changes.vars()).collect();
            if !changed.iter().any(|v| inv.given.get(*v).is_some() || vars.contains(v)) {
                continue;
            }
            out.push(Conflict {
                kind: Kind::Invariant,
                a: x.id,
                b: inv.id,
                vars,
                situation: scope,
                trigger: x.trigger.clone(),
                winner: winner(x.id, inv.id),
            });
        }
    }
    out
}

/// The states two expectations' Thens disagree about, starting from `before`. Absolute Thens
/// disagree if they can't both hold. Two different relative Thens always disagree. A relative
/// Then and an absolute one disagree unless `before` pins down the one value where they meet.
fn disagree(suite: &Suite, a: &Expectation, b: &Expectation, before: &Condition) -> Vec<VarId> {
    let v = &suite.vocabulary;
    let (ea, eb) = (a.effect(v, before), b.effect(v, before));
    let mut vars = ea.contradictions(&eb);
    let shift = |e: &Expectation, x: VarId| {
        e.shifts.iter().find(|(w, _)| *w == x).map(|(_, s)| match s {
            Shift::Same => 0.0,
            Shift::By(d) => *d,
        })
    };
    for x in a.sets().filter(|x| b.sets().any(|y| y == *x)) {
        let differ = match (shift(a, x), shift(b, x)) {
            (Some(p), Some(q)) => p != q,
            (Some(_), None) | (None, Some(_)) => ea.get(x) != eb.get(x),
            (None, None) => false,
        };
        if differ && !vars.contains(&x) {
            vars.push(x);
        }
    }
    vars
}

/// Errors for unsettled conflicts, warnings for overrides that settle nothing, and errors
/// for a Background that breaks an invariant.
pub fn diagnose(suite: &Suite, prec: &Precedence, conflicts: &[Conflict]) -> Vec<Diagnostic> {
    let v = &suite.vocabulary;
    let mut diags = vec![];
    for c in conflicts.iter().filter(|c| c.winner.is_none()) {
        let (a, b) = (suite.exp(c.a), suite.exp(c.b));
        let subjects: Vec<String> = c.vars.iter().map(|&x| v.var(x).subject.clone()).collect();
        let before = c.situation.overlay(&c.trigger.changes);
        let on = |e: &Expectation| v.describe(&e.effect(v, &before).restrict(c.vars.iter().copied()));
        let d = match c.kind {
            Kind::Outcome => {
                let mut when = v.describe(&c.situation);
                if !c.trigger.is_none() {
                    when = format!("{when}, and {}", suite.describe_trigger(&c.trigger));
                }
                Diagnostic::error(
                    "conflict",
                    format!(
                        "«{}» and «{}» disagree about {}",
                        a.name,
                        b.name,
                        expectation_model::text::join_and(&subjects)
                    ),
                )
                .at(&a.span, format!("«{}» says {}", a.name, on(a)))
                .at(&b.span, format!("«{}» says {}", b.name, on(b)))
                .note_that(format!("both apply when {when}"))
                .help(exception_help(a, b))
            }
            Kind::Invariant => Diagnostic::error("conflict", format!("«{}» breaks «{}»", a.name, b.name))
                .at(&a.span, format!("afterwards {}", v.describe(&c.situation.restrict(c.vars.iter().copied()))))
                .at(&b.span, format!("but whenever {}, {}", v.describe(&b.given), on(b)))
                .help(format!(
                    "narrow one of the Givens, or if «{}» is a deliberate exception, add `Overrides: {}` to it",
                    a.name, b.name
                )),
        };
        diags.push(d);
    }

    for &(w, l) in &prec.declared {
        if !conflicts.iter().any(|c| c.involves(w, l)) {
            let (w, l) = (suite.exp(w), suite.exp(l));
            diags.push(
                Diagnostic::warning(
                    "unneeded-override",
                    format!("«{}» overrides «{}», but they never disagree", w.name, l.name),
                )
                .at(&w.span, "")
                .help("either they can't apply at the same time or they agree; remove the Overrides line"),
            );
        }
    }

    for section in &suite.sections {
        for inv in suite.invariants() {
            let scope = section.start.and(&inv.given);
            if scope.is_satisfiable() && !scope.and(&inv.then).is_satisfiable() {
                let mut d = Diagnostic::error(
                    "conflict",
                    format!("the Background of «{}» breaks «{}»", section.name, inv.name),
                )
                .at(&inv.span, format!("whenever {}, {}", v.describe(&inv.given), v.describe(&inv.then)));
                if let Some(s) = section.setup_steps.first() {
                    d = d.at(&s.span, format!("but «{}» starts with {}", section.name, v.describe(&section.start)));
                }
                diags.push(d);
            }
        }
    }
    diags
}

fn exception_help(a: &Expectation, b: &Expectation) -> String {
    // The more specific one is the likely exception.
    let (special, general) = if a.given.implies(&b.given) && !b.given.implies(&a.given) { (a, b) } else { (b, a) };
    let specific = special.given.implies(&general.given) && !general.given.implies(&special.given);
    if specific {
        format!(
            "«{}» is the more specific; if it's an exception, add `Overrides: {}` to it. Otherwise narrow a Given so they can't both apply",
            special.name, general.name
        )
    } else {
        "if one is an exception to the other, add `Overrides: <the other>` to it; otherwise narrow a Given so they can't both apply".into()
    }
}
