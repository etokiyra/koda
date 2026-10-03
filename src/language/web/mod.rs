//! TypeScript and JavaScript provider.
//!
//! One scanner serves both languages because their surface syntax overlaps
//! almost entirely; the two differ in a handful of keywords and types and in
//! which files they claim. Like the other providers this is entirely built-in
//! and offline: highlighting, structural diagnostics, symbols, completion,
//! hover and within-file navigation. `typescript-language-server` can be
//! provisioned for rename, code actions and richer, type-aware analysis.
//!
//! Multi-line block comments carry over between lines; a template literal that
//! spans lines is highlighted only up to the end of the line it starts on,
//! which keeps the highlighter stateless beyond the existing comment flag.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_number, scan_quoted};
use crate::language::detection::LanguageDescriptor;
use crate::language::diagnostics::Diagnostic;
use crate::language::format::FormatOutcome;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Symbol, SymbolKind};

use std::path::Path;

/// Keywords common to JavaScript and TypeScript.
const KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "asserts",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "declare",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "finally",
    "for",
    "from",
    "function",
    "get",
    "if",
    "implements",
    "import",
    "in",
    "infer",
    "instanceof",
    "interface",
    "is",
    "keyof",
    "let",
    "namespace",
    "new",
    "of",
    "override",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "satisfies",
    "set",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "try",
    "type",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// TypeScript primitive and utility types.
const TYPES: &[&str] = &[
    "any",
    "bigint",
    "boolean",
    "never",
    "number",
    "object",
    "string",
    "symbol",
    "undefined",
    "unknown",
    "Array",
    "ReadonlyArray",
    "Promise",
    "Record",
    "Partial",
    "Required",
    "Readonly",
    "Pick",
    "Omit",
    "Exclude",
    "Extract",
    "ReturnType",
    "Parameters",
];

const BUILTINS: &[&str] = &[
    "Array",
    "BigInt",
    "Boolean",
    "Date",
    "Error",
    "Function",
    "JSON",
    "Map",
    "Math",
    "Number",
    "Object",
    "Promise",
    "RegExp",
    "Set",
    "String",
    "Symbol",
    "WeakMap",
    "clearInterval",
    "clearTimeout",
    "console",
    "decodeURIComponent",
    "document",
    "encodeURIComponent",
    "exports",
    "fetch",
    "globalThis",
    "isFinite",
    "isNaN",
    "module",
    "parseFloat",
    "parseInt",
    "process",
    "queueMicrotask",
    "require",
    "setInterval",
    "setTimeout",
    "structuredClone",
    "window",
];

const CONSTANTS: &[&str] = &["true", "false", "null", "undefined", "NaN", "Infinity"];

/// A provider instance for one of the two web languages.
pub struct WebProvider {
    id: LanguageId,
}

impl WebProvider {
    pub fn typescript() -> Self {
        WebProvider {
            id: LanguageId::TypeScript,
        }
    }

    pub fn javascript() -> Self {
        WebProvider {
            id: LanguageId::JavaScript,
        }
    }
}

impl LanguageProvider for WebProvider {
    fn id(&self) -> LanguageId {
        self.id
    }

