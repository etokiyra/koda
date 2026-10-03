//! Swift provider.
//!
//! Built-in and offline: `//` and `/* … */` comments, regular and multiline
//! strings, attributes, keywords, types, numbers and operators. Structural
//! diagnostics reuse the shared delimiter checker. `sourcekit-lsp` (which ships
//! with the Swift toolchain) is discovered and launched when present; Koda does
//! not pretend a Swift toolchain is portable to every platform.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_number, scan_quoted};
use crate::language::detection::LanguageDescriptor;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Location, Symbol, SymbolKind, word_at};

const KEYWORDS: &[&str] = &[
    "associatedtype",
    "as",
    "break",
    "case",
    "catch",
    "class",
    "continue",
    "default",
    "defer",
    "deinit",
    "do",
    "else",
    "enum",
    "extension",
    "fallthrough",
    "false",
    "fileprivate",
    "for",
    "func",
    "guard",
    "if",
    "import",
    "in",
    "indirect",
    "init",
    "inout",
    "internal",
    "is",
    "let",
    "nil",
    "open",
    "operator",
    "private",
    "protocol",
    "public",
    "repeat",
    "rethrows",
    "return",
    "self",
    "static",
    "struct",
    "subscript",
    "super",
    "switch",
    "throw",
    "throws",
    "true",
    "try",
    "typealias",
    "var",
    "where",
    "while",
    "actor",
    "async",
    "await",
    "some",
    "any",
];

const BUILTINS: &[&str] = &[
    "Array",
    "Bool",
    "Character",
    "Dictionary",
    "Double",
    "Error",
    "Float",
    "Int",
    "Optional",
    "Result",
    "Set",
    "String",
    "Substring",
    "UInt",
    "Void",
    "print",
    "assert",
    "fatalError",
];

pub struct SwiftProvider;

impl LanguageProvider for SwiftProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Swift
    }

    fn display_name(&self) -> &'static str {
        "Swift"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Swift,
            extensions: &["swift"],
            project_markers: &["Package.swift"],
            file_names: &[],
            shebangs: &[],
            content_hints: &["import Foundation", "func ", "let ", "guard "],
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
        swift_symbols(text)
    }

    fn definition(&self, text: &str, line: usize, col: usize) -> Option<Symbol> {
        let word = word_at(text, line, col)?;
        swift_symbols(text)
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
            .map(|word| Completion::new(*word, CompletionKind::Keyword))
            .collect();
        completions.extend(
            BUILTINS
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Type)),
        );
        completions
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = swift_symbols(text);
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

            // Multiline strings `""" … """` (single-line approximation).
            if c == '"' && next == Some('"') && chars.get(i + 2) == Some(&'"') {
                let end = find_triple(&chars, i + 3).unwrap_or(len);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }
            if c == '"' {
                let end = scan_quoted(&chars, i, '"');
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            // Attributes and property wrappers: `@Name`.
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
                    | '~'
                    | '?'
                    | ':'
                    | ';'
                    | ','
                    | '.'
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

fn find_triple(chars: &[char], from: usize) -> Option<usize> {
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

/// Types, functions and type aliases.
fn swift_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        for keyword in [
            "class",
            "struct",
            "enum",
            "protocol",
            "extension",
            "actor",
            "typealias",
            "func",
        ] {
            let prefix = format!("{keyword} ");
            let Some(rest) = trimmed.strip_prefix(&prefix) else {
                if let Some(rest) = trimmed.strip_prefix(keyword)
                    && rest.starts_with([':', '('])
                {
                    // `init(...)` style is not a named declaration.
                    let _ = rest;
                }
                continue;
            };
            let name: String = rest
                .trim_start()
                .chars()
                .take_while(|c| is_ident_continue(*c) || *c == '.')
                .collect();
            if !name.is_empty() {
                let kind = match keyword {
                    "class" | "struct" | "actor" => SymbolKind::Struct,
                    "enum" => SymbolKind::Enum,
                    "protocol" => SymbolKind::Interface,
                    "typealias" => SymbolKind::Type,
                    _ => SymbolKind::Function,
                };
                let col = line
                    .find(&name)
                    .map(|byte| line[..byte].chars().count())
                    .unwrap_or(0);
                symbols.push(Symbol::new(name, kind, row, col));
            }
            break;
        }
    }
    symbols
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
    fn descriptor_claims_swift() {
        let descriptor = SwiftProvider.descriptor();
        assert_eq!(descriptor.id, LanguageId::Swift);
        assert!(descriptor.project_markers.contains(&"Package.swift"));
    }

    #[test]
    fn highlights_keywords_and_attributes() {
        let (spans, _) =
            SwiftProvider.highlight("@MainActor struct App {}", HighlightState::default());
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Attribute));
        let (line, _) = SwiftProvider.highlight("func greet() {}", HighlightState::default());
        assert_eq!(kind_at(&line, 0), Some(TokenKind::Keyword));
        assert_eq!(kind_at(&line, 5), Some(TokenKind::Function));
    }

    #[test]
    fn multiline_strings_are_strings() {
        let (spans, _) =
            SwiftProvider.highlight("let s = \"\"\"a // b\"\"\"", HighlightState::default());
        assert_eq!(kind_at(&spans, 8), Some(TokenKind::String));
        assert!(!spans.iter().any(|s| s.kind == TokenKind::Comment));
    }

    #[test]
    fn symbols_cover_declarations() {
        let text = "\
struct Point { }
protocol Shape { }
enum Kind { case a }
func area() -> Double { 0 }
class Box { }
";
        let symbols = swift_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Point", "Shape", "Kind", "area", "Box"]);
    }

    #[test]
    fn diagnostics_report_unbalanced_braces() {
        let diagnostics = SwiftProvider.diagnostics("func f() {\n");
        assert!(diagnostics.iter().any(|d| d.message.contains("unclosed")));
    }
}
