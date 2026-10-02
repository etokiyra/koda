//! A document: a buffer plus cursor, selection, scroll and undo state.

use std::collections::HashMap;
use std::path::Path;

use crate::editor::buffer::{Buffer, LineEnding};
use crate::editor::history::{Coalesce, Edit, History};
use crate::editor::position::{Position, Selection};
use crate::language::id::LanguageId;
use crate::language::provider::{HighlightSpan, HighlightState, LanguageProvider, TokenKind};

/// The number of columns one level of indentation represents.
pub const INDENT_WIDTH: usize = 4;

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
        self.apply_edit_coalesced(start, end, inserted, None);
    }

    /// Like [`apply_edit`], but hints how the edit may merge with the previous one.
    fn apply_edit_coalesced(
        &mut self,
        start: Position,
        end: Position,
        inserted: &str,
        coalesce: Option<Coalesce>,
    ) {
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
            coalesce,
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
        let (start, end) = self.selection_range().unwrap_or((self.cursor, self.cursor));
        self.apply_edit_coalesced(start, end, &c.to_string(), Some(Coalesce::Insert));
    }

    /// Type a character, applying automatic pairing where it helps.
    ///
    /// Typing a closing bracket or double quote that is already under the cursor
    /// skips over it rather than inserting a duplicate.
    pub fn type_char(&mut self, c: char) {
        if (is_close_bracket(c) || c == '"') && self.char_at(self.cursor) == Some(c) {
            if let Some(next) = self.next_pos(self.cursor) {
                self.set_cursor(next, false);
            }
            return;
        }

        if let Some(close) = matching_close(c) {
            // Wrap a selection: `(selected)`.
            if let Some((start, end)) = self.selection_range() {
                let selected = self
                    .buffer
                    .as_rope()
                    .slice(self.buffer.position_to_char(start)..self.buffer.position_to_char(end))
                    .to_string();
                let text = format!("{c}{selected}{close}");
                self.apply_edit(start, end, &text);
                let after = start.advanced_by(&text);
                self.cursor = Position::new(after.row, after.col.saturating_sub(1));
                return;
            }

            // Insert a pair when the cursor sits at a natural boundary.
            let boundary = self.char_at(self.cursor).is_none_or(|next| {
                next.is_whitespace() || is_close_bracket(next) || next == ';' || next == ','
            });
            if boundary {
                let text = format!("{c}{close}");
                self.apply_edit(self.cursor, self.cursor, &text);
                self.cursor = Position::new(self.cursor.row, self.cursor.col.saturating_sub(1));
                return;
            }
        }

        self.insert_char(c);
    }

    pub fn insert_text(&mut self, text: &str) {
        let (start, end) = self.selection_range().unwrap_or((self.cursor, self.cursor));
        self.apply_edit(start, end, text);
    }

    pub fn insert_newline(&mut self) {
        let line = self.buffer.line_text(self.cursor.row);
        let leading: String = line.chars().take_while(|c| c.is_whitespace()).collect();
        let before: String = line.chars().take(self.cursor.col).collect();
        let after = self.char_at(self.cursor);

        // Expanding an empty pair: `{|}` becomes a three-line indented block.
        if let Some(open) = before.chars().last().filter(|c| is_open_bracket(*c))
            && after == matching_close(open)
        {
            let outer = leading;
            let inner = format!("{outer}{}", " ".repeat(INDENT_WIDTH));
            let text = format!("\n{inner}\n{outer}");
            self.apply_edit(self.cursor, self.cursor, &text);
            self.cursor = Position::new(self.cursor.row.saturating_sub(1), inner.chars().count());
            return;
        }

        // Otherwise keep the indentation, deepening after an opening bracket.
        let mut indent = if line.trim().is_empty() && self.cursor.row > 0 {
            self.buffer
                .line_text(self.cursor.row - 1)
                .chars()
                .take_while(|c| c.is_whitespace())
                .collect::<String>()
        } else {
            leading
        };
        if before.trim_end().ends_with(['{', '(', '[']) {
            indent.push_str(&" ".repeat(INDENT_WIDTH));
        }
        let text = format!("\n{indent}");
        let (start, end) = self.selection_range().unwrap_or((self.cursor, self.cursor));
        self.apply_edit(start, end, &text);
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor.col > 0 {
            let before = self.char_at(Position::new(self.cursor.row, self.cursor.col - 1));
            let at = self.char_at(self.cursor);
            // Deleting into an empty pair removes both halves.
            if let (Some(open), Some(close)) = (before, at)
                && matching_close(open) == Some(close)
            {
                let start = Position::new(self.cursor.row, self.cursor.col - 1);
                let end = Position::new(self.cursor.row, self.cursor.col + 1);
                self.apply_edit(start, end, "");
                return;
            }
            let start = Position::new(self.cursor.row, self.cursor.col - 1);
            self.apply_edit_coalesced(start, self.cursor, "", Some(Coalesce::DeleteBackward));
        } else if self.cursor.row > 0 {
            let prev_row = self.cursor.row - 1;
            let start = Position::new(prev_row, self.buffer.line_char_len(prev_row));
            self.apply_edit_coalesced(start, self.cursor, "", Some(Coalesce::DeleteBackward));
        }
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        let line_len = self.buffer.line_char_len(self.cursor.row);
        if self.cursor.col < line_len {
            let end = Position::new(self.cursor.row, self.cursor.col + 1);
            self.apply_edit_coalesced(self.cursor, end, "", Some(Coalesce::DeleteForward));
        } else if self.cursor.row + 1 < self.buffer.len_lines() {
            let end = Position::new(self.cursor.row + 1, 0);
            self.apply_edit_coalesced(self.cursor, end, "", Some(Coalesce::DeleteForward));
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

    // ----------------------------------------------------------------------
    // Indentation and line operations
    // ----------------------------------------------------------------------

    /// Indent the selected lines, or insert one indentation step at the cursor.
    pub fn indent(&mut self) {
        match self.selection_range() {
            Some((start, end)) => self.reindent_lines(start.row, end.row, true),
            None => self.insert_tab(),
        }
    }

    /// Outdent the selected lines, or one step on the current line.
    pub fn outdent(&mut self) {
        let (start, end) = self.selection_range().unwrap_or((self.cursor, self.cursor));
        self.reindent_lines(start.row, end.row, false);
    }

    fn insert_tab(&mut self) {
        let spaces = INDENT_WIDTH - (self.cursor.col % INDENT_WIDTH);
        self.apply_edit(self.cursor, self.cursor, &" ".repeat(spaces));
    }

    fn reindent_lines(&mut self, from_row: usize, to_row: usize, increase: bool) {
        let last = self.buffer.len_lines().saturating_sub(1);
        let from_row = from_row.min(last);
        let to_row = to_row.min(last).max(from_row);
        let newline = self.buffer.newline();

        let mut changed = false;
        let lines: Vec<String> = (from_row..=to_row)
            .map(|row| {
                let line = self.buffer.line_text(row);
                if increase {
                    if line.trim().is_empty() {
                        line
                    } else {
                        changed = true;
                        format!("{}{line}", " ".repeat(INDENT_WIDTH))
                    }
                } else {
                    let remove = leading_outdent(&line);
                    if remove > 0 {
                        changed = true;
                        line.chars().skip(remove).collect()
                    } else {
                        line
                    }
                }
            })
            .collect();
        if !changed {
            return;
        }

        let block = lines.join(newline);
        let start = Position::new(from_row, 0);
        let end = Position::new(to_row, self.buffer.line_char_len(to_row));
        self.apply_edit(start, end, &block);

        // Keep the affected lines selected so repeated indent/outdent works.
        let end_col = self.buffer.line_char_len(to_row);
        self.selection = Some(Selection::new(Position::new(from_row, 0)));
        self.cursor = Position::new(to_row, end_col);
        self.preferred_col = None;
        self.history.break_coalesce();
    }

    /// Move the current line (or selected lines) up one row.
    pub fn move_line_up(&mut self) {
        self.move_lines(-1);
    }

    /// Move the current line (or selected lines) down one row.
    pub fn move_line_down(&mut self) {
        self.move_lines(1);
    }

    fn move_lines(&mut self, direction: i32) {
        let last = self.buffer.len_lines().saturating_sub(1);
        let (start_row, end_row) = match self.selection_range() {
            Some((start, end)) => (start.row.min(last), end.row.min(last)),
            None => {
                let row = self.cursor.row.min(last);
                (row, row)
            }
        };
        let newline = self.buffer.newline();
        let col = self.cursor.col;

        if direction < 0 {
            if start_row == 0 {
                return;
            }
            let target = start_row - 1;
            let above = self.buffer.line_text(target);
            let mut lines: Vec<String> = (start_row..=end_row)
                .map(|row| self.buffer.line_text(row))
                .collect();
            lines.push(above);

            let start = Position::new(target, 0);
            let end = Position::new(end_row, self.buffer.line_char_len(end_row));
            self.apply_edit(start, end, &lines.join(newline));

            let first = start_row - 1;
            let last_row = end_row - 1;
            self.place_after_line_move(first, last_row, col);
        } else {
            let below = end_row + 1;
            if below > last {
                return;
            }
            let below_text = self.buffer.line_text(below);
            let lines: Vec<String> = (start_row..=end_row)
                .map(|row| self.buffer.line_text(row))
                .collect();
            let block = format!("{below_text}{newline}{}", lines.join(newline));

            let start = Position::new(start_row, 0);
            let end = Position::new(below, self.buffer.line_char_len(below));
            self.apply_edit(start, end, &block);

            self.place_after_line_move(start_row + 1, end_row + 1, col);
        }
    }

    fn place_after_line_move(&mut self, first_row: usize, last_row: usize, col: usize) {
        if self.selection.is_some() {
            self.selection = Some(Selection::new(Position::new(first_row, 0)));
            self.cursor = Position::new(last_row, col);
        } else {
            self.cursor = Position::new(first_row, col);
            self.selection = None;
        }
        self.preferred_col = None;
        self.history.break_coalesce();
    }

    /// Duplicate the current line below itself.
    pub fn duplicate_line(&mut self) {
        let last = self.buffer.len_lines().saturating_sub(1);
        let row = self.cursor.row.min(last);
        let text = self.buffer.line_text(row);
        let end = Position::new(row, self.buffer.line_char_len(row));
        let col = self.cursor.col;
        self.apply_edit(end, end, &format!("\n{text}"));
        let new_row = (row + 1).min(self.buffer.len_lines().saturating_sub(1));
        self.cursor = Position::new(new_row, col.min(self.buffer.line_char_len(new_row)));
        self.selection = None;
        self.preferred_col = None;
        self.history.break_coalesce();
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
        self.history.break_coalesce();
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
        self.history.break_coalesce();
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

    // ----------------------------------------------------------------------
    // Brackets
    // ----------------------------------------------------------------------

    /// The bracket pair surrounding the cursor, if any.
    ///
    /// Looks at the character under the cursor first, then the one before it.
    /// Matching is nesting- and type-aware, and skips brackets inside comments
    /// and strings using the provider's highlighting.
    pub fn matching_brackets(
        &mut self,
        provider: &dyn LanguageProvider,
    ) -> Option<(Position, Position)> {
        let cursor = self.clamped_cursor();
        let at = self.char_at(cursor);
        let before = if cursor.col > 0 {
            self.char_at(Position::new(cursor.row, cursor.col - 1))
        } else {
            None
        };

        let (position, bracket) =
            at.filter(|c| is_bracket(*c))
                .map(|c| (cursor, c))
                .or_else(|| {
                    before
                        .filter(|c| is_bracket(*c))
                        .map(|c| (Position::new(cursor.row, cursor.col.saturating_sub(1)), c))
                })?;

        let forward = is_open_bracket(bracket);
        let mut stack = vec![bracket];
        let mut cache: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
        let mut current = position;
        let mut steps = 0usize;

        loop {
            steps += 1;
            if steps > 100_000 {
                return None;
            }
            current = if forward {
                self.next_pos(current)?
            } else {
                self.prev_pos(current)?
            };

            cache.entry(current.row).or_insert_with(|| {
                self.highlight_spans(provider, current.row)
                    .into_iter()
                    .filter(|span| matches!(span.kind, TokenKind::Comment | TokenKind::String))
                    .map(|span| (span.range.start, span.range.end))
                    .collect()
            });
            if cache[&current.row]
                .iter()
                .any(|(start, end)| current.col >= *start && current.col < *end)
            {
                continue;
            }

            let Some(c) = self.char_at(current) else {
                continue;
            };

            if forward {
                if is_open_bracket(c) {
                    stack.push(c);
                } else if is_close_bracket(c) {
                    if stack.last().is_some_and(|open| brackets_match(*open, c)) {
                        stack.pop();
                        if stack.is_empty() {
                            return Some((position, current));
                        }
                    } else {
                        return None;
                    }
                }
            } else if is_close_bracket(c) {
                stack.push(c);
            } else if is_open_bracket(c) {
                if stack.last().is_some_and(|close| brackets_match(c, *close)) {
                    stack.pop();
                    if stack.is_empty() {
                        return Some((position, current));
                    }
                } else {
                    return None;
                }
            }
        }
    }
}

fn is_bracket(c: char) -> bool {
    is_open_bracket(c) || is_close_bracket(c)
}

fn is_open_bracket(c: char) -> bool {
    matches!(c, '(' | '[' | '{')
}

fn is_close_bracket(c: char) -> bool {
    matches!(c, ')' | ']' | '}')
}

fn matching_close(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        _ => None,
    }
}

/// How many leading characters outdent should remove from a line.
fn leading_outdent(line: &str) -> usize {
    match line.chars().next() {
        Some('\t') => 1,
        Some(' ') => line
            .chars()
            .take_while(|c| *c == ' ')
            .take(INDENT_WIDTH)
            .count(),
        _ => 0,
    }
}

fn brackets_match(open: char, close: char) -> bool {
    matches!((open, close), ('(', ')') | ('[', ']') | ('{', '}'))
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

    #[test]
    fn matches_brackets_on_current_line() {
        let service = crate::language::LanguageService::builtin();
        let provider = service.provider(LanguageId::Rust);
        let mut d = doc("fn main() {\n    let x = (1 + 2);\n}\n");
        d.move_to(Position::new(1, 13));
        assert_eq!(
            d.matching_brackets(provider),
            Some((Position::new(1, 12), Position::new(1, 18)))
        );
    }

    #[test]
    fn matches_brackets_across_lines() {
        let service = crate::language::LanguageService::builtin();
        let provider = service.provider(LanguageId::Rust);
        let mut d = doc("fn main() {\n}\n");
        d.move_to(Position::new(0, 11));
        assert_eq!(
            d.matching_brackets(provider),
            Some((Position::new(0, 10), Position::new(1, 0)))
        );
    }

    #[test]
    fn unmatched_bracket_returns_none() {
        let service = crate::language::LanguageService::builtin();
        let provider = service.provider(LanguageId::Rust);
        let mut d = doc("fn main() {\n");
        d.move_to(Position::new(0, 11));
        assert_eq!(d.matching_brackets(provider), None);
    }

    #[test]
    fn typing_coalesces_into_one_undo_step() {
        let mut d = doc("");
        d.type_char('a');
        d.type_char('b');
        d.type_char('c');
        assert_eq!(d.buffer.text(), "abc");
        d.undo();
        assert_eq!(d.buffer.text(), "");
        d.redo();
        assert_eq!(d.buffer.text(), "abc");
    }

    #[test]
    fn movement_breaks_the_undo_group() {
        let mut d = doc("");
        d.type_char('a');
        d.type_char('b');
        d.move_left(false);
        d.type_char('c');
        assert_eq!(d.buffer.text(), "acb");
        d.undo();
        assert_eq!(d.buffer.text(), "ab");
    }

    #[test]
    fn backspace_coalesces() {
        let mut d = doc("abcd");
        d.move_end(false);
        d.backspace();
        d.backspace();
        assert_eq!(d.buffer.text(), "ab");
        d.undo();
        assert_eq!(d.buffer.text(), "abcd");
    }

    #[test]
    fn auto_pairs_and_skips_closing() {
        let mut d = doc("");
        d.type_char('(');
        assert_eq!(d.buffer.text(), "()");
        assert_eq!(d.cursor, Position::new(0, 1));
        d.type_char(')');
        assert_eq!(d.buffer.text(), "()");
        assert_eq!(d.cursor, Position::new(0, 2));
    }

    #[test]
    fn auto_pair_wraps_a_selection() {
        let mut d = doc("foo");
        d.selection = Some(Selection::new(Position::new(0, 0)));
        d.cursor = Position::new(0, 3);
        d.type_char('(');
        assert_eq!(d.buffer.text(), "(foo)");
        assert_eq!(d.cursor, Position::new(0, 4));
    }

    #[test]
    fn backspace_removes_an_empty_pair() {
        let mut d = doc("");
        d.type_char('(');
        d.backspace();
        assert_eq!(d.buffer.text(), "");
    }

    #[test]
    fn smart_newline_expands_braces() {
        let mut d = doc("");
        d.type_char('{');
        d.insert_newline();
        assert_eq!(d.buffer.text(), "{\n    \n}");
        assert_eq!(d.cursor, Position::new(1, 4));
    }

    #[test]
    fn indent_and_outdent_selection() {
        let mut d = doc("a\nb\nc");
        d.selection = Some(Selection::new(Position::new(0, 0)));
        d.cursor = Position::new(1, 1);
        d.indent();
        assert_eq!(d.buffer.text(), "    a\n    b\nc");
        d.outdent();
        assert_eq!(d.buffer.text(), "a\nb\nc");
    }

    #[test]
    fn tab_inserts_to_the_next_stop() {
        let mut d = doc("ab");
        d.move_end(false);
        d.indent();
        assert_eq!(d.buffer.text(), "ab  ");
        let mut d = doc("");
        d.indent();
        assert_eq!(d.buffer.text(), "    ");
    }

    #[test]
    fn move_line_up_and_down() {
        let mut d = doc("a\nb\nc");
        d.move_to(Position::new(1, 0));
        d.move_line_up();
        assert_eq!(d.buffer.text(), "b\na\nc");
        assert_eq!(d.cursor.row, 0);
        d.move_line_down();
        assert_eq!(d.buffer.text(), "a\nb\nc");
        assert_eq!(d.cursor.row, 1);
    }

    #[test]
    fn duplicate_line() {
        let mut d = doc("a\nb");
        d.move_to(Position::new(0, 0));
        d.duplicate_line();
        assert_eq!(d.buffer.text(), "a\na\nb");
        assert_eq!(d.cursor.row, 1);
    }
}
