//! External processes with streamed output and a reliable stop.
//!
//! Output is read line by line on two threads (stdout and stderr) and handed
//! to callbacks as it arrives. On Unix the process leads its own process
//! group, so [`Stopper::stop`] also reaches what it started (rustc, the
//! program launched by `cargo run`): first `SIGTERM`, then `SIGKILL` after
//! [`STOP_GRACE`]. On Windows the whole tree is killed with `taskkill /T`.

use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use tracing::{debug, warn};

/// How long a stopped process may take to exit before it is killed.
pub const STOP_GRACE: Duration = Duration::from_secs(3);

/// How often [`Process::wait`] checks the child.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// After the process exits, how long to wait for the rest of its output
/// (a grandchild may keep the pipes open indefinitely).
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

/// A command line to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    /// Inherited variables not passed on.
    pub env_remove: Vec<OsString>,
}

impl ProcessCommand {
    pub fn new(program: impl Into<PathBuf>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: cwd.into(),
            env: Vec::new(),
            env_remove: Vec::new(),
        }
    }

    pub fn env_remove(mut self, key: impl AsRef<OsStr>) -> Self {
        self.env_remove.push(key.as_ref().to_owned());
        self
    }

    /// Runs the command to completion and collects its output (for quick
    /// queries such as `cargo locate-project`).
    pub fn output(&self) -> io::Result<std::process::Output> {
        self.to_command().output()
    }

    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.env
            .push((key.as_ref().to_owned(), value.as_ref().to_owned()));
        self
    }

    /// The command line as shown to the user: the program's file name and
    /// the arguments, quoted when they contain spaces.
    pub fn display(&self) -> String {
        let program = self
            .program
            .file_stem()
            .unwrap_or(self.program.as_os_str())
            .to_string_lossy();
        let mut line = program.into_owned();
        for arg in &self.args {
            let arg = arg.to_string_lossy();
            line.push(' ');
            if arg.is_empty() || arg.contains([' ', '"', '\'']) {
                line.push('"');
                line.push_str(&arg.replace('"', "\\\""));
                line.push('"');
            } else {
                line.push_str(&arg);
            }
        }
        line
    }

    fn to_command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for key in &self.env_remove {
            command.env_remove(key);
        }
        for (key, value) in &self.env {
            command.env(key, value);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
    }
}

/// Receives one line of output, without its terminator.
pub type LineHandler = Box<dyn FnMut(String) + Send>;

type OutputStream = Box<dyn Read + Send>;

struct Shared {
    child: Mutex<Child>,
    pid: u32,
    /// When a stop was requested (for the escalation to a kill).
    stop_requested: Mutex<Option<Instant>>,
}

impl Shared {
    fn child(&self) -> MutexGuard<'_, Child> {
        self.child.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn stop_requested(&self) -> MutexGuard<'_, Option<Instant>> {
        self.stop_requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// A running process. Dropping it kills the process if it is still alive.
pub struct Process {
    shared: Arc<Shared>,
    drained: mpsc::Receiver<()>,
    exit: Option<ExitStatus>,
}

/// A handle that stops a [`Process`] from another thread.
#[derive(Clone)]
pub struct Stopper {
    shared: Arc<Shared>,
}

impl Stopper {
    /// Asks the process (and its children) to terminate. Idempotent; after
    /// [`STOP_GRACE`], [`Process::wait`] kills what is left.
    pub fn stop(&self) {
        let mut child = self.shared.child();
        // Under the lock the child cannot be reaped, so its pid (and process
        // group) cannot have been reused yet.
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        let mut requested = self.shared.stop_requested();
        if requested.is_none() {
            *requested = Some(Instant::now());
            debug!("arresto del processo {}", self.shared.pid);
            terminate(&mut child, self.shared.pid, false);
        }
    }
}

impl Process {
    /// Starts `command`, delivering each line of stdout and stderr to the
    /// handlers (on two background threads).
    pub fn spawn(
        command: &ProcessCommand,
        on_stdout: LineHandler,
        on_stderr: LineHandler,
    ) -> io::Result<Self> {
        let mut child = command.to_command().spawn()?;
        let pid = child.id();
        let (drained_tx, drained) = mpsc::channel();
        let streams: [(Option<OutputStream>, LineHandler, &str); 2] = [
            (
                child.stdout.take().map(|s| Box::new(s) as OutputStream),
                on_stdout,
                "stdout",
            ),
            (
                child.stderr.take().map(|s| Box::new(s) as OutputStream),
                on_stderr,
                "stderr",
            ),
        ];
        for (stream, handler, name) in streams {
            let Some(stream) = stream else { continue };
            let drained_tx = drained_tx.clone();
            let spawned = thread::Builder::new()
                .name(format!("process-{pid}-{name}"))
                .spawn(move || {
                    read_lines(stream, handler);
                    drop(drained_tx);
                });
            if let Err(err) = spawned {
                let _ = child.kill();
                let _ = child.wait();
                return Err(err);
            }
        }
        Ok(Self {
            shared: Arc::new(Shared {
                child: Mutex::new(child),
                pid,
                stop_requested: Mutex::new(None),
            }),
            drained,
            exit: None,
        })
    }

