//! The suite's ubiquitous language: the states the system can be in and the events that
//! happen to it. A step only means something if it is written in these terms, which is what
//! lets two expectations be compared mechanically.

use crate::condition::{Condition, Constraint, Interval};
use crate::diagnostic::Span;
use crate::text;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VarId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId(pub u32);

/// Most values an enumerated state can have (they're stored as a bit set).
pub const MAX_VALUES: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum VarKind {
    /// One of a fixed set of named values.
    Enum(Vec<String>),
    /// A number in `min..=max`, with an optional unit such as `%`.
    Number { min: f64, max: f64, unit: Option<String> },
}

/// A state of the system: "the aircraft is in hand | flying | crashed".
#[derive(Clone, Debug)]
pub struct Variable {
    pub id: VarId,
    /// As declared, e.g. "the aircraft".
    pub subject: String,
    /// Normalised for matching, e.g. "aircraft".
    pub key: String,
    pub kind: VarKind,
    /// The verb it was declared with, which steps about it read with: "is", "are", "has"
    /// ("the plan has 0..100 items") or "have".
    pub verb: String,
    pub span: Span,
    /// How docs tell the reader to change it, with `{value}` for the value ("open the throttle
    /// to {value}"). Without it: "set the throttle to 80 %".
    pub set: Option<String>,
    /// What the reader sees, from `Shows:` lines: for one value ("crashed", "the HUD says
    /// CRASHED", or "at least 99 %" for a number), or for any value with `{value}` in it
    /// (`None`, "the button reads UPLOAD {value}").
    pub shows: Vec<(Option<String>, String)>,
}

/// Something that happens to the system: "the pilot presses reset".
#[derive(Clone, Debug)]
pub struct Event {
    pub id: EventId,
    pub text: String,
    pub key: String,
    pub span: Span,
    /// How docs tell the reader to make it happen ("press Space"). Without it, an event
    /// with an actor ("the operator clicks the map") becomes an instruction ("click the map"),
    /// and one without ("the plane reaches a waypoint") is something that happens.
    pub say: Option<String>,
}

/// A concrete value, as the system under test reports it or is driven to.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Enum(String),
    Number(f64),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Enum(s) => f.write_str(s),
            Value::Number(v) => f.write_str(&text::number(*v)),
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Enum(s.to_string())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Enum(s)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Number(v)
    }
}

impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Number(v as f64)
    }
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Enum(s) => Some(s),
            Value::Number(_) => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(v) => Some(*v),
            Value::Enum(_) => None,
        }
    }
}

impl Variable {
    /// What the reader sees when it's exactly one value, if a `Shows:` line says.
    /// What the reader sees when it changes by `shift`, if a `Shows:` line says: one keyed
    /// "goes up by 1", "goes down by 10 m" or "stays the same", as relative Thens write them.
    pub fn shown_shift(&self, shift: crate::spec::Shift) -> Option<String> {
        let keys: Vec<String> = match shift {
            crate::spec::Shift::Same => vec!["stays the same".into()],
            crate::spec::Shift::By(d) => {
                let way = if d > 0.0 { "up" } else { "down" };
                let n = text::number(d.abs());
                let unit = match &self.kind {
                    VarKind::Number { unit: Some(u), .. } => format!(" {u}"),
                    _ => String::new(),
                };
                vec![format!("goes {way} by {n}"), format!("goes {way} by {n}{unit}")]
            }
        };
        self.shows
            .iter()
            .find(|(v, _)| v.as_ref().is_some_and(|v| keys.iter().any(|k| k.eq_ignore_ascii_case(v))))
            .map(|(_, t)| t.clone())
    }

