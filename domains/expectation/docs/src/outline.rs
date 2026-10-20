//! The manual's outline: the order the reader meets the chapters in, grouped into parts, with
//! hand-written pages (install, controls, reference, troubleshooting) among the generated
//! ones. The sections stay as the specs have them, because their setups inherit from their
//! parents; the outline only says where each top-level chapter goes.
//!
//! `outline.txt` in the suite's directory:
//!
//! ```text
//! Part: Start here
//!   Page: installing          expectations/manual/installing.md
//!   Page: the-launcher [desktop]
//! Part: Flying
//!   Section: The simulator
//! ```
//!
//! A page's title is its first `# ` line. `[desktop]` limits a page to one platform. A
//! top-level section that no part names goes into a last part, "More".

use expectation_model::text::slug;
use expectation_model::{SectionId, Suite};
use std::path::Path;

pub struct Outline {
    pub parts: Vec<Part>,
    pub pages: Vec<Authored>,
}

pub struct Part {
    pub name: String,
    pub items: Vec<Item>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Item {
    Section(SectionId),
    Page(usize),
}

/// A page written by hand, in Markdown, with `[[links]]` into the generated pages.
pub struct Authored {
    pub slug: String,
    pub title: String,
    pub source: String,
    /// The part it is in.
    pub part: usize,
}

impl Authored {
    /// The first paragraph, for contents lists.
    pub fn about(&self) -> String {
        let mut para: Vec<&str> = vec![];
        for line in self.source.lines().skip(1) {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("```") || line.starts_with('|') {
                if para.is_empty() {
                    continue;
                }
                break;
            }
            para.push(line);
        }
        let text = para.join(" ");
        first_sentence(&text)
    }
}

/// Up to the first full stop that ends a sentence.
pub fn first_sentence(text: &str) -> String {
    match text.find(". ") {
        Some(i) => text[..=i].to_string(),
        None => text.to_string(),
    }
}

impl Outline {
    /// The outline in `dir`, if it has one. Problems (an unknown section, a missing page) are
    /// returned as warnings; the outline then leaves that entry out.
    pub fn load(suite: &Suite, dir: &Path) -> Option<(Outline, Vec<String>)> {
        let text = std::fs::read_to_string(dir.join("outline.txt")).ok()?;
        let mut warnings = vec![];
        let mut out = Outline { parts: vec![], pages: vec![] };
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                warnings.push(format!("outline.txt:{}: expected `Part:`, `Section:` or `Page:`", n + 1));
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "Part" => out.parts.push(Part { name: value.to_string(), items: vec![] }),
                "Section" => {
                    let found = suite.sections.iter().find(|s| s.parent.is_some() && s.name.eq_ignore_ascii_case(value));
                    // A section below the top level moves, with its subsections, to here.
                    match (found, out.parts.last_mut()) {
                        (Some(s), Some(part)) => part.items.push(Item::Section(s.id)),
                        (None, _) => warnings.push(format!("outline.txt:{}: no section named «{value}»", n + 1)),
                        (_, None) => warnings.push(format!("outline.txt:{}: a section before any part", n + 1)),
                    }
                }
                "Page" => {
                    let (name, platform) = match value.split_once('[') {
                        Some((a, b)) => (a.trim(), Some(b.trim_end_matches(']').trim())),
                        None => (value, None),
                    };
                    if let (Some(p), Some(current)) = (platform, suite.platform.as_deref())
                        && !p.eq_ignore_ascii_case(current)
                    {
                        continue;
                    }
                    let path = dir.join("manual").join(format!("{name}.md"));
                    let Ok(source) = std::fs::read_to_string(&path) else {
                        warnings.push(format!("outline.txt:{}: cannot read {}", n + 1, path.display()));
                        continue;
                    };
                    let title = source
                        .lines()
                        .find_map(|l| l.strip_prefix("# "))
                        .map(|t| t.trim().to_string())
                        .unwrap_or_else(|| name.to_string());
                    let Some(part) = out.parts.len().checked_sub(1) else {
                        warnings.push(format!("outline.txt:{}: a page before any part", n + 1));
                        continue;
                    };
                    out.pages.push(Authored { slug: slug(name), title, source, part });
                    let i = out.pages.len() - 1;
                    out.parts[part].items.push(Item::Page(i));
                }
                other => warnings.push(format!("outline.txt:{}: unknown entry «{other}»", n + 1)),
            }
        }
        // Chapters nobody placed.
        let placed: Vec<SectionId> = out
            .parts
            .iter()
            .flat_map(|p| p.items.iter())
            .filter_map(|i| if let Item::Section(s) = i { Some(*s) } else { None })
            .collect();
        let rest: Vec<Item> = suite
            .section(SectionId(0))
            .children
            .iter()
            .filter(|c| !placed.contains(c))
            .map(|&c| Item::Section(c))
            .collect();
        if !rest.is_empty() {
            for item in &rest {
                if let Item::Section(s) = item {
                    warnings.push(format!("outline.txt: «{}» is in no part", suite.section(*s).name));
                }
            }
            out.parts.push(Part { name: "More".into(), items: rest });
        }
        Some((out, warnings))
    }

    /// Whether the outline places this section itself (rather than under its parent).
    pub fn listed(&self, id: SectionId) -> bool {
        self.parts.iter().any(|p| p.items.contains(&Item::Section(id)))
    }

    /// The part a section is in, and the chapter it is under: the section itself or its
    /// nearest ancestor that the outline places.
    pub fn anchor(&self, suite: &Suite, id: SectionId) -> Option<(usize, SectionId)> {
        suite.chain(id).into_iter().rev().find_map(|s| {
            let part = self.parts.iter().position(|p| p.items.contains(&Item::Section(s.id)))?;
            Some((part, s.id))
        })
    }
}

/// The suite's outline, with its problems on stderr.
pub fn of(d: &crate::Docs) -> Option<Outline> {
    let (outline, warnings) = Outline::load(d.suite, d.suite_dir?)?;
    for w in warnings {
        eprintln!("warning: {w}");
    }
    Some(outline)
}
