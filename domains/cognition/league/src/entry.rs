//! A candidate as the league keeps it, in JSON: its numbers by group and name (the executor's
//! `tuning` and `tactics`, a captain's `captain`), so that a file stays readable and survives a
//! tuning that gains or loses a number.

use crate::contest::{Contest, Group};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Unique in the league: `g003-es-2`, `default`...
    pub name: String,
    /// The commit of its code. None: the code of the league's own build.
    #[serde(default)]
    pub commit: Option<String>,
    /// The strategy that gave it, and the entry it was made from.
    #[serde(default)]
    pub strategy: String,
    #[serde(default)]
    pub parent: Option<String>,
    /// What the strategy said it did, and why.
    #[serde(default)]
    pub notes: String,
    /// Each group's numbers, by name.
    #[serde(flatten)]
    pub params: BTreeMap<String, BTreeMap<String, f32>>,
}

impl Entry {
    /// An entry with each group's numbers: `(group, values in the group's order)`.
    pub fn with(name: &str, groups: &[Group], values: &[Vec<f32>]) -> Entry {
        let params = groups
            .iter()
            .zip(values)
            .map(|(g, v)| (g.name.to_string(), g.names.iter().zip(v).map(|(n, v)| (n.to_string(), *v)).collect()))
            .collect();
        Entry { name: name.into(), commit: None, strategy: String::new(), parent: None, notes: String::new(), params }
    }

    /// A group's numbers in the group's order, or why not: a name it does not know, or a number
    /// out of bounds. A number it leaves out is the group's default.
    pub fn values(&self, g: &Group) -> Result<Vec<f32>, String> {
        let mut v = g.defaults.clone();
        for (name, x) in self.params.get(g.name).into_iter().flatten() {
            let i = g.names.iter().position(|n| n == name).ok_or(format!("no {} number called {name}", g.name))?;
            v[i] = *x;
        }
        let bad: Vec<String> = g
            .names
            .iter()
            .zip(&v)
            .zip(&g.bounds)
            .filter(|((_, x), (lo, hi))| !(x.is_finite() && **x >= *lo && **x <= *hi))
            .map(|((n, x), (lo, hi))| format!("{n} = {x} is outside {lo}..{hi}"))
            .collect();
        if bad.is_empty() { Ok(v) } else { Err(bad.join("; ")) }
    }

    /// Every group's numbers, checked.
    pub fn check(&self, contest: &dyn Contest) -> Result<Vec<Vec<f32>>, String> {
        let groups = contest.groups();
        if let Some(extra) = self.params.keys().find(|k| !groups.iter().any(|g| g.name == k.as_str())) {
            return Err(format!("no group called {extra} in the {} league", contest.name()));
        }
        groups.iter().map(|g| self.values(g)).collect()
    }

    /// The hash of its numbers and code: what the reports quote.
    pub fn hash(&self) -> String {
        let mut h: u32 = 0x811c_9dc5;
        let mut mix = |b: u8| {
            h ^= b as u32;
            h = h.wrapping_mul(0x0100_0193);
        };
        for (group, numbers) in &self.params {
            group.bytes().for_each(&mut mix);
            for (name, v) in numbers {
                name.bytes().for_each(&mut mix);
                v.to_bits().to_le_bytes().into_iter().for_each(&mut mix);
            }
        }
        self.commit.iter().flat_map(|c| c.bytes()).for_each(&mut mix);
        format!("{h:08x}")
    }

    /// One of the contest's own entries (`default`, ...), or a JSON file of an entry.
    pub fn load_for(contest: &dyn Contest, name: &str) -> Result<Entry, String> {
        if let Some(e) = contest.fixture(name) {
            return Ok(e);
        }
        let text = std::fs::read_to_string(name).map_err(|e| format!("{name}: {e}"))?;
        let e: Entry = serde_json::from_str(&text).map_err(|e| format!("{name}: {e}"))?;
        e.check(contest)?;
        Ok(e)
    }

    /// The executor league's: `default`, `slow`, or a JSON file.
    pub fn load(name: &str) -> Result<Entry, String> {
        Entry::load_for(&crate::contest::executor::ExecutorContest, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contest::executor::ExecutorContest;
    use cognition_arena::{Candidate, Tuning};

    #[test]
    fn an_entry_gives_back_its_candidate() {
        let e = Entry::load("slow").unwrap();
        assert_eq!(e.candidate(), Ok(Candidate::slow()));
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains(r#""tuning":{"#) && json.contains(r#""tactics":{"#), "{json}");
        assert_eq!(serde_json::from_str::<Entry>(&json).unwrap(), e);
    }

    #[test]
    fn a_number_left_out_is_the_default_and_a_wild_one_is_refused() {
        let mut e = Entry::load("default").unwrap();
        e.params.get_mut("tuning").unwrap().remove("k_close");
        assert_eq!(e.candidate().unwrap().tuning.k_close, Tuning::DEFAULT.k_close);
        e.params.get_mut("tuning").unwrap().insert("k_close".into(), 99.0);
        assert!(e.candidate().unwrap_err().contains("k_close"));
        e.params.get_mut("tuning").unwrap().insert("warp".into(), 1.0);
        assert!(e.candidate().is_err());
        let mut e = Entry::load("default").unwrap();
        e.params.insert("captain".into(), BTreeMap::new());
        assert!(e.check(&ExecutorContest).is_err());
    }

    #[test]
    fn the_hash_follows_the_numbers_and_the_code() {
        let a = Entry::load("default").unwrap();
        let mut b = a.clone();
        b.name = "b".into();
        assert_eq!(a.hash(), b.hash());
        b.commit = Some("abc".into());
        assert_ne!(a.hash(), b.hash());
        assert_ne!(a.hash(), Entry::load("slow").unwrap().hash());
    }
}
