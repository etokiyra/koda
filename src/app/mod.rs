//! Application state and the event loop.
//!
//! This module is the conductor: it wires workspace, editor, language service,
//! commands and UI together, and translates terminal events into actions.

pub mod overlay;

use std::cmp::Reverse;
use std::collections::HashMap;
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
use crate::language::provider::TokenKind;
use crate::language::symbols::is_ident_char as is_word_char;
use crate::language::tools::{Tool, ToolPurpose, ToolRegistry};
use crate::language::{Capability, LanguageId, LanguageService, WorkspaceSymbol};
use crate::project::Workspace;
use crate::project::create::{self, CreateOutcome};
use crate::recent::Recent;
use crate::search::SearchMatch;
use crate::session::{self, Session};
use crate::terminal;
use crate::ui;
use overlay::{
    CompletionState, DiffState, DirPicker, Help, HoverState, NewProject, NewProjectStep, Overlay,
    Picker, PickerAction, PickerItem, Prompt, PromptKind, Search, SearchField, TreeFilter,
};

/// How long typing must pause before diagnostics are recomputed. Short enough to
/// feel immediate, long enough not to reanalyse on every keystroke.
const DIAGNOSTICS_DEBOUNCE: Duration = Duration::from_millis(150);

/// How long typing must pause before Koda offers completions automatically.
/// Short enough to feel instant, long enough that a burst of typing does not
/// build a candidate pool for every intermediate prefix.
const AUTOCOMPLETE_DELAY: Duration = Duration::from_millis(120);

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

/// One language's language-server job.
///
/// Koda keeps a server per language rather than one per session, so a workspace
/// that mixes languages — or a split pane showing two — gets tooling for each
/// instead of only the first language it happened to detect.
#[derive(Default)]
struct LspJob {
    /// The running server, once a scheduled start has fired.
    server: Option<Server>,
    /// A scheduled start that has not fired yet.
    start_at: Option<Instant>,
    /// When the current handshake started, for the timeout.
    started_at: Option<Instant>,
    /// Automatic restarts attempted since the server last connected.
    restarts: u32,
    /// The most recent failure, for the status summary.
    failed: Option<String>,
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

/// A scheduled automatic completion, pinned to the document state it was
/// requested for so a stray timer cannot pop a popup after the user moved on.
struct PendingCompletion {
    due_at: Instant,
    doc: usize,
    cursor: Position,
    version: u64,
}

/// The tone of a notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

/// A short-lived notification shown above the statusline.
pub struct Toast {
    pub message: String,
    pub kind: ToastKind,
    created: Instant,
}

/// An action offered on the welcome screen.
pub enum WelcomeAction {
    OpenFile,
    OpenProject,
    NewProject,
    Resume,
    OpenTarget(PathBuf),
    OpenRecentProject(PathBuf),
    OpenRecentFile(PathBuf),
    Help,
}

/// A row on the welcome screen.
pub struct WelcomeItem {
    pub label: String,
    pub detail: String,
    pub action: WelcomeAction,
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
    /// Global recent projects and files, shown on the welcome screen.
    pub recent: Recent,
    /// The path supplied on the command line, offered from the welcome screen.
    welcome_target: Option<PathBuf>,
    /// A session loaded for the current workspace, offered as "Resume".
    resume_session: Option<Session>,
    /// Selected row on the welcome screen.
    pub welcome_selected: usize,
    /// Whether the user engaged with a project this session. Gates session
    /// saving so starting Koda and quitting does not wipe an untouched session.
    engaged: bool,
    /// Whether a project creation is running on the worker.
    pending_project: bool,
    /// Inline file-tree filter, when active.
    pub tree_filter: Option<TreeFilter>,
    /// Completion popup, when open.
    pub completion: Option<CompletionState>,
    /// When a paused keystroke should offer automatic completion.
    completion_due: Option<PendingCompletion>,
    /// The id of the newest language-server completion request (with its
    /// language), so an older response can be discarded when the user has typed
    /// on.
    completion_request: Option<(LanguageId, i64)>,
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
    /// Transient notifications shown above the statusline.
    pub toasts: Vec<Toast>,
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
    /// A query to apply when the next workspace-symbol picker opens.
    pending_workspace_symbols_query: Option<String>,
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
    /// Whether the welcome scene and busy indicators animate. Users can turn
    /// motion off from the palette.
    pub motion: bool,
    /// The active welcome scene.
    pub welcome_scene: crate::ui::art::WelcomeScene,
    /// External tools Koda has probed for, once discovery completes.
    pub tools: Option<ToolRegistry>,
    /// One language server per language.
    lsp: HashMap<LanguageId, LspJob>,
    /// The symbol awaiting a new name, from the rename prompt.
    pending_rename: Option<(PathBuf, usize, usize)>,
    /// The path awaiting a new name, from the file-rename prompt.
    pending_rename_file: Option<PathBuf>,
    /// The path awaiting a destination, from the file-copy prompt.
    pending_copy_file: Option<PathBuf>,
    /// A tool install in progress, if any.
    pending_install: Option<Tool>,
    /// Whether a git commit is running on the worker.
    pending_commit: bool,
    /// Whether a stage/unstage is running and the changed-files picker should
    /// be rebuilt when the refreshed snapshot arrives.
    pending_stage_refresh: bool,
    /// Code actions from the most recent server response.
    pending_code_actions: Vec<convert::CodeAction>,
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
        app.recent.save();
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
            recent: Recent::default(),
            welcome_target: None,
            resume_session: None,
            welcome_selected: 0,
            engaged: false,
            pending_project: false,
            tree_filter: None,
            completion: None,
            completion_due: None,
            completion_request: None,
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
            toasts: Vec::new(),
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
            pending_workspace_symbols_query: None,
            ws_lsp_pending: false,
            project_search_seq: 0,
            pending_project_search: None,
            anim_phase: 0,
            anim_last: Instant::now(),
            motion: true,
            welcome_scene: crate::ui::art::WelcomeScene::default(),
            tools: None,
            lsp: HashMap::new(),
            pending_rename: None,
            pending_rename_file: None,
            pending_copy_file: None,
            pending_install: None,
            pending_commit: false,
            pending_stage_refresh: false,
            pending_code_actions: Vec::new(),
            last_disk_check: Instant::now(),
            setup_offered: std::collections::HashSet::new(),
        };

        // The welcome screen is always the first view. A path from the command
        // line is preserved as an obvious action rather than opened
        // immediately, and a saved session for the workspace is offered as a
        // "Resume" action instead of being restored silently.
        app.recent = Recent::load();
        app.welcome_target =
            target.map(|path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
        app.resume_session =
            session::session_path(app.workspace.root()).and_then(|path| Session::load_from(&path));

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
            let completion_changed = self.poll_auto_completion();
            let background_changed = self.apply_background_events();
            let animated = self.tick_animation();
            let external_changed = self.poll_external_changes();
            let offered = self.maybe_offer_tool_setup();
            if needs_redraw
                || background_changed
                || lsp_changed
                || completion_changed
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
            // Wake early when an automatic completion is due, so the popup is
            // not delayed by the idle timeout.
            let timeout = match self.completion_due.as_ref() {
                Some(pending) => {
                    timeout.min(pending.due_at.saturating_duration_since(Instant::now()))
                }
                None => timeout,
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
        self.motion && (self.editor.is_empty() || self.busy().is_some())
    }

    fn animation_interval(&self) -> Duration {
        if self.busy().is_some() {
            // Busy work is short-lived, so spin smoothly while it lasts.
            Duration::from_millis(90)
        } else {
            // The welcome scene drifts gently.
            Duration::from_millis(450)
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
        } else if self.pending_project {
            Some("creating project")
        } else if self.lsp_status() == LspStatus::Starting {
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
            Focus::Editor if self.editor.is_empty() => self.handle_welcome_key(key),
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
                        'd' => self.with_doc(|d| d.select_next_occurrence()),
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
                    if is_word_char(c) {
                        self.after_word_char_typed();
                    }
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
                self.completion_due = None;
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
                self.after_completion_edit();
                true
            }
            KeyCode::Char(c) if !ctrl => {
                self.with_doc(|doc| doc.type_char(c));
                if is_word_char(c) {
                    self.after_word_char_typed();
                } else {
                    // Whitespace or punctuation ends the word: dismiss the popup
                    // so it does not linger with an empty prefix.
                    self.completion = None;
                    self.completion_due = None;
                }
                true
            }
            _ => {
                self.completion = None;
                self.completion_due = None;
                false
            }
        }
    }

