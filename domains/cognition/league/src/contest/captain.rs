//! A captain at Towers: the candidate plays the opponent (the champion) twice on each seed, as
//! Blue and as Red, in the fast Towers arena. Its score for the seed is the mean of its two
//! games' (+1 a win, -1 a loss, 0 a draw, plus its margin over 500); against itself it is 0.
//!
//! A captain whose code is in a worktree plays as a process of its own, built there, speaking the
//! seat's JSON lines; so a challenger with new code meets the champion's code, not its own.

use super::{Code, Contest, Group, Role};
use crate::Entry;
use crate::judge::{Reason, Summary};
use cognition_arena::GameResult;
use cognition_towers::{Captain, SeatProcess, TowersResult, play};
use recreation_towers::{Commander, CommanderTuning, Outcome, Rules, Team};
use std::path::{Path, PathBuf};

pub struct CaptainContest;

/// The tuning of a captain entry.
pub fn tuning(e: &Entry) -> Result<CommanderTuning, String> {
    let v = e.check(&CaptainContest)?.remove(0);
    CommanderTuning::from_array(&v.try_into().unwrap()).ok_or("the captain's numbers are out of bounds".into())
}

/// A team's seat for one batch of games: a commander here, or the captain's own process.
enum Seat {
    Here(CommanderTuning),
    Process(SeatProcess, CommanderTuning),
}

impl Seat {
    fn new(entry: &Entry, code: &Code, bin: &Option<PathBuf>) -> Result<Seat, String> {
        let t = tuning(entry)?;
        Ok(match bin {
            Some(bin) => Seat::Process(SeatProcess::spawn(&bin.to_string_lossy(), &["captain-seat"], &code.dir())?, t),
            None => Seat::Here(t),
        })
    }

    /// The captain for a new game as `team`.
    fn begin(&mut self, team: Team) -> Result<Box<dyn Captain + '_>, String> {
        Ok(match self {
            Seat::Here(t) => Box::new(Commander::with_tuning(Rules::default(), *t)),
            Seat::Process(p, t) => {
                p.start(team, t)?;
                Box::new(p)
            }
        })
    }
}

/// The two games of a seed: `a` as Blue against `b`, then `b` as Blue against `a`.
fn both_sides(a: &mut Seat, b: &mut Seat, seed: u64) -> Result<(TowersResult, TowersResult), String> {
    let first = play(a.begin(Team::Blue)?.as_mut(), b.begin(Team::Red)?.as_mut(), seed)?;
    let second = play(b.begin(Team::Blue)?.as_mut(), a.begin(Team::Red)?.as_mut(), seed)?;
    Ok((first, second))
}

/// The record of a seed for `a`.
fn record(seed: u64, first: &TowersResult, second: &TowersResult) -> GameResult {
    let win = Rules::default().win_score;
    let (s1, s2) = (first.score_for(Team::Blue, win), second.score_for(Team::Red, win));
    let mut g = GameResult {
        seed,
        score: 0.5 * (s1 + s2),
        length: 0.5 * (first.length + second.length),
        players: 6,
        ..GameResult::default()
    };
    for (r, team) in [(first, Team::Blue), (second, Team::Red)] {
        let key = match r.outcome {
            Outcome::Won(t) if t == team => "won",
            Outcome::Won(_) => "lost",
            Outcome::Draw => "drawn",
        };
        *g.extra.entry(key.into()).or_default() += 1.0;
        *g.extra.entry("points".into()).or_default() += r.score[team.index()];
        *g.extra.entry("their points".into()).or_default() += r.score[team.other().index()];
    }
    g
}

impl Contest for CaptainContest {
    fn name(&self) -> &'static str {
        "captain"
    }

    fn dir(&self) -> &'static str {
        "league/captain"
    }

    fn groups(&self) -> Vec<Group> {
        vec![Group {
            name: "captain",
            names: CommanderTuning::NAMES.to_vec(),
            bounds: CommanderTuning::BOUNDS.to_vec(),
            defaults: CommanderTuning::DEFAULT.to_array().to_vec(),
        }]
    }

    fn fixture(&self, name: &str) -> Option<Entry> {
        let t = match name {
            "default" => CommanderTuning::DEFAULT,
            // Gives a fight up at once, stays away for five minutes, and never defends.
            "timid" => {
                CommanderTuning { give_up: 1.0, stay_away: 300.0, defend_range: 0.0, ..CommanderTuning::DEFAULT }
            }
            _ => return None,
        };
        Some(Entry::with(name, &self.groups(), &[t.to_array().to_vec()]))
    }

    fn play(
        &self,
        entry: (&Entry, &Code),
        opponent: (&Entry, &Code),
        seeds: &[u64],
        target: &Path,
    ) -> Result<Vec<GameResult>, String> {
        let (bin_a, bin_b) = (entry.1.program(target)?, opponent.1.program(target)?);
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        let chunk = seeds.len().div_ceil(cores).max(1);
        let results: Vec<Result<Vec<GameResult>, String>> = std::thread::scope(|scope| {
            let handles: Vec<_> = seeds
                .chunks(chunk)
                .map(|seeds| {
                    let (bin_a, bin_b) = (&bin_a, &bin_b);
                    scope.spawn(move || -> Result<Vec<GameResult>, String> {
                        let mut a = Seat::new(entry.0, entry.1, bin_a)?;
                        let mut b = Seat::new(opponent.0, opponent.1, bin_b)?;
                        seeds.iter().map(|&s| both_sides(&mut a, &mut b, s).map(|(x, y)| record(s, &x, &y))).collect()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("a game panicked".into()))).collect()
        });
        Ok(results.into_iter().collect::<Result<Vec<_>, _>>()?.into_iter().flatten().collect())
    }

    fn describe(&self, s: &Summary) -> String {
        let n = s.games.max(1) as f32;
        let x = |k: &str| s.extra.get(k).copied().unwrap_or(0.0);
        format!(
            "score {:+.3}; of its {} games {:.0} won, {:.0} drawn, {:.0} lost; {:.0} points a game against {:.0}",
            s.mean_score,
            2 * s.games,
            x("won"),
            x("drawn"),
            x("lost"),
            x("points") / (2.0 * n),
            x("their points") / (2.0 * n)
        )
    }

    fn line(&self, g: &GameResult) -> String {
        let x = |k: &str| g.extra.get(k).copied().unwrap_or(0.0);
        format!(
            "seed {:>4}: score {:+.3}  won {:.0} drawn {:.0} lost {:.0}  points {:.0} against {:.0} (two games)",
            g.seed,
            g.score,
            x("won"),
            x("drawn"),
            x("lost"),
            x("points"),
            x("their points")
        )
    }

    fn may_change(&self) -> &'static [&'static str] {
        &["domains/recreation/towers/src/commander.rs"]
    }

    fn protected(&self) -> &'static [&'static str] {
        &["domains/recreation/towers/", "domains/recreation/towers_server/", "domains/cognition/towers/"]
    }

    fn protected_reason(&self) -> Reason {
        Reason::RulesChanged
    }

    fn tests(&self) -> &'static [&'static str] {
        // The rules' and the arena's tests; not the commander's own, which pin today's captain.
        &["-p", "recreation_towers", "-p", "cognition_towers", "--lib", "--", "--skip", "commander::"]
    }

    fn prompt(&self, role: Role) -> &'static str {
        match role {
            Role::Tuner => include_str!("../../prompts/captain-tuner.md"),
            Role::Engineer => include_str!("../../prompts/captain-engineer.md"),
        }
    }
}
