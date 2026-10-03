//! Language provider abstraction.
//!
//! A [`LanguageProvider`] bundles everything Koda knows about supporting one
//! language: how to recognise it, how to highlight it, and which capabilities it
//! offers. Language-specific logic lives here — never in the editor core.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use crate::language::completion::Completion;
use crate::language::detection::LanguageDescriptor;
use crate::language::diagnostics::Diagnostic;
use crate::language::format::FormatOutcome;
use crate::language::hover::Hover;
use crate::language::id::LanguageId;
use crate::language::symbols::{Location, Symbol};

/// A feature a provider may offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    SyntaxHighlighting,
    Completion,
    Diagnostics,
    Hover,
    GotoDefinition,
    GotoReference,
    DocumentSymbols,
    Rename,
    Formatting,
    CodeActions,
}

impl Capability {
    pub fn label(self) -> &'static str {
        match self {
            Capability::SyntaxHighlighting => "syntax",
            Capability::Completion => "completion",
            Capability::Diagnostics => "diagnostics",
            Capability::Hover => "hover",
            Capability::GotoDefinition => "go-to-definition",
            Capability::GotoReference => "references",
            Capability::DocumentSymbols => "symbols",
            Capability::Rename => "rename",
            Capability::Formatting => "formatting",
            Capability::CodeActions => "code actions",
        }
    }
}

/// Lexical category attached to a highlighted span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Plain,
    Keyword,
    Type,
    Function,
    String,
    Number,
    Comment,
    Macro,
    Constant,
    Operator,
    Attribute,
}

/// A highlighted region, measured in **characters** relative to the line start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighlightSpan {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

impl HighlightSpan {
    pub fn new(start: usize, end: usize, kind: TokenKind) -> Self {
        HighlightSpan {
            range: start..end,
            kind,
        }
    }
}

/// Carry-over state for multi-line constructs such as block comments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HighlightState {
    pub in_block_comment: bool,
}

/// The contract every language implementation fulfils.
pub trait LanguageProvider: Send + Sync {
    fn id(&self) -> LanguageId;
    fn display_name(&self) -> &'static str;

    /// The detection descriptor for this language.
    fn descriptor(&self) -> LanguageDescriptor;

    /// Capabilities this provider currently offers.
    fn capabilities(&self) -> &'static [Capability] {
        &[Capability::SyntaxHighlighting]
    }

    /// Highlight a single line, given the state carried in from the previous line.
    ///
    /// Returns the spans for this line and the state to carry into the next one.
    fn highlight(&self, line: &str, state: HighlightState) -> (Vec<HighlightSpan>, HighlightState);

    /// Diagnostics for a whole document.
    ///
    /// Providers that cannot analyse need not override this; returning nothing
    /// simply means the provider contributes no diagnostics.
    fn diagnostics(&self, _text: &str) -> Vec<Diagnostic> {
        Vec::new()
    }

    /// Named definitions in a document, for the symbol outline.
    ///
    /// Providers without symbol support return nothing.
    fn symbols(&self, _text: &str) -> Vec<Symbol> {
        Vec::new()
    }

    /// The definition of the symbol at `(line, col)`, if the provider can find
    /// one. Built-in providers resolve within the current file.
    fn definition(&self, _text: &str, _line: usize, _col: usize) -> Option<Symbol> {
        None
    }

    /// Whole-word occurrences of the symbol at `(line, col)`.
    fn references(&self, _text: &str, _line: usize, _col: usize) -> Vec<Location> {
        Vec::new()
    }

    /// Language-specific completion candidates, such as keywords and builtins.
    ///
    /// The app merges these with identifiers from the buffer, so providers only
    /// need to contribute what is not already in the document.
    fn completions(&self, _text: &str, _line: usize, _col: usize) -> Vec<Completion> {
        Vec::new()
    }

    /// Format a document with the language's trusted formatter, if any.
    ///
    /// Providers without a formatter return [`FormatOutcome::Unsupported`].
    fn format(&self, _path: &Path, _text: &str) -> FormatOutcome {
        FormatOutcome::Unsupported
    }

    /// The external formatter executable this language prefers, if any.
    fn formatter(&self) -> Option<&'static str> {
        None
    }

    /// Information about the symbol at `(line, col)`, for the hover popup.
    fn hover(&self, _text: &str, _line: usize, _col: usize) -> Option<Hover> {
        None
    }

    /// The comment marker used by "toggle comment" and friends.
    fn line_comment(&self) -> &'static str {
        "//"
    }
}

