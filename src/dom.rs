//! A tiny SVG document model that mirrors what Mermaid does with d3:
//! append, `lower()` (move to first child), attributes, inline styles and text,
//! then serialization the way a browser's `innerHTML` writes it. Renderers that
//! follow Mermaid step by step use it to produce the same element order.

/// Index of a node in a [`Dom`].
pub(crate) type NodeId = usize;

struct Node {
    tag: &'static str,
    attrs: Vec<(&'static str, String)>,
    style: Vec<(&'static str, String)>,
    text: Option<String>,
    children: Vec<NodeId>,
    parent: Option<NodeId>,
}

/// An element tree, rooted at node 0.
pub(crate) struct Dom {
    nodes: Vec<Node>,
}

impl Dom {
    pub fn new(root: &'static str) -> Self {
        Dom {
            nodes: vec![Node {
                tag: root,
                attrs: Vec::new(),
                style: Vec::new(),
                text: None,
                children: Vec::new(),
                parent: None,
            }],
        }
    }

    pub const ROOT: NodeId = 0;

    fn create(&mut self, parent: NodeId, tag: &'static str) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            tag,
            attrs: Vec::new(),
            style: Vec::new(),
            text: None,
            children: Vec::new(),
            parent: Some(parent),
        });
        id
    }

    /// Appends a new element as the last child of `parent`.
    pub fn append(&mut self, parent: NodeId, tag: &'static str) -> NodeId {
        let id = self.create(parent, tag);
        self.nodes[parent].children.push(id);
        id
    }

    /// Inserts a new element as the first child of `parent`.
    pub fn prepend(&mut self, parent: NodeId, tag: &'static str) -> NodeId {
        let id = self.create(parent, tag);
        self.nodes[parent].children.insert(0, id);
        id
    }

    /// d3's `lower()`: moves the element to the front of its parent.
    pub fn lower(&mut self, id: NodeId) {
        if let Some(p) = self.nodes[id].parent {
            let children = &mut self.nodes[p].children;
            if let Some(i) = children.iter().position(|&c| c == id) {
                children.remove(i);
                children.insert(0, id);
            }
        }
    }

    /// Sets an attribute, keeping its position if it already exists.
    pub fn attr(&mut self, id: NodeId, name: &'static str, value: impl Into<String>) -> &mut Self {
        let value = value.into();
        let attrs = &mut self.nodes[id].attrs;
        match attrs.iter_mut().find(|(k, _)| *k == name) {
            Some(slot) => slot.1 = value,
            None => attrs.push((name, value)),
        }
        self
    }

    /// Sets a numeric attribute, formatted like JavaScript's `String(n)`.
    pub fn num(&mut self, id: NodeId, name: &'static str, value: f64) -> &mut Self {
        self.attr(id, name, js_num(value))
    }

    /// Reads an attribute.
    pub fn get_attr(&self, id: NodeId, name: &str) -> Option<&str> {
        self.nodes[id]
            .attrs
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Sets an inline style property. Colors are normalized to `rgb(...)`,
    /// as the browser's CSS object model does.
    pub fn style(&mut self, id: NodeId, name: &'static str, value: &str) -> &mut Self {
        let value = normalize_color(value).unwrap_or_else(|| value.to_string());
        let style = &mut self.nodes[id].style;
        match style.iter_mut().find(|(k, _)| *k == name) {
            Some(slot) => slot.1 = value,
            None => style.push((name, value)),
        }
        self
    }

    /// Sets the text content (children are dropped).
    pub fn text(&mut self, id: NodeId, text: impl Into<String>) -> &mut Self {
        self.nodes[id].children.clear();
        self.nodes[id].text = Some(text.into());
        self
    }

    /// Serializes `id` and its subtree like `outerHTML`.
    pub fn serialize(&self, id: NodeId, out: &mut String) {
        let node = &self.nodes[id];
        out.push('<');
        out.push_str(node.tag);
        for (k, v) in &node.attrs {
            out.push(' ');
            out.push_str(k);
            out.push_str("=\"");
            escape_attr(v, out);
            out.push('"');
        }
        if !node.style.is_empty() {
            out.push_str(" style=\"");
            for (i, (k, v)) in node.style.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                out.push_str(k);
                out.push_str(": ");
                escape_attr(v, out);
                out.push(';');
            }
            out.push('"');
        }
        out.push('>');
        if let Some(t) = &node.text {
            escape_text(t, out);
        }
        for &c in &node.children {
            self.serialize(c, out);
        }
        out.push_str("</");
        out.push_str(node.tag);
        out.push('>');
    }
}

