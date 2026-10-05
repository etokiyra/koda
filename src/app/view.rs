//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

impl App {
    pub(super) fn toggle_hidden(&mut self) {
        self.workspace.tree.toggle_hidden();
        let state = if self.workspace.tree.show_hidden {
            "shown"
        } else {
            "hidden"
        };
        self.set_status(format!("Dotfiles {state}"));
    }

    /// Centre `position` in the viewport and move the cursor there.
    pub(super) fn jump_to(&mut self, position: Position) {
        let center = self.viewport_height / 2;
        self.with_doc(|doc| {
            doc.move_to(position);
            doc.scroll_top = position.row.saturating_sub(center);
        });
    }

    /// Turn soft wrap on or off. The wrap width itself is recomputed by the
    /// renderer on the next frame.
    pub(super) fn toggle_wrap(&mut self) {
        self.wrap = !self.wrap;
        if let Some(doc) = self.editor.active_document_mut() {
            doc.preferred_col = None;
            doc.scroll_left = 0;
            doc.scroll_subline = 0;
        }
        let state = if self.wrap { "on" } else { "off" };
        self.set_status(format!("Soft wrap {state}"));
    }

    /// Open `path` and place the cursor at `position`, centred in the viewport.
    pub(super) fn reveal(&mut self, path: PathBuf, position: Position) {
        self.open_path(path);
        self.jump_to(position);
    }

    pub(super) fn open_tree_filter(&mut self) {
        if !self.tree_visible {
            self.tree_visible = true;
        }
        self.focus = Focus::FileTree;
        self.tree_filter = Some(TreeFilter::new(self.workspace.root()));
    }

    /// Toggle the inline diagnostic messages at the end of each line.
    pub(super) fn toggle_inline_diagnostics(&mut self) {
        self.inline_diagnostics = !self.inline_diagnostics;
        let state = if self.inline_diagnostics {
            "shown"
        } else {
            "hidden"
        };
        self.set_status(format!("Inline diagnostics {state}"));
    }

    pub(super) fn request_quit(&mut self) {
        if self.editor.has_unsaved() && !self.quit_armed {
            self.quit_armed = true;
            self.set_error("Unsaved changes — Ctrl+S to save, Ctrl+Q again to quit");
            return;
        }
        self.should_quit = true;
    }

    /// Like [`reveal`], but `position` is LSP-encoded and is converted using the
    /// opened document's text before the cursor moves.
    pub(super) fn reveal_lsp(&mut self, path: PathBuf, position: Position) {
        self.open_path(path);
        let position = self.lsp_position_to_char(position);
        self.jump_to(position);
    }
}
