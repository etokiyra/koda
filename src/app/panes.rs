//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

impl App {
    /// Clamp pane indices after arbitrary document removals.
    pub(super) fn clamp_panes(&mut self) {
        let len = self.editor.len();
        if len == 0 {
            self.pane_left = 0;
            self.pane_right = None;
            self.split = false;
            self.focus_pane = Pane::Primary;
            return;
        }
        self.pane_left = self.pane_left.min(len - 1);
        if self.pane_right.is_some_and(|index| index >= len) {
            self.pane_right = None;
        }
        if self.pane_right.is_none() {
            self.split = false;
            if self.focus_pane == Pane::Secondary {
                self.focus_pane = Pane::Primary;
            }
        }
        self.sync_active_pane();
    }

    /// The document index shown in the left pane.
    pub fn pane_left_index(&self) -> usize {
        self.pane_left
    }

    /// Activate the next tab within the focused pane.
    pub(super) fn next_tab(&mut self) {
        if self.editor.is_empty() {
            return;
        }
        self.editor.next_tab();
        let index = self.editor.active_index();
        self.set_active_pane_index(index);
    }

    /// The document index shown in the right pane, when split.
    pub fn pane_right_index(&self) -> Option<usize> {
        self.pane_right
    }

    /// Record that the focused pane now shows `index`.
    pub(super) fn set_active_pane_index(&mut self, index: usize) {
        match self.focus_pane {
            Pane::Primary => self.pane_left = index,
            Pane::Secondary => self.pane_right = Some(index),
        }
    }

    /// Move keyboard focus to the file tree, showing it first if needed. If the
    /// tree already has focus, return to the editor.
    pub(super) fn focus_tree(&mut self) {
        if !self.tree_visible {
            self.tree_visible = true;
            self.focus = Focus::FileTree;
        } else {
            self.focus = match self.focus {
                Focus::FileTree => Focus::Editor,
                Focus::Editor => Focus::FileTree,
            };
        }
    }

    /// Keep pane indices valid after document `closed` was removed.
    pub(super) fn remap_pane_indices(&mut self, closed: usize) {
        let len = self.editor.len();
        // The right pane loses its document if that was the one closed.
        if self.pane_right == Some(closed) {
            self.pane_right = None;
        } else if let Some(right) = self.pane_right
            && right > closed
        {
            self.pane_right = Some(right - 1);
        }
        // When the left pane's document closes, the right pane takes its place
        // and the split collapses.
        if self.pane_left == closed {
            self.pane_left = self.pane_right.take().unwrap_or(0);
        } else if self.pane_left > closed {
            self.pane_left -= 1;
        }
        self.pane_left = self.pane_left.min(len.saturating_sub(1));
        if self.pane_right.is_some_and(|index| index >= len) {
            self.pane_right = None;
        }
        if self.pane_right.is_none() {
            self.split = false;
            if self.focus_pane == Pane::Secondary {
                self.focus_pane = Pane::Primary;
            }
        }
        self.sync_active_pane();
    }

    /// Move editing focus to the other pane.
    pub(super) fn focus_other_pane(&mut self) {
        if !self.split || self.pane_right.is_none() {
            self.set_status("No split to focus");
            return;
        }
        self.focus_pane = match self.focus_pane {
            Pane::Primary => Pane::Secondary,
            Pane::Secondary => Pane::Primary,
        };
        self.sync_active_pane();
        if self.search.open {
            self.refresh_search_matches();
        }
        let name = self
            .editor
            .active_document()
            .map(|doc| doc.file_name())
            .unwrap_or_default();
        self.set_status(format!("Focused {name}"));
    }

    /// Toggle the side-by-side editor split.
    pub(super) fn toggle_split(&mut self) {
        if self.split {
            self.split = false;
            self.pane_right = None;
            self.focus_pane = Pane::Primary;
            self.editor.set_active(self.pane_left);
            self.set_status("Split closed");
            return;
        }
        if self.editor.len() < 2 {
            self.set_status("Open another file to split");
            return;
        }
        let active = self.editor.active_index();
        self.pane_left = active;
        self.pane_right = Some((active + 1) % self.editor.len());
        self.split = true;
        self.focus_pane = Pane::Primary;
        self.editor.set_active(active);
        self.set_status("Split · Alt+O switches panes");
    }

    /// Make `editor.active` follow the focused pane.
    pub(super) fn sync_active_pane(&mut self) {
        let index = self.active_pane_index();
        self.editor.set_active(index);
    }

    /// Activate the previous tab within the focused pane.
    pub(super) fn previous_tab(&mut self) {
        if self.editor.is_empty() {
            return;
        }
        self.editor.previous_tab();
        let index = self.editor.active_index();
        self.set_active_pane_index(index);
    }

    /// The document index shown by the focused pane.
    pub(super) fn active_pane_index(&self) -> usize {
        match self.focus_pane {
            Pane::Primary => self.pane_left,
            Pane::Secondary => self.pane_right.unwrap_or(self.pane_left),
        }
    }
}
