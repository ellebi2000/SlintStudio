//! Conversions from LSP types to Slint Studio's own models.

use std::path::Path;

use lsp_types::{CompletionItem, CompletionTextEdit, DiagnosticSeverity, InsertTextFormat};
use slint_studio_core::{
    Diagnostic, DiagnosticSource, LineCol, LineIndex, Location, PositionEncoding, Severity,
};

/// Converts the diagnostics of one file.
///
/// With the document `text` columns are converted exactly from the server's
/// `encoding` to characters; without it they are assumed to already match
/// (true for ASCII lines).
pub fn to_core_diagnostics(
    path: &Path,
    diagnostics: &[lsp_types::Diagnostic],
    text: Option<&str>,
    encoding: PositionEncoding,
) -> Vec<Diagnostic> {
    let index = text.map(|text| (text, LineIndex::new(text)));
    let convert = |position: lsp_types::Position| {
        let position = LineCol::new(position.line, position.character);
        match &index {
            Some((text, index)) => index.convert(text, position, encoding, PositionEncoding::Chars),
            None => position,
        }
    };
    diagnostics
        .iter()
        .map(|d| Diagnostic {
            severity: severity(d.severity),
            source: DiagnosticSource::Lsp,
            message: d.message.clone(),
            location: Some(Location {
                path: path.to_path_buf(),
                start: convert(d.range.start),
                end: convert(d.range.end),
            }),
        })
        .collect()
}

pub fn severity(severity: Option<DiagnosticSeverity>) -> Severity {
    match severity {
        Some(DiagnosticSeverity::WARNING) => Severity::Warning,
        Some(DiagnosticSeverity::INFORMATION) => Severity::Info,
        Some(DiagnosticSeverity::HINT) => Severity::Hint,
        // Unspecified severity is an error, as in most clients.
        _ => Severity::Error,
    }
}

/// Text to insert when `item` is accepted.
pub fn insert_text(item: &CompletionItem) -> String {
    let text = match &item.text_edit {
        Some(CompletionTextEdit::Edit(edit)) => edit.new_text.as_str(),
        Some(CompletionTextEdit::InsertAndReplace(edit)) => edit.new_text.as_str(),
        None => item.insert_text.as_deref().unwrap_or(&item.label),
    };
    if item.insert_text_format == Some(InsertTextFormat::SNIPPET) {
        strip_snippet(text)
    } else {
        text.to_owned()
    }
}

/// Removes snippet syntax: `$1`/`$0` tab stops vanish, `${1:default}` keeps
/// its default, `${1|a,b|}` keeps its first choice, `\$` becomes `$`.
///
/// We advertise no snippet support, so this is only a safety net.
pub fn strip_snippet(snippet: &str) -> String {
    let mut out = String::with_capacity(snippet.len());
    let mut chars = snippet.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            '$' if chars.peek().is_some_and(char::is_ascii_digit) => {
                while chars.peek().is_some_and(char::is_ascii_digit) {
                    chars.next();
                }
            }
            '$' if chars.peek() == Some(&'{') => {
                chars.next();
                while chars.peek().is_some_and(char::is_ascii_digit) {
                    chars.next();
                }
                match chars.next() {
                    Some(':') => {
                        let mut depth = 1;
                        for c in chars.by_ref() {
                            match c {
                                '{' => depth += 1,
                                '}' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            out.push(c);
                        }
                    }
                    Some('|') => {
                        let mut first = true;
                        for c in chars.by_ref() {
                            match c {
                                '}' => break,
                                ',' | '|' => first = false,
                                c if first => out.push(c),
                                _ => {}
                            }
                        }
                    }
                    _ => {} // `${1}`
                }
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Position, Range, TextEdit};

    fn lsp_diagnostic(
        line: u32,
        start: u32,
        end: u32,
        severity: Option<DiagnosticSeverity>,
    ) -> lsp_types::Diagnostic {
        lsp_types::Diagnostic {
            range: Range::new(Position::new(line, start), Position::new(line, end)),
            severity,
            message: "Parse error".into(),
            ..Default::default()
        }
    }

    #[test]
    fn diagnostics_columns_become_characters() {
        // "è" is two UTF-8 bytes: byte column 8 is character column 7.
        let text = "a\nText { è: x }";
        let diagnostics = [lsp_diagnostic(1, 8, 9, None)];
        let converted = to_core_diagnostics(
            Path::new("/p/a.slint"),
            &diagnostics,
            Some(text),
            PositionEncoding::Utf8,
        );
        let location = converted[0].location.as_ref().unwrap();
        assert_eq!(location.start, LineCol::new(1, 7));
        assert_eq!(location.end, LineCol::new(1, 8));
        assert_eq!(converted[0].severity, Severity::Error);
        assert_eq!(converted[0].source, DiagnosticSource::Lsp);

        let raw = to_core_diagnostics(
            Path::new("/p/a.slint"),
            &diagnostics,
            None,
            PositionEncoding::Utf8,
        );
        assert_eq!(raw[0].location.as_ref().unwrap().start, LineCol::new(1, 8));
    }

    #[test]
    fn severities() {
        assert_eq!(
            severity(Some(DiagnosticSeverity::WARNING)),
            Severity::Warning
        );
        assert_eq!(severity(Some(DiagnosticSeverity::HINT)), Severity::Hint);
        assert_eq!(severity(None), Severity::Error);
    }

    #[test]
    fn insert_text_preference_order() {
        let mut item = CompletionItem::new_simple("background".into(), "brush".into());
        assert_eq!(insert_text(&item), "background");

        item.insert_text = Some("background: ".into());
        assert_eq!(insert_text(&item), "background: ");

        item.text_edit = Some(CompletionTextEdit::Edit(TextEdit::new(
            Range::default(),
            "bg".into(),
        )));
        assert_eq!(insert_text(&item), "bg");
    }

    #[test]
    fn snippets_are_flattened() {
        assert_eq!(strip_snippet("Rectangle { $0 }"), "Rectangle {  }");
        assert_eq!(strip_snippet("width: ${1:100px};$0"), "width: 100px;");
        assert_eq!(strip_snippet("${1|left,right|}"), "left");
        assert_eq!(strip_snippet(r"cost: \$5 ${2}"), "cost: $5 ");
        assert_eq!(strip_snippet("${1:{ nested }}"), "{ nested }");

        let item = CompletionItem {
            label: "Button".into(),
            insert_text: Some("Button { text: \"${1:OK}\"; }".into()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..Default::default()
        };
        assert_eq!(insert_text(&item), "Button { text: \"OK\"; }");
    }
}
