//! Approximate text measurement.
//!
//! Mermaid measures text in the browser. There is no font engine here, so the
//! width is estimated from a table of average glyph widths roughly calibrated
//! on Trebuchet MS / Arial. This is deliberately simple and meant to be
//! replaced by real font metrics later.

/// Width of a character, as a fraction of the font size.
fn char_width(c: char) -> f64 {
    match c {
        ' ' => 0.28,
        'i' | 'l' | 'j' | 'I' | '.' | ',' | ';' | ':' | '\'' | '|' | '!' => 0.26,
        'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '-' | '/' => 0.38,
        'm' | 'w' | 'M' | 'W' => 0.85,
        c if c.is_ascii_uppercase() => 0.66,
        c if c.is_ascii_digit() => 0.56,
        c if (c as u32) >= 0x2E80 => 1.0,
        _ => 0.52,
    }
}

/// Estimated width of one line of text, in pixels.
pub fn text_width(line: &str, font_size: f64) -> f64 {
    line.chars().map(char_width).sum::<f64>() * font_size
}

/// Bounding box of a multi-line label.
#[derive(Debug, Clone, PartialEq)]
pub struct TextBox {
    pub width: f64,
    pub height: f64,
    pub lines: Vec<String>,
}

/// Measures a label where `\n` separates lines.
pub fn measure(label: &str, font_size: f64, line_height: f64) -> TextBox {
    let lines: Vec<String> = if label.is_empty() {
        Vec::new()
    } else {
        label.split('\n').map(|l| l.trim().to_string()).collect()
    };
    let width = lines
        .iter()
        .map(|l| text_width(l, font_size))
        .fold(0.0, f64::max);
    let height = lines.len() as f64 * line_height;
    TextBox {
        width,
        height,
        lines,
    }
}
