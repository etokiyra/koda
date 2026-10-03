//! Kotlin provider.
//!
//! Built-in and offline: `//` and `/* … */` comments, regular and raw (`"""`)
//! strings, character literals, string templates, annotations, keywords,
//! modifiers, numbers and operators. Structural diagnostics reuse the shared
//! delimiter checker. `kotlin-language-server`, run on a dedicated managed
//! JDK 21, provides the full LSP feature set.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_number, scan_quoted};
use crate::language::detection::LanguageDescriptor;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Location, Symbol, SymbolKind, word_at};

const KEYWORDS: &[&str] = &[
    "as",
    "break",
    "by",
    "catch",
    "class",
    "continue",
    "do",
    "else",
    "false",
    "finally",
    "for",
    "fun",
    "if",
    "in",
    "interface",
    "is",
    "null",
    "object",
    "package",
    "return",
    "super",
    "this",
    "throw",
    "true",
    "try",
    "typealias",
    "typeof",
    "val",
    "var",
    "when",
    "while",
];

const MODIFIERS: &[&str] = &[
    "abstract",
    "actual",
    "annotation",
    "companion",
    "const",
    "crossinline",
    "data",
    "enum",
    "expect",
    "external",
    "final",
    "infix",
    "inline",
    "inner",
    "internal",
    "lateinit",
    "noinline",
    "open",
    "operator",
    "out",
    "override",
    "private",
    "protected",
    "public",
    "reified",
    "sealed",
    "suspend",
    "tailrec",
    "vararg",
];

const CONSTANTS: &[&str] = &["false", "null", "true"];

const BUILTINS: &[&str] = &[
    "Any",
    "Array",
    "Boolean",
    "Byte",
    "Char",
    "Double",
    "Float",
    "Int",
    "List",
    "Long",
    "Map",
    "Nothing",
    "Pair",
    "Sequence",
    "Set",
    "Short",
    "String",
    "Triple",
    "Unit",
    "check",
    "error",
    "lazy",
    "let",
    "also",
    "apply",
    "run",
    "with",
    "repeat",
    "require",
    "listOf",
    "mapOf",
    "setOf",
    "println",
    "print",
    "readLine",
    "arrayOf",
    "intArrayOf",
    "mutableListOf",
];

pub struct KotlinProvider;

impl LanguageProvider for KotlinProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Kotlin
    }

    fn display_name(&self) -> &'static str {
        "Kotlin"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Kotlin,
            extensions: &["kt", "kts"],
            project_markers: &["build.gradle.kts", "settings.gradle.kts"],
            file_names: &[],
            shebangs: &[],
            content_hints: &["fun ", "val ", "package ", "import "],
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

    fn diagnostics(&self, text: &str) -> Vec<crate::language::diagnostics::Diagnostic> {
        crate::language::diagnostics::check_delimiters(self, text)
    }

    fn symbols(&self, text: &str) -> Vec<Symbol> {
        kotlin_symbols(text)
    }

    fn definition(&self, text: &str, line: usize, col: usize) -> Option<Symbol> {
        let word = word_at(text, line, col)?;
        kotlin_symbols(text)
            .into_iter()
            .find(|symbol| symbol.name == word)
    }

    fn references(&self, text: &str, line: usize, col: usize) -> Vec<Location> {
        match word_at(text, line, col) {
            Some(word) => crate::language::symbols::locations_of_word(text, &word),
            None => Vec::new(),
        }
    }

    fn completions(&self, _text: &str, _line: usize, _col: usize) -> Vec<Completion> {
        let mut completions: Vec<Completion> = KEYWORDS
            .iter()
            .chain(MODIFIERS)
            .map(|word| Completion::new(*word, CompletionKind::Keyword))
            .collect();
        completions.extend(
            BUILTINS
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Type)),
        );
        completions.extend(
            CONSTANTS
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Constant)),
        );
        completions
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = kotlin_symbols(text);
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
                    return (
                        spans,
                        HighlightState {
                            in_block_comment: true,
                        },
                    );
                }
            }
        }

        while i < len {
            let c = chars[i];
            let next = chars.get(i + 1).copied();

            if c == '/' && next == Some('/') {
                push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                break;
            }
            if c == '/' && next == Some('*') {
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

            // Raw strings `""" … """` (single-line approximation; the LSP owns
            // multi-line raw strings).
            if c == '"' && next == Some('"') && chars.get(i + 2) == Some(&'"') {
                let end = find_triple_quote(&chars, i + 3).unwrap_or(len);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }
            if c == '"' || c == '\'' {
                let end = scan_quoted(&chars, i, c);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            // Annotations: `@Name`.
            if c == '@' && chars.get(i + 1).is_some_and(|c| is_ident_start(*c)) {
                let mut j = i + 1;
                while j < len && is_ident_continue(chars[j]) {
                    j += 1;
                }
                push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Attribute));
                i = j;
                continue;
            }

            if c.is_ascii_digit() {
                let end = scan_number(&chars, i);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Number));
                i = end;
                continue;
            }

            if is_ident_start(c) {
                let mut j = i;
                while j < len && is_ident_continue(chars[j]) {
                    j += 1;
                }
                let word: String = chars[i..j].iter().collect();
                let kind = if KEYWORDS.contains(&word.as_str()) {
                    TokenKind::Keyword
                } else if CONSTANTS.contains(&word.as_str()) {
                    TokenKind::Constant
                } else if MODIFIERS.contains(&word.as_str()) {
                    TokenKind::Keyword
                } else if BUILTINS.contains(&word.as_str()) {
                    TokenKind::Type
                } else if chars.get(j) == Some(&'(') {
                    TokenKind::Function
                } else if word.chars().next().is_some_and(char::is_uppercase) {
                    TokenKind::Type
                } else {
                    TokenKind::Plain
                };
                push_merged(&mut spans, HighlightSpan::new(i, j, kind));
                i = j;
                continue;
            }

            if matches!(
                c,
                '=' | '+'
                    | '-'
                    | '*'
                    | '/'
                    | '%'
                    | '<'
                    | '>'
                    | '!'
                    | '&'
                    | '|'
                    | '^'
                    | '?'
                    | ':'
                    | ';'
                    | ','
                    | '.'
                    | '~'
            ) {
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

fn find_triple_quote(chars: &[char], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 2 < chars.len() {
        if chars[i] == '"' && chars[i + 1] == '"' && chars[i + 2] == '"' {
            return Some(i + 3);
        }
        i += 1;
    }
    None
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// Classes, interfaces, objects, functions, properties, type aliases and
/// packages, for the symbol outline.
fn kotlin_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        if let Some(symbol) = kotlin_line_symbol(line, row) {
            symbols.push(symbol);
        }
    }
    symbols
}

