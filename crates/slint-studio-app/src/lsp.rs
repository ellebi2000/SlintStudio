//! Lifecycle of the language server: locating or installing `slint-lsp`,
//! starting and restarting it, and turning its state into UI status.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use slint_studio_core::PositionEncoding;
use slint_studio_lsp::install::{
    self, BinarySource, InstallProgress, LocatedBinary, OVERRIDE_ENV, SLINT_LSP_VERSION,
};
use slint_studio_lsp::{LspClient, LspEvent, MessageLevel, ServerConfig, Waker};
use tracing::{debug, error, info, warn};

/// What the status bar shows about the language server.
#[derive(Debug, Clone, PartialEq)]
pub enum LspStatus {
    /// No project open: nothing to serve.
    Idle,
    Downloading {
        received: u64,
        total: Option<u64>,
    },
    Installing,
    Starting,
    Ready {
        server: String,
    },
    Failed {
        message: String,
    },
}

impl LspStatus {
    pub fn label(&self) -> String {
        match self {
            Self::Idle => "LSP: inattivo".to_owned(),
            Self::Downloading {
                received,
                total: Some(total),
            } if *total > 0 => format!("LSP: download {}%", received * 100 / total),
            Self::Downloading { received, .. } => {
                format!("LSP: download {:.1} MB", *received as f64 / 1_048_576.0)
            }
            Self::Installing => "LSP: installazione…".to_owned(),
            Self::Starting => "LSP: avvio…".to_owned(),
            Self::Ready { server } => format!("LSP: {server}"),
            Self::Failed { .. } => "LSP: errore".to_owned(),
        }
    }

