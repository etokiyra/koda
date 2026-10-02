//! A document: a buffer plus cursor, selection, scroll and undo state.

use std::path::Path;

use crate::editor::buffer::{Buffer, LineEnding};
use crate::editor::history::{Edit, History};
use crate::editor::position::{Position, Selection};
use crate::language::id::LanguageId;
use crate::language::provider::{HighlightSpan, HighlightState, LanguageProvider};

/// One open file.
pub struct Document {
    pub buffer: Buffer,
    pub cursor: Position,
    pub selection: Option<Selection>,
    /// Remembered column for vertical movement.
    pub preferred_col: Option<usize>,
    /// First visible line.
    pub scroll_top: usize,
    /// First visible column.
    pub scroll_left: usize,
    history: History,
    highlight_states: Vec<HighlightState>,
    highlight_computed: usize,
}

impl Document {
    pub fn new(buffer: Buffer) -> Self {
        Document {
            buffer,
            cursor: Position::zero(),
            selection: None,
            preferred_col: None,
            scroll_top: 0,
            scroll_left: 0,
            history: History::default(),
            highlight_states: vec![HighlightState::default()],
            highlight_computed: 1,
        }
    }

    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        Ok(Document::new(Buffer::from_path(path)?))
    }

    pub fn file_name(&self) -> String {
        self.buffer.file_name()
    }

    pub fn is_dirty(&self) -> bool {
        self.buffer.dirty
    }

    pub fn set_language(&mut self, language: LanguageId) {
        if self.buffer.language != language {
            self.buffer.language = language;
            self.invalidate_highlight(0);
        }
    }

    pub fn mark_clean(&mut self) {
        self.buffer.mark_clean();
    }

    // ----------------------------------------------------------------------
    // Editing
    // ----------------------------------------------------------------------

    /// The core edit primitive. Replaces the text between `start` and `end` with
    /// `inserted`, recording a single reversible [`Edit`].
    fn apply_edit(&mut self, start: Position, end: Position, inserted: &str) {
        let start = self.buffer.clamp_position(start);
        let end = self.buffer.clamp_position(end);
        let (a, b) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let start_char = self.buffer.position_to_char(a);
        let end_char = self.buffer.position_to_char(b);
        let removed = if end_char > start_char {
            self.buffer.remove(start_char..end_char)
        } else {
            String::new()
        };
        let inserted = self.normalize_newlines(inserted);
        if removed.is_empty() && inserted.is_empty() {
            return;
        }
        if !inserted.is_empty() {
            self.buffer.insert(start_char, &inserted);
        }
        let cursor_after = a.advanced_by(&inserted);
        self.history.push(Edit {
            start: start_char,
            removed,
            inserted,
            cursor_before: self.cursor,
            cursor_after,
        });
        self.cursor = cursor_after;
        self.selection = None;
        self.preferred_col = None;
        self.buffer.mark_dirty();
        self.invalidate_highlight(a.row);
    }

    /// Replace the text between `start` and `end` with `text`, as a single edit.
    pub fn replace_range(&mut self, start: Position, end: Position, text: &str) {
        self.apply_edit(start, end, text);
    }

    pub fn insert_char(&mut self, c: char) {
        self.insert_text(&c.to_string());
    }

    pub fn insert_text(&mut self, text: &str) {
        let (start, end) = self.selection_range().unwrap_or((self.cursor, self.cursor));
        self.apply_edit(start, end, text);
    }

    pub fn insert_newline(&mut self) {
        // Auto-indent: keep the current line's leading whitespace.
        let line = self.buffer.line_text(self.cursor.row);
        let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
        let text = format!("\n{indent}");
        let (start, end) = self.selection_range().unwrap_or((self.cursor, self.cursor));
        self.apply_edit(start, end, &text);
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor.col > 0 {
            let start = Position::new(self.cursor.row, self.cursor.col - 1);
            self.apply_edit(start, self.cursor, "");
        } else if self.cursor.row > 0 {
            let prev_row = self.cursor.row - 1;
            let start = Position::new(prev_row, self.buffer.line_char_len(prev_row));
            self.apply_edit(start, self.cursor, "");
        }
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        let line_len = self.buffer.line_char_len(self.cursor.row);
        if self.cursor.col < line_len {
            let end = Position::new(self.cursor.row, self.cursor.col + 1);
            self.apply_edit(self.cursor, end, "");
        } else if self.cursor.row + 1 < self.buffer.len_lines() {
            let end = Position::new(self.cursor.row + 1, 0);
            self.apply_edit(self.cursor, end, "");
        }
    }

    /// Delete the active selection, returning `true` if anything was removed.
    pub fn delete_selection(&mut self) -> bool {
        match self.selection_range() {
            Some((start, end)) => {
                self.apply_edit(start, end, "");
                true
            }
            None => false,
        }
    }

    pub fn undo(&mut self) {
        if let Some(edit) = self.history.undo() {
            let start = edit.start;
            let inserted_len = edit.inserted.chars().count();
            if inserted_len > 0 {
                self.buffer.remove(start..start + inserted_len);
            }
            if !edit.removed.is_empty() {
                self.buffer.insert(start, &edit.removed);
            }
            self.cursor = self.buffer.clamp_position(edit.cursor_before);
            self.selection = None;
            self.preferred_col = None;
            self.invalidate_highlight(self.cursor.row);
            self.buffer.mark_dirty();
        }
    }

    pub fn redo(&mut self) {
        if let Some(edit) = self.history.redo() {
            let start = edit.start;
            let removed_len = edit.removed.chars().count();
            if removed_len > 0 {
                self.buffer.remove(start..start + removed_len);
            }
            if !edit.inserted.is_empty() {
                self.buffer.insert(start, &edit.inserted);
            }
            self.cursor = self.buffer.clamp_position(edit.cursor_after);
            self.selection = None;
            self.preferred_col = None;
            self.invalidate_highlight(self.cursor.row);
            self.buffer.mark_dirty();
        }
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    fn normalize_newlines(&self, text: &str) -> String {
        let lf = text.replace("\r\n", "\n");
        match self.buffer.line_ending {
            LineEnding::Lf => lf,
            LineEnding::Crlf => lf.replace('\n', "\r\n"),
        }
    }

    // ----------------------------------------------------------------------
    // Selection
    // ----------------------------------------------------------------------

    pub fn selection_range(&self) -> Option<(Position, Position)> {
        let sel = self.selection?;
        let (start, end) = sel.range(self.cursor);
        if start == end {
            None
        } else {
            Some((start, end))
        }
    }

    pub fn has_selection(&self) -> bool {
        self.selection_range().is_some()
    }

    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        let s = self.buffer.position_to_char(start);
        let e = self.buffer.position_to_char(end);
        Some(self.buffer.as_rope().slice(s..e).to_string())
    }

    pub fn select_all(&mut self) {
        let last_row = self.buffer.len_lines().saturating_sub(1);
        let last_col = self.buffer.line_char_len(last_row);
        self.selection = Some(Selection::new(Position::zero()));
        self.cursor = Position::new(last_row, last_col);
        self.preferred_col = None;
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    // ----------------------------------------------------------------------
    // Movement
    // ----------------------------------------------------------------------

    fn set_cursor(&mut self, pos: Position, shift: bool) {
        self.set_cursor_inner(pos, shift, true);
    }

    fn set_cursor_inner(&mut self, pos: Position, shift: bool, reset_preferred: bool) {
        let pos = self.buffer.clamp_position(pos);
        if shift {
            if self.selection.is_none() {
                self.selection = Some(Selection::new(self.cursor));
            }
        } else {
            self.selection = None;
        }
        self.cursor = pos;
        if reset_preferred {
            self.preferred_col = None;
        }
    }

    pub fn move_left(&mut self, shift: bool) {
        if !shift && let Some(sel) = self.selection.take() {
            let (start, _) = sel.range(self.cursor);
            self.cursor = self.buffer.clamp_position(start);
            self.preferred_col = None;
            return;
        }
        let pos = self.prev_pos(self.cursor);
        if let Some(pos) = pos {
            self.set_cursor(pos, shift);
        }
    }

    pub fn move_right(&mut self, shift: bool) {
        if !shift && let Some(sel) = self.selection.take() {
            let (_, end) = sel.range(self.cursor);
            self.cursor = self.buffer.clamp_position(end);
            self.preferred_col = None;
            return;
        }
        let pos = self.next_pos(self.cursor);
        if let Some(pos) = pos {
            self.set_cursor(pos, shift);
        }
    }

    pub fn move_up(&mut self, shift: bool) {
        if self.cursor.row == 0 {
            return;
        }
        let target_col = *self.preferred_col.get_or_insert(self.cursor.col);
        let row = self.cursor.row - 1;
        let col = target_col.min(self.buffer.line_char_len(row));
        self.set_cursor_inner(Position::new(row, col), shift, false);
    }

    pub fn move_down(&mut self, shift: bool) {
        if self.cursor.row + 1 >= self.buffer.len_lines() {
            return;
        }
        let target_col = *self.preferred_col.get_or_insert(self.cursor.col);
        let row = self.cursor.row + 1;
        let col = target_col.min(self.buffer.line_char_len(row));
        self.set_cursor_inner(Position::new(row, col), shift, false);
    }

    pub fn move_home(&mut self, shift: bool) {
        let line = self.buffer.line_text(self.cursor.row);
        let indent = line.chars().take_while(|c| c.is_whitespace()).count();
        let col = if self.cursor.col == indent { 0 } else { indent };
        self.set_cursor(Position::new(self.cursor.row, col), shift);
    }

    pub fn move_end(&mut self, shift: bool) {
        let col = self.buffer.line_char_len(self.cursor.row);
        self.set_cursor(Position::new(self.cursor.row, col), shift);
    }

    pub fn move_document_start(&mut self, shift: bool) {
        self.set_cursor(Position::zero(), shift);
    }

    pub fn move_document_end(&mut self, shift: bool) {
        let row = self.buffer.len_lines().saturating_sub(1);
        let col = self.buffer.line_char_len(row);
        self.set_cursor(Position::new(row, col), shift);
    }

    pub fn move_word_left(&mut self, shift: bool) {
        let mut pos = self.cursor;
        // Skip whitespace before the cursor.
        while let Some(prev) = self.prev_pos(pos) {
            if self.char_at(prev).is_some_and(|c| c.is_whitespace()) {
                pos = prev;
            } else {
                break;
            }
        }
        // Skip the word.
        while let Some(prev) = self.prev_pos(pos) {
            if self.char_at(prev).is_some_and(is_word_char) {
                pos = prev;
            } else {
                break;
            }
        }
        self.set_cursor(pos, shift);
    }

    pub fn move_word_right(&mut self, shift: bool) {
        let mut pos = self.cursor;
        while self.char_at(pos).is_some_and(|c| c.is_whitespace()) {
            match self.next_pos(pos) {
                Some(next) => pos = next,
                None => break,
            }
        }
        while self.char_at(pos).is_some_and(is_word_char) {
            match self.next_pos(pos) {
                Some(next) => pos = next,
                None => break,
            }
        }
        self.set_cursor(pos, shift);
    }

    pub fn move_to(&mut self, pos: Position) {
        self.set_cursor(pos, false);
    }

    /// Jump to a 1-based line number, as used by "go to line".
    pub fn go_to_line(&mut self, line: usize) {
        let row = line
            .saturating_sub(1)
            .min(self.buffer.len_lines().saturating_sub(1));
        self.set_cursor(Position::new(row, 0), false);
    }

    /// The character at `pos`, if it exists.
    pub fn char_at(&self, pos: Position) -> Option<char> {
        if pos.row >= self.buffer.len_lines() {
            return None;
        }
        self.buffer.line_text(pos.row).chars().nth(pos.col)
    }

    fn next_pos(&self, pos: Position) -> Option<Position> {
        if pos.col < self.buffer.line_char_len(pos.row) {
            Some(Position::new(pos.row, pos.col + 1))
        } else if pos.row + 1 < self.buffer.len_lines() {
            Some(Position::new(pos.row + 1, 0))
        } else {
            None
        }
    }

    fn prev_pos(&self, pos: Position) -> Option<Position> {
        if pos.col > 0 {
            Some(Position::new(pos.row, pos.col - 1))
        } else if pos.row > 0 {
            Some(Position::new(
                pos.row - 1,
                self.buffer.line_char_len(pos.row - 1),
            ))
        } else {
            None
        }
    }

    /// A safe copy of the current position, clamped to the buffer.
    pub fn clamped_cursor(&self) -> Position {
        self.buffer.clamp_position(self.cursor)
    }

    // ----------------------------------------------------------------------
    // Highlighting
    // ----------------------------------------------------------------------

    fn invalidate_highlight(&mut self, row: usize) {
        self.highlight_computed = self.highlight_computed.min(row).max(1);
    }

    /// State at the beginning of `row`, computing and caching as needed.
    fn highlight_state(&mut self, provider: &dyn LanguageProvider, row: usize) -> HighlightState {
        let row = row.min(self.buffer.len_lines().saturating_sub(1));
        while self.highlight_computed <= row {
            let prev = self.highlight_computed - 1;
            let text = self.buffer.line_text(prev);
            let state = self.highlight_states[prev];
            let (_, next) = provider.highlight(&text, state);
            if self.highlight_states.len() > self.highlight_computed {
                self.highlight_states[self.highlight_computed] = next;
            } else {
                self.highlight_states.push(next);
            }
            self.highlight_computed += 1;
        }
        self.highlight_states[row]
    }

    /// Highlighted spans for a single line.
    pub fn highlight_spans(
        &mut self,
        provider: &dyn LanguageProvider,
        row: usize,
    ) -> Vec<HighlightSpan> {
        if row >= self.buffer.len_lines() {
            return Vec::new();
        }
        let state = self.highlight_state(provider, row);
        let text = self.buffer.line_text(row);
        provider.highlight(&text, state).0
    }

    // ----------------------------------------------------------------------
    // Search
    // ----------------------------------------------------------------------

    /// All occurrences of `query` in the buffer, as `(start, end)` positions.
    pub fn find_all(&self, query: &str) -> Vec<(Position, Position)> {
        if query.is_empty() {
            return Vec::new();
        }
        let haystack = self.buffer.text();
        let mut matches = Vec::new();
        let mut offset = 0usize;
        // Search over characters so positions line up with character offsets.
        let hay: Vec<char> = haystack.chars().collect();
        let needle: Vec<char> = query.chars().collect();
        if needle.len() > hay.len() {
            return matches;
        }
        while offset + needle.len() <= hay.len() {
            if hay[offset..offset + needle.len()] == needle[..] {
                let start = self.buffer.char_to_position(offset);
                let end = self.buffer.char_to_position(offset + needle.len());
                matches.push((start, end));
                offset += needle.len();
            } else {
                offset += 1;
            }
        }
        matches
    }
}

