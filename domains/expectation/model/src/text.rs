//! Text helpers shared by the parser, diagnostics and docs: matching keys, slugs, numbers.

/// Lowercase, collapse whitespace and drop a trailing full stop, for matching step text.
pub fn normalize(s: &str) -> String {
    let s = s.trim().trim_end_matches('.');
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// The matching key of a phrase: normalised, without a leading article.
pub fn key(s: &str) -> String {
    let n = normalize(s);
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = n.strip_prefix(article) {
            return rest.to_string();
        }
    }
    n
}

/// A file-name and anchor friendly form: "Throttle up!" -> "throttle-up".
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() { "untitled".into() } else { out }
}

/// Description lines as paragraphs: lines joined with spaces, an empty line between paragraphs.
pub fn paragraphs(lines: &[String]) -> Vec<String> {
    lines.split(|l| l.is_empty()).map(|p| p.join(" ")).filter(|p| !p.is_empty()).collect()
}

/// "a", "a and b", "a, b and c".
pub fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// Numbers as people write them: 55 rather than 55.0, 77.5 rather than 77.500.
pub fn number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Levenshtein distance, for "did you mean" suggestions.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = if ca == *cb { prev } else { 1 + prev.min(row[j]).min(row[j + 1]) };
            prev = cur;
        }
    }
    row[b.len()]
}

/// The candidate closest to `word`, if it's close enough to be a plausible typo.
pub fn closest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    candidates
        .into_iter()
        .map(|c| (edit_distance(word, c), c))
        .filter(|(d, c)| *d <= (c.len().max(word.len()) / 3).max(2))
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_ignore_case_spacing_and_articles() {
        assert_eq!(key("  The   Aircraft is In Hand. "), "aircraft is in hand");
        assert_eq!(key("an event"), "event");
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Throttle up throws the plane!"), "throttle-up-throws-the-plane");
        assert_eq!(slug("--"), "untitled");
    }

    #[test]
    fn numbers_and_lists() {
        assert_eq!(number(55.0), "55");
        assert_eq!(number(77.5), "77.5");
        assert_eq!(join_and(&["a".into(), "b".into(), "c".into()]), "a, b and c");
    }

    #[test]
    fn suggestions() {
        assert_eq!(closest("aircarft", ["aircraft", "throttle"]), Some("aircraft"));
        assert_eq!(closest("zebra", ["aircraft", "throttle"]), None);
    }
}
