//! A minimal LSP client that runs the language server as a child process.
//!
//! Three background threads do all I/O — writing to the server's stdin,
//! reading its stdout, and forwarding its stderr to the log — so the UI thread
//! never blocks. Results arrive as [`LspEvent`]s on a channel; a [`Waker`] is
//! called after every event so the UI can repaint.

use std::collections::{HashMap, VecDeque};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use lsp_types::{
    CompletionItem, CompletionResponse, GotoDefinitionResponse, InitializeResult, LogMessageParams,
    MessageType, PositionEncodingKind, PublishDiagnosticsParams,
};
use serde_json::{Value, json};
use slint_studio_core::{LineCol, PositionEncoding};
use tracing::{debug, trace, warn};

use crate::protocol::{self, Incoming, RequestId};
use crate::transport;
use crate::uri::{path_from_uri, uri_from_path};

/// Called after each event is queued; typically requests a UI repaint.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// How long to wait for the server to exit after `shutdown`/`exit`.
const EXIT_GRACE: Duration = Duration::from_secs(2);
/// Number of stderr lines kept to explain an unexpected exit.
const STDERR_TAIL: usize = 20;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub binary: PathBuf,
    pub args: Vec<String>,
    /// Project root: working directory and workspace folder of the server.
    pub root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageLevel {
    Error,
    Warning,
    Info,
    Log,
}

/// Where a go-to-definition request points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionTarget {
    pub uri: String,
    /// `None` for non-file URIs such as Slint's `builtin:` widgets.
    pub path: Option<PathBuf>,
    /// Start of the target, in the server's position encoding.
    pub position: LineCol,
}

