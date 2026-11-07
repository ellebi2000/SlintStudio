//! Slint Studio — an IDE for the Slint UI toolkit, in the spirit of Qt Creator.

mod app;
mod commands;
mod designer;
mod fonts;
mod layout;
mod logging;
mod lsp;
mod preview;
mod runner;
mod theme;
mod ui;
mod watcher;
mod workspace;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::anyhow;
use eframe::Renderer;
use slint_studio_compiler::viewer::SLINT_VIEWER;
use slint_studio_core::LogBuffer;
use slint_studio_core::install::{InstallProgress, ManagedTool};
use slint_studio_lsp::install::{SLINT_LSP, SLINT_LSP_VERSION};
use tracing::{info, warn};

use crate::app::StartupOptions;
use crate::logging::RepaintSignal;

/// Forces a rendering backend: `wgpu` or `glow` (OpenGL).
const RENDERER_ENV: &str = "SLINT_STUDIO_RENDERER";

const USAGE: &str = "\
Uso: slint-studio [OPZIONI] [CARTELLA]

Apre Slint Studio, opzionalmente sul progetto in CARTELLA.

Opzioni:
      --install-lsp     scarica e installa slint-lsp, poi esce
      --install-viewer  scarica e installa slint-viewer, poi esce
  -h, --help            mostra questo aiuto
  -V, --version         mostra la versione

Variabili d'ambiente:
  SLINT_STUDIO_LSP       eseguibile slint-lsp da usare al posto di quello gestito
  SLINT_STUDIO_VIEWER    eseguibile slint-viewer da usare al posto di quello gestito
  SLINT_STUDIO_RENDERER  renderer grafico: wgpu (predefinito) o glow
  RUST_LOG               livello di log (es. debug)";

enum Invocation {
    Run(Option<PathBuf>),
    Install(ManagedTool),
    Help,
    Version,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Invocation, String> {
    let mut project = None;
    for arg in args {
        match arg.as_str() {
            "--install-lsp" => return Ok(Invocation::Install(SLINT_LSP)),
            "--install-viewer" => return Ok(Invocation::Install(SLINT_VIEWER)),
            "-h" | "--help" => return Ok(Invocation::Help),
            "-V" | "--version" => return Ok(Invocation::Version),
            option if option.starts_with('-') => {
                return Err(format!("opzione sconosciuta: {option}"));
            }
            path if project.is_none() => project = Some(PathBuf::from(path)),
            extra => return Err(format!("argomento inatteso: {extra}")),
        }
    }
    Ok(Invocation::Run(project))
}

fn main() -> ExitCode {
    match parse_args(std::env::args().skip(1)) {
        Ok(Invocation::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Invocation::Version) => {
            println!(
                "slint-studio {} (slint-lsp {SLINT_LSP_VERSION}, slint-viewer {})",
                env!("CARGO_PKG_VERSION"),
                SLINT_VIEWER.version
            );
            ExitCode::SUCCESS
        }
        Ok(Invocation::Install(tool)) => install_tool(&tool),
        Ok(Invocation::Run(project)) => match run_gui(project) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("errore: {err:#}");
                ExitCode::FAILURE
            }
        },
        Err(message) => {
            eprintln!("errore: {message}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// `--install-lsp` / `--install-viewer`: what an installer runs to provision
/// the managed tools.
fn install_tool(tool: &ManagedTool) -> ExitCode {
    let (name, version) = (tool.name, tool.version);
    if let Some(path) = tool.managed_binary().filter(|p| p.is_file()) {
        println!("{name} {version} è già installato: {}", path.display());
        return ExitCode::SUCCESS;
    }
    println!("Download di {name} {version}…");
    let mut last_percent = None;
    let result = tool.install(&mut |progress| match progress {
        InstallProgress::Downloading {
            received,
            total: Some(total),
        } if total > 0 => {
            let percent = received * 100 / total;
            if last_percent != Some(percent / 10) {
                last_percent = Some(percent / 10);
                println!("  {percent}%");
            }
        }
        InstallProgress::Downloading { .. } => {}
        InstallProgress::Verifying => println!("Verifica del checksum SHA-256…"),
        InstallProgress::Extracting => println!("Estrazione…"),
    });
    match result {
        Ok(path) => {
            println!("{name} installato in {}", path.display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("errore: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_gui(project: Option<PathBuf>) -> anyhow::Result<()> {
    let log = LogBuffer::default();
    let repaint = logging::init(log.clone())?;
    info!("Avvio di Slint Studio v{}", env!("CARGO_PKG_VERSION"));

    let options = StartupOptions {
        project,
        language_server: true,
    };
    let result = match renderer_from_env() {
        Some(renderer) => run(renderer, &log, &repaint, &options),
        // wgpu first; machines without Vulkan/Metal/DX12/EGL (VMs, remote
        // desktops, old GPUs) often still have a working OpenGL driver.
        None => match run(Renderer::Wgpu, &log, &repaint, &options) {
            Err(eframe::Error::Wgpu(err)) => {
                warn!("Inizializzazione di wgpu non riuscita ({err}): uso il renderer OpenGL");
                run(Renderer::Glow, &log, &repaint, &options)
            }
            other => other,
        },
    };
    result.map_err(|err| anyhow!("impossibile avviare l'interfaccia grafica: {err}"))
}

fn run(
    renderer: Renderer,
    log: &LogBuffer,
    repaint: &RepaintSignal,
    options: &StartupOptions,
) -> eframe::Result {
    info!("Renderer: {renderer}");
    let native_options = eframe::NativeOptions {
        renderer,
        viewport: egui::ViewportBuilder::default()
            .with_title("Slint Studio")
            .with_app_id("slint-studio")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };

    let (log, repaint, options) = (log.clone(), repaint.clone(), options.clone());
    eframe::run_native(
        "Slint Studio",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::StudioApp::new(cc, log, repaint, options)))),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Invocation, String> {
        parse_args(args.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn command_line_arguments() {
        assert!(matches!(parse(&[]), Ok(Invocation::Run(None))));
        assert!(
            matches!(parse(&["progetto"]), Ok(Invocation::Run(Some(p))) if p == std::path::Path::new("progetto"))
        );
        assert!(matches!(
            parse(&["--install-lsp"]),
            Ok(Invocation::Install(tool)) if tool.name == "slint-lsp"
        ));
        assert!(matches!(
            parse(&["--install-viewer"]),
            Ok(Invocation::Install(tool)) if tool.name == "slint-viewer"
        ));
        assert!(matches!(parse(&["-h"]), Ok(Invocation::Help)));
        assert!(matches!(parse(&["--version"]), Ok(Invocation::Version)));
        assert!(parse(&["--boh"]).is_err());
        assert!(parse(&["a", "b"]).is_err());
    }
}