    fn display_name(&self) -> &'static str {
        self.id.name()
    }

    fn descriptor(&self) -> LanguageDescriptor {
        match self.id {
            LanguageId::TypeScript => LanguageDescriptor {
                id: LanguageId::TypeScript,
                extensions: &["ts", "tsx", "mts", "cts"],
                project_markers: &["tsconfig.json", "package.json"],
                file_names: &[],
                shebangs: &[],
                // TypeScript is a superset of JavaScript, so it shares the
                // generic hints and adds its own; otherwise a `.tsx` file with
                // only JS-shaped content would lose to the JavaScript provider.
                content_hints: &[
                    "interface ",
                    "type ",
                    ": string",
                    ": number",
                    "=>",
                    "const ",
                ],
            },
            _ => LanguageDescriptor {
                id: LanguageId::JavaScript,
                extensions: &["js", "jsx", "mjs", "cjs"],
                project_markers: &["package.json"],
                file_names: &[],
                shebangs: &["node"],
                content_hints: &["const ", "=>", "console.log", "function "],
            },
        }
    }

    fn capabilities(&self) -> &'static [Capability] {
        &[
            Capability::SyntaxHighlighting,
            Capability::Diagnostics,
            Capability::DocumentSymbols,
            Capability::GotoDefinition,
            Capability::GotoReference,
            Capability::Completion,
            Capability::Hover,
        ]
    }

    fn diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        crate::language::diagnostics::check_delimiters(self, text)
    }

    fn symbols(&self, text: &str) -> Vec<Symbol> {
        web_symbols(text)
    }

    fn definition(
        &self,
        text: &str,
        line: usize,
        col: usize,
    ) -> Option<crate::language::symbols::Symbol> {
        let word = crate::language::symbols::word_at(text, line, col)?;
        web_symbols(text)
            .into_iter()
            .find(|symbol| symbol.name == word)
    }

    fn references(
        &self,
        text: &str,
        line: usize,
        col: usize,
    ) -> Vec<crate::language::symbols::Location> {
        match crate::language::symbols::word_at(text, line, col) {
            Some(word) => crate::language::symbols::locations_of_word(text, &word),
            None => Vec::new(),
        }
    }

    fn completions(&self, _text: &str, _line: usize, _col: usize) -> Vec<Completion> {
        let mut completions: Vec<Completion> = KEYWORDS
            .iter()
            .map(|word| Completion::new(*word, CompletionKind::Keyword))
            .collect();
        completions.extend(
            TYPES
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Type)),
        );
        completions.extend(
            BUILTINS
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Function)),
        );
        completions.extend(
            CONSTANTS
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Constant)),
        );
        completions
    }

    fn format(&self, _path: &Path, _text: &str) -> FormatOutcome {
        FormatOutcome::Unsupported
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = web_symbols(text);
        crate::language::hover::describe(text, line, col, &symbols)
    }

    fn line_comment(&self) -> &'static str {
        "//"
    }

    fn highlight(&self, line: &str, state: HighlightState) -> (Vec<HighlightSpan>, HighlightState) {
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let mut spans = Vec::new();
        let mut i = 0;

        // Finish a block comment carried over from the previous line.
        if state.in_block_comment {
            match find_block_end(&chars, 0) {
                Some(end) => {
                    push_merged(&mut spans, HighlightSpan::new(0, end, TokenKind::Comment));
                    i = end;
                }
                None => {
                    if len > 0 {
                        push_merged(&mut spans, HighlightSpan::new(0, len, TokenKind::Comment));
                    }
                    return (spans, HighlightState::default());
                }
            }
        }

        let first_nonspace = chars.iter().position(|c| !c.is_whitespace());

        while i < len {
            let c = chars[i];

            if c == '/' && chars.get(i + 1) == Some(&'/') {
                push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                break;
            }

            if c == '/' && chars.get(i + 1) == Some(&'*') {
                match find_block_end(&chars, i + 2) {
                    Some(end) => {
                        push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Comment));
                        i = end;
                    }
                    None => {
                        push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                        return (
                            spans,
                            HighlightState {
                                in_block_comment: true,
                            },
                        );
                    }
                }
                continue;
            }

            // A regex literal is not distinguished from division; leave `/` as an
            // operator so paths and math read correctly.
            if c == '\'' || c == '"' {
                let end = scan_quoted(&chars, i, c);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            if c == '`' {
                let end = scan_template(&chars, i);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            if c.is_ascii_digit() {
                let end = scan_number(&chars, i);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Number));
                i = end;
                continue;
            }

            // A decorator only appears at the start of a line.
            if c == '@' && Some(i) == first_nonspace {
                let mut j = i + 1;
                while j < len && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '.')
                {
                    j += 1;
                }
                push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Attribute));
                i = j;
                continue;
            }

            if is_ident_start(c) {
                let mut j = i;
                while j < len && is_ident_continue(chars[j]) {
                    j += 1;
                }
                let word: String = chars[i..j].iter().collect();
                push_merged(&mut spans, HighlightSpan::new(i, j, classify_word(&word)));

                // `function name` / `class Name`: highlight the name too.
                let name_kind = match word.as_str() {
                    "function" => Some(TokenKind::Function),
                    "class" => Some(TokenKind::Type),
                    _ => None,
                };
                if let Some(kind) = name_kind
                    && let Some((start, end)) = name_after(&chars, j)
                {
                    push_merged(&mut spans, HighlightSpan::new(start, end, kind));
                    i = end;
                    continue;
                }
                i = j;
                continue;
            }

            if is_operator(c) {
                push_merged(
                    &mut spans,
                    HighlightSpan::new(i, i + 1, TokenKind::Operator),
                );
            }
            i += 1;
        }

        (spans, HighlightState::default())
    }
}

fn classify_word(word: &str) -> TokenKind {
    if KEYWORDS.contains(&word) {
        TokenKind::Keyword
    } else if CONSTANTS.contains(&word) {
        TokenKind::Constant
    } else if TYPES.contains(&word) {
        TokenKind::Type
    } else if BUILTINS.contains(&word) {
        TokenKind::Function
    } else {
        TokenKind::Plain
    }
}

/// The identifier following a `function`/`class`, skipping spaces and a
/// generator `*`.
fn name_after(chars: &[char], from: usize) -> Option<(usize, usize)> {
    let mut j = from;
    while j < chars.len() && chars[j] == ' ' {
        j += 1;
    }
    if chars.get(j) == Some(&'*') {
        j += 1;
        while j < chars.len() && chars[j] == ' ' {
            j += 1;
        }
    }
    let start = j;
    if start >= chars.len() || !is_ident_start(chars[start]) {
        return None;
    }
    let mut end = start;
    while end < chars.len() && is_ident_continue(chars[end]) {
        end += 1;
    }
    Some((start, end))
}

