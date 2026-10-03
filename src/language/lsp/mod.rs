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
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use serde_json::{Value, json};

use crate::language::diagnostics::{Diagnostic, Severity, TextPos};
use crate::language::id::LanguageId;

use jsonrpc::Message;

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
        _ => "plaintext",
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
}

/// Something the app consumes from a running server.
#[derive(Clone, Debug)]
pub enum ServerEvent {
    /// The `initialize` handshake completed; the server accepts document
    /// notifications.
    Ready,
    /// Fresh diagnostics for a document.
    Diagnostics {
        path: PathBuf,
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
    pending: HashMap<i64, RequestKind>,
    language: LanguageId,
}

impl Server {
    /// Spawn a server and begin the `initialize` handshake.
    pub fn start(
        language: LanguageId,
        program: &str,
        args: &[&str],
        root: &Path,
    ) -> io::Result<Self> {
        let mut child = Command::new(program)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("no stdout"))?;

        let (tx, rx) = mpsc::channel::<Message>();
        thread::Builder::new()
            .name("koda-lsp-reader".to_string())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                while let Ok(Some(value)) = jsonrpc::read_message(&mut reader) {
                    let Some(message) = jsonrpc::decode(value) else {
                        continue;
                    };
                    if tx.send(message).is_err() {
                        break;
                    }
                }
            })?;

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
        };

        let id = server.request(
            "initialize",
            json!({
                "processId": null,
                "clientInfo": { "name": "koda" },
                "rootUri": path_to_uri(root),
                "capabilities": {
                    "textDocument": {
                        "synchronization": { "dynamicRegistration": false, "didSave": false },
                        "publishDiagnostics": { "relatedInformation": false },
                        "completion": { "completionItem": { "snippetSupport": false } },
                        "hover": { "contentFormat": ["markdown", "plaintext"] },
                        "definition": {},
                        "references": {},
                        "rename": { "prepareSupport": false },
                        "codeAction": {}
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
                    "languageId": lsp_language_id(self.language),
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

    /// Ask the server for hover information at a position.
    pub fn hover(&mut self, path: &Path, line: usize, col: usize) {
        let _ = self.send_request(
            RequestKind::Hover,
            "textDocument/hover",
            position_params(path, line, col),
        );
    }

    /// Ask the server for the definition at a position.
    pub fn definition(&mut self, path: &Path, line: usize, col: usize) {
        let _ = self.send_request(
            RequestKind::Definition,
            "textDocument/definition",
            position_params(path, line, col),
        );
    }

    /// Ask the server for every reference to the symbol at a position.
    pub fn references(&mut self, path: &Path, line: usize, col: usize) {
        let mut params = position_params(path, line, col);
        if let Some(object) = params.as_object_mut() {
            object.insert("context".to_string(), json!({ "includeDeclaration": true }));
        }
        let _ = self.send_request(RequestKind::References, "textDocument/references", params);
    }

    /// Ask the server to rename the symbol at a position.
    pub fn rename(&mut self, path: &Path, line: usize, col: usize, new_name: &str) {
        let mut params = position_params(path, line, col);
        if let Some(object) = params.as_object_mut() {
            object.insert("newName".to_string(), json!(new_name));
        }
        let _ = self.send_request(RequestKind::Rename, "textDocument/rename", params);
    }

    /// Ask the server for code actions over a range.
    pub fn code_action(&mut self, path: &Path, start: (usize, usize), end: (usize, usize)) {
        let params = json!({
            "textDocument": { "uri": path_to_uri(path) },
            "range": {
                "start": { "line": start.0, "character": start.1 },
                "end": { "line": end.0, "character": end.1 }
            },
            "context": { "diagnostics": [] }
        });
        let _ = self.send_request(RequestKind::CodeActions, "textDocument/codeAction", params);
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
        loop {
            match self.events.try_recv() {
                Ok(Message::Response { id, result, error }) => {
                    let id_number = id.as_i64().unwrap_or(-1);
                    if Some(id_number) == self.init_id {
                        self.init_id = None;
                        match error {
                            Some(error) => events.push(ServerEvent::Failed(error.message)),
                            None if result.is_some() => {
                                self.ready = true;
                                let _ = self.notify("initialized", json!({}));
                                events.push(ServerEvent::Ready);
                            }
                            None => {}
                        }
                    } else if let Some(kind) = self.pending.remove(&id_number) {
                        let result = match error {
                            Some(error) => Err(error.message),
                            None => Ok(result.unwrap_or(Value::Null)),
                        };
                        events.push(ServerEvent::Response {
                            kind,
                            id: id_number,
                            result,
                        });
                    }
                }
                Ok(Message::Notification { method, params }) => {
                    if method == "textDocument/publishDiagnostics"
                        && let Some(event) = parse_publish_diagnostics(&params)
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
                    events.push(ServerEvent::Failed("language server exited".to_string()));
                    break;
                }
            }
        }
        events
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
        self.pending.insert(id, kind);
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

fn parse_publish_diagnostics(params: &Value) -> Option<ServerEvent> {
    let path = uri_to_path(params.get("uri")?.as_str()?)?;
    let diagnostics = params
        .get("diagnostics")?
        .as_array()?
        .iter()
        .filter_map(parse_diagnostic)
        .collect();
    Some(ServerEvent::Diagnostics { path, diagnostics })
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
        let ServerEvent::Diagnostics { path, diagnostics } =
            parse_publish_diagnostics(&params).expect("event")
        else {
            panic!("expected diagnostics");
        };
        assert_eq!(path, PathBuf::from("/tmp/main.rs"));
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
}
