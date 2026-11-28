//! Sequence diagrams (`sequenceDiagram`).
//!
//! The model mirrors what Mermaid 12.3's `sequenceDb` holds after parsing:
//! participants in declaration order, and one flat list of messages where
//! notes, activations, blocks (`loop`, `alt`, ...) and `autonumber` changes
//! are entries too, in source order.

mod parser;
mod render;

pub use parser::parse;
pub use render::render;

use crate::config::DiagramConfig;

/// How a participant is drawn.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParticipantKind {
    Participant,
    Actor,
    Boundary,
    Control,
    Entity,
    Database,
    Collections,
    Queue,
    /// A `type` Mermaid does not know: the participant is not drawn.
    Other(String),
}

impl ParticipantKind {
    pub fn from_name(name: &str) -> Self {
        match name {
            "participant" => Self::Participant,
            "actor" => Self::Actor,
            "boundary" => Self::Boundary,
            "control" => Self::Control,
            "entity" => Self::Entity,
            "database" => Self::Database,
            "collections" => Self::Collections,
            "queue" => Self::Queue,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Participant => "participant",
            Self::Actor => "actor",
            Self::Boundary => "boundary",
            Self::Control => "control",
            Self::Entity => "entity",
            Self::Database => "database",
            Self::Collections => "collections",
            Self::Queue => "queue",
            Self::Other(s) => s,
        }
    }
}

/// A participant (`participant`, `actor`, or one created by a message).
#[derive(Debug, Clone, PartialEq)]
pub struct Participant {
    /// Key used in messages.
    pub id: String,
    pub name: String,
    /// Displayed label (the alias after `as`, or the name).
    pub description: String,
    pub wrap: bool,
    pub kind: ParticipantKind,
    /// Index in [`SequenceDiagram::boxes`].
    pub group: Option<usize>,
    /// Participant declared just before this one, as Mermaid tracks it.
    pub prev: Option<String>,
    /// Participant declared just after this one.
    pub next: Option<String>,
    /// Popup menu links (`link` / `links`), label to URL.
    pub links: Vec<(String, String)>,
    /// `class` from `properties`.
    pub class: Option<String>,
    /// `icon` from `properties`.
    pub icon: Option<String>,
}

/// A `box` grouping participants.
#[derive(Debug, Clone, PartialEq)]
pub struct ParticipantBox {
    pub name: Option<String>,
    /// CSS color (`transparent` when none was given).
    pub fill: String,
    pub wrap: bool,
    pub participants: Vec<String>,
}

macro_rules! line_types {
    ($($name:ident = $code:expr),* $(,)?) => {
        /// Mermaid's `LINETYPE`: the kind of a message entry.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum LineType { $($name),* }

        impl LineType {
            /// Mermaid's numeric code.
            pub fn code(self) -> u8 {
                match self { $(Self::$name => $code),* }
            }
        }
    };
}

line_types! {
    Solid = 0,
    Dotted = 1,
    Note = 2,
    SolidCross = 3,
    DottedCross = 4,
    SolidOpen = 5,
    DottedOpen = 6,
    LoopStart = 10,
    LoopEnd = 11,
    AltStart = 12,
    AltElse = 13,
    AltEnd = 14,
    OptStart = 15,
    OptEnd = 16,
    ActiveStart = 17,
    ActiveEnd = 18,
    ParStart = 19,
    ParAnd = 20,
    ParEnd = 21,
    RectStart = 22,
    RectEnd = 23,
    SolidPoint = 24,
    DottedPoint = 25,
    Autonumber = 26,
    CriticalStart = 27,
    CriticalOption = 28,
    CriticalEnd = 29,
    BreakStart = 30,
    BreakEnd = 31,
    ParOverStart = 32,
    BidirectionalSolid = 33,
    BidirectionalDotted = 34,
    SolidTop = 41,
    SolidBottom = 42,
    StickTop = 43,
    StickBottom = 44,
    SolidArrowTopReverse = 45,
    SolidArrowBottomReverse = 46,
    StickArrowTopReverse = 47,
    StickArrowBottomReverse = 48,
    SolidTopDotted = 51,
    SolidBottomDotted = 52,
    StickTopDotted = 53,
    StickBottomDotted = 54,
    SolidArrowTopReverseDotted = 55,
    SolidArrowBottomReverseDotted = 56,
    StickArrowTopReverseDotted = 57,
    StickArrowBottomReverseDotted = 58,
    CentralConnection = 59,
    CentralConnectionReverse = 60,
    CentralConnectionDual = 61,
}

