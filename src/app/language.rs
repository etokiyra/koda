//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

impl App {
    /// Open completion for the word being typed.
    pub(super) fn open_completion(&mut self) {
        self.open_completion_inner(true);
    }

    /// Whether the cursor sits just after a member-access operator (`.` or `::`).
    pub(super) fn cursor_in_member_access(&self) -> bool {
        let Some(doc) = self.editor.active_document() else {
            return false;
        };
        let cursor = doc.clamped_cursor();
        let line = doc.buffer.line_text(cursor.row);
        let chars: Vec<char> = line.chars().collect();
        let mut start = cursor.col.min(chars.len());
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }
        start > 0 && matches!(chars[start - 1], '.' | ':')
    }

    /// After an edit that may shorten the typed prefix, re-filter an open popup
    /// and schedule the next automatic offer. Does nothing when the popup is
    /// closed, since Backspace should not by itself pop one open.
    pub(super) fn after_completion_edit(&mut self) {
        if self.completion.is_none() {
            return;
        }
        if self.completion_prefix().is_empty() {
            self.completion = None;
            self.completion_due = None;
            return;
        }
        self.refresh_completion();
        self.schedule_auto_completion();
    }

    /// After typing a word character, re-filter an open popup and schedule an
    /// automatic offer.
    pub(super) fn after_word_char_typed(&mut self) {
        if self.completion.is_some() {
            self.refresh_completion();
        }
        self.schedule_auto_completion();
    }

    /// Show information about the symbol under the cursor.
    pub(super) fn open_hover(&mut self) {
        let (language, text, cursor) = match self.editor.active_document() {
            Some(doc) => (doc.buffer.language, doc.buffer.text(), doc.clamped_cursor()),
            None => return,
        };
        let local = self
            .language
            .provider(language)
            .hover(&text, cursor.row, cursor.col);
        self.completion = None;
        self.hover = local.map(|hover| HoverState {
            title: hover.title,
            kind: hover.kind,
            body: hover.body,
        });

        // Ask the language server for a richer answer when one is attached.
        if let Some((path, row, col)) = self.lsp_target_for(RequestKind::Hover) {
            let col = self.lsp_col(language, &path, row, col);
            if let Some(server) = self.active_server_mut() {
                self.hover_request = server.hover(&path, row, col).map(|id| (language, id));
            }
            return;
        }
        if self.hover.is_none() {
            self.set_status("No symbol under the cursor");
        }
    }

    /// Re-filter the open completion for the current prefix, closing it when
    /// nothing matches.
    pub(super) fn refresh_completion(&mut self) {
        let prefix = self.completion_prefix();
        if let Some(state) = self.completion.as_mut() {
            state.set_prefix(prefix);
        }
        if self
            .completion
            .as_ref()
            .is_some_and(|state| state.items.is_empty())
        {
            self.completion = None;
        }
    }

    /// Whether the cursor is inside a comment or a string, according to the
    /// provider's highlighting. Used to keep automatic completion quiet in prose
    /// and literals.
    ///
    /// The probe is the start of the identifier being typed (or the character
    /// before the cursor when there is none), because the cursor sits just past
    /// the last typed character and a highlight span's end is exclusive.
    pub(super) fn cursor_in_comment_or_string(&mut self) -> bool {
        let Some((id, row, probe)) = self.editor.active_document().map(|doc| {
            let cursor = doc.clamped_cursor();
            let line = doc.buffer.line_text(cursor.row);
            let chars: Vec<char> = line.chars().collect();
            let end = cursor.col.min(chars.len());
            let mut start = end;
            while start > 0 && is_word_char(chars[start - 1]) {
                start -= 1;
            }
            let probe = if start < end {
                start
            } else {
                end.saturating_sub(1)
            };
            (doc.buffer.language, cursor.row, probe)
        }) else {
            return false;
        };
        let service = Arc::clone(&self.language);
        let provider = service.provider(id);
        let Some(doc) = self.editor.active_document_mut() else {
            return false;
        };
        matches!(
            doc.token_kind_at(provider, row, probe),
            TokenKind::Comment | TokenKind::String
        )
    }

    /// Open the popup once the typing pause has elapsed.
    pub(super) fn poll_auto_completion(&mut self) -> bool {
        let Some(pending) = self.completion_due.as_ref() else {
            return false;
        };
        if Instant::now() < pending.due_at {
            return false;
        }
        // Only offer if nothing else has happened: the same document is active,
        // the cursor has not moved and the buffer has not changed. This drops
        // the timer when the user dismissed the popup, opened an overlay or
        // switched tabs.
        let valid = self.overlay.is_none()
            && !self.search.open
            && self.focus == Focus::Editor
            && self.editor.active_index() == pending.doc
            && self.editor.active_document().is_some_and(|doc| {
                doc.clamped_cursor() == pending.cursor && doc.buffer.version == pending.version
            });
        self.completion_due = None;
        if !valid {
            return false;
        }
        if self.completion.is_some() {
            // The popup is open and already re-filtered; refresh the server's
            // candidates for the new prefix.
            self.request_lsp_completion();
            return false;
        }
        self.open_completion_inner(false)
    }

    /// Build and show the completion popup.
    ///
    /// `manual` distinguishes an explicit `Ctrl+Space` (which reports when there
    /// is nothing to offer) from the automatic, typing-driven offer, which stays
    /// silent rather than interrupting with an empty popup.
    pub(super) fn open_completion_inner(&mut self, manual: bool) -> bool {
        let (language, text, cursor) = match self.editor.active_document() {
            Some(doc) => (doc.buffer.language, doc.buffer.text(), doc.clamped_cursor()),
            None => return false,
        };

        // After `.` or `::` the buffer's own words are noise: only the language
        // (and the server, when attached) know the members.
        let member = self.cursor_in_member_access();
        let mut pool: Vec<Completion> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for item in self
            .language
            .provider(language)
            .completions(&text, cursor.row, cursor.col)
        {
            if seen.insert(item.label.clone()) {
                pool.push(item);
            }
        }
        if !member {
            for item in document_words(&text) {
                if seen.insert(item.label.clone()) {
                    pool.push(item);
                }
            }
        }

        let prefix = self.completion_prefix();
        let state = CompletionState::new(pool, prefix.clone());
        let lsp_available = self.lsp_supports(RequestKind::Completion);

        if state.items.is_empty() {
            if manual {
                // With a server attached, open an empty popup and let its
                // response fill it in.
                if lsp_available {
                    self.completion = Some(state);
                    self.request_lsp_completion();
                    return true;
                }
                self.set_status("No completions");
            }
            return false;
        }

        // An automatic popup whose only candidate is the word already being
        // typed would just sit there; leave it closed.
        let only_self = state.items.len() == 1 && state.items[0].label == prefix;
        if !manual && only_self && !lsp_available {
            return false;
        }

        self.completion = Some(state);
        self.request_lsp_completion();
        true
    }

    /// Ask the language server for completions at the cursor.
    pub(super) fn request_lsp_completion(&mut self) {
        let Some(language) = self.lsp_language() else {
            return;
        };
        let Some((path, row, col)) = self.lsp_target_for(RequestKind::Completion) else {
            return;
        };
        let col = self.lsp_col(language, &path, row, col);
        if let Some(server) = self
            .lsp
            .get_mut(&language)
            .and_then(|job| job.server.as_mut())
        {
            self.completion_request = server.completion(&path, row, col).map(|id| (language, id));
        }
    }

    /// Schedule an automatic completion a short pause after the last keystroke.
    ///
    /// Suppressed inside comments and strings, and pinned to the current
    /// document, cursor and buffer version so the offer is dropped if the user
    /// moves or edits before it fires.
    pub(super) fn schedule_auto_completion(&mut self) {
        if self.cursor_in_comment_or_string() {
            self.completion = None;
            self.completion_due = None;
            return;
        }
        let (doc_index, cursor, version) = match self.editor.active_document() {
            Some(doc) => (
                self.editor.active_index(),
                doc.clamped_cursor(),
                doc.buffer.version,
            ),
            None => {
                self.completion_due = None;
                return;
            }
        };
        self.completion_due = Some(PendingCompletion {
            due_at: Instant::now() + AUTOCOMPLETE_DELAY,
            doc: doc_index,
            cursor,
            version,
        });
    }

    /// The identifier characters immediately before the cursor.
    pub(super) fn completion_prefix(&self) -> String {
        let Some(doc) = self.editor.active_document() else {
            return String::new();
        };
        let cursor = doc.clamped_cursor();
        let line = doc.buffer.line_text(cursor.row);
        let chars: Vec<char> = line.chars().collect();
        let end = cursor.col.min(chars.len());
        let mut start = end;
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }
        chars[start..end].iter().collect()
    }

    /// Replace the typed prefix with the selected completion.
    pub(super) fn accept_completion(&mut self) {
        let Some(state) = self.completion.take() else {
            return;
        };
        self.completion_due = None;
        let Some(item) = state.selected_item().cloned() else {
            return;
        };
        let Some(doc) = self.editor.active_document() else {
            return;
        };
        let cursor = doc.clamped_cursor();
        let line = doc.buffer.line_text(cursor.row);
        let chars: Vec<char> = line.chars().collect();
        let end = cursor.col.min(chars.len());
        let mut start = end;
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }
        let start = Position::new(cursor.row, start);
        let end = Position::new(cursor.row, end);
        self.with_doc(|doc| doc.replace_range(start, end, &item.label));
    }

    /// React to a literal character typed in the editor: drive completion and
    /// signature help together.
    pub(super) fn after_typed_char(&mut self, c: char) {
        if is_word_char(c) {
            self.after_word_char_typed();
        } else {
            // Whitespace or punctuation ends the word: dismiss the completion
            // popup so it does not linger with an empty prefix.
            self.completion = None;
            self.completion_due = None;
        }

        if matches!(c, '(' | ',') {
            self.request_lsp_signature();
        } else if !is_word_char(c) && c != ' ' {
            // A closing bracket or any other punctuation leaves the arguments.
            self.signature = None;
            self.signature_request = None;
        }
    }

    /// Ask the language server for signature help at the cursor.
    pub(super) fn request_lsp_signature(&mut self) {
        let Some(language) = self.lsp_language() else {
            self.signature = None;
            return;
        };
        let Some((path, row, col)) = self.lsp_target_for(RequestKind::SignatureHelp) else {
            self.signature = None;
            return;
        };
        let col = self.lsp_col(language, &path, row, col);
        if let Some(server) = self
            .lsp
            .get_mut(&language)
            .and_then(|job| job.server.as_mut())
        {
            self.signature_request = server
                .signature_help(&path, row, col)
                .map(|id| (language, id));
        }
    }
    /// List every occurrence of the word under the cursor.
    pub(super) fn find_references(&mut self) {
        if let Some(language) = self.lsp_language()
            && let Some((path, row, col)) = self.lsp_target_for(RequestKind::References)
        {
            let col = self.lsp_col(language, &path, row, col);
            let version = self.document_version(&path).unwrap_or(0);
            if let Some(server) = self.active_server_mut() {
                self.pending_references =
                    server
                        .references(&path, row, col)
                        .map(|id| PendingDocRequest {
                            language,
                            id,
                            path: path.clone(),
                            version,
                        });
            }
            self.set_status("Finding references…");
            return;
        }

        let (path, language, file, text, cursor) = match self.editor.active_document() {
            Some(doc) => (
                doc.buffer.path.clone(),
                doc.buffer.language,
                doc.file_name(),
                doc.buffer.text(),
                doc.clamped_cursor(),
            ),
            None => return,
        };
        let Some(path) = path else {
            return;
        };
        let locations = self
            .language
            .provider(language)
            .references(&text, cursor.row, cursor.col);
        if locations.is_empty() {
            self.set_status("No symbol under the cursor");
            return;
        }

        let lines: Vec<&str> = text.lines().collect();
        let items = locations
            .into_iter()
            .map(|location| {
                let snippet = lines
                    .get(location.line)
                    .map(|line| line.trim().to_string())
                    .filter(|line| !line.is_empty())
                    .unwrap_or_else(|| format!("line {}", location.line + 1));
                let detail = format!("{file}:{}:{}", location.line + 1, location.col + 1);
                PickerItem::new(
                    snippet,
                    detail,
                    PickerAction::Reveal {
                        path: path.clone(),
                        position: Position::new(location.line, location.col),
                    },
                )
            })
            .collect();
        let mut picker = Picker::new("References", "Filter occurrences…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Offer a picker of jump targets.
    pub(super) fn open_location_picker(&mut self, title: &str, locations: Vec<convert::Location>) {
        let root = self.workspace.root().to_path_buf();
        let items = locations
            .into_iter()
            .map(|location| {
                let relative = location
                    .path
                    .strip_prefix(&root)
                    .unwrap_or(&location.path)
                    .display()
                    .to_string();
                let detail = format!("{}:{}", location.line + 1, location.col + 1);
                PickerItem::new(
                    relative,
                    detail,
                    PickerAction::RevealLsp {
                        path: location.path,
                        position: Position::new(location.line, location.col),
                    },
                )
            })
            .collect();
        let mut picker = Picker::new(title, "Filter locations…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Jump to the definition of the word under the cursor.
    pub(super) fn goto_definition(&mut self) {
        if let Some(language) = self.lsp_language()
            && let Some((path, row, col)) = self.lsp_target_for(RequestKind::Definition)
        {
            let col = self.lsp_col(language, &path, row, col);
            let version = self.document_version(&path).unwrap_or(0);
            if let Some(server) = self.active_server_mut() {
                self.pending_definition =
                    server
                        .definition(&path, row, col)
                        .map(|id| PendingDocRequest {
                            language,
                            id,
                            path: path.clone(),
                            version,
                        });
            }
            self.set_status("Resolving definition…");
            return;
        }

        let (path, language, file, text, cursor) = match self.editor.active_document() {
            Some(doc) => (
                doc.buffer.path.clone(),
                doc.buffer.language,
                doc.file_name(),
                doc.buffer.text(),
                doc.clamped_cursor(),
            ),
            None => return,
        };
        let Some(path) = path else {
            return;
        };
        let Some(symbol) = self
            .language
            .provider(language)
            .definition(&text, cursor.row, cursor.col)
        else {
            // Not defined in this file: search the project for a same-named
            // symbol, so F12 still works across files without a server.
            if let Some(word) = crate::language::symbols::word_at(&text, cursor.row, cursor.col) {
                self.open_workspace_symbols_with(Some(word));
                self.set_status("Searching the project…");
            } else {
                self.set_status("Nothing to look up here");
            }
            return;
        };
        let target = Position::new(symbol.line, symbol.col);
        self.reveal(path, target);
        self.set_status(format!(
            "{} {} · {file}:{}",
            symbol.kind.label(),
            symbol.name,
            symbol.line + 1
        ));
    }

    /// Ask the server for code actions over the cursor or selection.
    pub(super) fn code_actions(&mut self) {
        let Some(language) = self.lsp_language() else {
            self.set_status("Code actions need a language server (see Language Setup…)");
            return;
        };
        let Some((path, row, col)) = self.lsp_target_for(RequestKind::CodeActions) else {
            self.set_status("Code actions need a language server (see Language Setup…)");
            return;
        };
        let range = self
            .editor
            .active_document()
            .and_then(|doc| doc.selection_range())
            .map(|(start, end)| {
                (
                    (
                        start.row,
                        self.lsp_col(language, &path, start.row, start.col),
                    ),
                    (end.row, self.lsp_col(language, &path, end.row, end.col)),
                )
            })
            .unwrap_or_else(|| {
                let col = self.lsp_col(language, &path, row, col);
                ((row, col), (row, col))
            });
        let version = self.document_version(&path).unwrap_or(0);
        if let Some(server) = self.active_server_mut() {
            self.pending_code_action_request =
                server
                    .code_action(&path, range.0, range.1)
                    .map(|id| PendingDocRequest {
                        language,
                        id,
                        path: path.clone(),
                        version,
                    });
        }
        self.set_status("Finding code actions…");
    }

    /// [`apply_workspace_edit`] with the encoding supplied explicitly, so the
    /// conversion is testable without a live server.
    pub(super) fn apply_workspace_edit_with_encoding(
        &mut self,
        files: Vec<convert::FileEdit>,
        encoding: PositionEncoding,
    ) -> usize {
        let mut applied = 0;
        for file in files {
            if let Some(doc) = self
                .editor
                .documents
                .iter_mut()
                .find(|doc| same_file(doc.buffer.path.as_deref(), &file.path))
            {
                // Convert against the document's current text before applying;
                // edits are then applied from the end so earlier offsets stay
                // valid.
                let mut edits =
                    convert::edits_to_chars(&file.edits, |row| doc.buffer.line_text(row), encoding);
                edits.sort_by_key(|edit| Reverse((edit.start.0, edit.start.1)));
                for edit in edits {
                    doc.replace_range(
                        Position::new(edit.start.0, edit.start.1),
                        Position::new(edit.end.0, edit.end.1),
                        &edit.new_text,
                    );
                    applied += 1;
                }
            } else if let Ok(text) = crate::editor::buffer::read_text(&file.path) {
                let lines: Vec<&str> = text.split('\n').collect();
                let edits = convert::edits_to_chars(
                    &file.edits,
                    |row| lines.get(row).copied().unwrap_or("").to_string(),
                    encoding,
                );
                let updated = apply_text_edits(&text, &edits);
                if crate::filesystem::write_atomic(&file.path, &updated).is_ok() {
                    applied += file.edits.len();
                }
            }
        }
        applied
    }

    /// Apply a workspace edit produced by the server for `language`, returning
    /// the number of edits applied.
    ///
    /// Every range is converted from the connection's position encoding to Koda
    /// character offsets using the document's current text, so an edit can never
    /// land in the middle of a multi-byte character.
    pub(super) fn apply_workspace_edit(
        &mut self,
        files: Vec<convert::FileEdit>,
        language: LanguageId,
    ) -> usize {
        let encoding = self.lsp_encoding(language);
        self.apply_workspace_edit_with_encoding(files, encoding)
    }

    /// List the active document's definitions and jump to the chosen one.
    pub(super) fn open_symbols(&mut self) {
        let (path, language, file, text) = match self.editor.active_document() {
            Some(doc) => (
                doc.buffer.path.clone(),
                doc.buffer.language,
                doc.file_name(),
                doc.buffer.text(),
            ),
            None => return,
        };
        let Some(path) = path else {
            self.set_status("Symbols need a saved file");
            return;
        };

        let symbols = self.language.provider(language).symbols(&text);
        if symbols.is_empty() {
            self.set_status(format!("No symbols found in {file}"));
            return;
        }

        let items = symbols
            .into_iter()
            .map(|symbol| {
                let position = Position::new(symbol.line, symbol.col);
                let detail = format!("{}  ·  {file}:{}", symbol.kind.label(), symbol.line + 1);
                PickerItem::new(
                    symbol.name,
                    detail,
                    PickerAction::Reveal {
                        path: path.clone(),
                        position,
                    },
                )
            })
            .collect();
        let mut picker = Picker::new("Symbols", "Filter symbols…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Apply the code action at `index`, either as an edit or a command.
    pub(super) fn apply_code_action(&mut self, index: usize) {
        let Some(action) = self.pending_code_actions.get(index).cloned() else {
            return;
        };
        // The actions were computed against a snapshot; refuse to apply them if
        // the document has moved on since.
        let Some((path, version)) = self.pending_code_actions_context.clone() else {
            return;
        };
        if self.document_version(&path) != Some(version) {
            self.pending_code_actions.clear();
            self.pending_code_actions_context = None;
            self.set_status("The document changed; run code actions again");
            return;
        }
        let language = self
            .editor
            .documents
            .iter()
            .find(|doc| same_file(doc.buffer.path.as_deref(), &path))
            .map(|doc| doc.buffer.language)
            .or_else(|| self.lsp_language());
        let Some(language) = language else {
            return;
        };
        if let Some(edit) = action.edit {
            let files = convert::workspace_edit(&edit);
            let applied = self.apply_workspace_edit(files, language);
            if applied > 0 {
                self.set_status(format!("Applied {applied} edit(s)"));
            } else {
                self.set_status("Nothing to apply");
            }
        } else if let Some(command) = action.command {
            if let Some(server) = self.active_server_mut() {
                server.execute_command(&command.command, command.arguments);
            }
            self.set_status("Running action…");
        } else {
            self.set_status("This action does nothing");
        }
    }

    /// Prompt for a new name and ask the server to rename the symbol.
    pub(super) fn rename_symbol(&mut self) {
        let Some((path, row, col)) = self.lsp_target_for(RequestKind::Rename) else {
            self.set_status("Rename needs a language server (see Language Setup…)");
            return;
        };
        let word = self
            .editor
            .active_document()
            .and_then(|doc| crate::language::symbols::word_at(&doc.buffer.text(), row, col))
            .unwrap_or_default();

        self.pending_rename = Some((path, row, col));
        let mut prompt = Prompt::new(PromptKind::Rename, "Rename symbol", "new name");
        prompt.input = word;
        self.overlay = Overlay::Prompt(prompt);
    }
    /// Show the matches from a project-wide search.
    pub(super) fn open_project_search_picker(&mut self, matches: Vec<SearchMatch>) {
        if matches.is_empty() {
            self.set_status("No matches in project");
            return;
        }
        let total = matches.len();
        let root = self.workspace.root().to_path_buf();
        let items = matches
            .into_iter()
            .map(|entry| {
                let relative = entry
                    .path
                    .strip_prefix(&root)
                    .unwrap_or(&entry.path)
                    .display()
                    .to_string();
                let detail = format!("{relative}:{}", entry.line + 1);
                PickerItem::new(
                    entry.text,
                    detail,
                    PickerAction::Reveal {
                        path: entry.path,
                        position: Position::new(entry.line, entry.col),
                    },
                )
            })
            .collect();
        let mut picker = Picker::new("Search Results", "Filter results…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
        self.set_status(format!("{total} match(es)"));
    }

    /// Ask for project-wide symbols, optionally with a pre-applied query.
    ///
    /// `F12` uses a query so a definition that is not in the current file can
    /// still be found across the project without a language server.
    pub(super) fn open_workspace_symbols_with(&mut self, query: Option<String>) {
        self.pending_workspace_symbols_query = query.clone();
        if let Some(language) = self.ready_server_for(RequestKind::WorkspaceSymbols) {
            self.ws_lsp_pending = true;
            if let Some(server) = self
                .lsp
                .get_mut(&language)
                .and_then(|job| job.server.as_mut())
            {
                server.workspace_symbols(query.as_deref().unwrap_or(""));
            }
        }
        self.workspace_symbols_seq += 1;
        let revision = self.workspace_symbols_seq;
        self.pending_workspace_symbols = Some(revision);
        self.background
            .workspace_symbols(self.workspace.root().to_path_buf(), revision);
        self.set_status("Searching symbols…");
    }

    /// Ask for project-wide symbols: the language server when attached, plus the
    /// built-in scan as an immediate, always-available fallback.
    pub(super) fn open_workspace_symbols(&mut self) {
        self.open_workspace_symbols_with(None);
    }

    /// Prompt for a project-wide text query.
    pub(super) fn open_project_search(&mut self) {
        let mut prompt = Prompt::new(
            PromptKind::ProjectSearch,
            "Search in project",
            "text to find",
        );
        // Prefill from a single-line selection, so searching for the word under
        // the cursor is one keystroke.
        if let Some(text) = self
            .editor
            .active_document()
            .and_then(|doc| doc.selected_text())
        {
            let trimmed = text.trim();
            if !trimmed.is_empty() && !trimmed.contains('\n') {
                prompt.input = trimmed.to_string();
            }
        }
        self.overlay = Overlay::Prompt(prompt);
    }

    pub(super) fn open_workspace_symbol_picker(&mut self, symbols: Vec<WorkspaceSymbol>) {
        if symbols.is_empty() {
            // A server may still be answering; only report failure when nothing
            // else is coming and no picker is showing.
            if !self.ws_lsp_pending && self.overlay.is_none() {
                self.set_status("No symbols found");
            }
            return;
        }
        let root = self.workspace.root().to_path_buf();
        let items = symbols
            .into_iter()
            .map(|entry| {
                let relative = entry
                    .path
                    .strip_prefix(&root)
                    .unwrap_or(&entry.path)
                    .display()
                    .to_string();
                let detail = format!(
                    "{}  ·  {relative}:{}",
                    entry.symbol.kind.label(),
                    entry.symbol.line + 1
                );
                PickerItem::new(
                    entry.symbol.name,
                    detail,
                    PickerAction::Reveal {
                        path: entry.path,
                        position: Position::new(entry.symbol.line, entry.symbol.col),
                    },
                )
            })
            .collect();
        self.merge_workspace_symbols(items);
    }

    /// Show or extend the workspace-symbol picker, keeping any results already
    /// listed. Used by both the built-in scan and the language server.
    pub(super) fn merge_workspace_symbols(&mut self, items: Vec<PickerItem>) {
        if let Overlay::Picker(picker) = &mut self.overlay
            && picker.title == "Workspace Symbols"
        {
            picker.extend_items(items);
            return;
        }
        let mut picker = Picker::new("Workspace Symbols", "Filter symbols…", items);
        if let Some(query) = self.pending_workspace_symbols_query.take() {
            picker.query = query;
        }
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }
}
