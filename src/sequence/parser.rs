//! Sequence diagram parser.
//!
//! Mermaid 12.3 parses sequence diagrams with a Jison grammar
//! (`sequenceDiagram.jison`) whose lexer decides most of the syntax: rules are
//! tried in order and the first match wins, with lexer states for participant
//! names, aliases and "rest of line" text. This module reproduces that lexer
//! rule by rule, then parses the token stream by recursive descent following
//! the grammar, and finally applies the statements the way `sequenceDb` does.

use std::collections::HashMap;

use super::{
    Autonumber, LineType, Message, Participant, ParticipantBox, ParticipantKind, Placement,
    SequenceDiagram,
};
use crate::config::{self, DiagramConfig, Value};
use crate::error::{Error, ErrorKind};
use crate::source::{self, Prepared};
use crate::Result;

/// Parses a sequence diagram source (front matter and header included).
pub fn parse(src: &str) -> Result<SequenceDiagram> {
    let split = source::split(src);
    let config = DiagramConfig::from_source(
        split.front_matter.as_deref(),
        &split.directives,
        "sequence",
    );
    let prepared = Prepared::new(src);
    let tokens = Lexer::new(&prepared.text).tokenize().map_err(|i| {
        let (l, c) = prepared.line_col(i);
        Error::new(ErrorKind::Syntax, l, c, "unrecognized text")
    })?;
    let mut p = Parser {
        toks: &tokens,
        i: 0,
        src: &prepared,
        ops: Vec::new(),
        title: None,
        acc_title: None,
        acc_descr: None,
    };
    p.start()?;
    let wrap = config
        .raw
        .as_ref()
        .and_then(|r| r.get("wrap"))
        .and_then(Value::as_bool)
        .or_else(|| config.lookup("sequence", "wrap").and_then(Value::as_bool))
        .unwrap_or(false);
    let mut db = Db {
        auto_wrap: wrap,
        ..Db::default()
    };
    let ops = std::mem::take(&mut p.ops);
    for op in ops {
        db.apply(op).map_err(|m| {
            let (l, c) = prepared.line_col(0);
            Error::new(ErrorKind::Syntax, l, c, m)
        })?;
    }
    let title = p
        .title
        .take()
        .or_else(|| config.title.clone())
        .map(|t| sanitize_text(&t))
        .filter(|t| !t.is_empty());
    Ok(SequenceDiagram {
        participants: db.actors,
        messages: db.messages,
        boxes: db.boxes,
        created: db.created,
        destroyed: db.destroyed,
        title,
        acc_title: p
            .acc_title
            .take()
            .map(|t| sanitize_text(&t).trim_start().to_string())
            .filter(|t| !t.is_empty()),
        acc_descr: p
            .acc_descr
            .take()
            .map(|t| collapse_indent(&sanitize_text(&t)))
            .filter(|t| !t.is_empty()),
        config,
    })
}

// ---------------------------------------------------------------------------
// Lexer

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum T {
    Newline,
    Sd,
    Actor,
    Invalid,
    Num,
    ConfigStart,
    ConfigContent,
    ConfigEnd,
    Box,
    Participant,
    ParticipantActor,
    Create,
    Destroy,
    As,
    Loop,
    Rect,
    Opt,
    Alt,
    Else,
    Par,
    ParOver,
    And,
    Critical,
    OptionKw,
    Break,
    RestOfLine,
    End,
    LeftOf,
    RightOf,
    Links,
    Link,
    Properties,
    Details,
    Over,
    Note,
    Activate,
    Deactivate,
    Title,
    LegacyTitle,
    AccTitle,
    AccTitleValue,
    AccDescr,
    AccDescrValue,
    AccDescrMultilineValue,
    Autonumber,
    Off,
    Comma,
    Signal(LineType),
    Txt,
    Plus,
    Minus,
    Central,
    Eof,
}

impl T {
    fn describe(self) -> &'static str {
        match self {
            T::Newline => "end of line",
            T::Sd => "`sequenceDiagram`",
            T::Actor => "participant name",
            T::Invalid => "invalid text",
            T::Num => "number",
            T::ConfigStart => "`@{`",
            T::ConfigContent => "participant data",
            T::ConfigEnd => "`}`",
            T::Box => "`box`",
            T::Participant => "`participant`",
            T::ParticipantActor => "`actor`",
            T::Create => "`create`",
            T::Destroy => "`destroy`",
            T::As => "`as`",
            T::Loop => "`loop`",
            T::Rect => "`rect`",
            T::Opt => "`opt`",
            T::Alt => "`alt`",
            T::Else => "`else`",
            T::Par => "`par`",
            T::ParOver => "`par_over`",
            T::And => "`and`",
            T::Critical => "`critical`",
            T::OptionKw => "`option`",
            T::Break => "`break`",
            T::RestOfLine => "text",
            T::End => "`end`",
            T::LeftOf => "`left of`",
            T::RightOf => "`right of`",
            T::Links => "`links`",
            T::Link => "`link`",
            T::Properties => "`properties`",
            T::Details => "`details`",
            T::Over => "`over`",
            T::Note => "`note`",
            T::Activate => "`activate`",
            T::Deactivate => "`deactivate`",
            T::Title => "`title`",
            T::LegacyTitle => "`title:`",
            T::AccTitle => "`accTitle`",
            T::AccTitleValue => "title text",
            T::AccDescr => "`accDescr`",
            T::AccDescrValue => "description text",
            T::AccDescrMultilineValue => "description text",
            T::Autonumber => "`autonumber`",
            T::Off => "`off`",
            T::Comma => "`,`",
            T::Signal(_) => "arrow",
            T::Txt => "`:` text",
            T::Plus => "`+`",
            T::Minus => "`-`",
            T::Central => "`()`",
            T::Eof => "end of input",
        }
    }
}

#[derive(Debug, Clone)]
struct Tok {
    t: T,
    text: String,
    pos: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum S {
    Initial,
    Id,
    Alias,
    Line,
    Config,
    AccTitle,
    AccDescr,
    AccDescrMultiline,
}

struct Lexer<'a> {
    s: &'a [char],
    pos: usize,
    states: Vec<S>,
    done: bool,
}

fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{b}'
            | '\u{c}'
            | '\r'
            | ' '
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_line_end(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn js_trim(s: &str) -> String {
    s.trim_matches(is_space).to_string()
}

/// Signal arrows, in the lexer's rule order (rules 61-86).
const ARROWS: &[(&str, LineType)] = &[
    ("->>", LineType::Solid),
    ("<<->>", LineType::BidirectionalSolid),
    ("-->>", LineType::Dotted),
    ("<<-->>", LineType::BidirectionalDotted),
    ("->", LineType::SolidOpen),
    ("-->", LineType::DottedOpen),
    ("-x", LineType::SolidCross),
    ("--x", LineType::DottedCross),
    ("-)", LineType::SolidPoint),
    ("--)", LineType::DottedPoint),
    ("--|\\", LineType::SolidTopDotted),
    ("--|/", LineType::SolidBottomDotted),
    ("--\\\\", LineType::StickTopDotted),
    ("--//", LineType::StickBottomDotted),
    ("/|--", LineType::SolidArrowTopReverseDotted),
    ("\\|--", LineType::SolidArrowBottomReverseDotted),
    ("//--", LineType::StickArrowTopReverseDotted),
    ("\\\\--", LineType::StickArrowBottomReverseDotted),
    ("-|\\", LineType::SolidTop),
    ("-|/", LineType::SolidBottom),
    ("-\\\\", LineType::StickTop),
    ("-//", LineType::StickBottom),
    ("/|-", LineType::SolidArrowTopReverse),
    ("\\|-", LineType::SolidArrowBottomReverse),
    ("//-", LineType::StickArrowTopReverse),
    ("\\\\-", LineType::StickArrowBottomReverse),
];

/// What a matched rule does.
enum Act {
    Skip,
    Tok(T),
    /// Token with its text trimmed.
    Trim(T),
    Begin(S, T),
    Pop(T),
    PopTrim(T),
    PopPop(T),
    PopPopBegin(S, T),
    PopBegin(S, T),
    BeginSilent(S),
    PopSilent,
    TrimBegin(S, T),
}

impl<'a> Lexer<'a> {
    fn new(s: &'a [char]) -> Self {
        Lexer {
            s,
            pos: 0,
            states: vec![S::Initial],
            done: false,
        }
    }

    fn rest(&self) -> &[char] {
        &self.s[self.pos..]
    }

    /// Case-insensitive keyword followed by a word boundary.
    fn kw(&self, word: &str) -> Option<usize> {
        let r = self.rest();
        let n = word.chars().count();
        if r.len() < n {
            return None;
        }
        if !r.iter().zip(word.chars()).all(|(a, b)| a.eq_ignore_ascii_case(&b)) {
            return None;
        }
        let last_word = word.chars().last().is_some_and(is_word);
        let next_word = r.get(n).is_some_and(|&c| is_word(c));
        (last_word != next_word || !last_word).then_some(n)
    }

    /// Case-insensitive literal.
    fn lit(&self, word: &str) -> Option<usize> {
        let r = self.rest();
        let n = word.chars().count();
        (r.len() >= n && r.iter().zip(word.chars()).all(|(a, b)| a.eq_ignore_ascii_case(&b)))
            .then_some(n)
    }

    fn run(&self, from: usize, f: impl Fn(char) -> bool) -> usize {
        self.s[from..].iter().take_while(|&&c| f(c)).count()
    }

    fn tokenize(mut self) -> std::result::Result<Vec<Tok>, usize> {
        let mut out = Vec::new();
        loop {
            let t = self.next()?;
            let eof = t.t == T::Eof;
            out.push(t);
            if eof {
                return Ok(out);
            }
        }
    }

    fn next(&mut self) -> std::result::Result<Tok, usize> {
        loop {
            if self.done {
                return Ok(Tok {
                    t: T::Eof,
                    text: String::new(),
                    pos: self.pos,
                });
            }
            if self.pos >= self.s.len() {
                self.done = true;
            }
            let state = *self.states.last().unwrap_or(&S::Initial);
            let Some((len, act)) = self.match_rule(state) else {
                if self.pos >= self.s.len() {
                    return Ok(Tok {
                        t: T::Eof,
                        text: String::new(),
                        pos: self.pos,
                    });
                }
                return Err(self.pos);
            };
            let start = self.pos;
            let text: String = self.s[start..start + len].iter().collect();
            self.pos += len;
            let tok = |t: T, text: String| Tok { t, text, pos: start };
            match act {
                Act::Skip => continue,
                Act::Tok(t) => return Ok(tok(t, text)),
                Act::Trim(t) => return Ok(tok(t, js_trim(&text))),
                Act::Begin(s, t) => {
                    self.states.push(s);
                    return Ok(tok(t, text));
                }
                Act::TrimBegin(s, t) => {
                    self.states.push(s);
                    return Ok(tok(t, js_trim(&text)));
                }
                Act::Pop(t) => {
                    self.pop();
                    return Ok(tok(t, text));
                }
                Act::PopTrim(t) => {
                    self.pop();
                    return Ok(tok(t, js_trim(&text)));
                }
                Act::PopPop(t) => {
                    self.pop();
                    self.pop();
                    return Ok(tok(t, text));
                }
                Act::PopPopBegin(s, t) => {
                    self.pop();
                    self.pop();
                    self.states.push(s);
                    return Ok(tok(t, text));
                }
                Act::PopBegin(s, t) => {
                    self.pop();
                    self.states.push(s);
                    return Ok(tok(t, text));
                }
                Act::BeginSilent(s) => {
                    self.states.push(s);
                    continue;
                }
                Act::PopSilent => {
                    self.pop();
                    continue;
                }
            }
        }
    }

    fn pop(&mut self) {
        if self.states.len() > 1 {
            self.states.pop();
        }
    }

    fn match_rule(&self, state: S) -> Option<(usize, Act)> {
        let r = self.rest();
        let p = self.pos;
        match state {
            S::Initial => self.initial(),
            S::Id => {
                // 2: same-line whitespace
                let n = self.run(p, |c| is_space(c) && c != '\n');
                if n > 0 {
                    return Some((n, Act::Skip));
                }
                // 3: `#` comment
                if r.first() == Some(&'#') {
                    return Some((self.run(p, |c| c != '\n'), Act::Skip));
                }
                // 7: `@{`
                if r.starts_with(&['@', '{']) {
                    return Some((2, Act::Begin(S::Config, T::ConfigStart)));
                }
                let name = |c: char| !matches!(c, '<' | '>' | ':' | '\n' | ',' | ';' | '@');
                // 11: name before `@{`
                let n = self.run(p, |c| name(c) && !is_space(c));
                if n > 0 {
                    let mut k = p + n;
                    while k < self.s.len() && is_space(self.s[k]) {
                        k += 1;
                    }
                    if self.s[k..].starts_with(&['@', '{']) {
                        return Some((n, Act::Trim(T::Actor)));
                    }
                    // 12: name before ` as `
                    let ws = self.run(p + n, is_space);
                    if ws > 0 {
                        let a = p + n + ws;
                        if self.s.len() > a + 2
                            && self.s[a].eq_ignore_ascii_case(&'a')
                            && self.s[a + 1].eq_ignore_ascii_case(&'s')
                            && is_space(self.s[a + 2])
                        {
                            return Some((n, Act::TrimBegin(S::Alias, T::Actor)));
                        }
                    }
                }
                // 13: name up to the end of the line (with backtracking).
                let n = self.run(p, name);
                for len in (1..=n).rev() {
                    let mut k = p + len;
                    while k < self.s.len() && is_space(self.s[k]) && !matches!(self.s[k], '\n') {
                        k += 1;
                    }
                    let ok = k >= self.s.len()
                        || matches!(self.s[k], '\n' | ';' | '#')
                        || p + len == self.s.len();
                    // `\s*` may also swallow newlines before `[\n;#]`, which
                    // the `\n` check above already covers.
                    if ok {
                        return Some((len, Act::PopTrim(T::Actor)));
                    }
                }
                // 14: something with `<`
                let n = self.run(p, |c| name(c) || c == '<');
                if let Some(lt) = r[..n].iter().position(|&c| c == '<') {
                    if r[..lt].iter().all(|&c| name(c)) {
                        let len = lt + 1 + self.run(p + lt + 1, |c| c != '\n');
                        return Some((len, Act::Pop(T::Invalid)));
                    }
                }
                // 15: anything else on the line
                let n = self.run(p, |c| c != '\n');
                if n > 0 {
                    return Some((n, Act::PopTrim(T::Invalid)));
                }
                None
            }
            S::Alias => {
                let n = self.run(p, |c| is_space(c) && c != '\n');
                if n > 0 {
                    return Some((n, Act::Skip));
                }
                if r.first() == Some(&'#') {
                    return Some((self.run(p, |c| c != '\n'), Act::Skip));
                }
                if let Some(n) = self.kw("as") {
                    return Some((n, Act::PopPopBegin(S::Line, T::As)));
                }
                Some((0, Act::PopPop(T::Newline)))
            }
            S::Line => {
                let n = self.run(p, |c| is_space(c) && c != '\n');
                if n > 0 {
                    return Some((n, Act::Skip));
                }
                if r.first() == Some(&'#') {
                    return Some((self.run(p, |c| c != '\n'), Act::Skip));
                }
                let n = self.run(p, |c| !matches!(c, '#' | '\n' | ';'));
                Some((n, Act::Pop(T::RestOfLine)))
            }
            S::Config => {
                let n = self.run(p, |c| c != '}');
                if n > 0 {
                    return Some((n, Act::Tok(T::ConfigContent)));
                }
                if r.first() == Some(&'}') {
                    // 9: `}` before ` as `
                    let ws = self.run(p + 1, is_space);
                    let a = p + 1 + ws;
                    if ws > 0
                        && self.s.len() > a + 2
                        && self.s[a].eq_ignore_ascii_case(&'a')
                        && self.s[a + 1].eq_ignore_ascii_case(&'s')
                        && is_space(self.s[a + 2])
                    {
                        return Some((1, Act::PopBegin(S::Alias, T::ConfigEnd)));
                    }
                    return Some((1, Act::PopPop(T::ConfigEnd)));
                }
                None
            }
            S::AccTitle => Some((self.run(p, |c| c != '\n'), Act::Pop(T::AccTitleValue))),
            S::AccDescr => Some((self.run(p, |c| c != '\n'), Act::Pop(T::AccDescrValue))),
            S::AccDescrMultiline => {
                if r.first() == Some(&'}') {
                    return Some((1, Act::PopSilent));
                }
                Some((
                    self.run(p, |c| c != '}'),
                    Act::Tok(T::AccDescrMultilineValue),
                ))
            }
        }
    }

    fn initial(&self) -> Option<(usize, Act)> {
        let r = self.rest();
        let p = self.pos;
        let first = r.first().copied();
        // 0: newlines
        let n = self.run(p, |c| c == '\n');
        if n > 0 {
            return Some((n, Act::Tok(T::Newline)));
        }
        // 1: whitespace
        let n = self.run(p, is_space);
        if n > 0 {
            return Some((n, Act::Skip));
        }
        // 3: `#` comment
        if first == Some('#') {
            return Some((self.run(p, |c| c != '\n'), Act::Skip));
        }
        // 4: `%` comment
        if first == Some('%') && r.get(1) != Some(&'{') {
            return Some((self.run(p, |c| c != '\n'), Act::Skip));
        }
        // 5: `x%%` comment
        if first.is_some_and(|c| c != '}') && r.get(1) == Some(&'%') && r.get(2) == Some(&'%') {
            return Some((3 + self.run(p + 3, |c| c != '\n'), Act::Skip));
        }
        // 6: number
        if let Some(n) = self.number() {
            return Some((n, Act::Tok(T::Num)));
        }
        let keywords: &[(&str, Act)] = &[];
        let _ = keywords;
        macro_rules! kw {
            ($w:expr, $act:expr) => {
                if let Some(n) = self.kw($w) {
                    return Some((n, $act));
                }
            };
        }
        kw!("box", Act::Begin(S::Line, T::Box));
        kw!("participant", Act::Begin(S::Id, T::Participant));
        kw!("actor", Act::Begin(S::Id, T::ParticipantActor));
        kw!("create", Act::Tok(T::Create));
        kw!("destroy", Act::Begin(S::Id, T::Destroy));
        kw!("loop", Act::Begin(S::Line, T::Loop));
        kw!("rect", Act::Begin(S::Line, T::Rect));
        kw!("opt", Act::Begin(S::Line, T::Opt));
        kw!("alt", Act::Begin(S::Line, T::Alt));
        kw!("else", Act::Begin(S::Line, T::Else));
        kw!("par", Act::Begin(S::Line, T::Par));
        kw!("par_over", Act::Begin(S::Line, T::ParOver));
        kw!("and", Act::Begin(S::Line, T::And));
        kw!("critical", Act::Begin(S::Line, T::Critical));
        kw!("option", Act::Begin(S::Line, T::OptionKw));
        kw!("break", Act::Begin(S::Line, T::Break));
        kw!("end", Act::Tok(T::End));
        kw!("left of", Act::Tok(T::LeftOf));
        kw!("right of", Act::Tok(T::RightOf));
        for (w, t) in [
            ("links", T::Links),
            ("link", T::Link),
            ("properties", T::Properties),
            ("details", T::Details),
        ] {
            if let Some(n) = self.lit(w) {
                // Followed by spaces and a char that can start a name.
                let ws = self.run(p + n, |c| c == ' ' || c == '\t');
                let next = self.s.get(p + n + ws).copied();
                if ws > 0
                    && next.is_some_and(|c| {
                        !matches!(c, '/' | '\\' | '+' | '(' | ')' | '<' | '>' | ':' | '\n' | ',' | ';' | '-')
                            && !is_space(c)
                    })
                {
                    return Some((n, Act::Tok(t)));
                }
            }
        }
        kw!("over", Act::Tok(T::Over));
        kw!("note", Act::Tok(T::Note));
        kw!("activate", Act::Begin(S::Id, T::Activate));
        kw!("deactivate", Act::Begin(S::Id, T::Deactivate));
        // 46: `title text`
        if let Some(n) = self.lit("title") {
            if self.s.get(p + n).is_some_and(|&c| is_space(c)) {
                let m = self.run(p + n + 1, |c| !matches!(c, '#' | '\n' | ';'));
                if m > 0 {
                    return Some((n + 1 + m, Act::Tok(T::Title)));
                }
            }
            // 47: `title: text`
            if self.s.get(p + n) == Some(&':') && self.s.get(p + n + 1).is_some_and(|&c| is_space(c)) {
                let m = self.run(p + n + 2, |c| !matches!(c, '#' | '\n' | ';'));
                if m > 0 {
                    return Some((n + 2 + m, Act::Tok(T::LegacyTitle)));
                }
            }
        }
        // 48, 50, 52: accessibility
        for (w, colon) in [("accTitle", true), ("accDescr", true), ("accDescr", false)] {
            if let Some(n) = self.lit(w) {
                let ws = self.run(p + n, is_space);
                let sep = if colon { ':' } else { '{' };
                if self.s.get(p + n + ws) == Some(&sep) {
                    let ws2 = self.run(p + n + ws + 1, is_space);
                    let len = n + ws + 1 + ws2;
                    return Some((
                        len,
                        match (w, colon) {
                            ("accTitle", _) => Act::Begin(S::AccTitle, T::AccTitle),
                            (_, true) => Act::Begin(S::AccDescr, T::AccDescr),
                            _ => Act::BeginSilent(S::AccDescrMultiline),
                        },
                    ));
                }
            }
        }
        kw!("sequenceDiagram", Act::Tok(T::Sd));
        kw!("autonumber", Act::Tok(T::Autonumber));
        kw!("off", Act::Tok(T::Off));
        if first == Some(',') {
            return Some((1, Act::Tok(T::Comma)));
        }
        if first == Some(';') {
            return Some((1, Act::Tok(T::Newline)));
        }
        // 60: participant name
        if let Some(n) = self.actor() {
            return Some((n, Act::Trim(T::Actor)));
        }
        for (a, t) in ARROWS {
            if let Some(n) = self.lit(a) {
                return Some((n, Act::Tok(T::Signal(*t))));
            }
        }
        // 87: `:text`
        if first == Some(':') {
            let n = 1 + self.run(p + 1, |c| !matches!(c, '#' | '\n' | ';'));
            return Some((n, Act::Tok(T::Txt)));
        }
        match first {
            Some('+') => return Some((1, Act::Tok(T::Plus))),
            Some('-') => return Some((1, Act::Tok(T::Minus))),
            Some('(') if r.get(1) == Some(&')') => return Some((2, Act::Tok(T::Central))),
            None => return Some((0, Act::Tok(T::Newline))),
            Some(c) if !is_line_end(c) => return Some((1, Act::Tok(T::Invalid))),
            _ => {}
        }
        None
    }

    /// `([0-9]+(\.[0-9]{1,2})?|\.[0-9]{1,2})(?=[ \n]+)`
    fn number(&self) -> Option<usize> {
        let r = self.rest();
        let followed = |n: usize| r.get(n).is_some_and(|&c| c == ' ' || c == '\n');
        let digits = r.iter().take_while(|c| c.is_ascii_digit()).count();
        let frac = |at: usize| -> Vec<usize> {
            if r.get(at) != Some(&'.') {
                return vec![];
            }
            let d = r[at + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
            (1..=d.min(2)).rev().map(|k| at + 1 + k).collect()
        };
        if digits > 0 {
            for end in frac(digits) {
                if followed(end) {
                    return Some(end);
                }
            }
            if followed(digits) {
                return Some(digits);
            }
            return None;
        }
        frac(0).into_iter().find(|&end| followed(end))
    }

    /// The `ACTOR` rule:
    /// `A+ ((?!BAD) -* B+)*` where `A` excludes `/\+()<->:\n,;`, `B`
    /// excludes `+<->:\n,;`, and `BAD` are arrow beginnings.
    fn actor(&self) -> Option<usize> {
        let r = self.rest();
        let a = |c: char| {
            !matches!(c, '/' | '\\' | '+' | '(' | ')' | '<' | '-' | '>' | ':' | '\n' | ',' | ';')
        };
        let b = |c: char| !matches!(c, '+' | '<' | '-' | '>' | ':' | '\n' | ',' | ';');
        let mut i = r.iter().take_while(|&&c| a(c)).count();
        if i == 0 {
            return None;
        }
        const BAD: &[&str] = &[
            "-x", "--x", "-)", "--)", "-|\\", "-\\", "-/", "-//", "-|/", "/|-", "\\|-", "//-",
            "\\\\-", "/|-", "--|\\", "--", "()",
        ];
        loop {
            let at = &r[i..];
            let bad = BAD.iter().any(|w| {
                let n = w.chars().count();
                at.len() >= n && at.iter().zip(w.chars()).all(|(x, y)| x.eq_ignore_ascii_case(&y))
            });
            if bad {
                break;
            }
            let dashes = at.iter().take_while(|&&c| c == '-').count();
            let body = at[dashes..].iter().take_while(|&&c| b(c)).count();
            if body == 0 {
                break;
            }
            i += dashes + body;
        }
        Some(i)
    }
}

// ---------------------------------------------------------------------------
// Parser

/// A parsed message text (`parseMessage`).
#[derive(Debug, Clone, PartialEq)]
struct Msg {
    text: String,
    wrap: Option<bool>,
}

fn parse_message(s: &str) -> Msg {
    let t = js_trim(s);
    let (wrap, rest) = if let Some(r) = t.strip_prefix(":wrap:").or_else(|| t.strip_prefix("wrap:")) {
        (Some(true), r)
    } else if let Some(r) = t.strip_prefix(":nowrap:").or_else(|| t.strip_prefix("nowrap:")) {
        (Some(false), r)
    } else {
        (None, t.as_str())
    };
    Msg {
        text: js_trim(rest),
        wrap,
    }
}

/// One `apply` item, in Mermaid's order.
#[derive(Debug, Clone)]
enum Op {
    Participant {
        actor: String,
        draw: Option<&'static str>,
        description: Option<Msg>,
        config: Option<String>,
        create: bool,
    },
    Destroy(String),
    BoxStart(String),
    BoxEnd,
    Message {
        from: String,
        to: String,
        kind: LineType,
        msg: Msg,
        activate: bool,
        central: Option<LineType>,
    },
    /// `activeStart`, `activeEnd`, `centralConnection(Reverse)`.
    Signal(String, LineType),
    Autonumber(Autonumber),
    Note {
        placement: Placement,
        actors: (String, String),
        text: Msg,
    },
    Links(String, Msg),
    Link(String, Msg),
    Properties(String, Msg),
    Block(LineType, Option<Msg>),
}

struct Parser<'a> {
    toks: &'a [Tok],
    i: usize,
    src: &'a Prepared,
    ops: Vec<Op>,
    title: Option<String>,
    acc_title: Option<String>,
    acc_descr: Option<String>,
}

impl Parser<'_> {
    fn peek(&self) -> T {
        self.toks[self.i].t
    }

    fn bump(&mut self) -> Tok {
        let t = self.toks[self.i].clone();
        if self.i + 1 < self.toks.len() {
            self.i += 1;
        }
        t
    }

    fn error(&self, expected: &str) -> Error {
        let tok = &self.toks[self.i];
        let (l, c) = self.src.line_col(tok.pos);
        let got = match tok.t {
            T::Actor | T::Invalid | T::Txt if !tok.text.is_empty() => format!("`{}`", tok.text),
            t => t.describe().to_string(),
        };
        Error::new(
            ErrorKind::Syntax,
            l,
            c,
            format!("expected {expected}, found {got}"),
        )
    }

    fn expect(&mut self, t: T, what: &str) -> Result<Tok> {
        if self.peek() == t {
            Ok(self.bump())
        } else {
            Err(self.error(what))
        }
    }

    fn start(&mut self) -> Result<()> {
        while self.peek() == T::Newline {
            self.bump();
        }
        self.expect(T::Sd, "`sequenceDiagram`")?;
        self.document(&[])?;
        self.expect(T::Eof, "a statement")?;
        Ok(())
    }

    /// `document: line*`, up to one of `stops` (or the end).
    fn document(&mut self, stops: &[T]) -> Result<()> {
        loop {
            let t = self.peek();
            if t == T::Eof || stops.contains(&t) {
                return Ok(());
            }
            match t {
                T::Newline | T::Invalid => {
                    self.bump();
                }
                _ => self.statement()?,
            }
        }
    }

    fn actor(&mut self) -> Result<String> {
        match self.peek() {
            T::Actor | T::Link => Ok(self.bump().text),
            _ => Err(self.error("a participant")),
        }
    }

    fn text2(&mut self) -> Result<Msg> {
        let t = self.expect(T::Txt, "`:` and a text")?;
        let trimmed = js_trim(&t.text);
        Ok(parse_message(trimmed.get(1..).unwrap_or("")))
    }

    fn newline(&mut self) -> Result<()> {
        self.expect(T::Newline, "end of line").map(|_| ())
    }

    fn statement(&mut self) -> Result<()> {
        match self.peek() {
            T::Participant | T::ParticipantActor | T::Destroy => self.participant(false),
            T::Create => {
                self.bump();
                self.participant(true)
            }
            T::Box => {
                self.bump();
                let rest = self.expect(T::RestOfLine, "a box title")?.text;
                self.ops.push(Op::BoxStart(rest));
                loop {
                    match self.peek() {
                        T::Newline => {
                            self.bump();
                        }
                        T::Participant | T::ParticipantActor | T::Destroy => {
                            self.participant(false)?
                        }
                        T::End => break,
                        _ => return Err(self.error("a participant or `end`")),
                    }
                }
                self.bump();
                self.ops.push(Op::BoxEnd);
                Ok(())
            }
            T::Actor => self.signal(),
            T::Link => {
                // `link actor: text`, or a signal from a participant named `link`.
                let next = self.toks.get(self.i + 1).map(|t| t.t);
                if matches!(next, Some(T::Signal(_) | T::Central)) {
                    self.signal()
                } else {
                    self.bump();
                    let a = self.actor()?;
                    let text = self.text2()?;
                    self.newline()?;
                    self.ops.push(participant(&a));
                    self.ops.push(Op::Link(a, text));
                    Ok(())
                }
            }
            T::Autonumber => {
                self.bump();
                let auto = match self.peek() {
                    T::Num => {
                        let start = self.bump().text.parse::<f64>().ok();
                        if self.peek() == T::Num {
                            let step = self.bump().text.parse::<f64>().ok();
                            Autonumber {
                                start,
                                step,
                                visible: true,
                            }
                        } else {
                            Autonumber {
                                start,
                                step: Some(1.0),
                                visible: true,
                            }
                        }
                    }
                    T::Off => {
                        self.bump();
                        Autonumber {
                            start: None,
                            step: None,
                            visible: false,
                        }
                    }
                    _ => Autonumber {
                        start: None,
                        step: None,
                        visible: true,
                    },
                };
                self.newline()?;
                self.ops.push(Op::Autonumber(auto));
                Ok(())
            }
            T::Activate | T::Deactivate => {
                let kind = if self.bump().t == T::Activate {
                    LineType::ActiveStart
                } else {
                    LineType::ActiveEnd
                };
                let a = self.actor()?;
                self.newline()?;
                self.ops.push(Op::Signal(a, kind));
                Ok(())
            }
            T::Note => {
                self.bump();
                let placement = match self.peek() {
                    T::LeftOf => Placement::LeftOf,
                    T::RightOf => Placement::RightOf,
                    T::Over => Placement::Over,
                    _ => return Err(self.error("`left of`, `right of` or `over`")),
                };
                self.bump();
                let a = self.actor()?;
                let b = if placement == Placement::Over && self.peek() == T::Comma {
                    self.bump();
                    Some(self.actor()?)
                } else {
                    None
                };
                let text = self.text2()?;
                self.newline()?;
                self.ops.push(participant(&a));
                if let Some(b) = &b {
                    self.ops.push(participant(b));
                }
                let actors = match b {
                    Some(b) => (a, b),
                    None => (a.clone(), a),
                };
                self.ops.push(Op::Note {
                    placement,
                    actors,
                    text,
                });
                Ok(())
            }
            T::Links | T::Properties | T::Details => {
                let kw = self.bump().t;
                let a = self.actor()?;
                let text = self.text2()?;
                self.newline()?;
                self.ops.push(participant(&a));
                match kw {
                    T::Links => self.ops.push(Op::Links(a, text)),
                    T::Properties => self.ops.push(Op::Properties(a, text)),
                    // `details` reads an HTML element by id from the page,
                    // which never exists here.
                    _ => {}
                }
                Ok(())
            }
            T::Title => {
                let t = self.bump().text;
                self.title = Some(t.chars().skip(6).collect());
                Ok(())
            }
            T::LegacyTitle => {
                let t = self.bump().text;
                self.title = Some(t.chars().skip(7).collect());
                Ok(())
            }
            T::AccTitle => {
                self.bump();
                let v = self.expect(T::AccTitleValue, "a title")?.text;
                self.acc_title = Some(js_trim(&v));
                Ok(())
            }
            T::AccDescr => {
                self.bump();
                let v = self.expect(T::AccDescrValue, "a description")?.text;
                self.acc_descr = Some(js_trim(&v));
                Ok(())
            }
            T::AccDescrMultilineValue => {
                let v = self.bump().text;
                self.acc_descr = Some(js_trim(&v));
                Ok(())
            }
            T::Loop | T::Rect | T::Opt | T::Break => {
                let kw = self.bump().t;
                let rest = self.expect(T::RestOfLine, "text")?.text;
                let (start, end) = match kw {
                    T::Loop => (LineType::LoopStart, LineType::LoopEnd),
                    T::Rect => (LineType::RectStart, LineType::RectEnd),
                    T::Opt => (LineType::OptStart, LineType::OptEnd),
                    _ => (LineType::BreakStart, LineType::BreakEnd),
                };
                self.ops.push(Op::Block(start, Some(parse_message(&rest))));
                self.document(&[T::End])?;
                self.expect(T::End, "`end`")?;
                self.ops.push(Op::Block(end, None));
                Ok(())
            }
            T::Alt | T::Par | T::ParOver | T::Critical => {
                let kw = self.bump().t;
                let rest = self.expect(T::RestOfLine, "text")?.text;
                let (start, sep_tok, sep, end) = match kw {
                    T::Alt => (LineType::AltStart, T::Else, LineType::AltElse, LineType::AltEnd),
                    T::Par => (LineType::ParStart, T::And, LineType::ParAnd, LineType::ParEnd),
                    T::ParOver => (
                        LineType::ParOverStart,
                        T::And,
                        LineType::ParAnd,
                        LineType::ParEnd,
                    ),
                    _ => (
                        LineType::CriticalStart,
                        T::OptionKw,
                        LineType::CriticalOption,
                        LineType::CriticalEnd,
                    ),
                };
                self.ops.push(Op::Block(start, Some(parse_message(&rest))));
                loop {
                    self.document(&[T::End, sep_tok])?;
                    if self.peek() == sep_tok {
                        self.bump();
                        let rest = self.expect(T::RestOfLine, "text")?.text;
                        self.ops.push(Op::Block(sep, Some(parse_message(&rest))));
                    } else {
                        break;
                    }
                }
                self.expect(T::End, "`end`")?;
                self.ops.push(Op::Block(end, None));
                Ok(())
            }
            _ => Err(self.error("a statement")),
        }
    }

    fn participant(&mut self, create: bool) -> Result<()> {
        let kw = self.bump().t;
        let actor = match self.peek() {
            T::Actor => self.bump().text,
            _ => return Err(self.error("a participant name")),
        };
        if kw == T::Destroy {
            self.newline()?;
            self.ops.push(if create {
                Op::Participant {
                    actor,
                    draw: None,
                    description: None,
                    config: None,
                    create,
                }
            } else {
                Op::Destroy(actor)
            });
            return Ok(());
        }
        let mut config = None;
        if self.peek() == T::ConfigStart {
            self.bump();
            let content = self.expect(T::ConfigContent, "participant data")?.text;
            self.expect(T::ConfigEnd, "`}`")?;
            config = Some(js_trim(&content));
        }
        let mut description = None;
        if self.peek() == T::As {
            self.bump();
            let rest = self.expect(T::RestOfLine, "an alias")?.text;
            description = Some(parse_message(&rest));
        }
        self.newline()?;
        self.ops.push(Op::Participant {
            actor,
            draw: Some(if kw == T::Participant {
                "participant"
            } else {
                "actor"
            }),
            description,
            config,
            create,
        });
        Ok(())
    }

    fn signal_type(&mut self) -> Result<LineType> {
        match self.peek() {
            T::Signal(t) => {
                self.bump();
                Ok(t)
            }
            _ => Err(self.error("an arrow")),
        }
    }

    fn signal(&mut self) -> Result<()> {
        let from = self.actor()?;
        let mut central_from = false;
        if self.peek() == T::Central {
            self.bump();
            central_from = true;
        }
        let kind = self.signal_type()?;
        enum Mod {
            None,
            Plus,
            Minus,
            Central,
        }
        let m = match self.peek() {
            T::Plus if !central_from => {
                self.bump();
                Mod::Plus
            }
            T::Minus if !central_from => {
                self.bump();
                Mod::Minus
            }
            T::Central => {
                self.bump();
                Mod::Central
            }
            _ => Mod::None,
        };
        let to = self.actor()?;
        let msg = self.text2()?;
        self.newline()?;
        self.ops.push(participant(&from));
        self.ops.push(participant(&to));
        let message = |activate: bool, central: Option<LineType>| Op::Message {
            from: from.clone(),
            to: to.clone(),
            kind,
            msg: msg.clone(),
            activate,
            central,
        };
        match (central_from, m) {
            (false, Mod::Plus) => {
                self.ops.push(message(true, None));
                self.ops.push(Op::Signal(to.clone(), LineType::ActiveStart));
            }
            (false, Mod::Minus) => {
                self.ops.push(message(false, None));
                self.ops.push(Op::Signal(from.clone(), LineType::ActiveEnd));
            }
            (false, Mod::Central) => {
                self.ops
                    .push(message(true, Some(LineType::CentralConnection)));
                self.ops
                    .push(Op::Signal(to.clone(), LineType::CentralConnection));
            }
            (true, Mod::None) => {
                self.ops
                    .push(message(false, Some(LineType::CentralConnectionReverse)));
                self.ops
                    .push(Op::Signal(from.clone(), LineType::CentralConnectionReverse));
            }
            (true, Mod::Central) => {
                self.ops
                    .push(message(true, Some(LineType::CentralConnectionDual)));
                self.ops
                    .push(Op::Signal(to.clone(), LineType::CentralConnection));
                self.ops
                    .push(Op::Signal(from.clone(), LineType::CentralConnectionReverse));
            }
            _ => self.ops.push(message(false, None)),
        }
        Ok(())
    }
}

