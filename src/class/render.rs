//! Class diagram to SVG, in Mermaid 12's default `neo` look.
//!
//! Boxes follow Mermaid's `classBox` shape and its `textHelper` (the same
//! sizes, offsets and quirks, such as the box width taking the class name
//! centered but members left aligned), notes its `note` shape, and lollipop
//! interfaces an invisible box around their name. Labels are 14px Arial with
//! a 21px line height, the names in bold. Layout and link routes come from
//! the crate's ELK-style geometry, as for flowcharts, so positions are close
//! to Mermaid's ELK layout but not identical.
//!
//! Mermaid draws the outlines with rough.js, whose control points are
//! random; this renderer uses fixed ones, which draw the same straight lines.

use std::fmt::Write;

use super::{Class, ClassDiagram, Member, RelationKind};
use crate::config::Value;
use crate::dom::{escape_attr, escape_text};
use crate::elk::{
    self, base64, dist, js, points_json, rounded_length, rounded_path, End, GCluster, GEdge, GNode,
    Graph, Spacing,
};
use crate::flowchart::neo::{parse_markdown, Seg};
use crate::flowchart::Direction;
use crate::layout::Point;
use crate::svg::{css_color, safe_css_value};
use crate::text::{arial_width, layout_units, FontStyle};
use crate::RenderOptions;

/// Escapes an attribute value the way `XMLSerializer` does.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape_attr(s, &mut out);
    out
}

const CSS: &str = include_str!("neo.css");
const MARKERS: &str = include_str!("neo_markers.svg");

const FONT_SIZE: f64 = 14.0;
const LINE_HEIGHT: f64 = 21.0;
/// Cardinalities (`.edgeTerminals`) are 11px.
const TERMINAL_FONT_SIZE: f64 = 11.0;
const TERMINAL_HEIGHT: f64 = 16.5;
/// `class.padding`.
const PADDING: f64 = 12.0;
/// Notes wider than this get a fixed-width `display: table` div.
const NOTE_WRAP: f64 = 120.0;
const NOTE_PADDING: f64 = 6.0;
/// Mermaid's padding around the drawing in the `viewBox`.
const VIEW_PADDING: f64 = 8.0;
/// Class boxes are colored by declaration order, in 12 slots.
const COLOR_SLOTS: usize = 12;
const MAIN_BKG: &str = "#ffffff";
const NODE_BORDER: &str = "#28253D";
const NOTE_BKG: &str = "#fff5ad";
const NOTE_BORDER: &str = "#FACC15";

// ---------------------------------------------------------------------------
// Labels

/// A measured one-line label.
#[derive(Debug, Clone, Default)]
struct Label {
    html: String,
    width: f64,
    height: f64,
    lines: usize,
}

impl Label {
    /// Top of a class text item relative to its anchor: Mermaid's `addText`
    /// centers the first line, not the whole block, on it.
    fn top(&self) -> f64 {
        -self.height / (2.0 * self.lines.max(1) as f64)
    }
}

/// Mermaid's placeholders for `#name;` entities, back to entities.
fn decode_placeholders(s: &str) -> String {
    s.replace("\u{FB02}\u{B0}\u{B0}", "&#")
        .replace("\u{FB02}\u{B0}", "&")
        .replace("\u{B6}\u{DF}", ";")
}

/// Decodes the character references the browser would show.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let decoded = rest[1..].find(';').and_then(|e| {
            let name = &rest[1..1 + e];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ if name.starts_with("#x") || name.starts_with("#X") => {
                    u32::from_str_radix(&name[2..], 16)
                        .ok()
                        .and_then(char::from_u32)
                }
                _ if name.starts_with('#') => name[1..].parse().ok().and_then(char::from_u32),
                _ => None,
            };
            c.map(|c| (c, e + 2))
        });
        match decoded {
            Some((c, n)) => {
                out.push(c);
                rest = &rest[n..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The text a Markdown label shows: backslash escapes of punctuation are
/// dropped, character references decoded.
fn shown_text(text: &str) -> String {
    let text = decode_placeholders(text);
    let mut out = String::with_capacity(text.len());
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' && it.peek().is_some_and(|n| n.is_ascii_punctuation()) {
            continue;
        }
        out.push(c);
    }
    decode_entities(&out)
}

fn label(text: &str, style: FontStyle, font_size: f64, line_height: f64) -> Label {
    let shown = shown_text(text);
    if shown.is_empty() {
        return Label::default();
    }
    // Markdown turns each source line into a `<br>`-separated line, with
    // the indentation dropped.
    let mut html = String::with_capacity(shown.len());
    let mut width = 0f64;
    let mut lines = 0usize;
    for line in shown.split('\n') {
        let line = line.trim();
        if lines > 0 {
            html.push_str("<br>");
        }
        // `*italic*`, `**bold**` and their `_` forms become tags.
        let mut w = 0.0;
        for seg in parse_markdown(line) {
            match seg {
                Seg::Text(t, s) => {
                    escape_text(&t, &mut html);
                    let s = if s == FontStyle::Regular { style } else { s };
                    w += arial_width(&t, font_size, s);
                }
                Seg::Tag(t) => html.push_str(&t),
                Seg::Break => html.push_str("<br>"),
            }
        }
        width = width.max(layout_units(w));
        lines += 1;
    }
    Label {
        html,
        width,
        height: line_height * lines as f64,
        lines,
    }
}

fn text_label(text: &str, style: FontStyle) -> Label {
    label(text, style, FONT_SIZE, LINE_HEIGHT)
}

/// `max-width` Mermaid gives a class label: its estimated width plus 50.
fn max_width(text: &str) -> f64 {
    (arial_width(text, 15.0, FontStyle::Regular)).round() + 50.0
}

// ---------------------------------------------------------------------------
// Styles

/// A class's `style` / `classDef` declarations, `(property, value)`, with
/// unsafe values dropped.
fn class_styles(c: &Class) -> Vec<(String, String)> {
    c.styles
        .iter()
        .filter_map(|s| {
            let (k, v) = s.split_once(':')?;
            let k = k.trim();
            if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return None;
            }
            let v = v.trim().trim_end_matches("!important").trim();
            Some((k.to_string(), safe_css_value(v)?.to_string()))
        })
        .collect()
}

