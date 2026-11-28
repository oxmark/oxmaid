//! Sequence diagram rendering, a step-by-step port of Mermaid 12.3's
//! `sequenceRenderer.ts` and `svgDraw.js` for its default look (`neo`) and
//! theme (`redux-color`). The classic look keeps its geometry where it does
//! not depend on the browser measuring drawn shapes.
//!
//! Mermaid measures labels in the browser. The measurement text there gets
//! the configured font family, `"trebuchet ms", verdana, arial, sans-serif;`,
//! whose trailing `;` makes it invalid CSS: the browser falls back to its
//! default serif font, so layout uses Times New Roman metrics, while drawn
//! text inherits the stylesheet's Arial. Both are reproduced here.

use std::collections::HashMap;

use super::{LineType, Placement, SequenceDiagram};
use crate::config::Value;
use crate::dom::{js_num, Dom, NodeId};
use crate::options::RenderOptions;
use crate::text::{has_breaks, split_breaks, svg_text_height, text_dimensions, SvgFont, TextDims};

const CSS: &str = include_str!("neo.css");

const GLYPH_BAND_HEIGHT: f64 = 44.0;
const GLYPH_LABEL_GAP: f64 = 6.0;
const LABEL_LIFELINE_GAP: f64 = 6.0;
const ACTOR_TYPE_WIDTH: f64 = 36.0;
const ACTOR_GLYPH_BOTTOM: f64 = 60.0;
const ACTOR_GLYPH_HEIGHT: f64 = 65.0;

const BORDER_COLORS: [&str; 12] = [
    "#E879F9", "#2DD4BF", "#FB923C", "#22D3EE", "#4ADE80", "#A78BFA", "#F87171", "#FACC15",
    "#818CF8", "#A3E635 ", "#38BDF8", "#FB7185",
];
const BKG_COLORS: [&str; 12] = [
    "#FDF4FF", "#F0FDFA", "#FFF7ED", "#ECFEFF", "#F0FDF4", "#F5F3FF", "#FEF2F2", "#FEFCE8",
    "#EEF2FF", "#F7FEE7", "#F0F9FF", "#FFF1F2",
];
const RECT_BKG_COLOR: &str = "hsl(0, 0%, 96%)";
const ACTOR_BORDER: &str = "#28253D";
const ACTOR_BKG: &str = "#ffffff";

const DATABASE_ICON: &str = include_str!("database_icon.txt");

// ---------------------------------------------------------------------------
// Configuration

/// A font as the renderer uses it: the size for arithmetic, the CSS values
/// for inline styles, and the fonts the browser would resolve.
#[derive(Clone, Debug)]
struct Font {
    size: f64,
    size_px: String,
    weight: String,
    /// Inline `font-family`, when the configured value is valid CSS.
    family: Option<String>,
    /// Font used by `calculateTextDimensions` (no inherited style).
    measure: SvgFont,
    /// Font of drawn text (inherits the stylesheet's Arial).
    drawn: SvgFont,
}

impl Font {
    fn dims(&self, text: &str) -> TextDims {
        text_dimensions(text, self.size, self.measure)
    }

    fn width(&self, text: &str) -> f64 {
        self.dims(text).width
    }
}

struct Conf {
    width: f64,
    height: f64,
    actor_margin: f64,
    box_margin: f64,
    box_text_margin: f64,
    note_margin: f64,
    mirror_actors: bool,
    bottom_margin_adj: f64,
    diagram_margin_x: f64,
    diagram_margin_y: f64,
    activation_width: f64,
    wrap_padding: f64,
    label_box_width: f64,
    label_box_height: f64,
    message_align: String,
    note_align: String,
    right_angles: bool,
    show_sequence_numbers: bool,
    hide_unused: bool,
    use_max_width: bool,
    force_menus: bool,
    actor_font: Font,
    note_font: Font,
    message_font: Font,
    neo: bool,
    color_theme: bool,
    theme: String,
}

/// Resolves a CSS `font-family` list to one of the fonts with metrics.
/// `None` when the value is not valid CSS.
fn resolve_family(family: &str) -> Option<SvgFont> {
    if family.contains(';') || family.contains('{') || family.contains('}') {
        return None;
    }
    for f in family.split(',') {
        let name = f
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .to_ascii_lowercase();
        match name.as_str() {
            "arial" | "helvetica" | "sans-serif" | "liberation sans" | "arimo" => {
                return Some(SvgFont::Arial)
            }
            "times new roman" | "times" | "serif" | "liberation serif" | "tinos" => {
                return Some(SvgFont::Times)
            }
            _ => {}
        }
    }
    Some(SvgFont::Times)
}

fn value_str(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.clone()),
        Value::Num(n) => Some(js_num(*n)),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Mermaid's `parseFontSize`.
fn parse_font_size(v: &Value) -> Option<(f64, String)> {
    match v {
        Value::Num(n) => Some((*n, format!("{}px", js_num(*n)))),
        Value::Str(s) => {
            let t = s.trim_start();
            let digits: String = t
                .chars()
                .enumerate()
                .take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && (*c == '-' || *c == '+')))
                .map(|(_, c)| c)
                .collect();
            let n: i64 = digits.parse().ok()?;
            if s == &n.to_string() {
                Some((n as f64, format!("{s}px")))
            } else {
                Some((n as f64, s.clone()))
            }
        }
        _ => None,
    }
}

impl Conf {
    fn new(d: &SequenceDiagram) -> Conf {
        let raw = d.config.raw.as_ref();
        let seq = |k: &str| raw.and_then(|r| r.path(&format!("sequence.{k}")));
        let top = |k: &str| raw.and_then(|r| r.get(k));
        let num = |k: &str, def: f64| seq(k).and_then(Value::as_f64).unwrap_or(def);
        let flag = |k: &str, def: bool| seq(k).and_then(Value::as_bool).unwrap_or(def);
        let s = |k: &str, def: &str| seq(k).and_then(value_str).unwrap_or_else(|| def.into());
        let look = d.config.look.clone().unwrap_or_else(|| "neo".into());
        let theme = d
            .config
            .theme
            .clone()
            .unwrap_or_else(|| "redux-color".into());

        // `setConf`: the top-level font settings win over the per-kind ones.
        // Mermaid's defaults always carry a top-level `fontFamily` and
        // `fontSize`, so `actorFontFamily`, `noteFontSize`, ... only matter
        // when the top-level value is falsy.
        let default_family = "\"trebuchet ms\", verdana, arial, sans-serif;".to_string();
        let font = |kind: &str, def_size: f64, def_family: &str| {
            let falsy = |v: &&Value| match v {
                Value::Num(n) => *n == 0.0 || n.is_nan(),
                Value::Str(s) => s.is_empty(),
                Value::Bool(b) => !*b,
                Value::Null => true,
                _ => false,
            };
            let size_v = match top("fontSize") {
                None => Some(Value::Num(16.0)),
                Some(v) if falsy(&v) => seq(&format!("{kind}FontSize")).cloned(),
                Some(v) => Some(v.clone()),
            }
            .unwrap_or(Value::Num(def_size));
            let (size, size_px) =
                parse_font_size(&size_v).unwrap_or((def_size, format!("{def_size}px")));
            let family = match top("fontFamily") {
                None => Some(default_family.clone()),
                Some(v) if falsy(&v) => seq(&format!("{kind}FontFamily")).and_then(value_str),
                Some(v) => value_str(v),
            }
            .unwrap_or_else(|| def_family.to_string());
            let weight = top("fontWeight")
                .or_else(|| seq(&format!("{kind}FontWeight")))
                .and_then(value_str)
                .unwrap_or_else(|| "400".into());
            let resolved = resolve_family(&family);
            Font {
                size,
                size_px,
                weight,
                family: resolved.map(|_| family.trim().to_string()),
                measure: resolved.unwrap_or(SvgFont::Times),
                drawn: resolved.unwrap_or(SvgFont::Arial),
            }
        };
        let actor_font = font("actor", 16.0, &default_family);
        let note_font = font("note", 16.0, &default_family);
        let message_font = font("message", 16.0, &default_family);
        Conf {
            width: num("width", 150.0),
            height: num("height", 65.0),
            actor_margin: num("actorMargin", 50.0),
            box_margin: num("boxMargin", 10.0),
            box_text_margin: num("boxTextMargin", 5.0),
            note_margin: num("noteMargin", 10.0),
            mirror_actors: flag("mirrorActors", true),
            bottom_margin_adj: num("bottomMarginAdj", 1.0),
            diagram_margin_x: num("diagramMarginX", 50.0),
            diagram_margin_y: num("diagramMarginY", 10.0),
            activation_width: num("activationWidth", 10.0),
            wrap_padding: num("wrapPadding", 10.0),
            label_box_width: num("labelBoxWidth", 50.0),
            label_box_height: num("labelBoxHeight", 20.0),
            message_align: s("messageAlign", "center"),
            note_align: s("noteAlign", "center"),
            right_angles: flag("rightAngles", false),
            show_sequence_numbers: flag("showSequenceNumbers", false),
            hide_unused: flag("hideUnusedParticipants", false),
            use_max_width: flag("useMaxWidth", true),
            force_menus: flag("forceMenus", false),
            actor_font,
            note_font,
            message_font,
            neo: look == "neo",
            color_theme: theme == "redux-color" || theme == "redux-dark-color",
            theme,
        }
    }
}

/// Loop widths by message index, then the note and message models.
type LoopBounds = (
    HashMap<usize, f64>,
    Vec<Option<NoteModel>>,
    Vec<Option<MsgModel>>,
);

// ---------------------------------------------------------------------------
// Text helpers

/// Mermaid's `wrapLabel`.
fn wrap_label(label: &str, max_width: f64, font: &Font) -> String {
    if label.is_empty() || has_breaks(label) {
        return label.to_string();
    }
    let words: Vec<&str> = label.split(' ').filter(|w| !w.is_empty()).collect();
    let mut lines: Vec<String> = Vec::new();
    let mut next = String::new();
    for (i, word) in words.iter().enumerate() {
        let word_len = font.width(&format!("{word} "));
        let next_len = font.width(&next);
        if word_len > max_width {
            let (hyphenated, remaining) = break_string(word, max_width, font);
            lines.push(std::mem::take(&mut next));
            lines.extend(hyphenated);
            next = remaining;
        } else if next_len + word_len >= max_width {
            lines.push(std::mem::replace(&mut next, (*word).to_string()));
        } else if next.is_empty() {
            next = (*word).to_string();
        } else {
            next = format!("{next} {word}");
        }
        if i + 1 == words.len() {
            lines.push(next.clone());
        }
    }
    lines
        .into_iter()
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("<br/>")
}

fn break_string(word: &str, max_width: f64, font: &Font) -> (Vec<String>, String) {
    let chars: Vec<char> = word.chars().collect();
    let mut lines = Vec::new();
    let mut current = String::new();
    for (i, c) in chars.iter().enumerate() {
        let next = format!("{current}{c}");
        if font.width(&next) >= max_width {
            if i + 1 == chars.len() {
                lines.push(next);
            } else {
                lines.push(format!("{next}-"));
            }
            current.clear();
        } else {
            current = next;
        }
    }
    (lines, current)
}

/// `getBBox().height` of drawn text lines.
fn drawn_height(line: &str, font: &Font) -> f64 {
    let line = if line.is_empty() { "\u{200b}" } else { line };
    svg_text_height(line, font.size, font.drawn)
}

fn n(v: f64) -> String {
    js_num(v)
}

/// A number that may be `undefined` in Mermaid, formatted like JavaScript.
fn on(v: Option<f64>) -> String {
    v.map_or_else(|| "NaN".into(), js_num)
}

// ---------------------------------------------------------------------------
// Model

struct Actor {
    id: String,
    name: String,
    description: String,
    wrap: bool,
    kind: String,
    group: Option<usize>,
    prev: Option<String>,
    next: Option<String>,
    links: Vec<(String, String)>,
    class: Option<String>,
    icon: Option<String>,
    width: f64,
    height: f64,
    margin: Option<f64>,
    x: f64,
    starty: f64,
    stopy: Option<f64>,
    text_height: f64,
    actor_cnt: Option<usize>,
    lifeline: Option<NodeId>,
    rect: Option<RectData>,
}

#[derive(Clone, Default)]
struct RectData {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    fill: Option<String>,
    stroke: Option<String>,
    rx: f64,
    ry: f64,
    class: Option<String>,
    name: Option<String>,
}

impl RectData {
    fn note() -> Self {
        RectData {
            width: 100.0,
            height: 100.0,
            fill: Some("#EDF2AE".into()),
            stroke: Some("#666".into()),
            ..Default::default()
        }
    }
}

struct BoxModel {
    name: Option<String>,
    fill: String,
    wrap: bool,
    actors: Vec<String>,
    margin: f64,
    text_max_height: f64,
    x: f64,
    y: f64,
    width: f64,
}

#[derive(Clone)]
struct Msg {
    id: usize,
    kind: LineType,
    from: Option<String>,
    to: Option<String>,
    text: String,
    wrap: bool,
    activate: bool,
    placement: Option<Placement>,
    central: Option<LineType>,
    auto: Option<super::Autonumber>,
    /// Set by `adjustLoopHeightForWrap`.
    width: Option<f64>,
}

#[derive(Clone, Default)]
struct NoteModel {
    width: f64,
    height: f64,
    startx: f64,
    stopx: f64,
    starty: f64,
    stopy: f64,
    message: String,
}

#[derive(Clone, Default)]
struct MsgModel {
    width: f64,
    height: f64,
    startx: f64,
    stopx: f64,
    starty: f64,
    stopy: f64,
    message: String,
    kind: Option<LineType>,
    from_bounds: f64,
    to_bounds: f64,
    sequence_index: f64,
    sequence_visible: bool,
    id: usize,
    from: String,
    to: String,
}

#[derive(Clone, Default)]
struct Item {
    startx: Option<f64>,
    starty: Option<f64>,
    stopx: Option<f64>,
    stopy: Option<f64>,
    title: Option<String>,
    fill: Option<String>,
    sections: Vec<f64>,
    section_titles: Vec<Option<String>>,
}

struct Activation {
    startx: f64,
    starty: f64,
    stopx: f64,
    stopy: Option<f64>,
    actor: Option<String>,
    anchored: Option<NodeId>,
}

#[derive(Default)]
struct Data {
    startx: Option<f64>,
    starty: Option<f64>,
    stopx: Option<f64>,
    stopy: Option<f64>,
}

