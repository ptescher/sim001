//! The flow DAG: how far each expectation is from the start, and what leads to it.
//!
//! Starting from the Background (and the state each story arranges), it searches breadth
//! first: in each state, take every transition that can apply and see what state it leaves.
//! An expectation's *level* is the fewest steps before it applies, and its parents are the
//! expectations that take the last step on those shortest routes. Levels only grow along
//! edges, so the result is a DAG even though the system's behaviour loops (throw, crash,
//! reset, throw again). Each expectation also gets a shortest *route*, which is a ready-made
//! test for it.

use crate::precedence::Precedence;
use crate::semantics::{perform_among, settle};
use expectation_model::{Condition, ExpId, Expectation, SectionId, Suite, VarId};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Node {
    Start,
    Exp(ExpId),
}

/// A way to reach an expectation: arrange `start`, then take each transition in `steps`.
#[derive(Clone, Debug)]
pub struct Route {
    pub start: Condition,
    pub steps: Vec<ExpId>,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub depth: u32,
    pub states: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { depth: 24, states: 5000 }
    }
}

#[derive(Debug, Default)]
pub struct Flow {
    /// Per expectation: the fewest steps from a start before it applies, if it ever does.
    pub level: Vec<Option<u32>>,
    /// (parent, child) edges of the DAG.
    pub edges: BTreeSet<(Node, ExpId)>,
    /// Per expectation: a shortest route to it.
    pub routes: Vec<Option<Route>>,
    pub states: usize,
    /// The search hit its limits, so "unreachable" isn't conclusive.
    pub truncated: bool,
}

impl Flow {
    pub fn parents(&self, e: ExpId) -> impl Iterator<Item = Node> + '_ {
        self.edges.iter().filter(move |(_, c)| *c == e).map(|(p, _)| *p)
    }

    pub fn children(&self, n: Node) -> impl Iterator<Item = ExpId> + '_ {
        self.edges.iter().filter(move |(p, _)| *p == n).map(|(_, c)| *c)
    }

    fn record(&mut self, e: ExpId, level: u32, from: Node, route: impl FnOnce() -> Route) {
        let i = e.0 as usize;
        match self.level[i] {
            None => {
                self.level[i] = Some(level);
                self.routes[i] = Some(route());
                self.edges.insert((from, e));
            }
            Some(l) if l == level => {
                self.edges.insert((from, e));
            }
            _ => {}
        }
    }
}

struct State {
    cond: Condition,
    /// The section whose root it grew from: only that part of the suite applies.
    section: SectionId,
    level: u32,
    parent: Option<usize>,
    via: Option<ExpId>,
    /// What to arrange to get here: the root's state plus whatever the route assumed.
    start: Condition,
    /// States some step on the way has changed (assumptions about them can't be arranged up front).
    touched: BTreeSet<VarId>,
}

impl State {
    fn from(&self) -> Node {
        self.via.map_or(Node::Start, Node::Exp)
    }
}

fn steps_to(states: &[State], mut i: usize) -> Vec<ExpId> {
    let mut out = vec![];
    while let (Some(p), Some(v)) = (states[i].parent, states[i].via) {
        out.push(v);
        i = p;
    }
    out.reverse();
    out
}

/// The expectations that apply in a section: everything under the same top-level section, so
/// a ground station section can use the rest of the ground station's expectations but not the
/// simulator's. In the root, everything.
pub fn in_scope(suite: &Suite, section: SectionId) -> Vec<bool> {
    let mut scope = vec![false; suite.expectations.len()];
    let chain = suite.chain(section);
    let top = chain.get(1).unwrap_or(&chain[0]).id;
    for s in suite.subtree(top) {
        for &f in &suite.section(s).features {
            for &e in &suite.feature(f).expectations {
                scope[e.0 as usize] = true;
            }
        }
    }
    scope
}

