//! A small model of DOMPurify, which Mermaid runs over labels
//! (`sanitizeText`): text without `<` passes as is; anything else is parsed
//! as HTML, unknown elements are unwrapped, dangerous ones dropped with their
//! content, attributes filtered, and the tree serialized back.
//!
//! It covers what diagram sources contain (inline formatting tags, stray `<`
//! and `>`, generics written with angle brackets), not the whole HTML
//! parsing algorithm.

const ALLOWED_TAGS: &[&str] = &[
    "a",
    "abbr",
    "acronym",
    "address",
    "area",
    "article",
    "aside",
    "audio",
    "b",
    "bdi",
    "bdo",
    "big",
    "blink",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "content",
    "data",
    "datalist",
    "dd",
    "decorator",
    "del",
    "details",
    "dfn",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "element",
    "em",
    "fieldset",
    "figcaption",
    "figure",
    "font",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "i",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "main",
    "map",
    "mark",
    "marquee",
    "menu",
    "menuitem",
    "meter",
    "nav",
    "nobr",
    "ol",
    "optgroup",
    "option",
    "output",
    "p",
    "picture",
    "pre",
    "progress",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "search",
    "section",
    "select",
    "shadow",
    "slot",
    "small",
    "source",
    "spacer",
    "span",
    "strike",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "track",
    "tt",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
];

const ALLOWED_ATTRS: &[&str] = &[
    "accept",
    "action",
    "align",
    "alt",
    "autocapitalize",
    "autocomplete",
    "autopictureinpicture",
    "autoplay",
    "background",
    "bgcolor",
    "border",
    "capture",
    "cellpadding",
    "cellspacing",
    "checked",
    "cite",
    "class",
    "clear",
    "color",
    "cols",
    "colspan",
    "command",
    "commandfor",
    "controls",
    "controlslist",
    "coords",
    "crossorigin",
    "datetime",
    "decoding",
    "default",
    "dir",
    "disabled",
    "disablepictureinpicture",
    "disableremoteplayback",
    "download",
    "draggable",
    "enctype",
    "enterkeyhint",
    "exportparts",
    "face",
    "for",
    "headers",
    "height",
    "hidden",
    "high",
    "href",
    "hreflang",
    "id",
    "inert",
    "inputmode",
    "integrity",
    "ismap",
    "kind",
    "label",
    "lang",
    "list",
    "loading",
    "loop",
    "low",
    "max",
    "maxlength",
    "media",
    "method",
    "min",
    "minlength",
    "multiple",
    "muted",
    "name",
    "nonce",
    "noshade",
    "novalidate",
    "nowrap",
    "open",
    "optimum",
    "part",
    "pattern",
    "placeholder",
    "playsinline",
    "popover",
    "popovertarget",
    "popovertargetaction",
    "poster",
    "preload",
    "pubdate",
    "radiogroup",
    "readonly",
    "rel",
    "required",
    "rev",
    "reversed",
    "role",
    "rows",
    "rowspan",
    "spellcheck",
    "scope",
    "selected",
    "shape",
    "size",
    "sizes",
    "slot",
    "span",
    "srclang",
    "start",
    "src",
    "srcset",
    "step",
    "style",
    "summary",
    "tabindex",
    "title",
    "translate",
    "type",
    "usemap",
    "valign",
    "value",
    "width",
    "wrap",
    "xmlns",
];

/// Attributes whose value is not checked as a URI.
const URI_SAFE_ATTRS: &[&str] = &[
    "alt",
    "class",
    "for",
    "id",
    "label",
    "name",
    "pattern",
    "placeholder",
    "role",
    "summary",
    "title",
    "value",
    "style",
    "xmlns",
];

/// Disallowed elements removed with their content.
const FORBID_CONTENTS: &[&str] = &[
    "annotation-xml",
    "audio",
    "colgroup",
    "desc",
    "foreignobject",
    "head",
    "iframe",
    "math",
    "mi",
    "mn",
    "mo",
    "ms",
    "mtext",
    "noembed",
    "noframes",
    "noscript",
    "plaintext",
    "script",
    "selectedcontent",
    "style",
    "svg",
    "template",
    "thead",
    "title",
    "video",
    "xmp",
];

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// Elements whose content is raw text up to the matching end tag.
const RAW_TEXT: &[&str] = &[
    "script",
    "style",
    "textarea",
    "title",
    "xmp",
    "iframe",
    "noembed",
    "noframes",
    "plaintext",
];

/// An open element: tag, attributes, children.
type Open = (String, Vec<(String, String)>, Vec<Node>);
/// A start tag: tag, attributes, index after it.
type StartTag = (String, Vec<(String, String)>, usize);

enum Node {
    Text(String),
    Element {
        tag: String,
        attrs: Vec<(String, String)>,
        children: Vec<Node>,
    },
}

/// `DOMPurify.sanitize(text)` with Mermaid's default configuration.
pub(crate) fn sanitize(text: &str) -> String {
    if !text.contains('<') {
        return text.to_string();
    }
    let tree = parse(text);
    let mut out = String::with_capacity(text.len());
    for n in &tree {
        serialize(n, &mut out);
    }
    out
}