fn upd(slot: &mut Option<f64>, v: f64, f: fn(f64, f64) -> f64) {
    *slot = Some(match *slot {
        None => v,
        Some(old) => f(v, old),
    });
}

fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

#[derive(Default)]
struct Bounds {
    data: Data,
    vertical_pos: f64,
    items: Vec<Item>,
    activations: Vec<Activation>,
}

impl Bounds {
    fn update_bounds(&mut self, startx: f64, starty: f64, stopx: f64, stopy: f64, bm: f64) {
        let mut cnt = 0usize;
        let len = self.items.len() as f64;
        for item in &mut self.items {
            cnt += 1;
            let n = len - cnt as f64 + 1.0;
            upd(&mut item.starty, starty - n * bm, js_min);
            upd(&mut item.stopy, stopy + n * bm, js_max);
            upd(&mut self.data.startx, startx - n * bm, js_min);
            upd(&mut self.data.stopx, stopx + n * bm, js_max);
            upd(&mut item.startx, startx - n * bm, js_min);
            upd(&mut item.stopx, stopx + n * bm, js_max);
            upd(&mut self.data.starty, starty - n * bm, js_min);
            upd(&mut self.data.stopy, stopy + n * bm, js_max);
        }
        for a in &mut self.activations {
            cnt += 1;
            let n = len - cnt as f64 + 1.0;
            a.starty = match Some(a.starty) {
                Some(old) => js_min(starty - n * bm, old),
                None => unreachable!(),
            };
            upd(&mut a.stopy, stopy + n * bm, js_max);
            upd(&mut self.data.startx, startx - n * bm, js_min);
            upd(&mut self.data.stopx, stopx + n * bm, js_max);
        }
    }

    fn insert(&mut self, startx: f64, starty: f64, stopx: f64, stopy: f64, bm: f64) {
        let (sx, ex) = (js_min(startx, stopx), js_max(startx, stopx));
        let (sy, ey) = (js_min(starty, stopy), js_max(starty, stopy));
        upd(&mut self.data.startx, sx, js_min);
        upd(&mut self.data.starty, sy, js_min);
        upd(&mut self.data.stopx, ex, js_max);
        upd(&mut self.data.stopy, ey, js_max);
        self.update_bounds(sx, sy, ex, ey, bm);
    }

    fn bump(&mut self, by: f64) {
        self.vertical_pos += by;
        self.data.stopy = Some(match self.data.stopy {
            None => self.vertical_pos,
            Some(s) => js_max(s, self.vertical_pos),
        });
    }

    fn new_loop(&mut self, title: Option<String>, fill: Option<String>) {
        self.items.push(Item {
            starty: Some(self.vertical_pos),
            title,
            fill,
            ..Default::default()
        });
    }

    fn activations_of(&self, actor: &str) -> usize {
        self.activations
            .iter()
            .filter(|a| a.actor.as_deref() == Some(actor))
            .count()
    }
}

// ---------------------------------------------------------------------------
// Renderer

struct R<'a> {
    c: Conf,
    id: String,
    dom: Dom,
    root: NodeId,
    actors: Vec<Actor>,
    index: HashMap<String, usize>,
    keys: Vec<usize>,
    /// Position of each drawn participant, for its palette color.
    palette_index: HashMap<String, usize>,
    boxes: Vec<BoxModel>,
    msgs: Vec<Msg>,
    created: Vec<(String, usize)>,
    destroyed: Vec<(String, usize)>,
    b: Bounds,
    box_models: Vec<usize>,
    actor_cnt: i64,
    sequence_numbers: bool,
    d: &'a SequenceDiagram,
}

const ARROWS: &[LineType] = &[
    LineType::SolidOpen,
    LineType::DottedOpen,
    LineType::Solid,
    LineType::SolidTop,
    LineType::SolidBottom,
    LineType::StickTop,
    LineType::StickBottom,
    LineType::SolidTopDotted,
    LineType::SolidBottomDotted,
    LineType::StickTopDotted,
    LineType::StickBottomDotted,
    LineType::SolidArrowTopReverse,
    LineType::SolidArrowBottomReverse,
    LineType::StickArrowTopReverse,
    LineType::StickArrowBottomReverse,
    LineType::SolidArrowTopReverseDotted,
    LineType::SolidArrowBottomReverseDotted,
    LineType::StickArrowTopReverseDotted,
    LineType::StickArrowBottomReverseDotted,
    LineType::Dotted,
    LineType::SolidCross,
    LineType::DottedCross,
    LineType::SolidPoint,
    LineType::DottedPoint,
    LineType::BidirectionalSolid,
    LineType::BidirectionalDotted,
];

fn map_get(list: &[(String, usize)], key: &str) -> Option<usize> {
    list.iter().find(|(k, _)| k == key).map(|(_, v)| *v)
}

impl<'a> R<'a> {
    fn actor(&self, id: &str) -> Option<&Actor> {
        self.index.get(id).map(|&i| &self.actors[i])
    }

