//! What happens when the system is in some state and something happens to it. States here are
//! [`Condition`]s, sets of concrete states, so one step answers for all of them at once.
//! Story planning and the flow graph both step through a suite with [`perform`].

use crate::precedence::Precedence;
use expectation_model::{Condition, ExpId, Expectation, Suite, Trigger, VarId};
use std::collections::BTreeSet;

#[derive(Debug, Default)]
pub struct Outcome {
    /// Expectations that certainly fire.
    pub fired: Vec<ExpId>,
    /// Expectations that might fire: the state or the action doesn't pin down whether their
    /// Given or trigger holds.
    pub uncertain: Vec<ExpId>,
    /// (loser, winner): would have fired, but an exception that also fires wins.
    pub overridden: Vec<(ExpId, ExpId)>,
    /// Fired expectations whose Thens can't both hold, and the state they disagree on.
    pub clashes: Vec<(ExpId, ExpId, VarId)>,
    /// Invariants in force afterwards.
    pub invariants: Vec<ExpId>,
    /// The state afterwards.
    pub after: Condition,
}

/// Perform `action` in `state`.
///
/// An expectation fires if the state implies its Given and the action implies its trigger.
/// The rest of a compound action happens first: in "When the throttle is 0 % and the pilot
/// presses reset", an expectation about reset sees the throttle at 0. The new state is the old
/// one with the action's changes and every fired Then written over it (anything nobody
/// mentions keeps its value), and then the invariants applied.
pub fn perform(suite: &Suite, prec: &Precedence, state: &Condition, action: &Trigger) -> Outcome {
    perform_among(suite, prec, state, action, suite.transitions())
}

/// [`perform`], looking only at `candidates`: every transition the action might set off must
/// be among them. Searches that perform the same actions over and over work these out once.
pub fn perform_among<'a>(
    suite: &Suite,
    prec: &Precedence,
    state: &Condition,
    action: &Trigger,
    candidates: impl IntoIterator<Item = &'a Expectation>,
) -> Outcome {
    let mut fired = vec![];
    let mut uncertain = vec![];
    for e in candidates {
        if !action.may_fire(&e.trigger) {
            continue;
        }
        let seen = seen_by(state, action, &e.trigger);
        if !seen.overlaps(&e.given) {
            continue;
        }
        if action.fires(&e.trigger) && seen.implies(&e.given) {
            fired.push(e.id);
        } else {
            uncertain.push(e.id);
        }
    }

    // Exceptions win. A winner that only might fire leaves its loser uncertain too.
    let mut overridden = vec![];
    let all = fired.clone();
    fired.retain(|&l| match all.iter().find(|&&w| prec.beats(w, l)) {
        Some(&w) => {
            overridden.push((l, w));
            false
        }
        None => true,
    });
    uncertain.retain(|&l| !all.iter().any(|&w| prec.beats(w, l)));
    let (fired, maybe_beaten): (Vec<_>, Vec<_>) =
        fired.into_iter().partition(|&l| !uncertain.iter().any(|&w| prec.beats(w, l)));
    uncertain.extend(maybe_beaten);

    // What each Then makes of the state the action leaves, relative Thens included.
    let before = state.overlay(&action.changes);
    let effects: Vec<Condition> = fired.iter().map(|&e| suite.exp(e).effect(&suite.vocabulary, &before)).collect();
    let mut then = Condition::new();
    let mut clashes = vec![];
    for (i, &a) in fired.iter().enumerate() {
        for (j, &b) in fired[..i].iter().enumerate() {
            for v in effects[j].contradictions(&effects[i]) {
                clashes.push((b, a, v));
            }
        }
        then = then.and(&effects[i]);
    }
    let (after, invariants) = settle(suite, prec, &before.overlay(&then), &fired);
    Outcome { fired, uncertain, overridden, clashes, invariants, after }
}

/// The state an expectation with `trigger` checks its Given against when `action` happens in
/// `state`: the action's other changes already made.
pub fn seen_by(state: &Condition, action: &Trigger, trigger: &Trigger) -> Condition {
    let others = action.changes.vars().filter(|v| trigger.changes.get(*v).is_none());
    state.overlay(&action.changes.restrict(others))
}

/// Apply every invariant whose Given certainly holds, until nothing changes. Invariants that
/// one of `exempt_by` overrides are skipped.
pub fn settle(suite: &Suite, prec: &Precedence, state: &Condition, exempt_by: &[ExpId]) -> (Condition, Vec<ExpId>) {
    let mut s = state.clone();
    let mut in_force = BTreeSet::new();
    for _ in 0..8 {
        let mut changed = false;
        for inv in suite.invariants() {
            if !s.is_satisfiable() || exempt_by.iter().any(|&w| prec.beats(w, inv.id)) || !s.implies(&inv.given) {
                continue;
            }
            in_force.insert(inv.id);
            let next = s.and(&inv.then);
            if next != s {
                s = next;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    (s, in_force.into_iter().collect())
}
