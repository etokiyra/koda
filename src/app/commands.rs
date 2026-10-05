//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

use crate::language::setup::{
    ProjectSetupPlan, ProjectSetupRun, SetupState, SetupSummary, plan_project,
};

impl App {
    pub(super) fn run_picker_action(&mut self, action: PickerAction) {
        match action {
            PickerAction::Command(id) => self.execute_command(id),
            PickerAction::OpenPath(path) => self.open_path(path),
            PickerAction::Reveal { path, position } => self.reveal(path, position),
            PickerAction::RevealLsp { path, position } => self.reveal_lsp(path, position),
            PickerAction::Info(message) => self.set_status(message),
            PickerAction::InstallTool(tool) => self.install_tool(tool),
            PickerAction::ProjectSetup => self.start_project_setup(),
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
            ids::SETUP_PROJECT => self.open_project_setup(),
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
    pub(super) fn open_command_palette(&mut self) {
        let items = self
            .commands
            .all()
            .iter()
            .map(|command| {
                let (enabled, hint) = self.command_availability(command);
                let mut item = PickerItem::new(
                    command.palette_label(),
                    command.description.to_string(),
                    PickerAction::Command(command.id),
                )
                .shortcut(command.shortcut.unwrap_or(""));
                if !enabled {
                    item = item.disabled(hint.unwrap_or_else(|| "unavailable".to_string()));
                }
                item
            })
            .collect();
        let mut picker = Picker::new("Command Palette", "Type a command…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Install a tool through its trusted package manager, on the worker.
    pub(super) fn install_tool(&mut self, tool: Tool) {
        if self.pending_install.is_some() {
            self.set_status("An install is already running");
            return;
        }
        self.pending_install = Some(tool);
        self.background.install_tool(tool);
        self.set_status(format!("Installing {}…", tool.label()));
    }

    /// Offer project setup once per workspace, when there is something to say.
    ///
    /// Called from the interactive event loop, so it never blocks the first
    /// frame and never appears when Koda is embedded (tests, previews). If the
    /// project needs nothing the flag is still set, so the offer is not repeated
    /// on every iteration.
    pub(super) fn maybe_offer_project_setup(&mut self) -> bool {
        if !self.overlay.is_none()
            || self.pending_install.is_some()
            || self.project_setup.is_some()
            || self.project_setup_offered
            || !self.engaged
        {
            return false;
        }
        let Some(languages) = self.project_languages.clone() else {
            return false; // the scan has not finished
        };
        if languages.is_empty() {
            self.project_setup_offered = true;
            return false;
        }
        let Some(tools) = self.tools.clone() else {
            return false; // wait for the tool probe
        };
        let plan = plan_project(&languages, &tools);
        self.project_setup_offered = true;
        if !plan.needs_setup() && !plan.has_attention() {
            return false; // every language is ready: nothing to report
        }
        self.show_project_setup(plan);
        true
    }

    /// Open the project setup summary on demand (palette, or after "Later").
    pub(super) fn open_project_setup(&mut self) {
        let Some(tools) = self.tools.clone() else {
            self.background.discover_tools();
            self.set_status("Checking language tools…");
            return;
        };
        let languages = self
            .project_languages
            .clone()
            .unwrap_or_else(|| self.fallback_languages());
        if languages.is_empty() {
            self.set_status("No project languages detected");
            return;
        }
        let plan = plan_project(&languages, &tools);
        self.show_project_setup(plan);
    }

    /// The languages to plan for when the project scan has not run yet: the
    /// active file's language plus the detected project kind.
    fn fallback_languages(&self) -> Vec<LanguageId> {
        let mut languages = Vec::new();
        if let Some(language) = self
            .editor
            .active_document()
            .map(|doc| doc.buffer.language)
            .filter(|language| *language != LanguageId::Unknown)
        {
            languages.push(language);
        }
        let project = self.workspace.project.kind.language();
        if project != LanguageId::Unknown && !languages.contains(&project) {
            languages.push(project);
        }
        languages
    }

    /// Show the setup summary as a picker: one row per language, then the
    /// action rows.
    pub(super) fn show_project_setup(&mut self, plan: ProjectSetupPlan) {
        let mut items = Vec::new();
        for entry in &plan.languages {
            let tag = match &entry.state {
                SetupState::Ready => "ready".to_string(),
                SetupState::NeedsInstall(_) => "install".to_string(),
                SetupState::Prerequisite(program) => format!("needs {program}"),
                SetupState::Unavailable(_) => "unavailable".to_string(),
            };
            let label = format!("{} · {tag}", entry.language.name());
            items.push(PickerItem::new(
                label,
                entry.detail.clone(),
                PickerAction::Info(entry.detail.clone()),
            ));
        }

        let missing = plan.installable_tools();
        if !missing.is_empty() {
            let noun = if missing.len() == 1 { "tool" } else { "tools" };
            items.push(
                PickerItem::new(
                    "Set up project",
                    format!("Install {} missing language {noun}", missing.len()),
                    PickerAction::ProjectSetup,
                )
                .shortcut("Enter"),
            );
        }
        items.push(PickerItem::new(
            "Later",
            "Set up this project from the command palette another time",
            PickerAction::Info(
                "Project setup postponed — run “Set Up Project…” any time".to_string(),
            ),
        ));

        let mut picker = Picker::new("Project setup", "Choose…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Begin provisioning every missing tool the project needs.
    ///
    /// Installs run one at a time through the existing provisioning path; the
    /// queue is planned from the current tool state, so a ready server is never
    /// reinstalled.
    pub(super) fn start_project_setup(&mut self) {
        if self.pending_install.is_some() || self.project_setup.is_some() {
            self.set_status("An install is already running");
            return;
        }
        let Some(languages) = self.project_languages.clone() else {
            self.set_status("No project languages detected");
            return;
        };
        let Some(tools) = self.tools.clone() else {
            self.background.discover_tools();
            self.set_status("Checking language tools…");
            return;
        };
        let plan = plan_project(&languages, &tools);
        let queue: std::collections::VecDeque<Tool> = plan.installable_tools().into();
        if queue.is_empty() {
            self.set_status("Project is already set up");
            return;
        }
        self.project_setup = Some(ProjectSetupRun {
            plan,
            queue,
            installed: Vec::new(),
            failed: Vec::new(),
        });
        self.advance_project_setup();
    }

    /// Start the next queued install, or finish the run.
    pub(super) fn advance_project_setup(&mut self) {
        let next = self
            .project_setup
            .as_mut()
            .and_then(|run| run.queue.pop_front());
        match next {
            Some(tool) => {
                self.pending_install = Some(tool);
                self.background.install_tool(tool);
                self.set_status(format!("Setting up project — installing {}…", tool.label()));
            }
            None => {
                if self.project_setup.is_some() {
                    self.finish_project_setup();
                }
            }
        }
    }

    /// Report the outcome of a project setup run, distinguishing full success,
    /// partial success and outstanding attention.
    fn finish_project_setup(&mut self) {
        let Some(run) = self.project_setup.take() else {
            return;
        };
        let failed: Vec<Tool> = run.failed.iter().map(|(tool, _)| *tool).collect();
        let summary = SetupSummary::from_run(&run.plan, &run.installed, &failed);
        let headline = summary.headline();
        let kind = if summary.failed == 0 {
            ToastKind::Success
        } else {
            ToastKind::Error
        };
        self.set_status(headline.clone());
        self.push_toast(kind, headline);
        // Re-probe so Language Setup and the next plan reflect the new state.
        self.background.discover_tools();
    }

    /// Offer, once per language, to install a missing language server.
    ///
    /// This is called from the interactive event loop rather than at startup, so
    /// the prompt never blocks the first frame and never appears when Koda is
    /// embedded (tests, previews). The user always chooses; dismissing it keeps
    /// Koda's built-in intelligence, and **Language Setup…** stays available.
    pub(super) fn maybe_offer_tool_setup(&mut self) -> bool {
        if !self.overlay.is_none()
            || self.pending_install.is_some()
            || self.project_setup.is_some()
            || !self.lsp.is_empty()
        {
            return false;
        }
        let Some(tools) = self.tools.as_ref() else {
            return false;
        };
        // Prefer the active file's language; with no file open, use the
        // project's detected language so opening a project is enough to be
        // offered its tooling.
        let language = self
            .editor
            .active_document()
            .map(|doc| doc.buffer.language)
            .filter(|language| *language != LanguageId::Unknown)
            .unwrap_or_else(|| self.workspace.project.kind.language());
        if language == LanguageId::Unknown {
            return false;
        }
        // Project setup is the primary entry point for a project's languages, so
        // do not also prompt for a language the project plan already covers.
        if self
            .project_languages
            .as_ref()
            .is_some_and(|languages| languages.contains(&language))
        {
            return false;
        }
        // An available server means there is nothing to offer; otherwise offer
        // the first installable candidate.
        if Tool::available_server(language, tools).is_some() {
            return false;
        }
        let Some(tool) = Tool::installable_server(language, tools) else {
            return false;
        };
        if !self.setup_offered.insert(language) {
            return false;
        }

        let install = PickerItem::new(
            format!("Install {}", tool.label()),
            tool.setup_reason(),
            PickerAction::InstallTool(tool),
        )
        .shortcut("Enter");
        let later = PickerItem::new(
            "Not now",
            "Keep Koda's built-in intelligence — install later from Language Setup…",
            PickerAction::Info("Install language tools any time from Language Setup…".to_string()),
        );
        let mut picker = Picker::new(
            format!("{} is not installed", tool.label()),
            "Choose…",
            vec![install, later],
        );
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
        true
    }

    /// Offer Update and Remove for a Koda-managed tool.
    pub(super) fn tool_actions(&mut self, tool: Tool) {
        let Some(status) = self
            .tools
            .as_ref()
            .and_then(|tools| tools.status(tool))
            .cloned()
        else {
            self.set_status("Tool status is unavailable");
            return;
        };
        if !status.is_managed() {
            self.set_status(format!("{} is not managed by Koda", tool.label()));
            return;
        }
        let size = status
            .managed_size()
            .map(|bytes| format!(" ({})", crate::language::tools::human_bytes(bytes)))
            .unwrap_or_default();
        let update = PickerItem::new(
            format!("Update {}", tool.label()),
            format!("Reinstall Koda's pinned version{size}"),
            PickerAction::UpdateTool(tool),
        )
        .shortcut("Enter");
        let remove = PickerItem::new(
            format!("Remove {}", tool.label()),
            format!("Delete Koda's managed files{size}"),
            PickerAction::RemoveTool(tool),
        );
        let cancel = PickerItem::new(
            "Cancel",
            "Change nothing",
            PickerAction::Info("Nothing changed".to_string()),
        );
        let mut picker = Picker::new(
            format!("{} (Koda-managed)", tool.label()),
            "Choose…",
            vec![update, remove, cancel],
        );
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Remove a Koda-managed tool's files.
    ///
    /// Refuses to touch anything Koda does not own, and never removes the shared
    /// tools root itself. A `discover` afterwards refreshes the setup view.
    pub(super) fn remove_tool(&mut self, tool: Tool) {
        if self.pending_install.is_some() {
            self.set_status("An install is already running");
            return;
        }
        let Some(status) = self
            .tools
            .as_ref()
            .and_then(|tools| tools.status(tool))
            .cloned()
        else {
            self.set_status("Tool status is unavailable");
            return;
        };
        if !status.is_managed() {
            self.set_error(format!(
                "{} is not managed by Koda — not removing it",
                tool.label()
            ));
            return;
        }
        let Some(dir) = status.managed_dir() else {
            self.set_error(format!("Could not locate {}'s files", tool.label()));
            return;
        };
        if crate::language::tools::tools_dir().is_some_and(|root| dir == root) {
            self.set_error("Refusing to remove the shared tools directory");
            return;
        }
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {
                self.push_toast(ToastKind::Info, format!("Removed {}", tool.label()));
                self.background.discover_tools();
            }
            Err(err) => self.set_error(format!("Could not remove {}: {err}", tool.label())),
        }
    }

    /// Whether a command can run right now, and why not when it cannot.
    pub(super) fn command_availability(&self, command: &Command) -> (bool, Option<String>) {
        let document = self.editor.active_document();
        if command.needs_doc && document.is_none() {
            return (false, Some("no file open".to_string()));
        }
        if let Some(capability) = command.capability {
            let language = document
                .map(|doc| doc.buffer.language)
                .unwrap_or(LanguageId::Unknown);
            if !self
                .language
                .provider(language)
                .capabilities()
                .contains(&capability)
            {
                return (
                    false,
                    Some(format!("not available for {}", language.name())),
                );
            }
        }
        match command.id {
            ids::UNDO if !document.is_some_and(|doc| doc.can_undo()) => {
                (false, Some("nothing to undo".to_string()))
            }
            ids::REDO if !document.is_some_and(|doc| doc.can_redo()) => {
                (false, Some("nothing to redo".to_string()))
            }
            ids::COPY | ids::CUT if !document.is_some_and(|doc| doc.has_selection()) => {
                (false, Some("nothing selected".to_string()))
            }
            ids::YANK_POP if self.last_yank.is_none() => {
                (false, Some("nothing to yank".to_string()))
            }
            ids::REPLACE_ALL if !self.search.open || self.search.query.is_empty() => {
                (false, Some("open find first".to_string()))
            }
            ids::NEXT_TAB | ids::PREV_TAB if self.editor.len() < 2 => {
                (false, Some("only one tab".to_string()))
            }
            ids::CLOSE_TAB | ids::CLOSE_ALL | ids::SAVE_ALL if self.editor.is_empty() => {
                (false, Some("no files open".to_string()))
            }
            ids::REVERT if !document.is_some_and(|doc| doc.is_dirty()) => {
                (false, Some("no changes to revert".to_string()))
            }
            ids::RENAME_FILE | ids::DELETE_FILE if self.file_op_target().is_none() => {
                (false, Some("select a file first".to_string()))
            }
            ids::DUPLICATE_FILE | ids::COPY_FILE if self.file_op_target().is_none() => {
                (false, Some("select a file first".to_string()))
            }
            ids::GIT_COMMIT if !self.workspace.git.available => {
                (false, Some("not a git repository".to_string()))
            }
            ids::GIT_COMMIT if self.workspace.git.files.is_empty() => {
                (false, Some("nothing to commit".to_string()))
            }
            ids::GIT_TOGGLE_STAGE if !self.workspace.git.available => {
                (false, Some("not a git repository".to_string()))
            }
            ids::GIT_TOGGLE_STAGE if self.file_op_target().is_none() => {
                (false, Some("select a file first".to_string()))
            }
            ids::DIAGNOSTICS_NEXT | ids::DIAGNOSTICS_PREV
                if !document.is_some_and(|doc| !doc.diagnostics().is_empty()) =>
            {
                (false, Some("no diagnostics".to_string()))
            }
            ids::DIAGNOSTICS_LIST
                if !self
                    .editor
                    .documents
                    .iter()
                    .any(|doc| !doc.diagnostics().is_empty()) =>
            {
                (false, Some("no diagnostics".to_string()))
            }
            ids::FORMAT => self.format_availability(document),
            ids::RENAME if !self.lsp_supports(RequestKind::Rename) => {
                (false, Some("needs a language server".to_string()))
            }
            ids::CODE_ACTIONS if !self.lsp_supports(RequestKind::CodeActions) => {
                (false, Some("needs a language server".to_string()))
            }
            _ => (true, None),
        }
    }

    /// Whether formatting can run, and why not when it cannot.
    pub(super) fn format_availability(
        &self,
        document: Option<&Document>,
    ) -> (bool, Option<String>) {
        // A language server may format even when the provider has no formatter.
        if self.lsp_supports(RequestKind::Formatting) {
            return (true, None);
        }
        let language = document
            .map(|doc| doc.buffer.language)
            .unwrap_or(LanguageId::Unknown);
        let Some(tool) =
            Tool::for_language(language, crate::language::tools::ToolPurpose::Formatter)
        else {
            return (true, None);
        };
        // Prefer the probed registry; fall back to a PATH check before it lands.
        let available = match &self.tools {
            Some(tools) => tools.available(tool),
            None => format::is_available(tool.program()),
        };
        if available {
            (true, None)
        } else {
            (false, Some(format!("{} is not installed", tool.program())))
        }
    }

    /// Show which language tools Koda found, and how to install the rest.
    pub(super) fn language_setup(&mut self) {
        let Some(tools) = self.tools.clone() else {
            // Discovery is still running; ask again and tell the user.
            self.background.discover_tools();
            self.set_status("Checking language tools…");
            return;
        };

        let mut items = Vec::new();
        for status in tools.all() {
            let tool = status.tool;
            let purpose = match tool.purpose() {
                crate::language::tools::ToolPurpose::LanguageServer => "language server",
                crate::language::tools::ToolPurpose::Formatter => "formatter",
            };
            let label = format!("{}  ·  {} {purpose}", tool.label(), tool.language().name());
            let item = if status.available {
                // A managed tool shows its version and on-disk size, and can be
                // updated or removed. A user/system install is read-only.
                let detail = match status.managed_size() {
                    Some(bytes) => format!(
                        "{} · {}{}",
                        status.summary(),
                        crate::language::tools::human_bytes(bytes),
                        if status.is_managed() {
                            " · Koda-managed"
                        } else {
                            ""
                        }
                    ),
                    None => status.summary(),
                };
                if status.is_managed() {
                    PickerItem::new(label, detail, PickerAction::ToolActions(tool))
                        .shortcut("Enter")
                } else {
                    PickerItem::new(label, detail.clone(), PickerAction::Info(detail))
                }
            } else if crate::language::tools::can_install(tool) {
                let hint = tool.setup_reason();
                PickerItem::new(label, hint, PickerAction::InstallTool(tool)).shortcut("Enter")
            } else {
                // Explain exactly why Koda cannot install it, and what the user
                // can do instead, rather than repeating the generic install hint.
                let reason = tool.setup_reason();
                PickerItem::new(label, reason.clone(), PickerAction::Info(reason.clone()))
                    .disabled(reason)
            };
            items.push(item);
        }
        let mut picker = Picker::new("Language Setup", "Language tools…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }
    pub(super) fn open_prompt(&mut self, kind: PromptKind, label: &str, placeholder: &str) {
        self.overlay = Overlay::Prompt(Prompt::new(kind, label, placeholder));
    }

    pub(super) fn toggle_tree(&mut self) {
        // `Ctrl+B` is the file panel: reveal and focus it, focus it when the
        // editor has focus, and hide it once it is focused.
        if !self.tree_visible {
            self.tree_visible = true;
            self.focus = Focus::FileTree;
        } else if self.focus == Focus::Editor {
            self.focus = Focus::FileTree;
        } else {
            self.tree_visible = false;
            self.focus = Focus::Editor;
        }
    }

    pub(super) fn open_quick_open(&mut self) {
        let root = self.workspace.root().to_path_buf();
        let files = filesystem::collect_files(&root, 8000);

        // Recently-opened files come first, so Ctrl+P then Enter reopens the last
        // file without typing.
        let mut ordered: Vec<PathBuf> = Vec::new();
        let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
        for path in self.recent_files.iter().rev() {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !seen.insert(canonical) {
                continue;
            }
            if path.is_file() {
                ordered.push(path.clone());
            }
        }
        for path in files {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if seen.insert(canonical) {
                ordered.push(path);
            }
        }

        let items = ordered
            .into_iter()
            .map(|path| {
                let label = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let detail = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                PickerItem::new(label, detail, PickerAction::OpenPath(path))
            })
            .collect();
        let mut picker = Picker::new("Quick Open", "Type a file name…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Show the keyboard-shortcuts cheatsheet, or hide it if it is already up.
    pub(super) fn toggle_help(&mut self) {
        self.completion = None;
        self.hover = None;
        self.overlay = match self.overlay {
            Overlay::Help(_) => Overlay::None,
            _ => Overlay::Help(Help::default()),
        };
    }

    /// Re-read the project tree and git status on demand.
    pub(super) fn refresh_workspace(&mut self) {
        self.workspace.refresh();
        self.request_git_refresh();
        self.set_status("Refreshed");
    }
}
