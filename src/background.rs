//! A small background worker pool.
//!
//! Koda must never block the UI on work that is not the user's keystroke. A few
//! worker threads share a request channel; the app sends requests and drains
//! results as they arrive, so expensive operations stay off the render path and
//! one slow formatter or installer cannot stall unrelated editor services.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::git::GitInfo;
use crate::language::LanguageService;
use crate::language::WorkspaceSymbol;
use crate::language::detection::Confidence;
use crate::language::diagnostics::Diagnostic;
use crate::language::format::FormatOutcome;
use crate::language::id::LanguageId;
use crate::language::tools::{Tool, ToolRegistry};

/// Work sent to the background thread.
enum Request {
    /// Detect the language of a file within a project.
    Detect { path: PathBuf, markers: Vec<String> },
    /// Compute diagnostics for a document snapshot.
    Diagnostics {
        path: PathBuf,
        language: LanguageId,
        text: String,
        revision: u64,
    },
    /// Format a document snapshot with the language's formatter.
    Format {
        path: PathBuf,
        language: LanguageId,
        text: String,
        revision: u64,
    },
    /// Scan a project for named definitions.
    WorkspaceSymbols { root: PathBuf, revision: u64 },
    /// Detect the languages present in a project, for project setup.
    ProjectLanguages { root: PathBuf },
    /// Search a project for a text query.
    SearchProject {
        root: PathBuf,
        query: String,
        revision: u64,
    },
    /// Probe for the external tools Koda can drive.
    DiscoverTools,
    /// Install a tool through its trusted package manager.
    InstallTool(Tool),
    /// Recompute git status for a repository root.
    RefreshGit { root: PathBuf },
    /// Stage all changes and commit them.
    GitCommit { root: PathBuf, message: String },
    /// Stage or unstage one path.
    GitStage {
        root: PathBuf,
        path: PathBuf,
        staged: bool,
    },
    /// Scaffold a new project.
    CreateProject {
        parent: PathBuf,
        name: String,
        language: LanguageId,
    },
}

/// A finished piece of background work.
pub enum Event {
    Detected {
        path: PathBuf,
        language: LanguageId,
        confidence: Confidence,
    },
    Diagnostics {
        path: PathBuf,
        revision: u64,
        diagnostics: Vec<Diagnostic>,
    },
    Formatted {
        path: PathBuf,
        revision: u64,
        outcome: FormatOutcome,
    },
    WorkspaceSymbols {
        revision: u64,
        symbols: Vec<WorkspaceSymbol>,
    },
    /// The languages detected in a project, for project setup.
    ProjectLanguages(Vec<LanguageId>),
    /// The result of a project-wide text search.
    SearchResults {
        revision: u64,
        matches: Vec<crate::search::SearchMatch>,
    },
    /// The result of probing for external tools.
    Tools(ToolRegistry),
    /// The result of installing a tool.
    ToolInstalled {
        tool: Tool,
        result: Result<String, String>,
    },
    Git(GitInfo),
    /// The result of staging and committing.
    GitCommitted {
        result: Result<String, String>,
    },
    /// The result of staging or unstaging one path.
    GitStaged {
        path: PathBuf,
        staged: bool,
        result: Result<(), String>,
    },
    /// The result of scaffolding a new project.
    ProjectCreated {
        name: String,
        language: LanguageId,
        outcome: crate::project::create::CreateOutcome,
    },
}

/// Handle to the background worker.
pub struct Background {
    requests: mpsc::SyncSender<Request>,
    events: Receiver<Event>,
}

/// How many background operations may run at once. A small pool keeps a slow
/// formatter or installer from blocking detection, diagnostics and git.
const WORKER_THREADS: usize = 3;

/// The most queued requests before automatic snapshots are dropped. Control
/// requests still block briefly, but a flood of automatic work cannot grow the
/// queue without bound.
const REQUEST_QUEUE: usize = 128;

/// The most files a project-language scan reads. The walk is `.gitignore`-aware
/// and skips generated directories; the cap keeps a huge tree predictable.
const PROJECT_SCAN_LIMIT: usize = 4000;

impl Background {
    /// Spawn the workers, sharing the language service with the UI thread.
    pub fn spawn(language: Arc<LanguageService>) -> Self {
        let (request_tx, request_rx) = mpsc::sync_channel::<Request>(REQUEST_QUEUE);
        let (event_tx, event_rx) = mpsc::channel::<Event>();
        let request_rx = Arc::new(Mutex::new(request_rx));

        for index in 0..WORKER_THREADS {
            let requests = Arc::clone(&request_rx);
            let events = event_tx.clone();
            let language = Arc::clone(&language);
            let _ = thread::Builder::new()
                .name(format!("koda-background-{index}"))
                .spawn(move || {
                    loop {
                        // Hold the lock only while waiting, so the other workers
                        // can run their operations concurrently.
                        let request = {
                            let receiver = match requests.lock() {
                                Ok(receiver) => receiver,
                                Err(poisoned) => poisoned.into_inner(),
                            };
                            receiver.recv()
                        };
                        let Ok(request) = request else { break };
                        handle_request(request, &language, &events);
                    }
                });
        }
        drop(event_tx);

        Background {
            requests: request_tx,
            events: event_rx,
        }
    }

    /// Queue a request that must not be dropped.
    fn send(&self, request: Request) {
        let _ = self.requests.send(request);
    }

    /// Queue an automatic request, dropping it when the queue is saturated.
    ///
    /// Used for diagnostics snapshots, which a later edit supersedes anyway.
    fn try_send(&self, request: Request) {
        let _ = self.requests.try_send(request);
    }

