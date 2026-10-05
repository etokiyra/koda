//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

impl App {
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
    /// Replace the buffer with formatted text as a single undoable edit.
    pub(super) fn apply_formatted(&mut self, path: &Path, text: &str) {
        let applied = if let Some(doc) = self
            .editor
            .documents
            .iter_mut()
            .find(|doc| same_file(doc.buffer.path.as_deref(), path))
        {
            let cursor = doc.clamped_cursor();
            let last = doc.buffer.len_lines().saturating_sub(1);
            let end = Position::new(last, doc.buffer.line_char_len(last));
            doc.replace_range(Position::zero(), end, text);
            doc.cursor = doc.buffer.clamp_position(cursor);
            true
        } else {
            false
        };

        if applied {
            self.after_edit();
            self.set_status("Formatted");
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("file")
                .to_string();
            self.push_toast(ToastKind::Success, format!("Formatted {name}"));
        } else {
            self.set_status("File is no longer open");
        }
    }

    /// Format the active document with the language's trusted formatter.
    pub(super) fn format_document(&mut self) {
        let (path, language, text) = match self.editor.active_document() {
            Some(doc) => (
                doc.buffer.path.clone(),
                doc.buffer.language,
                doc.buffer.text(),
            ),
            None => return,
        };
        let Some(path) = path else {
            self.set_error("Save the file before formatting");
            return;
        };

        // Prefer the language server's formatter when it advertises one; the
        // built-in formatter tools remain the fallback.
        if self.lsp_supports(RequestKind::Formatting)
            && let Some(language) = self.lsp_language()
        {
            let tab_size = self
                .editor
                .active_document()
                .map(|doc| doc.indent_width())
                .unwrap_or(4);
            let version = self.document_version(&path).unwrap_or(0);
            if let Some(server) = self
                .lsp
                .get_mut(&language)
                .and_then(|job| job.server.as_mut())
            {
                self.pending_lsp_format = server
                    .formatting(&path, tab_size, true)
                    .map(|id| (path.clone(), language, id, version));
                if self.pending_lsp_format.is_some() {
                    self.set_status("Formatting…");
                    return;
                }
            }
        }

        if !self
            .language
            .provider(language)
            .capabilities()
            .contains(&Capability::Formatting)
        {
            self.set_status(format!(
                "Formatting is not available for {}",
                language.name()
            ));
            return;
        }

        self.format_seq += 1;
        let revision = self.format_seq;
        self.pending_format = Some((path.clone(), revision));
        self.background.format(path, language, text, revision);
    }

    /// Apply a formatting result, or explain why it could not run.
    pub(super) fn apply_format_outcome(&mut self, path: &Path, outcome: FormatOutcome) {
        match outcome {
            FormatOutcome::Formatted(text) => self.apply_formatted(path, &text),
            FormatOutcome::Unsupported => {
                self.set_status("Formatting is not available for this language")
            }
            FormatOutcome::ToolMissing { tool, hint } => {
                let message = format!("{tool} not found — {hint}");
                self.set_error(message.clone());
                self.push_toast(ToastKind::Error, message);
            }
            FormatOutcome::Failed(message) => {
                let text = format!("Format failed: {message}");
                self.set_error(text.clone());
                self.push_toast(ToastKind::Error, text);
            }
        }
    }
}
