//! The Markdown pages, for reading on GitHub: the same manual as the site. An overview, a
//! page per tutorial and per feature, and the test report with a page per test.

use crate::manual::{self, Note};
use crate::{
    Docs, Facts, NO_SCREENSHOT, NOT_SUPPORTED, Page, action_heading, feature_slug, missing_shot, narrative,
    status_label, values, verdict_label, voice,
};
use expectation_graph::Node;
use expectation_model::text::paragraphs;
use expectation_model::{ExpId, PhraseBody, SectionId, StepText, Suite};
use expectation_story::{Because, Check, CheckResult, Origin, Plan, Status, StepResult, Verdict};
use std::fmt::Write;

const GENERATED: &str =
    "<!-- Generated from the suite's .expect files by the expectation domain; edit those, not this. -->";

pub fn pages(d: &Docs, f: &Facts) -> Vec<Page> {
    let mut pages = vec![
        Page { path: "README.md".into(), content: index(d) },
        Page { path: "tests.md".into(), content: report(d, f) },
        Page { path: "glossary.md".into(), content: glossary(d) },
        Page { path: "vocabulary.md".into(), content: vocabulary(d) },
    ];
    for i in 0..d.plans.len() {
        pages.push(Page { path: format!("tests/{}.md", d.plans[i].slug), content: test_page(d, f, i) });
        if let Some(t) = manual::tutorial(d, f, i) {
            pages.push(Page { path: format!("tutorials/{}.md", d.plans[i].slug), content: tutorial_page(d, f, i, &t) });
        }
    }
    for i in 0..d.outline.map_or(0, |o| o.pages.len()) {
        pages.push(Page { path: format!("manual/{}.md", d.outline.unwrap().pages[i].slug), content: authored_page(d, i) });
    }
    for feature in &d.suite.features {
        let path = format!("features/{}.md", feature_slug(d.suite, feature));
        pages.push(Page { path, content: feature_page(d, f, feature.id.0 as usize) });
    }
    pages
}

fn verdict_mark(v: Option<Verdict>) -> String {
    let icon = match v {
        Some(Verdict::Verified) => "✅",
        Some(Verdict::Failed) => "❌",
        Some(Verdict::Unsupported) => "🚧",
        _ => "⚪",
    };
    format!("{icon} {}", verdict_label(v))
}

fn status_mark(s: Status) -> String {
    let icon = match s {
        Status::Passed => "✅",
        Status::Failed => "❌",
        Status::Skipped => "⏭️",
        Status::Unsupported => "🚧",
        Status::NotRun => "⚪",
    };
    format!("{icon} {}", status_label(s))
}

fn up(from_subdir: bool) -> &'static str {
    if from_subdir { "../" } else { "" }
}

/// Prose with its `[[links]]` made into Markdown links. Code spans are Markdown already.
fn prose(d: &Docs, text: &str, from_subdir: bool) -> String {
    let mut o = String::new();
    for part in manual::inline(text) {
        match part {
            manual::Inline::Text(t) => o.push_str(t),
            manual::Inline::Code(c) => {
                let _ = write!(o, "`{c}`");
            }
            manual::Inline::Link(name) => match manual::find(d, name) {
                Some(r) => {
                    let _ = write!(o, "[{name}]({}{})", up(from_subdir), ref_path(d, r));
                }
                None => o.push_str(name),
            },
        }
    }
    o
}

fn ref_path(d: &Docs, r: manual::Ref) -> String {
    let s = d.suite;
    match r {
        manual::Ref::Section(id) => format!("README.md#{}", expectation_model::text::slug(&s.section(id).name)),
        manual::Ref::Feature(f) => format!("features/{}.md", feature_slug(s, s.feature(f))),
        manual::Ref::Exp(e) => {
            let x = s.exp(e);
            format!("features/{}.md#{}", feature_slug(s, s.feature(x.feature)), x.slug)
        }
        manual::Ref::Tutorial(i) => format!("tutorials/{}.md", d.plans[i].slug),
        manual::Ref::Term(t) => format!("glossary.md#{}", s.glossary[t].slug),
        manual::Ref::Page(i) => format!("manual/{}.md", d.outline.map_or("", |o| o.pages[i].slug.as_str())),
    }
}

