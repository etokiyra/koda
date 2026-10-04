//! Java provider.
//!
//! Built-in and offline: highlighting (keywords, primitive and common library
//! types, annotations, strings and text blocks, line/block comments), structural
//! diagnostics, symbols (types, methods, fields), completion, hover and
//! within-file navigation. `Eclipse JDT Language Server` is provisioned
//! separately for rename, code actions and project-aware analysis.

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

const KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "interface",
    "native",
    "new",
    "non-sealed",
    "package",
    "permits",
    "private",
    "protected",
    "public",
    "record",
    "return",
    "sealed",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "var",
    "volatile",
    "while",
    "yield",
];

const TYPES: &[&str] = &[
    "boolean",
    "byte",
    "char",
    "double",
    "float",
    "int",
    "long",
    "short",
    "void",
    "String",
    "Object",
    "Integer",
    "Long",
    "Double",
    "Float",
    "Boolean",
    "Character",
    "Byte",
    "Short",
    "List",
    "ArrayList",
    "Map",
    "HashMap",
    "Set",
    "HashSet",
    "Optional",
    "Stream",
    "Collection",
    "Iterable",
];

const BUILTINS: &[&str] = &[
    "System",
    "Math",
    "Objects",
    "Arrays",
    "Collections",
    "StringBuilder",
    "Thread",
    "Runtime",
    "Files",
    "Paths",
    "Pattern",
    "Scanner",
    "Exception",
    "RuntimeException",
    "IllegalArgumentException",
];

const CONSTANTS: &[&str] = &["true", "false", "null"];

/// Modifiers that may precede a declaration.
const MODIFIERS: &[&str] = &[
    "public",
    "private",
    "protected",
    "static",
    "final",
    "abstract",
    "synchronized",
    "native",
    "transient",
    "volatile",
    "strictfp",
    "default",
    "sealed",
];

/// Words that never name a method.
const CONTROL: &[&str] = &[
    "if",
    "for",
    "while",
    "switch",
    "catch",
    "return",
    "new",
    "throw",
    "assert",
    "instanceof",
    "super",
    "this",
];

pub struct JavaProvider;

