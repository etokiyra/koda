//! Application state and the event loop.
//!
//! This module is the conductor: it wires workspace, editor, language service,
//! commands and UI together, and translates terminal events into actions.

pub mod overlay;

use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::commands::{CommandRegistry, ids};
use crate::editor::{Document, Editor, Position, Selection};
use crate::filesystem;
use crate::language::{Capability, LanguageId, LanguageService};
use crate::project::Workspace;
use crate::terminal;
use crate::ui;
use overlay::{Overlay, Picker, PickerAction, PickerItem, Prompt, PromptKind, Search, SearchField};

/// Which surface receives keyboard input when no overlay is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Editor,
    FileTree,
}

/// A short-lived status message.
#[derive(Default)]
pub struct Status {
    pub message: String,
    pub error: bool,
    expires_at: Option<Instant>,
}

/// The root application object.
pub struct App {
    pub workspace: Workspace,
    pub editor: Editor,
    pub language: LanguageService,
    pub commands: CommandRegistry,
    pub overlay: Overlay,
    pub search: Search,
    pub clipboard: String,
    pub tree_visible: bool,
    pub focus: Focus,
    pub status: Status,
    /// Height of the editor viewport, updated during rendering.
    pub viewport_height: usize,
    pub should_quit: bool,
    quit_armed: bool,
}

impl App {
    /// Run Koda. `target` is the optional CLI path argument.
    pub fn start(target: Option<String>) -> io::Result<()> {
        let target = target.map(PathBuf::from);
        let mut app = App::new(target.as_deref())?;
        let mut terminal = terminal::init()?;
        let result = app.run(&mut terminal);
        terminal::restore();
        result
    }

    /// Build the application state for a target path.
    pub fn new(target: Option<&std::path::Path>) -> io::Result<Self> {
        let workspace = Workspace::open(target)?;
        let mut app = App {
            workspace,
            editor: Editor::new(),
            language: LanguageService::builtin(),
            commands: CommandRegistry::builtin(),
            overlay: Overlay::None,
            search: Search::default(),
            clipboard: String::new(),
            tree_visible: true,
            focus: Focus::Editor,
            status: Status::default(),
            viewport_height: 20,
            should_quit: false,
            quit_armed: false,
        };

        if let Some(path) = target
            && path.is_file()
        {
            app.open_path(path.to_path_buf());
        }
        Ok(app)
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.should_quit {
            self.tick_status();
            terminal.draw(|frame| ui::render(frame, self))?;
            if event::poll(Duration::from_millis(250))? {
                match event::read()? {
                    Event::Key(key)
                        if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                    {
                        self.handle_key(key)
                    }
                    Event::Paste(text) => self.handle_paste(&text),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    // ----------------------------------------------------------------------
    // Event handling
    // ----------------------------------------------------------------------

    fn handle_key(&mut self, key: KeyEvent) {
        if self.handle_global_key(key) {
            return;
        }
        if !self.overlay.is_none() {
            self.handle_overlay_key(key);
            return;
        }
        if self.search.open {
            self.handle_search_key(key);
            return;
        }
        match self.focus {
            Focus::FileTree => self.handle_tree_key(key),
            Focus::Editor => self.handle_editor_key(key),
        }
    }

    /// Shortcuts that work regardless of focus.
    fn handle_global_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        if !ctrl {
            return false;
        }
        match key.code {
            KeyCode::Char(c) => {
                match (c.to_ascii_lowercase(), shift) {
                    ('q', _) => self.request_quit(),
                    ('s', _) => self.execute_command(ids::SAVE),
                    ('p', true) => self.open_command_palette(),
                    ('p', false) => self.open_quick_open(),
                    ('o', _) => self.execute_command(ids::OPEN),
                    ('f', _) => self.open_search(false),
                    ('h', _) => self.open_search(true),
                    ('g', _) => self.execute_command(ids::GOTO_LINE),
                    ('b', _) => self.toggle_tree(),
                    ('w', _) => self.execute_command(ids::CLOSE_TAB),
                    _ => return false,
                }
                true
            }
            KeyCode::Tab => {
                if shift {
                    self.editor.previous_tab();
                } else {
                    self.editor.next_tab();
                }
                true
            }
            _ => false,
        }
    }

    fn handle_paste(&mut self, text: &str) {
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
            Overlay::None => {}
        }
        if self.search.open {
            match self.search.field {
                SearchField::Query => self.search.query.push_str(text),
                SearchField::Replacement => self.search.replacement.push_str(text),
            }
            self.refresh_search_matches();
            self.jump_to_match(0);
            return;
        }
        if self.focus == Focus::Editor {
            let text = text.to_string();
            self.with_doc(|doc| doc.insert_text(&text));
        }
    }

    fn handle_editor_key(&mut self, key: KeyEvent) {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

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
                        '/' => self.toggle_comment(),
                        _ => {}
                    }
                } else if !alt {
                    self.with_doc(|d| d.insert_char(c));
                }
            }
            KeyCode::Enter => self.with_doc(|d| d.insert_newline()),
            KeyCode::Tab => self.with_doc(|d| d.insert_text("    ")),
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
            KeyCode::Up => self.with_doc(|d| d.move_up(shift)),
            KeyCode::Down => self.with_doc(|d| d.move_down(shift)),
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

