//! Step text to meaning. A step is either an event ("the pilot presses reset") or a fact about
//! a state: `<subject> is [not] <predicate>`, where the predicate is one or more of the
//! state's values ("in hand or flying") or, for numbers, a comparison ("above 55 %",
//! "at most 10", "between 20 and 40 %", "0 %"). A Then can also be relative:
//! `<subject> goes up by <n>`, `goes down by <n>` or `stays the same`.

use expectation_model::text;
use expectation_model::{
    Constraint, Diagnostic, EventId, Fact, Interval, PhraseId, Said, Shift, Span, VarId, VarKind, Variable, Vocabulary,
};

pub enum Meaning {
    Fact(Fact),
    Event(EventId),
    /// A relative Then.
    Shift(VarId, Shift),
    /// One of the suite's own phrases.
    Phrase(PhraseId),
}

impl Meaning {
    /// What docs need to know about it.
    pub fn said(&self) -> Said {
        match self {
            Meaning::Fact(f) => Said::Fact(f.var, f.constraint),
            Meaning::Event(e) => Said::Event(*e),
            Meaning::Shift(v, s) => Said::Shift(*v, *s),
            Meaning::Phrase(p) => Said::Phrase(*p),
        }
    }
}

/// Negated forms first, so " is not " isn't read as " is " + "not …".
const COPULAS: &[(&str, bool)] = &[
    (" is not ", true),
    (" are not ", true),
    (" isn't ", true),
    (" aren't ", true),
    (" doesn't have ", true),
    (" don't have ", true),
    (" is ", false),
    (" are ", false),
    (" has ", false),
    (" have ", false),
];

pub fn resolve(raw: &str, span: &Span, vocab: &Vocabulary) -> Result<Meaning, Diagnostic> {
    let k = text::key(raw);
    if let Some(e) = vocab.events.iter().find(|e| e.key == k) {
        return Ok(Meaning::Event(e.id));
    }
    if let Some(p) = vocab.phrases.iter().find(|p| p.key == k) {
        return Ok(Meaning::Phrase(p.id));
    }
    if let Some(shift) = shift(raw, &k, span, vocab) {
        return shift;
    }
    let mut best: Option<(&Variable, bool, &str)> = None;
    for v in &vocab.variables {
        let Some(rest) = k.strip_prefix(&v.key) else { continue };
        let Some((neg, pred)) = COPULAS.iter().find_map(|(c, neg)| Some((*neg, rest.strip_prefix(c)?))) else {
            continue;
        };
        if best.is_none_or(|(b, ..)| v.key.len() > b.key.len()) {
            best = Some((v, neg, pred));
        }
    }
    let Some((var, neg, pred)) = best else { return Err(unknown_step(raw, &k, span, vocab)) };
    let constraint = match &var.kind {
        VarKind::Enum(values) => enum_constraint(var, values, pred, neg),
        VarKind::Number { min, max, unit } => number_constraint(pred, neg, *min, *max, unit),
    }
    .map_err(|(msg, help)| {
        let d = Diagnostic::error("bad-value", msg).at(span, "");
        if let Some(h) = help { d.help(h) } else { d }
    })?;
    Ok(Meaning::Fact(Fact { var: var.id, constraint, span: span.clone() }))
}

/// "the plan goes up by 1", "the selected item stays the same", for the longest subject
/// it starts with.
fn shift(raw: &str, k: &str, span: &Span, vocab: &Vocabulary) -> Option<Result<Meaning, Diagnostic>> {
    let (var, rest) = vocab
        .variables
        .iter()
        .filter_map(|v| Some((v, k.strip_prefix(&v.key)?)))
        .filter(|(_, rest)| rest.starts_with(" goes ") || *rest == " stays the same")
        .max_by_key(|(v, _)| v.key.len())?;
    if rest == " stays the same" {
        return Some(Ok(Meaning::Shift(var.id, Shift::Same)));
    }
    let bad = |msg: String| Some(Err(Diagnostic::error("bad-value", msg).at(span, "")));
    let (sign, n) = if let Some(n) = rest.strip_prefix(" goes up by ") {
        (1.0, n)
    } else if let Some(n) = rest.strip_prefix(" goes down by ") {
        (-1.0, n)
    } else {
        return bad(format!("`{raw}`: expected `goes up by <n>` or `goes down by <n>`"));
    };
    let VarKind::Number { unit, .. } = &var.kind else {
        return bad(format!("{} isn't a number, so it can't go up or down", var.subject));
    };
    match parse_number(n, unit) {
        Some(v) => Some(Ok(Meaning::Shift(var.id, Shift::By(sign * v)))),
        None => bad(format!("expected a number after `goes {} by`", if sign > 0.0 { "up" } else { "down" })),
    }
}

