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

/// Keeps a CSS value only if it contains safe characters (no quotes, angle
/// brackets, `;` or backslashes) and no `url(` or `expression(`, which can
/// load content or run script.
pub(crate) fn safe_css_value(v: &str) -> Option<&str> {
    let v = v.trim();
    let lower = v.to_ascii_lowercase();
    let ok = !v.is_empty()
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || " #.,%()-".contains(c))
        && !lower.contains("url")
        && !lower.contains("expression");
    ok.then_some(v)
}

/// `#f96` → `rgb(255, 153, 102)`, as the browser reports a computed color.
pub(crate) fn css_color(v: &str) -> String {
    let hex = v
        .trim()
        .strip_prefix('#')
        .filter(|h| h.chars().all(|c| c.is_ascii_hexdigit()));
    let digits: Option<Vec<u8>> = hex.and_then(|h| match h.len() {
        3 => h
            .chars()
            .map(|c| u8::from_str_radix(&format!("{c}{c}"), 16).ok())
            .collect(),
        6 => (0..3)
            .map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).ok())
            .collect(),
        _ => None,
    });
    match digits {
        Some(d) => format!("rgb({}, {}, {})", d[0], d[1], d[2]),
        None => v.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::safe_css_value;

    #[test]
    fn keeps_only_safe_css_values() {
        assert_eq!(safe_css_value(" #f9f "), Some("#f9f"));
        assert_eq!(safe_css_value("rgb(1, 2, 3)"), Some("rgb(1, 2, 3)"));
        assert_eq!(safe_css_value("5px 2px"), Some("5px 2px"));
        for v in [
            "",
            "red;x:y",
            "url(x)",
            "URL(x)",
            "expression(alert(1))",
            "eXpReSsIoN(alert(1))",
            "\\65 xpression(x)",
            "red\"",
            "</style>",
        ] {
            assert_eq!(safe_css_value(v), None, "{v}");
        }
    }
}
