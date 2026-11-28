//! Class diagram parser.
//!
//! Mermaid 12.3 parses class diagrams with a Jison grammar
//! (`classDiagram.jison`). The LALR tables are Mermaid's own (see
//! [`super::tables`]), run by [`crate::jison`]; this module ports the lexer
//! rule by rule, the productions' actions, and the `ClassDB` methods they
//! call.

use std::collections::HashMap;

use super::tables::{sym, TABLES};
use super::{
    Class, ClassDiagram, Interface, Member, MemberKind, Namespace, Note, Relation, RelationKind,
    StyleClass,
};
use crate::config::{DiagramConfig, Value};
use crate::error::{Error, ErrorKind};
use crate::jison::{self, Failure, Grammar, Token};
use crate::source::{self, Prepared};
use crate::Result;

/// Parses a class diagram source (front matter and header included).
pub fn parse(src: &str) -> Result<ClassDiagram> {
    let split = source::split(src);
    let config =
        DiagramConfig::from_source(split.front_matter.as_deref(), &split.directives, "class");
    let html_labels = config
        .raw
        .as_ref()
        .and_then(|r| {
            [r.get("htmlLabels"), r.path("flowchart.htmlLabels")]
                .into_iter()
                .flatten()
                .find(|v| !matches!(v, Value::Null))
        })
        .map_or(true, evaluate);
    let prepared = Prepared::new(src);
    let mut g = Parser {
        lexer: Lexer {
            s: &prepared.text,
            pos: 0,
            stack: vec![St::Initial],
            done: false,
        },
        db: Db {
            html_labels,
            direction: "TB".into(),
            ..Db::default()
        },
    };
    jison::parse(&TABLES, &mut g).map_err(|f| {
        let (pos, msg) = match f {
            Failure::Lex(p) => (p, "unrecognized text".to_string()),
            Failure::Parse(p, t) if t.is_empty() => (p, "unexpected end of input".to_string()),
            Failure::Parse(p, t) => (p, format!("unexpected `{t}`")),
            Failure::Action(m, p) => (p, m),
        };
        let (l, c) = prepared.line_col(pos);
        Error::new(ErrorKind::Syntax, l, c, msg)
    })?;
    let mut db = g.db;
    let title = config.title.as_deref().map(|t| db.sanitize(t));
    Ok(ClassDiagram {
        direction: db.direction,
        classes: db.classes,
        relations: db.relations,
        notes: db.notes,
        namespaces: db.namespaces,
        style_classes: db.style_classes,
        interfaces: db.interfaces,
        title: title.filter(|t| !t.is_empty()),
        acc_title: db.acc_title.take().filter(|t| !t.is_empty()),
        acc_descr: db.acc_descr.take().filter(|t| !t.is_empty()),
        config,
    })
}

/// Mermaid's `evaluate`: `false`, `"false"`, `"null"` and `"0"` are false.
fn evaluate(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Num(n) => *n != 0.0,
        Value::Str(s) => !matches!(js_trim(s).to_lowercase().as_str(), "false" | "null" | "0"),
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Lexer

/// Lexer states, in the order of [`CONDITIONS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum St {
    Initial,
    Str,
    BqString,
    Generic,
    #[allow(dead_code)]
    Struct,
    #[allow(dead_code)]
    Href,
    CallbackName,
    CallbackArgs,
    AccTitle,
    AccDescr,
    AccDescrMultiline,
    Class,
    ClassBody,
    Namespace,
    NamespaceBody,
}

/// Rules tried in each lexer state, in order (from the compiled lexer).
const CONDITIONS: [&[u8]; 15] = [
    // INITIAL
    &[
        0, 1, 2, 3, 4, 5, 6, 8, 10, 13, 14, 15, 16, 17, 18, 26, 27, 28, 29, 39, 50, 51, 52, 53, 54,
        55, 56, 57, 58, 61, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81,
        82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98,
    ],
    // string
    &[
        24, 25, 26, 50, 51, 52, 53, 54, 55, 56, 57, 58, 61, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73,
        74, 75, 76, 77, 78, 79, 80, 81, 82, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 98,
    ],
    // bqstring
    &[
        26, 50, 51, 52, 53, 54, 55, 56, 57, 58, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73,
        74, 75, 76, 77, 78, 79, 80, 81, 82, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 98,
    ],
    // generic
    &[
        26, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73,
        74, 75, 76, 77, 78, 79, 80, 81, 82, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 98,
    ],
    // struct
    &[
        26, 50, 51, 52, 53, 54, 55, 56, 57, 58, 61, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75,
        76, 77, 78, 79, 80, 81, 82, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 98,
    ],
    // href
    &[
        26, 50, 51, 52, 53, 54, 55, 56, 57, 58, 61, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75,
        76, 77, 78, 79, 80, 81, 82, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 98,
    ],
    // callback_name
    &[
        19, 20, 21, 26, 50, 51, 52, 53, 54, 55, 56, 57, 58, 61, 64, 65, 66, 67, 68, 69, 70, 71, 72,
        73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 98,
    ],
    // callback_args
    &[
        22, 23, 26, 50, 51, 52, 53, 54, 55, 56, 57, 58, 61, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73,
        74, 75, 76, 77, 78, 79, 80, 81, 82, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 98,
    ],
    // acc_title
    &[
        7, 26, 50, 51, 52, 53, 54, 55, 56, 57, 58, 61, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74,
        75, 76, 77, 78, 79, 80, 81, 82, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 98,
    ],
    // acc_descr
    &[
        9, 26, 50, 51, 52, 53, 54, 55, 56, 57, 58, 61, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74,
        75, 76, 77, 78, 79, 80, 81, 82, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 98,
    ],
    // acc_descr_multiline
    &[
        11, 12, 26, 50, 51, 52, 53, 54, 55, 56, 57, 58, 61, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73,
        74, 75, 76, 77, 78, 79, 80, 81, 82, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 98,
    ],
    // class
    &[
        26, 40, 41, 42, 43, 50, 51, 52, 53, 54, 55, 56, 57, 58, 61, 64, 65, 66, 67, 68, 69, 70, 71,
        72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 98,
    ],
    // class-body
    &[
        26, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 61, 64, 65, 66, 67, 68, 69,
        70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96,
        98,
    ],
    // namespace
    &[
        26, 29, 30, 31, 32, 33, 50, 51, 52, 53, 54, 55, 56, 57, 58, 61, 64, 65, 66, 67, 68, 69, 70,
        71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 98,
    ],
    // namespace-body
    &[
        26, 29, 34, 35, 36, 37, 38, 39, 50, 51, 52, 53, 54, 55, 56, 57, 58, 61, 64, 65, 66, 67, 68,
        69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 87, 88, 89, 90, 91, 92, 93, 94, 95,
        96, 98,
    ],
];

