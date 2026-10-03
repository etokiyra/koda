//! Language identifiers.
//!
//! A [`LanguageId`] is a stable, cheap handle for "which language is this?".
//! It is deliberately separate from the provider implementation: detection decides
//! *what* a file is, providers decide *how* to support it.

use std::fmt;

/// A supported (or unknown) language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum LanguageId {
    Rust,
    Go,
    /// Markdown prose and documentation.
    Markdown,
    /// JSON (and its JSONC/GeoJSON relatives).
    Json,
    /// TOML configuration.
    Toml,
    /// YAML configuration.
    Yaml,
    /// Used when detection could not reach a confident answer.
    #[default]
    Unknown,
}

impl LanguageId {
    /// Every language Koda understands today. Keep this in sync with [`crate::language::provider`].
    pub const ALL: [LanguageId; 6] = [
        LanguageId::Rust,
        LanguageId::Go,
        LanguageId::Markdown,
        LanguageId::Json,
        LanguageId::Toml,
        LanguageId::Yaml,
    ];

    /// A human readable display name.
    pub fn name(self) -> &'static str {
        match self {
            LanguageId::Rust => "Rust",
            LanguageId::Go => "Go",
            LanguageId::Markdown => "Markdown",
            LanguageId::Json => "JSON",
            LanguageId::Toml => "TOML",
            LanguageId::Yaml => "YAML",
            LanguageId::Unknown => "Plain Text",
        }
    }

    /// The lowercase machine name, useful for logs and serialization.
    pub fn slug(self) -> &'static str {
        match self {
            LanguageId::Rust => "rust",
            LanguageId::Go => "go",
            LanguageId::Markdown => "markdown",
            LanguageId::Json => "json",
            LanguageId::Toml => "toml",
            LanguageId::Yaml => "yaml",
            LanguageId::Unknown => "text",
        }
    }
}

impl fmt::Display for LanguageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
