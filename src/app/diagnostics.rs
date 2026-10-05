//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

impl App {
    /// Queue a diagnostics recompute when the active document changed.
    pub(super) fn schedule_diagnostics(&mut self) {
        if self
            .editor
            .active_document()
            .is_some_and(|doc| doc.diagnostics_dirty())
        {
            self.diagnostics_dirty_at = Some(Instant::now());
        }
    }

    /// Dispatch a debounced recompute once typing pauses, or run the first pass
    /// for a document that has not been analysed yet.
    pub(super) fn poll_diagnostics(&mut self) {
        if let Some(at) = self.diagnostics_dirty_at
            && at.elapsed() >= DIAGNOSTICS_DEBOUNCE
        {
            self.diagnostics_dirty_at = None;
            self.dispatch_diagnostics();
            return;
        }

        let needs_first_pass = self.editor.active_document().is_some_and(|doc| {
            doc.diagnostics_revision() == 0
                && self
                    .language
                    .provider(doc.buffer.language)
                    .capabilities()
                    .contains(&Capability::Diagnostics)
        });
        if needs_first_pass {
            self.dispatch_diagnostics();
        }
    }

    /// Send the active document's text to the background worker for analysis.
    pub(super) fn dispatch_diagnostics(&mut self) {
        let (path, language) = match self.editor.active_document() {
            Some(doc) => (doc.buffer.path.clone(), doc.buffer.language),
            None => return,
        };

        self.diagnostics_seq += 1;
        let revision = self.diagnostics_seq;
        if let Some(doc) = self.editor.active_document_mut() {
            doc.set_diagnostics_revision(revision);
        }

        // If a language server owns this document, stream the change to it and
        // let it publish fresh diagnostics.
        let owned_by_lsp = path.as_deref().is_some_and(|path| {
            self.lsp
                .get(&language)
                .and_then(|job| job.server.as_ref())
                .is_some_and(|server| server.is_ready() && server.has_open_document(path))
        });
        if owned_by_lsp {
            let text = self
                .editor
                .active_document()
                .map(|doc| doc.buffer.text())
                .unwrap_or_default();
            if let Some(server) = self
                .lsp
                .get_mut(&language)
                .and_then(|job| job.server.as_mut())
                && let Some(path) = path.as_deref()
            {
                server.did_change(path, &text);
            }
            return;
        }

        let supported = self
            .language
            .provider(language)
            .capabilities()
            .contains(&Capability::Diagnostics);
        // Unsupported languages are marked analysed so we do not retry forever.
        if !supported {
            return;
        }
        let Some(path) = path else {
            return;
        };
        let text = self
            .editor
            .active_document()
            .map(|doc| doc.buffer.text())
            .unwrap_or_default();
        self.background.diagnose(path, language, text, revision);
    }

    /// List every diagnostic across open files, jumping on selection.
    pub(super) fn open_diagnostics_list(&mut self) {
        let mut items = Vec::new();
        for doc in &self.editor.documents {
            let Some(path) = doc.buffer.path.clone() else {
                continue;
            };
            let file = doc.file_name();
            for diagnostic in doc.diagnostics() {
                let position = Position::new(diagnostic.start.line, diagnostic.start.col);
                let detail = format!(
                    "{file}:{}:{}  ·  {}",
                    diagnostic.start.line + 1,
                    diagnostic.start.col + 1,
                    diagnostic.severity.label()
                );
                items.push(PickerItem::new(
                    diagnostic.message.clone(),
                    detail,
                    PickerAction::Reveal {
                        path: path.clone(),
                        position,
                    },
                ));
            }
        }
        if items.is_empty() {
            self.set_status("No diagnostics");
            return;
        }
        let mut picker = Picker::new("Diagnostics", "Filter problems…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Jump to the next (`direction > 0`) or previous diagnostic, wrapping.
    pub(super) fn goto_diagnostic(&mut self, direction: i32) {
        let Some(doc) = self.editor.active_document() else {
            return;
        };
        if doc.diagnostics().is_empty() {
            self.set_status("No diagnostics");
            return;
        }
        let cursor = doc.clamped_cursor();
        let targets: Vec<Position> = doc
            .diagnostics()
            .iter()
            .map(|diagnostic| Position::new(diagnostic.start.line, diagnostic.start.col))
            .collect();
        let target = if direction >= 0 {
            targets
                .iter()
                .copied()
                .find(|position| *position > cursor)
                .unwrap_or(targets[0])
        } else {
            targets
                .iter()
                .copied()
                .rev()
                .find(|position| *position < cursor)
                .unwrap_or_else(|| *targets.last().expect("non-empty"))
        };

        let center = self.viewport_height / 2;
        self.with_doc(|doc| {
            doc.move_to(target);
            doc.scroll_top = target.row.saturating_sub(center);
        });

        let message = self.editor.active_document().and_then(|doc| {
            doc.diagnostics()
                .iter()
                .find(|diagnostic| {
                    diagnostic.start.line == target.row && diagnostic.start.col == target.col
                })
                .map(|diagnostic| (diagnostic.severity, diagnostic.message.clone()))
        });
        if let Some((severity, message)) = message {
            let text = format!("{}: {message}", severity.label());
            if severity == Severity::Error {
                self.set_error(text);
            } else {
                self.set_status(text);
            }
        }
    }
}