fn exp_link(d: &Docs, e: ExpId, from_subdir: bool) -> String {
    let x = d.suite.exp(e);
    let feature = feature_slug(d.suite, d.suite.feature(x.feature));
    format!("[{}]({}features/{feature}.md#{})", x.name, up(from_subdir), x.slug)
}

fn test_link(d: &Docs, i: usize, from_subdir: bool) -> String {
    let p = &d.plans[i];
    format!("[{}]({}tests/{}.md)", p.title, up(from_subdir), p.slug)
}

fn tutorial_link(d: &Docs, i: usize, from_subdir: bool) -> String {
    let p = &d.plans[i];
    format!("[{}]({}tutorials/{}.md)", manual::short_title(d.suite, p), up(from_subdir), p.slug)
}

/// "Ground control station › Planning a mission".
fn section_path(suite: &Suite, id: SectionId) -> String {
    let chain = suite.chain(id);
    let names: Vec<&str> = chain.iter().skip(1).map(|s| s.name.as_str()).collect();
    if names.is_empty() { suite.name.clone() } else { names.join(" › ") }
}

/// A page's first lines: the generated marker and the way back.
fn head(o: &mut String, s: &Suite, section: SectionId) {
    let _ = write!(o, "{GENERATED}\n\n[{}](../README.md)", s.name);
    if s.section(section).parent.is_some() {
        let _ = write!(o, " › {}", section_path(s, section));
    }
    o.push_str("\n\n");
}

/// A screenshot, or a placeholder. `alt` says what it shows: the step's outcome.
fn image(o: &mut String, shot: Option<&str>, status: Status, alt: &str) {
    let alt = manual::plain(alt).replace('"', "'");
    let alt = if alt.is_empty() { "Screenshot".to_string() } else { alt };
    match shot {
        Some(shot) => {
            let _ = writeln!(o, "<img src=\"../{shot}\" width=\"720\" alt=\"{alt}\">\n");
        }
        None => {
            let placeholder = if status == Status::Unsupported { NOT_SUPPORTED } else { NO_SCREENSHOT };
            let _ = writeln!(o, "<img src=\"../{placeholder}\" width=\"360\" alt=\"{}\">\n", manual::missing(status));
        }
    }
}

