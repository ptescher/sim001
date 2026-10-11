//! Blocks to model: build the vocabulary from every file, then resolve each step against it
//! and check the shape of each expectation and story.

use crate::parse::{RawExpectation, RawFile, RawPhrase, RawScenario, RawSection, RawStep};
use crate::step::{self, Meaning};
use expectation_model::text;
use expectation_model::{
    Condition, Diagnostic, Event, EventId, ExpId, Expectation, Fact, Feature, FeatureId, Keyword, Phrase, PhraseBody,
    PhraseId, Said, Scenario, Section, SectionId, Shift, Span, StepText, Story, StoryId, StoryStep, Suite, Trigger,
    VarId, Vocabulary,
};

pub fn resolve(suite: &mut Suite, files: &[RawFile], diags: &mut Vec<Diagnostic>) {
    let mut named: Option<&(String, Span)> = None;
    for f in files {
        if let Some(n) = &f.name {
            match named {
                Some(prev) if prev.0 != n.0 => diags.push(
                    Diagnostic::warning("two-names", format!("the suite is already called «{}»", prev.0))
                        .at(&n.1, "")
                        .at(&prev.1, "named here first"),
                ),
                Some(_) => {}
                None => {
                    suite.name = n.0.clone();
                    named = Some(n);
                }
            }
        }
    }
    suite.vocabulary = vocabulary(files, diags);
    for f in files {
        for t in &f.terms {
            if suite.glossary.iter().any(|g| g.name.eq_ignore_ascii_case(&t.name)) {
                diags.push(Diagnostic::error("duplicate", format!("«{}» is already defined", t.name)).at(&t.span, ""));
                continue;
            }
            suite.glossary.push(expectation_model::Term {
                name: t.name.clone(),
                slug: text::slug(&t.name),
                description: t.description.clone(),
                span: t.span.clone(),
            });
        }
    }
    sections(suite, files, diags);
    let vocab = &suite.vocabulary;
    let dirs: Vec<(String, SectionId)> = suite.sections.iter().map(|s| (s.path.clone(), s.id)).collect();
    let section_of = |dir: &str| dirs.iter().find(|(d, _)| d == dir).map(|(_, id)| *id).unwrap();

    // Features and their expectations.
    let mut overrides: Vec<(ExpId, Vec<(String, Span)>)> = vec![];
    for f in files {
        for rf in &f.features {
            let fid = FeatureId(suite.features.len() as u32);
            let mut feature = Feature {
                id: fid,
                section: section_of(&f.dir),
                name: rf.name.clone(),
                slug: text::slug(&rf.name),
                description: rf.description.clone(),
                span: rf.span.clone(),
                expectations: vec![],
            };
            for re in &rf.expectations {
                let id = ExpId(suite.expectations.len() as u32);
                if let Some(prev) = suite.expectations.iter().find(|e| text::key(&e.name) == text::key(&re.name)) {
                    diags.push(
                        Diagnostic::error("duplicate", format!("there is already an expectation called «{}»", re.name))
                            .at(&re.span, "")
                            .at(&prev.span, "first defined here"),
                    );
                }
                suite.expectations.push(expectation(id, fid, re, vocab, diags));
                feature.expectations.push(id);
                overrides.push((id, re.overrides.clone()));
            }
            suite.features.push(feature);
        }
    }
    for f in 0..suite.features.len() {
        let section = suite.features[f].section;
        suite.sections[section.0 as usize].features.push(FeatureId(f as u32));
    }
    for (id, names) in overrides {
        for (name, span) in names {
            let k = text::key(&name);
            match suite.expectations.iter().find(|e| text::key(&e.name) == k) {
                Some(target) if target.id == id => {
                    diags.push(Diagnostic::error("bad-override", "an expectation can't override itself").at(&span, ""))
                }
                Some(target) => {
                    let t = target.id;
                    suite.expectations[id.0 as usize].overrides.push(t);
                }
                None => {
                    let d = Diagnostic::error("bad-override", format!("there is no expectation called «{name}»"))
                        .at(&span, "");
                    let names: Vec<String> = suite.expectations.iter().map(|e| text::key(&e.name)).collect();
                    let d = match text::closest(&k, names.iter().map(|s| s.as_str())) {
                        Some(c) => {
                            let e = suite.expectations.iter().find(|e| text::key(&e.name) == c).unwrap();
                            d.help(format!("did you mean «{}»?", e.name))
                        }
                        None => d,
                    };
                    diags.push(d);
                }
            }
        }
    }

    // Stories.
    for f in files {
        for rs in &f.stories {
            let id = StoryId(suite.stories.len() as u32);
            let scenarios = rs.scenarios.iter().filter_map(|sc| scenario(sc, vocab, diags)).collect::<Vec<_>>();
            if rs.scenarios.is_empty() {
                diags.push(
                    Diagnostic::error("empty-story", format!("story «{}» has no steps", rs.name)).at(&rs.span, ""),
                );
            }
            let section = section_of(&f.dir);
            suite.sections[section.0 as usize].stories.push(id);
            suite.stories.push(Story {
                id,
                section,
                name: rs.name.clone(),
                slug: text::slug(&rs.name),
                narrative: rs.narrative.clone(),
                description: rs.description.clone(),
                span: rs.span.clone(),
                scenarios,
            });
        }
    }
}