    /// After an edit that may shorten the typed prefix, re-filter an open popup
    /// and schedule the next automatic offer. Does nothing when the popup is
    /// closed, since Backspace should not by itself pop one open.
    fn after_completion_edit(&mut self) {
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

    /// After typing a word character, re-filter an open popup and schedule an
    /// automatic offer.
    fn after_word_char_typed(&mut self) {
        if self.completion.is_some() {
            self.refresh_completion();
        }
        self.schedule_auto_completion();
    }

    /// Schedule an automatic completion a short pause after the last keystroke.
    ///
    /// Suppressed inside comments and strings, and pinned to the current
    /// document, cursor and buffer version so the offer is dropped if the user
    /// moves or edits before it fires.
    fn schedule_auto_completion(&mut self) {
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

    /// Open the popup once the typing pause has elapsed.
    fn poll_auto_completion(&mut self) -> bool {
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

    /// Open completion for the word being typed.
    fn open_completion(&mut self) {
        self.open_completion_inner(true);
    }

    /// Build and show the completion popup.
    ///
    /// `manual` distinguishes an explicit `Ctrl+Space` (which reports when there
    /// is nothing to offer) from the automatic, typing-driven offer, which stays
    /// silent rather than interrupting with an empty popup.
    fn open_completion_inner(&mut self, manual: bool) -> bool {
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

    /// Ask the language server for completions at the cursor.
    fn request_lsp_completion(&mut self) {
        let Some(language) = self.lsp_language() else {
            return;
        };
        let Some((path, row, col)) = self.lsp_target_for(RequestKind::Completion) else {
            return;
        };
        if let Some(server) = self
            .lsp
            .get_mut(&language)
            .and_then(|job| job.server.as_mut())
        {
            self.completion_request = server.completion(&path, row, col).map(|id| (language, id));
        }
    }

    /// Whether the cursor sits just after a member-access operator (`.` or `::`).
    fn cursor_in_member_access(&self) -> bool {
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

    /// Whether the cursor is inside a comment or a string, according to the
    /// provider's highlighting. Used to keep automatic completion quiet in prose
    /// and literals.
    ///
    /// The probe is the start of the identifier being typed (or the character
    /// before the cursor when there is none), because the cursor sits just past
    /// the last typed character and a highlight span's end is exclusive.
    fn cursor_in_comment_or_string(&mut self) -> bool {
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
        if let Some((path, row, col)) = self.lsp_target_for(RequestKind::Hover) {
            if let Some(server) = self.active_server_mut() {
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
            ids::DUPLICATE_FILE => self.duplicate_file(),
            ids::COPY_FILE => self.copy_file(),
            ids::QUIT => self.request_quit(),
            ids::HOME => self.go_home(),
            ids::WELCOME_SCENE => {
                self.welcome_scene = self.welcome_scene.next();
                self.set_status(format!("Welcome scene: {}", self.welcome_scene.label()));
            }
            ids::TOGGLE_MOTION => {
                self.motion = !self.motion;
                let message = if self.motion {
                    "Animations on"
                } else {
                    "Animations off"
                };
                self.set_status(message);
            }
            ids::OPEN_PROJECT => self.open_dir_picker(),
            ids::NEW_PROJECT => self.open_new_project(),
            ids::UNDO => self.with_doc(|d| d.undo()),
            ids::REDO => self.with_doc(|d| d.redo()),
            ids::SELECT_ALL => self.with_doc(|d| d.select_all()),
            ids::SELECT_NEXT => self.with_doc(|d| d.select_next_occurrence()),
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
            ids::GIT_TOGGLE_STAGE => self.toggle_stage_target(),
            ids::DIFF => self.diff_active_file(),
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
            PickerAction::ShowDiff { path, staged } => self.show_diff(path, Some(staged)),
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
                    && let Some(server) = self.active_server_mut()
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
            PromptKind::CopyFile => {
                let Some(source) = self.pending_copy_file.take() else {
                    return;
                };
                if input.is_empty() {
                    self.set_error("No destination provided");
                    return;
                }
                let destination = if Path::new(&input).is_absolute() {
                    PathBuf::from(&input)
                } else {
                    self.workspace.root().join(&input)
                };
                match filesystem::copy_file(&source, &destination) {
                    Ok(()) => {
                        self.workspace.tree.refresh();
                        self.workspace.tree.select_path(&destination);
                        self.set_status(format!("Copied to {}", destination.display()));
                    }
                    Err(err) => self.set_error(format!("Copy failed: {err}")),
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
                self.engaged = true;
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
        self.recent_files.push(canonical.clone());
        if self.recent_files.len() > 20 {
            self.recent_files.remove(0);
        }
        if canonical.is_file() {
            self.recent.add_file(&canonical);
        }
    }

    // ----------------------------------------------------------------------
    // Welcome screen and project opening
    // ----------------------------------------------------------------------

    /// Whether the welcome screen is the active surface.
    pub fn welcome_active(&self) -> bool {
        self.editor.is_empty() && self.overlay.is_none() && !self.search.open
    }

    /// The actions offered on the welcome screen, best first.
    pub fn welcome_items(&self) -> Vec<WelcomeItem> {
        let mut items = vec![
            WelcomeItem {
                label: "Open a file…".to_string(),
                detail: "Ctrl+O".to_string(),
                action: WelcomeAction::OpenFile,
            },
            WelcomeItem {
                label: "Open a project…".to_string(),
                detail: "choose a folder".to_string(),
                action: WelcomeAction::OpenProject,
            },
            WelcomeItem {
                label: "Create a new project…".to_string(),
                detail: "Rust · Go · Python · C++".to_string(),
                action: WelcomeAction::NewProject,
            },
        ];

        if let Some(session) = &self.resume_session {
            let name = self
                .workspace
                .root()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("project");
            items.push(WelcomeItem {
                label: format!("Resume “{name}”"),
                detail: format!("{} file(s)", session.files.len()),
                action: WelcomeAction::Resume,
            });
        }

        if let Some(target) = &self.welcome_target {
            let is_workspace = target == self.workspace.root();
            if target.is_file() || !is_workspace {
                let verb = if target.is_file() {
                    "Open"
                } else {
                    "Open project"
                };
                items.push(WelcomeItem {
                    label: format!("{verb} {}", display_path(target)),
                    detail: "from the command line".to_string(),
                    action: WelcomeAction::OpenTarget(target.clone()),
                });
            }
        }

        for project in &self.recent.projects {
            if project == self.workspace.root() || !project.is_dir() {
                continue;
            }
            items.push(WelcomeItem {
                label: format!("Project {}", display_path(project)),
                detail: String::new(),
                action: WelcomeAction::OpenRecentProject(project.clone()),
            });
        }

        for file in &self.recent.files {
            if !file.is_file() {
                continue;
            }
            items.push(WelcomeItem {
                label: format!("File {}", display_path(file)),
                detail: String::new(),
                action: WelcomeAction::OpenRecentFile(file.clone()),
            });
        }

        items.push(WelcomeItem {
            label: "Keyboard shortcuts".to_string(),
            detail: "F1".to_string(),
            action: WelcomeAction::Help,
        });
        items
    }

    fn handle_welcome_key(&mut self, key: KeyEvent) {
        let count = self.welcome_items().len();
        match key.code {
            KeyCode::Up => {
                if self.welcome_selected > 0 {
                    self.welcome_selected -= 1;
                }
            }
            KeyCode::Down => {
                if self.welcome_selected + 1 < count {
                    self.welcome_selected += 1;
                }
            }
            KeyCode::Home => self.welcome_selected = 0,
            KeyCode::End => self.welcome_selected = count.saturating_sub(1),
            KeyCode::Char('v') | KeyCode::Char('V') => {
                self.welcome_scene = self.welcome_scene.next();
            }
            KeyCode::Enter => {
                if let Some(item) = self.welcome_items().into_iter().nth(self.welcome_selected) {
                    self.activate_welcome(item.action);
                }
            }
            _ => {}
        }
    }

    fn activate_welcome(&mut self, action: WelcomeAction) {
        match action {
            WelcomeAction::OpenFile => self.execute_command(ids::OPEN),
            WelcomeAction::OpenProject => self.open_dir_picker(),
            WelcomeAction::NewProject => self.open_new_project(),
            WelcomeAction::Resume => {
                if let Some(session) = self.resume_session.take() {
                    self.restore_session(session);
                }
                self.welcome_selected = 0;
            }
            WelcomeAction::OpenTarget(path) => self.open_target(path),
            WelcomeAction::OpenRecentProject(path) => self.open_project(path),
            WelcomeAction::OpenRecentFile(path) => self.open_recent_file(path),
            WelcomeAction::Help => self.toggle_help(),
        }
    }

    /// Open a path from the command line: a file directly, a directory as a
    /// project.
    fn open_target(&mut self, path: PathBuf) {
        if path.is_file() {
            self.open_recent_file(path);
        } else if path.is_dir() {
            self.open_project(path);
        } else {
            self.set_error(format!("{} no longer exists", path.display()));
        }
    }

    /// Open `path`, switching the workspace first if it belongs to another
    /// project.
    fn open_recent_file(&mut self, path: PathBuf) {
        if !path.is_file() {
            self.set_error(format!("{} no longer exists", path.display()));
            return;
        }
        let root = crate::project::Project::detect(&path).root;
        let root = root.canonicalize().unwrap_or(root);
        if root != self.workspace.root() && !self.open_workspace(root) {
            return;
        }
        self.open_path(path);
    }

    /// Open a project directory, restoring its session when one is available.
    fn open_project(&mut self, root: PathBuf) {
        if !self.open_workspace(root) {
            return;
        }
        if let Some(session) = self.resume_session.take() {
            self.restore_session(session);
        } else {
            let name = self
                .workspace
                .root()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("project")
                .to_string();
            self.set_status(format!("Opened project {name}"));
        }
    }

    /// Begin the "Open Project…" directory picker.
    fn open_dir_picker(&mut self) {
        self.overlay = Overlay::DirPicker(DirPicker::new(&self.welcome_start_dir()));
    }

    /// Begin the guided "Create a new project" flow.
    fn open_new_project(&mut self) {
        self.overlay = Overlay::NewProject(NewProject::new(&self.welcome_start_dir()));
    }

    /// Where a picker should start browsing.
    fn welcome_start_dir(&self) -> PathBuf {
        if let Some(directory) = self
            .editor
            .active_document()
            .and_then(|doc| doc.buffer.path.as_ref())
            .and_then(|path| path.parent())
            .filter(|parent| parent.is_dir())
        {
            return directory.to_path_buf();
        }
        std::env::current_dir().unwrap_or_else(|_| self.workspace.root().to_path_buf())
    }

    /// Replace the workspace with the project rooted at `root`.
    ///
    /// Returns `false` when the workspace could not be opened (an error status
    /// is set).
    pub fn open_workspace(&mut self, root: PathBuf) -> bool {
        // Persist the current project before leaving it.
        self.save_session();
        let workspace = match Workspace::open(Some(&root)) {
            Ok(workspace) => workspace,
            Err(err) => {
                self.set_error(format!("Could not open the project: {err}"));
                return false;
            }
        };
        self.workspace = workspace;
        self.reset_for_new_workspace();
        self.recent.add_project(self.workspace.root());
        self.engaged = true;
        self.resume_session =
            session::session_path(self.workspace.root()).and_then(|path| Session::load_from(&path));
        self.request_git_refresh();
        self.background.discover_tools();
        true
    }

    /// Clear per-project state when the workspace changes.
    fn reset_for_new_workspace(&mut self) {
        self.editor = Editor::new();
        self.split = false;
        self.pane_left = 0;
        self.pane_right = None;
        self.focus_pane = Pane::Primary;
        self.focus = Focus::Editor;
        self.overlay = Overlay::None;
        self.search = Search::default();
        self.tree_filter = None;
        self.completion = None;
        self.hover = None;
        self.cursor_screen = None;
        self.close_armed = None;
        self.diagnostics_dirty_at = None;
        self.pending_format = None;
        self.pending_workspace_symbols = None;
        self.pending_workspace_symbols_query = None;
        self.pending_project_search = None;
        self.pending_rename = None;
        self.pending_code_actions.clear();
        self.lsp.clear();
        self.welcome_target = None;
        self.welcome_selected = 0;
    }

    /// Send a create-project request to the background worker.
    fn start_project_creation(&mut self, parent: PathBuf, name: String, language: LanguageId) {
        if self.pending_project {
            self.set_status("A project is already being created");
            return;
        }
        self.pending_project = true;
        self.background
            .create_project(parent, name.clone(), language);
        self.set_status(format!("Creating {name}…"));
    }

    /// Return to the welcome screen by closing every tab.
    fn go_home(&mut self) {
        if self.editor.has_unsaved() {
            self.set_error("Unsaved changes — save first (Ctrl+S)");
            return;
        }
        self.editor.close_all();
        self.pane_left = 0;
        self.pane_right = None;
        self.split = false;
        self.focus_pane = Pane::Primary;
        self.welcome_selected = 0;
        self.focus = Focus::Editor;
        self.set_status("Welcome");
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
        self.engaged = true;
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
    ///
    /// Only writes once the user has engaged with a project, so starting Koda
    /// and quitting without opening anything does not overwrite an untouched
    /// session.
    pub fn save_session(&self) {
        if !self.engaged {
            return;
        }
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

    /// Duplicate the selected file with a `copy` suffix and open it.
    fn duplicate_file(&mut self) {
        let Some(source) = self.file_op_target() else {
            self.set_status("Select a file to duplicate");
            return;
        };
        if source.is_dir() {
            self.set_status("Folders cannot be duplicated yet");
            return;
        }
        let destination = unique_copy_path(&source);
        match filesystem::copy_file(&source, &destination) {
            Ok(()) => {
                self.workspace.tree.refresh();
                self.workspace.tree.select_path(&destination);
                self.open_path(destination);
            }
            Err(err) => self.set_error(format!("Duplicate failed: {err}")),
        }
    }

    /// Prompt for a destination and copy the selected file there.
    fn copy_file(&mut self) {
        let Some(source) = self.file_op_target() else {
            self.set_status("Select a file to copy");
            return;
        };
        if source.is_dir() {
            self.set_status("Folders cannot be copied yet");
            return;
        }
        let placeholder = source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("copy")
            .to_string();
        self.pending_copy_file = Some(source);
        self.open_prompt(PromptKind::CopyFile, "Copy file", &placeholder);
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
                if self.pending_stage_refresh {
                    self.pending_stage_refresh = false;
                    if matches!(&self.overlay, Overlay::Picker(picker) if picker.title == "Changed Files")
                    {
                        self.show_changed_files();
                    }
                }
                true
            }
            BackgroundEvent::GitStaged {
                path,
                staged,
                result,
            } => {
                match result {
                    Ok(()) => {
                        let action = if staged { "Staged" } else { "Unstaged" };
                        let name = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("file")
                            .to_string();
                        self.set_status(format!("{action} {name}"));
                    }
                    Err(message) => {
                        let text = format!("Git: {message}");
                        self.set_error(text.clone());
                        self.push_toast(ToastKind::Error, text);
                    }
                }
                true
            }
            BackgroundEvent::GitCommitted { result } => {
                self.pending_commit = false;
                match result {
                    Ok(message) => {
                        self.set_status(message.clone());
                        self.push_toast(ToastKind::Success, message);
                    }
                    Err(message) => {
                        let text = format!("Commit failed — {message}");
                        self.set_error(text.clone());
                        self.push_toast(ToastKind::Error, text);
                    }
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
                    Ok(message) => {
                        self.set_status(message.clone());
                        self.push_toast(ToastKind::Success, message);
                    }
                    Err(message) => {
                        let text = format!("Install failed — {message}");
                        self.set_error(text.clone());
                        self.push_toast(ToastKind::Error, text);
                    }
                }
                true
            }
            BackgroundEvent::ProjectCreated {
                name,
                language: _,
                outcome,
            } => {
                self.pending_project = false;
                match outcome {
                    CreateOutcome::Created { root, files } => {
                        self.push_toast(ToastKind::Success, format!("Created {name}"));
                        self.recent.add_project(&root);
                        if !self.open_workspace(root) {
                            return true;
                        }
                        if let Some(entry) = entry_file(&files) {
                            self.open_path(entry);
                        }
                    }
                    CreateOutcome::Failed {
                        root: Some(root),
                        message,
                    } => {
                        let text = format!(
                            "Could not finish creating {name}: {message}. A partial project is at {}",
                            root.display()
                        );
                        self.set_error(text.clone());
                        self.push_toast(ToastKind::Error, text);
                    }
                    CreateOutcome::Failed {
                        root: None,
                        message,
                    } => {
                        let text = format!("Could not create {name}: {message}");
                        self.set_error(text.clone());
                        self.push_toast(ToastKind::Error, text);
                    }
                }
                true
            }
        }
    }

    /// Display the unified diff for `path`.
    ///
    /// When `staged` is `None`, the working-tree diff is preferred and the
    /// staged diff is shown if the working tree is clean.
    fn show_diff(&mut self, path: PathBuf, staged: Option<bool>) {
        let root = self.workspace.root().to_path_buf();
        let try_show = |side: bool| crate::git::diff(&root, &path, side);
        let (text, side) = match staged {
            Some(side) => match try_show(side) {
                Ok(text) => (text, side),
                Err(err) => {
                    self.set_error(format!("Could not diff {}: {err}", path.display()));
                    return;
                }
            },
            None => match try_show(false) {
                Ok(text) if !text.is_empty() => (text, false),
                Ok(_) => match try_show(true) {
                    Ok(text) => (text, true),
                    Err(err) => {
                        self.set_error(format!("Could not diff {}: {err}", path.display()));
                        return;
                    }
                },
                Err(err) => {
                    self.set_error(format!("Could not diff {}: {err}", path.display()));
                    return;
                }
            },
        };

        if text.is_empty() {
            self.set_status(format!("No changes to show for {}", path.display()));
            return;
        }
        let name = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let where_ = if side { "staged" } else { "working tree" };
        let title = format!("{where_} · {name}");
        self.overlay = Overlay::Diff(DiffState::from_unified(title, &text));
    }

    /// Show the diff for the active file: working tree first, then staged.
    fn diff_active_file(&mut self) {
        if !self.workspace.git.available {
            self.set_status("Not a git repository");
            return;
        }
        let Some(path) = self
            .editor
            .active_document()
            .and_then(|doc| doc.buffer.path.clone())
        else {
            self.set_status("No file to diff");
            return;
        };
        self.show_diff(path, None);
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

    /// List the files changed in the working tree.
    fn open_changed_files(&mut self) {
        if !self.workspace.git.available {
            self.set_status("Not a git repository");
            return;
        }
        if self.workspace.git.files.is_empty() {
            self.set_status("Working tree clean");
            return;
        }
        self.show_changed_files();
    }

    /// Build and show the changed-files picker from the current snapshot.
    fn show_changed_files(&mut self) {
        let items = self.changed_files_items();
        let mut picker = Picker::new("Changed Files", "Filter files…  ·  Space stages", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Rows for the changed-files picker, sorted by path.
    fn changed_files_items(&self) -> Vec<PickerItem> {
        let root = self.workspace.root().to_path_buf();
        let mut entries: Vec<(PathBuf, GitFileStatus)> = self
            .workspace
            .git
            .files
            .iter()
            .map(|(path, status)| (path.clone(), *status))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
            .into_iter()
            .map(|(path, status)| {
                let relative = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                let stage = if self.workspace.git.is_staged(&path) {
                    "staged"
                } else {
                    "unstaged"
                };
                PickerItem::new(
                    relative,
                    format!("{} {} · {stage}", status.indicator(), status.label()),
                    PickerAction::OpenPath(path),
                )
            })
            .collect()
    }

    /// Stage or unstage the selected file (tree selection or active document).
    fn toggle_stage_target(&mut self) {
        if !self.workspace.git.available {
            self.set_status("Not a git repository");
            return;
        }
        let Some(path) = self.file_op_target() else {
            self.set_status("Select a file to stage");
            return;
        };
        let staged = !self.workspace.git.is_staged(&path);
        self.dispatch_stage(path, staged, true);
    }

    /// Send a stage/unstage request and show a short status hint.
    fn dispatch_stage(&mut self, path: PathBuf, staged: bool, refresh_picker: bool) {
        let root = self
            .workspace
            .git
            .repo_root
            .clone()
            .unwrap_or_else(|| self.workspace.root().to_path_buf());
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file")
            .to_string();
        self.pending_stage_refresh = refresh_picker;
        self.background.stage_path(root, path, staged);
        let action = if staged { "Staging" } else { "Unstaging" };
        self.set_status(format!("{action} {name}…"));
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
        self.open_workspace_symbols_with(None);
    }

    /// Ask for project-wide symbols, optionally with a pre-applied query.
    ///
    /// `F12` uses a query so a definition that is not in the current file can
    /// still be found across the project without a language server.
    fn open_workspace_symbols_with(&mut self, query: Option<String>) {
        self.pending_workspace_symbols_query = query.clone();
        if let Some(language) = self.ready_server_for(RequestKind::WorkspaceSymbols) {
            self.ws_lsp_pending = true;
            if let Some(server) = self
                .lsp
                .get_mut(&language)
                .and_then(|job| job.server.as_mut())
            {
                server.workspace_symbols(query.as_deref().unwrap_or(""));
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
        if let Some(query) = self.pending_workspace_symbols_query.take() {
            picker.query = query;
        }
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
                let message = format!("{tool} not found — {hint}");
                self.set_error(message.clone());
                self.push_toast(ToastKind::Error, message);
            }
            FormatOutcome::Failed(message) => {
                let text = format!("Format failed: {message}");
                self.set_error(text.clone());
                self.push_toast(ToastKind::Error, text);
            }
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
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("file")
                .to_string();
            self.push_toast(ToastKind::Success, format!("Formatted {name}"));
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

    // ----------------------------------------------------------------------
    // Language server
    // ----------------------------------------------------------------------

    /// Aggregate connection state across every language server, for the
    /// statusline.
    pub fn lsp_status(&self) -> LspStatus {
        let mut starting = false;
        let mut failed = None;
        for job in self.lsp.values() {
            if let Some(server) = &job.server {
                if server.is_ready() {
                    return LspStatus::Ready;
                }
                starting = true;
            }
            if let Some(message) = &job.failed {
                failed = Some(message.clone());
            }
        }
        if starting {
            LspStatus::Starting
        } else if let Some(message) = failed {
            LspStatus::Failed(message)
        } else {
            LspStatus::Offline
        }
    }

    /// Start a language server for `language` when one is installed, after a
    /// short delay so opening a file never waits on server startup.
    fn maybe_start_lsp(&mut self, language: LanguageId) {
        if self.lsp.contains_key(&language) {
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
        self.lsp.insert(
            language,
            LspJob {
                start_at: Some(Instant::now() + LSP_START_DELAY),
                ..LspJob::default()
            },
        );
    }

    /// Start any language server whose delay has elapsed.
    fn poll_lsp_start(&mut self) {
        let now = Instant::now();
        let due: Vec<LanguageId> = self
            .lsp
            .iter()
            .filter(|(_, job)| job.server.is_none() && job.start_at.is_some_and(|at| now >= at))
            .map(|(language, _)| *language)
            .collect();
        for language in due {
            if let Some(job) = self.lsp.get_mut(&language) {
                job.start_at = None;
            }
            let Some((program, args)) = self.lsp_launch(language) else {
                continue;
            };
            self.start_lsp(language, &program, args);
        }
    }

    /// The program and arguments for `language`'s server, when it is installed.
    fn lsp_launch(&self, language: LanguageId) -> Option<(String, &'static [&'static str])> {
        let tools = self.tools.as_ref()?;
        let tool = Tool::for_language(language, ToolPurpose::LanguageServer)?;
        if !tools.available(tool) {
            return None;
        }
        // Prefer the executable Koda discovered, which may live in a user bin
        // directory outside the process PATH.
        let program = tools
            .program_path(tool)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| tool.program().to_string());
        Some((program, tool.server_args()))
    }

    /// Start a language server, recording our attempt either way.
    fn start_lsp(&mut self, language: LanguageId, program: &str, args: &[&str]) {
        let root = self.workspace.root().to_path_buf();
        match Server::start(language, program, args, &root) {
            Ok(server) => {
                let job = self.lsp.entry(language).or_default();
                job.server = Some(server);
                job.start_at = None;
                job.started_at = Some(Instant::now());
                job.failed = None;
            }
            Err(err) => {
                let job = self.lsp.entry(language).or_default();
                job.server = None;
                job.start_at = None;
                job.started_at = None;
                job.failed = Some(err.to_string());
                self.set_error(format!("Could not start {program}: {err}"));
                self.schedule_lsp_restart(language);
            }
        }
    }

    /// Give up on any handshake that has taken too long, falling back cleanly.
    ///
    /// A server that never answers `initialize` would otherwise leave Koda
    /// "connecting" forever; this turns that hang into the same graceful
    /// fallback as a crash.
    fn poll_lsp_health(&mut self) -> bool {
        let now = Instant::now();
        let timed_out: Vec<LanguageId> = self
            .lsp
            .iter()
            .filter(|(_, job)| {
                job.server.as_ref().is_some_and(|server| !server.is_ready())
                    && job
                        .started_at
                        .is_some_and(|started| now.duration_since(started) >= LSP_HANDSHAKE_TIMEOUT)
            })
            .map(|(language, _)| *language)
            .collect();
        let mut changed = false;
        for language in timed_out {
            if let Some(job) = self.lsp.get_mut(&language) {
                job.server = None;
                job.started_at = None;
                job.failed = Some("initialize timed out".to_string());
            }
            for doc in &mut self.editor.documents {
                doc.use_builtin_diagnostics();
            }
            self.diagnostics_dirty_at = Some(Instant::now());
            let message = format!(
                "{} did not respond; using built-in intelligence",
                language.name()
            );
            self.set_error(message.clone());
            self.push_toast(ToastKind::Error, message);
            self.schedule_lsp_restart(language);
            changed = true;
        }
        changed
    }

    /// Schedule a bounded automatic restart for `language`, if a server tool is
    /// installed and the retry budget is not exhausted.
    fn schedule_lsp_restart(&mut self, language: LanguageId) {
        let label = {
            let Some(tools) = &self.tools else {
                return;
            };
            let Some(tool) = Tool::for_language(language, ToolPurpose::LanguageServer) else {
                return;
            };
            if !tools.available(tool) {
                return;
            }
            let Some(job) = self.lsp.get_mut(&language) else {
                return;
            };
            if job.restarts >= MAX_LSP_RESTARTS || job.start_at.is_some() {
                return;
            }
            job.restarts += 1;
            job.start_at = Some(Instant::now() + LSP_RESTART_DELAY);
            tool.label()
        };
        self.set_status(format!("Restarting {label}…"));
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

        // Drop this language's connection and schedule a fresh one now. The
        // restart budget resets because the user asked explicitly.
        self.lsp.remove(&language);
        for doc in &mut self.editor.documents {
            doc.use_builtin_diagnostics();
        }
        self.lsp.insert(
            language,
            LspJob {
                start_at: Some(Instant::now()),
                ..LspJob::default()
            },
        );
        self.set_status(format!("Restarting {}…", tool.label()));
    }

    /// Drain every language server's events. Returns `true` when something
    /// changed.
    fn poll_lsp(&mut self) -> bool {
        let languages: Vec<LanguageId> = self.lsp.keys().copied().collect();
        let mut changed = false;
        for language in languages {
            let events = match self
                .lsp
                .get_mut(&language)
                .and_then(|job| job.server.as_mut())
            {
                Some(server) => server.poll(),
                None => continue,
            };
            for event in events {
                changed = true;
                match event {
                    ServerEvent::Ready => self.lsp_ready(language),
                    ServerEvent::Diagnostics { path, diagnostics } => {
                        self.apply_lsp_diagnostics(&path, diagnostics);
                    }
                    ServerEvent::Response { kind, id, result } => {
                        self.handle_lsp_response(language, kind, id, result);
                    }
                    ServerEvent::ApplyEdit { id, params } => {
                        let edit = params.get("edit").cloned().unwrap_or(Value::Null);
                        let files = convert::workspace_edit(&edit);
                        let applied = self.apply_workspace_edit(files);
                        if let Some(server) = self
                            .lsp
                            .get_mut(&language)
                            .and_then(|job| job.server.as_mut())
                        {
                            server.apply_edit_response(&id, applied > 0);
                        }
                        if applied > 0 {
                            self.set_status(format!("Applied {applied} edit(s)"));
                        }
                    }
                    ServerEvent::Failed(message) => self.lsp_failed(language, message),
                }
            }
        }
        changed
    }

    /// A server exited or failed to start: fall back for its language.
    fn lsp_failed(&mut self, language: LanguageId, message: String) {
        if let Some(job) = self.lsp.get_mut(&language) {
            job.server = None;
            job.started_at = None;
            job.failed = Some(message.clone());
        }
        // Fall back to the built-in providers for every document.
        for doc in &mut self.editor.documents {
            doc.use_builtin_diagnostics();
        }
        self.diagnostics_dirty_at = Some(Instant::now());
        let text = format!("Language server stopped — {message}; using built-in intelligence");
        self.set_error(text.clone());
        self.push_toast(ToastKind::Error, text);
        self.schedule_lsp_restart(language);
    }

    /// The handshake finished: open every matching document on the server.
    fn lsp_ready(&mut self, language: LanguageId) {
        if let Some(job) = self.lsp.get_mut(&language) {
            // A healthy connection earns a fresh restart budget for later.
            job.restarts = 0;
            job.started_at = None;
            job.failed = None;
        }
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
        if let Some(server) = self
            .lsp
            .get_mut(&language)
            .and_then(|job| job.server.as_mut())
        {
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

    /// The language of the active document, when a ready server serves it.
    fn lsp_language(&self) -> Option<LanguageId> {
        let doc = self.editor.active_document()?;
        let language = doc.buffer.language;
        let server = self.lsp.get(&language)?.server.as_ref()?;
        server.is_ready().then_some(language)
    }

    /// A mutable handle to the ready server for the active document.
    fn active_server_mut(&mut self) -> Option<&mut Server> {
        let language = self.lsp_language()?;
        self.lsp.get_mut(&language)?.server.as_mut()
    }

    /// Whether the active document's ready server advertises `kind`.
    fn lsp_supports(&self, kind: RequestKind) -> bool {
        self.lsp_language()
            .and_then(|language| self.lsp.get(&language))
            .and_then(|job| job.server.as_ref())
            .is_some_and(|server| server.supports(kind))
    }

    /// The active document's target when a ready server that supports `kind`
    /// owns it. Falls back to `None` so built-in intelligence takes over.
    fn lsp_target_for(&self, kind: RequestKind) -> Option<(PathBuf, usize, usize)> {
        if !self.lsp_supports(kind) {
            return None;
        }
        self.lsp_target()
    }

    /// The active language's ready server that supports `kind`, or any ready
    /// server that does, for workspace-wide requests.
    fn ready_server_for(&self, kind: RequestKind) -> Option<LanguageId> {
        let supports = |language: &LanguageId| {
            self.lsp
                .get(language)
                .and_then(|job| job.server.as_ref())
                .is_some_and(|server| server.is_ready() && server.supports(kind))
        };
        let preferred = self.editor.active_document().map(|doc| doc.buffer.language);
        preferred
            .filter(supports)
            .or_else(|| self.lsp.keys().copied().find(supports))
    }

    /// The active document's `(path, line, col)` when a ready server owns it.
    fn lsp_target(&self) -> Option<(PathBuf, usize, usize)> {
        let language = self.lsp_language()?;
        let doc = self.editor.active_document()?;
        let server = self.lsp.get(&language)?.server.as_ref()?;
        let path = doc.buffer.path.clone()?;
        if !server.has_open_document(&path) {
            return None;
        }
        let cursor = doc.clamped_cursor();
        Some((path, cursor.row, cursor.col))
    }

    /// Apply a language-server feature response for `language`.
    fn handle_lsp_response(
        &mut self,
        language: LanguageId,
        kind: RequestKind,
        id: i64,
        result: Result<Value, String>,
    ) {
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
                // Ignore a response that a newer request has superseded: the
                // user typed on, so these candidates no longer match the cursor.
                if self.completion_request != Some((language, id)) {
                    return;
                }
                self.completion_request = None;
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
        let Some((path, row, col)) = self.lsp_target_for(RequestKind::CodeActions) else {
            self.set_status("Code actions need a language server (see Language Setup…)");
            return;
        };
        let range = self
            .editor
            .active_document()
            .and_then(|doc| doc.selection_range())
            .map(|(start, end)| ((start.row, start.col), (end.row, end.col)))
            .unwrap_or(((row, col), (row, col)));
        if let Some(server) = self.active_server_mut() {
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
            if let Some(server) = self.active_server_mut() {
                server.execute_command(&command.command, command.arguments);
            }
            self.set_status("Running action…");
        } else {
            self.set_status("This action does nothing");
        }
    }

    /// Prompt for a new name and ask the server to rename the symbol.
    fn rename_symbol(&mut self) {
        let Some((path, row, col)) = self.lsp_target_for(RequestKind::Rename) else {
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
        if let Some((path, row, col)) = self.lsp_target_for(RequestKind::Definition) {
            if let Some(server) = self.active_server_mut() {
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
            // Not defined in this file: search the project for a same-named
            // symbol, so F12 still works across files without a server.
            if let Some(word) = crate::language::symbols::word_at(&text, cursor.row, cursor.col) {
                self.open_workspace_symbols_with(Some(word));
                self.set_status("Searching the project…");
            } else {
                self.set_status("Nothing to look up here");
            }
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
        if let Some((path, row, col)) = self.lsp_target_for(RequestKind::References) {
            if let Some(server) = self.active_server_mut() {
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
    ///
    /// Exposed so embedders (and tests) can drive pending detection and git
    /// work deterministically before rendering.
    pub fn pump_background(&mut self, timeout: Duration) {
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
            ids::DUPLICATE_FILE | ids::COPY_FILE if self.file_op_target().is_none() => {
                (false, Some("select a file first".to_string()))
            }
            ids::GIT_COMMIT if !self.workspace.git.available => {
                (false, Some("not a git repository".to_string()))
            }
            ids::GIT_COMMIT if self.workspace.git.files.is_empty() => {
                (false, Some("nothing to commit".to_string()))
            }
            ids::GIT_TOGGLE_STAGE if !self.workspace.git.available => {
                (false, Some("not a git repository".to_string()))
            }
            ids::GIT_TOGGLE_STAGE if self.file_op_target().is_none() => {
                (false, Some("select a file first".to_string()))
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
            ids::RENAME if !self.lsp_supports(RequestKind::Rename) => {
                (false, Some("needs a language server".to_string()))
            }
            ids::CODE_ACTIONS if !self.lsp_supports(RequestKind::CodeActions) => {
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
        if !self.overlay.is_none() || self.pending_install.is_some() || !self.lsp.is_empty() {
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
        if tools.available(tool) || !crate::language::tools::can_install(tool) {
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
            } else if crate::language::tools::can_install(tool) {
                let hint = tool.install_hint();
                PickerItem::new(label, hint.to_string(), PickerAction::InstallTool(tool))
                    .shortcut("Enter")
            } else {
                // No package manager for this tool is present: say exactly what
                // is missing instead of only how to install the tool itself.
                let missing = tool.missing_prerequisites();
                let reason = if missing.is_empty() {
                    tool.install_hint().to_string()
                } else {
                    format!("needs {} — {}", missing.join(" or "), tool.install_hint())
                };
                PickerItem::new(label, reason.clone(), PickerAction::Info(reason.clone()))
                    .disabled(reason)
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

    /// Post a notification that lingers briefly above the statusline.
    ///
    /// Consecutive duplicates are ignored so a burst of work does not spam the
    /// screen, and only the most recent few are kept.
    pub fn push_toast(&mut self, kind: ToastKind, message: impl Into<String>) {
        const MAX: usize = 4;
        let message = message.into();
        if self
            .toasts
            .last()
            .is_some_and(|toast| toast.message == message && toast.kind == kind)
        {
            return;
        }
        self.toasts.push(Toast {
            message,
            kind,
            created: Instant::now(),
        });
        if self.toasts.len() > MAX {
            self.toasts.remove(0);
        }
    }

    /// Expire stale status messages and notifications.
    ///
    /// Returns `true` when something was cleared, so the caller knows to redraw.
    fn tick_status(&mut self) -> bool {
        const TOAST_TTL: Duration = Duration::from_secs(5);
        let mut changed = false;

        let before = self.toasts.len();
        self.toasts
            .retain(|toast| toast.created.elapsed() < TOAST_TTL);
        if self.toasts.len() != before {
            changed = true;
        }

        if let Some(expires_at) = self.status.expires_at
            && Instant::now() >= expires_at
        {
            self.status.message.clear();
            self.status.error = false;
            self.status.expires_at = None;
            changed = true;
        }
        changed
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

/// A sibling copy path that does not yet exist: `name copy.ext`,
/// `name copy 2.ext`, and so on.
fn unique_copy_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let extension = path.extension().and_then(|e| e.to_str());
    for index in 1..1000 {
        let base = if index == 1 {
            format!("{stem} copy")
        } else {
            format!("{stem} copy {index}")
        };
        let name = match extension {
            Some(ext) => format!("{base}.{ext}"),
            None => base,
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem} copy"))
}

/// A compact display path, using `~` for the home directory when possible.
fn display_path(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && let Ok(relative) = path.strip_prefix(&home)
    {
        return format!("~/{}", relative.display());
    }
    path.display().to_string()
}

/// Validate a project name against the chosen parent directory.
fn validate_project_name(parent: &Path, name: &str) -> Result<(), String> {
    create::validate_name(name)?;
    let target = parent.join(name.trim());
    if target.exists() {
        return Err(format!("{} already exists", target.display()));
    }
    Ok(())
}

/// Pick a sensible file to open in a freshly created project.
fn entry_file(files: &[PathBuf]) -> Option<PathBuf> {
    const PREFERRED: &[&str] = &["main.rs", "main.go", "__main__.py", "main.py"];
    files
        .iter()
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| PREFERRED.contains(&name))
        })
        .or_else(|| {
            files.iter().find(|path| {
                matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("rs" | "go" | "py" | "sh" | "bash")
                )
            })
        })
        .cloned()
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

    /// Build an app and open `file`, settling detection, as tests expect a
    /// document to be active. Koda itself launches into the welcome screen.
    ///
    /// Detection runs on the background worker behind any queued git/tool
    /// probes, so wait for the result rather than assuming a fixed delay. Only
    /// extensions a provider recognises are waited on; an unknown extension is
    /// expected to stay [`LanguageId::Unknown`].
    fn app_with_file(file: &Path) -> App {
        let mut app = App::new(Some(file)).unwrap();
        app.open_path(file.to_path_buf());
        let expected_known = std::fs::read(file)
            .map(|bytes| bytes.starts_with(b"#!"))
            .unwrap_or(false)
            || file
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| app.language.language_for_extension(extension).is_some());
        if expected_known {
            let deadline = Instant::now() + Duration::from_secs(5);
            while app
                .editor
                .active_document()
                .is_some_and(|doc| doc.buffer.language == LanguageId::Unknown)
                && Instant::now() < deadline
            {
                app.pump_background(Duration::from_millis(20));
            }
        } else {
            app.pump_background(Duration::from_millis(300));
        }
        app
    }

    #[test]
    fn opens_rust_file_and_detects_language() {
        let dir = temp_project("open");
        let file = dir.join("src/main.rs");
        let app = app_with_file(&file);
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
        let mut app = app_with_file(&file);
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
        let mut app = app_with_file(&file);
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
        let mut app = app_with_file(&file);

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
    fn duplicate_file_creates_a_copy_and_opens_it() {
        let dir = temp_project("duplicate");
        let file = dir.join("src/main.rs");
        let mut app = app_with_file(&file);

        app.execute_command(ids::DUPLICATE_FILE);
        let copy = dir.join("src/main copy.rs");
        assert!(copy.is_file(), "expected {}", copy.display());
        assert_eq!(
            app.editor.active_document().unwrap().buffer.path.as_deref(),
            Some(copy.as_path())
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn copy_file_prompts_and_writes_the_copy() {
        let dir = temp_project("copy-file");
        let file = dir.join("src/main.rs");
        let mut app = app_with_file(&file);

        app.execute_command(ids::COPY_FILE);
        assert!(matches!(
            &app.overlay,
            Overlay::Prompt(prompt) if prompt.kind == PromptKind::CopyFile
        ));
        app.submit_prompt(PromptKind::CopyFile, "src/backup.rs".to_string());
        assert!(dir.join("src/backup.rs").is_file());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rename_updates_the_open_document() {
        let dir = temp_project("rename-file");
        let file = dir.join("src/main.rs");
        let mut app = app_with_file(&file);

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
        let mut app = app_with_file(&a);
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
        let mut app = app_with_file(&file);

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
        let mut app = app_with_file(&file);
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
        let mut app = app_with_file(&file);
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
        let mut app = app_with_file(&file);
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
        let mut app = app_with_file(&a);
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
        let mut app = app_with_file(&a);
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
        let mut app = app_with_file(&a);
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
        let mut app = app_with_file(&file);
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

        let mut app = app_with_file(&a);
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
        let mut app = app_with_file(&file);
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

        let mut app = app_with_file(&a);
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
        let mut app = app_with_file(&file);
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
        let mut app = app_with_file(&file);
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
        let mut app = app_with_file(&file);

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
        let mut app = app_with_file(&file);

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
        let mut app = app_with_file(&file);

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

        let mut app = app_with_file(&file);
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

        let mut app = app_with_file(&file);
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

        let mut app = app_with_file(&file);
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

        let mut app = app_with_file(&file);
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

        let mut app = app_with_file(&file);
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
    fn go_to_definition_falls_back_to_the_project() {
        let dir = temp_project("definition-project");
        let a = dir.join("src/main.rs");
        let b = dir.join("src/lib.rs");
        fs::write(&a, "fn main() {\n    helper();\n}\n").unwrap();
        fs::write(&b, "pub fn helper() {}\n").unwrap();
        let mut app = app_with_file(&a);
        app.editor
            .active_document_mut()
            .unwrap()
            .move_to(Position::new(1, 4));

        app.execute_command(ids::GOTO_DEFINITION);

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            app.apply_background_events();
            if matches!(app.overlay, Overlay::Picker(_)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let Overlay::Picker(picker) = &app.overlay else {
            panic!("expected the workspace symbol picker");
        };
        let labels: Vec<String> = (0..picker.filtered.len())
            .filter_map(|index| picker.item(index))
            .map(|item| item.label.clone())
            .collect();
        assert!(
            labels.iter().any(|label| label == "helper"),
            "labels: {labels:?}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_references_lists_every_occurrence() {
        let dir = temp_project("references");
        let file = dir.join("src/main.rs");
        fs::write(&file, "fn main() {\n    helper();\n}\n\nfn helper() {}\n").unwrap();

        let mut app = app_with_file(&file);
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

        let mut app = app_with_file(&file);
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

        let mut app = app_with_file(&file);
        app.execute_command(ids::COMPLETE);
        let state = app.completion.as_ref().expect("completion should open");
        assert!(state.items.iter().any(|item| item.label == "fn"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn typing_offers_completion_automatically() {
        let dir = temp_project("complete-auto");
        let file = dir.join("src/main.rs");
        fs::write(&file, "fn main() {\n    let value = 1;\n}\n").unwrap();

        let mut app = app_with_file(&file);
        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        assert!(
            app.completion.is_none(),
            "the popup waits for the typing pause"
        );

        std::thread::sleep(Duration::from_millis(160));
        app.poll_auto_completion();
        let state = app.completion.as_ref().expect("completion should open");
        assert!(
            state.items.iter().any(|item| item.label == "value"),
            "the buffer identifier should be offered"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn alt_shortcuts_toggle_inline_diagnostics_and_diff() {
        let dir = temp_project("alt-keys");
        let file = dir.join("src/main.rs");
        let mut app = app_with_file(&file);

        let before = app.inline_diagnostics;
        app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::ALT));
        assert_ne!(
            app.inline_diagnostics, before,
            "Alt+I toggles inline diagnostics"
        );

        // Alt+D opens a diff; with no repository it explains itself but must not
        // panic or open an overlay.
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT));
        assert!(app.overlay.is_none());
        assert!(
            app.status_message()
                .unwrap_or("")
                .contains("Not a git repository"),
            "status was {:?}",
            app.status_message()
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn motion_and_scene_commands_toggle() {
        let dir = temp_project("motion");
        let mut app = App::new(Some(&dir)).unwrap();

        assert!(app.motion, "motion defaults on");
        app.execute_command(ids::TOGGLE_MOTION);
        assert!(!app.motion, "motion toggles off");
        app.execute_command(ids::TOGGLE_MOTION);
        assert!(app.motion, "motion toggles back on");

        let scene = app.welcome_scene;
        app.execute_command(ids::WELCOME_SCENE);
        assert_ne!(app.welcome_scene, scene, "the scene cycles");

        // `v` on the welcome screen cycles too.
        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        assert_ne!(app.welcome_scene, scene, "v cycles the scene");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn typing_a_space_dismisses_completion() {
        let dir = temp_project("complete-space");
        let file = dir.join("src/main.rs");
        fs::write(&file, "fn main() {\n    let value = 1;\n}\n").unwrap();

        let mut app = app_with_file(&file);
        app.execute_command(ids::COMPLETE);
        assert!(app.completion.is_some());
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(app.completion.is_none(), "a space should dismiss the popup");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn automatic_completion_is_silent_in_comments() {
        let dir = temp_project("complete-comment");
        let file = dir.join("src/main.rs");
        fs::write(&file, "// a comment with words\nfn main() {}\n").unwrap();

        let mut app = app_with_file(&file);
        let end_of_comment = app
            .editor
            .active_document()
            .unwrap()
            .buffer
            .line_text(0)
            .chars()
            .count();
        app.editor
            .active_document_mut()
            .unwrap()
            .move_to(Position::new(0, end_of_comment));
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));

        std::thread::sleep(Duration::from_millis(160));
        app.poll_auto_completion();
        assert!(
            app.completion.is_none(),
            "no popup should appear inside a comment"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stale_completion_response_is_dropped() {
        let dir = temp_project("complete-stale");
        let file = dir.join("src/main.rs");
        let mut app = app_with_file(&file);
        app.execute_command(ids::COMPLETE);
        assert!(app.completion.is_some());

        // A newer request is outstanding; the older response must be ignored.
        app.completion_request = Some((LanguageId::Rust, 2));
        app.handle_lsp_response(
            LanguageId::Rust,
            RequestKind::Completion,
            1,
            Ok(serde_json::json!([{ "label": "stale_member" }])),
        );
        assert!(
            !app.completion
                .as_ref()
                .unwrap()
                .items
                .iter()
                .any(|item| item.label == "stale_member"),
            "a superseded response must not be applied"
        );

        app.handle_lsp_response(
            LanguageId::Rust,
            RequestKind::Completion,
            2,
            Ok(serde_json::json!([{ "label": "fresh_member" }])),
        );
        assert!(
            app.completion
                .as_ref()
                .unwrap()
                .items
                .iter()
                .any(|item| item.label == "fresh_member"),
            "the current response should be applied"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn member_access_excludes_buffer_words() {
        let dir = temp_project("complete-member");
        let file = dir.join("src/main.rs");
        fs::write(&file, "fn main() {\n    total = counter.\n}\n").unwrap();

        let mut app = app_with_file(&file);
        // Put the cursor right after the `.` on line 1.
        let line = app.editor.active_document().unwrap().buffer.line_text(1);
        let col = line.chars().count();
        app.editor
            .active_document_mut()
            .unwrap()
            .move_to(Position::new(1, col));
        assert!(app.open_completion_inner(false));

        let state = app.completion.as_ref().unwrap();
        assert!(
            state
                .items
                .iter()
                .any(|item| item.label == "fn" || item.label == "let"),
            "language candidates remain available"
        );
        assert!(
            !state.items.iter().any(|item| item.label == "total"),
            "unrelated buffer words must not appear after a member operator"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn formatting_replaces_the_buffer_as_one_edit() {
        let dir = temp_project("format");
        let file = dir.join("src/main.rs");
        fs::write(&file, "fn main(){let x=1;}\n").unwrap();

        let mut app = app_with_file(&file);
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

        let mut app = app_with_file(&file);
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

        let mut app = app_with_file(&file);
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

        let mut app = app_with_file(&file);
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

        let mut app = app_with_file(&a);
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

        let mut app = app_with_file(&a);
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
        let mut app = app_with_file(&file);
        let mut files = std::collections::HashMap::new();
        files.insert(file.clone(), GitFileStatus::Modified);
        let mut staged = std::collections::HashSet::new();
        staged.insert(file.clone());
        app.workspace.git = crate::git::GitInfo {
            repo_root: Some(dir.clone()),
            branch: Some("main".to_string()),
            files,
            staged,
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
        assert!(
            item.detail.contains("modified · staged"),
            "detail was {}",
            item.detail
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn commit_flow_reports_state_and_prompts() {
        let dir = temp_project("commit");
        let file = dir.join("src/main.rs");
        let mut app = app_with_file(&file);

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
            staged: std::collections::HashSet::new(),
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
    fn space_in_changed_files_stages_the_selected_file() {
        let dir = temp_project("stage-space");
        let file = dir.join("src/main.rs");
        let mut app = app_with_file(&file);
        let mut files = std::collections::HashMap::new();
        files.insert(file.clone(), GitFileStatus::Modified);
        app.workspace.git = crate::git::GitInfo {
            repo_root: Some(dir.clone()),
            branch: Some("main".to_string()),
            files,
            staged: std::collections::HashSet::new(),
            available: true,
        };

        app.execute_command(ids::CHANGED_FILES);
        assert!(matches!(app.overlay, Overlay::Picker(_)));

        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(app.pending_stage_refresh);
        assert!(
            app.status_message().unwrap_or("").contains("Staging"),
            "status was {:?}",
            app.status_message()
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn diff_active_file_shows_working_tree_changes() {
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return; // Skip when git is unavailable.
        }
        let dir = temp_project("diff-file");
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
        };
        assert!(git(&["init", "-q"]).unwrap().status.success());
        assert!(
            git(&["config", "user.email", "koda@example.com"])
                .unwrap()
                .status
                .success()
        );
        assert!(
            git(&["config", "user.name", "Koda Test"])
                .unwrap()
                .status
                .success()
        );
        assert!(git(&["add", "-A"]).unwrap().status.success());
        assert!(
            git(&["commit", "-q", "-m", "init"])
                .unwrap()
                .status
                .success()
        );

        let file = dir.join("src/main.rs");
        let mut app = app_with_file(&file);
        assert!(app.workspace.git.available, "expected a git repository");

        // Change the file on disk so the working tree differs from the index.
        fs::write(&file, "fn main() {\n    let y = 2;\n}\n").unwrap();
        app.diff_active_file();

        match &app.overlay {
            Overlay::Diff(diff) => assert!(
                diff.lines
                    .iter()
                    .any(|line| line.kind == overlay::DiffLineKind::Add
                        && line.text.contains("let y")),
                "expected an added line, got {:?}",
                diff.lines.iter().map(|line| &line.text).collect::<Vec<_>>()
            ),
            _ => panic!("expected a diff overlay"),
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ctrl_shift_s_opens_save_as() {
        let dir = temp_project("save-as");
        let file = dir.join("src/main.rs");
        let mut app = app_with_file(&file);

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
        let mut app = app_with_file(&file);

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
        let mut app = app_with_file(&file);
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
        let mut app = app_with_file(&file);
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
    fn toasts_deduplicate_and_cap() {
        let dir = temp_project("toasts");
        let file = dir.join("src/main.rs");
        let mut app = app_with_file(&file);

        app.push_toast(ToastKind::Success, "Saved");
        app.push_toast(ToastKind::Success, "Saved");
        assert_eq!(app.toasts.len(), 1, "duplicate toasts are ignored");

        for index in 0..6 {
            app.push_toast(ToastKind::Info, format!("message {index}"));
        }
        assert_eq!(app.toasts.len(), 4, "toasts are capped");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn busy_reports_pending_background_work() {
        let dir = temp_project("busy");
        let file = dir.join("src/main.rs");
        let mut app = app_with_file(&file);
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
        let mut app = app_with_file(&file);

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
        let mut app = app_with_file(&file);

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
        let mut app = app_with_file(&file);

        let deadline = Instant::now() + Duration::from_secs(5);
        while app.tools.is_none() && Instant::now() < deadline {
            app.apply_background_events();
            std::thread::sleep(Duration::from_millis(5));
        }
        // Only meaningful when rust-analyzer is missing and rustup is present.
        if !app
            .tools
            .as_ref()
            .is_some_and(|tools| !tools.available(Tool::RustAnalyzer))
            || !crate::language::tools::can_install(Tool::RustAnalyzer)
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
        let mut app = app_with_file(&file);

        let deadline = Instant::now() + Duration::from_secs(5);
        while app.tools.is_none() && Instant::now() < deadline {
            app.apply_background_events();
            std::thread::sleep(Duration::from_millis(5));
        }

        let missing = app
            .tools
            .as_ref()
            .is_some_and(|tools| !tools.available(Tool::RustAnalyzer));
        let installable = crate::language::tools::can_install(Tool::RustAnalyzer);
        let offered = app.maybe_offer_tool_setup();
        assert_eq!(
            offered,
            missing && installable,
            "the offer must track tool availability and installability"
        );
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

    #[cfg(unix)]
    #[test]
    fn lsp_handshake_timeout_falls_back_to_builtin() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        // A server that accepts input and never answers `initialize`.
        const MOCK: &str = "#!/bin/sh\ncat >/dev/null\n";
        let dir = temp_project("lsp-timeout");
        let file = dir.join("src/main.rs");
        let script = dir.join("mock-lsp.sh");
        {
            let mut handle = fs::File::create(&script).unwrap();
            handle.write_all(MOCK.as_bytes()).unwrap();
        }
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();

        let mut app = app_with_file(&file);
        app.start_lsp(LanguageId::Rust, script.to_str().unwrap(), &[]);
        // Pretend the handshake has been pending for too long.
        if let Some(job) = app.lsp.get_mut(&LanguageId::Rust) {
            job.started_at = Some(Instant::now() - Duration::from_secs(60));
        }

        assert!(app.poll_lsp_health());
        assert!(matches!(app.lsp_status(), LspStatus::Failed(_)));
        let job = app.lsp.get(&LanguageId::Rust).unwrap();
        assert!(job.server.is_none());
        assert!(job.started_at.is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn automatic_restart_stops_after_the_budget() {
        let dir = temp_project("restart-budget");
        let file = dir.join("src/main.rs");
        let mut app = app_with_file(&file);
        app.lsp.insert(
            LanguageId::Rust,
            LspJob {
                restarts: MAX_LSP_RESTARTS,
                ..LspJob::default()
            },
        );

        app.schedule_lsp_restart(LanguageId::Rust);
        assert!(
            app.lsp.get(&LanguageId::Rust).unwrap().start_at.is_none(),
            "the budget is exhausted"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restart_server_reports_for_unsupported_files() {
        let dir = temp_project("restart-server");
        let file = dir.join("notes.txt");
        fs::write(&file, "hello\n").unwrap();
        let mut app = app_with_file(&file);

        app.execute_command(ids::RESTART_SERVER);
        assert!(app.lsp.is_empty());
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

        let mut app = app_with_file(&file);
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
        assert_eq!(app.lsp_status(), LspStatus::Ready);
        assert_eq!(
            app.editor.active_document().unwrap().diagnostics()[0].message,
            "boom"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_second_language_server_starts_alongside_the_first() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        // A minimal server that answers `initialize` and stays alive.
        const MOCK: &str = r#"#!/bin/sh
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
    *) result='null' ;;
  esac
  resp=$(printf '{"jsonrpc":"2.0","id":%s,"result":%s}' "$id" "$result")
  printf 'Content-Length: %s\r\n\r\n%s' "${#resp}" "$resp"
done
"#;

        let dir = temp_project("lsp-multi");
        let file = dir.join("src/main.rs");
        let script = dir.join("mock-lsp.sh");
        {
            let mut handle = fs::File::create(&script).unwrap();
            handle.write_all(MOCK.as_bytes()).unwrap();
        }
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();

        let mut app = app_with_file(&file);
        app.start_lsp(LanguageId::Rust, script.to_str().unwrap(), &[]);
        app.start_lsp(LanguageId::Python, script.to_str().unwrap(), &[]);
        assert_eq!(app.lsp.len(), 2, "each language gets its own server");

        let deadline = Instant::now() + Duration::from_secs(5);
        let ready = |app: &App| {
            [LanguageId::Rust, LanguageId::Python]
                .iter()
                .all(|language| {
                    app.lsp
                        .get(language)
                        .and_then(|job| job.server.as_ref())
                        .is_some_and(|server| server.is_ready())
                })
        };
        while !ready(&app) && Instant::now() < deadline {
            app.poll_lsp();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(ready(&app), "both servers should complete the handshake");
        assert_eq!(app.lsp_status(), LspStatus::Ready);
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
    initialize) result='{"capabilities":{"completionProvider":true,"hoverProvider":true,"definitionProvider":true,"referencesProvider":true,"renameProvider":true,"codeActionProvider":true,"workspaceSymbolProvider":true}}' ;;
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

        let mut app = app_with_file(&file);
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
                .get(&LanguageId::Rust)
                .and_then(|job| job.server.as_ref())
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

        let mut app = app_with_file(&a);
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
        let mut app = app_with_file(&file);

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
            let app = app_with_file(&file);
            let language = app.editor.active_document().unwrap().buffer.language;
            assert_eq!(language, expected, "detected {name} as {language:?}");
        }
        // `Cargo.toml` is a TOML file, even though it marks a Rust project.
        let manifest = dir.join("Cargo.toml");
        let app = app_with_file(&manifest);
        assert_eq!(
            app.editor.active_document().unwrap().buffer.language,
            LanguageId::Toml
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detects_c_and_cpp() {
        let dir = temp_project("c-langs");
        let cases = [
            (
                "main.c",
                "#include <stdio.h>\nint main(void) { return 0; }\n",
                LanguageId::C,
            ),
            (
                "app.cpp",
                "#include <iostream>\nint main() { std::cout << 1; }\n",
                LanguageId::Cpp,
            ),
            (
                "view.hpp",
                "class View { public: int w; };\n",
                LanguageId::Cpp,
            ),
        ];
        for (name, content, expected) in cases {
            let file = dir.join(name);
            fs::write(&file, content).unwrap();
            let app = app_with_file(&file);
            let language = app.editor.active_document().unwrap().buffer.language;
            assert_eq!(language, expected, "detected {name} as {language:?}");
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detects_typescript_and_javascript() {
        let dir = temp_project("web-langs");
        let cases = [
            (
                "app.ts",
                "export const total: number = 1;\n",
                LanguageId::TypeScript,
            ),
            (
                "view.tsx",
                "export const App = () => null;\n",
                LanguageId::TypeScript,
            ),
            ("index.js", "const total = 1;\n", LanguageId::JavaScript),
            (
                "component.jsx",
                "export default function App() { return null; }\n",
                LanguageId::JavaScript,
            ),
        ];
        for (name, content, expected) in cases {
            let file = dir.join(name);
            fs::write(&file, content).unwrap();
            let app = app_with_file(&file);
            let language = app.editor.active_document().unwrap().buffer.language;
            assert_eq!(language, expected, "detected {name} as {language:?}");
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detects_python_from_extension_and_shebang() {
        let dir = temp_project("python");
        let file = dir.join("app.py");
        fs::write(&file, "def main():\n    pass\n").unwrap();
        let app = app_with_file(&file);
        assert_eq!(
            app.editor.active_document().unwrap().buffer.language,
            LanguageId::Python
        );

        // An extensionless script is recognised from its shebang.
        let script = dir.join("tool");
        fs::write(&script, "#!/usr/bin/env python3\nprint('hi')\n").unwrap();
        let app = app_with_file(&script);
        assert_eq!(
            app.editor.active_document().unwrap().buffer.language,
            LanguageId::Python
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detects_shell_from_extension_and_shebang() {
        let dir = temp_project("shell");
        let file = dir.join("build.sh");
        fs::write(&file, "#!/usr/bin/env bash\necho hi\n").unwrap();
        let app = app_with_file(&file);
        assert_eq!(
            app.editor.active_document().unwrap().buffer.language,
            LanguageId::Shell
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn warns_when_a_dirty_file_changes_on_disk() {
        let dir = temp_project("external-dirty");
        let file = dir.join("src/main.rs");
        let mut app = app_with_file(&file);
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

    // ------------------------------------------------------------------
    // Welcome screen and project creation
    // ------------------------------------------------------------------

    #[test]
    fn startup_shows_the_welcome_screen_for_a_file_target() {
        let dir = temp_project("startup-file");
        let file = dir.join("src/main.rs");
        let app = App::new(Some(&file)).unwrap();

        assert!(
            app.editor.is_empty(),
            "a CLI file must not be opened automatically"
        );
        assert!(app.welcome_active());
        let labels: Vec<String> = app
            .welcome_items()
            .into_iter()
            .map(|item| item.label)
            .collect();
        assert!(labels.iter().any(|label| label.contains("Open a file")));
        assert!(
            labels
                .iter()
                .any(|label| label.contains("Create a new project"))
        );
        assert!(
            labels.iter().any(|label| label.contains("main.rs")),
            "the CLI target should be offered: {labels:?}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn startup_shows_the_welcome_screen_for_a_directory_target() {
        let dir = temp_project("startup-dir");
        let app = App::new(Some(&dir)).unwrap();
        assert!(app.editor.is_empty());
        assert!(app.welcome_active());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn welcome_target_action_opens_the_file() {
        let dir = temp_project("welcome-open");
        let file = dir.join("src/main.rs");
        let mut app = App::new(Some(&file)).unwrap();

        // Find and activate the command-line target row.
        let action = app
            .welcome_items()
            .into_iter()
            .find(|item| matches!(item.action, WelcomeAction::OpenTarget(_)))
            .expect("a target action");
        app.activate_welcome(action.action);
        assert_eq!(app.editor.len(), 1);
        // Detection runs behind the queued git/tool probes; wait for the result.
        let deadline = Instant::now() + Duration::from_secs(5);
        while app
            .editor
            .active_document()
            .is_some_and(|doc| doc.buffer.language != LanguageId::Rust)
            && Instant::now() < deadline
        {
            app.pump_background(Duration::from_millis(20));
        }
        assert_eq!(
            app.editor.active_document().unwrap().buffer.language,
            LanguageId::Rust
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resume_restores_the_saved_session() {
        let dir = temp_project("welcome-resume");
        let file = dir.join("src/main.rs");
        let app = app_with_file(&file);
        app.save_session();

        let mut fresh = App::new(Some(&file)).unwrap();
        assert!(
            fresh.resume_session.is_some(),
            "a saved session should be offered"
        );
        assert!(fresh.editor.is_empty());
        fresh.activate_welcome(WelcomeAction::Resume);
        assert_eq!(fresh.editor.len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn starting_and_quitting_without_engaging_keeps_the_session() {
        let dir = temp_project("welcome-keep");
        let file = dir.join("src/main.rs");

        // Establish a session.
        let engaged = app_with_file(&file);
        engaged.save_session();
        let session_path = session::session_path(engaged.workspace.root()).unwrap();
        let before = Session::load_from(&session_path).expect("a session");

        // A fresh start that quits untouched must not overwrite it.
        let untouched = App::new(Some(&file)).unwrap();
        untouched.save_session();
        assert_eq!(Session::load_from(&session_path), Some(before));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn home_command_returns_to_the_welcome_screen() {
        let dir = temp_project("welcome-home");
        let file = dir.join("src/main.rs");
        let mut app = app_with_file(&file);
        assert!(!app.editor.is_empty());

        app.execute_command(ids::HOME);
        assert!(app.editor.is_empty());
        assert!(app.welcome_active());
        fs::remove_dir_all(&dir).ok();
    }

    /// Drive the new-project flow up to the language step with `name`.
    fn new_project_to_language(app: &mut App, parent: &Path, name: &str) {
        app.execute_command(ids::NEW_PROJECT);
        if let Overlay::NewProject(flow) = &mut app.overlay {
            flow.browser.current = parent.to_path_buf();
            flow.browser.selected = 0;
            flow.browser.refresh();
        }
        // Choose the current folder.
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        for c in name.chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn new_project_flow_creates_and_opens_a_project() {
        let dir = temp_project("new-project");
        let parent = dir.join("projects");
        fs::create_dir_all(&parent).unwrap();
        let mut app = App::new(Some(&dir)).unwrap();

        new_project_to_language(&mut app, &parent, "myapp");
        // Language step defaults to Rust; Enter starts creation.
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.pending_project);

        let deadline = Instant::now() + Duration::from_secs(5);
        while app.pending_project && Instant::now() < deadline {
            app.pump_background(Duration::from_millis(20));
        }
        // Detection runs on the background worker behind the git and tool
        // probes `open_workspace` queues, so wait for the result.
        let deadline = Instant::now() + Duration::from_secs(5);
        while app
            .editor
            .active_document()
            .is_some_and(|doc| doc.buffer.language != LanguageId::Rust)
            && Instant::now() < deadline
        {
            app.pump_background(Duration::from_millis(20));
        }

        let root = parent.join("myapp");
        assert!(root.join("Cargo.toml").is_file(), "project not created");
        assert_eq!(
            app.workspace.root().canonicalize().unwrap(),
            root.canonicalize().unwrap()
        );
        assert_eq!(app.editor.len(), 1, "the entry file should be open");
        assert_eq!(
            app.editor.active_document().unwrap().buffer.language,
            LanguageId::Rust
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn new_project_rejects_invalid_and_existing_names() {
        let dir = temp_project("new-project-errors");
        let parent = dir.join("projects");
        fs::create_dir_all(parent.join("taken")).unwrap();
        let mut app = App::new(Some(&dir)).unwrap();

        // A traversal-style name stays on the name step with an error.
        new_project_to_language(&mut app, &parent, "..");
        assert!(matches!(
            &app.overlay,
            Overlay::NewProject(flow) if flow.step == NewProjectStep::Name && flow.error.is_some()
        ));

        // An existing directory is refused too.
        if let Overlay::NewProject(flow) = &mut app.overlay {
            flow.name.clear();
        }
        for c in "taken".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            &app.overlay,
            Overlay::NewProject(flow) if flow.error.as_deref().is_some_and(|e| e.contains("exists"))
        ));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn new_project_can_be_cancelled_and_stepped_back() {
        let dir = temp_project("new-project-cancel");
        let parent = dir.join("projects");
        fs::create_dir_all(&parent).unwrap();
        let mut app = App::new(Some(&dir)).unwrap();

        new_project_to_language(&mut app, &parent, "cancelme");
        // On the language step, Esc goes back to the name step.
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(
            &app.overlay,
            Overlay::NewProject(flow) if flow.step == NewProjectStep::Name
        ));
        // Then back to the parent step, then cancel.
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(
            &app.overlay,
            Overlay::NewProject(flow) if flow.step == NewProjectStep::Parent
        ));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.overlay.is_none());

        // Cancelling left nothing behind.
        assert!(!parent.join("cancelme").exists());
        assert!(!app.pending_project);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn opening_a_project_switches_the_workspace() {
        let dir = temp_project("switch");
        let file = dir.join("src/main.rs");
        let mut app = app_with_file(&file);

        // A second project with its own Cargo.toml and source.
        let other = dir.join("other");
        fs::create_dir_all(other.join("src")).unwrap();
        fs::write(other.join("Cargo.toml"), "[package]\nname = \"other\"\n").unwrap();
        fs::write(other.join("src/lib.rs"), "pub fn other() {}\n").unwrap();

        assert!(app.open_workspace(other.clone()));
        app.pump_background(Duration::from_millis(300));
        assert_eq!(
            app.workspace.root().canonicalize().unwrap(),
            other.canonicalize().unwrap()
        );
        assert!(
            app.editor.is_empty(),
            "the editor resets for the new project"
        );
        assert!(app.welcome_active());
        fs::remove_dir_all(&dir).ok();
    }
}
