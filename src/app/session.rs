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
}
