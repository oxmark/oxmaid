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

/// Font style of a run of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontStyle {
    #[default]
    Regular,
    Bold,
    Italic,
}

/// Width of a line set in Arial, as Chromium lays it out (the fallback for
/// Mermaid's `"Recursive Variable",arial,sans-serif`): glyph advances plus
/// kerning, unrounded.
pub fn arial_width(line: &str, font_size: f64, style: FontStyle) -> f64 {
    use crate::font_metrics as m;
    let (adv, kern): (&[u16], &[(u8, u8, i16)]) = match style {
        FontStyle::Regular => (&m::REGULAR_ADVANCE, &m::REGULAR_KERN),
        FontStyle::Bold => (&m::BOLD_ADVANCE, &m::BOLD_KERN),
        FontStyle::Italic => (&m::ITALIC_ADVANCE, &m::ITALIC_KERN),
    };
    let mut units = 0.0;
    let mut prev: Option<char> = None;
    for c in line.chars() {
        let code = c as u32;
        units += match code.checked_sub(m::FIRST).and_then(|i| adv.get(i as usize)) {
            Some(&a) => f64::from(a),
            // Wide scripts (CJK, emoji) take about one em, others an average glyph.
            None if code >= 0x2E80 => 2048.0,
            None => 1139.0,
        };
        if let Some(p) = prev {
            if let (Ok(a), Ok(b)) = (u8::try_from(p as u32), u8::try_from(code)) {
                if let Ok(i) = kern.binary_search_by(|k| (k.0, k.1).cmp(&(a, b))) {
                    units += f64::from(kern[i].2);
                }
            }
        }
        prev = Some(c);
    }
    units / 2048.0 * font_size
}

/// Rounds a length up to Chromium's layout unit (1/64 px), as
/// `getBoundingClientRect` reports it.
pub fn layout_units(v: f64) -> f64 {
    (v * 64.0 - 1e-6).ceil() / 64.0
}
