//! The equilibrium of Towers: every formation against every other on both sides of each seed (the
//! table of the game), its Nash equilibrium, and how much of it is cycles (plans/towers-rules-v4.md,
//! section 8.1).

use recreation_towers::Rules;

/// Each formation against each other: `cells[i][j]` is the mean score of `i` over `j`, and
/// `errors[i][j]` its standard error.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub names: Vec<String>,
    pub cells: Vec<Vec<f64>>,
    pub errors: Vec<Vec<f64>>,
}

impl Table {
    /// The formations of `rules`' layout, each against each other on both sides of `seeds`.
    /// Each pair is played in a thread of its own.
    pub fn play(rules: Rules, seeds: &[u64]) -> Result<Table, String> {
        let all = cognition_towers::formations(rules);
        let n = all.len();
        let pairs: Vec<(usize, usize)> = (0..n).flat_map(|i| (i + 1..n).map(move |j| (i, j))).collect();
        let results: Vec<Result<cognition_towers::Comparison, String>> = std::thread::scope(|s| {
            let handles: Vec<_> = pairs
                .iter()
                .map(|&(i, j)| {
                    let (a, b) = (&all[i].1, &all[j].1);
                    s.spawn(move || cognition_towers::compare_captains(rules, &|| a(), &|| b(), seeds))
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("a game panicked".into()))).collect()
        });
        let mut t = Table {
            names: all.iter().map(|(name, _)| name.to_string()).collect(),
            cells: vec![vec![0.0; n]; n],
            errors: vec![vec![0.0; n]; n],
        };
        for (&(i, j), c) in pairs.iter().zip(results) {
            let c = c?;
            (t.cells[i][j], t.cells[j][i]) = (c.margin as f64, -c.margin as f64);
            (t.errors[i][j], t.errors[j][i]) = (c.error as f64, c.error as f64);
        }
        Ok(t)
    }

    /// What `cognition_league nash` prints: the table, then what it says.
    pub fn report(&self) -> String {
        let mut out = format!("{:<14}", "row vs column");
        for name in &self.names {
            out += &format!("{name:>13.12}");
        }
        out += &format!("{:>9}\n", "mean");
        let n = self.names.len();
        for (i, name) in self.names.iter().enumerate() {
            out += &format!("{name:<14.13}");
            for j in 0..n {
                out += &if i == j { format!("{:>13}", "-") } else { format!("{:>+13.2}", self.cells[i][j]) };
            }
            let mean = self.cells[i].iter().sum::<f64>() / (n.max(2) - 1) as f64;
            out += &format!("{mean:>+9.2}\n");
        }
        let mix = equilibrium(&self.cells);
        let mut order: Vec<usize> = (0..n).collect();
        // By share as printed (whole percent), and in the table's order where those are equal.
        order.sort_by_key(|&i| std::cmp::Reverse((1000.0 * mix[i]).round() as i64));
        let shares: Vec<String> = order
            .iter()
            .filter(|&&i| mix[i] >= 0.005)
            .map(|&i| format!("{} {:.0} %", self.names[i], 100.0 * mix[i]))
            .collect();
        out += &format!("equilibrium: {}\n", shares.join(", "));
        out += &format!("formations with 5 % or more: {} of {n}\n", mix.iter().filter(|&&p| p >= 0.05).count());
        out += &format!("cycle share: {:.0} %\n", 100.0 * cycle_share(&self.cells));
        let top: Vec<&str> = dominant(self, 3.0).iter().map(|&i| self.names[i].as_str()).collect();
        let top = if top.is_empty() { "none".to_string() } else { top.join(", ") };
        out += &format!("formations that beat every other at 3 standard errors: {top}\n");
        out += &format!("the best answer to the equilibrium gains {:+.3}\n", best_answer(&self.cells, &mix));
        out
    }
}