/// One section per directory with files in it, and per directory above those. Each takes its
/// name and description from a `Section:` line in one of its files (the root: the suite's
/// name), and its setup from its files' Backgrounds, on top of its parent's.
fn sections(suite: &mut Suite, files: &[RawFile], diags: &mut Vec<Diagnostic>) {
    let mut dirs: Vec<String> = vec![String::new()];
    for f in files {
        let mut d = f.dir.clone();
        while !d.is_empty() {
            if !dirs.contains(&d) {
                dirs.push(d.clone());
            }
            d.truncate(d.rfind('/').unwrap_or(0));
        }
    }
    // Parents before children, siblings in name order.
    dirs.sort_by(|a, b| a.split('/').cmp(b.split('/')));

    for dir in dirs {
        let id = SectionId(suite.sections.len() as u32);
        let (parent, leaf) = match dir.rfind('/') {
            _ if dir.is_empty() => (None, ""),
            Some(i) => (Some(&dir[..i]), &dir[i + 1..]),
            None => (Some(""), dir.as_str()),
        };
        let parent = parent.map(|p| suite.sections.iter().find(|s| s.path == p).unwrap().id);
        let mut header: Option<&RawSection> = None;
        for f in files.iter().filter(|f| f.dir == dir) {
            if let (Some(h), Some(prev)) = (&f.section, header) {
                diags.push(
                    Diagnostic::error("two-names", format!("this section is already called «{}»", prev.name))
                        .at(&h.span, "")
                        .at(&prev.span, "named here first"),
                );
            } else if f.section.is_some() {
                header = f.section.as_ref();
            }
        }
        let name = match (header, parent) {
            (Some(h), _) => h.name.clone(),
            (None, None) => suite.name.clone(),
            (None, Some(_)) => title_of(leaf),
        };
        let slug = match parent {
            None => String::new(),
            Some(p) if suite.section(p).slug.is_empty() => text::slug(&title_of(leaf)),
            Some(p) => format!("{}--{}", suite.section(p).slug, text::slug(&title_of(leaf))),
        };

        let what = format!("the Background of «{name}» contradicts itself");
        let mut setup = Condition::new();
        let mut setup_steps = vec![];
        for f in files.iter().filter(|f| f.dir == dir) {
            for s in &f.background {
                if !matches!(s.keyword, Keyword::Given | Keyword::And | Keyword::But) {
                    diags.push(Diagnostic::error("syntax", "a Background only has Given steps").at(&s.span, ""));
                    continue;
                }
                for fact in facts_of(s, &suite.vocabulary, diags) {
                    add(&mut setup, &fact, &suite.vocabulary, diags, &what);
                }
                setup_steps.push(text_of(s, &suite.vocabulary));
            }
        }
        let start = match parent {
            Some(p) => suite.section(p).start.overlay(&setup),
            None => setup.clone(),
        };
        if let Some(p) = parent {
            suite.sections[p.0 as usize].children.push(id);
        }
        suite.sections.push(Section {
            id,
            parent,
            name,
            slug,
            path: dir.clone(),
            description: header.map(|h| h.description.clone()).unwrap_or_default(),
            setup,
            setup_steps,
            start,
            span: header.map(|h| h.span.clone()),
            children: vec![],
            features: vec![],
            stories: vec![],
        });
    }
    suite.background = suite.sections[0].start.clone();
    suite.background_steps = suite.sections[0].setup_steps.clone();
}

