//! Steps in the reader's words. A spec says what happens ("When the pilot launches the plane",
//! "Then the aircraft is flying"). A manual tells you what to do and what you'll see ("Press
//! Space. The plane leaves your hand and climbs away."), from the states' `Shows:` lines. Events and phrases say how with `Say:`, and states say how
//! to change them with `Set:`. Without those, the wording is worked out from the step itself.

use expectation_model::{Event, Keyword, Phrase, Said, StepText, Suite, VarKind, Variable};

/// Who does things in specs; docs talk to them directly instead.
const ACTORS: &[&str] = &["the pilot ", "the operator ", "the user ", "the reader ", "you ", "i "];

/// What a When line asks of the reader.
pub enum Action {
    /// Something to do: "press Space".
    Do(String),
    /// Something that happens by itself: "the plane reaches a waypoint".
    Happens(String),
}

/// "presses" -> "press", "carries" -> "carry", "throws" -> "throw".
fn imperative(verb: &str) -> String {
    let v = verb.to_string();
    if let Some(stem) = v.strip_suffix("ies") {
        return format!("{stem}y");
    }
    for ending in ["sses", "shes", "ches", "xes", "zes", "oes"] {
        if v.ends_with(ending) {
            return v[..v.len() - 2].to_string();
        }
    }
    v.strip_suffix('s').map(str::to_string).unwrap_or(v)
}

/// "the operator clicks the map" -> Do("click the map"); "the plane reaches a waypoint" ->
/// Happens(…), since nobody does it.
fn from_wording(text: &str) -> Action {
    let lower = text.to_lowercase();
    for actor in ACTORS {
        if lower.starts_with(actor) {
            let rest = &text[actor.len()..];
            let (verb, tail) = rest.split_once(' ').unwrap_or((rest, ""));
            let verb = imperative(verb);
            return Action::Do(if tail.is_empty() { verb } else { format!("{verb} {tail}") });
        }
    }
    Action::Happens(text.to_string())
}

/// The value part of a constraint: "80 %", "above 55 %", "LOITER_TURNS".
fn value(var: &Variable, c: &expectation_model::Constraint) -> String {
    let p = var.predicate(c);
    for verb in ["is ", "are ", "has ", "have "] {
        if let Some(rest) = p.strip_prefix(verb) {
            return rest.to_string();
        }
    }
    p
}

/// How to change a state: its `Set:`, or "set the throttle to 80 %".
fn set(var: &Variable, c: &expectation_model::Constraint) -> String {
    let v = value(var, c);
    let comparison = ["above", "below", "at least", "at most", "between", "more than", "fewer than", "not "]
        .iter()
        .any(|w| v.starts_with(w));
    if let Some(template) = &var.set {
        // "open the throttle to {value}" with "above 55 %": "open the throttle above 55 %".
        let template = if comparison { template.replace("to {value}", "{value}") } else { template.clone() };
        return template.replace("{value}", &v);
    }
    let numeric = matches!(var.kind, VarKind::Number { .. });
    if comparison || !numeric && v.contains(" or ") {
        format!("set {} {v}", var.subject)
    } else {
        format!("set {} to {v}", var.subject)
    }
}

/// What to do for an event: its `Say:`, or its wording as an instruction.
pub fn event(ev: &Event) -> Action {
    match &ev.say {
        Some(say) => Action::Do(say.clone()),
        None => from_wording(&ev.text),
    }
}

/// How to change a state, for the glossary: its `Set:` with "…" for the value.
pub fn how_to_set(var: &Variable) -> Option<String> {
    var.set.as_ref().map(|t| t.replace("{value}", "…"))
}

fn phrase_action(p: &Phrase) -> Action {
    match &p.say {
        Some(say) => Action::Do(say.clone()),
        None => from_wording(&p.name),
    }
}

/// What a When line asks: an instruction, or something to wait for.
pub fn action(suite: &Suite, step: &StepText) -> Action {
    let v = &suite.vocabulary;
    match step.meaning {
        Some(Said::Event(e)) => event(v.event(e)),
        Some(Said::Fact(var, c)) => Action::Do(set(v.var(var), &c)),
        Some(Said::Phrase(p)) => phrase_action(v.phrase(p)),
        _ => from_wording(&step.text),
    }
}

