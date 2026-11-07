//! Building and running the user's project ("Compila" and "Esegui").
//!
//! - [`project`]: what kind of project the open folder is (Cargo or plain
//!   `.slint` files);
//! - [`job`]: a build or run on a background thread, reported as events;
//! - [`process`]: external processes with streamed output and a stop that
//!   reaches their children;
//! - [`cargo`]: rustc and `slint-build` diagnostics from cargo's output;
//! - [`check`]: checking `.slint` files with `i-slint-compiler`;
//! - [`viewer`]: the managed `slint-viewer` used to run `.slint` files.

pub mod cargo;
pub mod check;
pub mod job;
pub mod process;
pub mod project;
pub mod viewer;

pub use job::{Job, JobEvent, JobKind, JobOutcome, JobPhase, JobRequest, Waker};
pub use project::ProjectKind;