type Problem = (String, Option<String>);

fn enum_constraint(var: &Variable, values: &[String], pred: &str, neg: bool) -> Result<Constraint, Problem> {
    let mut mask = 0u64;
    for part in pred.split(" or ") {
        let Some(i) = var.value_index(part) else {
            let help = match text::closest(&text::normalize(part), values.iter().map(|v| v.as_str())) {
                Some(c) => format!("did you mean `{c}`?"),
                None => format!("{} is one of: {}", var.subject, values.join(", ")),
            };
            return Err((format!("`{part}` isn't a value of {}", var.subject), Some(help)));
        };
        mask |= 1 << i;
    }
    let Constraint::OneOf(all) = var.full() else { unreachable!() };
    Ok(Constraint::OneOf(if neg { all & !mask } else { mask }))
}

fn number_constraint(pred: &str, neg: bool, min: f64, max: f64, unit: &Option<String>) -> Result<Constraint, Problem> {
    if neg {
        return Err((
            "a number can't be negated".into(),
            Some("say `above`, `below`, `at least` or `at most` instead".into()),
        ));
    }
    let num = |s: &str| -> Result<f64, Problem> {
        parse_number(s, unit).ok_or_else(|| {
            let expected = match unit {
                Some(u) => format!("a number, optionally followed by `{u}`"),
                None => "a number".into(),
            };
            (format!("expected {expected}, found `{s}`"), None)
        })
    };
    let range = if let Some(rest) = pred.strip_prefix("between ") {
        let (a, b) = rest.split_once(" and ").ok_or_else(|| ("expected `between <a> and <b>`".to_string(), None))?;
        Interval::closed(num(a)?, num(b)?)
    } else if let Some(v) = strip_any(pred, &["above ", "over ", "more than ", "greater than ", "higher than "]) {
        Interval { lo: num(v)?, lo_open: true, hi: max, hi_open: false }
    } else if let Some(v) = strip_any(pred, &["below ", "under ", "less than ", "fewer than ", "lower than "]) {
        Interval { lo: min, lo_open: false, hi: num(v)?, hi_open: true }
    } else if let Some(v) = pred.strip_prefix("at least ") {
        Interval { lo: num(v)?, lo_open: false, hi: max, hi_open: false }
    } else if let Some(v) = pred.strip_prefix("at most ") {
        Interval { lo: min, lo_open: false, hi: num(v)?, hi_open: false }
    } else {
        Interval::point(num(pred)?)
    };
    let clipped = range.intersect(&Interval::closed(min, max));
    if clipped.is_empty() {
        let u = unit.as_deref().map(|u| format!(" {u}")).unwrap_or_default();
        return Err((format!("no value in {}..{}{u} satisfies this", text::number(min), text::number(max)), None));
    }
    Ok(Constraint::Range(clipped))
}

fn strip_any<'a>(s: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|p| s.strip_prefix(p))
}

/// "55", "55%", "55 %", "-3.5 m".
fn parse_number(s: &str, unit: &Option<String>) -> Option<f64> {
    let s = s.trim();
    let end = s.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))).unwrap_or(s.len());
    let v: f64 = s[..end].parse().ok()?;
    let rest = s[end..].trim();
    // Units match either way round in number: "1 item" and "3 items" are both items.
    let singular = |s: &str| text::normalize(s).trim_end_matches('s').to_string();
    let unit_ok = rest.is_empty() || unit.as_deref().is_some_and(|u| singular(u) == singular(rest));
    unit_ok.then_some(v)
}