/// What the reader sees when a Then holds: the state's `Shows:` for its value ("the HUD says
/// CRASHED"), or else the state itself.
pub fn seen(suite: &Suite, step: &StepText) -> String {
    match step.meaning {
        Some(Said::Fact(var, c)) => suite.vocabulary.var(var).shown(&c).unwrap_or_else(|| state(suite, step)),
        Some(Said::Shift(var, s)) => suite.vocabulary.var(var).shown_shift(s).unwrap_or_else(|| state(suite, step)),
        _ => state(suite, step),
    }
}

/// What the reader sees of a state: its `Shows:`, or the state itself.
pub fn show(var: &Variable, c: &expectation_model::Constraint) -> String {
    var.shown(c).unwrap_or_else(|| var.describe(c))
}

/// A state as a clause: "the aircraft is in hand", or a setup phrase's `Say:`.
pub fn state(suite: &Suite, step: &StepText) -> String {
    let v = &suite.vocabulary;
    match step.meaning {
        Some(Said::Fact(var, c)) => v.var(var).describe(&c),
        Some(Said::Shift(var, s)) => {
            let var = v.var(var);
            let unit = match &var.kind {
                VarKind::Number { unit: Some(u), .. } => u.clone(),
                _ => String::new(),
            };
            // A count reads as a count: "the plan has 1 more item", not "goes up by 1 item".
            let counts = matches!(var.verb.as_str(), "has" | "have") && !unit.is_empty();
            match s {
                expectation_model::Shift::Same => format!("{} does not change", var.subject),
                expectation_model::Shift::By(d) => {
                    let n = expectation_model::text::number(d.abs());
                    let unit = if d.abs() == 1.0 { unit.trim_end_matches('s').to_string() } else { unit };
                    let unit = if unit.is_empty() { unit } else { format!(" {unit}") };
                    if counts {
                        let more = if d > 0.0 { "more" } else { "fewer" };
                        format!("{} {} {n} {more}{unit}", var.subject, var.verb)
                    } else {
                        format!("{} goes {} by {n}{unit}", var.subject, if d > 0.0 { "up" } else { "down" })
                    }
                }
            }
        }
        Some(Said::Phrase(p)) => {
            let p = v.phrase(p);
            p.say.clone().unwrap_or_else(|| p.name.clone())
        }
        _ => step.text.clone(),
    }
}

/// Something to have in place before you begin, as a sentence: a setup phrase's `Say:`, how
/// to set a state that has a `Set:`, or else the state itself ("The aircraft is in hand.").
pub fn setup(suite: &Suite, step: &StepText) -> String {
    let v = &suite.vocabulary;
    match step.meaning {
        Some(Said::Fact(var, c)) if v.var(var).set.is_some() => sentence(&set(v.var(var), &c)),
        _ => sentence(&state(suite, step)),
    }
}

/// What you'll see: Thens as one sentence. "The aircraft is flying and the throttle is 80 %."
pub fn outcome(suite: &Suite, thens: &[&StepText]) -> String {
    let clauses: Vec<String> = thens.iter().map(|t| seen(suite, t)).collect();
    sentences(&clauses)
}

/// One sentence per clause, as STE likes them short: "A new marker appears. The UPLOAD button
/// reads UPLOAD 3."
pub fn sentences(clauses: &[String]) -> String {
    clauses.iter().map(|c| sentence(c)).filter(|c| !c.is_empty()).collect::<Vec<_>>().join(" ")
}

/// Steps split into their Givens, Whens and Thens, each And or But with the part it continues.
pub fn parts(steps: &[StepText]) -> [Vec<&StepText>; 3] {
    let mut section = Keyword::Given;
    let mut out: [Vec<&StepText>; 3] = [vec![], vec![], vec![]];
    for s in steps {
        if !matches!(s.keyword, Keyword::And | Keyword::But) {
            section = s.keyword;
        }
        let i = match section {
            Keyword::Given => 0,
            Keyword::When => 1,
            _ => 2,
        };
        out[i].push(s);
    }
    out
}

/// The conditions of a how-to, first, as STE puts them: "If the plane is in hand and the sim
/// is paused, ". Empty without conditions.
pub fn conditions(suite: &Suite, givens: &[&StepText]) -> String {
    if givens.is_empty() {
        return String::new();
    }
    let clauses: Vec<String> = givens.iter().map(|g| state(suite, g)).collect();
    format!("If {}, ", expectation_model::text::join_and(&clauses))
}

