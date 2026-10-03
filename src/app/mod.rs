//! Application state and the event loop.
//!
//! This module is the conductor: it wires workspace, editor, language service,
//! commands and UI together, and translates terminal events into actions.

pub mod overlay;

use std::cmp::Reverse;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use serde_json::Value;

use crate::background::{Background, Event as BackgroundEvent};
use crate::commands::{Command, CommandRegistry, ids};
use crate::editor::{Document, Editor, Position, Selection};
use crate::filesystem;
use crate::git::GitFileStatus;
use crate::language::completion::{Completion, CompletionKind};
use crate::language::diagnostics::{Diagnostic, Severity};
use crate::language::format;
use crate::language::format::FormatOutcome;
use crate::language::lsp::{RequestKind, Server, ServerEvent, convert};
use crate::language::symbols::is_ident_char as is_word_char;
use crate::language::tools::{Tool, ToolPurpose, ToolRegistry};
use crate::language::{Capability, LanguageId, LanguageService, WorkspaceSymbol};
use crate::project::Workspace;
use crate::search::SearchMatch;
use crate::session::{self, Session};
use crate::terminal;
use crate::ui;
use overlay::{
    CompletionState, Help, HoverState, Overlay, Picker, PickerAction, PickerItem, Prompt,
    PromptKind, Search, SearchField, TreeFilter,
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

/// Which editor pane owns the cursor when the editor is split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Primary,
    Secondary,
}

/// How long to wait after opening a file before starting a language server.
///
/// Keeping this off the critical path means opening a file is instant and a
/// server is never spawned for the brief use of a throwaway process.
const LSP_START_DELAY: Duration = Duration::from_millis(600);

/// If a server does not finish its handshake within this window, Koda gives up,
/// falls back to its built-in providers and (bounded by [`MAX_LSP_RESTARTS`])
/// tries once more.
const LSP_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(25);

/// How long to wait before an automatic restart after a server failure.
const LSP_RESTART_DELAY: Duration = Duration::from_secs(2);

/// How many automatic restarts Koda attempts before settling on the built-in
/// providers. The counter resets whenever a server connects successfully.
const MAX_LSP_RESTARTS: u32 = 2;

/// The state of the language-server connection, for the statusline and setup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LspStatus {
    /// No server is running (and none is expected).
    #[default]
    Offline,
    /// A server was started and the handshake is in progress.
    Starting,
    /// The server is initialized and serving features.
    Ready,
    /// A server could not be started or exited.
    Failed(String),
}

/// A short-lived status message.
#[derive(Default)]
pub struct Status {
    pub message: String,
    pub error: bool,
    expires_at: Option<Instant>,
}

/// The region produced by the last paste, used by yank-pop.
#[derive(Clone, Copy)]
struct Yank {
    doc: usize,
    start: Position,
    end: Position,
    /// Buffer version at the time of the paste; any later edit invalidates it.
    version: u64,
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
    /// Recent kills, most recent last, for yank-pop.
    kill_ring: Vec<String>,
    /// Index of the entry currently yanked, within the ring.
    kill_index: usize,
    /// The region produced by the last paste, so `Alt+Y` can replace it.
    last_yank: Option<Yank>,
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
    /// Whether diagnostic messages are shown at the end of their line.
    pub inline_diagnostics: bool,
    /// Whether the editor shows two panes side by side.
    pub split: bool,
    /// The document shown in the left (primary) pane.
    pane_left: usize,
    /// The document shown in the right pane, when split.
    pane_right: Option<usize>,
    /// Which pane receives editing.
    pub focus_pane: Pane,
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
    /// Whether a language server workspace-symbol request is in flight.
    ws_lsp_pending: bool,
    /// Monotonic id for project text searches.
    project_search_seq: u64,
    /// The project search awaiting a result, if any.
    pending_project_search: Option<u64>,
    /// Animation frame, advanced while something on screen animates.
    pub anim_phase: usize,
    /// When the animation frame last advanced.
    anim_last: Instant,
    /// External tools Koda has probed for, once discovery completes.
    pub tools: Option<ToolRegistry>,
    /// The running language server, if any.
    lsp: Option<Server>,
    /// The language the running (or attempted) server serves.
    lsp_language: Option<LanguageId>,
    /// When set, a language server should start once this delay elapses.
    lsp_start_at: Option<(Instant, LanguageId)>,
    /// When the current handshake started, for the timeout.
    lsp_started_at: Option<Instant>,
    /// Automatic restarts attempted since the server last connected.
    lsp_restarts: u32,
    /// The symbol awaiting a new name, from the rename prompt.
    pending_rename: Option<(PathBuf, usize, usize)>,
    /// The path awaiting a new name, from the file-rename prompt.
    pending_rename_file: Option<PathBuf>,
    /// A tool install in progress, if any.
    pending_install: Option<Tool>,
    /// Whether a git commit is running on the worker.
    pending_commit: bool,
    /// Code actions from the most recent server response.
    pending_code_actions: Vec<convert::CodeAction>,
    /// Connection state, shown in the statusline.
    pub lsp_status: LspStatus,
    /// When open files were last checked for on-disk changes.
    last_disk_check: Instant,
    /// Languages for which Koda has already offered to install a missing
    /// language server this session.
    setup_offered: std::collections::HashSet<LanguageId>,
}

impl App {
    /// Run Koda. `target` is the optional CLI path argument.
    pub fn start(target: Option<String>) -> io::Result<()> {
        let target = target.map(PathBuf::from);
        let mut app = App::new(target.as_deref())?;
        let mut terminal = terminal::init()?;
        let result = app.run(&mut terminal);
        terminal::restore();
        app.save_session();
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
            kill_ring: Vec::new(),
            kill_index: 0,
            last_yank: None,
            recent_files: Vec::new(),
            tree_filter: None,
            completion: None,
            hover: None,
            cursor_screen: None,
            tree_visible: true,
            inline_diagnostics: true,
            split: false,
            pane_left: 0,
            pane_right: None,
            focus_pane: Pane::Primary,
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
            ws_lsp_pending: false,
            project_search_seq: 0,
            pending_project_search: None,
            anim_phase: 0,
            anim_last: Instant::now(),
            tools: None,
            lsp: None,
            lsp_language: None,
            lsp_start_at: None,
            lsp_started_at: None,
            lsp_restarts: 0,
            pending_rename: None,
            pending_rename_file: None,
            pending_install: None,
            pending_commit: false,
            pending_code_actions: Vec::new(),
            lsp_status: LspStatus::Offline,
            last_disk_check: Instant::now(),
            setup_offered: std::collections::HashSet::new(),
        };