    pub fn pid(&self) -> u32 {
        self.shared.pid
    }

    pub fn stopper(&self) -> Stopper {
        Stopper {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Whether [`Stopper::stop`] was called.
    pub fn was_stopped(&self) -> bool {
        self.shared.stop_requested().is_some()
    }

    /// Blocks until the process exits and its output has been delivered.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.exit {
            return Ok(status);
        }
        let status = loop {
            {
                let mut child = self.shared.child();
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                let requested = *self.shared.stop_requested();
                if requested.is_some_and(|at| at.elapsed() >= STOP_GRACE) {
                    warn!(
                        "il processo {} non si è chiuso: terminazione forzata",
                        self.shared.pid
                    );
                    terminate(&mut child, self.shared.pid, true);
                }
            }
            thread::sleep(POLL_INTERVAL);
        };
        self.exit = Some(status);
        // Both reader threads drop their sender at end of stream.
        match self.drained.recv_timeout(DRAIN_TIMEOUT) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {}
            Err(RecvTimeoutError::Timeout) => {
                debug!("output ancora aperto dopo l'uscita del processo")
            }
        }
        Ok(status)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if self.exit.is_some() {
            return;
        }
        let mut child = self.shared.child();
        if matches!(child.try_wait(), Ok(None)) {
            terminate(&mut child, self.shared.pid, true);
            let _ = child.wait();
        }
    }
}

fn read_lines(stream: OutputStream, mut handler: LineHandler) {
    let mut reader = BufReader::new(stream);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer) {
            Ok(0) => break,
            Ok(_) => {
                if buffer.last() == Some(&b'\n') {
                    buffer.pop();
                }
                if buffer.last() == Some(&b'\r') {
                    buffer.pop();
                }
                handler(String::from_utf8_lossy(&buffer).into_owned());
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => {
                debug!("lettura dell'output interrotta: {err}");
                break;
            }
        }
    }
}

#[cfg(unix)]
fn terminate(child: &mut Child, pid: u32, force: bool) {
    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid;

    let signal = if force {
        Signal::SIGKILL
    } else {
        Signal::SIGTERM
    };
    let group = i32::try_from(pid).map(Pid::from_raw);
    let sent = group.map(|group| killpg(group, signal));
    if !matches!(sent, Ok(Ok(()))) {
        // Not a group leader after all (or already gone): the child only.
        let _ = child.kill();
    }
}

#[cfg(windows)]
fn terminate(child: &mut Child, pid: u32, _force: bool) {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let killed_tree = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .is_ok_and(|status| status.success());
    if !killed_tree {
        let _ = child.kill();
    }
}

#[cfg(not(any(unix, windows)))]
fn terminate(child: &mut Child, _pid: u32, _force: bool) {
    let _ = child.kill();
}

/// How a process ended, in Italian ("codice 3", "segnale 15 (SIGTERM)").
pub fn describe_exit(status: ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("codice {code}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        if let Some(signal) = status.signal() {
            let name = nix::sys::signal::Signal::try_from(signal)
                .map(|s| format!(" ({})", s.as_str()))
                .unwrap_or_default();
            return format!("segnale {signal}{name}");
        }
    }
    status.to_string()
}

