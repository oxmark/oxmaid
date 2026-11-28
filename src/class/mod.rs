//! Class diagrams (`classDiagram`, `classDiagram-v2`).
//!
//! The model mirrors what Mermaid 12.3's `classDb` holds after parsing:
//! classes with their members and methods, relations, notes, namespaces and
//! `classDef` style classes, in declaration order.

mod parser;
mod render;
mod tables;

pub use parser::parse;
pub(crate) use render::render;

use crate::config::DiagramConfig;

/// A parsed class diagram.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ClassDiagram {
    /// `TB`, `BT`, `LR` or `RL`.
    pub direction: String,
    pub classes: Vec<Class>,
    pub relations: Vec<Relation>,
    pub notes: Vec<Note>,
    pub namespaces: Vec<Namespace>,
    /// `classDef` definitions.
    pub style_classes: Vec<StyleClass>,
    /// Lollipop interfaces (`A ()-- B`).
    pub interfaces: Vec<Interface>,
    pub title: Option<String>,
    pub acc_title: Option<String>,
    pub acc_descr: Option<String>,
    pub config: DiagramConfig,
}

/// A class box.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Class {
    pub id: String,
    /// Generic type, from `Name~T~`.
    pub generic: String,
    pub label: String,
    /// The label with its generic type (`Name&lt;T&gt;`).
    pub text: String,
    /// Space-separated CSS classes, starting with `default`.
    pub css_classes: String,
    pub members: Vec<Member>,
    pub methods: Vec<Member>,
    pub annotations: Vec<String>,
    pub styles: Vec<String>,
    /// `classId-<id>-<n>`: the element id Mermaid gives the class.
    pub dom_id: String,
    /// Enclosing namespace.
    pub parent: Option<String>,
    pub link: Option<String>,
    pub link_target: Option<String>,
    pub have_callback: bool,
    pub tooltip: Option<String>,
}

/// Whether a member is an attribute or a method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    Attribute,
    Method,
}

/// A class member, split the way Mermaid's `ClassMember` does.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Member {
    pub kind: MemberKind,
    /// `+`, `-`, `#`, `~` or empty.
    pub visibility: String,
    /// `$` (static) or `*` (abstract), or empty.
    pub classifier: String,
    pub id: String,
    pub parameters: String,
    pub return_type: String,
    /// Display text, HTML-escaped.
    pub text: String,
}

/// A relation end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    Aggregation,
    Extension,
    Composition,
    Dependency,
    Lollipop,
}

impl RelationKind {
    /// Mermaid's `relationType` number.
    pub fn index(self) -> u8 {
        match self {
            Self::Aggregation => 0,
            Self::Extension => 1,
            Self::Composition => 2,
            Self::Dependency => 3,
            Self::Lollipop => 4,
        }
    }
}

/// A relation between two classes.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Relation {
    pub id1: String,
    pub id2: String,
    pub type1: Option<RelationKind>,
    pub type2: Option<RelationKind>,
    /// `..` instead of `--`.
    pub dotted: bool,
    /// Cardinality next to `id1` (`"1"`), `none` when absent.
    pub title1: String,
    /// Cardinality next to `id2`, `none` when absent.
    pub title2: String,
    pub title: Option<String>,
}

/// A note, attached to a class or free.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Note {
    pub id: String,
    pub class: Option<String>,
    pub text: String,
    pub parent: Option<String>,
}

/// A namespace. Dotted names create their ancestors.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Namespace {
    pub id: String,
    pub label: String,
    pub dom_id: String,
    pub parent: Option<String>,
    /// Declared by the user rather than created as an ancestor.
    pub explicit: bool,
    pub classes: Vec<String>,
    pub notes: Vec<String>,
    pub children: Vec<String>,
}

/// A `classDef` style class.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct StyleClass {
    pub id: String,
    pub styles: Vec<String>,
    pub text_styles: Vec<String>,
}

/// A lollipop interface.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Interface {
    pub id: String,
    pub label: String,
    pub class_id: String,
}
