//! Diagnostics shown in the "Errori" panel and as squiggles in the editor.
//!
//! Diagnostics come from three producers (the language server, the live
//! preview and the compiler) and are normalised into this single model.

use std::fmt;
use std::path::PathBuf;

use crate::text::LineCol;

/// Severity of a diagnostic, ordered from least to most severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Hint,
    Info,
    Warning,
    Error,
}

impl Severity {
    /// Italian label used in the UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Hint => "Suggerimento",
            Self::Info => "Informazione",
            Self::Warning => "Avviso",
            Self::Error => "Errore",
        }
    }
}

/// Which subsystem produced a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticSource {
    Lsp,
    Preview,
    Compiler,
}

impl DiagnosticSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Lsp => "slint-lsp",
            Self::Preview => "Anteprima",
            Self::Compiler => "Compilatore",
        }
    }
}

/// A range in a source file.
///
/// Lines and columns are **0-based** and columns count characters
/// ([`PositionEncoding::Chars`](crate::text::PositionEncoding::Chars));
/// producers convert from their own encoding (e.g. LSP UTF-8/UTF-16) when
/// creating it. `Display` shows the 1-based start, as users expect.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Location {
    pub path: PathBuf,
    pub start: LineCol,
    pub end: LineCol,
}

impl Location {
    /// A zero-length location at `position`.
    pub fn at(path: impl Into<PathBuf>, position: LineCol) -> Self {
        Self {
            path: path.into(),
            start: position,
            end: position,
        }
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.path.display(),
            self.start.line + 1,
            self.start.column + 1
        )
    }
}

/// A single problem reported to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub source: DiagnosticSource,
    pub message: String,
    /// `None` for problems not tied to a file (e.g. "slint-lsp not found").
    pub location: Option<Location>,
}

/// Per-severity totals, used by the status bar and the panel title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiagnosticCounts {
    pub errors: usize,
    pub warnings: usize,
    /// Infos and hints.
    pub others: usize,
}

impl DiagnosticCounts {
    pub fn total(&self) -> usize {
        self.errors + self.warnings + self.others
    }
}

impl<'a> FromIterator<&'a Diagnostic> for DiagnosticCounts {
    fn from_iter<I: IntoIterator<Item = &'a Diagnostic>>(iter: I) -> Self {
        iter.into_iter()
            .fold(Self::default(), |mut counts, diagnostic| {
                match diagnostic.severity {
                    Severity::Error => counts.errors += 1,
                    Severity::Warning => counts.warnings += 1,
                    Severity::Info | Severity::Hint => counts.others += 1,
                }
                counts
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostic(severity: Severity) -> Diagnostic {
        Diagnostic {
            severity,
            source: DiagnosticSource::Lsp,
            message: "msg".into(),
            location: None,
        }
    }

    #[test]
    fn counts_group_by_severity() {
        let diagnostics = [
            diagnostic(Severity::Error),
            diagnostic(Severity::Warning),
            diagnostic(Severity::Error),
            diagnostic(Severity::Hint),
            diagnostic(Severity::Info),
        ];
        let counts: DiagnosticCounts = diagnostics.iter().collect();
        assert_eq!(
            counts,
            DiagnosticCounts {
                errors: 2,
                warnings: 1,
                others: 2
            }
        );
        assert_eq!(counts.total(), diagnostics.len());
    }

    #[test]
    fn empty_counts_are_zero() {
        let counts: DiagnosticCounts = [].iter().collect();
        assert_eq!(counts.total(), 0);
    }

    #[test]
    fn errors_are_the_most_severe() {
        assert!(Severity::Error > Severity::Warning);
        assert!(Severity::Warning > Severity::Info);
        assert!(Severity::Info > Severity::Hint);
    }

    #[test]
    fn location_displays_one_based_path_line_column() {
        let location = Location::at("ui/main.slint", LineCol::new(11, 4));
        assert_eq!(location.to_string(), "ui/main.slint:12:5");
    }
}
