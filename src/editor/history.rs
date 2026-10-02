//! Undo/redo history.
//!
//! Each [`Edit`] records just enough information to invert a single logical change:
//! the character offset it began at, the text that was removed, the text that was
//! inserted, and the cursor before/after. Consecutive single-character edits are
//! *coalesced* into one undo step, so undo feels like other editors rather than
//! unwinding a word one letter at a time.

use crate::editor::position::Position;

/// How an edit may be merged with the previous one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coalesce {
    /// Typing characters forward.
    Insert,
    /// Backspacing into previous text.
    DeleteBackward,
    /// Deleting forward.
    DeleteForward,
}

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
    /// Whether this edit may merge with the one before it.
    pub coalesce: Option<Coalesce>,
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

    /// Record a new edit, merging it with the previous one when appropriate.
    pub fn push(&mut self, edit: Edit) {
        self.redo.clear();
        if let Some(kind) = edit.coalesce
            && let Some(last) = self.undo.last_mut()
            && last.coalesce == Some(kind)
            && merge(last, &edit, kind)
        {
            return;
        }
        self.undo.push(edit);
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    /// Prevent the next edit from merging into the current step.
    ///
    /// Called when the cursor moves, so typing in two separate places becomes two
    /// undo steps.
    pub fn break_coalesce(&mut self) {
        if let Some(last) = self.undo.last_mut() {
            last.coalesce = None;
        }
    }

    /// Take the next edit to undo.
    pub fn undo(&mut self) -> Option<Edit> {
        let edit = self.undo.pop()?;
        if let Some(last) = self.undo.last_mut() {
            last.coalesce = None;
        }
        self.redo.push(edit.clone());
        Some(edit)
    }

    /// Take the next edit to redo.
    pub fn redo(&mut self) -> Option<Edit> {
        let edit = self.redo.pop()?;
        if let Some(last) = self.undo.last_mut() {
            last.coalesce = None;
        }
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

fn merge(last: &mut Edit, edit: &Edit, kind: Coalesce) -> bool {
    match kind {
        Coalesce::Insert => {
            if !last.removed.is_empty() || !edit.removed.is_empty() {
                return false;
            }
            if last.start + last.inserted.chars().count() != edit.start {
                return false;
            }
            last.inserted.push_str(&edit.inserted);
            last.cursor_after = edit.cursor_after;
            true
        }
        Coalesce::DeleteBackward => {
            if !last.inserted.is_empty() || !edit.inserted.is_empty() {
                return false;
            }
            if edit.start + edit.removed.chars().count() != last.start {
                return false;
            }
            last.removed = format!("{}{}", edit.removed, last.removed);
            last.start = edit.start;
            last.cursor_before = edit.cursor_before;
            true
        }
        Coalesce::DeleteForward => {
            if !last.inserted.is_empty() || !edit.inserted.is_empty() {
                return false;
            }
            if edit.start != last.start {
                return false;
            }
            last.removed.push_str(&edit.removed);
            last.cursor_after = edit.cursor_after;
            true
        }
    }
}
