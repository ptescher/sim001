//! The HTML site: nested navigation that follows the suite's sections, and pages that read
//! like a manual. A section page starts with what it assumes ("Before you begin": its setup,
//! and the expectations that lead there from its parent), then its features and stories. A
//! feature page shows each behaviour with a screenshot of it being tested. A test page walks
//! through the steps. Plain HTML and CSS, no scripts.

use crate::{
    Docs, Facts, NO_SCREENSHOT, NOT_SUPPORTED, Page, action_heading, feature_slug, missing_shot, narrative,
    status_label, values, verdict_label,
};
use crate::{manual, voice};
use expectation_graph::Node;
use expectation_model::text::paragraphs;
use expectation_model::{ExpId, FeatureId, Keyword, PhraseBody, SectionId, StepText, Suite};
use expectation_story::{Because, Check, CheckResult, Origin, Status, StepResult, Verdict};
use std::fmt::Write;

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[derive(Clone, Copy, PartialEq)]
enum Here {
    Section(SectionId),
    Feature(FeatureId),
    /// A written story, as a tutorial.
    Tutorial(usize),
    /// Any test's report (stories and generated ones).
    Test(usize),
    Tests,
    Map,
    /// The definitions of the suite's terms.
    Glossary,
    /// The states, events and phrases the tests are written in.
    Vocabulary,
    Theme,
}

struct Site<'a> {
    d: &'a Docs<'a>,
    f: &'a Facts,
}

pub fn pages(d: &Docs, f: &Facts) -> Vec<Page> {
    let site = Site { d, f };
    let s = d.suite;
    let mut pages = vec![];
    for section in &s.sections {
        pages.push(Page {
            path: format!("site/{}", site.section_url(section.id)),
            content: site.section_page(section.id),
        });
    }
    for feature in &s.features {
        pages.push(Page {
            path: format!("site/{}", site.feature_url(feature.id)),
            content: site.feature_page(feature.id),
        });
    }
    for i in 0..d.plans.len() {
        pages.push(Page { path: format!("site/{}", site.test_url(i)), content: site.test_page(i) });
        if matches!(d.plans[i].origin, Origin::Story { .. }) {
            pages.push(Page { path: format!("site/{}", site.tutorial_url(i)), content: site.tutorial_page(i) });
        }
    }
    pages.push(Page { path: "site/tests.html".into(), content: site.tests_page() });
    pages.push(Page { path: "site/map.html".into(), content: site.map_page() });
    pages.push(Page { path: "site/vocabulary.html".into(), content: site.vocabulary_page() });
    pages.push(Page { path: "site/glossary.html".into(), content: site.glossary_page() });
    pages.push(Page { path: "site/assets/theme.css".into(), content: design_theme::css::stylesheet() });
    pages.push(Page { path: "site/assets/style.css".into(), content: STYLE.into() });
    pages.push(Page { path: "site/theme.html".into(), content: site.theme_page() });
    pages.push(Page {
        path: NOT_SUPPORTED.into(),
        content: placeholder("Not supported yet", "The test driver cannot do this step yet"),
    });
    pages.push(Page {
        path: NO_SCREENSHOT.into(),
        content: placeholder("No screenshot", "This step did not run, or it took no picture"),
    });
    pages
}

