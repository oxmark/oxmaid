//! Mermaid 12's default rendering: the `neo` look with the `redux-color`
//! theme, laid out the way Mermaid's ELK integration does it.
//!
//! Sizes follow Mermaid's shape code (`rendering-elements/shapes`) and its
//! HTML labels: 14px Arial (the browser fallback for `"Recursive Variable"`),
//! a line height of 21px, labels wrapped at 120px in nodes and 200px on links,
//! and a 120px minimum label width. The SVG has the same structure, classes,
//! CSS, markers and filters as Mermaid's. The layout itself is not ELK: it is
//! the crate's layered engine run with ELK's spacing, with orthogonal links,
//! ports spread along node sides and rounded corners, as ELK draws them.

use super::{Arrow, EdgeEnd, Flowchart, LabelType, Node, NodeShape, Stroke};
use crate::elk::{
    self, base64, basis_path, dist, js, points_json, pull, rounded_length, rounded_path, End,
    GCluster, GEdge, GNode, Geometry, Graph, Spacing,
};
use crate::layout::Point;
use crate::options::RenderOptions;
use crate::svg::{css_color, esc, safe_css_value};
use crate::text::{arial_width, layout_units, FontStyle};
use std::fmt::Write;

const FONT_SIZE: f64 = 14.0;
const LINE_HEIGHT: f64 = 21.0;
/// `flowchart.wrappingWidth` for node labels in the neo look, also their
/// minimum width (`flowchart.minNodeWidth`).
const NODE_WRAP: f64 = 120.0;
/// Maximum width of a link label.
const EDGE_WRAP: f64 = 200.0;
/// `flowchart.padding`.
const NODE_PADDING: f64 = 15.0;
/// ELK's padding around the graph.
const MARGIN: f64 = 12.0;
/// Mermaid's padding around the drawing in the `viewBox`.
const VIEW_PADDING: f64 = 8.0;
const NODE_SPACING: f64 = 40.0;
const RANK_SPACING: f64 = 40.0;
const EDGE_SPACING: f64 = 20.0;
const CLUSTER_PADDING: f64 = 24.0;
/// Room for a cluster title, on top of its height.
const CLUSTER_TITLE_GAP: f64 = 3.0;
/// `themeVariables.radius` of the redux themes.
const RADIUS: f64 = 12.0;

const CSS: &str = include_str!("neo.css");
const MARKERS: &str = include_str!("neo_markers.svg");

/// Cluster colors of `redux-color` (stroke, fill), picked by declaration order.
const CLUSTER_COLORS: usize = 12;

// ---------------------------------------------------------------------------
// Numbers

// ---------------------------------------------------------------------------
// Labels

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Seg {
    Text(String, FontStyle),
    /// Markup copied as is (already safe).
    Tag(String),
    Break,
}

/// A measured HTML label.
#[derive(Debug, Clone, Default)]
struct Label {
    html: String,
    width: f64,
    height: f64,
    wrapped: bool,
    markdown: bool,
}

/// Inline tags kept in plain labels, as Mermaid's sanitizer does.
const INLINE_TAGS: &[&str] = &[
    "b", "strong", "i", "em", "u", "s", "del", "ins", "sub", "sup", "small", "mark", "code", "span",
];

fn tag_style(stack: &[String]) -> FontStyle {
    if stack.iter().any(|t| t == "b" || t == "strong") {
        FontStyle::Bold
    } else if stack.iter().any(|t| t == "i" || t == "em") {
        FontStyle::Italic
    } else {
        FontStyle::Regular
    }
}

/// Splits `fa:fa-car` icons out of a text segment.
fn push_text(segs: &mut Vec<Seg>, text: &str, style: FontStyle) {
    let mut rest = text;
    while let Some(i) = find_icon(rest) {
        let (prefix_len, name_end) = i.1;
        if i.0 > 0 {
            segs.push(Seg::Text(rest[..i.0].to_string(), style));
        }
        let prefix = &rest[i.0..i.0 + prefix_len];
        let name = &rest[i.0 + prefix_len + 4..name_end];
        segs.push(Seg::Tag(format!(
            r#"<i class="{} fa-{}"></i>"#,
            esc(prefix),
            esc(name)
        )));
        rest = &rest[name_end..];
    }
    if !rest.is_empty() {
        segs.push(Seg::Text(rest.to_string(), style));
    }
}

/// Finds `fa[bklrs]?:fa-name`: (start, (prefix length, end of name)).
fn find_icon(s: &str) -> Option<(usize, (usize, usize))> {
    let bytes = s.as_bytes();
    let mut from = 0;
    while let Some(i) = s[from..].find("fa").map(|i| i + from) {
        let mut p = i + 2;
        if p < bytes.len() && b"bklrs".contains(&bytes[p]) && s[p + 1..].starts_with(":fa-") {
            p += 1;
        }
        if s[p..].starts_with(":fa-") {
            let start = p + 4;
            let end = start
                + s[start..]
                    .bytes()
                    .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
                    .count();
            if end > start {
                return Some((i, (p - i, end)));
            }
        }
        from = i + 2;
    }
    None
}

fn parse_plain(text: &str) -> Vec<Seg> {
    let mut segs = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('\n') {
            push_text(&mut segs, &std::mem::take(&mut buf), tag_style(&stack));
            segs.push(Seg::Break);
            rest = after;
            continue;
        }
        if rest.starts_with('<') {
            if let Some(end) = rest.find('>') {
                let inner = rest[1..end].trim();
                let (closing, name) = match inner.strip_prefix('/') {
                    Some(n) => (true, n.trim()),
                    None => (false, inner),
                };
                let name = name.to_ascii_lowercase();
                if INLINE_TAGS.contains(&name.as_str()) {
                    push_text(&mut segs, &std::mem::take(&mut buf), tag_style(&stack));
                    if closing {
                        if let Some(i) = stack.iter().rposition(|t| *t == name) {
                            stack.truncate(i);
                            segs.push(Seg::Tag(format!("</{name}>")));
                        }
                    } else {
                        segs.push(Seg::Tag(format!("<{name}>")));
                        stack.push(name);
                    }
                    rest = &rest[end + 1..];
                    continue;
                }
            }
        }
        let c = rest.chars().next().expect("rest is not empty");
        buf.push(c);
        rest = &rest[c.len_utf8()..];
    }
    push_text(&mut segs, &buf, tag_style(&stack));
    for name in stack.iter().rev() {
        segs.push(Seg::Tag(format!("</{name}>")));
    }
    segs
}

