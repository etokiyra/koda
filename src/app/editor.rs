//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

impl App {
    pub(super) fn resolve_path(&self, input: &str) -> PathBuf {
        let path = PathBuf::from(input);
        if path.is_absolute() {
            path
        } else {
            self.workspace.root().join(path)
        }
    }

    pub(super) fn page_move(&mut self, direction: i32) {
        let step = self.viewport_height.max(1);
        self.with_doc(|doc| {
            for _ in 0..step {
                if direction < 0 {
                    doc.move_up(false);
                } else {
                    doc.move_down(false);
                }
            }
        });
    }

    pub(super) fn goto_hunk(&mut self, direction: i32) {
        let Some(doc) = self.editor.active_document() else {
            return;
        };
        if doc.git_markers.is_empty() {
            self.set_status("No hunks");
            return;
        }
        let cursor = doc.clamped_cursor();
        let mut lines: Vec<usize> = doc.git_markers.iter().map(|marker| marker.line).collect();
        lines.sort();
        lines.dedup();
        let target = if direction >= 0 {
            lines
                .iter()
                .copied()
                .find(|line| *line > cursor.row)
                .unwrap_or(lines[0])
        } else {
            lines
                .iter()
                .copied()
                .rev()
                .find(|line| *line < cursor.row)
                .unwrap_or_else(|| *lines.last().expect("non-empty"))
        };
        let center = self.viewport_height / 2;
        self.with_doc(|doc| {
            doc.move_to(Position::new(target, 0));
            doc.scroll_top = target.saturating_sub(center);
        });
        self.set_status(format!("Hunk at line {}", target + 1));
    }

    pub(super) fn toggle_breakpoint(&mut self) {
        self.with_doc(|doc| doc.toggle_breakpoint());
        self.set_status("Breakpoint toggled");
    }

    pub(super) fn debug_start(&mut self) {
        let breakpoints: Vec<usize> = self
            .editor
            .active_document()
            .map(|doc| doc.breakpoints.iter().copied().collect())
            .unwrap_or_default();
        let adapter = std::env::var("KODA_DEBUG_ADAPTER").unwrap_or_else(|_| "gdb".to_string());
        match crate::debug::launch(&adapter, &[], &breakpoints) {
            Ok(message) => self.set_status(message),
            Err(err) => self.set_error(err),
        }
    }

    pub(super) fn toggle_comment(&mut self) {
        let Some(language) = self.editor.active_document().map(|d| d.buffer.language) else {
            return;
        };
        let prefix = self.language.provider(language).line_comment().to_string();

        let rows = match self
            .editor
            .active_document()
            .and_then(|d| d.selected_rows())
        {
            Some((start, end)) => (start..=end).collect::<Vec<_>>(),
            None => match self.editor.active_document() {
                Some(doc) => vec![doc.cursor.row],
                None => return,
            },
        };

        // Are all affected lines already commented?
        let all_commented = rows.iter().all(|&row| {
            self.editor
                .active_document()
                .map(|doc| {
                    let text = doc.buffer.line_text(row);
                    text.trim_start().starts_with(&prefix)
                })
                .unwrap_or(false)
        });

        self.with_doc(|doc| {
            for row in rows {
                let text = doc.buffer.line_text(row);
                let indent = text.chars().take_while(|c| c.is_whitespace()).count();
                if all_commented {
                    let after: Vec<char> = text.chars().skip(indent).collect();
                    let prefix_chars = prefix.chars().count();
                    let mut remove = prefix_chars.min(after.len());
                    if after.get(prefix_chars) == Some(&' ') {
                        remove += 1;
                    }
                    doc.replace_range(
                        Position::new(row, indent),
                        Position::new(row, indent + remove),
                        "",
                    );
                } else {
                    doc.replace_range(
                        Position::new(row, indent),
                        Position::new(row, indent),
                        &format!("{prefix} "),
                    );
                }
            }
        });
    }

    pub(super) fn after_edit(&mut self) {
        if self.search.open {
            self.refresh_search_matches();
        }
        self.schedule_diagnostics();
    }

