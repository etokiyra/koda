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
    /// Used when detection could not reach a confident answer.
    #[default]
    Unknown,
}

impl LanguageId {
    /// Every language Koda understands today. Keep this in sync with [`crate::language::provider`].
    pub const ALL: [LanguageId; 2] = [LanguageId::Rust, LanguageId::Go];

    /// A human readable display name.
    pub fn name(self) -> &'static str {
        match self {
            LanguageId::Rust => "Rust",
            LanguageId::Go => "Go",
            LanguageId::Unknown => "Plain Text",
        }
    }

    /// The lowercase machine name, useful for logs and serialization.
    pub fn slug(self) -> &'static str {
        match self {
            LanguageId::Rust => "rust",
            LanguageId::Go => "go",
            LanguageId::Unknown => "text",
        }
    }
}

impl fmt::Display for LanguageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
