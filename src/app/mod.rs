//! Application state and the event loop.
//!
//! This module is the conductor: it wires workspace, editor, language service,
//! commands and UI together, and translates terminal events into actions.

pub mod overlay;

mod commands;
mod completion;
mod diagnostics;
mod files;
mod git;
mod keys;
mod language;
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

    // ----------------------------------------------------------------------
    // Commands
    // ----------------------------------------------------------------------

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

    // ----------------------------------------------------------------------
    // Diagnostics
    // ----------------------------------------------------------------------

    // ----------------------------------------------------------------------
    // Language server
    // ----------------------------------------------------------------------

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

    // ----------------------------------------------------------------------
    // Misc
    // ----------------------------------------------------------------------

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
