//! Lines to blocks. The format is line oriented like Gherkin: a line's first word decides
//! what it is, and indentation is only for people.

use expectation_model::{Diagnostic, Keyword, Span};

#[derive(Debug, Default)]
pub struct RawFile {
    /// The file's directory, relative to the suite's: which section it belongs to.
    pub dir: String,
    /// `Suite: <name>`: what the suite calls itself.
    pub name: Option<(String, Span)>,
    /// `Section: <name>` (or `Suite:`) and the text under it: its directory's title and description.
    pub section: Option<RawSection>,
    pub states: Vec<RawDecl>,
    pub events: Vec<RawDecl>,
    pub phrases: Vec<RawPhrase>,
    pub background: Vec<RawStep>,
    pub features: Vec<RawFeature>,
    pub stories: Vec<RawStory>,
    pub terms: Vec<RawTerm>,
    /// `Platform: desktop | iOS`: the platforms the suite is run on, the first the default.
    pub platforms: Vec<(String, Span)>,
    /// `On:` under a `Section:`: the platforms the whole file is about.
    pub on: Option<Vec<(String, Span)>>,
}

/// `Term: <name>` and its definition.
#[derive(Debug)]
pub struct RawTerm {
    pub name: String,
    pub span: Span,
    pub description: Vec<String>,
}

/// A `State:` or `Event:` line, with the `Say:` or `Set:` line under it, and a state's
/// `Shows:` lines.
#[derive(Debug)]
pub struct RawDecl {
    pub text: String,
    pub span: Span,
    pub say: Option<(String, Span)>,
    pub shows: Vec<(String, Span)>,
    /// `Say[iOS]:`, `Set[iOS]:` and `Shows[iOS]:`: what differs on another platform.
    pub variants: Vec<Variant>,
}

/// A line that applies to one platform only: `Say[iOS]: tap ARM`.
#[derive(Debug)]
pub struct Variant {
    pub platform: String,
    /// Which line it is: `Say:`/`Set:` replace, `Shows:` replace all of a state's.
    pub shows: bool,
    pub text: String,
    pub span: Span,
}

/// `Phrase: <name>`, its `Say:` and its steps.
#[derive(Debug)]
pub struct RawPhrase {
    pub name: String,
    pub span: Span,
    pub say: Option<(String, Span)>,
    pub steps: Vec<RawStep>,
    /// `Phrase[iOS]:`: the version of the phrase for that platform.
    pub platform: Option<(String, Span)>,
    pub variants: Vec<Variant>,
}

#[derive(Debug)]
pub struct RawSection {
    pub name: String,
    pub span: Span,
    pub description: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RawStep {
    pub keyword: Keyword,
    pub text: String,
    pub span: Span,
}

#[derive(Debug)]
pub struct RawFeature {
    pub name: String,
    pub span: Span,
    pub on: Option<Vec<(String, Span)>>,
    pub description: Vec<String>,
    pub expectations: Vec<RawExpectation>,
}

#[derive(Debug)]
pub struct RawExpectation {
    pub name: String,
    pub span: Span,
    pub on: Option<Vec<(String, Span)>>,
    pub description: Vec<String>,
    pub steps: Vec<RawStep>,
    pub overrides: Vec<(String, Span)>,
    /// `Docs: hidden`.
    pub hidden: bool,
}

#[derive(Debug)]
pub struct RawStory {
    pub name: String,
    pub span: Span,
    pub on: Option<Vec<(String, Span)>>,
    pub narrative: Vec<String>,
    pub description: Vec<String>,
    pub scenarios: Vec<RawScenario>,
}

#[derive(Debug)]
pub struct RawScenario {
    pub name: String,
    pub span: Span,
    pub description: Vec<String>,
    pub steps: Vec<RawStep>,
}

/// What a `Say:` or `Set:` line would describe.
#[derive(Clone, Copy, PartialEq)]
enum Last {
    None,
    State,
    Event,
    Phrase,
}

#[derive(Clone, Copy, PartialEq)]
enum Block {
    None,
    Section,
    Vocabulary,
    Phrase,
    Background,
    Feature,
    Expectation,
    Story,
    Scenario,
    Term,
}

const HEADERS: &[&str] = &["Vocabulary:", "Background:", "Feature:", "Expectation:", "Story:", "Scenario:", "Term:"];

/// `Say: …` or `Say[iOS]: …`: the platform tag, if any, and the text after the colon.
fn tagged<'a>(line: &'a str, word: &str) -> Option<(Option<String>, &'a str)> {
    let rest = line.strip_prefix(word)?;
    if let Some(rest) = rest.strip_prefix(':') {
        return Some((None, rest));
    }
    let (tag, rest) = rest.strip_prefix('[')?.split_once("]:")?;
    Some((Some(tag.trim().to_string()), rest))
}

