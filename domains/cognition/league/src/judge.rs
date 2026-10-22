//! Is the challenger better than the champion, and safe? Both play the same seeds; the verdict
//! rests on the paired differences in score, and on the gates.

use cognition_arena::GameResult;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    /// Better and safe: the new champion.
    Promoted,
    /// Safe, but not clearly better (or another challenger was better still).
    KeptOut,
    /// Not played, or it broke a gate: see the [`Reason`].
    Refused,
}

impl Verdict {
    pub fn label(&self) -> &'static str {
        match self {
            Verdict::Promoted => "promoted",
            Verdict::KeptOut => "kept out",
            Verdict::Refused => "refused",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reason {
    Crashed,
    MoreExits,
    MoreInterventions,
    BadInput,
    /// Its code fails the executor's or the agent's tests.
    TestsFail,
    /// Its code changes the safety envelope.
    EnvelopeChanged,
    /// Its code changes the rules, the referee or the fog of the game.
    RulesChanged,
    /// Its code changes a file a strategy may not change (or a tuner changed code at all).
    OtherFiles,
    /// Its code does not build, or its arena does not run.
    BuildFails,
    /// Its numbers are missing or out of bounds.
    Invalid,
    /// The strategy gave nothing usable.
    NoCandidate,
}

impl Reason {
    pub fn label(&self) -> &'static str {
        match self {
            Reason::Crashed => "crashed",
            Reason::MoreExits => "more exits",
            Reason::MoreInterventions => "more interventions",
            Reason::BadInput => "bad input",
            Reason::TestsFail => "tests fail",
            Reason::EnvelopeChanged => "envelope changed",
            Reason::OtherFiles => "other files changed",
            Reason::RulesChanged => "rules changed",
            Reason::BuildFails => "build fails",
            Reason::Invalid => "invalid",
            Reason::NoCandidate => "no candidate",
        }
    }
}

/// The sums of a set of games.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub games: usize,
    pub mean_score: f32,
    pub it_time: f32,
    pub tags: u32,
    pub tagged: u32,
    pub exits: u32,
    pub crashes: u32,
    pub bad_inputs: u32,
    /// Interventions per executor step.
    pub intervention_rate: f32,
    /// How often each envelope rule fired, per executor step.
    pub rules: [f32; 6],
    /// The sums of a game's own numbers (`GameResult::extra`).
    #[serde(default)]
    pub extra: std::collections::BTreeMap<String, f32>,
}

impl Summary {
    pub fn of(games: &[GameResult]) -> Summary {
        let n = games.len().max(1) as f32;
        let steps = games.iter().map(|g| g.steps).sum::<u32>().max(1) as f32;
        let mut rules = [0.0; 6];
        for g in games {
            for (r, k) in rules.iter_mut().zip(g.rules) {
                *r += k as f32 / steps;
            }
        }
        Summary {
            games: games.len(),
            mean_score: games.iter().map(|g| g.score).sum::<f32>() / n,
            it_time: games.iter().map(|g| g.it_time).sum::<f32>() / n,
            tags: games.iter().map(|g| g.tags).sum(),
            tagged: games.iter().map(|g| g.tagged).sum(),
            exits: games.iter().map(|g| g.exits).sum(),
            crashes: games.iter().map(|g| g.crashes).sum(),
            bad_inputs: games.iter().map(|g| g.bad_inputs).sum(),
            intervention_rate: games.iter().map(|g| g.interventions).sum::<u32>() as f32 / steps,
            rules,
            extra: games.iter().flat_map(|g| g.extra.iter()).fold(Default::default(), |mut m, (k, v)| {
                *m.entry(k.clone()).or_default() += v;
                m
            }),
        }
    }
}

/// The names of the envelope's rules, in the order of [`Summary::rules`].
pub const RULES: [&str; 6] = ["separation", "fence", "ceiling", "floor", "stall", "limits"];

