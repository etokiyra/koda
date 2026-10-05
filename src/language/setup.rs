//! Planning "Set up this project": what each detected language needs.
//!
//! The planner decides *what* is needed; the existing provisioning subsystem
//! decides *how* to install it. Nothing in this module downloads anything, and
//! the tool state it consults is behind [`ToolState`] so planning is
//! deterministic and testable without a particular host.

use crate::language::id::LanguageId;
use crate::language::tools::{Tool, ToolPurpose, ToolRegistry};

/// The subset of tool state the planner needs.
///
/// Implemented for the real [`ToolRegistry`]; tests substitute a fake so the
/// resulting plan does not depend on what happens to be installed on the host.
pub trait ToolState {
    /// Whether the tool is installed and usable.
    fn available(&self, tool: Tool) -> bool;
    /// Whether Koda could install the tool here.
    fn installable(&self, tool: Tool) -> bool;
    /// The system prerequisites the tool needs that are missing.
    fn missing_prerequisites(&self, tool: Tool) -> Vec<String>;
    /// A precise, user-facing reason the tool is not usable.
    fn reason(&self, tool: Tool) -> String;
}

impl ToolState for ToolRegistry {
    fn available(&self, tool: Tool) -> bool {
        ToolRegistry::available(self, tool)
    }

    fn installable(&self, tool: Tool) -> bool {
        crate::language::tools::can_install(tool)
    }

    fn missing_prerequisites(&self, tool: Tool) -> Vec<String> {
        tool.missing_prerequisites()
            .iter()
            .map(|program| (*program).to_string())
            .collect()
    }

    fn reason(&self, tool: Tool) -> String {
        tool.setup_reason()
    }
}

/// What setup needs for one language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupState {
    /// The server is available, or the language needs none.
    Ready,
    /// Koda can install this managed server.
    NeedsInstall(Tool),
    /// A system prerequisite is missing, so Koda will not install anything.
    Prerequisite(String),
    /// No supported install path on this platform.
    Unavailable(String),
}

/// One language's place in the setup plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageSetup {
    pub language: LanguageId,
    pub state: SetupState,
    /// A one-line explanation for the UI.
    pub detail: String,
}

/// The setup plan for a project.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectSetupPlan {
    pub languages: Vec<LanguageSetup>,
}

impl ProjectSetupPlan {
    pub fn is_empty(&self) -> bool {
        self.languages.is_empty()
    }

    /// Managed tools that still need installing, deduplicated in a stable order.
    ///
    /// This is the only list setup execution acts on, so an already-available
    /// server is never reinstalled.
    pub fn installable_tools(&self) -> Vec<Tool> {
        let mut tools: Vec<Tool> = Vec::new();
        for entry in &self.languages {
            if let SetupState::NeedsInstall(tool) = entry.state
                && !tools.contains(&tool)
            {
                tools.push(tool);
            }
        }
        tools
    }

    /// Whether anything at all needs managed tooling.
    pub fn needs_setup(&self) -> bool {
        !self.installable_tools().is_empty()
    }

    pub fn ready_count(&self) -> usize {
        self.languages
            .iter()
            .filter(|entry| entry.state == SetupState::Ready)
            .count()
    }

    /// Languages that need the user's attention (a prerequisite or a platform).
    pub fn attention_count(&self) -> usize {
        self.languages
            .iter()
            .filter(|entry| {
                matches!(
                    entry.state,
                    SetupState::Prerequisite(_) | SetupState::Unavailable(_)
                )
            })
            .count()
    }

    pub fn has_attention(&self) -> bool {
        self.attention_count() > 0
    }
}

/// Plan setup for `languages` given the current tool state.
pub fn plan_project(languages: &[LanguageId], state: &dyn ToolState) -> ProjectSetupPlan {
    ProjectSetupPlan {
        languages: languages
            .iter()
            .copied()
            .map(|language| plan_language(language, state))
            .collect(),
    }
}

