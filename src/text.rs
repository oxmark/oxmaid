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

/// Font family of an SVG `<text>` measured with `getBBox()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgFont {
    /// Arial (Liberation Sans): the `sans-serif` default and Mermaid's
    /// `"Recursive Variable",arial,sans-serif` theme font.
    Arial,
    /// Times New Roman (Liberation Serif): a page's default font, which
    /// Mermaid ends up measuring sequence labels with.
    Times,
}

/// Advance width of an SVG text line, as `getBBox().width` reports it.
pub fn svg_text_width(line: &str, font_size: f64, font: SvgFont) -> f64 {
    match font {
        SvgFont::Arial => arial_width(line, font_size, FontStyle::Regular),
        SvgFont::Times => {
            use crate::font_metrics_svg as m;
            let px = font_size / 2048.0;
            let (mut units, mut left, mut right) = (0.0_f64, 0.0_f64, 0.0_f64);
            let mut prev: Option<char> = None;
            // SVG text drops leading and trailing white space and collapses
            // the rest.
            for c in collapse_white_space(line).chars() {
                let code = c as u32;
                if let Some(p) = prev {
                    if let (Ok(a), Ok(b)) = (u8::try_from(p as u32), u8::try_from(code)) {
                        if let Ok(i) = m::TIMES_KERN.binary_search_by(|k| (k.0, k.1).cmp(&(a, b))) {
                            units += f64::from(m::TIMES_KERN[i].2);
                        }
                    }
                }
                let (advance, x0, x1) = match times_glyph(c) {
                    Some(g) => (f64::from(g.0), g.1, g.2),
                    None if (0x200B..=0x200D).contains(&code) || code == 0xFEFF => (0.0, 0, 0),
                    None if code >= 0x2E80 => (2048.0, 0, 0),
                    None => (1024.0, 0, 0),
                };
                // The ink of each glyph, rounded out to whole pixels, widens
                // the box past the advances.
                if x0 != x1 {
                    let o = units * px;
                    left = left.min(o + (f64::from(x0) * px).floor());
                    right = right.max(o + (f64::from(x1) * px).ceil());
                }
                units += advance;
                prev = Some(c);
            }
            // Advances add up exactly and land on a 1/64 px grid.
            let adv = (units * px * 64.0).round() / 64.0;
            adv.max(right) - left
        }
    }
}

/// Times New Roman (advance, xMin, xMax, yMin, yMax) of `c`, in 2048 em units.
fn times_glyph(c: char) -> Option<(u16, i16, i16, i16, i16)> {
    use crate::font_metrics_svg as m;
    let code = c as u32;
    if let Some(i) = code
        .checked_sub(32)
        .filter(|&i| (i as usize) < m::TIMES_ADVANCE.len())
    {
        let i = i as usize;
        let (x0, x1) = m::TIMES_HEXTENT[i];
        let (y0, y1) = m::TIMES_EXTENT[i];
        return Some((m::TIMES_ADVANCE[i], x0, x1, y0, y1));
    }
    m::TIMES_EXTRA
        .binary_search_by_key(&code, |e| e.0)
        .ok()
        .map(|i| {
            let e = m::TIMES_EXTRA[i];
            (e.1, e.2, e.3, e.4, e.5)
        })
}