/// `**bold**`, `__bold__`, `*italic*` and `_italic_`, as Mermaid's markdown
/// labels support them.
pub(crate) fn parse_markdown(text: &str) -> Vec<Seg> {
    let mut segs = Vec::new();
    let (mut bold, mut italic) = (false, false);
    let mut buf = String::new();
    let chars: Vec<char> = text.chars().collect();
    let style = |b: bool, i: bool| {
        if b {
            FontStyle::Bold
        } else if i {
            FontStyle::Italic
        } else {
            FontStyle::Regular
        }
    };
    // CommonMark flanking rules, simplified: a run opens when followed by
    // a non-space, closes when preceded by one, and `_` never acts inside
    // a word.
    // The delimiter run around `i`: (index before it, index after it).
    let run = |i: usize| {
        let c = chars[i];
        let mut a = i;
        while a > 0 && chars[a - 1] == c {
            a -= 1;
        }
        let mut b = i;
        while b < chars.len() && chars[b] == c {
            b += 1;
        }
        (a.checked_sub(1).map(|p| chars[p]), chars.get(b).copied())
    };
    let can_open = |i: usize, _n: usize| {
        let (prev, next) = run(i);
        next.is_some_and(|c| !c.is_whitespace())
            && (chars[i] != '_' || !prev.is_some_and(char::is_alphanumeric))
    };
    let can_close = |i: usize, _n: usize| {
        let (prev, next) = run(i);
        prev.is_some_and(|c| !c.is_whitespace())
            && (chars[i] != '_' || !next.is_some_and(char::is_alphanumeric))
    };
    let closes = |from: usize, marker: &[char]| {
        let n = marker.len();
        (from..chars.len().saturating_sub(n - 1))
            .any(|j| chars[j..j + n] == *marker && can_close(j, n))
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            push_text(&mut segs, &std::mem::take(&mut buf), style(bold, italic));
            segs.push(Seg::Break);
            i += 1;
            continue;
        }
        if (c == '*' || c == '_')
            && chars.get(i + 1) == Some(&c)
            && (if bold {
                can_close(i, 2)
            } else {
                can_open(i, 2) && closes(i + 2, &[c, c])
            })
        {
            push_text(&mut segs, &std::mem::take(&mut buf), style(bold, italic));
            segs.push(Seg::Tag(if bold { "</strong>" } else { "<strong>" }.into()));
            bold = !bold;
            i += 2;
            continue;
        }
        if (c == '*' || c == '_')
            && (if italic {
                can_close(i, 1)
            } else {
                can_open(i, 1) && closes(i + 1, &[c])
            })
        {
            push_text(&mut segs, &std::mem::take(&mut buf), style(bold, italic));
            segs.push(Seg::Tag(if italic { "</em>" } else { "<em>" }.into()));
            italic = !italic;
            i += 1;
            continue;
        }
        buf.push(c);
        i += 1;
    }
    push_text(&mut segs, &buf, style(bold, italic));
    if italic {
        segs.push(Seg::Tag("</em>".into()));
    }
    if bold {
        segs.push(Seg::Tag("</strong>".into()));
    }
    segs
}

/// Lays a label out like a `display: table` div with `max-width: wrap`:
/// on one line when it fits, else wrapped at spaces (`break-spaces`).
fn build_label(text: &str, ty: LabelType, wrap: f64, min_width: f64) -> Label {
    let markdown = ty == LabelType::Markdown;
    let segs = if markdown {
        parse_markdown(text)
    } else {
        parse_plain(text)
    };
    let mut html = String::new();
    // Lines of words; a word is a list of styled pieces.
    let mut lines: Vec<Vec<(String, FontStyle)>> = vec![Vec::new()];
    for seg in &segs {
        match seg {
            Seg::Text(t, style) => {
                html.push_str(&esc(t));
                lines
                    .last_mut()
                    .expect("at least one line")
                    .push((t.clone(), *style));
            }
            Seg::Tag(t) => html.push_str(t),
            Seg::Break => {
                html.push_str("<br/>");
                lines.push(Vec::new());
            }
        }
    }
    if text.trim().is_empty() && !text.contains('\n') {
        let empty = text.is_empty();
        return Label {
            html,
            width: 0.0,
            height: if empty { 0.0 } else { LINE_HEIGHT },
            wrapped: false,
            markdown,
        };
    }
    let run_width = |runs: &[(String, FontStyle)]| -> f64 {
        runs.iter()
            .map(|(t, s)| arial_width(t, FONT_SIZE, *s))
            .sum()
    };
    let natural = lines.iter().map(|l| run_width(l)).fold(0.0, f64::max);
    let natural = layout_units(natural);
    if natural <= wrap {
        return Label {
            html,
            width: natural.max(min_width),
            height: lines.len() as f64 * LINE_HEIGHT,
            wrapped: false,
            markdown,
        };
    }
    // Wrap each line at spaces; a word longer than the box widens it.
    let mut count = 0usize;
    let mut widest: f64 = wrap;
    let space = arial_width(" ", FONT_SIZE, FontStyle::Regular);
    for line in &lines {
        let mut words: Vec<f64> = Vec::new();
        let mut current = 0.0;
        let mut in_word = false;
        for (t, s) in line {
            for (k, piece) in t.split(' ').enumerate() {
                if k > 0 {
                    if in_word {
                        words.push(current);
                    }
                    current = 0.0;
                    in_word = false;
                }
                if !piece.is_empty() {
                    current += arial_width(piece, FONT_SIZE, *s);
                    in_word = true;
                }
            }
        }
        if in_word {
            words.push(current);
        }
        let mut used = 0.0;
        let mut rows = 1;
        for (k, w) in words.iter().enumerate() {
            widest = widest.max(layout_units(*w));
            if k == 0 {
                used = *w;
            } else if used + space + w <= wrap + 1e-9 {
                used += space + w;
            } else {
                rows += 1;
                used = *w;
            }
        }
        count += rows;
    }
    Label {
        html,
        width: widest.max(min_width),
        height: count as f64 * LINE_HEIGHT,
        wrapped: true,
        markdown,
    }
}

