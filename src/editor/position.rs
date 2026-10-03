//! Cursor positions and selection helpers.

/// A position in a document.
///
/// `row` is a zero-based line index and `col` is a zero-based **character** offset
/// within that line. Character offsets keep multi-byte and wide characters correct.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    pub row: usize,
    pub col: usize,
}

impl Position {
    pub const fn new(row: usize, col: usize) -> Self {
        Position { row, col }
    }

    pub const fn zero() -> Self {
        Position { row: 0, col: 0 }
    }

    /// The position reached after appending `text` at `self`.
    pub fn advanced_by(self, text: &str) -> Position {
        let mut row = self.row;
        let mut col = self.col;
        for c in text.chars() {
            if c == '\n' {
                row += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        Position { row, col }
    }
}

/// A selection is an ordered range between an anchor and the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Selection {
    pub anchor: Position,
}

impl Selection {
    pub fn new(anchor: Position) -> Self {
        Selection { anchor }
    }

    /// The normalized `(start, end)` range for a selection whose cursor is `cursor`.
    pub fn range(self, cursor: Position) -> (Position, Position) {
        if self.anchor <= cursor {
            (self.anchor, cursor)
        } else {
            (cursor, self.anchor)
        }
    }
}

/// A secondary cursor, with its own optional selection (via `anchor`).
///
/// The primary cursor stays on [`Document`](crate::editor::Document) as
/// `cursor`/`selection`; [`Cursor`] carries the additional ones. A caret has
/// `anchor == cursor`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub anchor: Position,
    pub cursor: Position,
}

impl Cursor {
    /// A plain caret at `position`.
    pub const fn caret(position: Position) -> Self {
        Cursor {
            anchor: position,
            cursor: position,
        }
    }

    /// A cursor with a selection from `anchor` to `cursor`.
    pub const fn selecting(anchor: Position, cursor: Position) -> Self {
        Cursor { anchor, cursor }
    }

    /// The normalized `(start, end)` range.
    pub fn range(self) -> (Position, Position) {
        if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }

    /// The text range this cursor would replace.
    pub fn is_caret(self) -> bool {
        self.anchor == self.cursor
    }
}

impl From<(Position, Position)> for Cursor {
    fn from((start, end): (Position, Position)) -> Self {
        Cursor {
            anchor: start,
            cursor: end,
        }
    }
}
