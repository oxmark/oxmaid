//! Hand-written flowchart parser.
//!
//! It follows Mermaid 12.3's Jison grammar (`flow.jison`) and the semantics
//! of `flowDb.ts`, but is written by hand: one statement at a time, with a
//! character cursor, so that errors carry an exact line and column. Where
//! Mermaid rejects odd input that is harmless (brackets inside an unquoted
//! label, for instance) this parser is more lenient.

use std::collections::HashMap;

use super::{
    Arrow, ClassDef, Direction, Edge, EdgeEnd, Flowchart, LabelType, Node, NodeShape, Stroke, StyleProp, Subgraph,
};
use crate::config::{self, DiagramConfig, Value};
use crate::error::{Error, ErrorKind};
use crate::source::{self, Line};
use crate::Result;

/// Mermaid caps link lengths at 10 ranks.
const MAX_LINK_LENGTH: usize = 10;

/// Parses a flowchart source (front matter and header included).
pub fn parse(src: &str) -> Result<Flowchart> {
    let source = source::split(src);
    let mut p = Parser::default();
    p.chart.config = DiagramConfig::from_source(source.front_matter.as_deref(), &source.directives, "flowchart");
    p.chart.title = p.chart.config.title.clone();
    let chunks = logical_lines(&source.lines);
    let mut header = false;
    for chunk in &chunks {
        if header && p.accessibility(chunk)? {
            continue;
        }
        for (start, end) in split_statements(&chunk.text) {
            let mut cur = Cursor::new(chunk, start, end);
            cur.skip_ws();
            cur.trim_end();
            if cur.at_end() {
                continue;
            }
            if header {
                p.statement(&mut cur)?;
            } else {
                p.header(&mut cur)?;
                header = true;
            }
        }
    }
    if !header {
        return Err(Error::new(ErrorKind::Empty, 0, 0, "diagram is empty"));
    }
    if let Some(open) = p.frames.last() {
        let last = source.lines.last().map_or(0, |l| l.number);
        return Err(Error::new(
            ErrorKind::Syntax,
            last,
            1,
            format!("subgraph `{}` is never closed with `end`", open.id.as_deref().unwrap_or(&open.title)),
        ));
    }
    Ok(p.finish())
}

/// A logical line: one or more source lines joined when a string, a
/// `@{ ... }` block or an `accDescr { ... }` block spans several lines.
struct Chunk {
    text: Vec<char>,
    /// (line, column) of every char, both starting at 1.
    pos: Vec<(usize, usize)>,
}

