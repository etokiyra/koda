//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

impl App {
    /// Open a file, focusing the editor and applying language detection.
    pub fn open_path(&mut self, path: PathBuf) {
        self.signature = None;
        self.signature_request = None;
        match self.editor.open_path(&path) {
            Ok(index) => {
                // The focused pane adopts the newly opened document.
                self.set_active_pane_index(index);
                self.sync_active_pane();
                self.request_detection_for_active();
                self.workspace.tree.select_path(&path);
                self.focus = Focus::Editor;
                self.close_armed = None;
                self.engaged = true;
                self.remember_recent(&path);
                self.set_status(format!("Opened {}", path.display()));
            }
            Err(err) => self.set_error(format!("Could not open {}: {err}", path.display())),
        }
    }

    /// Persist the session for the active project. Errors are non-fatal.
    ///
    /// Only writes once the user has engaged with a project, so starting Koda
    /// and quitting without opening anything does not overwrite an untouched
    /// session.
    pub fn save_session(&self) {
        if !self.engaged {
            return;
        }
        let Some(path) = crate::session::session_path(self.workspace.root()) else {
            return;
        };
        let _ = self.capture_session().save_to(&path);
    }

    /// Clear per-project state when the workspace changes.
    pub(super) fn reset_for_new_workspace(&mut self) {
        self.editor = Editor::new();
        self.split = false;
        self.pane_left = 0;
        self.pane_right = None;
        self.focus_pane = Pane::Primary;
        self.focus = Focus::Editor;
        self.overlay = Overlay::None;
        self.search = Search::default();
        self.tree_filter = None;
        self.completion = None;
        self.hover = None;
        self.cursor_screen = None;
        self.close_armed = None;
        self.diagnostics_dirty_at = None;
        self.pending_format = None;
        self.pending_workspace_symbols = None;
        self.pending_workspace_symbols_query = None;
        self.pending_project_search = None;
        self.pending_rename = None;
        self.pending_code_actions.clear();
        self.lsp.clear();
        self.welcome_target = None;
        self.welcome_selected = 0;
        self.project_languages = None;
        self.project_scan_started = false;
        self.project_setup_offered = false;
        self.project_setup = None;
    }

    /// Snapshot the session worth restoring for the active project.
    pub(super) fn capture_session(&self) -> Session {
        let mut files = Vec::new();
        let mut cursors = Vec::new();
        for doc in &self.editor.documents {
            if let Some(path) = &doc.buffer.path {
                files.push(path.clone());
                cursors.push((doc.cursor.row, doc.cursor.col));
            }
        }
        let active = self
            .editor
            .active_document()
            .and_then(|doc| doc.buffer.path.clone())
            .and_then(|path| files.iter().position(|file| *file == path))
            .unwrap_or(0);
        Session {
            files,
            active,
            cursors,
            expanded: self.workspace.tree.expanded_paths(),
            show_hidden: self.workspace.tree.show_hidden,
        }
    }

    /// Restore a saved session into the current workspace.
    pub(super) fn restore_session(&mut self, session: Session) {
        self.engaged = true;
        if session.show_hidden && !self.workspace.tree.show_hidden {
            self.workspace.tree.toggle_hidden();
        }
        self.workspace.tree.set_expanded(&session.expanded);

        for (index, path) in session.files.iter().enumerate() {
            if !path.is_file() {
                continue;
            }
            self.open_path(path.clone());
            if let Some(&(row, col)) = session.cursors.get(index)
                && let Some(doc) = self.editor.active_document_mut()
            {
                doc.move_to(Position::new(row, col));
            }
        }

        if !self.editor.is_empty() {
            let active = session.active.min(self.editor.len().saturating_sub(1));
            self.editor.set_active(active);
            self.pane_left = active;
            self.pane_right = None;
            self.split = false;
            self.focus_pane = Pane::Primary;
            self.request_detection_for_active();
            self.set_status(format!("Restored {} file(s)", self.editor.len()));
        }
    }