#[derive(Debug)]
pub enum LspEvent {
    /// The server finished initialising; documents can now be opened.
    Ready {
        server: String,
        encoding: PositionEncoding,
    },
    /// Ranges are in the server's position encoding.
    Diagnostics {
        path: PathBuf,
        version: Option<i32>,
        diagnostics: Vec<lsp_types::Diagnostic>,
    },
    Completion {
        request: RequestId,
        items: Vec<CompletionItem>,
    },
    Definition {
        request: RequestId,
        targets: Vec<DefinitionTarget>,
    },
    RequestFailed {
        request: RequestId,
        message: String,
    },
    Message {
        level: MessageLevel,
        text: String,
    },
    /// The server process ended (or its output closed).
    Exited {
        code: Option<i32>,
        stderr_tail: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Initialize,
    Completion,
    Definition,
    Shutdown,
}

struct Shared {
    root: PathBuf,
    pending: Mutex<HashMap<RequestId, Pending>>,
    ready: AtomicBool,
    encoding: Mutex<PositionEncoding>,
    stderr_tail: Mutex<VecDeque<String>>,
}

impl Shared {
    fn take_pending(&self, id: RequestId) -> Option<Pending> {
        lock(&self.pending).remove(&id)
    }
}

struct Emitter {
    events: Sender<LspEvent>,
    waker: Waker,
}

impl Emitter {
    fn emit(&self, event: LspEvent) {
        if self.events.send(event).is_ok() {
            (self.waker)();
        }
    }
}

/// Handle to a running language server. Dropping it shuts the server down.
pub struct LspClient {
    outgoing: Sender<String>,
    shared: Arc<Shared>,
    child: Arc<Mutex<Child>>,
    next_id: AtomicI64,
    shut_down: AtomicBool,
}

impl LspClient {
    /// Spawns the server and starts the `initialize` handshake.
    pub fn start(config: &ServerConfig, waker: Waker) -> io::Result<(Self, Receiver<LspEvent>)> {
        let mut command = Command::new(&config.binary);
        command
            .args(&config.args)
            .current_dir(&config.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("stdin non disponibile"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("stdout non disponibile"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("stderr non disponibile"))?;
        let child = Arc::new(Mutex::new(child));

        let shared = Arc::new(Shared {
            root: config.root.clone(),
            pending: Mutex::new(HashMap::new()),
            ready: AtomicBool::new(false),
            encoding: Mutex::new(PositionEncoding::Utf16),
            stderr_tail: Mutex::new(VecDeque::new()),
        });
        let (outgoing, outgoing_rx) = mpsc::channel::<String>();
        let (events, events_rx) = mpsc::channel();
        let emitter = Emitter { events, waker };

        thread::Builder::new()
            .name("lsp-writer".into())
            .spawn(move || writer_loop(stdin, &outgoing_rx))?;
        {
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name("lsp-stderr".into())
                .spawn(move || stderr_loop(stderr, &shared))?;
        }
        {
            let shared = Arc::clone(&shared);
            let outgoing = outgoing.clone();
            let child = Arc::clone(&child);
            thread::Builder::new()
                .name("lsp-reader".into())
                .spawn(move || reader_loop(stdout, &shared, &outgoing, &emitter, &child))?;
        }

        let client = Self {
            outgoing,
            shared,
            child,
            next_id: AtomicI64::new(1),
            shut_down: AtomicBool::new(false),
        };
        client.send_request(
            Pending::Initialize,
            "initialize",
            initialize_params(&config.root),
        );
        Ok((client, events_rx))
    }

    /// Whether the handshake completed and the server accepts documents.
    pub fn is_ready(&self) -> bool {
        self.shared.ready.load(Ordering::SeqCst)
    }

    /// Encoding of the columns in positions exchanged with the server.
    pub fn encoding(&self) -> PositionEncoding {
        *lock(&self.shared.encoding)
    }

    pub fn did_open(&self, path: &Path, text: &str, version: i32) {
        self.notify_document(
            path,
            "textDocument/didOpen",
            |uri| json!({ "textDocument": { "uri": uri, "languageId": "slint", "version": version, "text": text } }),
        );
    }

    /// Sends the whole new content (the server uses full synchronisation).
    pub fn did_change(&self, path: &Path, text: &str, version: i32) {
        self.notify_document(
            path,
            "textDocument/didChange",
            |uri| json!({ "textDocument": { "uri": uri, "version": version }, "contentChanges": [{ "text": text }] }),
        );
    }

    pub fn did_save(&self, path: &Path) {
        self.notify_document(
            path,
            "textDocument/didSave",
            |uri| json!({ "textDocument": { "uri": uri } }),
        );
    }

    pub fn did_close(&self, path: &Path) {
        self.notify_document(
            path,
            "textDocument/didClose",
            |uri| json!({ "textDocument": { "uri": uri } }),
        );
    }

    /// Requests completions at `position` (in the server's encoding).
    pub fn completion(&self, path: &Path, position: LineCol) -> Option<RequestId> {
        self.position_request(
            Pending::Completion,
            "textDocument/completion",
            path,
            position,
        )
    }

    /// Requests the definition of the symbol at `position`.
    pub fn definition(&self, path: &Path, position: LineCol) -> Option<RequestId> {
        self.position_request(
            Pending::Definition,
            "textDocument/definition",
            path,
            position,
        )
    }

    /// Asks the server to exit, killing it if it does not within a grace period.
    pub fn shutdown(&self) {
        if self.shut_down.swap(true, Ordering::SeqCst) {
            return;
        }
        if self.is_ready() {
            self.send_request(Pending::Shutdown, "shutdown", Value::Null);
            self.send(protocol::notification("exit", Value::Null));
        }
        self.shared.ready.store(false, Ordering::SeqCst);
        let child = Arc::clone(&self.child);
        let spawned = thread::Builder::new()
            .name("lsp-shutdown".into())
            .spawn(move || {
                wait_or_kill(&child, EXIT_GRACE);
            });
        if spawned.is_err() {
            let _ = lock(&self.child).kill();
        }
    }

    fn position_request(
        &self,
        kind: Pending,
        method: &str,
        path: &Path,
        position: LineCol,
    ) -> Option<RequestId> {
        if !self.is_ready() {
            return None;
        }
        let uri = uri_from_path(path)?;
        let params = json!({
            "textDocument": { "uri": uri.as_str() },
            "position": { "line": position.line, "character": position.column },
        });
        Some(self.send_request(kind, method, params))
    }

    fn notify_document(&self, path: &Path, method: &str, params: impl FnOnce(&str) -> Value) {
        if !self.is_ready() {
            trace!(method, "server non pronto: notifica rimandata");
            return;
        }
        let Some(uri) = uri_from_path(path) else {
            warn!("percorso non assoluto, ignorato: {}", path.display());
            return;
        };
        self.send(protocol::notification(method, params(uri.as_str())));
    }

    fn send_request(&self, kind: Pending, method: &str, params: Value) -> RequestId {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        lock(&self.shared.pending).insert(id, kind);
        self.send(protocol::request(id, method, params));
        id
    }

    fn send(&self, message: String) {
        // Fails only once the writer thread has stopped (server gone); the
        // reader thread reports that as `Exited`.
        let _ = self.outgoing.send(message);
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn initialize_params(root: &Path) -> Value {
    let root_uri = uri_from_path(root).map(|uri| uri.as_str().to_owned());
    let name = root.file_name().map_or_else(
        || root.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    json!({
        "processId": std::process::id(),
        "clientInfo": { "name": "Slint Studio", "version": env!("CARGO_PKG_VERSION") },
        "rootUri": root_uri,
        "workspaceFolders": root_uri.as_ref().map(|uri| json!([{ "uri": uri, "name": name }])),
        "capabilities": {
            "general": { "positionEncodings": ["utf-8", "utf-16"] },
            "textDocument": {
                "synchronization": { "didSave": true },
                "completion": {
                    "completionItem": { "snippetSupport": false, "documentationFormat": ["plaintext"] }
                },
                "definition": { "linkSupport": true },
                "publishDiagnostics": { "versionSupport": true }
            },
            "workspace": { "configuration": true, "workspaceFolders": true }
        }
    })
}

fn writer_loop(mut stdin: ChildStdin, messages: &Receiver<String>) {
    for message in messages {
        if let Err(err) = transport::write_message(&mut stdin, &message) {
            debug!("scrittura verso il language server interrotta: {err}");
            return;
        }
    }
}

fn stderr_loop(stderr: ChildStderr, shared: &Shared) {
    for line in BufReader::new(stderr).lines() {
        let Ok(line) = line else { return };
        debug!(target: "slint_lsp", "{line}");
        let mut tail = lock(&shared.stderr_tail);
        if tail.len() == STDERR_TAIL {
            tail.pop_front();
        }
        tail.push_back(line);
    }
}

fn reader_loop(
    stdout: ChildStdout,
    shared: &Shared,
    outgoing: &Sender<String>,
    emitter: &Emitter,
    child: &Mutex<Child>,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        match transport::read_message(&mut reader) {
            Ok(Some(text)) => match protocol::parse(&text) {
                Ok(message) => handle(message, shared, outgoing, emitter),
                Err(err) => warn!("messaggio non valido dal language server: {err}"),
            },
            Ok(None) => break,
            Err(err) => {
                warn!("lettura dal language server interrotta: {err}");
                break;
            }
        }
    }
    shared.ready.store(false, Ordering::SeqCst);
    let code = wait_or_kill(child, EXIT_GRACE);
    // Let the stderr thread drain the last lines, which usually explain why.
    thread::sleep(Duration::from_millis(100));
    let stderr_tail = lock(&shared.stderr_tail).iter().cloned().collect();
    emitter.emit(LspEvent::Exited { code, stderr_tail });
}

/// Waits for the process to exit, killing it after `grace`. Returns the exit code.
fn wait_or_kill(child: &Mutex<Child>, grace: Duration) -> Option<i32> {
    let deadline = Instant::now() + grace;
    loop {
        match lock(child).try_wait() {
            Ok(Some(status)) => return status.code(),
            Ok(None) if Instant::now() < deadline => {}
            Ok(None) | Err(_) => {
                let mut child = lock(child);
                let _ = child.kill();
                return child.wait().ok().and_then(|s| s.code());
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn handle(message: Incoming, shared: &Shared, outgoing: &Sender<String>, emitter: &Emitter) {
    match message {
        Incoming::Response {
            id: Some(id),
            result,
        } => {
            // Unknown ids (e.g. duplicated responses) are ignored.
            if let Some(kind) = shared.take_pending(id) {
                handle_response(id, kind, result, shared, outgoing, emitter);
            }
        }
        Incoming::Response { id: None, .. } => {}
        Incoming::Request { id, method, params } => {
            if method == "window/showMessageRequest"
                && let Some(text) = params.get("message").and_then(Value::as_str)
            {
                emitter.emit(LspEvent::Message {
                    level: MessageLevel::Info,
                    text: text.to_owned(),
                });
            }
            let _ = outgoing.send(server_request_reply(&id, &method, &params, &shared.root));
        }
        Incoming::Notification { method, params } => handle_notification(&method, params, emitter),
    }
}

fn handle_response(
    id: RequestId,
    kind: Pending,
    result: Result<Value, protocol::ResponseError>,
    shared: &Shared,
    outgoing: &Sender<String>,
    emitter: &Emitter,
) {
    let value = match result {
        Ok(value) => value,
        Err(err) => {
            let text = format!("richiesta {kind:?} fallita: {} ({})", err.message, err.code);
            if kind == Pending::Initialize {
                emitter.emit(LspEvent::Message {
                    level: MessageLevel::Error,
                    text,
                });
            } else if kind != Pending::Shutdown {
                emitter.emit(LspEvent::RequestFailed {
                    request: id,
                    message: text,
                });
            }
            return;
        }
    };

    match kind {
        Pending::Initialize => match serde_json::from_value::<InitializeResult>(value) {
            Ok(result) => {
                let encoding = encoding_from(result.capabilities.position_encoding.as_ref());
                *lock(&shared.encoding) = encoding;
                let _ = outgoing.send(protocol::notification("initialized", json!({})));
                shared.ready.store(true, Ordering::SeqCst);
                let server = result.server_info.map_or_else(
                    || "language server".to_owned(),
                    |info| match info.version {
                        Some(version) => format!("{} {version}", info.name),
                        None => info.name,
                    },
                );
                emitter.emit(LspEvent::Ready { server, encoding });
            }
            Err(err) => emitter.emit(LspEvent::Message {
                level: MessageLevel::Error,
                text: format!("risposta di inizializzazione non valida: {err}"),
            }),
        },
        Pending::Completion => {
            let items = match serde_json::from_value::<Option<CompletionResponse>>(value) {
                Ok(Some(CompletionResponse::Array(items))) => items,
                Ok(Some(CompletionResponse::List(list))) => list.items,
                Ok(None) => Vec::new(),
                Err(err) => {
                    return emitter.emit(LspEvent::RequestFailed {
                        request: id,
                        message: format!("completamento non valido: {err}"),
                    });
                }
            };
            emitter.emit(LspEvent::Completion { request: id, items });
        }
        Pending::Definition => {
            match serde_json::from_value::<Option<GotoDefinitionResponse>>(value) {
                Ok(response) => emitter.emit(LspEvent::Definition {
                    request: id,
                    targets: definition_targets(response),
                }),
                Err(err) => emitter.emit(LspEvent::RequestFailed {
                    request: id,
                    message: format!("definizione non valida: {err}"),
                }),
            }
        }
        Pending::Shutdown => {}
    }
}

fn handle_notification(method: &str, params: Value, emitter: &Emitter) {
    match method {
        "textDocument/publishDiagnostics" => {
            match serde_json::from_value::<PublishDiagnosticsParams>(params) {
                Ok(params) => match path_from_uri(&params.uri) {
                    Some(path) => emitter.emit(LspEvent::Diagnostics {
                        path,
                        version: params.version,
                        diagnostics: params.diagnostics,
                    }),
                    None => trace!(
                        "diagnostica per URI non locale ignorata: {}",
                        params.uri.as_str()
                    ),
                },
                Err(err) => warn!("diagnostica non valida: {err}"),
            }
        }
        "window/logMessage" | "window/showMessage" => {
            if let Ok(params) = serde_json::from_value::<LogMessageParams>(params) {
                emitter.emit(LspEvent::Message {
                    level: message_level(params.typ),
                    text: params.message,
                });
            }
        }
        _ => trace!(method, "notifica del language server ignorata"),
    }
}

/// Answers a request initiated by the server.
///
/// Every request must be answered, and with a well-formed result:
/// `slint-lsp` aborts if `workspace/configuration` returns anything other than
/// one entry per requested item.
fn server_request_reply(id: &Value, method: &str, params: &Value, root: &Path) -> String {
    match method {
        "workspace/configuration" => {
            let items = params
                .get("items")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            protocol::response(id, Value::Array(vec![Value::Null; items]))
        }
        "workspace/workspaceFolders" => {
            let folders = uri_from_path(root).map_or_else(
                || json!([]),
                |uri| json!([{ "uri": uri.as_str(), "name": root.file_name().map(|n| n.to_string_lossy()) }]),
            );
            protocol::response(id, folders)
        }
        "workspace/applyEdit" => protocol::response(
            id,
            json!({ "applied": false, "failureReason": "modifiche richieste dal server non ancora supportate" }),
        ),
        "window/workDoneProgress/create"
        | "client/registerCapability"
        | "client/unregisterCapability"
        | "window/showMessageRequest" => protocol::response(id, Value::Null),
        _ => protocol::error_response(
            id,
            protocol::METHOD_NOT_FOUND,
            &format!("metodo non supportato: {method}"),
        ),
    }
}

fn encoding_from(kind: Option<&PositionEncodingKind>) -> PositionEncoding {
    match kind {
        Some(kind) if *kind == PositionEncodingKind::UTF8 => PositionEncoding::Utf8,
        Some(kind) if *kind == PositionEncodingKind::UTF32 => PositionEncoding::Chars,
        _ => PositionEncoding::Utf16,
    }
}

fn message_level(typ: MessageType) -> MessageLevel {
    match typ {
        MessageType::ERROR => MessageLevel::Error,
        MessageType::WARNING => MessageLevel::Warning,
        MessageType::INFO => MessageLevel::Info,
        _ => MessageLevel::Log,
    }
}

fn definition_targets(response: Option<GotoDefinitionResponse>) -> Vec<DefinitionTarget> {
    let target = |uri: lsp_types::Uri, position: lsp_types::Position| DefinitionTarget {
        uri: uri.as_str().to_owned(),
        path: path_from_uri(&uri),
        position: LineCol::new(position.line, position.character),
    };
    match response {
        None => Vec::new(),
        Some(GotoDefinitionResponse::Scalar(location)) => {
            vec![target(location.uri, location.range.start)]
        }
        Some(GotoDefinitionResponse::Array(locations)) => locations
            .into_iter()
            .map(|l| target(l.uri, l.range.start))
            .collect(),
        Some(GotoDefinitionResponse::Link(links)) => links
            .into_iter()
            .map(|l| target(l.target_uri, l.target_selection_range.start))
            .collect(),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(method: &str, params: Value) -> Value {
        let text = server_request_reply(&json!(7), method, &params, Path::new("/progetto"));
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn configuration_is_answered_with_one_entry_per_item() {
        let answer = reply(
            "workspace/configuration",
            json!({ "items": [{ "section": "slint" }, { "section": "other" }] }),
        );
        assert_eq!(answer["id"], 7);
        assert_eq!(answer["result"], json!([null, null]));

        // Never `null`: slint-lsp would abort.
        assert_eq!(
            reply("workspace/configuration", json!({}))["result"],
            json!([])
        );
    }

    #[test]
    fn other_server_requests() {
        assert_eq!(
            reply("client/registerCapability", json!({}))["result"],
            Value::Null
        );
        assert_eq!(
            reply("workspace/applyEdit", json!({}))["result"]["applied"],
            false
        );
        let unknown = reply("slint/unknown", json!({}));
        assert_eq!(unknown["error"]["code"], protocol::METHOD_NOT_FOUND);
    }

    #[cfg(unix)]
    #[test]
    fn workspace_folders_reply_contains_the_root() {
        let answer = reply("workspace/workspaceFolders", Value::Null);
        assert_eq!(answer["result"][0]["uri"], "file:///progetto");
        assert_eq!(answer["result"][0]["name"], "progetto");
    }

    #[test]
    fn encoding_negotiation() {
        assert_eq!(
            encoding_from(Some(&PositionEncodingKind::UTF8)),
            PositionEncoding::Utf8
        );
        assert_eq!(
            encoding_from(Some(&PositionEncodingKind::UTF32)),
            PositionEncoding::Chars
        );
        assert_eq!(
            encoding_from(Some(&PositionEncodingKind::UTF16)),
            PositionEncoding::Utf16
        );
        assert_eq!(encoding_from(None), PositionEncoding::Utf16);
    }

    #[cfg(unix)]
    #[test]
    fn definition_links_use_the_selection_range() {
        // As returned by slint-lsp 1.18.
        let response: Option<GotoDefinitionResponse> = serde_json::from_value(json!([{
            "targetUri": "file:///p/ui/main.slint",
            "targetRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 6, "character": 1 } },
            "targetSelectionRange": { "start": { "line": 2, "character": 10 }, "end": { "line": 2, "character": 14 } }
        }]))
        .unwrap();
        let targets = definition_targets(response);
        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].path.as_deref(),
            Some(Path::new("/p/ui/main.slint"))
        );
        assert_eq!(targets[0].position, LineCol::new(2, 10));

        let builtin: Option<GotoDefinitionResponse> = serde_json::from_value(json!({
            "uri": "builtin:/fluent/std-widgets.slint",
            "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 1 } }
        }))
        .unwrap();
        assert_eq!(definition_targets(builtin)[0].path, None);
        assert!(definition_targets(None).is_empty());
    }

    #[test]
    fn initialize_params_offer_utf8_and_disable_snippets() {
        let params = initialize_params(Path::new("/progetto"));
        assert_eq!(
            params["capabilities"]["general"]["positionEncodings"][0],
            "utf-8"
        );
        assert_eq!(
            params["capabilities"]["textDocument"]["completion"]["completionItem"]["snippetSupport"],
            false
        );
    }
}
