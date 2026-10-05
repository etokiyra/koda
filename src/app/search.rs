//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

impl App {
    pub(super) fn replace_current(&mut self) {
        let Some(current) = self.search.current else {
            self.find_next();
            return;
        };
        let Some(&(start, end)) = self.search.matches.get(current) else {
            return;
        };
        let replacement = self.search.replacement.clone();
        if let Some(doc) = self.editor.active_document_mut() {
            doc.replace_range(start, end, &replacement);
        }
        self.refresh_search_matches();
        self.schedule_diagnostics();
        let next = current.min(self.search.matches.len().saturating_sub(1));
        self.jump_to_match(next);
    }

    pub(super) fn jump_to_match(&mut self, index: usize) {
        if self.search.matches.is_empty() {
            return;
        }
        let index = index.min(self.search.matches.len() - 1);
        self.search.current = Some(index);
        let (start, end) = self.search.matches[index];
        if let Some(doc) = self.editor.active_document_mut() {
            doc.selection = Some(Selection::new(start));
            doc.cursor = end;
            doc.preferred_col = None;
        }
    }

    pub(super) fn find_next(&mut self) {
        if self.search.matches.is_empty() {
            return;
        }
        let next = match self.search.current {
            Some(current) => (current + 1) % self.search.matches.len(),
            None => 0,
        };
        self.jump_to_match(next);
    }

    /// Jump to the first match at or after the cursor, wrapping around.
    pub(super) fn jump_to_first_from_cursor(&mut self) {
        if self.search.matches.is_empty() {
            return;
        }
        let cursor = self
            .editor
            .active_document()
            .map(|doc| doc.clamped_cursor())
            .unwrap_or_default();
        let index = self
            .search
            .matches
            .iter()
            .position(|(start, _)| *start >= cursor)
            .unwrap_or(0);
        self.jump_to_match(index);
    }

    pub(super) fn refresh_search_matches(&mut self) {
        let query = self.search.query.clone();
        let case_sensitive = self.search.case_sensitive;
        let whole_word = self.search.whole_word;
        let regex = self.search.regex;

        let result: Result<Vec<_>, String> = if query.is_empty() {
            Ok(Vec::new())
        } else if let Some(doc) = self.editor.active_document() {
            if regex {
                doc.find_all_regex(&query, case_sensitive)
            } else {
                Ok(doc.find_all_with(&query, case_sensitive, whole_word))
            }
        } else {
            Ok(Vec::new())
        };

        self.search.matches = match result {
            Ok(matches) => {
                self.search.regex_error = None;
                matches
            }
            Err(message) => {
                self.search.regex_error = Some(message);
                Vec::new()
            }
        };
        if self
            .search
            .current
            .is_none_or(|current| current >= self.search.matches.len())
        {
            self.search.current = if self.search.matches.is_empty() {
                None
            } else {
                Some(0)
            };
        }
    }

    pub(super) fn open_search(&mut self, replace: bool) {
        self.search.open = true;
        self.search.replace_mode = replace;
        self.search.field = SearchField::Query;
        if !replace {
            self.search.replacement.clear();
        }
        // Prefill from a single-line selection, so Ctrl+F searches the word the
        // user already highlighted.
        if let Some(text) = self
            .editor
            .active_document()
            .and_then(|doc| doc.selected_text())
        {
            let trimmed = text.trim();
            if !trimmed.is_empty() && !trimmed.contains('\n') {
                self.search.query = trimmed.to_string();
            }
        }
        self.refresh_search_matches();
        self.jump_to_first_from_cursor();
    }

    pub(super) fn find_previous(&mut self) {
        if self.search.matches.is_empty() {
            return;
        }
        let previous = match self.search.current {
            Some(0) | None => self.search.matches.len() - 1,
            Some(current) => current - 1,
        };
        self.jump_to_match(previous);
    }

    /// Replace every current match in the active file as one undoable edit.
    pub(super) fn replace_all(&mut self) {
        if !self.search.open || self.search.query.is_empty() {
            self.set_status("Open find and type a query first");
            return;
        }
        self.refresh_search_matches();
        let matches = self.search.matches.clone();
        if matches.is_empty() {
            self.set_status("No matches to replace");
            return;
        }

        let replacement = self.search.replacement.clone();
        let (text, end) = match self.editor.active_document() {
            Some(doc) => {
                let last = doc.buffer.len_lines().saturating_sub(1);
                let end = Position::new(last, doc.buffer.line_char_len(last));
                (doc.buffer.text(), end)
            }
            None => return,
        };

        // Map `(row, col)` positions to character offsets.
        let mut chars: Vec<char> = text.chars().collect();
        let total = chars.len();
        let mut line_starts = vec![0usize];
        for (index, ch) in chars.iter().enumerate() {
            if *ch == '\n' {
                line_starts.push(index + 1);
            }
        }
        let offset = |position: Position| -> usize {
            let start = line_starts.get(position.row).copied().unwrap_or(total);
            (start + position.col).min(total)
        };

        // Apply from the end so earlier offsets stay valid. Matches never
        // overlap, so this is safe.
        let mut replaced = 0usize;
        for (start, finish) in matches.iter().rev() {
            let from = offset(*start);
            let to = offset(*finish);
            if from >= to {
                continue;
            }
            chars.splice(from..to, replacement.chars());
            replaced += 1;
        }
        if replaced == 0 {
            self.set_status("No matches to replace");
            return;
        }

        let new_text: String = chars.into_iter().collect();
        self.with_doc(|doc| doc.replace_range(Position::zero(), end, &new_text));
        self.refresh_search_matches();
        self.set_status(format!("Replaced {replaced} occurrence(s)"));
    }
}
