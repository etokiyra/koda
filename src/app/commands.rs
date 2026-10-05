//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

impl App {
    pub(super) fn run_picker_action(&mut self, action: PickerAction) {
        match action {
            PickerAction::Command(id) => self.execute_command(id),
            PickerAction::OpenPath(path) => self.open_path(path),
            PickerAction::Reveal { path, position } => self.reveal(path, position),
            PickerAction::RevealLsp { path, position } => self.reveal_lsp(path, position),
            PickerAction::Info(message) => self.set_status(message),
            PickerAction::InstallTool(tool) => self.install_tool(tool),
            PickerAction::ToolActions(tool) => self.tool_actions(tool),
            PickerAction::UpdateTool(tool) => self.install_tool(tool),
            PickerAction::RemoveTool(tool) => self.remove_tool(tool),
            PickerAction::ApplyCodeAction(index) => self.apply_code_action(index),
            PickerAction::DeletePath(path) => self.delete_path(&path),
            PickerAction::ShowDiff { path, staged } => self.show_diff(path, Some(staged)),
        }
    }

    pub(super) fn submit_prompt(&mut self, kind: PromptKind, input: String) {
        let input = input.trim().to_string();
        match kind {
            PromptKind::GotoLine => match input.parse::<usize>() {
                Ok(line) => self.with_doc(|d| d.go_to_line(line)),
                Err(_) => self.set_error("Not a valid line number"),
            },
            PromptKind::OpenPath => {
                if input.is_empty() {
                    self.set_error("No path provided");
                } else {
                    self.open_path(self.resolve_path(&input));
                }
            }
            PromptKind::SaveAs => {
                if input.is_empty() {
                    self.set_error("No path provided");
                } else {
                    self.save_as(self.resolve_path(&input));
                }
            }
            PromptKind::Rename => {
                if input.is_empty() {
                    self.set_error("No new name provided");
                    return;
                }
                let Some((path, row, col)) = self.pending_rename.take() else {
                    return;
                };
                // The prompt may be answered after the user switched tabs, so
                // resolve the language and version from the document itself.
                let language = self
                    .editor
                    .documents
                    .iter()
                    .find(|doc| same_file(doc.buffer.path.as_deref(), &path))
                    .map(|doc| doc.buffer.language)
                    .or_else(|| self.lsp_language());
                let Some(language) = language else {
                    self.set_status("Rename needs a language server");
                    return;
                };
                let lsp_col = self.lsp_col(language, &path, row, col);
                let version = self.document_version(&path).unwrap_or(0);
                if let Some(server) = self
                    .lsp
                    .get_mut(&language)
                    .and_then(|job| job.server.as_mut())
                {
                    self.pending_rename_request =
                        server
                            .rename(&path, row, lsp_col, &input)
                            .map(|id| PendingDocRequest {
                                language,
                                id,
                                path: path.clone(),
                                version,
                            });
                }
                self.set_status("Renaming…");
            }
            PromptKind::NewFile => {
                if input.is_empty() {
                    self.set_error("No file name provided");
                    return;
                }
                let path = if Path::new(&input).is_absolute() {
                    PathBuf::from(&input)
                } else {
                    self.new_file_dir().join(&input)
                };
                match filesystem::create_empty_file(&path) {
                    Ok(()) => {
                        self.tree_visible = true;
                        self.workspace.tree.refresh();
                        self.workspace.tree.select_path(&path);
                        self.open_path(path);
                    }
                    Err(err) => self.set_error(format!("Could not create file: {err}")),
                }
            }
            PromptKind::RenameFile => {
                let Some(old) = self.pending_rename_file.take() else {
                    return;
                };
                if input.is_empty() {
                    self.set_error("No new name provided");
                    return;
                }
                let new = if Path::new(&input).is_absolute() {
                    PathBuf::from(&input)
                } else {
                    old.parent()
                        .map(|parent| parent.join(&input))
                        .unwrap_or_else(|| PathBuf::from(&input))
                };
                if new == old {
                    self.set_status("Name unchanged");
                    return;
                }
                match filesystem::rename_path(&old, &new) {
                    Ok(()) => {
                        self.after_file_rename(&old, &new);
                        self.set_status(format!("Renamed to {}", new.display()));
                    }
                    Err(err) => self.set_error(format!("Rename failed: {err}")),
                }
            }
            PromptKind::CopyFile => {
                let Some(source) = self.pending_copy_file.take() else {
                    return;
                };
                if input.is_empty() {
                    self.set_error("No destination provided");
                    return;
                }
                let destination = if Path::new(&input).is_absolute() {
                    PathBuf::from(&input)
                } else {
                    self.workspace.root().join(&input)
                };
                match filesystem::copy_file(&source, &destination) {
                    Ok(()) => {
                        self.workspace.tree.refresh();
                        self.workspace.tree.select_path(&destination);
                        self.set_status(format!("Copied to {}", destination.display()));
                    }
                    Err(err) => self.set_error(format!("Copy failed: {err}")),
                }
            }
            PromptKind::ProjectSearch => {
                if input.is_empty() {
                    self.set_error("No search text provided");
                    return;
                }
                self.project_search_seq += 1;
                let revision = self.project_search_seq;
                self.pending_project_search = Some(revision);
                self.background.search_project(
                    self.workspace.root().to_path_buf(),
                    input.clone(),
                    revision,
                );
                self.set_status(format!("Searching for \"{input}\"…"));
            }
            PromptKind::CommitMessage => {
                if input.is_empty() {
                    self.set_error("No commit message provided");
                    return;
                }
                if self.pending_commit {
                    self.set_status("A commit is already running");
                    return;
                }
                self.pending_commit = true;
                self.background
                    .commit_all(self.workspace.root().to_path_buf(), input.clone());
                self.set_status(format!("Committing: {input}"));
            }
        }
    }