impl Site<'_> {
    fn suite(&self) -> &Suite {
        self.d.suite
    }

    fn section_url(&self, id: SectionId) -> String {
        let s = self.suite().section(id);
        if s.parent.is_none() { "index.html".into() } else { format!("section-{}.html", s.slug) }
    }

    fn feature_url(&self, id: FeatureId) -> String {
        format!("feature-{}.html", feature_slug(self.suite(), self.suite().feature(id)))
    }

    fn exp_url(&self, e: ExpId) -> String {
        let x = self.suite().exp(e);
        format!("{}#{}", self.feature_url(x.feature), x.slug)
    }

    fn test_url(&self, i: usize) -> String {
        format!("test-{}.html", self.d.plans[i].slug)
    }

    fn tutorial_url(&self, i: usize) -> String {
        format!("tutorial-{}.html", self.d.plans[i].slug)
    }

    fn tutorial_link(&self, i: usize) -> String {
        format!(r#"<a href="{}">{}</a>"#, esc(&self.tutorial_url(i)), esc(&self.short_title(i)))
    }

    /// Where a test is read about: its tutorial for a story, its report otherwise.
    fn read_url(&self, i: usize) -> String {
        match self.d.plans[i].origin {
            Origin::Story { .. } => self.tutorial_url(i),
            Origin::Coverage { .. } => self.test_url(i),
        }
    }

    fn exp_link(&self, e: ExpId) -> String {
        format!(r#"<a href="{}">{}</a>"#, esc(&self.exp_url(e)), esc(&self.suite().exp(e).name))
    }

    fn test_link(&self, i: usize) -> String {
        format!(r#"<a href="{}">{}</a>"#, esc(&self.test_url(i)), esc(&self.d.plans[i].title))
    }

    fn section_link(&self, id: SectionId) -> String {
        format!(r#"<a href="{}">{}</a>"#, esc(&self.section_url(id)), esc(&self.suite().section(id).name))
    }

    fn verdict_badge(&self, e: ExpId) -> String {
        let v = self.f.verdict(e);
        let class = match v {
            Some(Verdict::Verified) => "ok",
            Some(Verdict::Failed) => "bad",
            Some(Verdict::Unsupported) => "warn",
            _ => "idle",
        };
        format!(r#"<span class="badge {class}">{}</span>"#, verdict_label(v))
    }

    fn status_badge(&self, s: Status) -> String {
        let class = match s {
            Status::Passed => "ok",
            Status::Failed => "bad",
            Status::Unsupported => "warn",
            _ => "idle",
        };
        format!(r#"<span class="badge {class}">{}</span>"#, status_label(s))
    }

    /// The pages in reading order, for previous/next links.
    fn reading_order(&self) -> Vec<Here> {
        manual::reading_order(self.d)
            .into_iter()
            .map(|r| match r {
                manual::Reading::Section(id) => Here::Section(id),
                manual::Reading::Tutorial(i) => Here::Tutorial(i),
                manual::Reading::Feature(f) => Here::Feature(f),
            })
            .collect()
    }

    /// Prose with its `code` and `[[links]]`.
    fn prose(&self, text: &str) -> String {
        let mut o = String::new();
        for part in manual::inline(text) {
            match part {
                manual::Inline::Text(t) => o.push_str(&esc(t)),
                manual::Inline::Code(c) => {
                    let _ = write!(o, "<code>{}</code>", esc(c));
                }
                manual::Inline::Link(name) => match manual::find(self.d, name) {
                    Some(r) => {
                        let _ = write!(o, r#"<a href="{}">{}</a>"#, esc(&self.ref_url(r)), esc(name));
                    }
                    None => o.push_str(&esc(name)),
                },
            }
        }
        o
    }

    fn ref_url(&self, r: manual::Ref) -> String {
        match r {
            manual::Ref::Section(id) => self.section_url(id),
            manual::Ref::Feature(f) => self.feature_url(f),
            manual::Ref::Exp(e) => self.exp_url(e),
            manual::Ref::Tutorial(i) => self.tutorial_url(i),
            manual::Ref::Term(t) => format!("glossary.html#{}", self.suite().glossary[t].slug),
        }
    }

    /// The spec file a page comes from, for its footer.
    fn source(&self, here: Here) -> Option<String> {
        let s = self.suite();
        let span = match here {
            Here::Section(id) => s.section(id).span.clone()?,
            Here::Feature(f) => s.feature(f).span.clone(),
            Here::Tutorial(i) | Here::Test(i) => match self.d.plans[i].origin {
                Origin::Story { story, .. } => s.story(story).span.clone(),
                Origin::Coverage { target } => s.exp(target).span.clone(),
            },
            _ => return None,
        };
        Some(span.file.to_string())
    }

    /// The written stories' tests in a section (not the generated ones).
    fn stories_in(&self, id: SectionId) -> Vec<usize> {
        (0..self.d.plans.len())
            .filter(|&i| self.d.plans[i].section == id && matches!(self.d.plans[i].origin, Origin::Story { .. }))
            .collect()
    }

    fn short_title(&self, i: usize) -> String {
        manual::short_title(self.suite(), &self.d.plans[i])
    }

    fn here_link(&self, h: Here) -> (String, String) {
        match h {
            Here::Section(id) => (self.section_url(id), self.suite().section(id).name.clone()),
            Here::Feature(id) => (self.feature_url(id), self.suite().feature(id).name.clone()),
            Here::Tutorial(i) => (self.tutorial_url(i), self.short_title(i)),
            Here::Test(i) => (self.test_url(i), self.d.plans[i].title.clone()),
            Here::Tests => ("tests.html".into(), "Test report".into()),
            Here::Map => ("map.html".into(), "Map".into()),
            Here::Glossary => ("glossary.html".into(), "Glossary".into()),
            Here::Vocabulary => ("vocabulary.html".into(), "Test vocabulary".into()),
            Here::Theme => ("theme.html".into(), "Theme".into()),
        }
    }

    /// The section a page sits in, for breadcrumbs and for opening the navigation.
    fn section_of(&self, h: Here) -> Option<SectionId> {
        match h {
            Here::Section(id) => Some(id),
            Here::Feature(f) => Some(self.suite().feature(f).section),
            Here::Test(i) | Here::Tutorial(i) => Some(self.d.plans[i].section),
            _ => None,
        }
    }

    fn nav(&self, here: Here) -> String {
        let open: Vec<SectionId> =
            self.section_of(here).map(|s| self.suite().chain(s).iter().map(|x| x.id).collect()).unwrap_or_default();
        let mut o = String::from(r#"<nav class="side" aria-label="Contents"><ul class="tree">"#);
        self.nav_section(&mut o, SectionId(0), &open, here);
        o.push_str("</ul><ul class=\"tree extra\"><li class=\"label\">Reference</li>");
        // The map and the theme are for the people who write the specs: the test report links them.
        for h in [Here::Glossary, Here::Tests] {
            let (url, name) = self.here_link(h);
            let cur = if h == here { r#" class="current""# } else { "" };
            let _ = write!(o, r#"<li><a href="{url}"{cur}>{name}</a></li>"#);
        }
        o.push_str("</ul></nav>");
        o
    }

    fn nav_section(&self, o: &mut String, id: SectionId, open: &[SectionId], here: Here) {
        let s = self.suite().section(id);
        let cur = |h: Here| if h == here { r#" class="current""# } else { "" };
        let title =
            format!(r#"<a href="{}"{}>{}</a>"#, esc(&self.section_url(id)), cur(Here::Section(id)), esc(&s.name));
        let mut inner = String::new();
        let stories = self.stories_in(id);
        if !stories.is_empty() {
            inner.push_str(r#"<li class="label">Tutorials</li>"#);
            for &i in &stories {
                let _ = write!(
                    inner,
                    r#"<li><a href="{}"{}>{}</a></li>"#,
                    esc(&self.tutorial_url(i)),
                    cur(Here::Tutorial(i)),
                    esc(&self.short_title(i))
                );
            }
            if !s.features.is_empty() {
                inner.push_str(r#"<li class="label">Topics</li>"#);
            }
        }
        for &f in &s.features {
            let _ = write!(
                inner,
                r#"<li><a href="{}"{}>{}</a></li>"#,
                esc(&self.feature_url(f)),
                cur(Here::Feature(f)),
                esc(&self.suite().feature(f).name)
            );
        }
        for &c in &s.children {
            self.nav_section(&mut inner, c, open, here);
        }
        if s.parent.is_none() {
            let _ = write!(o, r#"<li class="root">{title}<ul>{inner}</ul></li>"#);
        } else {
            let is_open = if open.contains(&id) { " open" } else { "" };
            let _ = write!(
                o,
                r#"<li class="section"><details{is_open}><summary>{title}</summary><ul>{inner}</ul></details></li>"#
            );
        }
    }

    fn crumbs(&self, here: Here) -> String {
        let mut parts: Vec<String> = vec![];
        if let Some(sec) = self.section_of(here) {
            for s in self.suite().chain(sec) {
                if here != Here::Section(s.id) {
                    parts.push(self.section_link(s.id));
                }
            }
        } else {
            parts.push(self.section_link(SectionId(0)));
        }
        if parts.is_empty() {
            return String::new();
        }
        format!(r#"<p class="crumbs">{}</p>"#, parts.join(r#" <span>›</span> "#))
    }

    fn summary(&self) -> String {
        let Some(r) = self.d.report else { return "Tests not run".into() };
        let mut parts = vec![format!("{} passed", r.count(Status::Passed))];
        if r.count(Status::Failed) > 0 {
            parts.push(format!("{} failed", r.count(Status::Failed)));
        }
        if r.count(Status::Unsupported) > 0 {
            parts.push(format!("{} not supported yet", r.count(Status::Unsupported)));
        }
        if r.count(Status::NotRun) > 0 {
            parts.push(format!("{} not run", r.count(Status::NotRun)));
        }
        format!("{} tests: {}", r.results.len(), parts.join(" · "))
    }

    fn page(&self, here: Here, title: &str, body: &str) -> String {
        let order = self.reading_order();
        let pos = order.iter().position(|h| *h == here);
        let mut pager = String::new();
        if let Some(p) = pos {
            pager.push_str(r#"<nav class="pager">"#);
            if let Some(prev) = p.checked_sub(1).map(|i| order[i]) {
                let (url, name) = self.here_link(prev);
                let _ =
                    write!(pager, r#"<a class="prev" href="{}"><small>Previous</small>{}</a>"#, esc(&url), esc(&name));
            }
            if let Some(&next) = order.get(p + 1) {
                let (url, name) = self.here_link(next);
                let _ = write!(pager, r#"<a class="next" href="{}"><small>Next</small>{}</a>"#, esc(&url), esc(&name));
            }
            pager.push_str("</nav>");
        }
        let footer = match self.source(here) {
            Some(file) => match self.f.edit_link(&file, 1) {
                Some((url, shown)) => {
                    format!(
                        r#"<footer>Edit this page: <a href="{}"><code>{}</code></a></footer>"#,
                        esc(&url),
                        esc(&shown)
                    )
                }
                None => format!("<footer>Edit this page: <code>{}</code></footer>", esc(&file)),
            },
            None => String::new(),
        };
        let suite = esc(&self.suite().name);
        let full = if title == self.suite().name { suite.clone() } else { format!("{} · {suite}", esc(title)) };
        format!(
            r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{full}</title>
<link rel="stylesheet" href="assets/theme.css">
<link rel="stylesheet" href="assets/style.css">
</head>
<body>
<header class="top"><a class="brand" href="index.html">{suite}</a></header>
<div class="layout">
{}
<main>
{}
{body}
{pager}
{footer}
</main>
</div>
</body>
</html>
"#,
            self.nav(here),
            self.crumbs(here)
        )
    }

    fn gherkin(&self, steps: &[StepText]) -> String {
        let mut o = String::from(r#"<pre class="gherkin">"#);
        for t in steps {
            let and = matches!(t.keyword, Keyword::And | Keyword::But);
            let _ = writeln!(
                o,
                r#"{}<span class="kw">{}</span> {}"#,
                if and { "  " } else { "" },
                t.keyword.as_str(),
                esc(&t.text)
            );
        }
        o.push_str("</pre>");
        o
    }

    /// Facts as Gherkin lines: "Given …", "  And …".
    fn given_lines(&self, facts: &[String]) -> String {
        let mut o = String::from(r#"<pre class="gherkin">"#);
        for (i, f) in facts.iter().enumerate() {
            let kw = if i == 0 { "Given" } else { "  And" };
            let _ = writeln!(o, r#"<span class="kw">{kw}</span> {}"#, esc(f));
        }
        o.push_str("</pre>");
        o
    }

    /// A screenshot, or a placeholder saying why there isn't one: briefly on reading pages
    /// (`detail` false), with the test's reason on the report.
    /// `alt` says what the picture shows, for readers who can't see it: the step's outcome.
    fn figure(&self, shot: Option<&str>, status: Status, detail: bool, alt: &str) -> String {
        let alt = manual::plain(alt);
        let alt = if alt.is_empty() { "Screenshot".to_string() } else { alt };
        match shot {
            Some(shot) => format!(
                r#"<figure><a href="../{0}"><img src="../{0}" alt="{1}" loading="lazy"></a></figure>"#,
                esc(shot),
                esc(&alt)
            ),
            None => {
                let img = if status == Status::Unsupported { NOT_SUPPORTED } else { NO_SCREENSHOT };
                let note = if detail { missing_shot(status) } else { manual::missing(status) };
                format!(
                    r#"<figure class="placeholder"><img src="../{}" alt="{}" loading="lazy"><figcaption>{}</figcaption></figure>"#,
                    esc(img),
                    esc(note),
                    esc(note)
                )
            }
        }
    }

    /// Verdict counts for a set of expectations: "12 verified · 3 not supported yet".
    fn tally(&self, exps: &[ExpId]) -> String {
        let mut counts: Vec<(&str, &str, usize)> = vec![];
        for (v, class) in [
            (Some(Verdict::Verified), "ok"),
            (Some(Verdict::Failed), "bad"),
            (Some(Verdict::Unsupported), "warn"),
            (Some(Verdict::Untested), "idle"),
            (Some(Verdict::NotRun), "idle"),
            (None, "idle"),
        ] {
            let n = exps.iter().filter(|&&e| self.f.verdict(e) == v).count();
            if n > 0 {
                counts.push((verdict_label(v), class, n));
            }
        }
        counts
            .iter()
            .map(|(l, c, n)| format!(r#"<span class="badge {c}">{n} {l}</span>"#))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// A test's first screenshot, for a thumbnail.
    fn thumbnail(&self, i: usize) -> (Option<String>, Status) {
        let status = self.f.status[i];
        let shot = self
            .d
            .report
            .and_then(|r| r.results.get(i))
            .and_then(|r| r.steps.iter().rev().chain(std::iter::once(&r.start)).find_map(|s| s.screenshot.clone()));
        (shot, status)
    }

    fn story_card(&self, i: usize) -> String {
        let plan = &self.d.plans[i];
        let (shot, status) = self.thumbnail(i);
        let img = match &shot {
            Some(s) => format!("../{s}"),
            None if status == Status::Unsupported => format!("../{NOT_SUPPORTED}"),
            None => format!("../{NO_SCREENSHOT}"),
        };
        let about = match plan.origin {
            Origin::Story { story, .. } => {
                let st = self.suite().story(story);
                match paragraphs(&st.description).into_iter().next() {
                    Some(p) => p,
                    None if !st.narrative.is_empty() => narrative(&st.narrative),
                    None => String::new(),
                }
            }
            Origin::Coverage { target } => format!("Generated to reach «{}».", self.suite().exp(target).name),
        };
        let _ = status;
        format!(
            r#"<a class="card story" href="{}"><img src="{}" alt="" loading="lazy"><span class="body"><strong>{}</strong><span>{}</span></span></a>"#,
            esc(&self.read_url(i)),
            esc(&img),
            esc(&self.short_title(i)),
            esc(&manual::plain(&about))
        )
    }

    fn list(&self, class: &str, ordered: bool, items: &[String]) -> String {
        let tag = if ordered { "ol" } else { "ul" };
        let mut o = format!(r#"<{tag} class="{class}">"#);
        for item in items {
            let _ = write!(o, "<li>{}</li>", self.prose(item));
        }
        let _ = write!(o, "</{tag}>");
        o
    }

    fn section_page(&self, id: SectionId) -> String {
        let s = self.suite();
        let section = s.section(id);
        let mut o = format!("<h1>{}</h1>", esc(&section.name));
        for p in &paragraphs(&section.description) {
            let _ = write!(o, "<p class=\"lede\">{}</p>", self.prose(p));
        }

        if let Some(before) = manual::before(self.d, id) {
            o.push_str(r#"<aside class="setup" id="before-you-begin"><h2>Before you begin</h2>"#);
            if let Some(parent) = before.parent {
                let then = if before.route.is_empty() || before.route_is_detail { "" } else { " Then:" };
                let _ = write!(o, "<p>Start from the setup in {}.{then}</p>", self.section_link(parent));
            }
            if !before.items.is_empty() {
                o.push_str(&self.list("before", false, &before.items));
            }
            if !before.route.is_empty() {
                let route = self.list("route", true, &before.route);
                if before.route_is_detail {
                    let _ = write!(o, r#"<details class="route"><summary>Step by step</summary>{route}</details>"#);
                } else {
                    o.push_str(&route);
                }
            }
            o.push_str("</aside>");
        }

        if !section.children.is_empty() {
            o.push_str(r#"<div class="cards">"#);
            for &c in &section.children {
                let child = s.section(c);
                let about = paragraphs(&child.description).into_iter().next().unwrap_or_default();
                let _ = write!(
                    o,
                    r#"<a class="card" href="{}"><strong>{}</strong><span>{}</span></a>"#,
                    esc(&self.section_url(c)),
                    esc(&child.name),
                    esc(&manual::plain(&about))
                );
            }
            o.push_str("</div>");
        }

        // Tutorials anywhere beneath it, in reading order.
        let under = s.subtree(id);
        let tutorials: Vec<usize> =
            manual::tutorials(self.d).into_iter().filter(|&i| under.contains(&self.d.plans[i].section)).collect();
        if !tutorials.is_empty() {
            o.push_str(r#"<h2>Tutorials</h2><div class="cards stories">"#);
            for i in tutorials {
                o.push_str(&self.story_card(i));
            }
            o.push_str("</div>");
        }
        if !section.features.is_empty() {
            o.push_str("<h2>Topics</h2><dl class=\"topics\">");
            for &f in &section.features {
                let feature = s.feature(f);
                let about = paragraphs(&feature.description).into_iter().next().unwrap_or_default();
                let _ = write!(
                    o,
                    r#"<dt><a href="{}">{}</a></dt><dd>{}</dd>"#,
                    esc(&self.feature_url(f)),
                    esc(&feature.name),
                    self.prose(&about)
                );
            }
            o.push_str("</dl>");
        }

        self.page(Here::Section(id), &section.name, &o)
    }

    /// Where an expectation stands among the others: what it follows, what it leads to, and
    /// what it overrides. For the "How this is tested" block.
    fn relations(&self, e: ExpId) -> Vec<(&'static str, String)> {
        let s = self.suite();
        let mut meta = vec![];
        let parents: Vec<String> = self
            .d
            .analysis
            .flow
            .parents(e)
            .map(|p| match p {
                Node::Start => "the section's setup".to_string(),
                Node::Exp(p) => self.exp_link(p),
            })
            .collect();
        match self.d.analysis.flow.level[e.0 as usize] {
            Some(_) => meta.push(("Follows", parents.join(", "))),
            None if self.d.analysis.flow.truncated => {
                meta.push(("Follows", "nothing the search found within its limits".into()))
            }
            None => meta.push(("Unreachable", "nothing leads to a state where it applies".into())),
        }
        let children: Vec<String> = self.d.analysis.flow.children(Node::Exp(e)).map(|c| self.exp_link(c)).collect();
        if !children.is_empty() {
            meta.push(("Leads to", children.join(", ")));
        }
        for c in self.d.analysis.exceptions() {
            let when = esc(&s.describe(&c.situation));
            if c.winner == Some(e) {
                meta.push(("Overrides", format!("{} when {when}", self.exp_link(c.loser().unwrap()))));
            } else if c.loser() == Some(e) {
                meta.push(("Overridden by", format!("{} when {when}", self.exp_link(c.winner.unwrap()))));
            }
        }
        let all = self.f.tested_by(e);
        let mut by: Vec<String> = all
            .iter()
            .take(5)
            .map(|&p| format!("{} {}", self.test_link(p), self.status_badge(self.f.status[p])))
            .collect();
        if all.len() > 5 {
            by.push(format!(r#"<a href="tests.html">{} more</a>"#, all.len() - 5));
        }
        meta.push(("Tested by", if by.is_empty() { "nothing yet".into() } else { by.join(", ") }));
        meta
    }

    fn exception_notes(&self, e: ExpId) -> String {
        let mut o = String::new();
        for note in manual::notes(self.d, e) {
            let text = match note {
                manual::Note::Except { when, see } if when.is_empty() => {
                    format!("<strong>Exception:</strong> see {}.", self.exp_link(see))
                }
                manual::Note::Except { when, see } => {
                    format!("<strong>Except</strong> when {}: see {}.", esc(&when), self.exp_link(see))
                }
                manual::Note::ExceptionTo(other) => format!("An exception to {}.", self.exp_link(other)),
                manual::Note::CanLeave { by, what } => {
                    format!("<strong>Exception:</strong> {} can leave {}.", self.exp_link(by), esc(&what))
                }
            };
            let _ = write!(o, r#"<p class="note">{text}</p>"#);
        }
        o
    }

    fn feature_page(&self, id: FeatureId) -> String {
        let s = self.suite();
        let feature = s.feature(id);
        let mut o = format!("<h1>{}</h1>", esc(&feature.name));
        for p in &paragraphs(&feature.description) {
            let _ = write!(o, "<p class=\"lede\">{}</p>", self.prose(p));
        }
        // Expectations marked `Docs: hidden` are only in the test report.
        let shown: Vec<ExpId> = feature.expectations.iter().copied().filter(|&e| !s.exp(e).hidden).collect();
        if shown.len() > 2 {
            o.push_str(r#"<nav class="toc"><strong>On this page</strong><ul>"#);
            for &e in &shown {
                let x = s.exp(e);
                let _ = write!(o, r##"<li><a href="#{}">{}</a></li>"##, esc(&x.slug), esc(&x.name));
            }
            o.push_str("</ul></nav>");
        }
        for &e in &shown {
            let x = s.exp(e);
            let _ = write!(o, r#"<section class="exp" id="{}"><h2>{}</h2>"#, esc(&x.slug), esc(&x.name));
            for p in &paragraphs(&x.description) {
                let _ = write!(o, "<p>{}</p>", self.prose(p));
            }
            let how = voice::how(s, &x.steps);
            let _ = write!(o, r#"<p class="how">{}</p>"#, self.prose(&how));
            o.push_str(&self.exception_notes(e));
            match self.f.shot_of(self.d, e) {
                Some((_, _, shot)) => o.push_str(&self.figure(Some(&shot), Status::Passed, false, &how)),
                None => {
                    let status = self.f.tested_by(e).first().map_or(Status::NotRun, |&p| self.f.status[p]);
                    o.push_str(&self.figure(None, status, false, ""));
                }
            }
            let tutorials: Vec<String> =
                manual::shown_in(self.d, self.f, e).into_iter().map(|p| self.tutorial_link(p)).collect();
            if !tutorials.is_empty() {
                let _ = write!(o, r#"<p class="see-also">See it in {}.</p>"#, tutorials.join(", "));
            }
            let _ = write!(
                o,
                r#"<details class="tested"><summary>How this is tested {}</summary>{}<dl class="meta">"#,
                self.verdict_badge(e),
                self.gherkin(&x.steps)
            );
            for (k, v) in self.relations(e) {
                let _ = write!(o, "<dt>{k}</dt><dd>{v}</dd>");
            }
            o.push_str("</dl></details></section>");
        }
        self.page(Here::Feature(id), &feature.name, &o)
    }

    fn tutorial_page(&self, i: usize) -> String {
        let s = self.suite();
        let plan = &self.d.plans[i];
        let t = manual::tutorial(self.d, self.f, i).expect("tutorials are stories");
        let mut o = String::new();
        if let Some(k) = &t.kicker {
            let _ = write!(o, r#"<p class="kicker">{}</p>"#, esc(k));
        }
        let _ = write!(o, "<h1>{}</h1>", esc(&t.title));
        for p in &t.intro {
            let _ = write!(o, "<p class=\"lede\">{}</p>", self.prose(p));
        }
        if !t.before.is_empty() {
            o.push_str(r#"<aside class="setup"><h2>Before you begin</h2>"#);
            o.push_str(&self.list("before", false, &t.before));
            if let Some(d) = t.details {
                let _ = write!(
                    o,
                    r##"<p class="muted">{} has <a href="{}#before-you-begin">the setup step by step</a>.</p>"##,
                    esc(&s.section(d).name),
                    esc(&self.section_url(d))
                );
            }
            o.push_str("</aside>");
        }
        let _ = plan;
        o.push_str(r#"<ol class="steps">"#);
        for step in &t.steps {
            let _ = write!(o, r#"<li><p class="do">{}</p>"#, self.prose(&step.doing));
            if !step.seen.is_empty() {
                let _ = write!(o, r#"<p class="see">{}</p>"#, self.prose(&step.seen));
            }
            let alt = if step.seen.is_empty() { &step.doing } else { &step.seen };
            o.push_str(&self.figure(step.shot.as_deref(), step.status, false, alt));
            o.push_str("</li>");
        }
        o.push_str("</ol>");
        if !t.related.is_empty() {
            o.push_str(r#"<h2>Learn more</h2><ul class="related">"#);
            for &f in &t.related {
                let _ =
                    write!(o, r#"<li><a href="{}">{}</a></li>"#, esc(&self.feature_url(f)), esc(&s.feature(f).name));
            }
            o.push_str("</ul>");
        }
        let _ = write!(
            o,
            r#"<details class="tested"><summary>How this is tested {}</summary><p>This tutorial is also an acceptance test. <a href="{}">Its report</a> has every step, check and screenshot from the last run.</p>"#,
            self.status_badge(self.f.status[i]),
            esc(&self.test_url(i))
        );
        if let Some(n) = &t.narrative {
            let _ = write!(o, "<p>The user story: {}</p>", esc(n));
        }
        o.push_str(&self.gherkin(&t.written));
        o.push_str("</details>");
        self.page(Here::Tutorial(i), &t.title, &o)
    }

    /// Every test and what the last run found.
    fn tests_page(&self) -> String {
        let s = self.suite();
        let mut o = String::from(
            "<h1>Test report</h1><p class=\"lede\">Every tutorial is also an acceptance test, and generated tests \
             reach the behaviour no tutorial does. This is what the last run found.</p>",
        );
        let all: Vec<ExpId> = s.expectations.iter().map(|x| x.id).collect();
        let _ = write!(o, r#"<p class="tally">{} · expectations: {}</p>"#, esc(&self.summary()), self.tally(&all));
        o.push_str(r#"<p>The tests use the states, actions and shorthand of the <a href="vocabulary.html">test vocabulary</a>. <a href="map.html">The map</a> shows which behaviour leads to which, and <a href="theme.html">the theme</a> shows the colours and type that the docs and the console share.</p>"#);
        for section in &s.sections {
            let tests: Vec<usize> =
                (0..self.d.plans.len()).filter(|&i| self.d.plans[i].section == section.id).collect();
            if tests.is_empty() {
                continue;
            }
            let chain: Vec<String> = s.chain(section.id).iter().map(|x| x.name.clone()).collect();
            let _ = write!(
                o,
                "<h2>{}</h2><table class=\"tests\"><thead><tr><th>Test</th><th>Kind</th><th>Result</th></tr></thead><tbody>",
                esc(&chain.join(" › "))
            );
            for i in tests {
                let kind = match self.d.plans[i].origin {
                    Origin::Story { .. } => format!(r#"<a href="{}">tutorial</a>"#, esc(&self.tutorial_url(i))),
                    Origin::Coverage { .. } => "generated".into(),
                };
                let _ = write!(
                    o,
                    "<tr><td>{}</td><td>{kind}</td><td>{}</td></tr>",
                    self.test_link(i),
                    self.status_badge(self.f.status[i])
                );
            }
            o.push_str("</tbody></table>");
        }
        let open: Vec<ExpId> =
            all.iter().copied().filter(|&e| self.f.verdict(e).is_some_and(|v| v != Verdict::Verified)).collect();
        if !open.is_empty() {
            o.push_str("<h2>Not verified yet</h2><ul class=\"exps\">");
            for e in open {
                let _ = write!(o, "<li>{} {}</li>", self.exp_link(e), self.verdict_badge(e));
            }
            o.push_str("</ul>");
        }
        self.page(Here::Tests, "Test report", &o)
    }

    fn test_page(&self, i: usize) -> String {
        let s = self.suite();
        let plan = &self.d.plans[i];
        let result = self.d.report.and_then(|r| r.results.get(i));
        let status = self.f.status[i];
        let mut o = format!(
            r#"<p class="kicker"><a href="tests.html">Test report</a></p><h1>{} {}</h1>"#,
            esc(&plan.title),
            self.status_badge(status)
        );
        match plan.origin {
            Origin::Story { story, .. } => {
                let st = s.story(story);
                if !st.narrative.is_empty() {
                    let _ = write!(o, "<blockquote>{}</blockquote>", esc(&narrative(&st.narrative)));
                }
                let _ = write!(
                    o,
                    r#"<p class="lede">The test behind the tutorial {}: every step it takes, what it checks and what it saw.</p>"#,
                    self.tutorial_link(i)
                );
            }
            Origin::Coverage { target } => {
                let _ = write!(
                    o,
                    "<p class=\"lede\">Generated. No story reaches {}, so this test takes the shortest route there from the setup of {}.</p>",
                    self.exp_link(target),
                    self.section_link(plan.section)
                );
            }
        }
        if let Some(e) = result.and_then(|r| r.start.error.as_ref()) {
            let class = if status == Status::Unsupported { "note warn" } else { "note bad" };
            let _ = write!(o, r#"<p class="{class}">{}</p>"#, esc(e));
        }

        // The section's setup is the page's context; what the test adds to it is the news.
        o.push_str(r#"<section class="step"><h2>Start</h2>"#);
        let section = s.section(plan.section);
        let own: Vec<String> = plan
            .start
            .iter()
            .filter(|(v, c)| section.start.get(*v) != Some(*c))
            .map(|(v, c)| s.vocabulary.var(v).describe(c))
            .collect();
        let _ = write!(
            o,
            "<p>From the setup of {}{}</p>",
            self.section_link(plan.section),
            if own.is_empty() { "." } else { ", and:" }
        );
        if !own.is_empty() {
            o.push_str(&self.given_lines(&own));
        }
        let _ = write!(
            o,
            r#"<details class="setup"><summary>Everything it starts with</summary>{}</details>"#,
            self.given_lines(&s.vocabulary.facts(&plan.start))
        );
        o.push_str(&self.step_body(&plan.start_checks, result.map(|r| &r.start), status));
        o.push_str("</section>");
        for (n, step) in plan.steps.iter().enumerate() {
            let _ = write!(
                o,
                r#"<section class="step"><h2><span class="n">{}</span> {}</h2>"#,
                n + 1,
                esc(&action_heading(&step.text))
            );
            o.push_str(&self.gherkin(&step.text));
            if !step.fired.is_empty() {
                let fired: Vec<String> = step.fired.iter().map(|&e| self.exp_link(e)).collect();
                let _ = write!(o, "<p class=\"muted\">Expected behaviour: {}.</p>", fired.join(", "));
            }
            o.push_str(&self.step_body(&step.checks, result.and_then(|r| r.steps.get(n)), status));
            o.push_str("</section>");
        }
        self.page(Here::Test(i), &plan.title, &o)
    }

    fn step_body(&self, checks: &[Check], result: Option<&StepResult>, plan_status: Status) -> String {
        let status = result.map_or(plan_status, |r| r.status);
        let mut o = self.figure(result.and_then(|r| r.screenshot.as_deref()), status, true, "");
        if let Some(e) = result.and_then(|r| r.error.as_ref()).filter(|_| status == Status::Failed) {
            let _ = write!(o, r#"<p class="note bad">{}</p>"#, esc(e));
        }
        o.push_str("<ul class=\"checks\">");
        let mut arranged = 0;
        for (k, c) in checks.iter().enumerate() {
            let cr = result.and_then(|r| r.checks.get(k));
            if c.because == Because::Arranged && cr.is_some_and(|r| r.passed) {
                arranged += 1;
                continue;
            }
            o.push_str(&self.check_item(c, cr));
        }
        if arranged > 0 {
            let _ = write!(o, r#"<li class="ok">Every arranged state read back as arranged ({arranged})</li>"#);
        }
        o.push_str("</ul>");
        o
    }

    fn check_item(&self, c: &Check, r: Option<&CheckResult>) -> String {
        let var = self.suite().vocabulary.var(c.var);
        let class = match r {
            Some(r) if r.passed => "ok",
            Some(_) => "bad",
            None => "idle",
        };
        let why = match &c.because {
            Because::Arranged => "arranged".to_string(),
            Because::Expectation(e) => self.exp_link(*e),
            Because::Story(_) => "the story".to_string(),
        };
        let seen = match r {
            Some(r) if !r.passed => match (&r.observed, &r.error) {
                (_, Some(e)) => format!(" — couldn't observe it: {}", esc(e)),
                (Some(v), _) => format!(" — observed <strong>{}</strong>", esc(&v.to_string())),
                (None, _) => String::new(),
            },
            _ => String::new(),
        };
        format!(
            r#"<li class="{class}">{} <span class="muted">· {why}</span>{seen}</li>"#,
            esc(&var.describe(&c.expect))
        )
    }

    fn map_page(&self) -> String {
        let s = self.suite();
        let mut o = String::from(
            "<h1>Map</h1><p class=\"lede\">Every expectation, placed after the ones that get the system into a state where it applies, starting from each section's setup. Dashed arrows are exceptions, red ones are conflicts nobody has settled, and dashed outlines are invariants. Colours show each expectation's verdict.</p>",
        );
        o.push_str(r#"<p class="legend"><span class="badge ok">verified</span> <span class="badge bad">failed</span> <span class="badge warn">not supported yet</span> <span class="badge idle">untested</span></p>"#);
        for section in &s.sections {
            let own: Vec<ExpId> = section.features.iter().flat_map(|&f| s.feature(f).expectations.clone()).collect();
            if own.is_empty() {
                continue;
            }
            let chain: Vec<String> = s.chain(section.id).iter().map(|x| x.name.clone()).collect();
            let map = crate::map::svg(self.d, self.f, &own, &|e| self.exp_url(e));
            let _ = write!(o, r#"<h2>{}</h2><div class="map">{map}</div>"#, esc(&chain.join(" › ")));
        }
        self.page(Here::Map, "Map", &o)
    }

    fn vocabulary_page(&self) -> String {
        let s = self.suite();
        let v = &s.vocabulary;
        let mut o = String::from(
            "<p class=\"kicker\"><a href=\"tests.html\">Test report</a></p><h1>Test vocabulary</h1><p class=\"lede\">The \
             terms the tests are written in: the states the system can be in, the things that can happen, and the \
             shorthand for a few of them together. The docs word each one with its <code>Say:</code>, <code>Set:</code> \
             and <code>Shows:</code>.</p>",
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
                let _ = write!(o, "<h2>{}</h2>", self.section_link(g));
            }
            if !vars.is_empty() {
                o.push_str("<h3>States</h3><table><thead><tr><th>State</th><th>Can be</th><th>To change it</th></tr></thead><tbody>");
                for var in vars {
                    let set = voice::how_to_set(var).map(|t| voice::sentence(&t)).unwrap_or_default();
                    let _ = write!(
                        o,
                        "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                        esc(&var.subject),
                        esc(&values(var)),
                        esc(&set)
                    );
                }
                o.push_str("</tbody></table>");
            }
            if !events.is_empty() {
                o.push_str("<h3>Actions</h3><table><thead><tr><th>Action</th><th>What to do</th></tr></thead><tbody>");
                for ev in events {
                    let what = match voice::event(ev) {
                        voice::Action::Do(d) => voice::sentence(&d),
                        voice::Action::Happens(_) => "Nothing: it happens by itself.".into(),
                    };
                    let _ = write!(o, "<tr><td>{}</td><td>{}</td></tr>", esc(&ev.text), esc(&what));
                }
                o.push_str("</tbody></table>");
            }
            if !phrases.is_empty() {
                o.push_str("<h3>Shorthand</h3><table><thead><tr><th>Phrase</th><th>Means</th></tr></thead><tbody>");
                for p in phrases {
                    let means = match &p.body {
                        PhraseBody::Setup(facts) => {
                            let f: Vec<String> = facts.iter().map(|f| v.var(f.var).describe(&f.constraint)).collect();
                            expectation_model::text::join_and(&f)
                        }
                        PhraseBody::Actions(actions) => {
                            let a: Vec<String> = actions.iter().map(|t| s.describe_trigger(t)).collect();
                            a.join(", then ")
                        }
                    };
                    let say = p.say.as_deref().map(voice::sentence).unwrap_or_default();
                    let _ = write!(
                        o,
                        "<tr><td>{}</td><td>{} <span class=\"muted\">({})</span></td></tr>",
                        esc(&p.name),
                        esc(&say),
                        esc(&means)
                    );
                }
                o.push_str("</tbody></table>");
            }
        }
        self.page(Here::Vocabulary, "Test vocabulary", &o)
    }
}

impl Site<'_> {
    /// The terms the manual uses, defined.
    fn glossary_page(&self) -> String {
        let s = self.suite();
        let mut o = String::from(
            "<h1>Glossary</h1><p class=\"lede\">What the words in this manual mean.</p><dl class=\"glossary\">",
        );
        for i in manual::terms(s) {
            let t = &s.glossary[i];
            let _ = write!(o, r#"<dt id="{}">{}</dt><dd>"#, esc(&t.slug), esc(&t.name));
            for p in paragraphs(&t.description) {
                let _ = write!(o, "<p>{}</p>", self.prose(&p));
            }
            o.push_str("</dd>");
        }
        o.push_str("</dl>");
        self.page(Here::Glossary, "Glossary", &o)
    }

    /// The shared theme itself: both palettes, and the console's controls in each.
    fn theme_page(&self) -> String {
        use design_theme::{DAY, NIGHT, geometry};
        let mut o = String::from(
            "<h1>Theme</h1><p class=\"lede\">The look the ground station, the sim's HUD and these docs share, from \
             <code>domains/design/theme</code>. Night is the console: near-black screens in olive bezels, phosphor-green \
             symbology, amber cautions and red warnings. Day is the same instrument in sunlight, and these docs use it in a \
             light browser. Colours are roles, so what each one is for is the same in both.</p>",
        );
        o.push_str("<h2>The console's controls</h2><div class=\"previews\">");
        for (class, name) in [("theme-night", "Night"), ("theme-day", "Day")] {
            let _ = write!(
                o,
                r#"<div class="preview {class}">
<div class="panel"><div class="title">{name} · Vehicle command</div><div class="screen"><div class="row"><span class="btn">FBWA</span><span class="btn active">AUTO</span><span class="btn caution">UPLOAD 5</span><span class="btn danger">DISARM</span></div></div></div>
<div class="panel"><div class="title">Caution / warning</div><div class="screen"><div class="row"><span class="lamp ok">LINK</span><span class="lamp ok">GPS</span><span class="lamp caution">BATT</span><span class="lamp warn">STALL</span><span class="lamp off">FENCE</span></div></div></div>
<div class="panel"><div class="title">Messages</div><div class="screen"><div class="ink-muted">12:04:31 Mission: 5 items</div><div class="ink-ok">Armed, AUTO</div><div class="ink-caution">Battery 3.55 V per cell</div><div class="ink-warning">Stall warning</div><div class="ink-info">GUIDED to 47.6201, -122.5224</div><div class="ink-highlight">Next: waypoint 2</div></div></div>
<div class="panel"><div class="title">In the docs</div><div class="screen"><div class="row"><span class="badge ok">verified</span><span class="badge warn">not supported yet</span><span class="badge bad">failed</span><span class="badge idle">not run</span></div><pre class="gherkin"><span class="kw">Given</span> the map tool is PLAN
<span class="kw">When</span> the operator clicks the map
<span class="kw">Then</span> the plan goes up by 1</pre></div></div>
</div>"#
            );
        }
        o.push_str("</div>");
        for p in [NIGHT, DAY] {
            let _ = write!(
                o,
                "<h2>{} palette</h2><div class=\"swatches\">",
                if p.name == "night" { "Night" } else { "Day" }
            );
            for (name, c) in p.roles() {
                let _ = write!(
                    o,
                    r#"<div class="swatch"><div class="chip" style="background:{0}"></div><div class="meta"><strong>{name}</strong><span>{0} · {1:.1}:1 on screen</span></div></div>"#,
                    c.css(),
                    c.contrast(p.screen)
                );
            }
            o.push_str("</div>");
        }
        let _ = write!(
            o,
            "<h2>Type and shape</h2><table><tbody>\
             <tr><td>Instruments, labels, headings, code</td><td><code>{}</code></td></tr>\
             <tr><td>Prose in the docs</td><td style=\"font-family: var(--font-sans)\">{}</td></tr>\
             <tr><td>Corners</td><td>window {} · panel {} · screen {} · control {} (each inset steps the radius down by the inset)</td></tr>\
             <tr><td>Spacing</td><td>gap {} · bezel padding {}</td></tr></tbody></table>\
             <p class=\"muted\">Every ink reads at 4.5:1 or better on every surface it sits on, in both variants. The theme's tests check it.</p>",
            esc(design_theme::typography::MONO),
            esc(design_theme::typography::SANS),
            geometry::WINDOW_RADIUS,
            geometry::PANEL_RADIUS,
            geometry::SCREEN_RADIUS,
            geometry::CONTROL_RADIUS,
            geometry::GAP,
            geometry::BEZEL_PAD,
        );
        self.page(Here::Theme, "Theme", &o)
    }
}

/// A 16:9 stand-in for a screenshot that doesn't exist: a dark screen in a bezel, in the
/// theme's colours, following the reader's light or dark preference.
fn placeholder(title: &str, subtitle: &str) -> String {
    let (d, n) = (design_theme::DAY, design_theme::NIGHT);
    let vars = |p: &design_theme::Palette| {
        format!(
            "--bezel:{};--edge:{};--screen:{};--stripe:{};--icon:{};--text:{};--muted:{};",
            p.bezel.css(),
            p.bezel_edge.css(),
            p.screen.css(),
            p.canvas.css(),
            p.caution.css(),
            p.text.css(),
            p.muted.css()
        )
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1280 720" width="1280" height="720">
<style>
svg {{ {day} }}
@media (prefers-color-scheme: dark) {{ svg {{ {night} }} }}
.bezel {{ fill: var(--bezel); }} .screen {{ fill: url(#stripes); }} .edge {{ fill: none; stroke: var(--edge); stroke-width: 2; }}
.s1 {{ fill: var(--screen); }} .s2 {{ fill: var(--stripe); }} .icon {{ fill: none; stroke: var(--icon); stroke-width: 10; stroke-linecap: round; stroke-linejoin: round; }}
.title {{ fill: var(--text); font: 700 40px {mono}; }} .sub {{ fill: var(--muted); font: 24px {mono}; }}
</style>
<defs><pattern id="stripes" width="28" height="28" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><rect class="s1" width="28" height="28"/><rect class="s2" width="6" height="28"/></pattern></defs>
<rect class="bezel" width="1280" height="720"/>
<rect class="screen" x="12" y="12" width="1256" height="696" rx="10"/>
<rect class="edge" x="12" y="12" width="1256" height="696" rx="10"/>
<g class="icon" transform="translate(640 300)">
<path d="M-62 -34h28l12-18h44l12 18h28a12 12 0 0 1 12 12v66a12 12 0 0 1-12 12h-112a12 12 0 0 1-12-12v-66a12 12 0 0 1 12-12z"/>
<circle cx="0" cy="10" r="26"/>
<path d="M-80 -70L80 74"/>
</g>
<text class="title" x="640" y="440" text-anchor="middle">{title}</text>
<text class="sub" x="640" y="482" text-anchor="middle">{subtitle}</text>
</svg>
"##,
        day = vars(&d),
        night = vars(&n),
        mono = design_theme::typography::MONO.replace('"', "'"),
    )
}

/// The site's components on the theme's tokens (assets/theme.css). The console's look: mono
/// headings and labels, badges like annunciator lamps, Gherkin on a screen, screenshots
/// framed in a bezel with concentric corners.
const STYLE: &str = r#"* { box-sizing: border-box; }
body { margin: 0; background: var(--canvas); color: var(--text); font: 16px/1.6 var(--font-sans); }
a { color: var(--info); text-decoration: none; }
a:hover { text-decoration: underline; }
code, pre { font-family: var(--font-mono); font-size: 14px; }
h1, h2, h3, header.top, nav.side, .badge, .crumbs, figcaption, dl.meta dt, nav.pager small, th, .label { font-family: var(--font-mono); }

header.top { position: sticky; top: 0; z-index: 3; display: flex; flex-wrap: wrap; gap: 6px 18px; align-items: baseline; padding: 11px 22px; background: var(--title); border-bottom: 1px solid var(--bezel-edge); }
header.top .brand { font-weight: 700; color: var(--text); font-size: 16px; letter-spacing: .02em; }
header.top .summary { color: var(--muted); font-size: 13px; }

.layout { display: grid; grid-template-columns: 300px minmax(0, 1fr); }
nav.side { position: sticky; top: 47px; align-self: start; height: calc(100vh - 47px); overflow: auto; padding: 18px 14px 40px; background: var(--title); border-right: 1px solid var(--bezel-edge); font-size: 13px; }
.tree, .tree ul { list-style: none; margin: 0; padding: 0; }
.tree ul { padding-left: 14px; border-left: 1px solid var(--bezel-edge); margin-left: 6px; }
.tree li { margin: 1px 0; }
.tree a { display: block; padding: 3px 8px; border-radius: var(--radius-control); color: var(--text); border: 1px solid transparent; }
.tree a:hover { background: var(--control-hover); text-decoration: none; }
.tree a.current { background: var(--ok-fill); color: var(--ok); border-color: var(--ok-dim); font-weight: 700; }
.tree li.root > a { font-weight: 700; }
.tree details > summary { list-style: none; cursor: pointer; display: flex; align-items: center; }
.tree details > summary::-webkit-details-marker { display: none; }
.tree details > summary::before { content: ""; flex: none; width: 0; height: 0; margin: 0 5px 0 3px; border: 4px solid transparent; border-left: 6px solid var(--muted); border-right: 0; transition: transform .15s; }
.tree details[open] > summary::before { transform: rotate(90deg); }
.tree details > summary > a { flex: 1; font-weight: 700; }
.tree li.label { color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: .08em; padding: 10px 8px 2px; }
.tree.extra { margin-top: 18px; padding-top: 12px; border-top: 1px solid var(--bezel-edge); }

main { min-width: 0; max-width: 940px; padding: 26px 44px 60px; }
h1 { font-size: 28px; line-height: 1.25; margin: 6px 0 14px; display: flex; flex-wrap: wrap; align-items: center; gap: 10px; letter-spacing: -.01em; }
h2 { font-size: 19px; margin: 36px 0 12px; display: flex; flex-wrap: wrap; align-items: center; gap: 10px; }
h3 { font-size: 15px; margin: 0 0 6px; }
.lede { font-size: 17px; }
.muted { color: var(--muted); }
.crumbs { margin: 0; font-size: 12px; color: var(--muted); text-transform: uppercase; letter-spacing: .06em; }
.crumbs span { margin: 0 4px; }
blockquote { margin: 0 0 16px; padding: 10px 16px; border-left: 3px solid var(--ok-dim); background: var(--bezel); border-radius: 0 var(--radius-screen) var(--radius-screen) 0; font-style: italic; }

/* Badges are annunciator lamps. */
.badge { display: inline-block; font-size: 11px; font-weight: 700; line-height: 1; padding: 5px 7px; border-radius: var(--radius-control); white-space: nowrap; vertical-align: middle; text-transform: uppercase; letter-spacing: .06em; border: 1px solid; }
.badge.ok { color: var(--ok); background: var(--ok-fill); border-color: var(--ok-dim); }
.badge.bad { color: var(--on-warning); background: var(--warning-fill); border-color: var(--warning); }
.badge.warn { color: var(--caution); background: var(--caution-fill); border-color: var(--caution); }
.badge.idle { color: var(--muted); background: var(--off-fill); border-color: var(--off-edge); }
.tally { display: flex; flex-wrap: wrap; gap: 6px; }

.setup { background: var(--screen); border: 1px solid var(--bezel-edge); border-radius: var(--radius-panel); padding: 0 18px 10px; margin: 20px 0; overflow: hidden; }
.setup h2 { font-size: 13px; margin: 0 -18px 10px; padding: 7px 18px; gap: 8px; background: var(--title); border-bottom: 1px solid var(--bezel-edge); text-transform: uppercase; letter-spacing: .12em; color: var(--muted); }
.setup h2::before { content: ""; width: 8px; height: 8px; border-radius: 50%; background: var(--ok); box-shadow: 0 0 6px var(--ok-dim); }
details.setup { padding: 10px 18px; background: var(--bezel); }
details.setup summary { cursor: pointer; color: var(--muted); }
ol.route li { margin: 4px 0; }
ol.route .kw { color: var(--ok); font-weight: 700; font-family: var(--font-mono); font-size: 14px; }

pre.gherkin { background: var(--screen); color: var(--text); border: 1px solid var(--bezel-edge); border-radius: var(--radius-screen); padding: 12px 16px; overflow-x: auto; line-height: 1.6; margin: 12px 0; }
pre.gherkin .kw { color: var(--ok); font-weight: 700; }

.cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(250px, 1fr)); gap: 14px; }
.card { display: flex; flex-direction: column; gap: 6px; padding: 14px 16px; background: var(--bezel); border: 1px solid var(--bezel-edge); border-radius: var(--radius-panel); color: var(--text); }
.card:hover { border-color: var(--ok-dim); text-decoration: none; }
.card > strong { font-family: var(--font-mono); }
.card > span { color: var(--muted); font-size: 14px; }
/* A card's summary is its first lines, not the whole paragraph. */
.card > span, .card.story .body > span { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 4; line-clamp: 4; overflow: hidden; }
.card.story { padding: var(--bezel-pad); gap: 0; }
.card.story img { width: 100%; aspect-ratio: 16 / 9; object-fit: cover; display: block; border-radius: var(--radius-screen); background: var(--screen); }
.card.story .body { display: flex; flex-direction: column; gap: 6px; padding: 10px 9px 9px; align-items: flex-start; color: var(--muted); font-size: 14px; }
.card.story .body strong { color: var(--text); font-size: 14px; font-family: var(--font-mono); }

.kicker { margin: 0; font: 700 12px var(--font-mono); color: var(--muted); text-transform: uppercase; letter-spacing: .06em; }
ul.before { margin: 6px 0 10px; padding-left: 20px; }
ul.before li, ul.related li { margin: 3px 0; }
dl.topics dt { font-family: var(--font-mono); font-weight: 700; margin-top: 14px; }
dl.topics dd { margin: 2px 0 0; color: var(--muted); }
ul.exps, ul.related { margin: 0; padding-left: 18px; }
ul.exps li { margin: 4px 0; }
dl.glossary dt { font-family: var(--font-mono); font-weight: 700; margin-top: 18px; scroll-margin-top: 60px; }
dl.glossary dd { margin: 4px 0 0; }
dl.glossary dd p { margin: 0 0 8px; }

/* A tutorial's steps, numbered like the console's waypoints. */
ol.steps { list-style: none; counter-reset: step; padding: 0; margin: 26px 0; }
ol.steps > li { counter-increment: step; position: relative; padding-left: 46px; margin: 0 0 30px; }
ol.steps > li::before { content: counter(step); position: absolute; left: 0; top: 0; display: grid; place-items: center; width: 30px; height: 30px; border-radius: var(--radius-control); background: var(--ok-fill); color: var(--ok); border: 1px solid var(--ok-dim); font: 700 14px var(--font-mono); }
ol.steps p.do { font-size: 17px; font-weight: 600; margin: 2px 0 4px; }
ol.steps p.see { margin: 0 0 8px; }
p.how { font-size: 17px; }
.see-also { color: var(--muted); font-size: 15px; }
details.tested { margin: 14px 0; padding: 8px 16px; background: var(--bezel); border: 1px solid var(--bezel-edge); border-radius: var(--radius-panel); }
details.tested > summary { cursor: pointer; color: var(--muted); font: 12px var(--font-mono); text-transform: uppercase; letter-spacing: .06em; }
details.tested[open] > summary { margin-bottom: 8px; }

nav.toc { background: var(--bezel); border: 1px solid var(--bezel-edge); border-radius: var(--radius-panel); padding: 10px 18px; margin: 18px 0; font-size: 15px; }
nav.toc strong { font-family: var(--font-mono); font-size: 12px; text-transform: uppercase; letter-spacing: .06em; color: var(--muted); }
nav.toc ul { margin: 6px 0 0; padding-left: 18px; }

section.exp { border-top: 1px solid var(--bezel-edge); margin-top: 30px; }
section.exp:first-of-type { border-top: 0; }
/* A screenshot is a screen in a bezel: concentric corners, as on the console. */
figure { margin: 14px 0; max-width: 870px; padding: var(--bezel-pad); background: var(--bezel); border: 1px solid var(--bezel-edge); border-radius: var(--radius-panel); }
figure img { display: block; width: 100%; aspect-ratio: 16 / 9; object-fit: cover; border-radius: var(--radius-screen); background: var(--screen); }
figcaption { color: var(--muted); font-size: 12px; padding: 7px 4px 2px; }
/* A missing screenshot is a gap to note, not something to look at: keep it small. */
figure.placeholder { max-width: 360px; }
dl.meta { display: grid; grid-template-columns: max-content 1fr; gap: 4px 16px; font-size: 15px; margin: 12px 0 0; }
dl.meta dt { color: var(--muted); font-size: 12px; text-transform: uppercase; letter-spacing: .06em; padding-top: 2px; }
dl.meta dd { margin: 0; }

section.step { margin-top: 28px; }
section.step h2 .n { display: inline-grid; place-items: center; min-width: 28px; height: 28px; padding: 0 6px; border-radius: var(--radius-control); background: var(--ok-fill); color: var(--ok); border: 1px solid var(--ok-dim); font-size: 14px; }
ul.checks { list-style: none; padding: 0; margin: 10px 0; }
ul.checks li { padding: 6px 10px 6px 30px; position: relative; border-radius: var(--radius-control); }
ul.checks li::before { position: absolute; left: 10px; font-weight: 700; font-family: var(--font-mono); }
ul.checks li.ok::before { content: "✓"; color: var(--ok); }
ul.checks li.bad { background: var(--warning-fill); color: var(--on-warning); }
ul.checks li.bad::before { content: "✕"; color: var(--warning); }
ul.checks li.idle::before { content: "○"; color: var(--muted); }
.note { margin: 12px 0; padding: 10px 14px; border-radius: var(--radius-screen); background: var(--bezel); border: 1px solid var(--bezel-edge); border-left: 3px solid var(--caution); }
.note.warn { background: var(--caution-fill); color: var(--caution); border-color: var(--caution); }
.note.bad { background: var(--warning-fill); color: var(--on-warning); border-color: var(--warning); }

/* The DAG is drawn on the tactical map's screen. */
.map { overflow-x: auto; background: var(--map); border: 1px solid var(--bezel-edge); border-radius: var(--radius-panel); padding: 8px; margin: 10px 0 20px; }
svg.dag { display: block; font: 12px var(--font-mono); }
svg.dag .edge { fill: none; stroke: var(--muted); stroke-width: 1.3; opacity: .7; }
svg.dag .override { fill: none; stroke: var(--caution); stroke-width: 1.5; stroke-dasharray: 5 4; }
svg.dag .conflict { fill: none; stroke: var(--warning); stroke-width: 2; }
svg.dag .head { fill: var(--muted); }
svg.dag .node rect { fill: var(--off-fill); stroke: var(--off-edge); stroke-width: 1.5; }
svg.dag .node text { fill: var(--text); text-anchor: middle; }
svg.dag .node.start rect { fill: var(--screen); stroke: var(--bezel-edge); }
svg.dag .node.v-ok rect { fill: var(--ok-fill); stroke: var(--ok); }
svg.dag .node.v-bad rect { fill: var(--warning-fill); stroke: var(--warning); }
svg.dag .node.v-warn rect { fill: var(--caution-fill); stroke: var(--caution); }
svg.dag .node.invariant rect { stroke-dasharray: 4 3; }
svg.dag .node.unreachable { opacity: .55; }
svg.dag .node.outside { opacity: .5; }
svg.dag .node.outside rect { stroke-dasharray: 2 3; }
svg.dag a:hover rect { stroke-width: 2.5; }

table { border-collapse: separate; border-spacing: 0; width: 100%; font-size: 15px; background: var(--bezel); border: 1px solid var(--bezel-edge); border-radius: var(--radius-panel); overflow: hidden; }
th, td { text-align: left; padding: 8px 12px; border-bottom: 1px solid var(--bezel-edge); vertical-align: top; }
tr:last-child td { border-bottom: 0; }
th { color: var(--muted); font-weight: 700; background: var(--title); font-size: 12px; text-transform: uppercase; letter-spacing: .06em; }

nav.pager { display: flex; justify-content: space-between; gap: 12px; margin-top: 48px; }
nav.pager a { flex: 1; max-width: 48%; padding: 10px 14px; border: 1px solid var(--bezel-edge); border-radius: var(--radius-panel); background: var(--bezel); }
nav.pager a.next { text-align: right; margin-left: auto; }
nav.pager small { display: block; color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: .06em; }
footer { margin-top: 40px; color: var(--muted); font-size: 13px; }

/* The theme page: swatches, and the console's controls as the ground station draws them. */
.swatches { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 8px; margin: 10px 0 24px; }
.swatch { border: 1px solid var(--bezel-edge); border-radius: var(--radius-screen); overflow: hidden; background: var(--bezel); font-family: var(--font-mono); font-size: 12px; }
.swatch .chip { height: 44px; border-bottom: 1px solid var(--bezel-edge); }
.swatch .meta { padding: 6px 8px; display: flex; flex-direction: column; }
.swatch .meta span { color: var(--muted); }
.previews { display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: var(--gap); }
.preview { background: var(--canvas); color: var(--text); padding: var(--gap); border-radius: var(--radius-window); border: 1px solid var(--bezel-edge); }
.preview .panel { background: var(--bezel); border-radius: var(--radius-panel); padding: var(--bezel-pad); }
.preview .panel + .panel { margin-top: var(--gap); }
.preview .title { font: 700 11px var(--font-mono); color: var(--muted); text-transform: uppercase; letter-spacing: .08em; padding: 4px 6px 6px; }
.preview .screen { background: var(--screen); border-radius: var(--radius-screen); padding: 10px 12px; font: 13px/1.7 var(--font-mono); }
.preview .row { display: flex; flex-wrap: wrap; gap: 6px; padding: 4px 2px; }
.btn, .lamp { font: 700 12px var(--font-mono); padding: 4px 9px; border-radius: var(--radius-control); border: 1px solid; }
.btn { background: var(--control); color: var(--text); border-color: var(--bezel-edge); }
.btn.active { background: var(--ok-fill); color: var(--ok); border-color: var(--ok-dim); }
.btn.caution { background: var(--caution-fill); color: var(--caution); border-color: var(--caution); }
.btn.danger { background: var(--warning-fill); color: var(--on-warning); border-color: var(--warning); }
.lamp { min-width: 52px; text-align: center; }
.lamp.off { background: var(--off-fill); color: var(--faint); border-color: var(--off-edge); }
.lamp.ok { background: var(--lamp-ok); color: var(--ok); border-color: var(--ok-dim); }
.lamp.caution { background: var(--caution); color: var(--canvas); border-color: var(--caution); }
.lamp.warn { background: var(--lamp-warning); color: var(--on-warning); border-color: var(--warning); }
.ink-ok { color: var(--ok); } .ink-caution { color: var(--caution); } .ink-warning { color: var(--warning); } .ink-info { color: var(--info); } .ink-highlight { color: var(--highlight); } .ink-muted { color: var(--muted); }

@media (max-width: 860px) {
  .layout { grid-template-columns: 1fr; }
  nav.side { position: static; height: auto; max-height: 50vh; border-right: 0; border-bottom: 1px solid var(--bezel-edge); }
  main { padding: 18px 16px 40px; }
  h1 { font-size: 23px; }
}
"#;
