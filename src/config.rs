//! Diagram configuration from YAML front matter and `%%{init: ...}%%` directives.
//!
//! Only a small subset of YAML and JSON is understood: nested mappings,
//! strings, numbers and booleans, which is what Mermaid configs use.

use std::collections::BTreeMap;

/// A configuration value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Num(f64),
    Bool(bool),
    Map(BTreeMap<String, Value>),
    List(Vec<Value>),
    Null,
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(m) => m.get(key),
            _ => None,
        }
    }

    /// Follows a dotted path such as `flowchart.curve`.
    pub fn path(&self, path: &str) -> Option<&Value> {
        path.split('.').try_fold(self, |v, k| v.get(k))
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            Value::Str(s) => s.trim_end_matches("px").parse().ok(),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::Str(s) if s == "true" => Some(true),
            Value::Str(s) if s == "false" => Some(false),
            _ => None,
        }
    }

    /// Merges `other` into `self`, `other` winning.
    fn merge(&mut self, other: Value) {
        match (self, other) {
            (Value::Map(a), Value::Map(b)) => {
                for (k, v) in b {
                    match a.get_mut(&k) {
                        Some(existing) => existing.merge(v),
                        None => {
                            a.insert(k, v);
                        }
                    }
                }
            }
            (slot, v) => *slot = v,
        }
    }
}

/// Settings a diagram can carry in its own source.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DiagramConfig {
    /// `title` from the front matter.
    pub title: Option<String>,
    /// `theme` (`redux-color` by default for flowcharts, `default` for the old look).
    pub theme: Option<String>,
    /// `look` (`neo` by default for flowcharts, `classic` for the old look).
    pub look: Option<String>,
    /// `layout` (`elk` by default for flowcharts, or `dagre`).
    pub layout: Option<String>,
    /// The whole `config` mapping, for options not mapped to a field.
    pub raw: Option<Value>,
}

impl DiagramConfig {
    /// Reads a value under `config`, preferring the diagram-scoped one
    /// (`flowchart.<key>`) over the top-level one.
    pub fn lookup(&self, diagram: &str, key: &str) -> Option<&Value> {
        let raw = self.raw.as_ref()?;
        raw.path(&format!("{diagram}.{key}")).or_else(|| raw.get(key))
    }

    pub(crate) fn from_source(front_matter: Option<&str>, directives: &[&str], diagram: &str) -> Self {
        let mut cfg = DiagramConfig::default();
        let mut raw = Value::Map(BTreeMap::new());
        if let Some(fm) = front_matter {
            let doc = parse_yaml(fm);
            cfg.title = doc.get("title").and_then(scalar_string);
            if let Some(c) = doc.get("config") {
                raw.merge(c.clone());
            }
        }
        for d in directives {
            let mut p = JsonParser { s: format!("{{{d}}}").chars().collect(), i: 0 };
            if let Some(Value::Map(m)) = p.object() {
                for (k, v) in m {
                    if k == "init" || k == "initialize" {
                        raw.merge(v);
                    }
                }
            }
        }
        let pick = |key: &str| {
            raw.path(&format!("{diagram}.{key}")).or_else(|| raw.get(key)).and_then(scalar_string)
        };
        cfg.theme = pick("theme");
        cfg.look = pick("look");
        cfg.layout = raw.get("layout").and_then(scalar_string);
        if matches!(&raw, Value::Map(m) if !m.is_empty()) {
            cfg.raw = Some(raw);
        }
        cfg
    }
}

fn scalar_string(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.clone()),
        Value::Num(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn scalar(text: &str) -> Value {
    let t = text.trim();
    let unquoted = |q: char| t.len() >= 2 && t.starts_with(q) && t.ends_with(q);
    if unquoted('"') || unquoted('\'') {
        return Value::Str(t[1..t.len() - 1].to_string());
    }
    match t {
        "" | "~" | "null" => Value::Null,
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => t.parse::<f64>().map(Value::Num).unwrap_or_else(|_| Value::Str(t.to_string())),
    }
}

/// Strips a ` # comment` outside quotes.
fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    for (i, c) in line.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '#') if i == 0 || line[..i].ends_with(' ') => return &line[..i],
            _ => {}
        }
    }
    line
}

/// Parses a YAML block mapping (indentation based). Flow mappings
/// (`{ a: 1 }`) on a value are handled by the JSON-ish parser.
pub(crate) fn parse_yaml(src: &str) -> Value {
    let lines: Vec<(usize, &str)> = src
        .lines()
        .map(strip_comment)
        .filter(|l| !l.trim().is_empty())
        .map(|l| (l.len() - l.trim_start().len(), l.trim()))
        .collect();
    let mut i = 0;
    yaml_block(&lines, &mut i, 0)
}

