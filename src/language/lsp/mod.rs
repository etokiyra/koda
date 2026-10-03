//! Asynchronous Language Server Protocol client.
//!
//! When a language server is installed, Koda starts one per workspace root and
//! speaks JSON-RPC over its stdio. This is deliberately minimal and grows
//! feature by feature: today it runs the lifecycle and turns
//! `textDocument/publishDiagnostics` into Koda diagnostics. Completion, hover,
//! navigation, rename and code actions build on the same connection later.
//!
//! Nothing here is required for basic editing. If no server is installed Koda
//! keeps using its built-in heuristic providers, so an offline machine still
//! has syntax highlighting, structural diagnostics, symbols and formatting.

pub mod convert;
pub mod jsonrpc;

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::language::diagnostics::{Diagnostic, Severity, TextPos};
use crate::language::id::LanguageId;

use jsonrpc::Message;

/// How long the server has to answer a request before Koda gives up and clears
/// the pending state. Formatting can legitimately take longer than the others.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const FORMAT_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// How a server counts the `character` offset in a `{line, character}` position.
///
/// LSP defaults to UTF-16 code units; Koda works in Unicode scalar values. Koda
/// asks for UTF-8 (which matches its own counting) and only the servers that do
/// not offer it fall back to UTF-16.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PositionEncoding {
    Utf8,
    #[default]
    Utf16,
    Utf32,
}

/// The server features Koda knows how to use, from its `initialize` result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ServerCapabilities {
    completion: bool,
    hover: bool,
    definition: bool,
    references: bool,
    rename: bool,
    code_action: bool,
    workspace_symbol: bool,
    signature_help: bool,
    document_formatting: bool,
}

/// The LSP `languageId` string for a language.
pub fn lsp_language_id(language: LanguageId) -> &'static str {
    match language {
        LanguageId::Rust => "rust",
        LanguageId::Go => "go",
        LanguageId::Python => "python",
        LanguageId::Shell => "shellscript",
        LanguageId::Markdown => "markdown",
        LanguageId::Json => "json",
        LanguageId::Toml => "toml",
        LanguageId::Yaml => "yaml",
        LanguageId::TypeScript => "typescript",
        LanguageId::JavaScript => "javascript",
        LanguageId::C => "c",
        LanguageId::Cpp => "cpp",
        LanguageId::Java => "java",
        LanguageId::CSharp => "csharp",
        LanguageId::Html => "html",
        LanguageId::Css => "css",
        _ => "plaintext",
    }
}

/// The `languageId` for a specific file, refining TSX/JSX to their React ids.
fn language_id_for(language: LanguageId, path: &Path) -> &'static str {
    match (
        language,
        path.extension().and_then(|extension| extension.to_str()),
    ) {
        (LanguageId::TypeScript, Some("tsx")) => "typescriptreact",
        (LanguageId::JavaScript, Some("jsx")) => "javascriptreact",
        _ => lsp_language_id(language),
    }
}

/// The Koda feature a server request belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    Completion,
    Hover,
    Definition,
    References,
    Rename,
    CodeActions,
    WorkspaceSymbols,
    SignatureHelp,
    Formatting,
}

/// Something the app consumes from a running server.
#[derive(Clone, Debug)]
pub enum ServerEvent {
    /// The `initialize` handshake completed; the server accepts document
    /// notifications.
    Ready,
    /// Fresh diagnostics for a document. `version` is the document version the
    /// server computed them against, when it reported one.
    Diagnostics {
        path: PathBuf,
        version: Option<i64>,
        diagnostics: Vec<Diagnostic>,
    },
    /// The answer to a [`RequestKind`] request. The `id` is the JSON-RPC id the
    /// request was sent with, so callers can ignore a response that a newer
    /// request has superseded (for example while typing).
    Response {
        kind: RequestKind,
        id: i64,
        result: Result<Value, String>,
    },
    /// The server asked Koda to apply a workspace edit. The app must apply it
    /// and then call [`Server::apply_edit_response`].
    ApplyEdit { id: Value, params: Value },
    /// The server could not start or exited unexpectedly.
    Failed(String),
}

/// A request awaiting a response, with the deadline that bounds it.
struct PendingRequest {
    kind: RequestKind,
    deadline: Instant,
}