    pub fn shown(&self, c: &Constraint) -> Option<String> {
        // A line for exactly this constraint first: "crashed", or "at least 99 %" for a number.
        let said = self.predicate(c);
        let said = ["is ", "are ", "has ", "have "].iter().find_map(|v| said.strip_prefix(v)).unwrap_or(&said);
        if let Some((_, t)) = self.shows.iter().find(|(v, _)| v.as_ref().is_some_and(|v| v.eq_ignore_ascii_case(said)))
        {
            return Some(t.clone());
        }
        let value = match (&self.kind, c) {
            (VarKind::Enum(values), Constraint::OneOf(mask)) if mask.count_ones() == 1 => {
                values[mask.trailing_zeros() as usize].clone()
            }
            (VarKind::Number { .. }, Constraint::Range(r)) if r.lo == r.hi && !r.lo_open && !r.hi_open => {
                text::number(r.lo)
            }
            _ => return None,
        };
        let own = self.shows.iter().find(|(v, _)| v.as_ref().is_some_and(|v| v.eq_ignore_ascii_case(&value)));
        let any = || self.shows.iter().find(|(v, _)| v.is_none());
        own.or_else(any).map(|(_, t)| t.replace("{value}", &value))
    }

    /// The constraint that allows every value.
    pub fn full(&self) -> Constraint {
        match &self.kind {
            VarKind::Enum(values) => Constraint::OneOf(mask_all(values.len())),
            VarKind::Number { min, max, .. } => Constraint::Range(Interval::closed(*min, *max)),
        }
    }

    pub fn value_index(&self, name: &str) -> Option<usize> {
        let VarKind::Enum(values) = &self.kind else { return None };
        let k = text::normalize(name);
        values.iter().position(|v| text::normalize(v) == k)
    }

    /// A concrete value inside `c` to drive the system with: the first allowed name, or the
    /// middle of a numeric range.
    pub fn witness(&self, c: &Constraint) -> Option<Value> {
        match (&self.kind, c) {
            (VarKind::Enum(values), Constraint::OneOf(mask)) => {
                (0..values.len()).find(|i| mask & (1 << i) != 0).map(|i| Value::Enum(values[i].clone()))
            }
            (VarKind::Number { .. }, Constraint::Range(r)) if !r.is_empty() => Some(Value::Number(r.midpoint())),
            _ => None,
        }
    }

    /// Does the observed value satisfy `c`? Numbers get a tolerance of a millionth of the
    /// state's range, so a value that went through an `f32` still reads as what it was set to.
    pub fn admits(&self, c: &Constraint, v: &Value) -> bool {
        match (&self.kind, c, v) {
            (_, Constraint::OneOf(mask), Value::Enum(name)) => {
                self.value_index(name).is_some_and(|i| mask & (1 << i) != 0)
            }
            (VarKind::Number { min, max, .. }, Constraint::Range(r), Value::Number(x)) => {
                let eps = (max - min) * 1e-6;
                !r.is_empty() && (r.contains(*x) || (r.lo - eps..=r.hi + eps).contains(x))
            }
            _ => false,
        }
    }

    /// "the aircraft is flying", "the throttle is above 55 %".
    pub fn describe(&self, c: &Constraint) -> String {
        format!("{} {}", self.subject, self.predicate(c))
    }

    /// "is flying", "is not crashed", "is between 20 and 40 %" ("are …" for a plural subject).
    pub fn predicate(&self, c: &Constraint) -> String {
        let p = self.singular_predicate(c);
        let Some(rest) = p.strip_prefix("is ") else { return p };
        match self.verb.as_str() {
            "has" | "have" => {
                let rest = rest.replacen("above ", "more than ", 1).replacen("below ", "fewer than ", 1);
                match rest.strip_prefix("not ") {
                    Some(r) => format!("{} {r}", if self.verb == "has" { "doesn't have" } else { "don't have" }),
                    None => format!("{} {rest}", self.verb),
                }
            }
            "are" => format!("are {rest}"),
            _ => p,
        }
    }