/// Search from each root state (each section's start, and each story's arranged start),
/// taking only the expectations in scope for the root's section.
pub fn explore(suite: &Suite, prec: &Precedence, roots: &[(Condition, SectionId)], limits: Limits) -> Flow {
    let n = suite.expectations.len();
    let mut flow = Flow { level: vec![None; n], routes: vec![None; n], ..Flow::default() };
    let mut states: Vec<State> = vec![];
    let mut seen: HashMap<Condition, usize> = HashMap::new();

    let mut scopes: HashMap<SectionId, Vec<bool>> = HashMap::new();
    for (root, section) in roots {
        let scope = scopes.entry(*section).or_insert_with(|| in_scope(suite, *section));
        let (cond, invariants) = settle(suite, prec, root, &[]);
        if !cond.is_satisfiable() || seen.contains_key(&cond) {
            continue;
        }
        for inv in invariants.into_iter().filter(|e| scope[e.0 as usize]) {
            flow.record(inv, 0, Node::Start, || Route { start: root.clone(), steps: vec![] });
        }
        seen.insert(cond.clone(), states.len());
        let section = *section;
        states.push(State {
            cond,
            section,
            level: 0,
            parent: None,
            via: None,
            start: root.clone(),
            touched: BTreeSet::new(),
        });
    }

    let transitions: Vec<&Expectation> = suite.transitions().collect();
    let companions = companions(&transitions);
    let mut i = 0;
    while i < states.len() {
        let scope = &scopes[&states[i].section];
        for (t, e) in transitions.iter().enumerate() {
            let s = &states[i];
            if !scope[e.id.0 as usize] || !s.cond.overlaps(&e.given) {
                continue;
            }
            // Take the part of this state where `e` applies. Assumptions about states nothing
            // has changed yet can be arranged at the start.
            let pre = s.cond.and(&e.given);
            let out = perform_among(suite, prec, &pre, &e.trigger, companions[t].iter().map(|&c| transitions[c]));
            if !out.fired.contains(&e.id) {
                continue;
            }
            let assumed = s.cond.unproven(&e.given).restrict(e.given.vars().filter(|v| !s.touched.contains(v)));
            let start = s.start.and(&assumed);
            let route = || {
                let mut steps = steps_to(&states, i);
                steps.push(e.id);
                Route { start: start.clone(), steps }
            };
            let (level, from) = (s.level, s.from());
            for &f in &out.fired {
                flow.record(f, level, from, route);
            }
            if !out.after.is_satisfiable() {
                continue;
            }
            for &inv in out.invariants.iter().filter(|e| scope[e.0 as usize]) {
                flow.record(inv, level + 1, Node::Exp(e.id), route);
            }
            if seen.contains_key(&out.after) {
                continue;
            }
            if level + 1 > limits.depth || states.len() >= limits.states {
                flow.truncated = true;
                continue;
            }
            let mut touched = states[i].touched.clone();
            touched.extend(e.trigger.changes.vars());
            for &f in &out.fired {
                touched.extend(suite.exp(f).sets());
            }
            seen.insert(out.after.clone(), states.len());
            let section = states[i].section;
            states.push(State {
                cond: out.after,
                section,
                level: level + 1,
                parent: Some(i),
                via: Some(e.id),
                start,
                touched,
            });
        }
        i += 1;
    }
    flow.states = states.len();
    flow
}

/// For each transition, the transitions its own trigger might also set off.
fn companions(transitions: &[&Expectation]) -> Vec<Vec<usize>> {
    transitions
        .iter()
        .map(|e| (0..transitions.len()).filter(|&c| e.trigger.may_fire(&transitions[c].trigger)).collect())
        .collect()
}