/// Plan setup for one language.
pub fn plan_language(language: LanguageId, state: &dyn ToolState) -> LanguageSetup {
    let candidates: Vec<Tool> = Tool::ALL
        .iter()
        .copied()
        .filter(|tool| tool.serves(language) && tool.purpose() == ToolPurpose::LanguageServer)
        .collect();

    // No server for this language: built-in offline support is enough.
    if candidates.is_empty() {
        return LanguageSetup {
            language,
            state: SetupState::Ready,
            detail: "built-in support — no server needed".to_string(),
        };
    }

    if let Some(tool) = candidates.iter().find(|tool| state.available(**tool)) {
        return LanguageSetup {
            language,
            state: SetupState::Ready,
            detail: format!("{} is available", tool.label()),
        };
    }

    if let Some(tool) = candidates.iter().find(|tool| state.installable(**tool)) {
        return LanguageSetup {
            language,
            state: SetupState::NeedsInstall(*tool),
            detail: state.reason(*tool),
        };
    }

    // Nothing installable: prefer a precise missing-prerequisite explanation.
    if let Some(tool) = candidates
        .iter()
        .find(|tool| !state.missing_prerequisites(**tool).is_empty())
    {
        let missing = state.missing_prerequisites(*tool).join(" or ");
        return LanguageSetup {
            language,
            state: SetupState::Prerequisite(missing),
            detail: state.reason(*tool),
        };
    }

    let reason = state.reason(candidates[0]);
    LanguageSetup {
        language,
        state: SetupState::Unavailable(reason.clone()),
        detail: reason,
    }
}

/// Counts and a one-line headline for a completed setup run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupSummary {
    pub installed: usize,
    pub failed: usize,
    pub ready: usize,
    pub attention: usize,
}

impl SetupSummary {
    /// Summarize a run: `installed` tools succeeded, `failed` did not, and the
    /// plan's remaining ready/attention counts are carried through.
    pub fn from_run(plan: &ProjectSetupPlan, installed: &[Tool], failed: &[Tool]) -> Self {
        SetupSummary {
            installed: installed.len(),
            failed: failed.len(),
            ready: plan.ready_count(),
            attention: plan.attention_count(),
        }
    }

    /// Whether any part of the project still needs the user's attention.
    pub fn has_outstanding(&self) -> bool {
        self.failed > 0 || self.attention > 0
    }