    /// Execute a command by id.
    pub fn execute_command(&mut self, id: &str) {
        match id {
            ids::SAVE => self.save(),
            ids::SAVE_ALL => self.save_all(),
            ids::SAVE_AS => self.open_prompt(PromptKind::SaveAs, "Save as", "path/to/file"),
            ids::OPEN => self.open_prompt(PromptKind::OpenPath, "Open file", "path/to/file.rs"),
            ids::QUICK_OPEN => self.open_quick_open(),
            ids::CLOSE_TAB => self.close_tab(),
            ids::CLOSE_ALL => self.close_all(),
            ids::REVERT => self.revert_file(),
            ids::NEW_FILE => self.new_file(),
            ids::RENAME_FILE => self.rename_selected(),
            ids::DELETE_FILE => self.delete_selected(),
            ids::DUPLICATE_FILE => self.duplicate_file(),
            ids::COPY_FILE => self.copy_file(),
            ids::QUIT => self.request_quit(),
            ids::HOME => self.go_home(),
            ids::WELCOME_SCENE => {
                self.welcome_scene = self.welcome_scene.next();
                self.set_status(format!("Welcome scene: {}", self.welcome_scene.label()));
            }
            ids::TOGGLE_MOTION => {
                self.motion = !self.motion;
                let message = if self.motion {
                    "Animations on"
                } else {
                    "Animations off"
                };
                self.set_status(message);
            }
            ids::OPEN_PROJECT => self.open_dir_picker(),
            ids::NEW_PROJECT => self.open_new_project(),
            ids::UNDO => self.with_doc(|d| d.undo()),
            ids::REDO => self.with_doc(|d| d.redo()),
            ids::SELECT_ALL => self.with_doc(|d| d.select_all()),
            ids::SELECT_NEXT => self.with_doc(|d| d.select_next_occurrence()),
            ids::SELECT_ALL_OCCURRENCES => self.with_doc(|d| d.select_all_occurrences()),
            ids::ADD_CURSOR_BELOW => self.with_doc(|d| d.add_cursor_below()),
            ids::ADD_CURSOR_ABOVE => self.with_doc(|d| d.add_cursor_above()),
            ids::COPY => self.copy(),
            ids::CUT => self.cut(),
            ids::PASTE => self.paste(),
            ids::YANK_POP => self.yank_pop(),
            ids::COMPLETE => self.open_completion(),
            ids::HOVER => self.open_hover(),
            ids::SETUP => self.language_setup(),
            ids::RESTART_SERVER => self.restart_language_server(),
            ids::WORKSPACE_SYMBOLS => self.open_workspace_symbols(),
            ids::PROJECT_SEARCH => self.open_project_search(),
            ids::CHANGED_FILES => self.open_changed_files(),
            ids::GIT_COMMIT => self.commit_changes(),
            ids::GIT_TOGGLE_STAGE => self.toggle_stage_target(),
            ids::DIFF => self.diff_active_file(),
            ids::FIND => self.open_search(false),
            ids::REPLACE => self.open_search(true),
            ids::REPLACE_ALL => self.replace_all(),
            ids::GOTO_LINE => self.open_prompt(PromptKind::GotoLine, "Go to line", "42"),
            ids::TOGGLE_COMMENT => self.toggle_comment(),
            ids::INDENT => self.with_doc(|d| d.indent()),
            ids::OUTDENT => self.with_doc(|d| d.outdent()),
            ids::MOVE_LINE_UP => self.with_doc(|d| d.move_line_up()),
            ids::MOVE_LINE_DOWN => self.with_doc(|d| d.move_line_down()),
            ids::DUPLICATE_LINE => self.with_doc(|d| d.duplicate_line()),
            ids::DELETE_LINE => self.with_doc(|d| d.delete_line()),
            ids::MATCHING_BRACKET => self.goto_matching_bracket(),
            ids::TOGGLE_TREE => self.toggle_tree(),
            ids::FOCUS_TREE => self.focus_tree(),
            ids::TOGGLE_HIDDEN => self.toggle_hidden(),
            ids::TOGGLE_INLINE_DIAGNOSTICS => self.toggle_inline_diagnostics(),
            ids::TOGGLE_WRAP => self.toggle_wrap(),
            ids::REFRESH => self.refresh_workspace(),
            ids::SPLIT => self.toggle_split(),
            ids::FOCUS_PANE => self.focus_other_pane(),
            ids::FILTER_TREE => self.open_tree_filter(),
            ids::NEXT_TAB => self.next_tab(),
            ids::PREV_TAB => self.previous_tab(),
            ids::PALETTE => self.open_command_palette(),
            ids::HELP => self.toggle_help(),
            ids::RENAME => self.rename_symbol(),
            ids::CODE_ACTIONS => self.code_actions(),
            ids::FORMAT => self.format_document(),
            ids::GOTO_DEFINITION => self.goto_definition(),
            ids::FIND_REFERENCES => self.find_references(),
            ids::SHOW_SYMBOLS => self.open_symbols(),
            ids::DIAGNOSTICS_NEXT => self.goto_diagnostic(1),
            ids::DIAGNOSTICS_PREV => self.goto_diagnostic(-1),
            ids::DIAGNOSTICS_LIST => self.open_diagnostics_list(),
            _ => {}
        }
    }
}