    /// Replace the workspace with the project rooted at `root`.
    ///
    /// Returns `false` when the workspace could not be opened (an error status
    /// is set).
    pub fn open_workspace(&mut self, root: PathBuf) -> bool {
        // Persist the current project before leaving it.
        self.save_session();
        let workspace = match Workspace::open(Some(&root)) {
            Ok(workspace) => workspace,
            Err(err) => {
                self.set_error(format!("Could not open the project: {err}"));
                return false;
            }
        };
        self.workspace = workspace;
        self.reset_for_new_workspace();
        self.recent.add_project(self.workspace.root());
        self.engaged = true;
        self.resume_session = crate::session::session_path(self.workspace.root())
            .and_then(|path| Session::load_from(&path));
        self.request_git_refresh();
        self.background.discover_tools();
        true
    }

    /// Return to the welcome screen by closing every tab.
    pub(super) fn go_home(&mut self) {
        if self.editor.has_unsaved() {
            self.set_error("Unsaved changes — save first (Ctrl+S)");
            return;
        }
        self.editor.close_all();
        self.pane_left = 0;
        self.pane_right = None;
        self.split = false;
        self.focus_pane = Pane::Primary;
        self.welcome_selected = 0;
        self.focus = Focus::Editor;
        self.set_status("Welcome");
    }

    pub(super) fn remember_recent(&mut self, path: &Path) {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.recent_files.retain(|existing| {
            existing.canonicalize().unwrap_or_else(|_| existing.clone()) != canonical
        });
        self.recent_files.push(canonical.clone());
        if self.recent_files.len() > 20 {
            self.recent_files.remove(0);
        }
        if canonical.is_file() {
            self.recent.add_file(&canonical);
        }
    }
    /// Send a create-project request to the background worker.
    pub(super) fn start_project_creation(
        &mut self,
        parent: PathBuf,
        name: String,
        language: LanguageId,
    ) {
        if self.pending_project {
            self.set_status("A project is already being created");
            return;
        }
        self.pending_project = true;
        self.background
            .create_project(parent, name.clone(), language);
        self.set_status(format!("Creating {name}…"));
    }

    /// Whether the welcome screen is the active surface.
    pub fn welcome_active(&self) -> bool {
        self.editor.is_empty() && self.overlay.is_none() && !self.search.open
    }

    /// Begin the guided "Create a new project" flow.
    pub(super) fn open_new_project(&mut self) {
        self.overlay = Overlay::NewProject(NewProject::new(&self.welcome_start_dir()));
    }

    pub(super) fn activate_welcome(&mut self, action: WelcomeAction) {
        match action {
            WelcomeAction::OpenFile => self.execute_command(ids::OPEN),
            WelcomeAction::OpenProject => self.open_dir_picker(),
            WelcomeAction::NewProject => self.open_new_project(),
            WelcomeAction::Resume => {
                if let Some(session) = self.resume_session.take() {
                    self.restore_session(session);
                }
                self.welcome_selected = 0;
            }
            WelcomeAction::OpenTarget(path) => self.open_target(path),
            WelcomeAction::OpenRecentProject(path) => self.open_project(path),
            WelcomeAction::OpenRecentFile(path) => self.open_recent_file(path),
            WelcomeAction::Help => self.toggle_help(),
        }
    }

    /// Where a picker should start browsing.
    pub(super) fn welcome_start_dir(&self) -> PathBuf {
        if let Some(directory) = self
            .editor
            .active_document()
            .and_then(|doc| doc.buffer.path.as_ref())
            .and_then(|path| path.parent())
            .filter(|parent| parent.is_dir())
        {
            return directory.to_path_buf();
        }
        std::env::current_dir().unwrap_or_else(|_| self.workspace.root().to_path_buf())
    }

    /// Begin the "Open Project…" directory picker.
    pub(super) fn open_dir_picker(&mut self) {
        self.overlay = Overlay::DirPicker(DirPicker::new(&self.welcome_start_dir()));
    }

    /// Open a path from the command line: a file directly, a directory as a
    /// project.
    pub(super) fn open_target(&mut self, path: PathBuf) {
        if path.is_file() {
            self.open_recent_file(path);
        } else if path.is_dir() {
            self.open_project(path);
        } else {
            self.set_error(format!("{} no longer exists", path.display()));
        }
    }

