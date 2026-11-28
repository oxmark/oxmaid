//! Source splitting shared by all diagram types.

/// A meaningful source line (not blank, not a comment, not front matter).
pub(crate) struct Line<'a> {
    /// Line number, starting at 1.
    pub number: usize,
    /// Text without leading whitespace. Trailing whitespace is kept: Mermaid
    /// gives `subgraph one ` (with a trailing space) a generated id.
    pub text: &'a str,
    /// Column (in chars, from 0) where `text` starts.
    pub offset: usize,
}

/// A source split into its parts.
pub(crate) struct Source<'a> {
    /// YAML front matter between the leading `---` lines, if any.
    pub front_matter: Option<String>,
    /// Bodies of `%%{ ... }%%` directives.
    pub directives: Vec<&'a str>,
    /// Meaningful lines.
    pub lines: Vec<Line<'a>>,
}

/// Splits the source: YAML front matter, `%%{...}%%` directives, and the
/// meaningful lines (`%%` comments and blank lines are dropped).
pub(crate) fn split(src: &str) -> Source<'_> {
    let mut out = Source {
        front_matter: None,
        directives: Vec::new(),
        lines: Vec::new(),
    };
    let mut seen_content = false;
    let mut front: Option<Vec<&str>> = None;
    for (i, raw) in src.lines().enumerate() {
        let text = raw.trim();
        if let Some(fm) = front.as_mut() {
            if text == "---" {
                out.front_matter = Some(fm.join("\n"));
                front = None;
            } else {
                fm.push(raw);
            }
            continue;
        }
        if text.is_empty() {
            continue;
        }
        if !seen_content && text == "---" {
            seen_content = true;
            front = Some(Vec::new());
            continue;
        }
        seen_content = true;
        if let Some(body) = text.strip_prefix("%%{").and_then(|t| t.strip_suffix("}%%")) {
            out.directives.push(body);
            continue;
        }
        if text.starts_with("%%") {
            continue;
        }
        let lead = raw.len() - raw.trim_start().len();
        out.lines.push(Line {
            number: i + 1,
            text: raw.trim_start(),
            offset: raw[..lead].chars().count(),
        });
    }
    out
}

/// Meaningful lines only.
pub(crate) fn lines(src: &str) -> Vec<Line<'_>> {
    split(src).lines
}

/// Source text prepared the way Mermaid hands it to a Jison parser: CRLF
/// normalized, front matter and `%%{...}%%` directives removed, whole-line
/// `%%` comments dropped, leading whitespace trimmed, `#name;` entities
/// encoded, and a final newline added. Each char keeps its position in the
/// original source for error reporting.
pub(crate) struct Prepared {
    pub text: Vec<char>,
    /// Original char index of each char of `text` (one past the end for the
    /// added newline).
    pub origin: Vec<usize>,
    /// Line starts (char indexes) of the original source.
    line_starts: Vec<usize>,
}

/// Marks the start of a named entity (`ﬂ°name¶ß` in Mermaid).
pub(crate) const ENTITY_OPEN: &str = "\u{FB02}\u{B0}";
/// Marks the start of a numeric entity.
pub(crate) const ENTITY_NUM_OPEN: &str = "\u{FB02}\u{B0}\u{B0}";
/// Marks the end of an entity.
pub(crate) const ENTITY_CLOSE: &str = "\u{B6}\u{DF}";

impl Prepared {
    pub fn new(src: &str) -> Self {
        let raw: Vec<char> = src.chars().collect();
        let mut line_starts = vec![0];
        for (i, c) in raw.iter().enumerate() {
            if *c == '\n' {
                line_starts.push(i + 1);
            }
        }
        // CRLF / CR to LF.
        let mut cs: Vec<(char, usize)> = Vec::with_capacity(raw.len() + 1);
        let mut i = 0;
        while i < raw.len() {
            if raw[i] == '\r' {
                cs.push(('\n', i));
                if raw.get(i + 1) == Some(&'\n') {
                    i += 1;
                }
            } else {
                cs.push((raw[i], i));
            }
            i += 1;
        }
        let cs = requote_attributes(cs);
        let cs = strip_front_matter(cs);
        let cs = remove_directives(cs);
        let cs = cleanup_comments(cs);
        let mut cs = encode_entities(cs);
        cs.push(('\n', raw.len()));
        let (text, origin) = cs.into_iter().unzip();
        Prepared {
            text,
            origin,
            line_starts,
        }
    }