fn participant(actor: &str) -> Op {
    Op::Participant {
        actor: actor.to_string(),
        draw: None,
        description: None,
        config: None,
        create: false,
    }
}

// ---------------------------------------------------------------------------
// Database (`sequenceDb.apply`)

#[derive(Default)]
struct Db {
    actors: Vec<Participant>,
    index: HashMap<String, usize>,
    prev_actor: Option<String>,
    messages: Vec<Message>,
    boxes: Vec<ParticipantBox>,
    current_box: Option<usize>,
    created: Vec<(String, usize)>,
    destroyed: Vec<(String, usize)>,
    last_created: Option<String>,
    last_destroyed: Option<String>,
    auto_wrap: bool,
}

impl Db {
    fn get_mut(&mut self, id: &str) -> Option<&mut Participant> {
        let i = *self.index.get(id)?;
        Some(&mut self.actors[i])
    }

    fn add_actor(
        &mut self,
        id: &str,
        description: Option<Msg>,
        draw: Option<&'static str>,
        metadata: Option<&str>,
    ) -> std::result::Result<(), String> {
        let mut assigned_box = self.current_box;
        let doc = metadata.map(config::parse_shape_data);
        let mut kind: Option<String> = draw.map(str::to_string);
        if let Some(t) = doc.as_ref().and_then(|d| d.get("type")).and_then(scalar) {
            kind = Some(t);
        }
        let mut description = description;
        if let Some(alias) = doc.as_ref().and_then(|d| d.get("alias")).and_then(scalar) {
            if !alias.is_empty()
                && description
                    .as_ref()
                    .map_or(true, |d| d.text == id)
            {
                description = Some(Msg {
                    text: alias,
                    wrap: description.and_then(|d| d.wrap),
                });
            }
        }
        if let Some(&i) = self.index.get(id) {
            let old_box = self.actors[i].group;
            if let (Some(cur), Some(old)) = (self.current_box, old_box) {
                if cur != old {
                    return Err(format!(
                        "a participant can only be in one box: `{}` is in `{}` and `{}`",
                        id,
                        self.boxes[old].name.as_deref().unwrap_or(""),
                        self.boxes[cur].name.as_deref().unwrap_or("")
                    ));
                }
            }
            assigned_box = old_box.or(self.current_box);
            self.actors[i].group = assigned_box;
            if description.is_none() {
                return Ok(());
            }
        }
        let description = match description {
            Some(d) if kind.is_some() => d,
            _ => Msg {
                text: id.to_string(),
                wrap: None,
            },
        };
        let actor = Participant {
            id: id.to_string(),
            name: id.to_string(),
            description: description.text,
            wrap: description.wrap.unwrap_or(self.auto_wrap),
            kind: ParticipantKind::from_name(kind.as_deref().unwrap_or("participant")),
            group: assigned_box,
            prev: self.prev_actor.clone(),
            next: None,
            links: Vec::new(),
            class: None,
            icon: None,
        };
        match self.index.get(id) {
            Some(&i) => self.actors[i] = actor,
            None => {
                self.index.insert(id.to_string(), self.actors.len());
                self.actors.push(actor);
            }
        }
        if let Some(prev) = self.prev_actor.clone() {
            if let Some(p) = self.get_mut(&prev) {
                p.next = Some(id.to_string());
            }
        }
        if let Some(b) = self.current_box {
            self.boxes[b].participants.push(id.to_string());
        }
        self.prev_actor = Some(id.to_string());
        Ok(())
    }