/// SVG text without `xml:space`: white space runs become one space, and the
/// ends are trimmed.
fn collapse_white_space(line: &str) -> std::borrow::Cow<'_, str> {
    let ws = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r');
    let t = line.trim_matches(ws);
    let mut prev_ws = false;
    if !t.chars().any(|c| {
        let double = ws(c) && (prev_ws || c != ' ');
        prev_ws = ws(c);
        double
    }) {
        return std::borrow::Cow::Borrowed(t);
    }
    let mut out = String::with_capacity(t.len());
    let mut prev_ws = false;
    for c in t.chars() {
        if ws(c) {
            if !prev_ws {
                out.push(' ');
            }
            prev_ws = true;
        } else {
            out.push(c);
            prev_ws = false;
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Height of an SVG text line's bounding box (`getBBox().height`): the font's
/// ascent and descent rounded to pixels, widened by any glyph that reaches
/// past them. A lone space measures 0.
pub fn svg_text_height(line: &str, font_size: f64, font: SvgFont) -> f64 {
    use crate::font_metrics_svg as m;
    if !line.is_empty() && line.chars().all(|c| c == ' ') {
        return 0.0;
    }
    let (asc, desc, ext) = match font {
        SvgFont::Arial => (m::ARIAL_ASCENT, m::ARIAL_DESCENT, &m::ARIAL_EXTENT),
        SvgFont::Times => (m::TIMES_ASCENT, m::TIMES_DESCENT, &m::TIMES_EXTENT),
    };
    let s = font_size / 2048.0;
    let mut top = -(asc * s).round();
    let mut bottom = (desc * s).round();
    for c in line.chars() {
        let extent = match font {
            SvgFont::Times => times_glyph(c).map(|g| (g.3, g.4)),
            SvgFont::Arial => (c as u32)
                .checked_sub(32)
                .and_then(|i| ext.get(i as usize))
                .copied(),
        };
        if let Some((y_min, y_max)) = extent {
            if y_min != 0 || y_max != 0 {
                top = top.min((-f64::from(y_max) * s).floor());
                bottom = bottom.max((-f64::from(y_min) * s).ceil());
            }
        }
    }
    bottom - top
}

/// Mermaid's line break: `<br>`, `<br/>`, `</br>`, `<br />`, any case.
pub(crate) fn split_breaks(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if let Some(len) = break_len(&bytes[i..]) {
                out.push(&text[start..i]);
                i += len;
                start = i;
                continue;
            }
        }
        i += 1;
    }
    out.push(&text[start..]);
    out
}

/// Length of a `<\/?br\s*\/?>` match at the start of `b`.
fn break_len(b: &[u8]) -> Option<usize> {
    let mut i = 1;
    if b.get(i) == Some(&b'/') {
        i += 1;
    }
    if !b.get(i..i + 2)?.eq_ignore_ascii_case(b"br") {
        return None;
    }
    i += 2;
    while b
        .get(i)
        .is_some_and(|c| c.is_ascii_whitespace() || *c == 0x0b)
    {
        i += 1;
    }
    if b.get(i) == Some(&b'/') {
        i += 1;
    }
    (b.get(i) == Some(&b'>')).then_some(i + 1)
}

/// Whether the text holds a Mermaid line break.
pub(crate) fn has_breaks(text: &str) -> bool {
    split_breaks(text).len() > 1
}

/// Result of Mermaid's `calculateTextDimensions`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextDims {
    pub width: f64,
    pub height: f64,
    pub line_height: f64,
}

/// Mermaid's `calculateTextDimensions` for SVG text: each line measured in
/// `sans-serif` and in the configured family, widths and heights rounded,
/// the configured family winning unless `sans-serif` is larger on all three.
/// `font` is the family the configured one resolves to.
pub fn text_dimensions(text: &str, font_size: f64, font: SvgFont) -> TextDims {
    if text.is_empty() {
        return TextDims::default();
    }
    let lines = split_breaks(text);
    let measure = |f: SvgFont| {
        let mut d = TextDims::default();
        for line in &lines {
            let line = if line.is_empty() { "\u{200b}" } else { line };
            d.width = d.width.max(svg_text_width(line, font_size, f)).round();
            let h = svg_text_height(line, font_size, f).round();
            d.height += h;
            d.line_height = d.line_height.max(h).round();
        }
        d
    };
    let sans = measure(SvgFont::Arial);
    let own = measure(font);
    if sans.height > own.height && sans.width > own.width && sans.line_height > own.line_height {
        sans
    } else {
        own
    }
}
