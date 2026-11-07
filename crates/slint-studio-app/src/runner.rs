//! "Compila" and "Esegui" from the UI's point of view: the current job, the
//! terminal output and the status shown in the status bar.

use std::time::{Duration, Instant};

use slint_studio_compiler::{Job, JobEvent, JobKind, JobOutcome, JobPhase, JobRequest, Waker};
use slint_studio_core::{AnsiColor, AnsiStyle, Diagnostic, Terminal, TerminalStream};
use tracing::{error, info, warn};

/// What the status bar and the terminal header show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerStatus {
    Idle,
    Working {
        kind: JobKind,
        phase: JobPhase,
        started: Instant,
    },
    Finished {
        kind: JobKind,
        outcome: JobOutcome,
        duration: Duration,
    },
}

impl RunnerStatus {
    pub fn is_working(&self) -> bool {
        matches!(self, Self::Working { .. })
    }

    /// Whether the last job ended badly (shown in the error colour).
    pub fn is_failure(&self) -> bool {
        matches!(
            self,
            Self::Finished {
                outcome: JobOutcome::BuildFailed { .. }
                    | JobOutcome::Failed(_)
                    | JobOutcome::Exited { success: false, .. },
                ..
            }
        )
    }

    /// Short text for the status bar.
    pub fn label(&self) -> String {
        match self {
            Self::Idle => String::new(),
            Self::Working { phase, kind, .. } => match phase {
                JobPhase::Downloading {
                    tool,
                    received,
                    total: Some(total),
                } if *total > 0 => format!("Download {tool} {}%", received * 100 / total),
                JobPhase::Downloading { tool, .. } => format!("Download {tool}…"),
                JobPhase::Building if *kind == JobKind::Build => "Compilazione…".to_owned(),
                JobPhase::Building => "Preparazione…".to_owned(),
                JobPhase::Running { .. } => "In esecuzione".to_owned(),
            },
            Self::Finished { kind, outcome, .. } => match outcome {
                JobOutcome::Built { .. } => "Compilazione riuscita".to_owned(),
                JobOutcome::BuildFailed { .. } => "Compilazione non riuscita".to_owned(),
                JobOutcome::Exited { success: true, .. } => "Programma terminato".to_owned(),
                JobOutcome::Exited { description, .. } => {
                    format!("Programma terminato ({description})")
                }
                JobOutcome::Stopped => format!("{} interrotta", kind.label()),
                JobOutcome::Failed(_) => format!("{} non riuscita", kind.label()),
            },
        }
    }

    /// The line written at the end of the terminal output.
    pub fn summary(&self) -> Option<String> {
        let Self::Finished {
            kind,
            outcome,
            duration,
        } = self
        else {
            return None;
        };
        let time = format_duration(*duration);
        Some(match outcome {
            JobOutcome::Built { warnings: 0 } => format!("Compilazione riuscita in {time}"),
            JobOutcome::Built { warnings } => format!(
                "Compilazione riuscita in {time} ({warnings} {})",
                if *warnings == 1 { "avviso" } else { "avvisi" }
            ),
            JobOutcome::BuildFailed { errors: 0 } => {
                "Compilazione non riuscita: vedi l'output qui sopra".to_owned()
            }
            JobOutcome::BuildFailed { errors } => format!(
                "Compilazione non riuscita: {errors} {} (pannello Errori)",
                if *errors == 1 { "errore" } else { "errori" }
            ),
            JobOutcome::Exited { description, .. } => {
                format!("Programma terminato con {description} dopo {time}")
            }
            JobOutcome::Stopped => format!("{} interrotta dopo {time}", kind.label()),
            JobOutcome::Failed(message) => format!("{} non riuscita: {message}", kind.label()),
        })
    }
}

