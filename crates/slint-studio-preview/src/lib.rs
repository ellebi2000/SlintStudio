//! Off-screen rendering of `.slint` files for the live preview.
//!
//! **Status: skeleton (implemented in Phase 2).**
//!
//! Planned responsibilities:
//! - compile `.slint` sources at runtime with `slint-interpreter`;
//! - render the root component with Slint's software renderer into an RGBA
//!   image (`render(path) -> image::RgbaImage`), on a dedicated thread because
//!   Slint objects are not `Send`;
//! - report parse/compile errors as `slint_studio_core::Diagnostic`s;
//! - expose element geometry for hit-testing in the designer (Phase 3).
