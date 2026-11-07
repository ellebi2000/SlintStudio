//! `tracing` setup: logs go to stderr and to the in-app "Output" panel.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::ThreadId;

use slint_studio_core::{LogBuffer, LogEntry, LogLevel};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_log::NormalizeEvent as _;
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer};

/// Default filter when `RUST_LOG` is not set: our crates at `info`, noisy
/// graphics crates only when they have something important to say.
const DEFAULT_FILTER: &str = "info,wgpu_core=warn,wgpu_hal=warn,naga=warn,egui_wgpu=warn";

/// Lets the log layer wake up the UI once the egui context exists.
#[derive(Debug, Clone, Default)]
pub struct RepaintSignal {
    target: Arc<Mutex<Option<(egui::Context, ThreadId)>>>,
}

impl RepaintSignal {
    /// Connects the signal to the UI. Must be called from the UI thread.
    ///
    /// Re-attaching replaces the previous context: this happens when the
    /// window is recreated with a fallback renderer.
    pub fn attach(&self, ctx: &egui::Context) {
        *self.lock() = Some((ctx.clone(), std::thread::current().id()));
    }

    fn notify(&self) {
        // Clone out of the lock: `request_repaint` may block on egui's own lock
        // and must not be called while holding ours.
        let Some((ctx, ui_thread)) = self.lock().clone() else {
            return;
        };
        // Events emitted on the UI thread may come from inside egui while it
        // holds its context lock, and `request_repaint` would then deadlock.
        // The UI thread is repainting anyway, so only wake it from elsewhere.
        if std::thread::current().id() != ui_thread {
            ctx.request_repaint();
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<(egui::Context, ThreadId)>> {
        self.target.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Installs the global tracing subscriber. Respects `RUST_LOG`.
pub fn init(buffer: LogBuffer) -> anyhow::Result<RepaintSignal> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let signal = RepaintSignal::default();

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(UiLogLayer {
            buffer,
            signal: signal.clone(),
        })
        .try_init()?;

    Ok(signal)
}

/// Forwards every event to the shared [`LogBuffer`].
struct UiLogLayer {
    buffer: LogBuffer,
    signal: RepaintSignal,
}

impl<S: Subscriber> Layer<S> for UiLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        // Events bridged from the `log` crate (used by eframe, winit, …) carry
        // their real target in `log.*` fields; normalise them.
        let normalized = event.normalized_metadata();
        let metadata = normalized.as_ref().unwrap_or_else(|| event.metadata());
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        self.buffer.push(LogEntry::new(
            level(*metadata.level()),
            metadata.target(),
            visitor.finish(),
        ));
        self.signal.notify();
    }
}

fn level(level: Level) -> LogLevel {
    match level {
        Level::TRACE => LogLevel::Trace,
        Level::DEBUG => LogLevel::Debug,
        Level::INFO => LogLevel::Info,
        Level::WARN => LogLevel::Warn,
        Level::ERROR => LogLevel::Error,
    }
}

/// Renders an event as `message key=value key=value`.
#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: String,
}

impl MessageVisitor {
    fn finish(mut self) -> String {
        if !self.fields.is_empty() {
            if !self.message.is_empty() {
                self.message.push(' ');
            }
            self.message.push_str(&self.fields);
        }
        self.message
    }
}

impl Visit for MessageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.record_debug(field, &value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name().starts_with("log.") {
            // Metadata of events bridged from the `log` crate, already normalised.
        } else if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            if !self.fields.is_empty() {
                self.fields.push(' ');
            }
            let _ = write!(self.fields, "{}={value:?}", field.name());
        }
    }
}
