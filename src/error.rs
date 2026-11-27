use std::fmt;

/// Error category, useful for bindings that need a stable code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The source contains no diagram.
    Empty,
    /// The header keyword is not a known Mermaid diagram type.
    UnknownDiagram,
    /// A known Mermaid diagram type that is not ported yet.
    UnsupportedDiagram,
    /// Syntax error in a supported diagram.
    Syntax,
}

/// Parse error with a position (line and column start at 1, 0 means unknown).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    line: usize,
    column: usize,
    message: String,
}

impl Error {
    pub(crate) fn new(
        kind: ErrorKind,
        line: usize,
        column: usize,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            line,
            column,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn line(&self) -> usize {
        self.line
    }

    pub fn column(&self) -> usize {
        self.column
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line > 0 {
            write!(
                f,
                "line {}, column {}: {}",
                self.line, self.column, self.message
            )
        } else {
            f.write_str(&self.message)
        }
    }
}

impl std::error::Error for Error {}
