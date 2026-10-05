//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

impl App {
    /// Open completion for the word being typed.
    pub(super) fn open_completion(&mut self) {
        self.open_completion_inner(true);
    }

    /// Replace the typed prefix with the selected completion.
    pub(super) fn accept_completion(&mut self) {
        let Some(state) = self.completion.take() else {
            return;
        };
        self.completion_due = None;
        let Some(item) = state.selected_item().cloned() else {
            return;
        };
        let Some(doc) = self.editor.active_document() else {
            return;
        };
        let cursor = doc.clamped_cursor();
        let line = doc.buffer.line_text(cursor.row);
        let chars: Vec<char> = line.chars().collect();
        let end = cursor.col.min(chars.len());
        let mut start = end;
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }
        let start = Position::new(cursor.row, start);
        let end = Position::new(cursor.row, end);
        self.with_doc(|doc| doc.replace_range(start, end, &item.label));
    }

    /// Build and show the completion popup.
    ///
    /// `manual` distinguishes an explicit `Ctrl+Space` (which reports when there
    /// is nothing to offer) from the automatic, typing-driven offer, which stays
    /// silent rather than interrupting with an empty popup.
    pub(super) fn open_completion_inner(&mut self, manual: bool) -> bool {
        let (language, text, cursor) = match self.editor.active_document() {
            Some(doc) => (doc.buffer.language, doc.buffer.text(), doc.clamped_cursor()),
            None => return false,
        };

        // After `.` or `::` the buffer's own words are noise: only the language
        // (and the server, when attached) know the members.
        let member = self.cursor_in_member_access();
        let mut pool: Vec<Completion> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for item in self
            .language
            .provider(language)
            .completions(&text, cursor.row, cursor.col)
        {
            if seen.insert(item.label.clone()) {
                pool.push(item);
            }
        }
        if !member {
            for item in document_words(&text) {
                if seen.insert(item.label.clone()) {
                    pool.push(item);
                }
            }
        }

        let prefix = self.completion_prefix();
        let state = CompletionState::new(pool, prefix.clone());
        let lsp_available = self.lsp_supports(RequestKind::Completion);

        if state.items.is_empty() {
            if manual {
                // With a server attached, open an empty popup and let its
                // response fill it in.
                if lsp_available {
                    self.completion = Some(state);
                    self.request_lsp_completion();
                    return true;
                }
                self.set_status("No completions");
            }
            return false;
        }

        // An automatic popup whose only candidate is the word already being
        // typed would just sit there; leave it closed.
        let only_self = state.items.len() == 1 && state.items[0].label == prefix;
        if !manual && only_self && !lsp_available {
            return false;
        }

        self.completion = Some(state);
        self.request_lsp_completion();
        true
    }

    /// The identifier characters immediately before the cursor.
    pub(super) fn completion_prefix(&self) -> String {
        let Some(doc) = self.editor.active_document() else {
            return String::new();
        };
        let cursor = doc.clamped_cursor();
        let line = doc.buffer.line_text(cursor.row);
        let chars: Vec<char> = line.chars().collect();
        let end = cursor.col.min(chars.len());
        let mut start = end;
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }
        chars[start..end].iter().collect()
    }

    /// After typing a word character, re-filter an open popup and schedule an
    /// automatic offer.
    pub(super) fn after_word_char_typed(&mut self) {
        if self.completion.is_some() {
            self.refresh_completion();
        }
        self.schedule_auto_completion();
    }

    /// Ask the language server for signature help at the cursor.
    pub(super) fn request_lsp_signature(&mut self) {
        let Some(language) = self.lsp_language() else {
            self.signature = None;
            return;
        };
        let Some((path, row, col)) = self.lsp_target_for(RequestKind::SignatureHelp) else {
            self.signature = None;
            return;
        };
        let col = self.lsp_col(language, &path, row, col);
        if let Some(server) = self
            .lsp
            .get_mut(&language)
            .and_then(|job| job.server.as_mut())
        {
            self.signature_request = server
                .signature_help(&path, row, col)
                .map(|id| (language, id));
        }
    }

    /// Whether the cursor is inside a comment or a string, according to the
    /// provider's highlighting. Used to keep automatic completion quiet in prose
    /// and literals.
    ///
    /// The probe is the start of the identifier being typed (or the character
    /// before the cursor when there is none), because the cursor sits just past
    /// the last typed character and a highlight span's end is exclusive.
    pub(super) fn cursor_in_comment_or_string(&mut self) -> bool {
        let Some((id, row, probe)) = self.editor.active_document().map(|doc| {
            let cursor = doc.clamped_cursor();
            let line = doc.buffer.line_text(cursor.row);
            let chars: Vec<char> = line.chars().collect();
            let end = cursor.col.min(chars.len());
            let mut start = end;
            while start > 0 && is_word_char(chars[start - 1]) {
                start -= 1;
            }
            let probe = if start < end {
                start
            } else {
                end.saturating_sub(1)
            };
            (doc.buffer.language, cursor.row, probe)
        }) else {
            return false;
        };
        let service = Arc::clone(&self.language);
        let provider = service.provider(id);
        let Some(doc) = self.editor.active_document_mut() else {
            return false;
        };
        matches!(
            doc.token_kind_at(provider, row, probe),
            TokenKind::Comment | TokenKind::String
        )
    }

    /// Whether the cursor sits just after a member-access operator (`.` or `::`).
    pub(super) fn cursor_in_member_access(&self) -> bool {
        let Some(doc) = self.editor.active_document() else {
            return false;
        };
        let cursor = doc.clamped_cursor();
        let line = doc.buffer.line_text(cursor.row);
        let chars: Vec<char> = line.chars().collect();
        let mut start = cursor.col.min(chars.len());
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }
        start > 0 && matches!(chars[start - 1], '.' | ':')
    }

    /// Schedule an automatic completion a short pause after the last keystroke.
    ///
    /// Suppressed inside comments and strings, and pinned to the current
    /// document, cursor and buffer version so the offer is dropped if the user
    /// moves or edits before it fires.
    pub(super) fn schedule_auto_completion(&mut self) {
        if self.cursor_in_comment_or_string() {
            self.completion = None;
            self.completion_due = None;
            return;
        }
        let (doc_index, cursor, version) = match self.editor.active_document() {
            Some(doc) => (
                self.editor.active_index(),
                doc.clamped_cursor(),
                doc.buffer.version,
            ),
            None => {
                self.completion_due = None;
                return;
            }
        };
        self.completion_due = Some(PendingCompletion {
            due_at: Instant::now() + AUTOCOMPLETE_DELAY,
            doc: doc_index,
            cursor,
            version,
        });
    }

    /// Ask the language server for completions at the cursor.
    pub(super) fn request_lsp_completion(&mut self) {
        let Some(language) = self.lsp_language() else {
            return;
        };
        let Some((path, row, col)) = self.lsp_target_for(RequestKind::Completion) else {
            return;
        };
        let col = self.lsp_col(language, &path, row, col);
        if let Some(server) = self
            .lsp
            .get_mut(&language)
            .and_then(|job| job.server.as_mut())
        {
            self.completion_request = server.completion(&path, row, col).map(|id| (language, id));
        }
    }

    /// React to a literal character typed in the editor: drive completion and
    /// signature help together.
    pub(super) fn after_typed_char(&mut self, c: char) {
        if is_word_char(c) {
            self.after_word_char_typed();
        } else {
            // Whitespace or punctuation ends the word: dismiss the completion
            // popup so it does not linger with an empty prefix.
            self.completion = None;
            self.completion_due = None;
        }

        if matches!(c, '(' | ',') {
            self.request_lsp_signature();
        } else if !is_word_char(c) && c != ' ' {
            // A closing bracket or any other punctuation leaves the arguments.
            self.signature = None;
            self.signature_request = None;
        }
    }

    /// Show information about the symbol under the cursor.
    pub(super) fn open_hover(&mut self) {
        let (language, text, cursor) = match self.editor.active_document() {
            Some(doc) => (doc.buffer.language, doc.buffer.text(), doc.clamped_cursor()),
            None => return,
        };
        let local = self
            .language
            .provider(language)
            .hover(&text, cursor.row, cursor.col);
        self.completion = None;
        self.hover = local.map(|hover| HoverState {
            title: hover.title,
            kind: hover.kind,
            body: hover.body,
        });

        // Ask the language server for a richer answer when one is attached.
        if let Some((path, row, col)) = self.lsp_target_for(RequestKind::Hover) {
            let col = self.lsp_col(language, &path, row, col);
            if let Some(server) = self.active_server_mut() {
                self.hover_request = server.hover(&path, row, col).map(|id| (language, id));
            }
            return;
        }
        if self.hover.is_none() {
            self.set_status("No symbol under the cursor");
        }
    }

    /// Re-filter the open completion for the current prefix, closing it when
    /// nothing matches.
    pub(super) fn refresh_completion(&mut self) {
        let prefix = self.completion_prefix();
        if let Some(state) = self.completion.as_mut() {
            state.set_prefix(prefix);
        }
        if self
            .completion
            .as_ref()
            .is_some_and(|state| state.items.is_empty())
        {
            self.completion = None;
        }
    }

    /// After an edit that may shorten the typed prefix, re-filter an open popup
    /// and schedule the next automatic offer. Does nothing when the popup is
    /// closed, since Backspace should not by itself pop one open.
    pub(super) fn after_completion_edit(&mut self) {
        if self.completion.is_none() {
            return;
        }
        if self.completion_prefix().is_empty() {
            self.completion = None;
            self.completion_due = None;
            return;
        }
        self.refresh_completion();
        self.schedule_auto_completion();
    }

    /// Open the popup once the typing pause has elapsed.
    pub(super) fn poll_auto_completion(&mut self) -> bool {
        let Some(pending) = self.completion_due.as_ref() else {
            return false;
        };
        if Instant::now() < pending.due_at {
            return false;
        }
        // Only offer if nothing else has happened: the same document is active,
        // the cursor has not moved and the buffer has not changed. This drops
        // the timer when the user dismissed the popup, opened an overlay or
        // switched tabs.
        let valid = self.overlay.is_none()
            && !self.search.open
            && self.focus == Focus::Editor
            && self.editor.active_index() == pending.doc
            && self.editor.active_document().is_some_and(|doc| {
                doc.clamped_cursor() == pending.cursor && doc.buffer.version == pending.version
            });
        self.completion_due = None;
        if !valid {
            return false;
        }
        if self.completion.is_some() {
            // The popup is open and already re-filtered; refresh the server's
            // candidates for the new prefix.
            self.request_lsp_completion();
            return false;
        }
        self.open_completion_inner(false)
    }
}