    /// Open a project directory, restoring its session when one is available.
    pub(super) fn open_project(&mut self, root: PathBuf) {
        if !self.open_workspace(root) {
            return;
        }
        if let Some(session) = self.resume_session.take() {
            self.restore_session(session);
        } else {
            let name = self
                .workspace
                .root()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("project")
                .to_string();
            self.set_status(format!("Opened project {name}"));
        }
    }

    /// Open `path`, switching the workspace first if it belongs to another
    /// project.
    pub(super) fn open_recent_file(&mut self, path: PathBuf) {
        if !path.is_file() {
            self.set_error(format!("{} no longer exists", path.display()));
            return;
        }
        let root = crate::project::Project::detect(&path).root;
        let root = root.canonicalize().unwrap_or(root);
        if root != self.workspace.root() && !self.open_workspace(root) {
            return;
        }
        self.open_path(path);
    }

    pub(super) fn handle_welcome_key(&mut self, key: KeyEvent) {
        let count = self.welcome_items().len();
        match key.code {
            KeyCode::Up => {
                if self.welcome_selected > 0 {
                    self.welcome_selected -= 1;
                }
            }
            KeyCode::Down => {
                if self.welcome_selected + 1 < count {
                    self.welcome_selected += 1;
                }
            }
            KeyCode::Home => self.welcome_selected = 0,
            KeyCode::End => self.welcome_selected = count.saturating_sub(1),
            KeyCode::Char('v') | KeyCode::Char('V') => {
                self.welcome_scene = self.welcome_scene.next();
            }
            KeyCode::Enter => {
                if let Some(item) = self.welcome_items().into_iter().nth(self.welcome_selected) {
                    self.activate_welcome(item.action);
                }
            }
            _ => {}
        }
    }

    /// The actions offered on the welcome screen, best first.
    pub fn welcome_items(&self) -> Vec<WelcomeItem> {
        let mut items = vec![
            WelcomeItem {
                label: "Open a file…".to_string(),
                detail: "Ctrl+O".to_string(),
                action: WelcomeAction::OpenFile,
            },
            WelcomeItem {
                label: "Open a project…".to_string(),
                detail: "choose a folder".to_string(),
                action: WelcomeAction::OpenProject,
            },
            WelcomeItem {
                label: "Create a new project…".to_string(),
                detail: "Rust · Go · Python · C++".to_string(),
                action: WelcomeAction::NewProject,
            },
        ];

        if let Some(session) = &self.resume_session {
            let name = self
                .workspace
                .root()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("project");
            items.push(WelcomeItem {
                label: format!("Resume “{name}”"),
                detail: format!("{} file(s)", session.files.len()),
                action: WelcomeAction::Resume,
            });
        }

        if let Some(target) = &self.welcome_target {
            let is_workspace = target == self.workspace.root();
            if target.is_file() || !is_workspace {
                let verb = if target.is_file() {
                    "Open"
                } else {
                    "Open project"
                };
                items.push(WelcomeItem {
                    label: format!("{verb} {}", display_path(target)),
                    detail: "from the command line".to_string(),
                    action: WelcomeAction::OpenTarget(target.clone()),
                });
            }
        }

        for project in &self.recent.projects {
            if project == self.workspace.root() || !project.is_dir() {
                continue;
            }
            items.push(WelcomeItem {
                label: format!("Project {}", display_path(project)),
                detail: String::new(),
                action: WelcomeAction::OpenRecentProject(project.clone()),
            });
        }

        for file in &self.recent.files {
            if !file.is_file() {
                continue;
            }
            items.push(WelcomeItem {
                label: format!("File {}", display_path(file)),
                detail: String::new(),
                action: WelcomeAction::OpenRecentFile(file.clone()),
            });
        }

        items.push(WelcomeItem {
            label: "Keyboard shortcuts".to_string(),
            detail: "F1".to_string(),
            action: WelcomeAction::Help,
        });
        items
    }
}
