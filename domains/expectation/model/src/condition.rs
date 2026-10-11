//! The condition algebra. A [`Condition`] is a conjunction of per-variable [`Constraint`]s and
//! stands for the set of states that satisfy it. Conflict detection, the flow graph and story
//! planning reduce to three questions about such sets: do two overlap, does one imply the
//! other, and what is left after an action overwrites some variables.

use crate::vocabulary::{VarId, Variable};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

/// A numeric range with open or closed ends.
#[derive(Clone, Copy, Debug)]
pub struct Interval {
    pub lo: f64,
    pub lo_open: bool,
    pub hi: f64,
    pub hi_open: bool,
}

impl Interval {
    pub fn closed(lo: f64, hi: f64) -> Self {
        Interval { lo, lo_open: false, hi, hi_open: false }
    }

    pub fn point(v: f64) -> Self {
        Self::closed(v, v)
    }

    pub fn is_empty(&self) -> bool {
        self.lo > self.hi || (self.lo == self.hi && (self.lo_open || self.hi_open))
    }

    pub fn contains(&self, v: f64) -> bool {
        let above = if self.lo_open { v > self.lo } else { v >= self.lo };
        let below = if self.hi_open { v < self.hi } else { v <= self.hi };
        above && below
    }

    pub fn midpoint(&self) -> f64 {
        0.5 * (self.lo + self.hi)
    }

    pub fn intersect(&self, o: &Interval) -> Interval {
        let (lo, lo_open) = match self.lo.partial_cmp(&o.lo) {
            Some(std::cmp::Ordering::Greater) => (self.lo, self.lo_open),
            Some(std::cmp::Ordering::Less) => (o.lo, o.lo_open),
            _ => (self.lo, self.lo_open || o.lo_open),
        };
        let (hi, hi_open) = match self.hi.partial_cmp(&o.hi) {
            Some(std::cmp::Ordering::Less) => (self.hi, self.hi_open),
            Some(std::cmp::Ordering::Greater) => (o.hi, o.hi_open),
            _ => (self.hi, self.hi_open || o.hi_open),
        };
        Interval { lo, lo_open, hi, hi_open }
    }

    /// Is every value in `self` also in `o`?
    pub fn within(&self, o: &Interval) -> bool {
        if self.is_empty() {
            return true;
        }
        let lo = o.lo < self.lo || (o.lo == self.lo && (!o.lo_open || self.lo_open));
        let hi = o.hi > self.hi || (o.hi == self.hi && (!o.hi_open || self.hi_open));
        lo && hi
    }

    fn bits(&self) -> (u64, bool, u64, bool) {
        // +0.0 and -0.0 are the same bound.
        ((self.lo + 0.0).to_bits(), self.lo_open, (self.hi + 0.0).to_bits(), self.hi_open)
    }
}

impl PartialEq for Interval {
    fn eq(&self, o: &Self) -> bool {
        self.bits() == o.bits()
    }
}

impl Eq for Interval {}

impl Hash for Interval {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.bits().hash(h)
    }
}

/// The values one variable may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Constraint {
    /// Bit `i` set = the variable's `i`th declared value is allowed.
    OneOf(u64),
    Range(Interval),
}

impl Constraint {
    pub fn is_empty(&self) -> bool {
        match self {
            Constraint::OneOf(m) => *m == 0,
            Constraint::Range(r) => r.is_empty(),
        }
    }

    pub fn intersect(&self, o: &Constraint) -> Constraint {
        match (self, o) {
            (Constraint::OneOf(a), Constraint::OneOf(b)) => Constraint::OneOf(a & b),
            (Constraint::Range(a), Constraint::Range(b)) => Constraint::Range(a.intersect(b)),
            // A variable has one kind; mixing them is a bug upstream, and nothing satisfies both.
            _ => Constraint::OneOf(0),
        }
    }

    pub fn within(&self, o: &Constraint) -> bool {
        match (self, o) {
            (Constraint::OneOf(a), Constraint::OneOf(b)) => a & !b == 0,
            (Constraint::Range(a), Constraint::Range(b)) => a.within(b),
            _ => self.is_empty(),
        }
    }

    pub fn overlaps(&self, o: &Constraint) -> bool {
        !self.intersect(o).is_empty()
    }
}

/// A set of states, written as a conjunction of constraints. Variables it doesn't mention are
/// unconstrained.
///
/// Invariant: a stored constraint is never a variable's full domain, so "mentioned" always
/// means "narrowed". [`Condition::constrain`] keeps that true; the other operations preserve it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Condition(BTreeMap<VarId, Constraint>);

impl Condition {
    pub fn new() -> Self {
        Self::default()
    }

    /// No constraints at all: every state.
    pub fn is_unconstrained(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, v: VarId) -> Option<&Constraint> {
        self.0.get(&v)
    }

