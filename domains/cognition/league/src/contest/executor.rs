//! The executor at tag: the candidate flies seat 0 of the fast arena against the computer pilots,
//! who are the same whatever its code, so it has no opponent of its own.

use super::{Code, Contest, Group, Role};
use crate::Entry;
use crate::judge::{Reason, Summary, describe};
use cognition_arena::{Candidate, GameResult, Tactics, Tuning, play_many};
use std::path::Path;
use std::process::Command;

pub struct ExecutorContest;

impl Contest for ExecutorContest {
    fn name(&self) -> &'static str {
        "executor"
    }

    fn dir(&self) -> &'static str {
        "league"
    }

    fn groups(&self) -> Vec<Group> {
        let d = Candidate::default_executor();
        vec![
            Group {
                name: "tuning",
                names: Tuning::NAMES.to_vec(),
                bounds: Tuning::BOUNDS.to_vec(),
                defaults: d.tuning.to_array().to_vec(),
            },
            Group {
                name: "tactics",
                names: Tactics::NAMES.to_vec(),
                bounds: Tactics::BOUNDS.to_vec(),
                defaults: d.tactics.to_array().to_vec(),
            },
        ]
    }

    fn fixture(&self, name: &str) -> Option<Entry> {
        let c = match name {
            "default" => Candidate::default_executor(),
            "slow" => Candidate::slow(),
            _ => return None,
        };
        Some(entry(name, &c))
    }

    fn play(
        &self,
        entry: (&Entry, &Code),
        _: (&Entry, &Code),
        seeds: &[u64],
        target: &Path,
    ) -> Result<Vec<GameResult>, String> {
        match entry.1.program(target)? {
            None => Ok(play_many(&entry.0.candidate()?, seeds)),
            Some(bin) => {
                let wt = entry.1.dir();
                let file = wt.join(format!(".league/candidate-played-{}.json", std::process::id()));
                std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
                std::fs::write(&file, serde_json::to_string(entry.0).unwrap()).map_err(|e| e.to_string())?;
                let list: Vec<String> = seeds.iter().map(|s| s.to_string()).collect();
                let out = Command::new(bin)
                    .args(["play", "--game", "executor", "--json", "--candidate"])
                    .arg(&file)
                    .args(["--seed-list", &list.join(",")])
                    .current_dir(&wt)
                    .output()
                    .map_err(|e| e.to_string())?;
                if !out.status.success() {
                    return Err(String::from_utf8_lossy(&out.stderr).into_owned());
                }
                serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
            }
        }
    }

    fn describe(&self, s: &Summary) -> String {
        describe(s)
    }

    fn line(&self, g: &GameResult) -> String {
        format!(
            "seed {:>4}: score {:+.3}  place {}  it {:>5.1} s  tags {}  tagged {}  exits {} ({} in launch)  crashes {} ({} in launch)  by plan {:>5.1} s  envelope {:>4.1} %  closest chase {:.0} m",
            g.seed,
            g.score,
            g.place,
            g.it_time,
            g.tags,
            g.tagged,
            g.exits,
            g.launch_exits,
            g.crashes,
            g.launch_crashes,
            g.by_plan,
            100.0 * g.interventions as f32 / g.steps.max(1) as f32,
            g.closest_chase
        )
    }

    fn may_change(&self) -> &'static [&'static str] {
        &[
            "domains/cognition/executor/src/primitives.rs",
            "domains/cognition/executor/src/tuning.rs",
            "domains/cognition/agent/src/scripted.rs",
        ]
    }

    fn protected(&self) -> &'static [&'static str] {
        &["domains/cognition/executor/src/lib.rs", "domains/cognition/executor/src/math.rs", "domains/cognition/plan/"]
    }

    fn protected_reason(&self) -> Reason {
        Reason::EnvelopeChanged
    }

    fn tests(&self) -> &'static [&'static str] {
        &["-p", "cognition_executor", "-p", "cognition_agent"]
    }

    fn prompt(&self, role: Role) -> &'static str {
        match role {
            Role::Tuner => include_str!("../../prompts/tuner.md"),
            Role::Engineer => include_str!("../../prompts/engineer.md"),
        }
    }
}

/// An entry for an executor candidate.
pub fn entry(name: &str, c: &Candidate) -> Entry {
    Entry::with(name, &ExecutorContest.groups(), &[c.tuning.to_array().to_vec(), c.tactics.to_array().to_vec()])
}

impl Entry {
    /// The executor candidate it is, or why not.
    pub fn candidate(&self) -> Result<Candidate, String> {
        let v = self.check(&ExecutorContest)?;
        let tuning = Tuning::from_array(&v[0].clone().try_into().unwrap()).ok_or("the tuning is out of bounds")?;
        let tactics = Tactics::from_array(&v[1].clone().try_into().unwrap()).ok_or("the tactics are out of bounds")?;
        Ok(Candidate { tuning, tactics })
    }
}