/// "Press Space." from "press space".
pub fn sentence(s: &str) -> String {
    let s = s.trim().trim_end_matches('.');
    let mut c = s.chars();
    match c.next() {
        Some(f) => format!("{}{}.", f.to_uppercase(), c.as_str()),
        None => String::new(),
    }
}

/// Lower-case the first letter, to go after a comma.
pub fn clause(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => format!("{}{}", f.to_lowercase(), c.as_str()),
        None => String::new(),
    }
}

/// When lines as one instruction: "Set the throttle to 0 %, then press R." Lines for things
/// that just happen read "Wait until the plane reaches a waypoint".
pub fn instruction(suite: &Suite, whens: &[&StepText]) -> String {
    let mut parts: Vec<String> = vec![];
    let mut waits: Vec<String> = vec![];
    for l in whens {
        match action(suite, l) {
            Action::Do(s) => parts.push(s),
            Action::Happens(s) => waits.push(s),
        }
    }
    let mut out = sequence(&parts);
    if !waits.is_empty() {
        let wait = sentence(&format!("wait until {}", expectation_model::text::join_and(&waits)));
        out = if out.is_empty() { wait } else { format!("{out} Then {}", clause(&wait)) };
    }
    out
}

/// Actions one sentence each, as STE has them: "Press 0 to close the throttle. Then press R."
fn sequence(actions: &[String]) -> String {
    let mut out: Vec<String> = vec![];
    for (i, a) in actions.iter().enumerate() {
        out.push(if i == 0 { sentence(a) } else { sentence(&format!("then {}", clause(a))) });
    }
    out.join(" ")
}

/// An expectation as one or two sentences of a manual: "If the plane is in hand, set the
/// throttle above 55 %. The plane leaves your hand and climbs away."
pub fn how(suite: &Suite, steps: &[StepText]) -> String {
    let [givens, whens, thens] = parts(steps);
    let outcome: Vec<String> = thens.iter().map(|t| seen(suite, t)).collect();
    // The first outcome finishes a sentence that starts with a condition; the rest stand alone.
    let (first, more) = match outcome.split_first() {
        Some((f, rest)) => (f.clone(), sentences(rest)),
        None => (String::new(), String::new()),
    };
    let with_more = |s: String| if more.is_empty() { s } else { format!("{s} {more}") };
    if whens.is_empty() {
        // An invariant: it holds whenever its Given does.
        let when: Vec<String> = givens.iter().map(|g| state(suite, g)).collect();
        return with_more(sentence(&format!("whenever {}, {first}", expectation_model::text::join_and(&when))));
    }
    let lead = conditions(suite, &givens);
    let mut does = vec![];
    let mut happens = vec![];
    for w in &whens {
        match action(suite, w) {
            Action::Do(s) => does.push(s),
            Action::Happens(s) => happens.push(s),
        }
    }
    if does.is_empty() {
        // Nobody does anything: it's what follows when something happens. "If the plane is
        // flying the mission and it reaches a waypoint, …".
        let when = happens.join(" and ");
        return with_more(match lead.strip_suffix(", ") {
            Some(lead) => sentence(&format!("{lead} and {when}, {first}")),
            None => sentence(&format!("when {when}, {first}")),
        });
    }
    let doing = match does.split_first() {
        Some((first, rest)) if !lead.is_empty() => {
            let first = sentence(&format!("{lead}{}", clause(first)));
            let then: Vec<String> = rest.iter().map(|a| sentence(&format!("then {}", clause(a)))).collect();
            if then.is_empty() { first } else { format!("{first} {}", then.join(" ")) }
        }
        _ => sequence(&does),
    };
    format!("{doing} {}", sentences(&outcome))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbs_become_instructions() {
        assert_eq!(imperative("presses"), "press");
        assert_eq!(imperative("clicks"), "click");
        assert_eq!(imperative("carries"), "carry");
        assert_eq!(imperative("launches"), "launch");
        assert_eq!(imperative("goes"), "go");
        assert!(matches!(from_wording("the operator clicks the map"), Action::Do(s) if s == "click the map"));
        assert!(matches!(from_wording("the plane reaches a waypoint"), Action::Happens(_)));
    }

    #[test]
    fn sentences_start_with_a_capital() {
        assert_eq!(sentence("press Space"), "Press Space.");
        assert_eq!(clause("Press Space"), "press Space");
    }
}
