//! Shared domain models for Slint Studio.
//!
//! This crate is deliberately free of GUI and Slint dependencies so it can be
//! used (and unit-tested) by every other crate in the workspace: the egui shell,
//! the LSP client, the preview renderer and the compiler wrapper.

pub mod diagnostics;
pub mod log;
pub mod palette;
pub mod view_mode;

pub use diagnostics::{Diagnostic, DiagnosticCounts, DiagnosticSource, Location, Severity};
pub use log::{LogBuffer, LogEntry, LogLevel};
pub use palette::{ComponentCategory, PaletteItem};
pub use view_mode::ViewMode;
