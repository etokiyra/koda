//! Dart provider.
//!
//! Built-in and offline: `//` and `/* … */` comments, single- and double-quoted
//! strings (including `'''`/`"""` blocks), string interpolation, annotations,
//! keywords, types, numbers and operators. Structural diagnostics reuse the
//! shared delimiter checker. The Dart SDK's analysis server is discovered when
//! installed for full LSP support.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_number, scan_quoted};
use crate::language::detection::LanguageDescriptor;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Location, Symbol, SymbolKind, word_at};

const KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "assert",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "covariant",
    "default",
    "deferred",
    "do",
    "dynamic",
    "else",
    "enum",
    "export",
    "extends",
    "extension",
    "external",
    "factory",
    "false",
    "final",
    "finally",
    "for",
    "get",
    "hide",
    "if",
    "implements",
    "import",
    "in",
    "interface",
    "is",
    "late",
    "library",
    "mixin",
    "new",
    "null",
    "on",
    "operator",
    "part",
    "required",
    "rethrow",
    "return",
    "sealed",
    "set",
    "show",
    "static",
    "super",
    "switch",
    "sync",
    "this",
    "throw",
    "true",
    "try",
    "typedef",
    "var",
    "void",
    "when",
    "while",
    "with",
    "yield",
];

const BUILTINS: &[&str] = &[
    "bool",
    "double",
    "int",
    "num",
    "Object",
    "String",
    "List",
    "Map",
    "Set",
    "Iterable",
    "Future",
    "Stream",
    "print",
    "assert",
    "identical",
    "Duration",
    "DateTime",
    "RegExp",
    "Error",
];

pub struct DartProvider;

impl LanguageProvider for DartProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Dart
    }

    fn display_name(&self) -> &'static str {
        "Dart"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Dart,
            extensions: &["dart"],
            project_markers: &["pubspec.yaml", "analysis_options.yaml"],
            file_names: &[],
            shebangs: &[],
            content_hints: &["import 'package:", "void main(", "Widget build", "class "],
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
        dart_symbols(text)
    }

    fn definition(&self, text: &str, line: usize, col: usize) -> Option<Symbol> {
        let word = word_at(text, line, col)?;
        dart_symbols(text)
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
        let symbols = dart_symbols(text);
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
            if c == '"' || c == '\'' {
                let triple = chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c);
                let end = if triple {
                    find_triple(&chars, i + 3, c).unwrap_or(len)
                } else {
                    scan_quoted(&chars, i, c)
                };
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

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

fn find_triple(chars: &[char], from: usize, quote: char) -> Option<usize> {
    let mut i = from;
    while i + 2 < chars.len() {
        if chars[i] == quote && chars[i + 1] == quote && chars[i + 2] == quote {
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

/// Classes, mixins, enums, extensions and typedefs.
fn dart_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        for (keyword, kind) in [
            ("class ", SymbolKind::Struct),
            ("mixin ", SymbolKind::Trait),
            ("enum ", SymbolKind::Enum),
            ("extension ", SymbolKind::Trait),
            ("typedef ", SymbolKind::Type),
            ("abstract class ", SymbolKind::Struct),
        ] {
            if let Some(rest) = trimmed.strip_prefix(keyword) {
                let name: String = rest
                    .trim_start()
                    .chars()
                    .take_while(|c| is_ident_continue(*c))
                    .collect();
                if !name.is_empty() {
                    let col = line
                        .find(&name)
                        .map(|byte| line[..byte].chars().count())
                        .unwrap_or(0);
                    symbols.push(Symbol::new(name, kind, row, col));
                }
                break;
            }
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
    fn descriptor_claims_dart() {
        let descriptor = DartProvider.descriptor();
        assert_eq!(descriptor.id, LanguageId::Dart);
        assert!(descriptor.project_markers.contains(&"pubspec.yaml"));
    }

    #[test]
    fn highlights_classes_strings_and_annotations() {
        let (spans, _) = DartProvider.highlight(
            "@override\nclass Greeter { final name = 'x'; }",
            HighlightState::default(),
        );
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Attribute)); // @override
        let (line, _) = DartProvider.highlight("class Greeter {}", HighlightState::default());
        assert_eq!(kind_at(&line, 0), Some(TokenKind::Keyword));
        assert_eq!(kind_at(&line, 6), Some(TokenKind::Type));
    }

    #[test]
    fn triple_quoted_strings_are_strings() {
        let (spans, _) = DartProvider.highlight("var s = '''a // b''';", HighlightState::default());
        assert_eq!(kind_at(&spans, 8), Some(TokenKind::String));
        assert!(!spans.iter().any(|s| s.kind == TokenKind::Comment));
    }

    #[test]
    fn symbols_cover_declarations() {
        let text = "\
class Greeter {}
mixin Speaker {}
enum Color { red }
typedef Handler = void Function();
";
        let symbols = dart_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Greeter", "Speaker", "Color", "Handler"]);
    }

    #[test]
    fn diagnostics_report_unbalanced_braces() {
        let diagnostics = DartProvider.diagnostics("void main() {\n");
        assert!(diagnostics.iter().any(|d| d.message.contains("unclosed")));
    }
}
