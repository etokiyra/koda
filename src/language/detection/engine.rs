//! The detection engine: combines independent signals into a language guess.

use std::path::Path;

use crate::language::detection::confidence::{Confidence, Evidence, SignalKind};
use crate::language::id::LanguageId;

/// A lightweight description of how to recognise a language.
///
/// Providers produce these; the detection engine consumes them. Adding a language
/// therefore never requires touching the engine.
#[derive(Clone, Debug)]
pub struct LanguageDescriptor {
    pub id: LanguageId,
    /// Extensions without the leading dot, lowercase (e.g. `rs`).
    pub extensions: &'static [&'static str],
    /// Files that establish project context (e.g. `Cargo.toml`).
    pub project_markers: &'static [&'static str],
    /// Special file names (e.g. `Dockerfile`).
    pub file_names: &'static [&'static str],
    /// Tokens found in a shebang line (e.g. `python`).
    pub shebangs: &'static [&'static str],
    /// Distinctive snippets that suggest this language in file content.
    pub content_hints: &'static [&'static str],
}

/// Input to a single detection pass.
#[derive(Clone, Debug, Default)]
pub struct DetectionInput<'a> {
    pub path: Option<&'a Path>,
    pub file_name: Option<&'a str>,
    pub extension: Option<&'a str>,
    /// Names of known project marker files found at the project root.
    pub project_markers: &'a [String],
    /// A small prefix of the file, used for shebang and content inspection.
    pub content_sample: Option<&'a str>,
}

/// The outcome of a detection pass.
#[derive(Clone, Debug)]
pub struct DetectionResult {
    pub language: LanguageId,
    pub confidence: Confidence,
    /// Human readable explanations, best first.
    pub reasons: Vec<String>,
    pub evidence: Vec<Evidence>,
    /// Scores per language, sorted descending.
    pub scores: Vec<(LanguageId, u32)>,
}

impl DetectionResult {
    fn unknown() -> Self {
        DetectionResult {
            language: LanguageId::Unknown,
            confidence: Confidence::Low,
            reasons: Vec::new(),
            evidence: Vec::new(),
            scores: Vec::new(),
        }
    }
}

// Signal weights.
//
// File-level signals decide *what a file is*. Project context is a
// corroborating bonus: a `Cargo.toml` elsewhere in the tree makes a `.rs` file
// *more* confidently Rust, but it must never turn `README.md` into Rust. This is
// why a project marker alone does not assign a file's language.
const W_FILE_NAME: u32 = 55;
const W_SHEBANG: u32 = 50;
const W_PROJECT_CONTEXT_BONUS: u32 = 40;
const W_EXTENSION: u32 = 35;
const W_CONTENT_HINT: u32 = 20;
const MAX_CONTENT_HINTS: u32 = 3;

/// The detection engine.
#[derive(Clone, Debug, Default)]
pub struct DetectionEngine {
    descriptors: Vec<LanguageDescriptor>,
}

impl DetectionEngine {
    pub fn new(descriptors: Vec<LanguageDescriptor>) -> Self {
        DetectionEngine { descriptors }
    }

    /// Add or replace the descriptor for a language.
    pub fn register(&mut self, descriptor: LanguageDescriptor) {
        self.descriptors.retain(|d| d.id != descriptor.id);
        self.descriptors.push(descriptor);
    }

    pub fn descriptors(&self) -> &[LanguageDescriptor] {
        &self.descriptors
    }

