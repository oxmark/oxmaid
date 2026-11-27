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

/// Returns the meaningful lines: skips YAML front matter (`---` ... `---`),
/// `%%` comments, `%%{...}%%` directives and blank lines.
pub(crate) fn lines(src: &str) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut seen_content = false;
    let mut in_front_matter = false;
    for (i, raw) in src.lines().enumerate() {
        let text = raw.trim();
        if in_front_matter {
            if text == "---" {
                in_front_matter = false;
            }
            continue;
        }
        if text.is_empty() {
            continue;
        }
        if !seen_content && text == "---" {
            seen_content = true;
            in_front_matter = true;
            continue;
        }
        seen_content = true;
        if text.starts_with("%%") {
            continue;
        }
        let lead = raw.len() - raw.trim_start().len();
        out.push(Line {
            number: i + 1,
            text,
            offset: raw[..lead].chars().count(),
        });
    }
    out
}
