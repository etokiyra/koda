//! Undo/redo history.
//!
//! Each [`Edit`] records just enough information to invert a single logical change:
//! the character offset it began at, the text that was removed, the text that was
//! inserted, and the cursor before/after. This makes undo cheap even for large files.

use crate::editor::position::Position;

/// A single reversible change.
#[derive(Clone, Debug)]
pub struct Edit {
    /// Character offset where the change began.
    pub start: usize,
    /// Text removed by the change (re-inserted on undo).
    pub removed: String,
    /// Text inserted by the change (removed on undo).
    pub inserted: String,
    pub cursor_before: Position,
    pub cursor_after: Position,
}

/// A bounded undo/redo stack.
#[derive(Debug)]
pub struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    limit: usize,
}

impl Default for History {
    fn default() -> Self {
        History::new(1000)
    }
}

impl History {
    pub fn new(limit: usize) -> Self {
        History {
            undo: Vec::new(),
            redo: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Record a new edit. Any pending redo history is discarded.
    pub fn push(&mut self, edit: Edit) {
        self.redo.clear();
        self.undo.push(edit);
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    /// Take the next edit to undo.
    pub fn undo(&mut self) -> Option<Edit> {
        let edit = self.undo.pop()?;
        self.redo.push(edit.clone());
        Some(edit)
    }

    /// Take the next edit to redo.
    pub fn redo(&mut self) -> Option<Edit> {
        let edit = self.redo.pop()?;
        self.undo.push(edit.clone());
        Some(edit)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}