fn label_div(
    label: &Label,
    wrap: Option<f64>,
    min_width: f64,
    class: &str,
    span_class: &str,
) -> String {
    let mut style = String::from("display: ");
    style.push_str(if wrap.is_some() && (label.wrapped || min_width > 0.0) {
        "table"
    } else {
        "table-cell"
    });
    style.push_str("; white-space: ");
    style.push_str(if label.wrapped {
        "break-spaces"
    } else {
        "nowrap"
    });
    style.push_str("; line-height: 1.5;");
    if let Some(w) = wrap {
        let _ = write!(style, " max-width: {}px; text-align: center;", js(w));
        if label.wrapped {
            let _ = write!(style, " width: {}px;", js(w));
        } else if min_width > 0.0 {
            let _ = write!(style, " width: {}px;", js(min_width));
        }
    }
    let class_attr = if class.is_empty() {
        String::new()
    } else {
        format!(r#" class="{class}""#)
    };
    let span = if label.markdown {
        format!("{span_class} markdown-node-label")
    } else {
        span_class.to_string()
    };
    format!(
        r#"<div xmlns="http://www.w3.org/1999/xhtml"{class_attr} style="{style}"><span class="{span}"><p>{}</p></span></div>"#,
        label.html
    )
}

// ---------------------------------------------------------------------------
// Styles

/// Mermaid's `styles2String`: label properties (`color`, `font-*`, `text-*`)
/// go to the label, the rest to the shape, each marked `!important`.
fn split_styles(props: &[(String, String)]) -> (String, String) {
    let mut shape = Vec::new();
    let mut label = Vec::new();
    for (k, v) in props {
        if v.is_empty() {
            continue;
        }
        let Some(v) = safe_css_value(v) else { continue };
        let k = k.trim();
        if !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            continue;
        }
        let decl = format!("{k}:{v} !important");
        if k == "color" || k.starts_with("font") || k.starts_with("text") {
            label.push(decl);
        } else {
            shape.push(decl);
        }
    }
    (shape.join(";"), label.join(";"))
}

fn class_props<'a>(
    fc: &'a Flowchart,
    classes: &'a [String],
) -> impl Iterator<Item = &'a (String, String)> {
    let names = std::iter::once("default").chain(classes.iter().map(String::as_str));
    names
        .filter_map(move |name| fc.class_defs.iter().find(|c| c.name == name))
        .flat_map(|c| c.styles.iter())
}

fn class_css(fc: &Flowchart, id: &str) -> String {
    let mut out = String::new();
    for def in &fc.class_defs {
        if !def
            .name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            continue;
        }
        let decls: String = def
            .styles
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .filter_map(|(k, v)| safe_css_value(v).map(|v| (k, v)))
            .filter(|(k, _)| k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
            .map(|(k, v)| format!("{k}:{}!important;", css_color(v)))
            .collect();
        if decls.is_empty() {
            continue;
        }
        let _ = write!(
            out,
            "#{id} .{0}>*{{{decls}}}#{id} .{0} span{{{decls}}}",
            def.name
        );
    }
    out
}

// ---------------------------------------------------------------------------
// Shapes

/// A node drawn around its label.
struct Shape {
    w: f64,
    h: f64,
    /// SVG of the outline, centered on the node.
    svg: String,
    /// Offset of the label center from the node center.
    label_dx: f64,
    label_dy: f64,
}

