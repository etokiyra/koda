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

/// The largest file Koda will load into the editor. Larger files are refused
/// with a clear message rather than risking a long freeze and huge allocations.
pub const MAX_OPEN_BYTES: u64 = 64 * 1024 * 1024;

/// Read a source file as UTF-8, rejecting binary and oversized files.
///
/// Unlike a lossy read, this never silently replaces bytes: a file that is not
/// valid UTF-8 would otherwise be rewritten with replacement characters on the
/// first save, corrupting it.
pub fn read_text(path: &Path) -> std::io::Result<String> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > MAX_OPEN_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "file is too large to open ({} MiB)",
                metadata.len() / (1024 * 1024)
            ),
        ));
    }
    let bytes = std::fs::read(path)?;
    if bytes.contains(&0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file appears to be binary",
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "file is not valid UTF-8")
    })
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

    /// Load a buffer from disk, refusing binary, oversized and non-UTF-8 files.
    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        let text = read_text(path)?;
        Ok(Buffer::from_text(&text, Some(path.to_path_buf())))
    }

    /// The full text, ready to be written to disk.
    pub fn text(&self) -> String {
        self.text.to_string()
    }

    /// Replace the entire contents (used when reloading from disk).
    pub fn replace_contents(&mut self, text: &str) {
        self.text = Rope::from_str(text);
        self.line_ending = if text.contains("\r\n") {
            LineEnding::Crlf
        } else {
            LineEnding::Lf
        };
        self.version += 1;
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
        crate::filesystem::write_atomic(path, &self.text())?;
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

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("koda-buffer-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("file")
    }

    #[test]
    fn refuses_binary_and_non_utf8_files() {
        let binary = temp_path("binary");
        std::fs::write(&binary, b"abc\0def").unwrap();
        let err = Buffer::from_path(&binary)
            .err()
            .expect("a binary file should be refused");
        assert!(err.to_string().contains("binary"), "{err}");

        let latin1 = temp_path("latin1");
        std::fs::write(&latin1, [0xff, 0xfe, 0x00, 0x01]).unwrap();
        assert!(Buffer::from_path(&latin1).is_err());

        let _ = std::fs::remove_dir_all(binary.parent().unwrap());
    }

    #[test]
    fn saving_is_atomic_and_preserves_content() {
        let path = temp_path("save");
        std::fs::write(&path, "old\n").unwrap();
        let mut buffer = Buffer::from_path(&path).unwrap();
        buffer.insert(0, "new\n");
        assert!(buffer.save().unwrap());

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\nold\n");
        // No temporary siblings are left behind.
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".koda-"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
