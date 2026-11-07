//! Minimal Language Server Protocol client for `slint-lsp`.
//!
//! **Status: skeleton (implemented in Phase 1).**
//!
//! Planned responsibilities:
//! - spawn `slint-lsp` as a child process and speak JSON-RPC over stdio;
//! - perform the `initialize` handshake and keep documents in sync
//!   (`textDocument/didOpen`, `didChange`, `didSave`);
//! - surface `publishDiagnostics`, completion and go-to-definition results to
//!   the UI as `slint_studio_core` types, via channels (the UI never blocks).
