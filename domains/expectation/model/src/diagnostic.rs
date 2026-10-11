//! Errors and warnings about a suite, pointing at the lines they're about.

use std::collections::BTreeMap;
use std::fmt::{self, Write};
use std::sync::Arc;

/// A line in a source file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub file: Arc<str>,
    /// 1-based.
    pub line: u32,
}

impl Span {
    pub fn new(file: &str, line: u32) -> Self {
        Span { file: file.into(), line }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}", self.file, self.line)
    }
}

/// The text of every file in a suite, so diagnostics can quote the lines they point at.
#[derive(Clone, Debug, Default)]
pub struct Sources(BTreeMap<Arc<str>, String>);

impl Sources {
    pub fn insert(&mut self, file: Arc<str>, text: String) {
        self.0.insert(file, text);
    }

    pub fn line(&self, span: &Span) -> Option<&str> {
        self.0.get(&span.file)?.lines().nth(span.line.checked_sub(1)? as usize)
    }

    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(|k| &**k)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

#[derive(Clone, Debug)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Stable short name, e.g. "conflict" or "unknown-step".
    pub code: &'static str,
    pub message: String,
    pub labels: Vec<Label>,
    pub notes: Vec<String>,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn new(severity: Severity, code: &'static str, message: impl Into<String>) -> Self {
        Diagnostic { severity, code, message: message.into(), labels: vec![], notes: vec![], help: None }
    }

    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, code, message)
    }

    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, code, message)
    }

    pub fn note(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(Severity::Note, code, message)
    }

    pub fn at(mut self, span: &Span, message: impl Into<String>) -> Self {
        self.labels.push(Label { span: span.clone(), message: message.into() });
        self
    }

    pub fn note_that(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// Where it sorts: by its first label's file and line.
    pub fn sort_key(&self) -> (Option<&Span>, Severity) {
        (self.labels.first().map(|l| &l.span), self.severity)
    }

    /// Rendered like a compiler message, quoting each labelled line.
    pub fn render(&self, sources: &Sources) -> String {
        let mut s = String::new();
        let level = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        };
        let _ = writeln!(s, "{level}[{}]: {}", self.code, self.message);
        for label in &self.labels {
            let _ = writeln!(s, "  --> {}", label.span);
            if let Some(line) = sources.line(&label.span) {
                let _ = writeln!(s, "   | {}", line.trim_end());
            }
            if !label.message.is_empty() {
                let _ = writeln!(s, "   = {}", label.message);
            }
        }
        for note in &self.notes {
            let _ = writeln!(s, "   = note: {note}");
        }
        if let Some(help) = &self.help {
            let _ = writeln!(s, "   = help: {help}");
        }
        s
    }
}

pub fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().any(Diagnostic::is_error)
}

/// All diagnostics rendered, errors and warnings sorted by position, with a count at the end.
pub fn render_all(diagnostics: &[Diagnostic], sources: &Sources) -> String {
    let mut sorted: Vec<&Diagnostic> = diagnostics.iter().collect();
    sorted.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    let mut s = String::new();
    for d in &sorted {
        s.push_str(&d.render(sources));
        s.push('\n');
    }
    let errors = diagnostics.iter().filter(|d| d.severity == Severity::Error).count();
    let warnings = diagnostics.iter().filter(|d| d.severity == Severity::Warning).count();
    let plural = |n: usize, w: &str| if n == 1 { format!("1 {w}") } else { format!("{n} {w}s") };
    let _ = writeln!(s, "{}, {}", plural(errors, "error"), plural(warnings, "warning"));
    s
}
