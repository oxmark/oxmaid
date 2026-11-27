//! Source splitting shared by all diagram types.

/// A meaningful source line (not blank, not a comment, not front matter).
pub(crate) struct Line<'a> {
    /// Line number, starting at 1.
    pub number: usize,
    /// Text without leading and trailing whitespace.
    pub text: &'a str,
    /// Column (in chars, from 0) where `text` starts.
    pub offset: usize,
}

/// A source split into its parts.
pub(crate) struct Source<'a> {
    /// YAML front matter between the leading `---` lines, if any.
    pub front_matter: Option<String>,
    /// Bodies of `%%{ ... }%%` directives.
    pub directives: Vec<&'a str>,
    /// Meaningful lines.
    pub lines: Vec<Line<'a>>,
}

/// Splits the source: YAML front matter, `%%{...}%%` directives, and the
/// meaningful lines (`%%` comments and blank lines are dropped).
pub(crate) fn split(src: &str) -> Source<'_> {
    let mut out = Source { front_matter: None, directives: Vec::new(), lines: Vec::new() };
    let mut seen_content = false;
    let mut front: Option<Vec<&str>> = None;
    for (i, raw) in src.lines().enumerate() {
        let text = raw.trim();
        if let Some(fm) = front.as_mut() {
            if text == "---" {
                out.front_matter = Some(fm.join("\n"));
                front = None;
            } else {
                fm.push(raw);
            }
            continue;
        }
        if text.is_empty() {
            continue;
        }
        if !seen_content && text == "---" {
            seen_content = true;
            front = Some(Vec::new());
            continue;
        }
        seen_content = true;
        if let Some(body) = text.strip_prefix("%%{").and_then(|t| t.strip_suffix("}%%")) {
            out.directives.push(body);
            continue;
        }
        if text.starts_with("%%") {
            continue;
        }
        let lead = raw.len() - raw.trim_start().len();
        out.lines.push(Line { number: i + 1, text, offset: raw[..lead].chars().count() });
    }
    out
}

/// Meaningful lines only.
pub(crate) fn lines(src: &str) -> Vec<Line<'_>> {
    split(src).lines
}
