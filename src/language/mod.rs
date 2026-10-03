//! Language intelligence.
//!
//! This module is deliberately split into two independent halves:
//!
//! * [`detection`] decides *which* language a file belongs to, combining project
//!   context, file metadata and content.
//! * [`provider`] decides *how* Koda supports that language.
//!
//! [`LanguageService`] wires the two together for convenient use by the app, but
//! neither half depends on the other.

pub mod c;
pub mod completion;
pub mod csharp;
pub mod data;
pub mod detection;
pub mod diagnostics;
pub mod format;
pub mod go;
pub mod hover;
pub mod id;
pub mod java;
pub mod json;
pub mod lsp;
pub mod markdown;
pub mod provider;
pub mod python;
pub mod rust;
pub mod shell;
pub mod symbols;
pub mod toml;
pub mod tools;
pub mod web;
pub mod yaml;

use std::path::Path;

pub use id::LanguageId;
pub use provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, ProviderRegistry, TokenKind,
};
pub use symbols::WorkspaceSymbol;
pub use tools::{Tool, ToolRegistry};

use detection::{DetectionEngine, DetectionResult};

/// Number of bytes read from the start of a file for content-based detection.
const CONTENT_SAMPLE_BYTES: usize = 4096;

/// A small facade over the provider registry and the detection engine.
pub struct LanguageService {
    pub registry: ProviderRegistry,
    pub detector: DetectionEngine,
}

impl LanguageService {
    /// Build the service with Koda's built-in languages.
    pub fn builtin() -> Self {
        let registry = ProviderRegistry::builtin();
        let detector = DetectionEngine::new(registry.descriptors());
        LanguageService { registry, detector }
    }

    /// Resolve a language to its provider (falling back to plain text).
    pub fn provider(&self, id: LanguageId) -> &dyn LanguageProvider {
        self.registry.get(id)
    }

    /// Detect the language of `path` within a project whose root declares
    /// `project_markers` (for example `["Cargo.toml"]`).
    pub fn detect_file(&self, path: &Path, project_markers: &[String]) -> DetectionResult {
        let content = read_sample(path);
        self.detector.detect(&detection::DetectionInput {
            path: Some(path),
            file_name: None,
            extension: None,
            project_markers,
            content_sample: content.as_deref(),
        })
    }

    /// Resolve a file extension to a language, using provider descriptors.
    ///
    /// Used for bulk tasks such as workspace symbol search, where reading every
    /// file for content-based detection would be too expensive.
    pub fn language_for_extension(&self, extension: &str) -> Option<LanguageId> {
        self.registry.iter().find_map(|provider| {
            provider
                .descriptor()
                .extensions
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(extension))
                .then(|| provider.id())
        })
    }

    /// Scan a project for named definitions, up to `limit` symbols.
    ///
    /// Intended for the background worker: it reads each source file once and
    /// reuses the providers' symbol scanners.
    pub fn workspace_symbols(&self, root: &Path, limit: usize) -> Vec<WorkspaceSymbol> {
        const MAX_FILE_BYTES: u64 = 512 * 1024;
        let mut found = Vec::new();
        for path in crate::filesystem::collect_files(root, 3000) {
            if found.len() >= limit {
                break;
            }
            let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            let Some(language) = self.language_for_extension(extension) else {
                continue;
            };
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            if metadata.len() > MAX_FILE_BYTES {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for symbol in self.provider(language).symbols(&text) {
                found.push(WorkspaceSymbol {
                    path: path.clone(),
                    symbol,
                });
                if found.len() >= limit {
                    break;
                }
            }
        }
        found
    }
}

/// Read a small prefix of a file for content inspection, ignoring binary files.
fn read_sample(path: &Path) -> Option<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; CONTENT_SAMPLE_BYTES];
    let read = file.read(&mut buf).ok()?;
    buf.truncate(read);
    if buf.contains(&0) {
        return None;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}
