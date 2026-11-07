//! Slint Studio — an IDE for the Slint UI toolkit, in the spirit of Qt Creator.

mod app;
mod commands;
mod fonts;
mod layout;
mod logging;
mod theme;
mod ui;

use anyhow::anyhow;
use eframe::Renderer;
use slint_studio_core::LogBuffer;
use tracing::{info, warn};

use crate::logging::RepaintSignal;

/// Forces a rendering backend: `wgpu` or `glow` (OpenGL).
const RENDERER_ENV: &str = "SLINT_STUDIO_RENDERER";

fn main() -> anyhow::Result<()> {
    let log = LogBuffer::default();
    let repaint = logging::init(log.clone())?;
    info!("Avvio di Slint Studio v{}", env!("CARGO_PKG_VERSION"));

    let result = match renderer_from_env() {
        Some(renderer) => run(renderer, &log, &repaint),
        // wgpu first; machines without Vulkan/Metal/DX12/EGL (VMs, remote
        // desktops, old GPUs) often still have a working OpenGL driver.
        None => match run(Renderer::Wgpu, &log, &repaint) {
            Err(eframe::Error::Wgpu(err)) => {
                warn!("Inizializzazione di wgpu non riuscita ({err}): uso il renderer OpenGL");
                run(Renderer::Glow, &log, &repaint)
            }
            other => other,
        },
    };
    result.map_err(|err| anyhow!("impossibile avviare l'interfaccia grafica: {err}"))
}

fn run(renderer: Renderer, log: &LogBuffer, repaint: &RepaintSignal) -> eframe::Result {
    info!("Renderer: {renderer}");
    let options = eframe::NativeOptions {
        renderer,
        viewport: egui::ViewportBuilder::default()
            .with_title("Slint Studio")
            .with_app_id("slint-studio")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };

    let (log, repaint) = (log.clone(), repaint.clone());
    eframe::run_native(
        "Slint Studio",
        options,
        Box::new(move |cc| Ok(Box::new(app::StudioApp::new(cc, log, repaint)))),
    )
}

fn renderer_from_env() -> Option<Renderer> {
    let value = std::env::var(RENDERER_ENV).ok()?;
    match value.trim().parse() {
        Ok(renderer) => Some(renderer),
        Err(_) => {
            warn!("{RENDERER_ENV}={value:?} non riconosciuto (valori ammessi: wgpu, glow)");
            None
        }
    }
}
