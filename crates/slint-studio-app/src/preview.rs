//! Live preview state on the UI side: scheduling (debounce), the texture
//! shown in the canvas and the last error.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{ColorImage, TextureHandle, TextureOptions};
use slint_studio_core::Diagnostic;
use slint_studio_preview::{PreviewError, PreviewRequest, PreviewService, RenderedPreview, Waker};
use tracing::{debug, error};

/// Delay between the last keystroke and the render.
pub const EDIT_DEBOUNCE: Duration = Duration::from_millis(300);

/// Zoom of the canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Zoom {
    /// Fit the component in the canvas, never above 100%.
    Fit,
    Scale(f32),
}

impl Zoom {
    pub const STEPS: [f32; 9] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0];

    pub fn zoom_in(current: f32) -> Self {
        Self::Scale(
            Self::STEPS
                .iter()
                .copied()
                .find(|s| *s > current + 0.01)
                .unwrap_or(6.0),
        )
    }

    pub fn zoom_out(current: f32) -> Self {
        Self::Scale(
            Self::STEPS
                .iter()
                .rev()
                .copied()
                .find(|s| *s < current - 0.01)
                .unwrap_or(0.25),
        )
    }
}

/// The image currently displayed.
pub struct PreviewImage {
    pub texture: TextureHandle,
    pub path: PathBuf,
    pub component: String,
    /// Size in logical pixels (egui points at 100% zoom).
    pub logical_size: egui::Vec2,
    pub duration: Duration,
}

/// Why the latest render failed; shown over the (stale) image.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewFailure {
    pub message: String,
    pub diagnostics: Vec<Diagnostic>,
    /// Not a real error (e.g. a file with only structs): shown neutrally.
    pub informational: bool,
}

pub struct PreviewManager {
    service: Option<PreviewService>,
    pub image: Option<PreviewImage>,
    pub failure: Option<PreviewFailure>,
    pub zoom: Zoom,
    /// Zoom actually applied in the last frame (resolves [`Zoom::Fit`]).
    pub effective_zoom: f32,
    /// File being previewed.
    pub target: Option<PathBuf>,
    /// When the next render is due (debounce).
    due: Option<Instant>,
    in_flight: Option<u64>,
}

impl PreviewManager {
    pub fn new(waker: Waker) -> Self {
        let service = match PreviewService::start(waker) {
            Ok(service) => Some(service),
            Err(err) => {
                error!("impossibile avviare il thread dell'anteprima: {err}");
                None
            }
        };
        Self {
            service,
            image: None,
            failure: None,
            zoom: Zoom::Fit,
            effective_zoom: 1.0,
            target: None,
            due: None,
            in_flight: None,
        }
    }

    pub fn is_rendering(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Renders `delay` from now; a later call before then postpones it.
    pub fn schedule(&mut self, delay: Duration) {
        if self.target.is_some() {
            self.due = Some(Instant::now() + delay);
        }
    }

    /// Time left before a scheduled render, if any.
    pub fn time_until_due(&self) -> Option<Duration> {
        self.due
            .map(|due| due.saturating_duration_since(Instant::now()))
    }

    pub fn is_due(&self) -> bool {
        self.due.is_some_and(|due| Instant::now() >= due)
    }

    /// Sends the render request; `overrides` are the unsaved buffers.
    pub fn render_now(&mut self, overrides: HashMap<PathBuf, String>, scale_factor: f32) {
        self.due = None;
        let (Some(service), Some(path)) = (&mut self.service, &self.target) else {
            return;
        };
        let mut request = PreviewRequest::new(path.clone());
        request.overrides = overrides;
        request.scale_factor = scale_factor;
        self.in_flight = Some(service.request(request));
    }

    /// Forgets the current preview (e.g. when the project is closed).
    pub fn clear(&mut self) {
        self.image = None;
        self.failure = None;
        self.target = None;
        self.due = None;
    }

    /// Applies the response to the latest request, if it arrived; older
    /// responses are stale and dropped. Returns whether something changed.
    pub fn poll(&mut self, ctx: &egui::Context) -> bool {
        let Some(service) = &self.service else {
            return false;
        };
        let Some(response) = service.poll() else {
            return false;
        };
        if Some(response.generation) != self.in_flight {
            return false;
        }
        self.in_flight = None;
        match response.result {
            Ok(preview) => self.show(ctx, preview),
            Err(err) => self.fail(&err),
        }
        true
    }

    fn show(&mut self, ctx: &egui::Context, preview: RenderedPreview) {
        debug!(
            "anteprima di {} in {:?}",
            preview.component, preview.duration
        );
        let size = [
            preview.image.width() as usize,
            preview.image.height() as usize,
        ];
        let color_image = ColorImage::from_rgba_unmultiplied(size, preview.image.as_raw());
        let logical_size = egui::vec2(preview.logical_size.0, preview.logical_size.1);
        let texture = match self.image.take() {
            Some(mut image) => {
                image.texture.set(color_image, TextureOptions::LINEAR);
                image.texture
            }
            None => ctx.load_texture("slint-preview", color_image, TextureOptions::LINEAR),
        };
        self.image = Some(PreviewImage {
            texture,
            path: preview.path,
            component: preview.component,
            logical_size,
            duration: preview.duration,
        });
        self.failure = None;
    }

    fn fail(&mut self, err: &PreviewError) {
        let (diagnostics, informational) = match err {
            PreviewError::Compile { diagnostics } => (diagnostics.clone(), false),
            PreviewError::NoComponent => (Vec::new(), true),
            _ => (Vec::new(), false),
        };
        self.failure = Some(PreviewFailure {
            message: err.to_string(),
            diagnostics,
            informational,
        });
    }

    pub fn previews(&self, path: &Path) -> bool {
        self.target.as_deref() == Some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps() {
        assert_eq!(Zoom::zoom_in(1.0), Zoom::Scale(1.5));
        assert_eq!(Zoom::zoom_out(1.0), Zoom::Scale(0.75));
        assert_eq!(Zoom::zoom_in(0.6), Zoom::Scale(0.75));
        assert_eq!(Zoom::zoom_in(6.0), Zoom::Scale(6.0));
        assert_eq!(Zoom::zoom_out(0.25), Zoom::Scale(0.25));
    }
}