    /// A concise, honest one-line summary for the status bar and a toast.
    pub fn headline(&self) -> String {
        let mut parts = Vec::new();
        if self.installed > 0 {
            parts.push(format!("{} installed", self.installed));
        }
        if self.ready > 0 {
            parts.push(format!("{} ready", self.ready));
        }
        if self.failed > 0 {
            parts.push(format!("{} failed", self.failed));
        }
        if self.attention > 0 {
            parts.push(format!("{} need attention", self.attention));
        }
        if parts.is_empty() {
            "Project setup complete".to_string()
        } else {
            format!("Project setup — {}", parts.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// A deterministic stand-in for the host's tool state.
    struct Fake {
        available: Vec<Tool>,
        installable: Vec<Tool>,
        missing: Vec<(Tool, &'static str)>,
    }

    impl Fake {
        fn new() -> Self {
            Fake {
                available: Vec::new(),
                installable: Vec::new(),
                missing: Vec::new(),
            }
        }
    }

    impl ToolState for Fake {
        fn available(&self, tool: Tool) -> bool {
            self.available.contains(&tool)
        }
        fn installable(&self, tool: Tool) -> bool {
            self.installable.contains(&tool)
        }
        fn missing_prerequisites(&self, tool: Tool) -> Vec<String> {
            self.missing
                .iter()
                .filter(|(candidate, _)| *candidate == tool)
                .map(|(_, program)| (*program).to_string())
                .collect()
        }
        fn reason(&self, tool: Tool) -> String {
            format!("{} is not available here", tool.label())
        }
    }

    #[test]
    fn an_available_server_is_ready() {
        let state = Fake {
            available: vec![Tool::RustAnalyzer],
            ..Fake::new()
        };
        let setup = plan_language(LanguageId::Rust, &state);
        assert_eq!(setup.state, SetupState::Ready);
    }

    #[test]
    fn a_server_without_a_server_is_ready() {
        // Markdown/JSON/TOML/YAML have no language server.
        let state = Fake::new();
        for language in [
            LanguageId::Markdown,
            LanguageId::Json,
            LanguageId::Toml,
            LanguageId::Yaml,
        ] {
            assert_eq!(plan_language(language, &state).state, SetupState::Ready);
        }
    }

    #[test]
    fn a_missing_but_installable_server_needs_installation() {
        let state = Fake {
            installable: vec![Tool::Gopls],
            ..Fake::new()
        };
        assert_eq!(
            plan_language(LanguageId::Go, &state).state,
            SetupState::NeedsInstall(Tool::Gopls)
        );
    }

    #[test]
    fn a_missing_prerequisite_is_reported_as_such() {
        let state = Fake {
            missing: vec![(Tool::Phpactor, "php")],
            ..Fake::new()
        };
        assert_eq!(
            plan_language(LanguageId::Php, &state).state,
            SetupState::Prerequisite("php".to_string())
        );
    }

    #[test]
    fn an_unavailable_platform_is_reported_as_unavailable() {
        let state = Fake::new();
        assert!(matches!(
            plan_language(LanguageId::Swift, &state).state,
            SetupState::Unavailable(_)
        ));
    }

    #[test]
    fn a_mixed_project_keeps_each_language_distinct() {
        let state = Fake {
            available: vec![Tool::RustAnalyzer],
            installable: vec![Tool::TypeScriptLs],
            missing: vec![(Tool::Phpactor, "php")],
        };
        let plan = plan_project(
            &[
                LanguageId::Rust,
                LanguageId::TypeScript,
                LanguageId::Php,
                LanguageId::Markdown,
            ],
            &state,
        );
        assert!(plan.needs_setup());
        assert_eq!(plan.installable_tools(), vec![Tool::TypeScriptLs]);
        assert_eq!(plan.ready_count(), 2); // Rust + Markdown
        assert_eq!(plan.attention_count(), 1); // PHP
        assert!(plan.has_attention());
    }

    #[test]
    fn duplicate_languages_and_tools_are_deduplicated() {
        let state = Fake {
            installable: vec![Tool::TypeScriptLs],
            ..Fake::new()
        };
        // JavaScript is served by the TypeScript server, so both map to one tool.
        let plan = plan_project(
            &[
                LanguageId::TypeScript,
                LanguageId::JavaScript,
                LanguageId::TypeScript,
            ],
            &state,
        );
        assert_eq!(plan.installable_tools(), vec![Tool::TypeScriptLs]);
        assert_eq!(plan.languages.len(), 3);
    }

    #[test]
    fn an_empty_project_needs_nothing() {
        let plan = plan_project(&[], &Fake::new());
        assert!(plan.is_empty());
        assert!(!plan.needs_setup());
        assert!(!plan.has_attention());
    }

    #[test]
    fn summary_reports_partial_success_honestly() {
        let state = Fake {
            installable: vec![Tool::Gopls],
            missing: vec![(Tool::Phpactor, "php")],
            ..Fake::new()
        };
        let plan = plan_project(
            &[LanguageId::Go, LanguageId::Php, LanguageId::Markdown],
            &state,
        );
        let summary = SetupSummary::from_run(&plan, &[Tool::Gopls], &[]);
        assert_eq!(
            summary,
            SetupSummary {
                installed: 1,
                failed: 0,
                ready: 1,
                attention: 1,
            }
        );
        assert!(summary.has_outstanding()); // PHP still needs attention
        let headline = summary.headline();
        assert!(headline.contains("1 installed"), "{headline}");
        assert!(headline.contains("1 need attention"), "{headline}");

        let failed = SetupSummary::from_run(&plan, &[], &[Tool::Gopls]);
        assert!(failed.headline().contains("1 failed"));
        assert!(failed.has_outstanding());
    }

    #[test]
    fn a_shared_server_is_planned_once() {
        // Sanity: the tool list has no duplicate entries for a shared server.
        let state = Fake {
            installable: vec![Tool::TypeScriptLs],
            ..Fake::new()
        };
        let plan = plan_project(&[LanguageId::TypeScript, LanguageId::JavaScript], &state);
        let unique: HashSet<Tool> = plan.installable_tools().into_iter().collect();
        assert_eq!(unique.len(), 1);
    }
}