impl LineType {
    /// Whether this entry is an arrow between participants.
    pub fn is_arrow(self) -> bool {
        use LineType::*;
        matches!(
            self,
            SolidOpen
                | DottedOpen
                | Solid
                | SolidTop
                | SolidBottom
                | StickTop
                | StickBottom
                | SolidTopDotted
                | SolidBottomDotted
                | StickTopDotted
                | StickBottomDotted
                | SolidArrowTopReverse
                | SolidArrowBottomReverse
                | StickArrowTopReverse
                | StickArrowBottomReverse
                | SolidArrowTopReverseDotted
                | SolidArrowBottomReverseDotted
                | StickArrowTopReverseDotted
                | StickArrowBottomReverseDotted
                | Dotted
                | SolidCross
                | DottedCross
                | SolidPoint
                | DottedPoint
                | BidirectionalSolid
                | BidirectionalDotted
        )
    }

    /// Dashed arrows.
    pub fn is_dotted(self) -> bool {
        use LineType::*;
        matches!(
            self,
            Dotted
                | DottedCross
                | DottedPoint
                | DottedOpen
                | BidirectionalDotted
                | SolidTopDotted
                | SolidBottomDotted
                | StickTopDotted
                | StickBottomDotted
                | SolidArrowTopReverseDotted
                | SolidArrowBottomReverseDotted
                | StickArrowTopReverseDotted
                | StickArrowBottomReverseDotted
        )
    }

    /// Half arrows drawn at the start of the line (`/|-`, `\\-`, ...).
    pub fn is_reverse(self) -> bool {
        use LineType::*;
        matches!(
            self,
            SolidArrowTopReverse
                | SolidArrowTopReverseDotted
                | SolidArrowBottomReverse
                | SolidArrowBottomReverseDotted
                | StickArrowTopReverse
                | StickArrowTopReverseDotted
                | StickArrowBottomReverse
                | StickArrowBottomReverseDotted
        )
    }

    pub fn is_bidirectional(self) -> bool {
        matches!(self, Self::BidirectionalSolid | Self::BidirectionalDotted)
    }
}

/// Where a note sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Placement {
    LeftOf,
    RightOf,
    Over,
}

/// An `autonumber` change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Autonumber {
    pub start: Option<f64>,
    pub step: Option<f64>,
    pub visible: bool,
}

/// One entry of the message list.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub kind: LineType,
    pub from: Option<String>,
    pub to: Option<String>,
    pub text: String,
    pub wrap: bool,
    /// `+` after the arrow (or a central connection): activates the target.
    pub activate: bool,
    /// For notes.
    pub placement: Option<Placement>,
    /// `()` ends: [`LineType::CentralConnection`], `...Reverse` or `...Dual`.
    pub central: Option<LineType>,
    /// For [`LineType::Autonumber`] entries.
    pub autonumber: Option<Autonumber>,
}

/// A parsed sequence diagram.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SequenceDiagram {
    /// Participants in declaration order.
    pub participants: Vec<Participant>,
    pub messages: Vec<Message>,
    pub boxes: Vec<ParticipantBox>,
    /// Participants introduced by `create`, with the index of the message
    /// count at that point.
    pub created: Vec<(String, usize)>,
    /// Participants ended by `destroy`, likewise.
    pub destroyed: Vec<(String, usize)>,
    pub title: Option<String>,
    pub acc_title: Option<String>,
    pub acc_descr: Option<String>,
    pub config: DiagramConfig,
}

impl SequenceDiagram {
    pub fn participant(&self, id: &str) -> Option<&Participant> {
        self.participants.iter().find(|p| p.id == id)
    }
}