    /// Line and column (both from 1) in the original source of `text[i]`.
    pub fn line_col(&self, i: usize) -> (usize, usize) {
        let o = self
            .origin
            .get(i)
            .copied()
            .unwrap_or_else(|| self.origin.last().copied().unwrap_or(0));
        let line = self.line_starts.partition_point(|&s| s <= o);
        (line, o - self.line_starts[line - 1] + 1)
    }
}

type Chars = Vec<(char, usize)>;

/// `<tag a="x">` becomes `<tag a='x'>`, as Mermaid's `cleanupText` does.
fn requote_attributes(cs: Chars) -> Chars {
    let mut out = cs;
    let mut i = 0;
    while i < out.len() {
        if out[i].0 == '<'
            && out
                .get(i + 1)
                .is_some_and(|(c, _)| c.is_ascii_alphanumeric() || *c == '_')
        {
            if let Some(end) = out[i..].iter().position(|(c, _)| *c == '>') {
                let end = i + end;
                // Inside the tag, `="..."` becomes `='...'`.
                let mut j = i + 1;
                while j + 1 < end {
                    if out[j].0 == '=' && out[j + 1].0 == '"' {
                        if let Some(close) = out[j + 2..end].iter().position(|(c, _)| *c == '"') {
                            out[j + 1].0 = '\'';
                            out[j + 2 + close].0 = '\'';
                            j = j + 3 + close;
                            continue;
                        }
                    }
                    j += 1;
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// Removes a leading `---` ... `---` YAML block.
fn strip_front_matter(cs: Chars) -> Chars {
    let text: String = cs.iter().map(|c| c.0).collect();
    let indent_len = text
        .chars()
        .take_while(|&c| is_js_space(c) && c != '\n' && c != '\r')
        .count();
    let indent: String = text.chars().take(indent_len).collect();
    let rest: Vec<char> = text.chars().skip(indent_len).collect();
    if !rest.starts_with(&['-', '-', '-']) {
        return cs;
    }
    // Opening line: `---` then whitespace up to a newline.
    let mut i = 3;
    let mut open_end = None;
    while i < rest.len() && is_js_space(rest[i]) {
        if rest[i] == '\n' {
            open_end = Some(i);
            break;
        }
        i += 1;
    }
    let Some(open_end) = open_end else {
        return cs;
    };
    // Closing line: newline, indent, `---`, whitespace, then newlines.
    let ind: Vec<char> = indent.chars().collect();
    let mut j = open_end;
    while j < rest.len() {
        if rest[j] == '\n'
            && rest[j + 1..].starts_with(&ind)
            && rest[j + 1 + ind.len()..].starts_with(&['-', '-', '-'])
        {
            let mut k = j + 1 + ind.len() + 3;
            while k < rest.len() && is_js_space(rest[k]) && rest[k] != '\n' {
                k += 1;
            }
            if k < rest.len() && rest[k] == '\n' {
                while k < rest.len() && is_js_space(rest[k]) {
                    k += 1;
                }
                // Back up to just after the last newline of the run.
                let mut end = k;
                while end > 0 && rest[end - 1] != '\n' {
                    end -= 1;
                }
                return cs[indent_len + end..].to_vec();
            }
        }
        j += 1;
    }
    cs
}

/// Removes `%%{ ... }%%` directives.
fn remove_directives(cs: Chars) -> Chars {
    let mut out = Vec::with_capacity(cs.len());
    let mut i = 0;
    while i < cs.len() {
        if cs[i].0 == '%'
            && cs.get(i + 1).map(|c| c.0) == Some('%')
            && cs.get(i + 2).map(|c| c.0) == Some('{')
        {
            let mut j = i + 3;
            while j < cs.len() && is_js_space(cs[j].0) {
                j += 1;
            }
            if j < cs.len() && (cs[j].0.is_ascii_alphanumeric() || cs[j].0 == '_') {
                // Up to and including the next `}%%`, or to the end.
                let mut k = j;
                let mut end = cs.len();
                while k + 2 < cs.len() {
                    if cs[k].0 == '}' && cs[k + 1].0 == '%' && cs[k + 2].0 == '%' {
                        end = k + 3;
                        break;
                    }
                    k += 1;
                }
                i = end;
                continue;
            }
        }
        out.push(cs[i]);
        i += 1;
    }
    out
}

/// Mermaid's `cleanupComments`: `^\s*%%(?!{)[^\n]+\n?` lines removed, then
/// leading whitespace trimmed.
fn cleanup_comments(cs: Chars) -> Chars {
    let mut out: Chars = Vec::with_capacity(cs.len());
    let mut i = 0;
    let mut at_line_start = true;
    while i < cs.len() {
        if at_line_start {
            let mut j = i;
            while j < cs.len() && is_js_space(cs[j].0) {
                j += 1;
            }
            // `\s*` may cross lines: the match starts here and ends after the
            // `%%` line.
            if j + 2 < cs.len()
                && cs[j].0 == '%'
                && cs.get(j + 1).map(|c| c.0) == Some('%')
                && cs.get(j + 2).is_some_and(|c| c.0 != '{' && c.0 != '\n')
            {
                let mut k = j + 2;
                while k < cs.len() && cs[k].0 != '\n' {
                    k += 1;
                }
                if k < cs.len() {
                    k += 1;
                }
                i = k;
                at_line_start = true;
                continue;
            }
        }
        at_line_start = cs[i].0 == '\n';
        out.push(cs[i]);
        i += 1;
    }
    let lead = out.iter().take_while(|c| is_js_space(c.0)).count();
    out.drain(..lead);
    out
}

/// Mermaid's `encodeEntities`.
fn encode_entities(cs: Chars) -> Chars {
    let mut cs = drop_style_semicolon(cs, "style");
    cs = drop_style_semicolon(cs, "classDef");
    let mut out: Chars = Vec::with_capacity(cs.len());
    let mut i = 0;
    while i < cs.len() {
        if cs[i].0 == '#' {
            let len = cs[i + 1..]
                .iter()
                .take_while(|(c, _)| c.is_ascii_alphanumeric() || *c == '_')
                .count();
            if len > 0 && cs.get(i + 1 + len).is_some_and(|(c, _)| *c == ';') {
                let name: String = cs[i + 1..i + 1 + len].iter().map(|c| c.0).collect();
                let numeric = name.chars().all(|c| c.is_ascii_digit());
                let open = if numeric {
                    ENTITY_NUM_OPEN
                } else {
                    ENTITY_OPEN
                };
                out.extend(open.chars().map(|c| (c, cs[i].1)));
                out.extend_from_slice(&cs[i + 1..i + 1 + len]);
                out.extend(ENTITY_CLOSE.chars().map(|c| (c, cs[i + 1 + len].1)));
                i += len + 2;
                continue;
            }
        }
        out.push(cs[i]);
        i += 1;
    }
    out
}

/// `/keyword.*:\S*#.*;/g` matches lose their last char (the line's last `;`).
fn drop_style_semicolon(mut cs: Chars, keyword: &str) -> Chars {
    let kw: Vec<char> = keyword.chars().collect();
    let mut line_start = 0;
    while line_start < cs.len() {
        let line_end = cs[line_start..]
            .iter()
            .position(|c| matches!(c.0, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
            .map_or(cs.len(), |p| line_start + p);
        let line: Vec<char> = cs[line_start..line_end].iter().map(|c| c.0).collect();
        let mut remove = None;
        if let Some(s) = (0..line.len()).find(|&s| line[s..].starts_with(&kw)) {
            // A `:` after the keyword, then non-space chars up to a `#`,
            // then a `;` somewhere after.
            let last_semi = line.iter().rposition(|&c| c == ';');
            let ok = (s + kw.len()..line.len()).any(|c| {
                line[c] == ':'
                    && line[c + 1..]
                        .iter()
                        .take_while(|&&ch| !is_js_space(ch))
                        .enumerate()
                        .any(|(k, &ch)| ch == '#' && last_semi.is_some_and(|ls| ls > c + 1 + k))
            });
            if ok {
                remove = last_semi.map(|p| line_start + p);
            }
        }
        if let Some(r) = remove {
            cs.remove(r);
            line_start = line_end; // the line shrank by one; `line_end - 1` + 1
            continue;
        }
        line_start = line_end + 1;
    }
    cs
}
