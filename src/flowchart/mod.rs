//! Flowcharts (`flowchart` and `graph` headers).
//!
//! The syntax follows Mermaid 12.3 (`flow.jison` and `flowDb.ts`).

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
    /// Parses `TB`, `TD`, `BT`, `LR`, `RL` and Mermaid's `v ^ > <` aliases.
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

macro_rules! shapes {
    ($($(#[$doc:meta])* $variant:ident = $name:literal $(| $alias:literal)*;)*) => {
        /// Node shape. Each variant documents its Mermaid syntax; the string
        /// is the canonical `@{ shape: ... }` name.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
        pub enum NodeShape {
            #[default]
            $($(#[$doc])* $variant,)*
        }

        impl NodeShape {
            /// Canonical short name, as used by `@{ shape: ... }`.
            pub fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)*
                }
            }

            /// Looks up a shape by short name or alias (`@{ shape: ... }`).
            pub fn from_name(name: &str) -> Option<Self> {
                match name {
                    $($name $(| $alias)* => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

shapes! {
    /// `A[text]`, `rect`
    Rect = "rect" | "proc" | "process" | "rectangle" | "square";
    /// `A(text)`, `rounded`
    Rounded = "rounded" | "event" | "round";
    /// `A([text])`
    Stadium = "stadium" | "terminal" | "pill";
    /// `A[[text]]`
    Subroutine = "fr-rect" | "subprocess" | "subproc" | "framed-rectangle" | "subroutine";
    /// `A[(text)]`
    Cylinder = "cyl" | "db" | "database" | "cylinder";
    /// `A((text))`
    Circle = "circle" | "circ";
    /// `A(((text)))`
    DoubleCircle = "dbl-circ" | "double-circle" | "doublecircle";
    /// `A(-text-)`
    Ellipse = "ellipse";
    /// `A{text}`
    Diamond = "diam" | "decision" | "diamond" | "question";
    /// `A{{text}}`
    Hexagon = "hex" | "hexagon" | "prepare";
    /// `A>text]`
    Odd = "odd";
    /// `A[/text/]`
    LeanRight = "lean-r" | "lean-right" | "in-out" | "lean_right";
    /// `A[\text\]`
    LeanLeft = "lean-l" | "lean-left" | "out-in" | "lean_left";
    /// `A[/text\]`
    TrapezoidBottom = "trap-b" | "priority" | "trapezoid-bottom" | "trapezoid";
    /// `A[\text/]`
    TrapezoidTop = "trap-t" | "manual" | "trapezoid-top" | "inv-trapezoid" | "inv_trapezoid";
    DataStore = "datastore" | "data-store";
    Folder = "folder" | "directory";
    Bucket = "bucket";
    Console = "console";
    Browser = "browser";
    Person = "person";
    Bang = "bang";
    Cloud = "cloud";
    Text = "text";
    NotchedRect = "notch-rect" | "card" | "notched-rectangle";
    LinedRect = "lin-rect" | "lined-rectangle" | "lined-process" | "lin-proc" | "shaded-process";
    SmallCircle = "sm-circ" | "start" | "small-circle";
    FramedCircle = "fr-circ" | "stop" | "framed-circle";
    Fork = "fork" | "join";
    Hourglass = "hourglass" | "collate";
    Brace = "brace" | "comment" | "brace-l";
    BraceRight = "brace-r";
    Braces = "braces";
    Bolt = "bolt" | "com-link" | "lightning-bolt";
    Document = "doc" | "document";
    Delay = "delay" | "half-rounded-rectangle";
    HorizontalCylinder = "h-cyl" | "das" | "horizontal-cylinder";
    LinedCylinder = "lin-cyl" | "disk" | "lined-cylinder";
    CurvedTrapezoid = "curv-trap" | "curved-trapezoid" | "display";
    DividedRect = "div-rect" | "div-proc" | "divided-rectangle" | "divided-process";
    Triangle = "tri" | "extract" | "triangle";
    WindowPane = "win-pane" | "internal-storage" | "window-pane";
    FilledCircle = "f-circ" | "junction" | "filled-circle";
    NotchedPentagon = "notch-pent" | "loop-limit" | "notched-pentagon";
    FlippedTriangle = "flip-tri" | "manual-file" | "flipped-triangle";
    SlopedRect = "sl-rect" | "manual-input" | "sloped-rectangle";
    Documents = "docs" | "documents" | "st-doc" | "stacked-document";
    StackedRect = "st-rect" | "procs" | "processes" | "stacked-rectangle";
    BowTieRect = "bow-rect" | "stored-data" | "bow-tie-rectangle";
    CrossedCircle = "cross-circ" | "summary" | "crossed-circle";
    TaggedDocument = "tag-doc" | "tagged-document";
    TaggedRect = "tag-rect" | "tagged-rectangle" | "tag-proc" | "tagged-process";
    Flag = "flag" | "paper-tape";
    LinedDocument = "lin-doc" | "lined-document";
}

/// How a label's text is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LabelType {
    /// Plain text; `<br>` and a literal `\n` break lines, inline HTML is kept.
    #[default]
    Text,
    /// A `"quoted string"`.
    String,
    /// A ``"`markdown string`"``: `**bold**`, `*italic*`, automatic wrapping.
    Markdown,
}

/// A CSS-like property such as `fill:#f9f`.
pub type StyleProp = (String, String);

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Node {
    pub id: String,
    /// Label text; lines are separated by `\n`.
    pub label: String,
    pub label_type: LabelType,
    pub shape: NodeShape,
    /// Classes from `:::name` and `class` statements.
    pub classes: Vec<String>,
    /// Properties from `style` statements.
    pub styles: Vec<StyleProp>,
    /// Innermost subgraph containing the node (index in [`Flowchart::subgraphs`]).
    pub subgraph: Option<usize>,
    /// Target of a `click` link.
    pub link: Option<String>,
    pub link_target: Option<String>,
    pub tooltip: Option<String>,
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
    /// User id (`A e1@--> B`) or a generated `L_A_B_0`.
    pub id: String,
    pub from: EdgeEnd,
    pub to: EdgeEnd,
    pub label: Option<String>,
    pub label_type: LabelType,
    pub stroke: Stroke,
    /// Marker at the source end (`<-->`, `o--o`, `x--x`).
    pub start: Arrow,
    /// Marker at the target end.
    pub end: Arrow,
    /// Minimum length in ranks (`-->` is 1, `--->` is 2, ... at most 10).
    pub length: usize,
    /// Properties from `linkStyle`.
    pub styles: Vec<StyleProp>,
    /// Curve from `linkStyle ... interpolate` or `@{ curve: ... }`.
    pub curve: Option<String>,
    /// `@{ animate: true }`.
    pub animate: bool,
    /// `@{ animation: fast | slow }`.
    pub animation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Subgraph {
    pub id: String,
    pub title: String,
    pub label_type: LabelType,
    pub parent: Option<usize>,
    /// `direction` statement inside the subgraph.
    pub direction: Option<Direction>,
    pub classes: Vec<String>,
    pub styles: Vec<StyleProp>,
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
    /// `title` from the front matter.
    pub title: Option<String>,
    pub acc_title: Option<String>,
    pub acc_descr: Option<String>,
    /// `linkStyle default ...`.
    pub default_link_styles: Vec<StyleProp>,
    /// `linkStyle default interpolate ...`.
    pub default_curve: Option<String>,
    /// Theme, look and layout from the front matter or an `init` directive.
    pub config: crate::config::DiagramConfig,
}

impl Flowchart {
    /// Finds a node by its id.
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Finds a node index by its id.
    pub fn node_index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// Id of an edge end (node or subgraph id).
    pub fn end_id(&self, end: EdgeEnd) -> &str {
        match end {
            EdgeEnd::Node(i) => &self.nodes[i].id,
            EdgeEnd::Subgraph(i) => &self.subgraphs[i].id,
        }
    }
}
