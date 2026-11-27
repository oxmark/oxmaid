//! # oxmaid
//!
//! A Rust port of [Mermaid](https://mermaid.js.org): text to SVG diagrams,
//! without a browser or JavaScript.
//!
//! Only flowcharts (`flowchart` / `graph`) are ported so far.
//!
//! ```
//! let svg = oxmaid::render("flowchart LR\n  A[Start] --> B{Ok?}\n  B -->|yes| C[End]").unwrap();
//! assert!(svg.starts_with("<svg"));
//! ```
#![forbid(unsafe_code)]

pub mod config;
mod error;
pub mod flowchart;
pub mod layout;
mod options;
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
}

/// A parsed diagram.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Diagram {
    Flowchart(flowchart::Flowchart),
}

impl Diagram {
    pub fn kind(&self) -> DiagramKind {
        match self {
            Diagram::Flowchart(_) => DiagramKind::Flowchart,
        }
    }

    /// Renders the diagram to SVG.
    pub fn to_svg(&self, options: &RenderOptions) -> String {
        match self {
            Diagram::Flowchart(f) => flowchart::render(f, options),
        }
    }
}

/// Known Mermaid types that are not ported yet.
const NOT_YET_PORTED: &[&str] = &[
    "sequenceDiagram",
    "classDiagram",
    "classDiagram-v2",
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
        "flowchart" | "graph" | "flowchart-elk" => Ok(DiagramKind::Flowchart),
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