    fn activation_count(&self, part: &str) -> i64 {
        self.messages.iter().fold(0, |n, m| {
            match (m.kind, m.from.as_deref()) {
                (LineType::ActiveStart, Some(f)) if f == part => n + 1,
                (LineType::ActiveEnd, Some(f)) if f == part => n - 1,
                _ => n,
            }
        })
    }

    fn push(&mut self, kind: LineType, from: Option<String>, to: Option<String>, msg: Option<Msg>) {
        let (text, wrap) = match msg {
            Some(m) => (m.text, m.wrap.unwrap_or(self.auto_wrap)),
            None => (String::new(), self.auto_wrap),
        };
        self.messages.push(Message {
            kind,
            from,
            to,
            text,
            wrap,
            activate: false,
            placement: None,
            central: None,
            autonumber: None,
        });
    }

    fn apply(&mut self, op: Op) -> std::result::Result<(), String> {
        match op {
            Op::Participant {
                actor,
                draw,
                description,
                config,
                create,
            } => {
                if create {
                    if self.index.contains_key(&actor) {
                        return Err(format!(
                            "participant `{actor}` is created but already exists; use `as` aliases instead"
                        ));
                    }
                    self.last_created = Some(actor.clone());
                }
                self.add_actor(&actor, description, draw, config.as_deref())?;
                if create {
                    self.created.retain(|(a, _)| *a != actor);
                    self.created.push((actor, self.messages.len()));
                }
            }
            Op::Destroy(actor) => {
                self.last_destroyed = Some(actor.clone());
                self.destroyed.retain(|(a, _)| *a != actor);
                self.destroyed.push((actor, self.messages.len()));
            }
            Op::BoxStart(rest) => {
                let (text, fill, wrap) = parse_box_data(&rest);
                self.boxes.push(ParticipantBox {
                    name: text,
                    fill,
                    wrap: wrap.unwrap_or(self.auto_wrap),
                    participants: Vec::new(),
                });
                self.current_box = Some(self.boxes.len() - 1);
            }
            Op::BoxEnd => self.current_box = None,
            Op::Message {
                from,
                to,
                kind,
                msg,
                activate,
                central,
            } => {
                if let Some(c) = &self.last_created {
                    if *c != to {
                        return Err(format!(
                            "the created participant `{c}` has no creating message right after its declaration"
                        ));
                    }
                    self.last_created = None;
                } else if let Some(d) = &self.last_destroyed {
                    if *d != to && *d != from {
                        return Err(format!(
                            "the destroyed participant `{d}` has no destroying message right after its declaration"
                        ));
                    }
                    self.last_destroyed = None;
                }
                self.push(kind, Some(from), Some(to), Some(msg));
                let m = self.messages.last_mut().expect("just pushed");
                m.activate = activate;
                m.central = central;
            }
            Op::Signal(actor, kind) => {
                if kind == LineType::ActiveEnd && self.activation_count(&actor) < 1 {
                    return Err(format!(
                        "trying to deactivate an inactive participant (`{actor}`)"
                    ));
                }
                self.push(kind, Some(actor), None, None);
            }
            Op::Autonumber(a) => {
                self.push(LineType::Autonumber, None, None, None);
                let m = self.messages.last_mut().expect("just pushed");
                m.wrap = false;
                m.autonumber = Some(a);
            }
            Op::Note {
                placement,
                actors,
                text,
            } => {
                self.push(LineType::Note, Some(actors.0), Some(actors.1), Some(text));
                self.messages.last_mut().expect("just pushed").placement = Some(placement);
            }
            Op::Links(actor, text) => {
                let s = sanitize_text(&text.text)
                    .replace("&equals;", "=")
                    .replace("&amp;", "&");
                if let Some(m) = config::parse_json_object(&s) {
                    if let Some(a) = self.get_mut(&actor) {
                        for (k, v) in m {
                            if let Some(v) = scalar(&v) {
                                insert_link(&mut a.links, k, v);
                            }
                        }
                    }
                }
            }
            Op::Link(actor, text) => {
                let s = sanitize_text(&text.text);
                let sep = s.find('@');
                let s = s.replace("&equals;", "=").replace("&amp;", "&");
                // `slice(0, sep - 1)` and `slice(sep + 1)`, with `sep` from
                // before the replacements, as Mermaid does.
                let chars: Vec<char> = s.chars().collect();
                let sep_i = sep.map_or(-1, |b| s[..b.min(s.len())].chars().count() as i64);
                let label_end = if sep_i - 1 < 0 {
                    (chars.len() as i64 + sep_i - 1).max(0) as usize
                } else {
                    ((sep_i - 1) as usize).min(chars.len())
                };
                let label = js_trim(&chars[..label_end].iter().collect::<String>());
                let link_start = ((sep_i + 1).max(0) as usize).min(chars.len());
                let link = js_trim(&chars[link_start..].iter().collect::<String>());
                if let Some(a) = self.get_mut(&actor) {
                    insert_link(&mut a.links, label, link);
                }
            }
            Op::Properties(actor, text) => {
                let s = sanitize_text(&text.text);
                if let Some(m) = config::parse_json_object(&s) {
                    if let Some(a) = self.get_mut(&actor) {
                        for (k, v) in &m {
                            match k.as_str() {
                                "class" => a.class = scalar(v),
                                "icon" => a.icon = scalar(v),
                                _ => {}
                            }
                        }
                    }
                }
            }
            Op::Block(kind, msg) => self.push(kind, None, None, msg),
        }
        Ok(())
    }
}