fn index(d: &Docs) -> String {
    let s = d.suite;
    let mut o = String::new();
    let _ = writeln!(o, "{GENERATED}\n\n# {}\n", s.name);
    if let Some(root) = s.sections.first() {
        for p in paragraphs(&root.description) {
            let _ = writeln!(o, "{}\n", prose(d, &p, false));
        }
    }
    let _ = writeln!(
        o,
        "There is also an [HTML version](site/index.html) of this manual. The [glossary](glossary.md) explains \
         its terms, and the [test report](tests.md) shows the results of the last test run.\n"
    );
    // With an outline: a contents list first, then the chapters in the outline's order under
    // the heading of their part. Without one, the sections in the suite's own order.
    // Each section with its part and its depth below the chapter it is placed as.
    let mut order: Vec<(Option<(usize, usize)>, &expectation_model::Section)> = vec![];
    if let Some(outline) = d.outline {
        let _ = writeln!(o, "## Contents\n");
        for part in &outline.parts {
            let _ = writeln!(o, "**{}**\n", part.name);
            for item in &part.items {
                match *item {
                    crate::outline::Item::Section(id) => {
                        let sec = s.section(id);
                        let about = paragraphs(&sec.description).into_iter().next().unwrap_or_default();
                        let _ = writeln!(
                            o,
                            "- [{}](README.md#{}): {}",
                            sec.name,
                            expectation_model::text::slug(&sec.name),
                            manual::plain(&crate::outline::first_sentence(&about))
                        );
                    }
                    crate::outline::Item::Page(i) => {
                        let page = &outline.pages[i];
                        let _ = writeln!(o, "- [{}](manual/{}.md): {}", page.title, page.slug, prose(d, &page.about(), false));
                    }
                }
            }
            if part.name == "Reference" {
                let _ = writeln!(o, "- [Glossary](glossary.md): What the words in this manual mean.");
            }
            o.push('\n');
        }
        for (p, part) in outline.parts.iter().enumerate() {
            for item in &part.items {
                if let crate::outline::Item::Section(id) = *item {
                    // Depth first, as the suite lists them, without the sections the outline
                    // places elsewhere.
                    let mut stack = vec![(id, 1usize)];
                    while let Some((x, depth)) = stack.pop() {
                        order.push((Some((p, depth)), s.section(x)));
                        let below = s.section(x).children.iter().rev().filter(|&&c| !outline.listed(c));
                        stack.extend(below.map(|&c| (c, depth + 1)));
                    }
                }
            }
        }
    } else {
        order = s.sections.iter().skip(1).map(|x| (None, x)).collect();
    }
    let mut last_part = None;
    for (part, section) in order {
        if let (Some((p, _)), Some(outline)) = (part, d.outline)
            && last_part != Some(p)
        {
            last_part = Some(p);
            let _ = writeln!(o, "## {}\n", outline.parts[p].name);
        }
        let depth = match part {
            Some((_, below)) => below + 2,
            None => s.chain(section.id).len(),
        }
        .min(6);
        let _ = writeln!(o, "{} {}\n", "#".repeat(depth), section.name);
        for p in paragraphs(&section.description) {
            let _ = writeln!(o, "{}\n", prose(d, &p, false));
        }
        if let Some(before) = manual::before(d, section.id) {
            o.push_str("**Before you begin:**");
            if let Some(parent) = before.parent {
                let _ = write!(o, " start from the setup in {}.", s.section(parent).name);
            }
            o.push_str("\n\n");
            for item in &before.items {
                let _ = writeln!(o, "- {}", prose(d, item, false));
            }
            if !before.items.is_empty() {
                o.push('\n');
            }
            if !before.route.is_empty() {
                if before.route_is_detail {
                    o.push_str("<details><summary>Step by step</summary>\n\n");
                }
                for (n, step) in before.route.iter().enumerate() {
                    let _ = writeln!(o, "{}. {}", n + 1, prose(d, step, false));
                }
                o.push('\n');
                if before.route_is_detail {
                    o.push_str("</details>\n\n");
                }
            }
        }
        let tutorials: Vec<usize> = (0..d.plans.len())
            .filter(|&i| d.plans[i].section == section.id && matches!(d.plans[i].origin, Origin::Story { .. }))
            .collect();
        if !tutorials.is_empty() {
            o.push_str("**Tutorials**\n\n");
            for i in tutorials {
                let Origin::Story { story, .. } = d.plans[i].origin else { continue };
                let about = paragraphs(&s.story(story).description).into_iter().next().unwrap_or_default();
                let _ = writeln!(o, "- **{}**: {}", tutorial_link(d, i, false), prose(d, &about, false));
            }
            o.push('\n');
        }
        if !section.features.is_empty() {
            o.push_str("**Topics**\n\n");
            for &fid in &section.features {
                let feature = s.feature(fid);
                let about = paragraphs(&feature.description).into_iter().next().unwrap_or_default();
                let about = prose(d, &about, false);
                let _ = writeln!(o, "- **[{}](features/{}.md)**: {about}", feature.name, feature_slug(s, feature));
            }
            o.push('\n');
        }
    }
    o
}

/// A hand-written page: its Markdown, with `[[links]]` made into links and a way back.
fn authored_page(d: &Docs, i: usize) -> String {
    let outline = d.outline.expect("hand-written pages come from the outline");
    let page = &outline.pages[i];
    let mut o = format!(
        "{GENERATED}\n\n[{}](../README.md) › {}\n\n",
        d.suite.name,
        outline.parts[page.part].name
    );
    let mut in_code = false;
    for line in page.source.lines() {
        if line.starts_with("```") {
            in_code = !in_code;
            o.push_str(line);
        } else if in_code {
            o.push_str(line);
        } else {
            o.push_str(&prose(d, line, true));
        }
        o.push('\n');
    }
    o
}

