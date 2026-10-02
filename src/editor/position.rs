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