fn unknown_step(raw: &str, k: &str, span: &Span, vocab: &Vocabulary) -> Diagnostic {
    let d = Diagnostic::error("unknown-step", format!("`{raw}` isn't in the vocabulary")).at(span, "");
    let subject = COPULAS.iter().find_map(|(c, _)| k.split_once(c).map(|(s, _)| s));
    let suggestion = match subject {
        Some(s) => text::closest(s, vocab.variables.iter().map(|v| v.key.as_str()))
            .and_then(|key| vocab.variables.iter().find(|v| v.key == key))
            .map(|v| format!("did you mean `{}`? It {}", v.subject, describe_kind(v))),
        None => text::closest(k, vocab.events.iter().map(|e| e.key.as_str()))
            .and_then(|key| vocab.events.iter().find(|e| e.key == key))
            .map(|e| format!("did you mean the event `{}`?", e.text)),
    };
    d.help(suggestion.unwrap_or_else(|| "declare it in a `Vocabulary:` block, with `State:` or `Event:`".into()))
}

fn describe_kind(v: &Variable) -> String {
    match &v.kind {
        VarKind::Enum(values) => format!("is {}", values.join(" | ")),
        VarKind::Number { min, max, unit } => {
            format!(
                "is a number in {}..{}{}",
                text::number(*min),
                text::number(*max),
                unit.as_deref().map(|u| format!(" {u}")).unwrap_or_default()
            )
        }
    }
}