fn step_keyword(line: &str) -> Option<(Keyword, &str)> {
    let (word, rest) = line.split_once(char::is_whitespace)?;
    let k = match word {
        "Given" => Keyword::Given,
        "When" => Keyword::When,
        "Then" => Keyword::Then,
        "And" => Keyword::And,
        "But" => Keyword::But,
        _ => return None,
    };
    Some((k, rest.trim()))
}

pub fn parse(file: &str, text: &str, diags: &mut Vec<Diagnostic>) -> RawFile {
    let mut out = RawFile::default();
    let mut block = Block::None;
    let mut last = Last::None;
    // A blank line between lines of description text starts a new paragraph.
    let mut blank = false;
    for (i, raw) in text.lines().enumerate() {
        let span = Span::new(file, i as u32 + 1);
        let line = raw.trim();
        if line.is_empty() {
            blank = true;
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let paragraph = std::mem::take(&mut blank);
        let push = |desc: &mut Vec<String>, text: String| {
            if paragraph && !desc.is_empty() {
                desc.push(String::new());
            }
            desc.push(text);
        };

        if let Some((h, name)) = ["Suite:", "Section:"].iter().find_map(|h| Some((*h, line.strip_prefix(h)?.trim()))) {
            if h == "Suite:" {
                out.name = Some((name.to_string(), span.clone()));
            }
            if let Some(prev) = &out.section {
                diags.push(
                    Diagnostic::error("syntax", "a file names its section once")
                        .at(&span, "")
                        .at(&prev.span, "named here first"),
                );
            }
            out.section = Some(RawSection { name: name.to_string(), span, description: vec![] });
            block = Block::Section;
            continue;
        }
        if let Some(h) = HEADERS.iter().find(|h| line.starts_with(**h)) {
            let title = line[h.len()..].trim().to_string();
            let needs_title = !matches!(*h, "Vocabulary:" | "Background:");
            if needs_title && title.is_empty() {
                diags.push(Diagnostic::error("syntax", format!("`{h}` needs a name")).at(&span, ""));
            }
            block = match *h {
                "Vocabulary:" => Block::Vocabulary,
                "Background:" => Block::Background,
                "Feature:" => {
                    out.features.push(RawFeature { name: title, span, on: None, description: vec![], expectations: vec![] });
                    Block::Feature
                }
                "Expectation:" => match out.features.last_mut() {
                    Some(f) if matches!(block, Block::Feature | Block::Expectation) => {
                        f.expectations.push(RawExpectation {
                            name: title,
                            span,
                            on: None,
                            description: vec![],
                            steps: vec![],
                            overrides: vec![],
                            hidden: false,
                        });
                        Block::Expectation
                    }
                    _ => {
                        diags.push(
                            Diagnostic::error("syntax", "an Expectation must be inside a Feature")
                                .at(&span, "")
                                .help("add a `Feature: <name>` line above it"),
                        );
                        Block::None
                    }
                },
                "Story:" => {
                    out.stories.push(RawStory {
                        name: title,
                        span,
                        on: None,
                        narrative: vec![],
                        description: vec![],
                        scenarios: vec![],
                    });
                    Block::Story
                }
                "Term:" => {
                    out.terms.push(RawTerm { name: title, span, description: vec![] });
                    Block::Term
                }
                _ => match out.stories.last_mut() {
                    Some(s) if matches!(block, Block::Story | Block::Scenario) => {
                        s.scenarios.push(RawScenario { name: title, span, description: vec![], steps: vec![] });
                        Block::Scenario
                    }
                    _ => {
                        diags.push(
                            Diagnostic::error("syntax", "a Scenario must be inside a Story")
                                .at(&span, "")
                                .help("add a `Story: <name>` line above it"),
                        );
                        Block::None
                    }
                },
            };
            continue;
        }

        if let Some(rest) = line.strip_prefix("Platform:") {
            if block != Block::Vocabulary {
                diags.push(Diagnostic::error("syntax", "`Platform:` belongs in a Vocabulary block").at(&span, ""));
            }
            out.platforms.extend(rest.split('|').map(|p| (p.trim().to_string(), span.clone())).filter(|(p, _)| !p.is_empty()));
            continue;
        }
        if let Some((prefix, tag, rest)) = ["State", "Event", "Phrase"].iter().find_map(|p| {
            let (tag, rest) = tagged(line, p)?;
            Some((*p, tag, rest))
        }) {
            let decl = RawDecl { text: rest.trim().to_string(), span: span.clone(), say: None, shows: vec![], variants: vec![] };
            if !matches!(block, Block::Vocabulary | Block::Phrase) {
                diags.push(
                    Diagnostic::error("syntax", format!("`{prefix}:` belongs in a Vocabulary block")).at(&span, ""),
                );
            } else if prefix != "Phrase" && tag.is_some() {
                diags.push(Diagnostic::error("syntax", format!("`{prefix}:` has no platform variants: tag its `Say:` or `Shows:` lines instead")).at(&span, ""));
            } else if prefix == "State" {
                out.states.push(decl);
                last = Last::State;
                block = Block::Vocabulary;
            } else if prefix == "Event" {
                out.events.push(decl);
                last = Last::Event;
                block = Block::Vocabulary;
            } else {
                out.phrases.push(RawPhrase {
                    name: decl.text,
                    span: span.clone(),
                    say: None,
                    steps: vec![],
                    platform: tag.map(|t| (t, span)),
                    variants: vec![],
                });
                last = Last::Phrase;
                block = Block::Phrase;
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("On:") {
            let on: Vec<(String, Span)> = rest.split(',').map(|p| (p.trim().to_string(), span.clone())).filter(|(p, _)| !p.is_empty()).collect();
            let slot = match block {
                Block::Section => Some(&mut out.on),
                Block::Feature => out.features.last_mut().map(|f| &mut f.on),
                Block::Expectation => out.features.last_mut().and_then(|f| f.expectations.last_mut()).map(|e| &mut e.on),
                Block::Story => out.stories.last_mut().map(|s| &mut s.on),
                _ => None,
            };
            match slot {
                Some(slot) if !on.is_empty() => *slot = Some(on),
                Some(_) => diags.push(Diagnostic::error("syntax", "`On:` needs platforms: `On: iOS` or `On: desktop, iOS`").at(&span, "")),
                None => diags.push(
                    Diagnostic::error("syntax", "`On:` goes right under a Section, Feature, Expectation or Story")
                        .at(&span, ""),
                ),
            }
            continue;
        }

        if let Some((tag, rest)) = tagged(line, "Shows") {
            match (block, last, out.states.last_mut(), tag) {
                (Block::Vocabulary, Last::State, Some(d), None) => d.shows.push((rest.trim().to_string(), span)),
                (Block::Vocabulary, Last::State, Some(d), Some(platform)) => {
                    d.variants.push(Variant { platform, shows: true, text: rest.trim().to_string(), span })
                }
                _ => diags.push(
                    Diagnostic::error("syntax", "`Shows:` goes right under the `State:` it describes").at(&span, ""),
                ),
            }
            continue;
        }
        if let Some((prefix, tag, rest)) = ["Say", "Set"].iter().find_map(|p| {
            let (tag, rest) = tagged(line, p)?;
            Some((*p, tag, rest))
        }) {
            let said = Some((rest.trim().to_string(), span.clone()));
            let ok = matches!((block, last, prefix), (Block::Vocabulary, Last::State, "Set") | (Block::Vocabulary, Last::Event, "Say") | (Block::Phrase, _, "Say"));
            if !ok {
                diags.push(
                    Diagnostic::error("syntax", format!("`{prefix}:` goes right under the thing it describes"))
                        .at(&span, "")
                        .help("`Set:` under a `State:`, and `Say:` under an `Event:` or a `Phrase:`"),
                );
                continue;
            }
            if let Some(platform) = tag {
                let variant = Variant { platform, shows: false, text: rest.trim().to_string(), span };
                match (block, last) {
                    (Block::Phrase, _) => out.phrases.last_mut().unwrap().variants.push(variant),
                    (_, Last::State) => out.states.last_mut().unwrap().variants.push(variant),
                    _ => out.events.last_mut().unwrap().variants.push(variant),
                }
                continue;
            }
            let target = match (block, last) {
                (Block::Phrase, _) => out.phrases.last_mut().map(|p| &mut p.say),
                (_, Last::State) => out.states.last_mut().map(|d| &mut d.say),
                _ => out.events.last_mut().map(|d| &mut d.say),
            };
            if let Some(t) = target {
                *t = said;
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("Docs:") {
            match (block, out.features.last_mut().and_then(|f| f.expectations.last_mut())) {
                (Block::Expectation, Some(e)) if rest.trim() == "hidden" => e.hidden = true,
                (Block::Expectation, Some(_)) => diags.push(
                    Diagnostic::error("syntax", "`Docs:` can only say `hidden`")
                        .at(&span, "")
                        .help("`Docs: hidden` keeps an expectation out of the reading pages"),
                ),
                _ => diags.push(Diagnostic::error("syntax", "`Docs:` belongs in an Expectation").at(&span, "")),
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("Overrides:") {
            match (block, out.features.last_mut().and_then(|f| f.expectations.last_mut())) {
                (Block::Expectation, Some(e)) => e.overrides.push((rest.trim().to_string(), span)),
                _ => diags.push(Diagnostic::error("syntax", "`Overrides:` belongs in an Expectation").at(&span, "")),
            }
            continue;
        }

        // Features, sections and terms hold no steps, so a line there that starts with "When"
        // or "Then" is prose: "When PLAN is selected, each click adds a waypoint".
        let prose = matches!(block, Block::Feature | Block::Section | Block::Term);
        if let Some((keyword, text)) = step_keyword(line).filter(|_| !prose) {
            let step = RawStep { keyword, text: text.to_string(), span: span.clone() };
            match block {
                Block::Phrase => out.phrases.last_mut().unwrap().steps.push(step),
                Block::Background => out.background.push(step),
                Block::Expectation => {
                    out.features.last_mut().unwrap().expectations.last_mut().unwrap().steps.push(step)
                }
                Block::Scenario => out.stories.last_mut().unwrap().scenarios.last_mut().unwrap().steps.push(step),
                Block::Story => {
                    // Steps straight under a Story form one scenario named after it.
                    let story = out.stories.last_mut().unwrap();
                    story.scenarios.push(RawScenario {
                        name: story.name.clone(),
                        span: story.span.clone(),
                        description: vec![],
                        steps: vec![step],
                    });
                    block = Block::Scenario;
                }
                _ => diags.push(
                    Diagnostic::error("syntax", "a step must be inside a Background, Expectation, Story or Scenario")
                        .at(&span, ""),
                ),
            }
            continue;
        }

        // Free text: descriptions and narratives.
        let text = line.to_string();
        match block {
            Block::Section => push(&mut out.section.as_mut().unwrap().description, text),
            Block::Feature => push(&mut out.features.last_mut().unwrap().description, text),
            Block::Term => push(&mut out.terms.last_mut().unwrap().description, text),
            Block::Expectation => {
                let e = out.features.last_mut().unwrap().expectations.last_mut().unwrap();
                if e.steps.is_empty() {
                    diags.extend(near_keyword(&span, line));
                    push(&mut e.description, text);
                } else {
                    diags.push(unexpected(&span, line));
                }
            }
            Block::Story => {
                // The user story's own lines are the narrative; anything else introduces it.
                let story = out.stories.last_mut().unwrap();
                let narrative =
                    ["As a ", "As an ", "I want", "So that", "In order to"].iter().any(|p| line.starts_with(p));
                if narrative { story.narrative.push(text) } else { push(&mut story.description, text) }
            }
            Block::Scenario => {
                let s = out.stories.last_mut().unwrap().scenarios.last_mut().unwrap();
                if s.steps.is_empty() {
                    diags.extend(near_keyword(&span, line));
                    push(&mut s.description, text);
                } else {
                    diags.push(unexpected(&span, line));
                }
            }
            _ => diags.push(unexpected(&span, line)),
        }
    }
    out
}

/// Free text before the first step is a description, so a misspelt first step would vanish
/// into it silently. Flag lines that start with something close to a step keyword.
fn near_keyword(span: &Span, line: &str) -> Option<Diagnostic> {
    let word = line.split_whitespace().next()?;
    // A wrapped line ("the plan.") continues a sentence; only a capitalised word could be a
    // misspelt keyword.
    if !word.starts_with(|c: char| c.is_uppercase()) {
        return None;
    }
    let common = ["The", "This", "That", "Than", "They", "There", "Gives", "Wen"];
    if common.contains(&word) || step_keyword(line).is_some() {
        return None;
    }
    let k = expectation_model::text::closest(word, ["Given", "When", "Then"])?;
    Some(
        Diagnostic::warning("maybe-step", format!("this reads as description text; did you mean `{k}`?"))
            .at(span, "")
            .help("descriptions are the free text before the first step"),
    )
}

fn unexpected(span: &Span, line: &str) -> Diagnostic {
    let word = line.split_whitespace().next().unwrap_or("");
    let d = Diagnostic::error("syntax", "expected a step (Given, When, Then, And or But)").at(span, "");
    match expectation_model::text::closest(word, ["Given", "When", "Then", "And", "But"]) {
        Some(k) if k != word => d.help(format!("did you mean `{k}`?")),
        _ => d,
    }
}
