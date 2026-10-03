//! A document: a buffer plus cursor, selection, scroll and undo state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::editor::buffer::{Buffer, LineEnding};
use crate::editor::history::{Coalesce, Edit, EditOp, History};
use crate::editor::layout::LineLayout;
use crate::editor::position::{Cursor, Position, Selection};
use crate::language::diagnostics::{Diagnostic, Severity};
use crate::language::id::LanguageId;
use crate::language::provider::{HighlightSpan, HighlightState, LanguageProvider, TokenKind};

/// The number of columns one level of indentation represents when a file has no
/// discernible style.
pub const INDENT_WIDTH: usize = 4;

/// A planned replacement at one cursor, used by multi-cursor edits.
#[derive(Clone, Debug)]
struct MultiEdit {
    start: Position,
    end: Position,
    inserted: String,
}

/// One open file.
pub struct Document {
    pub buffer: Buffer,
    pub cursor: Position,
    pub selection: Option<Selection>,
    /// Additional cursors beyond the primary one. Each carries its own anchor
    /// (and therefore its own selection). Kept sorted by start position and
    /// free of duplicates and of the primary cursor.
    pub cursors: Vec<Cursor>,
    /// Remembered column for vertical movement.
    pub preferred_col: Option<usize>,
    /// First visible line.
    pub scroll_top: usize,
    /// First visible column.
    pub scroll_left: usize,
    /// Which visual row of `scroll_top` is at the top when soft wrap is on.
    pub scroll_subline: usize,
    /// The text width soft wrap uses, in display columns. `0` disables wrapping
    /// and restores horizontal scrolling. Updated by the renderer.
    pub wrap_width: usize,
    history: History,
    highlight_states: Vec<HighlightState>,
    highlight_computed: usize,
    /// Diagnostics from the provider, sorted by position.
    diagnostics: Vec<Diagnostic>,
    /// Revision of the most recent diagnostics request for this document. Used
    /// to drop results that arrive after a newer request.
    diagnostics_revision: u64,
    /// Whether the text changed since diagnostics were last requested.
    diagnostics_dirty: bool,
    /// Whether the current diagnostics came from a language server.
    diagnostics_from_lsp: bool,
    /// The file's modification time when it was last read or written.
    disk_mtime: Option<SystemTime>,
    /// The indentation unit detected for this file, in spaces.
    indent_width: usize,
    /// The history id of the top edit at the last save (or load). Comparing
    /// against it lets undo/redo restore the clean state, so undoing every edit
    /// no longer leaves a file marked modified.
    saved_id: Option<u64>,
}

impl Document {
    pub fn new(buffer: Buffer) -> Self {
        let indent_width = detect_indent_width(&buffer.text());
        Document {
            buffer,
            cursor: Position::zero(),
            selection: None,
            cursors: Vec::new(),
            preferred_col: None,
            scroll_top: 0,
            scroll_left: 0,
            scroll_subline: 0,
            wrap_width: 0,
            history: History::default(),
            highlight_states: vec![HighlightState::default()],
            highlight_computed: 1,
            diagnostics: Vec::new(),
            diagnostics_revision: 0,
            diagnostics_dirty: false,
            diagnostics_from_lsp: false,
            disk_mtime: None,
            indent_width,
            saved_id: None,
        }
    }