fn style_string(props: &[(String, String)]) -> String {
    props
        .iter()
        .map(|(k, v)| format!("{k}:{v}"))
        .collect::<Vec<_>>()
        .join(";")
}

/// Label properties (`color`, `font-*`, `text-*`), as `styles2String` sorts
/// them.
fn prop<'a>(props: &'a [(String, String)], key: &str) -> Option<&'a str> {
    props
        .iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// `#id .Class>*{...}` rules for classes with styles.
fn class_css(d: &ClassDiagram, id: &str) -> String {
    let mut out = String::new();
    for c in &d.classes {
        if c.styles.is_empty()
            || !c
                .id
                .chars()
                .all(|ch| ch.is_alphanumeric() || ch == '-' || ch == '_')
        {
            continue;
        }
        let decls: String = class_styles(c)
            .iter()
            .map(|(k, v)| format!("{k}:{}!important;", css_color(v)))
            .collect();
        if decls.is_empty() {
            continue;
        }
        let _ = write!(
            out,
            "#{id} .{0}>*{{{decls}}}#{id} .{0} span{{{decls}}}",
            c.id
        );
    }
    out
}

// ---------------------------------------------------------------------------
// Outlines

/// A straight line as rough.js draws it with no roughness: two cubic
/// strokes with control points along the line.
fn rough_line(out: &mut String, a: Point, b: Point) {
    for (k, t) in [0.25, 0.32].into_iter().enumerate() {
        if k > 0 {
            out.push(' ');
        }
        let _ = write!(
            out,
            "M{} {} C{} {}, {} {}, {} {}",
            js(a.x),
            js(a.y),
            js(a.x + (b.x - a.x) * t),
            js(a.y + (b.y - a.y) * t),
            js(a.x + (b.x - a.x) * 2.0 * t),
            js(a.y + (b.y - a.y) * 2.0 * t),
            js(b.x),
            js(b.y)
        );
    }
}

/// Paint of an outline: fill, stroke, stroke width and dash array.
struct Paint {
    fill: String,
    stroke: String,
    width: String,
    dash: String,
    style: String,
}

impl Paint {
    fn new(props: &[(String, String)], fill: &str, stroke: &str, style: String) -> Self {
        Paint {
            fill: prop(props, "fill").unwrap_or(fill).to_string(),
            stroke: prop(props, "stroke").unwrap_or(stroke).to_string(),
            width: prop(props, "stroke-width")
                .map(|w| w.trim_end_matches("px").to_string())
                .unwrap_or_else(|| "1.3".into()),
            dash: prop(props, "stroke-dasharray").unwrap_or("0 0").to_string(),
            style,
        }
    }
}

/// The background and the rough outline of a box centered on the origin.
fn rough_rect(out: &mut String, x: f64, y: f64, w: f64, h: f64, p: &Paint) {
    let (x1, y1) = (x + w, y + h);
    let _ = write!(
        out,
        r#"<path d="M{} {} L{} {} L{} {} L{} {}" stroke="none" stroke-width="0" fill="{}" style="{}"></path><path d=""#,
        js(x),
        js(y),
        js(x1),
        js(y),
        js(x1),
        js(y1),
        js(x),
        js(y1),
        esc(&p.fill),
        esc(&p.style)
    );
    let corners = [
        Point { x, y },
        Point { x: x1, y },
        Point { x: x1, y: y1 },
        Point { x, y: y1 },
    ];
    for i in 0..4 {
        if i > 0 {
            out.push(' ');
        }
        rough_line(out, corners[i], corners[(i + 1) % 4]);
    }
    let _ = write!(
        out,
        r#"" stroke="{}" stroke-width="{}" fill="none" stroke-dasharray="{}" style="{}"></path>"#,
        esc(&p.stroke),
        esc(&p.width),
        esc(&p.dash),
        esc(&p.style)
    );
}

