//! The flow DAG, overrides and unsettled conflicts as a Mermaid flowchart (rendered by GitHub
//! in Markdown) or Graphviz DOT.

use crate::Analysis;
use crate::flow::Node;
use illustration_theme::DAY as P;
use expectation_model::{ExpId, Suite};
use std::fmt::Write;

fn id(n: Node) -> String {
    match n {
        Node::Start => "start".into(),
        Node::Exp(e) => format!("e{}", e.0),
    }
}

fn mermaid_label(s: &str) -> String {
    s.replace('"', "#quot;")
}

/// `class_of` adds a Mermaid class to an expectation's node, e.g. "passed" or "failed".
pub fn mermaid(suite: &Suite, analysis: &Analysis, class_of: &dyn Fn(ExpId) -> Option<&'static str>) -> String {
    let mut s = String::from("flowchart LR\n");
    let _ = writeln!(s, "  start([\"Background\"])");
    for f in analysis.feature_order(suite).into_iter().map(|f| suite.feature(f)) {
        let _ = writeln!(s, "  subgraph f{}[\"{}\"]", f.id.0, mermaid_label(&f.name));
        for &e in &f.expectations {
            let x = suite.exp(e);
            let label = mermaid_label(&x.name);
            let node = if x.is_invariant() { format!("{{{{\"{label}\"}}}}") } else { format!("[\"{label}\"]") };
            let _ = writeln!(s, "    e{}{node}", e.0);
        }
        let _ = writeln!(s, "  end");
    }
    let mut links = 0;
    for (p, c) in &analysis.flow.edges {
        let _ = writeln!(s, "  {} --> {}", id(*p), id(Node::Exp(*c)));
        links += 1;
    }
    for (w, l) in &analysis.precedence.declared {
        let _ = writeln!(s, "  e{} -. overrides .-> e{}", w.0, l.0);
        links += 1;
    }
    let mut conflict_links = vec![];
    for c in analysis.conflicts.iter().filter(|c| c.winner.is_none()) {
        let _ = writeln!(s, "  e{} <-. conflict .-> e{}", c.a.0, c.b.0);
        conflict_links.push(links.to_string());
        links += 1;
    }
    if !conflict_links.is_empty() {
        let _ = writeln!(s, "  linkStyle {} stroke:{},stroke-width:2px", conflict_links.join(","), P.warning.css());
    }
    // In the shared theme's Day colours: GitHub draws Mermaid on a light page.
    let _ = write!(
        s,
        "  classDef passed fill:{},stroke:{},color:{}\n  \
         classDef failed fill:{},stroke:{},color:{}\n  \
         classDef untested fill:{},stroke:{},color:{}\n  \
         classDef unreachable stroke-dasharray:4 3,color:{}\n",
        P.ok_fill.css(),
        P.ok.css(),
        P.text.css(),
        P.warning_fill.css(),
        P.warning.css(),
        P.on_warning.css(),
        P.off_fill.css(),
        P.bezel_edge.css(),
        P.text.css(),
        P.muted.css(),
    );
    for e in &suite.expectations {
        let class = class_of(e.id).or(analysis.flow.level[e.id.0 as usize].is_none().then_some("unreachable"));
        if let Some(class) = class {
            let _ = writeln!(s, "  class e{} {class}", e.id.0);
        }
    }
    s
}

pub fn dot(suite: &Suite, analysis: &Analysis) -> String {
    let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let mut s =
        String::from("digraph expectations {\n  rankdir=LR;\n  node [shape=box, style=rounded, fontname=Helvetica];\n");
    let _ = writeln!(s, "  start [label=\"Background\", shape=oval];");
    for f in &suite.features {
        let _ = writeln!(s, "  subgraph cluster_f{} {{\n    label={};", f.id.0, q(&f.name));
        for &e in &f.expectations {
            let x = suite.exp(e);
            let shape = if x.is_invariant() { ", shape=hexagon" } else { "" };
            let dashed = if analysis.flow.level[e.0 as usize].is_none() { ", style=\"rounded,dashed\"" } else { "" };
            let _ = writeln!(s, "    e{} [label={}{shape}{dashed}];", e.0, q(&x.name));
        }
        s.push_str("  }\n");
    }
    for (p, c) in &analysis.flow.edges {
        let _ = writeln!(s, "  {} -> {};", id(*p), id(Node::Exp(*c)));
    }
    for (w, l) in &analysis.precedence.declared {
        let _ = writeln!(s, "  e{} -> e{} [style=dashed, label=\"overrides\"];", w.0, l.0);
    }
    for c in analysis.conflicts.iter().filter(|c| c.winner.is_none()) {
        let _ = writeln!(s, "  e{} -> e{} [dir=both, color=red, label=\"conflict\"];", c.a.0, c.b.0);
    }
    s.push_str("}\n");
    s
}
