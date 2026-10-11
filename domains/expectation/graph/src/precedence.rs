//! Exceptions. `Overrides: X` says "where we both apply, I win". Overrides must form a DAG:
//! a cycle would have two expectations each winning over the other.

use expectation_model::{Diagnostic, ExpId, Suite};

pub struct Precedence {
    /// `beats[w][l]`: `w` wins over `l`, directly or through a chain of overrides.
    beats: Vec<Vec<bool>>,
    /// The `Overrides:` lines as written: (winner, loser).
    pub declared: Vec<(ExpId, ExpId)>,
}

impl Precedence {
    pub fn beats(&self, winner: ExpId, loser: ExpId) -> bool {
        self.beats.get(winner.0 as usize).and_then(|r| r.get(loser.0 as usize)).copied().unwrap_or(false)
    }

    /// Expectations that win over `e`.
    pub fn winners_over(&self, e: ExpId) -> Vec<ExpId> {
        (0..self.beats.len()).filter(|&w| self.beats[w][e.0 as usize]).map(|w| ExpId(w as u32)).collect()
    }
}

pub fn build(suite: &Suite) -> (Precedence, Vec<Diagnostic>) {
    let n = suite.expectations.len();
    let declared: Vec<(ExpId, ExpId)> =
        suite.expectations.iter().flat_map(|e| e.overrides.iter().map(move |l| (e.id, *l))).collect();
    let mut adj = vec![vec![]; n];
    for (w, l) in &declared {
        adj[w.0 as usize].push(l.0 as usize);
    }

    let mut beats = vec![vec![false; n]; n];
    for (start, row) in beats.iter_mut().enumerate() {
        let mut stack = adj[start].clone();
        while let Some(x) = stack.pop() {
            if !row[x] {
                row[x] = true;
                stack.extend(&adj[x]);
            }
        }
    }

    let mut diags = vec![];
    let mut reported = vec![false; n];
    for start in 0..n {
        if !beats[start][start] || reported[start] {
            continue;
        }
        let cycle = find_cycle(&adj, start);
        for &c in &cycle {
            reported[c] = true;
        }
        let names: Vec<String> = cycle.iter().map(|&c| format!("«{}»", suite.expectations[c].name)).collect();
        let mut d = Diagnostic::error("override-cycle", "these expectations override each other in a circle")
            .note_that(format!("{} → {}", names.join(" → "), names[0]))
            .help("an exception has to be more specific than what it overrides; remove one of the Overrides lines");
        for &c in &cycle {
            d = d.at(&suite.expectations[c].span, "");
        }
        diags.push(d);
    }
    (Precedence { beats, declared }, diags)
}

/// The nodes of one cycle through `start`, in order.
fn find_cycle(adj: &[Vec<usize>], start: usize) -> Vec<usize> {
    fn walk(adj: &[Vec<usize>], at: usize, start: usize, path: &mut Vec<usize>, seen: &mut [bool]) -> bool {
        for &next in &adj[at] {
            if next == start {
                return true;
            }
            if !seen[next] {
                seen[next] = true;
                path.push(next);
                if walk(adj, next, start, path, seen) {
                    return true;
                }
                path.pop();
            }
        }
        false
    }
    let mut path = vec![start];
    let mut seen = vec![false; adj.len()];
    walk(adj, start, start, &mut path, &mut seen);
    path
}