fn insert_link(links: &mut Vec<(String, String)>, k: String, v: String) {
    match links.iter_mut().find(|(lk, _)| *lk == k) {
        Some(slot) => slot.1 = v,
        None => links.push((k, v)),
    }
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.clone()),
        Value::Num(n) => Some(crate::dom::js_num(*n)),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// `parseBoxData`: an optional leading color, then the title.
fn parse_box_data(s: &str) -> (Option<String>, String, Option<bool>) {
    let lower = s.to_string();
    let (mut color, mut title) = {
        let func = ["rgba", "rgb", "hsla", "hsl"]
            .iter()
            .find(|f| lower.starts_with(**f))
            .and_then(|f| {
                let after = &s[f.len()..];
                let ws = after.len() - after.trim_start_matches(is_space).len();
                let rest = &after[ws..];
                if !rest.starts_with('(') {
                    return None;
                }
                let close = rest.rfind(')')?;
                let end = f.len() + ws + close + 1;
                Some(end)
            });
        let end = func.unwrap_or_else(|| s.find(|c: char| !is_word(c)).unwrap_or(s.len()));
        let c = js_trim(&s[..end]);
        let t = js_trim(&s[end..]);
        (
            if c.is_empty() { "transparent".to_string() } else { c },
            (!t.is_empty()).then_some(t),
        )
    };
    if !is_css_color(&color) {
        color = "transparent".to_string();
        title = Some(js_trim(s));
    }
    let (text, wrap) = match title {
        Some(t) => {
            let m = parse_message(&t);
            (Some(m.text), m.wrap)
        }
        None => (None, None),
    };
    let text = text.filter(|t| !t.is_empty()).map(|t| sanitize_text(&t));
    (text, color, wrap)
}

