//! Hand-written flowchart parser.
//!
//! Mermaid uses a Jison grammar. This parser follows the same syntax for the
//! common subset but is written by hand: one statement at a time, with a
//! character cursor, so that errors carry an exact line and column.

use std::collections::HashMap;

use super::{
    Arrow, ClassDef, Direction, Edge, EdgeEnd, Flowchart, Node, NodeShape, Stroke, StyleProp,
    Subgraph,
};
use crate::error::{Error, ErrorKind};
use crate::source;
use crate::Result;

/// Parses a flowchart source (header included).
pub fn parse(src: &str) -> Result<Flowchart> {
    let lines = source::lines(src);
    let mut p = Parser::default();
    let mut header = false;
    for line in &lines {
        for (col, stmt) in split_statements(line.text) {
            let trimmed = stmt.trim();
            if trimmed.is_empty() {
                continue;
            }
            let lead = stmt.chars().take_while(|c| c.is_whitespace()).count();
            let mut cur = Cursor::new(trimmed, line.number, line.offset + col + lead);
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
    if let Some(&open) = p.stack.last() {
        let last = lines.last().map_or(0, |l| l.number);
        return Err(Error::new(
            ErrorKind::Syntax,
            last,
            1,
            format!(
                "subgraph `{}` is never closed with `end`",
                p.chart.subgraphs[open].id
            ),
        ));
    }
    Ok(p.finish())
}

/// Splits a line on `;` outside quotes and brackets. Returns (char column, text).
fn split_statements(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let (mut start_byte, mut start_col, mut col) = (0, 0, 0);
    let mut depth = 0i32;
    let mut quoted = false;
    for (b, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '[' | '(' | '{' if !quoted => depth += 1,
            ']' | ')' | '}' if !quoted => depth = (depth - 1).max(0),
            ';' if !quoted && depth == 0 => {
                out.push((start_col, &text[start_byte..b]));
                start_byte = b + 1;
                start_col = col + 1;
            }
            _ => {}
        }
        col += 1;
    }
    out.push((start_col, &text[start_byte..]));
    out
}

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

struct Cursor {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
}

impl Cursor {
    fn new(s: &str, line: usize, column: usize) -> Self {
        Self {
            chars: s.chars().collect(),
            pos: 0,
            line,
            column,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn at_end(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    fn starts_with(&self, s: &str) -> bool {
        s.chars()
            .enumerate()
            .all(|(i, c)| self.peek_at(i) == Some(c))
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.starts_with(s) {
            self.pos += s.chars().count();
            true
        } else {
            false
        }
    }

    /// Position of the next occurrence of `s` at or after the cursor.
    fn find(&self, s: &str) -> Option<usize> {
        let pat: Vec<char> = s.chars().collect();
        (self.pos..=self.chars.len().saturating_sub(pat.len()))
            .find(|&i| self.chars[i..].starts_with(&pat))
    }

    fn count_run(&mut self, c: char) -> usize {
        let start = self.pos;
        while self.peek() == Some(c) {
            self.pos += 1;
        }
        self.pos - start
    }

    fn slice(&self, from: usize, to: usize) -> String {
        self.chars[from..to].iter().collect()
    }

    fn rest(&self) -> String {
        self.slice(self.pos, self.chars.len())
    }

    fn read_word(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(|c| !c.is_whitespace()) {
            self.pos += 1;
        }
        self.slice(start, self.pos)
    }

    /// Reads a node id. `-` and `.` are allowed inside an id (`my-node`)
    /// but never where they would start a link (`A-->B`, `A-.->B`).
    fn read_id(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek() {
            let inner = (c == '-' || c == '.') && self.peek_at(1).is_some_and(is_id_char);
            if is_id_char(c) || inner {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.slice(start, self.pos)
    }

    /// Reads `"..."`, the cursor being on the opening quote.
    fn read_quoted(&mut self) -> Result<String> {
        let start = self.pos;
        self.bump();
        let content_start = self.pos;
        while let Some(c) = self.bump() {
            if c == '"' {
                return Ok(self.slice(content_start, self.pos - 1));
            }
        }
        Err(self.err_at(start, "unterminated string, expected a closing `\"`"))
    }

    fn err(&self, msg: impl Into<String>) -> Error {
        self.err_at(self.pos, msg)
    }

    fn err_at(&self, pos: usize, msg: impl Into<String>) -> Error {
        Error::new(ErrorKind::Syntax, self.line, self.column + pos + 1, msg)
    }
}

/// Shape openers, longest first, with their possible closers.
const SHAPES: &[(&str, &[(&str, NodeShape)])] = &[
    ("(((", &[(")))", NodeShape::DoubleCircle)]),
    ("((", &[("))", NodeShape::Circle)]),
    ("([", &[("])", NodeShape::Stadium)]),
    ("[[", &[("]]", NodeShape::Subroutine)]),
    ("[(", &[(")]", NodeShape::Cylinder)]),
    (
        "[/",
        &[
            ("/]", NodeShape::Parallelogram),
            ("\\]", NodeShape::Trapezoid),
        ],
    ),
    (
        "[\\",
        &[
            ("\\]", NodeShape::ParallelogramAlt),
            ("/]", NodeShape::TrapezoidAlt),
        ],
    ),
    ("{{", &[("}}", NodeShape::Hexagon)]),
    ("(", &[(")", NodeShape::Round)]),
    ("[", &[("]", NodeShape::Rect)]),
    ("{", &[("}", NodeShape::Diamond)]),
    (">", &[("]", NodeShape::Asymmetric)]),
];

struct Link {
    stroke: Stroke,
    start: Arrow,
    end: Arrow,
    length: usize,
    label: Option<String>,
}

#[derive(Default)]
struct Parser {
    chart: Flowchart,
    index: HashMap<String, usize>,
    /// Open subgraphs, innermost last.
    stack: Vec<usize>,
    /// `class` statements, applied at the end once every node is known.
    pending_classes: Vec<(String, String)>,
    /// `style` statements, applied at the end.
    pending_styles: Vec<(String, Vec<StyleProp>)>,
}

impl Parser {
    fn header(&mut self, cur: &mut Cursor) -> Result<()> {
        let keyword = cur.read_word();
        if !matches!(keyword.as_str(), "flowchart" | "graph" | "flowchart-elk") {
            return Err(cur.err_at(
                0,
                format!("expected `flowchart` or `graph`, found `{keyword}`"),
            ));
        }
        cur.skip_ws();
        if !cur.at_end() {
            let start = cur.pos;
            let dir = cur.read_word();
            self.chart.direction = Direction::from_keyword(&dir).ok_or_else(|| {
                cur.err_at(
                    start,
                    format!("unknown direction `{dir}`, expected TB, TD, BT, LR or RL"),
                )
            })?;
        }
        Ok(())
    }

    fn statement(&mut self, cur: &mut Cursor) -> Result<()> {
        let start = cur.pos;
        let word = cur.read_word();
        match word.as_str() {
            "subgraph" => return self.subgraph(cur),
            "end"
                if {
                    cur.skip_ws();
                    cur.at_end()
                } =>
            {
                self.stack
                    .pop()
                    .ok_or_else(|| cur.err_at(start, "`end` without a matching `subgraph`"))?;
                return Ok(());
            }
            "direction" => {
                cur.skip_ws();
                let at = cur.pos;
                let d = cur.read_word();
                let d = Direction::from_keyword(&d)
                    .ok_or_else(|| cur.err_at(at, format!("unknown direction `{d}`")))?;
                match self.stack.last() {
                    Some(&sg) => self.chart.subgraphs[sg].direction = Some(d),
                    None => self.chart.direction = d,
                }
                return Ok(());
            }
            "classDef" => return self.class_def(cur),
            "class" => return self.class_statement(cur),
            "style" => return self.style_statement(cur),
            // Parsed elsewhere in Mermaid, ignored for now.
            "linkStyle" | "click" | "accTitle:" | "accDescr:" | "accTitle" | "accDescr" => {
                return Ok(())
            }
            _ => {}
        }
        cur.pos = start;
        self.chain(cur)
    }

    fn subgraph(&mut self, cur: &mut Cursor) -> Result<()> {
        cur.skip_ws();
        let auto_id = format!("subGraph{}", self.chart.subgraphs.len());
        let (id, title) = if cur.at_end() {
            (auto_id, String::new())
        } else if cur.peek() == Some('"') {
            (auto_id, cur.read_quoted()?)
        } else {
            let start = cur.pos;
            let id = cur.read_id();
            cur.skip_ws();
            if !id.is_empty() && cur.at_end() {
                (id.clone(), id)
            } else if !id.is_empty() && cur.peek() == Some('[') {
                let open = cur.pos;
                cur.bump();
                let text = if cur.peek() == Some('"') {
                    cur.read_quoted()?
                } else {
                    let end = cur
                        .find("]")
                        .ok_or_else(|| cur.err_at(open, "unclosed `[` in subgraph title"))?;
                    let t = cur.slice(cur.pos, end);
                    cur.pos = end;
                    t
                };
                if !cur.eat("]") {
                    return Err(cur.err("expected `]` after subgraph title"));
                }
                (id, text)
            } else {
                // `subgraph some title`: the whole text is both id and title.
                let whole = cur.slice(start, cur.chars.len()).trim().to_string();
                (whole.clone(), whole)
            }
        };
        self.chart.subgraphs.push(Subgraph {
            id,
            title: clean_label(&title),
            parent: self.stack.last().copied(),
            direction: None,
        });
        self.stack.push(self.chart.subgraphs.len() - 1);
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
                Some(def) => def.styles = styles.clone(),
                None => self.chart.class_defs.push(ClassDef {
                    name: name.to_string(),
                    styles: styles.clone(),
                }),
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
        for id in ids.split(',').filter(|i| !i.is_empty()) {
            self.pending_classes.push((id.to_string(), class.clone()));
        }
        Ok(())
    }

    fn style_statement(&mut self, cur: &mut Cursor) -> Result<()> {
        cur.skip_ws();
        let id = cur.read_word();
        if id.is_empty() {
            return Err(cur.err("expected a node id after `style`"));
        }
        cur.skip_ws();
        self.pending_styles.push((id, parse_styles(&cur.rest())));
        Ok(())
    }

    /// `A --> B & C -- text --> D`
    fn chain(&mut self, cur: &mut Cursor) -> Result<()> {
        let mut left = self.node_group(cur)?;
        loop {
            cur.skip_ws();
            if cur.at_end() {
                return Ok(());
            }
            let link = self.link(cur)?;
            let right = self.node_group(cur)?;
            for &from in &left {
                for &to in &right {
                    self.chart.edges.push(Edge {
                        from: EdgeEnd::Node(from),
                        to: EdgeEnd::Node(to),
                        label: link.label.clone(),
                        stroke: link.stroke,
                        start: link.start,
                        end: link.end,
                        length: link.length.max(1),
                    });
                }
            }
            left = right;
        }
    }

    fn node_group(&mut self, cur: &mut Cursor) -> Result<Vec<usize>> {
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

    fn node(&mut self, cur: &mut Cursor) -> Result<usize> {
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
        let idx = self.ensure_node(&id);
        let node = &mut self.chart.nodes[idx];
        if let Some((shape, label)) = shape {
            node.shape = shape;
            node.label = label;
        }
        node.classes.extend(classes);
        self.assign_subgraph(idx);
        Ok(idx)
    }

    fn shape(&mut self, cur: &mut Cursor) -> Result<Option<(NodeShape, String)>> {
        let start = cur.pos;
        for (open, closers) in SHAPES {
            if !cur.eat(open) {
                continue;
            }
            if let Some(found) = shape_text(cur, closers)? {
                return Ok(Some(found));
            }
            // No matching closer: try a shorter opener (`[/path]` is a rect).
            cur.pos = start;
        }
        Ok(None)
    }

    fn link(&mut self, cur: &mut Cursor) -> Result<Link> {
        let mut start = Arrow::None;
        match cur.peek() {
            Some('<') => {
                start = Arrow::Point;
                cur.bump();
            }
            Some(c @ ('o' | 'x')) if matches!(cur.peek_at(1), Some('-' | '=')) => {
                start = if c == 'o' {
                    Arrow::Circle
                } else {
                    Arrow::Cross
                };
                cur.bump();
            }
            _ => {}
        }
        let (stroke, length, mut label, end) = match cur.peek() {
            Some('-') if cur.peek_at(1) == Some('.') => dotted_link(cur)?,
            Some('-') => solid_link(cur, '-')?,
            Some('=') => solid_link(cur, '=')?,
            Some('~') => {
                let n = cur.count_run('~');
                if n < 3 {
                    return Err(cur.err("invisible links are written `~~~`"));
                }
                (Stroke::Invisible, n - 2, None, Arrow::None)
            }
            Some(c) => return Err(cur.err(format!("expected a link such as `-->`, found `{c}`"))),
            None => return Err(cur.err("expected a link such as `-->`")),
        };
        let save = cur.pos;
        cur.skip_ws();
        if cur.peek() == Some('|') {
            let open = cur.pos;
            cur.bump();
            let end_pos = cur
                .find("|")
                .ok_or_else(|| cur.err_at(open, "unterminated `|` link label"))?;
            label = Some(clean_label(&cur.slice(cur.pos, end_pos)));
            cur.pos = end_pos + 1;
        } else {
            cur.pos = save;
        }
        Ok(Link {
            stroke,
            start,
            end,
            length,
            label: label.filter(|l| !l.is_empty()),
        })
    }

    fn ensure_node(&mut self, id: &str) -> usize {
        if let Some(&i) = self.index.get(id) {
            return i;
        }
        self.chart.nodes.push(Node {
            id: id.to_string(),
            label: id.to_string(),
            ..Node::default()
        });
        let i = self.chart.nodes.len() - 1;
        self.index.insert(id.to_string(), i);
        i
    }

    /// A node belongs to the subgraph where it was last mentioned, except
    /// that mentioning it again in an ancestor keeps the deeper subgraph.
    /// (Guess at Mermaid's behaviour; to check against its test-suite.)
    fn assign_subgraph(&mut self, idx: usize) {
        let Some(&current) = self.stack.last() else {
            return;
        };
        match self.chart.nodes[idx].subgraph {
            Some(existing) if existing == current || self.is_ancestor(current, existing) => {}
            _ => self.chart.nodes[idx].subgraph = Some(current),
        }
    }

    fn is_ancestor(&self, ancestor: usize, mut sg: usize) -> bool {
        while let Some(parent) = self.chart.subgraphs[sg].parent {
            if parent == ancestor {
                return true;
            }
            sg = parent;
        }
        false
    }

    fn finish(mut self) -> Flowchart {
        for (id, class) in self.pending_classes.drain(..) {
            if let Some(&i) = self.index.get(&id) {
                self.chart.nodes[i].classes.push(class);
            }
        }
        for (id, styles) in self.pending_styles.drain(..) {
            if let Some(&i) = self.index.get(&id) {
                self.chart.nodes[i].styles.extend(styles);
            }
        }
        self.link_subgraphs();
        self.chart
    }

    /// A bare node whose id is a subgraph id (`A --> sub1`) is a link to the
    /// subgraph itself: retarget its links and drop the node.
    fn link_subgraphs(&mut self) {
        let is_ref = |n: &Node, sgs: &[Subgraph]| {
            n.shape == NodeShape::Rect
                && n.label == n.id
                && n.classes.is_empty()
                && n.styles.is_empty()
                && sgs.iter().any(|s| s.id == n.id)
        };
        if !self
            .chart
            .nodes
            .iter()
            .any(|n| is_ref(n, &self.chart.subgraphs))
        {
            return;
        }
        let mut remap: Vec<EdgeEnd> = Vec::with_capacity(self.chart.nodes.len());
        let mut kept = Vec::new();
        for n in std::mem::take(&mut self.chart.nodes) {
            if is_ref(&n, &self.chart.subgraphs) {
                let sg = self
                    .chart
                    .subgraphs
                    .iter()
                    .position(|s| s.id == n.id)
                    .unwrap_or(0);
                remap.push(EdgeEnd::Subgraph(sg));
            } else {
                remap.push(EdgeEnd::Node(kept.len()));
                kept.push(n);
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
        self.index = self
            .chart
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), i))
            .collect();
    }
}

/// Reads a node label up to one of `closers`. `None` means no closer was found.
fn shape_text(
    cur: &mut Cursor,
    closers: &[(&str, NodeShape)],
) -> Result<Option<(NodeShape, String)>> {
    if cur.peek() == Some('"') {
        let text = cur.read_quoted()?;
        for (closer, shape) in closers {
            if cur.eat(closer) {
                return Ok(Some((*shape, clean_label(&text))));
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
        (shape, clean_label(&text))
    }))
}

/// Marker right after the line of a link: `>`, `o` or `x`. Like Mermaid,
/// `A--oB` is a circle end towards `B`; write `A--- oB` to link to `oB`.
fn link_head(cur: &mut Cursor) -> Option<Arrow> {
    match cur.peek() {
        Some('>') => {
            cur.bump();
            Some(Arrow::Point)
        }
        Some(c @ ('o' | 'x')) => {
            cur.bump();
            Some(if c == 'o' {
                Arrow::Circle
            } else {
                Arrow::Cross
            })
        }
        _ => None,
    }
}

/// `-->`, `---`, `-- text -->`, and the thick `==>` variants.
fn solid_link(cur: &mut Cursor, ch: char) -> Result<(Stroke, usize, Option<String>, Arrow)> {
    let stroke = if ch == '-' {
        Stroke::Normal
    } else {
        Stroke::Thick
    };
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
    // Text form: `-- text -->`.
    let pattern: String = [ch, ch].iter().collect();
    let text_start = cur.pos;
    let end = cur.find(&pattern).ok_or_else(|| {
        cur.err_at(
            start,
            format!("unterminated link text, expected `{pattern}>` after it"),
        )
    })?;
    let text = cur.slice(text_start, end);
    cur.pos = end;
    let m = cur.count_run(ch);
    let head = link_head(cur);
    let length = if head.is_some() {
        m - 1
    } else {
        m.saturating_sub(2)
    };
    Ok((
        stroke,
        length.max(1),
        Some(clean_label(&text)),
        head.unwrap_or(Arrow::None),
    ))
}

/// `-.->`, `-.-`, `-..->`, `-. text .->`.
fn dotted_link(cur: &mut Cursor) -> Result<(Stroke, usize, Option<String>, Arrow)> {
    let start = cur.pos;
    cur.bump();
    let dots = cur.count_run('.');
    if cur.eat("-") {
        let head = link_head(cur).unwrap_or(Arrow::None);
        return Ok((Stroke::Dotted, dots, None, head));
    }
    let text_start = cur.pos;
    let end = cur.find(".-").ok_or_else(|| {
        cur.err_at(
            start,
            "unterminated dotted link text, expected `.->` after it",
        )
    })?;
    let text = cur.slice(text_start, end);
    cur.pos = end;
    let dots = cur.count_run('.');
    cur.bump();
    let head = link_head(cur).unwrap_or(Arrow::None);
    Ok((Stroke::Dotted, dots.max(1), Some(clean_label(&text)), head))
}

/// `fill:#f9f,stroke:#333,stroke-width:4px` (commas inside parentheses are kept).
fn parse_styles(s: &str) -> Vec<StyleProp> {
    let s = s.trim().trim_end_matches(';');
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
        .into_iter()
        .filter_map(|p| p.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .filter(|(k, v)| !k.is_empty() && !v.is_empty())
        .collect()
}

/// Normalizes a label: trims, turns `<br>` into `\n`, decodes Mermaid
/// entities (`#quot;`, `#35;`).
pub(crate) fn clean_label(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s.trim();
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(i) = lower.find("<br") else {
            out.push_str(rest);
            break;
        };
        match lower[i..].find('>') {
            Some(j)
                if lower[i + 3..i + j]
                    .trim()
                    .trim_end_matches('/')
                    .trim()
                    .is_empty() =>
            {
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
    let decoded = decode_entities(&out);
    decoded
        .split('\n')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('#') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let entity = after
            .find(';')
            .filter(|&j| j > 0 && j <= 8)
            .map(|j| &after[..j]);
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
        assert_eq!(clean_label(" a<br>b<BR/>c <br />d "), "a\nb\nc\nd");
        assert_eq!(
            clean_label("say #quot;hi#quot; #35;1 #x"),
            "say \"hi\" #1 #x"
        );
    }

    #[test]
    fn statements_split_outside_brackets() {
        let parts: Vec<_> = split_statements("A[a;b]-->B; C")
            .into_iter()
            .map(|p| p.1)
            .collect();
        assert_eq!(parts, ["A[a;b]-->B", " C"]);
    }

    #[test]
    fn styles() {
        assert_eq!(
            parse_styles("fill:rgb(1,2,3),stroke:#333;"),
            vec![
                ("fill".into(), "rgb(1,2,3)".into()),
                ("stroke".into(), "#333".into())
            ]
        );
    }
}