/// `State: <subject> is <a> | <b> | …` or `State: <subject> is <min>..<max> [unit]`.
pub fn declare_state(decl: &str, span: &Span, id: VarId) -> Result<Variable, Diagnostic> {
    let bad = |msg: &str| {
        Diagnostic::error("bad-state", msg.to_string())
            .at(span, "")
            .help("write `State: the aircraft is in hand | flying` or `State: the throttle is 0..100 %`")
    };
    let (at, len) = [" is ", " are ", " has ", " have "]
        .iter()
        .filter_map(|c| decl.find(c).map(|i| (i, c.len())))
        .min()
        .ok_or_else(|| bad("expected `<subject> is <values>`"))?;
    let subject = decl[..at].trim().to_string();
    let values = decl[at + len..].trim();
    let kind = if let Some((lo, hi)) = values.split_once("..") {
        let lo: f64 = lo.trim().parse().map_err(|_| bad("expected a number before `..`"))?;
        let hi = hi.trim();
        let end = hi.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))).unwrap_or(hi.len());
        let max: f64 = hi[..end].parse().map_err(|_| bad("expected a number after `..`"))?;
        let unit = Some(hi[end..].trim().to_string()).filter(|u| !u.is_empty());
        if lo >= max {
            return Err(bad("the range is empty"));
        }
        VarKind::Number { min: lo, max, unit }
    } else {
        let names: Vec<String> = values.split('|').map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect();
        if names.len() < 2 {
            return Err(bad("a state needs at least two values, separated by `|`"));
        }
        if names.len() > expectation_model::vocabulary::MAX_VALUES {
            return Err(bad("a state can have at most 64 values"));
        }
        for (i, n) in names.iter().enumerate() {
            if names[..i].iter().any(|m| text::normalize(m) == text::normalize(n)) {
                return Err(bad(&format!("`{n}` is listed twice")));
            }
            if n.contains(" or ") {
                return Err(bad(&format!("`{n}` contains ` or `, which steps use to list values")));
            }
        }
        VarKind::Enum(names)
    };
    let verb = decl[at + 1..at + len - 1].to_string();
    Ok(Variable { id, key: text::key(&subject), subject, kind, verb, span: span.clone(), set: None, shows: vec![] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use expectation_model::Event;

    fn vocab() -> Vocabulary {
        let span = Span::new("t", 1);
        let aircraft = declare_state("the aircraft is in hand | flying | crashed", &span, VarId(0)).ok().unwrap();
        let throttle = declare_state("the throttle is 0..100 %", &span, VarId(1)).ok().unwrap();
        let reset = Event {
            id: EventId(0),
            text: "the pilot presses reset".into(),
            key: text::key("the pilot presses reset"),
            span: span.clone(),
            say: None,
        };
        Vocabulary { variables: vec![aircraft, throttle], events: vec![reset], phrases: vec![] }
    }

    fn fact(s: &str) -> Constraint {
        match resolve(s, &Span::new("t", 1), &vocab()) {
            Ok(Meaning::Fact(f)) => f.constraint,
            Ok(Meaning::Event(_) | Meaning::Shift(..) | Meaning::Phrase(_)) => panic!("{s}: not a fact"),
            Err(d) => panic!("{s}: {}", d.message),
        }
    }

    #[test]
    fn enum_facts() {
        assert_eq!(fact("the aircraft is flying"), Constraint::OneOf(0b010));
        assert_eq!(fact("The Aircraft is In Hand"), Constraint::OneOf(0b001));
        assert_eq!(fact("the aircraft is not crashed"), Constraint::OneOf(0b011));
        assert_eq!(fact("aircraft is flying or crashed"), Constraint::OneOf(0b110));
    }

    #[test]
    fn number_facts() {
        let r = |lo, lo_open, hi, hi_open| Constraint::Range(Interval { lo, lo_open, hi, hi_open });
        assert_eq!(fact("the throttle is above 55 %"), r(55.0, true, 100.0, false));
        assert_eq!(fact("the throttle is 0%"), r(0.0, false, 0.0, false));
        assert_eq!(fact("the throttle is at most 10"), r(0.0, false, 10.0, false));
        assert_eq!(fact("the throttle is between 20 and 40 %"), r(20.0, false, 40.0, false));
    }

    #[test]
    fn has_reads_like_has() {
        let span = Span::new("t", 1);
        let plan = declare_state("the plan has 0..100 items", &span, VarId(0)).ok().unwrap();
        let v = Vocabulary { variables: vec![plan], events: vec![], phrases: vec![] };
        let c = |s: &str| match resolve(s, &span, &v) {
            Ok(Meaning::Fact(f)) => f.constraint,
            _ => panic!("{s}"),
        };
        let p = &v.variables[0];
        assert_eq!(p.describe(&c("the plan has 1 item")), "the plan has 1 item");
        assert_eq!(p.describe(&c("the plan has more than 3 items")), "the plan has more than 3 items");
        assert_eq!(p.describe(&c("the plan has at least 1 item")), "the plan has at least 1 item");
    }

    #[test]
    fn relative_thens() {
        let v = vocab();
        let span = Span::new("t", 1);
        let shift = |s: &str| match resolve(s, &span, &v) {
            Ok(Meaning::Shift(_, shift)) => Ok(shift),
            Ok(_) => panic!("{s}: not a shift"),
            Err(d) => Err(d.code),
        };
        assert_eq!(shift("the throttle goes up by 10 %"), Ok(Shift::By(10.0)));
        assert_eq!(shift("the throttle goes down by 5"), Ok(Shift::By(-5.0)));
        assert_eq!(shift("the aircraft stays the same"), Ok(Shift::Same));
        assert_eq!(shift("the aircraft goes up by 1"), Err("bad-value"));
    }

    #[test]
    fn events_and_errors() {
        let v = vocab();
        let span = Span::new("t", 1);
        assert!(matches!(resolve("The pilot presses reset.", &span, &v), Ok(Meaning::Event(EventId(0)))));
        let err = |s| resolve(s, &span, &v).err().unwrap();
        assert_eq!(err("the aircraft is landed").code, "bad-value");
        assert!(err("the aircarft is flying").help.unwrap().contains("the aircraft"));
        assert!(err("the pilot presses rest").help.unwrap().contains("reset"));
        assert_eq!(err("the throttle is above 120 %").code, "bad-value");
        assert_eq!(err("the throttle is not 5").code, "bad-value");
    }
}