/// Whether `CSS.supports('color', c)` would accept the value.
fn is_css_color(c: &str) -> bool {
    let l = c.to_ascii_lowercase();
    if NAMED_COLORS.binary_search(&l.as_str()).is_ok() {
        return true;
    }
    if let Some(open) = l.find('(') {
        let name = &l[..open];
        if !matches!(name, "rgb" | "rgba" | "hsl" | "hsla") || !l.ends_with(')') {
            return false;
        }
        let inner = &l[open + 1..l.len() - 1];
        let hsl = name.starts_with('h');
        let parts: Vec<&str> = if inner.contains(',') {
            inner.split(',').map(str::trim).collect()
        } else {
            inner
                .split(|c: char| c.is_whitespace() || c == '/')
                .filter(|p| !p.is_empty())
                .collect()
        };
        if !(parts.len() == 3 || parts.len() == 4) {
            return false;
        }
        return parts.iter().enumerate().all(|(i, p)| {
            let num = |s: &str| s.parse::<f64>().is_ok();
            *p == "none"
                || num(p)
                || p.strip_suffix('%').is_some_and(num)
                || (hsl && i == 0 && ["deg", "rad", "grad", "turn"].iter().any(|u| p.strip_suffix(u).is_some_and(num)))
        });
    }
    false
}