impl LanguageProvider for JavaProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Java
    }

    fn display_name(&self) -> &'static str {
        "Java"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Java,
            extensions: &["java"],
            project_markers: &[
                "pom.xml",
                "build.gradle",
                "build.gradle.kts",
                "settings.gradle",
            ],
            file_names: &[],
            shebangs: &[],
            content_hints: &[
                "public class ",
                "import java",
                "private ",
                "static void main",
            ],
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
        java_symbols(text)
    }

    fn definition(
        &self,
        text: &str,
        line: usize,
        col: usize,
    ) -> Option<crate::language::symbols::Symbol> {
        let word = crate::language::symbols::word_at(text, line, col)?;
        java_symbols(text)
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
        let symbols = java_symbols(text);
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
                            ..Default::default()
                        },
                    );
                }
            }
        }

        while i < len {
            let c = chars[i];

            if c == '/' && chars.get(i + 1) == Some(&'/') {
                push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                break;
            }

            if c == '/' && chars.get(i + 1) == Some(&'*') {
                // `/**` is a Javadoc comment; still a comment.
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
                                ..Default::default()
                            },
                        );
                    }
                }
                continue;
            }

            // Text block `"""` (scanned to the end of the line).
            if c == '"' && chars.get(i + 1) == Some(&'"') && chars.get(i + 2) == Some(&'"') {
                let end = chars.len();
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

            if c.is_ascii_digit() {
                let end = scan_number(&chars, i);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Number));
                i = end;
                continue;
            }

            // Annotations: `@Override`.
            if c == '@' && chars.get(i + 1).is_some_and(|c| is_ident_start(*c)) {
                let mut j = i + 1;
                while j < len && (is_ident_continue(chars[j]) || chars[j] == '.') {
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

                if matches!(
                    word.as_str(),
                    "class" | "interface" | "enum" | "record" | "new"
                ) && let Some((start, end)) = name_after(&chars, j)
                {
                    push_merged(&mut spans, HighlightSpan::new(start, end, TokenKind::Type));
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

fn name_after(chars: &[char], from: usize) -> Option<(usize, usize)> {
    let start = skip_ws(chars, from);
    let (name, end) = word_at(chars, start);
    (!name.is_empty()).then_some((start, end))
}

fn skip_ws(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

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

/// Named declarations in a document.
fn java_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = skip_ws(&chars, 0);
        // Skip annotations and modifiers.
        loop {
            i = skip_ws(&chars, i);
            if chars.get(i) == Some(&'@') {
                i += 1;
                while i < chars.len() && (is_ident_continue(chars[i]) || chars[i] == '.') {
                    i += 1;
                }
                continue;
            }
            let (word, end) = word_at(&chars, i);
            if MODIFIERS.contains(&word.as_str()) {
                i = end;
                continue;
            }
            break;
        }

        let (first, after) = word_at(&chars, i);
        if first == "package" {
            if let Some((name, col)) = qualified_name(&chars, after) {
                symbols.push(Symbol::new(name, SymbolKind::Module, row, col));
            }
            continue;
        }
        if first == "import" {
            continue;
        }
        if matches!(first.as_str(), "class" | "interface" | "enum" | "record") {
            if let Some((name, col)) = name_at(&chars, after) {
                let kind = if first == "interface" {
                    SymbolKind::Interface
                } else if first == "enum" {
                    SymbolKind::Enum
                } else {
                    SymbolKind::Type
                };
                symbols.push(Symbol::new(name, kind, row, col));
            }
            continue;
        }
        // A method or field on the line.
        if let Some((name, col)) = member_name(&chars) {
            let kind = if chars.contains(&'(') {
                SymbolKind::Method
            } else {
                SymbolKind::Variable
            };
            symbols.push(Symbol::new(name, kind, row, col));
        }
    }
    symbols
}

/// The dotted name after `package`.
fn qualified_name(chars: &[char], from: usize) -> Option<(String, usize)> {
    let start = skip_ws(chars, from);
    let mut end = start;
    while end < chars.len() && (is_ident_continue(chars[end]) || chars[end] == '.') {
        end += 1;
    }
    if end == start {
        return None;
    }
    let text: String = chars[start..end].iter().collect();
    Some((text.trim_end_matches('.').to_string(), start))
}

fn name_at(chars: &[char], from: usize) -> Option<(String, usize)> {
    let start = skip_ws(chars, from);
    let (name, _) = word_at(chars, start);
    (!name.is_empty()).then_some((name, start))
}

/// A method or field name declared on a line: an identifier before `(` with a
/// type before it, or the last identifier before `=`/`;`/`,`.
fn member_name(chars: &[char]) -> Option<(String, usize)> {
    // Method: identifier immediately before `(`.
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '(' {
            let mut end = i;
            while end > 0 && chars[end - 1].is_whitespace() {
                end -= 1;
            }
            let mut start = end;
            while start > 0 && is_ident_continue(chars[start - 1]) {
                start -= 1;
            }
            if start < end {
                let name: String = chars[start..end].iter().collect();
                let before = (0..start)
                    .rev()
                    .find(|index| !chars[*index].is_whitespace());
                let plausible = before
                    .is_some_and(|b| is_ident_continue(chars[b]) || matches!(chars[b], '>' | ']'));
                if plausible && !CONTROL.contains(&name.as_str()) {
                    return Some((name, start));
                }
            }
        }
        i += 1;
    }

    // Field: identifier before `=` or `;` with a type before it.
    let terminator = chars
        .iter()
        .position(|c| *c == '=' || *c == ';')
        .unwrap_or(chars.len());
    let mut end = terminator;
    while end > 0 && chars[end - 1].is_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && is_ident_continue(chars[start - 1]) {
        start -= 1;
    }
    if start < end {
        let name: String = chars[start..end].iter().collect();
        let before = (0..start)
            .rev()
            .find(|index| !chars[*index].is_whitespace());
        if before.is_some_and(|b| is_ident_continue(chars[b]) || matches!(chars[b], '>' | ']')) {
            return Some((name, start));
        }
    }
    None
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

fn is_operator(c: char) -> bool {
    matches!(
        c,
        '+' | '-'
            | '*'
            | '/'
            | '%'
            | '='
            | '<'
            | '>'
            | '!'
            | '&'
            | '|'
            | '^'
            | '~'
            | '?'
            | ':'
            | '.'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_claims_java_and_maven_gradle() {
        let descriptor = JavaProvider.descriptor();
        assert_eq!(descriptor.id, LanguageId::Java);
        assert!(descriptor.extensions.contains(&"java"));
        assert!(descriptor.project_markers.contains(&"pom.xml"));
        assert!(descriptor.project_markers.contains(&"build.gradle"));
    }

    #[test]
    fn highlights_annotations_and_keywords() {
        let kind_at = |spans: &[HighlightSpan], col: usize| {
            spans
                .iter()
                .find(|span| span.range.contains(&col))
                .map(|span| span.kind)
        };
        let (spans, _) = JavaProvider.highlight(
            "@Override public String name() { return \"koda\"; }",
            HighlightState::default(),
        );
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Attribute));
        assert_eq!(kind_at(&spans, 10), Some(TokenKind::Keyword)); // public
        assert_eq!(kind_at(&spans, 17), Some(TokenKind::Type)); // String
    }

    #[test]
    fn carries_block_comments_across_lines() {
        let (_, state) = JavaProvider.highlight("/** doc", HighlightState::default());
        assert!(state.in_block_comment);
        let (spans, state) = JavaProvider.highlight(" * more */ int x;", state);
        assert!(!state.in_block_comment);
        assert!(spans.iter().any(|span| span.kind == TokenKind::Comment));
    }

    #[test]
    fn extracts_types_methods_and_fields() {
        let text = "package com.demo;\n\npublic class Widget {\n    private int count = 0;\n    public String name() { return \"w\"; }\n}\n";
        let names: Vec<_> = java_symbols(text)
            .into_iter()
            .map(|symbol| (symbol.name, symbol.kind))
            .collect();
        assert!(names.contains(&("com.demo".to_string(), SymbolKind::Module)));
        assert!(names.contains(&("Widget".to_string(), SymbolKind::Type)));
        assert!(names.contains(&("count".to_string(), SymbolKind::Variable)));
        assert!(names.contains(&("name".to_string(), SymbolKind::Method)));
    }
}