fn escape_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            _ => out.push(c),
        }
    }
}

pub(crate) fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            _ => out.push(c),
        }
    }
}

/// Formats a number like JavaScript's `String(n)`.
pub(crate) fn js_num(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if v == 0.0 {
        return "0".into();
    }
    let a = v.abs();
    if (1e-6..1e21).contains(&a) {
        return format!("{v}");
    }
    // Exponent form: `1e-7`, `1.5e+21`.
    let s = format!("{v:e}");
    let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    if exp >= 0 {
        format!("{mant}e+{exp}")
    } else {
        format!("{mant}e{exp}")
    }
}

/// Converts a hex or named color to the `rgb(r, g, b)` / `rgba(...)` form the
/// browser serializes inline styles with. Returns `None` for other values.
pub(crate) fn normalize_color(v: &str) -> Option<String> {
    let v = v.trim();
    if let Some(hex) = v.strip_prefix('#') {
        let digits: Vec<u8> = hex
            .chars()
            .map(|c| c.to_digit(16).map(|d| d as u8))
            .collect::<Option<_>>()?;
        let (r, g, b, a) = match digits.len() {
            3 => (digits[0] * 17, digits[1] * 17, digits[2] * 17, None),
            4 => (digits[0] * 17, digits[1] * 17, digits[2] * 17, Some(digits[3] * 17)),
            6 => (
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
                None,
            ),
            8 => (
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
                Some(digits[6] * 16 + digits[7]),
            ),
            _ => return None,
        };
        return Some(match a {
            None | Some(255) => format!("rgb({r}, {g}, {b})"),
            Some(a) => format!("rgba({r}, {g}, {b}, {})", alpha(a)),
        });
    }
    None
}

/// Alpha as Chromium serializes an 8-bit alpha: the shortest decimal that
/// maps back to the same byte.
fn alpha(a: u8) -> String {
    for digits in 1..=3 {
        let scale = 10f64.powi(digits);
        let v = (f64::from(a) / 255.0 * scale).round() / scale;
        if (v * 255.0).round() as u8 == a {
            return js_num(v);
        }
    }
    js_num(f64::from(a) / 255.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(js_num(75.0), "75");
        assert_eq!(js_num(-0.0), "0");
        assert_eq!(js_num(20.307692307692307), "20.307692307692307");
        assert_eq!(js_num(1e-7), "1e-7");
        assert_eq!(js_num(0.5), "0.5");
    }

    #[test]
    fn colors() {
        assert_eq!(normalize_color("#E879F9").unwrap(), "rgb(232, 121, 249)");
        assert_eq!(normalize_color("#fff").unwrap(), "rgb(255, 255, 255)");
        assert!(normalize_color("none").is_none());
    }

    #[test]
    fn tree() {
        let mut d = Dom::new("svg");
        let a = d.append(Dom::ROOT, "g");
        let b = d.append(Dom::ROOT, "rect");
        d.num(b, "x", 1.5).style(b, "fill", "#000");
        d.lower(b);
        d.text(a, "a<b");
        let mut s = String::new();
        d.serialize(Dom::ROOT, &mut s);
        assert_eq!(
            s,
            "<svg><rect x=\"1.5\" style=\"fill: rgb(0, 0, 0);\"></rect><g>a&lt;b</g></svg>"
        );
    }
}
