//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

impl App {
    /// Display the unified diff for `path`.
    ///
    /// When `staged` is `None`, the working-tree diff is preferred and the
    /// staged diff is shown if the working tree is clean.
    pub(super) fn show_diff(&mut self, path: PathBuf, staged: Option<bool>) {
        let root = self.workspace.root().to_path_buf();
        let try_show = |side: bool| crate::git::diff(&root, &path, side);
        let (text, side) = match staged {
            Some(side) => match try_show(side) {
                Ok(text) => (text, side),
                Err(err) => {
                    self.set_error(format!("Could not diff {}: {err}", path.display()));
                    return;
                }
            },
            None => match try_show(false) {
                Ok(text) if !text.is_empty() => (text, false),
                Ok(_) => match try_show(true) {
                    Ok(text) => (text, true),
                    Err(err) => {
                        self.set_error(format!("Could not diff {}: {err}", path.display()));
                        return;
                    }
                },
                Err(err) => {
                    self.set_error(format!("Could not diff {}: {err}", path.display()));
                    return;
                }
            },
        };

        if text.is_empty() {
            self.set_status(format!("No changes to show for {}", path.display()));
            return;
        }
        let name = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let where_ = if side { "staged" } else { "working tree" };
        let title = format!("{where_} · {name}");
        self.overlay = Overlay::Diff(DiffState::from_unified(title, &text));
    }

    /// Build and show the changed-files picker from the current snapshot.
    pub(super) fn show_changed_files(&mut self) {
        let items = self.changed_files_items();
        let mut picker = Picker::new("Changed Files", "Filter files…  ·  Space stages", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// List the files changed in the working tree.
    pub(super) fn open_changed_files(&mut self) {
        if !self.workspace.git.available {
            self.set_status("Not a git repository");
            return;
        }
        if self.workspace.git.files.is_empty() {
            self.set_status("Working tree clean");
            return;
        }
        self.show_changed_files();
    }

    /// Rows for the changed-files picker, sorted by path.
    pub(super) fn changed_files_items(&self) -> Vec<PickerItem> {
        let root = self.workspace.root().to_path_buf();
        let mut entries: Vec<(PathBuf, GitFileStatus)> = self
            .workspace
            .git
            .files
            .iter()
            .map(|(path, status)| (path.clone(), *status))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
            .into_iter()
            .map(|(path, status)| {
                let relative = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                let stage = if self.workspace.git.is_staged(&path) {
                    "staged"
                } else {
                    "unstaged"
                };
                PickerItem::new(
                    relative,
                    format!("{} {} · {stage}", status.indicator(), status.label()),
                    PickerAction::OpenPath(path),
                )
            })
            .collect()
    }

    /// Stage or unstage the selected file (tree selection or active document).
    pub(super) fn toggle_stage_target(&mut self) {
        if !self.workspace.git.available {
            self.set_status("Not a git repository");
            return;
        }
        let Some(path) = self.file_op_target() else {
            self.set_status("Select a file to stage");
            return;
        };
        let staged = !self.workspace.git.is_staged(&path);
        self.dispatch_stage(path, staged, true);
    }

    /// Send a stage/unstage request and show a short status hint.
    pub(super) fn dispatch_stage(&mut self, path: PathBuf, staged: bool, refresh_picker: bool) {
        let root = self
            .workspace
            .git
            .repo_root
            .clone()
            .unwrap_or_else(|| self.workspace.root().to_path_buf());
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file")
            .to_string();
        self.pending_stage_refresh = refresh_picker;
        self.background.stage_path(root, path, staged);
        let action = if staged { "Staging" } else { "Unstaging" };
        self.set_status(format!("{action} {name}…"));
    }

    /// Show the diff for the active file: working tree first, then staged.
    pub(super) fn diff_active_file(&mut self) {
        if !self.workspace.git.available {
            self.set_status("Not a git repository");
            return;
        }
        let Some(path) = self
            .editor
            .active_document()
            .and_then(|doc| doc.buffer.path.clone())
        else {
            self.set_status("No file to diff");
            return;
        };
        self.show_diff(path, None);
    }

    /// Prompt for a commit message, then stage everything and commit.
    pub(super) fn commit_changes(&mut self) {
        if !self.workspace.git.available {
            self.set_status("Not a git repository");
            return;
        }
        if self.workspace.git.files.is_empty() {
            self.set_status("Nothing to commit");
            return;
        }
        self.open_prompt(
            PromptKind::CommitMessage,
            "Commit message",
            "describe the change",
        );
    }
}
