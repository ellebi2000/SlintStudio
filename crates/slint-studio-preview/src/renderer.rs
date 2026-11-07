//! Compiling and rendering a `.slint` file into an image.

use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use image::RgbaImage;
use slint::LogicalSize;
use slint::platform::software_renderer::PremultipliedRgbaColor;
use slint::platform::{SetPlatformError, WindowEvent};
use slint_interpreter::{ComponentHandle as _, DiagnosticLevel};
use slint_studio_core::{
    Diagnostic, DiagnosticSource, LineCol, LineIndex, Location, PositionEncoding, Severity,
};

use crate::window::{PreviewPlatform, take_created_windows};

/// Size used when the component has no preferred size (e.g. an empty
/// `Rectangle`), in logical pixels.
pub const DEFAULT_SIZE: (f32, f32) = (400.0, 300.0);
/// Largest rendered size, in logical pixels, to bound memory use.
pub const MAX_SIZE: f32 = 4096.0;
/// Error the Slint compiler reports for files without any component.
const NO_COMPONENT_MESSAGE: &str = "No component found";
/// Style used for `std-widgets.slint` unless the request names another one.
pub const DEFAULT_STYLE: &str = "fluent";

/// What to render.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewRequest {
    /// The `.slint` file whose last exported component is rendered.
    pub path: PathBuf,
    /// Unsaved editor buffers, used instead of the files on disk (for the
    /// main file and for anything it imports).
    pub overrides: HashMap<PathBuf, String>,
    /// Physical pixels per logical pixel.
    pub scale_factor: f32,
    /// Widget style (`fluent`, `material`, `cupertino`, `cosmic`, …).
    pub style: Option<String>,
}

impl PreviewRequest {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            overrides: HashMap::new(),
            scale_factor: 1.0,
            style: None,
        }
    }
}

/// A successfully rendered component.
#[derive(Debug, Clone)]
pub struct RenderedPreview {
    pub path: PathBuf,
    /// Name of the rendered component.
    pub component: String,
    /// Straight (non-premultiplied) RGBA pixels, at physical resolution.
    pub image: RgbaImage,
    /// Size in logical pixels.
    pub logical_size: (f32, f32),
    pub scale_factor: f32,
    /// Non-fatal compiler diagnostics.
    pub warnings: Vec<Diagnostic>,
    pub duration: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    #[error("impossibile leggere {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{}", count_errors(diagnostics))]
    Compile { diagnostics: Vec<Diagnostic> },
    #[error("il file non contiene componenti da visualizzare")]
    NoComponent,
    #[error("impossibile creare il componente {component}: {message}")]
    Instantiate { component: String, message: String },
    #[error("piattaforma Slint non disponibile: {0}")]
    Platform(String),
    #[error("errore interno dell'anteprima: {0}")]
    Internal(String),
}

fn count_errors(diagnostics: &[Diagnostic]) -> String {
    match diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count()
    {
        1 => "il codice contiene un errore".to_owned(),
        count => format!("il codice contiene {count} errori"),
    }
}

/// Renders `.slint` files. Slint objects are bound to the thread that
/// created them, so a `Previewer` is neither `Send` nor `Sync`; use
/// [`crate::PreviewService`] to render from another thread.
pub struct Previewer {
    _not_send: PhantomData<Rc<()>>,
}

impl Previewer {
    /// Installs the headless Slint platform on the current thread.
    pub fn new() -> Result<Self, PreviewError> {
        match slint::platform::set_platform(Box::new(PreviewPlatform)) {
            // Already installed by an earlier previewer on this thread.
            Ok(()) | Err(SetPlatformError::AlreadySet) => Ok(Self {
                _not_send: PhantomData,
            }),
            Err(err) => Err(PreviewError::Platform(err.to_string())),
        }
    }

