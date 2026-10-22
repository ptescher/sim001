//! What the league improves, and how it plays it: a [`Contest`]. The executor at tag
//! ([`executor::ExecutorContest`]) and a captain at Towers ([`captain::CaptainContest`]) are two;
//! the league, the judge and the strategies are the same for both.

pub mod captain;
pub mod executor;

use crate::Entry;
use crate::judge::{Reason, Summary};
use cognition_arena::GameResult;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A group of numbers a candidate has: their names, bounds and defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub name: &'static str,
    pub names: Vec<&'static str>,
    pub bounds: Vec<(f32, f32)>,
    pub defaults: Vec<f32>,
}

/// Where a candidate's code is: this build's, a worktree's (built there), or a program already
/// built from it.
#[derive(Clone, Debug, PartialEq)]
pub enum Code {
    Here,
    At(PathBuf),
    Bin(PathBuf),
}

impl Code {
    /// The program to play it with: none for this build's code.
    pub fn program(&self, target: &Path) -> Result<Option<PathBuf>, String> {
        match self {
            Code::Here => Ok(None),
            Code::At(wt) => build_in(wt, target).map(Some),
            Code::Bin(b) => Ok(Some(b.clone())),
        }
    }

    /// Where to run its program.
    pub fn dir(&self) -> PathBuf {
        match self {
            Code::At(wt) => wt.clone(),
            _ => std::env::temp_dir(),
        }
    }
}

/// What Claude does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Proposes numbers.
    Tuner,
    /// May change the code too.
    Engineer,
}

pub trait Contest: Send + Sync {
    /// `executor`, `captain`: what `--game` names.
    fn name(&self) -> &'static str;
    /// The league's folder, from the top of the repository.
    fn dir(&self) -> &'static str;
    fn groups(&self) -> Vec<Group>;
    /// Its own entries by name (`default`, and a weak one for the tests).
    fn fixture(&self, name: &str) -> Option<Entry>;
    /// `entry`'s games on `seeds`, against `opponent` where the game has one (each with its code).
    /// The record is the arena's; a game other than tag fills what it has.
    fn play(
        &self,
        entry: (&Entry, &Code),
        opponent: (&Entry, &Code),
        seeds: &[u64],
        target: &Path,
    ) -> Result<Vec<GameResult>, String>;
    /// A line about a set of games, and one about a game.
    fn describe(&self, s: &Summary) -> String;
    fn line(&self, g: &GameResult) -> String;
    /// The files a Claude engineer may change; those whose change refuses it at once (and why).
    fn may_change(&self) -> &'static [&'static str];
    fn protected(&self) -> &'static [&'static str];
    fn protected_reason(&self) -> Reason;
    /// The arguments to `cargo test --release -q` a code change must pass.
    fn tests(&self) -> &'static [&'static str];
    fn prompt(&self, role: Role) -> &'static str;
}

/// The contests there are, by `--game` name.
pub fn by_name(name: &str) -> Option<Box<dyn Contest>> {
    match name {
        "executor" => Some(Box::new(executor::ExecutorContest)),
        "captain" => Some(Box::new(captain::CaptainContest)),
        _ => None,
    }
}

/// This league's program built from the code in `wt` (into the shared `target`), copied into
/// `wt/.league/bin` so that the next build does not replace it. Built once per worktree.
pub fn build_in(wt: &Path, target: &Path) -> Result<PathBuf, String> {
    let bin = wt.join(".league/bin/cognition_league");
    if bin.exists() {
        return Ok(bin);
    }
    let out = Command::new("cargo")
        .args(["build", "--release", "-q", "-p", "cognition_league"])
        .current_dir(wt)
        .env("CARGO_TARGET_DIR", target)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    std::fs::create_dir_all(bin.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::copy(target.join("release/cognition_league"), &bin).map_err(|e| e.to_string())?;
    Ok(bin)
}