fn tutorial_page(d: &Docs, f: &Facts, i: usize, t: &manual::Tutorial) -> String {
    let s = d.suite;
    let plan = &d.plans[i];
    let mut o = String::new();
    head(&mut o, s, plan.section);
    if let Some(k) = &t.kicker {
        let _ = writeln!(o, "*{k}*\n");
    }
    let _ = writeln!(o, "# {}\n", t.title);
    for p in &t.intro {
        let _ = writeln!(o, "{}\n", prose(d, p, true));
    }
    if !t.before.is_empty() {
        o.push_str("## Before you begin\n\n");
        for item in &t.before {
            let _ = writeln!(o, "- {}", prose(d, item, true));
        }
        o.push('\n');
        if let Some(sec) = t.details {
            let name = &s.section(sec).name;
            let _ = writeln!(
                o,
                "*[{name}](../README.md#{}) has the setup step by step.*\n",
                expectation_model::text::slug(name)
            );
        }
    }
    o.push_str("## Steps\n\n");
    for (n, step) in t.steps.iter().enumerate() {
        let _ = writeln!(o, "{}. **{}**", n + 1, prose(d, &step.doing, true));
        if !step.seen.is_empty() {
            let _ = writeln!(o, "\n   {}", prose(d, &step.seen, true));
        }
        o.push('\n');
        let mut img = String::new();
        image(&mut img, step.shot.as_deref(), step.status, if step.seen.is_empty() { &step.doing } else { &step.seen });
        for line in img.lines() {
            let _ = writeln!(o, "   {line}");
        }
    }
    if !t.related.is_empty() {
        o.push_str("## Learn more\n\n");
        for &fid in &t.related {
            let feature = s.feature(fid);
            let _ = writeln!(o, "- [{}](../features/{}.md)", feature.name, feature_slug(s, feature));
        }
        o.push('\n');
    }
    let _ = writeln!(
        o,
        "<details><summary>How this is tested: {}</summary>\n\nThis tutorial is also an acceptance test. \
         [Its report](../tests/{}.md) has every step, check and screenshot from the last run.\n",
        status_mark(f.status[i]),
        plan.slug
    );
    if let Some(n) = &t.narrative {
        let _ = writeln!(o, "The user story: {n}\n");
    }
    let _ = writeln!(o, "```gherkin\n{}```\n\n</details>", gherkin(&t.written));
    if let Origin::Story { .. } = plan.origin {
    }
    o
}

fn feature_page(d: &Docs, f: &Facts, fi: usize) -> String {
    let s = d.suite;
    let feature = &s.features[fi];
    let mut o = String::new();
    head(&mut o, s, feature.section);
    let _ = writeln!(o, "# {}\n", feature.name);
    for p in paragraphs(&feature.description) {
        let _ = writeln!(o, "{}\n", prose(d, &p, true));
    }
    // Expectations marked `Docs: hidden` are only in the test report.
    for &e in feature.expectations.iter().filter(|&&e| !s.exp(e).hidden) {
        let x = s.exp(e);
        let _ = writeln!(o, "<a id=\"{}\"></a>\n\n## {}\n", x.slug, x.name);
        for p in paragraphs(&x.description) {
            let _ = writeln!(o, "{}\n", prose(d, &p, true));
        }
        let _ = writeln!(o, "{}\n", prose(d, &voice::how(s, &x.steps), true));
        for note in manual::notes(d, e) {
            let line = match note {
                Note::Except { when, see } if when.is_empty() => {
                    format!("**Exception:** see {}.", exp_link(d, see, true))
                }
                Note::Except { when, see } => format!("**Except** when {when}: see {}.", exp_link(d, see, true)),
                Note::ExceptionTo(other) => format!("An exception to {}.", exp_link(d, other, true)),
                Note::CanLeave { by, what } => format!("**Exception:** {} can leave {what}.", exp_link(d, by, true)),
            };
            let _ = writeln!(o, "> {line}\n");
        }
        match f.shot_of(d, e) {
            Some((_, _, shot)) => image(&mut o, Some(&shot), Status::Passed, &voice::how(s, &x.steps)),
            None => image(&mut o, None, f.tested_by(e).first().map_or(Status::NotRun, |&p| f.status[p]), ""),
        }
        let shown: Vec<String> = manual::shown_in(d, f, e).into_iter().map(|p| tutorial_link(d, p, true)).collect();
        if !shown.is_empty() {
            let _ = writeln!(o, "See it in {}.\n", shown.join(", "));
        }
        let _ = writeln!(
            o,
            "<details><summary>How this is tested: {}</summary>\n\n```gherkin\n{}```\n",
            verdict_mark(f.verdict(e)),
            gherkin(&x.steps)
        );
        let parents: Vec<String> = d
            .analysis
            .flow
            .parents(e)
            .map(|p| match p {
                Node::Start => "the section's setup".to_string(),
                Node::Exp(p) => exp_link(d, p, true),
            })
            .collect();
        match d.analysis.flow.level[e.0 as usize] {
            Some(_) => {
                let _ = writeln!(o, "- **Follows:** {}", parents.join(", "));
            }
            None if d.analysis.flow.truncated => {
                let _ = writeln!(o, "- **Follows:** nothing the search found within its limits");
            }
            None => {
                let _ = writeln!(o, "- **Unreachable:** nothing leads to a state where it applies");
            }
        }
        let children: Vec<String> = d.analysis.flow.children(Node::Exp(e)).map(|c| exp_link(d, c, true)).collect();
        if !children.is_empty() {
            let _ = writeln!(o, "- **Leads to:** {}", children.join(", "));
        }
        let by = f.tested_by(e);
        let mut links: Vec<String> = by.iter().take(5).map(|&p| test_link(d, p, true)).collect();
        if by.len() > 5 {
            links.push(format!("[{} more](../tests.md)", by.len() - 5));
        }
        let _ = writeln!(
            o,
            "- **Tested by:** {}\n\n</details>\n",
            if links.is_empty() { "nothing yet".into() } else { links.join(", ") }
        );
    }
    o
}

