//! Checking `.slint` files with the Slint compiler (`i-slint-compiler`).
//!
//! Each file goes through the whole compiler pipeline, configured like
//! `slint-viewer` (interpreter output, last exported component), so a file
//! that passes the check also opens in the viewer.

use std::collections::HashSet;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::task::{Context, Poll, Waker};

use i_slint_compiler::diagnostics::{BuildDiagnostics, DiagnosticLevel};
use i_slint_compiler::generator::OutputFormat;
use i_slint_compiler::{CompilerConfiguration, ComponentSelection};
use slint_studio_core::{
    Diagnostic, DiagnosticCounts, DiagnosticSource, LineCol, LineIndex, Location, PositionEncoding,
    Severity,
};

/// Style used when none is given; the same as the live preview's.
pub const DEFAULT_STYLE: &str = "fluent";

/// Result of checking a set of files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckReport {
    /// Files compiled (imports are compiled too, but not counted).
    pub files: usize,
    /// Diagnostics of every file, each reported once.
    pub diagnostics: Vec<Diagnostic>,
}

impl CheckReport {
    pub fn counts(&self) -> DiagnosticCounts {
        self.diagnostics.iter().collect()
    }

    pub fn has_errors(&self) -> bool {
        self.counts().errors > 0
    }
}

/// Compiles `path` and returns its diagnostics (and those of its imports).
pub fn check_file(path: &Path, style: &str) -> Vec<Diagnostic> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(err) => {
            return vec![Diagnostic {
                severity: Severity::Error,
                source: DiagnosticSource::Compiler,
                message: format!("impossibile leggere il file: {err}"),
                location: Some(Location::at(path, LineCol::new(0, 0))),
            }];
        }
    };
    let compiled = panic::catch_unwind(AssertUnwindSafe(|| compile(path, source, style)));
    match compiled {
        Ok(Some(diagnostics)) => diagnostics,
        Ok(None) => vec![internal_error(path, "la compilazione non è terminata")],
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".to_owned());
            vec![internal_error(path, &message)]
        }
    }
}

/// Compiles every file in `paths`, calling `progress` before each one.
pub fn check_files(paths: &[PathBuf], style: &str, mut progress: impl FnMut(&Path)) -> CheckReport {
    let mut seen = HashSet::new();
    let mut report = CheckReport::default();
    for path in paths {
        progress(path);
        report.files += 1;
        for diagnostic in check_file(path, style) {
            // An error in an imported file is reported by every importer.
            if seen.insert(diagnostic.clone()) {
                report.diagnostics.push(diagnostic);
            }
        }
    }
    report
}

fn internal_error(path: &Path, message: &str) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        source: DiagnosticSource::Compiler,
        message: format!("errore interno del compilatore Slint: {message}"),
        location: Some(Location::at(path, LineCol::new(0, 0))),
    }
}

fn compile(path: &Path, source: String, style: &str) -> Option<Vec<Diagnostic>> {
    let mut config = CompilerConfiguration::new(OutputFormat::Interpreter);
    config.components_to_generate = ComponentSelection::LastExported;
    config.style = Some(style.to_owned());
    let (_, diagnostics, _) = block_on(i_slint_compiler::load_root_file(
        path,
        path,
        source,
        BuildDiagnostics::default(),
        config,
    ))?;
    Some(
        diagnostics
            .iter()
            .map(|diagnostic| convert(diagnostic, path))
            .collect(),
    )
}