/// A minimal provider used for files Koda does not yet recognise.
pub struct PlainTextProvider;

impl LanguageProvider for PlainTextProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Unknown
    }

    fn display_name(&self) -> &'static str {
        "Plain Text"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Unknown,
            extensions: &[],
            project_markers: &[],
            file_names: &[],
            shebangs: &[],
            content_hints: &[],
        }
    }

    fn capabilities(&self) -> &'static [Capability] {
        &[]
    }

    fn highlight(
        &self,
        _line: &str,
        state: HighlightState,
    ) -> (Vec<HighlightSpan>, HighlightState) {
        (Vec::new(), state)
    }
}

/// Owns every registered provider and resolves language ids to implementations.
pub struct ProviderRegistry {
    providers: HashMap<LanguageId, Box<dyn LanguageProvider>>,
}

impl ProviderRegistry {
    /// The registry Koda ships with. This is the single place where built-in
    /// languages are wired in.
    pub fn builtin() -> Self {
        let mut registry = ProviderRegistry {
            providers: HashMap::new(),
        };
        registry.register(Box::new(PlainTextProvider));
        registry.register(Box::new(crate::language::rust::RustProvider));
        registry.register(Box::new(crate::language::go::GoProvider));
        registry.register(Box::new(crate::language::python::PythonProvider));
        registry.register(Box::new(crate::language::shell::ShellProvider));
        registry.register(Box::new(crate::language::markdown::MarkdownProvider));
        registry.register(Box::new(crate::language::json::JsonProvider));
        registry.register(Box::new(crate::language::toml::TomlProvider));
        registry.register(Box::new(crate::language::yaml::YamlProvider));
        registry.register(Box::new(crate::language::web::WebProvider::typescript()));
        registry.register(Box::new(crate::language::web::WebProvider::javascript()));
        registry.register(Box::new(crate::language::c::CProvider::c()));
        registry.register(Box::new(crate::language::c::CProvider::cpp()));
        registry.register(Box::new(crate::language::java::JavaProvider));
        registry.register(Box::new(crate::language::csharp::CSharpProvider));
        registry.register(Box::new(crate::language::php::PhpProvider));
        registry.register(Box::new(crate::language::html::HtmlProvider));
        registry.register(Box::new(crate::language::css::CssProvider));
        registry
    }

    pub fn register(&mut self, provider: Box<dyn LanguageProvider>) {
        self.providers.insert(provider.id(), provider);
    }

    /// Resolve a language to its provider, falling back to plain text.
    pub fn get(&self, id: LanguageId) -> &dyn LanguageProvider {
        match self.providers.get(&id) {
            Some(provider) => provider.as_ref(),
            None => self
                .providers
                .get(&LanguageId::Unknown)
                .map(|p| p.as_ref())
                .expect("plain text provider must always be registered"),
        }
    }

    pub fn get_opt(&self, id: LanguageId) -> Option<&dyn LanguageProvider> {
        self.providers.get(&id).map(|p| p.as_ref())
    }

    /// Descriptors for every real provider, used to build the detection engine.
    ///
    /// Sorted by language id so detection is deterministic: the registry is a
    /// `HashMap` whose iteration order is randomised per process, which would
    /// otherwise leak into tie-breaking and the project-context bonus.
    pub fn descriptors(&self) -> Vec<LanguageDescriptor> {
        let mut descriptors: Vec<LanguageDescriptor> = self
            .providers
            .values()
            .filter(|p| p.id() != LanguageId::Unknown)
            .map(|p| p.descriptor())
            .collect();
        descriptors.sort_by_key(|descriptor| descriptor.id);
        descriptors
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn LanguageProvider> {
        self.providers.values().map(|p| p.as_ref())
    }
}
