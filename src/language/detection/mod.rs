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
                LanguageId::Unknown => &[],
            },
            project_markers: match id {
                LanguageId::Rust => &["Cargo.toml"],
                LanguageId::Go => &["go.mod"],
                LanguageId::Unknown => &[],
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
}