    fn singular_predicate(&self, c: &Constraint) -> String {
        match (&self.kind, c) {
            (VarKind::Enum(values), Constraint::OneOf(mask)) => {
                let named = |m: u64| -> Vec<String> {
                    (0..values.len()).filter(|i| m & (1 << i) != 0).map(|i| values[i].clone()).collect()
                };
                let yes = named(*mask);
                let no = named(mask_all(values.len()) & !mask);
                if yes.is_empty() {
                    "is impossible".into()
                } else if yes.len() > 1 && no.len() == 1 {
                    format!("is not {}", no[0])
                } else {
                    format!("is {}", yes.join(" or "))
                }
            }
            (VarKind::Number { min, max, unit }, Constraint::Range(r)) => {
                let u = |v: f64| match unit {
                    // "1 item", not "1 items".
                    Some(u) if v == 1.0 && u.len() > 1 && u.ends_with('s') => format!("1 {}", &u[..u.len() - 1]),
                    Some(u) => format!("{} {u}", text::number(v)),
                    None => text::number(v),
                };
                let open_lo = r.lo <= *min && !r.lo_open;
                let open_hi = r.hi >= *max && !r.hi_open;
                if r.is_empty() {
                    "is impossible".into()
                } else if r.lo == r.hi {
                    format!("is {}", u(r.lo))
                } else if open_lo && open_hi {
                    "is anything".into()
                } else if open_lo {
                    format!("is {} {}", if r.hi_open { "below" } else { "at most" }, u(r.hi))
                } else if open_hi {
                    format!("is {} {}", if r.lo_open { "above" } else { "at least" }, u(r.lo))
                } else if !r.lo_open && !r.hi_open {
                    format!("is between {} and {}", text::number(r.lo), u(r.hi))
                } else {
                    let lo = if r.lo_open { "above" } else { "at least" };
                    let hi = if r.hi_open { "below" } else { "at most" };
                    format!("is {lo} {} and {hi} {}", u(r.lo), u(r.hi))
                }
            }
            _ => "is (mismatched constraint)".into(),
        }
    }
}

pub(crate) fn mask_all(n: usize) -> u64 {
    if n >= 64 { u64::MAX } else { (1u64 << n) - 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_compared_with_a_little_tolerance() {
        let thr = Variable {
            id: VarId(0),
            subject: "the throttle".into(),
            key: "throttle".into(),
            kind: VarKind::Number { min: 0.0, max: 100.0, unit: Some("%".into()) },
            verb: "is".into(),
            span: Span::new("t", 1),
            set: None,
            shows: vec![],
        };
        let eighty = Constraint::Range(Interval::point(80.0));
        assert!(thr.admits(&eighty, &Value::Number(0.8f32 as f64 * 100.0)));
        assert!(!thr.admits(&eighty, &Value::Number(80.01)));
    }
}

#[derive(Clone, Debug, Default)]
pub struct Vocabulary {
    pub variables: Vec<Variable>,
    pub events: Vec<Event>,
    pub phrases: Vec<crate::spec::Phrase>,
}

impl Vocabulary {
    pub fn var(&self, id: VarId) -> &Variable {
        &self.variables[id.0 as usize]
    }

    pub fn event(&self, id: EventId) -> &Event {
        &self.events[id.0 as usize]
    }

    pub fn find_var(&self, subject: &str) -> Option<&Variable> {
        let k = text::key(subject);
        self.variables.iter().find(|v| v.key == k)
    }

    pub fn phrase(&self, id: crate::spec::PhraseId) -> &crate::spec::Phrase {
        &self.phrases[id.0 as usize]
    }

    pub fn find_event(&self, phrase: &str) -> Option<&Event> {
        let k = text::key(phrase);
        self.events.iter().find(|e| e.key == k)
    }

    /// Each constraint of a condition as a phrase, in declaration order.
    pub fn facts(&self, c: &Condition) -> Vec<String> {
        c.iter().map(|(v, k)| self.var(v).describe(k)).collect()
    }

    /// A whole condition as one phrase: "the aircraft is in hand and the throttle is 0 %".
    pub fn describe(&self, c: &Condition) -> String {
        let facts = self.facts(c);
        if facts.is_empty() { "anything".into() } else { text::join_and(&facts) }
    }
}