fn report(d: &Docs, f: &Facts) -> String {
    let s = d.suite;
    let mut o = String::new();
    let _ = writeln!(
        o,
        "{GENERATED}\n\n[{}](README.md)\n\n# Test report\n\nEvery tutorial is also an acceptance test, and generated \
         tests reach the behaviour no tutorial does. This is what the last run found. The tests are written in the \
         [test vocabulary](vocabulary.md).\n",
        s.name
    );
    match (d.report, &f.verdicts) {
        (Some(r), Some(v)) => {
            let count = |x: Verdict| v.iter().filter(|&&y| y == x).count();
            let _ = writeln!(
                o,
                "**{} tests:** {} passed, {} failed, {} not supported yet. **{} expectations:** {} verified, {} failed, \
                 {} not supported yet, {} untested.\n",
                r.results.len(),
                r.count(Status::Passed),
                r.count(Status::Failed),
                r.count(Status::Unsupported),
                s.expectations.len(),
                count(Verdict::Verified),
                count(Verdict::Failed),
                count(Verdict::Unsupported),
                count(Verdict::Untested)
            );
        }
        _ => o.push_str("The tests haven't run.\n\n"),
    }
    for section in &s.sections {
        let tests: Vec<usize> = (0..d.plans.len()).filter(|&i| d.plans[i].section == section.id).collect();
        if tests.is_empty() {
            continue;
        }
        let _ = writeln!(o, "## {}\n\n| Test | Kind | Result |\n| --- | --- | --- |", section_path(s, section.id));
        for i in tests {
            let kind = match d.plans[i].origin {
                Origin::Story { .. } => format!("[tutorial](tutorials/{}.md)", d.plans[i].slug),
                Origin::Coverage { .. } => "generated".into(),
            };
            let _ = writeln!(o, "| {} | {kind} | {} |", test_link(d, i, false), status_mark(f.status[i]));
        }
        o.push('\n');
    }
    let _ = writeln!(
        o,
        "## Map\n\nWhat leads to what, starting from each section's setup. Each expectation is placed after the ones \
         that get the system into a state where it applies. Dotted arrows are exceptions. Hexagons are invariants.\n"
    );
    let classes = |e: ExpId| -> Option<&'static str> {
        f.verdict(e).map(|v| match v {
            Verdict::Verified => "passed",
            Verdict::Failed => "failed",
            Verdict::Untested | Verdict::Unsupported | Verdict::NotRun => "untested",
        })
    };
    let _ = writeln!(o, "```mermaid\n{}```", expectation_graph::export::mermaid(s, d.analysis, &classes));
    o
}

