//! A text buffer backed by a rope.
//!
//! Ropey gives Koda efficient insertion, deletion and slicing even for large
//! files, which keeps editing responsive without a bespoke data structure.

use std::ops::Range;
use std::path::{Path, PathBuf};

use ropey::Rope;

use crate::editor::position::Position;
use crate::language::id::LanguageId;

/// The newline convention a document uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

impl LineEnding {
    fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        }
    }

    /// A short label for the statusline.
    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::Crlf => "CRLF",
        }
    }
}

/// An in-memory text buffer with identity and dirty tracking.
pub struct Buffer {
    text: Rope,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub language: LanguageId,
    pub line_ending: LineEnding,
    /// Bumped on every mutation. Useful for invalidating caches.
    pub version: u64,
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer::from_text("", None)
    }
}

impl Buffer {
    pub fn from_text(text: &str, path: Option<PathBuf>) -> Self {
        let rope = Rope::from_str(text);
        let line_ending = if text.contains("\r\n") {
            LineEnding::Crlf
        } else {
            LineEnding::Lf
        };
        Buffer {
            text: rope,
            path,
            dirty: false,
            language: LanguageId::Unknown,
            line_ending,
            version: 0,
        }
    }

    /// Load a buffer from disk. Non UTF-8 bytes are replaced rather than rejected.
    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let text = String::from_utf8_lossy(&bytes);
        Ok(Buffer::from_text(&text, Some(path.to_path_buf())))
    }

    /// The full text, ready to be written to disk.
    pub fn text(&self) -> String {
        self.text.to_string()
    }

    pub fn as_rope(&self) -> &Rope {
        &self.text
    }

    pub fn len_lines(&self) -> usize {
        self.text.len_lines()
    }

    pub fn len_chars(&self) -> usize {
        self.text.len_chars()
    }

    /// The text of a line *without* its trailing newline.
    pub fn line_text(&self, row: usize) -> String {
        if row >= self.len_lines() {
            return String::new();
        }
        let mut line = self.text.line(row).to_string();
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        line
    }

    /// The number of characters on a line, excluding the line ending.
    pub fn line_char_len(&self, row: usize) -> usize {
        if row >= self.len_lines() {
            return 0;
        }
        let slice = self.text.line(row);
        let mut len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;
            if len > 0 && slice.char(len - 1) == '\r' {
                len -= 1;
            }
        }
        len
    }

    /// Convert a cursor position to a character offset in the rope.
    pub fn position_to_char(&self, pos: Position) -> usize {
        let row = pos.row.min(self.len_lines().saturating_sub(1));
        let line_start = self.text.line_to_char(row);
        line_start + pos.col.min(self.line_char_len(row))
    }

    /// Convert a character offset to a cursor position.
    pub fn char_to_position(&self, idx: usize) -> Position {
        let idx = idx.min(self.text.len_chars());
        let row = self.text.char_to_line(idx);
        let line_start = self.text.line_to_char(row);
        Position::new(row, idx - line_start)
    }

    /// Clamp a position so it is always valid for the current text.
    pub fn clamp_position(&self, pos: Position) -> Position {
        let row = pos.row.min(self.len_lines().saturating_sub(1));
        let col = pos.col.min(self.line_char_len(row));
        Position::new(row, col)
    }

    /// Insert text at a character offset.
    pub fn insert(&mut self, char_idx: usize, text: &str) {
        if text.is_empty() {
            return;
        }
        let idx = char_idx.min(self.text.len_chars());
        self.text.insert(idx, text);
        self.version += 1;
    }

    /// Remove a character range, returning the removed text.
    pub fn remove(&mut self, range: Range<usize>) -> String {
        if range.start >= range.end {
            return String::new();
        }
        let start = range.start.min(self.text.len_chars());
        let end = range.end.min(self.text.len_chars());
        let removed = self.text.slice(start..end).to_string();
        self.text.remove(start..end);
        self.version += 1;
        removed
    }

    /// The newline string for this document's convention.
    pub fn newline(&self) -> &'static str {
        self.line_ending.as_str()
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    pub fn file_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| "Untitled".to_string())
    }

    /// Write the buffer to its path. Does nothing if the buffer is untitled.
    pub fn save(&mut self) -> std::io::Result<bool> {
        match self.path.clone() {
            Some(path) => self.save_as(&path),
            None => Ok(false),
        }
    }

    pub fn save_as(&mut self, path: &Path) -> std::io::Result<bool> {
        std::fs::write(path, self.text())?;
        self.path = Some(path.to_path_buf());
        self.mark_clean();
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_roundtrip() {
        let buffer = Buffer::from_text("hello\nworld", None);
        assert_eq!(buffer.position_to_char(Position::new(1, 3)), 9);
        assert_eq!(buffer.char_to_position(9), Position::new(1, 3));
    }

    #[test]
    fn insert_and_remove() {
        let mut buffer = Buffer::from_text("hello", None);
        buffer.insert(5, " world");
        assert_eq!(buffer.text(), "hello world");
        let removed = buffer.remove(5..11);
        assert_eq!(removed, " world");
        assert_eq!(buffer.text(), "hello");
    }

    #[test]
    fn line_lengths_ignore_newline() {
        let buffer = Buffer::from_text("ab\r\ncde\n", None);
        assert_eq!(buffer.line_char_len(0), 2);
        assert_eq!(buffer.line_char_len(1), 3);
        assert_eq!(buffer.line_text(0), "ab");
    }
}