/// `2,4 s` / `1 min 05 s`, Italian style.
pub fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs_f64();
    if seconds < 60.0 {
        format!("{seconds:.1} s").replace('.', ",")
    } else {
        let total = duration.as_secs();
        format!("{} min {:02} s", total / 60, total % 60)
    }
}

/// What the app must act on after [`Runner::poll`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerUpdate {
    Diagnostic(Diagnostic),
    Finished(JobKind, JobOutcome),
}

pub struct Runner {
    job: Option<Job>,
    pub terminal: Terminal,
    pub status: RunnerStatus,
    /// Command line of the running program, for the terminal header.
    pub command: Option<String>,
    pub auto_scroll: bool,
    waker: Waker,
}

impl Runner {
    pub fn new(waker: Waker) -> Self {
        Self {
            job: None,
            terminal: Terminal::default(),
            status: RunnerStatus::Idle,
            command: None,
            auto_scroll: true,
            waker,
        }
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    pub fn is_stopping(&self) -> bool {
        self.job.as_ref().is_some_and(Job::is_stopping)
    }

    /// Starts a job, stopping the current one (its output is cleared, as in
    /// Qt Creator).
    pub fn start(&mut self, request: JobRequest) {
        if let Some(previous) = self.job.take() {
            info!("{} precedente interrotta", previous.kind().label());
            previous.stop();
        }
        let kind = request.kind;
        self.terminal.clear();
        self.command = None;
        match Job::start(request, self.waker.clone()) {
            Ok(job) => {
                self.job = Some(job);
                self.status = RunnerStatus::Working {
                    kind,
                    phase: JobPhase::Building,
                    started: Instant::now(),
                };
            }
            Err(err) => {
                error!("Impossibile avviare {}: {err}", kind.label().to_lowercase());
                self.finish(
                    kind,
                    JobOutcome::Failed(format!("thread non avviato: {err}")),
                    Duration::ZERO,
                );
            }
        }
    }

    pub fn stop(&self) {
        if let Some(job) = &self.job {
            info!("Interruzione di {}", job.kind().label().to_lowercase());
            job.stop();
        }
    }

    /// Moves the job's events into the terminal; returns what the app must
    /// handle (diagnostics, completion).
    pub fn poll(&mut self) -> Vec<RunnerUpdate> {
        let Some(job) = &self.job else {
            return Vec::new();
        };
        let kind = job.kind();
        let mut updates = Vec::new();
        let mut finished = None;
        for event in job.poll() {
            match event {
                JobEvent::Output { stream, text } => self.terminal.push(stream, &text),
                JobEvent::Info(text) => self.terminal.push(TerminalStream::System, &text),
                JobEvent::Diagnostic(diagnostic) => {
                    updates.push(RunnerUpdate::Diagnostic(diagnostic));
                }
                JobEvent::Phase(phase) => {
                    if let JobPhase::Running { command } = &phase {
                        self.command = Some(command.clone());
                        self.terminal.reset_styles();
                    }
                    if let RunnerStatus::Working { phase: current, .. } = &mut self.status {
                        *current = phase;
                    }
                }
                JobEvent::Finished(outcome) => finished = Some(outcome),
            }
        }
        if let Some(outcome) = finished {
            self.job = None;
            let duration = match &self.status {
                RunnerStatus::Working { started, .. } => started.elapsed(),
                _ => Duration::ZERO,
            };
            updates.push(RunnerUpdate::Finished(kind, outcome.clone()));
            self.finish(kind, outcome, duration);
        }
        updates
    }

    fn finish(&mut self, kind: JobKind, outcome: JobOutcome, duration: Duration) {
        self.status = RunnerStatus::Finished {
            kind,
            outcome,
            duration,
        };
        let color = if self.status.is_failure() { 1 } else { 2 };
        if let Some(summary) = self.status.summary() {
            match &self.status {
                status if status.is_failure() => warn!("{summary}"),
                _ => info!("{summary}"),
            }
            self.terminal.reset_styles();
            self.terminal.push_styled(
                TerminalStream::System,
                &format!("■ {summary}"),
                AnsiStyle {
                    foreground: Some(AnsiColor::Indexed(color)),
                    bold: true,
                    ..AnsiStyle::default()
                },
            );
        }
    }

    pub fn clear(&mut self) {
        self.terminal.clear();
        if !self.is_running() {
            self.status = RunnerStatus::Idle;
            self.command = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint_studio_compiler::ProjectKind;
    use std::fs;
    use std::sync::Arc;

    fn wait(runner: &mut Runner) -> Vec<RunnerUpdate> {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut updates = Vec::new();
        while runner.is_running() {
            assert!(Instant::now() < deadline, "job did not finish");
            updates.extend(runner.poll());
            std::thread::sleep(Duration::from_millis(10));
        }
        updates
    }

    #[test]
    fn check_job_fills_terminal_and_reports_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("main.slint");
        fs::write(&file, "export component M inherits Window { Buton {} }").unwrap();
        let mut runner = Runner::new(Arc::new(|| {}));
        assert_eq!(runner.status, RunnerStatus::Idle);
        runner.start(JobRequest {
            kind: JobKind::Build,
            project: ProjectKind::Slint,
            root: dir.path().to_path_buf(),
            slint_files: vec![file],
            run_target: None,
            style: "fluent".into(),
        });
        assert!(runner.is_running());
        assert!(runner.status.is_working());
        assert_eq!(runner.status.label(), "Compilazione…");

        let updates = wait(&mut runner);
        assert_eq!(updates.len(), 2, "{updates:#?}");
        assert!(matches!(updates[0], RunnerUpdate::Diagnostic(_)));
        assert_eq!(
            updates[1],
            RunnerUpdate::Finished(JobKind::Build, JobOutcome::BuildFailed { errors: 1 })
        );
        assert!(runner.status.is_failure());
        assert_eq!(runner.status.label(), "Compilazione non riuscita");
        let text = runner.terminal.plain_text();
        assert!(text.contains("errore: Unknown element 'Buton'"), "{text}");
        assert!(
            text.trim_end()
                .ends_with("■ Compilazione non riuscita: 1 errore (pannello Errori)"),
            "{text}"
        );

        runner.clear();
        assert!(runner.terminal.is_empty());
        assert_eq!(runner.status, RunnerStatus::Idle);
    }

    #[test]
    fn status_texts() {
        let finished = |outcome| RunnerStatus::Finished {
            kind: JobKind::Run,
            outcome,
            duration: Duration::from_millis(2400),
        };
        assert_eq!(
            finished(JobOutcome::Built { warnings: 2 })
                .summary()
                .unwrap(),
            "Compilazione riuscita in 2,4 s (2 avvisi)"
        );
        let exited = finished(JobOutcome::Exited {
            success: true,
            description: "codice 0".into(),
        });
        assert!(!exited.is_failure());
        assert_eq!(
            exited.summary().unwrap(),
            "Programma terminato con codice 0 dopo 2,4 s"
        );
        let stopped = finished(JobOutcome::Stopped);
        assert_eq!(stopped.label(), "Esecuzione interrotta");
        assert!(!stopped.is_failure());
        let failed = finished(JobOutcome::Failed("cargo non trovato".into()));
        assert!(failed.is_failure());
        assert_eq!(
            failed.summary().unwrap(),
            "Esecuzione non riuscita: cargo non trovato"
        );
        let downloading = RunnerStatus::Working {
            kind: JobKind::Run,
            phase: JobPhase::Downloading {
                tool: "slint-viewer",
                received: 50,
                total: Some(200),
            },
            started: Instant::now(),
        };
        assert_eq!(downloading.label(), "Download slint-viewer 25%");
        assert_eq!(RunnerStatus::Idle.summary(), None);
        assert_eq!(format_duration(Duration::from_secs(65)), "1 min 05 s");
    }
}
