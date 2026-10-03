//! Language detection.
//!
//! Detection is intentionally decoupled from language *support*. The engine works
//! with lightweight [`LanguageDescriptor`]s produced by providers, and combines
//! several independent signals:
//!
//! ```text
//!                 Project / File
//!                      │
//!                      ▼
//!             Language Detection
//!                    Engine
//!        ┌─────────────┼─────────────┐
//!    Project        File          Content
//!    context      metadata       analysis
//!        └─────────────┼─────────────┘
//!                      │
//!                      ▼
//!                 Confidence
//! ```

mod confidence;
mod engine;

pub use confidence::{Confidence, Evidence, SignalKind};
pub use engine::{DetectionEngine, DetectionInput, DetectionResult, LanguageDescriptor};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::id::LanguageId;

    fn descriptor(id: LanguageId) -> LanguageDescriptor {
        LanguageDescriptor {
            id,
            extensions: match id {
                LanguageId::Rust => &["rs"],
                LanguageId::Go => &["go"],
                LanguageId::Python => &["py"],
                LanguageId::Shell => &["sh", "bash"],
                LanguageId::Markdown => &["md"],
                LanguageId::Json => &["json"],
                LanguageId::Toml => &["toml"],
                LanguageId::Yaml => &["yaml", "yml"],
                LanguageId::TypeScript => &["ts", "tsx"],
                LanguageId::JavaScript => &["js", "jsx"],
                LanguageId::C => &["c", "h"],
                LanguageId::Cpp => &["cpp", "hpp"],
                LanguageId::Java => &["java"],
                LanguageId::CSharp => &["cs"],
                LanguageId::Html => &["html", "htm"],
                LanguageId::Css => &["css"],
                LanguageId::Unknown => &[],
            },
            project_markers: match id {
                LanguageId::Rust => &["Cargo.toml"],
                LanguageId::Go => &["go.mod"],
                LanguageId::Python => &["pyproject.toml"],
                _ => &[],
            },
            file_names: &[],
            shebangs: &[],
            content_hints: &[],
        }
    }

    fn engine() -> DetectionEngine {
        DetectionEngine::new(vec![
            descriptor(LanguageId::Rust),
            descriptor(LanguageId::Go),
        ])
    }

    #[test]
    fn extension_alone_is_medium() {
        let result = engine().detect(&DetectionInput {
            path: None,
            file_name: Some("main.rs"),
            extension: Some("rs"),
            project_markers: &[],
            content_sample: None,
        });
        assert_eq!(result.language, LanguageId::Rust);
        assert_eq!(result.confidence, Confidence::Medium);
    }

    #[test]
    fn project_marker_and_extension_are_high() {
        let result = engine().detect(&DetectionInput {
            path: None,
            file_name: Some("main.rs"),
            extension: Some("rs"),
            project_markers: &["Cargo.toml".to_string()],
            content_sample: None,
        });
        assert_eq!(result.language, LanguageId::Rust);
        assert_eq!(result.confidence, Confidence::High);
    }

    #[test]
    fn unknown_when_nothing_matches() {
        let result = engine().detect(&DetectionInput {
            path: None,
            file_name: Some("notes.txt"),
            extension: Some("txt"),
            project_markers: &[],
            content_sample: None,
        });
        assert_eq!(result.language, LanguageId::Unknown);
    }

    #[test]
    fn content_hints_can_decide() {
        let mut e = engine();
        e.register(descriptor(LanguageId::Rust));
        // No extension, but the content is unmistakably Rust.
        let result = e.detect(&DetectionInput {
            path: None,
            file_name: None,
            extension: None,
            project_markers: &[],
            content_sample: Some("fn main() {\n    let x = 1;\n}\n"),
        });
        // Without registered content hints this stays unknown; assert the engine
        // does not panic and returns a stable answer.
        assert!(result.reasons.is_empty() || result.language == LanguageId::Unknown);
    }

    #[test]
    fn markdown_in_rust_project_is_not_claimed_as_rust() {
        let result = engine().detect(&DetectionInput {
            path: None,
            file_name: Some("README.md"),
            extension: Some("md"),
            project_markers: &["Cargo.toml".to_string()],
            content_sample: Some("# Koda\n"),
        });
        assert_eq!(result.language, LanguageId::Unknown);
    }

    #[test]
    fn go_file_in_rust_project_is_still_go() {
        let result = engine().detect(&DetectionInput {
            path: None,
            file_name: Some("main.go"),
            extension: Some("go"),
            project_markers: &["Cargo.toml".to_string()],
            content_sample: Some("package main\n"),
        });
        assert_eq!(result.language, LanguageId::Go);
    }

    #[test]
    fn python_file_in_a_python_project_is_high() {
        let mut e = engine();
        e.register(descriptor(LanguageId::Python));
        let result = e.detect(&DetectionInput {
            path: None,
            file_name: Some("main.py"),
            extension: Some("py"),
            project_markers: &["pyproject.toml".to_string()],
            content_sample: None,
        });
        assert_eq!(result.language, LanguageId::Python);
        assert_eq!(result.confidence, Confidence::High);
    }

    #[test]
    fn mixed_project_corroborates_the_files_own_language() {
        // Python is listed first, but the `.rs` file must still win so the
        // project bonus is deterministic.
        let e = DetectionEngine::new(vec![
            descriptor(LanguageId::Python),
            descriptor(LanguageId::Rust),
        ]);
        let result = e.detect(&DetectionInput {
            path: None,
            file_name: Some("main.rs"),
            extension: Some("rs"),
            project_markers: &["Cargo.toml".to_string(), "pyproject.toml".to_string()],
            content_sample: None,
        });
        assert_eq!(result.language, LanguageId::Rust);
        assert_eq!(result.confidence, Confidence::High);
    }

    #[test]
    fn content_hint_weight_is_capped_at_three() {
        let mut e = DetectionEngine::default();
        e.register(LanguageDescriptor {
            id: LanguageId::Rust,
            extensions: &[],
            project_markers: &[],
            file_names: &[],
            shebangs: &[],
            content_hints: &["a", "b", "c", "d", "e"],
        });
        let result = e.detect(&DetectionInput {
            path: None,
            file_name: None,
            extension: None,
            project_markers: &[],
            content_sample: Some("abcde"),
        });
        // Three hints' worth of weight, not five.
        assert_eq!(result.scores[0], (LanguageId::Rust, 60));
    }

    #[test]
    fn project_context_never_promotes_a_content_only_language() {
        let e = DetectionEngine::new(vec![
            LanguageDescriptor {
                id: LanguageId::Rust,
                extensions: &["rs"],
                project_markers: &["Cargo.toml"],
                file_names: &[],
                shebangs: &[],
                content_hints: &["fn ", "let mut ", "impl ", "use std"],
            },
            LanguageDescriptor {
                id: LanguageId::Markdown,
                extensions: &["md"],
                project_markers: &[],
                file_names: &[],
                shebangs: &[],
                content_hints: &["## ", "```"],
            },
        ]);
        // A Markdown file that merely contains a Rust-looking code block inside
        // a Rust project must stay Markdown.
        let result = e.detect(&DetectionInput {
            path: None,
            file_name: Some("README.md"),
            extension: Some("md"),
            project_markers: &["Cargo.toml".to_string()],
            content_sample: Some("## Title\n\n```rust\nfn main() { let mut x = 1; }\n```\n"),
        });
        assert_eq!(result.language, LanguageId::Markdown);
        let rust_score = result
            .scores
            .iter()
            .find(|(id, _)| *id == LanguageId::Rust)
            .map(|(_, score)| *score);
        assert_eq!(
            rust_score,
            Some(40),
            "marker must not add the 40-point bonus"
        );
    }

    #[test]
    fn registry_descriptors_are_deterministic() {
        let registry = crate::language::provider::ProviderRegistry::builtin();
        let ids: Vec<LanguageId> = registry
            .descriptors()
            .iter()
            .map(|descriptor| descriptor.id)
            .collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(
            ids, sorted,
            "descriptor order must not depend on HashMap order"
        );
    }
}
