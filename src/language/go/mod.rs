//! Go language provider.
//!
//! Like the Rust provider, this is a focused scanner rather than a full parser.
//! It shares the same [`LanguageProvider`] contract, so the editor and UI never
//! need to know it exists.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::detection::LanguageDescriptor;
use crate::language::diagnostics::Diagnostic;
use crate::language::format::{FormatOutcome, gofmt};
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};

use std::path::Path;

const KEYWORDS: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];

const TYPES: &[&str] = &[
    "bool",
    "byte",
    "complex64",
    "complex128",
    "error",
    "float32",
    "float64",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "rune",
    "string",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
    "any",
];

const BUILTINS: &[&str] = &[
    "append", "cap", "close", "complex", "copy", "delete", "imag", "len", "make", "new", "panic",
    "print", "println", "real", "recover",
];

const CONSTANTS: &[&str] = &["true", "false", "nil", "iota"];

pub struct GoProvider;

impl LanguageProvider for GoProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Go
    }

    fn display_name(&self) -> &'static str {
        "Go"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Go,
            extensions: &["go"],
            project_markers: &["go.mod", "go.sum"],
            file_names: &[],
            shebangs: &[],
            content_hints: &["package ", "func ", "import (", ":= ", "go func"],
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
            Capability::Formatting,
            Capability::Hover,
            Capability::Rename,
        ]
    }

    fn diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        crate::language::diagnostics::check_delimiters(self, text)
    }

    fn symbols(&self, text: &str) -> Vec<crate::language::symbols::Symbol> {
        crate::language::symbols::go_symbols(text)
    }

    fn definition(
        &self,
        text: &str,
        line: usize,
        col: usize,
    ) -> Option<crate::language::symbols::Symbol> {
        let word = crate::language::symbols::word_at(text, line, col)?;
        crate::language::symbols::go_symbols(text)
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
        let mut completions = Vec::new();
        completions.extend(
            KEYWORDS
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Keyword)),
        );
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

    fn format(&self, _path: &Path, text: &str) -> FormatOutcome {
        gofmt(text)
    }

    fn formatter(&self) -> Option<&'static str> {
        Some("gofmt")
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = crate::language::symbols::go_symbols(text);
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
            if let Some(end) = find_block_comment_end(&chars, 0) {
                spans.push(HighlightSpan::new(0, end, TokenKind::Comment));
                i = end;
            } else {
                if len > 0 {
                    spans.push(HighlightSpan::new(0, len, TokenKind::Comment));
                }
                return (
                    spans,
                    HighlightState {
                        in_block_comment: true,
                    },
                );
            }
        }

        while i < len {
            let c = chars[i];

            if c == '/' && i + 1 < len && chars[i + 1] == '*' {
                match find_block_comment_end(&chars, i) {
                    Some(end) => {
                        spans.push(HighlightSpan::new(i, end, TokenKind::Comment));
                        i = end;
                    }
                    None => {
                        spans.push(HighlightSpan::new(i, len, TokenKind::Comment));
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

            if c == '/' && i + 1 < len && chars[i + 1] == '/' {
                spans.push(HighlightSpan::new(i, len, TokenKind::Comment));
                break;
            }

            // Interpreted string literal.
            if c == '"' {
                let end = scan_quoted(&chars, i, '"');
                spans.push(HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            // Raw string literal.
            if c == '`' {
                let end = scan_quoted(&chars, i, '`');
                spans.push(HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            // Rune literal.
            if c == '\'' {
                let end = scan_rune(&chars, i);
                spans.push(HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            if c.is_ascii_digit() {
                let end = scan_number(&chars, i);
                spans.push(HighlightSpan::new(i, end, TokenKind::Number));
                i = end;
                continue;
            }

            if is_ident_start(c) {
                let mut j = i;
                while j < len && is_ident_continue(chars[j]) {
                    j += 1;
                }
                let word: String = chars[i..j].iter().collect();
                let kind = classify_word(&word);
                push_merged(&mut spans, HighlightSpan::new(i, j, kind));
                i = j;
                continue;
            }

            if is_operator(c) {
                spans.push(HighlightSpan::new(i, i + 1, TokenKind::Operator));
            }
            i += 1;
        }

        (
            spans,
            HighlightState {
                in_block_comment: false,
            },
        )
    }
}

fn classify_word(word: &str) -> TokenKind {
    if KEYWORDS.contains(&word) {
        return TokenKind::Keyword;
    }
    if TYPES.contains(&word) {
        return TokenKind::Type;
    }
    if BUILTINS.contains(&word) {
        return TokenKind::Function;
    }
    if CONSTANTS.contains(&word) {
        return TokenKind::Constant;
    }
    if word.chars().next().is_some_and(char::is_uppercase) {
        // Exported identifiers read as types/functions; keep them understated.
        return TokenKind::Plain;
    }
    TokenKind::Plain
}

fn find_block_comment_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < chars.len() {
        if chars[i] == '*' && chars[i + 1] == '/' {
            return Some(i + 2);
        }
        i += 1;
    }
    None
}

fn scan_quoted(chars: &[char], start: usize, quote: char) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        if quote == '"' && chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == quote {
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

fn scan_rune(chars: &[char], start: usize) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == '\'' {
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

fn scan_number(chars: &[char], start: usize) -> usize {
    let mut i = start;
    if chars[i] == '0'
        && i + 1 < chars.len()
        && matches!(chars[i + 1], 'x' | 'X' | 'b' | 'B' | 'o' | 'O')
    {
        i += 2;
        while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
            i += 1;
        }
        return i;
    }
    while i < chars.len()
        && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.')
    {
        i += 1;
    }
    i
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
        '+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | '~' | '?' | ':'
    )
}

fn push_merged(spans: &mut Vec<HighlightSpan>, span: HighlightSpan) {
    if let Some(last) = spans.last_mut()
        && last.kind == span.kind
        && last.range.end == span.range.start
    {
        last.range.end = span.range.end;
        return;
    }
    spans.push(span);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_package_and_func() {
        let (spans, _) = GoProvider.highlight("func main() {}", HighlightState::default());
        assert!(spans.iter().any(|s| s.kind == TokenKind::Keyword));
    }

    #[test]
    fn raw_strings_span_whole_line() {
        let line = "x := `a // b`";
        let (spans, _) = GoProvider.highlight(line, HighlightState::default());
        let len = line.chars().count();
        assert!(
            spans
                .iter()
                .any(|s| s.kind == TokenKind::String && s.range == (5..len))
        );
    }
}