    pub fn render(&mut self, request: &PreviewRequest) -> Result<RenderedPreview, PreviewError> {
        let start = Instant::now();
        let source = match request.overrides.get(&request.path) {
            Some(text) => text.clone(),
            None => fs::read_to_string(&request.path).map_err(|source| PreviewError::Read {
                path: request.path.clone(),
                source,
            })?,
        };

        let mut compiler = slint_interpreter::Compiler::default();
        compiler.set_style(
            request
                .style
                .clone()
                .unwrap_or_else(|| DEFAULT_STYLE.to_owned()),
        );
        let overrides = Rc::new(request.overrides.clone());
        {
            let overrides = Rc::clone(&overrides);
            compiler.set_file_loader(move |path| {
                let text = lookup(&overrides, path);
                Box::pin(async move { text.map(Ok) })
            });
        }
        let result = block_on(compiler.build_from_source(source, request.path.clone()))
            .ok_or_else(|| PreviewError::Internal("la compilazione non è terminata".into()))?;

        let text_of =
            |path: &Path| lookup(&overrides, path).or_else(|| fs::read_to_string(path).ok());
        let diagnostics: Vec<Diagnostic> = result
            .diagnostics()
            .map(|d| convert_diagnostic(&d, &text_of))
            .collect();
        if result.has_errors() {
            // A file declaring only structs/globals is a library, not an error.
            let only_no_component = diagnostics
                .iter()
                .filter(|d| d.severity == Severity::Error)
                .all(|d| d.message == NO_COMPONENT_MESSAGE && d.location.is_none());
            return Err(if only_no_component {
                PreviewError::NoComponent
            } else {
                PreviewError::Compile { diagnostics }
            });
        }
        let definition = result
            .components()
            .last()
            .ok_or(PreviewError::NoComponent)?;
        let component = definition.name().to_owned();

        let instantiate_error = |message: String| PreviewError::Instantiate {
            component: component.clone(),
            message,
        };
        drop(take_created_windows());
        let instance = definition
            .create()
            .map_err(|err| instantiate_error(err.to_string()))?;
        let window = take_created_windows()
            .into_iter()
            .next()
            .ok_or_else(|| instantiate_error("nessuna finestra creata".into()))?;
        let scale_factor = sanitize_scale(request.scale_factor);
        window
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor });
        instance.show().map_err(|err| PreviewError::Instantiate {
            component: component.clone(),
            message: err.to_string(),
        })?;

        let logical = sanitize_size(window.preferred_size());
        window
            .window()
            .set_size(LogicalSize::new(logical.0, logical.1));
        slint::platform::update_timers_and_animations();

        let size = window.window().size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        let mut pixels = vec![PremultipliedRgbaColor::default(); width as usize * height as usize];
        window
            .software_renderer()
            .render(&mut pixels, width as usize);
        // Hiding fails only if the platform refuses; nothing to clean up then.
        let _ = instance.hide();

        Ok(RenderedPreview {
            path: request.path.clone(),
            component,
            image: to_rgba_image(width, height, &pixels),
            logical_size: logical,
            scale_factor,
            warnings: diagnostics,
            duration: start.elapsed(),
        })
    }
}

/// Finds `path` among the overrides, also trying its canonical form (the
/// compiler may spell import paths differently, e.g. with `..`).
fn lookup(overrides: &HashMap<PathBuf, String>, path: &Path) -> Option<String> {
    overrides
        .get(path)
        .or_else(|| path.canonicalize().ok().and_then(|p| overrides.get(&p)))
        .cloned()
}

/// Runs a future that never waits on I/O (our file loader answers at once).
fn block_on<F: Future>(future: F) -> Option<F::Output> {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    for _ in 0..10_000 {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return Some(output);
        }
        std::thread::yield_now();
    }
    None
}

fn sanitize_scale(scale_factor: f32) -> f32 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor.clamp(0.25, 8.0)
    } else {
        1.0
    }
}

fn sanitize_size(preferred: Option<LogicalSize>) -> (f32, f32) {
    let clean = |value: f32, default: f32| {
        if value.is_finite() && value >= 1.0 {
            value.min(MAX_SIZE)
        } else {
            default
        }
    };
    match preferred {
        Some(size) => (
            clean(size.width, DEFAULT_SIZE.0),
            clean(size.height, DEFAULT_SIZE.1),
        ),
        None => DEFAULT_SIZE,
    }
}