    fn handle_tree_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.workspace.tree.select_up(),
            KeyCode::Down => self.workspace.tree.select_down(),
            KeyCode::Enter | KeyCode::Right => {
                if let Some(path) = self.workspace.tree.activate_selected() {
                    self.open_path(path);
                }
            }
            KeyCode::Left => self.workspace.tree.collapse_selected(),
            KeyCode::Esc => self.focus = Focus::Editor,
            _ => {}
        }
    }

    fn handle_overlay_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        enum Outcome {
            Nothing,
            Close,
            Run(PickerAction),
            Submit(PromptKind, String),
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
                    Some(item) => Outcome::Run(item.action.clone()),
                    None => Outcome::Close,
                },
                KeyCode::Backspace => {
                    picker.backspace();
                    Outcome::Nothing
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
        };

        match outcome {
            Outcome::Nothing => {}
            Outcome::Close => self.overlay = Overlay::None,
            Outcome::Run(action) => {
                self.overlay = Overlay::None;
                self.run_picker_action(action);
            }
            Outcome::Submit(kind, input) => {
                self.overlay = Overlay::None;
                self.submit_prompt(kind, input);
            }
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Esc => self.search.close(),
            KeyCode::Enter => {
                if self.search.replace_mode && self.search.field == SearchField::Replacement {
                    self.replace_current();
                } else if shift {
                    self.find_previous();
                } else {
                    self.find_next();
                }
            }
            KeyCode::Tab => {
                if self.search.replace_mode {
                    self.search.field = match self.search.field {
                        SearchField::Query => SearchField::Replacement,
                        SearchField::Replacement => SearchField::Query,
                    };
                }
            }
            KeyCode::Up => self.find_previous(),
            KeyCode::Down => self.find_next(),
            KeyCode::Backspace => {
                match self.search.field {
                    SearchField::Query => {
                        self.search.query.pop();
                    }
                    SearchField::Replacement => {
                        self.search.replacement.pop();
                    }
                }
                self.refresh_search_matches();
                self.jump_to_match(0);
            }
            KeyCode::Char(c) if !ctrl => {
                match self.search.field {
                    SearchField::Query => self.search.query.push(c),
                    SearchField::Replacement => self.search.replacement.push(c),
                }
                self.refresh_search_matches();
                self.jump_to_match(0);
            }
            _ => {}
        }
    }

    // ----------------------------------------------------------------------
    // Commands
    // ----------------------------------------------------------------------

    /// Execute a command by id.
    pub fn execute_command(&mut self, id: &str) {
        match id {
            ids::SAVE => self.save(),
            ids::OPEN => self.open_prompt(PromptKind::OpenPath, "Open file", "path/to/file.rs"),
            ids::QUICK_OPEN => self.open_quick_open(),
            ids::CLOSE_TAB => self.close_tab(),
            ids::QUIT => self.request_quit(),
            ids::UNDO => self.with_doc(|d| d.undo()),
            ids::REDO => self.with_doc(|d| d.redo()),
            ids::SELECT_ALL => self.with_doc(|d| d.select_all()),
            ids::COPY => self.copy(),
            ids::CUT => self.cut(),
            ids::PASTE => self.paste(),
            ids::FIND => self.open_search(false),
            ids::REPLACE => self.open_search(true),
            ids::GOTO_LINE => self.open_prompt(PromptKind::GotoLine, "Go to line", "42"),
            ids::TOGGLE_COMMENT => self.toggle_comment(),
            ids::TOGGLE_TREE => self.toggle_tree(),
            ids::NEXT_TAB => self.editor.next_tab(),
            ids::PREV_TAB => self.editor.previous_tab(),
            ids::PALETTE => self.open_command_palette(),
            ids::FORMAT
            | ids::GOTO_DEFINITION
            | ids::FIND_REFERENCES
            | ids::SHOW_SYMBOLS
            | ids::RENAME
            | ids::CODE_ACTIONS => self.report_language_capability(id),
            _ => {}
        }
    }

    fn run_picker_action(&mut self, action: PickerAction) {
        match action {
            PickerAction::Command(id) => self.execute_command(id),
            PickerAction::OpenPath(path) => self.open_path(path),
        }
    }

    fn submit_prompt(&mut self, kind: PromptKind, input: String) {
        let input = input.trim().to_string();
        match kind {
            PromptKind::GotoLine => match input.parse::<usize>() {
                Ok(line) => self.with_doc(|d| d.go_to_line(line)),
                Err(_) => self.set_error("Not a valid line number"),
            },
            PromptKind::OpenPath => {
                if input.is_empty() {
                    self.set_error("No path provided");
                } else {
                    self.open_path(self.resolve_path(&input));
                }
            }
            PromptKind::SaveAs => {
                if input.is_empty() {
                    self.set_error("No path provided");
                } else {
                    self.save_as(self.resolve_path(&input));
                }
            }
        }
    }

    fn resolve_path(&self, input: &str) -> PathBuf {
        let path = PathBuf::from(input);
        if path.is_absolute() {
            path
        } else {
            self.workspace.root().join(path)
        }
    }

    // ----------------------------------------------------------------------
    // Editing helpers
    // ----------------------------------------------------------------------

    fn with_doc<F: FnOnce(&mut Document)>(&mut self, action: F) {
        if let Some(doc) = self.editor.active_document_mut() {
            action(doc);
        }
        self.after_edit();
    }

    fn after_edit(&mut self) {
        if self.search.open {
            self.refresh_search_matches();
        }
    }

    fn page_move(&mut self, direction: i32) {
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

    fn copy(&mut self) {
        let selected = self
            .editor
            .active_document()
            .and_then(|doc| doc.selected_text());
        match selected {
            Some(text) => {
                self.clipboard = text.clone();
                terminal::set_clipboard(&text);
                self.set_status(format!("Copied {} character(s)", text.chars().count()));
            }
            None => self.set_status("Nothing selected"),
        }
    }

    fn cut(&mut self) {
        let text = self.editor.active_document_mut().and_then(|doc| {
            let text = doc.selected_text()?;
            doc.delete_selection();
            Some(text)
        });
        match text {
            Some(text) => {
                self.clipboard = text.clone();
                terminal::set_clipboard(&text);
                self.set_status(format!("Cut {} character(s)", text.chars().count()));
                self.after_edit();
            }
            None => self.set_status("Nothing to cut"),
        }
    }

    fn paste(&mut self) {
        let text = self.clipboard.clone();
        if text.is_empty() {
            self.set_status("Clipboard is empty");
            return;
        }
        self.with_doc(|doc| doc.insert_text(&text));
    }

    fn toggle_comment(&mut self) {
        let Some(language) = self.editor.active_document().map(|d| d.buffer.language) else {
            return;
        };
        let prefix = self.language.provider(language).line_comment().to_string();

        let rows = match self
            .editor
            .active_document()
            .and_then(|d| d.selection_range())
        {
            Some((start, end)) => (start.row..=end.row).collect::<Vec<_>>(),
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

    // ----------------------------------------------------------------------
    // File and tab operations
    // ----------------------------------------------------------------------

    /// Open a file, focusing the editor and applying language detection.
    pub fn open_path(&mut self, path: PathBuf) {
        match self.editor.open_path(&path) {
            Ok(_) => {
                self.detect_language_for_active();
                self.workspace.tree.select_path(&path);
                self.focus = Focus::Editor;
                self.set_status(format!("Opened {}", path.display()));
            }
            Err(err) => self.set_error(format!("Could not open {}: {err}", path.display())),
        }
    }

    fn save(&mut self) {
        let has_path = self
            .editor
            .active_document()
            .map(|doc| doc.buffer.path.is_some())
            .unwrap_or(false);
        if !has_path {
            self.open_prompt(PromptKind::SaveAs, "Save as", "path/to/file");
            return;
        }
        match self.editor.save_active() {
            Ok(true) => {
                self.workspace.refresh_git();
                self.quit_armed = false;
                self.set_status("Saved");
            }
            Ok(false) => self.set_status("Nothing to save"),
            Err(err) => self.set_error(format!("Save failed: {err}")),
        }
    }

    fn save_as(&mut self, path: PathBuf) {
        let result = self
            .editor
            .active_document_mut()
            .map(|doc| doc.buffer.save_as(&path));
        match result {
            Some(Ok(true)) => {
                self.detect_language_for_active();
                self.workspace.refresh_git();
                self.set_status(format!("Saved {}", path.display()));
            }
            Some(Err(err)) => self.set_error(format!("Save failed: {err}")),
            _ => self.set_error("Nothing to save"),
        }
    }

    fn close_tab(&mut self) {
        let index = self.editor.active_index();
        if self
            .editor
            .documents
            .get(index)
            .map(|doc| doc.is_dirty())
            .unwrap_or(false)
        {
            self.set_error("Unsaved changes — save first (Ctrl+S)");
            return;
        }
        self.editor.close(index);
        self.set_status("Tab closed");
    }

    fn detect_language_for_active(&mut self) {
        let markers = self.workspace.marker_names();
        if let Some(doc) = self.editor.active_document_mut()
            && let Some(path) = doc.buffer.path.clone()
        {
            let result = self.language.detect_file(&path, &markers);
            doc.set_language(result.language);
        }
    }

    // ----------------------------------------------------------------------
    // Search
    // ----------------------------------------------------------------------

    fn open_search(&mut self, replace: bool) {
        self.search.open = true;
        self.search.replace_mode = replace;
        self.search.field = SearchField::Query;
        if !replace {
            self.search.replacement.clear();
        }
        self.refresh_search_matches();
        self.jump_to_match(0);
    }

    fn refresh_search_matches(&mut self) {
        let query = self.search.query.clone();
        let matches = if query.is_empty() {
            Vec::new()
        } else {
            self.editor
                .active_document()
                .map(|doc| doc.find_all(&query))
                .unwrap_or_default()
        };
        self.search.matches = matches;
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

    fn jump_to_match(&mut self, index: usize) {
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

    fn find_next(&mut self) {
        if self.search.matches.is_empty() {
            return;
        }
        let next = match self.search.current {
            Some(current) => (current + 1) % self.search.matches.len(),
            None => 0,
        };
        self.jump_to_match(next);
    }

    fn find_previous(&mut self) {
        if self.search.matches.is_empty() {
            return;
        }
        let previous = match self.search.current {
            Some(0) | None => self.search.matches.len() - 1,
            Some(current) => current - 1,
        };
        self.jump_to_match(previous);
    }

    fn replace_current(&mut self) {
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
        let next = current.min(self.search.matches.len().saturating_sub(1));
        self.jump_to_match(next);
    }

    // ----------------------------------------------------------------------
    // Overlays
    // ----------------------------------------------------------------------

    fn open_command_palette(&mut self) {
        let items = self
            .commands
            .all()
            .iter()
            .map(|command| PickerItem {
                label: command.palette_label(),
                detail: command.category.to_string(),
                shortcut: command.shortcut.unwrap_or("").to_string(),
                action: PickerAction::Command(command.id),
            })
            .collect();
        let mut picker = Picker::new("Command Palette", "Type a command…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    fn open_quick_open(&mut self) {
        let root = self.workspace.root().to_path_buf();
        let files = filesystem::collect_files(&root, 8000);
        let items = files
            .into_iter()
            .map(|path| {
                let label = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let detail = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                PickerItem {
                    label,
                    detail,
                    shortcut: String::new(),
                    action: PickerAction::OpenPath(path),
                }
            })
            .collect();
        let mut picker = Picker::new("Quick Open", "Type a file name…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    fn open_prompt(&mut self, kind: PromptKind, label: &str, placeholder: &str) {
        self.overlay = Overlay::Prompt(Prompt::new(kind, label, placeholder));
    }

    // ----------------------------------------------------------------------
    // Misc
    // ----------------------------------------------------------------------

    fn toggle_tree(&mut self) {
        self.tree_visible = !self.tree_visible;
        self.focus = if self.tree_visible {
            Focus::FileTree
        } else {
            Focus::Editor
        };
    }

    fn request_quit(&mut self) {
        if self.editor.has_unsaved() && !self.quit_armed {
            self.quit_armed = true;
            self.set_error("Unsaved changes — Ctrl+S to save, Ctrl+Q again to quit");
            return;
        }
        self.should_quit = true;
    }

    fn report_language_capability(&mut self, id: &str) {
        let (capability, label) = match id {
            ids::FORMAT => (Capability::Formatting, "Formatting"),
            ids::GOTO_DEFINITION => (Capability::GotoDefinition, "Go to definition"),
            ids::FIND_REFERENCES => (Capability::GotoReference, "Find references"),
            ids::SHOW_SYMBOLS => (Capability::DocumentSymbols, "Symbol navigation"),
            ids::RENAME => (Capability::Rename, "Rename"),
            ids::CODE_ACTIONS => (Capability::CodeActions, "Code actions"),
            _ => return,
        };
        let language = self
            .editor
            .active_document()
            .map(|doc| doc.buffer.language)
            .unwrap_or(LanguageId::Unknown);
        let available = self
            .language
            .provider(language)
            .capabilities()
            .contains(&capability);
        if available {
            self.set_status(format!("{label} is starting for {}", language.name()));
        } else {
            self.set_status(format!(
                "{label} is not available for {} yet",
                language.name()
            ));
        }
    }

    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status.message = message.into();
        self.status.error = false;
        self.status.expires_at = Some(Instant::now() + Duration::from_secs(4));
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.status.message = message.into();
        self.status.error = true;
        self.status.expires_at = Some(Instant::now() + Duration::from_secs(8));
    }

    /// Expire stale status messages so the language/git summary returns.
    fn tick_status(&mut self) {
        if let Some(expires_at) = self.status.expires_at
            && Instant::now() >= expires_at
        {
            self.status.message.clear();
            self.status.error = false;
            self.status.expires_at = None;
        }
    }

    /// The message shown in the status bar.
    pub fn status_message(&self) -> Option<&str> {
        if self.status.message.is_empty() {
            None
        } else {
            Some(self.status.message.as_str())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koda-app-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        fs::write(dir.join("src/main.rs"), "fn main() {\n    let x = 1;\n}\n").unwrap();
        dir
    }

    #[test]
    fn opens_rust_file_and_detects_language() {
        let dir = temp_project("open");
        let file = dir.join("src/main.rs");
        let app = App::new(Some(&file)).unwrap();
        assert_eq!(app.editor.len(), 1);
        assert_eq!(
            app.editor.active_document().unwrap().buffer.language,
            LanguageId::Rust
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn saves_and_marks_clean() {
        let dir = temp_project("save");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();
        app.editor
            .active_document_mut()
            .unwrap()
            .insert_text("// hello\n");
        assert!(app.editor.active_document().unwrap().is_dirty());

        app.execute_command(ids::SAVE);
        assert!(!app.editor.active_document().unwrap().is_dirty());
        assert!(fs::read_to_string(&file).unwrap().starts_with("// hello\n"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn toggle_comment_adds_prefix() {
        let dir = temp_project("comment");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();
        app.editor
            .active_document_mut()
            .unwrap()
            .move_to(Position::new(1, 0));

        app.execute_command(ids::TOGGLE_COMMENT);

        let doc = app.editor.active_document().unwrap();
        assert!(doc.buffer.line_text(1).contains("// let x = 1;"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn command_palette_opens() {
        let dir = temp_project("palette");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();
        app.execute_command(ids::PALETTE);
        assert!(!app.overlay.is_none());
        fs::remove_dir_all(&dir).ok();
    }
}
