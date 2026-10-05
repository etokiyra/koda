//! Application state and the event loop.
//!
//! This module is the conductor: it wires workspace, editor, language service,
//! commands and UI together, and translates terminal events into actions.

pub mod overlay;

mod diagnostics;
mod files;
mod git;
mod keys;
mod lsp;
mod session;

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
use crate::language::lsp::{PositionEncoding, RequestKind, Server, ServerEvent, convert};
use crate::language::provider::TokenKind;
use crate::language::symbols::is_ident_char as is_word_char;
use crate::language::tools::{Tool, ToolPurpose, ToolRegistry};
use crate::language::{Capability, LanguageId, LanguageService, WorkspaceSymbol};
use crate::project::Workspace;
use crate::project::create::{self, CreateOutcome};
use crate::recent::Recent;
use crate::search::SearchMatch;
use crate::session::Session;
use crate::terminal;
use crate::ui;
use overlay::{
    CompletionState, DiffState, DirPicker, Help, HoverState, NewProject, NewProjectStep, Overlay,
    Picker, PickerAction, PickerItem, Prompt, PromptKind, Search, SearchField, SignatureState,
    TreeFilter,
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

/// How long a server must stay ready before a later crash earns a fresh restart
/// budget. Without this, a server that initializes and then crashes on real
/// work would reset its budget on every handshake and restart forever.
const LSP_STABLE_UPTIME: Duration = Duration::from_secs(60);

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

/// How to launch a language server: the program, its arguments and any extra
/// environment (used to point managed servers at Koda-provisioned runtimes).
struct LspLaunch {
    program: String,
    args: &'static [&'static str],
    env: Vec<(String, String)>,
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
    /// When the server last became ready, for the stable-uptime budget reset.
    ready_at: Option<Instant>,
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

/// A document-sensitive language-server request.
///
/// The response may only be applied if it is still the newest request for its
/// kind and the document has not changed since it was issued, so a slow server
/// cannot edit or move the cursor over newer text.
#[derive(Clone)]
struct PendingDocRequest {
    language: LanguageId,
    id: i64,
    path: PathBuf,
    version: u64,
}

impl PendingDocRequest {
    /// Whether `(language, id)` is still this request and the document at
    /// `path` is still open with the same buffer version.
    fn is_current(&self, app: &App, language: LanguageId, id: i64) -> bool {
        if self.language != language || self.id != id {
            return false;
        }
        app.editor
            .documents
            .iter()
            .find(|doc| same_file(doc.buffer.path.as_deref(), &self.path))
            .is_some_and(|doc| doc.buffer.version == self.version)
    }
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
    /// Signature-help popup, when open.
    pub signature: Option<SignatureState>,
    /// The id of the newest signature-help request, so an older response can be
    /// discarded when the cursor has moved on.
    signature_request: Option<(LanguageId, i64)>,
    /// The id of the newest hover request, so a late answer cannot replace a
    /// newer one.
    hover_request: Option<(LanguageId, i64)>,
    /// Screen position of the editor cursor, updated during rendering.
    pub cursor_screen: Option<(u16, u16)>,
    pub tree_visible: bool,
    /// Whether diagnostic messages are shown at the end of their line.
    pub inline_diagnostics: bool,
    /// Whether long lines soft-wrap instead of scrolling horizontally.
    pub wrap: bool,
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
    /// A language-server formatting request awaiting a result, with the buffer
    /// version it was computed against.
    pending_lsp_format: Option<(PathBuf, LanguageId, i64, u64)>,
    /// An in-flight definition request.
    pending_definition: Option<PendingDocRequest>,
    /// An in-flight references request.
    pending_references: Option<PendingDocRequest>,
    /// An in-flight rename request.
    pending_rename_request: Option<PendingDocRequest>,
    /// An in-flight code-action request.
    pending_code_action_request: Option<PendingDocRequest>,
    /// The document and version the offered code actions were computed against.
    pending_code_actions_context: Option<(PathBuf, u64)>,
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
            signature: None,
            signature_request: None,
            hover_request: None,
            cursor_screen: None,
            tree_visible: true,
            inline_diagnostics: true,
            wrap: false,
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
            pending_lsp_format: None,
            pending_definition: None,
            pending_references: None,
            pending_rename_request: None,
            pending_code_action_request: None,
            pending_code_actions_context: None,
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
        app.resume_session = crate::session::session_path(app.workspace.root())
            .and_then(|path| Session::load_from(&path));

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
        if self.pending_format.is_some() || self.pending_lsp_format.is_some() {
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
                self.after_typed_char(c);
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

    /// React to a literal character typed in the editor: drive completion and
    /// signature help together.
    fn after_typed_char(&mut self, c: char) {
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

    /// Ask the language server for signature help at the cursor.
    fn request_lsp_signature(&mut self) {
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
        let col = self.lsp_col(language, &path, row, col);
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
            ids::SELECT_ALL_OCCURRENCES => self.with_doc(|d| d.select_all_occurrences()),
            ids::ADD_CURSOR_BELOW => self.with_doc(|d| d.add_cursor_below()),
            ids::ADD_CURSOR_ABOVE => self.with_doc(|d| d.add_cursor_above()),
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
            ids::TOGGLE_WRAP => self.toggle_wrap(),
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
            PickerAction::RevealLsp { path, position } => self.reveal_lsp(path, position),
            PickerAction::Info(message) => self.set_status(message),
            PickerAction::InstallTool(tool) => self.install_tool(tool),
            PickerAction::ToolActions(tool) => self.tool_actions(tool),
            PickerAction::UpdateTool(tool) => self.install_tool(tool),
            PickerAction::RemoveTool(tool) => self.remove_tool(tool),
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
                let Some((path, row, col)) = self.pending_rename.take() else {
                    return;
                };
                // The prompt may be answered after the user switched tabs, so
                // resolve the language and version from the document itself.
                let language = self
                    .editor
                    .documents
                    .iter()
                    .find(|doc| same_file(doc.buffer.path.as_deref(), &path))
                    .map(|doc| doc.buffer.language)
                    .or_else(|| self.lsp_language());
                let Some(language) = language else {
                    self.set_status("Rename needs a language server");
                    return;
                };
                let lsp_col = self.lsp_col(language, &path, row, col);
                let version = self.document_version(&path).unwrap_or(0);
                if let Some(server) = self
                    .lsp
                    .get_mut(&language)
                    .and_then(|job| job.server.as_mut())
                {
                    self.pending_rename_request =
                        server
                            .rename(&path, row, lsp_col, &input)
                            .map(|id| PendingDocRequest {
                                language,
                                id,
                                path: path.clone(),
                                version,
                            });
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

    // ----------------------------------------------------------------------
    // File and tab operations
    // ----------------------------------------------------------------------

    // ----------------------------------------------------------------------
    // Welcome screen and project opening
    // ----------------------------------------------------------------------

    // ----------------------------------------------------------------------
    // Session persistence
    // ----------------------------------------------------------------------

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

    /// Turn soft wrap on or off. The wrap width itself is recomputed by the
    /// renderer on the next frame.
    fn toggle_wrap(&mut self) {
        self.wrap = !self.wrap;
        if let Some(doc) = self.editor.active_document_mut() {
            doc.preferred_col = None;
            doc.scroll_left = 0;
            doc.scroll_subline = 0;
        }
        let state = if self.wrap { "on" } else { "off" };
        self.set_status(format!("Soft wrap {state}"));
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
                    // If a server for this language is already ready, attach the
                    // newly detected document to it. Handshake-time `lsp_ready`
                    // only covers the files open at that instant.
                    self.sync_document_to_lsp(&path, language);
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

        // Prefer the language server's formatter when it advertises one; the
        // built-in formatter tools remain the fallback.
        if self.lsp_supports(RequestKind::Formatting)
            && let Some(language) = self.lsp_language()
        {
            let tab_size = self
                .editor
                .active_document()
                .map(|doc| doc.indent_width())
                .unwrap_or(4);
            let version = self.document_version(&path).unwrap_or(0);
            if let Some(server) = self
                .lsp
                .get_mut(&language)
                .and_then(|job| job.server.as_mut())
            {
                self.pending_lsp_format = server
                    .formatting(&path, tab_size, true)
                    .map(|id| (path.clone(), language, id, version));
                if self.pending_lsp_format.is_some() {
                    self.set_status("Formatting…");
                    return;
                }
            }
        }

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

    // ----------------------------------------------------------------------
    // Language server
    // ----------------------------------------------------------------------

    /// Ask the server for code actions over the cursor or selection.
    fn code_actions(&mut self) {
        let Some(language) = self.lsp_language() else {
            self.set_status("Code actions need a language server (see Language Setup…)");
            return;
        };
        let Some((path, row, col)) = self.lsp_target_for(RequestKind::CodeActions) else {
            self.set_status("Code actions need a language server (see Language Setup…)");
            return;
        };
        let range = self
            .editor
            .active_document()
            .and_then(|doc| doc.selection_range())
            .map(|(start, end)| {
                (
                    (
                        start.row,
                        self.lsp_col(language, &path, start.row, start.col),
                    ),
                    (end.row, self.lsp_col(language, &path, end.row, end.col)),
                )
            })
            .unwrap_or_else(|| {
                let col = self.lsp_col(language, &path, row, col);
                ((row, col), (row, col))
            });
        let version = self.document_version(&path).unwrap_or(0);
        if let Some(server) = self.active_server_mut() {
            self.pending_code_action_request =
                server
                    .code_action(&path, range.0, range.1)
                    .map(|id| PendingDocRequest {
                        language,
                        id,
                        path: path.clone(),
                        version,
                    });
        }
        self.set_status("Finding code actions…");
    }

    /// Apply the code action at `index`, either as an edit or a command.
    fn apply_code_action(&mut self, index: usize) {
        let Some(action) = self.pending_code_actions.get(index).cloned() else {
            return;
        };
        // The actions were computed against a snapshot; refuse to apply them if
        // the document has moved on since.
        let Some((path, version)) = self.pending_code_actions_context.clone() else {
            return;
        };
        if self.document_version(&path) != Some(version) {
            self.pending_code_actions.clear();
            self.pending_code_actions_context = None;
            self.set_status("The document changed; run code actions again");
            return;
        }
        let language = self
            .editor
            .documents
            .iter()
            .find(|doc| same_file(doc.buffer.path.as_deref(), &path))
            .map(|doc| doc.buffer.language)
            .or_else(|| self.lsp_language());
        let Some(language) = language else {
            return;
        };
        if let Some(edit) = action.edit {
            let files = convert::workspace_edit(&edit);
            let applied = self.apply_workspace_edit(files, language);
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

    /// Apply a workspace edit produced by the server for `language`, returning
    /// the number of edits applied.
    ///
    /// Every range is converted from the connection's position encoding to Koda
    /// character offsets using the document's current text, so an edit can never
    /// land in the middle of a multi-byte character.
    fn apply_workspace_edit(
        &mut self,
        files: Vec<convert::FileEdit>,
        language: LanguageId,
    ) -> usize {
        let encoding = self.lsp_encoding(language);
        self.apply_workspace_edit_with_encoding(files, encoding)
    }

    /// [`apply_workspace_edit`] with the encoding supplied explicitly, so the
    /// conversion is testable without a live server.
    fn apply_workspace_edit_with_encoding(
        &mut self,
        files: Vec<convert::FileEdit>,
        encoding: PositionEncoding,
    ) -> usize {
        let mut applied = 0;
        for file in files {
            if let Some(doc) = self
                .editor
                .documents
                .iter_mut()
                .find(|doc| same_file(doc.buffer.path.as_deref(), &file.path))
            {
                // Convert against the document's current text before applying;
                // edits are then applied from the end so earlier offsets stay
                // valid.
                let mut edits =
                    convert::edits_to_chars(&file.edits, |row| doc.buffer.line_text(row), encoding);
                edits.sort_by_key(|edit| Reverse((edit.start.0, edit.start.1)));
                for edit in edits {
                    doc.replace_range(
                        Position::new(edit.start.0, edit.start.1),
                        Position::new(edit.end.0, edit.end.1),
                        &edit.new_text,
                    );
                    applied += 1;
                }
            } else if let Ok(text) = crate::editor::buffer::read_text(&file.path) {
                let lines: Vec<&str> = text.split('\n').collect();
                let edits = convert::edits_to_chars(
                    &file.edits,
                    |row| lines.get(row).copied().unwrap_or("").to_string(),
                    encoding,
                );
                let updated = apply_text_edits(&text, &edits);
                if crate::filesystem::write_atomic(&file.path, &updated).is_ok() {
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
                    PickerAction::RevealLsp {
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

    /// Open `path` and place the cursor at `position`, centred in the viewport.
    fn reveal(&mut self, path: PathBuf, position: Position) {
        self.open_path(path);
        self.jump_to(position);
    }

    /// Like [`reveal`], but `position` is LSP-encoded and is converted using the
    /// opened document's text before the cursor moves.
    fn reveal_lsp(&mut self, path: PathBuf, position: Position) {
        self.open_path(path);
        let position = self.lsp_position_to_char(position);
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
        if let Some(language) = self.lsp_language()
            && let Some((path, row, col)) = self.lsp_target_for(RequestKind::Definition)
        {
            let col = self.lsp_col(language, &path, row, col);
            let version = self.document_version(&path).unwrap_or(0);
            if let Some(server) = self.active_server_mut() {
                self.pending_definition =
                    server
                        .definition(&path, row, col)
                        .map(|id| PendingDocRequest {
                            language,
                            id,
                            path: path.clone(),
                            version,
                        });
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
        if let Some(language) = self.lsp_language()
            && let Some((path, row, col)) = self.lsp_target_for(RequestKind::References)
        {
            let col = self.lsp_col(language, &path, row, col);
            let version = self.document_version(&path).unwrap_or(0);
            if let Some(server) = self.active_server_mut() {
                self.pending_references =
                    server
                        .references(&path, row, col)
                        .map(|id| PendingDocRequest {
                            language,
                            id,
                            path: path.clone(),
                            version,
                        });
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
        // A language server may format even when the provider has no formatter.
        if self.lsp_supports(RequestKind::Formatting) {
            return (true, None);
        }
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
        // Prefer the active file's language; with no file open, use the
        // project's detected language so opening a project is enough to be
        // offered its tooling.
        let language = self
            .editor
            .active_document()
            .map(|doc| doc.buffer.language)
            .filter(|language| *language != LanguageId::Unknown)
            .unwrap_or_else(|| self.workspace.project.kind.language());
        if language == LanguageId::Unknown {
            return false;
        }
        // An available server means there is nothing to offer; otherwise offer
        // the first installable candidate.
        if Tool::available_server(language, tools).is_some() {
            return false;
        }
        let Some(tool) = Tool::installable_server(language, tools) else {
            return false;
        };
        if !self.setup_offered.insert(language) {
            return false;
        }

        let install = PickerItem::new(
            format!("Install {}", tool.label()),
            tool.setup_reason(),
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
                // A managed tool shows its version and on-disk size, and can be
                // updated or removed. A user/system install is read-only.
                let detail = match status.managed_size() {
                    Some(bytes) => format!(
                        "{} · {}{}",
                        status.summary(),
                        crate::language::tools::human_bytes(bytes),
                        if status.is_managed() {
                            " · Koda-managed"
                        } else {
                            ""
                        }
                    ),
                    None => status.summary(),
                };
                if status.is_managed() {
                    PickerItem::new(label, detail, PickerAction::ToolActions(tool))
                        .shortcut("Enter")
                } else {
                    PickerItem::new(label, detail.clone(), PickerAction::Info(detail))
                }
            } else if crate::language::tools::can_install(tool) {
                let hint = tool.setup_reason();
                PickerItem::new(label, hint, PickerAction::InstallTool(tool)).shortcut("Enter")
            } else {
                // Explain exactly why Koda cannot install it, and what the user
                // can do instead, rather than repeating the generic install hint.
                let reason = tool.setup_reason();
                PickerItem::new(label, reason.clone(), PickerAction::Info(reason.clone()))
                    .disabled(reason)
            };
            items.push(item);
        }
        let mut picker = Picker::new("Language Setup", "Language tools…", items);
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Offer Update and Remove for a Koda-managed tool.
    fn tool_actions(&mut self, tool: Tool) {
        let Some(status) = self
            .tools
            .as_ref()
            .and_then(|tools| tools.status(tool))
            .cloned()
        else {
            self.set_status("Tool status is unavailable");
            return;
        };
        if !status.is_managed() {
            self.set_status(format!("{} is not managed by Koda", tool.label()));
            return;
        }
        let size = status
            .managed_size()
            .map(|bytes| format!(" ({})", crate::language::tools::human_bytes(bytes)))
            .unwrap_or_default();
        let update = PickerItem::new(
            format!("Update {}", tool.label()),
            format!("Reinstall Koda's pinned version{size}"),
            PickerAction::UpdateTool(tool),
        )
        .shortcut("Enter");
        let remove = PickerItem::new(
            format!("Remove {}", tool.label()),
            format!("Delete Koda's managed files{size}"),
            PickerAction::RemoveTool(tool),
        );
        let cancel = PickerItem::new(
            "Cancel",
            "Change nothing",
            PickerAction::Info("Nothing changed".to_string()),
        );
        let mut picker = Picker::new(
            format!("{} (Koda-managed)", tool.label()),
            "Choose…",
            vec![update, remove, cancel],
        );
        picker.refilter();
        self.overlay = Overlay::Picker(picker);
    }

    /// Remove a Koda-managed tool's files.
    ///
    /// Refuses to touch anything Koda does not own, and never removes the shared
    /// tools root itself. A `discover` afterwards refreshes the setup view.
    fn remove_tool(&mut self, tool: Tool) {
        if self.pending_install.is_some() {
            self.set_status("An install is already running");
            return;
        }
        let Some(status) = self
            .tools
            .as_ref()
            .and_then(|tools| tools.status(tool))
            .cloned()
        else {
            self.set_status("Tool status is unavailable");
            return;
        };
        if !status.is_managed() {
            self.set_error(format!(
                "{} is not managed by Koda — not removing it",
                tool.label()
            ));
            return;
        }
        let Some(dir) = status.managed_dir() else {
            self.set_error(format!("Could not locate {}'s files", tool.label()));
            return;
        };
        if crate::language::tools::tools_dir().is_some_and(|root| dir == root) {
            self.set_error("Refusing to remove the shared tools directory");
            return;
        }
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {
                self.push_toast(ToastKind::Info, format!("Removed {}", tool.label()));
                self.background.discover_tools();
            }
            Err(err) => self.set_error(format!("Could not remove {}: {err}", tool.label())),
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
        // `Ctrl+B` is the file panel: reveal and focus it, focus it when the
        // editor has focus, and hide it once it is focused.
        if !self.tree_visible {
            self.tree_visible = true;
            self.focus = Focus::FileTree;
        } else if self.focus == Focus::Editor {
            self.focus = Focus::FileTree;
        } else {
            self.tree_visible = false;
            self.focus = Focus::Editor;
        }
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

/// Whether a key is "shifted".
///
/// Real terminals vary: some report `Shift` explicitly, some report an
/// uppercase character for a shifted letter, and some (without the kitty
/// keyboard protocol) conflate `Ctrl+Shift+P` with `Ctrl+P`. Treating an
/// uppercase character as shifted recovers the former cases, so `Ctrl+Shift+P`
/// opens the command palette rather than quick open.
fn key_shift(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::SHIFT)
        || matches!(key.code, KeyCode::Char(c) if c.is_ascii_uppercase())
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
mod tests;
