//! A candidate as the league keeps it, in JSON: the numbers by name, so that a file stays readable
//! and survives a tuning that gains or loses a number.

use cognition_arena::{Candidate, Tactics, Tuning};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Unique in the league: `g003-es-2`, `default`...
    pub name: String,
    pub tuning: BTreeMap<String, f32>,
    pub tactics: BTreeMap<String, f32>,
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
}

impl Entry {
    pub fn new(name: &str, c: &Candidate) -> Entry {
        let tuning = Tuning::NAMES.iter().zip(c.tuning.to_array()).map(|(n, v)| (n.to_string(), v)).collect();
        let tactics = Tactics::NAMES.iter().zip(c.tactics.to_array()).map(|(n, v)| (n.to_string(), v)).collect();
        Entry {
            name: name.into(),
            tuning,
            tactics,
            commit: None,
            strategy: String::new(),
            parent: None,
            notes: String::new(),
        }
    }

    /// The candidate, or why not: a name it does not know, a number missing or out of bounds.
    /// A number it leaves out is the default's.
    pub fn candidate(&self) -> Result<Candidate, String> {
        let base = Candidate::default_executor();
        let mut t = base.tuning.to_array();
        let mut k = base.tactics.to_array();
        for (name, v) in &self.tuning {
            let i = Tuning::NAMES.iter().position(|n| n == name).ok_or(format!("no tuning called {name}"))?;
            t[i] = *v;
        }
        for (name, v) in &self.tactics {
            let i = Tactics::NAMES.iter().position(|n| n == name).ok_or(format!("no tactic called {name}"))?;
            k[i] = *v;
        }
        let out = |names: &[&str], a: &[f32], bounds: &[(f32, f32)]| {
            names
                .iter()
                .zip(a)
                .zip(bounds)
                .filter(|((_, v), (lo, hi))| !(v.is_finite() && **v >= *lo && **v <= *hi))
                .map(|((n, v), (lo, hi))| format!("{n} = {v} is outside {lo}..{hi}"))
                .collect::<Vec<_>>()
        };
        let mut bad = out(&Tuning::NAMES, &t, &Tuning::BOUNDS);
        bad.extend(out(&Tactics::NAMES, &k, &Tactics::BOUNDS));
        if !bad.is_empty() {
            return Err(bad.join("; "));
        }
        Ok(Candidate { tuning: Tuning::from_array(&t).unwrap(), tactics: Tactics::from_array(&k).unwrap() })
    }

    /// The hash of its numbers and code: what the reports quote.
    pub fn hash(&self) -> String {
        let c = self.candidate().map(|c| c.tuning.hash()).unwrap_or(0);
        let mut h: u32 = c;
        for b in self
            .tactics
            .values()
            .flat_map(|v| v.to_bits().to_le_bytes())
            .chain(self.commit.iter().flat_map(|s| s.bytes().collect::<Vec<_>>()))
        {
            h ^= b as u32;
            h = h.wrapping_mul(0x0100_0193);
        }
        format!("{h:08x}")
    }

    /// `default`, `slow`, or a JSON file of an entry.
    pub fn load(name: &str) -> Result<Entry, String> {
        match name {
            "default" => Ok(Entry::new("default", &Candidate::default_executor())),
            "slow" => Ok(Entry::new("slow", &Candidate::slow())),
            path => {
                let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
                serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_gives_back_its_candidate() {
        let e = Entry::new("slow", &Candidate::slow());
        assert_eq!(e.candidate(), Ok(Candidate::slow()));
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(serde_json::from_str::<Entry>(&json).unwrap(), e);
    }

    #[test]
    fn a_number_left_out_is_the_default_and_a_wild_one_is_refused() {
        let mut e = Entry::new("x", &Candidate::default_executor());
        e.tuning.remove("k_close");
        assert_eq!(e.candidate().unwrap().tuning.k_close, Tuning::DEFAULT.k_close);
        e.tuning.insert("k_close".into(), 99.0);
        assert!(e.candidate().unwrap_err().contains("k_close"));
        e.tuning.insert("warp".into(), 1.0);
        assert!(e.candidate().is_err());
    }

    #[test]
    fn the_hash_follows_the_numbers_and_the_code() {
        let a = Entry::new("a", &Candidate::default_executor());
        let mut b = a.clone();
        b.name = "b".into();
        assert_eq!(a.hash(), b.hash());
        b.commit = Some("abc".into());
        assert_ne!(a.hash(), b.hash());
        assert_ne!(a.hash(), Entry::new("s", &Candidate::slow()).hash());
    }
}
