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
    /// Python source and stubs.
    Python,
    /// Shell scripts (bash, zsh, POSIX sh).
    Shell,
    /// Markdown prose and documentation.
    Markdown,
    /// JSON (and its JSONC/GeoJSON relatives).
    Json,
    /// TOML configuration.
    Toml,
    /// YAML configuration.
    Yaml,
    /// TypeScript source (and `.tsx`).
    TypeScript,
    /// JavaScript source (and `.jsx`), including ES modules.
    JavaScript,
    /// C source and headers.
    C,
    /// C++ source and headers.
    Cpp,
    /// Java sources.
    Java,
    /// C# sources.
    CSharp,
    /// PHP sources and templates.
    Php,
    /// HTML documents.
    Html,
    /// CSS stylesheets.
    Css,
    /// Lua scripts.
    Lua,
    /// Kotlin sources and scripts.
    Kotlin,
    /// SQL scripts and migrations (dialect-neutral baseline).
    Sql,
    /// Ruby sources and build files.
    Ruby,
    /// Assembly (x86/x86-64 and AArch64 baseline).
    Assembly,
    /// Perl scripts, modules and tests.
    Perl,
    /// Dart and Flutter sources.
    Dart,
    /// Elixir sources and scripts.
    Elixir,
    /// Swift sources.
    Swift,
    /// Used when detection could not reach a confident answer.
    #[default]
    Unknown,
}

impl LanguageId {
    /// Every language Koda understands today. Keep this in sync with [`crate::language::provider`].
    pub const ALL: [LanguageId; 26] = [
        LanguageId::Rust,
        LanguageId::Go,
        LanguageId::Python,
        LanguageId::Shell,
        LanguageId::Markdown,
        LanguageId::Json,
        LanguageId::Toml,
        LanguageId::Yaml,
        LanguageId::TypeScript,
        LanguageId::JavaScript,
        LanguageId::C,
        LanguageId::Cpp,
        LanguageId::Java,
        LanguageId::CSharp,
        LanguageId::Php,
        LanguageId::Html,
        LanguageId::Css,
        LanguageId::Lua,
        LanguageId::Kotlin,
        LanguageId::Sql,
        LanguageId::Ruby,
        LanguageId::Assembly,
        LanguageId::Perl,
        LanguageId::Dart,
        LanguageId::Elixir,
        LanguageId::Swift,
    ];

    /// A human readable display name.
    pub fn name(self) -> &'static str {
        match self {
            LanguageId::Rust => "Rust",
            LanguageId::Go => "Go",
            LanguageId::Python => "Python",
            LanguageId::Shell => "Shell",
            LanguageId::Markdown => "Markdown",
            LanguageId::Json => "JSON",
            LanguageId::Toml => "TOML",
            LanguageId::Yaml => "YAML",
            LanguageId::TypeScript => "TypeScript",
            LanguageId::JavaScript => "JavaScript",
            LanguageId::C => "C",
            LanguageId::Cpp => "C++",
            LanguageId::Java => "Java",
            LanguageId::CSharp => "C#",
            LanguageId::Php => "PHP",
            LanguageId::Html => "HTML",
            LanguageId::Css => "CSS",
            LanguageId::Lua => "Lua",
            LanguageId::Kotlin => "Kotlin",
            LanguageId::Sql => "SQL",
            LanguageId::Ruby => "Ruby",
            LanguageId::Assembly => "Assembly",
            LanguageId::Perl => "Perl",
            LanguageId::Dart => "Dart",
            LanguageId::Elixir => "Elixir",
            LanguageId::Swift => "Swift",
            LanguageId::Unknown => "Plain Text",
        }
    }

    /// The lowercase machine name, useful for logs and serialization.
    pub fn slug(self) -> &'static str {
        match self {
            LanguageId::Rust => "rust",
            LanguageId::Go => "go",
            LanguageId::Python => "python",
            LanguageId::Shell => "shell",
            LanguageId::Markdown => "markdown",
            LanguageId::Json => "json",
            LanguageId::Toml => "toml",
            LanguageId::Yaml => "yaml",
            LanguageId::TypeScript => "typescript",
            LanguageId::JavaScript => "javascript",
            LanguageId::C => "c",
            LanguageId::Cpp => "cpp",
            LanguageId::Java => "java",
            LanguageId::CSharp => "csharp",
            LanguageId::Php => "php",
            LanguageId::Html => "html",
            LanguageId::Css => "css",
            LanguageId::Lua => "lua",
            LanguageId::Kotlin => "kotlin",
            LanguageId::Sql => "sql",
            LanguageId::Ruby => "ruby",
            LanguageId::Assembly => "assembly",
            LanguageId::Perl => "perl",
            LanguageId::Dart => "dart",
            LanguageId::Elixir => "elixir",
            LanguageId::Swift => "swift",
            LanguageId::Unknown => "text",
        }
    }
}

impl fmt::Display for LanguageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