    /// The indentation unit detected for this file, in spaces.
    pub fn indent_width(&self) -> usize {
        self.indent_width
    }

    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        let mut document = Document::new(Buffer::from_path(path)?);
        document.record_disk_mtime();
        Ok(document)
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
        self.mark_saved();
    }

    /// Make the current history position the clean/save point.
    fn mark_saved(&mut self) {
        // A later edit must not coalesce into the edit that was saved, or the
        // top id (and therefore the clean state) would be unchanged.
        self.history.break_coalesce();
        self.saved_id = self.history.top_id();
        self.buffer.mark_clean();
    }

    /// Recompute dirty state after moving through history. An undo/redo that
    /// returns to the last save point makes the document clean again.
    fn refresh_dirty(&mut self) {
        self.buffer.dirty = self.history.top_id() != self.saved_id;
    }

    // ----------------------------------------------------------------------
    // Disk state
    // ----------------------------------------------------------------------

    /// Remember the file's current modification time.
    pub fn record_disk_mtime(&mut self) {
        self.disk_mtime = self
            .buffer
            .path
            .as_ref()
            .and_then(|path| crate::filesystem::modified_time(path));
    }

    /// The modification time recorded when the file was last read or written.
    pub fn disk_modified(&self) -> Option<SystemTime> {
        self.disk_mtime
    }

    /// Reload the buffer from disk, discarding history and diagnostics.
    ///
    /// The cursor is preserved (clamped) and the buffer ends up clean.
    pub fn reload_from_disk(&mut self) -> std::io::Result<bool> {
        let Some(path) = self.buffer.path.clone() else {
            return Ok(false);
        };
        let text = crate::editor::buffer::read_text(&path)?;
        let cursor = self.cursor;

        self.buffer.replace_contents(&text);
        self.history.clear();
        self.mark_saved();
        self.indent_width = detect_indent_width(&text);
        self.selection = None;
        self.cursors.clear();
        self.preferred_col = None;
        self.cursor = self.buffer.clamp_position(cursor);
        self.invalidate_highlight(0);
        self.clear_diagnostics();
        self.diagnostics_dirty = true;
        self.record_disk_mtime();
        Ok(true)
    }

    /// Write the buffer and record the resulting modification time.
    pub fn save(&mut self) -> std::io::Result<bool> {
        let saved = self.buffer.save()?;
        if saved {
            self.mark_saved();
            self.record_disk_mtime();
        }
        Ok(saved)
    }

    /// Write the buffer to `path` and record the modification time.
    pub fn save_as(&mut self, path: &Path) -> std::io::Result<bool> {
        let saved = self.buffer.save_as(path)?;
        if saved {
            self.mark_saved();
            self.record_disk_mtime();
        }
        Ok(saved)
    }

    /// Point the document at a new path after a rename or move on disk.
    pub fn set_path(&mut self, path: PathBuf) {
        self.buffer.path = Some(path);
        self.record_disk_mtime();
    }

    // ----------------------------------------------------------------------
    // Diagnostics
    // ----------------------------------------------------------------------

    /// The diagnostics currently attached to this document.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The revision of the most recent diagnostics request for this document.
    pub fn diagnostics_revision(&self) -> u64 {
        self.diagnostics_revision
    }

    /// Whether the text has changed since diagnostics were last requested.
    pub fn diagnostics_dirty(&self) -> bool {
        self.diagnostics_dirty
    }

    /// Record that a diagnostics request with `revision` was dispatched. The
    /// pending edit, if any, is considered handled.
    pub fn set_diagnostics_revision(&mut self, revision: u64) {
        self.diagnostics_revision = revision;
        self.diagnostics_dirty = false;
    }

    /// Replace the diagnostics if `revision` is still the newest request.
    ///
    /// Results computed from an older snapshot are ignored so markers never
    /// reflect text that has since changed.
    pub fn apply_diagnostics(&mut self, revision: u64, mut diagnostics: Vec<Diagnostic>) -> bool {
        if revision != self.diagnostics_revision {
            return false;
        }
        diagnostics.sort_by_key(|diagnostic| (diagnostic.start.line, diagnostic.start.col));
        self.diagnostics = diagnostics;
        true
    }

    /// Discard diagnostics, e.g. because the document just changed.
    pub fn clear_diagnostics(&mut self) {
        self.diagnostics.clear();
    }

    /// Replace diagnostics with ones published by a language server.
    pub fn set_lsp_diagnostics(&mut self, mut diagnostics: Vec<Diagnostic>) {
        diagnostics.sort_by_key(|diagnostic| (diagnostic.start.line, diagnostic.start.col));
        self.diagnostics = diagnostics;
        self.diagnostics_from_lsp = true;
        self.diagnostics_dirty = false;
    }

    /// Whether the current diagnostics came from a language server.
    pub fn diagnostics_from_lsp(&self) -> bool {
        self.diagnostics_from_lsp
    }

    /// Hand diagnostic ownership back to the built-in providers, e.g. after a
    /// language server exits.
    pub fn use_builtin_diagnostics(&mut self) {
        if self.diagnostics_from_lsp {
            self.diagnostics_from_lsp = false;
            self.diagnostics.clear();
            self.diagnostics_revision = 0;
        }
    }

    /// Number of `(errors, warnings)`; other severities are not counted.
    pub fn diagnostic_counts(&self) -> (usize, usize) {
        let mut errors = 0;
        let mut warnings = 0;
        for diagnostic in &self.diagnostics {
            match diagnostic.severity {
                Severity::Error => errors += 1,
                Severity::Warning => warnings += 1,
                _ => {}
            }
        }
        (errors, warnings)
    }

    /// The most severe diagnostic that *starts* on `row`, for the gutter marker.
    pub fn diagnostic_severity_on_line(&self, row: usize) -> Option<Severity> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.start.line == row)
            .map(|diagnostic| diagnostic.severity)
            .max()
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
        self.history.push(Edit::single(
            start_char,
            removed,
            inserted,
            self.cursor,
            cursor_after,
            coalesce,
        ));
        self.cursor = cursor_after;
        self.selection = None;
        // A single-point edit collapses any multi-cursor session.
        self.cursors.clear();
        self.preferred_col = None;
        self.buffer.mark_dirty();
        self.invalidate_highlight(a.row);
        // Diagnostics describe the previous text; drop them until recomputed.
        self.diagnostics.clear();
        self.diagnostics_dirty = true;
    }

    // ----------------------------------------------------------------------
    // Multiple cursors
    // ----------------------------------------------------------------------

    /// Whether there is more than one cursor.
    pub fn has_multiple_cursors(&self) -> bool {
        !self.cursors.is_empty()
    }

    /// Drop every secondary cursor, keeping the primary in place.
    pub fn clear_extra_cursors(&mut self) {
        self.cursors.clear();
    }

    /// The primary cursor's range (a zero-width caret when nothing is selected).
    pub fn primary_range(&self) -> (Position, Position) {
        self.selection_range().unwrap_or((self.cursor, self.cursor))
    }

    /// Every cursor as `(start, end, caret)`, primary first, ranges normalized.
    fn all_cursor_specs(&self) -> Vec<(Position, Position, Position)> {
        let mut specs = Vec::with_capacity(self.cursors.len() + 1);
        let (start, end) = self.primary_range();
        specs.push((start, end, self.cursor));
        for cursor in &self.cursors {
            let (start, end) = cursor.range();
            specs.push((start, end, cursor.cursor));
        }
        specs
    }

    /// The text inside a normalized range.
    fn range_text(&self, start: Position, end: Position) -> String {
        let s = self.buffer.position_to_char(start);
        let e = self.buffer.position_to_char(end);
        if e <= s {
            String::new()
        } else {
            self.buffer.as_rope().slice(s..e).to_string()
        }
    }

    /// Keep secondary cursors sorted, unique and distinct from the primary.
    fn normalize_extra_cursors(&mut self) {
        let primary = self.primary_range();
        let mut seen = std::collections::HashSet::new();
        seen.insert(primary);
        let mut kept: Vec<Cursor> = Vec::new();
        for cursor in self.cursors.drain(..) {
            let range = cursor.range();
            if seen.insert(range) {
                kept.push(Cursor::from(range));
            }
        }
        kept.sort_by_key(|cursor| cursor.range());
        self.cursors = kept;
    }

    /// Apply replacements at every cursor as a single undoable edit.
    ///
    /// `edits` are normalized and applied from the bottom of the buffer upward
    /// so earlier edits stay valid; `cursors` holds the resulting cursors with
    /// the primary first. A zero-op call still moves the cursors, which is what
    /// makes bracket skip-over work at every cursor.
    fn apply_multi_edit(
        &mut self,
        mut edits: Vec<MultiEdit>,
        cursors: Vec<Cursor>,
        coalesce: Option<Coalesce>,
    ) {
        if edits.is_empty() || cursors.is_empty() {
            return;
        }
        // A genuine single cursor keeps the exact existing single-cursor path
        // (and its coalescing).
        if edits.len() == 1 && self.cursors.is_empty() {
            let edit = edits.remove(0);
            self.apply_edit_coalesced(edit.start, edit.end, &edit.inserted, coalesce);
            return;
        }

        let primary = cursors[0];
        let cursor_before = self.cursor;

        for edit in &mut edits {
            if edit.start > edit.end {
                std::mem::swap(&mut edit.start, &mut edit.end);
            }
        }
        let mut application = edits.clone();
        application.sort_by(|a, b| b.start.cmp(&a.start).then(b.end.cmp(&a.end)));

        let mut ops = Vec::new();
        let mut min_row = usize::MAX;
        for edit in &application {
            let start = self.buffer.clamp_position(edit.start);
            let end = self.buffer.clamp_position(edit.end);
            let start_char = self.buffer.position_to_char(start);
            let end_char = self.buffer.position_to_char(end);
            let removed = if end_char > start_char {
                self.buffer.remove(start_char..end_char)
            } else {
                String::new()
            };
            let inserted = self.normalize_newlines(&edit.inserted);
            if removed.is_empty() && inserted.is_empty() {
                continue;
            }
            if !inserted.is_empty() {
                self.buffer.insert(start_char, &inserted);
            }
            min_row = min_row.min(start.row);
            ops.push(EditOp {
                start: start_char,
                removed,
                inserted,
            });
        }

        // Move every cursor even when the keystroke produced no text (a
        // skip-over), so all cursors stay coherent.
        self.cursor = self.buffer.clamp_position(primary.cursor);
        self.selection = if primary.anchor == primary.cursor {
            None
        } else {
            Some(Selection::new(primary.anchor))
        };
        self.cursors = cursors.into_iter().skip(1).collect();
        self.preferred_col = None;
        self.normalize_extra_cursors();

        if ops.is_empty() {
            return;
        }
        self.history.push(Edit {
            id: 0,
            ops,
            cursor_before,
            cursor_after: self.cursor,
            coalesce,
        });
        self.buffer.mark_dirty();
        if min_row != usize::MAX {
            self.invalidate_highlight(min_row);
        }
        self.diagnostics.clear();
        self.diagnostics_dirty = true;
    }

    /// Plan a newline at one cursor (`start`/`end` normalized, `caret` actual).
    fn plan_newline(&self, start: Position, end: Position, caret: Position) -> (MultiEdit, Cursor) {
        let line = self.buffer.line_text(caret.row);
        let leading: String = line.chars().take_while(|c| c.is_whitespace()).collect();
        let before: String = line.chars().take(caret.col).collect();
        let after = self.char_at(caret);

        // Expanding an empty pair: `{|}` becomes a three-line indented block.
        if let Some(open) = before.chars().last().filter(|c| is_open_bracket(*c))
            && after == matching_close(open)
        {
            let outer = leading;
            let inner = format!("{outer}{}", " ".repeat(self.indent_width));
            let text = format!("\n{inner}\n{outer}");
            let cursor = start.advanced_by(&text);
            let cursor = Position::new(cursor.row.saturating_sub(1), inner.chars().count());
            return (
                MultiEdit {
                    start,
                    end,
                    inserted: text,
                },
                Cursor::caret(cursor),
            );
        }

        let mut indent = if line.trim().is_empty() && caret.row > 0 {
            self.buffer
                .line_text(caret.row - 1)
                .chars()
                .take_while(|c| c.is_whitespace())
                .collect::<String>()
        } else {
            leading
        };
        if before.trim_end().ends_with(['{', '(', '[']) {
            indent.push_str(&" ".repeat(self.indent_width));
        }
        let text = format!("\n{indent}");
        let cursor = start.advanced_by(&text);
        (
            MultiEdit {
                start,
                end,
                inserted: text,
            },
            Cursor::caret(cursor),
        )
    }

    /// Plan a backspace at one cursor.
    fn plan_backspace(
        &self,
        start: Position,
        end: Position,
        caret: Position,
    ) -> (MultiEdit, Cursor) {
        if start != end {
            return (
                MultiEdit {
                    start,
                    end,
                    inserted: String::new(),
                },
                Cursor::caret(start),
            );
        }
        if caret.col > 0 {
            let before = self.char_at(Position::new(caret.row, caret.col - 1));
            let at = self.char_at(caret);
            // Deleting into an empty pair removes both halves.
            if let (Some(open), Some(close)) = (before, at)
                && matching_close(open) == Some(close)
            {
                let s = Position::new(caret.row, caret.col - 1);
                let e = Position::new(caret.row, caret.col + 1);
                return (
                    MultiEdit {
                        start: s,
                        end: e,
                        inserted: String::new(),
                    },
                    Cursor::caret(s),
                );
            }
            let s = Position::new(caret.row, caret.col - 1);
            return (
                MultiEdit {
                    start: s,
                    end: caret,
                    inserted: String::new(),
                },
                Cursor::caret(s),
            );
        }
        if caret.row > 0 {
            let prev = caret.row - 1;
            let s = Position::new(prev, self.buffer.line_char_len(prev));
            return (
                MultiEdit {
                    start: s,
                    end: caret,
                    inserted: String::new(),
                },
                Cursor::caret(s),
            );
        }
        (
            MultiEdit {
                start,
                end,
                inserted: String::new(),
            },
            Cursor::caret(caret),
        )
    }

    /// Plan a forward delete at one cursor.
    fn plan_delete_forward(
        &self,
        start: Position,
        end: Position,
        caret: Position,
    ) -> (MultiEdit, Cursor) {
        if start != end {
            return (
                MultiEdit {
                    start,
                    end,
                    inserted: String::new(),
                },
                Cursor::caret(start),
            );
        }
        let line_len = self.buffer.line_char_len(caret.row);
        if caret.col < line_len {
            let e = Position::new(caret.row, caret.col + 1);
            return (
                MultiEdit {
                    start: caret,
                    end: e,
                    inserted: String::new(),
                },
                Cursor::caret(caret),
            );
        }
        if caret.row + 1 < self.buffer.len_lines() {
            let e = Position::new(caret.row + 1, 0);
            return (
                MultiEdit {
                    start: caret,
                    end: e,
                    inserted: String::new(),
                },
                Cursor::caret(caret),
            );
        }
        (
            MultiEdit {
                start,
                end,
                inserted: String::new(),
            },
            Cursor::caret(caret),
        )
    }

    /// Add a caret one row below every current cursor.
    pub fn add_cursor_below(&mut self) {
        self.add_cursor_vertical(1);
    }

    /// Add a caret one row above every current cursor.
    pub fn add_cursor_above(&mut self) {
        self.add_cursor_vertical(-1);
    }

    fn add_cursor_vertical(&mut self, direction: isize) {
        let last = self.buffer.len_lines().saturating_sub(1);
        let specs = self.all_cursor_specs();
        let mut added: Vec<Cursor> = Vec::new();
        for (_, _, caret) in specs {
            let row = match caret.row.checked_add_signed(direction) {
                Some(row) if row <= last => row,
                _ => continue,
            };
            let col = caret.col.min(self.buffer.line_char_len(row));
            added.push(Cursor::caret(Position::new(row, col)));
        }
        if added.is_empty() {
            return;
        }
        // Fold the primary's caret into place and append the new ones.
        self.selection = None;
        let mut cursors: Vec<Cursor> = self.cursors.clone();
        cursors.push(Cursor::caret(self.cursor));
        cursors.extend(added);
        // The primary stays where it is; everything else is normalized below.
        self.cursors = cursors;
        self.normalize_extra_cursors();
        self.preferred_col = None;
        self.history.break_coalesce();
    }

    /// Select every occurrence of the current word/selection and add a cursor
    /// at each, as Visual Studio Code's "Select All Occurrences".
    pub fn select_all_occurrences(&mut self) {
        let query = match self.selection_range() {
            Some((start, end)) => {
                let text = self.range_text(start, end);
                if text.is_empty() || text.contains('\n') {
                    return;
                }
                text
            }
            None => {
                let cursor = self.clamped_cursor();
                match self.word_bounds(cursor.row, cursor.col) {
                    Some((start, end)) => self.range_text(start, end),
                    None => return,
                }
            }
        };
        if query.is_empty() {
            return;
        }
        let matches = self.find_all_with(&query, true, true);
        if matches.len() < 2 {
            return;
        }
        // Primary takes the first match; the rest become secondary cursors.
        self.selection = Some(Selection::new(matches[0].0));
        self.cursor = matches[0].1;
        self.cursors = matches.into_iter().skip(1).map(Cursor::from).collect();
        self.preferred_col = None;
        self.normalize_extra_cursors();
        self.history.break_coalesce();
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
    /// With multiple cursors, every cursor types independently and the whole
    /// keystroke is one undo step.
    pub fn type_char(&mut self, c: char) {
        if self.cursors.is_empty() {
            self.type_char_single(c);
            return;
        }
        let specs = self.all_cursor_specs();
        let mut edits = Vec::with_capacity(specs.len());
        let mut cursors = Vec::with_capacity(specs.len());
        for (start, end, caret) in specs {
            let (edit, cursor) = self.plan_type_char(c, start, end, caret);
            edits.push(edit);
            cursors.push(cursor);
        }
        self.apply_multi_edit(edits, cursors, Some(Coalesce::Insert));
    }

    /// Type a character on a single cursor, applying automatic pairing.
    fn type_char_single(&mut self, c: char) {
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

    /// Plan typing `c` at one cursor range `(start, end)` whose caret is `caret`.
    fn plan_type_char(
        &self,
        c: char,
        start: Position,
        end: Position,
        caret: Position,
    ) -> (MultiEdit, Cursor) {
        // Skip over an already-inserted partner (no edit, just move).
        if (is_close_bracket(c) || c == '"') && start == end && self.char_at(caret) == Some(c) {
            let next = self.next_pos(caret).unwrap_or(caret);
            return (
                MultiEdit {
                    start,
                    end,
                    inserted: String::new(),
                },
                Cursor::caret(next),
            );
        }
        if let Some(close) = matching_close(c) {
            if start != end {
                let selected = self.range_text(start, end);
                let text = format!("{c}{selected}{close}");
                let inside = Position::new(start.row, start.col + 1);
                return (
                    MultiEdit {
                        start,
                        end,
                        inserted: text,
                    },
                    Cursor::caret(inside),
                );
            }
            let boundary = self.char_at(caret).is_none_or(|next| {
                next.is_whitespace() || is_close_bracket(next) || next == ';' || next == ','
            });
            if boundary {
                let text = format!("{c}{close}");
                let inside = Position::new(start.row, start.col + 1);
                return (
                    MultiEdit {
                        start,
                        end,
                        inserted: text,
                    },
                    Cursor::caret(inside),
                );
            }
        }
        let text = c.to_string();
        let after = start.advanced_by(&text);
        (
            MultiEdit {
                start,
                end,
                inserted: text,
            },
            Cursor::caret(after),
        )
    }

    pub fn insert_text(&mut self, text: &str) {
        if self.cursors.is_empty() {
            let (start, end) = self.selection_range().unwrap_or((self.cursor, self.cursor));
            self.apply_edit(start, end, text);
            return;
        }
        let specs = self.all_cursor_specs();
        let mut edits = Vec::with_capacity(specs.len());
        let mut cursors = Vec::with_capacity(specs.len());
        for (start, end, _) in specs {
            let after = start.advanced_by(text);
            edits.push(MultiEdit {
                start,
                end,
                inserted: text.to_string(),
            });
            cursors.push(Cursor::caret(after));
        }
        self.apply_multi_edit(edits, cursors, None);
    }

    pub fn insert_newline(&mut self) {
        if self.cursors.is_empty() {
            self.insert_newline_single();
            return;
        }
        let specs = self.all_cursor_specs();
        let mut edits = Vec::with_capacity(specs.len());
        let mut cursors = Vec::with_capacity(specs.len());
        for (start, end, caret) in specs {
            let (edit, cursor) = self.plan_newline(start, end, caret);
            edits.push(edit);
            cursors.push(cursor);
        }
        self.apply_multi_edit(edits, cursors, None);
    }

    fn insert_newline_single(&mut self) {
        let line = self.buffer.line_text(self.cursor.row);
        let leading: String = line.chars().take_while(|c| c.is_whitespace()).collect();
        let before: String = line.chars().take(self.cursor.col).collect();
        let after = self.char_at(self.cursor);

        // Expanding an empty pair: `{|}` becomes a three-line indented block.
        if let Some(open) = before.chars().last().filter(|c| is_open_bracket(*c))
            && after == matching_close(open)
        {
            let outer = leading;
            let inner = format!("{outer}{}", " ".repeat(self.indent_width));
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
            indent.push_str(&" ".repeat(self.indent_width));
        }
        let text = format!("\n{indent}");
        let (start, end) = self.selection_range().unwrap_or((self.cursor, self.cursor));
        self.apply_edit(start, end, &text);
    }

    pub fn backspace(&mut self) {
        if self.cursors.is_empty() {
            self.backspace_single();
            return;
        }
        let specs = self.all_cursor_specs();
        let mut edits = Vec::with_capacity(specs.len());
        let mut cursors = Vec::with_capacity(specs.len());
        for (start, end, caret) in specs {
            let (edit, cursor) = self.plan_backspace(start, end, caret);
            edits.push(edit);
            cursors.push(cursor);
        }
        self.apply_multi_edit(edits, cursors, None);
    }

    fn backspace_single(&mut self) {
        if self.delete_selection_single() {
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
        if self.cursors.is_empty() {
            self.delete_forward_single();
            return;
        }
        let specs = self.all_cursor_specs();
        let mut edits = Vec::with_capacity(specs.len());
        let mut cursors = Vec::with_capacity(specs.len());
        for (start, end, caret) in specs {
            let (edit, cursor) = self.plan_delete_forward(start, end, caret);
            edits.push(edit);
            cursors.push(cursor);
        }
        self.apply_multi_edit(edits, cursors, None);
    }

    fn delete_forward_single(&mut self) {
        if self.delete_selection_single() {
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

    /// Delete the active selection (every cursor's selection), returning `true`
    /// if anything was removed.
    pub fn delete_selection(&mut self) -> bool {
        if self.cursors.is_empty() {
            return self.delete_selection_single();
        }
        let specs = self.all_cursor_specs();
        if !specs.iter().any(|(start, end, _)| start != end) {
            return false;
        }
        let mut edits = Vec::with_capacity(specs.len());
        let mut cursors = Vec::with_capacity(specs.len());
        for (start, end, _) in specs {
            edits.push(MultiEdit {
                start,
                end,
                inserted: String::new(),
            });
            cursors.push(Cursor::caret(start));
        }
        self.apply_multi_edit(edits, cursors, None);
        true
    }

    fn delete_selection_single(&mut self) -> bool {
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
        if !self.cursors.is_empty() {
            self.indent_multi(true);
            return;
        }
        match self.selected_rows() {
            Some((start, end)) => self.reindent_lines(start, end, true),
            None => self.insert_tab(),
        }
    }

    /// Outdent the selected lines, or one step on the current line.
    pub fn outdent(&mut self) {
        if !self.cursors.is_empty() {
            self.indent_multi(false);
            return;
        }
        let (start, end) = self
            .selected_rows()
            .unwrap_or((self.cursor.row, self.cursor.row));
        self.reindent_lines(start, end, false);
    }

    /// Indent or outdent every cursor's line as one undoable edit.
    ///
    /// Rows are made unique first, so overlapping cursors on the same line
    /// cannot indent it twice. Cursors shift with the whitespace they gained or
    /// lost, and selections are preserved.
    fn indent_multi(&mut self, increase: bool) {
        let specs = self.all_cursor_specs();
        let width = self.indent_width.max(1);
        let mut rows: Vec<usize> = Vec::new();
        for (start, end, caret) in &specs {
            if start != end {
                let last = if end.col == 0 && end.row > start.row {
                    end.row - 1
                } else {
                    end.row
                };
                rows.extend(start.row..=last);
            } else {
                rows.push(caret.row);
            }
        }
        rows.sort_unstable();
        rows.dedup();

        let mut edits = Vec::new();
        let mut deltas: Vec<(usize, isize)> = Vec::new();
        for &row in &rows {
            let line = self.buffer.line_text(row);
            if increase {
                if line.trim().is_empty() {
                    continue;
                }
                edits.push(MultiEdit {
                    start: Position::new(row, 0),
                    end: Position::new(row, 0),
                    inserted: " ".repeat(width),
                });
                deltas.push((row, width as isize));
            } else {
                let remove = leading_outdent(&line, width);
                if remove == 0 {
                    continue;
                }
                edits.push(MultiEdit {
                    start: Position::new(row, 0),
                    end: Position::new(row, remove),
                    inserted: String::new(),
                });
                deltas.push((row, -(remove as isize)));
            }
        }
        if edits.is_empty() {
            return;
        }

        let shift = |pos: Position| -> Position {
            match deltas.iter().find(|(row, _)| *row == pos.row) {
                Some((_, delta)) => {
                    let col = (pos.col as isize + delta).max(0) as usize;
                    Position::new(pos.row, col)
                }
                None => pos,
            }
        };
        let mut cursors = Vec::with_capacity(specs.len());
        for (start, end, caret) in &specs {
            let new_caret = shift(*caret);
            let anchor = if start == end {
                None
            } else if caret == start {
                Some(shift(*end))
            } else {
                Some(shift(*start))
            };
            cursors.push(Cursor::selecting(anchor.unwrap_or(new_caret), new_caret));
        }
        self.apply_multi_edit(edits, cursors, None);
    }

    fn insert_tab(&mut self) {
        let width = self.indent_width.max(1);
        let spaces = width - (self.cursor.col % width);
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
                        format!("{}{line}", " ".repeat(self.indent_width))
                    }
                } else {
                    let remove = leading_outdent(&line, self.indent_width);
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
        let (start_row, end_row) = match self.selected_rows() {
            Some((start, end)) => (start.min(last), end.min(last)),
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

    /// Delete the current line, or every line the selection touches.
    pub fn delete_line(&mut self) {
        let (first, last) = self
            .selected_rows()
            .unwrap_or((self.cursor.row, self.cursor.row));
        let start = Position::new(first, 0);
        let end = if last + 1 < self.buffer.len_lines() {
            Position::new(last + 1, 0)
        } else {
            let final_row = self.buffer.len_lines().saturating_sub(1);
            Position::new(final_row, self.buffer.line_char_len(final_row))
        };
        self.replace_range(start, end, "");
        self.cursor = self.buffer.clamp_position(start);
        self.selection = None;
        self.preferred_col = None;
        self.history.break_coalesce();
    }

    pub fn undo(&mut self) {
        if let Some(edit) = self.history.undo() {
            // Undo the primitive ops in reverse application order so earlier
            // (lower) offsets are restored first.
            for op in edit.ops.iter().rev() {
                let inserted_len = op.inserted.chars().count();
                if inserted_len > 0 {
                    self.buffer.remove(op.start..op.start + inserted_len);
                }
                if !op.removed.is_empty() {
                    self.buffer.insert(op.start, &op.removed);
                }
            }
            self.cursor = self.buffer.clamp_position(edit.cursor_before);
            self.selection = None;
            self.cursors.clear();
            self.preferred_col = None;
            self.invalidate_highlight(self.cursor.row);
            self.refresh_dirty();
            self.diagnostics.clear();
            self.diagnostics_dirty = true;
        }
    }

    pub fn redo(&mut self) {
        if let Some(edit) = self.history.redo() {
            for op in &edit.ops {
                let removed_len = op.removed.chars().count();
                if removed_len > 0 {
                    self.buffer.remove(op.start..op.start + removed_len);
                }
                if !op.inserted.is_empty() {
                    self.buffer.insert(op.start, &op.inserted);
                }
            }
            self.cursor = self.buffer.clamp_position(edit.cursor_after);
            self.selection = None;
            self.cursors.clear();
            self.preferred_col = None;
            self.invalidate_highlight(self.cursor.row);
            self.refresh_dirty();
            self.diagnostics.clear();
            self.diagnostics_dirty = true;
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

    /// The inclusive row range a selection actually touches.
    ///
    /// A selection that ends at column 0 of a later row has zero selected
    /// columns on that row (the UI draws no highlight there), so the row is
    /// excluded. This keeps indent, outdent, delete-line, move-line and
    /// toggle-comment from acting on a line the user did not select.
    pub fn selected_rows(&self) -> Option<(usize, usize)> {
        let (start, end) = self.selection_range()?;
        let last = if end.col == 0 && end.row > start.row {
            end.row - 1
        } else {
            end.row
        };
        Some((start.row, last))
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

    /// Select the word under the cursor, then add a cursor at the next
    /// occurrence on each further call. This is Visual Studio Code's `Ctrl+D`.
    ///
    /// The primary selection moves to the newest occurrence and each earlier
    /// one becomes a secondary cursor, so the whole set can be edited at once.
    /// Once every occurrence is selected, further calls wrap to the first
    /// occurrence that is not already a cursor.
    pub fn select_next_occurrence(&mut self) {
        let (query, from) = match self.selection_range() {
            Some((_, end)) => {
                let text = self.selected_text().unwrap_or_default();
                if text.is_empty() || text.contains('\n') {
                    return;
                }
                (text, end)
            }
            None => {
                let cursor = self.clamped_cursor();
                let Some((start, end)) = self.word_bounds(cursor.row, cursor.col) else {
                    return;
                };
                self.selection = Some(Selection::new(start));
                self.cursor = end;
                self.preferred_col = None;
                self.history.break_coalesce();
                return;
            }
        };

        let matches = self.find_all_with(&query, true, true);
        if matches.is_empty() {
            return;
        }
        // Everything already selected: the primary plus the secondaries.
        let mut taken: Vec<(Position, Position)> =
            self.cursors.iter().map(|cursor| cursor.range()).collect();
        let Some((primary_start, primary_end)) = self.selection_range() else {
            return;
        };
        taken.push((primary_start, primary_end));

        // The next occurrence strictly after the primary, wrapping at the end,
        // skipping anything already selected.
        let next = matches
            .iter()
            .find(|m| **m >= (from, from) && !taken.contains(m))
            .copied()
            .or_else(|| matches.iter().copied().find(|m| !taken.contains(m)))
            .unwrap_or(matches[0]);

        // If nothing new can be added, stop growing (as VS Code does).
        if taken.contains(&next) {
            return;
        }
        // The previous primary becomes a secondary; the new match becomes primary.
        self.cursors
            .push(Cursor::from((primary_start, primary_end)));
        self.selection = Some(Selection::new(next.0));
        self.cursor = next.1;
        self.preferred_col = None;
        self.normalize_extra_cursors();
        self.history.break_coalesce();
    }

    /// The whole-word bounds around `(row, col)`, if the cursor is on a word.
    fn word_bounds(&self, row: usize, col: usize) -> Option<(Position, Position)> {
        let chars: Vec<char> = self.buffer.line_text(row).chars().collect();
        if chars.is_empty() {
            return None;
        }
        let mut start = col.min(chars.len());
        if !(start < chars.len() && is_word_char(chars[start])) {
            // Not on a word character; fall back to the one just before it.
            if start > 0 && is_word_char(chars[start - 1]) {
                start -= 1;
            } else {
                return None;
            }
        }
        let mut end = start;
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }
        while end < chars.len() && is_word_char(chars[end]) {
            end += 1;
        }
        (start < end).then(|| (Position::new(row, start), Position::new(row, end)))
    }

    pub fn clear_selection(&mut self) {
        // `Esc` first ends a multi-cursor session, then clears a selection.
        if !self.cursors.is_empty() {
            self.cursors.clear();
            return;
        }
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
        // Navigating collapses a multi-cursor session onto the primary; that is
        // predictable, and a new session starts with `Ctrl+D`/`Ctrl+Alt+Arrow`.
        self.cursors.clear();
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
            self.cursors.clear();
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
            self.cursors.clear();
            self.preferred_col = None;
            return;
        }
        let pos = self.next_pos(self.cursor);
        if let Some(pos) = pos {
            self.set_cursor(pos, shift);
        }
    }

    pub fn move_up(&mut self, shift: bool) {
        if self.wrap_width > 0 {
            self.move_visual(-1, shift);
            return;
        }
        if self.cursor.row == 0 {
            return;
        }
        let target_col = *self.preferred_col.get_or_insert(self.cursor.col);
        let row = self.cursor.row - 1;
        let col = target_col.min(self.buffer.line_char_len(row));
        self.set_cursor_inner(Position::new(row, col), shift, false);
    }

    pub fn move_down(&mut self, shift: bool) {
        if self.wrap_width > 0 {
            self.move_visual(1, shift);
            return;
        }
        if self.cursor.row + 1 >= self.buffer.len_lines() {
            return;
        }
        let target_col = *self.preferred_col.get_or_insert(self.cursor.col);
        let row = self.cursor.row + 1;
        let col = target_col.min(self.buffer.line_char_len(row));
        self.set_cursor_inner(Position::new(row, col), shift, false);
    }

    /// The display column of `(row, col)`.
    pub fn display_col_of(&self, row: usize, col: usize) -> usize {
        self.line_layout(row).display_col(col)
    }

    /// The visual rows of a logical line at the current wrap width.
    pub fn row_segments(&self, row: usize, width: usize) -> Vec<(usize, usize)> {
        crate::editor::layout::wrap_segments(&self.line_layout(row), width)
    }

    /// A display layout for one logical line.
    pub fn line_layout(&self, row: usize) -> LineLayout {
        LineLayout::new(&self.buffer.line_text(row), self.indent_width.max(1))
    }

    /// The number of visual rows a logical line occupies at `width`.
    pub fn row_visual_count(&self, row: usize, width: usize) -> usize {
        self.row_segments(row, width).len()
    }

    /// Move the cursor up or down by one **visual** row when soft wrap is on.
    ///
    /// The display column is remembered across movements (like `preferred_col`
    /// for unwrapped lines), so moving through short and wrapped rows keeps the
    /// cursor in a stable column.
    fn move_visual(&mut self, direction: isize, shift: bool) {
        let width = self.wrap_width;
        let row = self.cursor.row;
        let layout = self.line_layout(row);
        let display = layout.display_col(self.cursor.col);
        let segments = crate::editor::layout::wrap_segments(&layout, width);
        let seg = crate::editor::layout::segment_index(&segments, display);
        let preferred = *self.preferred_col.get_or_insert(display);

        let (target_row, target_seg) = if direction < 0 {
            if seg > 0 {
                (row, Some(seg - 1))
            } else if row > 0 {
                (row - 1, None)
            } else {
                return;
            }
        } else if seg + 1 < segments.len() {
            (row, Some(seg + 1))
        } else if row + 1 < self.buffer.len_lines() {
            (row + 1, Some(0))
        } else {
            return;
        };

        let target_layout = self.line_layout(target_row);
        let target_segments = crate::editor::layout::wrap_segments(&target_layout, width);
        let seg_index = match target_seg {
            Some(index) => index.min(target_segments.len().saturating_sub(1)),
            // `None` means the last visual row of the previous logical line.
            None => target_segments.len().saturating_sub(1),
        };
        let (segment_start, segment_end) = target_segments[seg_index];
        // Keep the caret inside this visual row: for every row but the last, the
        // end column is the start of the next row.
        let upper = if seg_index + 1 == target_segments.len() {
            segment_end
        } else {
            segment_end.saturating_sub(1).max(segment_start)
        };
        let target_display = preferred.clamp(segment_start, upper);
        let col = target_layout.char_col(target_display);
        self.set_cursor_inner(Position::new(target_row, col), shift, false);
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

    /// The highlighted token at a character position.
    ///
    /// Used to keep automatic completion quiet inside comments and strings,
    /// where document words would only be noise.
    pub fn token_kind_at(
        &mut self,
        provider: &dyn LanguageProvider,
        row: usize,
        col: usize,
    ) -> TokenKind {
        self.highlight_spans(provider, row)
            .into_iter()
            .find(|span| span.range.contains(&col))
            .map(|span| span.kind)
            .unwrap_or(TokenKind::Plain)
    }

    // ----------------------------------------------------------------------
    // Search
    // ----------------------------------------------------------------------

    /// All occurrences of `query` in the buffer, as `(start, end)` positions.
    ///
    /// Searches line by line so a keystroke never copies the whole file. This is
    /// a case-sensitive substring search; the find bar uses
    /// [`Self::find_all_with`] for its options.
    pub fn find_all(&self, query: &str) -> Vec<(Position, Position)> {
        self.find_all_with(query, true, false)
    }

    /// All occurrences of `query`, honouring case sensitivity and whole-word
    /// matching.
    pub fn find_all_with(
        &self,
        query: &str,
        case_sensitive: bool,
        whole_word: bool,
    ) -> Vec<(Position, Position)> {
        let needle: Vec<char> = query.chars().collect();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut matches = Vec::new();
        for row in 0..self.buffer.len_lines() {
            let line = self.buffer.line_text(row);
            let hay: Vec<char> = line.chars().collect();
            if hay.len() < needle.len() {
                continue;
            }
            let mut offset = 0usize;
            while offset + needle.len() <= hay.len() {
                if matches_at(&hay, offset, &needle, case_sensitive)
                    && (!whole_word || is_word_boundary(&hay, offset, needle.len()))
                {
                    matches.push((
                        Position::new(row, offset),
                        Position::new(row, offset + needle.len()),
                    ));
                    offset += needle.len();
                } else {
                    offset += 1;
                }
            }
        }
        matches
    }

    /// All occurrences of `pattern` interpreted as a regular expression.
    ///
    /// Returns an error for unsupported syntax so the find bar can explain it.
    /// Zero-width matches are skipped because they are rarely useful to step
    /// through.
    pub fn find_all_regex(
        &self,
        pattern: &str,
        case_sensitive: bool,
    ) -> Result<Vec<(Position, Position)>, String> {
        let regex = crate::regex::Regex::new(pattern)?;
        let case_insensitive = !case_sensitive;
        let mut matches = Vec::new();
        for row in 0..self.buffer.len_lines() {
            let line = self.buffer.line_text(row);
            let chars: Vec<char> = line.chars().collect();
            let mut from = 0usize;
            while from <= chars.len() {
                let Some((start, end)) = regex.find(&chars, from, case_insensitive) else {
                    break;
                };
                if end == start {
                    from = start + 1;
                    continue;
                }
                matches.push((Position::new(row, start), Position::new(row, end)));
                from = end;
            }
        }
        Ok(matches)
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

    /// Move the cursor to the bracket matching the one under (or just before)
    /// it, if the provider finds a pair.
    pub fn goto_matching_bracket(&mut self, provider: &dyn LanguageProvider) {
        if let Some((_, partner)) = self.matching_brackets(provider) {
            self.cursor = self.buffer.clamp_position(partner);
            self.selection = None;
            self.cursors.clear();
            self.preferred_col = None;
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
fn leading_outdent(line: &str, width: usize) -> usize {
    match line.chars().next() {
        Some('\t') => 1,
        Some(' ') => line
            .chars()
            .take_while(|c| *c == ' ')
            .take(width.max(1))
            .count(),
        _ => 0,
    }
}

/// Infer a file's indentation unit from its leading whitespace.
///
/// The smallest increase in indentation between consecutive non-blank lines is
/// taken as one level, which matches 2-space JavaScript and 4-space Rust alike.
/// A file indented with tabs keeps Koda's default width for any spaces it
/// inserts, since Koda does not convert tabs.
fn detect_indent_width(text: &str) -> usize {
    let mut tabs = 0usize;
    let mut spaces = 0usize;
    let mut unit: Option<usize> = None;
    let mut previous = 0usize;
    let mut seen = false;

    for line in text.lines().take(2000) {
        if line.trim().is_empty() {
            continue;
        }
        let leading: String = line.chars().take_while(|c| c.is_whitespace()).collect();
        let width = if leading.contains('\t') {
            tabs += 1;
            // Tabs have no fixed width here; treat each as one level.
            leading.chars().filter(|c| *c == '\t').count() * INDENT_WIDTH
        } else {
            if !leading.is_empty() {
                spaces += 1;
            }
            leading.chars().count()
        };

        if seen && width > previous {
            let delta = width - previous;
            if (1..=8).contains(&delta) {
                unit = Some(unit.map_or(delta, |current| current.min(delta)));
            }
        }
        previous = width;
        seen = true;
    }

    if tabs > spaces && tabs > 0 {
        // Tab-indented file: keep the conventional width for inserted spaces.
        INDENT_WIDTH
    } else {
        unit.unwrap_or(INDENT_WIDTH).clamp(1, 8)
    }
}

fn brackets_match(open: char, close: char) -> bool {
    matches!((open, close), ('(', ')') | ('[', ']') | ('{', '}'))
}

fn is_word_char(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// Whether `needle` matches `hay` at `offset`, honouring case sensitivity.
fn matches_at(hay: &[char], offset: usize, needle: &[char], case_sensitive: bool) -> bool {
    let window = &hay[offset..offset + needle.len()];
    if case_sensitive {
        window == needle
    } else {
        window
            .iter()
            .zip(needle)
            .all(|(a, b)| a == b || a.to_lowercase().eq(b.to_lowercase()))
    }
}

/// Whether the match at `offset` of length `len` is a whole word.
fn is_word_boundary(hay: &[char], offset: usize, len: usize) -> bool {
    let before_ok = offset == 0 || !is_word_char(hay[offset - 1]);
    let after = offset + len;
    let after_ok = after >= hay.len() || !is_word_char(hay[after]);
    before_ok && after_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::diagnostics::TextPos;

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
    fn select_next_occurrence_adds_a_cursor_each_time() {
        let mut d = doc("let foo = foo + foo;");
        d.move_to(Position::new(0, 4));

        // First press selects the word under the cursor.
        d.select_next_occurrence();
        assert_eq!(d.selected_text().as_deref(), Some("foo"));
        assert_eq!(
            d.selection_range(),
            Some((Position::new(0, 4), Position::new(0, 7)))
        );
        assert!(!d.has_multiple_cursors());

        // Each further press keeps the previous occurrence as a secondary cursor
        // and moves the primary to the next one.
        d.select_next_occurrence();
        assert_eq!(
            d.selection_range(),
            Some((Position::new(0, 10), Position::new(0, 13)))
        );
        assert_eq!(d.cursors.len(), 1);
        assert_eq!(
            d.cursors[0].range(),
            (Position::new(0, 4), Position::new(0, 7))
        );

        d.select_next_occurrence();
        assert_eq!(
            d.selection_range(),
            Some((Position::new(0, 16), Position::new(0, 19)))
        );
        assert_eq!(d.cursors.len(), 2);

        // Every occurrence now has a cursor, so further presses stop growing.
        d.select_next_occurrence();
        assert_eq!(
            d.selection_range(),
            Some((Position::new(0, 16), Position::new(0, 19)))
        );
        assert_eq!(d.cursors.len(), 2);
    }

    #[test]
    fn typing_replaces_every_selected_occurrence_in_one_undo_step() {
        let mut d = doc("foo = foo + foo;");
        d.move_to(Position::new(0, 0));
        d.select_next_occurrence();
        d.select_next_occurrence();
        d.select_next_occurrence();
        assert_eq!(d.cursors.len(), 2);

        d.insert_text("bar");
        assert_eq!(d.buffer.text(), "bar = bar + bar;");

        // One undo restores all three replacements.
        d.undo();
        assert_eq!(d.buffer.text(), "foo = foo + foo;");
        d.redo();
        assert_eq!(d.buffer.text(), "bar = bar + bar;");
    }

    #[test]
    fn select_all_occurrences_puts_a_cursor_on_each() {
        let mut d = doc("let x = 1;\nlet y = 2;\nlet z = 3;\n");
        d.move_to(Position::new(0, 0));
        d.select_all_occurrences();
        // "let" appears three times: primary plus two secondaries.
        assert_eq!(d.selected_text().as_deref(), Some("let"));
        assert_eq!(d.cursors.len(), 2);
        d.insert_text("var");
        assert_eq!(d.buffer.text(), "var x = 1;\nvar y = 2;\nvar z = 3;\n");
    }

    #[test]
    fn add_cursor_below_stacks_carets_and_types_across_lines() {
        let mut d = doc("aaa\naaa\naaa\n");
        d.move_to(Position::new(0, 3));
        d.add_cursor_below();
        d.add_cursor_below();
        // Primary on line 0, carets on lines 1 and 2: three in total.
        assert_eq!(d.cursors.len() + 1, 3);
        d.type_char('!');
        assert_eq!(d.buffer.text(), "aaa!\naaa!\naaa!\n");
        // Selections are distinct and the whole thing undoes in one step.
        d.undo();
        assert_eq!(d.buffer.text(), "aaa\naaa\naaa\n");
    }

    #[test]
    fn multi_cursor_backspace_and_newline() {
        let mut d = doc("ab\nab\nab\n");
        d.move_to(Position::new(0, 1));
        d.add_cursor_below();
        d.add_cursor_below();
        d.backspace();
        assert_eq!(d.buffer.text(), "b\nb\nb\n");

        // Undo the backspaces, then insert a newline at every caret.
        d.undo();
        assert_eq!(d.buffer.text(), "ab\nab\nab\n");
        d.move_to(Position::new(0, 2));
        d.add_cursor_below();
        d.add_cursor_below();
        d.insert_newline();
        assert_eq!(d.buffer.text(), "ab\n\nab\n\nab\n\n");
    }

    #[test]
    fn multi_cursor_indent_is_grouped_and_deduplicates_rows() {
        let mut d = doc("one\ntwo\nthree\n");
        d.move_to(Position::new(0, 0));
        d.add_cursor_below();
        d.add_cursor_below();
        // Two cursors land on the same line as the primary is moved; ensure a
        // row is never indented twice.
        d.add_cursor_below();
        d.indent();
        assert_eq!(d.buffer.text(), "    one\n    two\n    three\n");
        d.undo();
        assert_eq!(d.buffer.text(), "one\ntwo\nthree\n");
    }

    #[test]
    fn clear_selection_ends_a_multi_cursor_session() {
        let mut d = doc("a a a");
        d.move_to(Position::new(0, 0));
        d.select_all_occurrences();
        assert!(d.has_multiple_cursors());
        d.clear_selection();
        assert!(!d.has_multiple_cursors());
    }

    #[test]
    fn moving_the_cursor_collapses_multi_cursor() {
        let mut d = doc("a a a");
        d.move_to(Position::new(0, 0));
        d.select_all_occurrences();
        assert!(d.has_multiple_cursors());
        d.move_left(false);
        assert!(!d.has_multiple_cursors());
    }

    #[test]
    fn multi_cursor_delete_line_removes_each_line() {
        let mut d = doc("one\ntwo\nthree\n");
        d.move_to(Position::new(0, 0));
        d.add_cursor_below();
        // Carets on rows 0 and 1; `delete_line` acts on the primary only and
        // collapses, so exactly one line is removed.
        d.delete_line();
        assert_eq!(d.cursors.len(), 0);
        assert_eq!(d.buffer.text(), "two\nthree\n");
    }

    #[test]
    fn auto_indent_on_newline() {
        let mut d = doc("    let x = 1;");
        d.move_end(false);
        d.insert_newline();
        assert_eq!(d.buffer.text(), "    let x = 1;\n    ");
    }

    #[test]
    fn detects_and_uses_the_file_indentation_unit() {
        // Two-space JavaScript.
        let javascript = doc("function f() {\n  return 1;\n}");
        assert_eq!(javascript.indent_width(), 2);

        // Four-space Rust.
        let rust = doc("fn main() {\n    let x = 1;\n}");
        assert_eq!(rust.indent_width(), 4);

        // Tab-indented files keep the default width for inserted spaces.
        let tabbed = doc("fn main() {\n\tlet x = 1;\n}");
        assert_eq!(tabbed.indent_width(), INDENT_WIDTH);

        // New lines adopt the detected unit after an opening brace.
        let mut d = doc("function f() {\n  return 1;\n}");
        d.move_to(Position::new(0, 14));
        d.insert_newline();
        assert_eq!(d.buffer.text(), "function f() {\n  \n  return 1;\n}");
    }

    #[test]
    fn indent_and_outdent_use_the_detected_unit() {
        let mut d = doc("function f() {\n  return 1;\n}");
        d.selection = Some(Selection::new(Position::new(1, 0)));
        d.cursor = Position::new(1, 2);
        d.outdent();
        assert_eq!(d.buffer.text(), "function f() {\nreturn 1;\n}");

        d.indent();
        assert_eq!(d.buffer.text(), "function f() {\n  return 1;\n}");
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
    fn find_all_regex_matches_and_reports_errors() {
        let d = doc("let count = 42;\nlet total = 7;");
        let matches = d.find_all_regex(r"\d+", false).unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0], (Position::new(0, 12), Position::new(0, 14)));
        assert_eq!(matches[1], (Position::new(1, 12), Position::new(1, 13)));

        assert!(d.find_all_regex("(a|b)", false).is_err());
    }

    #[test]
    fn find_all_honours_case_and_whole_word() {
        let d = doc("Foo foo food foo");

        // Case-insensitive by default for the find bar.
        assert_eq!(d.find_all_with("foo", false, false).len(), 4);
        assert_eq!(d.find_all_with("foo", true, false).len(), 3);

        // Whole word excludes `food`.
        let whole = d.find_all_with("foo", false, true);
        assert_eq!(
            whole,
            vec![
                (Position::new(0, 0), Position::new(0, 3)),
                (Position::new(0, 4), Position::new(0, 7)),
                (Position::new(0, 13), Position::new(0, 16)),
            ]
        );
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
    fn wrapped_vertical_movement_steps_through_visual_rows() {
        let mut d = doc("abcdefghij klmnopqrst uvwxyz\nshort\n");
        d.wrap_width = 10;
        d.move_to(Position::new(0, 0));

        // Down moves within the same logical line, visual row by visual row.
        d.move_down(false);
        assert_eq!(d.cursor.row, 0, "still on the wrapped line");
        assert!(d.cursor.col > 0, "moved to a later visual row");
        let second = d.cursor.col;

        d.move_down(false);
        assert_eq!(d.cursor.row, 0);
        assert!(d.cursor.col > second);

        // One more step leaves the wrapped line for the next logical line.
        d.move_down(false);
        assert_eq!(d.cursor.row, 1);
    }

    #[test]
    fn wrapped_movement_keeps_the_display_column() {
        // The two logical lines wrap to different widths; the display column
        // (not the character column) should be preserved across the step.
        let mut d = doc("aaaaaaaaaa bbbb\nccccc ddddd eeeee\n");
        d.wrap_width = 11;
        d.move_to(Position::new(0, 7)); // display column 7
        d.move_down(false); // next visual row of line 0
        assert_eq!(d.cursor.row, 0);
        d.move_down(false); // line 1, first visual row
        assert_eq!(d.cursor.row, 1);
        assert_eq!(d.display_col_of(1, d.cursor.col), 7);
    }

    #[test]
    fn wrapping_does_not_change_the_text_or_undo() {
        let mut d = doc("one long line that will wrap\n");
        d.wrap_width = 8;
        assert!(d.row_segments(0, 8).len() > 1);
        d.move_end(false);
        d.insert_text("!");
        assert_eq!(d.buffer.text(), "one long line that will wrap!\n");
        d.undo();
        assert_eq!(d.buffer.text(), "one long line that will wrap\n");
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
    fn goto_matching_bracket_jumps_to_the_partner() {
        let service = crate::language::LanguageService::builtin();
        let provider = service.provider(LanguageId::Rust);
        let mut d = doc("fn main() {}\n");

        d.move_to(Position::new(0, 7)); // on `(`
        d.goto_matching_bracket(provider);
        assert_eq!(d.cursor, Position::new(0, 8));

        d.move_to(Position::new(0, 10)); // on `{`
        d.goto_matching_bracket(provider);
        assert_eq!(d.cursor, Position::new(0, 11));
    }

    #[test]
    fn delete_line_removes_the_current_line() {
        let mut d = doc("one\ntwo\nthree\n");
        d.move_to(Position::new(1, 1));
        d.delete_line();
        assert_eq!(d.buffer.text(), "one\nthree\n");
        assert_eq!(d.cursor, Position::new(1, 0));
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
    fn coalesced_backspace_undo_restores_the_cursor() {
        // Regression: merging backspaces overwrote `cursor_before`, so undo
        // left the cursor one character short of where the run began.
        let mut d = doc("abcd");
        d.move_end(false);
        d.backspace();
        d.backspace();
        d.undo();
        assert_eq!(d.cursor, Position::new(0, 4));
        d.redo();
        assert_eq!(d.cursor, Position::new(0, 2));
    }

    #[test]
    fn undoing_to_the_save_point_marks_the_document_clean() {
        let mut d = doc("hello");
        d.mark_saved();
        assert!(!d.is_dirty());

        d.insert_text("!");
        assert!(d.is_dirty());
        d.undo();
        assert!(
            !d.is_dirty(),
            "undoing the only edit returns to the save point"
        );

        d.redo();
        assert!(d.is_dirty(), "redoing past the save point is dirty again");
    }

    #[test]
    fn indent_skips_a_trailing_line_selected_at_column_zero() {
        // Regression: Shift+Down leaves the cursor at column 0 of the row after
        // the highlighted lines; that row used to be indented too.
        let mut d = doc("a\nb\nc");
        d.selection = Some(Selection::new(Position::new(0, 0)));
        d.cursor = Position::new(2, 0);
        d.indent();
        assert_eq!(d.buffer.text(), "    a\n    b\nc");
    }

    #[test]
    fn delete_line_skips_a_trailing_line_selected_at_column_zero() {
        let mut d = doc("a\nb\nc");
        d.selection = Some(Selection::new(Position::new(0, 0)));
        d.cursor = Position::new(2, 0);
        d.delete_line();
        assert_eq!(d.buffer.text(), "c");
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

    #[test]
    fn diagnostics_apply_and_reject_stale_results() {
        let mut d = doc("fn main() {\n");
        let diagnostic = Diagnostic::new(
            TextPos::new(0, 10),
            TextPos::new(0, 11),
            Severity::Error,
            "unclosed `{`",
        );

        d.set_diagnostics_revision(2);
        assert!(d.apply_diagnostics(2, vec![diagnostic.clone()]));
        assert_eq!(d.diagnostics().len(), 1);
        assert_eq!(d.diagnostic_counts(), (1, 0));
        assert_eq!(d.diagnostic_severity_on_line(0), Some(Severity::Error));
        assert_eq!(d.diagnostic_severity_on_line(1), None);

        // A result from an older request must not replace the current one.
        assert!(!d.apply_diagnostics(1, vec![diagnostic]));
    }

    #[test]
    fn editing_clears_diagnostics_and_marks_them_dirty() {
        let mut d = doc("fn main() {\n");
        d.set_diagnostics_revision(1);
        d.apply_diagnostics(
            1,
            vec![Diagnostic::new(
                TextPos::new(0, 10),
                TextPos::new(0, 11),
                Severity::Error,
                "unclosed `{`",
            )],
        );
        assert!(!d.diagnostics().is_empty());
        assert!(!d.diagnostics_dirty());

        d.insert_text("// hi\n");
        assert!(d.diagnostics().is_empty());
        assert!(d.diagnostics_dirty());
    }

    #[test]
    fn lsp_diagnostics_replace_builtin_ones() {
        let mut d = doc("fn main() {\n}\n");
        d.set_diagnostics_revision(1);
        d.apply_diagnostics(
            1,
            vec![Diagnostic::new(
                TextPos::new(0, 0),
                TextPos::new(0, 1),
                Severity::Error,
                "builtin",
            )],
        );
        assert!(!d.diagnostics_from_lsp());

        d.set_lsp_diagnostics(vec![
            Diagnostic::new(
                TextPos::new(1, 0),
                TextPos::new(1, 1),
                Severity::Warning,
                "lsp warn",
            ),
            Diagnostic::new(
                TextPos::new(0, 2),
                TextPos::new(0, 3),
                Severity::Error,
                "lsp err",
            ),
        ]);
        assert!(d.diagnostics_from_lsp());
        // Sorted by position regardless of arrival order.
        assert_eq!(d.diagnostics()[0].message, "lsp err");
        assert_eq!(d.diagnostic_counts(), (1, 1));

        // Handing ownership back clears them and re-arms the built-in pass.
        d.use_builtin_diagnostics();
        assert!(!d.diagnostics_from_lsp());
        assert!(d.diagnostics().is_empty());
        assert_eq!(d.diagnostics_revision(), 0);
    }

    #[test]
    fn reload_from_disk_replaces_contents_and_cleans() {
        let dir = std::env::temp_dir().join(format!("koda-reload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("main.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();

        let mut d = Document::from_path(&path).unwrap();
        d.move_to(Position::new(0, 3));
        d.insert_text("// edit\n");
        assert!(d.is_dirty());
        assert!(d.can_undo());

        std::fs::write(&path, "fn b() {}\n").unwrap();
        assert!(d.reload_from_disk().unwrap());
        assert_eq!(d.buffer.text(), "fn b() {}\n");
        assert!(!d.is_dirty());
        assert!(!d.can_undo());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