fn points_attr(points: &[(f64, f64)]) -> String {
    points
        .iter()
        .map(|(x, y)| format!("{},{}", js(*x), js(*y)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn style_attr(style: &str) -> String {
    format!(r#" style="{}""#, esc(style))
}

fn polygon(points: &[(f64, f64)], w: f64, h: f64, style: &str) -> String {
    format!(
        r#"<polygon points="{}" class="label-container" transform="translate({},{})"{}></polygon>"#,
        points_attr(points),
        js(-w / 2.0),
        js(h / 2.0),
        if style.is_empty() {
            String::new()
        } else {
            style_attr(style)
        }
    )
}

fn outer_path(d: &str, transform: Option<String>, style: &str) -> String {
    let transform = transform
        .map(|t| format!(r#" transform="{t}""#))
        .unwrap_or_default();
    format!(
        r#"<g class="basic label-container outer-path"{transform}><path d="{d}"{}></path></g>"#,
        style_attr(style)
    )
}

fn shape(kind: NodeShape, label: &Label, style: &str) -> Shape {
    let (lw, lh) = (label.width, label.height);
    let rect = |w: f64, h: f64, r: Option<f64>| {
        let radius = r
            .map(|r| format!(r#" rx="{0}" ry="{0}""#, js(r)))
            .unwrap_or_default();
        format!(
            r#"<rect class="basic label-container"{}{radius} x="{}" y="{}" width="{}" height="{}"></rect>"#,
            style_attr(style),
            js(-w / 2.0),
            js(-h / 2.0),
            js(w),
            js(h)
        )
    };
    let plain = |w: f64, h: f64, svg: String| Shape {
        w,
        h,
        svg,
        label_dx: 0.0,
        label_dy: 0.0,
    };
    match kind {
        NodeShape::Rounded => {
            let (w, h) = (lw + 2.0 * NODE_PADDING, lh + 2.0 * NODE_PADDING);
            plain(w, h, rect(w, h, Some(RADIUS)))
        }
        NodeShape::Stadium => {
            let h = lh + 24.0;
            let inscribed = h * (std::f64::consts::PI / 98.0).cos();
            let cap = (inscribed * inscribed - lh * lh).max(0.0).sqrt();
            let w = (lw + h / 4.0 + 40.0)
                .max(1.5 * h + (h - inscribed))
                .max(lw + 40.0 + h - cap);
            let r = h / 2.0;
            let (x0, x1) = (-w / 2.0 + r, w / 2.0 - r);
            let d = format!(
                "M{},{} L{},{} A{r},{r} 0 0 1 {},{} L{},{} A{r},{r} 0 0 1 {},{} Z",
                js(x0),
                js(-r),
                js(x1),
                js(-r),
                js(x1),
                js(r),
                js(x0),
                js(r),
                js(x0),
                js(-r),
                r = js(r)
            );
            plain(w, h, outer_path(&d, None, style))
        }
        NodeShape::Subroutine => {
            let total = lw + 16.0 + 28.0;
            let h = lh + 12.0;
            let w = total - 16.0;
            let pts = [
                (0.0, 0.0),
                (w, 0.0),
                (w, -h),
                (0.0, -h),
                (0.0, 0.0),
                (-8.0, 0.0),
                (w + 8.0, 0.0),
                (w + 8.0, -h),
                (-8.0, -h),
                (-8.0, 0.0),
            ];
            plain(total, h, polygon(&pts, w, h, style))
        }
        NodeShape::Cylinder | NodeShape::LinedCylinder | NodeShape::DataStore => {
            let w = lw + 24.0;
            let rx = w / 2.0;
            let ry = rx / (2.5 + w / 50.0);
            let body = lh + 24.0 + ry;
            let d = format!(
                "M0,{ry} a{rx},{ry} 0,0,0 {w},0 a{rx},{ry} 0,0,0 {mw},0 l0,{body} a{rx},{ry} 0,0,0 {w},0 l0,{mbody}",
                ry = js(ry),
                rx = js(rx),
                w = js(w),
                mw = js(-w),
                body = js(body),
                mbody = js(-body)
            );
            let total = body + 2.0 * ry;
            let svg = format!(
                r#"<path d="{d}" class="basic label-container outer-path"{} transform="translate({}, {})"></path>"#,
                style_attr(style),
                js(-w / 2.0),
                js(-total / 2.0)
            );
            Shape {
                w,
                h: total,
                svg,
                label_dx: 0.0,
                label_dy: NODE_PADDING / 1.5,
            }
        }
        NodeShape::Circle
        | NodeShape::SmallCircle
        | NodeShape::FilledCircle
        | NodeShape::FramedCircle => {
            let r = (lw * lw + lh * lh).sqrt() / 2.0 + 16.0;
            let svg = format!(
                r#"<circle class="basic label-container"{} r="{}" cx="0" cy="0"></circle>"#,
                style_attr(style),
                js(r)
            );
            plain(2.0 * r, 2.0 * r, svg)
        }
        NodeShape::DoubleCircle => {
            let inner = (lw * lw + lh * lh).sqrt() / 2.0 + 16.0;
            let outer = inner + 12.0;
            let svg = format!(
                r#"<g class="basic label-container"{0}><circle class="outer-circle"{0} r="{1}" cx="0" cy="0"></circle><circle class="inner-circle"{0} r="{2}" cx="0" cy="0"></circle></g>"#,
                style_attr(style),
                js(outer),
                js(inner)
            );
            plain(2.0 * outer, 2.0 * outer, svg)
        }
        NodeShape::Ellipse => {
            let (w, h) = (lw + 2.0 * NODE_PADDING, lh + 2.0 * NODE_PADDING);
            let svg = format!(
                r#"<ellipse class="basic label-container"{} rx="{}" ry="{}" cx="0" cy="0"></ellipse>"#,
                style_attr(style),
                js(w / 2.0),
                js(h / 2.0)
            );
            plain(w, h, svg)
        }
        NodeShape::Diamond => {
            let s = lw + NODE_PADDING + lh + NODE_PADDING;
            let pts = [
                (s / 2.0, 0.0),
                (s, -s / 2.0),
                (s / 2.0, -s),
                (0.0, -s / 2.0),
            ];
            let svg = format!(
                r#"<polygon points="{}" class="label-container" transform="translate({}, {})"{}></polygon>"#,
                points_attr(&pts),
                js(-s / 2.0 + 0.5),
                js(s / 2.0),
                if style.is_empty() {
                    String::new()
                } else {
                    style_attr(style)
                }
            );
            plain(s, s, svg)
        }
        NodeShape::Hexagon => {
            let h = lh + 70.0;
            let m = h / 3.5;
            let w = lw + 2.0 * m + 32.0;
            let pts = [
                (m, 0.0),
                (w - m, 0.0),
                (w, -h / 2.0),
                (w - m, -h),
                (m, -h),
                (0.0, -h / 2.0),
            ];
            plain(w, h, polygon(&pts, w, h, style))
        }
        NodeShape::Odd => {
            let w = lw + 42.0;
            let h = lh + 24.0;
            let (x, y) = (-w / 2.0, -h / 2.0);
            let notch = y / 2.0;
            let pts = [(x + notch, y), (x, 0.0), (x + notch, -y), (-x, -y), (-x, y)];
            let mut d = String::new();
            for (i, (px, py)) in pts.iter().enumerate() {
                let _ = write!(
                    d,
                    "{}{},{} ",
                    if i == 0 { "M" } else { "L" },
                    js(*px),
                    js(*py)
                );
            }
            d.push('Z');
            let svg = outer_path(
                &d,
                Some(format!("translate({},0)", js(-notch / 2.0))),
                style,
            );
            Shape {
                w: w - notch,
                h,
                svg,
                label_dx: -notch / 2.0,
                label_dy: 0.0,
            }
        }
        NodeShape::LeanRight | NodeShape::LeanLeft | NodeShape::TrapezoidBottom => {
            let h = lh + NODE_PADDING;
            let w = lw + 2.0 * NODE_PADDING;
            let k = 3.0 * h / 6.0;
            let pts = match kind {
                NodeShape::LeanRight => [(-k, 0.0), (w, 0.0), (w + k, -h), (0.0, -h)],
                NodeShape::LeanLeft => [(0.0, 0.0), (w + k, 0.0), (w, -h), (-k, -h)],
                _ => [(-k, 0.0), (w + k, 0.0), (w, -h), (0.0, -h)],
            };
            plain(w + 2.0 * k, h, polygon(&pts, w, h, style))
        }
        NodeShape::TrapezoidTop => {
            let h = lh + 2.0 * NODE_PADDING;
            let w = lw + 4.0 * NODE_PADDING;
            let k = 3.0 * h / 6.0;
            let pts = [(0.0, 0.0), (w, 0.0), (w + k, -h), (-k, -h)];
            plain(w + 2.0 * k, h, polygon(&pts, w, h, style))
        }
        NodeShape::Text => {
            let (w, h) = (lw + 2.0 * 16.0, lh + 2.0 * 12.0);
            let svg = format!(
                r#"<rect class="text"{} x="{}" y="{}" width="{}" height="{}"></rect>"#,
                style_attr(style),
                js(-w / 2.0),
                js(-h / 2.0),
                js(w),
                js(h)
            );
            plain(w, h, svg)
        }
        // Square corners: `A[text]` and the v12 shapes not drawn specially yet.
        _ => {
            let (w, h) = (lw + 2.0 * 16.0, lh + 2.0 * 12.0);
            plain(w, h, rect(w, h, None))
        }
    }
}

/// Half the extent of a shape along one axis, at offset `d` on the other:
/// where a link leaving the node's box at that offset meets the outline.
/// `w` and `h` are the box size; `forward` tells the side (right or bottom).
fn half_extent(kind: NodeShape, w: f64, h: f64, vertical: bool, d: f64, forward: bool) -> f64 {
    let (along, across) = if vertical { (h, w) } else { (w, h) };
    if !vertical {
        // Slanted sides: `t` runs from 0 at the top to 1 at the bottom.
        let t = (d / h + 0.5).clamp(0.0, 1.0);
        let slant = match kind {
            NodeShape::LeanRight => Some(if forward { 1.0 - t } else { t }),
            NodeShape::LeanLeft => Some(if forward { t } else { 1.0 - t }),
            NodeShape::TrapezoidBottom => Some(t),
            NodeShape::TrapezoidTop => Some(1.0 - t),
            _ => None,
        };
        if let Some(f) = slant {
            // The box holds the polygon's full width: base plus two slants.
            let k = h / 2.0;
            return (w / 2.0 - k + k * f).clamp(0.0, w / 2.0);
        }
        if kind == NodeShape::Odd && !forward {
            let notch = h / 4.0;
            return (w / 2.0 - notch * (1.0 - (d.abs() / (h / 2.0)).min(1.0))).max(0.0);
        }
    }
    let d = d.abs();
    let full = along / 2.0;
    let v = match kind {
        NodeShape::Diamond => full * (1.0 - d / (across / 2.0)),
        NodeShape::Circle
        | NodeShape::DoubleCircle
        | NodeShape::SmallCircle
        | NodeShape::FilledCircle
        | NodeShape::FramedCircle => {
            let r = w / 2.0;
            (r * r - d * d).max(0.0).sqrt()
        }
        NodeShape::Ellipse => full * (1.0 - (d / (across / 2.0)).powi(2)).max(0.0).sqrt(),
        NodeShape::Hexagon => {
            let m = h / 3.5;
            if vertical {
                if d <= w / 2.0 - m {
                    full
                } else {
                    full * (w / 2.0 - d) / m
                }
            } else {
                full - m * d / (h / 2.0)
            }
        }
        NodeShape::Stadium => {
            let r = h / 2.0;
            if vertical {
                let flat = w / 2.0 - r;
                if d <= flat {
                    full
                } else {
                    (r * r - (d - flat).powi(2)).max(0.0).sqrt()
                }
            } else {
                w / 2.0 - r + (r * r - d * d).max(0.0).sqrt()
            }
        }
        NodeShape::Cylinder | NodeShape::LinedCylinder | NodeShape::DataStore if vertical => {
            let rx = w / 2.0;
            let ry = rx / (2.5 + w / 50.0);
            full - ry + ry * (1.0 - (d / rx).powi(2)).max(0.0).sqrt()
        }
        _ => full,
    };
    v.clamp(0.0, full)
}

// ---------------------------------------------------------------------------
// Links

/// Space a marker takes at an end, for the dash array (`markerOffsets2`).
fn marker_gap(a: Arrow) -> f64 {
    match a {
        Arrow::None => 0.0,
        Arrow::Point => 4.0,
        Arrow::Circle | Arrow::Cross => 12.5,
    }
}

fn marker_name(a: Arrow, end: bool) -> Option<&'static str> {
    Some(match (a, end) {
        (Arrow::None, _) => return None,
        (Arrow::Point, true) => "pointEnd-margin",
        (Arrow::Point, false) => "pointStart-margin",
        (Arrow::Circle, true) => "circleEnd-margin",
        (Arrow::Circle, false) => "circleStart-margin",
        (Arrow::Cross, true) => "crossEnd-margin",
        (Arrow::Cross, false) => "crossStart-margin",
    })
}

// ---------------------------------------------------------------------------
// Rendering
pub(crate) fn render(fc: &Flowchart, o: &RenderOptions) -> String {
    let id = esc(&o.id_prefix);
    let dir = fc.direction;

    // Nodes: label, outline and size.
    let node_styles: Vec<(String, String)> = fc
        .nodes
        .iter()
        .map(|n| {
            let props: Vec<(String, String)> = class_props(fc, &n.classes)
                .chain(n.styles.iter())
                .cloned()
                .collect();
            split_styles(&props)
        })
        .collect();
    let node_labels: Vec<Label> = fc
        .nodes
        .iter()
        .map(|n| {
            let round = matches!(n.shape, NodeShape::Circle | NodeShape::DoubleCircle);
            let min = if round || n.label.is_empty() {
                0.0
            } else {
                NODE_WRAP
            };
            build_label(&n.label, n.label_type, NODE_WRAP, min)
        })
        .collect();
    let shapes: Vec<Shape> = fc
        .nodes
        .iter()
        .zip(&node_labels)
        .zip(&node_styles)
        .map(|((n, l), (s, _))| shape(n.shape, l, s))
        .collect();
    let edge_labels: Vec<Option<Label>> = fc
        .edges
        .iter()
        .map(|e| {
            e.label
                .as_ref()
                .map(|l| build_label(l, e.label_type, EDGE_WRAP, 0.0))
        })
        .collect();
    let cluster_labels: Vec<Label> = fc
        .subgraphs
        .iter()
        .map(|s| build_label(&s.title, s.label_type, f64::INFINITY, 0.0))
        .collect();

    // Layout and link routes.
    let gnodes: Vec<GNode> = shapes
        .iter()
        .zip(&fc.nodes)
        .map(|(s, n)| GNode {
            w: s.w,
            h: s.h,
            cluster: n.subgraph,
        })
        .collect();
    let end = |e: EdgeEnd| match e {
        EdgeEnd::Node(i) => End::Node(i),
        EdgeEnd::Subgraph(s) => End::Cluster(s),
    };
    let gedges: Vec<GEdge> = fc
        .edges
        .iter()
        .zip(&edge_labels)
        .map(|(e, l)| GEdge {
            from: end(e.from),
            to: end(e.to),
            min_len: e.length,
            label: l.as_ref().map(|l| (l.width, l.height)),
        })
        .collect();
    let gclusters: Vec<GCluster> = fc
        .subgraphs
        .iter()
        .zip(&cluster_labels)
        .map(|(s, l)| GCluster {
            parent: s.parent,
            title: (l.width, l.height),
        })
        .collect();
    let extent = |i: usize, w: f64, h: f64, vertical: bool, d: f64, forward: bool| {
        half_extent(fc.nodes[i].shape, w, h, vertical, d, forward)
    };
    let geo = elk::geometry(&Graph {
        dir,
        nodes: &gnodes,
        edges: &gedges,
        clusters: &gclusters,
        spacing: Spacing {
            node: NODE_SPACING,
            rank: RANK_SPACING,
            edge: EDGE_SPACING,
            cluster_padding: CLUSTER_PADDING,
            title_gap: CLUSTER_TITLE_GAP,
            margin: MARGIN,
        },
        extent: &extent,
    });
    let Geometry {
        node_rects,
        cluster_rects,
        label_rects,
        polylines,
        bounds: (mut x0, mut y0, mut x1, y1),
    } = geo;
    let depth = |s: usize| elk::depth(&gclusters, s);
    let title = fc.title.as_deref().filter(|t| !t.trim().is_empty());
    let title_x = (x0 + x1) / 2.0;
    if let Some(t) = title {
        let tw = arial_width(t, 18.0, FontStyle::Regular);
        x0 = x0.min(title_x - tw / 2.0);
        x1 = x1.max(title_x + tw / 2.0);
        y0 = y0.min(-25.0 - 16.0);
    }
    let (vx, vy) = (x0 - VIEW_PADDING, y0 - VIEW_PADDING);
    let (vw, vh) = (x1 - x0 + 2.0 * VIEW_PADDING, y1 - y0 + 2.0 * VIEW_PADDING);

    // SVG.
    let mut out = String::with_capacity(32 * 1024);
    let acc_title = fc.acc_title.as_deref().filter(|t| !t.is_empty());
    let acc_descr = fc.acc_descr.as_deref().filter(|t| !t.is_empty());
    let _ = write!(
        out,
        r#"<svg id="{id}" width="100%" xmlns="http://www.w3.org/2000/svg" class="flowchart" style="max-width: {}px;" viewBox="{} {} {} {}" role="graphics-document document" aria-roledescription="flowchart-v2""#,
        js(vw),
        js(vx),
        js(vy),
        js(vw),
        js(vh)
    );
    if acc_title.is_some() {
        let _ = write!(out, r#" aria-labelledby="chart-title-{id}""#);
    }
    if acc_descr.is_some() {
        let _ = write!(out, r#" aria-describedby="chart-desc-{id}""#);
    }
    out.push('>');
    if let Some(t) = acc_title {
        let _ = write!(out, r#"<title id="chart-title-{id}">{}</title>"#, esc(t));
    }
    if let Some(d) = acc_descr {
        let _ = write!(out, r#"<desc id="chart-desc-{id}">{}</desc>"#, esc(d));
    }
    out.push_str("<style>");
    out.push_str(&CSS.replace("#ID", &format!("#{id}")));
    out.push_str(&class_css(fc, &id));
    out.push_str("</style><g>");
    out.push_str(&MARKERS.replace("id=\"ID_", &format!("id=\"{id}_")));
    out.push_str(r#"<g class="root"><g class="clusters">"#);

    // Clusters: outer ones first, later declarations first among equals.
    let mut draw: Vec<usize> = (0..fc.subgraphs.len()).collect();
    let declared = declaration_order(fc);
    draw.sort_by_key(|&s| (depth(s), std::cmp::Reverse(declared[s])));
    for s in draw {
        let Some(r) = cluster_rects[s] else { continue };
        let sg = &fc.subgraphs[s];
        let props: Vec<(String, String)> = class_props(fc, &sg.classes)
            .chain(sg.styles.iter())
            .cloned()
            .collect();
        let (style, label_style) = split_styles(&props);
        let classes: String = sg.classes.iter().map(|c| format!(" {}", esc(c))).collect();
        let l = &cluster_labels[s];
        let _ = write!(
            out,
            r#"<g class="cluster{classes}" id="{id}-{}" data-look="neo" data-color-id="color-{}"><rect style="{}" x="{}" y="{}" width="{}" height="{}"></rect>"#,
            esc(&sg.id),
            declared[s] % CLUSTER_COLORS,
            esc(&style),
            js(r.x),
            js(r.y),
            js(r.w),
            js(r.h)
        );
        let _ = write!(
            out,
            r#"<g class="cluster-label"{} transform="translate({}, {})"><foreignObject width="{}" height="{}">{}</foreignObject></g></g>"#,
            if label_style.is_empty() {
                String::new()
            } else {
                style_attr(&label_style)
            },
            js(r.x + (r.w - l.width) / 2.0),
            js(r.y),
            js(l.width),
            js(l.height),
            label_div(l, None, 0.0, "", "nodeLabel")
        );
    }
    out.push_str(r#"</g><g class="edges edgePaths">"#);

    let default_curve = fc.default_curve.clone().or_else(|| {
        fc.config
            .lookup("flowchart", "curve")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    });
    for (k, e) in fc.edges.iter().enumerate() {
        let Some(line) = &polylines[k] else { continue };
        if line.len() < 2 {
            continue;
        }
        let mut pts = line.clone();
        let n = pts.len();
        if e.start == Arrow::Point {
            pts[0] = pull(pts[0], pts[1], 4.0);
        }
        if e.end == Arrow::Point {
            pts[n - 1] = pull(pts[n - 1], pts[n - 2], 4.0);
        }
        let curve = e.curve.clone().or_else(|| default_curve.clone());
        let (d, len) = match curve.as_deref() {
            Some("basis") => (basis_path(&pts), rounded_length(&pts)),
            Some("linear") => {
                let mut d = String::new();
                for (i, p) in pts.iter().enumerate() {
                    let _ = write!(
                        d,
                        "{}{},{}",
                        if i == 0 { "M" } else { "L" },
                        js(p.x),
                        js(p.y)
                    );
                }
                (d, pts.windows(2).map(|w| dist(w[0], w[1])).sum())
            }
            _ => (rounded_path(&pts), rounded_length(&pts)),
        };
        let (thickness, pattern) = match e.stroke {
            Stroke::Normal => ("normal", "solid"),
            Stroke::Thick => ("thick", "solid"),
            Stroke::Dotted => ("normal", "dotted"),
            Stroke::Invisible => ("invisible", "solid"),
        };
        let animated = e.animate || e.animation.is_some();
        let animation_class = if animated {
            format!(
                " edge-animation-{}",
                e.animation
                    .as_deref()
                    .filter(|a| *a == "fast")
                    .unwrap_or("slow")
            )
        } else {
            String::new()
        };
        let (os, oe) = (marker_gap(e.start), marker_gap(e.end));
        let mut style = String::new();
        if !animated {
            let dash = if e.stroke == Stroke::Dotted {
                let pairs = ((len - os - oe) / 4.0).floor().max(0.0) as usize;
                format!("0 {} {} {}", js(os), vec!["2 2"; pairs].join(" "), js(oe))
            } else {
                format!("0 {} {} {}", js(os), js(len - os - oe), js(oe))
            };
            let _ = write!(style, "stroke-dasharray: {dash}; stroke-dashoffset: 0;");
        }
        let edge_styles: Vec<String> = fc
            .default_link_styles
            .iter()
            .chain(e.styles.iter())
            .filter(|(_, v)| !v.is_empty())
            .filter_map(|(k, v)| safe_css_value(v).map(|v| format!("{k}:{v}")))
            .filter(|s| {
                s.split(':')
                    .next()
                    .is_some_and(|k| k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
            })
            .collect();
        let joined: String = edge_styles.iter().map(|s| format!("{s};")).collect();
        style.push_str(&joined);
        style.push(';');
        style.push_str(
            &edge_styles
                .iter()
                .map(|s| format!(";{s}"))
                .collect::<String>(),
        );
        let _ = write!(
            out,
            r#"<path d="{d}" id="{id}-{}" class="edge-thickness-{thickness} edge-pattern-{pattern} edge-thickness-normal edge-pattern-solid flowchart-link{animation_class}" style="{}" data-edge="true" data-et="edge" data-id="{}" data-points="{}" data-look="neo""#,
            esc(&e.id),
            esc(&style),
            esc(&e.id),
            base64(points_json(line).as_bytes())
        );
        if let Some(m) = marker_name(e.start, false) {
            let _ = write!(out, r#" marker-start="url(#{id}_flowchart-v2-{m})""#);
        }
        if let Some(m) = marker_name(e.end, true) {
            let _ = write!(out, r#" marker-end="url(#{id}_flowchart-v2-{m})""#);
        }
        out.push_str("></path>");
    }
    out.push_str(r#"</g><g class="edgeLabels">"#);
    for (k, e) in fc.edges.iter().enumerate() {
        let (Some(l), Some(r)) = (&edge_labels[k], label_rects[k]) else {
            continue;
        };
        if polylines[k].is_none() {
            continue;
        }
        // ELK sizes the label box in whole pixels; Mermaid centers the real
        // label in its top-left corner box.
        let cx = r.x + l.width / 2.0;
        let cy = r.y + l.height / 2.0;
        let _ = write!(
            out,
            r#"<g class="edgeLabel" transform="translate({}, {})"><g class="label" data-id="{}" transform="translate({}, {})"><foreignObject width="{}" height="{}">{}</foreignObject></g></g>"#,
            js(cx),
            js(cy),
            esc(&e.id),
            js(-l.width / 2.0),
            js(-l.height / 2.0),
            js(l.width),
            js(l.height),
            label_div(l, Some(EDGE_WRAP), 0.0, "labelBkg", "edgeLabel")
        );
    }
    out.push_str(r#"</g><g class="nodes">"#);
    for (i, n) in fc.nodes.iter().enumerate() {
        write_node(
            &mut out,
            &id,
            n,
            &shapes[i],
            &node_labels[i],
            &node_styles[i].1,
            node_rects[i].center(),
        );
    }
    out.push_str("</g></g></g>");
    let _ = write!(
        out,
        r##"<defs><filter id="{id}-drop-shadow" height="130%" width="130%"><feDropShadow dx="4" dy="4" stdDeviation="0" flood-opacity="0.06" flood-color="#000000"></feDropShadow></filter></defs><defs><filter id="{id}-drop-shadow-small" height="150%" width="150%"><feDropShadow dx="2" dy="2" stdDeviation="0" flood-opacity="0.06" flood-color="#000000"></feDropShadow></filter></defs>"##
    );
    if let Some(t) = title {
        let _ = write!(
            out,
            r#"<text text-anchor="middle" x="{}" y="-25" class="flowchartTitleText">{}</text>"#,
            js(title_x),
            esc(t)
        );
    }
    out.push_str("</svg>");
    out
}

/// Subgraph indices in the order their `subgraph` lines appear. Subgraphs
/// are stored in closing order, where a parent follows its children.
fn declaration_order(fc: &Flowchart) -> Vec<usize> {
    // Opening order is a pre-order walk of the tree, children by closing order.
    let mut out = vec![0; fc.subgraphs.len()];
    let mut next = 0;
    fn visit(fc: &Flowchart, s: usize, next: &mut usize, out: &mut [usize]) {
        out[s] = *next;
        *next += 1;
        for c in (0..fc.subgraphs.len()).filter(|&c| fc.subgraphs[c].parent == Some(s)) {
            visit(fc, c, next, out);
        }
    }
    for s in (0..fc.subgraphs.len()).filter(|&s| fc.subgraphs[s].parent.is_none()) {
        visit(fc, s, &mut next, &mut out);
    }
    out
}

fn write_node(
    out: &mut String,
    id: &str,
    n: &Node,
    shape: &Shape,
    label: &Label,
    label_style: &str,
    c: Point,
) {
    let classes: String = n.classes.iter().map(|c| format!(" {}", esc(c))).collect();
    if let Some(link) = &n.link {
        let target = n
            .link_target
            .as_deref()
            .map(|t| format!(r#" target="{}""#, esc(t)))
            .unwrap_or_default();
        let _ = write!(out, r#"<a href="{}"{target}>"#, esc(link));
    }
    let _ = write!(
        out,
        r#"<g class="node default{classes}" id="{id}-flowchart-{}-{}" data-look="neo" transform="translate({}, {})">"#,
        esc(&n.id),
        n.dom_index,
        js(c.x),
        js(c.y)
    );
    if let Some(t) = &n.tooltip {
        let _ = write!(out, "<title>{}</title>", esc(t));
    }
    out.push_str(&shape.svg);
    let min =
        if label.width > 0.0 && !matches!(n.shape, NodeShape::Circle | NodeShape::DoubleCircle) {
            NODE_WRAP
        } else {
            0.0
        };
    let _ = write!(
        out,
        r#"<g class="label" style="{}" transform="translate({}, {})"><rect></rect><foreignObject width="{}" height="{}">{}</foreignObject></g></g>"#,
        esc(label_style),
        js(shape.label_dx - label.width / 2.0),
        js(shape.label_dy - label.height / 2.0),
        js(label.width),
        js(label.height),
        label_div(label, Some(NODE_WRAP), min, "", "nodeLabel")
    );
    if n.link.is_some() {
        out.push_str("</a>");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_sizes_match_chromium() {
        let l = build_label("Get money", LabelType::Text, EDGE_WRAP, 0.0);
        assert_eq!((l.width, l.height), (68.484375, 21.0));
        let l = build_label("Christmas", LabelType::Text, NODE_WRAP, NODE_WRAP);
        assert_eq!(l.width, 120.0);
        let l = build_label(
            "This is a very long label that needs wrapping",
            LabelType::Text,
            NODE_WRAP,
            NODE_WRAP,
        );
        assert_eq!((l.width, l.height, l.wrapped), (120.0, 63.0, true));
        let l = build_label(
            "a very long edge label that will need to be wrapped somewhere",
            LabelType::Text,
            EDGE_WRAP,
            0.0,
        );
        assert_eq!((l.width, l.height), (200.0, 63.0));
    }

    #[test]
    fn labels_keep_inline_markup_only() {
        let l = build_label(
            "<b>bold</b> <script>x</script>\nfa:fa-car Car",
            LabelType::Text,
            NODE_WRAP,
            0.0,
        );
        assert_eq!(
            l.html,
            r#"<b>bold</b> &lt;script&gt;x&lt;/script&gt;<br/><i class="fa fa-car"></i> Car"#
        );
        let l = build_label("**md** *it*", LabelType::Markdown, NODE_WRAP, 0.0);
        assert_eq!(l.html, "<strong>md</strong> <em>it</em>");
    }

    #[test]
    fn shapes_match_mermaid_sizes() {
        let label = build_label("Rect", LabelType::Text, NODE_WRAP, NODE_WRAP);
        let size = |k| {
            let s = shape(k, &label, "");
            (s.w, s.h)
        };
        assert_eq!(size(NodeShape::Rect), (152.0, 45.0));
        assert_eq!(size(NodeShape::Rounded), (150.0, 51.0));
        assert_eq!(size(NodeShape::Diamond), (171.0, 171.0));
        assert_eq!(size(NodeShape::Hexagon), (204.0, 91.0));
        assert_eq!(size(NodeShape::Subroutine), (164.0, 33.0));
        let s = shape(NodeShape::Stadium, &label, "");
        assert!((s.w - 171.25).abs() < 1e-9 && s.h == 45.0);
    }

    #[test]
    fn base64_and_numbers() {
        assert_eq!(base64(b"[{\"x\":1}]"), "W3sieCI6MX1d");
        assert_eq!(js(88.0), "88");
        assert_eq!(js(-0.0), "0");
        assert_eq!(css_color("#f96"), "rgb(255, 153, 102)");
    }
}