/// First `name` (with the platform's executable suffix) in the `PATH`.
pub fn find_program(name: &str) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    slint_studio_core::install::find_in_path(&file)
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;

    fn collect() -> (Arc<Mutex<Vec<String>>>, LineHandler) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        (lines, Box::new(move |line| sink.lock().unwrap().push(line)))
    }

    fn sh(script: &str) -> ProcessCommand {
        ProcessCommand::new("/bin/sh", std::env::temp_dir())
            .arg("-c")
            .arg(script)
    }

    #[test]
    fn streams_stdout_and_stderr_and_reports_exit_code() {
        let (out, on_out) = collect();
        let (err, on_err) = collect();
        let command = sh("echo uno; echo due >&2; printf 'tre\\r\\n'; printf senza-a-capo; exit 3")
            .env("SLINT_STUDIO_TEST", "1");
        let mut process = Process::spawn(&command, on_out, on_err).unwrap();
        let status = process.wait().unwrap();
        assert_eq!(status.code(), Some(3));
        assert_eq!(describe_exit(status), "codice 3");
        assert!(!process.was_stopped());
        assert_eq!(*out.lock().unwrap(), ["uno", "tre", "senza-a-capo"]);
        assert_eq!(*err.lock().unwrap(), ["due"]);
        // Waiting again returns the cached status.
        assert_eq!(process.wait().unwrap().code(), Some(3));
    }

    #[test]
    fn invalid_utf8_is_replaced() {
        let (out, on_out) = collect();
        let (_, on_err) = collect();
        let mut process = Process::spawn(&sh("printf 'a\\377b\\n'"), on_out, on_err).unwrap();
        assert!(process.wait().unwrap().success());
        assert_eq!(*out.lock().unwrap(), ["a\u{fffd}b"]);
    }

    #[test]
    fn stop_terminates_the_whole_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("grandchild-survived");
        // The grandchild would create the marker if it outlived the stop.
        let script = format!(
            "(sleep 1; touch '{}') & echo pronto; wait",
            marker.display()
        );
        let (out, on_out) = collect();
        let (_, on_err) = collect();
        let mut process = Process::spawn(&sh(&script), on_out, on_err).unwrap();
        let started = Instant::now();
        while out.lock().unwrap().is_empty() {
            assert!(started.elapsed() < Duration::from_secs(5), "no output");
            thread::sleep(Duration::from_millis(10));
        }
        let stopper = process.stopper();
        stopper.stop();
        stopper.stop();
        let status = process.wait().unwrap();
        assert!(!status.success());
        assert!(process.was_stopped());
        assert!(describe_exit(status).contains("SIGTERM"), "{status}");
        assert!(started.elapsed() < STOP_GRACE);
        thread::sleep(Duration::from_millis(1300));
        assert!(!marker.exists(), "the grandchild was not stopped");
        // Stopping an exited process is harmless.
        stopper.stop();
    }

    #[test]
    fn processes_ignoring_sigterm_are_killed_after_the_grace_period() {
        let (out, on_out) = collect();
        let (_, on_err) = collect();
        let script = "trap '' TERM; echo pronto; while true; do sleep 0.05; done";
        let mut process = Process::spawn(&sh(script), on_out, on_err).unwrap();
        while out.lock().unwrap().is_empty() {
            thread::sleep(Duration::from_millis(10));
        }
        let started = Instant::now();
        process.stopper().stop();
        let status = process.wait().unwrap();
        assert!(started.elapsed() >= STOP_GRACE);
        assert!(describe_exit(status).contains("SIGKILL"), "{status}");
    }

    #[test]
    fn dropping_a_running_process_kills_it() {
        let (_, on_out) = collect();
        let (_, on_err) = collect();
        let process = Process::spawn(&sh("sleep 30"), on_out, on_err).unwrap();
        let pid = process.pid();
        drop(process);
        let alive = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok();
        assert!(!alive, "process {pid} still running");
    }

    #[test]
    fn missing_programs_fail_to_spawn() {
        let (_, on_out) = collect();
        let (_, on_err) = collect();
        let command = ProcessCommand::new("/nonexistent/program", std::env::temp_dir());
        assert!(Process::spawn(&command, on_out, on_err).is_err());
    }

    #[test]
    fn command_lines_are_displayed_with_quotes() {
        let command = ProcessCommand::new("/usr/bin/cargo", "/tmp")
            .arg("run")
            .arg("--message-format=json")
            .arg("my file.slint")
            .arg("");
        assert_eq!(
            command.display(),
            "cargo run --message-format=json \"my file.slint\" \"\""
        );
        assert!(find_program("sh").is_some());
        assert!(find_program("surely-not-a-program-name").is_none());
    }
}
