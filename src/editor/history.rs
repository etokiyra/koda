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

/// A single primitive replacement inside an [`Edit`].
///
/// One edit usually has one op, but a multi-cursor keystroke produces several
/// and is undone/redone as a single step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditOp {
    /// Character offset where the change began.
    pub start: usize,
    /// Text removed by the change (re-inserted on undo).
    pub removed: String,
    /// Text inserted by the change (removed on undo).
    pub inserted: String,
}

/// A reversible change.
#[derive(Clone, Debug)]
pub struct Edit {
    /// A process-unique identity, used to compare against the save point even
    /// after edits have been undone and re-applied.
    pub id: u64,
    /// The primitive replacements, in the order they were applied.
    pub ops: Vec<EditOp>,
    pub cursor_before: Position,
    pub cursor_after: Position,
    /// Whether this edit may merge with the one before it.
    pub coalesce: Option<Coalesce>,
}

impl Edit {
    /// A single-op edit.
    pub fn single(
        start: usize,
        removed: String,
        inserted: String,
        cursor_before: Position,
        cursor_after: Position,
        coalesce: Option<Coalesce>,
    ) -> Self {
        Edit {
            id: 0,
            ops: vec![EditOp {
                start,
                removed,
                inserted,
            }],
            cursor_before,
            cursor_after,
            coalesce,
        }
    }

    /// Whether this edit consists of exactly one primitive op.
    fn is_single(&self) -> bool {
        self.ops.len() == 1
    }
}

/// A bounded undo/redo stack.
#[derive(Debug)]
pub struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    limit: usize,
    next_id: u64,
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
            next_id: 1,
        }
    }

    /// Record a new edit, merging it with the previous one when appropriate.
    pub fn push(&mut self, mut edit: Edit) {
        self.redo.clear();
        if let Some(kind) = edit.coalesce
            && let Some(last) = self.undo.last_mut()
            && last.coalesce == Some(kind)
            && merge(last, &edit, kind)
        {
            return;
        }
        edit.id = self.next_id;
        self.next_id += 1;
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

    /// Number of undoable edits.
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Number of redoable edits.
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// The identity of the top undo edit, or `None` when history is at the
    /// initial state. Used to test whether the document is back at its save
    /// point.
    pub fn top_id(&self) -> Option<u64> {
        self.undo.last().map(|edit| edit.id)
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

fn merge(last: &mut Edit, edit: &Edit, kind: Coalesce) -> bool {
    // Only single-op edits coalesce; a multi-cursor group is always its own step.
    if !last.is_single() || !edit.is_single() {
        return false;
    }
    let last_op = &mut last.ops[0];
    let edit_op = &edit.ops[0];
    match kind {
        Coalesce::Insert => {
            if !last_op.removed.is_empty() || !edit_op.removed.is_empty() {
                return false;
            }
            if last_op.start + last_op.inserted.chars().count() != edit_op.start {
                return false;
            }
            last_op.inserted.push_str(&edit_op.inserted);
            last.cursor_after = edit.cursor_after;
            true
        }
        Coalesce::DeleteBackward => {
            if !last_op.inserted.is_empty() || !edit_op.inserted.is_empty() {
                return false;
            }
            if edit_op.start + edit_op.removed.chars().count() != last_op.start {
                return false;
            }
            last_op.removed = format!("{}{}", edit_op.removed, last_op.removed);
            last_op.start = edit_op.start;
            // `cursor_before` must stay at the *first* deletion's right edge,
            // while `cursor_after` follows the most recent deletion's left edge,
            // so both undo and redo land where the user left off.
            last.cursor_after = edit.cursor_after;
            true
        }
        Coalesce::DeleteForward => {
            if !last_op.inserted.is_empty() || !edit_op.inserted.is_empty() {
                return false;
            }
            if edit_op.start != last_op.start {
                return false;
            }
            last_op.removed.push_str(&edit_op.removed);
            last.cursor_after = edit.cursor_after;
            true
        }
    }
}
