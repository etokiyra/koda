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
    pub(super) fn handle_editor_key(&mut self, key: KeyEvent) {
        let shift = key_shift(&key);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        // Signature help is transient: any key that is not a literal character
        // dismisses it.
        let typing = matches!(key.code, KeyCode::Char(_)) && !ctrl && !alt;
        if !typing {
            self.signature = None;
            self.signature_request = None;
        }

        match key.code {
            KeyCode::Char(c) => {
                if ctrl {
                    match c.to_ascii_lowercase() {
                        'z' => {
                            if shift {
                                self.with_doc(|d| d.redo());
                            } else {
                                self.with_doc(|d| d.undo());
                            }
                        }
                        'y' => self.with_doc(|d| d.redo()),
                        'a' => self.with_doc(|d| d.select_all()),
                        'c' => self.copy(),
                        'x' => self.cut(),
                        'v' => self.paste(),
                        'd' if shift => self.with_doc(|d| d.duplicate_line()),
                        'd' => self.with_doc(|d| d.select_next_occurrence()),
                        'l' if shift => self.with_doc(|d| d.select_all_occurrences()),
                        'k' if shift => self.with_doc(|d| d.delete_line()),
                        'm' => self.goto_matching_bracket(),
                        '/' => self.toggle_comment(),
                        '.' => self.code_actions(),
                        _ => {}
                    }
                } else if alt {
                    match c {
                        'y' | 'Y' => self.yank_pop(),
                        'm' | 'M' => self.goto_matching_bracket(),
                        _ => {}
                    }
                } else {
                    self.with_doc(|d| d.type_char(c));
                    self.after_typed_char(c);
                }
            }
            KeyCode::Enter => self.with_doc(|d| d.insert_newline()),
            KeyCode::Tab => self.with_doc(|d| d.indent()),
            KeyCode::BackTab => self.with_doc(|d| d.outdent()),
            KeyCode::Backspace => self.with_doc(|d| d.backspace()),
            KeyCode::Delete => self.with_doc(|d| d.delete_forward()),
            KeyCode::Left => self.with_doc(|d| {
                if ctrl {
                    d.move_word_left(shift);
                } else {
                    d.move_left(shift);
                }
            }),
            KeyCode::Right => self.with_doc(|d| {
                if ctrl {
                    d.move_word_right(shift);
                } else {
                    d.move_right(shift);
                }
            }),
            KeyCode::Up => {
                if ctrl && alt {
                    self.with_doc(|d| d.add_cursor_above());
                } else if alt {
                    self.with_doc(|d| d.move_line_up());
                } else {
                    self.with_doc(|d| d.move_up(shift));
                }
            }
            KeyCode::Down => {
                if ctrl && alt {
                    self.with_doc(|d| d.add_cursor_below());
                } else if alt {
                    if shift {
                        self.with_doc(|d| d.duplicate_line());
                    } else {
                        self.with_doc(|d| d.move_line_down());
                    }
                } else {
                    self.with_doc(|d| d.move_down(shift));
                }
            }
            KeyCode::Home => self.with_doc(|d| {
                if ctrl {
                    d.move_document_start(shift);
                } else {
                    d.move_home(shift);
                }
            }),
            KeyCode::End => self.with_doc(|d| {
                if ctrl {
                    d.move_document_end(shift);
                } else {
                    d.move_end(shift);
                }
            }),
            KeyCode::PageUp => self.page_move(-1),
            KeyCode::PageDown => self.page_move(1),
            KeyCode::Esc => self.with_doc(|d| d.clear_selection()),
            _ => {}
        }
    }

    pub(super) fn handle_tree_key(&mut self, key: KeyEvent) {
        if self.tree_filter.is_some() {
            self.handle_tree_filter_key(key);
            return;
        }
        match key.code {
            KeyCode::Up => self.workspace.tree.select_up(),
            KeyCode::Down => self.workspace.tree.select_down(),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char(' ') => {
                if let Some(path) = self.workspace.tree.activate_selected() {
                    self.open_path(path);
                }
            }
            KeyCode::Left => self.workspace.tree.collapse_selected(),
            KeyCode::Char('/') => self.open_tree_filter(),
            KeyCode::Char('.') => self.toggle_hidden(),
            KeyCode::Esc => self.focus = Focus::Editor,
            _ => {}
        }
    }

    pub(super) fn handle_tree_filter_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.tree_filter = None,
            KeyCode::Up => {
                if let Some(filter) = self.tree_filter.as_mut() {
                    filter.move_up();
                }
            }
            KeyCode::Down => {
                if let Some(filter) = self.tree_filter.as_mut() {
                    filter.move_down();
                }
            }
            KeyCode::Enter => {
                let path = self
                    .tree_filter
                    .as_ref()
                    .and_then(|filter| filter.selected_path().map(Path::to_path_buf));
                self.tree_filter = None;
                if let Some(path) = path {
                    self.open_path(path);
                }
            }
            KeyCode::Backspace => {
                if let Some(filter) = self.tree_filter.as_mut() {
                    filter.backspace();
                }
            }
            KeyCode::Char(c) if !ctrl => {
                if let Some(filter) = self.tree_filter.as_mut() {
                    filter.push_char(c);
                }
            }
            _ => {}
        }
    }
    pub(super) fn handle_overlay_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        enum Outcome {
            Nothing,
            Close,
            Run(PickerAction, Option<String>),
            Submit(PromptKind, String),
            Stage(PathBuf, bool),
            OpenDir(PathBuf),
            CreateProject {
                parent: PathBuf,
                name: String,
                language: LanguageId,
            },
        }

        let outcome = match &mut self.overlay {
            Overlay::None => Outcome::Nothing,
            Overlay::Picker(picker) => match key.code {
                KeyCode::Esc => Outcome::Close,
                KeyCode::Up => {
                    picker.move_up();
                    Outcome::Nothing
                }
                KeyCode::Down => {
                    picker.move_down();
                    Outcome::Nothing
                }
                KeyCode::Enter => match picker.selected_item() {
                    Some(item) => Outcome::Run(
                        item.action.clone(),
                        item.hint.clone().filter(|_| !item.enabled),
                    ),
                    None => Outcome::Close,
                },
                KeyCode::Backspace => {
                    picker.backspace();
                    Outcome::Nothing
                }
                KeyCode::Char(' ') if picker.title == "Changed Files" => {
                    match picker.selected_item().map(|item| item.action.clone()) {
                        Some(PickerAction::OpenPath(path)) => {
                            let staged = !self.workspace.git.is_staged(&path);
                            Outcome::Stage(path, staged)
                        }
                        _ => Outcome::Nothing,
                    }
                }
                KeyCode::Char('d') if picker.title == "Changed Files" => {
                    match picker.selected_item().map(|item| item.action.clone()) {
                        Some(PickerAction::OpenPath(path)) => {
                            let staged = self.workspace.git.is_staged(&path);
                            Outcome::Run(PickerAction::ShowDiff { path, staged }, None)
                        }
                        _ => Outcome::Nothing,
                    }
                }
                KeyCode::Char(c) if !ctrl => {
                    picker.push_char(c);
                    Outcome::Nothing
                }
                _ => Outcome::Nothing,
            },
            Overlay::Prompt(prompt) => match key.code {
                KeyCode::Esc => Outcome::Close,
                KeyCode::Enter => Outcome::Submit(prompt.kind, prompt.input.clone()),
                KeyCode::Backspace => {
                    prompt.backspace();
                    Outcome::Nothing
                }
                KeyCode::Char(c) if !ctrl => {
                    prompt.push_char(c);
                    Outcome::Nothing
                }
                _ => Outcome::Nothing,
            },
            Overlay::DirPicker(picker) => match key.code {
                KeyCode::Esc => Outcome::Close,
                KeyCode::Up => {
                    picker.browser.move_up();
                    Outcome::Nothing
                }
                KeyCode::Down => {
                    picker.browser.move_down();
                    Outcome::Nothing
                }
                KeyCode::Backspace | KeyCode::Left => {
                    if let Some(parent) = picker.browser.current.parent() {
                        picker.browser.current = parent.to_path_buf();
                        picker.browser.selected = 0;
                        picker.browser.refresh();
                    }
                    Outcome::Nothing
                }
                KeyCode::Enter => match picker.browser.activate() {
                    Some(directory) => Outcome::OpenDir(directory),
                    None => Outcome::Nothing,
                },
                _ => Outcome::Nothing,
            },
            Overlay::NewProject(flow) => match flow.step {
                NewProjectStep::Parent => match key.code {
                    KeyCode::Esc => Outcome::Close,
                    KeyCode::Up => {
                        flow.browser.move_up();
                        Outcome::Nothing
                    }
                    KeyCode::Down => {
                        flow.browser.move_down();
                        Outcome::Nothing
                    }
                    KeyCode::Backspace | KeyCode::Left => {
                        if let Some(parent) = flow.browser.current.parent() {
                            flow.browser.current = parent.to_path_buf();
                            flow.browser.selected = 0;
                            flow.browser.refresh();
                        }
                        Outcome::Nothing
                    }
                    KeyCode::Enter => {
                        if let Some(directory) = flow.browser.activate() {
                            flow.parent = directory;
                            flow.step = NewProjectStep::Name;
                            flow.error = None;
                        }
                        Outcome::Nothing
                    }
                    _ => Outcome::Nothing,
                },
                NewProjectStep::Name => match key.code {
                    KeyCode::Esc => {
                        flow.step = NewProjectStep::Parent;
                        flow.error = None;
                        Outcome::Nothing
                    }
                    KeyCode::Enter => {
                        match validate_project_name(&flow.parent, &flow.name) {
                            Ok(()) => {
                                flow.name = flow.name.trim().to_string();
                                flow.step = NewProjectStep::Language;
                                flow.error = None;
                            }
                            Err(message) => flow.error = Some(message),
                        }
                        Outcome::Nothing
                    }
                    KeyCode::Backspace => {
                        flow.name.pop();
                        flow.error = None;
                        Outcome::Nothing
                    }
                    KeyCode::Char(c) if !ctrl => {
                        flow.name.push(c);
                        flow.error = None;
                        Outcome::Nothing
                    }
                    _ => Outcome::Nothing,
                },
                NewProjectStep::Language => match key.code {
                    KeyCode::Esc => {
                        flow.step = NewProjectStep::Name;
                        flow.error = None;
                        Outcome::Nothing
                    }
                    KeyCode::Up => {
                        flow.language = flow.language.saturating_sub(1);
                        flow.error = None;
                        Outcome::Nothing
                    }
                    KeyCode::Down => {
                        if flow.language + 1 < create::CREATABLE.len() {
                            flow.language += 1;
                        }
                        flow.error = None;
                        Outcome::Nothing
                    }
                    KeyCode::Enter => {
                        let index = flow.language.min(create::CREATABLE.len().saturating_sub(1));
                        let language = create::CREATABLE[index];
                        match validate_project_name(&flow.parent, &flow.name) {
                            Ok(()) => Outcome::CreateProject {
                                parent: flow.parent.clone(),
                                name: flow.name.trim().to_string(),
                                language,
                            },
                            Err(message) => {
                                flow.error = Some(message);
                                Outcome::Nothing
                            }
                        }
                    }
                    _ => Outcome::Nothing,
                },
            },
            Overlay::Help(help) => match key.code {
                KeyCode::Esc => Outcome::Close,
                KeyCode::Up => {
                    help.scroll = help.scroll.saturating_sub(1);
                    Outcome::Nothing
                }
                KeyCode::Down => {
                    help.scroll = help.scroll.saturating_add(1);
                    Outcome::Nothing
                }
                KeyCode::PageUp => {
                    help.scroll = help.scroll.saturating_sub(8);
                    Outcome::Nothing
                }
                KeyCode::PageDown => {
                    help.scroll = help.scroll.saturating_add(8);
                    Outcome::Nothing
                }
                _ => Outcome::Nothing,
            },
            Overlay::Diff(diff) => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => Outcome::Close,
                KeyCode::Up => {
                    diff.scroll_by(-1);
                    Outcome::Nothing
                }
                KeyCode::Down => {
                    diff.scroll_by(1);
                    Outcome::Nothing
                }
                KeyCode::PageUp => {
                    diff.scroll_by(-16);
                    Outcome::Nothing
                }
                KeyCode::PageDown => {
                    diff.scroll_by(16);
                    Outcome::Nothing
                }
                KeyCode::Home => {
                    diff.scroll = 0;
                    Outcome::Nothing
                }
                KeyCode::End => {
                    diff.scroll = diff.lines.len();
                    Outcome::Nothing
                }
                _ => Outcome::Nothing,
            },
        };

        match outcome {
            Outcome::Nothing => {}
            Outcome::Close => self.overlay = Overlay::None,
            Outcome::Run(action, hint) => {
                self.overlay = Overlay::None;
                match hint {
                    Some(hint) => self.set_status(hint),
                    None => self.run_picker_action(action),
                }
            }
            Outcome::Submit(kind, input) => {
                self.overlay = Overlay::None;
                self.submit_prompt(kind, input);
            }
            Outcome::Stage(path, staged) => self.dispatch_stage(path, staged, true),
            Outcome::OpenDir(directory) => {
                self.overlay = Overlay::None;
                self.open_project(directory);
            }
            Outcome::CreateProject {
                parent,
                name,
                language,
            } => {
                self.overlay = Overlay::None;
                self.start_project_creation(parent, name, language);
            }
        }
    }
}