/// A language server process and its message loop.
pub struct Server {
    child: Child,
    stdin: ChildStdin,
    events: Receiver<Message>,
    next_id: i64,
    init_id: Option<i64>,
    ready: bool,
    root: PathBuf,
    versions: HashMap<PathBuf, i64>,
    pending: HashMap<i64, PendingRequest>,
    language: LanguageId,
    /// The encoding the server uses for `character` offsets.
    encoding: PositionEncoding,
    /// The features the server advertised.
    capabilities: ServerCapabilities,
    /// The last few stderr lines, for a more useful failure message.
    stderr: Arc<Mutex<String>>,
}

impl Server {
    /// Spawn a server and begin the `initialize` handshake.
    pub fn start(
        language: LanguageId,
        program: &str,
        args: &[&str],
        root: &Path,
    ) -> io::Result<Self> {
        Self::start_with_env(language, program, args, root, &[])
    }

    /// Spawn a server with extra environment variables, used to point managed
    /// servers at Koda-provisioned runtimes (the .NET SDK, a managed JDK).
    pub fn start_with_env(
        language: LanguageId,
        program: &str,
        args: &[&str],
        root: &Path,
        env: &[(String, String)],
    ) -> io::Result<Self> {
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env {
            command.env(key, value);
        }
        let mut child = command.spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("no stdout"))?;
        let stderr_pipe = child.stderr.take();

