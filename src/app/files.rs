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
    /// Duplicate the selected file with a `copy` suffix and open it.
    pub(super) fn duplicate_file(&mut self) {
        let Some(source) = self.file_op_target() else {
            self.set_status("Select a file to duplicate");
            return;
        };
        if source.is_dir() {
            self.set_status("Folders cannot be duplicated yet");
            return;
        }
        let destination = unique_copy_path(&source);
        match filesystem::copy_file(&source, &destination) {
            Ok(()) => {
                self.workspace.tree.refresh();
                self.workspace.tree.select_path(&destination);
                self.open_path(destination);
            }
            Err(err) => self.set_error(format!("Duplicate failed: {err}")),
        }
    }

    pub(super) fn delete_selected(&mut self) {
        let Some(path) = self.file_op_target() else {
            self.set_status("Nothing to delete");
            return;
        };
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        let kind = if path.is_dir() { "folder" } else { "file" };
        let delete = PickerItem::new(
            format!("Delete {kind}"),
            format!("Permanently remove {name}"),
            PickerAction::DeletePath(path),
        )
        .shortcut("Enter");
        let cancel = PickerItem::new(
            "Cancel",
            "Keep it",
            PickerAction::Info("Nothing deleted".to_string()),
        );
        let mut picker = Picker::new(format!("Delete {name}?"), "Choose…", vec![delete, cancel]);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// The file or folder a rename or delete should act on.
    pub(super) fn file_op_target(&self) -> Option<PathBuf> {
        if self.focus == Focus::FileTree
            && let Some(entry) = self.workspace.tree.selected_entry()
        {
            return Some(entry.path.clone());
        }
        if let Some(path) = self
            .editor
            .active_document()
            .and_then(|doc| doc.buffer.path.clone())
        {
            return Some(path);
        }
        self.workspace
            .tree
            .selected_entry()
            .map(|entry| entry.path.clone())
    }

    /// Update open documents and recent files after a rename or move.
    pub(super) fn after_file_rename(&mut self, old: &Path, new: &Path) {
        for doc in &mut self.editor.documents {
            let Some(path) = doc.buffer.path.clone() else {
                continue;
            };
            if path == old {
                doc.set_path(new.to_path_buf());
            } else if let Ok(relative) = path.strip_prefix(old) {
                doc.set_path(new.join(relative));
            }
        }
        self.recent_files = std::mem::take(&mut self.recent_files)
            .into_iter()
            .map(|path| {
                if path == old {
                    new.to_path_buf()
                } else if let Ok(relative) = path.strip_prefix(old) {
                    new.join(relative)
                } else {
                    path
                }
            })
            .collect();
        self.workspace.tree.refresh();
        self.workspace.tree.select_path(new);
        self.close_armed = None;
        self.request_detection_for_active();
    }

    /// Prompt for a destination and copy the selected file there.
    pub(super) fn copy_file(&mut self) {
        let Some(source) = self.file_op_target() else {
            self.set_status("Select a file to copy");
            return;
        };
        if source.is_dir() {
            self.set_status("Folders cannot be copied yet");
            return;
        }
        let placeholder = source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("copy")
            .to_string();
        self.pending_copy_file = Some(source);
        self.open_prompt(PromptKind::CopyFile, "Copy file", &placeholder);
    }

    /// Delete `path` (a file or a whole directory), closing affected buffers.
    pub(super) fn delete_path(&mut self, path: &Path) {
        let modified = self.editor.documents.iter().any(|doc| {
            doc.is_dirty()
                && doc
                    .buffer
                    .path
                    .as_deref()
                    .is_some_and(|candidate| candidate.starts_with(path))
        });
        if modified {
            self.set_error("Save or close modified files before deleting");
            return;
        }
        match filesystem::remove_path(path) {
            Ok(()) => {
                let mut closed: Vec<(PathBuf, LanguageId)> = Vec::new();
                let mut index = self.editor.documents.len();
                while index > 0 {
                    index -= 1;
                    let matches = self.editor.documents[index]
                        .buffer
                        .path
                        .as_deref()
                        .is_some_and(|candidate| candidate.starts_with(path));
                    if matches {
                        if let Some(doc) = self.editor.documents.get(index)
                            && let Some(closed_path) = doc.buffer.path.clone()
                        {
                            closed.push((closed_path, doc.buffer.language));
                        }
                        self.editor.close(index);
                    }
                }
                for (closed_path, language) in closed {
                    self.notify_lsp_close(&closed_path, language);
                }
                self.clamp_panes();
                self.recent_files
                    .retain(|candidate| !candidate.starts_with(path));
                self.workspace.tree.refresh();
                self.close_armed = None;
                self.set_status(format!("Deleted {}", path.display()));
            }
            Err(err) => self.set_error(format!("Delete failed: {err}")),
        }
    }

    /// The directory a new file should be created in: the selected folder when
    /// the tree has focus, otherwise the active file's folder or the root.
    pub(super) fn new_file_dir(&self) -> PathBuf {
        if self.focus == Focus::FileTree
            && let Some(entry) = self.workspace.tree.selected_entry()
        {
            return if entry.is_dir {
                entry.path.clone()
            } else {
                entry
                    .path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| self.workspace.root().to_path_buf())
            };
        }
        self.editor
            .active_document()
            .and_then(|doc| doc.buffer.path.as_ref())
            .and_then(|path| path.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.workspace.root().to_path_buf())
    }

    pub(super) fn new_file(&mut self) {
        let dir = self.new_file_dir();
        let label = match dir.strip_prefix(self.workspace.root()) {
            Ok(relative) if !relative.as_os_str().is_empty() => {
                format!("New file in {}", relative.display())
            }
            _ => "New file".to_string(),
        };
        self.open_prompt(PromptKind::NewFile, &label, "name.rs");
    }

    pub(super) fn rename_selected(&mut self) {
        let Some(path) = self.file_op_target() else {
            self.set_status("Nothing to rename");
            return;
        };
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        self.pending_rename_file = Some(path);
        let mut prompt = Prompt::new(PromptKind::RenameFile, "Rename", "new name");
        prompt.input = name;
        self.overlay = Overlay::Prompt(prompt);
    }
}
