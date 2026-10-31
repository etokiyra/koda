//! Per-language support class, surfaced in-app.
//!
//! The authoritative source is `docs/LANGUAGE-SUPPORT.md`; this table mirrors
//! it so Koda can answer "what can I realistically expect for this language?"
//! without a second hand-written document in the UI.

use crate::language::id::LanguageId;

/// The support class for a language, as defined in `docs/LANGUAGE-SUPPORT.md`.
pub fn class(language: LanguageId) -> &'static str {
    match language {
        LanguageId::Rust
        | LanguageId::Go
        | LanguageId::Shell
        | LanguageId::TypeScript
        | LanguageId::JavaScript
        | LanguageId::CSharp
        | LanguageId::Kotlin
        | LanguageId::Html
        | LanguageId::Css
        | LanguageId::Lua
        | LanguageId::Sql
        | LanguageId::Dart => "Complete",
        LanguageId::Python
        | LanguageId::C
        | LanguageId::Cpp
        | LanguageId::Java
        | LanguageId::Php
        | LanguageId::Ruby
        | LanguageId::Perl
        | LanguageId::Nix => "Complete (prerequisite)",
        LanguageId::Markdown | LanguageId::Json | LanguageId::Toml | LanguageId::Yaml => {
            "Offline only"
        }
        LanguageId::Assembly | LanguageId::Elixir | LanguageId::Swift => "Platform-limited",
        LanguageId::Unknown => "—",
    }
}

/// Every language Koda knows, with its support class.
pub fn all() -> Vec<(LanguageId, &'static str)> {
    LanguageId::ALL
        .iter()
        .copied()
        .map(|language| (language, class(language)))
        .collect()
}