// ---------------------------------------------------------------------------
// Class boxes

/// A class box, laid out the way `textHelper` and `classBox` do it.
struct ClassBox {
    annotation: Option<Label>,
    name: Label,
    members: Vec<(Label, &'static str, String)>,
    methods: Vec<(Label, &'static str, String)>,
    /// Size of the outline.
    w: f64,
    h: f64,
    /// Top left corner of the outline, relative to the center.
    rx: f64,
    ry: f64,
    /// Group translations: annotation, name, members, methods.
    groups: [(f64, f64); 4],
    dividers: Vec<f64>,
}

fn classifier_style(m: &Member) -> &'static str {
    match m.classifier.as_str() {
        "*" => "font-style:italic;",
        "$" => "text-decoration:underline;",
        _ => "",
    }
}

fn member_labels(ms: &[Member]) -> Vec<(Label, &'static str, String)> {
    ms.iter()
        .map(|m| {
            let css = classifier_style(m);
            let style = if m.classifier == "*" {
                FontStyle::Italic
            } else {
                FontStyle::Regular
            };
            (text_label(&m.text, style), css, m.text.clone())
        })
        .collect()
}

fn class_box(c: &Class, hide_empty: bool) -> ClassBox {
    const GAP: f64 = PADDING;
    let annotation = c
        .annotations
        .first()
        .map(|a| text_label(&format!("«{a}»"), FontStyle::Regular));
    let name = text_label(&c.text, FontStyle::Bold);
    let members = member_labels(&c.members);
    let methods = member_labels(&c.methods);

    // textHelper: the groups before the box is centered.
    let ah = annotation.as_ref().map_or(0.0, |a| a.height);
    let lh = name.height;
    let members_h: f64 = members.iter().map(|m| m.0.height).sum();
    let mh = if members_h <= 0.0 {
        GAP / 2.0
    } else {
        members_h
    };
    let members_y = ah + lh + GAP * 2.0;
    let methods_y = ah + lh + mh + GAP * 4.0;
    // Bounding box of what is drawn: (x0, y0, x1, y1).
    let mut bb: Option<(f64, f64, f64, f64)> = None;
    let mut add = |x0: f64, y0: f64, x1: f64, y1: f64| {
        bb = Some(match bb {
            None => (x0, y0, x1, y1),
            Some(b) => (b.0.min(x0), b.1.min(y0), b.2.max(x1), b.3.max(y1)),
        });
    };
    if let Some(a) = &annotation {
        add(-a.width / 2.0, a.top(), a.width / 2.0, a.top() + a.height);
    }
    add(
        -name.width / 2.0,
        ah + name.top(),
        name.width / 2.0,
        ah + name.top() + name.height,
    );
    let mut y = 0.0;
    for (l, _, _) in &members {
        add(
            0.0,
            members_y + y + l.top(),
            l.width,
            members_y + y + l.top() + l.height,
        );
        y += l.height;
    }
    let mut y = 0.0;
    for (l, _, _) in &methods {
        add(
            0.0,
            methods_y + y + l.top(),
            l.width,
            methods_y + y + l.top() + l.height,
        );
        y += l.height;
    }
    let (bx0, by0, bx1, by1) = bb.unwrap_or_default();
    let w = bx1 - bx0;
    let mut h = by1 - by0;

    // classBox.
    let none = c.members.is_empty() && c.methods.is_empty();
    let extra = none && !hide_empty;
    if none {
        h += GAP;
    } else if !c.members.is_empty() && c.methods.is_empty() {
        h += GAP * 2.0;
    }
    let x = -w / 2.0;
    let y = -h / 2.0;
    let extra_height = if extra {
        PADDING * 2.0
    } else if none {
        -PADDING
    } else {
        0.0
    };
    let shift = if extra {
        PADDING
    } else if none {
        -PADDING / 2.0
    } else {
        0.0
    };
    let rx = x - PADDING;
    let ry = y - PADDING - shift;
    let rw = w + 2.0 * PADDING;
    let rh = h + 2.0 * PADDING + extra_height;
    let cut = if extra { PADDING / 2.0 } else { 0.0 };
    let ah_b = ah - cut;
    let lh_b = lh - cut;
    let mh_b = members_h - cut;
    let base = y + PADDING - shift;
    let mut ty = [0.0, ah + base, members_y + base, 0.0];
    ty[0] = base;
    ty[3] = ah_b + lh_b + mh_b.max(GAP / 2.0) + y + GAP * 4.0 + PADDING;
    if none && hide_empty {
        ty = [
            if c.annotations.is_empty() { 0.0 } else { -GAP },
            ah - if c.annotations.is_empty() { 0.0 } else { GAP },
            members_y,
            methods_y,
        ];
    }
    let groups = [
        (-annotation.as_ref().map_or(0.0, |a| a.width) / 2.0, ty[0]),
        (-name.width / 2.0, ty[1]),
        (x, ty[2]),
        (x, ty[3]),
    ];
    let mut dividers = Vec::new();
    if !none || extra {
        dividers.push(ah_b + lh_b + y + PADDING);
        dividers.push(ah_b + lh_b + mh_b + y + GAP * 2.0 + PADDING);
    }
    ClassBox {
        annotation,
        name,
        members,
        methods,
        w: rw,
        h: rh,
        rx,
        ry,
        groups,
        dividers,
    }
}

fn write_text_item(out: &mut String, l: &Label, css: &str, y: f64, raw: &str, span_style: &str) {
    let _ = write!(
        out,
        r#"<g class="label" style="{}" transform="translate(0,{})"><foreignObject width="{}" height="{}"><div xmlns="http://www.w3.org/1999/xhtml" style="display: table-cell; white-space: nowrap; line-height: 1.5; max-width: {}px; text-align: center;"><span class="nodeLabel markdown-node-label" style="{}"><p>{}</p></span></div></foreignObject></g>"#,
        esc(css),
        js(y),
        js(l.width),
        js(l.height),
        js(max_width(raw)),
        esc(span_style),
        l.html
    );
}

#[allow(clippy::too_many_arguments)]
fn write_class(out: &mut String, id: &str, c: &Class, b: &ClassBox, color: usize, center: Point) {
    let props = class_styles(c);
    let styles = style_string(&props);
    let span_style = styles.clone();
    let mut classes = format!("node {}", c.css_classes);
    if c.have_callback {
        classes.push_str("  clickable");
    }
    let title = c
        .tooltip
        .as_deref()
        .map(|t| format!(r#" title="{}""#, esc(t)))
        .unwrap_or_default();
    let transform = format!(r#"translate({}, {})"#, js(center.x), js(center.y));
    match &c.link {
        Some(link) => {
            let _ = write!(
                out,
                r#"<a xlink:href="{}" data-look="neo" transform="{transform}"><g class="{}" id="{id}-{}" data-color-id="color-{}"{title}>"#,
                esc(link),
                esc(&classes),
                esc(&c.dom_id),
                color % COLOR_SLOTS
            );
        }
        None => {
            let _ = write!(
                out,
                r#"<g class="{}" id="{id}-{}" data-color-id="color-{}" data-look="neo"{title} transform="{transform}">"#,
                esc(&classes),
                esc(&c.dom_id),
                color % COLOR_SLOTS
            );
        }
    }
    let paint = Paint::new(&props, MAIN_BKG, NODE_BORDER, styles.clone());
    out.push_str(r#"<g class="basic label-container outer-path">"#);
    rough_rect(out, b.rx, b.ry, b.w, b.h, &paint);
    out.push_str("</g>");
    let group = |out: &mut String, class: &str, k: usize| {
        let _ = write!(
            out,
            r#"<g class="{class} text" transform="translate({}, {})">"#,
            js(b.groups[k].0),
            js(b.groups[k].1)
        );
    };
    group(out, "annotation-group", 0);
    if let (Some(a), Some(raw)) = (&b.annotation, c.annotations.first()) {
        write_text_item(out, a, "", a.top(), &format!("«{raw}»"), &span_style);
    }
    out.push_str("</g>");
    group(out, "label-group", 1);
    write_text_item(
        out,
        &b.name,
        "font-weight: bolder",
        b.name.top(),
        &c.text,
        &span_style,
    );
    out.push_str("</g>");
    for (k, (class, items)) in [("members-group", &b.members), ("methods-group", &b.methods)]
        .into_iter()
        .enumerate()
    {
        group(out, class, k + 2);
        let mut y = 0.0;
        for (l, css, raw) in items {
            write_text_item(out, l, css, l.top() + y, raw, &span_style);
            y += l.height;
        }
        out.push_str("</g>");
    }
    for &y in &b.dividers {
        let _ = write!(
            out,
            r#"<g class="divider neo-line" style="{}"><path d=""#,
            esc(&styles)
        );
        rough_line(
            out,
            Point { x: b.rx, y },
            Point {
                x: b.rx + b.w,
                y: y + 0.001,
            },
        );
        let _ = write!(
            out,
            r#"" stroke="{}" stroke-width="{}" fill="none" stroke-dasharray="{}" style="{}"></path></g>"#,
            esc(&paint.stroke),
            esc(&paint.width),
            esc(&paint.dash),
            esc(&styles)
        );
    }
    out.push_str("</g>");
    if c.link.is_some() {
        out.push_str("</a>");
    }
}

// ---------------------------------------------------------------------------
// Links

fn marker(kind: RelationKind) -> &'static str {
    match kind {
        RelationKind::Aggregation => "aggregation",
        RelationKind::Extension => "extension",
        RelationKind::Composition => "composition",
        RelationKind::Dependency => "dependency",
        RelationKind::Lollipop => "lollipop",
    }
}

/// `markerOffsets`: the line stops short of a marker by this much.
fn marker_offset(kind: Option<RelationKind>) -> f64 {
    match kind {
        Some(RelationKind::Dependency) => 6.0,
        Some(RelationKind::Lollipop) => 13.5,
        Some(_) => 17.25,
        None => 0.0,
    }
}

/// Moves `p` by `by` towards `toward`.
fn toward(p: Point, to: Point, by: f64) -> Point {
    let l = dist(p, to);
    if l < 1e-9 {
        return p;
    }
    Point {
        x: p.x + (to.x - p.x) / l * by,
        y: p.y + (to.y - p.y) / l * by,
    }
}

/// The point `d` along a polyline (`calculatePoint`).
fn point_along(pts: &[Point], d: f64) -> Point {
    let mut left = d;
    for w in pts.windows(2) {
        let l = dist(w[0], w[1]);
        if l >= left {
            return toward(w[0], w[1], left);
        }
        left -= l;
    }
    pts.last().copied().unwrap_or_default()
}

/// Where a cardinality goes (`calcTerminalLabelPosition`, with the 10px
/// marker allowance Mermaid always applies).
fn terminal_position(pts: &[Point], at_start: bool) -> Point {
    let mut pts = pts.to_vec();
    if !at_start {
        pts.reverse();
    }
    let center = point_along(&pts, 35.0);
    let d = 15.0;
    let p0 = pts[0];
    let a = (p0.y - center.y).atan2(p0.x - center.x);
    Point {
        x: a.sin() * d + (p0.x + center.x) / 2.0,
        y: -a.cos() * d + (p0.y + center.y) / 2.0,
    }
}

fn dash_array(len: f64, dashed: bool) -> String {
    if dashed {
        let pairs = (len / 4.0).floor().max(0.0) as usize;
        format!("0 0 {} 0", vec!["2 2"; pairs].join(" "))
    } else {
        format!("0 0 {} 0", js(len))
    }
}

/// A link as drawn: id, ends, markers and labels.
struct Link {
    id: String,
    from: usize,
    to: usize,
    start: Option<RelationKind>,
    end: Option<RelationKind>,
    pattern: &'static str,
    style: &'static str,
    label: Option<Label>,
    start_label: Option<(Label, usize)>,
    end_label: Option<(Label, usize)>,
}

// ---------------------------------------------------------------------------
// Diagram

pub(crate) fn render(d: &ClassDiagram, o: &RenderOptions) -> String {
    let id = esc(&o.id_prefix);
    let dir = Direction::from_keyword(&d.direction).unwrap_or_default();
    let class_cfg = |key: &str| d.config.lookup("class", key);
    let hide_empty = class_cfg("hideEmptyMembersBox")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let nc = d.classes.len();
    let nn = d.notes.len();
    // With `hierarchicalNamespaces: false`, only declared namespaces are
    // drawn, flat, and their contents move to the nearest declared one.
    let hierarchical = class_cfg("hierarchicalNamespaces")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let ns_pos = |id: &str| d.namespaces.iter().position(|n| n.id == id);
    let ns_index = |ns: &Option<String>| -> Option<usize> {
        let mut cur = ns_pos(ns.as_deref()?)?;
        if hierarchical {
            return Some(cur);
        }
        loop {
            let n = &d.namespaces[cur];
            if n.explicit {
                return Some(cur);
            }
            cur = ns_pos(n.parent.as_deref()?)?;
        }
    };
    let drawn = |n: &super::Namespace| hierarchical || n.explicit;
    let class_index = |name: &str| d.classes.iter().position(|c| c.id == name);

    // Nodes: classes, then notes, then interfaces.
    let boxes: Vec<ClassBox> = d.classes.iter().map(|c| class_box(c, hide_empty)).collect();
    let note_labels: Vec<Label> = d
        .notes
        .iter()
        .map(|n| text_label(&n.text, FontStyle::Regular))
        .collect();
    let iface_labels: Vec<Label> = d
        .interfaces
        .iter()
        .map(|i| text_label(&i.label, FontStyle::Regular))
        .collect();
    let mut gnodes: Vec<GNode> = Vec::with_capacity(nc + nn + d.interfaces.len());
    for (c, b) in d.classes.iter().zip(&boxes) {
        gnodes.push(GNode {
            w: b.w,
            h: b.h,
            cluster: ns_index(&c.parent),
        });
    }
    for (n, l) in d.notes.iter().zip(&note_labels) {
        gnodes.push(GNode {
            w: l.width + 2.0 * NOTE_PADDING,
            h: l.height.max(LINE_HEIGHT) + 2.0 * NOTE_PADDING,
            cluster: ns_index(&n.parent),
        });
    }
    for l in &iface_labels {
        gnodes.push(GNode {
            w: l.width + 32.0,
            h: LINE_HEIGHT + 24.0,
            cluster: None,
        });
    }
    let node_of = |name: &str| -> Option<usize> {
        if let Some(i) = class_index(name) {
            return Some(i);
        }
        name.strip_prefix("interface")
            .and_then(|k| k.parse::<usize>().ok())
            .filter(|&k| k < d.interfaces.len())
            .map(|k| nc + nn + k)
    };

    // Links: notes first, then relations.
    let mut links: Vec<Link> = Vec::new();
    for (k, n) in d.notes.iter().enumerate() {
        let Some(c) = n.class.as_deref().and_then(class_index) else {
            continue;
        };
        links.push(Link {
            id: format!("edgeNote{k}"),
            from: nc + k,
            to: c,
            start: None,
            end: None,
            pattern: "dotted",
            style: "fill: none;;;fill: none",
            label: None,
            start_label: None,
            end_label: None,
        });
    }
    for (k, r) in d.relations.iter().enumerate() {
        let (Some(from), Some(to)) = (node_of(&r.id1), node_of(&r.id2)) else {
            continue;
        };
        let terminal = |t: &str| {
            (t != "none" && !t.is_empty()).then(|| {
                (
                    label(t, FontStyle::Regular, TERMINAL_FONT_SIZE, TERMINAL_HEIGHT),
                    shown_text(t).encode_utf16().count(),
                )
            })
        };
        links.push(Link {
            id: format!("id_{}_{}_{}", r.id1, r.id2, k + 1),
            from,
            to,
            start: r.type1,
            end: r.type2,
            pattern: if r.dotted { "dashed" } else { "solid" },
            style: ";;;",
            label: r
                .title
                .as_deref()
                .filter(|t| !t.is_empty())
                .map(|t| text_label(t, FontStyle::Regular)),
            start_label: terminal(&r.title1),
            end_label: terminal(&r.title2),
        });
    }
    let gedges: Vec<GEdge> = links
        .iter()
        .map(|l| GEdge {
            from: End::Node(l.from),
            to: End::Node(l.to),
            min_len: 1,
            label: l.label.as_ref().map(|l| (l.width, l.height)),
        })
        .collect();
    let ns_labels: Vec<Label> = d
        .namespaces
        .iter()
        .map(|n| {
            text_label(
                if hierarchical { &n.label } else { &n.id },
                FontStyle::Regular,
            )
        })
        .collect();
    let gclusters: Vec<GCluster> = d
        .namespaces
        .iter()
        .zip(&ns_labels)
        .map(|(n, l)| GCluster {
            parent: if hierarchical {
                ns_index(&n.parent)
            } else {
                None
            },
            title: (l.width, l.height),
        })
        .collect();
    let extent = |_: usize, w: f64, h: f64, vertical: bool, _: f64, _: bool| {
        if vertical {
            h / 2.0
        } else {
            w / 2.0
        }
    };
    let geo = elk::geometry(&Graph {
        dir,
        nodes: &gnodes,
        edges: &gedges,
        clusters: &gclusters,
        spacing: Spacing {
            node: 40.0,
            rank: 40.0,
            edge: 20.0,
            cluster_padding: 24.0,
            title_gap: 3.0,
            margin: 12.0,
        },
        extent: &extent,
    });

    // Bounds, with the title.
    let (mut x0, mut y0, mut x1, y1) = geo.bounds;
    let title = d.title.as_deref().filter(|t| !t.trim().is_empty());
    let title_x = (x0 + x1) / 2.0;
    if let Some(t) = title {
        let tw = arial_width(t, FONT_SIZE, FontStyle::Regular);
        x0 = x0.min(title_x - tw / 2.0);
        x1 = x1.max(title_x + tw / 2.0);
        y0 = y0.min(-25.0 - 0.905 * FONT_SIZE);
    }
    let (vx, vy) = (x0 - VIEW_PADDING, y0 - VIEW_PADDING);
    let (vw, vh) = (x1 - x0 + 2.0 * VIEW_PADDING, y1 - y0 + 2.0 * VIEW_PADDING);

    let mut out = String::with_capacity(16 * 1024);
    let acc_title = d.acc_title.as_deref().filter(|t| !t.is_empty());
    let acc_descr = d.acc_descr.as_deref().filter(|t| !t.is_empty());
    let _ = write!(
        out,
        r#"<svg id="{id}" width="100%" xmlns="http://www.w3.org/2000/svg" class="classDiagram" style="max-width: {}px;" viewBox="{} {} {} {}" role="graphics-document document" aria-roledescription="classDiagram""#,
        js(vw),
        js(vx),
        js(vy),
        js(vw),
        js(vh)
    );
    if acc_descr.is_some() {
        let _ = write!(out, r#" aria-describedby="chart-desc-{id}""#);
    }
    if acc_title.is_some() {
        let _ = write!(out, r#" aria-labelledby="chart-title-{id}""#);
    }
    out.push('>');
    if let Some(t) = acc_title {
        let _ = write!(out, r#"<title id="chart-title-{id}">{}</title>"#, esc(t));
    }
    if let Some(t) = acc_descr {
        let _ = write!(out, r#"<desc id="chart-desc-{id}">{}</desc>"#, esc(t));
    }
    out.push_str("<style>");
    out.push_str(&CSS.replace("#ID", &format!("#{id}")));
    out.push_str(&class_css(d, &id));
    out.push_str("</style><g>");
    out.push_str(&MARKERS.replace("id=\"ID_", &format!("id=\"{id}_")));

    // Namespaces, outer ones first.
    out.push_str(r#"<g class="root"><g class="clusters">"#);
    let mut order: Vec<usize> = (0..d.namespaces.len()).collect();
    order.sort_by_key(|&s| elk::depth(&gclusters, s));
    for s in order {
        if !drawn(&d.namespaces[s]) {
            continue;
        }
        let Some(r) = geo.cluster_rects[s] else {
            continue;
        };
        let l = &ns_labels[s];
        let _ = write!(
            out,
            r#"<g class="cluster undefined" id="{id}-{}" data-look="neo"><rect style="" x="{}" y="{}" width="{}" height="{}"></rect><g class="cluster-label" transform="translate({}, {})"><foreignObject width="{}" height="{}"><div xmlns="http://www.w3.org/1999/xhtml" style="display: table-cell; white-space: nowrap; line-height: 1.5;"><span class="nodeLabel"><p>{}</p></span></div></foreignObject></g></g>"#,
            esc(&d.namespaces[s].id),
            js(r.x),
            js(r.y),
            js(r.w),
            js(r.h),
            js(r.x + (r.w - l.width) / 2.0),
            js(r.y),
            js(l.width),
            js(l.height),
            l.html
        );
    }

    // Link paths.
    out.push_str(r#"</g><g class="edges edgePaths">"#);
    for (k, l) in links.iter().enumerate() {
        let Some(line) = geo.polylines[k].as_ref().filter(|p| p.len() >= 2) else {
            continue;
        };
        let mut pts = line.clone();
        let n = pts.len();
        let (os, oe) = (marker_offset(l.start), marker_offset(l.end));
        if os > 0.0 {
            pts[0] = toward(pts[0], pts[1], os);
        }
        if oe > 0.0 {
            pts[n - 1] = toward(pts[n - 1], pts[n - 2], oe);
        }
        let len = rounded_length(&pts);
        let dash = dash_array(len, l.pattern != "solid");
        let _ = write!(
            out,
            r#"<path d="{}" id="{id}-{}" class="edge-thickness-normal edge-pattern-{} relation" style="stroke-dasharray: {dash}; stroke-dashoffset: 0;{}" data-edge="true" data-et="edge" data-id="{}" data-points="{}" data-look="neo""#,
            rounded_path(&pts),
            esc(&l.id),
            l.pattern,
            l.style,
            esc(&l.id),
            base64(points_json(line).as_bytes())
        );
        if let Some(m) = l.start {
            let _ = write!(
                out,
                r#" marker-start="url(#{id}_classDiagram-{}Start-margin)""#,
                marker(m)
            );
        }
        if let Some(m) = l.end {
            let _ = write!(
                out,
                r#" marker-end="url(#{id}_classDiagram-{}End-margin)""#,
                marker(m)
            );
        }
        out.push_str("></path>");
    }

    // Link labels and cardinalities.
    out.push_str(r#"</g><g class="edgeLabels">"#);
    for (k, l) in links.iter().enumerate() {
        let Some(line) = geo.polylines[k].as_ref().filter(|p| p.len() >= 2) else {
            continue;
        };
        if l.label.is_none() && l.start_label.is_none() && l.end_label.is_none() {
            continue;
        }
        // Self loops get no label slot from the layout: the label then sits
        // beside the loop, at its middle.
        let slot = geo.label_rects[k].map(|r| r.center()).or_else(|| {
            let lb = l.label.as_ref()?;
            let total: f64 = line.windows(2).map(|w| dist(w[0], w[1])).sum();
            let m = point_along(line, total / 2.0);
            Some(Point {
                x: m.x - lb.width / 2.0,
                y: m.y,
            })
        });
        match (&l.label, slot) {
            (Some(lb), Some(c)) => {
                let _ = write!(
                    out,
                    r#"<g class="edgeLabel" transform="translate({}, {})"><g class="label" data-id="{}" transform="translate({}, {})"><foreignObject width="{}" height="{}"><div xmlns="http://www.w3.org/1999/xhtml" class="labelBkg" style="display: table-cell; white-space: nowrap; line-height: 1.5; max-width: 200px; text-align: center;"><span class="edgeLabel"><p>{}</p></span></div></foreignObject></g></g>"#,
                    js(c.x),
                    js(c.y),
                    esc(&l.id),
                    js(-lb.width / 2.0),
                    js(-lb.height / 2.0),
                    js(lb.width),
                    js(lb.height),
                    lb.html
                );
            }
            _ => {
                let _ = write!(
                    out,
                    r#"<g class="edgeLabel"><g class="label" data-id="{}" transform="translate(0, 0)"><foreignObject width="0" height="0"><div xmlns="http://www.w3.org/1999/xhtml" class="labelBkg" style="display: table-cell; white-space: nowrap; line-height: 1.5; max-width: 200px; text-align: center;"><span class="edgeLabel"></span></div></foreignObject></g></g>"#,
                    esc(&l.id)
                );
            }
        }
        for (t, at_start) in [(&l.start_label, true), (&l.end_label, false)] {
            let Some((t, chars)) = t else { continue };
            let p = terminal_position(line, at_start);
            let _ = write!(
                out,
                r#"<g class="edgeTerminals" transform="translate({}, {})"><g class="inner" transform="translate({}, {})"><foreignObject width="{}" height="{}" style="width: {}px; height: {}px;"><div xmlns="http://www.w3.org/1999/xhtml" style="display: table-cell; white-space: nowrap; line-height: 1.5;"><span class="edgeLabel"><p>{}</p></span></div></foreignObject></g></g>"#,
                js(p.x),
                js(p.y),
                js(-t.width / 2.0),
                js(-t.height / 2.0),
                js(t.width),
                js(t.height),
                js((*chars as f64 * 9.0).max(t.width)),
                js(t.height),
                t.html
            );
        }
    }

    // Nodes.
    out.push_str(r#"</g><g class="nodes">"#);
    for (i, (c, b)) in d.classes.iter().zip(&boxes).enumerate() {
        write_class(&mut out, &id, c, b, i, geo.node_rects[i].center());
    }
    let note_style = "fill:#fff5ad !important;stroke:#FACC15 !important";
    let note_label_style = "text-align:left !important;white-space:nowrap !important";
    for (k, (n, l)) in d.notes.iter().zip(&note_labels).enumerate() {
        let r = geo.node_rects[nc + k];
        let c = r.center();
        let _ = write!(
            out,
            r#"<g class="node undefined" id="{id}-{}" data-look="neo" transform="translate({}, {})"><g class="basic label-container outer-path">"#,
            esc(&n.id),
            js(c.x),
            js(c.y)
        );
        let paint = Paint {
            fill: NOTE_BKG.into(),
            stroke: NOTE_BORDER.into(),
            width: "1.3".into(),
            dash: "0 0".into(),
            style: note_style.into(),
        };
        rough_rect(&mut out, -r.w / 2.0, -r.h / 2.0, r.w, r.h, &paint);
        let _ = write!(
            out,
            r#"</g><g class="label noteLabel" style="{note_label_style}" transform="translate({}, {})"><rect></rect><foreignObject width="{}" height="{}"><div style="text-align: center; {}" xmlns="http://www.w3.org/1999/xhtml"><span style="{note_label_style}" class="nodeLabel markdown-node-label"><p>{}</p></span></div></foreignObject></g></g>"#,
            js(-l.width / 2.0),
            js(-l.height / 2.0),
            js(l.width),
            js(l.height),
            if l.width > NOTE_WRAP {
                "white-space: break-spaces; display: table; line-height: 1.5; max-width: 120px; width: 120px;"
            } else {
                "white-space: nowrap; display: table-cell; line-height: 1.5; max-width: 120px;"
            },
            l.html
        );
    }
    for (k, (i, l)) in d.interfaces.iter().zip(&iface_labels).enumerate() {
        let r = geo.node_rects[nc + nn + k];
        let c = r.center();
        let _ = write!(
            out,
            r#"<g class="node undefined" id="{id}-{}" data-look="neo" transform="translate({}, {})"><rect class="basic label-container" style="opacity:0; !important" x="{}" y="{}" width="{}" height="{}"></rect><g class="label" style="" transform="translate({}, {})"><rect></rect><foreignObject width="{}" height="{}"><div xmlns="http://www.w3.org/1999/xhtml" style="display: table-cell; white-space: nowrap; line-height: 1.5; max-width: 120px; text-align: center;"><span class="nodeLabel"><p>{}</p></span></div></foreignObject></g></g>"#,
            esc(&i.id),
            js(c.x),
            js(c.y),
            js(-r.w / 2.0),
            js(-r.h / 2.0),
            js(r.w),
            js(r.h),
            js(-l.width / 2.0),
            js(-LINE_HEIGHT / 2.0),
            js(l.width),
            js(l.height),
            l.html
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
            r#"<text text-anchor="middle" x="{}" y="-25" class="classDiagramTitleText">{}</text>"#,
            js(title_x),
            esc(t)
        );
    }
    out.push_str("</svg>");
    out
}