fn glossary(d: &Docs) -> String {
    let s = d.suite;
    let mut o = String::new();
    let _ =
        writeln!(o, "{GENERATED}\n\n[{}](README.md)\n\n# Glossary\n\nWhat the words in this manual mean.\n", s.name);
    for i in manual::terms(s) {
        let t = &s.glossary[i];
        let _ = writeln!(o, "<a id=\"{}\"></a>\n\n**{}**\n", t.slug, t.name);
        for p in paragraphs(&t.description) {
            let _ = writeln!(o, "{}\n", prose(d, &p, false));
        }
    }
    o
}

fn vocabulary(d: &Docs) -> String {
    let s = d.suite;
    let v = &s.vocabulary;
    let mut o = String::new();
    let _ = writeln!(
        o,
        "{GENERATED}\n\n[{}](README.md) › [Test report](tests.md)\n\n# Test vocabulary\n\nThe terms the tests are \
         written in: the states the system can be in, the things that can happen, and the shorthand for a few of them \
         together.\n",
        s.name
    );
    for group in manual::glossary_groups(s) {
        let mine = |span: &expectation_model::Span| manual::declared_in(s, span) == group;
        let vars: Vec<_> = v.variables.iter().filter(|x| mine(&x.span)).collect();
        let events: Vec<_> = v.events.iter().filter(|x| mine(&x.span)).collect();
        let phrases: Vec<_> = v.phrases.iter().filter(|x| mine(&x.span)).collect();
        if vars.is_empty() && events.is_empty() && phrases.is_empty() {
            continue;
        }
        if let Some(g) = group {
            let _ = writeln!(o, "## {}\n", s.section(g).name);
        }
        if !vars.is_empty() {
            o.push_str("**States**\n\n| State | Can be | To change it |\n| --- | --- | --- |\n");
            for var in vars {
                let set = voice::how_to_set(var).map(|t| voice::sentence(&t)).unwrap_or_default();
                let _ = writeln!(o, "| {} | {} | {set} |", var.subject, values(var));
            }
            o.push('\n');
        }
        if !events.is_empty() {
            o.push_str("**Actions**\n\n| Action | What to do |\n| --- | --- |\n");
            for ev in events {
                let what = match voice::event(ev) {
                    voice::Action::Do(d) => voice::sentence(&d),
                    voice::Action::Happens(_) => "Nothing: it happens by itself.".into(),
                };
                let _ = writeln!(o, "| {} | {what} |", ev.text);
            }
            o.push('\n');
        }
        if !phrases.is_empty() {
            o.push_str("**Shorthand**\n\n| Phrase | Means |\n| --- | --- |\n");
            for p in phrases {
                let means = match &p.body {
                    PhraseBody::Setup(facts) => {
                        let f: Vec<String> = facts.iter().map(|f| v.var(f.var).describe(&f.constraint)).collect();
                        expectation_model::text::join_and(&f)
                    }
                    PhraseBody::Actions(actions) => {
                        actions.iter().map(|t| s.describe_trigger(t)).collect::<Vec<_>>().join(", then ")
                    }
                };
                let say = p.say.as_deref().map(voice::sentence).unwrap_or_default();
                let _ = writeln!(o, "| {} | {say} *({means})* |", p.name);
            }
            o.push('\n');
        }
    }
    o
}