fn kotlin_line_symbol(line: &str, row: usize) -> Option<Symbol> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*") {
        return None;
    }
    let words = words(trimmed);
    let mut index = 0;
    while index < words.len() {
        let word = words[index];
        let kind = match word {
            "fun" => Some(SymbolKind::Function),
            "class" => Some(SymbolKind::Struct),
            "interface" => Some(SymbolKind::Interface),
            "object" => Some(SymbolKind::Struct),
            "typealias" => Some(SymbolKind::Type),
            "val" | "var" => Some(SymbolKind::Variable),
            "package" => Some(SymbolKind::Module),
            _ => None,
        };
        if let Some(kind) = kind {
            let raw = words.get(index + 1)?;
            let name = identifier(raw);
            if name.is_empty() {
                return None;
            }
            return Some(Symbol::new(&name, kind, row, column_of(line, &name)));
        }
        // Skip annotations and modifiers before the declaration keyword.
        if word.starts_with('@') || MODIFIERS.contains(&word) || word == "enum" {
            index += 1;
            continue;
        }
        break;
    }
    None
}

/// Whitespace-delimited tokens on a line.
fn words(text: &str) -> Vec<&str> {
    text.split_whitespace().collect()
}

/// The leading identifier of a token, dropping any trailing `(`, `:`, `<` …
fn identifier(token: &str) -> String {
    token
        .chars()
        .take_while(|c| is_ident_continue(*c) || *c == '.')
        .collect()
}

/// The character column where `needle` first appears on the line.
fn column_of(line: &str, needle: &str) -> usize {
    match line.find(needle) {
        Some(byte) => line[..byte].chars().count(),
        None => line.chars().take_while(|c| c.is_whitespace()).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind_at(spans: &[HighlightSpan], col: usize) -> Option<TokenKind> {
        spans
            .iter()
            .find(|span| span.range.contains(&col))
            .map(|span| span.kind)
    }

    #[test]
    fn descriptor_claims_kotlin() {
        let descriptor = KotlinProvider.descriptor();
        assert_eq!(descriptor.id, LanguageId::Kotlin);
        assert!(descriptor.extensions.contains(&"kt"));
        assert!(descriptor.extensions.contains(&"kts"));
    }

    #[test]
    fn highlights_keywords_strings_and_functions() {
        let (spans, _) = KotlinProvider.highlight(
            "fun main() { val x = \"hi\"; println(x) }",
            HighlightState::default(),
        );
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Keyword)); // fun
        assert_eq!(kind_at(&spans, 4), Some(TokenKind::Function)); // main
        assert_eq!(kind_at(&spans, 22), Some(TokenKind::String)); // "hi"
    }

    #[test]
    fn annotations_and_block_comments() {
        let (spans, _) = KotlinProvider.highlight("@JvmStatic fun f()", HighlightState::default());
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Attribute));
        let (first, state) = KotlinProvider.highlight("/* comment", HighlightState::default());
        assert!(state.in_block_comment);
        assert_eq!(kind_at(&first, 0), Some(TokenKind::Comment));
    }

    #[test]
    fn raw_strings_highlight_as_strings() {
        let (spans, _) =
            KotlinProvider.highlight(r#"val s = """a // b""""#, HighlightState::default());
        assert_eq!(kind_at(&spans, 9), Some(TokenKind::String));
        // `//` inside the raw string is not a comment.
        assert!(!spans.iter().any(|span| span.kind == TokenKind::Comment));
    }

    #[test]
    fn symbols_cover_declarations() {
        let text = "\
package com.example
class Greeter {
    fun greet(name: String): String = \"hi\"
}
interface Sink {}
object Registry {}
val VERSION = 1
typealias Handler = (Int) -> Unit
";
        let symbols = kotlin_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"com.example"));
        assert!(names.contains(&"Greeter"));
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"Sink"));
        assert!(names.contains(&"Registry"));
        assert!(names.contains(&"VERSION"));
        assert!(names.contains(&"Handler"));
    }

    #[test]
    fn diagnostics_report_unbalanced_braces() {
        let diagnostics = KotlinProvider.diagnostics("fun main() {\n");
        assert!(
            diagnostics.iter().any(|d| d.message.contains("unclosed")),
            "expected an unclosed brace: {diagnostics:?}"
        );
        assert!(KotlinProvider.diagnostics("fun main() {\n}\n").is_empty());
    }
}