        if let Some(path) = target
            && path.is_file()
        {
            app.open_path(path.to_path_buf());
        } else if let Some(path) = session::session_path(app.workspace.root())
            && let Some(session) = Session::load_from(&path)
        {
            app.restore_session(session);
        }
        app.request_git_refresh();
        // Probe for external tools on the worker so startup never waits on it.
        app.background.discover_tools();
        // Apply the initial detection and git snapshot before the first frame so
        // startup is deterministic.
        app.pump_background(Duration::from_millis(300));
        Ok(app)
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        let mut needs_redraw = true;
        while !self.should_quit {
            // Only repaint when something changed: input arrived, background
            // work completed, the animation advanced, a status message expired,
            // or the terminal was resized. An idle Koda does no work at all.
            self.poll_diagnostics();
            self.poll_lsp_start();
            let lsp_health_changed = self.poll_lsp_health();
            let lsp_changed = self.poll_lsp() || lsp_health_changed;
            let background_changed = self.apply_background_events();
            let animated = self.tick_animation();
            let external_changed = self.poll_external_changes();
            let offered = self.maybe_offer_tool_setup();
            if needs_redraw
                || background_changed
                || lsp_changed
                || animated
                || external_changed
                || offered
                || self.tick_status()
            {
                terminal.draw(|frame| ui::render(frame, self))?;
                needs_redraw = false;
            }
            let timeout = if self.wants_animation() {
                // Wake exactly when the next frame is due, not sooner.
                self.animation_interval()
                    .saturating_sub(self.anim_last.elapsed())
                    .max(Duration::from_millis(16))
            } else {
                Duration::from_millis(250)
            };
            if event::poll(timeout)? {
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
                    Event::FocusGained => {
                        // Files may have appeared or vanished while Koda was in
                        // the background; catch up quietly.
                        self.workspace.refresh();
                        self.request_git_refresh();
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
    // Animation
    // ----------------------------------------------------------------------

    /// Whether anything on screen animates right now.
    fn wants_animation(&self) -> bool {
        self.editor.is_empty() || self.busy().is_some()
    }

    fn animation_interval(&self) -> Duration {
        if self.busy().is_some() {
            // Busy work is short-lived, so spin smoothly while it lasts.
            Duration::from_millis(90)
        } else {
            // The welcome mascot only blinks now and then.
            Duration::from_millis(650)
        }
    }

    /// Advance the animation frame once enough time has passed.
    fn tick_animation(&mut self) -> bool {
        if !self.wants_animation() {
            return false;
        }
        if self.anim_last.elapsed() >= self.animation_interval() {
            self.anim_last = Instant::now();
            self.anim_phase = self.anim_phase.wrapping_add(1);
            return true;
        }
        false
    }

    /// Reload clean files that changed on disk, and warn about dirty ones.
    ///
    /// Throttled, because it touches the filesystem. Returns `true` when the UI
    /// should repaint.
    fn poll_external_changes(&mut self) -> bool {
        const INTERVAL: Duration = Duration::from_millis(1200);
        if self.last_disk_check.elapsed() < INTERVAL {
            return false;
        }
        self.last_disk_check = Instant::now();

        let mut message: Option<(bool, String)> = None;
        for doc in &mut self.editor.documents {
            let Some(path) = doc.buffer.path.clone() else {
                continue;
            };
            let Some(current) = filesystem::modified_time(&path) else {
                continue;
            };
            if doc.disk_modified() == Some(current) {
                continue;
            }
            if doc.is_dirty() {
                // Remember the new time so we warn once, not every tick.
                doc.record_disk_mtime();
                message = Some((
                    false,
                    format!("{} changed on disk — save to overwrite", doc.file_name()),
                ));
            } else if doc.reload_from_disk().is_ok() {
                message = Some((true, format!("{} reloaded from disk", doc.file_name())));
            }
        }

        match message {
            Some((true, message)) => {
                self.set_status(message);
                true
            }
            Some((false, message)) => {
                self.set_error(message);
                true
            }
            None => false,
        }
    }

    /// A short label for background work in progress, if any.
    pub fn busy(&self) -> Option<&'static str> {
        if self.pending_format.is_some() {
            Some("formatting")
        } else if self.pending_workspace_symbols.is_some() {
            Some("searching symbols")
        } else if self.pending_project_search.is_some() {
            Some("searching project")
        } else if self.pending_install.is_some() {
            Some("installing")
        } else if self.pending_commit {
            Some("committing")
        } else if self.lsp_status == LspStatus::Starting {
            Some("connecting")
        } else {
            None
        }
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
        // Pane shortcuts work from any surface.
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
            Overlay::Help(_) => return,
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
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if alt {
            match key.code {
                KeyCode::Char(c) => match c.to_ascii_lowercase() {
                    'c' => self.search.case_sensitive = !self.search.case_sensitive,
                    'w' => self.search.whole_word = !self.search.whole_word,
                    'r' => self.search.regex = !self.search.regex,
                    _ => return,
                },
                KeyCode::Enter => {
                    self.replace_all();
                    return;
                }
                _ => return,
            }
            self.refresh_search_matches();
            self.jump_to_first_from_cursor();
            return;
        }
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
        let lsp_available = self.lsp_target().is_some();
        if state.items.is_empty() && !lsp_available {
            self.set_status("No completions");
            return;
        }
        self.completion = Some(state);
        self.request_lsp_completion();
    }

    /// Ask the language server for completions at the cursor.
    fn request_lsp_completion(&mut self) {
        if let Some((path, row, col)) = self.lsp_target()
            && let Some(server) = self.lsp.as_mut()
        {
            server.completion(&path, row, col);
        }
    }

    /// Show information about the symbol under the cursor.
    fn open_hover(&mut self) {
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
        if let Some((path, row, col)) = self.lsp_target() {
            if let Some(server) = self.lsp.as_mut() {
                server.hover(&path, row, col);
            }
            return;
        }
        if self.hover.is_none() {
            self.set_status("No symbol under the cursor");
        }
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
            ids::SAVE_AS => self.open_prompt(PromptKind::SaveAs, "Save as", "path/to/file"),
            ids::OPEN => self.open_prompt(PromptKind::OpenPath, "Open file", "path/to/file.rs"),
            ids::QUICK_OPEN => self.open_quick_open(),
            ids::CLOSE_TAB => self.close_tab(),
            ids::CLOSE_ALL => self.close_all(),
            ids::REVERT => self.revert_file(),
            ids::NEW_FILE => self.new_file(),
            ids::RENAME_FILE => self.rename_selected(),
            ids::DELETE_FILE => self.delete_selected(),
            ids::QUIT => self.request_quit(),
            ids::UNDO => self.with_doc(|d| d.undo()),
            ids::REDO => self.with_doc(|d| d.redo()),
            ids::SELECT_ALL => self.with_doc(|d| d.select_all()),
            ids::COPY => self.copy(),
            ids::CUT => self.cut(),
            ids::PASTE => self.paste(),
            ids::YANK_POP => self.yank_pop(),
            ids::COMPLETE => self.open_completion(),
            ids::HOVER => self.open_hover(),
            ids::SETUP => self.language_setup(),
            ids::RESTART_SERVER => self.restart_language_server(),
            ids::WORKSPACE_SYMBOLS => self.open_workspace_symbols(),
            ids::PROJECT_SEARCH => self.open_project_search(),
            ids::CHANGED_FILES => self.open_changed_files(),
            ids::GIT_COMMIT => self.commit_changes(),
            ids::FIND => self.open_search(false),
            ids::REPLACE => self.open_search(true),
            ids::REPLACE_ALL => self.replace_all(),
            ids::GOTO_LINE => self.open_prompt(PromptKind::GotoLine, "Go to line", "42"),
            ids::TOGGLE_COMMENT => self.toggle_comment(),
            ids::INDENT => self.with_doc(|d| d.indent()),
            ids::OUTDENT => self.with_doc(|d| d.outdent()),
            ids::MOVE_LINE_UP => self.with_doc(|d| d.move_line_up()),
            ids::MOVE_LINE_DOWN => self.with_doc(|d| d.move_line_down()),
            ids::DUPLICATE_LINE => self.with_doc(|d| d.duplicate_line()),
            ids::DELETE_LINE => self.with_doc(|d| d.delete_line()),
            ids::MATCHING_BRACKET => self.goto_matching_bracket(),
            ids::TOGGLE_TREE => self.toggle_tree(),
            ids::FOCUS_TREE => self.focus_tree(),
            ids::TOGGLE_HIDDEN => self.toggle_hidden(),
            ids::TOGGLE_INLINE_DIAGNOSTICS => self.toggle_inline_diagnostics(),
            ids::REFRESH => self.refresh_workspace(),
            ids::SPLIT => self.toggle_split(),
            ids::FOCUS_PANE => self.focus_other_pane(),
            ids::FILTER_TREE => self.open_tree_filter(),
            ids::NEXT_TAB => self.next_tab(),
            ids::PREV_TAB => self.previous_tab(),
            ids::PALETTE => self.open_command_palette(),
            ids::HELP => self.toggle_help(),
            ids::RENAME => self.rename_symbol(),
            ids::CODE_ACTIONS => self.code_actions(),
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
            PickerAction::Info(message) => self.set_status(message),
            PickerAction::InstallTool(tool) => self.install_tool(tool),
            PickerAction::ApplyCodeAction(index) => self.apply_code_action(index),
            PickerAction::DeletePath(path) => self.delete_path(&path),
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
            PromptKind::Rename => {
                if input.is_empty() {
                    self.set_error("No new name provided");
                    return;
                }
                if let Some((path, row, col)) = self.pending_rename.take()
                    && let Some(server) = self.lsp.as_mut()
                {
                    server.rename(&path, row, col, &input);
                }
                self.set_status("Renaming…");
            }
            PromptKind::NewFile => {
                if input.is_empty() {
                    self.set_error("No file name provided");
                    return;
                }
                let path = if Path::new(&input).is_absolute() {
                    PathBuf::from(&input)
                } else {
                    self.new_file_dir().join(&input)
                };
                match filesystem::create_empty_file(&path) {
                    Ok(()) => {
                        self.tree_visible = true;
                        self.workspace.tree.refresh();
                        self.workspace.tree.select_path(&path);
                        self.open_path(path);
                    }
                    Err(err) => self.set_error(format!("Could not create file: {err}")),
                }
            }
            PromptKind::RenameFile => {
                let Some(old) = self.pending_rename_file.take() else {
                    return;
                };
                if input.is_empty() {
                    self.set_error("No new name provided");
                    return;
                }
                let new = if Path::new(&input).is_absolute() {
                    PathBuf::from(&input)
                } else {
                    old.parent()
                        .map(|parent| parent.join(&input))
                        .unwrap_or_else(|| PathBuf::from(&input))
                };
                if new == old {
                    self.set_status("Name unchanged");
                    return;
                }
                match filesystem::rename_path(&old, &new) {
                    Ok(()) => {
                        self.after_file_rename(&old, &new);
                        self.set_status(format!("Renamed to {}", new.display()));
                    }
                    Err(err) => self.set_error(format!("Rename failed: {err}")),
                }
            }
            PromptKind::ProjectSearch => {
                if input.is_empty() {
                    self.set_error("No search text provided");
                    return;
                }
                self.project_search_seq += 1;
                let revision = self.project_search_seq;
                self.pending_project_search = Some(revision);
                self.background.search_project(
                    self.workspace.root().to_path_buf(),
                    input.clone(),
                    revision,
                );
                self.set_status(format!("Searching for \"{input}\"…"));
            }
            PromptKind::CommitMessage => {
                if input.is_empty() {
                    self.set_error("No commit message provided");
                    return;
                }
                if self.pending_commit {
                    self.set_status("A commit is already running");
                    return;
                }
                self.pending_commit = true;
                self.background
                    .commit_all(self.workspace.root().to_path_buf(), input.clone());
                self.set_status(format!("Committing: {input}"));
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
                self.push_kill(&text);
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
                self.push_kill(&text);
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

    /// Record `text` as the newest kill, deduplicating consecutive copies.
    fn push_kill(&mut self, text: &str) {
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

    /// Replace the last paste with an earlier kill, Emacs-style.
    fn yank_pop(&mut self) {
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

    /// Move the cursor to the bracket matching the one under it.
    fn goto_matching_bracket(&mut self) {
        let Some(language) = self.editor.active_document().map(|doc| doc.buffer.language) else {
            return;
        };
        let provider = self.language.provider(language);
        if let Some(doc) = self.editor.active_document_mut() {
            doc.goto_matching_bracket(provider);
        }
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
            Ok(index) => {
                // The focused pane adopts the newly opened document.
                self.set_active_pane_index(index);
                self.sync_active_pane();
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

    // ----------------------------------------------------------------------
    // Session persistence
    // ----------------------------------------------------------------------

    /// Snapshot the session worth restoring for the active project.
    fn capture_session(&self) -> Session {
        let mut files = Vec::new();
        let mut cursors = Vec::new();
        for doc in &self.editor.documents {
            if let Some(path) = &doc.buffer.path {
                files.push(path.clone());
                cursors.push((doc.cursor.row, doc.cursor.col));
            }
        }
        let active = self
            .editor
            .active_document()
            .and_then(|doc| doc.buffer.path.clone())
            .and_then(|path| files.iter().position(|file| *file == path))
            .unwrap_or(0);
        Session {
            files,
            active,
            cursors,
            expanded: self.workspace.tree.expanded_paths(),
            show_hidden: self.workspace.tree.show_hidden,
        }
    }

    /// Restore a saved session into the current workspace.
    fn restore_session(&mut self, session: Session) {
        if session.show_hidden && !self.workspace.tree.show_hidden {
            self.workspace.tree.toggle_hidden();
        }
        self.workspace.tree.set_expanded(&session.expanded);

        for (index, path) in session.files.iter().enumerate() {
            if !path.is_file() {
                continue;
            }
            self.open_path(path.clone());
            if let Some(&(row, col)) = session.cursors.get(index)
                && let Some(doc) = self.editor.active_document_mut()
            {
                doc.move_to(Position::new(row, col));
            }
        }

        if !self.editor.is_empty() {
            let active = session.active.min(self.editor.len().saturating_sub(1));
            self.editor.set_active(active);
            self.pane_left = active;
            self.pane_right = None;
            self.split = false;
            self.focus_pane = Pane::Primary;
            self.request_detection_for_active();
            self.set_status(format!("Restored {} file(s)", self.editor.len()));
        }
    }

    /// Persist the session for the active project. Errors are non-fatal.
    pub fn save_session(&self) {
        let Some(path) = session::session_path(self.workspace.root()) else {
            return;
        };
        let _ = self.capture_session().save_to(&path);
    }

    /// Save the active document, if it has a path.
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
            .map(|doc| doc.save_as(&path));
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
        self.remap_pane_indices(index);
        self.set_status("Tab closed");
    }

    fn close_all(&mut self) {
        if self.editor.has_unsaved() {
            self.set_error("Unsaved changes — save first (Ctrl+S)");
            return;
        }
        self.editor.close_all();
        self.pane_left = 0;
        self.pane_right = None;
        self.split = false;
        self.focus_pane = Pane::Primary;
        self.close_armed = None;
        self.set_status("All tabs closed");
    }

    /// Discard the active file's edits and reload it from disk.
    fn revert_file(&mut self) {
        let Some(doc) = self.editor.active_document_mut() else {
            return;
        };
        if !doc.is_dirty() {
            self.set_status("No changes to revert");
            return;
        }
        match doc.reload_from_disk() {
            Ok(true) => {
                self.close_armed = None;
                self.after_edit();
                self.set_status("Reverted to the version on disk");
            }
            Ok(false) => self.set_status("This file is not on disk"),
            Err(err) => self.set_error(format!("Revert failed: {err}")),
        }
    }

    /// The directory a new file should be created in: the selected folder when
    /// the tree has focus, otherwise the active file's folder or the root.
    fn new_file_dir(&self) -> PathBuf {
        if self.focus == Focus::FileTree
            && let Some(entry) = self.workspace.tree.selected_entry()
        {
            return if entry.is_dir {
                entry.path.clone()
            } else {
                entry
                    .path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| self.workspace.root().to_path_buf())
            };
        }
        self.editor
            .active_document()
            .and_then(|doc| doc.buffer.path.as_ref())
            .and_then(|path| path.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.workspace.root().to_path_buf())
    }

    fn new_file(&mut self) {
        let dir = self.new_file_dir();
        let label = match dir.strip_prefix(self.workspace.root()) {
            Ok(relative) if !relative.as_os_str().is_empty() => {
                format!("New file in {}", relative.display())
            }
            _ => "New file".to_string(),
        };
        self.open_prompt(PromptKind::NewFile, &label, "name.rs");
    }

    /// The file or folder a rename or delete should act on.
    fn file_op_target(&self) -> Option<PathBuf> {
        if self.focus == Focus::FileTree
            && let Some(entry) = self.workspace.tree.selected_entry()
        {
            return Some(entry.path.clone());
        }
        if let Some(path) = self
            .editor
            .active_document()
            .and_then(|doc| doc.buffer.path.clone())
        {
            return Some(path);
        }
        self.workspace
            .tree
            .selected_entry()
            .map(|entry| entry.path.clone())
    }

    fn rename_selected(&mut self) {
        let Some(path) = self.file_op_target() else {
            self.set_status("Nothing to rename");
            return;
        };
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        self.pending_rename_file = Some(path);
        let mut prompt = Prompt::new(PromptKind::RenameFile, "Rename", "new name");
        prompt.input = name;
        self.overlay = Overlay::Prompt(prompt);
    }

    fn delete_selected(&mut self) {
        let Some(path) = self.file_op_target() else {
            self.set_status("Nothing to delete");
            return;
        };
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        let kind = if path.is_dir() { "folder" } else { "file" };
        let delete = PickerItem::new(
            format!("Delete {kind}"),
            format!("Permanently remove {name}"),
            PickerAction::DeletePath(path),
        )
        .shortcut("Enter");
        let cancel = PickerItem::new(
            "Cancel",
            "Keep it",
            PickerAction::Info("Nothing deleted".to_string()),
        );
        let mut picker = Picker::new(format!("Delete {name}?"), "Choose…", vec![delete, cancel]);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Update open documents and recent files after a rename or move.
    fn after_file_rename(&mut self, old: &Path, new: &Path) {
        for doc in &mut self.editor.documents {
            let Some(path) = doc.buffer.path.clone() else {
                continue;
            };
            if path == old {
                doc.set_path(new.to_path_buf());
            } else if let Ok(relative) = path.strip_prefix(old) {
                doc.set_path(new.join(relative));
            }
        }
        self.recent_files = std::mem::take(&mut self.recent_files)
            .into_iter()
            .map(|path| {
                if path == old {
                    new.to_path_buf()
                } else if let Ok(relative) = path.strip_prefix(old) {
                    new.join(relative)
                } else {
                    path
                }
            })
            .collect();
        self.workspace.tree.refresh();
        self.workspace.tree.select_path(new);
        self.close_armed = None;
        self.request_detection_for_active();
    }

    /// Delete `path` (a file or a whole directory), closing affected buffers.
    fn delete_path(&mut self, path: &Path) {
        let modified = self.editor.documents.iter().any(|doc| {
            doc.is_dirty()
                && doc
                    .buffer
                    .path
                    .as_deref()
                    .is_some_and(|candidate| candidate.starts_with(path))
        });
        if modified {
            self.set_error("Save or close modified files before deleting");
            return;
        }
        match filesystem::remove_path(path) {
            Ok(()) => {
                let mut index = self.editor.documents.len();
                while index > 0 {
                    index -= 1;
                    let matches = self.editor.documents[index]
                        .buffer
                        .path
                        .as_deref()
                        .is_some_and(|candidate| candidate.starts_with(path));
                    if matches {
                        self.editor.close(index);
                    }
                }
                self.clamp_panes();
                self.recent_files
                    .retain(|candidate| !candidate.starts_with(path));
                self.workspace.tree.refresh();
                self.close_armed = None;
                self.set_status(format!("Deleted {}", path.display()));
            }
            Err(err) => self.set_error(format!("Delete failed: {err}")),
        }
    }

    fn save_all(&mut self) {
        let mut saved = 0usize;
        let mut error = None;
        for doc in &mut self.editor.documents {
            if doc.is_dirty() && doc.buffer.path.is_some() {
                match doc.save() {
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

    /// Toggle the inline diagnostic messages at the end of each line.
    fn toggle_inline_diagnostics(&mut self) {
        self.inline_diagnostics = !self.inline_diagnostics;
        let state = if self.inline_diagnostics {
            "shown"
        } else {
            "hidden"
        };
        self.set_status(format!("Inline diagnostics {state}"));
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
        let fallback = self.workspace.marker_names();
        if let Some(path) = self
            .editor
            .active_document()
            .and_then(|doc| doc.buffer.path.clone())
        {
            // Use the file's nearest project markers so a monorepo subproject
            // contributes its own context.
            let markers = crate::project::nearest_markers(&path, &fallback);
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
                let changed = if let Some(doc) = self
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
                    changed
                } else {
                    false
                };
                if changed {
                    self.maybe_start_lsp(language);
                }
                changed
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
            BackgroundEvent::GitCommitted { result } => {
                self.pending_commit = false;
                match result {
                    Ok(message) => self.set_status(message),
                    Err(message) => self.set_error(format!("Commit failed — {message}")),
                }
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
            BackgroundEvent::SearchResults { revision, matches } => {
                if self.pending_project_search != Some(revision) {
                    return false;
                }
                self.pending_project_search = None;
                self.open_project_search_picker(matches);
                true
            }
            BackgroundEvent::Tools(registry) => {
                self.tools = Some(registry);
                // Discovery may finish after a file was already detected.
                if let Some(language) = self
                    .editor
                    .active_document()
                    .map(|doc| doc.buffer.language)
                    .filter(|language| *language != LanguageId::Unknown)
                {
                    self.maybe_start_lsp(language);
                }
                true
            }
            BackgroundEvent::ToolInstalled { tool: _, result } => {
                self.pending_install = None;
                match result {
                    Ok(message) => self.set_status(message),
                    Err(message) => self.set_error(format!("Install failed — {message}")),
                }
                true
            }
        }
    }

    /// Prompt for a commit message, then stage everything and commit.
    fn commit_changes(&mut self) {
        if !self.workspace.git.available {
            self.set_status("Not a git repository");
            return;
        }
        if self.workspace.git.files.is_empty() {
            self.set_status("Nothing to commit");
            return;
        }
        self.open_prompt(
            PromptKind::CommitMessage,
            "Commit message",
            "describe the change",
        );
    }

    /// List the files changed in the working tree, newest snapshot.
    fn open_changed_files(&mut self) {
        if !self.workspace.git.available {
            self.set_status("Not a git repository");
            return;
        }
        if self.workspace.git.files.is_empty() {
            self.set_status("Working tree clean");
            return;
        }
        let root = self.workspace.root().to_path_buf();
        let mut entries: Vec<(PathBuf, GitFileStatus)> = self
            .workspace
            .git
            .files
            .iter()
            .map(|(path, status)| (path.clone(), *status))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let items = entries
            .into_iter()
            .map(|(path, status)| {
                let relative = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                PickerItem::new(
                    relative,
                    format!("{} {}", status.indicator(), status.label()),
                    PickerAction::OpenPath(path),
                )
            })
            .collect();
        let mut picker = Picker::new("Changed Files", "Filter files…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Prompt for a project-wide text query.
    fn open_project_search(&mut self) {
        let mut prompt = Prompt::new(
            PromptKind::ProjectSearch,
            "Search in project",
            "text to find",
        );
        // Prefill from a single-line selection, so searching for the word under
        // the cursor is one keystroke.
        if let Some(text) = self
            .editor
            .active_document()
            .and_then(|doc| doc.selected_text())
        {
            let trimmed = text.trim();
            if !trimmed.is_empty() && !trimmed.contains('\n') {
                prompt.input = trimmed.to_string();
            }
        }
        self.overlay = Overlay::Prompt(prompt);
    }

    /// Show the matches from a project-wide search.
    fn open_project_search_picker(&mut self, matches: Vec<SearchMatch>) {
        if matches.is_empty() {
            self.set_status("No matches in project");
            return;
        }
        let total = matches.len();
        let root = self.workspace.root().to_path_buf();
        let items = matches
            .into_iter()
            .map(|entry| {
                let relative = entry
                    .path
                    .strip_prefix(&root)
                    .unwrap_or(&entry.path)
                    .display()
                    .to_string();
                let detail = format!("{relative}:{}", entry.line + 1);
                PickerItem::new(
                    entry.text,
                    detail,
                    PickerAction::Reveal {
                        path: entry.path,
                        position: Position::new(entry.line, entry.col),
                    },
                )
            })
            .collect();
        let mut picker = Picker::new("Search Results", "Filter results…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
        self.set_status(format!("{total} match(es)"));
    }

    /// Ask for project-wide symbols: the language server when attached, plus the
    /// built-in scan as an immediate, always-available fallback.
    fn open_workspace_symbols(&mut self) {
        if self.lsp.as_ref().is_some_and(|server| server.is_ready()) {
            self.ws_lsp_pending = true;
            if let Some(server) = self.lsp.as_mut() {
                server.workspace_symbols("");
            }
        }
        self.workspace_symbols_seq += 1;
        let revision = self.workspace_symbols_seq;
        self.pending_workspace_symbols = Some(revision);
        self.background
            .workspace_symbols(self.workspace.root().to_path_buf(), revision);
        self.set_status("Searching symbols…");
    }

    fn open_workspace_symbol_picker(&mut self, symbols: Vec<WorkspaceSymbol>) {
        if symbols.is_empty() {
            // A server may still be answering; only report failure when nothing
            // else is coming and no picker is showing.
            if !self.ws_lsp_pending && self.overlay.is_none() {
                self.set_status("No symbols found");
            }
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
        self.merge_workspace_symbols(items);
    }

    /// Show or extend the workspace-symbol picker, keeping any results already
    /// listed. Used by both the built-in scan and the language server.
    fn merge_workspace_symbols(&mut self, items: Vec<PickerItem>) {
        if let Overlay::Picker(picker) = &mut self.overlay
            && picker.title == "Workspace Symbols"
        {
            picker.extend_items(items);
            return;
        }
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

        self.diagnostics_seq += 1;
        let revision = self.diagnostics_seq;
        if let Some(doc) = self.editor.active_document_mut() {
            doc.set_diagnostics_revision(revision);
        }

        // If a language server owns this document, stream the change to it and
        // let it publish fresh diagnostics.
        let owned_by_lsp = self.lsp.as_ref().is_some_and(|server| {
            server.is_ready()
                && path
                    .as_deref()
                    .is_some_and(|path| server.has_open_document(path))
        });
        if owned_by_lsp {
            let text = self
                .editor
                .active_document()
                .map(|doc| doc.buffer.text())
                .unwrap_or_default();
            if let (Some(server), Some(path)) = (self.lsp.as_mut(), path.as_deref()) {
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

    // ----------------------------------------------------------------------
    // Language server
    // ----------------------------------------------------------------------

    /// Start a language server for `language` when one is installed, after a
    /// short delay so opening a file never waits on server startup.
    fn maybe_start_lsp(&mut self, language: LanguageId) {
        if self.lsp.is_some() || self.lsp_language.is_some() || self.lsp_start_at.is_some() {
            return;
        }
        let Some(tools) = &self.tools else {
            return;
        };
        let Some(tool) = Tool::for_language(language, ToolPurpose::LanguageServer) else {
            return;
        };
        if !tools.available(tool) {
            return;
        }
        self.lsp_start_at = Some((Instant::now() + LSP_START_DELAY, language));
    }

    /// Start the scheduled language server once its delay has elapsed.
    fn poll_lsp_start(&mut self) {
        let Some((at, language)) = self.lsp_start_at else {
            return;
        };
        if Instant::now() < at {
            return;
        }
        self.lsp_start_at = None;
        let Some(tool) = Tool::for_language(language, ToolPurpose::LanguageServer) else {
            return;
        };
        // Prefer the executable Koda discovered, which may live in a user bin
        // directory outside the process PATH.
        let program = self
            .tools
            .as_ref()
            .and_then(|tools| tools.program_path(tool))
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| tool.program().to_string());
        self.start_lsp(language, &program, &[]);
    }

    /// Start a language server, recording our attempt either way.
    fn start_lsp(&mut self, language: LanguageId, program: &str, args: &[&str]) {
        self.lsp_language = Some(language);
        self.lsp_started_at = Some(Instant::now());
        let root = self.workspace.root().to_path_buf();
        match Server::start(language, program, args, &root) {
            Ok(server) => {
                self.lsp = Some(server);
                self.lsp_status = LspStatus::Starting;
            }
            Err(err) => {
                self.lsp_language = None;
                self.lsp_started_at = None;
                self.lsp_status = LspStatus::Failed(err.to_string());
                self.set_error(format!("Could not start {program}: {err}"));
                self.schedule_lsp_restart(language);
            }
        }
    }

    /// Give up on a handshake that has taken too long, falling back cleanly.
    ///
    /// A server that never answers `initialize` would otherwise leave Koda
    /// "connecting" forever; this turns that hang into the same graceful
    /// fallback as a crash.
    fn poll_lsp_health(&mut self) -> bool {
        if self.lsp_status != LspStatus::Starting {
            return false;
        }
        let Some(started) = self.lsp_started_at else {
            return false;
        };
        if started.elapsed() < LSP_HANDSHAKE_TIMEOUT {
            return false;
        }
        let language = self.lsp_language.take();
        self.lsp = None;
        self.lsp_started_at = None;
        self.lsp_status = LspStatus::Failed("initialize timed out".to_string());
        for doc in &mut self.editor.documents {
            doc.use_builtin_diagnostics();
        }
        self.diagnostics_dirty_at = Some(Instant::now());
        if let Some(language) = language {
            self.set_error(format!(
                "{} did not respond; using built-in intelligence",
                language.name()
            ));
            self.schedule_lsp_restart(language);
        }
        true
    }

    /// Schedule a bounded automatic restart for `language`, if a server tool is
    /// installed and the retry budget is not exhausted.
    fn schedule_lsp_restart(&mut self, language: LanguageId) {
        if self.lsp_restarts >= MAX_LSP_RESTARTS || self.lsp_start_at.is_some() {
            return;
        }
        let Some(tools) = &self.tools else {
            return;
        };
        let Some(tool) = Tool::for_language(language, ToolPurpose::LanguageServer) else {
            return;
        };
        if !tools.available(tool) {
            return;
        }
        self.lsp_restarts += 1;
        self.lsp_start_at = Some((Instant::now() + LSP_RESTART_DELAY, language));
        self.set_status(format!("Restarting {}…", tool.label()));
    }

    /// Restart the language server for the active document on demand.
    fn restart_language_server(&mut self) {
        let language = self
            .editor
            .active_document()
            .map(|doc| doc.buffer.language)
            .unwrap_or(LanguageId::Unknown);
        let Some(tool) = Tool::for_language(language, ToolPurpose::LanguageServer) else {
            self.set_status(format!("No language server for {}", language.name()));
            return;
        };
        let available = self
            .tools
            .as_ref()
            .is_some_and(|tools| tools.available(tool));
        if !available {
            self.set_status(format!(
                "{} is not installed — see Language Setup…",
                tool.label()
            ));
            return;
        }

        // Drop the current connection and start a fresh one immediately.
        self.lsp = None;
        self.lsp_language = None;
        self.lsp_start_at = None;
        self.lsp_started_at = None;
        self.lsp_restarts = 0;
        self.lsp_status = LspStatus::Offline;
        for doc in &mut self.editor.documents {
            doc.use_builtin_diagnostics();
        }
        self.lsp_start_at = Some((Instant::now(), language));
        self.set_status(format!("Restarting {}…", tool.label()));
    }

    /// Drain language-server events. Returns `true` when something changed.
    fn poll_lsp(&mut self) -> bool {
        let events = match self.lsp.as_mut() {
            Some(server) => server.poll(),
            None => return false,
        };
        if events.is_empty() {
            return false;
        }
        for event in events {
            match event {
                ServerEvent::Ready => self.lsp_ready(),
                ServerEvent::Diagnostics { path, diagnostics } => {
                    self.apply_lsp_diagnostics(&path, diagnostics);
                }
                ServerEvent::Response { kind, result } => {
                    self.handle_lsp_response(kind, result);
                }
                ServerEvent::ApplyEdit { id, params } => {
                    let edit = params.get("edit").cloned().unwrap_or(Value::Null);
                    let files = convert::workspace_edit(&edit);
                    let applied = self.apply_workspace_edit(files);
                    if let Some(server) = self.lsp.as_mut() {
                        server.apply_edit_response(&id, applied > 0);
                    }
                    if applied > 0 {
                        self.set_status(format!("Applied {applied} edit(s)"));
                    }
                }
                ServerEvent::Failed(message) => {
                    let language = self.lsp_language.take();
                    self.lsp = None;
                    self.lsp_started_at = None;
                    self.lsp_status = LspStatus::Failed(message.clone());
                    // Fall back to the built-in providers for every document.
                    for doc in &mut self.editor.documents {
                        doc.use_builtin_diagnostics();
                    }
                    self.diagnostics_dirty_at = Some(Instant::now());
                    self.set_error(format!(
                        "Language server stopped — {message}; using built-in intelligence"
                    ));
                    if let Some(language) = language {
                        self.schedule_lsp_restart(language);
                    }
                }
            }
        }
        true
    }

    /// The handshake finished: open every matching document on the server.
    fn lsp_ready(&mut self) {
        self.lsp_status = LspStatus::Ready;
        // A healthy connection earns a fresh restart budget for later crashes.
        self.lsp_restarts = 0;
        let Some(language) = self.lsp_language else {
            return;
        };
        let documents: Vec<(PathBuf, String)> = self
            .editor
            .documents
            .iter()
            .filter(|doc| doc.buffer.language == language)
            .filter_map(|doc| {
                doc.buffer
                    .path
                    .clone()
                    .map(|path| (path, doc.buffer.text()))
            })
            .collect();
        if let Some(server) = self.lsp.as_mut() {
            for (path, text) in documents {
                server.did_open(&path, &text);
            }
        }
    }

    fn apply_lsp_diagnostics(&mut self, path: &Path, diagnostics: Vec<Diagnostic>) {
        if let Some(doc) = self
            .editor
            .documents
            .iter_mut()
            .find(|doc| same_file(doc.buffer.path.as_deref(), path))
        {
            doc.set_lsp_diagnostics(diagnostics);
        }
    }

    /// The active document's `(path, line, col)` when a ready server owns it.
    fn lsp_target(&self) -> Option<(PathBuf, usize, usize)> {
        let server = self.lsp.as_ref()?;
        if !server.is_ready() {
            return None;
        }
        let doc = self.editor.active_document()?;
        let path = doc.buffer.path.clone()?;
        if !server.has_open_document(&path) {
            return None;
        }
        let cursor = doc.clamped_cursor();
        Some((path, cursor.row, cursor.col))
    }

    /// Apply a language-server feature response.
    fn handle_lsp_response(&mut self, kind: RequestKind, result: Result<Value, String>) {
        let value = match result {
            Ok(value) => value,
            Err(message) => {
                if kind == RequestKind::WorkspaceSymbols {
                    // Keep the built-in scan's results; a server that cannot
                    // answer workspace symbols is not an error worth shouting.
                    self.ws_lsp_pending = false;
                } else {
                    self.set_error(format!("Language server: {message}"));
                }
                return;
            }
        };
        match kind {
            RequestKind::Completion => {
                let items = convert::completions(&value);
                if let Some(state) = self.completion.as_mut() {
                    state.extend(items);
                }
            }
            RequestKind::Hover => match convert::hover(&value) {
                Some(hover) => {
                    self.hover = Some(HoverState {
                        title: hover.title,
                        kind: hover.kind,
                        body: hover.body,
                    });
                }
                None if self.hover.is_none() => {
                    self.set_status("No information available");
                }
                None => {}
            },
            RequestKind::Definition => {
                let locations = convert::locations(&value);
                match locations.into_iter().next() {
                    Some(location) => {
                        self.reveal(location.path, Position::new(location.line, location.col));
                    }
                    None => self.set_status("No definition found"),
                }
            }
            RequestKind::References => {
                let locations = convert::locations(&value);
                if locations.is_empty() {
                    self.set_status("No references found");
                } else {
                    self.open_location_picker("References", locations);
                }
            }
            RequestKind::Rename => {
                let files = convert::workspace_edit(&value);
                let applied = self.apply_workspace_edit(files);
                if applied > 0 {
                    self.set_status(format!("Renamed in {applied} place(s)"));
                } else {
                    self.set_status("Nothing to rename");
                }
            }
            RequestKind::CodeActions => {
                let actions = convert::code_actions(&value);
                if actions.is_empty() {
                    self.set_status("No code actions available");
                    return;
                }
                self.pending_code_actions = actions;
                let items = self
                    .pending_code_actions
                    .iter()
                    .enumerate()
                    .map(|(index, action)| {
                        PickerItem::new(
                            action.title.clone(),
                            String::new(),
                            PickerAction::ApplyCodeAction(index),
                        )
                    })
                    .collect();
                let mut picker = Picker::new("Code Actions", "Filter actions…", items);
                picker.refilter();
                self.overlay = Overlay::Picker(picker);
            }
            RequestKind::WorkspaceSymbols => {
                self.ws_lsp_pending = false;
                let root = self.workspace.root().to_path_buf();
                let items: Vec<PickerItem> = convert::workspace_symbols(&value)
                    .into_iter()
                    .take(2000)
                    .map(|item| {
                        let relative = item
                            .path
                            .strip_prefix(&root)
                            .unwrap_or(&item.path)
                            .display()
                            .to_string();
                        let detail =
                            format!("{}  ·  {relative}:{}", item.kind.label(), item.line + 1);
                        PickerItem::new(
                            item.name,
                            detail,
                            PickerAction::Reveal {
                                path: item.path,
                                position: Position::new(item.line, item.col),
                            },
                        )
                    })
                    .collect();
                if !items.is_empty() {
                    self.merge_workspace_symbols(items);
                }
            }
        }
    }

    /// Ask the server for code actions over the cursor or selection.
    fn code_actions(&mut self) {
        let Some((path, row, col)) = self.lsp_target() else {
            self.set_status("Code actions need a language server (see Language Setup…)");
            return;
        };
        let range = self
            .editor
            .active_document()
            .and_then(|doc| doc.selection_range())
            .map(|(start, end)| ((start.row, start.col), (end.row, end.col)))
            .unwrap_or(((row, col), (row, col)));
        if let Some(server) = self.lsp.as_mut() {
            server.code_action(&path, range.0, range.1);
        }
        self.set_status("Finding code actions…");
    }

    /// Apply the code action at `index`, either as an edit or a command.
    fn apply_code_action(&mut self, index: usize) {
        let Some(action) = self.pending_code_actions.get(index).cloned() else {
            return;
        };
        if let Some(edit) = action.edit {
            let files = convert::workspace_edit(&edit);
            let applied = self.apply_workspace_edit(files);
            if applied > 0 {
                self.set_status(format!("Applied {applied} edit(s)"));
            } else {
                self.set_status("Nothing to apply");
            }
        } else if let Some(command) = action.command {
            if let Some(server) = self.lsp.as_mut() {
                server.execute_command(&command.command, command.arguments);
            }
            self.set_status("Running action…");
        } else {
            self.set_status("This action does nothing");
        }
    }

    /// Prompt for a new name and ask the server to rename the symbol.
    fn rename_symbol(&mut self) {
        let Some((path, row, col)) = self.lsp_target() else {
            self.set_status("Rename needs a language server (see Language Setup…)");
            return;
        };
        let word = self
            .editor
            .active_document()
            .and_then(|doc| crate::language::symbols::word_at(&doc.buffer.text(), row, col))
            .unwrap_or_default();

        self.pending_rename = Some((path, row, col));
        let mut prompt = Prompt::new(PromptKind::Rename, "Rename symbol", "new name");
        prompt.input = word;
        self.overlay = Overlay::Prompt(prompt);
    }

    /// Apply a workspace edit, returning the number of edits applied.
    fn apply_workspace_edit(&mut self, files: Vec<convert::FileEdit>) -> usize {
        let mut applied = 0;
        for file in files {
            if let Some(doc) = self
                .editor
                .documents
                .iter_mut()
                .find(|doc| same_file(doc.buffer.path.as_deref(), &file.path))
            {
                // Apply from the end so earlier offsets stay valid.
                let mut edits = file.edits;
                edits.sort_by_key(|edit| Reverse((edit.start.0, edit.start.1)));
                for edit in edits {
                    doc.replace_range(
                        Position::new(edit.start.0, edit.start.1),
                        Position::new(edit.end.0, edit.end.1),
                        &edit.new_text,
                    );
                    applied += 1;
                }
            } else if let Ok(text) = std::fs::read_to_string(&file.path) {
                let updated = apply_text_edits(&text, &file.edits);
                if std::fs::write(&file.path, updated).is_ok() {
                    applied += file.edits.len();
                }
            }
        }
        applied
    }

    /// Offer a picker of jump targets.
    fn open_location_picker(&mut self, title: &str, locations: Vec<convert::Location>) {
        let root = self.workspace.root().to_path_buf();
        let items = locations
            .into_iter()
            .map(|location| {
                let relative = location
                    .path
                    .strip_prefix(&root)
                    .unwrap_or(&location.path)
                    .display()
                    .to_string();
                let detail = format!("{}:{}", location.line + 1, location.col + 1);
                PickerItem::new(
                    relative,
                    detail,
                    PickerAction::Reveal {
                        path: location.path,
                        position: Position::new(location.line, location.col),
                    },
                )
            })
            .collect();
        let mut picker = Picker::new(title, "Filter locations…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
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

    /// Jump to the definition of the word under the cursor.
    fn goto_definition(&mut self) {
        if let Some((path, row, col)) = self.lsp_target() {
            if let Some(server) = self.lsp.as_mut() {
                server.definition(&path, row, col);
            }
            self.set_status("Resolving definition…");
            return;
        }

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

    /// List every occurrence of the word under the cursor.
    fn find_references(&mut self) {
        if let Some((path, row, col)) = self.lsp_target() {
            if let Some(server) = self.lsp.as_mut() {
                server.references(&path, row, col);
            }
            self.set_status("Finding references…");
            return;
        }

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

    /// Replace every current match in the active file as one undoable edit.
    fn replace_all(&mut self) {
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
            ids::YANK_POP if self.last_yank.is_none() => {
                (false, Some("nothing to yank".to_string()))
            }
            ids::REPLACE_ALL if !self.search.open || self.search.query.is_empty() => {
                (false, Some("open find first".to_string()))
            }
            ids::NEXT_TAB | ids::PREV_TAB if self.editor.len() < 2 => {
                (false, Some("only one tab".to_string()))
            }
            ids::CLOSE_TAB | ids::CLOSE_ALL | ids::SAVE_ALL if self.editor.is_empty() => {
                (false, Some("no files open".to_string()))
            }
            ids::REVERT if !document.is_some_and(|doc| doc.is_dirty()) => {
                (false, Some("no changes to revert".to_string()))
            }
            ids::RENAME_FILE | ids::DELETE_FILE if self.file_op_target().is_none() => {
                (false, Some("select a file first".to_string()))
            }
            ids::GIT_COMMIT if !self.workspace.git.available => {
                (false, Some("not a git repository".to_string()))
            }
            ids::GIT_COMMIT if self.workspace.git.files.is_empty() => {
                (false, Some("nothing to commit".to_string()))
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
            ids::FORMAT => self.format_availability(document),
            ids::RENAME if self.lsp_target().is_none() => {
                (false, Some("needs a language server".to_string()))
            }
            ids::CODE_ACTIONS if self.lsp_target().is_none() => {
                (false, Some("needs a language server".to_string()))
            }
            _ => (true, None),
        }
    }

    /// Whether formatting can run, and why not when it cannot.
    fn format_availability(&self, document: Option<&Document>) -> (bool, Option<String>) {
        let language = document
            .map(|doc| doc.buffer.language)
            .unwrap_or(LanguageId::Unknown);
        let Some(tool) =
            Tool::for_language(language, crate::language::tools::ToolPurpose::Formatter)
        else {
            return (true, None);
        };
        // Prefer the probed registry; fall back to a PATH check before it lands.
        let available = match &self.tools {
            Some(tools) => tools.available(tool),
            None => format::is_available(tool.program()),
        };
        if available {
            (true, None)
        } else {
            (false, Some(format!("{} is not installed", tool.program())))
        }
    }

    /// Install a tool through its trusted package manager, on the worker.
    fn install_tool(&mut self, tool: Tool) {
        if self.pending_install.is_some() {
            self.set_status("An install is already running");
            return;
        }
        self.pending_install = Some(tool);
        self.background.install_tool(tool);
        self.set_status(format!("Installing {}…", tool.label()));
    }

    /// Offer, once per language, to install a missing language server.
    ///
    /// This is called from the interactive event loop rather than at startup, so
    /// the prompt never blocks the first frame and never appears when Koda is
    /// embedded (tests, previews). The user always chooses; dismissing it keeps
    /// Koda's built-in intelligence, and **Language Setup…** stays available.
    fn maybe_offer_tool_setup(&mut self) -> bool {
        if !self.overlay.is_none()
            || self.pending_install.is_some()
            || self.lsp.is_some()
            || self.lsp_language.is_some()
            || self.lsp_start_at.is_some()
        {
            return false;
        }
        let Some(tools) = self.tools.as_ref() else {
            return false;
        };
        let Some(language) = self.editor.active_document().map(|doc| doc.buffer.language) else {
            return false;
        };
        if language == LanguageId::Unknown {
            return false;
        }
        let Some(tool) = Tool::for_language(language, ToolPurpose::LanguageServer) else {
            return false;
        };
        if tools.available(tool) || tool.install_command().is_none() {
            return false;
        }
        if !self.setup_offered.insert(language) {
            return false;
        }

        let install = PickerItem::new(
            format!("Install {}", tool.label()),
            tool.install_hint().to_string(),
            PickerAction::InstallTool(tool),
        )
        .shortcut("Enter");
        let later = PickerItem::new(
            "Not now",
            "Keep Koda's built-in intelligence — install later from Language Setup…",
            PickerAction::Info("Install language tools any time from Language Setup…".to_string()),
        );
        let mut picker = Picker::new(
            format!("{} is not installed", tool.label()),
            "Choose…",
            vec![install, later],
        );
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
        true
    }

    /// Show which language tools Koda found, and how to install the rest.
    fn language_setup(&mut self) {
        let Some(tools) = self.tools.clone() else {
            // Discovery is still running; ask again and tell the user.
            self.background.discover_tools();
            self.set_status("Checking language tools…");
            return;
        };

        let mut items = Vec::new();
        for status in tools.all() {
            let tool = status.tool;
            let purpose = match tool.purpose() {
                crate::language::tools::ToolPurpose::LanguageServer => "language server",
                crate::language::tools::ToolPurpose::Formatter => "formatter",
            };
            let label = format!("{}  ·  {} {purpose}", tool.label(), tool.language().name());
            let item = if status.available {
                let detail = status.summary();
                PickerItem::new(label, detail.clone(), PickerAction::Info(detail))
            } else if tool.install_command().is_some() {
                let hint = tool.install_hint();
                PickerItem::new(label, hint.to_string(), PickerAction::InstallTool(tool))
                    .shortcut("Enter")
            } else {
                let hint = tool.install_hint();
                PickerItem::new(label, hint, PickerAction::Info(hint.to_string())).disabled(hint)
            };
            items.push(item);
        }
        let mut picker = Picker::new("Language Setup", "Language tools…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
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

    /// Show the keyboard-shortcuts cheatsheet, or hide it if it is already up.
    fn toggle_help(&mut self) {
        self.completion = None;
        self.hover = None;
        self.overlay = match self.overlay {
            Overlay::Help(_) => Overlay::None,
            _ => Overlay::Help(Help::default()),
        };
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

    /// Re-read the project tree and git status on demand.
    fn refresh_workspace(&mut self) {
        self.workspace.refresh();
        self.request_git_refresh();
        self.set_status("Refreshed");
    }

    /// The document index shown in the left pane.
    pub fn pane_left_index(&self) -> usize {
        self.pane_left
    }

    /// The document index shown in the right pane, when split.
    pub fn pane_right_index(&self) -> Option<usize> {
        self.pane_right
    }

    /// The document index shown by the focused pane.
    fn active_pane_index(&self) -> usize {
        match self.focus_pane {
            Pane::Primary => self.pane_left,
            Pane::Secondary => self.pane_right.unwrap_or(self.pane_left),
        }
    }

    /// Record that the focused pane now shows `index`.
    fn set_active_pane_index(&mut self, index: usize) {
        match self.focus_pane {
            Pane::Primary => self.pane_left = index,
            Pane::Secondary => self.pane_right = Some(index),
        }
    }

    /// Make `editor.active` follow the focused pane.
    fn sync_active_pane(&mut self) {
        let index = self.active_pane_index();
        self.editor.set_active(index);
    }

    /// Activate the next tab within the focused pane.
    fn next_tab(&mut self) {
        if self.editor.is_empty() {
            return;
        }
        self.editor.next_tab();
        let index = self.editor.active_index();
        self.set_active_pane_index(index);
    }

    /// Activate the previous tab within the focused pane.
    fn previous_tab(&mut self) {
        if self.editor.is_empty() {
            return;
        }
        self.editor.previous_tab();
        let index = self.editor.active_index();
        self.set_active_pane_index(index);
    }

    /// Toggle the side-by-side editor split.
    fn toggle_split(&mut self) {
        if self.split {
            self.split = false;
            self.pane_right = None;
            self.focus_pane = Pane::Primary;
            self.editor.set_active(self.pane_left);
            self.set_status("Split closed");
            return;
        }
        if self.editor.len() < 2 {
            self.set_status("Open another file to split");
            return;
        }
        let active = self.editor.active_index();
        self.pane_left = active;
        self.pane_right = Some((active + 1) % self.editor.len());
        self.split = true;
        self.focus_pane = Pane::Primary;
        self.editor.set_active(active);
        self.set_status("Split · Alt+O switches panes");
    }

    /// Move editing focus to the other pane.
    fn focus_other_pane(&mut self) {
        if !self.split || self.pane_right.is_none() {
            self.set_status("No split to focus");
            return;
        }
        self.focus_pane = match self.focus_pane {
            Pane::Primary => Pane::Secondary,
            Pane::Secondary => Pane::Primary,
        };
        self.sync_active_pane();
        if self.search.open {
            self.refresh_search_matches();
        }
        let name = self
            .editor
            .active_document()
            .map(|doc| doc.file_name())
            .unwrap_or_default();
        self.set_status(format!("Focused {name}"));
    }

    /// Keep pane indices valid after document `closed` was removed.
    fn remap_pane_indices(&mut self, closed: usize) {
        let len = self.editor.len();
        // The right pane loses its document if that was the one closed.
        if self.pane_right == Some(closed) {
            self.pane_right = None;
        } else if let Some(right) = self.pane_right
            && right > closed
        {
            self.pane_right = Some(right - 1);
        }
        // When the left pane's document closes, the right pane takes its place
        // and the split collapses.
        if self.pane_left == closed {
            self.pane_left = self.pane_right.take().unwrap_or(0);
        } else if self.pane_left > closed {
            self.pane_left -= 1;
        }
        self.pane_left = self.pane_left.min(len.saturating_sub(1));
        if self.pane_right.is_some_and(|index| index >= len) {
            self.pane_right = None;
        }
        if self.pane_right.is_none() {
            self.split = false;
            if self.focus_pane == Pane::Secondary {
                self.focus_pane = Pane::Primary;
            }
        }
        self.sync_active_pane();
    }

    /// Clamp pane indices after arbitrary document removals.
    fn clamp_panes(&mut self) {
        let len = self.editor.len();
        if len == 0 {
            self.pane_left = 0;
            self.pane_right = None;
            self.split = false;
            self.focus_pane = Pane::Primary;
            return;
        }
        self.pane_left = self.pane_left.min(len - 1);
        if self.pane_right.is_some_and(|index| index >= len) {
            self.pane_right = None;
        }
        if self.pane_right.is_none() {
            self.split = false;
            if self.focus_pane == Pane::Secondary {
                self.focus_pane = Pane::Primary;
            }
        }
        self.sync_active_pane();
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

/// Apply LSP text edits to a string, from the end so offsets stay valid.
fn apply_text_edits(text: &str, edits: &[convert::TextEdit]) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    // Character offset where each line begins.
    let mut line_starts = vec![0usize];
    for (index, ch) in chars.iter().enumerate() {
        if *ch == '\n' {
            line_starts.push(index + 1);
        }
    }
    let total = chars.len();
    let offset = |line: usize, character: usize| -> usize {
        let start = line_starts.get(line).copied().unwrap_or(total);
        (start + character).min(total)
    };

    let mut sorted: Vec<&convert::TextEdit> = edits.iter().collect();
    sorted.sort_by_key(|edit| Reverse((edit.start.0, edit.start.1)));
    for edit in sorted {
        let start = offset(edit.start.0, edit.start.1);
        let end = offset(edit.end.0, edit.end.1).max(start);
        chars.splice(start..end, edit.new_text.chars());
    }
    chars.into_iter().collect()
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
    fn revert_discards_edits_and_reloads_from_disk() {
        let dir = temp_project("revert");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();
        app.editor
            .active_document_mut()
            .unwrap()
            .insert_text("// changed\n");
        assert!(app.editor.active_document().unwrap().is_dirty());

        app.execute_command(ids::REVERT);
        let doc = app.editor.active_document().unwrap();
        assert!(!doc.is_dirty());
        assert!(!doc.buffer.text().contains("// changed"));
        assert!(doc.buffer.text().contains("fn main"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn new_file_creates_and_opens_it() {
        let dir = temp_project("new-file");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();

        app.execute_command(ids::NEW_FILE);
        assert!(matches!(app.overlay, Overlay::Prompt(_)));
        app.submit_prompt(PromptKind::NewFile, "created.rs".to_string());

        let created = dir.join("src/created.rs");
        assert!(created.is_file());
        assert_eq!(
            app.editor.active_document().unwrap().buffer.path.as_deref(),
            Some(created.as_path())
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rename_updates_the_open_document() {
        let dir = temp_project("rename-file");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();

        app.execute_command(ids::RENAME_FILE);
        assert!(matches!(app.overlay, Overlay::Prompt(_)));
        app.submit_prompt(PromptKind::RenameFile, "renamed.rs".to_string());

        assert!(!file.exists());
        let renamed = dir.join("src/renamed.rs");
        assert!(renamed.is_file());
        assert_eq!(
            app.editor.active_document().unwrap().buffer.path.as_deref(),
            Some(renamed.as_path())
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn delete_removes_the_file_and_closes_its_tab() {
        let dir = temp_project("delete-file");
        let a = dir.join("src/main.rs");
        let b = dir.join("src/extra.rs");
        fs::write(&b, "pub fn extra() {}\n").unwrap();
        let mut app = App::new(Some(&a)).unwrap();
        app.open_path(b.clone());
        assert_eq!(app.editor.len(), 2);

        app.delete_path(&b);
        assert!(!b.exists());
        assert_eq!(app.editor.len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn kill_ring_supports_yank_pop() {
        let dir = temp_project("kill-ring");
        let file = dir.join("src/main.rs");
        fs::write(&file, "").unwrap();
        let mut app = App::new(Some(&file)).unwrap();

        app.push_kill("first");
        app.push_kill("second");
        app.paste();
        assert_eq!(
            app.editor.active_document().unwrap().buffer.text(),
            "second"
        );

        app.yank_pop();
        assert_eq!(app.editor.active_document().unwrap().buffer.text(), "first");

        // Nothing older: the text is left alone.
        app.yank_pop();
        assert_eq!(app.editor.active_document().unwrap().buffer.text(), "first");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn yank_pop_is_refused_after_an_edit() {
        let dir = temp_project("yank-edit");
        let file = dir.join("src/main.rs");
        fs::write(&file, "").unwrap();
        let mut app = App::new(Some(&file)).unwrap();
        app.push_kill("first");
        app.push_kill("second");
        app.paste();

        // Any further edit invalidates the yank-pop target.
        app.with_doc(|doc| doc.type_char('!'));
        app.yank_pop();
        assert_eq!(
            app.editor.active_document().unwrap().buffer.text(),
            "second!"
        );
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
    fn split_shows_two_documents_and_focus_switches() {
        let dir = temp_project("split");
        let a = dir.join("src/main.rs");
        let b = dir.join("src/lib.rs");
        fs::write(&a, "fn main() {}\n").unwrap();
        fs::write(&b, "pub fn lib() {}\n").unwrap();
        let mut app = App::new(Some(&a)).unwrap();
        app.open_path(b.clone());
        assert_eq!(app.editor.len(), 2);
        let active = app.editor.active_index();

        app.execute_command(ids::SPLIT);
        assert!(app.split);
        assert_eq!(app.pane_left_index(), active);
        assert_eq!(app.pane_right_index(), Some((active + 1) % 2));
        assert_eq!(app.focus_pane, Pane::Primary);

        app.execute_command(ids::FOCUS_PANE);
        assert_eq!(app.focus_pane, Pane::Secondary);
        assert_eq!(app.editor.active_index(), (active + 1) % 2);

        // Typing edits only the focused (right) document.
        app.with_doc(|doc| doc.insert_text("// right\n"));
        let right = (active + 1) % 2;
        assert!(
            app.editor.documents[right]
                .buffer
                .text()
                .contains("// right")
        );
        assert!(
            !app.editor.documents[active]
                .buffer
                .text()
                .contains("// right")
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn alt_v_splits_and_alt_o_switches_panes() {
        let dir = temp_project("alt-split");
        let a = dir.join("src/main.rs");
        let b = dir.join("src/lib.rs");
        fs::write(&b, "pub fn lib() {}\n").unwrap();
        let mut app = App::new(Some(&a)).unwrap();
        app.open_path(b.clone());

        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT));
        assert!(app.split);

        let before = app.editor.active_index();
        app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::ALT));
        assert_ne!(app.editor.active_index(), before);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn closing_a_split_document_collapses_the_split() {
        let dir = temp_project("split-close");
        let a = dir.join("src/main.rs");
        let b = dir.join("src/lib.rs");
        fs::write(&a, "fn main() {}\n").unwrap();
        fs::write(&b, "pub fn lib() {}\n").unwrap();
        let mut app = App::new(Some(&a)).unwrap();
        app.open_path(b.clone());
        app.execute_command(ids::SPLIT);
        assert!(app.split);

        // Close the focused (left) document; the split collapses to one pane.
        app.execute_command(ids::CLOSE_TAB);
        assert_eq!(app.editor.len(), 1);
        assert!(!app.split);
        assert!(app.pane_right_index().is_none());
        assert_eq!(app.editor.active_index(), app.pane_left_index());

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
        // Formatting availability tracks whether the tool is actually installed.
        let rustfmt_available = app
            .tools
            .as_ref()
            .map(|tools| tools.available(Tool::Rustfmt))
            .unwrap_or_else(|| format::is_available("rustfmt"));
        assert_eq!(
            find("Format Document").expect("format").enabled,
            rustfmt_available
        );
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
    fn refresh_picks_up_new_files() {
        let dir = temp_project("refresh");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();
        assert!(
            !app.workspace
                .tree
                .entries()
                .iter()
                .any(|entry| entry.name == "added.rs")
        );

        fs::write(dir.join("added.rs"), "pub fn added() {}\n").unwrap();
        app.execute_command(ids::REFRESH);
        assert!(
            app.workspace
                .tree
                .entries()
                .iter()
                .any(|entry| entry.name == "added.rs")
        );
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

    #[test]
    fn search_options_change_the_matches() {
        let dir = temp_project("search-options");
        let file = dir.join("src/main.rs");
        fs::write(&file, "Foo foo food foo\n").unwrap();
        let mut app = App::new(Some(&file)).unwrap();

        app.execute_command(ids::FIND);
        app.search.query = "foo".to_string();
        app.refresh_search_matches();
        assert_eq!(app.search.matches.len(), 4, "case-insensitive substring");

        // Alt+C toggles case sensitivity.
        app.handle_search_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT));
        assert!(app.search.case_sensitive);
        assert_eq!(app.search.matches.len(), 3);

        // Alt+W toggles whole-word matching.
        app.handle_search_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT));
        assert!(app.search.whole_word);
        assert_eq!(app.search.matches.len(), 2);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn regex_search_matches_and_reports_errors() {
        let dir = temp_project("search-regex");
        let file = dir.join("src/main.rs");
        fs::write(&file, "let count = 42;\nlet total = 7;\n").unwrap();
        let mut app = App::new(Some(&file)).unwrap();

        app.execute_command(ids::FIND);
        app.handle_search_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT));
        assert!(app.search.regex);

        app.search.query = r"\d+".to_string();
        app.refresh_search_matches();
        assert_eq!(app.search.matches.len(), 2);
        assert!(app.search.regex_error.is_none());

        // Unsupported syntax is reported instead of matching silently wrong.
        app.search.query = "(a|b)".to_string();
        app.refresh_search_matches();
        assert!(app.search.matches.is_empty());
        assert!(app.search.regex_error.is_some());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn replace_all_applies_every_match_as_one_edit() {
        let dir = temp_project("replace-all");
        let file = dir.join("src/main.rs");
        fs::write(&file, "foo foo foo\nbar foo\n").unwrap();
        let mut app = App::new(Some(&file)).unwrap();

        app.execute_command(ids::REPLACE);
        app.search.query = "foo".to_string();
        app.search.replacement = "baz".to_string();
        app.refresh_search_matches();
        assert_eq!(app.search.matches.len(), 4);

        app.replace_all();
        assert_eq!(
            app.editor.active_document().unwrap().buffer.text(),
            "baz baz baz\nbar baz\n"
        );

        // A single undo restores the original.
        app.with_doc(|d| d.undo());
        assert_eq!(
            app.editor.active_document().unwrap().buffer.text(),
            "foo foo foo\nbar foo\n"
        );
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

    #[test]
    fn project_search_lists_matches_across_files() {
        let dir = temp_project("project-search");
        let a = dir.join("src/main.rs");
        let b = dir.join("src/lib.rs");
        fs::write(&a, "fn main() { needle(); }\n").unwrap();
        fs::write(&b, "pub fn needle() {}\n").unwrap();

        let mut app = App::new(Some(&a)).unwrap();
        app.execute_command(ids::PROJECT_SEARCH);
        assert!(matches!(app.overlay, Overlay::Prompt(_)));
        app.submit_prompt(PromptKind::ProjectSearch, "needle".to_string());

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            app.apply_background_events();
            if matches!(app.overlay, Overlay::Picker(_)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let Overlay::Picker(picker) = &app.overlay else {
            panic!("expected search results");
        };
        assert!(picker.filtered.len() >= 2, "expected both files to match");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn changed_files_lists_git_status() {
        let dir = temp_project("changed-files");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();
        let mut files = std::collections::HashMap::new();
        files.insert(file.clone(), GitFileStatus::Modified);
        app.workspace.git = crate::git::GitInfo {
            repo_root: Some(dir.clone()),
            branch: Some("main".to_string()),
            files,
            available: true,
        };

        app.execute_command(ids::CHANGED_FILES);
        let Overlay::Picker(picker) = &app.overlay else {
            panic!("expected the changed-files picker");
        };
        assert!(!picker.filtered.is_empty());
        let item = picker.item(0).expect("an item");
        assert!(matches!(item.action, PickerAction::OpenPath(_)));
        assert!(item.detail.contains('M'), "detail was {}", item.detail);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn commit_flow_reports_state_and_prompts() {
        let dir = temp_project("commit");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();

        // Not a repository: the command explains instead of prompting.
        app.workspace.git = crate::git::GitInfo::default();
        app.execute_command(ids::GIT_COMMIT);
        assert!(matches!(app.overlay, Overlay::None));
        assert!(
            app.status_message()
                .unwrap_or("")
                .contains("Not a git repository")
        );

        // With changes, it opens a commit-message prompt and dispatches work.
        let mut files = std::collections::HashMap::new();
        files.insert(file.clone(), GitFileStatus::Modified);
        app.workspace.git = crate::git::GitInfo {
            repo_root: Some(dir.clone()),
            branch: Some("main".to_string()),
            files,
            available: true,
        };
        app.execute_command(ids::GIT_COMMIT);
        assert!(matches!(app.overlay, Overlay::Prompt(_)));

        app.submit_prompt(PromptKind::CommitMessage, "a message".to_string());
        assert!(app.pending_commit);
        assert_eq!(app.busy(), Some("committing"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ctrl_shift_s_opens_save_as() {
        let dir = temp_project("save-as");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();

        app.handle_key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        assert!(matches!(
            &app.overlay,
            Overlay::Prompt(prompt) if prompt.kind == PromptKind::SaveAs
        ));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ctrl_n_opens_the_new_file_prompt() {
        let dir = temp_project("ctrl-n");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert!(matches!(
            &app.overlay,
            Overlay::Prompt(prompt) if prompt.kind == PromptKind::NewFile
        ));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn alt_m_jumps_to_the_matching_bracket() {
        let dir = temp_project("alt-m");
        let file = dir.join("src/main.rs");
        fs::write(&file, "fn main() {}\n").unwrap();
        let mut app = App::new(Some(&file)).unwrap();
        app.editor
            .active_document_mut()
            .unwrap()
            .move_to(Position::new(0, 7));

        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::ALT));
        assert_eq!(
            app.editor.active_document().unwrap().clamped_cursor(),
            Position::new(0, 8)
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn f3_advances_to_the_next_match() {
        let dir = temp_project("f3");
        let file = dir.join("src/main.rs");
        fs::write(&file, "foo\nfoo\nfoo\n").unwrap();
        let mut app = App::new(Some(&file)).unwrap();
        app.execute_command(ids::FIND);
        app.search.query = "foo".to_string();
        app.refresh_search_matches();
        app.jump_to_match(0);

        app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE));
        assert_eq!(app.search.current, Some(1));
        app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::SHIFT));
        assert_eq!(app.search.current, Some(0));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn busy_reports_pending_background_work() {
        let dir = temp_project("busy");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();
        assert!(app.busy().is_none());

        app.pending_format = Some((file.clone(), 1));
        assert_eq!(app.busy(), Some("formatting"));

        app.pending_format = None;
        app.pending_workspace_symbols = Some(1);
        assert_eq!(app.busy(), Some("searching symbols"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn language_setup_lists_discovered_tools() {
        let dir = temp_project("setup");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();

        // Tool discovery happens on the worker; wait briefly for it.
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.tools.is_none() && Instant::now() < deadline {
            app.apply_background_events();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.tools.is_some(), "tool discovery should complete");

        app.execute_command(ids::SETUP);
        let Overlay::Picker(picker) = &app.overlay else {
            panic!("expected the language setup list");
        };
        assert!(picker.filtered.len() >= crate::language::tools::Tool::ALL.len());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn installing_a_tool_reports_progress() {
        let dir = temp_project("install");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();

        app.install_tool(Tool::Gofmt);
        assert_eq!(app.busy(), Some("installing"));
        assert!(app.pending_install.is_some());

        // Simulate the worker's result.
        app.apply_background_event(BackgroundEvent::ToolInstalled {
            tool: Tool::Gofmt,
            result: Err("gofmt: it ships with the Go toolchain".to_string()),
        });
        assert!(app.pending_install.is_none());
        assert!(app.status.error);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn language_setup_offers_install_for_missing_tools() {
        let dir = temp_project("setup-install");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        while app.tools.is_none() && Instant::now() < deadline {
            app.apply_background_events();
            std::thread::sleep(Duration::from_millis(5));
        }
        // Only meaningful when rust-analyzer is actually missing.
        if !app
            .tools
            .as_ref()
            .is_some_and(|tools| !tools.available(Tool::RustAnalyzer))
        {
            fs::remove_dir_all(&dir).ok();
            return;
        }

        app.execute_command(ids::SETUP);
        let Overlay::Picker(picker) = &app.overlay else {
            panic!("expected the language setup list");
        };
        let item = (0..picker.filtered.len())
            .filter_map(|index| picker.item(index))
            .find(|item| item.label.contains("rust-analyzer"))
            .expect("rust-analyzer entry");
        assert!(
            item.enabled,
            "a missing installable tool should be actionable"
        );
        assert!(matches!(
            item.action,
            PickerAction::InstallTool(Tool::RustAnalyzer)
        ));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn offers_to_install_a_missing_language_server_once() {
        let dir = temp_project("offer-install");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        while app.tools.is_none() && Instant::now() < deadline {
            app.apply_background_events();
            std::thread::sleep(Duration::from_millis(5));
        }

        let missing = app
            .tools
            .as_ref()
            .is_some_and(|tools| !tools.available(Tool::RustAnalyzer));
        let offered = app.maybe_offer_tool_setup();
        assert_eq!(offered, missing, "the offer must track tool availability");
        if offered {
            let Overlay::Picker(picker) = &app.overlay else {
                panic!("expected the install prompt");
            };
            assert!(matches!(
                picker.item(0).map(|item| &item.action),
                Some(PickerAction::InstallTool(Tool::RustAnalyzer))
            ));
            // Dismissing must not re-offer in the same session.
            app.overlay = Overlay::None;
            assert!(!app.maybe_offer_tool_setup());
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lsp_handshake_timeout_falls_back_to_builtin() {
        let dir = temp_project("lsp-timeout");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();
        app.lsp_status = LspStatus::Starting;
        app.lsp_language = Some(LanguageId::Rust);
        app.lsp_started_at = Some(Instant::now() - Duration::from_secs(60));

        assert!(app.poll_lsp_health());
        assert!(matches!(app.lsp_status, LspStatus::Failed(_)));
        assert!(app.lsp.is_none());
        assert!(app.lsp_started_at.is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn automatic_restart_stops_after_the_budget() {
        let dir = temp_project("restart-budget");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();
        app.lsp_start_at = None;
        app.lsp_restarts = MAX_LSP_RESTARTS;

        app.schedule_lsp_restart(LanguageId::Rust);
        assert!(app.lsp_start_at.is_none(), "the budget is exhausted");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restart_server_reports_for_unsupported_files() {
        let dir = temp_project("restart-server");
        let file = dir.join("notes.txt");
        fs::write(&file, "hello\n").unwrap();
        let mut app = App::new(Some(&file)).unwrap();

        app.execute_command(ids::RESTART_SERVER);
        assert!(app.lsp.is_none());
        assert!(app.lsp_start_at.is_none());
        assert!(
            app.status_message()
                .unwrap_or("")
                .contains("No language server"),
            "status was {:?}",
            app.status_message()
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn language_server_diagnostics_reach_the_document() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        // Mock server: answer `initialize` (id 1) and publish one diagnostic
        // for the file passed as the first argument.
        const MOCK: &str = r#"#!/bin/sh
target="$1"
read -r header
len=$(printf '%s' "$header" | tr -dc '0-9')
read -r blank
dd bs=1 count="$len" of=/dev/null 2>/dev/null
resp='{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}'
printf 'Content-Length: %s\r\n\r\n%s' "${#resp}" "$resp"
note=$(printf '{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"file://%s","diagnostics":[{"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":3}},"severity":1,"message":"boom"}]}}' "$target")
printf 'Content-Length: %s\r\n\r\n%s' "${#note}" "$note"
cat >/dev/null
"#;

        let dir = temp_project("lsp-wire");
        let file = dir.join("src/main.rs");
        let script = dir.join("mock-lsp.sh");
        {
            let mut handle = fs::File::create(&script).unwrap();
            handle.write_all(MOCK.as_bytes()).unwrap();
        }
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();

        let mut app = App::new(Some(&file)).unwrap();
        app.start_lsp(
            LanguageId::Rust,
            script.to_str().unwrap(),
            &[file.to_str().unwrap()],
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut applied = false;
        while Instant::now() < deadline {
            app.poll_lsp();
            applied = app
                .editor
                .active_document()
                .is_some_and(|doc| doc.diagnostics_from_lsp() && !doc.diagnostics().is_empty());
            if applied {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(applied, "expected language-server diagnostics");
        assert_eq!(app.lsp_status, LspStatus::Ready);
        assert_eq!(
            app.editor.active_document().unwrap().diagnostics()[0].message,
            "boom"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn language_server_answers_hover_completion_and_definition() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        // A mock server that answers each feature request by method.
        const MOCK: &str = r#"#!/bin/sh
target="$1"
while read -r header; do
  header=$(printf '%s' "$header" | tr -d '\r')
  case "$header" in
    Content-Length:*) len=${header#Content-Length: } ;;
    *) continue ;;
  esac
  read -r blank
  body=$(dd bs=1 count="$len" 2>/dev/null)
  id=$(printf '%s' "$body" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  method=$(printf '%s' "$body" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p')
  [ -z "$id" ] && continue
  case "$method" in
    initialize) result='{"capabilities":{}}' ;;
    textDocument/hover) result='{"contents":{"kind":"plaintext","value":"LSP HOVER TEXT"}}' ;;
    textDocument/completion) result='{"items":[{"label":"koda_lsp_item","kind":3}]}' ;;
    textDocument/definition) result="[{\"uri\":\"file://$target\",\"range\":{\"start\":{\"line\":2,\"character\":0},\"end\":{\"line\":2,\"character\":5}}}]" ;;
    textDocument/references) result="[{\"uri\":\"file://$target\",\"range\":{\"start\":{\"line\":1,\"character\":4},\"end\":{\"line\":1,\"character\":9}}}]" ;;
    textDocument/rename) result="{\"changes\":{\"file://$target\":[{\"range\":{\"start\":{\"line\":1,\"character\":8},\"end\":{\"line\":1,\"character\":13}},\"newText\":\"renamed\"}]}}" ;;
    textDocument/codeAction) result="[{\"title\":\"Apply fix\",\"edit\":{\"changes\":{\"file://$target\":[{\"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":0}},\"newText\":\"FIX \"}]}}}]" ;;
    workspace/symbol) result="[{\"name\":\"koda_ws_symbol\",\"kind\":12,\"location\":{\"uri\":\"file://$target\",\"range\":{\"start\":{\"line\":3,\"character\":0},\"end\":{\"line\":3,\"character\":3}}}}]" ;;
    *) result='null' ;;
  esac
  resp=$(printf '{"jsonrpc":"2.0","id":%s,"result":%s}' "$id" "$result")
  printf 'Content-Length: %s\r\n\r\n%s' "${#resp}" "$resp"
done
"#;

        let dir = temp_project("lsp-features");
        let file = dir.join("src/main.rs");
        fs::write(&file, "fn main() {\n    let value = 1;\n    value;\n}\n").unwrap();
        let script = dir.join("mock-lsp.sh");
        {
            let mut handle = fs::File::create(&script).unwrap();
            handle.write_all(MOCK.as_bytes()).unwrap();
        }
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();

        let mut app = App::new(Some(&file)).unwrap();
        app.editor
            .active_document_mut()
            .unwrap()
            .set_language(LanguageId::Rust);
        app.start_lsp(
            LanguageId::Rust,
            script.to_str().unwrap(),
            &[file.to_str().unwrap()],
        );
        app.editor
            .active_document_mut()
            .unwrap()
            .move_to(Position::new(2, 4));

        let wait = |app: &mut App, predicate: &dyn Fn(&App) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                app.poll_lsp();
                if predicate(app) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            false
        };

        assert!(
            wait(&mut app, &|app| app
                .lsp
                .as_ref()
                .is_some_and(|server| server.is_ready())),
            "server should become ready"
        );

        // Hover.
        app.open_hover();
        assert!(
            wait(&mut app, &|app| app
                .hover
                .as_ref()
                .is_some_and(|hover| hover.title.contains("LSP HOVER"))),
            "expected the server's hover"
        );

        // Completion.
        app.hover = None;
        app.open_completion();
        assert!(
            wait(&mut app, &|app| app.completion.as_ref().is_some_and(
                |state| state.items.iter().any(|item| item.label == "koda_lsp_item")
            )),
            "expected the server's completion"
        );

        // Definition (same file, line 2 column 0).
        app.completion = None;
        app.goto_definition();
        assert!(
            wait(&mut app, &|app| app.editor.active_document().is_some_and(
                |doc| doc.clamped_cursor() == Position::new(2, 0)
            )),
            "expected the server's definition to move the cursor"
        );

        // References (the server returns one location).
        app.find_references();
        assert!(
            wait(&mut app, &|app| matches!(app.overlay, Overlay::Picker(_))),
            "expected the references picker"
        );

        // Rename (the server returns a workspace edit).
        app.editor
            .active_document_mut()
            .unwrap()
            .move_to(Position::new(1, 8));
        app.rename_symbol();
        assert!(
            matches!(app.overlay, Overlay::Prompt(_)),
            "rename should prompt for a new name"
        );
        app.submit_prompt(PromptKind::Rename, "renamed".to_string());
        assert!(
            wait(&mut app, &|app| app.editor.active_document().is_some_and(
                |doc| doc.buffer.line_text(1).contains("renamed")
            )),
            "expected the rename edit to apply"
        );
        assert_eq!(
            app.editor.active_document().unwrap().buffer.line_text(1),
            "    let renamed = 1;"
        );

        // Code actions (the server returns one edit-based action).
        app.code_actions();
        assert!(
            wait(&mut app, &|app| matches!(app.overlay, Overlay::Picker(_))),
            "expected the code actions picker"
        );
        app.apply_code_action(0);
        assert!(
            wait(&mut app, &|app| app.editor.active_document().is_some_and(
                |doc| doc.buffer.line_text(0).starts_with("FIX ")
            )),
            "expected the code action edit to apply"
        );

        // Workspace symbols: the server's results are merged into the picker.
        app.open_workspace_symbols();
        assert!(
            wait(&mut app, &|app| match &app.overlay {
                Overlay::Picker(picker) => (0..picker.filtered.len())
                    .filter_map(|index| picker.item(index))
                    .any(|item| item.label == "koda_ws_symbol"),
                _ => false,
            }),
            "expected the server's workspace symbol"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn captures_and_restores_open_files() {
        let dir = temp_project("session-restore");
        let a = dir.join("src/main.rs");
        let b = dir.join("src/lib.rs");
        fs::write(&b, "pub fn lib() {\n    let x = 1;\n}\n").unwrap();

        let mut app = App::new(Some(&a)).unwrap();
        app.open_path(b.clone());
        app.editor
            .active_document_mut()
            .unwrap()
            .move_to(Position::new(1, 4));

        let session = app.capture_session();
        assert_eq!(session.files.len(), 2);
        assert_eq!(session.active, 1);

        let mut fresh = App::new(Some(&dir)).unwrap();
        fresh.restore_session(session);
        assert_eq!(fresh.editor.len(), 2);
        assert_eq!(fresh.editor.active_index(), 1);
        assert_eq!(
            fresh.editor.active_document().unwrap().clamped_cursor(),
            Position::new(1, 4)
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reloads_clean_files_changed_on_disk() {
        let dir = temp_project("external-clean");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();

        std::thread::sleep(Duration::from_millis(10));
        fs::write(&file, "fn changed() {}\n").unwrap();
        app.last_disk_check = Instant::now() - Duration::from_secs(5);

        assert!(app.poll_external_changes());
        assert_eq!(
            app.editor.active_document().unwrap().buffer.text(),
            "fn changed() {}\n"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detects_markup_and_config_languages() {
        let dir = temp_project("config-langs");
        let cases = [
            ("notes.md", "# Title\n", LanguageId::Markdown),
            ("data.json", "{\"a\": 1}\n", LanguageId::Json),
            ("config.yaml", "a: true\n", LanguageId::Yaml),
        ];
        for (name, content, expected) in cases {
            let file = dir.join(name);
            fs::write(&file, content).unwrap();
            let app = App::new(Some(&file)).unwrap();
            let language = app.editor.active_document().unwrap().buffer.language;
            assert_eq!(language, expected, "detected {name} as {language:?}");
        }
        // `Cargo.toml` is a TOML file, even though it marks a Rust project.
        let manifest = dir.join("Cargo.toml");
        let app = App::new(Some(&manifest)).unwrap();
        assert_eq!(
            app.editor.active_document().unwrap().buffer.language,
            LanguageId::Toml
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detects_python_from_extension_and_shebang() {
        let dir = temp_project("python");
        let file = dir.join("app.py");
        fs::write(&file, "def main():\n    pass\n").unwrap();
        let app = App::new(Some(&file)).unwrap();
        assert_eq!(
            app.editor.active_document().unwrap().buffer.language,
            LanguageId::Python
        );

        // An extensionless script is recognised from its shebang.
        let script = dir.join("tool");
        fs::write(&script, "#!/usr/bin/env python3\nprint('hi')\n").unwrap();
        let app = App::new(Some(&script)).unwrap();
        assert_eq!(
            app.editor.active_document().unwrap().buffer.language,
            LanguageId::Python
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn warns_when_a_dirty_file_changes_on_disk() {
        let dir = temp_project("external-dirty");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();
        app.editor
            .active_document_mut()
            .unwrap()
            .insert_text("// mine\n");

        std::thread::sleep(Duration::from_millis(10));
        fs::write(&file, "fn theirs() {}\n").unwrap();
        app.last_disk_check = Instant::now() - Duration::from_secs(5);

        assert!(app.poll_external_changes());
        assert!(app.status.error);
        assert!(
            app.status_message()
                .unwrap_or("")
                .contains("changed on disk")
        );
        // The edit is preserved rather than silently overwritten.
        assert!(
            app.editor
                .active_document()
                .unwrap()
                .buffer
                .text()
                .starts_with("// mine")
        );
        fs::remove_dir_all(&dir).ok();
    }
}