const NAMED_COLORS: &[&str] = &[
    "aliceblue", "antiquewhite", "aqua", "aquamarine", "azure", "beige", "bisque", "black",
    "blanchedalmond", "blue", "blueviolet", "brown", "burlywood", "cadetblue", "chartreuse",
    "chocolate", "coral", "cornflowerblue", "cornsilk", "crimson", "currentcolor", "cyan",
    "darkblue", "darkcyan", "darkgoldenrod", "darkgray", "darkgreen", "darkgrey", "darkkhaki",
    "darkmagenta", "darkolivegreen", "darkorange", "darkorchid", "darkred", "darksalmon",
    "darkseagreen", "darkslateblue", "darkslategray", "darkslategrey", "darkturquoise",
    "darkviolet", "deeppink", "deepskyblue", "dimgray", "dimgrey", "dodgerblue", "firebrick",
    "floralwhite", "forestgreen", "fuchsia", "gainsboro", "ghostwhite", "gold", "goldenrod",
    "gray", "green", "greenyellow", "grey", "honeydew", "hotpink", "indianred", "indigo",
    "inherit", "initial", "ivory", "khaki", "lavender", "lavenderblush", "lawngreen",
    "lemonchiffon", "lightblue", "lightcoral", "lightcyan", "lightgoldenrodyellow", "lightgray",
    "lightgreen", "lightgrey", "lightpink", "lightsalmon", "lightseagreen", "lightskyblue",
    "lightslategray", "lightslategrey", "lightsteelblue", "lightyellow", "lime", "limegreen",
    "linen", "magenta", "maroon", "mediumaquamarine", "mediumblue", "mediumorchid",
    "mediumpurple", "mediumseagreen", "mediumslateblue", "mediumspringgreen", "mediumturquoise",
    "mediumvioletred", "midnightblue", "mintcream", "mistyrose", "moccasin", "navajowhite",
    "navy", "oldlace", "olive", "olivedrab", "orange", "orangered", "orchid", "palegoldenrod",
    "palegreen", "paleturquoise", "palevioletred", "papayawhip", "peachpuff", "peru", "pink",
    "plum", "powderblue", "purple", "rebeccapurple", "red", "revert", "revert-layer",
    "rosybrown", "royalblue", "saddlebrown", "salmon", "sandybrown", "seagreen", "seashell",
    "sienna", "silver", "skyblue", "slateblue", "slategray", "slategrey", "snow", "springgreen",
    "steelblue", "tan", "teal", "thistle", "tomato", "transparent", "turquoise", "unset",
    "violet", "wheat", "white", "whitesmoke", "yellow", "yellowgreen",
];