fn parse(s: &str) -> Vec<Node> {
    let c: Vec<char> = s.chars().collect();
    // Open elements: (tag, attrs, children).
    let mut stack: Vec<Open> = Vec::new();
    let mut root: Vec<Node> = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    fn push(stack: &mut [Open], root: &mut Vec<Node>, n: Node) {
        match stack.last_mut() {
            Some(top) => top.2.push(n),
            None => root.push(n),
        }
    }
    let flush = |text: &mut String, stack: &mut Vec<Open>, root: &mut Vec<Node>| {
        if !text.is_empty() {
            let t = decode_refs(&std::mem::take(text));
            push(stack, root, Node::Text(t));
        }
    };
    while i < c.len() {
        if c[i] != '<' {
            text.push(c[i]);
            i += 1;
            continue;
        }
        let next = c.get(i + 1).copied();
        match next {
            Some(n) if n.is_ascii_alphabetic() => {
                // Start tag.
                let Some((tag, attrs, end)) = start_tag(&c, i + 1) else {
                    // EOF inside a tag: the tag and the rest are dropped.
                    break;
                };
                flush(&mut text, &mut stack, &mut root);
                i = end;
                if VOID.contains(&tag.as_str()) {
                    push(
                        &mut stack,
                        &mut root,
                        Node::Element {
                            tag,
                            attrs,
                            children: Vec::new(),
                        },
                    );
                } else if RAW_TEXT.contains(&tag.as_str()) {
                    let close: Vec<char> = format!("</{tag}").chars().collect();
                    let mut j = i;
                    while j < c.len()
                        && !(c[j..].len() >= close.len()
                            && c[j..j + close.len()]
                                .iter()
                                .zip(&close)
                                .all(|(a, b)| a.to_ascii_lowercase() == *b))
                    {
                        j += 1;
                    }
                    let body: String = c[i..j].iter().collect();
                    let children = if body.is_empty() {
                        Vec::new()
                    } else {
                        vec![Node::Text(body)]
                    };
                    push(
                        &mut stack,
                        &mut root,
                        Node::Element {
                            tag,
                            attrs,
                            children,
                        },
                    );
                    i = c[j..]
                        .iter()
                        .position(|&x| x == '>')
                        .map_or(c.len(), |p| j + p + 1);
                } else {
                    stack.push((tag, attrs, Vec::new()));
                }
            }
            Some('/') if c.get(i + 2).is_some_and(|n| n.is_ascii_alphabetic()) => {
                let mut j = i + 2;
                while j < c.len() && !c[j].is_whitespace() && c[j] != '/' && c[j] != '>' {
                    j += 1;
                }
                let tag: String = c[i + 2..j].iter().collect::<String>().to_ascii_lowercase();
                let Some(p) = c[j..].iter().position(|&x| x == '>') else {
                    break;
                };
                flush(&mut text, &mut stack, &mut root);
                i = j + p + 1;
                if tag == "br" {
                    push(
                        &mut stack,
                        &mut root,
                        Node::Element {
                            tag,
                            attrs: Vec::new(),
                            children: Vec::new(),
                        },
                    );
                } else if let Some(k) = stack.iter().rposition(|e| e.0 == tag) {
                    while stack.len() > k {
                        let (t, a, ch) = stack.pop().unwrap();
                        push(
                            &mut stack,
                            &mut root,
                            Node::Element {
                                tag: t,
                                attrs: a,
                                children: ch,
                            },
                        );
                    }
                }
            }
            Some('!') | Some('?') | Some('/') => {
                // Comments and bogus comments are dropped.
                flush(&mut text, &mut stack, &mut root);
                if c[i..].starts_with(&['<', '!', '-', '-']) {
                    i = c[i + 4..]
                        .windows(3)
                        .position(|w| w == ['-', '-', '>'])
                        .map_or(c.len(), |p| i + 4 + p + 3);
                } else {
                    i = c[i..]
                        .iter()
                        .position(|&x| x == '>')
                        .map_or(c.len(), |p| i + p + 1);
                }
            }
            _ => {
                text.push('<');
                i += 1;
            }
        }
    }
    flush(&mut text, &mut stack, &mut root);
    while let Some((t, a, ch)) = stack.pop() {
        push(
            &mut stack,
            &mut root,
            Node::Element {
                tag: t,
                attrs: a,
                children: ch,
            },
        );
    }
    root
}