fn logical_lines(lines: &[Line<'_>]) -> Vec<Chunk> {
    let mut out: Vec<Chunk> = Vec::new();
    let mut open = false;
    for line in lines {
        if open {
            let chunk = out.last_mut().expect("an open chunk exists");
            chunk.text.push('\n');
            chunk.pos.push((line.number, line.offset + 1));
        } else {
            out.push(Chunk { text: Vec::new(), pos: Vec::new() });
        }
        let chunk = out.last_mut().expect("chunk was just pushed");
        for (i, c) in line.text.chars().enumerate() {
            chunk.text.push(c);
            chunk.pos.push((line.number, line.offset + i + 1));
        }
        open = is_unclosed(&chunk.text);
    }
    out
}

/// True when the text ends inside a string, a `@{` block or an `accDescr {` block.
fn is_unclosed(text: &[char]) -> bool {
    let s: String = text.iter().collect();
    let trimmed = s.trim_start();
    if trimmed.starts_with("accDescr") && !trimmed.starts_with("accDescr:") {
        let after = trimmed["accDescr".len()..].trim_start();
        if after.starts_with('{') {
            return !after.contains('}');
        }
    }
    let mut quoted = false;
    let mut data = false;
    let mut i = 0;
    while i < text.len() {
        let c = text[i];
        if c == '"' {
            quoted = !quoted;
        } else if !quoted && c == '@' && text.get(i + 1) == Some(&'{') {
            data = true;
        } else if !quoted && data && c == '}' {
            data = false;
        }
        i += 1;
    }
    quoted || data
}

/// Splits a chunk on `;` outside strings, brackets, `|text|` and `@{ }`.
/// Returns (start, end) char ranges.
fn split_statements(text: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let (mut start, mut depth, mut quoted, mut piped) = (0, 0i32, false, false);
    for (i, &c) in text.iter().enumerate() {
        match c {
            '"' => quoted = !quoted,
            _ if quoted => {}
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth = (depth - 1).max(0),
            '|' if depth == 0 => piped = !piped,
            ';' if depth == 0 && !piped => {
                out.push((start, i));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push((start, text.len()));
    out
}

/// Characters allowed in a node id (Mermaid's `NODE_STRING` and the other
/// `idString` tokens), except `-` and `=` which depend on what follows.
fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || "_!#$%&'*+.`?\\/,".contains(c) || (c == ':')
}

struct Cursor<'a> {
    chunk: &'a Chunk,
    pos: usize,
    end: usize,
}

impl<'a> Cursor<'a> {
    fn new(chunk: &'a Chunk, start: usize, end: usize) -> Self {
        Self { chunk, pos: start, end }
    }

    fn peek(&self) -> Option<char> {
        self.peek_at(0)
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        let i = self.pos + n;
        (i < self.end).then(|| self.chunk.text[i])
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn at_end(&self) -> bool {
        self.pos >= self.end
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    fn trim_end(&mut self) {
        while self.end > self.pos && self.chunk.text[self.end - 1].is_whitespace() {
            self.end -= 1;
        }
    }

    fn starts_with(&self, s: &str) -> bool {
        s.chars().enumerate().all(|(i, c)| self.peek_at(i) == Some(c))
    }

    /// `starts_with` followed by a word boundary.
    fn starts_with_word(&self, s: &str) -> bool {
        self.starts_with(s) && !self.peek_at(s.chars().count()).is_some_and(|c| c.is_alphanumeric() || c == '_')
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.starts_with(s) {
            self.pos += s.chars().count();
            true
        } else {
            false
        }
    }

    fn find(&self, s: &str) -> Option<usize> {
        let pat: Vec<char> = s.chars().collect();
        (self.pos..=self.end.saturating_sub(pat.len())).find(|&i| self.chunk.text[i..self.end].starts_with(&pat))
    }

    fn count_run(&mut self, c: char) -> usize {
        let start = self.pos;
        while self.peek() == Some(c) {
            self.pos += 1;
        }
        self.pos - start
    }

    fn slice(&self, from: usize, to: usize) -> String {
        self.chunk.text[from..to].iter().collect()
    }

    fn rest(&self) -> String {
        self.slice(self.pos, self.end)
    }

    fn read_word(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(|c| !c.is_whitespace()) {
            self.pos += 1;
        }
        self.slice(start, self.pos)
    }

    /// Reads a node id: Mermaid's `idString`. `-` is part of an id unless a
    /// link starts there (`-` followed by `-`, `.` or `>`), and `=` unless
    /// followed by another `=`. `:::` starts a class.
    fn read_id(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek() {
            let next = self.peek_at(1);
            let ok = match c {
                '-' => next.is_some_and(|n| !matches!(n, '>' | '-' | '.')),
                '=' => next != Some('='),
                ':' => !self.starts_with(":::"),
                '@' => false,
                c => is_id_char(c),
            };
            if !ok {
                break;
            }
            self.pos += 1;
        }
        self.slice(start, self.pos)
    }

    /// Reads `"..."` or ``"`...`"``, the cursor being on the opening quote.
    fn read_string(&mut self) -> Result<(String, LabelType)> {
        let start = self.pos;
        self.bump();
        if self.peek() == Some('`') {
            let content = self.pos + 1;
            let end = self.find("`\"").ok_or_else(|| self.err_at(start, "unterminated markdown string, expected `\"`"))?;
            self.pos = end + 2;
            return Ok((self.slice(content, end), LabelType::Markdown));
        }
        let content = self.pos;
        while let Some(c) = self.bump() {
            if c == '"' {
                return Ok((self.slice(content, self.pos - 1), LabelType::String));
            }
        }
        Err(self.err_at(start, "unterminated string, expected a closing `\"`"))
    }

    fn err(&self, msg: impl Into<String>) -> Error {
        self.err_at(self.pos, msg)
    }

    fn err_at(&self, pos: usize, msg: impl Into<String>) -> Error {
        let (line, column) = self
            .chunk
            .pos
            .get(pos)
            .copied()
            .or_else(|| self.chunk.pos.last().map(|&(l, c)| (l, c + 1)))
            .unwrap_or((0, 0));
        Error::new(ErrorKind::Syntax, line, column, msg)
    }
}

/// Shape openers, longest first, with their closers.
const SHAPES: &[(&str, &[(&str, NodeShape)])] = &[
    ("(((", &[(")))", NodeShape::DoubleCircle)]),
    ("((", &[("))", NodeShape::Circle)]),
    ("(-", &[("-)", NodeShape::Ellipse)]),
    ("([", &[("])", NodeShape::Stadium)]),
    ("[[", &[("]]", NodeShape::Subroutine)]),
    ("[(", &[(")]", NodeShape::Cylinder)]),
    ("[/", &[("/]", NodeShape::LeanRight), ("\\]", NodeShape::TrapezoidBottom)]),
    ("[\\", &[("\\]", NodeShape::LeanLeft), ("/]", NodeShape::TrapezoidTop)]),
    ("{{", &[("}}", NodeShape::Hexagon)]),
    ("(", &[(")", NodeShape::Rounded)]),
    ("[", &[("]", NodeShape::Rect)]),
    ("{", &[("}", NodeShape::Diamond)]),
    (">", &[("]", NodeShape::Odd)]),
];

struct Link {
    id: Option<String>,
    stroke: Stroke,
    start: Arrow,
    end: Arrow,
    length: usize,
    label: Option<(String, LabelType)>,
}

/// An open `subgraph ... end` block.
struct Frame {
    id: Option<String>,
    title: String,
    label_type: LabelType,
    /// Ids mentioned in the block, in Mermaid's order.
    items: Vec<String>,
    direction: Option<Direction>,
}

/// A closed subgraph with its member ids.
struct SubgraphDef {
    sg: Subgraph,
    members: Vec<String>,
}

#[derive(Default)]
struct Parser {
    chart: Flowchart,
    index: HashMap<String, usize>,
    frames: Vec<Frame>,
    subgraphs: Vec<SubgraphDef>,
    sub_count: usize,
}

impl Parser {
    fn header(&mut self, cur: &mut Cursor) -> Result<()> {
        let start = cur.pos;
        let keyword = cur.read_word();
        if !matches!(keyword.as_str(), "flowchart" | "graph" | "flowchart-elk") {
            return Err(cur.err_at(start, format!("expected `flowchart` or `graph`, found `{keyword}`")));
        }
        if keyword == "flowchart-elk" && self.chart.config.layout.is_none() {
            self.chart.config.layout = Some("elk".into());
        }
        cur.skip_ws();
        if !cur.at_end() {
            let start = cur.pos;
            let dir = cur.read_word();
            self.chart.direction = Direction::from_keyword(&dir).ok_or_else(|| {
                cur.err_at(start, format!("unknown direction `{dir}`, expected TB, TD, BT, LR or RL"))
            })?;
            cur.skip_ws();
            if !cur.at_end() {
                return Err(cur.err("unexpected text after the diagram header"));
            }
        }
        Ok(())
    }

    /// `accTitle: ...`, `accDescr: ...` and `accDescr { ... }` take the
    /// whole logical line, `;` included.
    fn accessibility(&mut self, chunk: &Chunk) -> Result<bool> {
        let text: String = chunk.text.iter().collect();
        let t = text.trim_start();
        if let Some(rest) = t.strip_prefix("accTitle") {
            if let Some(value) = rest.trim_start().strip_prefix(':') {
                self.chart.acc_title = Some(value.trim().to_string());
                return Ok(true);
            }
        }
        if let Some(rest) = t.strip_prefix("accDescr") {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix(':') {
                self.chart.acc_descr = Some(value.trim().to_string());
                return Ok(true);
            }
            if let Some(body) = rest.strip_prefix('{') {
                let body = body.split_once('}').map_or(body, |(b, _)| b);
                let lines: Vec<&str> = body.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
                self.chart.acc_descr = Some(lines.join("\n"));
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn statement(&mut self, cur: &mut Cursor) -> Result<()> {
        let start = cur.pos;
        if cur.starts_with_word("subgraph") {
            cur.pos += "subgraph".len();
            return self.subgraph(cur);
        }
        if cur.starts_with_word("end") {
            cur.pos += 3;
            cur.skip_ws();
            if cur.at_end() {
                return self.end_subgraph(cur, start);
            }
            cur.pos = start;
        }
        if cur.starts_with_word("direction") {
            cur.pos += "direction".len();
            cur.skip_ws();
            let at = cur.pos;
            let d = cur.read_word();
            let d = Direction::from_keyword(&d).ok_or_else(|| cur.err_at(at, format!("unknown direction `{d}`")))?;
            match self.frames.last_mut() {
                Some(frame) => frame.direction = Some(d),
                None => self.chart.direction = d,
            }
            return Ok(());
        }
        for (keyword, f) in [
            ("classDef", Parser::class_def as fn(&mut Parser, &mut Cursor) -> Result<()>),
            ("class", Parser::class_statement),
            ("style", Parser::style_statement),
            ("linkStyle", Parser::link_style),
            ("click", Parser::click),
        ] {
            if cur.starts_with_word(keyword) {
                cur.pos += keyword.len();
                return f(self, cur);
            }
        }
        let ids = self.chain(cur)?;
        if let Some(frame) = self.frames.last_mut() {
            frame.items.extend(ids);
        }
        Ok(())
    }

    fn subgraph(&mut self, cur: &mut Cursor) -> Result<()> {
        cur.skip_ws();
        let mut frame = Frame { id: None, title: String::new(), label_type: LabelType::Text, items: Vec::new(), direction: None };
        if !cur.at_end() {
            let (text, ty) = if cur.peek() == Some('"') {
                let (t, ty) = cur.read_string()?;
                // A quoted subgraph id/title is plain text unless it is markdown.
                (t, if ty == LabelType::Markdown { ty } else { LabelType::Text })
            } else {
                let start = cur.pos;
                while cur.peek().is_some_and(|c| c != '[') {
                    cur.pos += 1;
                }
                (cur.slice(start, cur.pos).trim().to_string(), LabelType::Text)
            };
            cur.skip_ws();
            if cur.peek() == Some('[') {
                let open = cur.pos;
                cur.bump();
                let (title, title_type) = if cur.peek() == Some('"') {
                    cur.read_string()?
                } else {
                    let end = cur.find("]").ok_or_else(|| cur.err_at(open, "unclosed `[` in subgraph title"))?;
                    let t = cur.slice(cur.pos, end);
                    cur.pos = end;
                    (t, LabelType::Text)
                };
                cur.skip_ws();
                if !cur.eat("]") {
                    return Err(cur.err("expected `]` after the subgraph title"));
                }
                frame.id = Some(text.trim().to_string());
                frame.title = title;
                frame.label_type = title_type;
            } else {
                // `subgraph one two`: a title with spaces gets a generated id.
                if !text.chars().any(char::is_whitespace) {
                    frame.id = Some(text.clone());
                }
                frame.title = text;
                frame.label_type = ty;
            }
            cur.skip_ws();
            if !cur.at_end() {
                return Err(cur.err("unexpected text after the subgraph title"));
            }
        }
        self.frames.push(frame);
        Ok(())
    }

    /// Closes a subgraph like `FlowDB.addSubGraph`: members already owned by
    /// an earlier subgraph stay there, and a repeated id merges.
    fn end_subgraph(&mut self, cur: &Cursor, at: usize) -> Result<()> {
        let frame = self.frames.pop().ok_or_else(|| cur.err_at(at, "`end` without a matching `subgraph`"))?;
        let mut seen = std::collections::HashSet::new();
        let mut members: Vec<String> =
            frame.items.into_iter().filter(|i| !i.trim().is_empty() && seen.insert(i.clone())).collect();
        let id = frame.id.unwrap_or_else(|| format!("subGraph{}", self.sub_count));
        self.sub_count += 1;
        members.retain(|m| !self.subgraphs.iter().any(|s| s.members.contains(m)) && *m != id);
        match self.subgraphs.iter_mut().find(|s| s.sg.id == id) {
            Some(existing) => existing.members.extend(members),
            None => self.subgraphs.push(SubgraphDef {
                sg: Subgraph {
                    id: id.clone(),
                    title: clean_label(&frame.title, frame.label_type),
                    label_type: frame.label_type,
                    direction: frame.direction,
                    ..Subgraph::default()
                },
                members,
            }),
        }
        if let Some(parent) = self.frames.last_mut() {
            parent.items.push(id);
        }
        Ok(())
    }

    fn class_def(&mut self, cur: &mut Cursor) -> Result<()> {
        cur.skip_ws();
        let names = cur.read_word();
        if names.is_empty() {
            return Err(cur.err("expected a class name after `classDef`"));
        }
        cur.skip_ws();
        let styles = parse_styles(&cur.rest());
        for name in names.split(',').filter(|n| !n.is_empty()) {
            match self.chart.class_defs.iter_mut().find(|c| c.name == name) {
                Some(def) => def.styles.extend(styles.iter().cloned()),
                None => self.chart.class_defs.push(ClassDef { name: name.to_string(), styles: styles.clone() }),
            }
        }
        Ok(())
    }

    fn class_statement(&mut self, cur: &mut Cursor) -> Result<()> {
        cur.skip_ws();
        let ids = cur.read_word();
        cur.skip_ws();
        let class = cur.read_word();
        if ids.is_empty() || class.is_empty() {
            return Err(cur.err("expected `class <node ids> <class name>`"));
        }
        self.set_class(&ids, &class);
        Ok(())
    }

    fn set_class(&mut self, ids: &str, class: &str) {
        for id in ids.split(',') {
            if let Some(&i) = self.index.get(id) {
                self.chart.nodes[i].classes.push(class.to_string());
            }
            if let Some(s) = self.subgraphs.iter_mut().find(|s| s.sg.id == id) {
                s.sg.classes.push(class.to_string());
            }
        }
    }

    fn style_statement(&mut self, cur: &mut Cursor) -> Result<()> {
        cur.skip_ws();
        let id = cur.read_word();
        if id.is_empty() {
            return Err(cur.err("expected a node id after `style`"));
        }
        cur.skip_ws();
        let styles = parse_styles(&cur.rest());
        // Like Mermaid, styling an unknown node creates it.
        let i = self.ensure_node(&id);
        self.chart.nodes[i].styles.extend(styles);
        Ok(())
    }

    /// `linkStyle default|0,1 [interpolate curve] styles`
    fn link_style(&mut self, cur: &mut Cursor) -> Result<()> {
        cur.skip_ws();
        let at = cur.pos;
        let target = cur.read_word();
        let positions: Vec<Option<usize>> = if target == "default" {
            vec![None]
        } else {
            target
                .split(',')
                .map(|n| n.parse::<usize>().map(Some).map_err(|_| cur.err_at(at, format!("invalid linkStyle index `{n}`"))))
                .collect::<Result<_>>()?
        };
        for p in positions.iter().flatten() {
            if *p >= self.chart.edges.len() {
                return Err(cur.err_at(
                    at,
                    format!(
                        "the index {p} for linkStyle is out of bounds, valid indices are between 0 and {}",
                        self.chart.edges.len() as isize - 1
                    ),
                ));
            }
        }
        cur.skip_ws();
        let mut curve = None;
        if cur.starts_with_word("interpolate") {
            cur.pos += "interpolate".len();
            cur.skip_ws();
            curve = Some(cur.read_word());
            cur.skip_ws();
        }
        let mut styles = parse_styles(&cur.rest());
        for p in positions {
            match p {
                None => {
                    if curve.is_some() {
                        self.chart.default_curve = curve.clone();
                    }
                    if !styles.is_empty() {
                        self.chart.default_link_styles = styles.clone();
                    }
                }
                Some(i) => {
                    let edge = &mut self.chart.edges[i];
                    if curve.is_some() {
                        edge.curve = curve.clone();
                    }
                    if !styles.is_empty() {
                        if !styles.iter().any(|(k, _)| k.starts_with("fill")) {
                            styles.push(("fill".into(), "none".into()));
                        }
                        edge.styles = styles.clone();
                    }
                }
            }
        }
        Ok(())
    }

    /// `click id callback`, `click id call fn(args)`, `click id href "url"`,
    /// `click id "url"`, each with an optional `"tooltip"` and link target.
    fn click(&mut self, cur: &mut Cursor) -> Result<()> {
        cur.skip_ws();
        let id = cur.read_word();
        if id.is_empty() {
            return Err(cur.err("expected a node id after `click`"));
        }
        cur.skip_ws();
        let mut link = None;
        if cur.starts_with_word("href") {
            cur.pos += 4;
            cur.skip_ws();
            link = Some(cur.read_string()?.0);
        } else if cur.peek() == Some('"') {
            link = Some(cur.read_string()?.0);
        } else if cur.starts_with_word("call") {
            cur.pos += 4;
            cur.skip_ws();
            while cur.peek().is_some_and(|c| c != ')') {
                cur.pos += 1;
            }
            cur.bump();
        } else {
            cur.read_word();
        }
        cur.skip_ws();
        let mut tooltip = None;
        if cur.peek() == Some('"') {
            tooltip = Some(cur.read_string()?.0);
            cur.skip_ws();
        }
        let target = (!cur.at_end()).then(|| cur.read_word());
        if let Some(&i) = self.index.get(&id) {
            let node = &mut self.chart.nodes[i];
            if link.is_some() {
                node.link = link;
                node.link_target = target;
            }
            if tooltip.is_some() {
                node.tooltip = tooltip;
            }
        }
        Ok(())
    }

    /// `A --> B & C -- text --> D`. Returns the node ids in Mermaid's order
    /// (last group first), for subgraph membership.
    fn chain(&mut self, cur: &mut Cursor) -> Result<Vec<String>> {
        let mut left = self.node_group(cur)?;
        let mut groups = vec![left.clone()];
        loop {
            cur.skip_ws();
            if cur.at_end() {
                break;
            }
            let link = self.link(cur)?;
            let right = self.node_group(cur)?;
            self.add_links(&left, &right, &link);
            groups.push(right.clone());
            left = right;
        }
        Ok(groups.into_iter().rev().flatten().map(|g| g.0).collect())
    }

    fn add_links(&mut self, left: &[(String, Option<usize>)], right: &[(String, Option<usize>)], link: &Link) {
        let last_start = left.last().map(|l| &l.0);
        let first_end = right.first().map(|r| &r.0);
        for (start_id, from) in left {
            for (end_id, to) in right {
                let (Some(from), Some(to)) = (*from, *to) else { continue };
                let user_id = link
                    .id
                    .clone()
                    .filter(|id| Some(start_id) == last_start && Some(end_id) == first_end && !self.chart.edges.iter().any(|e| &e.id == id));
                let id = user_id.unwrap_or_else(|| {
                    let same = self
                        .chart
                        .edges
                        .iter()
                        .filter(|e| e.from == EdgeEnd::Node(from) && e.to == EdgeEnd::Node(to))
                        .count();
                    format!("L_{start_id}_{end_id}_{}", if same == 0 { 0 } else { same + 1 })
                });
                let (label, label_type) = match &link.label {
                    Some((l, t)) => (Some(l.clone()), *t),
                    None => (None, LabelType::Text),
                };
                self.chart.edges.push(Edge {
                    id,
                    from: EdgeEnd::Node(from),
                    to: EdgeEnd::Node(to),
                    label: label.filter(|l| !l.is_empty()),
                    label_type,
                    stroke: link.stroke,
                    start: link.start,
                    end: link.end,
                    length: link.length.clamp(1, MAX_LINK_LENGTH),
                    curve: self.chart.default_curve.clone(),
                    ..Edge::default()
                });
            }
        }
    }

    /// Nodes separated by `&`. Each entry is (id, node index); the index is
    /// `None` when the id named an edge (`e1@{ ... }`).
    fn node_group(&mut self, cur: &mut Cursor) -> Result<Vec<(String, Option<usize>)>> {
        let mut group = vec![self.node(cur)?];
        loop {
            let save = cur.pos;
            cur.skip_ws();
            if cur.eat("&") {
                group.push(self.node(cur)?);
            } else {
                cur.pos = save;
                return Ok(group);
            }
        }
    }

    fn node(&mut self, cur: &mut Cursor) -> Result<(String, Option<usize>)> {
        cur.skip_ws();
        let id = cur.read_id();
        if id.is_empty() {
            return Err(match cur.peek() {
                Some(c) => cur.err(format!("expected a node id, found `{c}`")),
                None => cur.err("expected a node id"),
            });
        }
        let shape = self.shape(cur)?;
        if shape.is_none() && matches!(cur.peek(), Some('[' | '(' | '{')) {
            return Err(cur.err("this node shape is never closed"));
        }
        let mut classes = Vec::new();
        while cur.eat(":::") {
            let class = cur.read_id();
            if class.is_empty() {
                return Err(cur.err("expected a class name after `:::`"));
            }
            classes.push(class);
        }
        let data_at = cur.pos;
        let data = if cur.starts_with("@{") { Some(self.shape_data(cur)?) } else { None };

        // `e1@{ animate: true }` sets options of the edge `e1`.
        if shape.is_none() && !self.index.contains_key(&id) {
            if let (Some(data), Some(edge)) = (&data, self.chart.edges.iter_mut().find(|e| e.id == id)) {
                apply_edge_data(edge, data);
                return Ok((id, None));
            }
            if data.is_some() && self.subgraphs.iter().any(|s| s.sg.id == id) {
                return Ok((id, None));
            }
        }

        let idx = self.ensure_node(&id);
        let node = &mut self.chart.nodes[idx];
        if let Some((shape, label, ty)) = shape {
            node.shape = shape;
            node.label = clean_label(&label, ty);
            node.label_type = ty;
        }
        if let Some(data) = data {
            apply_node_data(node, &data).map_err(|m| cur.err_at(data_at, m))?;
        }
        for class in classes {
            self.set_class(&id, &class);
        }
        Ok((id, Some(idx)))
    }

    /// Reads `@{ ... }`, the cursor being on `@`.
    fn shape_data(&mut self, cur: &mut Cursor) -> Result<Value> {
        let open = cur.pos;
        cur.pos += 2;
        let start = cur.pos;
        let mut quoted = false;
        while let Some(c) = cur.peek() {
            match c {
                '"' => quoted = !quoted,
                '}' if !quoted => {
                    let body = cur.slice(start, cur.pos);
                    cur.bump();
                    return Ok(config::parse_shape_data(&body));
                }
                _ => {}
            }
            cur.bump();
        }
        Err(cur.err_at(open, "unclosed `@{`, expected `}`"))
    }

    fn shape(&mut self, cur: &mut Cursor) -> Result<Option<(NodeShape, String, LabelType)>> {
        let start = cur.pos;
        for (open, closers) in SHAPES {
            if !cur.eat(open) {
                continue;
            }
            if let Some(found) = shape_text(cur, closers)? {
                return Ok(Some(found));
            }
            cur.pos = start;
        }
        Ok(None)
    }

    fn link(&mut self, cur: &mut Cursor) -> Result<Link> {
        // Edge id: `A e1@--> B`.
        let mut id = None;
        let save = cur.pos;
        let word = cur.read_id();
        if !word.is_empty() && cur.peek() == Some('@') && !matches!(cur.peek_at(1), Some('{' | '"')) {
            cur.bump();
            id = Some(word);
        } else {
            cur.pos = save;
        }

        let link_start = cur.pos;
        let mut start = Arrow::None;
        match cur.peek() {
            Some('<') => {
                start = Arrow::Point;
                cur.bump();
            }
            Some(c @ ('o' | 'x')) if matches!(cur.peek_at(1), Some('-' | '=' | '.')) => {
                start = if c == 'o' { Arrow::Circle } else { Arrow::Cross };
                cur.bump();
            }
            _ => {}
        }
        let (stroke, length, mut label, end) = match cur.peek() {
            Some('-') if cur.peek_at(1) == Some('.') => dotted_link(cur)?,
            Some('.') => dotted_link(cur)?,
            Some('-') => solid_link(cur, '-')?,
            Some('=') => solid_link(cur, '=')?,
            Some('~') => {
                let n = cur.count_run('~');
                if n < 3 {
                    return Err(cur.err_at(link_start, "invisible links are written `~~~`"));
                }
                (Stroke::Invisible, n - 2, None, Arrow::None)
            }
            Some(c) => return Err(cur.err(format!("expected a link such as `-->`, found `{c}`"))),
            None => return Err(cur.err("expected a link such as `-->`")),
        };
        // A start marker only counts when it matches the end one (`<-->`,
        // `o--o`). With link text, a mismatch is an error, as in Mermaid.
        let start = match (start, end) {
            (s, e) if s == e => s,
            (Arrow::None, _) => Arrow::None,
            _ if label.is_some() => {
                return Err(cur.err_at(link_start, "the two ends of this link use different markers"))
            }
            _ => Arrow::None,
        };
        let save = cur.pos;
        cur.skip_ws();
        if cur.peek() == Some('|') {
            let open = cur.pos;
            cur.bump();
            if cur.peek() == Some('"') {
                let (t, ty) = cur.read_string()?;
                cur.skip_ws();
                if !cur.eat("|") {
                    return Err(cur.err_at(open, "unterminated `|` link label"));
                }
                label = Some((t, ty));
            } else {
                let end_pos = cur.find("|").ok_or_else(|| cur.err_at(open, "unterminated `|` link label"))?;
                label = Some((cur.slice(cur.pos, end_pos), LabelType::Text));
                cur.pos = end_pos + 1;
            }
        } else {
            cur.pos = save;
        }
        let label = label.map(|(t, ty)| (clean_label(&t, ty), ty));
        Ok(Link { id, stroke, start, end, length, label })
    }

    fn ensure_node(&mut self, id: &str) -> usize {
        if let Some(&i) = self.index.get(id) {
            return i;
        }
        self.chart.nodes.push(Node { id: id.to_string(), label: id.to_string(), ..Node::default() });
        let i = self.chart.nodes.len() - 1;
        self.index.insert(id.to_string(), i);
        i
    }

    fn finish(mut self) -> Flowchart {
        // Subgraphs, in closing order, and their members.
        let defs = std::mem::take(&mut self.subgraphs);
        let sg_index: HashMap<String, usize> = defs.iter().enumerate().map(|(i, d)| (d.sg.id.clone(), i)).collect();
        let mut subgraphs: Vec<Subgraph> = defs.iter().map(|d| d.sg.clone()).collect();
        for (s, def) in defs.iter().enumerate() {
            for m in &def.members {
                if let Some(&child) = sg_index.get(m) {
                    if child != s && subgraphs[child].parent.is_none() {
                        subgraphs[child].parent = Some(s);
                    }
                } else if let Some(&n) = self.index.get(m) {
                    if self.chart.nodes[n].subgraph.is_none() {
                        self.chart.nodes[n].subgraph = Some(s);
                    }
                }
            }
        }
        self.chart.subgraphs = subgraphs;
        self.link_subgraphs(&sg_index);
        self.chart
    }

    /// A node whose id is a subgraph id is the subgraph itself: retarget its
    /// links, move its classes and styles, and drop the node.
    fn link_subgraphs(&mut self, sg_index: &HashMap<String, usize>) {
        if !self.chart.nodes.iter().any(|n| sg_index.contains_key(&n.id)) {
            return;
        }
        let mut remap: Vec<EdgeEnd> = Vec::with_capacity(self.chart.nodes.len());
        let mut kept = Vec::new();
        for n in std::mem::take(&mut self.chart.nodes) {
            match sg_index.get(&n.id) {
                Some(&s) => {
                    let sg = &mut self.chart.subgraphs[s];
                    sg.classes.extend(n.classes);
                    sg.styles.extend(n.styles);
                    remap.push(EdgeEnd::Subgraph(s));
                }
                None => {
                    remap.push(EdgeEnd::Node(kept.len()));
                    kept.push(n);
                }
            }
        }
        self.chart.nodes = kept;
        for e in &mut self.chart.edges {
            for end in [&mut e.from, &mut e.to] {
                if let EdgeEnd::Node(i) = *end {
                    *end = remap[i];
                }
            }
        }
        self.index = self.chart.nodes.iter().enumerate().map(|(i, n)| (n.id.clone(), i)).collect();
    }
}

fn apply_node_data(node: &mut Node, data: &Value) -> std::result::Result<(), String> {
    if let Some(shape) = data.get("shape").and_then(Value::as_str) {
        if shape != shape.to_lowercase() || shape.contains('_') {
            return Err(format!("no such shape: `{shape}`, shape names are lowercase"));
        }
        node.shape = NodeShape::from_name(shape).ok_or_else(|| format!("no such shape: `{shape}`"))?;
    }
    if let Some(label) = data.get("label").and_then(Value::as_str) {
        if !label.is_empty() {
            node.label_type = match data.get("labelType").and_then(Value::as_str) {
                Some("string") => LabelType::String,
                Some("text") => LabelType::Text,
                _ => LabelType::Markdown,
            };
            node.label = clean_label(label, node.label_type);
        }
    }
    let has_asset = data.get("icon").is_some() || data.get("img").is_some();
    if has_asset && data.get("label").is_none() && node.label == node.id {
        node.label.clear();
    }
    Ok(())
}

fn apply_edge_data(edge: &mut Edge, data: &Value) {
    if let Some(a) = data.get("animate").and_then(Value::as_bool) {
        edge.animate = a;
    }
    if let Some(a) = data.get("animation").and_then(Value::as_str) {
        edge.animation = Some(a.to_string());
    }
    if let Some(c) = data.get("curve").and_then(Value::as_str) {
        edge.curve = Some(c.to_string());
    }
}

/// Reads a node label up to one of `closers`. `None` means no closer was found.
fn shape_text(cur: &mut Cursor, closers: &[(&str, NodeShape)]) -> Result<Option<(NodeShape, String, LabelType)>> {
    if cur.peek() == Some('"') {
        let (text, ty) = cur.read_string()?;
        cur.skip_ws();
        for (closer, shape) in closers {
            if cur.eat(closer) {
                return Ok(Some((*shape, text, ty)));
            }
        }
        return Ok(None);
    }
    let best = closers
        .iter()
        .filter_map(|(closer, shape)| cur.find(closer).map(|p| (p, *closer, *shape)))
        .min_by_key(|(p, _, _)| *p);
    Ok(best.map(|(p, closer, shape)| {
        let text = cur.slice(cur.pos, p);
        cur.pos = p + closer.chars().count();
        (shape, text, LabelType::Text)
    }))
}

/// Marker right after the line of a link: `>`, `o` or `x`. Like Mermaid,
/// `A--oB` is a circle end towards `B`; write `A--- oB` to link to `oB`.
fn link_head(cur: &mut Cursor) -> Option<Arrow> {
    let arrow = match cur.peek()? {
        '>' => Arrow::Point,
        'o' => Arrow::Circle,
        'x' => Arrow::Cross,
        _ => return None,
    };
    cur.bump();
    Some(arrow)
}

type LinkParts = (Stroke, usize, Option<(String, LabelType)>, Arrow);

/// Edge text between `--` (or `==`) and the closing link, plain or quoted.
fn edge_text(cur: &mut Cursor, end: usize) -> Result<(String, LabelType)> {
    let start = cur.pos;
    let raw = cur.slice(start, end);
    let trimmed = raw.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        let lead = raw.len() - raw.trim_start().len();
        cur.pos = start + raw[..lead].chars().count();
        let t = cur.read_string()?;
        cur.pos = end;
        return Ok(t);
    }
    cur.pos = end;
    Ok((raw, LabelType::Text))
}

/// `-->`, `---`, `-- text -->`, and the thick `==>` variants.
fn solid_link(cur: &mut Cursor, ch: char) -> Result<LinkParts> {
    let stroke = if ch == '-' { Stroke::Normal } else { Stroke::Thick };
    let start = cur.pos;
    let n = cur.count_run(ch);
    if n < 2 {
        return Err(cur.err_at(start, format!("expected a link such as `{ch}{ch}>`")));
    }
    if let Some(head) = link_head(cur) {
        return Ok((stroke, n - 1, None, head));
    }
    if n >= 3 {
        return Ok((stroke, n - 2, None, Arrow::None));
    }
    // Text form: `-- text -->`. The text runs up to the next `--`.
    let pattern: String = [ch, ch].iter().collect();
    let end = cur
        .find(&pattern)
        .ok_or_else(|| cur.err_at(start, format!("unterminated link text, expected `{pattern}>` after it")))?;
    let text = edge_text(cur, end)?;
    let m = cur.count_run(ch);
    let head = link_head(cur);
    if head.is_none() && m < 3 {
        return Err(cur.err_at(end, format!("expected `{pattern}>` or `{pattern}{ch}` to close the link text")));
    }
    let length = if head.is_some() { m - 1 } else { m - 2 };
    Ok((stroke, length.max(1), Some(text), head.unwrap_or(Arrow::None)))
}

/// `-.->`, `-.-`, `-..->`, `.->`, `-. text .->`.
fn dotted_link(cur: &mut Cursor) -> Result<LinkParts> {
    let start = cur.pos;
    let dash = cur.eat("-");
    let dots = cur.count_run('.');
    if cur.eat("-") {
        let head = link_head(cur).unwrap_or(Arrow::None);
        return Ok((Stroke::Dotted, dots, None, head));
    }
    if !dash || dots != 1 {
        return Err(cur.err_at(start, "expected a dotted link such as `-.->`"));
    }
    let end = cur
        .find(".-")
        .ok_or_else(|| cur.err_at(start, "unterminated dotted link text, expected `.->` after it"))?;
    let text = edge_text(cur, end)?;
    let dots = cur.count_run('.');
    cur.bump();
    let head = link_head(cur).unwrap_or(Arrow::None);
    Ok((Stroke::Dotted, dots.max(1), Some(text), head))
}

/// `fill:#f9f,stroke:#333,stroke-width:4px`. A `\,` keeps a comma inside a
/// value, as in Mermaid.
fn parse_styles(s: &str) -> Vec<StyleProp> {
    let s = s.trim().trim_end_matches(';');
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&',') => {
                current.push(',');
                chars.next();
            }
            ',' | ';' => parts.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
    }
    parts.push(current);
    parts
        .into_iter()
        .filter_map(|p| p.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .filter(|(k, v)| !k.is_empty() && !v.is_empty())
        .collect()
}

/// Normalizes a label: trims, strips surrounding quotes, turns `<br>` (and,
/// in plain text, a literal `\n`) into a newline, and decodes Mermaid
/// entities (`#quot;`, `#35;`). Inline HTML and Markdown stay as written.
pub(crate) fn clean_label(s: &str, ty: LabelType) -> String {
    let mut s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s = &s[1..s.len() - 1];
    }
    let mut out = String::new();
    let mut rest = s;
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(i) = lower.find("<br") else {
            out.push_str(rest);
            break;
        };
        match lower[i..].find('>') {
            Some(j) if lower[i + 3..i + j].trim().trim_end_matches('/').trim().is_empty() => {
                out.push_str(&rest[..i]);
                out.push('\n');
                rest = &rest[i + j + 1..];
            }
            _ => {
                out.push_str(&rest[..i + 3]);
                rest = &rest[i + 3..];
            }
        }
    }
    if ty != LabelType::Markdown {
        out = out.replace("\\n", "\n");
    }
    let decoded = decode_entities(&out);
    if ty == LabelType::Markdown {
        // Markdown: a newline and the indentation after it is one line break.
        decoded.split('\n').map(str::trim_start).collect::<Vec<_>>().join("\n")
    } else {
        decoded
    }
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('#') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let entity = after.find(';').filter(|&j| j > 0 && j <= 8).map(|j| &after[..j]);
        let decoded = entity.and_then(|e| match e {
            "quot" => Some('"'),
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => e.parse::<u32>().ok().and_then(char::from_u32),
        });
        match (entity, decoded) {
            (Some(e), Some(c)) => {
                out.push(c);
                rest = &after[e.len() + 1..];
            }
            _ => {
                out.push('#');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(clean_label(" a<br>b<BR/>c <br />d ", LabelType::Text), "a\nb\nc \nd");
        assert_eq!(clean_label("say #quot;hi#quot; #35;1 #x", LabelType::Text), "say \"hi\" #1 #x");
        assert_eq!(clean_label("a\\nb", LabelType::Text), "a\nb");
        assert_eq!(clean_label("a\n   b", LabelType::Markdown), "a\nb");
    }

    #[test]
    fn statements_split_outside_brackets() {
        let chars: Vec<char> = "A[a;b]-->|x;y|B; C".chars().collect();
        let parts: Vec<String> =
            split_statements(&chars).into_iter().map(|(s, e)| chars[s..e].iter().collect()).collect();
        assert_eq!(parts, ["A[a;b]-->|x;y|B", " C"]);
    }

    #[test]
    fn styles() {
        assert_eq!(
            parse_styles("fill:#f9f,stroke:#333,font-family:a\\,b;"),
            vec![
                ("fill".into(), "#f9f".into()),
                ("stroke".into(), "#333".into()),
                ("font-family".into(), "a,b".into())
            ]
        );
    }
}