    fn ai(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    fn insert(&mut self, a: f64, b: f64, c: f64, d: f64) {
        let bm = self.c.box_margin;
        self.b.insert(a, b, c, d, bm);
    }

    fn palette(&self, name: &str) -> (String, String) {
        let i = self.palette_index.get(name).copied().unwrap_or(0);
        (
            BORDER_COLORS[i % BORDER_COLORS.len()].to_string(),
            BKG_COLORS[i % BKG_COLORS.len()].to_string(),
        )
    }

    fn drop_shadow(&self) -> String {
        format!("url(#{}-drop-shadow)", self.id)
    }

    // -- svgDraw primitives -------------------------------------------------

    fn draw_rect(&mut self, parent: NodeId, r: &RectData) -> NodeId {
        let e = self.dom.append(parent, "rect");
        self.dom.num(e, "x", r.x).num(e, "y", r.y);
        if let Some(f) = &r.fill {
            self.dom.attr(e, "fill", f.clone());
        }
        if let Some(s) = &r.stroke {
            self.dom.attr(e, "stroke", s.clone());
        }
        self.dom.num(e, "width", r.width).num(e, "height", r.height);
        if let Some(name) = r.name.as_ref().filter(|s| !s.is_empty()) {
            self.dom.attr(e, "name", name.clone());
        }
        if r.rx != 0.0 && !r.rx.is_nan() {
            self.dom.num(e, "rx", r.rx);
        }
        if r.ry != 0.0 && !r.ry.is_nan() {
            self.dom.num(e, "ry", r.ry);
        }
        if let Some(c) = r.class.as_ref().filter(|s| !s.is_empty()) {
            self.dom.attr(e, "class", c.clone());
        }
        if self.c.neo {
            self.dom.attr(e, "data-look", "neo");
        }
        e
    }

    fn draw_background_rect(&mut self, parent: NodeId, it: &Item) {
        let (sx, sy) = (it.startx.unwrap_or(f64::NAN), it.starty.unwrap_or(f64::NAN));
        let (ex, ey) = (it.stopx.unwrap_or(f64::NAN), it.stopy.unwrap_or(f64::NAN));
        let e = self.dom.append(parent, "rect");
        self.dom.num(e, "x", sx).num(e, "y", sy);
        if let Some(f) = &it.fill {
            self.dom.attr(e, "fill", f.clone());
        }
        self.dom
            .num(e, "width", ex - sx)
            .num(e, "height", ey - sy)
            .attr(e, "class", "rect");
        self.dom.lower(e);
    }

    /// `drawText` of `svgDraw.js`. Returns the created text elements' bbox
    /// heights.
    fn draw_text(&mut self, parent: NodeId, t: &mut TextObj) -> Vec<f64> {
        let lines: Vec<String> = split_breaks(&t.text)
            .into_iter()
            .map(str::to_string)
            .collect();
        let measured = t.valign.is_some() && t.text_margin > 0.0;
        if let (Some(anchor), Some(width)) = (t.anchor.clone(), t.width) {
            match anchor.as_str() {
                "left" | "start" => {
                    t.x = (t.x + t.text_margin).js_round();
                    t.anchor = Some("start".into());
                    t.baseline = true;
                }
                "middle" | "center" => {
                    t.x = (t.x + width / 2.0).js_round();
                    t.anchor = Some("middle".into());
                    t.baseline = true;
                }
                "right" | "end" => {
                    t.x = (t.x + width - t.text_margin).js_round();
                    t.anchor = Some("end".into());
                    t.baseline = true;
                }
                _ => {}
            }
        }
        let mut heights = Vec::new();
        let mut text_height = 0.0;
        let mut prev = 0.0;
        for (i, line) in lines.iter().enumerate() {
            let mut dy = 0.0;
            if t.text_margin == 0.0 {
                dy = i as f64 * t.font.size;
            }
            let e = self.dom.append(parent, "text");
            let y = if measured {
                match t.valign.as_deref() {
                    Some("top" | "start") => (t.y + t.text_margin).js_round(),
                    Some("middle" | "center") => {
                        (t.y + (prev + text_height + t.text_margin) / 2.0).js_round()
                    }
                    Some("bottom" | "end") => (t.y + (prev + text_height + 2.0 * t.text_margin)
                        - t.text_margin)
                        .js_round(),
                    _ => t.y,
                }
            } else {
                t.y
            };
            self.dom.num(e, "x", t.x).num(e, "y", y);
            if let Some(a) = &t.anchor {
                self.dom.attr(e, "text-anchor", a.clone());
                if t.baseline {
                    self.dom.attr(e, "dominant-baseline", "middle").attr(
                        e,
                        "alignment-baseline",
                        "middle",
                    );
                }
            }
            if let Some(f) = &t.font.family {
                self.dom.style(e, "font-family", f);
            }
            self.dom.style(e, "font-size", &t.font.size_px.clone());
            self.dom.style(e, "font-weight", &t.font.weight.clone());
            if let Some(c) = &t.class {
                self.dom.attr(e, "class", c.clone());
            }
            if let Some(d) = &t.dy {
                self.dom.attr(e, "dy", d.clone());
            } else if dy != 0.0 {
                self.dom.num(e, "dy", dy);
            }
            let content = if line.is_empty() {
                "\u{200b}"
            } else {
                line.as_str()
            };
            if t.tspan {
                let s = self.dom.append(e, "tspan");
                self.dom.num(s, "x", t.x).text(s, content);
            } else {
                self.dom.text(e, content);
            }
            let h = drawn_height(line, &t.font);
            if measured {
                text_height += h;
                prev = text_height;
            }
            heights.push(h);
        }
        heights
    }

    /// `_drawTextCandidateFunc` with the default `tspan` placement.
    #[allow(clippy::too_many_arguments)]
    fn text_by_tspan(
        &mut self,
        content: &str,
        parent: NodeId,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        class: &str,
    ) {
        let font = self.c.actor_font.clone();
        let lines = split_breaks(content);
        let count = lines.len() as f64;
        for (i, line) in lines.iter().enumerate() {
            let dy = i as f64 * font.size - (font.size * (count - 1.0)) / 2.0;
            let e = self.dom.append(parent, "text");
            self.dom.num(e, "x", x + width / 2.0).num(e, "y", y);
            self.dom.style(e, "text-anchor", "middle");
            self.dom.style(e, "font-size", &font.size_px);
            self.dom.style(e, "font-weight", &font.weight);
            if let Some(f) = &font.family {
                self.dom.style(e, "font-family", f);
            }
            let s = self.dom.append(e, "tspan");
            self.dom.num(s, "x", x + width / 2.0).num(s, "dy", dy);
            if !line.is_empty() {
                self.dom.text(s, *line);
            }
            self.dom
                .num(e, "y", y + height / 2.0)
                .attr(e, "dominant-baseline", "central")
                .attr(e, "alignment-baseline", "central")
                .attr(e, "class", class);
        }
    }

    // -- actors ---------------------------------------------------------------

    /// The lifeline group of a header participant (`boxplusLineGroup`).
    fn lifeline(&mut self, parent: NodeId, ai: usize, center: f64, center_y: f64) -> NodeId {
        self.actor_cnt += 1;
        let cnt = self.actor_cnt;
        let name = self.actors[ai].name.clone();
        let l = self.dom.append(parent, "line");
        self.dom
            .attr(l, "id", format!("actor{cnt}"))
            .num(l, "x1", center)
            .num(l, "y1", center_y)
            .num(l, "x2", center)
            .attr(l, "y2", "2000")
            .attr(l, "class", "actor-line 200")
            .attr(l, "stroke-width", "0.5px")
            .attr(l, "stroke", "#999")
            .attr(l, "name", name.clone())
            .attr(l, "data-et", "life-line")
            .attr(l, "data-id", name);
        let a = &mut self.actors[ai];
        a.actor_cnt = Some(cnt as usize);
        a.lifeline = Some(l);
        l
    }

    fn popup_toggle(&mut self, g: NodeId, ai: usize) {
        if !self.actors[ai].links.is_empty() && !self.c.force_menus {
            let id = format!("actor{}_popup", self.actor_cnt + 1);
            self.dom
                .attr(
                    g,
                    "onclick",
                    format!(
                        "var pu = document.getElementById('{id}'); if (pu != null) {{ pu.style.display = pu.style.display == 'block' ? 'none' : 'block'; }}"
                    ),
                )
                .attr(g, "cursor", "pointer");
        }
    }

    fn bands(&self, ai: usize, footer: bool, actor_y: f64) -> Option<(f64, f64, f64)> {
        if !self.c.neo {
            return None;
        }
        let a = &self.actors[ai];
        let th = a.text_height;
        if footer {
            let glyph_top = actor_y + LABEL_LIFELINE_GAP;
            let glyph_bottom = glyph_top + GLYPH_BAND_HEIGHT;
            // (lifeline start, glyph bottom, label center)
            Some((
                f64::NAN,
                glyph_bottom,
                glyph_bottom + GLYPH_LABEL_GAP + th / 2.0,
            ))
        } else {
            let lifeline = actor_y + a.height;
            Some((
                lifeline,
                lifeline - LABEL_LIFELINE_GAP - th - GLYPH_LABEL_GAP,
                lifeline - LABEL_LIFELINE_GAP - th / 2.0,
            ))
        }
    }

    fn draw_icon(&mut self, g: NodeId, ai: usize, x: f64, y: f64) {
        if let Some(icon) = self.actors[ai].icon.clone() {
            let src = icon.trim();
            if let Some(rest) = src.strip_prefix('@') {
                let e = self.dom.append(g, "use");
                self.dom.num(e, "x", x).num(e, "y", y).attr(
                    e,
                    "xlink:href",
                    format!("#{}", sanitize_url(rest)),
                );
            } else {
                let e = self.dom.append(g, "image");
                self.dom
                    .num(e, "x", x)
                    .num(e, "y", y)
                    .attr(e, "xlink:href", sanitize_url(src));
            }
        }
    }

    fn draw_actor(&mut self, ai: usize, footer: bool) -> f64 {
        match self.actors[ai].kind.as_str() {
            "actor" => self.draw_stick(ai, footer),
            "participant" => self.draw_participant(ai, footer),
            "boundary" => self.draw_boundary(ai, footer),
            "control" => self.draw_control(ai, footer),
            "entity" => self.draw_entity(ai, footer),
            "database" => self.draw_database(ai, footer),
            "collections" => self.draw_collections(ai, footer),
            "queue" => self.draw_queue(ai, footer),
            _ => f64::NAN,
        }
    }

    fn actor_y(&self, ai: usize, footer: bool) -> f64 {
        let a = &self.actors[ai];
        if footer {
            a.stopy.unwrap_or(f64::NAN)
        } else {
            a.starty
        }
    }

    /// The `g` holding a boxed participant: the lifeline group itself in the
    /// footer, a new group after the lifeline in the header.
    fn boxed_group(&mut self, ai: usize, footer: bool, center: f64, center_y: f64) -> NodeId {
        let root = self.root;
        let outer = self.dom.append(root, "g");
        self.dom.lower(outer);
        if footer {
            return outer;
        }
        self.popup_toggle(outer, ai);
        self.lifeline(outer, ai, center, center_y);
        let g = self.dom.append(outer, "g");
        self.dom.attr(g, "id", format!("root-{}", self.actor_cnt));
        if self.c.neo {
            self.dom.attr(g, "data-look", "neo");
        }
        g
    }

    fn actor_class(&self, ai: usize, footer: bool) -> (String, bool) {
        let a = &self.actors[ai];
        let (base, custom) = match &a.class {
            Some(c) if !c.is_empty() => (c.clone(), true),
            _ => ("actor".to_string(), false),
        };
        let suffix = if footer { "actor-bottom" } else { "actor-top" };
        (format!("{base} {suffix}"), custom)
    }

    fn draw_participant(&mut self, ai: usize, footer: bool) -> f64 {
        let actor_y = self.actor_y(ai, footer);
        let (x, w, h) = {
            let a = &self.actors[ai];
            (a.x, a.width, a.height)
        };
        let center = x + w / 2.0;
        let g = self.boxed_group(ai, footer, center, actor_y + h);
        let (class, custom) = self.actor_class(ai, footer);
        let mut rect = RectData::note();
        if !custom {
            rect.fill = Some("#eaeaea".into());
        }
        rect.x = x;
        rect.y = actor_y;
        rect.width = w;
        rect.height = h;
        rect.class = Some(class);
        rect.rx = if self.c.neo { 6.0 } else { 3.0 };
        rect.ry = rect.rx;
        rect.name = Some(self.actors[ai].name.clone());
        let e = self.draw_rect(g, &rect);
        let name = self.actors[ai].name.clone();
        if self.c.color_theme {
            let (stroke, fill) = self.palette(&name);
            self.dom.style(e, "stroke", &stroke).style(e, "fill", &fill);
        }
        if self.c.neo {
            let f = self.drop_shadow();
            self.dom.attr(e, "filter", f);
        }
        self.actors[ai].rect = Some(rect.clone());
        self.draw_icon(g, ai, rect.x + rect.width - 20.0, rect.y + 10.0);
        if !footer {
            self.dom
                .attr(g, "data-et", "participant")
                .attr(g, "data-type", "participant")
                .attr(g, "data-id", name);
        }
        let desc = self.actors[ai].description.clone();
        self.text_by_tspan(
            &desc,
            g,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            "actor actor-box",
        );
        self.actors[ai].height
    }

    fn draw_collections(&mut self, ai: usize, footer: bool) -> f64 {
        let actor_y = self.actor_y(ai, footer);
        let (x, w, h) = {
            let a = &self.actors[ai];
            (a.x, a.width, a.height)
        };
        let g = self.boxed_group(ai, footer, x + w / 2.0, actor_y + h);
        let (class, custom) = self.actor_class(ai, footer);
        let mut rect = RectData::note();
        if !custom {
            rect.fill = Some("#eaeaea".into());
        }
        rect.x = x;
        rect.y = actor_y;
        rect.width = w;
        rect.height = h;
        rect.class = Some(class);
        rect.name = Some(self.actors[ai].name.clone());
        let offset = 6.0;
        if self.c.neo {
            rect.height = h - offset;
        }
        let mut shadow = rect.clone();
        shadow.x = rect.x - offset;
        shadow.y = rect.y + offset;
        shadow.class = Some("actor".into());
        let e = self.draw_rect(g, &rect);
        let s = self.draw_rect(g, &shadow);
        self.actors[ai].rect = Some(rect.clone());
        if self.c.neo {
            let f = self.drop_shadow();
            self.dom.attr(g, "filter", f);
        }
        let name = self.actors[ai].name.clone();
        if self.c.color_theme {
            let (stroke, fill) = self.palette(&name);
            self.dom.style(e, "stroke", &stroke).style(e, "fill", &fill);
            self.dom.style(s, "stroke", &stroke).style(s, "fill", &fill);
        }
        self.draw_icon(g, ai, rect.x + rect.width - 20.0, rect.y + 10.0);
        let desc = self.actors[ai].description.clone();
        self.text_by_tspan(
            &desc,
            g,
            rect.x - offset,
            rect.y + offset,
            rect.width,
            rect.height,
            "actor actor-box",
        );
        if !footer {
            self.dom
                .attr(g, "data-et", "participant")
                .attr(g, "data-type", "collections")
                .attr(g, "data-id", name);
        }
        self.actors[ai].height
    }

    fn draw_queue(&mut self, ai: usize, footer: bool) -> f64 {
        let actor_y = self.actor_y(ai, footer);
        let (x, w, h) = {
            let a = &self.actors[ai];
            (a.x, a.width, a.height)
        };
        let g = self.boxed_group(ai, footer, x + w / 2.0, actor_y + h);
        let (class, _) = self.actor_class(ai, footer);
        self.dom.attr(g, "class", class);
        let ry = h / 2.0;
        let rx = ry / (2.5 + h / 50.0);
        let cyl = self.dom.append(g, "g");
        let arc = self.dom.append(g, "g");
        let p = self.dom.append(cyl, "path");
        self.dom.attr(
            p,
            "d",
            format!(
                "M {},{}\n    a {},{} 0 0 0 0,{}\n    h {}\n    a {},{} 0 0 0 0,-{}\n    Z",
                n(x),
                n(actor_y + ry),
                n(rx),
                n(ry),
                n(h),
                n(w - 2.0 * rx),
                n(rx),
                n(ry),
                n(h)
            ),
        );
        let p2 = self.dom.append(arc, "path");
        self.dom.attr(
            p2,
            "d",
            format!(
                "M {},{}\n      a {},{} 0 0 0 0,{}",
                n(x),
                n(actor_y + ry),
                n(rx),
                n(ry),
                n(h)
            ),
        );
        self.dom.attr(
            cyl,
            "transform",
            format!("translate({}, {})", n(rx), n(-(h / 2.0))),
        );
        self.dom.attr(
            arc,
            "transform",
            format!("translate({}, {})", n(w - rx), n(-h / 2.0)),
        );
        self.actors[ai].rect = Some(RectData {
            x,
            y: actor_y,
            width: w,
            height: h,
            ..RectData::note()
        });
        if self.c.neo {
            let f = self.drop_shadow();
            self.dom.attr(cyl, "filter", f);
        }
        let name = self.actors[ai].name.clone();
        if self.c.color_theme {
            let (stroke, fill) = self.palette(&name);
            self.dom
                .style(cyl, "stroke", &stroke)
                .style(cyl, "fill", &fill);
            self.dom
                .style(arc, "stroke", &stroke)
                .style(arc, "fill", &fill);
        }
        self.draw_icon(g, ai, x + w - 20.0, actor_y + 10.0);
        let desc = self.actors[ai].description.clone();
        self.text_by_tspan(&desc, g, x, actor_y, w, h, "actor actor-box");
        if !footer {
            self.dom
                .attr(g, "data-et", "participant")
                .attr(g, "data-type", "queue")
                .attr(g, "data-id", name);
        }
        self.actors[ai].height
    }

    /// The empty lowered group of the glyph-shaped participants, holding the
    /// lifeline in the header.
    fn line_group(&mut self) -> NodeId {
        let root = self.root;
        let g = self.dom.append(root, "g");
        self.dom.lower(g);
        g
    }

    fn man_group(&mut self, ai: usize, footer: bool, base: &str) -> NodeId {
        let root = self.root;
        let e = self.dom.append(root, "g");
        let suffix = if footer { "actor-bottom" } else { "actor-top" };
        let name = self.actors[ai].name.clone();
        self.dom
            .attr(e, "class", format!("{base} {suffix}"))
            .attr(e, "name", name);
        e
    }

    fn glyph_colors(&mut self, e: NodeId, ai: usize, border_fallback: bool, bkg_fallback: bool) {
        let name = self.actors[ai].name.clone();
        if self.c.color_theme {
            let (stroke, fill) = self.palette(&name);
            self.dom.style(e, "stroke", &stroke).style(e, "fill", &fill);
        } else {
            if border_fallback {
                self.dom.style(e, "stroke", ACTOR_BORDER);
            }
            if bkg_fallback {
                self.dom.style(e, "fill", ACTOR_BKG);
            }
        }
    }

    fn label_y(&self, bands: Option<(f64, f64, f64)>, ai: usize, classic: f64) -> f64 {
        match bands {
            Some((_, _, label)) => label - self.actors[ai].height / 2.0,
            None => classic,
        }
    }

    fn draw_control(&mut self, ai: usize, footer: bool) -> f64 {
        let actor_y = self.actor_y(ai, footer);
        let (x, w) = (self.actors[ai].x, self.actors[ai].width);
        let center = x + w / 2.0;
        let bands = self.bands(ai, footer, actor_y);
        let center_y = bands.map_or(actor_y + 75.0, |b| b.0);
        let line = self.line_group();
        if !footer {
            self.lifeline(line, ai, center, center_y);
        }
        let e = self.man_group(ai, footer, "actor-man");
        let cy = bands.map_or(actor_y + 32.0, |b| b.1 - 22.0);
        let r = 22.0;
        let defs = self.dom.append(e, "defs");
        let m = self.dom.append(defs, "marker");
        self.dom
            .attr(m, "id", format!("{}-filled-head-control", self.id))
            .attr(m, "refX", "11")
            .attr(m, "refY", "5.8")
            .attr(m, "markerWidth", "20")
            .attr(m, "markerHeight", "28")
            .attr(m, "orient", "172.5")
            .attr(m, "stroke-width", "1.2");
        let p = self.dom.append(m, "path");
        self.dom
            .attr(p, "d", "M 14.4 5.6 L 7.2 10.4 L 8.8 5.6 L 7.2 0.8 Z");
        let c = self.dom.append(e, "circle");
        let filter = if self.c.neo {
            self.drop_shadow()
        } else {
            String::new()
        };
        self.dom
            .num(c, "cx", center)
            .num(c, "cy", cy)
            .num(c, "r", r)
            .attr(c, "filter", filter);
        let l = self.dom.append(e, "line");
        self.dom
            .attr(
                l,
                "marker-end",
                format!("url(#{}-filled-head-control)", self.id),
            )
            .attr(
                l,
                "transform",
                format!("translate({}, {})", n(center), n(cy - r)),
            );
        self.glyph_colors(e, ai, true, true);
        // The label box keeps the height the actor had before measuring.
        let h = self.actors[ai].height;
        if bands.is_none() {
            self.actors[ai].height = 2.0 * r + 2.0 * self.c.label_box_height;
        }
        let ty = self.label_y(bands, ai, actor_y + r + if footer { 5.0 } else { 12.0 });
        let desc = self.actors[ai].description.clone();
        self.text_by_tspan(&desc, e, x, ty, w, h, "actor actor-man");
        if !footer {
            let name = self.actors[ai].name.clone();
            self.dom
                .attr(e, "data-et", "participant")
                .attr(e, "data-type", "control")
                .attr(e, "data-id", name);
        }
        self.actors[ai].height
    }

    fn draw_entity(&mut self, ai: usize, footer: bool) -> f64 {
        let actor_y = self.actor_y(ai, footer);
        let (x, w) = (self.actors[ai].x, self.actors[ai].width);
        let center = x + w / 2.0;
        let bands = self.bands(ai, footer, actor_y);
        let center_y = bands.map_or(actor_y + 75.0, |b| b.0);
        let line = self.line_group();
        let e = self.man_group(ai, footer, "actor");
        let cy = bands.map_or(actor_y + if footer { 10.0 } else { 25.0 }, |b| b.1 - 22.0);
        let r = 22.0;
        let h = self.actors[ai].height;
        let c = self.dom.append(e, "circle");
        self.dom
            .num(c, "cx", center)
            .num(c, "cy", cy)
            .num(c, "r", r)
            .num(c, "width", w)
            .num(c, "height", h);
        let l = self.dom.append(e, "line");
        self.dom
            .num(l, "x1", center - r)
            .num(l, "x2", center + r)
            .num(l, "y1", cy + r)
            .num(l, "y2", cy + r)
            .attr(l, "stroke-width", "2");
        if self.c.neo {
            let f = self.drop_shadow();
            self.dom.attr(e, "filter", f);
        }
        self.glyph_colors(e, ai, false, false);
        let h = self.actors[ai].height;
        if bands.is_none() {
            self.actors[ai].height = 2.0 * r + self.c.label_box_height;
        }
        if !footer {
            self.lifeline(line, ai, center, center_y);
        }
        let ty = self.label_y(bands, ai, actor_y + if footer { 15.0 } else { 30.0 });
        let desc = self.actors[ai].description.clone();
        self.text_by_tspan(&desc, e, x, ty, w, h, "actor actor-man");
        if bands.is_none() {
            let t = if footer { r } else { r / 2.0 - 5.0 };
            self.dom
                .attr(e, "transform", format!("translate(0, {})", n(t)));
        }
        if !footer {
            let name = self.actors[ai].name.clone();
            self.dom
                .attr(e, "data-et", "participant")
                .attr(e, "data-type", "entity")
                .attr(e, "data-id", name);
        }
        self.actors[ai].height
    }

    fn draw_database(&mut self, ai: usize, footer: bool) -> f64 {
        let actor_y = self.actor_y(ai, footer);
        let (x, w, h) = {
            let a = &self.actors[ai];
            (a.x, a.width, a.height)
        };
        let bands = self.bands(ai, footer, actor_y);
        let center_y = bands.map_or(actor_y + h + 2.0 * self.c.box_text_margin, |b| b.0);
        let g = self.boxed_group(ai, footer, x + w / 2.0, center_y);
        let (class, _) = self.actor_class(ai, footer);
        let cw = w / 3.0;
        let rx = cw / 2.0;
        let ry = rx / (2.5 + cw / 50.0);
        let ch = if bands.is_some() {
            GLYPH_BAND_HEIGHT
        } else {
            w / 3.0
        };
        let cyl_y = bands.map_or(actor_y, |b| b.1 - ch - ry);
        let cyl = self.dom.append(g, "g");
        self.dom.attr(cyl, "class", class);
        let p = self.dom.append(cyl, "path");
        self.dom.attr(
            p,
            "d",
            format!(
                "M {},{}\n  a {},{} 0 0 0 {},0\n  a {},{} 0 0 0 -{},0\n  l 0,{}\n  a {},{} 0 0 0 {},0\n  l 0,-{}",
                n(x),
                n(cyl_y + ry),
                n(rx),
                n(ry),
                n(cw),
                n(rx),
                n(ry),
                n(cw),
                n(ch - 2.0 * ry),
                n(rx),
                n(ry),
                n(cw),
                n(ch - 2.0 * ry)
            ),
        );
        if self.c.neo {
            let f = self.drop_shadow();
            self.dom.attr(cyl, "filter", f);
        }
        self.glyph_colors(cyl, ai, true, false);
        self.dom
            .attr(cyl, "transform", format!("translate({}, {})", n(cw), n(ry)));
        self.actors[ai].rect = Some(RectData {
            x,
            y: actor_y,
            width: w,
            height: h,
            ..RectData::note()
        });
        let ty = self.label_y(bands, ai, actor_y + 35.0);
        let desc = self.actors[ai].description.clone();
        self.text_by_tspan(&desc, g, x, ty, w, h, "actor actor-box");
        if bands.is_none() {
            // The path's bounding box: the cylinder from the top of its upper
            // ellipse to the bottom of the lower one.
            self.actors[ai].height = ch + self.c.label_box_height;
        }
        if !footer {
            let name = self.actors[ai].name.clone();
            self.dom
                .attr(g, "data-et", "participant")
                .attr(g, "data-type", "database")
                .attr(g, "data-id", name);
        }
        self.actors[ai].height
    }

    fn draw_boundary(&mut self, ai: usize, footer: bool) -> f64 {
        let actor_y = self.actor_y(ai, footer);
        let (x, w) = (self.actors[ai].x, self.actors[ai].width);
        let center = x + w / 2.0;
        let bands = self.bands(ai, footer, actor_y);
        let center_y = bands.map_or(actor_y + 80.0, |b| b.0);
        let radius = 22.0;
        let icy = bands.map_or(actor_y + 12.0, |b| b.1 - radius);
        let line = self.line_group();
        if !footer {
            self.lifeline(line, ai, center, center_y);
        }
        let e = self.man_group(ai, footer, "actor-man");
        let cnt = self.actor_cnt;
        let l1 = self.dom.append(e, "line");
        self.dom
            .attr(l1, "id", format!("actor-man-torso{cnt}"))
            .num(l1, "x1", center - radius * 2.5)
            .num(l1, "y1", icy)
            .num(l1, "x2", center - 15.0)
            .num(l1, "y2", icy);
        let l2 = self.dom.append(e, "line");
        self.dom
            .attr(l2, "id", format!("actor-man-arms{cnt}"))
            .num(l2, "x1", center - radius * 2.5)
            .num(l2, "y1", icy - 10.0)
            .num(l2, "x2", center - radius * 2.5)
            .num(l2, "y2", icy + 10.0);
        let c = self.dom.append(e, "circle");
        self.dom
            .num(c, "cx", center)
            .num(c, "cy", icy)
            .num(c, "r", radius);
        if self.c.neo {
            let f = self.drop_shadow();
            self.dom.attr(e, "filter", f);
        }
        self.glyph_colors(e, ai, true, false);
        let h = self.actors[ai].height;
        if bands.is_none() {
            self.actors[ai].height = 2.0 * radius + self.c.label_box_height;
        }
        let ty = self.label_y(bands, ai, actor_y + 15.0);
        let desc = self.actors[ai].description.clone();
        self.text_by_tspan(&desc, e, x, ty, w, h, "actor actor-man");
        if bands.is_none() {
            self.dom.attr(
                e,
                "transform",
                format!("translate(0,{})", n(radius / 2.0 + 10.0)),
            );
        }
        if !footer {
            let name = self.actors[ai].name.clone();
            self.dom
                .attr(e, "data-et", "participant")
                .attr(e, "data-type", "boundary")
                .attr(e, "data-id", name);
        }
        self.actors[ai].height
    }

    fn draw_stick(&mut self, ai: usize, footer: bool) -> f64 {
        let actor_y = self.actor_y(ai, footer);
        let (x, w) = (self.actors[ai].x, self.actors[ai].width);
        let center = x + w / 2.0;
        let bands = self.bands(ai, footer, actor_y);
        let center_y = bands.map_or(actor_y + 80.0, |b| b.0);
        let line = self.line_group();
        if !footer {
            self.lifeline(line, ai, center, center_y);
        }
        let e = self.man_group(ai, footer, "actor-man");
        let name = self.actors[ai].name.clone();
        if !footer {
            self.dom
                .attr(e, "data-et", "participant")
                .attr(e, "data-type", "actor")
                .attr(e, "data-id", name.clone());
        }
        let scale = if bands.is_some() {
            GLYPH_BAND_HEIGHT / ACTOR_GLYPH_HEIGHT
        } else {
            1.0
        };
        let gy = |o: f64| match bands {
            Some(b) => b.1 - (ACTOR_GLYPH_BOTTOM - o) * scale,
            None => actor_y + o,
        };
        let gx = |o: f64| center + o * scale;
        let cnt = self.actor_cnt;
        let torso = self.dom.append(e, "line");
        self.dom
            .attr(torso, "id", format!("actor-man-torso{cnt}"))
            .num(torso, "x1", center)
            .num(torso, "y1", gy(25.0))
            .num(torso, "x2", center)
            .num(torso, "y2", gy(45.0));
        let arms = self.dom.append(e, "line");
        self.dom
            .attr(arms, "id", format!("actor-man-arms{cnt}"))
            .num(arms, "x1", gx(-ACTOR_TYPE_WIDTH / 2.0))
            .num(arms, "y1", gy(33.0))
            .num(arms, "x2", gx(ACTOR_TYPE_WIDTH / 2.0))
            .num(arms, "y2", gy(33.0));
        let l3 = self.dom.append(e, "line");
        self.dom
            .num(l3, "x1", gx(-ACTOR_TYPE_WIDTH / 2.0))
            .num(l3, "y1", gy(60.0))
            .num(l3, "x2", center)
            .num(l3, "y2", gy(45.0));
        let l4 = self.dom.append(e, "line");
        self.dom
            .num(l4, "x1", center)
            .num(l4, "y1", gy(45.0))
            .num(l4, "x2", gx(ACTOR_TYPE_WIDTH / 2.0 - 2.0))
            .num(l4, "y2", gy(60.0));
        let h = self.actors[ai].height;
        let c = self.dom.append(e, "circle");
        self.dom
            .num(c, "cx", center)
            .num(c, "cy", gy(10.0))
            .num(c, "r", 15.0 * scale)
            .num(c, "width", w)
            .num(c, "height", h);
        if bands.is_none() {
            self.actors[ai].height = ACTOR_GLYPH_HEIGHT;
        }
        self.glyph_colors(e, ai, true, false);
        let h = self.actors[ai].height;
        let ty = self.label_y(bands, ai, actor_y + 35.0);
        let desc = self.actors[ai].description.clone();
        self.text_by_tspan(&desc, e, x, ty, w, h, "actor actor-man");
        self.actors[ai].height
    }

    // -- layout passes ----------------------------------------------------------

    fn activation_bounds(&self, actor: &str) -> (f64, f64) {
        let a = match self.actor(actor) {
            Some(a) => a,
            None => return (f64::NAN, f64::NAN),
        };
        let mut left = a.x + a.width / 2.0 - 1.0;
        let mut right = a.x + a.width / 2.0 + 1.0;
        for act in self
            .b
            .activations
            .iter()
            .filter(|x| x.actor.as_deref() == Some(actor))
        {
            left = js_min(left, act.startx);
            right = js_max(right, act.stopx);
        }
        (left, right)
    }

    fn max_message_width_per_actor(&self) -> HashMap<String, f64> {
        let mut m: HashMap<String, f64> = HashMap::new();
        let put = |m: &mut HashMap<String, f64>, k: &str, v: f64| {
            let e = m.entry(k.to_string()).or_insert(0.0);
            *e = js_max(*e, v);
        };
        for msg in &self.msgs {
            let (Some(from), Some(to)) = (&msg.from, &msg.to) else {
                continue;
            };
            let (Some(_), Some(actor)) = (self.actor(from), self.actor(to)) else {
                continue;
            };
            if msg.placement == Some(Placement::LeftOf) && actor.prev.is_none() {
                continue;
            }
            if msg.placement == Some(Placement::RightOf) && actor.next.is_none() {
                continue;
            }
            let is_note = msg.placement.is_some();
            let font = if is_note {
                &self.c.note_font
            } else {
                &self.c.message_font
            };
            let wrapped = if msg.wrap {
                wrap_label(&msg.text, self.c.width - 2.0 * self.c.wrap_padding, font)
            } else {
                msg.text.clone()
            };
            let w = font.dims(&wrapped).width + 2.0 * self.c.wrap_padding;
            let prev = actor.prev.as_deref();
            let next = actor.next.as_deref();
            if !is_note && Some(from.as_str()) == next {
                put(&mut m, to, w);
            } else if !is_note && Some(from.as_str()) == prev {
                put(&mut m, from, w);
            } else if !is_note && from == to {
                put(&mut m, from, w / 2.0);
                put(&mut m, to, w / 2.0);
            } else if msg.placement == Some(Placement::RightOf) {
                put(&mut m, from, w);
            } else if msg.placement == Some(Placement::LeftOf) {
                put(&mut m, prev.unwrap_or("undefined"), w);
            } else if msg.placement == Some(Placement::Over) {
                if let Some(p) = prev {
                    put(&mut m, p, w / 2.0);
                }
                if next.is_some() {
                    put(&mut m, from, w / 2.0);
                }
            }
        }
        m
    }

    fn calculate_actor_margins(&mut self, widths: &HashMap<String, f64>) -> f64 {
        let mut max_height: f64 = 0.0;
        let c = &self.c;
        for a in &mut self.actors {
            if a.wrap {
                a.description = wrap_label(
                    &a.description,
                    c.width - 2.0 * c.wrap_padding,
                    &c.actor_font,
                );
            }
            let dims = c.actor_font.dims(&a.description);
            a.width = if a.wrap {
                c.width
            } else {
                js_max(c.width, dims.width + 2.0 * c.wrap_padding)
            };
            if c.neo {
                a.text_height = dims.height;
                a.height = js_max(
                    GLYPH_BAND_HEIGHT + GLYPH_LABEL_GAP + dims.height + LABEL_LIFELINE_GAP,
                    c.height,
                );
            } else {
                a.height = if a.wrap {
                    js_max(dims.height, c.height)
                } else {
                    c.height
                };
                a.text_height = dims.height;
            }
            max_height = js_max(max_height, a.height);
        }
        for (key, &mw) in widths {
            let Some(ai) = self.ai(key) else { continue };
            let next_w = self.actors[ai]
                .next
                .as_deref()
                .and_then(|nx| self.actor(nx))
                .map(|n| n.width);
            let a = &self.actors[ai];
            let w = match next_w {
                None => mw + self.c.actor_margin - a.width / 2.0,
                Some(nw) => mw + self.c.actor_margin - a.width / 2.0 - nw / 2.0,
            };
            self.actors[ai].margin = Some(js_max(w, self.c.actor_margin));
        }
        let mut max_box: f64 = 0.0;
        for bi in 0..self.boxes.len() {
            let mut total = 0.0;
            for k in &self.boxes[bi].actors {
                if let Some(a) = self.actor(k) {
                    total += a.width + a.margin.unwrap_or(0.0);
                }
            }
            total += self.c.box_margin * 8.0;
            total -= 2.0 * self.c.box_text_margin;
            let font = self.c.message_font.clone();
            let bx = &mut self.boxes[bi];
            if bx.wrap {
                if let Some(name) = &bx.name {
                    bx.name = Some(wrap_label(name, total - 2.0 * self.c.wrap_padding, &font));
                }
            }
            let dims = bx
                .name
                .as_deref()
                .map_or(TextDims::default(), |nm| font.dims(nm));
            max_box = js_max(dims.height, max_box);
            let min_width = js_max(total, dims.width + 2.0 * self.c.wrap_padding);
            bx.margin = self.c.box_text_margin;
            if total < min_width {
                bx.margin += (min_width - total) / 2.0;
            }
        }
        for bx in &mut self.boxes {
            bx.text_max_height = max_box;
        }
        js_max(max_height, self.c.height)
    }

    fn add_actor_rendering_data(&mut self) {
        let mut prev_width = 0.0;
        let mut prev_margin = 0.0;
        let mut prev_box: Option<usize> = None;
        let mut max_height: f64 = 0.0;
        let keys = self.keys.clone();
        for ai in keys {
            let group = self.actors[ai].group;
            if let Some(pb) = prev_box {
                if group != Some(pb) {
                    self.box_models.push(pb);
                    prev_margin += self.c.box_margin + self.boxes[pb].margin;
                }
            }
            if let Some(g) = group {
                if group != prev_box {
                    self.boxes[g].x = prev_width + prev_margin;
                    self.boxes[g].y = 0.0;
                    prev_margin += self.boxes[g].margin;
                }
            }
            let (cw, ch, cm) = (self.c.width, self.c.height, self.c.actor_margin);
            let a = &mut self.actors[ai];
            a.width = js_max(if a.width == 0.0 { cw } else { a.width }, cw);
            a.height = js_max(if a.height == 0.0 { ch } else { a.height }, ch);
            let margin = match a.margin {
                Some(m) if m != 0.0 => m,
                _ => cm,
            };
            a.margin = Some(margin);
            max_height = js_max(max_height, a.height);
            if map_get(&self.created, &a.name).is_some_and(|v| v != 0) {
                prev_margin += a.width / 2.0;
            }
            a.x = prev_width + prev_margin;
            a.starty = self.b.vertical_pos;
            let (x, w, h) = (a.x, a.width, a.height);
            self.insert(x, 0.0, x + w, h);
            prev_width += w + prev_margin;
            if let Some(g) = group {
                self.boxes[g].width = prev_width + self.boxes[g].margin - self.boxes[g].x;
            }
            prev_margin = margin;
            prev_box = group;
        }
        if let Some(pb) = prev_box {
            self.box_models.push(pb);
        }
        self.b.bump(max_height);
    }

    fn build_note_model(&self, msg: &Msg) -> NoteModel {
        let c = &self.c;
        let from = self
            .actor(msg.from.as_deref().unwrap_or(""))
            .expect("note actor");
        let to = self
            .actor(msg.to.as_deref().unwrap_or(""))
            .expect("note actor");
        let (startx, stopx) = (from.x, to.x);
        let should_wrap = msg.wrap && !msg.text.is_empty();
        let font = &c.note_font;
        let mut dims = font.dims(&if should_wrap {
            wrap_label(&msg.text, c.width, font)
        } else {
            msg.text.clone()
        });
        let mut nm = NoteModel {
            width: if should_wrap {
                c.width
            } else {
                js_max(c.width, dims.width + 2.0 * c.note_margin)
            },
            startx: from.x,
            message: msg.text.clone(),
            ..Default::default()
        };
        match msg.placement {
            Some(Placement::RightOf) => {
                nm.width = if should_wrap {
                    js_max(c.width, dims.width)
                } else {
                    js_max(
                        from.width / 2.0 + to.width / 2.0,
                        dims.width + 2.0 * c.note_margin,
                    )
                };
                nm.startx = startx + (from.width + c.actor_margin) / 2.0;
            }
            Some(Placement::LeftOf) => {
                nm.width = if should_wrap {
                    js_max(c.width, dims.width + 2.0 * c.note_margin)
                } else {
                    js_max(
                        from.width / 2.0 + to.width / 2.0,
                        dims.width + 2.0 * c.note_margin,
                    )
                };
                nm.startx = startx - nm.width + (from.width - c.actor_margin) / 2.0;
            }
            _ if msg.to == msg.from => {
                dims = font.dims(&if should_wrap {
                    wrap_label(&msg.text, js_max(c.width, from.width), font)
                } else {
                    msg.text.clone()
                });
                nm.width = if should_wrap {
                    js_max(c.width, from.width)
                } else {
                    js_max(
                        js_max(from.width, c.width),
                        dims.width + 2.0 * c.note_margin,
                    )
                };
                nm.startx = startx + (from.width - nm.width) / 2.0;
            }
            _ => {
                nm.width =
                    (startx + from.width / 2.0 - (stopx + to.width / 2.0)).abs() + c.actor_margin;
                nm.startx = if startx < stopx {
                    startx + from.width / 2.0 - c.actor_margin / 2.0
                } else {
                    stopx + to.width / 2.0 - c.actor_margin / 2.0
                };
            }
        }
        if should_wrap {
            nm.message = wrap_label(&msg.text, nm.width - 2.0 * c.wrap_padding, font);
        }
        nm
    }

    fn build_message_model(&self, msg: &mut Msg) -> Option<MsgModel> {
        if !ARROWS.contains(&msg.kind) {
            return None;
        }
        use LineType::*;
        let from = msg.from.clone().unwrap_or_default();
        let to = msg.to.clone().unwrap_or_default();
        let (from_left, from_right) = self.activation_bounds(&from);
        let (to_left, to_right) = self.activation_bounds(&to);
        let right = from_left <= to_left;
        let mut startx = if right { from_right } else { from_left };
        let mut stopx = if right { to_left } else { to_right };
        if self.c.neo {
            let offset = 3.0;
            if msg.kind != SolidOpen {
                stopx += if right { -offset } else { offset };
            }
            if matches!(msg.kind, BidirectionalSolid | BidirectionalDotted) {
                startx += if right { offset } else { -offset };
            }
        }
        let mut offset = 0.0;
        if matches!(
            msg.central,
            Some(CentralConnectionReverse | CentralConnectionDual)
        ) {
            offset += 4.0;
            if matches!(msg.kind, BidirectionalSolid | BidirectionalDotted) {
                offset += if right { 0.0 } else { -6.0 };
            }
        }
        startx += offset;
        let to_activation = (to_left - to_right).abs() > 2.0;
        let adjust = |v: f64| if right { -v } else { v };
        if from == to {
            stopx = startx;
        } else {
            if msg.activate && !to_activation {
                stopx += adjust(self.c.activation_width / 2.0 - 1.0);
            }
            if !matches!(
                msg.kind,
                SolidOpen
                    | DottedOpen
                    | StickTop
                    | StickBottom
                    | StickTopDotted
                    | StickBottomDotted
                    | SolidArrowTopReverseDotted
                    | SolidArrowBottomReverseDotted
                    | StickArrowTopReverse
                    | StickArrowBottomReverse
                    | StickArrowTopReverseDotted
                    | StickArrowBottomReverseDotted
                    | SolidArrowTopReverse
                    | SolidArrowBottomReverse
            ) {
                stopx += adjust(3.0);
            }
            if matches!(
                msg.kind,
                BidirectionalSolid
                    | BidirectionalDotted
                    | SolidArrowTopReverseDotted
                    | SolidArrowBottomReverseDotted
                    | SolidArrowTopReverse
                    | SolidArrowBottomReverse
            ) {
                startx -= adjust(3.0);
            }
        }
        let all = [from_left, from_right, to_left, to_right];
        let bounded = (startx - stopx).abs();
        if msg.wrap && !msg.text.is_empty() {
            msg.text = wrap_label(
                &msg.text,
                js_max(bounded + 2.0 * self.c.wrap_padding, self.c.width),
                &self.c.message_font,
            );
        }
        let dims = self.c.message_font.dims(&msg.text);
        Some(MsgModel {
            width: js_max(
                js_max(
                    if msg.wrap {
                        0.0
                    } else {
                        dims.width + 2.0 * self.c.wrap_padding
                    },
                    bounded + 2.0 * self.c.wrap_padding,
                ),
                self.c.width,
            ),
            startx,
            stopx,
            message: msg.text.clone(),
            kind: Some(msg.kind),
            from_bounds: all.iter().copied().fold(f64::INFINITY, js_min),
            to_bounds: all.iter().copied().fold(f64::NEG_INFINITY, js_max),
            ..Default::default()
        })
    }

    /// `calculateLoopBounds`: also builds every note and message model.
    fn calculate_loop_bounds(&mut self) -> LoopBounds {
        use LineType::*;
        struct Frame {
            id: usize,
            from: f64,
            to: f64,
            width: f64,
        }
        let mut loops: HashMap<usize, usize> = HashMap::new();
        let mut frames: Vec<Frame> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        let mut notes = Vec::new();
        let mut models = Vec::new();
        for i in 0..self.msgs.len() {
            let kind = self.msgs[i].kind;
            match kind {
                LoopStart | AltStart | OptStart | ParStart | ParOverStart | CriticalStart
                | BreakStart => {
                    frames.push(Frame {
                        id: self.msgs[i].id,
                        from: 9007199254740991.0,
                        to: -9007199254740991.0,
                        width: 0.0,
                    });
                    stack.push(frames.len() - 1);
                }
                AltElse | ParAnd | CriticalOption => {
                    if !self.msgs[i].text.is_empty() {
                        if let Some(&cur) = stack.last() {
                            loops.insert(frames[cur].id, cur);
                            loops.insert(self.msgs[i].id, cur);
                        }
                    }
                }
                LoopEnd | AltEnd | OptEnd | ParEnd | CriticalEnd | BreakEnd => {
                    if let Some(cur) = stack.pop() {
                        loops.insert(frames[cur].id, cur);
                    }
                }
                ActiveStart => {
                    let key = self.msgs[i]
                        .from
                        .clone()
                        .or_else(|| self.msgs[i].to.clone());
                    if let Some(a) = key.as_deref().and_then(|k| self.actor(k)) {
                        let stacked = self.b.activations_of(key.as_deref().unwrap_or("")) as f64;
                        let x =
                            a.x + a.width / 2.0 + ((stacked - 1.0) * self.c.activation_width) / 2.0;
                        self.b.activations.push(Activation {
                            startx: x,
                            starty: 0.0,
                            stopx: x + self.c.activation_width,
                            stopy: None,
                            actor: self.msgs[i].from.clone(),
                            anchored: None,
                        });
                    }
                }
                ActiveEnd => {
                    let from = self.msgs[i].from.clone();
                    if let Some(p) = self.b.activations.iter().rposition(|a| a.actor == from) {
                        self.b.activations.remove(p);
                    } else if !self.b.activations.is_empty() {
                        // `splice(-1, 1)` removes the last one.
                        self.b.activations.pop();
                    }
                }
                _ => {}
            }
            let lbw = self.c.label_box_width;
            if self.msgs[i].placement.is_some() {
                let nm = self.build_note_model(&self.msgs[i]);
                for &f in &stack {
                    let fr = &mut frames[f];
                    fr.from = js_min(fr.from, nm.startx);
                    fr.to = js_max(fr.to, nm.startx + nm.width);
                    fr.width = js_max(fr.width, (fr.from - fr.to).abs()) - lbw;
                }
                notes.push(Some(nm));
                models.push(None);
            } else {
                let mut msg = self.msgs[i].clone();
                let mm = self.build_message_model(&mut msg);
                self.msgs[i] = msg;
                if let Some(mm) = &mm {
                    if mm.startx != 0.0
                        && mm.stopx != 0.0
                        && !mm.startx.is_nan()
                        && !mm.stopx.is_nan()
                    {
                        let (from, to) = (
                            self.msgs[i].from.clone().unwrap_or_default(),
                            self.msgs[i].to.clone().unwrap_or_default(),
                        );
                        for &f in &stack {
                            let fr = &mut frames[f];
                            if mm.startx == mm.stopx {
                                let fa = self.actor(&from);
                                let ta = self.actor(&to);
                                if let (Some(fa), Some(ta)) = (fa, ta) {
                                    fr.from = js_min(
                                        js_min(fa.x - mm.width / 2.0, fa.x - fa.width / 2.0),
                                        fr.from,
                                    );
                                    fr.to = js_max(
                                        js_max(ta.x + mm.width / 2.0, ta.x + fa.width / 2.0),
                                        fr.to,
                                    );
                                    fr.width = js_max(fr.width, (fr.to - fr.from).abs()) - lbw;
                                }
                            } else {
                                fr.from = js_min(mm.startx, fr.from);
                                fr.to = js_max(mm.stopx, fr.to);
                                fr.width = js_max(fr.width, mm.width) - lbw;
                            }
                        }
                    }
                }
                notes.push(None);
                models.push(mm);
            }
        }
        self.b.activations.clear();
        let widths = loops
            .into_iter()
            .map(|(k, f)| (k, frames[f].width))
            .collect();
        (widths, notes, models)
    }

    // -- drawing -----------------------------------------------------------------

    fn draw_note(&mut self, nm: &mut NoteModel, id: usize) {
        let c_bm = self.c.box_margin;
        self.b.bump(c_bm);
        nm.height = c_bm;
        nm.starty = self.b.vertical_pos;
        let mut rect = RectData::note();
        rect.x = nm.startx;
        rect.y = nm.starty;
        rect.width = if nm.width == 0.0 {
            self.c.width
        } else {
            nm.width
        };
        rect.class = Some("note".into());
        let root = self.root;
        let g = self.dom.append(root, "g");
        self.dom
            .attr(g, "data-et", "note")
            .attr(g, "data-id", format!("i{id}"));
        let r = self.draw_rect(g, &rect);
        let mut t = TextObj {
            x: nm.startx,
            y: nm.starty,
            width: Some(rect.width),
            dy: Some("1em".into()),
            text: nm.message.clone(),
            class: Some("noteText".into()),
            font: self.c.note_font.clone(),
            anchor: Some(self.c.note_align.clone()),
            text_margin: self.c.note_margin,
            valign: Some("center".into()),
            tspan: true,
            baseline: false,
        };
        let heights = self.draw_text(g, &mut t);
        let th = heights.iter().sum::<f64>().js_round();
        let nmar = self.c.note_margin;
        self.dom.num(r, "height", th + 2.0 * nmar);
        nm.height += th + 2.0 * nmar;
        self.b.bump(th + 2.0 * nmar);
        nm.stopy = nm.starty + th + 2.0 * nmar;
        nm.stopx = nm.startx + rect.width;
        self.insert(nm.startx, nm.starty, nm.stopx, nm.stopy);
    }

    fn bound_message(&mut self, mm: &mut MsgModel) -> f64 {
        self.b.bump(10.0);
        let lines = split_breaks(&mm.message).len() as f64;
        let dims = self.c.message_font.dims(&mm.message);
        let line_height = dims.height / lines;
        mm.height += line_height;
        self.b.bump(line_height);
        let mut total = dims.height - 10.0;
        let line_start_y;
        if mm.startx == mm.stopx {
            total += if self.c.right_angles {
                0.0
            } else {
                self.c.box_margin
            };
            line_start_y = self.b.vertical_pos + total;
            total += 30.0;
            let dx = js_max(dims.width / 2.0, self.c.width / 2.0);
            let vp = self.b.vertical_pos;
            self.insert(
                mm.startx - dx,
                vp - 10.0 + total,
                mm.stopx + dx,
                vp + 30.0 + total,
            );
        } else {
            total += self.c.box_margin;
            line_start_y = self.b.vertical_pos + total;
            self.insert(mm.startx, line_start_y - 10.0, mm.stopx, line_start_y);
        }
        self.b.bump(total);
        mm.height += total;
        mm.stopy = mm.starty + mm.height;
        self.insert(mm.from_bounds, mm.starty, mm.to_bounds, mm.stopy);
        line_start_y
    }

    fn adjust_created_destroyed(
        &mut self,
        msg: &Msg,
        mm: &mut MsgModel,
        line_start_y: f64,
        index: usize,
    ) {
        let glyphs = ["actor", "control", "entity", "database"];
        let from = msg.from.clone().unwrap_or_default();
        let to = msg.to.clone().unwrap_or_default();
        let nmar = self.c.note_margin;
        let receiver = |r: &mut Self, ai: usize, adj: f64, mm: &mut MsgModel| {
            let ax = r.actors[ai].x;
            let fx = r.actor(&from).map_or(f64::NAN, |a| a.x);
            let h = r.actors[ai].height;
            if ax < fx {
                r.insert(
                    mm.stopx - adj,
                    mm.starty,
                    mm.startx,
                    mm.stopy + h / 2.0 + nmar,
                );
                mm.stopx += adj;
            } else {
                r.insert(
                    mm.startx,
                    mm.starty,
                    mm.stopx + adj,
                    mm.stopy + h / 2.0 + nmar,
                );
                mm.stopx -= adj;
            }
        };
        if map_get(&self.created, &to) == Some(index) {
            let ai = self.ai(&to).expect("created actor");
            let a = &self.actors[ai];
            let adj = if glyphs.contains(&a.kind.as_str()) {
                ACTOR_TYPE_WIDTH / 2.0 + 3.0
            } else {
                a.width / 2.0 + 3.0
            };
            receiver(self, ai, adj, mm);
            let h = self.actors[ai].height;
            self.actors[ai].starty = line_start_y - h / 2.0;
            self.b.bump(h / 2.0);
        } else if map_get(&self.destroyed, &from) == Some(index) {
            let ai = self.ai(&from).expect("destroyed actor");
            if self.c.mirror_actors {
                let a = &self.actors[ai];
                let adj = if glyphs.contains(&a.kind.as_str()) {
                    ACTOR_TYPE_WIDTH / 2.0
                } else {
                    a.width / 2.0
                };
                let ax = a.x;
                let h = a.height;
                let tx = self.actor(&to).map_or(f64::NAN, |a| a.x);
                if ax < tx {
                    self.insert(
                        mm.startx - adj,
                        mm.starty,
                        mm.stopx,
                        mm.stopy + h / 2.0 + nmar,
                    );
                    mm.startx += adj;
                } else {
                    self.insert(
                        mm.stopx,
                        mm.starty,
                        mm.startx + adj,
                        mm.stopy + h / 2.0 + nmar,
                    );
                    mm.startx -= adj;
                }
            }
            let h = self.actors[ai].height;
            self.actors[ai].stopy = Some(line_start_y - h / 2.0);
            self.b.bump(h / 2.0);
        } else if map_get(&self.destroyed, &to) == Some(index) {
            let ai = self.ai(&to).expect("destroyed actor");
            if self.c.mirror_actors {
                let a = &self.actors[ai];
                let adj = if glyphs.contains(&a.kind.as_str()) {
                    ACTOR_TYPE_WIDTH / 2.0 + 3.0
                } else {
                    a.width / 2.0 + 3.0
                };
                receiver(self, ai, adj, mm);
            }
            let h = self.actors[ai].height;
            self.actors[ai].stopy = Some(line_start_y - h / 2.0);
            self.b.bump(h / 2.0);
        }
    }

    fn adjust_loop_height_for_wrap(
        &mut self,
        widths: &HashMap<usize, f64>,
        i: usize,
        pre: f64,
        post: f64,
    ) {
        self.b.bump(pre);
        let mut height_adjust = post;
        let id = self.msgs[i].id;
        if !self.msgs[i].text.is_empty() {
            if let Some(&lw) = widths.get(&id) {
                let font = self.c.message_font.clone();
                let m = &mut self.msgs[i];
                m.text = wrap_label(
                    &format!("[{}]", m.text),
                    lw - 2.0 * self.c.wrap_padding,
                    &font,
                );
                m.width = Some(lw);
                m.wrap = true;
                let dims = font.dims(&m.text);
                height_adjust = post + js_max(dims.height, self.c.label_box_height);
            }
        }
        let kind = self.msgs[i].kind;
        use LineType::*;
        match kind {
            AltElse | ParAnd | CriticalOption => {
                let vp = self.b.vertical_pos;
                let title = Some(self.msgs[i].text.clone()).filter(|t| !t.is_empty());
                if let Some(it) = self.b.items.last_mut() {
                    it.sections.push(vp);
                    it.section_titles.push(title);
                }
            }
            RectStart => {
                let fill = if self.msgs[i].text.is_empty() {
                    RECT_BKG_COLOR.to_string()
                } else {
                    self.msgs[i].text.clone()
                };
                self.b.new_loop(None, Some(fill));
            }
            _ => {
                let m = &self.msgs[i];
                let title = Some(m.text.clone());
                self.b.new_loop(title, None);
            }
        }
        self.b.bump(height_adjust);
    }

    fn draw_loop(&mut self, it: &Item, label: &str, msg_id: usize) {
        let c_bm = self.c.box_margin;
        let btm = self.c.box_text_margin;
        let lbw = self.c.label_box_width;
        let root = self.root;
        let g = self.dom.append(root, "g");
        self.dom
            .attr(g, "data-et", "control-structure")
            .attr(g, "data-id", format!("i{msg_id}"));
        let (sx, sy, ex, ey) = (
            it.startx.unwrap_or(f64::NAN),
            it.starty.unwrap_or(f64::NAN),
            it.stopx.unwrap_or(f64::NAN),
            it.stopy.unwrap_or(f64::NAN),
        );
        let line = |r: &mut Self, a: f64, b: f64, c: f64, d: f64| {
            let l = r.dom.append(g, "line");
            r.dom
                .attr(l, "x1", on(Some(a)))
                .attr(l, "y1", on(Some(b)))
                .attr(l, "x2", on(Some(c)))
                .attr(l, "y2", on(Some(d)))
                .attr(l, "class", "loopLine");
            l
        };
        line(self, sx, sy, ex, sy);
        line(self, ex, sy, ex, ey);
        line(self, sx, ey, ex, ey);
        line(self, sx, sy, sx, ey);
        for &y in &it.sections {
            let l = line(self, sx, y, ex, y);
            self.dom.style(l, "stroke-dasharray", "3, 3");
        }
        // The label box.
        let height = if self.c.label_box_height + if self.c.neo { 15.0 } else { 0.0 } != 0.0 {
            self.c.label_box_height + if self.c.neo { 15.0 } else { 0.0 }
        } else {
            20.0
        };
        let width = js_max(lbw, 50.0);
        let poly = self.dom.append(g, "polygon");
        let cut = 7.0;
        self.dom
            .attr(
                poly,
                "points",
                format!(
                    "{},{} {},{} {},{} {},{} {},{}",
                    n(sx),
                    n(sy),
                    n(sx + width),
                    n(sy),
                    n(sx + width),
                    n(sy + height - cut),
                    n(sx + width - cut * 1.2),
                    n(sy + height),
                    n(sx),
                    n(sy + height)
                ),
            )
            .attr(poly, "class", "labelBox");
        let font = self.c.message_font.clone();
        let mut t = TextObj {
            x: sx,
            y: sy + height / 2.0,
            width: Some(width),
            dy: None,
            text: label.to_string(),
            class: Some("labelText".into()),
            font: font.clone(),
            anchor: Some("middle".into()),
            text_margin: btm,
            valign: Some("middle".into()),
            tspan: false,
            baseline: false,
        };
        self.draw_text(g, &mut t);
        let mut t = TextObj {
            x: sx + lbw / 2.0 + (ex - sx) / 2.0,
            y: sy + c_bm + btm,
            width: None,
            dy: None,
            text: it.title.clone().unwrap_or_default(),
            class: Some("loopText".into()),
            font: font.clone(),
            anchor: Some("middle".into()),
            text_margin: btm,
            valign: Some("middle".into()),
            tspan: true,
            baseline: false,
        };
        if it.title.is_some() {
            self.draw_text(g, &mut t);
        } else {
            // `drawText` with an undefined text throws in Mermaid; nothing
            // more is drawn for this block.
            return;
        }
        for (idx, title) in it.section_titles.iter().enumerate() {
            if let Some(title) = title {
                t.text = title.clone();
                t.x = sx + (ex - sx) / 2.0;
                t.y = it.sections[idx] + c_bm + btm;
                t.class = Some("sectionTitle".into());
                t.anchor = Some("middle".into());
                t.tspan = false;
                self.draw_text(g, &mut t);
            }
        }
    }

    fn draw_activation(&mut self, act: &Activation, vertical_pos: f64, count: usize) {
        let g = act.anchored.expect("anchored activation");
        let mut rect = RectData::note();
        rect.x = act.startx;
        rect.y = act.starty;
        rect.class = Some(format!("activation{}", count % 3));
        rect.width = act.stopx - act.startx;
        rect.height = vertical_pos - act.starty;
        let e = self.draw_rect(g, &rect);
        if self.c.color_theme {
            let (stroke, fill) = self.palette(act.actor.as_deref().unwrap_or(""));
            self.dom.style(e, "stroke", &stroke).style(e, "fill", &fill);
        }
    }

    fn draw_central(&mut self, msg: &Msg, mm: &MsgModel, line_start_y: f64) {
        use LineType::*;
        let from = self.actor(msg.from.as_deref().unwrap_or(""));
        let to = self.actor(msg.to.as_deref().unwrap_or(""));
        let (Some(from), Some(to)) = (from, to) else {
            return;
        };
        let mut fc = from.x + from.width / 2.0;
        let mut tc = to.x + to.width / 2.0;
        let ltr = fc <= tc;
        let rev = msg.kind.is_reverse();
        let off = |ltr: bool, rev: bool| {
            let base = if ltr { 16.5 } else { -16.5 };
            if rev {
                -base
            } else {
                base
            }
        };
        if mm.sequence_visible {
            match msg.central {
                Some(CentralConnection) => {
                    if rev {
                        tc += off(ltr, true);
                    }
                }
                Some(CentralConnectionReverse) => {
                    if !rev {
                        fc += off(ltr, false);
                    }
                }
                Some(CentralConnectionDual) => {
                    if rev {
                        tc += off(ltr, true);
                    } else {
                        fc += off(ltr, false);
                    }
                }
                _ => {}
            }
        }
        let root = self.root;
        let g = self.dom.append(root, "g");
        let circle = |r: &mut Self, cx: f64| {
            let c = r.dom.append(g, "circle");
            r.dom
                .num(c, "cx", cx)
                .num(c, "cy", line_start_y)
                .attr(c, "r", "5")
                .attr(c, "width", "10")
                .attr(c, "height", "10");
        };
        match msg.central {
            Some(CentralConnection) => circle(self, tc),
            Some(CentralConnectionReverse) => circle(self, fc),
            Some(CentralConnectionDual) => {
                circle(self, fc);
                circle(self, tc);
            }
            _ => {}
        }
    }

    fn draw_message(&mut self, mm: &MsgModel, line_start_y: f64, msg: &Msg) {
        use LineType::*;
        let (startx, stopx) = (mm.startx, mm.stopx);
        let kind = mm.kind.expect("arrow");
        let dims = self.c.message_font.dims(&mm.message);
        let root = self.root;
        let mut t = TextObj {
            x: js_min(startx, stopx),
            y: mm.starty + 10.0,
            width: Some((stopx - startx).abs()),
            dy: Some("1em".into()),
            text: mm.message.clone(),
            class: Some("messageText".into()),
            font: self.c.message_font.clone(),
            anchor: Some(self.c.message_align.clone()),
            text_margin: self.c.wrap_padding,
            valign: Some("center".into()),
            tspan: false,
            baseline: false,
        };
        self.draw_text(root, &mut t);
        let text_width = dims.width;
        let central = matches!(
            msg.central,
            Some(CentralConnection | CentralConnectionReverse | CentralConnectionDual)
        );
        let auto_on = mm.sequence_visible || self.c.show_sequence_numbers;
        let line;
        if startx == stopx {
            let lsx = startx
                + if auto_on && (kind.is_reverse() || kind.is_bidirectional()) {
                    10.0
                } else {
                    0.0
                };
            line = self.dom.append(root, "path");
            let d = if self.c.right_angles {
                format!(
                    "M  {},{} H {} V {} H {}",
                    n(lsx),
                    n(line_start_y),
                    n(startx + js_max(self.c.width / 2.0, text_width / 2.0)),
                    n(line_start_y + 25.0),
                    n(startx)
                )
            } else {
                format!(
                    "M {},{} C {},{} {},{} {},{}",
                    n(lsx),
                    n(line_start_y),
                    n(lsx + 60.0),
                    n(line_start_y - 10.0),
                    n(startx + 60.0),
                    n(line_start_y + 30.0),
                    n(startx),
                    n(line_start_y + 20.0)
                )
            };
            self.dom.attr(line, "d", d);
            if central {
                self.draw_central(msg, mm, line_start_y);
            }
        } else {
            line = self.dom.append(root, "line");
            self.dom
                .num(line, "x1", startx)
                .num(line, "y1", line_start_y)
                .num(line, "x2", stopx)
                .num(line, "y2", line_start_y);
            if central {
                self.draw_central(msg, mm, line_start_y);
            }
        }
        if kind.is_dotted() {
            self.dom.style(line, "stroke-dasharray", "3, 3");
            self.dom.attr(line, "class", "messageLine1");
        } else {
            self.dom.attr(line, "class", "messageLine0");
        }
        self.dom
            .attr(line, "data-et", "message")
            .attr(line, "data-id", format!("i{}", mm.id))
            .attr(line, "data-from", mm.from.clone())
            .attr(line, "data-to", mm.to.clone())
            .attr(line, "stroke-width", "2")
            .attr(line, "stroke", "none");
        self.dom.style(line, "fill", "none");
        let id = self.id.clone();
        let marker = |name: &str| format!("url(#{id}-{name})");
        let end = match kind {
            SolidTop | SolidTopDotted => Some("solidTopArrowHead"),
            SolidBottom | SolidBottomDotted => Some("solidBottomArrowHead"),
            StickTop | StickTopDotted => Some("stickTopArrowHead"),
            StickBottom | StickBottomDotted => Some("stickBottomArrowHead"),
            _ => None,
        };
        if let Some(m) = end {
            self.dom.attr(line, "marker-end", marker(m));
        }
        let start = match kind {
            SolidArrowTopReverse | SolidArrowTopReverseDotted => Some("solidBottomArrowHead"),
            SolidArrowBottomReverse | SolidArrowBottomReverseDotted => Some("solidTopArrowHead"),
            StickArrowTopReverse | StickArrowTopReverseDotted => Some("stickBottomArrowHead"),
            StickArrowBottomReverse | StickArrowBottomReverseDotted => Some("stickTopArrowHead"),
            _ => None,
        };
        if let Some(m) = start {
            self.dom.attr(line, "marker-start", marker(m));
        }
        if matches!(kind, Solid | Dotted) {
            self.dom.attr(line, "marker-end", marker("arrowhead"));
        }
        if kind.is_bidirectional() {
            self.dom
                .attr(line, "marker-start", marker("arrowhead"))
                .attr(line, "marker-end", marker("arrowhead"));
        }
        if matches!(kind, SolidPoint | DottedPoint) {
            self.dom.attr(line, "marker-end", marker("filled-head"));
        }
        if matches!(kind, SolidCross | DottedCross) {
            self.dom.attr(line, "marker-end", marker("crosshead"));
        }
        if auto_on {
            let r = 6.0;
            let mut lsx = startx;
            if kind.is_bidirectional() {
                if startx < stopx {
                    lsx = startx + r * 2.0;
                } else {
                    lsx = startx - r + if central { -5.0 } else { 0.0 };
                    if matches!(
                        msg.central,
                        Some(CentralConnectionDual | CentralConnectionReverse)
                    ) {
                        lsx += -7.5;
                    }
                }
                self.dom.num(line, "x1", lsx);
            } else if kind.is_reverse() {
                let mut lstop;
                if stopx > startx {
                    lstop = stopx - 2.0 * r;
                } else {
                    lstop = stopx - r;
                    if matches!(
                        msg.central,
                        Some(CentralConnectionDual | CentralConnectionReverse)
                    ) {
                        lsx += -7.5;
                    }
                }
                lstop += if central { 15.0 } else { 0.0 };
                self.dom.num(line, "x2", lstop).num(line, "x1", lsx);
            } else {
                self.dom.num(line, "x1", startx + r);
            }
            let ax = if startx == stopx {
                mm.from_bounds + 1.0
            } else if kind.is_reverse() {
                if startx <= stopx {
                    mm.to_bounds - 1.0
                } else {
                    mm.from_bounds + 1.0
                }
            } else if startx <= stopx {
                mm.from_bounds + 1.0
            } else {
                mm.to_bounds - 1.0
            };
            let idx = n(mm.sequence_index);
            let size = if idx.len() > 5 {
                "7px"
            } else if idx.len() > 3 {
                "9px"
            } else {
                "12px"
            };
            let l = self.dom.append(root, "line");
            self.dom
                .num(l, "x1", ax)
                .num(l, "y1", line_start_y)
                .num(l, "x2", ax)
                .num(l, "y2", line_start_y)
                .attr(l, "stroke-width", "0")
                .attr(l, "marker-start", marker("sequencenumber"));
            let tx = self.dom.append(root, "text");
            self.dom
                .num(tx, "x", ax)
                .num(tx, "y", line_start_y + 4.0)
                .attr(tx, "font-family", "sans-serif")
                .attr(tx, "font-size", size)
                .attr(tx, "text-anchor", "middle")
                .attr(tx, "class", "sequenceNumber")
                .text(tx, idx);
        }
    }

    fn draw_box(&mut self, bi: usize, startx: f64, starty: f64, stopx: f64, stopy: f64) {
        let root = self.root;
        let g = self.dom.append(root, "g");
        let bx = &self.boxes[bi];
        let r = self.dom.append(g, "rect");
        let (fill, name, x, y, w, tmh) = (
            bx.fill.clone(),
            bx.name.clone(),
            bx.x,
            bx.y,
            bx.width,
            bx.text_max_height,
        );
        self.dom
            .num(r, "x", startx)
            .num(r, "y", starty)
            .attr(r, "fill", fill)
            .attr(r, "stroke", "rgb(0,0,0, 0.5)")
            .num(r, "width", stopx - startx)
            .num(r, "height", stopy - starty)
            .attr(r, "class", "rect");
        self.dom.lower(r);
        if let Some(name) = name.filter(|s| !s.is_empty()) {
            let btm = self.c.box_text_margin;
            self.text_by_tspan(&name, g, x, y + btm + tmh / 2.0, w, 0.0, "text");
        }
        self.dom.lower(g);
    }

    fn draw_popups(&mut self) -> (f64, f64) {
        let mut max_h: f64 = 0.0;
        let mut max_w: f64 = 0.0;
        let keys = self.keys.clone();
        for ai in keys {
            if self.actors[ai].links.is_empty() {
                continue;
            }
            let Some(rd) = self.actors[ai].rect.clone() else {
                continue;
            };
            let mut min_w: f64 = 0.0;
            for (k, _) in &self.actors[ai].links {
                let w = self.c.actor_font.dims(k).width
                    + 2.0 * self.c.wrap_padding
                    + 2.0 * self.c.box_margin;
                if min_w < w {
                    min_w = w;
                }
            }
            let root = self.root;
            let g = self.dom.append(root, "g");
            let cnt = self.actors[ai]
                .actor_cnt
                .map_or("undefined".to_string(), |c| c.to_string());
            self.dom
                .attr(g, "id", format!("actor{cnt}_popup"))
                .attr(g, "class", "actorPopupMenu")
                .attr(
                    g,
                    "display",
                    if self.c.force_menus {
                        "block !important"
                    } else {
                        "none"
                    },
                );
            let class = rd.class.as_ref().map_or(String::new(), |c| format!(" {c}"));
            let menu_w = if rd.width > min_w { rd.width } else { min_w };
            let r = self.dom.append(g, "rect");
            self.dom
                .attr(r, "class", format!("actorPopupMenuPanel{class}"))
                .num(r, "x", rd.x)
                .num(r, "y", rd.height);
            if let Some(f) = &rd.fill {
                self.dom.attr(r, "fill", f.clone());
            }
            if let Some(s) = &rd.stroke {
                self.dom.attr(r, "stroke", s.clone());
            }
            self.dom.num(r, "width", menu_w).num(r, "height", rd.height);
            self.dom.num(r, "rx", rd.rx).num(r, "ry", rd.ry);
            let mut link_y = 20.0;
            let links = self.actors[ai].links.clone();
            let font = self.c.actor_font.clone();
            for (label, url) in links {
                let a = self.dom.append(g, "a");
                self.dom
                    .attr(a, "xlink:href", sanitize_url(&url))
                    .attr(a, "target", "_blank");
                let lines = split_breaks(&label);
                let count = lines.len() as f64;
                let (x, y) = (rd.x + 10.0, rd.height + link_y);
                for (i, line) in lines.iter().enumerate() {
                    let dy = i as f64 * font.size - (font.size * (count - 1.0)) / 2.0;
                    let t = self.dom.append(a, "text");
                    self.dom.num(t, "x", x).num(t, "y", y);
                    self.dom.style(t, "text-anchor", "start");
                    self.dom.style(t, "font-weight", &font.weight);
                    if let Some(f) = &font.family {
                        self.dom.style(t, "font-family", f);
                    }
                    let s = self.dom.append(t, "tspan");
                    self.dom.num(s, "x", x).num(s, "dy", dy);
                    if !line.is_empty() {
                        self.dom.text(s, *line);
                    }
                    self.dom
                        .num(t, "y", y + 10.0)
                        .attr(t, "dominant-baseline", "central")
                        .attr(t, "alignment-baseline", "central")
                        .attr(t, "class", "actor");
                }
                link_y += 30.0;
            }
            self.dom.num(r, "height", link_y);
            max_h = max_h.max(rd.height + link_y);
            max_w = max_w.max(menu_w + self.actors[ai].x);
        }
        (max_h, max_w)
    }
}

struct TextObj {
    x: f64,
    y: f64,
    width: Option<f64>,
    dy: Option<String>,
    text: String,
    class: Option<String>,
    font: Font,
    anchor: Option<String>,
    text_margin: f64,
    valign: Option<String>,
    tspan: bool,
    /// Set by `drawText` with the anchor: adds the `middle` baselines.
    baseline: bool,
}

/// `@braintree/sanitize-url`, reduced to its scheme check.
fn sanitize_url(url: &str) -> String {
    let u = url.trim();
    if u.is_empty() {
        return "about:blank".into();
    }
    let lower: String = u
        .chars()
        .filter(|c| !c.is_control() && !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    for bad in ["javascript:", "data:", "vbscript:"] {
        if lower.starts_with(bad) {
            return "about:blank".into();
        }
    }
    u.to_string()
}

fn insert_defs(dom: &mut Dom, root: NodeId, id: &str) {
    // The icon symbols are constant but for the id: one pre-escaped
    // fragment instead of three escaped 10 kB paths.
    let mut eid = String::new();
    crate::dom::escape_text(id, &mut eid);
    let eid = eid.replace('"', "&quot;");
    dom.append_raw(
        root,
        format!(
            "<defs><symbol id=\"{eid}-computer\" width=\"24\" height=\"24\"><path transform=\"scale(.5)\" d=\"{COMPUTER_ICON}\"></path></symbol></defs>\
             <defs><symbol id=\"{eid}-database\" fill-rule=\"evenodd\" clip-rule=\"evenodd\"><path transform=\"scale(.5)\" d=\"{}\"></path></symbol></defs>\
             <defs><symbol id=\"{eid}-clock\" width=\"24\" height=\"24\"><path transform=\"scale(.5)\" d=\"{CLOCK_ICON}\"></path></symbol></defs>",
            DATABASE_ICON.trim_end()
        ),
    );
}

const COMPUTER_ICON: &str = "M2 2v13h20v-13h-20zm18 11h-16v-9h16v9zm-10.228 6l.466-1h3.524l.467 1h-4.457zm14.228 3h-24l2-6h2.104l-1.33 4h18.45l-1.297-4h2.073l2 6zm-5-10h-14v-7h14v7z";
const CLOCK_ICON: &str = "M12 2c5.514 0 10 4.486 10 10s-4.486 10-10 10-10-4.486-10-10 4.486-10 10-10zm0-2c-6.627 0-12 5.373-12 12s5.373 12 12 12 12-5.373 12-12-5.373-12-12-12zm5.848 12.459c.202.038.202.333.001.372-1.907.361-6.045 1.111-6.547 1.111-.719 0-1.301-.582-1.301-1.301 0-.512.77-5.447 1.125-7.445.034-.192.312-.181.343.014l.985 6.238 5.394 1.011z";

fn marker(dom: &mut Dom, root: NodeId, attrs: &[(&'static str, String)]) -> NodeId {
    let d = dom.append(root, "defs");
    let m = dom.append(d, "marker");
    for (k, v) in attrs {
        dom.attr(m, k, v.clone());
    }
    m
}

fn insert_markers(dom: &mut Dom, root: NodeId, id: &str) {
    let s = |v: &str| v.to_string();
    let user = |name: &str, rx: &str, ry: &str| {
        vec![
            ("id", format!("{id}-{name}")),
            ("refX", s(rx)),
            ("refY", s(ry)),
            ("markerUnits", s("userSpaceOnUse")),
            ("markerWidth", s("12")),
            ("markerHeight", s("12")),
            ("orient", s("auto-start-reverse")),
        ]
    };
    let m = marker(dom, root, &user("arrowhead", "7.9", "5"));
    let p = dom.append(m, "path");
    dom.attr(p, "d", "M -1 0 L 10 5 L 0 10 z");
    let m = marker(
        dom,
        root,
        &[
            ("id", format!("{id}-crosshead")),
            ("markerWidth", s("15")),
            ("markerHeight", s("8")),
            ("orient", s("auto")),
            ("refX", s("4")),
            ("refY", s("4.5")),
        ],
    );
    let p = dom.append(m, "path");
    dom.attr(p, "fill", "none").attr(p, "stroke", "#000000");
    dom.style(p, "stroke-dasharray", "0, 0");
    dom.attr(p, "stroke-width", "1pt")
        .attr(p, "d", "M 1,2 L 6,7 M 6,2 L 1,7");
    let m = marker(
        dom,
        root,
        &[
            ("id", format!("{id}-filled-head")),
            ("refX", s("15.5")),
            ("refY", s("7")),
            ("markerWidth", s("20")),
            ("markerHeight", s("28")),
            ("orient", s("auto")),
        ],
    );
    let p = dom.append(m, "path");
    dom.attr(p, "d", "M 18,7 L9,13 L14,7 L9,1 Z");
    let m = marker(
        dom,
        root,
        &[
            ("id", format!("{id}-sequencenumber")),
            ("refX", s("15")),
            ("refY", s("15")),
            ("markerWidth", s("60")),
            ("markerHeight", s("40")),
            ("orient", s("auto")),
        ],
    );
    let c = dom.append(m, "circle");
    dom.attr(c, "cx", "15")
        .attr(c, "cy", "15")
        .attr(c, "r", "6");
    let m = marker(dom, root, &user("solidTopArrowHead", "7.9", "7.25"));
    let p = dom.append(m, "path");
    dom.attr(p, "d", "M 0 0 L 10 8 L 0 8 z");
    let m = marker(dom, root, &user("solidBottomArrowHead", "7.9", "0.75"));
    let p = dom.append(m, "path");
    dom.attr(p, "d", "M 0 0 L 10 0 L 0 8 z");
    let m = marker(dom, root, &user("stickTopArrowHead", "7.5", "7"));
    let p = dom.append(m, "path");
    dom.attr(p, "d", "M 0 0 L 7 7")
        .attr(p, "stroke", "black")
        .attr(p, "stroke-width", "1.5")
        .attr(p, "fill", "none");
    let m = marker(dom, root, &user("stickBottomArrowHead", "7.5", "0"));
    let p = dom.append(m, "path");
    dom.attr(p, "d", "M 0 7 L 7 0")
        .attr(p, "stroke", "black")
        .attr(p, "stroke-width", "1.5")
        .attr(p, "fill", "none");
}

/// Undoes Mermaid's entity encoding (`decodeEntities`).
fn decode_entities(s: String) -> String {
    // Both placeholders start with a non-ASCII character, so most diagrams
    // skip the copy.
    if !s.contains(['\u{FB02}', '\u{B6}']) {
        return s;
    }
    s.replace("\u{FB02}\u{B0}\u{B0}", "&#")
        .replace("\u{FB02}\u{B0}", "&")
        .replace("\u{B6}\u{DF}", ";")
}

/// Renders a sequence diagram to SVG.
pub fn render(d: &SequenceDiagram, o: &RenderOptions) -> String {
    let c = Conf::new(d);
    let id = o.id_prefix.clone();
    let mut dom = Dom::new("svg");
    let root = Dom::ROOT;
    dom.attr(root, "id", id.clone())
        .attr(root, "width", "100%")
        .attr(root, "xmlns", "http://www.w3.org/2000/svg");
    let style = dom.append(root, "style");
    let mut css = String::with_capacity(CSS.len() + 64 * id.len());
    let mut parts = CSS.split("#ID");
    css.push_str(parts.next().unwrap_or_default());
    for p in parts {
        css.push('#');
        css.push_str(&id);
        css.push_str(p);
    }
    if !c.neo {
        // The classic look draws loop label boxes flat.
        css = css.replace(
            ".labelBox{stroke:#28253D;fill:#ffffff;filter:url(#drop-shadow);}",
            ".labelBox{stroke:#28253D;fill:#ffffff;filter:none;}",
        );
    }
    dom.text(style, css);
    dom.append(root, "g");

    let actors: Vec<Actor> = d
        .participants
        .iter()
        .map(|p| Actor {
            id: p.id.clone(),
            name: p.name.clone(),
            description: p.description.clone(),
            wrap: p.wrap,
            kind: p.kind.name().to_string(),
            group: p.group,
            prev: p.prev.clone(),
            next: p.next.clone(),
            links: p.links.clone(),
            class: p.class.clone(),
            icon: p.icon.clone(),
            width: 0.0,
            height: 0.0,
            margin: None,
            x: 0.0,
            starty: 0.0,
            stopy: None,
            text_height: 0.0,
            actor_cnt: None,
            lifeline: None,
            rect: None,
        })
        .collect();
    let index: HashMap<String, usize> = actors
        .iter()
        .enumerate()
        .map(|(i, a)| (a.id.clone(), i))
        .collect();
    let msgs: Vec<Msg> = d
        .messages
        .iter()
        .enumerate()
        .map(|(i, m)| Msg {
            id: i,
            kind: m.kind,
            from: m.from.clone(),
            to: m.to.clone(),
            text: m.text.clone(),
            wrap: m.wrap,
            activate: m.activate,
            placement: m.placement,
            central: m.central,
            auto: m.autonumber,
            width: None,
        })
        .collect();
    let boxes = d
        .boxes
        .iter()
        .map(|b| BoxModel {
            name: b.name.clone(),
            fill: b.fill.clone(),
            wrap: b.wrap,
            actors: b.participants.clone(),
            margin: 0.0,
            text_max_height: 0.0,
            x: 0.0,
            y: 0.0,
            width: 0.0,
        })
        .collect();
    let mut r = R {
        c,
        id: id.clone(),
        dom,
        root,
        actors,
        index,
        keys: Vec::new(),
        palette_index: HashMap::new(),
        boxes,
        msgs,
        created: d.created.clone(),
        destroyed: d.destroyed.clone(),
        b: Bounds::default(),
        box_models: Vec::new(),
        actor_cnt: -1,
        sequence_numbers: false,
        d,
    };
    r.draw();
    let mut out = String::with_capacity(32 * 1024);
    r.dom.serialize(root, &mut out);
    decode_entities(out)
}

impl<'a> R<'a> {
    fn draw(&mut self) {
        use LineType::*;
        let widths = self.max_message_width_per_actor();
        self.c.height = self.calculate_actor_margins(&widths);
        let id = self.id.clone();
        let root = self.root;
        insert_defs(&mut self.dom, root, &id);
        let has_boxes = !self.boxes.is_empty();
        if has_boxes {
            self.b.bump(self.c.box_margin);
            if self
                .boxes
                .iter()
                .any(|b| b.name.as_deref().is_some_and(|n| !n.is_empty()))
            {
                let h = self.boxes[0].text_max_height;
                self.b.bump(h);
            }
        }
        let mut keys: Vec<usize> = (0..self.actors.len()).collect();
        if self.c.hide_unused {
            let used: Vec<&str> = self
                .msgs
                .iter()
                .flat_map(|m| [m.from.as_deref(), m.to.as_deref()])
                .flatten()
                .collect();
            keys.retain(|&i| used.contains(&self.actors[i].id.as_str()));
        }
        self.keys = keys;
        for (i, &ai) in self.keys.iter().enumerate() {
            self.palette_index
                .entry(self.actors[ai].name.clone())
                .or_insert(i);
        }
        // `new Map(...)` keeps the last index of a repeated name.
        for (i, &ai) in self.keys.iter().enumerate() {
            self.palette_index.insert(self.actors[ai].name.clone(), i);
        }
        self.add_actor_rendering_data();
        let (loop_widths, mut notes, mut models) = self.calculate_loop_bounds();
        insert_markers(&mut self.dom, root, &id);
        if self.c.neo {
            let d = self.dom.append(root, "defs");
            let f = self.dom.append(d, "filter");
            self.dom
                .attr(f, "id", format!("{id}-drop-shadow"))
                .attr(f, "height", "130%")
                .attr(f, "width", "130%");
            let s = self.dom.append(f, "feDropShadow");
            let color = if self.c.theme == "redux" || self.c.theme == "redux-color" {
                "#000000"
            } else {
                "#FFFFFF"
            };
            self.dom
                .attr(s, "dx", "4")
                .attr(s, "dy", "4")
                .attr(s, "stdDeviation", "0")
                .attr(s, "flood-opacity", "0.06")
                .attr(s, "flood-color", color);
        }

        let mut sequence_index = 1.0;
        let mut sequence_step = 1.0;
        let mut to_draw: Vec<(MsgModel, f64, Msg)> = Vec::new();
        let mut backgrounds: Vec<Item> = Vec::new();
        let bm = self.c.box_margin;
        let btm = self.c.box_text_margin;
        for i in 0..self.msgs.len() {
            let kind = self.msgs[i].kind;
            match kind {
                Note => {
                    if let Some(mut nm) = notes[i].take() {
                        let id = self.msgs[i].id;
                        self.draw_note(&mut nm, id);
                    }
                }
                ActiveStart | CentralConnection | CentralConnectionReverse => {
                    let from = self.msgs[i].from.clone().unwrap_or_default();
                    if let Some(a) = self.actor(&from) {
                        let stacked = self.b.activations_of(&from) as f64;
                        let x =
                            a.x + a.width / 2.0 + ((stacked - 1.0) * self.c.activation_width) / 2.0;
                        let g = self.dom.append(root, "g");
                        let vp = self.b.vertical_pos;
                        self.b.activations.push(Activation {
                            startx: x,
                            starty: vp + 2.0,
                            stopx: x + self.c.activation_width,
                            stopy: None,
                            actor: Some(from),
                            anchored: Some(g),
                        });
                    }
                }
                ActiveEnd => {
                    let from = self.msgs[i].from.clone();
                    let pos = self.b.activations.iter().rposition(|a| a.actor == from);
                    let idx = pos.unwrap_or(self.b.activations.len().saturating_sub(1));
                    if idx < self.b.activations.len() {
                        let mut act = self.b.activations.remove(idx);
                        let mut vp = self.b.vertical_pos;
                        if act.starty + 18.0 > vp {
                            act.starty = vp - 6.0;
                            vp += 12.0;
                        }
                        let count = self.b.activations_of(from.as_deref().unwrap_or(""));
                        self.draw_activation(&act, vp, count);
                        self.insert(act.startx, vp - 10.0, act.stopx, vp);
                    }
                }
                LoopStart | OptStart | AltStart | CriticalStart | BreakStart | ParStart
                | ParOverStart => {
                    self.adjust_loop_height_for_wrap(&loop_widths, i, bm, bm + btm);
                }
                RectStart => {
                    self.adjust_loop_height_for_wrap(&loop_widths, i, bm, bm);
                }
                AltElse | ParAnd | CriticalOption => {
                    self.adjust_loop_height_for_wrap(&loop_widths, i, bm + btm, bm);
                }
                RectEnd => {
                    if let Some(it) = self.b.items.pop() {
                        let stopy = it.stopy.unwrap_or(f64::NAN);
                        backgrounds.push(it);
                        let vp = self.b.vertical_pos;
                        self.b.bump(stopy - vp);
                    }
                }
                LoopEnd | OptEnd | AltEnd | ParEnd | CriticalEnd | BreakEnd => {
                    let label = match kind {
                        LoopEnd => "loop",
                        OptEnd => "opt",
                        AltEnd => "alt",
                        ParEnd => "par",
                        CriticalEnd => "critical",
                        _ => "break",
                    };
                    if let Some(it) = self.b.items.pop() {
                        let id = self.msgs[i].id;
                        self.draw_loop(&it, label, id);
                        let stopy = it.stopy.unwrap_or(f64::NAN);
                        let vp = self.b.vertical_pos;
                        self.b.bump(stopy - vp);
                    }
                }
                Autonumber => {
                    if let Some(a) = self.msgs[i].auto {
                        if let Some(s) = a.start.filter(|s| *s != 0.0 && !s.is_nan()) {
                            sequence_index = s;
                        }
                        if let Some(s) = a.step.filter(|s| *s != 0.0 && !s.is_nan()) {
                            sequence_step = s;
                        }
                        self.sequence_numbers = a.visible;
                    }
                }
                _ => {
                    if let Some(mut mm) = models[i].take() {
                        mm.starty = self.b.vertical_pos;
                        mm.sequence_index = sequence_index;
                        mm.sequence_visible = self.sequence_numbers;
                        mm.id = self.msgs[i].id;
                        mm.from = self.msgs[i].from.clone().unwrap_or_default();
                        mm.to = self.msgs[i].to.clone().unwrap_or_default();
                        let lsy = self.bound_message(&mut mm);
                        let msg = self.msgs[i].clone();
                        self.adjust_created_destroyed(&msg, &mut mm, lsy, i);
                        to_draw.push((mm, lsy, msg));
                    }
                }
            }
            if ARROWS.contains(&kind) {
                sequence_index = ((sequence_index + sequence_step) * 100.0).js_round() / 100.0;
            }
        }

        let keys = self.keys.clone();
        for &ai in &keys {
            self.draw_actor(ai, false);
        }
        for (mm, lsy, msg) in &to_draw {
            self.draw_message(mm, *lsy, msg);
        }
        if self.c.mirror_actors {
            let mut max_h: f64 = 0.0;
            self.b.bump(bm * 2.0);
            for &ai in &keys {
                if self.actors[ai].stopy.map_or(true, |s| s == 0.0) {
                    self.actors[ai].stopy = Some(self.b.vertical_pos);
                }
                let h = self.draw_actor(ai, true);
                max_h = js_max(max_h, h);
            }
            self.b.bump(max_h + bm);
        }
        for it in &backgrounds {
            self.draw_background_rect(root, it);
        }
        for &ai in &keys {
            let a = &self.actors[ai];
            if let Some(l) = a.lifeline {
                if !self.c.mirror_actors {
                    if let Some(s) = a.stopy.filter(|s| *s != 0.0) {
                        let v = s + a.height / 2.0;
                        self.dom.num(l, "y2", v);
                    }
                } else {
                    let v = a.stopy;
                    self.dom.attr(l, "y2", on(v));
                }
            }
        }
        let models = self.box_models.clone();
        for bi in models {
            let h = self.b.vertical_pos - self.boxes[bi].y;
            let (x, y, w) = (self.boxes[bi].x, self.boxes[bi].y, self.boxes[bi].width);
            self.insert(x, y, x + w, h);
            let pad = bm * 2.0;
            let sx = x - pad;
            let sy = y - pad * 0.25;
            let ex = sx + w + 2.0 * pad;
            let ey = sy + h + pad * 0.75;
            self.draw_box(bi, sx, sy, ex, ey);
        }
        if has_boxes {
            self.b.bump(bm);
        }
        let (popup_h, popup_w) = self.draw_popups();

        let data = &self.b.data;
        let (sx, sy, ex, ey) = (
            data.startx.unwrap_or(0.0),
            data.starty.unwrap_or(0.0),
            data.stopx.unwrap_or(0.0),
            data.stopy.unwrap_or(0.0),
        );
        let mut box_h = ey - sy;
        if box_h < popup_h {
            box_h = popup_h;
        }
        let mut height = box_h + 2.0 * self.c.diagram_margin_y;
        if self.c.mirror_actors {
            height = height - bm + self.c.bottom_margin_adj;
        }
        let mut box_w = ex - sx;
        if box_w < popup_w {
            box_w = popup_w;
        }
        let width = box_w + 2.0 * self.c.diagram_margin_x;
        let title = self.d.title.clone().filter(|t| !t.is_empty());
        if let Some(t) = &title {
            let e = self.dom.append(root, "text");
            self.dom
                .text(e, t.clone())
                .num(e, "x", (ex - sx) / 2.0 - 2.0 * self.c.diagram_margin_x)
                .attr(e, "y", "-25");
        }
        if self.c.use_max_width {
            self.dom.attr(root, "width", "100%").attr(
                root,
                "style",
                format!("max-width: {}px;", n(width)),
            );
        } else {
            self.dom
                .num(root, "height", height)
                .num(root, "width", width);
        }
        let extra_title = if title.is_some() { 40.0 } else { 0.0 };
        let extra_neo = if !self.actors.is_empty() && self.c.neo {
            30.0
        } else {
            0.0
        };
        self.dom.attr(
            root,
            "viewBox",
            format!(
                "{} -{} {} {}",
                n(sx - self.c.diagram_margin_x),
                n(self.c.diagram_margin_y + extra_title),
                n(width),
                n(height + extra_title + extra_neo)
            ),
        );
        self.dom
            .attr(root, "role", "graphics-document document")
            .attr(root, "aria-roledescription", "sequence");
        let acc_descr = self.d.acc_descr.clone().filter(|t| !t.is_empty());
        let acc_title = self.d.acc_title.clone().filter(|t| !t.is_empty());
        if let Some(desc) = acc_descr {
            self.dom
                .attr(root, "aria-describedby", format!("chart-desc-{id}"));
            let e = self.dom.prepend(root, "desc");
            self.dom
                .attr(e, "id", format!("chart-desc-{id}"))
                .text(e, desc);
        }
        if let Some(t) = acc_title {
            self.dom
                .attr(root, "aria-labelledby", format!("chart-title-{id}"));
            let e = self.dom.prepend(root, "title");
            self.dom
                .attr(e, "id", format!("chart-title-{id}"))
                .text(e, t);
        }
    }
}

/// `Math.round`: halves go up, also below zero.
trait JsRound {
    fn js_round(self) -> f64;
}

impl JsRound for f64 {
    fn js_round(self) -> f64 {
        (self + 0.5).floor()
    }
}