/// End of a `*/` run starting at `from` (past the opening `/*`).
fn find_block_end(chars: &[char], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < chars.len() {
        if chars[i] == '*' && chars[i + 1] == '/' {
            return Some(i + 2);
        }
        i += 1;
    }
    None
}

/// End of a template literal starting at `start` (the opening backtick).
///
/// Escaped backticks are honoured; the literal is assumed to end on the line.
fn scan_template(chars: &[char], start: usize) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '`' => return i + 1,
            _ => i += 1,
        }
    }
    chars.len()
}

/// Words that may precede a declaration keyword.
const MODIFIERS: &[&str] = &[
    "export",
    "default",
    "async",
    "declare",
    "abstract",
    "public",
    "private",
    "protected",
    "static",
    "readonly",
    "override",
];

/// Named top-level declarations in a document.
fn web_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = skip_ws(&chars, 0);
        // Skip leading modifiers so `export default class` still registers.
        loop {
            let (word, next) = word_at(&chars, i);
            if MODIFIERS.contains(&word.as_str()) {
                i = skip_ws(&chars, next);
            } else {
                break;
            }
        }
        let (keyword, next) = word_at(&chars, i);
        let kind = match keyword.as_str() {
            "function" => SymbolKind::Function,
            "class" => SymbolKind::Type,
            "interface" => SymbolKind::Interface,
            "enum" => SymbolKind::Enum,
            "type" => SymbolKind::Type,
            "namespace" | "module" => SymbolKind::Module,
            "const" => SymbolKind::Constant,
            "let" | "var" => SymbolKind::Variable,
            _ => continue,
        };
        let mut j = skip_ws(&chars, next);
        if keyword == "function" && chars.get(j) == Some(&'*') {
            j = skip_ws(&chars, j + 1);
        }
        let (name, _) = word_at(&chars, j);
        if name.is_empty() {
            continue;
        }
        symbols.push(Symbol::new(name, kind, row, j));
    }
    symbols
}

fn skip_ws(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

/// The identifier starting at `i` and the index just past it.
fn word_at(chars: &[char], i: usize) -> (String, usize) {
    if i >= chars.len() || !is_ident_start(chars[i]) {
        return (String::new(), i);
    }
    let mut end = i;
    while end < chars.len() && is_ident_continue(chars[end]) {
        end += 1;
    }
    (chars[i..end].iter().collect(), end)
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphanumeric()
}

fn is_operator(c: char) -> bool {
    matches!(
        c,
        '+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | '~' | '?' | ':'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_claim_the_right_extensions() {
        let ts = WebProvider::typescript().descriptor();
        assert_eq!(ts.id, LanguageId::TypeScript);
        assert!(ts.extensions.contains(&"ts") && ts.extensions.contains(&"tsx"));
        let js = WebProvider::javascript().descriptor();
        assert_eq!(js.id, LanguageId::JavaScript);
        assert!(js.extensions.contains(&"js") && js.extensions.contains(&"jsx"));
    }

    #[test]
    fn highlights_comments_strings_and_keywords() {
        let provider = WebProvider::typescript();
        let (spans, state) =
            provider.highlight("const name = \"koda\"; // hi", HighlightState::default());
        assert!(!state.in_block_comment);
        let kind_at = |col: usize| {
            spans
                .iter()
                .find(|span| span.range.contains(&col))
                .map(|span| span.kind)
        };
        assert_eq!(kind_at(0), Some(TokenKind::Keyword)); // const
        assert_eq!(kind_at(13), Some(TokenKind::String)); // "koda"
        assert_eq!(kind_at(21), Some(TokenKind::Comment)); // // hi
    }

    #[test]
    fn carries_block_comments_across_lines() {
        let provider = WebProvider::typescript();
        let (_, state) = provider.highlight("/* open", HighlightState::default());
        assert!(state.in_block_comment);
        let (spans, state) = provider.highlight(" still open */ let x = 1;", state);
        assert!(!state.in_block_comment);
        assert!(
            spans
                .iter()
                .any(|span| span.kind == TokenKind::Comment && span.range.contains(&0))
        );
    }

    #[test]
    fn extracts_declarations() {
        let text = "import x from \"y\";\n\
                    export async function run() {}\n\
                    export default class Server {}\n\
                    interface Options {}\n\
                    type Handler = () => void;\n\
                    const PORT = 8080;\n";
        let symbols = web_symbols(text);
        let names: Vec<_> = symbols
            .iter()
            .map(|symbol| (symbol.name.as_str(), symbol.kind))
            .collect();
        assert!(names.contains(&("run", SymbolKind::Function)));
        assert!(names.contains(&("Server", SymbolKind::Type)));
        assert!(names.contains(&("Options", SymbolKind::Interface)));
        assert!(names.contains(&("Handler", SymbolKind::Type)));
        assert!(names.contains(&("PORT", SymbolKind::Constant)));
    }
}