/// "02-flight_modes" -> "Flight modes": a directory's name, without an ordering prefix.
fn title_of(dir: &str) -> String {
    let rest = dir.trim_start_matches(|c: char| c.is_ascii_digit());
    let rest = if rest.len() < dir.len() { rest.trim_start_matches(['-', '_', ' ']) } else { dir };
    let words = rest.replace(['-', '_'], " ");
    let mut c = words.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// A `Shows:` line: "crashed: the HUD says CRASHED" for one value of a named state, or a
/// line with `{value}` in it for any value.
fn shows(
    var: &expectation_model::Variable,
    line: &str,
    span: &Span,
    diags: &mut Vec<Diagnostic>,
) -> (Option<String>, String) {
    if let Some((value, text)) = line.split_once(':') {
        let value = value.trim();
        // A change, as a relative Then writes it: "goes up by 1", "stays the same".
        if ["goes up by ", "goes down by ", "stays the same"].iter().any(|w| value.starts_with(w)) {
            return (Some(value.to_string()), text.trim().to_string());
        }
        match &var.kind {
            expectation_model::VarKind::Enum(values) => {
                if let Some(v) = values.iter().find(|v| v.eq_ignore_ascii_case(value)) {
                    return (Some(v.clone()), text.trim().to_string());
                }
            }
            // "80 %", "at least 99 %": a value or comparison as steps write it.
            expectation_model::VarKind::Number { .. } => {
                let comparison = ["above", "below", "at least", "at most", "between", "more than", "fewer than"];
                if value.starts_with(|c: char| c.is_ascii_digit()) || comparison.iter().any(|w| value.starts_with(w)) {
                    return (Some(value.to_string()), text.trim().to_string());
                }
            }
        }
    }
    if !line.contains("{value}") {
        diags.push(
            Diagnostic::warning(
                "shows-what",
                format!("this `Shows:` doesn't say which value of {} it's for", var.subject),
            )
            .at(span, "")
            .help("start it with the value and a colon (`crashed: …`), or use `{value}` for every value"),
        );
    }
    (None, line.to_string())
}

fn vocabulary(files: &[RawFile], diags: &mut Vec<Diagnostic>) -> Vocabulary {
    let mut vocab = Vocabulary::default();
    for f in files {
        for d in &f.states {
            let id = VarId(vocab.variables.len() as u32);
            let Ok(mut var) = step::declare_state(&d.text, &d.span, id).map_err(|e| diags.push(e)) else { continue };
            var.set = d.say.as_ref().map(|(s, _)| s.clone());
            var.shows = d.shows.iter().map(|(t, span)| shows(&var, t, span, diags)).collect();
            // A state can be declared in more than one section; a `Set:` on any of them counts.
            match vocab.variables.iter_mut().find(|v| v.key == var.key) {
                Some(prev) if prev.kind == var.kind => {
                    if prev.set.is_none() {
                        prev.set = var.set;
                    }
                    prev.shows.extend(var.shows);
                }
                Some(prev) => diags.push(
                    Diagnostic::error("bad-state", format!("{} is declared twice, differently", var.subject))
                        .at(&d.span, "")
                        .at(&prev.span, "first declared here"),
                ),
                None => vocab.variables.push(var),
            }
        }
        for d in &f.events {
            let key = text::key(&d.text);
            if key.is_empty() {
                diags.push(Diagnostic::error("bad-event", "an event needs a name").at(&d.span, ""));
            } else if let Some(prev) = vocab.events.iter_mut().find(|e| e.key == key) {
                if prev.say.is_none() {
                    prev.say = d.say.as_ref().map(|(s, _)| s.clone());
                }
            } else {
                let id = EventId(vocab.events.len() as u32);
                let say = d.say.as_ref().map(|(s, _)| s.clone());
                vocab.events.push(Event { id, text: d.text.clone(), key, span: d.span.clone(), say });
            }
        }
    }
    // Phrases after every state and event; each can use the phrases declared before it.
    for f in files {
        for rp in &f.phrases {
            let key = text::key(&rp.name);
            if vocab.phrases.iter().any(|p| p.key == key) || vocab.events.iter().any(|e| e.key == key) {
                diags.push(
                    Diagnostic::error("duplicate", format!("«{}» is already declared", rp.name)).at(&rp.span, ""),
                );
                continue;
            }
            let Some(body) = phrase_body(rp, &vocab, diags) else { continue };
            let id = PhraseId(vocab.phrases.len() as u32);
            let say = rp.say.as_ref().map(|(s, _)| s.clone());
            vocab.phrases.push(Phrase { id, name: rp.name.clone(), key, say, span: rp.span.clone(), body });
        }
    }
    vocab
}

/// A phrase is a setup (Givens) or actions in order (each When starts one, And adds to it).
fn phrase_body(rp: &RawPhrase, vocab: &Vocabulary, diags: &mut Vec<Diagnostic>) -> Option<PhraseBody> {
    let mut sections = Sections { current: Keyword::Given };
    let mut facts = vec![];
    let mut actions: Vec<Trigger> = vec![];
    let mut kind: Option<Keyword> = None;
    for s in &rp.steps {
        let section = sections.next(s.keyword);
        if section == Keyword::Then || kind.is_some_and(|k| k != section) {
            diags.push(
                Diagnostic::error("bad-phrase", "a phrase is either a setup (Givens) or actions (Whens)")
                    .at(&s.span, "")
                    .help("what happens after an action belongs in the expectations, not the phrase"),
            );
            return None;
        }
        kind = Some(section);
        if section == Keyword::Given {
            facts.extend(facts_of(s, vocab, diags));
        } else {
            act(&mut actions, s, vocab, diags);
        }
    }
    match kind {
        None => {
            diags.push(Diagnostic::error("bad-phrase", format!("phrase «{}» has no steps", rp.name)).at(&rp.span, ""));
            None
        }
        Some(Keyword::Given) => Some(PhraseBody::Setup(facts)),
        _ => Some(PhraseBody::Actions(actions)),
    }
}

/// What one step line contributes, phrases expanded.
enum Part {
    /// A fact, or a setup phrase's facts.
    Facts(Vec<Fact>),
    /// An event, or an action phrase's actions.
    Actions(Vec<Trigger>),
    Shift(VarId, Shift),
}

fn part(s: &RawStep, vocab: &Vocabulary, diags: &mut Vec<Diagnostic>) -> Option<Part> {
    match step::resolve(&s.text, &s.span, vocab) {
        Ok(Meaning::Fact(f)) => Some(Part::Facts(vec![f])),
        Ok(Meaning::Event(e)) => Some(Part::Actions(vec![Trigger { event: Some(e), changes: Condition::new() }])),
        Ok(Meaning::Shift(v, shift)) => Some(Part::Shift(v, shift)),
        Ok(Meaning::Phrase(p)) => Some(match &vocab.phrase(p).body {
            // Facts point at where the phrase is used, for diagnostics.
            PhraseBody::Setup(facts) => {
                Part::Facts(facts.iter().map(|f| Fact { span: s.span.clone(), ..f.clone() }).collect())
            }
            PhraseBody::Actions(actions) => Part::Actions(actions.clone()),
        }),
        Err(d) => {
            diags.push(d);
            None
        }
    }
}

fn text_of(s: &RawStep, vocab: &Vocabulary) -> StepText {
    let meaning = step::resolve(&s.text, &s.span, vocab).ok().map(|m| m.said());
    StepText { keyword: s.keyword, text: s.text.clone(), span: s.span.clone(), meaning }
}

/// The facts a Given, Then or Background line says.
fn facts_of(s: &RawStep, vocab: &Vocabulary, diags: &mut Vec<Diagnostic>) -> Vec<Fact> {
    let place = if s.keyword == Keyword::Then { "a Then" } else { "a Given" };
    match part(s, vocab, diags) {
        Some(Part::Facts(f)) => f,
        Some(Part::Shift(..)) => {
            diags.push(
                Diagnostic::error(
                    "misplaced-shift",
                    format!("`{}` is relative, which only an expectation's Then can be", s.text),
                )
                .at(&s.span, "")
                .help("say what it is: `the plan has 3 items`"),
            );
            vec![]
        }
        Some(Part::Actions(_)) => {
            diags.push(
                Diagnostic::error("misplaced-event", format!("`{}` is an action, which can't be {place}", s.text))
                    .at(&s.span, "")
                    .help("events and action phrases go in `When` steps"),
            );
            vec![]
        }
        None => vec![],
    }
}

/// Fold `t` into `action`: its event and its changes.
fn merge(action: &mut Trigger, t: &Trigger, vocab: &Vocabulary, span: &Span, diags: &mut Vec<Diagnostic>) {
    if let Some(e) = t.event {
        if action.event.is_some() {
            diags.push(
                Diagnostic::error("two-events", "one action can only have one event")
                    .at(span, "")
                    .help("put a `Then` between them, or split them into two `When` steps"),
            );
        }
        action.event = Some(e);
    }
    for (v, c) in t.changes.iter() {
        let f = Fact { var: v, constraint: *c, span: span.clone() };
        add(&mut action.changes, &f, vocab, diags, "this When contradicts itself");
    }
}

/// A When or And line in a sequence of actions: a When starts the next action, an And adds to
/// the last one, and an action phrase brings its own.
fn act(actions: &mut Vec<Trigger>, s: &RawStep, vocab: &Vocabulary, diags: &mut Vec<Diagnostic>) -> usize {
    let before = actions.len();
    let fresh = s.keyword == Keyword::When || actions.is_empty();
    match part(s, vocab, diags) {
        Some(Part::Facts(facts)) => {
            if fresh {
                actions.push(Trigger::default());
            }
            let last = actions.last_mut().unwrap();
            for f in facts {
                add(&mut last.changes, &f, vocab, diags, "this When contradicts itself");
            }
        }
        Some(Part::Actions(ts)) => {
            for (i, t) in ts.iter().enumerate() {
                if i > 0 || fresh {
                    actions.push(Trigger::default());
                }
                merge(actions.last_mut().unwrap(), t, vocab, &s.span, diags);
            }
        }
        Some(Part::Shift(..)) => diags.push(
            Diagnostic::error("misplaced-shift", "a When says what happens, not how much something changes")
                .at(&s.span, "")
                .help("relative steps go in the Then"),
        ),
        None => {}
    }
    actions.len() - before
}

fn add(c: &mut Condition, f: &Fact, vocab: &Vocabulary, diags: &mut Vec<Diagnostic>, what: &str) {
    let var = vocab.var(f.var);
    if f.constraint == var.full() {
        diags.push(
            Diagnostic::warning(
                "says-nothing",
                format!("every value of {} satisfies this, so it says nothing", var.subject),
            )
            .at(&f.span, "")
            .help("narrow it, or remove the step"),
        );
    }
    let before = c.get(f.var).copied();
    c.constrain(var, f.constraint);
    if !c.is_satisfiable() {
        let earlier = before.map(|b| var.describe(&b)).unwrap_or_default();
        diags.push(
            Diagnostic::error("contradiction", what.to_string())
                .at(&f.span, format!("this can't hold together with: {earlier}")),
        );
    }
}

/// Which section a step is in, following And/But back to the last Given/When/Then.
struct Sections {
    current: Keyword,
}

impl Sections {
    fn next(&mut self, k: Keyword) -> Keyword {
        if !matches!(k, Keyword::And | Keyword::But) {
            self.current = k;
        }
        self.current
    }
}

fn expectation(
    id: ExpId,
    feature: FeatureId,
    re: &RawExpectation,
    vocab: &Vocabulary,
    diags: &mut Vec<Diagnostic>,
) -> Expectation {
    let mut given = Condition::new();
    let mut actions: Vec<Trigger> = vec![];
    let mut then = Condition::new();
    let mut shifts = vec![];
    let mut sections = Sections { current: Keyword::Given };
    let mut seen = Keyword::Given;
    let order = |k: Keyword| match k {
        Keyword::Given => 0,
        Keyword::When => 1,
        _ => 2,
    };
    for (i, s) in re.steps.iter().enumerate() {
        if i == 0 && matches!(s.keyword, Keyword::And | Keyword::But) {
            diags.push(
                Diagnostic::error("syntax", format!("an expectation can't start with `{}`", s.keyword.as_str()))
                    .at(&s.span, ""),
            );
            continue;
        }
        let section = sections.next(s.keyword);
        if order(section) < order(seen) {
            diags.push(
                Diagnostic::error("syntax", format!("`{}` after `{}`", section.as_str(), seen.as_str()))
                    .at(&s.span, "")
                    .help("an expectation reads Given…, When…, Then…; split it into two expectations"),
            );
            continue;
        }
        seen = section;
        match section {
            Keyword::When => {
                // One action: several When lines all add to it.
                let mut line = s.clone();
                if !actions.is_empty() {
                    line.keyword = Keyword::And;
                }
                if act(&mut actions, &line, vocab, diags) > 1 || actions.len() > 1 {
                    diags.push(
                        Diagnostic::error("bad-phrase", "an expectation's When is one action").at(&s.span, "").help(
                            "this phrase is a sequence of actions; use it in a story, or describe one of its actions",
                        ),
                    );
                    actions.truncate(1);
                }
            }
            Keyword::Given => {
                for f in facts_of(s, vocab, diags) {
                    add(&mut given, &f, vocab, diags, "this Given can never hold");
                }
            }
            _ => match part(s, vocab, diags) {
                Some(Part::Shift(v, shift)) => {
                    if then.get(v).is_some() || shifts.iter().any(|(w, _)| *w == v) {
                        diags.push(
                            Diagnostic::error(
                                "contradiction",
                                format!("this Then already says what {} becomes", vocab.var(v).subject),
                            )
                            .at(&s.span, ""),
                        );
                    }
                    shifts.push((v, shift));
                }
                Some(Part::Facts(facts)) => {
                    for f in facts {
                        add(&mut then, &f, vocab, diags, "this Then can never hold");
                    }
                }
                Some(Part::Actions(_)) => diags.push(
                    Diagnostic::error("misplaced-event", format!("`{}` is an action, which can't be a Then", s.text))
                        .at(&s.span, "")
                        .help("events and action phrases go in `When` steps"),
                ),
                None => {}
            },
        }
    }
    if !re.steps.iter().any(|s| s.keyword == Keyword::Then) {
        diags.push(
            Diagnostic::error("no-then", format!("expectation «{}» has no Then", re.name))
                .at(&re.span, "")
                .help("say what the system does: `Then …`"),
        );
    }
    Expectation {
        id,
        feature,
        name: re.name.clone(),
        slug: text::slug(&re.name),
        description: re.description.clone(),
        span: re.span.clone(),
        given,
        trigger: actions.pop().unwrap_or_default(),
        then,
        shifts,
        overrides: vec![],
        steps: re.steps.iter().map(|s| text_of(s, vocab)).collect(),
        hidden: re.hidden,
    }
}

/// An action spelt out as steps, for the test report: "When the throttle is 80 % And …".
fn spelt(action: &Trigger, vocab: &Vocabulary, span: &Span) -> Vec<StepText> {
    let mut out: Vec<StepText> = action
        .changes
        .iter()
        .map(|(v, c)| StepText {
            keyword: Keyword::And,
            text: vocab.var(v).describe(c),
            span: span.clone(),
            meaning: Some(Said::Fact(v, *c)),
        })
        .collect();
    if let Some(e) = action.event {
        out.push(StepText {
            keyword: Keyword::And,
            text: vocab.event(e).text.clone(),
            span: span.clone(),
            meaning: Some(Said::Event(e)),
        });
    }
    if let Some(first) = out.first_mut() {
        first.keyword = Keyword::When;
    }
    out
}

fn scenario(sc: &RawScenario, vocab: &Vocabulary, diags: &mut Vec<Diagnostic>) -> Option<Scenario> {
    let mut given = Condition::new();
    let mut given_steps = vec![];
    let mut steps: Vec<StoryStep> = vec![];
    let mut sections = Sections { current: Keyword::Given };
    let mut last = Keyword::Given;
    let mut group = 0;
    for s in &sc.steps {
        let section = sections.next(s.keyword);
        match section {
            Keyword::Given => {
                if !steps.is_empty() {
                    diags.push(
                        Diagnostic::error("syntax", "`Given` after `When`")
                            .at(&s.span, "")
                            .help("arrange everything before the first When, or start a new Scenario"),
                    );
                } else {
                    for f in facts_of(s, vocab, diags) {
                        add(&mut given, &f, vocab, diags, "this Given can never hold");
                    }
                    given_steps.push(text_of(s, vocab));
                }
            }
            Keyword::When => {
                // Each `When` starts a step (a group of them, for a phrase of several actions);
                // an `And`/`But` after it adds to the last action.
                let mut line = s.clone();
                if s.keyword == Keyword::When || last != Keyword::When || steps.is_empty() {
                    line.keyword = Keyword::When;
                    group += 1;
                }
                let starts = line.keyword == Keyword::When;
                let mut actions: Vec<Trigger> = match (starts, steps.last()) {
                    (false, Some(st)) => vec![st.action.clone()],
                    _ => vec![],
                };
                let kept = actions.len();
                act(&mut actions, &line, vocab, diags);
                if kept == 1 {
                    steps.last_mut().unwrap().action = actions.remove(0);
                }
                for a in actions {
                    steps.push(StoryStep {
                        action: a,
                        checks: vec![],
                        steps: vec![],
                        span: s.span.clone(),
                        group,
                        written: vec![],
                    });
                }
                // The test report spells each action out; the docs keep the line as written.
                let first = steps.iter().position(|st| st.group == group).unwrap_or(0);
                for st in &mut steps[first..] {
                    st.steps = spelt(&st.action, vocab, &st.span);
                }
                if let Some(st) = steps.get_mut(first) {
                    st.written.push(text_of(s, vocab));
                }
            }
            _ => match steps.len() {
                0 => diags.push(
                    Diagnostic::error("syntax", "`Then` before any `When`")
                        .at(&s.span, "")
                        .help("to start from a particular state, use `Given`"),
                ),
                n => {
                    let checks = facts_of(s, vocab, diags);
                    let text = text_of(s, vocab);
                    let first = steps.iter().position(|st| st.group == group).unwrap_or(n - 1);
                    steps[first].written.push(text.clone());
                    let step = steps.last_mut().unwrap();
                    step.steps.push(text);
                    step.checks.extend(checks);
                }
            },
        }
        last = section;
    }
    if steps.is_empty() {
        diags.push(
            Diagnostic::error("empty-story", format!("scenario «{}» has no `When`", sc.name))
                .at(&sc.span, "")
                .help("a scenario is a sequence of `When … Then …` steps"),
        );
        return None;
    }
    Some(Scenario {
        name: sc.name.clone(),
        slug: text::slug(&sc.name),
        description: sc.description.clone(),
        span: sc.span.clone(),
        given,
        given_steps,
        steps,
    })
}
