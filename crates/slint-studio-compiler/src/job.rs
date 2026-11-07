//! "Compila" and "Esegui" as background jobs.
//!
//! A [`Job`] runs on its own thread and reports through [`JobEvent`]s:
//! terminal output, diagnostics for the "Errori" panel, phase changes and,
//! last, a [`JobOutcome`]. The UI drains the events with [`Job::poll`] and can
//! stop the job at any time with [`Job::stop`].

use std::env;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use slint_studio_core::install::{InstallProgress, LocatedBinary};
use slint_studio_core::terminal::strip_ansi;
use slint_studio_core::{Diagnostic, Severity, TerminalStream};
use tracing::{debug, info, warn};

use crate::cargo::{StderrScanner, StdoutEvent, StdoutParser};
use crate::check;
use crate::process::{self, Process, ProcessCommand, Stopper};
use crate::project::ProjectKind;
use crate::viewer::{self, SLINT_VIEWER};

/// Called after each event is queued; typically requests a UI repaint.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Build,
    Run,
}

impl JobKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Build => "Compilazione",
            Self::Run => "Esecuzione",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRequest {
    pub kind: JobKind,
    pub project: ProjectKind,
    /// The open folder; also the working directory of processes.
    pub root: PathBuf,
    /// Slint projects: the files "Compila" checks.
    pub slint_files: Vec<PathBuf>,
    /// Slint projects: the file "Esegui" opens (the one in the preview).
    pub run_target: Option<PathBuf>,
    /// Slint widget style for checks and the viewer.
    pub style: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobPhase {
    /// Downloading a managed tool.
    Downloading {
        tool: &'static str,
        received: u64,
        total: Option<u64>,
    },
    Building,
    /// The program is running.
    Running {
        command: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    /// The build (or check) found no errors.
    Built { warnings: usize },
    /// The build found errors; they were sent as diagnostics (there may be
    /// none if the tool failed before reporting any: see the terminal).
    BuildFailed { errors: usize },
    /// The program ran and exited on its own.
    Exited { success: bool, description: String },
    /// Stopped by the user.
    Stopped,
    /// The job could not start (missing tool, download failure, …).
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobEvent {
    /// Output of a process, possibly with ANSI escape sequences.
    Output {
        stream: TerminalStream,
        text: String,
    },
    /// A message from Slint Studio (command line, progress, summary).
    Info(String),
    Diagnostic(Diagnostic),
    Phase(JobPhase),
    /// Always the last event.
    Finished(JobOutcome),
}

#[derive(Default)]
struct Control {
    stop: AtomicBool,
    process: Mutex<Option<Stopper>>,
}

impl Control {
    fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Makes `process` the one [`Job::stop`] stops.
    fn attach(&self, process: &Process) {
        let stopper = process.stopper();
        if self.stop_requested() {
            stopper.stop();
        }
        *self.process.lock().unwrap_or_else(PoisonError::into_inner) = Some(stopper);
    }

    fn detach(&self) {
        *self.process.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// Sends events to the UI.
#[derive(Clone)]
struct Emitter {
    sender: Sender<JobEvent>,
    waker: Waker,
}

impl Emitter {
    fn emit(&self, event: JobEvent) {
        // The UI may have dropped the job: nothing left to report to.
        if self.sender.send(event).is_ok() {
            (self.waker)();
        }
    }

    fn info(&self, text: impl Into<String>) {
        self.emit(JobEvent::Info(text.into()));
    }

    fn output(&self, stream: TerminalStream, text: String) {
        self.emit(JobEvent::Output { stream, text });
    }
}

/// A running "Compila" or "Esegui". Dropping it stops it.
pub struct Job {
    kind: JobKind,
    events: Receiver<JobEvent>,
    control: Arc<Control>,
}

impl Job {
    pub fn start(request: JobRequest, waker: Waker) -> io::Result<Self> {
        let (sender, events) = mpsc::channel();
        let control = Arc::new(Control::default());
        let kind = request.kind;
        let emitter = Emitter { sender, waker };
        {
            let control = Arc::clone(&control);
            thread::Builder::new()
                .name("slint-studio-job".into())
                .spawn(move || {
                    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
                        execute(&request, &emitter, &control)
                    }))
                    .unwrap_or_else(|_| JobOutcome::Failed("errore interno".into()));
                    control.detach();
                    info!("{}: {outcome:?}", request.kind.label());
                    emitter.emit(JobEvent::Finished(outcome));
                })?;
        }
        Ok(Self {
            kind,
            events,
            control,
        })
    }

    pub fn kind(&self) -> JobKind {
        self.kind
    }

    /// Stops the job: the running process (and its children) are terminated.
    pub fn stop(&self) {
        self.control.stop.store(true, Ordering::SeqCst);
        let process = self
            .control
            .process
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(stopper) = process.as_ref() {
            stopper.stop();
        }
    }

    pub fn is_stopping(&self) -> bool {
        self.control.stop_requested()
    }

    /// Events queued since the last call.
    pub fn poll(&self) -> Vec<JobEvent> {
        self.events.try_iter().collect()
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.stop();
    }
}

fn execute(request: &JobRequest, emitter: &Emitter, control: &Control) -> JobOutcome {
    match &request.project {
        ProjectKind::Cargo { .. } => run_cargo(request, emitter, control),
        ProjectKind::Slint => match request.kind {
            JobKind::Build => check_slint(request, emitter, control),
            JobKind::Run => run_viewer(request, emitter, control),
        },
        ProjectKind::Unsupported => JobOutcome::Failed(
            "progetto non supportato: serve un Cargo.toml o dei file .slint".into(),
        ),
    }
}

/// `cargo`: the rustup proxy in the `PATH` (it honours the project's
/// `rust-toolchain.toml`), else `$CARGO_HOME/bin`, else `$CARGO`.
pub fn find_cargo() -> Option<PathBuf> {
    let exe = format!("cargo{}", env::consts::EXE_SUFFIX);
    process::find_program("cargo")
        .or_else(|| {
            let home = env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| {
                env::var_os("HOME")
                    .or_else(|| env::var_os("USERPROFILE"))
                    .map(|home| PathBuf::from(home).join(".cargo"))
            })?;
            Some(home.join("bin").join(&exe)).filter(|p| p.is_file())
        })
        .or_else(|| {
            env::var_os("CARGO")
                .map(PathBuf::from)
                .filter(|p| p.is_file())
        })
}

/// The directory of the workspace's `Cargo.toml` (where rustc's relative
/// paths start), or `root` if cargo cannot tell.
fn workspace_root(cargo: &Path, root: &Path) -> PathBuf {
    let command = ProcessCommand::new(cargo, root)
        .arg("locate-project")
        .arg("--workspace")
        .arg("--message-format")
        .arg("plain");
    match command.output() {
        Ok(output) if output.status.success() => {
            let manifest = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            Path::new(&manifest)
                .parent()
                .map_or_else(|| root.to_path_buf(), Path::to_path_buf)
        }
        Ok(output) => {
            debug!(
                "cargo locate-project: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            root.to_path_buf()
        }
        Err(err) => {
            debug!("cargo locate-project: {err}");
            root.to_path_buf()
        }
    }
}

#[derive(Default)]
struct BuildCounters {
    errors: AtomicUsize,
    warnings: AtomicUsize,
    /// `build-finished` was seen: from here on, output is the program's.
    finished: AtomicBool,
    success: AtomicBool,
}

impl BuildCounters {
    fn count(&self, diagnostic: &Diagnostic) {
        match diagnostic.severity {
            Severity::Error => self.errors.fetch_add(1, Ordering::SeqCst),
            Severity::Warning => self.warnings.fetch_add(1, Ordering::SeqCst),
            Severity::Info | Severity::Hint => 0,
        };
    }
}

fn run_cargo(request: &JobRequest, emitter: &Emitter, control: &Control) -> JobOutcome {
    let Some(cargo) = find_cargo() else {
        return JobOutcome::Failed(
            "cargo non trovato: installa Rust da https://rustup.rs e riavvia Slint Studio".into(),
        );
    };
    let workspace = workspace_root(&cargo, &request.root);
    let subcommand = match request.kind {
        JobKind::Build => "build",
        JobKind::Run => "run",
    };
    let command = ProcessCommand::new(&cargo, &request.root)
        .arg(subcommand)
        .arg("--message-format=json-diagnostic-rendered-ansi")
        .arg("--color")
        .arg("always")
        .env("CARGO_TERM_COLOR", "always")
        .env("CARGO_TERM_PROGRESS_WHEN", "never")
        // Set by rustup for Slint Studio itself when started from cargo:
        // the project must pick its own toolchain.
        .env_remove("RUSTUP_TOOLCHAIN");
    let display = command.display();
    emitter.info(format!("▶ {display}"));
    emitter.emit(JobEvent::Phase(JobPhase::Building));

    let counters = Arc::new(BuildCounters::default());
    let scanner = Arc::new(Mutex::new(StderrScanner::new(&workspace)));

    let on_stdout: process::LineHandler = {
        let emitter = emitter.clone();
        let counters = Arc::clone(&counters);
        let mut parser = StdoutParser::new(&workspace);
        let run_command = display.clone();
        Box::new(move |line| {
            for event in parser.line(&line) {
                match event {
                    StdoutEvent::Output { text, from_build } => {
                        let stream = if from_build {
                            TerminalStream::Stderr
                        } else {
                            TerminalStream::Stdout
                        };
                        emitter.output(stream, text);
                    }
                    StdoutEvent::Diagnostic(diagnostic) => {
                        counters.count(&diagnostic);
                        emitter.emit(JobEvent::Diagnostic(diagnostic));
                    }
                    StdoutEvent::BuildFinished { success } => {
                        counters.success.store(success, Ordering::SeqCst);
                        counters.finished.store(true, Ordering::SeqCst);
                        if success && subcommand == "run" {
                            emitter.emit(JobEvent::Phase(JobPhase::Running {
                                command: run_command.clone(),
                            }));
                        }
                    }
                }
            }
        })
    };
    let on_stderr: process::LineHandler = {
        let emitter = emitter.clone();
        let counters = Arc::clone(&counters);
        let scanner = Arc::clone(&scanner);
        let mut program_started = false;
        Box::new(move |line| {
            // `Running `target/debug/app`` comes right before the program.
            if !program_started
                && (counters.finished.load(Ordering::SeqCst)
                    || strip_ansi(&line).trim_start().starts_with("Running `"))
            {
                program_started = true;
            }
            if !program_started {
                let found = scanner
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .line(&line);
                if let Some(diagnostic) = found {
                    counters.count(&diagnostic);
                    emitter.emit(JobEvent::Diagnostic(diagnostic));
                }
            }
            emitter.output(TerminalStream::Stderr, line);
        })
    };

    let mut process = match Process::spawn(&command, on_stdout, on_stderr) {
        Ok(process) => process,
        Err(err) => return JobOutcome::Failed(format!("impossibile avviare cargo: {err}")),
    };
    control.attach(&process);
    let status = process.wait();
    control.detach();

    let pending = scanner
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .finish();
    if let Some(diagnostic) = pending
        && !counters.finished.load(Ordering::SeqCst)
    {
        counters.count(&diagnostic);
        emitter.emit(JobEvent::Diagnostic(diagnostic));
    }

    let status = match status {
        Ok(status) => status,
        Err(err) => return JobOutcome::Failed(format!("attesa di cargo non riuscita: {err}")),
    };
    if process.was_stopped() || control.stop_requested() {
        return JobOutcome::Stopped;
    }
    let errors = counters.errors.load(Ordering::SeqCst);
    let warnings = counters.warnings.load(Ordering::SeqCst);
    let built = counters.success.load(Ordering::SeqCst);
    match request.kind {
        JobKind::Build if status.success() => JobOutcome::Built { warnings },
        JobKind::Run if built => JobOutcome::Exited {
            success: status.success(),
            description: process::describe_exit(status),
        },
        _ => JobOutcome::BuildFailed { errors },
    }
}

/// `path` relative to `root`, for messages.
fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Prints `diagnostic` like a compiler would (`file:line:col: errore: …`).
fn format_diagnostic(diagnostic: &Diagnostic, root: &Path) -> String {
    let (label, color) = match diagnostic.severity {
        Severity::Error => ("errore", "1;31"),
        Severity::Warning => ("avviso", "1;33"),
        Severity::Info | Severity::Hint => ("nota", "1;36"),
    };
    let location = diagnostic.location.as_ref().map_or_else(String::new, |l| {
        format!(
            "{}:{}:{}: ",
            relative(&l.path, root),
            l.start.line + 1,
            l.start.column + 1
        )
    });
    format!(
        "{location}\u{1b}[{color}m{label}\u{1b}[0m: {}",
        diagnostic.message
    )
}

fn report_check(
    diagnostics: &[Diagnostic],
    request: &JobRequest,
    emitter: &Emitter,
) -> (usize, usize) {
    let mut errors = 0;
    let mut warnings = 0;
    for diagnostic in diagnostics {
        match diagnostic.severity {
            Severity::Error => errors += 1,
            Severity::Warning => warnings += 1,
            Severity::Info | Severity::Hint => {}
        }
        emitter.output(
            TerminalStream::Stderr,
            format_diagnostic(diagnostic, &request.root),
        );
        emitter.emit(JobEvent::Diagnostic(diagnostic.clone()));
    }
    (errors, warnings)
}

fn summary(errors: usize, warnings: usize) -> String {
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    format!(
        "{}, {}",
        plural(errors, "errore", "errori"),
        plural(warnings, "avviso", "avvisi")
    )
}

fn check_slint(request: &JobRequest, emitter: &Emitter, control: &Control) -> JobOutcome {
    if request.slint_files.is_empty() {
        return JobOutcome::Failed("nessun file .slint nel progetto".into());
    }
    let count = request.slint_files.len();
    emitter.info(format!(
        "▶ Controllo di {count} file .slint con il compilatore Slint (stile {})",
        request.style
    ));
    emitter.emit(JobEvent::Phase(JobPhase::Building));
    let mut stopped = false;
    let mut diagnostics = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for path in &request.slint_files {
        if control.stop_requested() {
            stopped = true;
            break;
        }
        emitter.output(
            TerminalStream::Stdout,
            format!("   Compilazione di {}", relative(path, &request.root)),
        );
        for diagnostic in check::check_file(path, &request.style) {
            if seen.insert(diagnostic.clone()) {
                diagnostics.push(diagnostic);
            }
        }
    }
    let (errors, warnings) = report_check(&diagnostics, request, emitter);
    if stopped {
        return JobOutcome::Stopped;
    }
    emitter.info(format!(
        "Controllo terminato: {}",
        summary(errors, warnings)
    ));
    if errors > 0 {
        JobOutcome::BuildFailed { errors }
    } else {
        JobOutcome::Built { warnings }
    }
}

/// The managed `slint-viewer`, downloading it on first use; falls back to
/// one in the `PATH` if the download fails.
fn locate_viewer(emitter: &Emitter) -> Result<LocatedBinary, String> {
    if let Some(binary) = SLINT_VIEWER.locate_installed() {
        return Ok(binary);
    }
    emitter.info(format!(
        "Download di slint-viewer {} dalle release ufficiali di Slint…",
        SLINT_VIEWER.version
    ));
    let mut last_percent = None;
    let result = SLINT_VIEWER.install(&mut |progress| match progress {
        InstallProgress::Downloading { received, total } => {
            // One event per percent is plenty.
            let percent = total.filter(|t| *t > 0).map(|t| received * 100 / t);
            if percent.is_none() || percent != last_percent {
                last_percent = percent;
                emitter.emit(JobEvent::Phase(JobPhase::Downloading {
                    tool: SLINT_VIEWER.name,
                    received,
                    total,
                }));
            }
        }
        InstallProgress::Verifying => emitter.info("Verifica del checksum SHA-256…"),
        InstallProgress::Extracting => emitter.info("Estrazione…"),
    });
    match result {
        Ok(path) => {
            emitter.info(format!("slint-viewer installato in {}", path.display()));
            Ok(LocatedBinary {
                path,
                source: slint_studio_core::install::BinarySource::Managed,
            })
        }
        Err(err) => {
            warn!("installazione di slint-viewer non riuscita: {err}");
            match SLINT_VIEWER.find_in_path() {
                Some(path) => {
                    emitter.info(format!(
                        "Download non riuscito ({err}): uso {}",
                        path.display()
                    ));
                    Ok(LocatedBinary {
                        path,
                        source: slint_studio_core::install::BinarySource::SystemPath,
                    })
                }
                None => Err(format!(
                    "slint-viewer non disponibile: {err}. Installalo con `cargo install slint-viewer`"
                )),
            }
        }
    }
}

fn run_viewer(request: &JobRequest, emitter: &Emitter, control: &Control) -> JobOutcome {
    let Some(target) = request
        .run_target
        .clone()
        .or_else(|| request.slint_files.first().cloned())
    else {
        return JobOutcome::Failed("nessun file .slint da eseguire".into());
    };

    // Like `cargo run`: nothing starts if the file does not compile.
    emitter.info(format!(
        "▶ Controllo di {} (stile {})",
        relative(&target, &request.root),
        request.style
    ));
    emitter.emit(JobEvent::Phase(JobPhase::Building));
    let diagnostics = check::check_file(&target, &request.style);
    let (errors, _) = report_check(&diagnostics, request, emitter);
    if errors > 0 {
        return JobOutcome::BuildFailed { errors };
    }
    if control.stop_requested() {
        return JobOutcome::Stopped;
    }

    let viewer = match locate_viewer(emitter) {
        Ok(viewer) => viewer,
        Err(message) => return JobOutcome::Failed(message),
    };
    if control.stop_requested() {
        return JobOutcome::Stopped;
    }

    let command = viewer::command(&viewer.path, &target, &request.root, &request.style);
    let display = command.display();
    emitter.info(format!("▶ {display}  ({})", viewer.source.label()));
    emitter.emit(JobEvent::Phase(JobPhase::Running {
        command: display.clone(),
    }));
    let on_stdout: process::LineHandler = {
        let emitter = emitter.clone();
        Box::new(move |line| emitter.output(TerminalStream::Stdout, line))
    };
    let on_stderr: process::LineHandler = {
        let emitter = emitter.clone();
        Box::new(move |line| emitter.output(TerminalStream::Stderr, line))
    };
    let mut process = match Process::spawn(&command, on_stdout, on_stderr) {
        Ok(process) => process,
        Err(err) => {
            return JobOutcome::Failed(format!(
                "impossibile avviare {}: {err}",
                viewer.path.display()
            ));
        }
    };
    control.attach(&process);
    let status = process.wait();
    control.detach();
    match status {
        Ok(_) if process.was_stopped() || control.stop_requested() => JobOutcome::Stopped,
        Ok(status) => JobOutcome::Exited {
            success: status.success(),
            description: process::describe_exit(status),
        },
        Err(err) => JobOutcome::Failed(format!("attesa di slint-viewer non riuscita: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, Instant};

    fn wait_for_outcome(job: &Job, events: &mut Vec<JobEvent>) -> JobOutcome {
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            for event in job.poll() {
                let finished = match &event {
                    JobEvent::Finished(outcome) => Some(outcome.clone()),
                    _ => None,
                };
                events.push(event);
                if let Some(outcome) = finished {
                    return outcome;
                }
            }
            assert!(Instant::now() < deadline, "job did not finish: {events:#?}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn no_waker() -> Waker {
        Arc::new(|| {})
    }

    fn slint_request(dir: &Path, kind: JobKind, files: &[&str]) -> JobRequest {
        JobRequest {
            kind,
            project: ProjectKind::Slint,
            root: dir.to_path_buf(),
            slint_files: files.iter().map(|f| dir.join(f)).collect(),
            run_target: None,
            style: check::DEFAULT_STYLE.to_owned(),
        }
    }

    fn diagnostics(events: &[JobEvent]) -> Vec<&Diagnostic> {
        events
            .iter()
            .filter_map(|e| match e {
                JobEvent::Diagnostic(d) => Some(d),
                _ => None,
            })
            .collect()
    }

    fn output(events: &[JobEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                JobEvent::Output { text, .. } | JobEvent::Info(text) => Some(strip_ansi(text)),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn slint_check_reports_errors_and_a_summary() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(
            root.join("ok.slint"),
            "export component Ok inherits Window {}",
        )
        .unwrap();
        fs::write(
            root.join("broken.slint"),
            "export component Broken inherits Window { Buton {} }",
        )
        .unwrap();
        let job = Job::start(
            slint_request(&root, JobKind::Build, &["ok.slint", "broken.slint"]),
            no_waker(),
        )
        .unwrap();
        assert_eq!(job.kind(), JobKind::Build);
        let mut events = Vec::new();
        let outcome = wait_for_outcome(&job, &mut events);
        assert_eq!(outcome, JobOutcome::BuildFailed { errors: 1 });
        assert_eq!(
            events[0],
            JobEvent::Info(
                "▶ Controllo di 2 file .slint con il compilatore Slint (stile fluent)".into()
            )
        );
        assert_eq!(events[1], JobEvent::Phase(JobPhase::Building));
        let found = diagnostics(&events);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].location.as_ref().unwrap().path,
            root.join("broken.slint")
        );
        let text = output(&events);
        assert!(text.contains("Compilazione di ok.slint"), "{text}");
        assert!(
            text.contains("broken.slint:1:43: errore: Unknown element 'Buton'"),
            "{text}"
        );
        assert!(
            text.contains("Controllo terminato: 1 errore, 0 avvisi"),
            "{text}"
        );
    }

    #[test]
    fn slint_check_without_errors_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.slint"), "component A inherits Window {}").unwrap();
        let job = Job::start(
            slint_request(dir.path(), JobKind::Build, &["a.slint"]),
            no_waker(),
        )
        .unwrap();
        let mut events = Vec::new();
        assert_eq!(
            wait_for_outcome(&job, &mut events),
            JobOutcome::Built { warnings: 1 }
        );
        let empty = Job::start(slint_request(dir.path(), JobKind::Build, &[]), no_waker()).unwrap();
        assert!(matches!(
            wait_for_outcome(&empty, &mut Vec::new()),
            JobOutcome::Failed(_)
        ));
    }

    #[test]
    fn run_does_not_start_the_viewer_when_the_file_has_errors() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("main.slint"),
            "export component M { Buton {} }",
        )
        .unwrap();
        let mut request = slint_request(dir.path(), JobKind::Run, &["main.slint"]);
        request.run_target = Some(dir.path().join("main.slint"));
        let job = Job::start(request, no_waker()).unwrap();
        let mut events = Vec::new();
        assert_eq!(
            wait_for_outcome(&job, &mut events),
            JobOutcome::BuildFailed { errors: 1 }
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, JobEvent::Phase(JobPhase::Running { .. })))
        );
    }

    #[test]
    fn unsupported_projects_fail() {
        let request = JobRequest {
            kind: JobKind::Run,
            project: ProjectKind::Unsupported,
            root: std::env::temp_dir(),
            slint_files: Vec::new(),
            run_target: None,
            style: "fluent".into(),
        };
        let job = Job::start(request, no_waker()).unwrap();
        assert!(matches!(
            wait_for_outcome(&job, &mut Vec::new()),
            JobOutcome::Failed(message) if message.contains("Cargo.toml")
        ));
    }

    #[test]
    fn summaries_use_italian_plurals() {
        assert_eq!(summary(0, 1), "0 errori, 1 avviso");
        assert_eq!(summary(2, 3), "2 errori, 3 avvisi");
        assert_eq!(summary(1, 0), "1 errore, 0 avvisi");
    }

    fn cargo_project(dir: &Path, main: &str) -> JobRequest {
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"hello\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/main.rs"), main).unwrap();
        JobRequest {
            kind: JobKind::Build,
            project: ProjectKind::detect(dir, false),
            root: dir.to_path_buf(),
            slint_files: Vec::new(),
            run_target: None,
            style: "fluent".into(),
        }
    }

    /// Builds and runs real cargo projects (needs a Rust toolchain; slow).
    #[test]
    fn cargo_build_and_run() {
        if find_cargo().is_none() {
            eprintln!("cargo non disponibile: test saltato");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut request = cargo_project(
            &root,
            "fn main() {\n    let unused = 1;\n    println!(\"{{\\\"reason\\\":\\\"x\\\"}}\");\n    println!(\"ciao\");\n    eprintln!(\"error: dal programma\");\n    std::process::exit(4);\n}\n",
        );

        let job = Job::start(request.clone(), no_waker()).unwrap();
        let mut events = Vec::new();
        assert_eq!(
            wait_for_outcome(&job, &mut events),
            JobOutcome::Built { warnings: 1 }
        );
        let warning = diagnostics(&events)[0];
        assert_eq!(warning.severity, Severity::Warning);
        let location = warning.location.as_ref().unwrap();
        assert_eq!(location.path, root.join("src/main.rs"));
        assert_eq!(location.start.line, 1);
        assert!(output(&events).contains("Compiling hello v0.1.0"));

        request.kind = JobKind::Run;
        let job = Job::start(request.clone(), no_waker()).unwrap();
        let mut events = Vec::new();
        assert_eq!(
            wait_for_outcome(&job, &mut events),
            JobOutcome::Exited {
                success: false,
                description: "codice 4".into()
            }
        );
        let program_stdout: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                JobEvent::Output {
                    stream: TerminalStream::Stdout,
                    text,
                } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(program_stdout, [r#"{"reason":"x"}"#, "ciao"]);
        assert!(
            diagnostics(&events)
                .iter()
                .all(|d| !d.message.contains("dal programma")),
            "program stderr is not a diagnostic"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, JobEvent::Phase(JobPhase::Running { .. })))
        );

        // A compile error.
        fs::write(root.join("src/main.rs"), "fn main() { nope }\n").unwrap();
        let job = Job::start(request.clone(), no_waker()).unwrap();
        let mut events = Vec::new();
        assert_eq!(
            wait_for_outcome(&job, &mut events),
            JobOutcome::BuildFailed { errors: 1 }
        );
        let error = diagnostics(&events)[0];
        assert!(error.message.contains("nope"), "{}", error.message);
        assert!(error.message.ends_with("[E0425]"), "{}", error.message);

        // Stopping a running program.
        fs::write(
            root.join("src/main.rs"),
            "fn main() { println!(\"via\"); loop { std::thread::sleep(std::time::Duration::from_millis(10)); } }\n",
        )
        .unwrap();
        let job = Job::start(request, no_waker()).unwrap();
        let mut events = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(300);
        while !events.iter().any(|e| {
            matches!(e, JobEvent::Output { stream: TerminalStream::Stdout, text } if text == "via")
        }) {
            assert!(Instant::now() < deadline);
            events.extend(job.poll());
            thread::sleep(Duration::from_millis(10));
        }
        job.stop();
        assert!(job.is_stopping());
        assert_eq!(wait_for_outcome(&job, &mut events), JobOutcome::Stopped);
    }
}
