//! A small background worker.
//!
//! Koda must never block the UI on work that is not the user's keystroke. This
//! worker owns a thread and a channel pair: the app sends requests and drains
//! results as they arrive, so expensive operations stay off the render path.
//!
//! Today it handles language detection and git refreshes. The same shape is
//! where diagnostics, completion and other language intelligence will live, so
//! the UI is already written to receive results asynchronously.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::git::GitInfo;
use crate::language::LanguageService;
use crate::language::WorkspaceSymbol;
use crate::language::detection::Confidence;
use crate::language::diagnostics::Diagnostic;
use crate::language::format::FormatOutcome;
use crate::language::id::LanguageId;
use crate::language::tools::ToolRegistry;

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
    /// Probe for the external tools Koda can drive.
    DiscoverTools,
    /// Recompute git status for a repository root.
    RefreshGit { root: PathBuf },
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
    /// The result of probing for external tools.
    Tools(ToolRegistry),
    Git(GitInfo),
}

/// Handle to the background thread.
pub struct Background {
    requests: Sender<Request>,
    events: Receiver<Event>,
}

impl Background {
    /// Spawn the worker, sharing the language service with the UI thread.
    pub fn spawn(language: Arc<LanguageService>) -> Self {
        let (request_tx, request_rx) = mpsc::channel::<Request>();
        let (event_tx, event_rx) = mpsc::channel::<Event>();

        let _ = thread::Builder::new()
            .name("koda-background".to_string())
            .spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    match request {
                        Request::Detect { path, markers } => {
                            let result = language.detect_file(&path, &markers);
                            let _ = event_tx.send(Event::Detected {
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
                            let _ = event_tx.send(Event::Diagnostics {
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
                            let _ = event_tx.send(Event::Formatted {
                                path,
                                revision,
                                outcome,
                            });
                        }
                        Request::WorkspaceSymbols { root, revision } => {
                            let symbols = language.workspace_symbols(&root, 3000);
                            let _ = event_tx.send(Event::WorkspaceSymbols { revision, symbols });
                        }
                        Request::DiscoverTools => {
                            let _ = event_tx.send(Event::Tools(ToolRegistry::discover()));
                        }
                        Request::RefreshGit { root } => {
                            let _ = event_tx.send(Event::Git(GitInfo::detect(&root)));
                        }
                    }
                }
            });

        Background {
            requests: request_tx,
            events: event_rx,
        }
    }

    /// Ask for a file's language to be detected.
    pub fn detect(&self, path: PathBuf, markers: Vec<String>) {
        let _ = self.requests.send(Request::Detect { path, markers });
    }

    /// Ask for diagnostics on a document snapshot.
    ///
    /// `revision` lets the app discard results that arrive out of order.
    pub fn diagnose(&self, path: PathBuf, language: LanguageId, text: String, revision: u64) {
        let _ = self.requests.send(Request::Diagnostics {
            path,
            language,
            text,
            revision,
        });
    }

    /// Ask for a document snapshot to be formatted.
    pub fn format(&self, path: PathBuf, language: LanguageId, text: String, revision: u64) {
        let _ = self.requests.send(Request::Format {
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

    /// Ask for the external tools to be probed.
    pub fn discover_tools(&self) {
        let _ = self.requests.send(Request::DiscoverTools);
    }

    /// Ask for git status to be refreshed.
    pub fn refresh_git(&self, root: PathBuf) {
        let _ = self.requests.send(Request::RefreshGit { root });
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
