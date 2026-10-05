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

pub mod asm;
pub mod c;
pub mod completion;
pub mod csharp;
pub mod css;
pub mod dart;
pub mod data;
pub mod detection;
pub mod diagnostics;
pub mod elixir;
pub mod format;
pub mod go;
pub mod hover;
pub mod html;
pub mod id;
pub mod java;
pub mod json;
pub mod kotlin;
pub mod lsp;
pub mod lua;
pub mod markdown;
pub mod perl;
pub mod php;
pub mod provider;
pub mod python;
pub mod ruby;
pub mod rust;
pub mod setup;
pub mod shell;
pub mod sql;
pub mod swift;
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

    /// Detect the languages present in a project by file extension.
    ///
    /// Uses the same bounded, `.gitignore`-aware walk as quick open, so it skips
    /// hidden entries, generated directories (`target`, `node_modules`, …) and
    /// ignored files, and stops after `limit` files. Extension detection is
    /// deliberately cheap: a script whose language only a shebang or its content
    /// would reveal is not counted, which keeps the project scan predictable.
    pub fn detect_project_languages(&self, root: &Path, limit: usize) -> Vec<LanguageId> {
        let mut seen: Vec<LanguageId> = Vec::new();
        for path in crate::filesystem::collect_files(root, limit) {
            let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
                continue;
            };
            let Some(language) = self.language_for_extension(extension) else {
                continue;
            };
            if language != LanguageId::Unknown && !seen.contains(&language) {
                seen.push(language);
            }
        }
        // Deterministic order: the canonical `LanguageId::ALL` order.
        LanguageId::ALL
            .iter()
            .copied()
            .filter(|language| seen.contains(language))
            .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koda-langs-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_the_languages_present_in_a_project() {
        let dir = temp_dir("mixed");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(dir.join("app.ts"), "export const x = 1;\n").unwrap();
        fs::write(dir.join("README.md"), "# hi\n").unwrap();
        let service = LanguageService::builtin();
        assert_eq!(
            service.detect_project_languages(&dir, 100),
            vec![
                LanguageId::Rust,
                LanguageId::Markdown,
                LanguageId::TypeScript
            ]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn generated_directories_are_not_scanned() {
        let dir = temp_dir("ignored");
        fs::create_dir_all(dir.join("target")).unwrap();
        fs::write(dir.join("target/generated.rs"), "fn main() {}\n").unwrap();
        fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
        fs::write(dir.join("node_modules/pkg/index.ts"), "export {};\n").unwrap();
        let service = LanguageService::builtin();
        assert!(service.detect_project_languages(&dir, 100).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_project_detects_nothing() {
        let dir = temp_dir("empty");
        let service = LanguageService::builtin();
        assert!(service.detect_project_languages(&dir, 100).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn detection_order_is_deterministic() {
        let dir = temp_dir("order");
        fs::write(dir.join("a.ts"), "export {};\n").unwrap();
        fs::write(dir.join("b.rs"), "fn main() {}\n").unwrap();
        let service = LanguageService::builtin();
        assert_eq!(
            service.detect_project_languages(&dir, 100),
            vec![LanguageId::Rust, LanguageId::TypeScript]
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
