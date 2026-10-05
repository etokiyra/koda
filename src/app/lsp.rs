//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

impl App {
    /// Tell a language's server that a document was closed, if it had it open.
    pub(super) fn notify_lsp_close(&mut self, path: &Path, language: LanguageId) {
        if let Some(server) = self
            .lsp
            .get_mut(&language)
            .and_then(|job| job.server.as_mut())
        {
            server.did_close(path);
        }
    }

    /// Start any language server whose delay has elapsed.
    pub(super) fn poll_lsp_start(&mut self) {
        let now = Instant::now();
        let due: Vec<LanguageId> = self
            .lsp
            .iter()
            .filter(|(_, job)| job.server.is_none() && job.start_at.is_some_and(|at| now >= at))
            .map(|(language, _)| *language)
            .collect();
        for language in due {
            if let Some(job) = self.lsp.get_mut(&language) {
                job.start_at = None;
            }
            let Some(launch) = self.lsp_launch(language) else {
                continue;
            };
            self.start_lsp_with_env(language, &launch.program, launch.args, &launch.env);
        }
    }

    /// Restart the language server for the active document on demand.
    pub(super) fn restart_language_server(&mut self) {
        let language = self
            .editor
            .active_document()
            .map(|doc| doc.buffer.language)
            .unwrap_or(LanguageId::Unknown);
        let Some(tool) = self
            .tools
            .as_ref()
            .and_then(|tools| Tool::available_server(language, tools))
        else {
            // Name the preferred server (or the language) so the message is
            // actionable when nothing is installed.
            match Tool::for_language(language, ToolPurpose::LanguageServer) {
                Some(tool) => self.set_status(format!(
                    "{} is not installed — see Language Setup…",
                    tool.label()
                )),
                None => self.set_status(format!("No language server for {}", language.name())),
            }
            return;
        };

        // Drop this language's connection and schedule a fresh one now. The
        // restart budget resets because the user asked explicitly.
        self.clear_lsp_pending();
        self.lsp.remove(&language);
        for doc in &mut self.editor.documents {
            doc.use_builtin_diagnostics();
        }
        self.lsp.insert(
            language,
            LspJob {
                start_at: Some(Instant::now()),
                ..LspJob::default()
            },
        );
        self.set_status(format!("Restarting {}…", tool.label()));
    }

    /// Give up on any handshake that has taken too long, falling back cleanly.
    ///
    /// A server that never answers `initialize` would otherwise leave Koda
    /// "connecting" forever; this turns that hang into the same graceful
    /// fallback as a crash.
    pub(super) fn poll_lsp_health(&mut self) -> bool {
        let now = Instant::now();
        let timed_out: Vec<LanguageId> = self
            .lsp
            .iter()
            .filter(|(_, job)| {
                job.server.as_ref().is_some_and(|server| !server.is_ready())
                    && job
                        .started_at
                        .is_some_and(|started| now.duration_since(started) >= LSP_HANDSHAKE_TIMEOUT)
            })
            .map(|(language, _)| *language)
            .collect();
        let mut changed = false;
        for language in timed_out {
            if let Some(job) = self.lsp.get_mut(&language) {
                job.server = None;
                job.started_at = None;
                job.failed = Some("initialize timed out".to_string());
            }
            for doc in &mut self.editor.documents {
                doc.use_builtin_diagnostics();
            }
            self.diagnostics_dirty_at = Some(Instant::now());
            let message = format!(
                "{} did not respond; using built-in intelligence",
                language.name()
            );
            self.set_error(message.clone());
            self.push_toast(ToastKind::Error, message);
            self.schedule_lsp_restart(language);
            changed = true;
        }
        changed
    }

