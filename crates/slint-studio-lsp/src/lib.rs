//! Language Server Protocol support for Slint Studio.
//!
//! - [`client`]: runs `slint-lsp` as a child process and speaks JSON-RPC over
//!   stdio, delivering results to the UI as [`LspEvent`]s.
//! - [`install`]: finds `slint-lsp` or downloads the pinned release.
//! - [`convert`]: turns LSP diagnostics and completions into Slint Studio's
//!   own models.

pub mod client;
pub mod convert;
pub mod install;
pub mod protocol;
pub mod transport;
pub mod uri;

pub use client::{DefinitionTarget, LspClient, LspEvent, MessageLevel, ServerConfig, Waker};
pub use lsp_types;
pub use protocol::RequestId;
