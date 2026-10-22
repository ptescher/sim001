//! The league: the loop that keeps making the executor, and Towers' captains, better
//! (plans/league.md).
//!
//! - A [`contest::Contest`] is what it improves and how it plays it: the executor at tag, a
//!   captain at Towers.
//! - An [`Entry`] is a candidate as the league keeps it: its numbers by group and name, the commit
//!   of its code (none: the code of this build), where it came from and its hash.
//! - [`judge`] compares a challenger with the champion on the same seeds: the paired difference
//!   in score, and the safety gates.
//! - A [`Strategy`] gives challengers each generation: [`strategy::Es`] (an evolution strategy),
//!   [`strategy::Claude`] (a headless Claude run, as a tuner or as an engineer in a worktree).
//! - [`League`] is the folder that holds it all, and runs the generations.

pub mod contest;
pub mod entry;
pub mod judge;
pub mod league;
pub mod strategy;

pub use entry::Entry;
pub use judge::{Judgement, Reason, Verdict, judge};
pub use league::{Generation, League, Options};
pub use strategy::{Context, Proposal, Strategy};