    /// The program, arguments and environment for `language`'s server.
    pub(super) fn lsp_launch(&self, language: LanguageId) -> Option<LspLaunch> {
        let tools = self.tools.as_ref()?;
        let tool = Tool::available_server(language, tools)?;
        // Prefer the executable Koda discovered, which may live in a user bin
        // directory outside the process PATH.
        let program = tools
            .program_path(tool)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| tool.program().to_string());
        // Managed servers are launched with the runtimes Koda provisioned.
        Some(LspLaunch {
            program,
            args: tool.server_args(),
            env: crate::language::tools::launch_env(tool),
        })
    }

    /// Install diagnostics published by a language server.
    ///
    /// The ranges arrive in the connection's position encoding and the buffer
    /// may have moved on since the server last synchronized, so results for a
    /// dirty buffer are dropped and ranges are converted to character columns.
    pub(super) fn apply_lsp_diagnostics(&mut self, path: &Path, mut diagnostics: Vec<Diagnostic>) {
        let Some(index) = self
            .editor
            .documents
            .iter()
            .position(|doc| same_file(doc.buffer.path.as_deref(), path))
        else {
            return;
        };
        // `diagnostics_dirty` is set on every edit and cleared when the change
        // is streamed to the server, so it marks results computed against text
        // the server has not yet seen.
        if self.editor.documents[index].diagnostics_dirty() {
            return;
        }
        let language = self.editor.documents[index].buffer.language;
        let encoding = self.lsp_encoding(language);
        {
            let doc = &self.editor.documents[index];
            for diagnostic in &mut diagnostics {
                diagnostic.start.col = convert::lsp_to_char(
                    &doc.buffer.line_text(diagnostic.start.line),
                    diagnostic.start.col,
                    encoding,
                );
                diagnostic.end.col = convert::lsp_to_char(
                    &doc.buffer.line_text(diagnostic.end.line),
                    diagnostic.end.col,
                    encoding,
                );
            }
        }
        self.editor.documents[index].set_lsp_diagnostics(diagnostics);
    }

    /// Start a language server with extra environment variables.
    pub(super) fn start_lsp_with_env(
        &mut self,
        language: LanguageId,
        program: &str,
        args: &[&str],
        env: &[(String, String)],
    ) {
        let root = self.workspace.root().to_path_buf();
        match Server::start_with_env(language, program, args, &root, env) {
            Ok(server) => {
                let job = self.lsp.entry(language).or_default();
                job.server = Some(server);
                job.start_at = None;
                job.started_at = Some(Instant::now());
                job.failed = None;
            }
            Err(err) => {
                let job = self.lsp.entry(language).or_default();
                job.server = None;
                job.start_at = None;
                job.started_at = None;
                job.failed = Some(err.to_string());
                self.set_error(format!("Could not start {program}: {err}"));
                self.schedule_lsp_restart(language);
            }
        }
    }

    /// A server exited or failed to start: fall back for its language.
    pub(super) fn lsp_failed(&mut self, language: LanguageId, message: String) {
        if let Some(job) = self.lsp.get_mut(&language) {
            job.server = None;
            job.started_at = None;
            // A server that served for a good while earns a fresh budget; one
            // that crashes immediately after connecting keeps counting down.
            let stable = job
                .ready_at
                .take()
                .is_some_and(|ready| ready.elapsed() >= LSP_STABLE_UPTIME);
            if stable {
                job.restarts = 0;
            }
            job.failed = Some(message.clone());
        }
        // Requests in flight on the failed connection will never be answered.
        self.clear_lsp_pending();
        // Fall back to the built-in providers for every document.
        for doc in &mut self.editor.documents {
            doc.use_builtin_diagnostics();
        }
        self.diagnostics_dirty_at = Some(Instant::now());
        let text = format!("Language server stopped — {message}; using built-in intelligence");
        self.set_error(text.clone());
        self.push_toast(ToastKind::Error, text);
        self.schedule_lsp_restart(language);
    }

    /// Open a freshly detected document on its language's ready server.
    pub(super) fn sync_document_to_lsp(&mut self, path: &Path, language: LanguageId) {
        let text = match self.editor.documents.iter().find(|doc| {
            same_file(doc.buffer.path.as_deref(), path) && doc.buffer.language == language
        }) {
            Some(doc) => doc.buffer.text(),
            None => return,
        };
        if let Some(server) = self
            .lsp
            .get_mut(&language)
            .and_then(|job| job.server.as_mut())
            && server.is_ready()
            && !server.has_open_document(path)
        {
            server.did_open(path, &text);
        }
    }

    /// Aggregate connection state across every language server, for the
    /// statusline.
    pub fn lsp_status(&self) -> LspStatus {
        let mut starting = false;
        let mut failed = None;
        for job in self.lsp.values() {
            if let Some(server) = &job.server {
                if server.is_ready() {
                    return LspStatus::Ready;
                }
                starting = true;
            }
            if let Some(message) = &job.failed {
                failed = Some(message.clone());
            }
        }
        if starting {
            LspStatus::Starting
        } else if let Some(message) = failed {
            LspStatus::Failed(message)
        } else {
            LspStatus::Offline
        }
    }

    /// Schedule a bounded automatic restart for `language`, if a server tool is
    /// installed and the retry budget is not exhausted.
    pub(super) fn schedule_lsp_restart(&mut self, language: LanguageId) {
        let label = {
            let Some(tools) = &self.tools else {
                return;
            };
            let Some(tool) = Tool::available_server(language, tools) else {
                return;
            };
            let Some(job) = self.lsp.get_mut(&language) else {
                return;
            };
            if job.restarts >= MAX_LSP_RESTARTS || job.start_at.is_some() {
                return;
            }
            job.restarts += 1;
            job.start_at = Some(Instant::now() + LSP_RESTART_DELAY);
            tool.label()
        };
        self.set_status(format!("Restarting {label}…"));
    }

    /// The handshake finished: open every matching document on the server.
    pub(super) fn lsp_ready(&mut self, language: LanguageId) {
        if let Some(job) = self.lsp.get_mut(&language) {
            // Do not reset the restart budget here: a server that initializes
            // and then crashes must still exhaust its budget. A stable server
            // earns a fresh budget when it eventually crashes (see `lsp_failed`).
            job.started_at = None;
            job.ready_at = Some(Instant::now());
            job.failed = None;
        }
        let documents: Vec<(PathBuf, String)> = self
            .editor
            .documents
            .iter()
            .filter(|doc| doc.buffer.language == language)
            .filter_map(|doc| {
                doc.buffer
                    .path
                    .clone()
                    .map(|path| (path, doc.buffer.text()))
            })
            .collect();
        if let Some(server) = self
            .lsp
            .get_mut(&language)
            .and_then(|job| job.server.as_mut())
        {
            for (path, text) in documents {
                server.did_open(&path, &text);
            }
        }
    }

    /// Start a language server for `language` when one is installed, after a
    /// short delay so opening a file never waits on server startup.
    pub(super) fn maybe_start_lsp(&mut self, language: LanguageId) {
        if self.lsp.contains_key(&language) {
            return;
        }
        let Some(tools) = &self.tools else {
            return;
        };
        let Some(_tool) = Tool::available_server(language, tools) else {
            return;
        };
        self.lsp.insert(
            language,
            LspJob {
                start_at: Some(Instant::now() + LSP_START_DELAY),
                ..LspJob::default()
            },
        );
    }

    /// Drain every language server's events. Returns `true` when something
    /// changed.
    pub(super) fn poll_lsp(&mut self) -> bool {
        let languages: Vec<LanguageId> = self.lsp.keys().copied().collect();
        let mut changed = false;
        for language in languages {
            let events = match self
                .lsp
                .get_mut(&language)
                .and_then(|job| job.server.as_mut())
            {
                Some(server) => server.poll(),
                None => continue,
            };
            for event in events {
                changed = true;
                match event {
                    ServerEvent::Ready => self.lsp_ready(language),
                    ServerEvent::Diagnostics {
                        path,
                        version: _,
                        diagnostics,
                    } => {
                        self.apply_lsp_diagnostics(&path, diagnostics);
                    }
                    ServerEvent::Response { kind, id, result } => {
                        self.handle_lsp_response(language, kind, id, result);
                    }
                    ServerEvent::ApplyEdit { id, params } => {
                        let edit = params.get("edit").cloned().unwrap_or(Value::Null);
                        let files = convert::workspace_edit(&edit);
                        let applied = self.apply_workspace_edit(files, language);
                        if let Some(server) = self
                            .lsp
                            .get_mut(&language)
                            .and_then(|job| job.server.as_mut())
                        {
                            server.apply_edit_response(&id, applied > 0);
                        }
                        if applied > 0 {
                            self.set_status(format!("Applied {applied} edit(s)"));
                        }
                    }
                    ServerEvent::Failed(message) => self.lsp_failed(language, message),
                }
            }
        }
        changed
    }

    /// Start a language server, recording our attempt either way.
    ///
    /// Test helper: production launches go through `start_lsp_with_env`.
    #[cfg(test)]
    pub(super) fn start_lsp(&mut self, language: LanguageId, program: &str, args: &[&str]) {
        self.start_lsp_with_env(language, program, args, &[]);
    }
    /// The position encoding the server for `language` negotiated, or the
    /// protocol default when there is no server.
    pub(super) fn lsp_encoding(&self, language: LanguageId) -> PositionEncoding {
        self.lsp
            .get(&language)
            .and_then(|job| job.server.as_ref())
            .map(Server::position_encoding)
            .unwrap_or_default()
    }

    /// The language of the active document, when a ready server serves it.
    pub(super) fn lsp_language(&self) -> Option<LanguageId> {
        let doc = self.editor.active_document()?;
        let language = doc.buffer.language;
        let server = self.lsp.get(&language)?.server.as_ref()?;
        server.is_ready().then_some(language)
    }

    /// Whether the active document's ready server advertises `kind`.
    pub(super) fn lsp_supports(&self, kind: RequestKind) -> bool {
        self.lsp_language()
            .and_then(|language| self.lsp.get(&language))
            .and_then(|job| job.server.as_ref())
            .is_some_and(|server| server.supports(kind))
    }

    /// A mutable handle to the ready server for the active document.
    pub(super) fn active_server_mut(&mut self) -> Option<&mut Server> {
        let language = self.lsp_language()?;
        self.lsp.get_mut(&language)?.server.as_mut()
    }

    /// The active language's ready server that supports `kind`, or any ready
    /// server that does, for workspace-wide requests.
    pub(super) fn ready_server_for(&self, kind: RequestKind) -> Option<LanguageId> {
        let supports = |language: &LanguageId| {
            self.lsp
                .get(language)
                .and_then(|job| job.server.as_ref())
                .is_some_and(|server| server.is_ready() && server.supports(kind))
        };
        let preferred = self.editor.active_document().map(|doc| doc.buffer.language);
        preferred
            .filter(supports)
            .or_else(|| self.lsp.keys().copied().find(supports))
    }

    /// Convert a Koda character column to the LSP `character` offset the
    /// document's server expects, using the line's actual text.
    pub(super) fn lsp_col(
        &self,
        language: LanguageId,
        path: &Path,
        row: usize,
        col: usize,
    ) -> usize {
        let encoding = self.lsp_encoding(language);
        let line = self
            .editor
            .documents
            .iter()
            .find(|doc| same_file(doc.buffer.path.as_deref(), path))
            .map(|doc| doc.buffer.line_text(row))
            .unwrap_or_default();
        convert::char_to_lsp(&line, col, encoding)
    }

    /// The active document's `(path, line, col)` when a ready server owns it.
    pub(super) fn lsp_target(&self) -> Option<(PathBuf, usize, usize)> {
        let language = self.lsp_language()?;
        let doc = self.editor.active_document()?;
        let server = self.lsp.get(&language)?.server.as_ref()?;
        let path = doc.buffer.path.clone()?;
        if !server.has_open_document(&path) {
            return None;
        }
        let cursor = doc.clamped_cursor();
        Some((path, cursor.row, cursor.col))
    }

    /// Convert an LSP-encoded position from a server into a Koda character
    /// position, using the open document's line text.
    pub(super) fn lsp_position_to_char(&self, position: Position) -> Position {
        let Some(doc) = self.editor.active_document() else {
            return position;
        };
        let encoding = self.lsp_encoding(doc.buffer.language);
        let line = doc.buffer.line_text(position.row);
        Position::new(
            position.row,
            convert::lsp_to_char(&line, position.col, encoding),
        )
    }

    /// Whether `path` is the active document.
    pub(super) fn active_document_is(&self, path: &Path) -> bool {
        self.editor
            .active_document()
            .is_some_and(|doc| same_file(doc.buffer.path.as_deref(), path))
    }

    /// Forget the in-flight request bookkeeping for one feature.
    pub(super) fn clear_lsp_pending_for(&mut self, kind: RequestKind) {
        match kind {
            RequestKind::Completion => self.completion_request = None,
            RequestKind::Hover => self.hover_request = None,
            RequestKind::SignatureHelp => self.signature_request = None,
            RequestKind::Definition => self.pending_definition = None,
            RequestKind::References => self.pending_references = None,
            RequestKind::Rename => self.pending_rename_request = None,
            RequestKind::CodeActions => self.pending_code_action_request = None,
            RequestKind::Formatting => self.pending_lsp_format = None,
            RequestKind::WorkspaceSymbols => self.ws_lsp_pending = false,
        }
    }

    /// The active document's target when a ready server that supports `kind`
    /// owns it. Falls back to `None` so built-in intelligence takes over.
    pub(super) fn lsp_target_for(&self, kind: RequestKind) -> Option<(PathBuf, usize, usize)> {
        if !self.lsp_supports(kind) {
            return None;
        }
        self.lsp_target()
    }

    /// Forget every in-flight language-server request, e.g. after a crash or a
    /// restart, so stale callbacks cannot touch the new connection's state.
    pub(super) fn clear_lsp_pending(&mut self) {
        self.completion_request = None;
        self.hover_request = None;
        self.signature_request = None;
        self.pending_definition = None;
        self.pending_references = None;
        self.pending_rename_request = None;
        self.pending_code_action_request = None;
        self.pending_lsp_format = None;
        self.pending_code_actions_context = None;
        self.ws_lsp_pending = false;
    }

    /// The buffer version of the document at `path`, if it is open.
    pub(super) fn document_version(&self, path: &Path) -> Option<u64> {
        self.editor
            .documents
            .iter()
            .find(|doc| same_file(doc.buffer.path.as_deref(), path))
            .map(|doc| doc.buffer.version)
    }
}
