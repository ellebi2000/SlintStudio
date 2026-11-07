//! Understanding cargo's output.
//!
//! Builds run with `--message-format=json-diagnostic-rendered-ansi`: on
//! stdout cargo writes one JSON message per line, and rustc diagnostics come
//! with both a structured form (for the "Errori" panel) and the coloured text
//! rustc would have printed (for the terminal). With `cargo run`, everything
//! after the `build-finished` message is the program's own output.
//!
//! Errors of `.slint` files compiled by a build script (`slint-build`) are not
//! JSON: cargo prints the build script's stderr, where Slint reports them as
//! `error: message` followed by ` --> path:line:column`. [`StderrScanner`]
//! recognises that shape, and cargo's own `error:` lines.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use slint_studio_core::terminal::strip_ansi;
use slint_studio_core::{Diagnostic, DiagnosticSource, LineCol, Location, Severity};

/// Something to do with one line of cargo's stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdoutEvent {
    /// Text for the terminal: a rendered rustc diagnostic (normally on
    /// stderr) or, after the build, the program's stdout.
    Output {
        text: String,
        from_build: bool,
    },
    Diagnostic(Diagnostic),
    /// The build ended; with `cargo run` the program starts next.
    BuildFinished {
        success: bool,
    },
}

#[derive(Deserialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
enum CargoMessage {
    CompilerMessage {
        message: RustcDiagnostic,
    },
    BuildFinished {
        success: bool,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct RustcDiagnostic {
    message: String,
    code: Option<RustcCode>,
    level: String,
    #[serde(default)]
    spans: Vec<RustcSpan>,
    rendered: Option<String>,
}

#[derive(Deserialize)]
struct RustcCode {
    code: String,
}

#[derive(Deserialize)]
struct RustcSpan {
    file_name: String,
    line_start: u32,
    line_end: u32,
    column_start: u32,
    column_end: u32,
    is_primary: bool,
    expansion: Option<Box<RustcExpansion>>,
}

#[derive(Deserialize)]
struct RustcExpansion {
    span: RustcSpan,
}

/// Interprets cargo's stdout, line by line.
#[derive(Debug, Clone)]
pub struct StdoutParser {
    workspace_root: PathBuf,
    build_finished: bool,
}

impl StdoutParser {
    /// `workspace_root` is where rustc's relative paths start: the directory
    /// of the workspace's `Cargo.toml`.
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            build_finished: false,
        }
    }

    pub fn line(&mut self, line: &str) -> Vec<StdoutEvent> {
        if self.build_finished {
            return vec![StdoutEvent::Output {
                text: line.to_owned(),
                from_build: false,
            }];
        }
        if !line.starts_with('{') {
            return vec![StdoutEvent::Output {
                text: line.to_owned(),
                from_build: true,
            }];
        }
        match serde_json::from_str::<CargoMessage>(line) {
            Ok(CargoMessage::CompilerMessage { message }) => {
                let mut events = Vec::new();
                if let Some(rendered) = &message.rendered {
                    events.push(StdoutEvent::Output {
                        text: rendered.trim_end_matches('\n').to_owned(),
                        from_build: true,
                    });
                }
                if let Some(diagnostic) = convert(&message, &self.workspace_root) {
                    events.push(StdoutEvent::Diagnostic(diagnostic));
                }
                events
            }
            Ok(CargoMessage::BuildFinished { success }) => {
                self.build_finished = true;
                vec![StdoutEvent::BuildFinished { success }]
            }
            Ok(CargoMessage::Other) => Vec::new(),
            // Not one of cargo's messages after all.
            Err(_) => vec![StdoutEvent::Output {
                text: line.to_owned(),
                from_build: true,
            }],
        }
    }
}

