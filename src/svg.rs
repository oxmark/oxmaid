//! Small SVG writing helpers.

/// Escapes text for an XML text node or attribute.
pub(crate) fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Formats a number with at most two decimals and no trailing zeros.
pub(crate) fn num(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == 0.0 {
        "0".to_string()
    } else {
        format!("{r}")
    }
}

/// Keeps a CSS value only if it contains safe characters
/// (no quotes, angle brackets, `;` or `url(`).
pub(crate) fn safe_css_value(v: &str) -> Option<&str> {
    let v = v.trim();
    let ok = !v.is_empty()
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || " #.,%()-".contains(c))
        && !v.to_ascii_lowercase().contains("url");
    ok.then_some(v)
}
