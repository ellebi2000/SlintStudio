//! Shared domain models for Slint Studio.
//!
//! This crate is deliberately free of GUI and Slint dependencies so it can be
//! used (and unit-tested) by every other crate in the workspace: the egui shell,
//! the LSP client, the preview renderer and the compiler wrapper.

pub mod diagnostics;
pub mod document;
pub mod log;
pub mod palette;
pub mod project;
pub mod recent;
pub mod syntax;
pub mod text;
pub mod view_mode;

pub use diagnostics::{Diagnostic, DiagnosticCounts, DiagnosticSource, Location, Severity};
pub use document::{Document, Language, OpenError};
pub use log::{LogBuffer, LogEntry, LogLevel};
pub use palette::{ComponentCategory, PaletteItem};
pub use project::{FileNode, FileTree, SearchHit, SearchResults};
pub use recent::RecentProjects;
pub use text::{LineCol, LineIndex, PositionEncoding};
pub use view_mode::ViewMode;

/// Identifiers used to locate per-user directories (configuration, data).
pub mod app_dirs {
    pub const QUALIFIER: &str = "dev";
    pub const ORGANIZATION: &str = "slint-studio";
    pub const APPLICATION: &str = "slint-studio";
}