fn convert(message: &RustcDiagnostic, root: &Path) -> Option<Diagnostic> {
    let severity = match message.level.as_str() {
        "error" | "error: internal compiler error" => Severity::Error,
        "warning" => Severity::Warning,
        "note" | "failure-note" => Severity::Info,
        "help" => Severity::Hint,
        _ => Severity::Info,
    };
    // Summaries repeat what the other diagnostics already say.
    if message.level == "failure-note"
        || (message.spans.is_empty()
            && (message.message.starts_with("aborting due to")
                || message.message.ends_with("warning emitted")
                || message.message.ends_with("warnings emitted")))
    {
        return None;
    }
    let text = match &message.code {
        Some(code) if !code.code.is_empty() => format!("{} [{}]", message.message, code.code),
        _ => message.message.clone(),
    };
    let location = message
        .spans
        .iter()
        .find(|span| span.is_primary)
        .or_else(|| message.spans.first())
        .map(|span| user_span(span, root))
        .and_then(|span| span_location(span, root));
    Some(Diagnostic {
        severity,
        source: DiagnosticSource::Compiler,
        message: text,
        location,
    })
}

/// For a span inside a macro defined elsewhere, the call site in the user's
/// code (the outermost expansion whose file is in the workspace).
fn user_span<'a>(span: &'a RustcSpan, root: &Path) -> &'a RustcSpan {
    let mut current = span;
    while !is_workspace_file(&current.file_name, root) {
        match &current.expansion {
            Some(expansion) => current = &expansion.span,
            None => return span,
        }
    }
    current
}

fn is_workspace_file(file_name: &str, root: &Path) -> bool {
    let path = Path::new(file_name);
    (path.is_relative() && !file_name.starts_with('<')) || path.starts_with(root)
}

fn span_location(span: &RustcSpan, root: &Path) -> Option<Location> {
    if span.file_name.starts_with('<') {
        return None; // `<anon>`, `<built-in>`, …
    }
    let path = resolve(root, &span.file_name);
    // rustc lines and columns are 1-based, columns in characters.
    let position =
        |line: u32, column: u32| LineCol::new(line.saturating_sub(1), column.saturating_sub(1));
    Some(Location {
        path,
        start: position(span.line_start, span.column_start),
        end: position(span.line_end, span.column_end),
    })
}

/// `file` (relative to `root` unless absolute), canonicalised when it exists
/// so it matches the paths of open documents.
pub fn resolve(root: &Path, file: &str) -> PathBuf {
    let path = root.join(file);
    path.canonicalize().unwrap_or(path)
}

/// Finds diagnostics in cargo's stderr: `.slint` errors printed by build
/// scripts, and cargo's own errors (missing targets, failed build scripts).
#[derive(Debug, Clone)]
pub struct StderrScanner {
    workspace_root: PathBuf,
    pending: Option<(Severity, String)>,
}

impl StderrScanner {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            pending: None,
        }
    }

    /// Feeds one line; returns a diagnostic once one is complete.
    pub fn line(&mut self, raw: &str) -> Option<Diagnostic> {
        let plain = strip_ansi(raw);
        let (text, from_build_script) = strip_build_script_prefix(&plain);

        if let Some(location) = text.trim_start().strip_prefix("-->")
            && let Some((severity, message)) = self.pending.take()
        {
            return Some(Diagnostic {
                severity,
                source: DiagnosticSource::Compiler,
                message,
                location: parse_location(location.trim(), &self.workspace_root),
            });
        }

        let headline = [
            ("error: ", Severity::Error),
            ("warning: ", Severity::Warning),
        ]
        .into_iter()
        .find_map(|(prefix, severity)| {
            // Cargo indents the stderr of a failed build script by two spaces.
            text.trim_start()
                .strip_prefix(prefix)
                .map(|message| (severity, message.trim().to_owned()))
        })
        .or_else(|| {
            // slint-build forwards each line of its warnings, dropping the
            // `warning: ` of the first: a headline is a line that is not
            // part of a code snippet (indented, or `12 | …`).
            let headline = from_build_script
                && text.starts_with(|c: char| !c.is_whitespace() && !c.is_ascii_digit());
            headline.then(|| (Severity::Warning, text.trim().to_owned()))
        });
        match headline {
            Some((severity, message)) => {
                let flushed = self.flush();
                if !is_summary(&message) {
                    self.pending = Some((severity, message));
                }
                flushed
            }
            None if text.trim().is_empty() => None,
            None => self.flush(),
        }
    }

    /// Call at end of output: completes a pending error.
    pub fn finish(&mut self) -> Option<Diagnostic> {
        self.flush()
    }

    /// A headline not followed by a location: kept if it is an error (cargo
    /// warnings without a location are mostly noise).
    fn flush(&mut self) -> Option<Diagnostic> {
        match self.pending.take() {
            Some((Severity::Error, message)) => Some(Diagnostic {
                severity: Severity::Error,
                source: DiagnosticSource::Compiler,
                message,
                location: None,
            }),
            _ => None,
        }
    }
}