/// JavaScript's `\s`.
fn is_space(c: char) -> bool {
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

/// JavaScript's `\w`.
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Chars JavaScript's `.` does not match.
fn is_line_end(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// `String.prototype.trim`.
fn js_trim(s: &str) -> &str {
    s.trim_matches(is_space)
}

fn is_unicode_text(c: char) -> bool {
    let c = c as u32;
    UNICODE_TEXT
        .binary_search_by(|&(a, b)| {
            if b < c {
                std::cmp::Ordering::Less
            } else if a > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// What a lexer action does besides returning its token.
enum Act {
    /// Returns a token.
    Tok(Option<u16>),
    /// Returns nothing.
    Skip,
}

struct Lexer<'a> {
    s: &'a [char],
    pos: usize,
    stack: Vec<St>,
    done: bool,
}

impl Lexer<'_> {
    fn rest(&self) -> &[char] {
        &self.s[self.pos..]
    }

    fn state(&self) -> St {
        *self.stack.last().unwrap()
    }

    fn begin(&mut self, s: St) {
        self.stack.push(s);
    }

    fn pop(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    /// Jison's `lex()`: skips the rules that return nothing.
    fn lex(&mut self) -> std::result::Result<Token, usize> {
        loop {
            if self.done {
                return Ok(Token {
                    symbol: Some(sym::N_END),
                    text: String::new(),
                    pos: self.pos,
                });
            }
            if self.pos >= self.s.len() {
                self.done = true;
            }
            let rules = CONDITIONS[self.state() as usize];
            let Some((rule, len)) = rules
                .iter()
                .find_map(|&r| self.match_rule(r).map(|n| (r, n)))
            else {
                if self.pos >= self.s.len() {
                    return Ok(Token {
                        symbol: Some(sym::N_END),
                        text: String::new(),
                        pos: self.pos,
                    });
                }
                return Err(self.pos);
            };
            let start = self.pos;
            self.pos += len;
            let act = self.action(rule, start);
            if self.done && self.pos < self.s.len() {
                self.done = false;
            }
            if let Act::Tok(symbol) = act {
                return Ok(Token {
                    symbol,
                    text: self.s[start..self.pos].iter().collect(),
                    pos: start,
                });
            }
        }
    }

    fn action(&mut self, rule: u8, start: usize) -> Act {
        use Act::{Skip, Tok};
        let t = |s: u16| Tok(Some(s));
        match rule {
            0 => t(sym::DIRECTION_TB),
            1 => t(sym::DIRECTION_BT),
            2 => t(sym::DIRECTION_RL),
            3 => t(sym::DIRECTION_LR),
            4 | 5 | 14 | 31 | 37 | 41 | 48 => Skip,
            6 => {
                self.begin(St::AccTitle);
                t(sym::ACC_TITLE)
            }
            7 => {
                self.pop();
                t(sym::ACC_TITLE_VALUE)
            }
            8 => {
                self.begin(St::AccDescr);
                t(sym::ACC_DESCR)
            }
            9 => {
                self.pop();
                t(sym::ACC_DESCR_VALUE)
            }
            10 => {
                self.begin(St::AccDescrMultiline);
                Skip
            }
            11 | 19 | 22 | 24 | 59 | 62 => {
                self.pop();
                Skip
            }
            12 => t(sym::ACC_DESCR_MULTILINE_VALUE),
            13 | 36 => t(sym::NEWLINE),
            15 | 16 => t(sym::CLASS_DIAGRAM),
            // `EDGE_STATE`, `EOF_IN_STRUCT`, `OPEN_IN_STRUCT`, `PLUS`,
            // `EQUALS` and `PUNCTUATION` are not grammar symbols.
            17 | 35 | 38 | 45 | 46 | 47 | 82 | 88 | 89 | 93 => Tok(None),
            18 => {
                self.begin(St::CallbackName);
                Skip
            }
            20 => {
                self.pop();
                self.begin(St::CallbackArgs);
                Skip
            }
            21 => t(sym::CALLBACK_NAME),
            23 => t(sym::CALLBACK_ARGS),
            25 => t(sym::STR),
            26 => {
                self.begin(St::Str);
                Skip
            }
            27 => t(sym::STYLE),
            28 => t(sym::CLASSDEF),
            29 => {
                self.begin(St::Namespace);
                t(sym::NAMESPACE)
            }
            30 | 40 => {
                self.pop();
                t(sym::NEWLINE)
            }
            32 => {
                self.begin(St::NamespaceBody);
                t(sym::STRUCT_START)
            }
            33 => {
                self.pop();
                self.pos = start;
                Skip
            }
            34 | 44 => {
                self.pop();
                t(sym::STRUCT_STOP)
            }
            39 => {
                self.begin(St::Class);
                t(sym::CLASS)
            }
            42 => {
                self.pop();
                self.pop();
                t(sym::STRUCT_STOP)
            }
            43 => {
                self.begin(St::ClassBody);
                t(sym::STRUCT_START)
            }
            49 => t(sym::MEMBER),
            50 => t(sym::CSSCLASS),
            51 => t(sym::CALLBACK),
            52 => t(sym::LINK),
            53 => t(sym::CLICK),
            54 => t(sym::NOTE_FOR),
            55 => t(sym::NOTE),
            56 => t(sym::ANNOTATION_START),
            57 => t(sym::ANNOTATION_END),
            58 => t(sym::HREF),
            60 => t(sym::GENERICTYPE),
            61 => {
                self.begin(St::Generic);
                Skip
            }
            63 => t(sym::BQUOTE_STR),
            64 => {
                self.begin(St::BqString);
                Skip
            }
            65..=68 => t(sym::LINK_TARGET),
            69 | 70 => t(sym::EXTENSION),
            71 | 72 => t(sym::DEPENDENCY),
            73 => t(sym::COMPOSITION),
            74 => t(sym::AGGREGATION),
            75 => t(sym::LOLLIPOP),
            76 => t(sym::LINE),
            77 => t(sym::DOTTED_LINE),
            78 => t(sym::LABEL),
            79 => t(sym::STYLE_SEPARATOR),
            80 => t(sym::MINUS),
            81 => t(sym::DOT),
            83 => t(sym::COLON),
            84 => t(sym::COMMA),
            85 | 86 => t(sym::BRKT),
            87 => t(sym::PCT),
            90 => t(sym::ALPHA),
            91 => t(sym::SQS),
            92 => t(sym::SQE),
            94 => t(sym::NUM),
            95 => t(sym::UNICODE_TEXT),
            96 | 97 => t(sym::SPACE),
            98 => t(sym::EOF),
            _ => unreachable!("lexer rule {rule}"),
        }
    }

    /// Length of the match of rule `r` at the current position.
    fn match_rule(&self, r: u8) -> Option<usize> {
        let s = self.rest();
        let run = |from: usize, f: &dyn Fn(char) -> bool| {
            s[from.min(s.len())..].iter().take_while(|&&c| f(c)).count()
        };
        // A literal, with `\b` after it when it ends with a word char.
        let lit = |w: &str| -> Option<usize> {
            let n = w.chars().count();
            if s.len() < n || !s.iter().zip(w.chars()).all(|(a, b)| *a == b) {
                return None;
            }
            if w.chars().last().is_some_and(is_word) && s.get(n).is_some_and(|&c| is_word(c)) {
                return None;
            }
            Some(n)
        };
        let ws_lit = |w: &str| -> Option<usize> {
            let n = run(0, &is_space);
            let m = w.chars().count();
            (s.len() >= n + m && s[n..n + m].iter().zip(w.chars()).all(|(a, b)| *a == b))
                .then_some(n + m)
        };
        let not_nl = |from: usize| run(from, &|c| c != '\n');
        match r {
            0 => self.direction("TB"),
            1 => self.direction("BT"),
            2 => self.direction("RL"),
            3 => self.direction("LR"),
            4 => {
                lit("%%")?;
                let n = 2 + not_nl(2);
                Some(n + run(n, &|c| c == '\r' || c == '\n'))
            }
            5 => {
                lit("%%")?;
                let mut n = 2 + not_nl(2);
                loop {
                    if s.get(n) == Some(&'\n') {
                        n += 1;
                    } else if s.get(n) == Some(&'\r') && s.get(n + 1) == Some(&'\n') {
                        n += 2;
                    } else {
                        return Some(n);
                    }
                }
            }
            6 | 8 | 10 => {
                let word = if r == 6 { "accTitle" } else { "accDescr" };
                let n = 8;
                (s.len() >= n && s.iter().zip(word.chars()).all(|(a, b)| *a == b)).then_some(())?;
                let n = n + run(n, &is_space);
                let sep = if r == 10 { '{' } else { ':' };
                (s.get(n) == Some(&sep)).then_some(())?;
                Some(n + 1 + run(n + 1, &is_space))
            }
            7 | 9 => Some(not_nl(0)),
            11 | 33 | 34 | 42 | 44 => lit("}"),
            12 => Some(run(0, &|c| c != '}')),
            13 | 30 | 36 | 40 => {
                let n = run(0, &is_space);
                s[..n].iter().rposition(|&c| c == '\n').map(|p| p + 1)
            }
            14 | 31 | 37 | 41 => {
                let n = run(0, &is_space);
                (n > 0).then_some(n)
            }
            15 => lit("classDiagram-v2"),
            16 => lit("classDiagram"),
            17 | 38 | 46 => lit("[*]"),
            18 => {
                lit("call")?;
                let n = run(4, &is_space);
                (n > 0).then_some(4 + n)
            }
            19 => {
                lit("(")?;
                let n = 1 + run(1, &is_space);
                (s.get(n) == Some(&')')).then_some(n + 1)
            }
            20 => lit("("),
            21 => Some(run(0, &|c| c != '(')),
            22 => lit(")"),
            23 => Some(run(0, &|c| c != ')')),
            24 | 26 => lit("\""),
            25 => Some(run(0, &|c| c != '"')),
            27 => lit("style"),
            28 => lit("classDef"),
            29 => lit("namespace"),
            32 | 43 | 47 => lit("{"),
            35 | 45 | 98 => s.is_empty().then_some(0),
            39 => lit("class"),
            48 => lit("\n"),
            49 => Some(run(0, &|c| !matches!(c, '{' | '}' | '\n'))),
            50 => lit("cssClass"),
            51 => lit("callback"),
            52 => lit("link"),
            53 => lit("click"),
            54 => lit("note for"),
            55 => lit("note"),
            56 => lit("<<"),
            57 => lit(">>"),
            58 => lit("href"),
            59 | 61 => lit("~"),
            60 => Some(run(0, &|c| c != '~')),
            62 | 64 => lit("`"),
            63 => {
                let n = run(0, &|c| c != '`');
                (n > 0).then_some(n)
            }
            65 => lit("_self"),
            66 => lit("_blank"),
            67 => lit("_parent"),
            68 => lit("_top"),
            69 => ws_lit("<|"),
            70 => ws_lit("|>"),
            71 => ws_lit(">"),
            72 => ws_lit("<"),
            73 => ws_lit("*"),
            74 => {
                let n = ws_lit("o")?;
                (!s.get(n).is_some_and(|&c| is_word(c))).then_some(n)
            }
            75 => ws_lit("()"),
            76 => lit("--"),
            77 => lit(".."),
            78 => {
                lit(":")?;
                let n = run(1, &|c| !matches!(c, ':' | '\n' | ';'));
                (n > 0).then_some(1 + n)
            }
            79 => lit(":::"),
            80 => lit("-"),
            81 => lit("."),
            82 => lit("+"),
            83 => lit(":"),
            84 => lit(","),
            85 | 86 => lit("#"),
            87 => lit("%"),
            88 | 89 => lit("="),
            90 => {
                let n = run(0, &is_word);
                (n > 0).then_some(n)
            }
            91 => lit("["),
            92 => lit("]"),
            93 => s
                .first()
                .is_some_and(|c| "!\"#$%&'*+,-.`?\\/".contains(*c))
                .then_some(1),
            94 => {
                let n = run(0, &|c| c.is_ascii_digit());
                (n > 0).then_some(n)
            }
            95 => s.first().is_some_and(|&c| is_unicode_text(c)).then_some(1),
            96 | 97 => s.first().is_some_and(|&c| is_space(c)).then_some(1),
            _ => unreachable!("lexer rule {r}"),
        }
    }

    /// `.*direction\s+XX[^\n]*`: the last `direction` on the line wins.
    fn direction(&self, dir: &str) -> Option<usize> {
        let s = self.rest();
        let line = s.iter().position(|&c| is_line_end(c)).unwrap_or(s.len());
        let word: Vec<char> = "direction".chars().collect();
        let d: Vec<char> = dir.chars().collect();
        (0..=line).rev().find_map(|p| {
            if !s[p..].starts_with(&word) {
                return None;
            }
            let mut i = p + word.len();
            let ws = s[i..].iter().take_while(|&&c| is_space(c)).count();
            if ws == 0 {
                return None;
            }
            i += ws;
            if !s[i..].starts_with(&d) {
                return None;
            }
            i += 2;
            Some(i + s[i..].iter().take_while(|&&c| c != '\n').count())
        })
    }
}

// ---------------------------------------------------------------------------
// Grammar actions

/// A relation end: a `RelationKind` index, or `none`.
type End = Option<u8>;

#[derive(Debug, Clone, Copy)]
struct RelSpec {
    type1: End,
    type2: End,
    line: u8,
}

#[derive(Debug, Clone)]
struct RelStmt {
    id1: String,
    id2: String,
    relation: RelSpec,
    title1: String,
    title2: String,
    title: Option<String>,
}

/// Values of Jison's value stack.
#[derive(Debug, Clone, Default)]
enum V {
    #[default]
    Undefined,
    Str(String),
    List(Vec<String>),
    /// Class and note ids of a namespace body.
    Stmts(Vec<String>, Vec<String>),
    Rel(Box<RelStmt>),
    Spec(RelSpec),
    Num(u8),
}

impl V {
    fn s(&self) -> String {
        match self {
            V::Str(s) => s.clone(),
            V::Num(n) => n.to_string(),
            V::List(l) => l.join(","),
            _ => "undefined".into(),
        }
    }

    fn opt(&self) -> Option<String> {
        match self {
            V::Undefined => None,
            v => Some(v.s()),
        }
    }

    fn list(self) -> Vec<String> {
        match self {
            V::List(l) => l,
            V::Str(s) => vec![s],
            _ => Vec::new(),
        }
    }
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    db: Db,
}

impl Grammar for Parser<'_> {
    type Value = V;

    fn lex(&mut self) -> std::result::Result<Token, usize> {
        self.lexer.lex()
    }

    fn token_value(&mut self, token: &Token) -> V {
        V::Str(token.text.clone())
    }

    fn reduce(&mut self, prod: usize, rhs: Vec<V>) -> std::result::Result<V, String> {
        let db = &mut self.db;
        let mut rhs = rhs;
        // `$n`, 1-based.
        let a = |rhs: &[V], n: usize| rhs[n - 1].clone();
        let s = |rhs: &[V], n: usize| rhs[n - 1].s();
        let first = rhs.first().cloned().unwrap_or_default();
        let v = match prod {
            8 => a(&rhs, 2),
            11 | 14 => V::Str(format!("{}.{}", s(&rhs, 1), s(&rhs, 3))),
            12 | 16 | 110 => V::Str(format!("{}{}", s(&rhs, 1), s(&rhs, 2))),
            17 | 18 => V::Str(format!("{}~{}~", s(&rhs, 1), s(&rhs, 2))),
            19 | 20 => {
                let V::Rel(mut rel) = std::mem::take(&mut rhs[0]) else {
                    return Err("bad relation".into());
                };
                if prod == 20 {
                    rel.title = Some(db.cleanup_label(&s(&rhs, 2)));
                }
                db.add_relation(*rel);
                V::Undefined
            }
            31 | 32 => {
                let t = js_trim(&s(&rhs, 2)).to_string();
                if prod == 31 {
                    db.set_acc_title(&t);
                } else {
                    db.set_acc_descr(&t);
                }
                V::Str(t)
            }
            33 => {
                let t = js_trim(&s(&rhs, 1)).to_string();
                db.set_acc_descr(&t);
                V::Str(t)
            }
            34 | 35 => {
                let n = rhs.len() - 2;
                let body = std::mem::take(&mut rhs[n]);
                let (classes, notes) = match body {
                    V::Stmts(c, n) => (c, n),
                    _ => (Vec::new(), Vec::new()),
                };
                db.add_classes_to_namespace(&s(&rhs, 1), &classes, &notes)?;
                db.namespace_stack.pop();
                first
            }
            36 => V::Str(db.add_namespace(&s(&rhs, 2), None)),
            37 => V::Str(db.add_namespace(&s(&rhs, 2), rhs[2].opt())),
            38 | 39 => V::Stmts(vec![s(&rhs, 1)], Vec::new()),
            41 | 42 => V::Stmts(Vec::new(), vec![s(&rhs, 1)]),
            40 | 43 => {
                let id = s(&rhs, 1);
                match std::mem::take(&mut rhs[2]) {
                    V::Stmts(mut c, mut n) => {
                        if prod == 40 {
                            c.insert(0, id);
                        } else {
                            n.insert(0, id);
                        }
                        V::Stmts(c, n)
                    }
                    other => other,
                }
            }
            44 | 45 => V::Stmts(Vec::new(), Vec::new()),
            46 => std::mem::take(&mut rhs[2]),
            48 => {
                db.set_css_class(&s(&rhs, 1), &s(&rhs, 3));
                first
            }
            49 => {
                db.add_members(&s(&rhs, 1), std::mem::take(&mut rhs[2]).list())?;
                first
            }
            51 => {
                db.set_css_class(&s(&rhs, 1), &s(&rhs, 3));
                db.add_members(&s(&rhs, 1), std::mem::take(&mut rhs[4]).list())?;
                first
            }
            52 | 54 => {
                db.add_annotation(&s(&rhs, 1), &s(&rhs, 3))?;
                first
            }
            53 => {
                db.add_annotation(&s(&rhs, 1), &s(&rhs, 3))?;
                db.add_members(&s(&rhs, 1), std::mem::take(&mut rhs[5]).list())?;
                first
            }
            55 => {
                db.add_class(&s(&rhs, 2));
                a(&rhs, 2)
            }
            56 => {
                db.add_class(&s(&rhs, 2));
                db.set_class_label(&s(&rhs, 2), &s(&rhs, 3))?;
                a(&rhs, 2)
            }
            60 => {
                db.add_annotation(&s(&rhs, 4), &s(&rhs, 2))?;
                first
            }
            61 | 74 | 107 => V::List(vec![s(&rhs, 1)]),
            62 => {
                let mut l = std::mem::take(&mut rhs[1]).list();
                l.push(s(&rhs, 1));
                V::List(l)
            }
            64 => {
                let label = db.cleanup_label(&s(&rhs, 2));
                db.add_member(&s(&rhs, 1), &label)?;
                first
            }
            67..=70 => {
                let (spec, t1, t2, id2) = match prod {
                    67 => (&rhs[1], "none".into(), "none".into(), 3),
                    68 => (&rhs[2], s(&rhs, 2), "none".into(), 4),
                    69 => (&rhs[1], "none".into(), s(&rhs, 3), 4),
                    _ => (&rhs[2], s(&rhs, 2), s(&rhs, 4), 5),
                };
                let V::Spec(relation) = spec else {
                    return Err("bad relation".into());
                };
                V::Rel(Box::new(RelStmt {
                    id1: s(&rhs, 1),
                    id2: s(&rhs, id2),
                    relation: *relation,
                    title1: t1,
                    title2: t2,
                    title: None,
                }))
            }
            71 => V::Str(db.add_note(s(&rhs, 3), Some(s(&rhs, 2)))),
            72 => V::Str(db.add_note(s(&rhs, 2), None)),
            73 => {
                let ids = std::mem::take(&mut rhs[1]).list();
                let styles = std::mem::take(&mut rhs[2]).list();
                db.define_class(&ids, &styles);
                first
            }
            75 | 108 => {
                let mut l = std::mem::take(&mut rhs[0]).list();
                l.push(s(&rhs, 3));
                V::List(l)
            }
            76 => db.set_direction("TB"),
            77 => db.set_direction("BT"),
            78 => db.set_direction("RL"),
            79 => db.set_direction("LR"),
            80..=83 => {
                let num = |v: &V| match v {
                    V::Num(n) => Some(*n),
                    _ => None,
                };
                let (type1, line, type2) = match prod {
                    80 => (num(&rhs[0]), &rhs[1], num(&rhs[2])),
                    81 => (None, &rhs[0], num(&rhs[1])),
                    82 => (num(&rhs[0]), &rhs[1], None),
                    _ => (None, &rhs[0], None),
                };
                V::Spec(RelSpec {
                    type1,
                    type2,
                    line: num(line).unwrap_or(0),
                })
            }
            84..=88 => V::Num((prod - 84) as u8),
            89 => V::Num(0),
            90 => V::Num(1),
            91 | 97 => {
                db.set_click_event(&s(&rhs, 2));
                first
            }
            92 | 98 => {
                db.set_click_event(&s(&rhs, 2));
                db.set_tooltip(&s(&rhs, 2), rhs[3].opt());
                first
            }
            99 => {
                db.set_click_event(&s(&rhs, 2));
                first
            }
            100 => {
                db.set_click_event(&s(&rhs, 2));
                db.set_tooltip(&s(&rhs, 2), rhs[4].opt());
                first
            }
            93 => {
                db.set_link(&s(&rhs, 2), &s(&rhs, 3), None);
                first
            }
            94 => {
                db.set_link(&s(&rhs, 2), &s(&rhs, 3), rhs[3].opt());
                first
            }
            95 => {
                db.set_link(&s(&rhs, 2), &s(&rhs, 3), None);
                db.set_tooltip(&s(&rhs, 2), rhs[3].opt());
                first
            }
            96 => {
                db.set_link(&s(&rhs, 2), &s(&rhs, 3), rhs[4].opt());
                db.set_tooltip(&s(&rhs, 2), rhs[3].opt());
                first
            }
            101 => {
                db.set_link(&s(&rhs, 2), &s(&rhs, 4), None);
                first
            }
            102 => {
                db.set_link(&s(&rhs, 2), &s(&rhs, 4), rhs[4].opt());
                first
            }
            103 => {
                db.set_link(&s(&rhs, 2), &s(&rhs, 4), None);
                db.set_tooltip(&s(&rhs, 2), rhs[4].opt());
                first
            }
            104 => {
                db.set_link(&s(&rhs, 2), &s(&rhs, 4), rhs[5].opt());
                db.set_tooltip(&s(&rhs, 2), rhs[4].opt());
                first
            }
            105 => {
                let styles = std::mem::take(&mut rhs[2]).list();
                db.set_css_style(&s(&rhs, 2), &styles);
                first
            }
            106 => {
                db.set_css_class(&s(&rhs, 2), &s(&rhs, 3));
                first
            }
            // `$$ = $1`, undefined for an empty right-hand side.
            _ => first,
        };
        Ok(v)
    }
}

// ---------------------------------------------------------------------------
// ClassDB

const DOM_ID_PREFIX: &str = "classId-";

#[derive(Default)]
struct Db {
    html_labels: bool,
    direction: String,
    classes: Vec<Class>,
    class_index: HashMap<String, usize>,
    relations: Vec<Relation>,
    notes: Vec<Note>,
    namespaces: Vec<Namespace>,
    namespace_index: HashMap<String, usize>,
    namespace_stack: Vec<String>,
    style_classes: Vec<StyleClass>,
    interfaces: Vec<Interface>,
    acc_title: Option<String>,
    acc_descr: Option<String>,
}

impl Db {
    /// `common.sanitizeText` with the `strict` security level.
    fn sanitize(&self, s: &str) -> String {
        if s.is_empty() || !s.contains('<') {
            return s.to_string();
        }
        if self.html_labels {
            let once = crate::purify::sanitize(s);
            crate::purify::sanitize(&once)
        } else {
            crate::purify::sanitize(s)
        }
    }

    fn class_mut(&mut self, id: &str) -> Option<&mut Class> {
        let i = *self.class_index.get(id)?;
        Some(&mut self.classes[i])
    }

    /// `splitClassNameAndType`: `(className, type)`.
    fn split_name(&self, id: &str) -> (String, String) {
        let id = self.sanitize(id);
        match id.find('~') {
            Some(i) if i > 0 => {
                let mut parts = id.split('~');
                let name = self.sanitize(parts.next().unwrap_or(""));
                let ty = self.sanitize(parts.next().unwrap_or(""));
                (name, ty)
            }
            _ => (id, String::new()),
        }
    }

    fn set_class_label(&mut self, id: &str, label: &str) -> std::result::Result<(), String> {
        let id = self.sanitize(id);
        let label = self.sanitize(label);
        let (name, _) = self.split_name(&id);
        let class = self
            .class_mut(&name)
            .ok_or_else(|| format!("class `{name}` is not defined"))?;
        class.text = if class.generic.is_empty() {
            label.clone()
        } else {
            format!("{label}<{}>", class.generic)
        };
        class.label = label;
        Ok(())
    }

    fn add_class(&mut self, id: &str) {
        let id = self.sanitize(id);
        let (name, ty) = self.split_name(&id);
        if self.class_index.contains_key(&name) {
            return;
        }
        let name = self.sanitize(&name);
        let text = if ty.is_empty() {
            name.clone()
        } else {
            format!("{name}&lt;{ty}&gt;")
        };
        let dom_id = format!("{DOM_ID_PREFIX}{name}-{}", self.classes.len());
        let class = Class {
            id: name.clone(),
            generic: ty,
            label: name.clone(),
            text,
            css_classes: "default".into(),
            members: Vec::new(),
            methods: Vec::new(),
            annotations: Vec::new(),
            styles: Vec::new(),
            dom_id,
            parent: None,
            link: None,
            link_target: None,
            have_callback: false,
            tooltip: None,
        };
        match self.class_index.get(&name) {
            Some(&i) => self.classes[i] = class,
            None => {
                self.class_index.insert(name, self.classes.len());
                self.classes.push(class);
            }
        }
    }

    fn add_interface(&mut self, label: String, class_id: String) {
        self.interfaces.push(Interface {
            id: format!("interface{}", self.interfaces.len()),
            label,
            class_id,
        });
    }

    fn add_relation(&mut self, mut rel: RelStmt) {
        const LOLLIPOP: u8 = 4;
        let spec = rel.relation;
        if spec.type1 == Some(LOLLIPOP) && spec.type2.is_none() {
            self.add_class(&rel.id2);
            self.add_interface(rel.id1.clone(), rel.id2.clone());
            rel.id1 = format!("interface{}", self.interfaces.len() - 1);
        } else if spec.type2 == Some(LOLLIPOP) && spec.type1.is_none() {
            self.add_class(&rel.id1);
            self.add_interface(rel.id2.clone(), rel.id1.clone());
            rel.id2 = format!("interface{}", self.interfaces.len() - 1);
        } else {
            self.add_class(&rel.id1);
            self.add_class(&rel.id2);
        }
        let kind = |t: End| {
            t.map(|n| match n {
                0 => RelationKind::Aggregation,
                1 => RelationKind::Extension,
                2 => RelationKind::Composition,
                3 => RelationKind::Dependency,
                _ => RelationKind::Lollipop,
            })
        };
        self.relations.push(Relation {
            id1: self.split_name(&rel.id1).0,
            id2: self.split_name(&rel.id2).0,
            type1: kind(spec.type1),
            type2: kind(spec.type2),
            dotted: spec.line == 1,
            title1: self.sanitize(js_trim(&rel.title1)),
            title2: self.sanitize(js_trim(&rel.title2)),
            title: rel.title,
        });
    }

    fn add_annotation(&mut self, class: &str, annotation: &str) -> std::result::Result<(), String> {
        let (name, _) = self.split_name(class);
        self.class_mut(&name)
            .ok_or_else(|| format!("class `{name}` is not defined"))?
            .annotations
            .push(annotation.to_string());
        Ok(())
    }

    fn add_member(&mut self, class: &str, member: &str) -> std::result::Result<(), String> {
        self.add_class(class);
        let (name, _) = self.split_name(class);
        let m = js_trim(member);
        let html = self.html_labels;
        let sanitized = |s: &str| {
            let db = Db {
                html_labels: html,
                ..Db::default()
            };
            db.sanitize(s)
        };
        enum Add {
            Annotation(String),
            Method(Member),
            Attribute(Member),
        }
        let add = if m.starts_with("<<") && m.ends_with(">>") {
            let chars: Vec<char> = m.chars().collect();
            let (a, b) = (2.min(chars.len()), chars.len().saturating_sub(2));
            let (a, b) = if a > b { (b, a) } else { (a, b) };
            Some(Add::Annotation(sanitized(
                &chars[a..b].iter().collect::<String>(),
            )))
        } else if m.find(')').is_some_and(|i| i > 0) {
            Some(Add::Method(
                class_member(&sanitized(m), MemberKind::Method).ok_or("cannot read the method")?,
            ))
        } else if !m.is_empty() {
            Some(Add::Attribute(
                class_member(&sanitized(m), MemberKind::Attribute)
                    .ok_or("cannot read the attribute")?,
            ))
        } else {
            None
        };
        let class = self
            .class_mut(&name)
            .ok_or_else(|| format!("class `{name}` is not defined"))?;
        match add {
            Some(Add::Annotation(a)) => class.annotations.push(a),
            Some(Add::Method(x)) => class.methods.push(x),
            Some(Add::Attribute(x)) => class.members.push(x),
            None => {}
        }
        Ok(())
    }

    fn add_members(
        &mut self,
        class: &str,
        mut members: Vec<String>,
    ) -> std::result::Result<(), String> {
        members.reverse();
        for m in &members {
            self.add_member(class, m)?;
        }
        Ok(())
    }

    fn add_note(&mut self, text: String, class: Option<String>) -> String {
        let id = format!("note{}", self.notes.len());
        self.notes.push(Note {
            id: id.clone(),
            class,
            text,
            parent: None,
        });
        id
    }

    fn cleanup_label(&self, label: &str) -> String {
        let label = label.strip_prefix(':').unwrap_or(label);
        self.sanitize(js_trim(label))
    }

    /// Ids of `setCssClass`/`setLink`: a leading digit gets the dom prefix.
    fn lookup_ids(&self, ids: &str) -> Vec<String> {
        ids.split(',')
            .map(|id| {
                if id.starts_with(|c: char| c.is_ascii_digit()) {
                    self.split_name(&format!("{DOM_ID_PREFIX}{id}")).0
                } else {
                    self.split_name(id).0
                }
            })
            .collect()
    }

    fn set_css_class(&mut self, ids: &str, class_name: &str) {
        for id in self.lookup_ids(ids) {
            if let Some(c) = self.class_mut(&id) {
                c.css_classes.push(' ');
                c.css_classes.push_str(class_name);
            }
        }
    }

    fn define_class(&mut self, ids: &[String], styles: &[String]) {
        for id in ids {
            let i = match self.style_classes.iter().position(|s| s.id == *id) {
                Some(i) => i,
                None => {
                    self.style_classes.push(StyleClass {
                        id: id.clone(),
                        styles: Vec::new(),
                        text_styles: Vec::new(),
                    });
                    self.style_classes.len() - 1
                }
            };
            let sc = &mut self.style_classes[i];
            for s in styles {
                if s.contains("color") {
                    sc.text_styles.push(s.replacen("fill", "bgFill", 1));
                }
                sc.styles.push(s.clone());
            }
            for c in &mut self.classes {
                if c.css_classes.contains(id.as_str()) {
                    c.styles
                        .extend(styles.iter().flat_map(|s| s.split(',').map(String::from)));
                }
            }
        }
    }

    fn set_tooltip(&mut self, ids: &str, tooltip: Option<String>) {
        let Some(tooltip) = tooltip else { return };
        let tooltip = self.sanitize(&tooltip);
        for id in ids.split(',') {
            let (name, _) = self.split_name(id);
            if let Some(c) = self.class_mut(&name) {
                c.tooltip = Some(tooltip.clone());
            }
        }
    }

    fn set_link(&mut self, ids: &str, link: &str, target: Option<String>) {
        let url = crate::flowchart::format_url(link);
        let target = target.map(|t| self.sanitize(&t));
        for id in self.lookup_ids(ids) {
            if let Some(c) = self.class_mut(&id) {
                c.link = url.clone();
                c.link_target = Some(target.clone().unwrap_or_else(|| "_blank".into()));
            }
        }
        self.set_css_class(ids, "clickable");
    }

    fn set_click_event(&mut self, ids: &str) {
        for id in ids.split(',') {
            let (name, _) = self.split_name(id);
            if let Some(c) = self.class_mut(&name) {
                c.have_callback = true;
            }
        }
        self.set_css_class(ids, "clickable");
    }

    fn set_css_style(&mut self, id: &str, styles: &[String]) {
        let Some(c) = self.class_mut(id) else { return };
        for s in styles {
            c.styles.extend(s.split(',').map(String::from));
        }
    }

    fn set_direction(&mut self, dir: &str) -> V {
        self.direction = dir.into();
        V::Undefined
    }

    fn set_acc_title(&mut self, t: &str) {
        let t = self.sanitize(t);
        self.acc_title = Some(t.trim_start_matches(is_space).to_string());
    }

    fn set_acc_descr(&mut self, t: &str) {
        let t = self.sanitize(t);
        // `replace(/\n\s+/g, '\n')`
        let mut out = String::with_capacity(t.len());
        let mut it = t.chars().peekable();
        while let Some(c) = it.next() {
            out.push(c);
            if c == '\n' {
                while it.peek().is_some_and(|&n| is_space(n)) {
                    it.next();
                }
            }
        }
        self.acc_descr = Some(out);
    }

    fn add_namespace(&mut self, id: &str, label: Option<String>) -> String {
        let qualified = match self.namespace_stack.last() {
            Some(p) if !p.is_empty() => format!("{p}.{id}"),
            _ => id.to_string(),
        };
        self.namespace_stack.push(qualified.clone());
        let label = label.filter(|l| !l.is_empty());
        if let Some(&i) = self.namespace_index.get(&qualified) {
            let ns = &mut self.namespaces[i];
            ns.explicit = true;
            if let Some(l) = label {
                ns.label = l;
            }
            return qualified;
        }
        let parts: Vec<&str> = qualified.split('.').collect();
        let mut ancestors: Vec<String> = Vec::with_capacity(parts.len());
        for (i, p) in parts.iter().enumerate() {
            ancestors.push(if i == 0 {
                p.to_string()
            } else {
                format!("{}.{p}", ancestors[i - 1])
            });
        }
        for i in 0..ancestors.len() {
            let current = &ancestors[i];
            let parent = (i > 0).then(|| ancestors[i - 1].clone());
            let leaf = i == ancestors.len() - 1;
            match self.namespace_index.get(current) {
                None => {
                    let node_label = match (&label, leaf) {
                        (Some(l), true) => l.clone(),
                        _ => parts[i].to_string(),
                    };
                    self.namespace_index
                        .insert(current.clone(), self.namespaces.len());
                    let n = self.namespaces.len();
                    self.namespaces.push(Namespace {
                        id: current.clone(),
                        label: node_label,
                        dom_id: format!("{DOM_ID_PREFIX}{current}-{n}"),
                        parent: parent.clone(),
                        explicit: leaf,
                        classes: Vec::new(),
                        notes: Vec::new(),
                        children: Vec::new(),
                    });
                }
                Some(&k) if leaf => self.namespaces[k].explicit = true,
                Some(_) => {}
            }
            if let Some(p) = parent {
                let (Some(&pi), Some(&ci)) = (
                    self.namespace_index.get(&p),
                    self.namespace_index.get(current),
                ) else {
                    continue;
                };
                if !self.namespaces[pi].children.contains(current) {
                    self.namespaces[pi].children.push(current.clone());
                }
                self.namespaces[ci].parent.get_or_insert(p);
            }
        }
        qualified
    }

    fn add_classes_to_namespace(
        &mut self,
        id: &str,
        classes: &[String],
        notes: &[String],
    ) -> std::result::Result<(), String> {
        let Some(&ni) = self.namespace_index.get(id) else {
            return Ok(());
        };
        for c in classes {
            let (name, _) = self.split_name(c);
            let class = self
                .class_mut(&name)
                .ok_or_else(|| format!("class `{name}` is not defined"))?;
            class.parent = Some(id.to_string());
            let ns = &mut self.namespaces[ni];
            if !ns.classes.contains(&name) {
                ns.classes.push(name);
            }
        }
        for n in notes {
            let note = self
                .notes
                .iter_mut()
                .find(|x| x.id == *n)
                .ok_or_else(|| format!("note `{n}` is not defined"))?;
            note.parent = Some(id.to_string());
            let ns = &mut self.namespaces[ni];
            if !ns.notes.contains(n) {
                ns.notes.push(n.clone());
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ClassMember

/// `new ClassMember(input, kind)`, on an already sanitized input. `None`
/// where Mermaid throws (a method without `(`).
fn class_member(input: &str, kind: MemberKind) -> Option<Member> {
    let mut visibility = String::new();
    let mut classifier = String::new();
    let mut parameters = String::new();
    let mut return_type = String::new();
    let id;
    let c: Vec<char> = input.chars().collect();
    if kind == MemberKind::Method {
        let m = method_regex(&c)?;
        let vis = m.vis.map(|v| v.to_string()).unwrap_or_default();
        if matches!(vis.as_str(), "#" | "+" | "~" | "-" | "") {
            visibility = vis;
        }
        id = m.id;
        parameters = js_trim(&m.params).to_string();
        let mut potential = m
            .classifier
            .map_or(String::new(), |c| js_trim(&c.to_string()).to_string());
        return_type = js_trim(&m.rest).to_string();
        if potential.is_empty() {
            if let Some(last) = return_type
                .chars()
                .last()
                .filter(|c| matches!(c, '$' | '*'))
            {
                potential = last.to_string();
                return_type.pop();
            }
        }
        classifier = potential;
    } else {
        let first = c.first().copied();
        let last = c.last().copied();
        if let Some(f) = first.filter(|f| matches!(f, '#' | '+' | '~' | '-')) {
            visibility = f.to_string();
        }
        if let Some(l) = last.filter(|l| matches!(l, '$' | '*')) {
            classifier = l.to_string();
        }
        let a = usize::from(!visibility.is_empty());
        let b = if classifier.is_empty() {
            c.len()
        } else {
            c.len() - 1
        };
        let (a, b) = if a > b { (b, a) } else { (a, b) };
        id = c[a..b].iter().collect::<String>();
    }
    let id = if id.starts_with(' ') {
        format!(" {}", js_trim(&id))
    } else {
        js_trim(&id).to_string()
    };
    let mut combined = String::new();
    if !visibility.is_empty() {
        combined.push('\\');
        combined.push_str(&visibility);
    }
    combined.push_str(&parse_generic_types(&id));
    if kind == MemberKind::Method {
        combined.push('(');
        combined.push_str(&parse_generic_types(&parameters));
        combined.push(')');
        if !return_type.is_empty() {
            combined.push_str(" : ");
            combined.push_str(&parse_generic_types(&return_type));
        }
    }
    let mut text = combined.replace('<', "&lt;").replace('>', "&gt;");
    if text.starts_with("\\&lt;") {
        text = text.replacen("\\&lt;", "~", 1);
    }
    Some(Member {
        kind,
        visibility,
        classifier,
        id,
        parameters,
        return_type,
        text,
    })
}

struct MethodMatch {
    vis: Option<char>,
    id: String,
    params: String,
    classifier: Option<char>,
    rest: String,
}

/// `/([#+~-])?(.+)\((.*)\)([\s$*])?(.*)([$*])?/.exec(input)`.
fn method_regex(c: &[char]) -> Option<MethodMatch> {
    for st in 0..c.len() {
        // `.` stops at line ends.
        let end = c[st..]
            .iter()
            .position(|&x| is_line_end(x))
            .map_or(c.len(), |p| st + p);
        let close = c[st..end].iter().rposition(|&x| x == ')').map(|p| st + p);
        let Some(close) = close else { continue };
        let vis_options: &[bool] = if matches!(c[st], '#' | '+' | '~' | '-') {
            &[true, false]
        } else {
            &[false]
        };
        for &take in vis_options {
            let a = st + usize::from(take);
            // `(.+)` is greedy: the last `(` with a `)` after it.
            let Some(open) = c[a..close]
                .iter()
                .rposition(|&x| x == '(')
                .map(|p| a + p)
                .filter(|&o| o > a)
            else {
                continue;
            };
            let mut i = close + 1;
            let classifier = c
                .get(i)
                .copied()
                .filter(|&x| is_space(x) || x == '$' || x == '*');
            if classifier.is_some() {
                i += 1;
            }
            let rest_end = c[i.min(c.len())..]
                .iter()
                .position(|&x| is_line_end(x))
                .map_or(c.len(), |p| i + p);
            return Some(MethodMatch {
                vis: take.then_some(c[st]),
                id: c[a..open].iter().collect(),
                params: c[open + 1..close].iter().collect(),
                classifier,
                rest: c[i.min(c.len())..rest_end.max(i.min(c.len()))]
                    .iter()
                    .collect(),
            });
        }
    }
    None
}

/// Mermaid's `parseGenericTypes`: `List~int~` gives `List<int>`.
pub(crate) fn parse_generic_types(input: &str) -> String {
    // `split(/(,)/)` keeps the commas as their own items.
    let mut sets: Vec<&str> = Vec::new();
    for (i, part) in input.split(',').enumerate() {
        if i > 0 {
            sets.push(",");
        }
        sets.push(part);
    }
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < sets.len() {
        let mut this = sets[i].to_string();
        if this == "," && i > 0 && i + 1 < sets.len() {
            let (prev, next) = (sets[i - 1], sets[i + 1]);
            if prev.matches('~').count() == 1 && next.matches('~').count() == 1 {
                this = format!("{prev},{next}");
                i += 1;
                out.pop();
            }
        }
        out.push(process_set(&this));
        i += 1;
    }
    out.concat()
}

fn process_set(input: &str) -> String {
    let tildes = input.matches('~').count();
    if tildes <= 1 {
        return input.to_string();
    }
    let mut s = input;
    let mut starting = false;
    if tildes % 2 != 0 && s.starts_with('~') {
        s = &s[1..];
        starting = true;
    }
    let mut chars: Vec<char> = s.chars().collect();
    loop {
        let first = chars.iter().position(|&c| c == '~');
        let last = chars.iter().rposition(|&c| c == '~');
        match (first, last) {
            (Some(f), Some(l)) if f != l => {
                chars[f] = '<';
                chars[l] = '>';
            }
            _ => break,
        }
    }
    let mut out = String::new();
    if starting {
        out.push('~');
    }
    out.extend(chars);
    out
}

include!("unicode_text.rs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_types() {
        assert_eq!(parse_generic_types("List~int~"), "List<int>");
        assert_eq!(parse_generic_types("Map~K, V~"), "Map<K, V>");
        assert_eq!(parse_generic_types("List~List~int~~"), "List<List<int>>");
    }

    #[test]
    fn methods() {
        let m = class_member("+getName(int id) String$", MemberKind::Method).unwrap();
        assert_eq!(m.visibility, "+");
        assert_eq!(m.id, "getName");
        assert_eq!(m.parameters, "int id");
        assert_eq!(m.return_type, "String");
        assert_eq!(m.classifier, "$");
    }

    #[test]
    fn parses_basic() {
        let d = parse("classDiagram\nA <|-- B : x\nclass A {\n +int a\n foo()\n}").unwrap();
        assert_eq!(d.classes.len(), 2);
        assert_eq!(d.classes[0].members.len(), 1);
        assert_eq!(d.classes[0].methods.len(), 1);
        assert_eq!(d.relations[0].type1, Some(RelationKind::Extension));
    }
}