        let (tx, rx) = mpsc::channel::<Message>();
        thread::Builder::new()
            .name("koda-lsp-reader".to_string())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    match jsonrpc::read_message(&mut reader) {
                        Ok(Some(value)) => {
                            let Some(message) = jsonrpc::decode(value) else {
                                continue;
                            };
                            if tx.send(message).is_err() {
                                break;
                            }
                        }
                        // A message whose *body* was not valid JSON has already
                        // been consumed, so we are still framed correctly and can
                        // keep serving. Framing/I/O errors are fatal.
                        Err(err) if err.kind() == io::ErrorKind::Other => continue,
                        Ok(None) | Err(_) => break,
                    }
                }
            })?;

        // Keep the last few stderr lines so a crash can explain itself, without
        // letting server noise reach the terminal.
        let stderr = Arc::new(Mutex::new(String::new()));
        if let Some(pipe) = stderr_pipe {
            let buffer = Arc::clone(&stderr);
            let _ = thread::Builder::new()
                .name("koda-lsp-stderr".to_string())
                .spawn(move || {
                    let reader = BufReader::new(pipe);
                    for line in reader.lines().map_while(Result::ok) {
                        let mut buffer = match buffer.lock() {
                            Ok(buffer) => buffer,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        if !buffer.is_empty() {
                            buffer.push('\n');
                        }
                        buffer.push_str(&line);
                        // Bound what we keep: only the tail matters.
                        if buffer.len() > 2048 {
                            let drain = buffer.len() - 2048;
                            buffer.drain(..drain);
                        }
                    }
                });
        }

        let mut server = Server {
            child,
            stdin,
            events: rx,
            next_id: 0,
            init_id: None,
            ready: false,
            root: root.to_path_buf(),
            versions: HashMap::new(),
            pending: HashMap::new(),
            language,
            encoding: PositionEncoding::default(),
            capabilities: ServerCapabilities::default(),
            stderr,
        };

        let id = server.request(
            "initialize",
            json!({
                "processId": null,
                "clientInfo": { "name": "koda" },
                "rootUri": path_to_uri(root),
                "rootPath": root.to_string_lossy(),
                "capabilities": {
                    "general": { "positionEncodings": ["utf-8", "utf-16"] },
                    "textDocument": {
                        "synchronization": { "dynamicRegistration": false, "didSave": false },
                        "publishDiagnostics": { "relatedInformation": false },
                        "completion": { "completionItem": { "snippetSupport": false } },
                        "hover": { "contentFormat": ["markdown", "plaintext"] },
                        "definition": {},
                        "references": {},
                        "rename": { "prepareSupport": false },
                        "codeAction": {},
                        "signatureHelp": {
                            "signatureInformation": {
                                "documentationFormat": ["markdown", "plaintext"],
                                "parameterInformation": { "labelOffsetSupport": true }
                            },
                            "contextSupport": true
                        },
                        "formatting": { "dynamicRegistration": false }
                    },
                    "workspace": { "configuration": true, "symbol": {} }
                },
                "workspaceFolders": null
            }),
        )?;
        server.init_id = Some(id);
        Ok(server)
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// The encoding the server uses for character offsets.
    pub fn position_encoding(&self) -> PositionEncoding {
        self.encoding
    }

    /// Whether the server advertised support for `kind`.
    pub fn supports(&self, kind: RequestKind) -> bool {
        match kind {
            RequestKind::Completion => self.capabilities.completion,
            RequestKind::Hover => self.capabilities.hover,
            RequestKind::Definition => self.capabilities.definition,
            RequestKind::References => self.capabilities.references,
            RequestKind::Rename => self.capabilities.rename,
            RequestKind::CodeActions => self.capabilities.code_action,
            RequestKind::WorkspaceSymbols => self.capabilities.workspace_symbol,
            RequestKind::SignatureHelp => self.capabilities.signature_help,
            RequestKind::Formatting => self.capabilities.document_formatting,
        }
    }

    /// Read the negotiated position encoding and the advertised capabilities.
    fn absorb_initialize(&mut self, result: &Value) {
        let (encoding, capabilities) = parse_initialize(result);
        self.encoding = encoding;
        self.capabilities = capabilities;
    }

    /// Append the server's recent stderr, when there is any, to a failure
    /// message so a crash can explain itself.
    fn explain(&self, message: String) -> String {
        let stderr = match self.stderr.lock() {
            Ok(buffer) => buffer,
            Err(poisoned) => poisoned.into_inner(),
        };
        let detail = stderr.trim();
        if detail.is_empty() {
            message
        } else {
            format!("{message}: {detail}")
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Notify the server that a document was opened.
    pub fn did_open(&mut self, path: &Path, text: &str) {
        let version = self.next_version(path);
        let _ = self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": path_to_uri(path),
                    "languageId": language_id_for(self.language, path),
                    "version": version,
                    "text": text
                }
            }),
        );
    }

    /// Notify the server that a document changed (full-text sync).
    pub fn did_change(&mut self, path: &Path, text: &str) {
        if !self.versions.contains_key(path) {
            return self.did_open(path, text);
        }
        let version = self.next_version(path);
        let _ = self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": path_to_uri(path), "version": version },
                "contentChanges": [ { "text": text } ]
            }),
        );
    }

    /// Notify the server that a document was closed.
    pub fn did_close(&mut self, path: &Path) {
        if self.versions.remove(path).is_none() {
            return;
        }
        let _ = self.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": path_to_uri(path) } }),
        );
    }

    /// Whether the server has this document open.
    pub fn has_open_document(&self, path: &Path) -> bool {
        self.versions.contains_key(path)
    }

    /// Ask the server to format a document, returning the request id so a
    /// caller can discard a superseded response.
    pub fn formatting(&mut self, path: &Path, tab_size: usize, insert_spaces: bool) -> Option<i64> {
        let params = json!({
            "textDocument": { "uri": path_to_uri(path) },
            "options": { "tabSize": tab_size, "insertSpaces": insert_spaces }
        });
        self.send_request(RequestKind::Formatting, "textDocument/formatting", params)
            .ok()
    }

    /// Ask the server for signature help, returning the request id so a caller
    /// can discard a superseded response.
    pub fn signature_help(&mut self, path: &Path, line: usize, col: usize) -> Option<i64> {
        let mut params = position_params(path, line, col);
        if let Some(object) = params.as_object_mut() {
            // 1 = Invoked; 2 = TriggerCharacter; 3 = ContentChange.
            object.insert("context".to_string(), json!({ "triggerKind": 1 }));
        }
        self.send_request(
            RequestKind::SignatureHelp,
            "textDocument/signatureHelp",
            params,
        )
        .ok()
    }

    /// Ask the server to complete at a position, returning the request id so a
    /// caller can discard the response if it becomes stale.
    pub fn completion(&mut self, path: &Path, line: usize, col: usize) -> Option<i64> {
        self.send_request(
            RequestKind::Completion,
            "textDocument/completion",
            position_params(path, line, col),
        )
        .ok()
    }

    /// Ask the server for hover information at a position, returning the
    /// request id so a late answer can be discarded.
    pub fn hover(&mut self, path: &Path, line: usize, col: usize) -> Option<i64> {
        self.send_request(
            RequestKind::Hover,
            "textDocument/hover",
            position_params(path, line, col),
        )
        .ok()
    }

    /// Ask the server for the definition at a position, returning the request id
    /// so the caller can ignore a response the cursor has moved past.
    pub fn definition(&mut self, path: &Path, line: usize, col: usize) -> Option<i64> {
        self.send_request(
            RequestKind::Definition,
            "textDocument/definition",
            position_params(path, line, col),
        )
        .ok()
    }

    /// Ask the server for every reference to the symbol at a position.
    pub fn references(&mut self, path: &Path, line: usize, col: usize) -> Option<i64> {
        let mut params = position_params(path, line, col);
        if let Some(object) = params.as_object_mut() {
            object.insert("context".to_string(), json!({ "includeDeclaration": true }));
        }
        self.send_request(RequestKind::References, "textDocument/references", params)
            .ok()
    }

    /// Ask the server to rename the symbol at a position.
    pub fn rename(&mut self, path: &Path, line: usize, col: usize, new_name: &str) -> Option<i64> {
        let mut params = position_params(path, line, col);
        if let Some(object) = params.as_object_mut() {
            object.insert("newName".to_string(), json!(new_name));
        }
        self.send_request(RequestKind::Rename, "textDocument/rename", params)
            .ok()
    }

    /// Ask the server for code actions over a range.
    pub fn code_action(
        &mut self,
        path: &Path,
        start: (usize, usize),
        end: (usize, usize),
    ) -> Option<i64> {
        let params = json!({
            "textDocument": { "uri": path_to_uri(path) },
            "range": {
                "start": { "line": start.0, "character": start.1 },
                "end": { "line": end.0, "character": end.1 }
            },
            "context": { "diagnostics": [] }
        });
        self.send_request(RequestKind::CodeActions, "textDocument/codeAction", params)
            .ok()
    }

    /// Ask the server for workspace-wide symbols matching `query`.
    ///
    /// An empty query asks for everything the server is willing to return,
    /// which the picker then filters locally.
    pub fn workspace_symbols(&mut self, query: &str) {
        let _ = self.send_request(
            RequestKind::WorkspaceSymbols,
            "workspace/symbol",
            json!({ "query": query }),
        );
    }

    /// Ask the server to execute one of its commands. The response is ignored;
    /// the server may follow up with a `workspace/applyEdit` request.
    pub fn execute_command(&mut self, command: &str, arguments: Value) {
        let _ = self.request(
            "workspace/executeCommand",
            json!({ "command": command, "arguments": arguments }),
        );
    }

    /// Answer the server's `workspace/applyEdit` request.
    pub fn apply_edit_response(&mut self, id: &Value, applied: bool) {
        let _ = self.respond(id, json!({ "applied": applied }));
    }

    /// Drain pending events without blocking.
    pub fn poll(&mut self) -> Vec<ServerEvent> {
        let mut events = Vec::new();

        // Expire requests the server never answered, so `pending` stays bounded
        // and the UI is not left waiting forever.
        let expired = expire_requests(&mut self.pending, Instant::now());
        for (id, kind) in expired {
            events.push(ServerEvent::Response {
                kind,
                id,
                result: Err("request timed out".to_string()),
            });
        }

        loop {
            match self.events.try_recv() {
                Ok(Message::Response { id, result, error }) => {
                    let id_number = id.as_i64().unwrap_or(-1);
                    if Some(id_number) == self.init_id {
                        self.init_id = None;
                        match error {
                            Some(error) => {
                                events.push(ServerEvent::Failed(self.explain(error.message)))
                            }
                            None if result.is_some() => {
                                self.absorb_initialize(result.as_ref().unwrap_or(&Value::Null));
                                self.ready = true;
                                let _ = self.notify("initialized", json!({}));
                                events.push(ServerEvent::Ready);
                            }
                            None => {}
                        }
                    } else if let Some(request) = self.pending.remove(&id_number) {
                        let result = match error {
                            Some(error) => Err(error.message),
                            None => Ok(result.unwrap_or(Value::Null)),
                        };
                        events.push(ServerEvent::Response {
                            kind: request.kind,
                            id: id_number,
                            result,
                        });
                    }
                }
                Ok(Message::Notification { method, params }) => {
                    if method == "textDocument/publishDiagnostics"
                        && let Some(event) = parse_publish_diagnostics(&params)
                        && !self.is_stale_diagnostics(&event)
                    {
                        events.push(event);
                    }
                }
                Ok(Message::Request { id, method, params }) => {
                    if method == "workspace/applyEdit" {
                        // The app applies the edit and answers for us.
                        events.push(ServerEvent::ApplyEdit { id, params });
                    } else {
                        // Answer other server-initiated requests so it does not stall.
                        let result = self.server_request_result(&method, &params);
                        let _ = self.respond(&id, result);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    events.push(ServerEvent::Failed(
                        self.explain("language server exited".to_string()),
                    ));
                    break;
                }
            }
        }
        events
    }

    /// Whether published diagnostics are known to describe an older document
    /// version than the server has since received.
    ///
    /// A missing version is accepted for compatibility; a present one that is
    /// behind the last synchronized version is dropped so stale markers cannot
    /// appear over newer text.
    fn is_stale_diagnostics(&self, event: &ServerEvent) -> bool {
        let ServerEvent::Diagnostics {
            path,
            version: Some(version),
            ..
        } = event
        else {
            return false;
        };
        diagnostics_are_stale(Some(*version), self.versions.get(path).copied())
    }

    fn request(&mut self, method: &str, params: Value) -> io::Result<i64> {
        self.next_id += 1;
        let id = self.next_id;
        jsonrpc::write_message(
            &mut self.stdin,
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )?;
        Ok(id)
    }

    fn send_request(&mut self, kind: RequestKind, method: &str, params: Value) -> io::Result<i64> {
        let id = self.request(method, params)?;
        self.pending.insert(
            id,
            PendingRequest {
                kind,
                deadline: Instant::now() + request_timeout(kind),
            },
        );
        Ok(id)
    }

    fn notify(&mut self, method: &str, params: Value) -> io::Result<()> {
        jsonrpc::write_message(
            &mut self.stdin,
            &json!({ "jsonrpc": "2.0", "method": method, "params": params }),
        )
    }

    fn respond(&mut self, id: &Value, result: Value) -> io::Result<()> {
        jsonrpc::write_message(
            &mut self.stdin,
            &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        )
    }

    fn next_version(&mut self, path: &Path) -> i64 {
        let version = self.versions.entry(path.to_path_buf()).or_insert(0);
        *version += 1;
        *version
    }

    fn server_request_result(&self, method: &str, params: &Value) -> Value {
        match method {
            // Return one `null` configuration per requested item.
            "workspace/configuration" => {
                let count = params
                    .get("items")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0);
                Value::Array(vec![Value::Null; count])
            }
            "workspace/workspaceFolders" => {
                json!([{ "uri": path_to_uri(&self.root), "name": "workspace" }])
            }
            _ => Value::Null,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Parse the `initialize` result into the encoding and capabilities Koda uses.
fn parse_initialize(result: &Value) -> (PositionEncoding, ServerCapabilities) {
    let encoding = match result
        .pointer("/capabilities/positionEncoding")
        .and_then(Value::as_str)
    {
        Some("utf-8") => PositionEncoding::Utf8,
        Some("utf-32") => PositionEncoding::Utf32,
        _ => PositionEncoding::Utf16,
    };
    let capabilities = result.get("capabilities").unwrap_or(&Value::Null);
    let advertised = |name: &str| {
        capabilities
            .get(name)
            .is_some_and(|value| !value.is_null() && value != &Value::Bool(false))
    };
    (
        encoding,
        ServerCapabilities {
            completion: advertised("completionProvider"),
            hover: advertised("hoverProvider"),
            definition: advertised("definitionProvider"),
            references: advertised("referencesProvider"),
            rename: advertised("renameProvider"),
            code_action: advertised("codeActionProvider"),
            workspace_symbol: advertised("workspaceSymbolProvider"),
            signature_help: advertised("signatureHelpProvider"),
            document_formatting: advertised("documentFormattingProvider"),
        },
    )
}

fn parse_publish_diagnostics(params: &Value) -> Option<ServerEvent> {
    let path = uri_to_path(params.get("uri")?.as_str()?)?;
    let version = params.get("version").and_then(Value::as_i64);
    let diagnostics = params
        .get("diagnostics")?
        .as_array()?
        .iter()
        .filter_map(parse_diagnostic)
        .collect();
    Some(ServerEvent::Diagnostics {
        path,
        version,
        diagnostics,
    })
}

/// Whether diagnostics stamped with `version` are behind the version the server
/// has since received.
///
/// A missing version on either side is treated as current, matching the
/// protocol's optional field and servers that do not version diagnostics.
fn diagnostics_are_stale(version: Option<i64>, current: Option<i64>) -> bool {
    matches!((version, current), (Some(version), Some(current)) if version < current)
}

/// The deadline for a request of `kind`.
fn request_timeout(kind: RequestKind) -> Duration {
    if kind == RequestKind::Formatting {
        FORMAT_REQUEST_TIMEOUT
    } else {
        REQUEST_TIMEOUT
    }
}

/// Remove and return every request whose deadline has passed.
fn expire_requests(
    pending: &mut HashMap<i64, PendingRequest>,
    now: Instant,
) -> Vec<(i64, RequestKind)> {
    let expired: Vec<i64> = pending
        .iter()
        .filter(|(_, request)| request.deadline <= now)
        .map(|(id, _)| *id)
        .collect();
    expired
        .into_iter()
        .filter_map(|id| pending.remove(&id).map(|request| (id, request.kind)))
        .collect()
}

fn parse_diagnostic(item: &Value) -> Option<Diagnostic> {
    let range = item.get("range")?;
    let start = text_pos(range.get("start")?)?;
    let end = text_pos(range.get("end")?)?;
    let severity = match item.get("severity").and_then(Value::as_u64) {
        Some(1) => Severity::Error,
        Some(2) => Severity::Warning,
        Some(3) => Severity::Info,
        _ => Severity::Hint,
    };
    let message = item.get("message").and_then(Value::as_str).unwrap_or("");
    Some(Diagnostic::new(start, end, severity, message))
}

fn text_pos(value: &Value) -> Option<TextPos> {
    Some(TextPos::new(
        value.get("line")?.as_u64()? as usize,
        value.get("character")?.as_u64()? as usize,
    ))
}

/// The `textDocument` + `position` parameters shared by most requests.
fn position_params(path: &Path, line: usize, col: usize) -> Value {
    json!({
        "textDocument": { "uri": path_to_uri(path) },
        "position": { "line": line, "character": col }
    })
}

/// Encode a filesystem path as a `file://` URI, percent-encoding the bytes LSP
/// cares about.
pub fn path_to_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                uri.push(byte as char)
            }
            other => uri.push_str(&format!("%{other:02X}")),
        }
    }
    uri
}