fn test_page(d: &Docs, f: &Facts, i: usize) -> String {
    let s = d.suite;
    let plan = &d.plans[i];
    let result = d.report.and_then(|r| r.results.get(i));
    let mut o = String::new();
    let _ = writeln!(o, "{GENERATED}\n\n[{}](../README.md) › [Test report](../tests.md)\n\n# {}\n", s.name, plan.title);
    match plan.origin {
        Origin::Story { story, .. } => {
            let st = s.story(story);
            if !st.narrative.is_empty() {
                let _ = writeln!(o, "> {}\n", narrative(&st.narrative));
            }
            let _ = writeln!(
                o,
                "The test behind the tutorial {}: every step it takes, what it checks and what it saw.\n",
                tutorial_link(d, i, true)
            );
        }
        Origin::Coverage { target } => {
            let _ = writeln!(
                o,
                "*Generated.* No tutorial reaches {}, so this test takes the shortest route there from the setup of «{}».\n",
                exp_link(d, target, true),
                s.section(plan.section).name
            );
        }
    }
    let _ = writeln!(o, "**Result:** {}\n", status_mark(f.status[i]));
    if let Some(e) = result.and_then(|r| r.start.error.as_ref()) {
        let _ = writeln!(o, "> {e}\n");
    }

    let _ = writeln!(o, "## Start\n\n```gherkin\n{}```\n", start_lines(s, plan));
    step_body(d, &mut o, &plan.start_checks, result.map(|r| &r.start), f.status[i]);
    for (n, step) in plan.steps.iter().enumerate() {
        let _ = writeln!(o, "## {}. {}\n\n```gherkin\n{}```\n", n + 1, action_heading(&step.text), gherkin(&step.text));
        if !step.fired.is_empty() {
            let fired: Vec<String> = step.fired.iter().map(|&e| exp_link(d, e, true)).collect();
            let _ = writeln!(o, "Expected behaviour: {}.\n", fired.join(", "));
        }
        step_body(d, &mut o, &step.checks, result.and_then(|r| r.steps.get(n)), f.status[i]);
    }
    o
}

/// The state a test starts from, one line per state: what it arranges on top of its section's
/// start first, then the rest.
pub(crate) fn start_lines(s: &Suite, plan: &Plan) -> String {
    let section = &s.section(plan.section).start;
    let (own, rest): (Vec<_>, Vec<_>) = plan.start.iter().partition(|(v, c)| section.get(*v) != Some(*c));
    let mut out = String::new();
    for (i, (v, c)) in own.into_iter().chain(rest).enumerate() {
        let keyword = if i == 0 { "Given" } else { "  And" };
        let _ = writeln!(out, "{keyword} {}", s.vocabulary.var(v).describe(c));
    }
    out
}

fn step_body(d: &Docs, o: &mut String, checks: &[Check], result: Option<&StepResult>, plan_status: Status) {
    let status = result.map_or(plan_status, |r| r.status);
    image(o, result.and_then(|r| r.screenshot.as_deref()), status, "");
    if result.and_then(|r| r.screenshot.as_ref()).is_none() {
        let _ = writeln!(o, "*{}*\n", missing_shot(status));
    }
    if let Some(e) = result.and_then(|r| r.error.as_ref()).filter(|_| status == Status::Failed) {
        let _ = writeln!(o, "❌ **{e}**\n");
    }
    // Arranged states that read back as arranged say nothing the Start block doesn't.
    let mut arranged = 0;
    for (k, c) in checks.iter().enumerate() {
        let cr = result.and_then(|r| r.checks.get(k));
        if c.because == Because::Arranged && cr.is_some_and(|r| r.passed) {
            arranged += 1;
            continue;
        }
        let _ = writeln!(o, "- {}", check_line(d, c, cr));
    }
    if arranged > 0 {
        let _ = writeln!(o, "- ✅ every arranged state read back as arranged ({arranged})");
    }
    o.push('\n');
}

fn check_line(d: &Docs, c: &Check, r: Option<&CheckResult>) -> String {
    let var = d.suite.vocabulary.var(c.var);
    let mark = match r {
        Some(r) if r.passed => "✅",
        Some(_) => "❌",
        None => "◻️",
    };
    let why = match &c.because {
        Because::Arranged => " · arranged".to_string(),
        Because::Expectation(e) => format!(" · {}", exp_link(d, *e, true)),
        Because::Story(_) => " · the story".to_string(),
    };
    let seen = match r {
        Some(r) if !r.passed => match (&r.observed, &r.error) {
            (_, Some(e)) => format!(" — couldn't observe it: {e}"),
            (Some(v), _) => format!(" — observed **{v}**"),
            (None, _) => String::new(),
        },
        _ => String::new(),
    };
    format!("{mark} {}{why}{seen}", var.describe(&c.expect))
}

pub(crate) fn gherkin(steps: &[StepText]) -> String {
    let mut s = String::new();
    for t in steps {
        let and = matches!(t.keyword, expectation_model::Keyword::And | expectation_model::Keyword::But);
        let _ = writeln!(s, "{}{} {}", if and { "  " } else { "" }, t.keyword.as_str(), t.text);
    }
    s
}