/// A line about a set of games.
pub fn describe(s: &Summary) -> String {
    let rules: Vec<String> =
        RULES.iter().zip(s.rules).filter(|(_, r)| *r > 0.0).map(|(n, r)| format!("{n} {:.1} %", 100.0 * r)).collect();
    format!(
        "score {:+.3}, it {:.1} s, tags {}, tagged {}, exits {}, crashes {}, envelope {:.1} % ({})",
        s.mean_score,
        s.it_time,
        s.tags,
        s.tagged,
        s.exits,
        s.crashes,
        100.0 * s.intervention_rate,
        rules.join(", ")
    )
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Judgement {
    pub verdict: Verdict,
    pub reason: Option<Reason>,
    /// The mean of the paired differences (challenger less champion), and its lower confidence bound.
    pub mean: f32,
    pub lower: f32,
    pub champion: Summary,
    pub challenger: Summary,
}

impl Judgement {
    pub fn refused(reason: Reason) -> Judgement {
        Judgement {
            verdict: Verdict::Refused,
            reason: Some(reason),
            mean: 0.0,
            lower: 0.0,
            champion: Summary::default(),
            challenger: Summary::default(),
        }
    }
}

/// How many times the champion's intervention rate a challenger may have. Not much less: a chaser
/// that follows its target to the edge of the arena is turned back by the fence more often than
/// one that does not, and is better for it (the default executor against the slow one: 1.6 times).
pub const INTERVENTION_ALLOWANCE: f32 = 2.0;

/// How many more exits than the champion's are chance: two standard deviations of a Poisson
/// count as large as the champion's (at least one). Exits are rare, and each already costs score;
/// this catches an executor that leaves the arena clearly more often.
pub fn exit_allowance(champion: u32) -> f32 {
    (2.0 * (champion.max(1) as f32).sqrt()).floor().max(1.0)
}

/// Judges `challenger` against `champion`, game by game (the same seeds, in the same order),
/// with `others` more challengers judged on the same games (for the confidence: the more
/// challengers, the surer each must be).
pub fn judge(champion: &[GameResult], challenger: &[GameResult], others: usize) -> Judgement {
    let (a, b) = (Summary::of(champion), Summary::of(challenger));
    let n = champion.len().min(challenger.len());
    let diffs: Vec<f64> = champion.iter().zip(challenger).map(|(a, b)| (b.score - a.score) as f64).collect();
    let mean = diffs.iter().sum::<f64>() / n.max(1) as f64;
    let var = diffs.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / (n.max(2) - 1) as f64;
    // One-sided, 95 % shared among the challengers judged on the same games (Bonferroni).
    let z = normal_quantile(1.0 - 0.05 / others.max(1) as f64);
    let lower = mean - z * (var / n.max(1) as f64).sqrt();

    let reason = if n == 0 {
        Some(Reason::NoCandidate)
    } else if b.crashes > 0 {
        Some(Reason::Crashed)
    } else if b.bad_inputs > 0 {
        Some(Reason::BadInput)
    } else if b.exits as f32 > a.exits as f32 + exit_allowance(a.exits) {
        Some(Reason::MoreExits)
    } else if b.intervention_rate > INTERVENTION_ALLOWANCE * a.intervention_rate + 1e-6 {
        Some(Reason::MoreInterventions)
    } else {
        None
    };
    let verdict = match reason {
        Some(_) => Verdict::Refused,
        None if mean > 0.0 && lower > 0.0 => Verdict::Promoted,
        None => Verdict::KeptOut,
    };
    Judgement { verdict, reason, mean: mean as f32, lower: lower as f32, champion: a, challenger: b }
}

/// The standard normal's quantile (Acklam's rational approximation, to about 1e-9), for p in (0, 1).
#[allow(clippy::excessive_precision)] // Acklam's published coefficients, as published.
fn normal_quantile(p: f64) -> f64 {
    let p = p.clamp(1e-12, 1.0 - 1e-12);
    const A: [f64; 6] = [
        -3.969683028665376e1,
        2.209460984245205e2,
        -2.759285104469687e2,
        1.383577518672690e2,
        -3.066479806614716e1,
        2.506628277459239,
    ];
    const B: [f64; 5] =
        [-5.447609879822406e1, 1.615858368580409e2, -1.556989798598866e2, 6.680131188771972e1, -1.328068155288572e1];
    const C: [f64; 6] = [
        -7.784894002430293e-3,
        -3.223964580411365e-1,
        -2.400758277161838,
        -2.549732539343734,
        4.374664141464968,
        2.938163982698783,
    ];
    const D: [f64; 4] = [7.784695709041462e-3, 3.224671290700398e-1, 2.445134137142996, 3.754408661907416];
    let tail = |q: f64| {
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    if p < 0.02425 {
        tail((-2.0 * p.ln()).sqrt())
    } else if p > 1.0 - 0.02425 {
        -tail((-2.0 * (1.0 - p).ln()).sqrt())
    } else {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn games(scores: &[f32]) -> Vec<GameResult> {
        scores
            .iter()
            .enumerate()
            .map(|(i, &score)| GameResult {
                seed: i as u64,
                score,
                steps: 6000,
                interventions: 600,
                ..GameResult::default()
            })
            .collect()
    }

    /// Scores round `mean`, the same wobble for each seed.
    fn around(mean: f32) -> Vec<f32> {
        (0..64).map(|i| mean + ((i * 37 % 11) as f32 - 5.0) * 0.1).collect()
    }

    #[test]
    fn the_quantiles_are_the_normal_s() {
        assert!((normal_quantile(0.95) - 1.6449).abs() < 1e-3);
        assert!((normal_quantile(0.5)).abs() < 1e-9);
        assert!((normal_quantile(1.0 - 0.05 / 8.0) - 2.4977).abs() < 1e-3);
    }

    #[test]
    fn a_challenger_clearly_better_on_the_same_seeds_is_promoted() {
        let j = judge(&games(&around(-1.0)), &games(&around(-0.8)), 1);
        assert_eq!((j.verdict, j.reason), (Verdict::Promoted, None));
        assert!((j.mean - 0.2).abs() < 1e-4 && j.lower > 0.0, "{j:?}");
    }

    #[test]
    fn a_challenger_no_better_or_worse_is_kept_out() {
        assert_eq!(judge(&games(&around(-1.0)), &games(&around(-1.0)), 1).verdict, Verdict::KeptOut);
        assert_eq!(judge(&games(&around(-1.0)), &games(&around(-1.3)), 1).verdict, Verdict::KeptOut);
    }

    #[test]
    fn a_small_gain_lost_in_the_noise_is_kept_out_and_more_rivals_ask_for_more() {
        // Differences of +0.05 on average, but spread ±1.
        let champion = games(&around(-1.0));
        let noisy: Vec<f32> =
            around(-1.0).iter().enumerate().map(|(i, s)| s + 0.05 + if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
        assert_eq!(judge(&champion, &games(&noisy), 1).verdict, Verdict::KeptOut);
        // A gain that is just clear for one challenger is not for twenty.
        let clear: Vec<f32> =
            around(-1.0).iter().enumerate().map(|(i, s)| s + 0.3 + if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
        let one = judge(&champion, &games(&clear), 1);
        let twenty = judge(&champion, &games(&clear), 20);
        assert!(twenty.lower < one.lower, "{one:?} {twenty:?}");
    }

    #[test]
    fn a_crash_or_a_bad_input_refuses_whatever_the_score() {
        let mut better = games(&around(0.5));
        better[3].crashes = 1;
        let j = judge(&games(&around(-1.0)), &better, 1);
        assert_eq!((j.verdict, j.reason), (Verdict::Refused, Some(Reason::Crashed)));
        let mut better = games(&around(0.5));
        better[3].bad_inputs = 1;
        assert_eq!(judge(&games(&around(-1.0)), &better, 1).reason, Some(Reason::BadInput));
    }

    #[test]
    fn more_exits_or_leaning_on_the_envelope_refuses() {
        // Exits are rare: one more than the champion's is chance; clearly more is not.
        let mut champion = games(&around(-1.0));
        champion[0].exits = 9;
        let mut better = games(&around(0.5));
        better[0].exits = 10;
        assert_eq!(judge(&champion, &better, 1).verdict, Verdict::Promoted);
        better[0].exits = 16;
        assert_eq!(judge(&champion, &better, 1).reason, Some(Reason::MoreExits));
        let mut better = games(&around(0.5));
        better[0].exits = 3;
        assert_eq!(judge(&games(&around(-1.0)), &better, 1).reason, Some(Reason::MoreExits));
        let mut better = games(&around(0.5));
        for g in &mut better {
            g.interventions = 1300; // 22 % against the champion's 10 %
        }
        assert_eq!(judge(&games(&around(-1.0)), &better, 1).reason, Some(Reason::MoreInterventions));
        for g in &mut better {
            g.interventions = 1100; // within 2 times
        }
        assert_eq!(judge(&games(&around(-1.0)), &better, 1).verdict, Verdict::Promoted);
    }
}