/// Mermaid's `sanitizeText` for plain text: an HTML round trip, so a bare
/// `&` comes back as `&amp;`. Markup is kept as written.
pub(crate) fn sanitize_text(s: &str) -> String {
    if s.contains('<') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let ent = after
            .find(';')
            .filter(|&j| j > 0 && after[..j].chars().all(|c| c.is_ascii_alphanumeric() || c == '#'));
        match ent {
            Some(j) => {
                out.push_str(&rest[i..i + 2 + j]);
                rest = &after[j + 1..];
            }
            None => {
                out.push_str("&amp;");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace('>', "&gt;")
}

/// `replace(/\n\s+/g, '\n')`
fn collapse_indent(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '\n' {
            while chars.peek().is_some_and(|&n| is_space(n)) {
                chars.next();
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<T> {
        let p = Prepared::new(src);
        Lexer::new(&p.text)
            .tokenize()
            .unwrap()
            .into_iter()
            .map(|t| t.t)
            .collect()
    }

    #[test]
    fn lexes_signals() {
        assert_eq!(
            kinds("sequenceDiagram\nAlice->>Bob: Hi"),
            vec![
                T::Sd,
                T::Newline,
                T::Actor,
                T::Signal(LineType::Solid),
                T::Actor,
                T::Txt,
                T::Newline,
                T::Newline,
                T::Eof
            ]
        );
    }

    #[test]
    fn parses_basic() {
        let d = parse("sequenceDiagram\nparticipant A as Alice\nA->>+B: hi\nB-->>-A: yo").unwrap();
        assert_eq!(d.participants.len(), 2);
        assert_eq!(d.participants[0].description, "Alice");
        assert_eq!(d.messages.len(), 4);
        assert_eq!(d.messages[1].kind, LineType::ActiveStart);
    }

    #[test]
    fn rejects_missing_text() {
        assert!(parse("sequenceDiagram\nA->>B").is_err());
    }
}
