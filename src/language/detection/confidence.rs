//! Confidence and evidence types for language detection.

use std::fmt;

use crate::language::id::LanguageId;

/// How sure the detector is about its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Confidence {
    /// Very little evidence, or conflicting evidence.
    #[default]
    Low,
    /// A decent signal such as an extension or a nearby project.
    Medium,
    /// Strong, corroborated evidence such as a project marker plus matching content.
    High,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::Low => "low",
            Confidence::Medium => "medium",
            Confidence::High => "high",
        }
    }
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Where a piece of evidence came from. Useful for debugging and, later, for
/// explaining detection decisions to the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalKind {
    /// A file like `Cargo.toml` or `go.mod` at the project root.
    ProjectMarker,
    /// The file lives inside a project already believed to be of a language.
    ProjectContext,
    /// The file extension (e.g. `.rs`).
    Extension,
    /// A special file name without an extension (e.g. `Dockerfile`).
    FileName,
    /// The `#!` line of an executable script.
    Shebang,
    /// Lightweight inspection of the file's contents.
    Content,
}

impl SignalKind {
    pub fn description(self) -> &'static str {
        match self {
            SignalKind::ProjectMarker => "project marker",
            SignalKind::ProjectContext => "project context",
            SignalKind::Extension => "file extension",
            SignalKind::FileName => "file name",
            SignalKind::Shebang => "shebang",
            SignalKind::Content => "file content",
        }
    }
}

/// A single reason contributing to a detection decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub language: LanguageId,
    pub kind: SignalKind,
    /// Points contributed by this signal.
    pub weight: u32,
    /// Human readable explanation.
    pub reason: String,
}
