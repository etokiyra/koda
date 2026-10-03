//! The editor foundation: buffers, documents, cursors and tabs.
//!
//! Language-specific behaviour is intentionally absent here. The editor only knows
//! about text and positions; providers are consulted by the UI layer.

pub mod buffer;
pub mod document;
pub mod history;
pub mod position;

use std::path::Path;

pub use buffer::{Buffer, LineEnding};
pub use document::Document;
pub use position::{Cursor, Position, Selection};

/// The set of open documents and the active tab.
#[derive(Default)]
pub struct Editor {
    pub documents: Vec<Document>,
    active: usize,
}

impl Editor {
    pub fn new() -> Self {
        Editor::default()
    }

    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    pub fn len(&self) -> usize {
        self.documents.len()
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active_document(&self) -> Option<&Document> {
        self.documents.get(self.active)
    }

    pub fn active_document_mut(&mut self) -> Option<&mut Document> {
        self.documents.get_mut(self.active)
    }

    /// Open a file, reusing an existing tab when the same path is already open.
    pub fn open_path(&mut self, path: &Path) -> std::io::Result<usize> {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if let Some(idx) = self.documents.iter().position(|d| {
            d.buffer
                .path
                .as_ref()
                .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()) == canonical)
                .unwrap_or(false)
        }) {
            self.active = idx;
            return Ok(idx);
        }
        let doc = Document::from_path(&canonical)?;
        Ok(self.push(doc))
    }

    pub fn push(&mut self, doc: Document) -> usize {
        self.documents.push(doc);
        self.active = self.documents.len() - 1;
        self.active
    }

    pub fn set_active(&mut self, index: usize) {
        if index < self.documents.len() {
            self.active = index;
        }
    }

    /// Activate the next tab, wrapping around.
    pub fn next_tab(&mut self) {
        if self.documents.is_empty() {
            return;
        }
        self.active = (self.active + 1) % self.documents.len();
    }

    /// Activate the previous tab, wrapping around.
    pub fn previous_tab(&mut self) {
        if self.documents.is_empty() {
            return;
        }
        self.active = if self.active == 0 {
            self.documents.len() - 1
        } else {
            self.active - 1
        };
    }

    /// Close a tab and keep the active index valid.
    pub fn close(&mut self, index: usize) {
        if index >= self.documents.len() {
            return;
        }
        self.documents.remove(index);
        // Closing a tab before the active one shifts it down; closing the
        // active tab naturally keeps the index on the following document.
        if index < self.active {
            self.active -= 1;
        }
        if self.active >= self.documents.len() {
            self.active = self.documents.len().saturating_sub(1);
        }
    }

    pub fn has_unsaved(&self) -> bool {
        self.documents.iter().any(|d| d.is_dirty())
    }

    /// Close every tab.
    pub fn close_all(&mut self) {
        self.documents.clear();
        self.active = 0;
    }

    /// Save the active document, if it has a path.
    pub fn save_active(&mut self) -> std::io::Result<bool> {
        match self.active_document_mut() {
            Some(doc) => doc.save(),
            None => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_with(names: &[&str]) -> Editor {
        let mut editor = Editor::new();
        for name in names {
            editor.push(Document::new(Buffer::from_text(name, None)));
        }
        editor
    }

    #[test]
    fn closing_an_earlier_tab_keeps_the_active_document() {
        // Regression: closing a tab before the active one left `active` pointing
        // at the wrong document because it was never shifted down.
        let mut editor = editor_with(&["a", "b", "c"]);
        editor.set_active(2);
        editor.close(0);
        assert_eq!(editor.active_index(), 1);
        assert_eq!(
            editor.active_document().unwrap().buffer.text(),
            "c",
            "the active document should still be `c`"
        );
    }

    #[test]
    fn closing_the_active_tab_focuses_the_following_one() {
        let mut editor = editor_with(&["a", "b", "c"]);
        editor.set_active(1);
        editor.close(1);
        assert_eq!(editor.active_document().unwrap().buffer.text(), "c");
    }

    #[test]
    fn closing_the_last_tab_falls_back_to_the_previous() {
        let mut editor = editor_with(&["a", "b"]);
        editor.set_active(1);
        editor.close(1);
        assert_eq!(editor.active_document().unwrap().buffer.text(), "a");
    }
}