/// Converts a compiler diagnostic: Slint reports 1-based lines and columns
/// in UTF-8 bytes; ours are 0-based character columns.
fn convert(diagnostic: &i_slint_compiler::diagnostics::Diagnostic, root_file: &Path) -> Diagnostic {
    let severity = match diagnostic.level() {
        DiagnosticLevel::Error => Severity::Error,
        DiagnosticLevel::Warning => Severity::Warning,
        _ => Severity::Info,
    };
    let path = diagnostic.source_file().unwrap_or(root_file);
    let (line, column) = diagnostic.line_column();
    let start = LineCol::new(
        u32::try_from(line.saturating_sub(1)).unwrap_or(0),
        u32::try_from(column.saturating_sub(1)).unwrap_or(0),
    );
    let location = match fs::read_to_string(path) {
        Ok(text) => {
            let index = LineIndex::new(&text);
            let start_byte = index.offset(&text, start, PositionEncoding::Utf8);
            let end_byte = (start_byte + diagnostic.length()).min(text.len());
            Location {
                path: path.to_path_buf(),
                start: index.position(&text, start_byte, PositionEncoding::Chars),
                end: index.position(&text, end_byte, PositionEncoding::Chars),
            }
        }
        Err(_) => Location::at(path, start),
    };
    Diagnostic {
        severity,
        source: DiagnosticSource::Compiler,
        message: diagnostic.message().to_owned(),
        location: Some(location),
    }
}

/// Runs a future that never waits on I/O (the compiler reads files
/// synchronously when no import callback is set).
fn block_on<F: Future>(future: F) -> Option<F::Output> {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    for _ in 0..10_000 {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return Some(output);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path.canonicalize().unwrap()
    }

    #[test]
    fn valid_files_have_no_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let main = write(
            dir.path(),
            "main.slint",
            r#"import { Button } from "std-widgets.slint";
export component Main inherits Window {
    Button { text: "Ciao"; }
}"#,
        );
        assert_eq!(check_file(&main, DEFAULT_STYLE), Vec::new());
    }

    #[test]
    fn errors_have_character_based_locations() {
        let dir = tempfile::tempdir().unwrap();
        let main = write(
            dir.path(),
            "main.slint",
            "export component Main inherits Window {\n    Text { text: \"è\"; colr: red; }\n}\n",
        );
        let diagnostics = check_file(&main, DEFAULT_STYLE);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.severity, Severity::Error);
        assert_eq!(diagnostic.source, DiagnosticSource::Compiler);
        assert!(
            diagnostic.message.contains("colr"),
            "{}",
            diagnostic.message
        );
        let location = diagnostic.location.as_ref().unwrap();
        assert_eq!(location.path, main);
        // `colr` starts after `    Text { text: "è"; ` (22 characters).
        assert_eq!(location.start, LineCol::new(1, 22));
        assert_eq!(location.end, LineCol::new(1, 26));
    }

    #[test]
    fn errors_in_imports_are_reported_once() {
        let dir = tempfile::tempdir().unwrap();
        let card = write(
            dir.path(),
            "components/card.slint",
            "export component Card inherits Rectangle { Buton {} }\n",
        );
        let main = write(
            dir.path(),
            "main.slint",
            "import { Card } from \"components/card.slint\";\nexport component Main inherits Window { Card {} }\n",
        );
        let mut visited = Vec::new();
        let report = check_files(&[main.clone(), card.clone()], DEFAULT_STYLE, |p| {
            visited.push(p.to_path_buf())
        });
        assert_eq!(visited, [main, card.clone()]);
        assert_eq!(report.files, 2);
        assert_eq!(report.diagnostics.len(), 1, "{:#?}", report.diagnostics);
        assert_eq!(report.diagnostics[0].location.as_ref().unwrap().path, card);
        assert!(report.has_errors());
        assert_eq!(report.counts().errors, 1);
    }

    #[test]
    fn missing_files_are_errors() {
        let diagnostics = check_file(Path::new("/nonexistent/main.slint"), DEFAULT_STYLE);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.starts_with("impossibile leggere"));
    }

    #[test]
    fn warnings_do_not_fail_the_check() {
        let dir = tempfile::tempdir().unwrap();
        let main = write(
            dir.path(),
            "main.slint",
            "component Main inherits Window { }\n",
        );
        let report = check_files(&[main], DEFAULT_STYLE, |_| {});
        assert_eq!(report.diagnostics.len(), 1, "{:#?}", report.diagnostics);
        assert_eq!(report.diagnostics[0].severity, Severity::Warning);
        assert!(
            report.diagnostics[0]
                .message
                .contains("implicitly marked for export")
        );
        assert!(!report.has_errors());
    }
}