/// Lines a build script prints with `cargo:warning=` come out as
/// `warning: package@version: text`. Returns the text, and whether the
/// prefix was there.
fn strip_build_script_prefix(line: &str) -> (&str, bool) {
    let Some(rest) = line.strip_prefix("warning: ") else {
        return (line, false);
    };
    match rest.split_once(": ") {
        Some((package, text)) if package.contains('@') && !package.contains(' ') => (text, true),
        _ => (line, false),
    }
}

/// Cargo's end-of-build summaries.
fn is_summary(message: &str) -> bool {
    message.starts_with("could not compile")
        || message.starts_with("build failed")
        || message.contains(") generated ")
        || message.starts_with("aborting due to")
}

/// Parses `path:line:column` (1-based).
fn parse_location(text: &str, root: &Path) -> Option<Location> {
    let mut parts = text.rsplitn(3, ':');
    let column: u32 = parts.next()?.trim().parse().ok()?;
    let line: u32 = parts.next()?.trim().parse().ok()?;
    let path = parts.next()?.trim();
    if path.is_empty() {
        return None;
    }
    Some(Location::at(
        resolve(root, path),
        LineCol::new(line.saturating_sub(1), column.saturating_sub(1)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from `cargo run --message-format=json-diagnostic-rendered-ansi`.
    const WARNING: &str = r#"{"reason":"compiler-message","package_id":"path+file:///w/hello#0.1.0","manifest_path":"/w/hello/Cargo.toml","target":{"kind":["bin"],"crate_types":["bin"],"name":"hello","src_path":"/w/hello/src/main.rs","edition":"2024","doc":true,"doctest":false,"test":true},"message":{"rendered":"\u001b[1m\u001b[33mwarning\u001b[0m: unused variable: `unused`\n --> src/main.rs:2:9\n","$message_type":"diagnostic","children":[],"code":{"code":"unused_variables","explanation":null},"level":"warning","message":"unused variable: `unused`","spans":[{"byte_end":26,"byte_start":20,"column_end":15,"column_start":9,"expansion":null,"file_name":"src/main.rs","is_primary":true,"label":null,"line_end":2,"line_start":2,"suggested_replacement":null,"suggestion_applicability":null,"text":[]}]}}"#;

    const ERROR_IN_MACRO: &str = r#"{"reason":"compiler-message","message":{"rendered":"error\n","code":{"code":"E0308","explanation":"..."},"level":"error","message":"mismatched types","spans":[{"file_name":"/rustc/abc/library/std/src/macros.rs","line_start":10,"line_end":10,"column_start":1,"column_end":5,"is_primary":true,"expansion":{"span":{"file_name":"src/ui.rs","line_start":7,"line_end":8,"column_start":5,"column_end":2,"is_primary":false,"expansion":null},"macro_decl_name":"println!","def_site_span":null}}],"children":[]}}"#;

    const ABORTING: &str = r#"{"reason":"compiler-message","message":{"rendered":"error: aborting due to 1 previous error\n","code":null,"level":"error","message":"aborting due to 1 previous error","spans":[],"children":[]}}"#;

    const LINKER: &str = r#"{"reason":"compiler-message","message":{"rendered":"error: linking with `cc` failed\n","code":null,"level":"error","message":"linking with `cc` failed: exit status: 1","spans":[],"children":[]}}"#;

    fn diagnostics(events: &[StdoutEvent]) -> Vec<&Diagnostic> {
        events
            .iter()
            .filter_map(|event| match event {
                StdoutEvent::Diagnostic(d) => Some(d),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn compiler_messages_become_diagnostics_and_rendered_output() {
        let mut parser = StdoutParser::new("/w/hello");
        let events = parser.line(WARNING);
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            StdoutEvent::Output { text, from_build: true }
                if text.ends_with(" --> src/main.rs:2:9")
        ));
        let diagnostic = diagnostics(&events)[0];
        assert_eq!(diagnostic.severity, Severity::Warning);
        assert_eq!(diagnostic.source, DiagnosticSource::Compiler);
        assert_eq!(
            diagnostic.message,
            "unused variable: `unused` [unused_variables]"
        );
        let location = diagnostic.location.as_ref().unwrap();
        assert_eq!(location.path, Path::new("/w/hello/src/main.rs"));
        assert_eq!(location.start, LineCol::new(1, 8));
        assert_eq!(location.end, LineCol::new(1, 14));
    }

    #[test]
    fn macro_spans_point_at_the_call_site() {
        let mut parser = StdoutParser::new("/w/hello");
        let events = parser.line(ERROR_IN_MACRO);
        let diagnostic = diagnostics(&events)[0];
        assert_eq!(diagnostic.severity, Severity::Error);
        assert_eq!(diagnostic.message, "mismatched types [E0308]");
        let location = diagnostic.location.as_ref().unwrap();
        assert_eq!(location.path, Path::new("/w/hello/src/ui.rs"));
        assert_eq!(location.start, LineCol::new(6, 4));
    }

    #[test]
    fn summaries_are_skipped_but_unlocated_errors_kept() {
        let mut parser = StdoutParser::new("/w");
        assert!(diagnostics(&parser.line(ABORTING)).is_empty());
        let events = parser.line(LINKER);
        let linker = diagnostics(&events)[0];
        assert!(linker.location.is_none());
        assert!(linker.message.starts_with("linking with `cc` failed"));
    }

    #[test]
    fn program_output_follows_build_finished() {
        let mut parser = StdoutParser::new("/w");
        assert!(
            parser
                .line(r#"{"reason":"compiler-artifact","target":{}}"#)
                .is_empty()
        );
        assert_eq!(
            parser.line("not json"),
            vec![StdoutEvent::Output {
                text: "not json".into(),
                from_build: true
            }]
        );
        assert_eq!(
            parser.line(r#"{"reason":"build-finished","success":true}"#),
            vec![StdoutEvent::BuildFinished { success: true }]
        );
        // The program may print JSON too: it is just output now.
        assert_eq!(
            parser.line(r#"{"reason":"build-finished","success":false}"#),
            vec![StdoutEvent::Output {
                text: r#"{"reason":"build-finished","success":false}"#.into(),
                from_build: false
            }]
        );
        // Broken JSON before the end of the build is shown as is.
        assert!(matches!(
            StdoutParser::new("/w").line("{oops").as_slice(),
            [StdoutEvent::Output {
                from_build: true,
                ..
            }]
        ));
    }

    #[test]
    fn slint_errors_from_build_scripts() {
        let mut scanner = StderrScanner::new("/w/app");
        let lines = [
            "\u{1b}[1m\u{1b}[92m   Compiling\u{1b}[0m app v0.1.0 (/w/app)",
            // As printed by cargo for a failing `slint_build::compile`.
            "\u{1b}[1m\u{1b}[91merror\u{1b}[0m: failed to run custom build command for `app v0.1.0 (/w/app)`",
            "note: To improve backtraces for build dependencies, set the CARGO_PROFILE_DEV_BUILD_OVERRIDE_DEBUG=true environment variable to enable debug information generation.",
            "",
            "Caused by:",
            "  process didn't exit successfully: `/w/app/target/debug/build/app-1/build-script-build` (exit status: 101)",
            "  --- stderr",
            "  \u{1b}[1m\u{1b}[91merror\u{1b}[0m\u{1b}[1m: Unknown element 'Buton'\u{1b}[0m",
            "    \u{1b}[1m\u{1b}[94m--> \u{1b}[0m/w/app/ui/app-window.slint:12:9",
            "     \u{1b}[1m\u{1b}[94m|\u{1b}[0m",
            "  \u{1b}[1m\u{1b}[94m12\u{1b}[0m \u{1b}[1m\u{1b}[94m|\u{1b}[0m         Buton {",
            "",
            "  thread 'main' (21406) panicked at build.rs:2:49:",
            "  compilazione Slint non riuscita: CompileError([\"/w/app/ui/app-window.slint:12: Unknown element 'Buton'\"])",
            "warning: app@0.1.0: Unused property 'x'",
            "warning: app@0.1.0:  --> ui/card.slint:3:5",
            "warning: app@0.1.0:   |",
            "warning: app@0.1.0: 3 |     x: 1;",
            "warning: `app` (bin \"app\") generated 1 warning",
            "error: could not compile `app` (bin \"app\") due to 1 previous error",
        ];
        let mut found: Vec<Diagnostic> =
            lines.iter().filter_map(|line| scanner.line(line)).collect();
        found.extend(scanner.finish());
        assert_eq!(found.len(), 3, "{found:#?}");

        assert_eq!(found[0].severity, Severity::Error);
        assert!(
            found[0]
                .message
                .starts_with("failed to run custom build command")
        );
        assert!(found[0].location.is_none());

        assert_eq!(found[1].message, "Unknown element 'Buton'");
        let location = found[1].location.as_ref().unwrap();
        assert_eq!(location.path, Path::new("/w/app/ui/app-window.slint"));
        assert_eq!(location.start, LineCol::new(11, 8));

        assert_eq!(found[2].severity, Severity::Warning);
        assert_eq!(found[2].message, "Unused property 'x'");
        let location = found[2].location.as_ref().unwrap();
        assert_eq!(location.path, Path::new("/w/app/ui/card.slint"));
        assert_eq!(location.start, LineCol::new(2, 4));
    }

    #[test]
    fn cargo_errors_without_location() {
        let mut scanner = StderrScanner::new("/w");
        assert!(scanner.line("error: no bin target named `x`").is_none());
        let diagnostic = scanner.finish().unwrap();
        assert_eq!(diagnostic.message, "no bin target named `x`");
        // Location-less warnings are dropped.
        assert!(scanner.line("warning: unused manifest key: foo").is_none());
        assert!(scanner.line("next").is_none());
        assert!(scanner.finish().is_none());
    }

    #[test]
    fn locations_are_parsed_strictly() {
        let root = Path::new("/w");
        let location = parse_location("C:/w/ui/a.slint:3:4", root).unwrap();
        assert_eq!(location.start, LineCol::new(2, 3));
        assert!(location.path.ends_with("ui/a.slint"));
        assert!(parse_location("a.slint:x:4", root).is_none());
        assert!(parse_location(":3:4", root).is_none());
        assert!(parse_location("nothing", root).is_none());
        assert_eq!(
            strip_build_script_prefix("warning: plain text"),
            ("warning: plain text", false)
        );
        assert_eq!(
            strip_build_script_prefix("warning: a@1.0: text"),
            ("text", true)
        );
    }
}