/// A short chain of transitions from `from` to a state where `goal` certainly holds, if one
/// is within `limits`. How a section's setup is reached from its parent's.
///
/// It only takes transitions that change something relevant: a state the goal needs changed,
/// or one some relevant transition's Given or trigger needs. Among those it expands first the
/// state with the fewest steps taken plus half the goal states still wrong, so it reaches deep
/// setups (plan a mission, upload it, arm, switch to AUTO) without trying every other button
/// on the way.
pub fn route_to(suite: &Suite, prec: &Precedence, from: &Condition, goal: &Condition, limits: Limits) -> Option<Route> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    // Relevant: the states the goal needs changed, and whatever the transitions that change
    // them need. A transition that only touches states that are already right can only undo them.
    let (start, _) = settle(suite, prec, from, &[]);
    let mut relevant: BTreeSet<VarId> =
        goal.iter().filter(|(v, g)| !start.get(*v).is_some_and(|x| x.within(g))).map(|(v, _)| v).collect();
    loop {
        let before = relevant.len();
        for e in suite.transitions() {
            if e.sets().any(|v| relevant.contains(&v)) {
                relevant.extend(e.given.vars().chain(e.trigger.changes.vars()));
            }
        }
        if relevant.len() == before {
            break;
        }
    }
    let transitions: Vec<&Expectation> =
        suite.transitions().filter(|e| e.sets().any(|v| relevant.contains(&v))).collect();
    let companions = companions(&transitions);
    let wrong = |c: &Condition| goal.iter().filter(|(v, g)| !c.get(*v).is_some_and(|x| x.within(g))).count() as u32;

    // (state, parent, via, steps taken)
    let mut states: Vec<(Condition, Option<usize>, Option<ExpId>, u32)> = vec![(start.clone(), None, None, 0)];
    let mut seen: HashMap<Condition, usize> = HashMap::from([(start.clone(), 0)]);
    let mut queue = BinaryHeap::from([(Reverse(wrong(&start)), Reverse(0usize))]);
    // Ordered by 2 × steps + wrong states, i.e. steps + wrong / 2, in whole numbers.
    while let Some((_, Reverse(i))) = queue.pop() {
        if states[i].0.implies(goal) {
            let mut steps = vec![];
            let mut at = i;
            while let (Some(p), Some(v)) = (states[at].1, states[at].2) {
                steps.push(v);
                at = p;
            }
            steps.reverse();
            // The search isn't exact: drop any step the route works without.
            let replay = |steps: &[ExpId]| {
                let mut state = start.clone();
                for &id in steps {
                    let t = transitions.iter().position(|e| e.id == id)?;
                    let e = transitions[t];
                    if !state.implies(&e.given) {
                        return None;
                    }
                    let candidates = companions[t].iter().map(|&c| transitions[c]);
                    let out = perform_among(suite, prec, &state, &e.trigger, candidates);
                    if !out.fired.contains(&e.id) || !out.uncertain.is_empty() || !out.after.is_satisfiable() {
                        return None;
                    }
                    state = out.after;
                }
                Some(state)
            };
            let mut k = 0;
            while k < steps.len() {
                let mut fewer = steps.clone();
                fewer.remove(k);
                if replay(&fewer).is_some_and(|end| end.implies(goal)) {
                    steps = fewer;
                } else {
                    k += 1;
                }
            }
            // Where the order doesn't matter, keep the suite's: the way its authors would tell
            // it (arm, then AUTO). Swap neighbours that are out of order if the route still works.
            let mut swapped = true;
            while swapped {
                swapped = false;
                for k in 1..steps.len() {
                    if steps[k] < steps[k - 1] {
                        let mut other = steps.clone();
                        other.swap(k - 1, k);
                        if replay(&other).is_some_and(|end| end.implies(goal)) {
                            steps = other;
                            swapped = true;
                        }
                    }
                }
            }
            return Some(Route { start: from.clone(), steps });
        }
        let depth = states[i].3;
        if depth >= limits.depth {
            continue;
        }
        for (t, e) in transitions.iter().enumerate() {
            // Only from states where it certainly applies: the route has to work as written.
            if states.len() >= limits.states || !states[i].0.implies(&e.given) {
                continue;
            }
            let out =
                perform_among(suite, prec, &states[i].0, &e.trigger, companions[t].iter().map(|&c| transitions[c]));
            if !out.fired.contains(&e.id) || !out.uncertain.is_empty() || !out.after.is_satisfiable() {
                continue;
            }
            if !seen.contains_key(&out.after) {
                let n = states.len();
                seen.insert(out.after.clone(), n);
                // Half the wrong goal states: most steps put right one or two.
                queue.push((Reverse(2 * (depth + 1) + wrong(&out.after)), Reverse(n)));
                states.push((out.after, Some(i), Some(e.id), depth + 1));
            }
        }
    }
    None
}
