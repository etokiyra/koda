//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

impl App {
    pub(super) fn save_all(&mut self) {
        let mut saved = 0usize;
        let mut error = None;
        for doc in &mut self.editor.documents {
            if doc.is_dirty() && doc.buffer.path.is_some() {
                match doc.save() {
                    Ok(true) => saved += 1,
                    Ok(false) => {}
                    Err(err) => {
                        error = Some(err);
                        break;
                    }
                }
            }
        }
        if let Some(err) = error {
            self.set_error(format!("Save failed: {err}"));
            return;
        }
        if saved > 0 {
            self.request_git_refresh();
            self.quit_armed = false;
            self.set_status(format!("Saved {saved} file(s)"));
        } else {
            self.set_status("Nothing to save");
        }
    }

    pub(super) fn close_all(&mut self) {
        if self.editor.has_unsaved() {
            self.set_error("Unsaved changes — save first (Ctrl+S)");
            return;
        }
        let closed: Vec<(PathBuf, LanguageId)> = self
            .editor
            .documents
            .iter()
            .filter_map(|doc| doc.buffer.path.clone().map(|p| (p, doc.buffer.language)))
            .collect();
        self.editor.close_all();
        for (path, language) in closed {
            self.notify_lsp_close(&path, language);
        }
        self.pane_left = 0;
        self.pane_right = None;
        self.split = false;
        self.focus_pane = Pane::Primary;
        self.close_armed = None;
        self.set_status("All tabs closed");
    }

    /// Discard the active file's edits and reload it from disk.
    pub(super) fn revert_file(&mut self) {
        let Some(doc) = self.editor.active_document_mut() else {
            return;
        };
        if !doc.is_dirty() {
            self.set_status("No changes to revert");
            return;
        }
        match doc.reload_from_disk() {
            Ok(true) => {
                self.close_armed = None;
                self.after_edit();
                self.set_status("Reverted to the version on disk");
            }
            Ok(false) => self.set_status("This file is not on disk"),
            Err(err) => self.set_error(format!("Revert failed: {err}")),
        }
    }

    /// Save the active document, if it has a path.
    pub(super) fn save(&mut self) {
        let has_path = self
            .editor
            .active_document()
            .map(|doc| doc.buffer.path.is_some())
            .unwrap_or(false);
        if !has_path {
            self.open_prompt(PromptKind::SaveAs, "Save as", "path/to/file");
            return;
        }
        match self.editor.save_active() {
            Ok(true) => {
                self.request_git_refresh();
                self.quit_armed = false;
                self.set_status("Saved");
            }
            Ok(false) => self.set_status("Nothing to save"),
            Err(err) => self.set_error(format!("Save failed: {err}")),
        }
    }

    pub(super) fn close_tab(&mut self) {
        let index = self.editor.active_index();
        let dirty = self
            .editor
            .documents
            .get(index)
            .map(|doc| doc.is_dirty())
            .unwrap_or(false);
        if dirty && self.close_armed != Some(index) {
            self.close_armed = Some(index);
            self.set_error("Unsaved changes — press Ctrl+W again to close without saving");
            return;
        }
        self.close_armed = None;
        let closed = self
            .editor
            .documents
            .get(index)
            .and_then(|doc| doc.buffer.path.clone().map(|p| (p, doc.buffer.language)));
        self.editor.close(index);
        if let Some((path, language)) = closed {
            self.notify_lsp_close(&path, language);
        }
        self.remap_pane_indices(index);
        self.set_status("Tab closed");
    }

    pub(super) fn save_as(&mut self, path: PathBuf) {
        let result = self
            .editor
            .active_document_mut()
            .map(|doc| doc.save_as(&path));
        match result {
            Some(Ok(true)) => {
                self.request_detection_for_active();
                self.request_git_refresh();
                self.set_status(format!("Saved {}", path.display()));
            }
            Some(Err(err)) => self.set_error(format!("Save failed: {err}")),
            _ => self.set_error("Nothing to save"),
        }
    }
}
