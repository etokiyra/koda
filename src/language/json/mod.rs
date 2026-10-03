//! JSON language provider.
//!
//! A focused scanner rather than a parser: it highlights strings (distinguishing
//! object keys from values), numbers, the `true`/`false`/`null` literals and
//! punctuation, and it understands `//` and `/* */` comments so JSONC files do
//! not look broken. Diagnostics reuse the shared delimiter checker.

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

/// Scalar literals JSON recognises.
const LITERALS: &[&str] = &["true", "false", "null"];

pub struct JsonProvider;

impl LanguageProvider for JsonProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Json
    }

    fn display_name(&self) -> &'static str {
        "JSON"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Json,
            extensions: &["json", "jsonc", "geojson", "webmanifest"],
            project_markers: &[],
            file_names: &[],
            shebangs: &[],
            content_hints: &["\": true", "\": false", "\": null", "\": \"", "\": ["],
        }
    }

    fn capabilities(&self) -> &'static [Capability] {
        &[
            Capability::SyntaxHighlighting,
            Capability::Diagnostics,
            Capability::DocumentSymbols,
            Capability::Completion,
            Capability::Formatting,
        ]
    }

    fn format(&self, path: &Path, text: &str) -> FormatOutcome {
        crate::language::format::prettier(path, text)
    }

    fn formatter(&self) -> Option<&'static str> {
        Some("prettier")
    }

    fn diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        crate::language::diagnostics::check_delimiters(self, text)
    }

    fn symbols(&self, text: &str) -> Vec<Symbol> {
        json_symbols(text)
    }

    fn completions(&self, _text: &str, _line: usize, _col: usize) -> Vec<Completion> {
        LITERALS
            .iter()
            .map(|word| Completion::new(*word, CompletionKind::Constant))
            .collect()
    }

    fn line_comment(&self) -> &'static str {
        "//"
    }

    fn highlight(&self, line: &str, state: HighlightState) -> (Vec<HighlightSpan>, HighlightState) {
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let mut spans = Vec::new();
        let mut i = 0;

        // Finish a `/* ... */` comment carried in from the previous line.
        if state.in_block_comment {
            match find_block_comment_end(&chars, 0) {
                Some(end) => {
                    spans.push(HighlightSpan::new(0, end, TokenKind::Comment));
                    i = end;
                }
                None => {
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
        }

        while i < len {
            let c = chars[i];

            if c == '/' && chars.get(i + 1) == Some(&'/') {
                spans.push(HighlightSpan::new(i, len, TokenKind::Comment));
                break;
            }
            if c == '/' && chars.get(i + 1) == Some(&'*') {
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

            if c == '"' {
                let end = scan_quoted(&chars, i, '"');
                let kind = if is_key_at(&chars, end) {
                    TokenKind::Attribute
                } else {
                    TokenKind::String
                };
                push_merged(&mut spans, HighlightSpan::new(i, end, kind));
                i = end;
                continue;
            }

            if c == '-' || c.is_ascii_digit() {
                let end = scan_number(&chars, i);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Number));
                i = end;
                continue;
            }

            if c.is_alphabetic() || c == '_' {
                let mut j = i;
                while j < len && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let word: String = chars[i..j].iter().collect();
                if LITERALS.contains(&word.as_str()) {
                    push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Constant));
                }
                i = j;
                continue;
            }

            if matches!(c, '{' | '}' | '[' | ']' | ',' | ':') {
                push_merged(
                    &mut spans,
                    HighlightSpan::new(i, i + 1, TokenKind::Operator),
                );
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

/// Whether the string ending at `end` is an object key (the next significant
/// character is `:`).
fn is_key_at(chars: &[char], end: usize) -> bool {
    let mut j = end;
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }
    chars.get(j) == Some(&':')
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

/// Top-level object keys, for the symbol outline and workspace search.
pub fn json_symbols(text: &str) -> Vec<Symbol> {
    let chars: Vec<char> = text.chars().collect();
    let mut symbols = Vec::new();
    let mut depth = 0i32;
    let mut row = 0usize;
    let mut col = 0usize;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            row += 1;
            col = 0;
            i += 1;
            continue;
        }
        if c == '"' {
            let end = scan_quoted(&chars, i, '"');
            let close = if end > i && chars[end - 1] == '"' {
                end - 1
            } else {
                end
            };
            let name: String = chars[(i + 1).min(close)..close].iter().collect();
            if depth == 1 && !name.is_empty() && is_key_at(&chars, end) {
                symbols.push(Symbol::new(name, SymbolKind::Key, row, col + 1));
            }
            for &ch in &chars[i..end] {
                if ch == '\n' {
                    row += 1;
                    col = 0;
                } else {
                    col += 1;
                }
            }
            i = end;
            continue;
        }
        match c {
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            _ => {}
        }
        col += 1;
        i += 1;
    }
    symbols
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::diagnostics::Severity;

    fn kinds(line: &str) -> Vec<TokenKind> {
        let (spans, _) = JsonProvider.highlight(line, HighlightState::default());
        spans.into_iter().map(|span| span.kind).collect()
    }

    #[test]
    fn distinguishes_keys_from_string_values() {
        let (spans, _) = JsonProvider.highlight("  \"name\": \"koda\",", HighlightState::default());
        let kinds: Vec<TokenKind> = spans.iter().map(|s| s.kind).collect();
        assert_eq!(kinds[0], TokenKind::Attribute);
        assert_eq!(kinds[1], TokenKind::Operator);
        assert_eq!(kinds[2], TokenKind::String);
    }

    #[test]
    fn highlights_literals_and_numbers() {
        assert_eq!(
            kinds("{\"on\": true, \"n\": -1.5e3}"),
            vec![
                TokenKind::Operator,
                TokenKind::Attribute,
                TokenKind::Operator,
                TokenKind::Constant,
                TokenKind::Operator,
                TokenKind::Attribute,
                TokenKind::Operator,
                TokenKind::Number,
                TokenKind::Operator,
            ]
        );
    }

    #[test]
    fn line_and_block_comments() {
        assert_eq!(kinds("// hi"), vec![TokenKind::Comment]);
        let (_, state) = JsonProvider.highlight("/* start", HighlightState::default());
        assert!(state.in_block_comment);
        let (spans, state) = JsonProvider.highlight("end */ \"x\"", state);
        assert!(!state.in_block_comment);
        assert_eq!(spans[0].kind, TokenKind::Comment);
    }

    #[test]
    fn finds_top_level_keys() {
        let text = "{\n  \"name\": \"koda\",\n  \"nested\": {\n    \"deep\": 1\n  }\n}\n";
        let symbols = json_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["name", "nested"]);
        assert_eq!(symbols[0].kind, SymbolKind::Key);
        assert_eq!(symbols[0].line, 1);
        assert_eq!(symbols[0].col, 3);
    }

    #[test]
    fn unbalanced_object_is_reported() {
        let diags = JsonProvider.diagnostics("{\n  \"a\": [1, 2\n");
        assert!(
            diags.iter().any(|d| d.severity == Severity::Error),
            "expected a structural diagnostic"
        );
    }
}
