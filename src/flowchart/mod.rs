//! Flowcharts (`flowchart` and `graph` headers).

mod parser;
mod render;

pub use parser::parse;
pub(crate) use render::render;

/// Main flow direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Direction {
    #[default]
    TopBottom,
    BottomTop,
    LeftRight,
    RightLeft,
}

impl Direction {
    /// Parses `TB`, `TD`, `BT`, `LR`, `RL` (and Mermaid's `v ^ > <` aliases).
    pub fn from_keyword(s: &str) -> Option<Self> {
        match s {
            "TB" | "TD" | "v" => Some(Self::TopBottom),
            "BT" | "^" => Some(Self::BottomTop),
            "LR" | ">" => Some(Self::LeftRight),
            "RL" | "<" => Some(Self::RightLeft),
            _ => None,
        }
    }

    pub fn is_horizontal(self) -> bool {
        matches!(self, Self::LeftRight | Self::RightLeft)
    }
}

/// Node shape, named after Mermaid's syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NodeShape {
    /// `A[text]`
    #[default]
    Rect,
    /// `A(text)`
    Round,
    /// `A([text])`
    Stadium,
    /// `A[[text]]`
    Subroutine,
    /// `A[(text)]`
    Cylinder,
    /// `A((text))`
    Circle,
    /// `A(((text)))`
    DoubleCircle,
    /// `A{text}`
    Diamond,
    /// `A{{text}}`
    Hexagon,
    /// `A>text]`
    Asymmetric,
    /// `A[/text/]`
    Parallelogram,
    /// `A[\text\]`
    ParallelogramAlt,
    /// `A[/text\]`
    Trapezoid,
    /// `A[\text/]`
    TrapezoidAlt,
}

/// A CSS-like property such as `fill:#f9f`.
pub type StyleProp = (String, String);

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Node {
    pub id: String,
    /// Label text; lines are separated by `\n`.
    pub label: String,
    pub shape: NodeShape,
    /// Classes from `:::name` and `class` statements.
    pub classes: Vec<String>,
    /// Properties from `style` statements.
    pub styles: Vec<StyleProp>,
    /// Innermost subgraph containing the node (index in [`Flowchart::subgraphs`]).
    pub subgraph: Option<usize>,
}

/// Line style of a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Stroke {
    /// `-->`, `---`
    #[default]
    Normal,
    /// `-.->`, `-.-`
    Dotted,
    /// `==>`, `===`
    Thick,
    /// `~~~`
    Invisible,
}

/// Marker at one end of a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Arrow {
    #[default]
    None,
    /// `>`
    Point,
    /// `o`
    Circle,
    /// `x`
    Cross,
}

/// One end of a link: a node, or a whole subgraph (`A --> sub1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeEnd {
    /// Index in [`Flowchart::nodes`].
    Node(usize),
    /// Index in [`Flowchart::subgraphs`].
    Subgraph(usize),
}

impl Default for EdgeEnd {
    fn default() -> Self {
        Self::Node(0)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Edge {
    pub from: EdgeEnd,
    pub to: EdgeEnd,
    pub label: Option<String>,
    pub stroke: Stroke,
    /// Marker at the source end (`<-->`, `o--o`, `x--x`).
    pub start: Arrow,
    /// Marker at the target end.
    pub end: Arrow,
    /// Minimum length in ranks (`-->` is 1, `--->` is 2, ...).
    pub length: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Subgraph {
    pub id: String,
    pub title: String,
    pub parent: Option<usize>,
    /// `direction` statement inside the subgraph (parsed, not used by the layout yet).
    pub direction: Option<Direction>,
}

/// A `classDef` statement.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClassDef {
    pub name: String,
    pub styles: Vec<StyleProp>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Flowchart {
    pub direction: Direction,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub subgraphs: Vec<Subgraph>,
    pub class_defs: Vec<ClassDef>,
}

impl Flowchart {
    /// Finds a node by its id.
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Id of an edge end (node or subgraph id).
    pub fn end_id(&self, end: EdgeEnd) -> &str {
        match end {
            EdgeEnd::Node(i) => &self.nodes[i].id,
            EdgeEnd::Subgraph(i) => &self.subgraphs[i].id,
        }
    }

    /// Finds a node index by its id.
    pub fn node_index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }
}