    pub fn tooltip(&self) -> String {
        match self {
            Self::Idle => "Il language server si avvia all'apertura di un progetto".to_owned(),
            Self::Downloading { .. } | Self::Installing => format!(
                "Download di slint-lsp {SLINT_LSP_VERSION} dalle release ufficiali di Slint"
            ),
            Self::Starting => "Inizializzazione del language server".to_owned(),
            Self::Ready { .. } => "Diagnostica, completamento e definizioni attivi".to_owned(),
            Self::Failed { message } => message.clone(),
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

enum InstallUpdate {
    Progress(InstallProgress),
    Finished(Result<PathBuf, String>),
}

pub struct LspManager {
    status: LspStatus,
    root: Option<PathBuf>,
    client: Option<LspClient>,
    events: Option<Receiver<LspEvent>>,
    install: Option<Receiver<InstallUpdate>>,
    encoding: PositionEncoding,
    waker: Waker,
}

impl LspManager {
    pub fn new(waker: Waker) -> Self {
        Self {
            status: LspStatus::Idle,
            root: None,
            client: None,
            events: None,
            install: None,
            encoding: PositionEncoding::Utf16,
            waker,
        }
    }

    pub fn status(&self) -> &LspStatus {
        &self.status
    }

    /// The client, once the server is initialised.
    pub fn client(&self) -> Option<&LspClient> {
        self.client.as_ref().filter(|client| client.is_ready())
    }

    /// Column encoding of positions exchanged with the server.
    pub fn encoding(&self) -> PositionEncoding {
        self.encoding
    }

    /// Starts a server for the project at `root`, installing `slint-lsp`
    /// first if needed.
    pub fn start(&mut self, root: &Path) {
        self.stop();
        self.root = Some(root.to_path_buf());
        match install::locate_installed() {
            Some(binary) => self.spawn(&binary),
            None => self.begin_install(),
        }
    }

    pub fn restart(&mut self) {
        if let Some(root) = self.root.clone() {
            info!("Riavvio del language server");
            self.start(&root);
        }
    }

    /// Stops the server (a running download continues and is kept).
    pub fn stop(&mut self) {
        if let Some(client) = self.client.take() {
            client.shutdown();
        }
        self.events = None;
        self.root = None;
        if self.install.is_none() {
            self.status = LspStatus::Idle;
        }
    }

    fn begin_install(&mut self) {
        self.status = LspStatus::Downloading {
            received: 0,
            total: None,
        };
        if self.install.is_some() {
            return; // already downloading; it will start the server when done
        }
        info!("slint-lsp {SLINT_LSP_VERSION} non installato: download in corso");
        let (tx, rx) = mpsc::channel();
        let waker = self.waker.clone();
        let spawned = thread::Builder::new()
            .name("slint-lsp-install".into())
            .spawn(move || {
                let result = install::install(&mut |progress| {
                    let _ = tx.send(InstallUpdate::Progress(progress));
                    waker();
                });
                let _ = tx.send(InstallUpdate::Finished(result.map_err(|e| e.to_string())));
                waker();
            });
        match spawned {
            Ok(_) => self.install = Some(rx),
            Err(err) => self.fail(format!(
                "impossibile avviare il download di slint-lsp: {err}"
            )),
        }
    }

    fn spawn(&mut self, binary: &LocatedBinary) {
        let Some(root) = self.root.clone() else {
            return;
        };
        info!(
            "Avvio di {} ({})",
            binary.path.display(),
            binary.source.label()
        );
        let config = ServerConfig {
            binary: binary.path.clone(),
            args: Vec::new(),
            root,
        };
        match LspClient::start(&config, self.waker.clone()) {
            Ok((client, events)) => {
                self.client = Some(client);
                self.events = Some(events);
                self.status = LspStatus::Starting;
            }
            Err(err) => self.fail(format!(
                "impossibile avviare {}: {err}",
                binary.path.display()
            )),
        }
    }

    fn fail(&mut self, message: String) {
        error!("{message}");
        self.status = LspStatus::Failed { message };
    }

    /// Processes background progress and returns the server's events for
    /// the application to act on. Call once per frame.
    pub fn poll(&mut self) -> Vec<LspEvent> {
        self.poll_install();

        let events: Vec<LspEvent> = self
            .events
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for event in &events {
            match event {
                LspEvent::Ready { server, encoding } => {
                    info!("Language server pronto: {server} (posizioni {encoding:?})");
                    self.encoding = *encoding;
                    self.status = LspStatus::Ready {
                        server: server.clone(),
                    };
                }
                LspEvent::Exited { code, stderr_tail } => {
                    self.client = None;
                    self.events = None;
                    for line in stderr_tail {
                        warn!(target: "slint_lsp", "{line}");
                    }
                    let detail = stderr_tail
                        .last()
                        .map(|line| format!(": {line}"))
                        .unwrap_or_default();
                    let code = code.map_or_else(|| "?".to_owned(), |c| c.to_string());
                    self.fail(format!("slint-lsp è terminato (codice {code}){detail}"));
                }
                LspEvent::Message { level, text } => match level {
                    MessageLevel::Error => error!(target: "slint_lsp", "{text}"),
                    MessageLevel::Warning => warn!(target: "slint_lsp", "{text}"),
                    MessageLevel::Info => info!(target: "slint_lsp", "{text}"),
                    MessageLevel::Log => debug!(target: "slint_lsp", "{text}"),
                },
                _ => {}
            }
        }
        events
    }

    fn poll_install(&mut self) {
        let Some(rx) = &self.install else {
            return;
        };
        let updates: Vec<InstallUpdate> = rx.try_iter().collect();
        for update in updates {
            match update {
                InstallUpdate::Progress(progress) if self.root.is_some() => {
                    self.status = match progress {
                        InstallProgress::Downloading { received, total } => {
                            LspStatus::Downloading { received, total }
                        }
                        InstallProgress::Verifying | InstallProgress::Extracting => {
                            LspStatus::Installing
                        }
                    };
                }
                InstallUpdate::Progress(_) => {}
                InstallUpdate::Finished(result) => {
                    self.install = None;
                    self.finish_install(result);
                }
            }
        }
    }

    fn finish_install(&mut self, result: Result<PathBuf, String>) {
        match result {
            Ok(path) => {
                info!(
                    "slint-lsp {SLINT_LSP_VERSION} installato in {}",
                    path.display()
                );
                if self.root.is_some() && self.client.is_none() {
                    self.spawn(&LocatedBinary {
                        path,
                        source: BinarySource::Managed,
                    });
                }
            }
            Err(err) => {
                warn!("Download di slint-lsp non riuscito: {err}");
                if self.root.is_none() {
                    self.status = LspStatus::Idle;
                } else if let Some(path) = install::find_in_path() {
                    warn!("Uso slint-lsp dal PATH: {}", path.display());
                    self.spawn(&LocatedBinary {
                        path,
                        source: BinarySource::SystemPath,
                    });
                } else {
                    self.fail(format!(
                        "slint-lsp non disponibile ({err}). Installalo con \
                         `cargo install slint-lsp --version {SLINT_LSP_VERSION}` \
                         oppure indica l'eseguibile con la variabile {OVERRIDE_ENV}"
                    ));
                }
            }
        }
    }
}

impl Drop for LspManager {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_labels() {
        let downloading = LspStatus::Downloading {
            received: 5 * 1_048_576,
            total: Some(20 * 1_048_576),
        };
        assert_eq!(downloading.label(), "LSP: download 25%");
        let unknown_size = LspStatus::Downloading {
            received: 3 * 1_048_576,
            total: None,
        };
        assert_eq!(unknown_size.label(), "LSP: download 3.0 MB");
        let failed = LspStatus::Failed {
            message: "boom".into(),
        };
        assert!(failed.is_error());
        assert_eq!(failed.tooltip(), "boom");
    }
}