    /// Ask for a file's language to be detected.
    pub fn detect(&self, path: PathBuf, markers: Vec<String>) {
        self.send(Request::Detect { path, markers });
    }

    /// Ask for diagnostics on a document snapshot.
    ///
    /// `revision` lets the app discard results that arrive out of order.
    pub fn diagnose(&self, path: PathBuf, language: LanguageId, text: String, revision: u64) {
        self.try_send(Request::Diagnostics {
            path,
            language,
            text,
            revision,
        });
    }

    /// Ask for a document snapshot to be formatted.
    pub fn format(&self, path: PathBuf, language: LanguageId, text: String, revision: u64) {
        self.send(Request::Format {
            path,
            language,
            text,
            revision,
        });
    }

    /// Ask for a project-wide symbol scan.
    pub fn workspace_symbols(&self, root: PathBuf, revision: u64) {
        let _ = self
            .requests
            .send(Request::WorkspaceSymbols { root, revision });
    }

    /// Ask for the languages present in a project, for project setup.
    pub fn detect_project_languages(&self, root: PathBuf) {
        let _ = self.requests.send(Request::ProjectLanguages { root });
    }

    /// Ask for a project-wide text search.
    pub fn search_project(&self, root: PathBuf, query: String, revision: u64) {
        let _ = self.requests.send(Request::SearchProject {
            root,
            query,
            revision,
        });
    }

    /// Ask for the external tools to be probed.
    pub fn discover_tools(&self) {
        let _ = self.requests.send(Request::DiscoverTools);
    }

    /// Ask for a tool to be installed through its trusted package manager.
    pub fn install_tool(&self, tool: Tool) {
        let _ = self.requests.send(Request::InstallTool(tool));
    }

    /// Ask for git status to be refreshed.
    pub fn refresh_git(&self, root: PathBuf) {
        let _ = self.requests.send(Request::RefreshGit { root });
    }

    /// Ask to stage all changes and commit them.
    pub fn commit_all(&self, root: PathBuf, message: String) {
        let _ = self.requests.send(Request::GitCommit { root, message });
    }

    /// Ask to stage or unstage one path.
    pub fn stage_path(&self, root: PathBuf, path: PathBuf, staged: bool) {
        let _ = self.requests.send(Request::GitStage { root, path, staged });
    }

    /// Ask to scaffold a new project.
    pub fn create_project(&self, parent: PathBuf, name: String, language: LanguageId) {
        let _ = self.requests.send(Request::CreateProject {
            parent,
            name,
            language,
        });
    }

    /// Take the next finished event, if any.
    pub fn try_recv(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// Wait up to `timeout` for the next event.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }
}

/// Execute one request and publish its event(s).
fn handle_request(request: Request, language: &LanguageService, events: &Sender<Event>) {
    match request {
        Request::Detect { path, markers } => {
            let result = language.detect_file(&path, &markers);
            let _ = events.send(Event::Detected {
                path,
                language: result.language,
                confidence: result.confidence,
            });
        }
        Request::Diagnostics {
            path,
            language: id,
            text,
            revision,
        } => {
            let diagnostics = language.provider(id).diagnostics(&text);
            let _ = events.send(Event::Diagnostics {
                path,
                revision,
                diagnostics,
            });
        }
        Request::Format {
            path,
            language: id,
            text,
            revision,
        } => {
            let outcome = language.provider(id).format(&path, &text);
            let _ = events.send(Event::Formatted {
                path,
                revision,
                outcome,
            });
        }
        Request::WorkspaceSymbols { root, revision } => {
            let symbols = language.workspace_symbols(&root, 3000);
            let _ = events.send(Event::WorkspaceSymbols { revision, symbols });
        }
        Request::ProjectLanguages { root } => {
            let languages = language.detect_project_languages(&root, PROJECT_SCAN_LIMIT);
            let _ = events.send(Event::ProjectLanguages(languages));
        }
        Request::SearchProject {
            root,
            query,
            revision,
        } => {
            let matches = crate::search::search_project(&root, &query, 500);
            let _ = events.send(Event::SearchResults { revision, matches });
        }
        Request::DiscoverTools => {
            let _ = events.send(Event::Tools(ToolRegistry::discover_cached()));
        }
        Request::InstallTool(tool) => {
            let result = crate::language::tools::install(tool);
            let _ = events.send(Event::ToolInstalled { tool, result });
            // Re-probe so the setup view and server startup see the new state
            // immediately.
            let _ = events.send(Event::Tools(ToolRegistry::discover_cached()));
        }
        Request::RefreshGit { root } => {
            let _ = events.send(Event::Git(GitInfo::detect(&root)));
        }
        Request::GitCommit { root, message } => {
            let result = crate::git::commit_all(&root, &message);
            let _ = events.send(Event::GitCommitted { result });
            // Refresh so the status bar and changed-files list reflect the new
            // state immediately.
            let _ = events.send(Event::Git(GitInfo::detect(&root)));
        }
        Request::GitStage { root, path, staged } => {
            let result = if staged {
                crate::git::stage(&root, &path)
            } else {
                crate::git::unstage(&root, &path)
            };
            let _ = events.send(Event::GitStaged {
                path,
                staged,
                result,
            });
            let _ = events.send(Event::Git(GitInfo::detect(&root)));
        }
        Request::CreateProject {
            parent,
            name,
            language,
        } => {
            let outcome = crate::project::create::create(&parent, &name, language);
            let _ = events.send(Event::ProjectCreated {
                name,
                language,
                outcome,
            });
        }
    }
}