fn is_word_char(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::new(Buffer::from_text(text, None))
    }

    #[test]
    fn insert_and_undo() {
        let mut d = doc("");
        d.insert_text("hello");
        assert_eq!(d.buffer.text(), "hello");
        d.undo();
        assert_eq!(d.buffer.text(), "");
        d.redo();
        assert_eq!(d.buffer.text(), "hello");
    }

    #[test]
    fn backspace_joins_lines() {
        let mut d = doc("ab\ncd");
        d.move_to(Position::new(1, 0));
        d.backspace();
        assert_eq!(d.buffer.text(), "abcd");
    }

    #[test]
    fn auto_indent_on_newline() {
        let mut d = doc("    let x = 1;");
        d.move_end(false);
        d.insert_newline();
        assert_eq!(d.buffer.text(), "    let x = 1;\n    ");
    }

    #[test]
    fn selection_delete() {
        let mut d = doc("hello world");
        d.selection = Some(Selection::new(Position::new(0, 0)));
        d.cursor = Position::new(0, 5);
        assert_eq!(d.selected_text().as_deref(), Some("hello"));
        d.delete_selection();
        assert_eq!(d.buffer.text(), " world");
    }

    #[test]
    fn find_all_matches() {
        let d = doc("foo bar foo");
        let matches = d.find_all("foo");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0], (Position::new(0, 0), Position::new(0, 3)));
        assert_eq!(matches[1], (Position::new(0, 8), Position::new(0, 11)));
    }

    #[test]
    fn word_movement() {
        let mut d = doc("let value = 1;");
        d.move_word_right(false);
        assert_eq!(d.cursor, Position::new(0, 3));
        d.move_word_right(false);
        assert_eq!(d.cursor, Position::new(0, 9));
    }
}
