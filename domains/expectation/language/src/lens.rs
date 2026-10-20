//! Platforms: one suite for several systems that do the same thing differently.
//!
//! A suite declares its platforms (`Platform: desktop | iOS`, the first the default). Everything
//! written plainly is about every platform. Where one differs it says so:
//!
//! ```text
//! Event: the operator arms the vehicle
//!   Say: press ARM, then CONFIRM ARM
//!   Say[iOS]: tap ARM, then CONFIRM ARM
//!
//! Phrase: the console is connected to a plane on the ground     # desktop (the default)
//!   Given the map tool is pan
//! Phrase[iOS]: the console is connected to a plane on the ground
//!   Given the link is connected
//!
//! Feature: Measuring
//!   On: desktop                       # this one is not about iOS at all
//! ```
//!
//! [`apply`] reads the suite *for one platform*: variants for it replace the plain lines they
//! differ from, other platforms' variants are dropped, and what is `On:` elsewhere is left out.
//! Everything after (resolving, checking, planning, running, docs) sees an ordinary suite.

use crate::parse::{RawFile, Variant};
use expectation_model::{Diagnostic, Span};

/// Apply `want` (a platform name, or `None` for the default) to `files`. Returns the declared
/// platforms and the one the suite is now read for (`None` when it declares none).
pub fn apply(files: &mut [RawFile], want: Option<&str>, diags: &mut Vec<Diagnostic>) -> (Vec<String>, Option<String>) {
    let mut platforms: Vec<String> = vec![];
    for (p, _) in files.iter().flat_map(|f| &f.platforms) {
        if !platforms.contains(p) {
            platforms.push(p.clone());
        }
    }
    let list = platforms.join(", ");
    let known = |name: &str, span: &Span, tag: bool, diags: &mut Vec<Diagnostic>| -> bool {
        if platforms.is_empty() {
            diags.push(
                Diagnostic::error("unknown-platform", format!("`{name}` is a platform, but the suite declares none"))
                    .at(span, "")
                    .help("declare them in a Vocabulary block: `Platform: desktop | iOS`"),
            );
            false
        } else if !platforms.iter().any(|p| p == name) {
            diags.push(
                Diagnostic::error("unknown-platform", format!("no platform called `{name}`"))
                    .at(span, "")
                    .help(format!("the platforms are {list}")),
            );
            false
        } else if tag && name == platforms[0] {
            diags.push(
                Diagnostic::error("default-platform", format!("`{name}` is the default platform: write its lines plainly"))
                    .at(span, "")
                    .help("a tag is for the platforms that differ from the default"),
            );
            false
        } else {
            true
        }
    };

    // Every name used must be declared, whichever platform this is.
    for f in files.iter() {
        let mut names: Vec<(&str, &Span, bool)> = vec![];
        let ons = f.on.iter().chain(f.features.iter().filter_map(|x| x.on.as_ref())).chain(
            f.features.iter().flat_map(|x| x.expectations.iter().filter_map(|e| e.on.as_ref())),
        ).chain(f.stories.iter().filter_map(|x| x.on.as_ref()));
        for on in ons {
            names.extend(on.iter().map(|(p, s)| (p.as_str(), s, false)));
        }
        let variants = |v: &[Variant]| v.iter().map(|v| (v.platform.clone(), v.span.clone())).collect::<Vec<_>>();
        let mut tagged: Vec<(String, Span)> = vec![];
        for d in f.states.iter().chain(&f.events) {
            tagged.extend(variants(&d.variants));
        }
        for p in &f.phrases {
            tagged.extend(variants(&p.variants));
            tagged.extend(p.platform.iter().cloned());
        }
        for (name, span, tag) in names.into_iter().chain(tagged.iter().map(|(p, s)| (p.as_str(), s, true))) {
            known(name, span, tag, diags);
        }
    }

    if platforms.is_empty() {
        if let Some(want) = want {
            diags.push(Diagnostic::error("unknown-platform", format!("no platforms are declared, so there is no `{want}`")));
        }
        return (platforms, None);
    }
    let current = match want {
        Some(w) if platforms.iter().any(|p| p == w) => w.to_string(),
        Some(w) => {
            diags.push(Diagnostic::error("unknown-platform", format!("no platform called `{w}`")).help(format!("the platforms are {list}")));
            platforms[0].clone()
        }
        None => platforms[0].clone(),
    };
    let other = current != platforms[0];
    let here = |on: &Option<Vec<(String, Span)>>| on.as_ref().is_none_or(|on| on.iter().any(|(p, _)| *p == current));

    // Phrases: the platform's own version replaces the plain one of the same name.
    let own: Vec<String> = files
        .iter()
        .flat_map(|f| &f.phrases)
        .filter(|p| p.platform.as_ref().is_some_and(|(t, _)| *t == current))
        .map(|p| p.name.clone())
        .collect();
    // A section is a directory: an `On:` under its `Section:` is about every file in it.
    let left_out: Vec<String> = files.iter().filter(|f| f.on.is_some() && !here(&f.on)).map(|f| f.dir.clone()).collect();
    let skipped = |dir: &str| left_out.iter().any(|d| d.is_empty() || dir == d || dir.starts_with(&format!("{d}/")));
    let mut left_out_names: Vec<String> = vec![];
    for f in files.iter_mut() {
        f.phrases.retain(|p| match &p.platform {
            Some((t, _)) => *t == current,
            None => !own.contains(&p.name),
        });
        for p in &mut f.phrases {
            if other && let Some(v) = p.variants.iter().rev().find(|v| v.platform == current) {
                p.say = Some((v.text.clone(), v.span.clone()));
            }
            p.variants.clear();
        }
        for d in f.states.iter_mut().chain(f.events.iter_mut()) {
            if other {
                if let Some(v) = d.variants.iter().rev().find(|v| v.platform == current && !v.shows) {
                    d.say = Some((v.text.clone(), v.span.clone()));
                }
                let shows: Vec<_> = d.variants.iter().filter(|v| v.platform == current && v.shows).collect();
                if !shows.is_empty() {
                    d.shows = shows.iter().map(|v| (v.text.clone(), v.span.clone())).collect();
                }
            }
            d.variants.clear();
        }
        // `On:` leaves things out.
        if skipped(&f.dir) {
            left_out_names.extend(f.features.iter().flat_map(|x| &x.expectations).map(|e| e.name.clone()));
            f.features.clear();
            f.stories.clear();
        }
        f.features.retain(|x| here(&x.on) || { left_out_names.extend(x.expectations.iter().map(|e| e.name.clone())); false });
        for x in &mut f.features {
            x.expectations.retain(|e| here(&e.on) || { left_out_names.push(e.name.clone()); false });
        }
        f.stories.retain(|x| here(&x.on));
    }
    // An exception to an expectation that isn't on this platform has nothing to override.
    for e in files.iter_mut().flat_map(|f| &mut f.features).flat_map(|x| &mut x.expectations) {
        e.overrides.retain(|(name, _)| !left_out_names.contains(name));
    }
    (platforms, Some(current))
}
