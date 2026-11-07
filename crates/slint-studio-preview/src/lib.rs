//! Off-screen rendering of `.slint` files for the live preview.
//!
//! Files are compiled at runtime with `slint-interpreter` and rendered with
//! Slint's software renderer into an RGBA image, without any window on
//! screen. Unsaved editor buffers can be supplied so the preview follows the
//! text as it is typed.
//!
//! - [`render`]: one-shot rendering of a file on the current thread.
//! - [`Previewer`]: reusable renderer bound to the current thread.
//! - [`PreviewService`]: renders on a dedicated thread, for UIs.

mod geometry;
mod renderer;
mod service;
mod window;

use std::cell::RefCell;
use std::path::Path;

pub use geometry::{ElementGeometry, ElementRect, hit_test};
pub use image::RgbaImage;
pub use renderer::{
    DEFAULT_SIZE, DEFAULT_STYLE, MAX_SIZE, PreviewError, PreviewRequest, Previewer, RenderedPreview,
};
pub use service::{PreviewResponse, PreviewService, Waker};

thread_local! {
    static PREVIEWER: RefCell<Option<Previewer>> = const { RefCell::new(None) };
}

/// Renders the last exported component of the `.slint` file at `path`, at
/// its preferred size and a scale factor of 1.
pub fn render(path: &Path) -> Result<RgbaImage, PreviewError> {
    PREVIEWER.with(|cell| {
        let mut slot = cell.borrow_mut();
        let previewer = match slot.as_mut() {
            Some(previewer) => previewer,
            None => slot.insert(Previewer::new()?),
        };
        previewer
            .render(&PreviewRequest::new(path))
            .map(|preview| preview.image)
    })
}
