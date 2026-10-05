//! Application state and the event loop.
//!
//! This module is the conductor: it wires workspace, editor, language service,
//! commands and UI together, and translates terminal events into actions.

pub mod overlay;

mod background;
mod commands;
mod completion;
mod diagnostics;
mod editor;
mod files;
mod git;
mod keys;
mod language;
mod lsp;
mod panes;
mod search;
mod session;
mod status;

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

    // ----------------------------------------------------------------------
    // Event handling
    // ----------------------------------------------------------------------

    // ----------------------------------------------------------------------
    // Commands
    // ----------------------------------------------------------------------

    // ----------------------------------------------------------------------
    // Editing helpers
    // ----------------------------------------------------------------------

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

    // ----------------------------------------------------------------------
    // Overlays
    // ----------------------------------------------------------------------

    // ----------------------------------------------------------------------
    // Misc
    // ----------------------------------------------------------------------

    fn request_quit(&mut self) {
        if self.editor.has_unsaved() && !self.quit_armed {
            self.quit_armed = true;
            self.set_error("Unsaved changes — Ctrl+S to save, Ctrl+Q again to quit");
            return;
        }
        self.should_quit = true;
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
