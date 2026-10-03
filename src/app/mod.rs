//! Application state and the event loop.
//!
//! This module is the conductor: it wires workspace, editor, language service,
//! commands and UI together, and translates terminal events into actions.

pub mod overlay;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::background::{Background, Event as BackgroundEvent};
use crate::commands::{Command, CommandRegistry, ids};
use crate::editor::{Document, Editor, Position, Selection};
use crate::filesystem;
use crate::language::completion::{Completion, CompletionKind};
use crate::language::diagnostics::Severity;
use crate::language::format::FormatOutcome;
use crate::language::symbols::is_ident_char as is_word_char;
use crate::language::{Capability, LanguageId, LanguageService, WorkspaceSymbol};
use crate::project::Workspace;
use crate::terminal;
use crate::ui;
use overlay::{
    CompletionState, HoverState, Overlay, Picker, PickerAction, PickerItem, Prompt, PromptKind,
    Search, SearchField, TreeFilter,
};

/// How long typing must pause before diagnostics are recomputed. Short enough to
/// feel immediate, long enough not to reanalyse on every keystroke.
const DIAGNOSTICS_DEBOUNCE: Duration = Duration::from_millis(150);

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
    pub language: Arc<LanguageService>,
    background: Background,
    pub commands: CommandRegistry,
    pub overlay: Overlay,
    pub search: Search,
    pub clipboard: String,
    pub recent_files: Vec<PathBuf>,
    /// Inline file-tree filter, when active.
    pub tree_filter: Option<TreeFilter>,
    /// Completion popup, when open.
    pub completion: Option<CompletionState>,
    /// Hover popup, when open.
    pub hover: Option<HoverState>,
    /// Screen position of the editor cursor, updated during rendering.
    pub cursor_screen: Option<(u16, u16)>,
    pub tree_visible: bool,
    pub focus: Focus,
    pub status: Status,
    /// Height of the editor viewport, updated during rendering.
    pub viewport_height: usize,
    pub should_quit: bool,
    quit_armed: bool,
    /// Tab index armed for a forced close (dirty, first Ctrl+W).
    close_armed: Option<usize>,
    /// Monotonic id for diagnostics requests.
    diagnostics_seq: u64,
    /// When set, diagnostics should be recomputed once typing pauses.
    diagnostics_dirty_at: Option<Instant>,
    /// Monotonic id for format requests.
    format_seq: u64,
    /// The format request awaiting a result, if any.
    pending_format: Option<(PathBuf, u64)>,
    /// Monotonic id for workspace symbol scans.
    workspace_symbols_seq: u64,
    /// The workspace symbol scan awaiting a result, if any.
    pending_workspace_symbols: Option<u64>,
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
        let language = Arc::new(LanguageService::builtin());
        let background = Background::spawn(Arc::clone(&language));
        let workspace = Workspace::open(target)?;
        let mut app = App {
            workspace,
            editor: Editor::new(),
            language,
            background,
            commands: CommandRegistry::builtin(),
            overlay: Overlay::None,
            search: Search::default(),
            clipboard: String::new(),
            recent_files: Vec::new(),
            tree_filter: None,
            completion: None,
            hover: None,
            cursor_screen: None,
            tree_visible: true,
            focus: Focus::Editor,
            status: Status::default(),
            viewport_height: 20,
            should_quit: false,
            quit_armed: false,
            close_armed: None,
            diagnostics_seq: 0,
            diagnostics_dirty_at: None,
            format_seq: 0,
            pending_format: None,
            workspace_symbols_seq: 0,
            pending_workspace_symbols: None,
        };

        if let Some(path) = target
            && path.is_file()
        {
            app.open_path(path.to_path_buf());
        }
        app.request_git_refresh();
        // Apply the initial detection and git snapshot before the first frame so
        // startup is deterministic.
        app.pump_background(Duration::from_millis(300));
        Ok(app)
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        let mut needs_redraw = true;
        while !self.should_quit {
            // Only repaint when something changed: input arrived, background
            // work completed, a status message expired, or the terminal was
            // resized. An idle Koda does no work at all.
            self.poll_diagnostics();
            let background_changed = self.apply_background_events();
            if needs_redraw || background_changed || self.tick_status() {
                terminal.draw(|frame| ui::render(frame, self))?;
                needs_redraw = false;
            }
            if event::poll(Duration::from_millis(250))? {
                match event::read()? {
                    Event::Key(key)
                        if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                    {
                        self.handle_key(key);
                        needs_redraw = true;
                    }
                    Event::Paste(text) => {
                        self.handle_paste(&text);
                        needs_redraw = true;
                    }
                    Event::Resize(..) => needs_redraw = true,
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
        // A hover popup is informational; the next key dismisses it.
        if self.hover.is_some() {
            self.hover = None;
            if key.code == KeyCode::Esc {
                return;
            }
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
            Focus::Editor => self.handle_editor_key(key),
        }
    }

    /// Shortcuts that work regardless of focus.
    fn handle_global_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        if key.code == KeyCode::F(8) {
            self.completion = None;
            self.goto_diagnostic(if shift { -1 } else { 1 });
            return true;
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
                    ('s', _) => self.execute_command(ids::SAVE),
                    ('p', true) => self.open_command_palette(),
                    ('p', false) => self.open_quick_open(),
                    ('o', _) => self.execute_command(ids::OPEN),
                    ('f', _) => self.open_search(false),
                    ('h', true) => self.open_hover(),
                    ('h', false) => self.open_search(true),
                    ('g', _) => self.execute_command(ids::GOTO_LINE),
                    ('b', _) => self.toggle_tree(),
                    ('e', _) => self.focus_tree(),
                    ('w', _) => self.execute_command(ids::CLOSE_TAB),
                    ('t', _) => self.open_workspace_symbols(),
                    ('i', true) => self.execute_command(ids::FORMAT),
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
        self.completion = None;
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
                        'd' if shift => self.with_doc(|d| d.duplicate_line()),
                        '/' => self.toggle_comment(),
                        _ => {}
                    }
                } else if !alt {
                    self.with_doc(|d| d.type_char(c));
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
                if alt {
                    self.with_doc(|d| d.move_line_up());
                } else {
                    self.with_doc(|d| d.move_up(shift));
                }
            }
            KeyCode::Down => {
                if alt {
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

    fn handle_tree_key(&mut self, key: KeyEvent) {
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

    fn handle_tree_filter_key(&mut self, key: KeyEvent) {
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

    fn handle_overlay_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        enum Outcome {
            Nothing,
            Close,
            Run(PickerAction, Option<String>),
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
                self.jump_to_first_from_cursor();
            }
            KeyCode::Char(c) if !ctrl => {
                match self.search.field {
                    SearchField::Query => self.search.query.push(c),
                    SearchField::Replacement => self.search.replacement.push(c),
                }
                self.refresh_search_matches();
                self.jump_to_first_from_cursor();
            }
            _ => {}
        }
    }

    /// Handle a key while the completion popup is open. Returns `false` for keys
    /// the popup does not claim, after dismissing it.
    fn handle_completion_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.completion = None;
                true
            }
            KeyCode::Up => {
                if let Some(state) = self.completion.as_mut() {
                    state.move_up();
                }
                true
            }
            KeyCode::Down => {
                if let Some(state) = self.completion.as_mut() {
                    state.move_down();
                }
                true
            }
            KeyCode::PageUp => {
                if let Some(state) = self.completion.as_mut() {
                    state.move_by(-8);
                }
                true
            }
            KeyCode::PageDown => {
                if let Some(state) = self.completion.as_mut() {
                    state.move_by(8);
                }
                true
            }
            KeyCode::Enter | KeyCode::Tab => {
                self.accept_completion();
                true
            }
            KeyCode::Backspace => {
                self.with_doc(|doc| doc.backspace());
                self.refresh_completion();
                true
            }
            KeyCode::Char(c) if !ctrl => {
                self.with_doc(|doc| doc.type_char(c));
                self.refresh_completion();
                true
            }
            _ => {
                self.completion = None;
                false
            }
        }
    }

    /// Open completion for the word being typed.
    fn open_completion(&mut self) {
        let (language, text, cursor) = match self.editor.active_document() {
            Some(doc) => (doc.buffer.language, doc.buffer.text(), doc.clamped_cursor()),
            None => return,
        };

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
        for item in document_words(&text) {
            if seen.insert(item.label.clone()) {
                pool.push(item);
            }
        }

        let state = CompletionState::new(pool, self.completion_prefix());
        if state.items.is_empty() {
            self.set_status("No completions");
            return;
        }
        self.completion = Some(state);
    }

    /// Show information about the symbol under the cursor.
    fn open_hover(&mut self) {
        let (language, text, cursor) = match self.editor.active_document() {
            Some(doc) => (doc.buffer.language, doc.buffer.text(), doc.clamped_cursor()),
            None => return,
        };
        let Some(hover) = self
            .language
            .provider(language)
            .hover(&text, cursor.row, cursor.col)
        else {
            self.set_status("No symbol under the cursor");
            return;
        };
        self.completion = None;
        self.hover = Some(HoverState {
            title: hover.title,
            kind: hover.kind,
            body: hover.body,
        });
    }

    /// The identifier characters immediately before the cursor.
    fn completion_prefix(&self) -> String {
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

    /// Re-filter the open completion for the current prefix, closing it when
    /// nothing matches.
    fn refresh_completion(&mut self) {
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

    /// Replace the typed prefix with the selected completion.
    fn accept_completion(&mut self) {
        let Some(state) = self.completion.take() else {
            return;
        };
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

    // ----------------------------------------------------------------------
    // Commands
    // ----------------------------------------------------------------------

    /// Execute a command by id.
    pub fn execute_command(&mut self, id: &str) {
        match id {
            ids::SAVE => self.save(),
            ids::SAVE_ALL => self.save_all(),
            ids::OPEN => self.open_prompt(PromptKind::OpenPath, "Open file", "path/to/file.rs"),
            ids::QUICK_OPEN => self.open_quick_open(),
            ids::CLOSE_TAB => self.close_tab(),
            ids::CLOSE_ALL => self.close_all(),
            ids::QUIT => self.request_quit(),
            ids::UNDO => self.with_doc(|d| d.undo()),
            ids::REDO => self.with_doc(|d| d.redo()),
            ids::SELECT_ALL => self.with_doc(|d| d.select_all()),
            ids::COPY => self.copy(),
            ids::CUT => self.cut(),
            ids::PASTE => self.paste(),
            ids::COMPLETE => self.open_completion(),
            ids::HOVER => self.open_hover(),
            ids::WORKSPACE_SYMBOLS => self.open_workspace_symbols(),
            ids::FIND => self.open_search(false),
            ids::REPLACE => self.open_search(true),
            ids::GOTO_LINE => self.open_prompt(PromptKind::GotoLine, "Go to line", "42"),
            ids::TOGGLE_COMMENT => self.toggle_comment(),
            ids::INDENT => self.with_doc(|d| d.indent()),
            ids::OUTDENT => self.with_doc(|d| d.outdent()),
            ids::MOVE_LINE_UP => self.with_doc(|d| d.move_line_up()),
            ids::MOVE_LINE_DOWN => self.with_doc(|d| d.move_line_down()),
            ids::DUPLICATE_LINE => self.with_doc(|d| d.duplicate_line()),
            ids::TOGGLE_TREE => self.toggle_tree(),
            ids::FOCUS_TREE => self.focus_tree(),
            ids::TOGGLE_HIDDEN => self.toggle_hidden(),
            ids::FILTER_TREE => self.open_tree_filter(),
            ids::NEXT_TAB => self.editor.next_tab(),
            ids::PREV_TAB => self.editor.previous_tab(),
            ids::PALETTE => self.open_command_palette(),
            ids::RENAME | ids::CODE_ACTIONS => self.report_language_capability(id),
            ids::FORMAT => self.format_document(),
            ids::GOTO_DEFINITION => self.goto_definition(),
            ids::FIND_REFERENCES => self.find_references(),
            ids::SHOW_SYMBOLS => self.open_symbols(),
            ids::DIAGNOSTICS_NEXT => self.goto_diagnostic(1),
            ids::DIAGNOSTICS_PREV => self.goto_diagnostic(-1),
            ids::DIAGNOSTICS_LIST => self.open_diagnostics_list(),
            _ => {}
        }
    }

    fn run_picker_action(&mut self, action: PickerAction) {
        match action {
            PickerAction::Command(id) => self.execute_command(id),
            PickerAction::OpenPath(path) => self.open_path(path),
            PickerAction::Reveal { path, position } => self.reveal(path, position),
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
        self.schedule_diagnostics();
    }

    /// Queue a diagnostics recompute when the active document changed.
    fn schedule_diagnostics(&mut self) {
        if self
            .editor
            .active_document()
            .is_some_and(|doc| doc.diagnostics_dirty())
        {
            self.diagnostics_dirty_at = Some(Instant::now());
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
                self.request_detection_for_active();
                self.workspace.tree.select_path(&path);
                self.focus = Focus::Editor;
                self.close_armed = None;
                self.remember_recent(&path);
                self.set_status(format!("Opened {}", path.display()));
            }
            Err(err) => self.set_error(format!("Could not open {}: {err}", path.display())),
        }
    }

    fn remember_recent(&mut self, path: &Path) {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.recent_files.retain(|existing| {
            existing.canonicalize().unwrap_or_else(|_| existing.clone()) != canonical
        });
        self.recent_files.push(canonical);
        if self.recent_files.len() > 20 {
            self.recent_files.remove(0);
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
                self.request_git_refresh();
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
                self.request_detection_for_active();
                self.request_git_refresh();
                self.set_status(format!("Saved {}", path.display()));
            }
            Some(Err(err)) => self.set_error(format!("Save failed: {err}")),
            _ => self.set_error("Nothing to save"),
        }
    }

    fn close_tab(&mut self) {
        let index = self.editor.active_index();
        let dirty = self
            .editor
            .documents
            .get(index)
            .map(|doc| doc.is_dirty())
            .unwrap_or(false);
        if dirty && self.close_armed != Some(index) {
            self.close_armed = Some(index);
            self.set_error("Unsaved changes — press Ctrl+W again to close without saving");
            return;
        }
        self.close_armed = None;
        self.editor.close(index);
        self.set_status("Tab closed");
    }

    fn close_all(&mut self) {
        if self.editor.has_unsaved() {
            self.set_error("Unsaved changes — save first (Ctrl+S)");
            return;
        }
        self.editor.close_all();
        self.close_armed = None;
        self.set_status("All tabs closed");
    }

    fn save_all(&mut self) {
        let mut saved = 0usize;
        let mut error = None;
        for doc in &mut self.editor.documents {
            if doc.is_dirty() && doc.buffer.path.is_some() {
                match doc.buffer.save() {
                    Ok(true) => saved += 1,
                    Ok(false) => {}
                    Err(err) => {
                        error = Some(err);
                        break;
                    }
                }
            }
        }
        if let Some(err) = error {
            self.set_error(format!("Save failed: {err}"));
            return;
        }
        if saved > 0 {
            self.request_git_refresh();
            self.quit_armed = false;
            self.set_status(format!("Saved {saved} file(s)"));
        } else {
            self.set_status("Nothing to save");
        }
    }

    fn toggle_hidden(&mut self) {
        self.workspace.tree.toggle_hidden();
        let state = if self.workspace.tree.show_hidden {
            "shown"
        } else {
            "hidden"
        };
        self.set_status(format!("Dotfiles {state}"));
    }

    fn open_tree_filter(&mut self) {
        if !self.tree_visible {
            self.tree_visible = true;
        }
        self.focus = Focus::FileTree;
        self.tree_filter = Some(TreeFilter::new(self.workspace.root()));
    }

    /// Ask the background worker to detect the active file's language.
    fn request_detection_for_active(&mut self) {
        let markers = self.workspace.marker_names();
        if let Some(path) = self
            .editor
            .active_document()
            .and_then(|doc| doc.buffer.path.clone())
        {
            self.background.detect(path, markers);
        }
    }

    /// Ask the background worker to refresh git status.
    fn request_git_refresh(&self) {
        self.background
            .refresh_git(self.workspace.root().to_path_buf());
    }

    /// Apply any finished background work. Returns `true` if something changed.
    fn apply_background_events(&mut self) -> bool {
        let mut changed = false;
        while let Some(event) = self.background.try_recv() {
            changed |= self.apply_background_event(event);
        }
        changed
    }

    fn apply_background_event(&mut self, event: BackgroundEvent) -> bool {
        match event {
            BackgroundEvent::Detected { path, language, .. } => {
                if let Some(doc) = self
                    .editor
                    .documents
                    .iter_mut()
                    .find(|doc| same_file(doc.buffer.path.as_deref(), &path))
                {
                    let changed = doc.buffer.language != language;
                    doc.set_language(language);
                    if changed {
                        // The provider changed, so any earlier analysis is void:
                        // reset the revision to force a fresh first pass.
                        doc.clear_diagnostics();
                        doc.set_diagnostics_revision(0);
                    }
                    return changed;
                }
                false
            }
            BackgroundEvent::Diagnostics {
                path,
                revision,
                diagnostics,
            } => self
                .editor
                .documents
                .iter_mut()
                .find(|doc| same_file(doc.buffer.path.as_deref(), &path))
                .is_some_and(|doc| doc.apply_diagnostics(revision, diagnostics)),
            BackgroundEvent::Formatted {
                path,
                revision,
                outcome,
            } => {
                let current =
                    self.pending_format
                        .as_ref()
                        .is_some_and(|(pending_path, pending_revision)| {
                            *pending_path == path && *pending_revision == revision
                        });
                if !current {
                    return false;
                }
                self.pending_format = None;
                self.apply_format_outcome(&path, outcome);
                true
            }
            BackgroundEvent::Git(info) => {
                self.workspace.git = info;
                true
            }
            BackgroundEvent::WorkspaceSymbols { revision, symbols } => {
                if self.pending_workspace_symbols != Some(revision) {
                    return false;
                }
                self.pending_workspace_symbols = None;
                self.open_workspace_symbol_picker(symbols);
                true
            }
        }
    }

    /// Ask the background worker to scan the project for definitions.
    fn open_workspace_symbols(&mut self) {
        self.workspace_symbols_seq += 1;
        let revision = self.workspace_symbols_seq;
        self.pending_workspace_symbols = Some(revision);
        self.background
            .workspace_symbols(self.workspace.root().to_path_buf(), revision);
        self.set_status("Searching for symbols…");
    }

    fn open_workspace_symbol_picker(&mut self, symbols: Vec<WorkspaceSymbol>) {
        if symbols.is_empty() {
            self.set_status("No symbols found");
            return;
        }
        let root = self.workspace.root().to_path_buf();
        let items = symbols
            .into_iter()
            .map(|entry| {
                let relative = entry
                    .path
                    .strip_prefix(&root)
                    .unwrap_or(&entry.path)
                    .display()
                    .to_string();
                let detail = format!(
                    "{}  ·  {relative}:{}",
                    entry.symbol.kind.label(),
                    entry.symbol.line + 1
                );
                PickerItem::new(
                    entry.symbol.name,
                    detail,
                    PickerAction::Reveal {
                        path: entry.path,
                        position: Position::new(entry.symbol.line, entry.symbol.col),
                    },
                )
            })
            .collect();
        let mut picker = Picker::new("Workspace Symbols", "Filter symbols…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Format the active document with the language's trusted formatter.
    fn format_document(&mut self) {
        let (path, language, text) = match self.editor.active_document() {
            Some(doc) => (
                doc.buffer.path.clone(),
                doc.buffer.language,
                doc.buffer.text(),
            ),
            None => return,
        };
        let Some(path) = path else {
            self.set_error("Save the file before formatting");
            return;
        };
        if !self
            .language
            .provider(language)
            .capabilities()
            .contains(&Capability::Formatting)
        {
            self.set_status(format!(
                "Formatting is not available for {}",
                language.name()
            ));
            return;
        }

        self.format_seq += 1;
        let revision = self.format_seq;
        self.pending_format = Some((path.clone(), revision));
        self.background.format(path, language, text, revision);
        self.set_status("Formatting…");
    }

    /// Apply a formatting result, or explain why it could not run.
    fn apply_format_outcome(&mut self, path: &Path, outcome: FormatOutcome) {
        match outcome {
            FormatOutcome::Formatted(text) => self.apply_formatted(path, &text),
            FormatOutcome::Unsupported => {
                self.set_status("Formatting is not available for this language")
            }
            FormatOutcome::ToolMissing { tool, hint } => {
                self.set_error(format!("{tool} not found — {hint}"))
            }
            FormatOutcome::Failed(message) => self.set_error(format!("Format failed: {message}")),
        }
    }

    /// Replace the buffer with formatted text as a single undoable edit.
    fn apply_formatted(&mut self, path: &Path, text: &str) {
        let applied = if let Some(doc) = self
            .editor
            .documents
            .iter_mut()
            .find(|doc| same_file(doc.buffer.path.as_deref(), path))
        {
            let cursor = doc.clamped_cursor();
            let last = doc.buffer.len_lines().saturating_sub(1);
            let end = Position::new(last, doc.buffer.line_char_len(last));
            doc.replace_range(Position::zero(), end, text);
            doc.cursor = doc.buffer.clamp_position(cursor);
            true
        } else {
            false
        };

        if applied {
            self.after_edit();
            self.set_status("Formatted");
        } else {
            self.set_status("File is no longer open");
        }
    }

    // ----------------------------------------------------------------------
    // Diagnostics
    // ----------------------------------------------------------------------

    /// Dispatch a debounced recompute once typing pauses, or run the first pass
    /// for a document that has not been analysed yet.
    fn poll_diagnostics(&mut self) {
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
    fn dispatch_diagnostics(&mut self) {
        let (path, language) = match self.editor.active_document() {
            Some(doc) => (doc.buffer.path.clone(), doc.buffer.language),
            None => return,
        };
        let supported = self
            .language
            .provider(language)
            .capabilities()
            .contains(&Capability::Diagnostics);

        self.diagnostics_seq += 1;
        let revision = self.diagnostics_seq;
        if let Some(doc) = self.editor.active_document_mut() {
            doc.set_diagnostics_revision(revision);
        }

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

    /// Jump to the next (`direction > 0`) or previous diagnostic, wrapping.
    fn goto_diagnostic(&mut self, direction: i32) {
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

    /// List every diagnostic across open files, jumping on selection.
    fn open_diagnostics_list(&mut self) {
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

    /// Open `path` and place the cursor at `position`, centred in the viewport.
    fn reveal(&mut self, path: PathBuf, position: Position) {
        self.open_path(path);
        self.jump_to(position);
    }

    /// Centre `position` in the viewport and move the cursor there.
    fn jump_to(&mut self, position: Position) {
        let center = self.viewport_height / 2;
        self.with_doc(|doc| {
            doc.move_to(position);
            doc.scroll_top = position.row.saturating_sub(center);
        });
    }

    /// Jump to the definition of the word under the cursor (within this file).
    fn goto_definition(&mut self) {
        let (path, language, file, text, cursor) = match self.editor.active_document() {
            Some(doc) => (
                doc.buffer.path.clone(),
                doc.buffer.language,
                doc.file_name(),
                doc.buffer.text(),
                doc.clamped_cursor(),
            ),
            None => return,
        };
        let Some(path) = path else {
            return;
        };
        let Some(symbol) = self
            .language
            .provider(language)
            .definition(&text, cursor.row, cursor.col)
        else {
            self.set_status("No definition found in this file");
            return;
        };
        let target = Position::new(symbol.line, symbol.col);
        self.reveal(path, target);
        self.set_status(format!(
            "{} {} · {file}:{}",
            symbol.kind.label(),
            symbol.name,
            symbol.line + 1
        ));
    }

    /// List every occurrence of the word under the cursor in this file.
    fn find_references(&mut self) {
        let (path, language, file, text, cursor) = match self.editor.active_document() {
            Some(doc) => (
                doc.buffer.path.clone(),
                doc.buffer.language,
                doc.file_name(),
                doc.buffer.text(),
                doc.clamped_cursor(),
            ),
            None => return,
        };
        let Some(path) = path else {
            return;
        };
        let locations = self
            .language
            .provider(language)
            .references(&text, cursor.row, cursor.col);
        if locations.is_empty() {
            self.set_status("No symbol under the cursor");
            return;
        }

        let lines: Vec<&str> = text.lines().collect();
        let items = locations
            .into_iter()
            .map(|location| {
                let snippet = lines
                    .get(location.line)
                    .map(|line| line.trim().to_string())
                    .filter(|line| !line.is_empty())
                    .unwrap_or_else(|| format!("line {}", location.line + 1));
                let detail = format!("{file}:{}:{}", location.line + 1, location.col + 1);
                PickerItem::new(
                    snippet,
                    detail,
                    PickerAction::Reveal {
                        path: path.clone(),
                        position: Position::new(location.line, location.col),
                    },
                )
            })
            .collect();
        let mut picker = Picker::new("References", "Filter occurrences…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// List the active document's definitions and jump to the chosen one.
    fn open_symbols(&mut self) {
        let (path, language, file, text) = match self.editor.active_document() {
            Some(doc) => (
                doc.buffer.path.clone(),
                doc.buffer.language,
                doc.file_name(),
                doc.buffer.text(),
            ),
            None => return,
        };
        let Some(path) = path else {
            self.set_status("Symbols need a saved file");
            return;
        };

        let symbols = self.language.provider(language).symbols(&text);
        if symbols.is_empty() {
            self.set_status(format!("No symbols found in {file}"));
            return;
        }

        let items = symbols
            .into_iter()
            .map(|symbol| {
                let position = Position::new(symbol.line, symbol.col);
                let detail = format!("{}  ·  {file}:{}", symbol.kind.label(), symbol.line + 1);
                PickerItem::new(
                    symbol.name,
                    detail,
                    PickerAction::Reveal {
                        path: path.clone(),
                        position,
                    },
                )
            })
            .collect();
        let mut picker = Picker::new("Symbols", "Filter symbols…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Wait up to `timeout` for startup background work to settle.
    fn pump_background(&mut self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match self.background.recv_timeout(remaining) {
                Some(event) => {
                    self.apply_background_event(event);
                }
                None => break,
            }
        }
        self.apply_background_events();
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

    /// Jump to the first match at or after the cursor, wrapping around.
    fn jump_to_first_from_cursor(&mut self) {
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
        self.schedule_diagnostics();
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
            .map(|command| {
                let (enabled, hint) = self.command_availability(command);
                let mut item = PickerItem::new(
                    command.palette_label(),
                    command.description.to_string(),
                    PickerAction::Command(command.id),
                )
                .shortcut(command.shortcut.unwrap_or(""));
                if !enabled {
                    item = item.disabled(hint.unwrap_or_else(|| "unavailable".to_string()));
                }
                item
            })
            .collect();
        let mut picker = Picker::new("Command Palette", "Type a command…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Whether a command can run right now, and why not when it cannot.
    fn command_availability(&self, command: &Command) -> (bool, Option<String>) {
        let document = self.editor.active_document();
        if command.needs_doc && document.is_none() {
            return (false, Some("no file open".to_string()));
        }
        if let Some(capability) = command.capability {
            let language = document
                .map(|doc| doc.buffer.language)
                .unwrap_or(LanguageId::Unknown);
            if !self
                .language
                .provider(language)
                .capabilities()
                .contains(&capability)
            {
                return (
                    false,
                    Some(format!("not available for {}", language.name())),
                );
            }
        }
        match command.id {
            ids::UNDO if !document.is_some_and(|doc| doc.can_undo()) => {
                (false, Some("nothing to undo".to_string()))
            }
            ids::REDO if !document.is_some_and(|doc| doc.can_redo()) => {
                (false, Some("nothing to redo".to_string()))
            }
            ids::COPY | ids::CUT if !document.is_some_and(|doc| doc.has_selection()) => {
                (false, Some("nothing selected".to_string()))
            }
            ids::NEXT_TAB | ids::PREV_TAB if self.editor.len() < 2 => {
                (false, Some("only one tab".to_string()))
            }
            ids::CLOSE_TAB | ids::CLOSE_ALL | ids::SAVE_ALL if self.editor.is_empty() => {
                (false, Some("no files open".to_string()))
            }
            ids::DIAGNOSTICS_NEXT | ids::DIAGNOSTICS_PREV
                if !document.is_some_and(|doc| !doc.diagnostics().is_empty()) =>
            {
                (false, Some("no diagnostics".to_string()))
            }
            ids::DIAGNOSTICS_LIST
                if !self
                    .editor
                    .documents
                    .iter()
                    .any(|doc| !doc.diagnostics().is_empty()) =>
            {
                (false, Some("no diagnostics".to_string()))
            }
            _ => (true, None),
        }
    }

    fn open_quick_open(&mut self) {
        let root = self.workspace.root().to_path_buf();
        let files = filesystem::collect_files(&root, 8000);

        // Recently-opened files come first, so Ctrl+P then Enter reopens the last
        // file without typing.
        let mut ordered: Vec<PathBuf> = Vec::new();
        let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
        for path in self.recent_files.iter().rev() {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !seen.insert(canonical) {
                continue;
            }
            if path.is_file() {
                ordered.push(path.clone());
            }
        }
        for path in files {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if seen.insert(canonical) {
                ordered.push(path);
            }
        }

        let items = ordered
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
                PickerItem::new(label, detail, PickerAction::OpenPath(path))
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

    /// Move keyboard focus to the file tree, showing it first if needed. If the
    /// tree already has focus, return to the editor.
    fn focus_tree(&mut self) {
        if !self.tree_visible {
            self.tree_visible = true;
            self.focus = Focus::FileTree;
        } else {
            self.focus = match self.focus {
                Focus::FileTree => Focus::Editor,
                Focus::Editor => Focus::FileTree,
            };
        }
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
    ///
    /// Returns `true` when a message was cleared, so the caller knows to redraw.
    fn tick_status(&mut self) -> bool {
        if let Some(expires_at) = self.status.expires_at
            && Instant::now() >= expires_at
        {
            self.status.message.clear();
            self.status.error = false;
            self.status.expires_at = None;
            return true;
        }
        false
    }

    /// Clear any transient status message.
    pub fn clear_status(&mut self) {
        self.status.message.clear();
        self.status.error = false;
        self.status.expires_at = None;
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

/// Whether two optional/actual paths refer to the same file.
fn same_file(a: Option<&Path>, b: &Path) -> bool {
    let Some(a) = a else {
        return false;
    };
    let a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    a == b
}

/// Identifier words already present in a document, deduplicated in order.
///
/// Buffer completion is language-agnostic and always useful; providers only add
/// the parts a language knows (keywords, types, builtins).
fn document_words(text: &str) -> Vec<Completion> {
    let mut seen = std::collections::HashSet::new();
    let mut words = Vec::new();
    for word in text.split(|c: char| !is_word_char(c)) {
        if word.chars().count() < 2 {
            continue;
        }
        if seen.insert(word.to_string()) {
            words.push(Completion::new(word.to_string(), CompletionKind::Variable));
        }
        if words.len() >= 2000 {
            break;
        }
    }
    words
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

    #[test]
    fn focus_tree_command_toggles_focus() {
        let dir = temp_project("focus");
        let mut app = App::new(Some(&dir)).unwrap();
        assert_eq!(app.focus, Focus::Editor);

        app.execute_command(ids::FOCUS_TREE);
        assert_eq!(app.focus, Focus::FileTree);

        app.execute_command(ids::FOCUS_TREE);
        assert_eq!(app.focus, Focus::Editor);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn palette_marks_unavailable_commands() {
        let dir = temp_project("palette-avail");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();
        app.execute_command(ids::PALETTE);

        let Overlay::Picker(picker) = &app.overlay else {
            panic!("expected the command palette");
        };
        let find = |needle: &str| {
            (0..picker.filtered.len())
                .filter_map(|index| picker.item(index))
                .find(|item| item.label.contains(needle))
        };

        assert!(find("File: Save").expect("save").enabled);
        assert!(find("Format Document").expect("format").enabled);
        assert!(
            !find("Rename Symbol").expect("rename").enabled,
            "rename is not available for Rust yet"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn quick_open_lists_recent_files_first() {
        let dir = temp_project("recents");
        let a = dir.join("src/main.rs");
        let b = dir.join("src/lib.rs");
        fs::write(&b, "pub fn lib() {}\n").unwrap();

        let mut app = App::new(Some(&a)).unwrap();
        app.open_path(b.clone());
        app.execute_command(ids::QUICK_OPEN);

        let Overlay::Picker(picker) = &app.overlay else {
            panic!("expected quick open");
        };
        let first = picker.item(0).expect("an item");
        match &first.action {
            PickerAction::OpenPath(path) => {
                assert_eq!(path, &b.canonicalize().unwrap());
            }
            _ => panic!("expected a file"),
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn closing_a_dirty_tab_asks_first() {
        let dir = temp_project("close-dirty");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();
        app.editor
            .active_document_mut()
            .unwrap()
            .insert_text("// x\n");

        app.execute_command(ids::CLOSE_TAB);
        assert_eq!(app.editor.len(), 1, "the dirty tab should stay open");

        app.execute_command(ids::CLOSE_TAB);
        assert_eq!(app.editor.len(), 0, "the second press closes it");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_all_writes_every_dirty_file() {
        let dir = temp_project("save-all");
        let a = dir.join("src/main.rs");
        let b = dir.join("src/lib.rs");
        fs::write(&b, "pub fn f() {}\n").unwrap();

        let mut app = App::new(Some(&a)).unwrap();
        app.open_path(b.clone());
        app.editor.documents[0].insert_text("// a\n");
        app.editor.documents[1].insert_text("// b\n");

        app.execute_command(ids::SAVE_ALL);
        assert!(!app.editor.has_unsaved());
        assert!(fs::read_to_string(&a).unwrap().starts_with("// a\n"));
        assert!(fs::read_to_string(&b).unwrap().starts_with("// b\n"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn toggle_hidden_flips_tree_state() {
        let dir = temp_project("hidden");
        let mut app = App::new(Some(&dir)).unwrap();
        assert!(!app.workspace.tree.show_hidden);
        app.execute_command(ids::TOGGLE_HIDDEN);
        assert!(app.workspace.tree.show_hidden);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_prefills_from_the_selection() {
        let dir = temp_project("find-prefill");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();
        {
            let doc = app.editor.active_document_mut().unwrap();
            doc.selection = Some(Selection::new(Position::new(0, 0)));
            doc.cursor = Position::new(0, 2);
        }

        app.execute_command(ids::FIND);
        assert_eq!(app.search.query, "fn");
        assert!(!app.search.matches.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    /// Spin the background channel until the active document has diagnostics.
    fn wait_for_diagnostics(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            app.apply_background_events();
            if app
                .editor
                .active_document()
                .is_some_and(|doc| !doc.diagnostics().is_empty())
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn diagnostics_are_computed_in_the_background() {
        let dir = temp_project("diagnostics");
        let file = dir.join("src/main.rs");
        fs::write(&file, "fn main() {\n    let x = 1;\n").unwrap();

        let mut app = App::new(Some(&file)).unwrap();
        assert_eq!(
            app.editor.active_document().unwrap().buffer.language,
            LanguageId::Rust
        );
        app.poll_diagnostics();
        wait_for_diagnostics(&mut app);

        let doc = app.editor.active_document().unwrap();
        assert!(
            !doc.diagnostics().is_empty(),
            "expected an unclosed-brace diagnostic"
        );
        assert_eq!(doc.diagnostic_counts().0, 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn next_diagnostic_moves_the_cursor() {
        let dir = temp_project("diagnostics-next");
        let file = dir.join("src/main.rs");
        fs::write(&file, "fn main() {\n    let x = 1;\n").unwrap();

        let mut app = App::new(Some(&file)).unwrap();
        app.poll_diagnostics();
        wait_for_diagnostics(&mut app);
        app.editor
            .active_document_mut()
            .unwrap()
            .move_to(Position::new(0, 0));

        app.execute_command(ids::DIAGNOSTICS_NEXT);
        let cursor = app.editor.active_document().unwrap().clamped_cursor();
        assert_eq!(cursor, Position::new(0, 10));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn diagnostics_list_opens_when_there_are_problems() {
        let dir = temp_project("diagnostics-list");
        let file = dir.join("src/main.rs");
        fs::write(&file, "fn main() {\n").unwrap();

        let mut app = App::new(Some(&file)).unwrap();
        app.poll_diagnostics();
        wait_for_diagnostics(&mut app);

        app.execute_command(ids::DIAGNOSTICS_LIST);
        let Overlay::Picker(picker) = &app.overlay else {
            panic!("expected the diagnostics list");
        };
        assert!(!picker.filtered.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn symbol_outline_lists_definitions() {
        let dir = temp_project("symbols");
        let file = dir.join("src/main.rs");

        let mut app = App::new(Some(&file)).unwrap();
        app.execute_command(ids::SHOW_SYMBOLS);

        let Overlay::Picker(picker) = &app.overlay else {
            panic!("expected the symbol list");
        };
        let labels: Vec<String> = (0..picker.filtered.len())
            .filter_map(|index| picker.item(index))
            .map(|item| item.label.clone())
            .collect();
        assert!(labels.iter().any(|label| label == "main"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn go_to_definition_jumps_to_the_symbol() {
        let dir = temp_project("definition");
        let file = dir.join("src/main.rs");
        fs::write(&file, "fn main() {\n    helper();\n}\n\nfn helper() {}\n").unwrap();

        let mut app = App::new(Some(&file)).unwrap();
        app.editor
            .active_document_mut()
            .unwrap()
            .move_to(Position::new(1, 4));

        app.execute_command(ids::GOTO_DEFINITION);
        let cursor = app.editor.active_document().unwrap().clamped_cursor();
        assert_eq!(cursor, Position::new(4, 3));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_references_lists_every_occurrence() {
        let dir = temp_project("references");
        let file = dir.join("src/main.rs");
        fs::write(&file, "fn main() {\n    helper();\n}\n\nfn helper() {}\n").unwrap();

        let mut app = App::new(Some(&file)).unwrap();
        app.editor
            .active_document_mut()
            .unwrap()
            .move_to(Position::new(1, 4));

        app.execute_command(ids::FIND_REFERENCES);
        let Overlay::Picker(picker) = &app.overlay else {
            panic!("expected the references list");
        };
        assert_eq!(picker.filtered.len(), 2);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn completion_accepts_the_selected_candidate() {
        let dir = temp_project("complete");
        let file = dir.join("src/main.rs");
        fs::write(&file, "let counter = 0;\ncount\n").unwrap();

        let mut app = App::new(Some(&file)).unwrap();
        app.editor
            .active_document_mut()
            .unwrap()
            .move_to(Position::new(1, 5));
        app.execute_command(ids::COMPLETE);
        assert!(app.completion.is_some(), "completion should open");

        // Candidates are ordered shortest-first, so move to `counter`.
        if let Some(state) = app.completion.as_mut() {
            state.move_down();
        }
        app.accept_completion();
        assert_eq!(
            app.editor.active_document().unwrap().buffer.line_text(1),
            "counter"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn completion_offers_language_keywords() {
        let dir = temp_project("complete-keywords");
        let file = dir.join("src/main.rs");

        let mut app = App::new(Some(&file)).unwrap();
        app.execute_command(ids::COMPLETE);
        let state = app.completion.as_ref().expect("completion should open");
        assert!(state.items.iter().any(|item| item.label == "fn"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn formatting_replaces_the_buffer_as_one_edit() {
        let dir = temp_project("format");
        let file = dir.join("src/main.rs");
        fs::write(&file, "fn main(){let x=1;}\n").unwrap();

        let mut app = App::new(Some(&file)).unwrap();
        let before = app.editor.active_document().unwrap().buffer.text();
        app.apply_formatted(&file, "fn main() {\n    let x = 1;\n}\n");
        assert_eq!(
            app.editor.active_document().unwrap().buffer.text(),
            "fn main() {\n    let x = 1;\n}\n"
        );
        assert!(app.editor.active_document().unwrap().is_dirty());

        app.with_doc(|doc| doc.undo());
        assert_eq!(app.editor.active_document().unwrap().buffer.text(), before);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn formatting_is_unavailable_for_plain_text() {
        let dir = temp_project("format-plain");
        let file = dir.join("notes.txt");
        fs::write(&file, "hello\n").unwrap();

        let mut app = App::new(Some(&file)).unwrap();
        app.format_document();
        assert!(
            app.pending_format.is_none(),
            "plain text has no formatter, so no request should be sent"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn format_document_runs_the_formatter() {
        if std::process::Command::new("rustfmt")
            .arg("--version")
            .output()
            .is_err()
        {
            return; // Skip when rustfmt is unavailable.
        }
        let dir = temp_project("format-flow");
        let file = dir.join("src/main.rs");
        fs::write(&file, "fn main(){let x=1;}\n").unwrap();

        let mut app = App::new(Some(&file)).unwrap();
        app.format_document();

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            app.apply_background_events();
            if app.pending_format.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        let text = app.editor.active_document().unwrap().buffer.text();
        assert!(
            text.contains("fn main() {") && text.contains("    let x = 1;"),
            "expected formatted output, got:\n{text}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hover_describes_the_symbol_under_the_cursor() {
        let dir = temp_project("hover");
        let file = dir.join("src/main.rs");
        fs::write(&file, "fn main() {\n    helper();\n}\n\nfn helper() {}\n").unwrap();

        let mut app = App::new(Some(&file)).unwrap();
        app.editor
            .active_document_mut()
            .unwrap()
            .move_to(Position::new(1, 4));

        app.execute_command(ids::HOVER);
        let hover = app.hover.as_ref().expect("hover should open");
        assert!(hover.title.contains("helper"), "title was {}", hover.title);
        assert!(hover.body.iter().any(|line| line.contains("occurrence")));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn workspace_symbols_lists_definitions_across_files() {
        let dir = temp_project("workspace-symbols");
        let a = dir.join("src/main.rs");
        let b = dir.join("src/lib.rs");
        fs::write(&b, "pub fn beta() {}\n").unwrap();

        let mut app = App::new(Some(&a)).unwrap();
        app.open_workspace_symbols();

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            app.apply_background_events();
            if !matches!(app.overlay, Overlay::None) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        let Overlay::Picker(picker) = &app.overlay else {
            panic!("expected the workspace symbol list");
        };
        let labels: Vec<String> = (0..picker.filtered.len())
            .filter_map(|index| picker.item(index))
            .map(|item| item.label.clone())
            .collect();
        assert!(
            labels.iter().any(|label| label == "beta"),
            "labels: {labels:?}"
        );
        assert!(
            labels.iter().any(|label| label == "main"),
            "labels: {labels:?}"
        );
        fs::remove_dir_all(&dir).ok();
    }
}
