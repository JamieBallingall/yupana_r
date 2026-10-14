//! Problems found in a `.yup` file, each with the place it came from.

use std::fmt;

/// A place in a source file: a line and a column, both counted from 1.
///
/// Columns count characters rather than bytes, so that a position matches what an editor shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    /// The line, counted from 1.
    pub line: u32,
    /// The column, counted from 1 in characters.
    pub column: u32,
}

impl Pos {
    /// The first character of a file.
    pub const START: Pos = Pos { line: 1, column: 1 };

    /// The position at `line` and `column`.
    pub fn new(line: u32, column: u32) -> Pos {
        Pos { line, column }
    }
}

impl fmt::Display for Pos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// How serious a problem is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// The workbook cannot be written.
    Error,
    /// The workbook can be written, but may not behave as intended when it calculates.
    Warning,
}

/// One problem, with where it was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Whether the problem stops the workbook being written.
    pub severity: Severity,
    /// The input file the problem is in, or `None` for the `.yup` source itself.
    pub file: Option<String>,
    /// Where the problem is.
    pub pos: Pos,
    /// What the problem is, in a sentence.
    pub message: String,
}

impl Diagnostic {
    /// A problem that stops the workbook being written.
    pub fn error(pos: Pos, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            file: None,
            pos,
            message: message.into(),
        }
    }

    /// A problem that leaves the workbook writable.
    pub fn warning(pos: Pos, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            ..Diagnostic::error(pos, message)
        }
    }

    /// The same problem, placed in the input file `file` rather than in the `.yup` source.
    pub fn in_file(self, file: &str) -> Diagnostic {
        Diagnostic {
            file: Some(file.to_string()),
            ..self
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(file) = &self.file {
            write!(f, "{file}:")?;
        }
        let severity = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{}: {severity}: {}", self.pos, self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_diagnostic_reads_as_position_severity_message() {
        let error = Diagnostic::error(Pos::new(3, 7), "unknown name `x`");
        assert_eq!(error.to_string(), "3:7: error: unknown name `x`");
        let warning = Diagnostic::warning(Pos::new(1, 1), "careful").in_file("data.csv");
        assert_eq!(warning.to_string(), "data.csv:1:1: warning: careful");
    }
}