fn to_rgba_image(width: u32, height: u32, pixels: &[PremultipliedRgbaColor]) -> RgbaImage {
    let unpremultiply = |channel: u8, alpha: u8| {
        if alpha == 0 {
            0
        } else {
            u8::try_from((u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha))
                .unwrap_or(u8::MAX)
        }
    };
    let mut bytes = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels {
        bytes.extend_from_slice(&[
            unpremultiply(pixel.red, pixel.alpha),
            unpremultiply(pixel.green, pixel.alpha),
            unpremultiply(pixel.blue, pixel.alpha),
            pixel.alpha,
        ]);
    }
    // The buffer has exactly width × height pixels, so this cannot fail.
    RgbaImage::from_raw(width, height, bytes).unwrap_or_else(|| RgbaImage::new(width, height))
}

/// Converts a compiler diagnostic: Slint reports 1-based lines and columns
/// in UTF-8 bytes; ours are 0-based character columns.
fn convert_diagnostic(
    diagnostic: &slint_interpreter::Diagnostic,
    text_of: &dyn Fn(&Path) -> Option<String>,
) -> Diagnostic {
    let severity = match diagnostic.level() {
        DiagnosticLevel::Error => Severity::Error,
        DiagnosticLevel::Warning => Severity::Warning,
        _ => Severity::Info,
    };
    let location = diagnostic.source_file().map(|path| {
        let (line, column) = diagnostic.line_column();
        let start = LineCol::new(
            u32::try_from(line.saturating_sub(1)).unwrap_or(0),
            u32::try_from(column.saturating_sub(1)).unwrap_or(0),
        );
        match text_of(path) {
            Some(text) => {
                let index = LineIndex::new(&text);
                let start_byte = index.offset(&text, start, PositionEncoding::Utf8);
                let end_byte = (start_byte + diagnostic.length()).min(text.len());
                Location {
                    path: path.to_path_buf(),
                    start: index.position(&text, start_byte, PositionEncoding::Chars),
                    end: index.position(&text, end_byte, PositionEncoding::Chars),
                }
            }
            None => Location::at(path, start),
        }
    });
    Diagnostic {
        severity,
        source: DiagnosticSource::Preview,
        message: diagnostic.message().to_owned(),
        location,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_sanitized() {
        assert_eq!(sanitize_size(None), DEFAULT_SIZE);
        assert_eq!(
            sanitize_size(Some(LogicalSize::new(0.0, 50.0))),
            (DEFAULT_SIZE.0, 50.0)
        );
        assert_eq!(
            sanitize_size(Some(LogicalSize::new(f32::INFINITY, 1e9))),
            (DEFAULT_SIZE.0, MAX_SIZE)
        );
        assert!((sanitize_scale(f32::NAN) - 1.0).abs() < f32::EPSILON);
        assert!((sanitize_scale(2.0) - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn error_count_message_is_pluralised() {
        let error = Diagnostic {
            severity: Severity::Error,
            source: DiagnosticSource::Preview,
            message: String::new(),
            location: None,
        };
        let one = PreviewError::Compile {
            diagnostics: vec![error.clone()],
        };
        assert_eq!(one.to_string(), "il codice contiene un errore");
        let two = PreviewError::Compile {
            diagnostics: vec![error.clone(), error],
        };
        assert_eq!(two.to_string(), "il codice contiene 2 errori");
    }

    #[test]
    fn unpremultiplies_colours() {
        let pixels = [
            PremultipliedRgbaColor {
                red: 255,
                green: 0,
                blue: 0,
                alpha: 255,
            },
            PremultipliedRgbaColor {
                red: 64,
                green: 32,
                blue: 0,
                alpha: 128,
            },
            PremultipliedRgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 0,
            },
        ];
        let image = to_rgba_image(3, 1, &pixels);
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [128, 64, 0, 128]);
        assert_eq!(image.get_pixel(2, 0).0, [0, 0, 0, 0]);
    }
}