fn yaml_block(lines: &[(usize, &str)], i: &mut usize, indent: usize) -> Value {
    let mut map = BTreeMap::new();
    let mut list = Vec::new();
    while *i < lines.len() {
        let (ind, text) = lines[*i];
        if ind < indent {
            break;
        }
        *i += 1;
        if let Some(item) = text.strip_prefix("- ") {
            list.push(scalar(item));
            continue;
        }
        let Some((key, rest)) = text.split_once(':') else { continue };
        let key = scalar(key).as_str().map(str::to_string).unwrap_or_else(|| key.trim().to_string());
        let rest = rest.trim();
        let value = if rest.is_empty() {
            match lines.get(*i) {
                Some(&(next, _)) if next > ind => yaml_block(lines, i, next),
                _ => Value::Null,
            }
        } else if rest.starts_with('{') {
            let mut p = JsonParser { s: rest.chars().collect(), i: 0 };
            p.object().unwrap_or(Value::Null)
        } else if rest == "|" || rest == ">" {
            let mut text = Vec::new();
            while let Some(&(next, t)) = lines.get(*i) {
                if next <= ind {
                    break;
                }
                text.push(t);
                *i += 1;
            }
            Value::Str(text.join(if rest == "|" { "\n" } else { " " }))
        } else {
            scalar(rest)
        };
        map.insert(key, value);
    }
    if map.is_empty() && !list.is_empty() {
        Value::List(list)
    } else {
        Value::Map(map)
    }
}

/// A tolerant JSON / JSON5 reader: single or double quotes, bare keys.
struct JsonParser {
    s: Vec<char>,
    i: usize,
}

impl JsonParser {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_whitespace() || *c == ',') {
            self.i += 1;
        }
    }

    fn value(&mut self) -> Option<Value> {
        self.ws();
        match *self.s.get(self.i)? {
            '{' => self.object(),
            '[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    if self.s.get(self.i) == Some(&']') {
                        self.i += 1;
                        return Some(Value::List(items));
                    }
                    items.push(self.value()?);
                }
            }
            '"' | '\'' => self.string().map(Value::Str),
            _ => {
                let start = self.i;
                while self.s.get(self.i).is_some_and(|c| !matches!(c, ',' | '}' | ']')) {
                    self.i += 1;
                }
                let raw: String = self.s[start..self.i].iter().collect();
                Some(scalar(&raw))
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        let q = *self.s.get(self.i)?;
        self.i += 1;
        let mut out = String::new();
        while let Some(&c) = self.s.get(self.i) {
            self.i += 1;
            match c {
                '\\' => {
                    if let Some(&n) = self.s.get(self.i) {
                        out.push(n);
                        self.i += 1;
                    }
                }
                c if c == q => return Some(out),
                c => out.push(c),
            }
        }
        None
    }

    fn object(&mut self) -> Option<Value> {
        self.ws();
        if self.s.get(self.i) != Some(&'{') {
            return None;
        }
        self.i += 1;
        let mut map = BTreeMap::new();
        loop {
            self.ws();
            match *self.s.get(self.i)? {
                '}' => {
                    self.i += 1;
                    return Some(Value::Map(map));
                }
                '"' | '\'' => {
                    let k = self.string()?;
                    self.ws();
                    if self.s.get(self.i) == Some(&':') {
                        self.i += 1;
                    }
                    map.insert(k, self.value()?);
                }
                _ => {
                    let start = self.i;
                    while self.s.get(self.i).is_some_and(|c| *c != ':' && *c != '}') {
                        self.i += 1;
                    }
                    let k: String = self.s[start..self.i].iter().collect::<String>().trim().to_string();
                    if self.s.get(self.i) == Some(&':') {
                        self.i += 1;
                    }
                    map.insert(k, self.value()?);
                }
            }
        }
    }
}

/// Parses the body of a `@{ ... }` shape-data block: a YAML flow mapping on
/// one line, or a block mapping over several lines.
pub(crate) fn parse_shape_data(body: &str) -> Value {
    if body.contains('\n') {
        parse_yaml(body)
    } else {
        let mut p = JsonParser { s: format!("{{{body}}}").chars().collect(), i: 0 };
        p.object().unwrap_or(Value::Map(BTreeMap::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_and_directive() {
        let fm = "title: Hello # c\nconfig:\n  theme: default\n  look: classic\n  layout: dagre\n  flowchart:\n    curve: linear";
        let c = DiagramConfig::from_source(Some(fm), &["init: {'flowchart': {'nodeSpacing': 20}}"], "flowchart");
        assert_eq!(c.title.as_deref(), Some("Hello"));
        assert_eq!(c.theme.as_deref(), Some("default"));
        assert_eq!(c.look.as_deref(), Some("classic"));
        assert_eq!(c.layout.as_deref(), Some("dagre"));
        assert_eq!(c.lookup("flowchart", "curve").and_then(Value::as_str), Some("linear"));
        assert_eq!(c.lookup("flowchart", "nodeSpacing").and_then(Value::as_f64), Some(20.0));
    }

    #[test]
    fn shape_data() {
        let v = parse_shape_data(r#" shape: rect, label: "a, b" "#);
        assert_eq!(v.get("shape").and_then(Value::as_str), Some("rect"));
        assert_eq!(v.get("label").and_then(Value::as_str), Some("a, b"));
        let v = parse_shape_data("\n  shape: diam\n  label: Is it?\n");
        assert_eq!(v.get("label").and_then(Value::as_str), Some("Is it?"));
    }
}
