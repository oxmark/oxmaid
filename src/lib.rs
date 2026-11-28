//! # oxmaid
//!
//! A Rust port of [Mermaid](https://mermaid.js.org): text to SVG diagrams,
//! without a browser or JavaScript.
//!
//! Ported so far: flowcharts (`flowchart` / `graph`), sequence diagrams
//! (`sequenceDiagram`) and class diagrams (`classDiagram`).
//!
//! ```
//! let svg = oxmaid::render("flowchart LR\n  A[Start] --> B{Ok?}\n  B -->|yes| C[End]").unwrap();
//! assert!(svg.starts_with("<svg"));
//! ```
#![forbid(unsafe_code)]

pub mod class;
pub mod config;
mod dom;
mod error;
pub mod flowchart;
mod font_metrics;
mod font_metrics_svg;
mod jison;
pub mod layout;
mod options;
mod purify;
pub mod sequence;
mod source;
mod svg;
pub mod text;

pub use error::{Error, ErrorKind};
pub use options::{RenderOptions, Theme};

pub type Result<T> = std::result::Result<T, Error>;

/// Diagram type recognized from the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagramKind {
    Flowchart,
    Sequence,
    Class,
}

/// A parsed diagram.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Diagram {
    Flowchart(flowchart::Flowchart),
    Sequence(sequence::SequenceDiagram),
    Class(class::ClassDiagram),
}

impl Diagram {
    pub fn kind(&self) -> DiagramKind {
        match self {
            Diagram::Flowchart(_) => DiagramKind::Flowchart,
            Diagram::Sequence(_) => DiagramKind::Sequence,
            Diagram::Class(_) => DiagramKind::Class,
        }
    }

    /// Renders the diagram to SVG.
    pub fn to_svg(&self, options: &RenderOptions) -> String {
        match self {
            Diagram::Flowchart(f) => flowchart::render(f, options),
            Diagram::Sequence(d) => sequence::render(d, options),
            Diagram::Class(d) => class::render(d, options),
        }
    }
}

/// Known Mermaid types that are not ported yet.
const NOT_YET_PORTED: &[&str] = &[
    "stateDiagram",
    "stateDiagram-v2",
    "erDiagram",
    "journey",
    "gantt",
    "pie",
    "quadrantChart",
    "requirementDiagram",
    "gitGraph",
    "C4Context",
    "C4Container",
    "C4Component",
    "C4Dynamic",
    "C4Deployment",
    "mindmap",
    "timeline",
    "zenuml",
    "sankey-beta",
    "xychart-beta",
    "block-beta",
    "packet-beta",
    "kanban",
    "architecture-beta",
    "radar-beta",
];

/// Detects the diagram type from its header.
pub fn detect(src: &str) -> Result<DiagramKind> {
    let lines = source::lines(src);
    let first = lines
        .first()
        .ok_or_else(|| Error::new(ErrorKind::Empty, 0, 0, "diagram is empty"))?;
    let keyword = first
        .text
        .split(|c: char| c.is_whitespace() || c == ';')
        .next()
        .unwrap_or("");
    match keyword {
        "flowchart" | "graph" | "flowchart-elk" | "swimlane-beta" => Ok(DiagramKind::Flowchart),
        "sequenceDiagram" => Ok(DiagramKind::Sequence),
        k if k.starts_with("classDiagram") => Ok(DiagramKind::Class),
        k if NOT_YET_PORTED.contains(&k) => Err(Error::new(
            ErrorKind::UnsupportedDiagram,
            first.number,
            first.offset + 1,
            format!("`{k}` diagrams are not supported yet"),
        )),
        k => Err(Error::new(
            ErrorKind::UnknownDiagram,
            first.number,
            first.offset + 1,
            format!("unknown diagram type `{k}`"),
        )),
    }
}

/// Parses a Mermaid source.
pub fn parse(src: &str) -> Result<Diagram> {
    match detect(src)? {
        DiagramKind::Flowchart => flowchart::parse(src).map(Diagram::Flowchart),
        DiagramKind::Sequence => sequence::parse(src).map(Diagram::Sequence),
        DiagramKind::Class => class::parse(src).map(Diagram::Class),
    }
}

/// Parses and renders a Mermaid source to SVG with default options.
pub fn render(src: &str) -> Result<String> {
    render_with(src, &RenderOptions::default())
}

/// Parses and renders a Mermaid source to SVG.
pub fn render_with(src: &str, options: &RenderOptions) -> Result<String> {
    Ok(parse(src)?.to_svg(options))
}