/// Decode a `file://` URI back into a path.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // Drop an optional authority component (`file://localhost/...`).
    let rest = match rest.find('/') {
        Some(slash) => &rest[slash..],
        None => rest,
    };
    let bytes = rest.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(hex) = bytes.get(index + 1..index + 3)
            && let Ok(hex) = std::str::from_utf8(hex)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            decoded.push(byte);
            index += 3;
            continue;
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    Some(PathBuf::from(
        String::from_utf8_lossy(&decoded).into_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_round_trips_paths_with_spaces() {
        let path = PathBuf::from("/tmp/koda lsp/main.rs");
        let uri = path_to_uri(&path);
        assert_eq!(uri, "file:///tmp/koda%20lsp/main.rs");
        assert_eq!(uri_to_path(&uri).as_deref(), Some(path.as_path()));
    }

    #[test]
    fn reads_capabilities_and_position_encoding() {
        let (encoding, caps) = parse_initialize(&json!({
            "capabilities": {
                "positionEncoding": "utf-8",
                "completionProvider": {},
                "hoverProvider": true,
                "renameProvider": { "prepareProvider": false }
            }
        }));
        assert_eq!(encoding, PositionEncoding::Utf8);
        assert!(caps.completion);
        assert!(caps.hover);
        assert!(caps.rename);
        assert!(!caps.definition);
        assert!(!caps.workspace_symbol);

        // No declaration means the protocol default, UTF-16.
        let (encoding, caps) = parse_initialize(&json!({ "capabilities": {} }));
        assert_eq!(encoding, PositionEncoding::Utf16);
        assert!(!caps.completion);
    }

    #[test]
    fn parses_published_diagnostics() {
        let params = json!({
            "uri": "file:///tmp/main.rs",
            "diagnostics": [
                {
                    "range": { "start": { "line": 1, "character": 2 }, "end": { "line": 1, "character": 5 } },
                    "severity": 1,
                    "message": "unused variable"
                },
                {
                    "range": { "start": { "line": 3, "character": 0 }, "end": { "line": 3, "character": 1 } },
                    "severity": 2,
                    "message": "try using `mut`"
                }
            ]
        });
        let ServerEvent::Diagnostics {
            path,
            version,
            diagnostics,
        } = parse_publish_diagnostics(&params).expect("event")
        else {
            panic!("expected diagnostics");
        };
        assert_eq!(path, PathBuf::from("/tmp/main.rs"));
        assert_eq!(version, None);
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert_eq!(diagnostics[0].start, TextPos::new(1, 2));
        assert_eq!(diagnostics[1].severity, Severity::Warning);
        assert_eq!(diagnostics[1].message, "try using `mut`");
    }

    #[cfg(unix)]
    #[test]
    fn initializes_and_receives_diagnostics_from_a_mock_server() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};

        // A tiny shell server: answer `initialize` (id 1), publish one
        // diagnostic, then wait for the client to close our stdin.
        const MOCK: &str = r#"#!/bin/sh
read -r header
len=$(printf '%s' "$header" | tr -dc '0-9')
read -r blank
dd bs=1 count="$len" of=/dev/null 2>/dev/null
resp='{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}'
printf 'Content-Length: %s\r\n\r\n%s' "${#resp}" "$resp"
note='{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"file:///tmp/main.rs","diagnostics":[{"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":3}},"severity":1,"message":"boom"}]}}'
printf 'Content-Length: %s\r\n\r\n%s' "${#note}" "$note"
cat >/dev/null
"#;

        let dir = std::env::temp_dir().join(format!("koda-lsp-mock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("mock-lsp.sh");
        {
            let mut file = std::fs::File::create(&script).unwrap();
            file.write_all(MOCK.as_bytes()).unwrap();
        }
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        let mut server =
            Server::start(LanguageId::Rust, script.to_str().unwrap(), &[], &dir).unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut ready = false;
        let mut diagnostics = None;
        while Instant::now() < deadline && !(ready && diagnostics.is_some()) {
            for event in server.poll() {
                match event {
                    ServerEvent::Ready => ready = true,
                    ServerEvent::Diagnostics { diagnostics: d, .. } => diagnostics = Some(d),
                    ServerEvent::Response { .. } => {}
                    ServerEvent::ApplyEdit { .. } => {}
                    ServerEvent::Failed(_) => {}
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(ready, "the server should complete initialize");
        let diagnostics = diagnostics.expect("diagnostics");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "boom");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_a_published_diagnostic_version() {
        let params = json!({
            "uri": "file:///tmp/a.rs",
            "version": 7,
            "diagnostics": []
        });
        match parse_publish_diagnostics(&params).expect("event") {
            ServerEvent::Diagnostics { version, .. } => assert_eq!(version, Some(7)),
            other => panic!("expected diagnostics, got {other:?}"),
        }
    }

    #[test]
    fn diagnostics_staleness_rules() {
        assert!(diagnostics_are_stale(Some(1), Some(2)), "older is stale");
        assert!(!diagnostics_are_stale(Some(2), Some(2)));
        assert!(!diagnostics_are_stale(Some(3), Some(2)), "newer is kept");
        assert!(!diagnostics_are_stale(None, Some(2)), "missing is kept");
        assert!(!diagnostics_are_stale(Some(1), None));
    }

    #[test]
    fn requests_expire_and_bounded() {
        // Formatting is allowed longer than the other requests.
        assert!(
            request_timeout(RequestKind::Formatting) > request_timeout(RequestKind::Completion)
        );

        let now = Instant::now();
        let mut pending = HashMap::new();
        pending.insert(
            1,
            PendingRequest {
                kind: RequestKind::Hover,
                deadline: now - Duration::from_secs(1),
            },
        );
        pending.insert(
            2,
            PendingRequest {
                kind: RequestKind::Definition,
                deadline: now + Duration::from_secs(60),
            },
        );

        let expired = expire_requests(&mut pending, now);
        assert_eq!(expired, vec![(1, RequestKind::Hover)]);
        assert!(!pending.contains_key(&1), "expired entries are removed");
        assert!(pending.contains_key(&2), "live entries are kept");
    }
}