    pub(super) fn paste(&mut self) {
        let text = self.clipboard.clone();
        if text.is_empty() {
            self.set_status("Clipboard is empty");
            return;
        }
        let Some(start) = self
            .editor
            .active_document()
            .map(|doc| doc.clamped_cursor())
        else {
            return;
        };
        let index = self.editor.active_index();
        self.with_doc(|doc| doc.insert_text(&text));
        if let Some(doc) = self.editor.active_document() {
            self.last_yank = Some(Yank {
                doc: index,
                start,
                end: doc.cursor,
                version: doc.buffer.version,
            });
        }
        self.kill_index = self.kill_ring.len().saturating_sub(1);
    }

    pub(super) fn cut(&mut self) {
        let text = self.editor.active_document_mut().and_then(|doc| {
            let text = doc.selected_text()?;
            doc.delete_selection();
            Some(text)
        });
        match text {
            Some(text) => {
                self.push_kill(&text);
                terminal::set_clipboard(&text);
                self.set_status(format!("Cut {} character(s)", text.chars().count()));
                self.after_edit();
            }
            None => self.set_status("Nothing to cut"),
        }
    }

    /// Record `text` as the newest kill, deduplicating consecutive copies.
    pub(super) fn push_kill(&mut self, text: &str) {
        if self.kill_ring.last().map(String::as_str) != Some(text) {
            self.kill_ring.push(text.to_string());
            if self.kill_ring.len() > 64 {
                self.kill_ring.remove(0);
            }
        }
        self.kill_index = self.kill_ring.len().saturating_sub(1);
        self.clipboard = text.to_string();
        self.last_yank = None;
    }

    /// Move the cursor to the bracket matching the one under it.
    pub(super) fn goto_matching_bracket(&mut self) {
        let Some(language) = self.editor.active_document().map(|doc| doc.buffer.language) else {
            return;
        };
        let provider = self.language.provider(language);
        if let Some(doc) = self.editor.active_document_mut() {
            doc.goto_matching_bracket(provider);
        }
    }

    /// Replace the last paste with an earlier kill, Emacs-style.
    pub(super) fn yank_pop(&mut self) {
        let Some(yank) = self.last_yank else {
            self.set_status("Nothing to yank-pop");
            return;
        };
        if self.editor.active_index() != yank.doc {
            self.set_status("Yank-pop applies to the file just pasted into");
            return;
        }
        let unchanged = self
            .editor
            .active_document()
            .is_some_and(|doc| doc.buffer.version == yank.version && doc.cursor == yank.end);
        if !unchanged {
            self.set_status("Yank-pop is only available right after a paste");
            return;
        }
        if self.kill_index == 0 || self.kill_ring.is_empty() {
            self.set_status("No earlier kill");
            return;
        }
        let next = self.kill_index - 1;
        let text = self.kill_ring[next].clone();
        let (start, end) = (yank.start, yank.end);
        self.with_doc(|doc| doc.replace_range(start, end, &text));
        if let Some(doc) = self.editor.active_document() {
            self.last_yank = Some(Yank {
                doc: yank.doc,
                start,
                end: doc.cursor,
                version: doc.buffer.version,
            });
        }
        self.kill_index = next;
        self.clipboard = text;
        self.set_status(format!(
            "Yank pop {}/{}",
            self.kill_ring.len() - next,
            self.kill_ring.len()
        ));
    }

    pub(super) fn copy(&mut self) {
        let selected = self
            .editor
            .active_document()
            .and_then(|doc| doc.selected_text());
        match selected {
            Some(text) => {
                self.push_kill(&text);
                terminal::set_clipboard(&text);
                self.set_status(format!("Copied {} character(s)", text.chars().count()));
            }
            None => self.set_status("Nothing selected"),
        }
    }

    pub(super) fn with_doc<F: FnOnce(&mut Document)>(&mut self, action: F) {
        if let Some(doc) = self.editor.active_document_mut() {
            action(doc);
        }
        self.after_edit();
    }
}
