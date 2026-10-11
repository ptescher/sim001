//! The flow DAG drawn as SVG for the HTML site: one column per level with the start on the
//! left, an arrow for each "leads to", dashed arrows for exceptions and red ones for conflicts
//! nobody has settled. Nodes link to their expectations and are coloured by verdict.

use crate::{Docs, Facts};
use expectation_graph::Node;
use expectation_model::ExpId;
use expectation_story::Verdict;
use std::collections::HashMap;
use std::fmt::Write;

const W: f32 = 196.0;
const H: f32 = 46.0;
const GAP_X: f32 = 54.0;
const GAP_Y: f32 = 12.0;
const PAD: f32 = 12.0;
const START_W: f32 = 64.0;

/// Expectation names on two lines at most.
fn lines(name: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![String::new()];
    for word in name.split_whitespace() {
        let line = out.last_mut().unwrap();
        if !line.is_empty() && line.len() + 1 + word.len() > 27 {
            if out.len() == 2 {
                out[1].push('…');
                return out;
            }
            out.push(String::new());
        }
        let line = out.last_mut().unwrap();
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    out
}

/// A map of `exps` (in the order given), with the expectations outside them that lead into
/// them shown faded; `link` is each expectation's URL.
pub fn svg(d: &Docs, f: &Facts, exps: &[ExpId], link: &dyn Fn(ExpId) -> String) -> String {
    let flow = &d.analysis.flow;
    let mut outside: Vec<ExpId> = vec![];
    for (p, c) in &flow.edges {
        if let Node::Exp(p) = p
            && exps.contains(c)
            && !exps.contains(p)
            && !outside.contains(p)
        {
            outside.push(*p);
        }
    }
    let own = exps.len();
    let exps: Vec<ExpId> = exps.iter().copied().chain(outside).collect();
    let exps = &exps[..];
    let deepest = exps.iter().filter_map(|e| flow.level[e.0 as usize]).max().unwrap_or(0) as usize;
    // Column 0 is the start; unreachable expectations go in a column of their own at the end.
    let col_of = |e: ExpId| flow.level[e.0 as usize].map_or(deepest + 2, |l| l as usize + 1);
    let mut rows: HashMap<usize, usize> = HashMap::new();
    let mut at: HashMap<ExpId, (f32, f32)> = HashMap::new();
    let x_of = |col: usize| PAD + START_W + GAP_X + (col - 1) as f32 * (W + GAP_X);
    for &e in exps {
        let col = col_of(e);
        let row = rows.entry(col).or_insert(0);
        at.insert(e, (x_of(col), PAD + *row as f32 * (H + GAP_Y)));
        *row += 1;
    }
    let cols = exps.iter().map(|&e| col_of(e)).max().unwrap_or(1);
    let tallest = rows.values().copied().max().unwrap_or(1).max(1);
    let width = x_of(cols) + W + PAD;
    let height = PAD * 2.0 + tallest as f32 * (H + GAP_Y) - GAP_Y;
    let start = (PAD, PAD + ((tallest as f32 * (H + GAP_Y) - GAP_Y) / 2.0 - H / 2.0).max(0.0));

    let mut s = String::new();
    let _ = write!(
        s,
        r#"<svg class="dag" viewBox="0 0 {width:.0} {height:.0}" width="{width:.0}" height="{height:.0}" role="img" aria-label="Map of expectations"><defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" class="head"/></marker></defs>"#
    );
    let curve = |s: &mut String, (x1, y1): (f32, f32), (x2, y2): (f32, f32), class: &str| {
        let mid = (x1 + x2) / 2.0;
        let _ = write!(
            s,
            r#"<path class="{class}" d="M{x1:.1} {y1:.1}C{mid:.1} {y1:.1} {mid:.1} {y2:.1} {x2:.1} {y2:.1}" marker-end="url(#arrow)"/>"#
        );
    };

    let mut from_start = false;
    for (p, c) in &flow.edges {
        let Some(&(cx, cy)) = at.get(c) else { continue };
        let to = (cx, cy + H / 2.0);
        match p {
            Node::Start => {
                from_start = true;
                curve(&mut s, (start.0 + START_W, start.1 + H / 2.0), to, "edge");
            }
            Node::Exp(p) => {
                if let Some(&(px, py)) = at.get(p) {
                    curve(&mut s, (px + W, py + H / 2.0), to, "edge");
                }
            }
        }
    }
    // Exceptions and conflicts link node to node, whichever way round they sit.
    let side = |a: (f32, f32), b: (f32, f32)| {
        if a.0 + W <= b.0 {
            ((a.0 + W, a.1 + H / 2.0), (b.0, b.1 + H / 2.0))
        } else if b.0 + W <= a.0 {
            ((a.0, a.1 + H / 2.0), (b.0 + W, b.1 + H / 2.0))
        } else if a.1 < b.1 {
            ((a.0 + W - 24.0, a.1 + H), (b.0 + W - 24.0, b.1))
        } else {
            ((a.0 + W - 24.0, a.1), (b.0 + W - 24.0, b.1 + H))
        }
    };
    let link_pair = |s: &mut String, a: ExpId, b: ExpId, class: &str| {
        if let (Some(&pa), Some(&pb)) = (at.get(&a), at.get(&b)) {
            let (from, to) = side(pa, pb);
            if (from.0 - to.0).abs() < 1.0 {
                let bulge = from.0 + 40.0;
                let _ = write!(
                    s,
                    r#"<path class="{class}" d="M{:.1} {:.1}C{bulge:.1} {:.1} {bulge:.1} {:.1} {:.1} {:.1}" marker-end="url(#arrow)"/>"#,
                    from.0, from.1, from.1, to.1, to.0, to.1
                );
            } else {
                curve(s, from, to, class);
            }
        }
    };
    for &(w, l) in &d.analysis.precedence.declared {
        link_pair(&mut s, w, l, "override");
    }
    for c in d.analysis.conflicts.iter().filter(|c| c.winner.is_none()) {
        link_pair(&mut s, c.a, c.b, "conflict");
    }

    if from_start {
        let _ = write!(
            s,
            r#"<g class="node start"><rect x="{:.1}" y="{:.1}" width="{START_W}" height="{H}" rx="23"/><text x="{:.1}" y="{:.1}">Start</text></g>"#,
            start.0,
            start.1,
            start.0 + START_W / 2.0,
            start.1 + H / 2.0 + 4.0
        );
    }
    for (i, &e) in exps.iter().enumerate() {
        let (x, y) = at[&e];
        let x_ = d.suite.exp(e);
        let verdict = match f.verdict(e) {
            Some(Verdict::Verified) => "v-ok",
            Some(Verdict::Failed) => "v-bad",
            Some(Verdict::Unsupported) => "v-warn",
            _ => "v-idle",
        };
        let mut class = format!("node {verdict}");
        if x_.is_invariant() {
            class.push_str(" invariant");
        }
        if flow.level[e.0 as usize].is_none() {
            class.push_str(" unreachable");
        }
        if i >= own {
            class.push_str(" outside");
        }
        let names = lines(&x_.name);
        let first = y + H / 2.0 + 4.0 - (names.len() - 1) as f32 * 7.5;
        let _ = write!(
            s,
            r#"<a href="{}"><g class="{class}"><title>{}</title><rect x="{x:.1}" y="{y:.1}" width="{W}" height="{H}" rx="8"/>"#,
            crate::html::esc(&link(e)),
            crate::html::esc(&x_.name)
        );
        for (i, line) in names.iter().enumerate() {
            let _ = write!(
                s,
                r#"<text x="{:.1}" y="{:.1}">{}</text>"#,
                x + W / 2.0,
                first + i as f32 * 15.0,
                crate::html::esc(line)
            );
        }
        s.push_str("</g></a>");
    }
    s.push_str("</svg>");
    s
}

#[cfg(test)]
mod tests {
    use super::lines;

    #[test]
    fn names_wrap_onto_two_lines() {
        assert_eq!(lines("Short"), vec!["Short"]);
        assert_eq!(lines("The wizard holds the plane while you sweep the throttle").len(), 2);
        assert!(lines("one two three four five six seven eight nine ten eleven twelve")[1].ends_with('…'));
    }
}
