//! Extracted from `app/mod.rs`: see `docs/DECISIONS.md`.

use super::*;

impl App {
    pub(super) fn handle_paste(&mut self, text: &str) {
        match &mut self.overlay {
            Overlay::Prompt(prompt) => {
                prompt.input.push_str(text);
                return;
            }
            Overlay::Picker(picker) => {
                picker.query.push_str(text);
                picker.refilter();
                return;
            }
            Overlay::Help(_) => return,
            Overlay::DirPicker(_) => return,
            Overlay::Diff(_) => return,
            Overlay::NewProject(flow) => {
                if flow.step == NewProjectStep::Name {
                    flow.name.push_str(text);
                    flow.error = None;
                }
                return;
            }
            Overlay::None => {}
        }
        self.completion = None;
        self.signature = None;
        if self.search.open {
            match self.search.field {
                SearchField::Query => self.search.query.push_str(text),
                SearchField::Replacement => self.search.replacement.push_str(text),
            }
            self.refresh_search_matches();
            self.jump_to_first_from_cursor();
            return;
        }
        if self.focus == Focus::Editor {
            let text = text.to_string();
            self.with_doc(|doc| doc.insert_text(&text));
        }
    }

    /// Shortcuts that work regardless of focus.
    pub(super) fn handle_global_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key_shift(&key);
        if key.code == KeyCode::F(1) {
            self.completion = None;
            self.toggle_help();
            return true;
        }
        if key.code == KeyCode::F(5) {
            self.completion = None;
            self.refresh_workspace();
            return true;
        }
        if key.code == KeyCode::F(8) {
            self.completion = None;
            self.goto_diagnostic(if shift { -1 } else { 1 });
            return true;
        }
        if key.code == KeyCode::F(3) {
            // Repeat the last find, reopening the bar when it was closed.
            self.completion = None;
            if !self.search.open {
                self.search.open = true;
                self.refresh_search_matches();
            }
            if shift {
                self.find_previous();
            } else {
                self.find_next();
            }
            return true;
        }
        // Pane and view shortcuts work from any surface.
        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char('v') | KeyCode::Char('V') => {
                    self.execute_command(ids::SPLIT);
                    return true;
                }
                KeyCode::Char('o') | KeyCode::Char('O') => {
                    self.execute_command(ids::FOCUS_PANE);
                    return true;
                }
                KeyCode::Char('d') | KeyCode::Char('D') => {
                    self.execute_command(ids::DIFF);
                    return true;
                }
                KeyCode::Char('i') | KeyCode::Char('I') => {
                    self.execute_command(ids::TOGGLE_INLINE_DIAGNOSTICS);
                    return true;
                }
                KeyCode::Char('z') | KeyCode::Char('Z') => {
                    self.execute_command(ids::TOGGLE_WRAP);
                    return true;
                }
                _ => {}
            }
        }
        if !ctrl {
            return false;
        }
        // Any Ctrl chord other than the completion trigger dismisses completion.
        if key.code != KeyCode::Char(' ') {
            self.completion = None;
        }
        match key.code {
            KeyCode::Char(c) => {
                match (c.to_ascii_lowercase(), shift) {
                    (' ', _) => self.execute_command(ids::COMPLETE),
                    ('q', _) => self.request_quit(),
                    ('s', true) => self.execute_command(ids::SAVE_AS),
                    ('s', false) => self.execute_command(ids::SAVE),
                    ('n', _) => self.execute_command(ids::NEW_FILE),
                    ('p', true) => self.open_command_palette(),
                    ('p', false) => self.open_quick_open(),
                    ('o', _) => self.execute_command(ids::OPEN),
                    ('f', true) => self.execute_command(ids::PROJECT_SEARCH),
                    ('f', false) => self.open_search(false),
                    ('h', true) => self.open_hover(),
                    ('h', false) => self.open_search(true),
                    ('g', true) => self.execute_command(ids::CHANGED_FILES),
                    ('g', false) => self.execute_command(ids::GOTO_LINE),
                    ('b', _) => self.toggle_tree(),
                    ('e', _) => self.focus_tree(),
                    ('w', _) => self.execute_command(ids::CLOSE_TAB),
                    ('t', _) => self.open_workspace_symbols(),
                    ('m', true) => self.execute_command(ids::DIAGNOSTICS_LIST),
                    ('i', true) => self.execute_command(ids::FORMAT),
                    _ => return false,
                }
                true
            }
            KeyCode::Tab => {
                if shift {
                    self.previous_tab();
                } else {
                    self.next_tab();
                }
                true
            }
            KeyCode::PageUp => {
                self.previous_tab();
                true
            }
            KeyCode::PageDown => {
                self.next_tab();
                true
            }
            _ => false,
        }
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) {
        // A hover popup is informational; the next key dismisses it.
        if self.hover.is_some() {
            self.hover = None;
            if key.code == KeyCode::Esc {
                return;
            }
        }
        // Signature help persists while literal characters and Backspace keep
        // the call open; any other key dismisses it.
        let keep_signature = matches!(key.code, KeyCode::Char(_) | KeyCode::Backspace)
            && !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT);
        if !keep_signature {
            self.signature = None;
            self.signature_request = None;
        }
        if self.handle_global_key(key) {
            return;
        }
        if !self.overlay.is_none() {
            self.completion = None;
            self.handle_overlay_key(key);
            return;
        }
        if self.search.open {
            self.completion = None;
            self.handle_search_key(key);
            return;
        }
        if self.completion.is_some() && self.handle_completion_key(key) {
            return;
        }
        match self.focus {
            Focus::FileTree => self.handle_tree_key(key),
            Focus::Editor if self.editor.is_empty() => self.handle_welcome_key(key),
            Focus::Editor => self.handle_editor_key(key),
        }
    }
}