    /// Run detection, combining every available signal.
    pub fn detect(&self, input: &DetectionInput<'_>) -> DetectionResult {
        let mut evidence: Vec<Evidence> = Vec::new();

        // Derive metadata from the path when the caller did not provide it.
        let file_name = input.file_name.map(str::to_string).or_else(|| {
            input
                .path
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
        });
        let extension = input
            .extension
            .map(str::to_string)
            .or_else(|| {
                input
                    .path
                    .and_then(|p| p.extension())
                    .and_then(|s| s.to_str())
                    .map(str::to_string)
            })
            .map(|e| e.to_ascii_lowercase());

        // ---- Phase 1: file-level signals decide what this file is ----
        for descriptor in &self.descriptors {
            // 1. Special file names.
            if let Some(name) = &file_name
                && descriptor
                    .file_names
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(name))
            {
                evidence.push(Evidence {
                    language: descriptor.id,
                    kind: SignalKind::FileName,
                    weight: W_FILE_NAME,
                    reason: format!("file named `{name}`"),
                });
            }

            // 2. File extension.
            if let Some(ext) = &extension
                && descriptor
                    .extensions
                    .iter()
                    .any(|e| e.eq_ignore_ascii_case(ext))
            {
                evidence.push(Evidence {
                    language: descriptor.id,
                    kind: SignalKind::Extension,
                    weight: W_EXTENSION,
                    reason: format!("`.{ext}` extension"),
                });
            }

            // 3. Shebang and content hints both inspect the sample.
            if let Some(sample) = input.content_sample {
                if let Some(first_line) = sample.lines().next()
                    && first_line.starts_with("#!")
                    && descriptor
                        .shebangs
                        .iter()
                        .any(|token| first_line.contains(token))
                {
                    evidence.push(Evidence {
                        language: descriptor.id,
                        kind: SignalKind::Shebang,
                        weight: W_SHEBANG,
                        reason: format!("shebang `{first_line}`"),
                    });
                }

                let mut hits = 0;
                for hint in descriptor.content_hints {
                    if sample.contains(hint) {
                        hits += 1;
                        if hits > MAX_CONTENT_HINTS {
                            break;
                        }
                    }
                }
                if hits > 0 {
                    evidence.push(Evidence {
                        language: descriptor.id,
                        kind: SignalKind::Content,
                        weight: W_CONTENT_HINT * hits,
                        reason: format!(
                            "{hits} distinctive {}-like syntax signal(s)",
                            descriptor.id.name()
                        ),
                    });
                }
            }
        }

        // A project marker alone says what the *project* is, not what this file
        // is. Without any file-level signal, stay honest and return unknown.
        if evidence.is_empty() {
            return DetectionResult::unknown();
        }

        // Tally file-level scores per language.
        let mut scores: Vec<(LanguageId, u32)> = Vec::new();
        for ev in &evidence {
            match scores.iter_mut().find(|(id, _)| *id == ev.language) {
                Some((_, score)) => *score += ev.weight,
                None => scores.push((ev.language, ev.weight)),
            }
        }

        // ---- Phase 2: project context corroborates the file's own signals ----
        if let Some((project_language, project_reason)) = self.project_language(input, &scores)
            && let Some((_, score)) = scores.iter_mut().find(|(id, _)| *id == project_language)
        {
            *score += W_PROJECT_CONTEXT_BONUS;
            evidence.push(Evidence {
                language: project_language,
                kind: SignalKind::ProjectContext,
                weight: W_PROJECT_CONTEXT_BONUS,
                reason: project_reason,
            });
        }

        scores.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

        let (best_language, best_score) = scores[0];
        let second_score = scores.get(1).map(|(_, s)| *s).unwrap_or(0);
        let confidence = Confidence::from_scores(best_score, second_score);

        // Reasons, strongest evidence first, only for the winning language.
        let mut winning: Vec<&Evidence> = evidence
            .iter()
            .filter(|e| e.language == best_language)
            .collect();
        winning.sort_by_key(|e| std::cmp::Reverse(e.weight));
        let reasons = winning.iter().map(|e| e.reason.clone()).collect();

        DetectionResult {
            language: best_language,
            confidence,
            reasons,
            evidence,
            scores,
        }
    }

    /// The language implied by the project markers, if any.
    ///
    /// When several markers match (a mixed repository), prefer the language that
    /// already has file-level evidence so corroboration is deterministic and
    /// always supports the file's own nature.
    fn project_language(
        &self,
        input: &DetectionInput<'_>,
        scores: &[(LanguageId, u32)],
    ) -> Option<(LanguageId, String)> {
        let mut candidates: Vec<(LanguageId, String)> = Vec::new();
        for descriptor in &self.descriptors {
            for marker in input.project_markers {
                if descriptor
                    .project_markers
                    .iter()
                    .any(|m| m.eq_ignore_ascii_case(marker))
                {
                    candidates.push((descriptor.id, format!("`{marker}` project context")));
                    break;
                }
            }
        }
        candidates
            .iter()
            .find(|(id, _)| scores.iter().any(|(score_id, _)| score_id == id))
            .cloned()
            .or_else(|| candidates.into_iter().next())
    }
}

impl Confidence {
    /// Derive confidence from the winning score and the gap to the runner-up.
    fn from_scores(best: u32, second: u32) -> Confidence {
        let mut confidence = if best >= 70 {
            Confidence::High
        } else if best >= 35 {
            Confidence::Medium
        } else {
            Confidence::Low
        };

        // When two languages are nearly tied, be honest about the uncertainty.
        if best.saturating_sub(second) < 10 {
            confidence = match confidence {
                Confidence::High => Confidence::Medium,
                Confidence::Medium => Confidence::Low,
                Confidence::Low => Confidence::Low,
            };
        }
        confidence
    }
}