/// Parses a start tag from its name; returns the tag, its attributes and the
/// index after `>`.
fn start_tag(c: &[char], from: usize) -> Option<StartTag> {
    let mut i = from;
    while i < c.len() && !c[i].is_whitespace() && c[i] != '/' && c[i] != '>' {
        i += 1;
    }
    let tag = c[from..i].iter().collect::<String>().to_ascii_lowercase();
    let mut attrs: Vec<(String, String)> = Vec::new();
    loop {
        while i < c.len() && (c[i].is_whitespace() || c[i] == '/') {
            i += 1;
        }
        if i >= c.len() {
            return None;
        }
        if c[i] == '>' {
            return Some((tag, attrs, i + 1));
        }
        let ns = i;
        i += 1;
        while i < c.len() && !c[i].is_whitespace() && !matches!(c[i], '/' | '>' | '=') {
            i += 1;
        }
        let name = c[ns..i].iter().collect::<String>().to_ascii_lowercase();
        while i < c.len() && c[i].is_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if c.get(i) == Some(&'=') {
            i += 1;
            while i < c.len() && c[i].is_whitespace() {
                i += 1;
            }
            match c.get(i) {
                Some(&q) if q == '"' || q == '\'' => {
                    let end = c[i + 1..].iter().position(|&x| x == q)?;
                    value = c[i + 1..i + 1 + end].iter().collect();
                    i += end + 2;
                }
                _ => {
                    let vs = i;
                    while i < c.len() && !c[i].is_whitespace() && c[i] != '>' {
                        i += 1;
                    }
                    value = c[vs..i].iter().collect();
                }
            }
        }
        if !attrs.iter().any(|(n, _)| *n == name) {
            attrs.push((name, decode_refs(&value)));
        }
    }
}

/// Decodes the character references of HTML text.
fn decode_refs(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let body = &rest[1..];
        let end = body.find(';');
        let decoded = end.and_then(|e| {
            let name = &body[..e];
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
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
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

fn uri_allowed(value: &str) -> bool {
    // IS_ALLOWED_URI, on the value without ASCII whitespace and controls.
    let v: String = value
        .chars()
        .filter(|c| !matches!(c, '\u{0}'..='\u{20}' | '\u{a0}' | '\u{1680}' | '\u{180e}' | '\u{2000}'..='\u{2029}' | '\u{205f}' | '\u{3000}'))
        .collect::<String>()
        .to_ascii_lowercase();
    const SCHEMES: &[&str] = &[
        "http:", "https:", "ftp:", "ftps:", "mailto:", "tel:", "callto:", "sms:", "cid:", "xmpp:",
        "matrix:",
    ];
    if SCHEMES.iter().any(|s| v.starts_with(s)) {
        return true;
    }
    match v.chars().next() {
        None => true,
        Some(c) if !c.is_ascii_lowercase() => true,
        _ => {
            // `[a-z+.\-]+(?:[^a-z+.\-:]|$)`: no scheme before the first
            // other character.
            let n = v
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || matches!(c, '+' | '.' | '-'))
                .count();
            !v[n..].starts_with(':')
        }
    }
}

fn attr_allowed(tag: &str, name: &str, value: &str) -> bool {
    if let Some(rest) = name.strip_prefix("data-") {
        return !rest.is_empty();
    }
    if let Some(rest) = name.strip_prefix("aria-") {
        return !rest.is_empty()
            && rest
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    }
    if tag == "a" && name == "target" {
        // Mermaid's hook keeps link targets.
        return true;
    }
    if !ALLOWED_ATTRS.contains(&name) {
        return false;
    }
    URI_SAFE_ATTRS.contains(&name) || uri_allowed(value) || value.is_empty()
}

fn serialize(n: &Node, out: &mut String) {
    match n {
        Node::Text(t) => escape(t, out, false),
        Node::Element {
            tag,
            attrs,
            children,
        } => {
            if !ALLOWED_TAGS.contains(&tag.as_str()) {
                if FORBID_CONTENTS.contains(&tag.as_str()) || tag == "script" {
                    return;
                }
                for ch in children {
                    serialize(ch, out);
                }
                return;
            }
            out.push('<');
            out.push_str(tag);
            for (k, v) in attrs {
                if attr_allowed(tag, k, v) {
                    out.push(' ');
                    out.push_str(k);
                    out.push_str("=\"");
                    escape(v, out, true);
                    out.push('"');
                }
            }
            if tag == "a"
                && attrs.iter().any(|(k, v)| k == "target" && v == "_blank")
                && !attrs.iter().any(|(k, _)| k == "rel")
            {
                out.push_str(" rel=\"noopener\"");
            }
            out.push('>');
            if VOID.contains(&tag.as_str()) {
                return;
            }
            for ch in children {
                serialize(ch, out);
            }
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        }
    }
}

fn escape(s: &str, out: &mut String, attr: bool) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '"' if attr => out.push_str("&quot;"),
            '<' if !attr => out.push_str("&lt;"),
            '>' if !attr => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize;

    #[test]
    fn matches_dompurify() {
        assert_eq!(sanitize("a & b"), "a & b");
        assert_eq!(sanitize("<<x>>"), "&lt;&gt;");
        assert_eq!(sanitize("a<int> b"), "a b");
        assert_eq!(sanitize("x < y"), "x &lt; y");
        assert_eq!(sanitize("<i>i"), "<i>i</i>");
        assert_eq!(sanitize("a<br/>b"), "a<br>b");
        assert_eq!(
            sanitize("<span style='color:red'>s</span>"),
            "<span style=\"color:red\">s</span>"
        );
        assert_eq!(sanitize("<img src=x onerror=1>"), "<img src=\"x\">");
        assert_eq!(sanitize("<x y>"), "");
    }
}