/// A Nash equilibrium of a zero-sum table: the share of each row (the mix that the row player
/// keeps, so that no column does better against it than the value of the game).
///
/// The linear program: with every cell made positive (`b = cells + shift`), the column player's
/// `max Σy` subject to `b·y ≤ 1, y ≥ 0` starts feasible at `y = 0`, so the simplex needs no first
/// phase. The row player's mix is that program's dual: the prices of its slacks, normalised.
pub fn equilibrium(cells: &[Vec<f64>]) -> Vec<f64> {
    let n = cells.len();
    if n == 0 {
        return vec![];
    }
    let low = cells.iter().flatten().cloned().fold(f64::INFINITY, f64::min);
    let shift = 1.0 - low.min(0.0);
    // Rows 0..n: the constraints [b | I | 1]. Row n: the objective, -1 for each y.
    let width = 2 * n + 1;
    let mut t = vec![vec![0.0; width]; n + 1];
    for i in 0..n {
        for j in 0..n {
            t[i][j] = cells[i][j] + shift;
        }
        t[i][n + i] = 1.0;
        t[i][2 * n] = 1.0;
    }
    t[n][..n].fill(-1.0);
    let mut basis: Vec<usize> = (n..2 * n).collect();
    const EPS: f64 = 1e-12;
    // Bland's rule: the first column that improves, and of the tied rows the one whose basic
    // variable is first. It cannot cycle.
    while let Some(col) = (0..2 * n).find(|&j| t[n][j] < -EPS) {
        let mut best: Option<(f64, usize)> = None;
        for i in 0..n {
            if t[i][col] > EPS {
                let ratio = t[i][2 * n] / t[i][col];
                let better = match best {
                    None => true,
                    Some((r, b)) => ratio < r - EPS || (ratio < r + EPS && basis[i] < basis[b]),
                };
                if better {
                    best = Some((ratio, i));
                }
            }
        }
        let Some((_, row)) = best else { break };
        let p = t[row][col];
        for v in t[row].iter_mut() {
            *v /= p;
        }
        let pivot = t[row].clone();
        for (i, r) in t.iter_mut().enumerate() {
            if i != row && r[col].abs() > 0.0 {
                let f = r[col];
                for (v, q) in r.iter_mut().zip(&pivot) {
                    *v -= f * q;
                }
            }
        }
        basis[row] = col;
    }
    let x: Vec<f64> = (0..n).map(|i| t[n][n + i].max(0.0)).collect();
    let sum: f64 = x.iter().sum();
    x.iter().map(|v| v / sum).collect()
}

/// What the best row scores against `mix`.
pub fn best_answer(cells: &[Vec<f64>], mix: &[f64]) -> f64 {
    cells.iter().map(|row| row.iter().zip(mix).map(|(c, p)| c * p).sum::<f64>()).fold(f64::NEG_INFINITY, f64::max)
}

/// Whether each row against itself is 0 and each cell the opposite of its mirror.
pub fn mirrored(cells: &[Vec<f64>]) -> bool {
    let n = cells.len();
    cells.iter().all(|r| r.len() == n)
        && (0..n).all(|i| cells[i][i] == 0.0 && (0..n).all(|j| cells[i][j] == -cells[j][i]))
}

/// How much of an antisymmetric table is cycles, from 0 (a ranking) to 1 (rock, paper, scissors):
/// the table less its best ranking (`cells[i][j] = r[i] - r[j]`), as a share of its sum of squares.
pub fn cycle_share(cells: &[Vec<f64>]) -> f64 {
    let n = cells.len() as f64;
    // The best ranking of an antisymmetric table (least squares) is each row's mean.
    let r: Vec<f64> = cells.iter().map(|row| row.iter().sum::<f64>() / n).collect();
    let (mut all, mut cycles) = (0.0, 0.0);
    for (i, row) in cells.iter().enumerate() {
        for (j, c) in row.iter().enumerate() {
            all += c * c;
            cycles += (c - (r[i] - r[j])).powi(2);
        }
    }
    if all > 0.0 { cycles / all } else { 0.0 }
}