    pub fn iter(&self) -> impl Iterator<Item = (VarId, &Constraint)> + '_ {
        self.0.iter().map(|(v, c)| (*v, c))
    }

    pub fn vars(&self) -> impl Iterator<Item = VarId> + '_ {
        self.0.keys().copied()
    }

    /// Narrow `var` to `c` (on top of anything already required of it).
    pub fn constrain(&mut self, var: &Variable, c: Constraint) {
        let c = c.intersect(&var.full());
        let c = match self.0.get(&var.id) {
            Some(prev) => prev.intersect(&c),
            None => c,
        };
        if c == var.full() {
            self.0.remove(&var.id);
        } else {
            self.0.insert(var.id, c);
        }
    }

    /// Replace whatever is required of `var` with `c`.
    pub fn set(&mut self, var: &Variable, c: Constraint) {
        let c = c.intersect(&var.full());
        if c == var.full() {
            self.0.remove(&var.id);
        } else {
            self.0.insert(var.id, c);
        }
    }

    /// States in both.
    pub fn and(&self, o: &Condition) -> Condition {
        let mut out = self.0.clone();
        for (v, c) in &o.0 {
            out.entry(*v).and_modify(|e| *e = e.intersect(c)).or_insert(*c);
        }
        Condition(out)
    }

    /// Does any state satisfy it?
    pub fn is_satisfiable(&self) -> bool {
        self.0.values().all(|c| !c.is_empty())
    }

    /// Is there a state in both?
    pub fn overlaps(&self, o: &Condition) -> bool {
        self.and(o).is_satisfiable()
    }

    /// Is every state of `self` also in `o`?
    pub fn implies(&self, o: &Condition) -> bool {
        !self.is_satisfiable() || o.0.iter().all(|(v, c)| self.0.get(v).is_some_and(|s| s.within(c)))
    }

    /// `self` with `o`'s variables replaced by what `o` says: the state after an action that
    /// sets those variables, leaving the rest alone.
    pub fn overlay(&self, o: &Condition) -> Condition {
        let mut out = self.0.clone();
        out.extend(o.0.iter().map(|(v, c)| (*v, *c)));
        Condition(out)
    }

    /// Variables both mention but on which they can't both hold.
    pub fn contradictions(&self, o: &Condition) -> Vec<VarId> {
        self.0.iter().filter(|(v, c)| o.0.get(v).is_some_and(|d| !c.overlaps(d))).map(|(v, _)| *v).collect()
    }

    /// Only the given variables.
    pub fn restrict(&self, vars: impl IntoIterator<Item = VarId>) -> Condition {
        Condition(vars.into_iter().filter_map(|v| self.0.get(&v).map(|c| (v, *c))).collect())
    }

    /// The parts of `o` that `self` doesn't already guarantee.
    pub fn unproven(&self, o: &Condition) -> Condition {
        Condition(
            o.0.iter().filter(|(v, c)| !self.0.get(v).is_some_and(|s| s.within(c))).map(|(v, c)| (*v, *c)).collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Span;
    use crate::vocabulary::VarKind;

    fn var(id: u32, kind: VarKind) -> Variable {
        Variable {
            id: VarId(id),
            subject: format!("v{id}"),
            key: format!("v{id}"),
            kind,
            verb: "is".into(),
            set: None,
            shows: vec![],
            span: Span::new("t", 1),
        }
    }

    fn above(v: f64) -> Constraint {
        Constraint::Range(Interval { lo: v, lo_open: true, hi: 100.0, hi_open: false })
    }

    #[test]
    fn intervals() {
        let a = Interval { lo: 55.0, lo_open: true, hi: 100.0, hi_open: false };
        let b = Interval { lo: 0.0, lo_open: false, hi: 55.0, hi_open: false };
        assert!(a.intersect(&b).is_empty());
        assert!(!a.intersect(&Interval::closed(50.0, 60.0)).is_empty());
        assert!(Interval::point(80.0).within(&a));
        assert!(!Interval::point(55.0).within(&a));
        assert!(a.within(&Interval::closed(0.0, 100.0)));
        assert_eq!(Interval::point(0.0), Interval::point(-0.0));
    }

    #[test]
    fn full_domains_are_not_stored() {
        let thr = var(0, VarKind::Number { min: 0.0, max: 100.0, unit: None });
        let mut c = Condition::new();
        c.constrain(&thr, Constraint::Range(Interval::closed(-10.0, 200.0)));
        assert!(c.is_unconstrained());
        c.constrain(&thr, above(55.0));
        assert_eq!(c.vars().count(), 1);
    }

    #[test]
    fn implication_and_overlay() {
        let phase = var(0, VarKind::Enum(vec!["held".into(), "flying".into(), "crashed".into()]));
        let thr = var(1, VarKind::Number { min: 0.0, max: 100.0, unit: None });
        let mut held = Condition::new();
        held.constrain(&phase, Constraint::OneOf(0b001));
        let mut held_hot = held.clone();
        held_hot.constrain(&thr, above(55.0));
        assert!(held_hot.implies(&held));
        assert!(!held.implies(&held_hot));
        assert!(held.overlaps(&held_hot));

        let mut flying = Condition::new();
        flying.constrain(&phase, Constraint::OneOf(0b010));
        assert!(!held.overlaps(&flying));
        assert_eq!(held.contradictions(&flying), vec![VarId(0)]);
        let after = held_hot.overlay(&flying);
        assert!(after.implies(&flying));
        assert_eq!(after.get(VarId(1)), Some(&above(55.0)));
    }
}