/// The rows that beat every other row by more than `k` standard errors.
pub fn dominant(table: &Table, k: f64) -> Vec<usize> {
    let n = table.cells.len();
    (0..n)
        .filter(|&i| n > 1 && (0..n).filter(|&j| j != i).all(|j| table.cells[i][j] - k * table.errors[i][j] > 0.0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rps() -> Vec<Vec<f64>> {
        vec![vec![0.0, -1.0, 1.0], vec![1.0, 0.0, -1.0], vec![-1.0, 1.0, 0.0]]
    }

    /// `cells[i][j] = r[i] - r[j]`: a pure ranking.
    fn ranking(r: &[f64]) -> Vec<Vec<f64>> {
        r.iter().map(|a| r.iter().map(|b| a - b).collect()).collect()
    }

    fn close(a: &[f64], b: &[f64]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    /// A random antisymmetric table of `n` rows.
    fn random(n: usize, seed: u64) -> Vec<Vec<f64>> {
        let mut rng = fastrand::Rng::with_seed(seed);
        let mut c = vec![vec![0.0; n]; n];
        for (i, j) in (0..n).flat_map(|i| (i + 1..n).map(move |j| (i, j))) {
            c[i][j] = rng.f64() * 2.0 - 1.0;
            c[j][i] = -c[i][j];
        }
        c
    }

    #[test]
    fn a_table_with_a_zero_diagonal_and_opposite_mirrors_is_mirrored() {
        assert!(mirrored(&rps()));
        let mut c = rps();
        c[0][1] = 0.5;
        assert!(!mirrored(&c));
        let mut d = rps();
        d[1][1] = 0.2;
        assert!(!mirrored(&d));
    }

    #[test]
    fn the_equilibrium_of_rock_paper_scissors_is_a_third_each() {
        assert!(close(&equilibrium(&rps()), &[1.0 / 3.0; 3]), "{:?}", equilibrium(&rps()));
    }

    #[test]
    fn a_row_that_beats_every_other_is_the_whole_equilibrium() {
        assert!(close(&equilibrium(&ranking(&[0.1, 0.5, 0.2])), &[0.0, 1.0, 0.0]));
    }

    #[test]
    fn rock_paper_scissors_with_a_dud_leaves_the_dud_out() {
        // A fourth row that loses to all three.
        let mut c = rps();
        for (i, row) in c.iter_mut().enumerate() {
            row.push(if i < 3 { 0.5 } else { 0.0 });
        }
        c.push(vec![-0.5, -0.5, -0.5, 0.0]);
        let mix = equilibrium(&c);
        assert!(close(&mix, &[1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0, 0.0]), "{mix:?}");
    }

    #[test]
    fn no_row_gains_against_the_equilibrium_of_any_table() {
        for seed in 0..50 {
            let c = random(9, seed);
            let mix = equilibrium(&c);
            assert!((mix.iter().sum::<f64>() - 1.0).abs() < 1e-9, "seed {seed}: {mix:?}");
            assert!(mix.iter().all(|&p| p >= 0.0), "seed {seed}: {mix:?}");
            assert!(best_answer(&c, &mix) < 1e-9, "seed {seed}: {}", best_answer(&c, &mix));
        }
    }

    #[test]
    fn the_best_answer_is_the_best_row_against_the_mix() {
        assert!((best_answer(&rps(), &[1.0, 0.0, 0.0]) - 1.0).abs() < 1e-12, "paper beats rock");
        assert!((best_answer(&rps(), &[1.0 / 3.0; 3])).abs() < 1e-12);
    }

    #[test]
    fn rock_paper_scissors_is_all_cycles_and_a_ranking_none() {
        assert!((cycle_share(&rps()) - 1.0).abs() < 1e-9);
        assert!(cycle_share(&ranking(&[0.3, -0.2, 0.9, 0.0])).abs() < 1e-9);
        let mut mixed = ranking(&[1.0, 0.0, -1.0]);
        for (i, row) in mixed.iter_mut().enumerate() {
            for (j, c) in row.iter_mut().enumerate() {
                *c += rps()[i][j];
            }
        }
        let s = cycle_share(&mixed);
        assert!(s > 0.0 && s < 1.0, "{s}");
    }

    #[test]
    fn the_report_gives_the_table_the_equilibrium_its_support_the_cycles_and_the_dominant_rows() {
        let t = Table {
            names: vec!["rock".into(), "paper".into(), "scissors".into()],
            cells: rps(),
            errors: vec![vec![0.1; 3]; 3],
        };
        let r = t.report();
        assert!(r.contains("paper"), "{r}");
        assert!(r.contains("equilibrium: rock 33 %, paper 33 %, scissors 33 %"), "{r}");
        assert!(r.contains("formations with 5 % or more: 3 of 3"), "{r}");
        assert!(r.contains("cycle share: 100 %"), "{r}");
        assert!(r.contains("beat every other at 3 standard errors: none"), "{r}");
        assert!(r.contains("the best answer to the equilibrium gains +0.000"), "{r}");
    }

    #[test]
    fn a_row_dominates_only_when_it_beats_every_other_by_k_errors() {
        let t = Table {
            names: vec!["a".into(), "b".into(), "c".into()],
            cells: ranking(&[1.0, 0.0, -0.5]),
            errors: vec![vec![0.1; 3]; 3],
        };
        assert_eq!(dominant(&t, 3.0), vec![0]);
        let wide = Table { errors: vec![vec![0.5; 3]; 3], ..t.clone() };
        assert_eq!(dominant(&wide, 3.0), Vec::<usize>::new(), "a beats b by 1, less than 3 errors of 0.5");
        assert_eq!(dominant(&Table { cells: rps(), ..t }, 3.0), Vec::<usize>::new());
    }
}
