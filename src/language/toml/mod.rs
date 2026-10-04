//! TOML language provider.
//!
//! Highlights comments, `[table]` headers, keys, quoted strings (including the
//! multi-line `"""`/`'''` forms), numbers and the `true`/`false` literals. It
//! deliberately never declares `Cargo.toml` as a *project marker*: that would
//! make a TOML file look like the project's language and would compete with the
//! Rust provider's project context. As a *file name* signal it is unambiguous.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_number, scan_quoted, scan_single_quoted};
use crate::language::detection::LanguageDescriptor;
use crate::language::diagnostics::Diagnostic;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Symbol, SymbolKind};

const LITERALS: &[&str] = &["true", "false"];

pub struct TomlProvider;

impl LanguageProvider for TomlProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Toml
    }

    fn display_name(&self) -> &'static str {
        "TOML"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Toml,
            extensions: &["toml"],
            // Deliberately empty: `Cargo.toml` establishes the *Rust* project,
            // so TOML must not claim it as its own project marker.
            project_markers: &[],
            file_names: &[
                "Cargo.toml",
                "Cargo.lock",
                "pyproject.toml",
                "rustfmt.toml",
                ".rustfmt.toml",
            ],
            shebangs: &[],
            content_hints: &[" = ", "[[", " = true", " = false", "[package]"],
        }
    }

    fn capabilities(&self) -> &'static [Capability] {
        &[
            Capability::SyntaxHighlighting,
            Capability::Diagnostics,
            Capability::DocumentSymbols,
            Capability::Completion,
        ]
    }

    fn diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        crate::language::diagnostics::check_delimiters(self, text)
    }

    fn symbols(&self, text: &str) -> Vec<Symbol> {
        toml_symbols(text)
    }

    fn completions(&self, _text: &str, _line: usize, _col: usize) -> Vec<Completion> {
        LITERALS
            .iter()
            .map(|word| Completion::new(*word, CompletionKind::Constant))
            .collect()
    }

    fn line_comment(&self) -> &'static str {
        "#"
    }

    fn highlight(&self, line: &str, state: HighlightState) -> (Vec<HighlightSpan>, HighlightState) {
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let mut spans = Vec::new();
        let mut i = 0;

        if state.in_block_comment {
            match find_triple_end(&chars, 0) {
                Some(end) => {
                    push_merged(&mut spans, HighlightSpan::new(0, end, TokenKind::String));
                    i = end;
                }
                None => {
                    if len > 0 {
                        push_merged(&mut spans, HighlightSpan::new(0, len, TokenKind::String));
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

        let first_nonspace = chars.iter().position(|c| !c.is_whitespace());
        let mut in_key = true;

        while i < len {
            let c = chars[i];

            // Comments run to the end of the line.
            if c == '#' {
                push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                break;
            }

            // A table header only exists at the start of a line and must be the
            // whole statement (optionally followed by a comment).
            if c == '[' && Some(i) == first_nonspace {
                let end = scan_table_header(&chars, i);
                if matches!(next_significant(&chars, end), None | Some('#')) {
                    push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Attribute));
                    i = end;
                    in_key = false;
                    continue;
                }
            }

            // Multi-line strings.
            if c == '"' && chars.get(i + 1) == Some(&'"') && chars.get(i + 2) == Some(&'"') {
                match find_triple_end(&chars, i + 3) {
                    Some(end) => {
                        push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                        i = end;
                    }
                    None => {
                        push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::String));
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
            if c == '\'' && chars.get(i + 1) == Some(&'\'') && chars.get(i + 2) == Some(&'\'') {
                match find_triple_end(&chars, i + 3) {
                    Some(end) => {
                        push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                        i = end;
                    }
                    None => {
                        push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::String));
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

            if c == '"' {
                let end = scan_quoted(&chars, i, '"');
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }
            if c == '\'' {
                let end = scan_single_quoted(&chars, i);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            if c.is_ascii_digit()
                || (matches!(c, '+' | '-') && chars.get(i + 1).is_some_and(char::is_ascii_digit))
            {
                let end = scan_number(&chars, i);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Number));
                i = end;
                continue;
            }

            if c.is_alphanumeric() || c == '_' || c == '-' {
                let mut j = i;
                while j < len && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '-')
                {
                    j += 1;
                }
                let word: String = chars[i..j].iter().collect();
                let kind = if LITERALS.contains(&word.as_str()) {
                    TokenKind::Constant
                } else if in_key {
                    TokenKind::Function
                } else {
                    TokenKind::Plain
                };
                if kind != TokenKind::Plain {
                    push_merged(&mut spans, HighlightSpan::new(i, j, kind));
                }
                i = j;
                continue;
            }

            if c == '=' {
                in_key = false;
                push_merged(
                    &mut spans,
                    HighlightSpan::new(i, i + 1, TokenKind::Operator),
                );
            } else if matches!(c, '.' | ',' | '{' | '}' | '[' | ']') {
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
                ..Default::default()
            },
        )
    }
}

/// The first non-whitespace character at or after `from`.
fn next_significant(chars: &[char], from: usize) -> Option<char> {
    chars[from..].iter().copied().find(|c| !c.is_whitespace())
}

/// End of a `[table]` or `[[array of tables]]` header.
fn scan_table_header(chars: &[char], start: usize) -> usize {
    let mut i = start;
    while i < chars.len() && chars[i] == '[' {
        i += 1;
    }
    while i < chars.len() && chars[i] != ']' {
        i += 1;
    }
    while i < chars.len() && chars[i] == ']' {
        i += 1;
    }
    i
}

/// End of a `"""` or `'''` run starting at `from` (past the opening quotes).
fn find_triple_end(chars: &[char], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 2 < chars.len() {
        let c = chars[i];
        if (c == '"' || c == '\'') && chars[i + 1] == c && chars[i + 2] == c {
            return Some(i + 3);
        }
        i += 1;
    }
    None
}

/// Table headers and root-level keys, for the symbol outline.
pub fn toml_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let mut seen_table = false;
    for (row, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.chars().take_while(|c| c.is_whitespace()).count();
        if trimmed.starts_with('[') {
            let name = trimmed.trim_start_matches('[').trim_end_matches(']').trim();
            if !name.is_empty() {
                symbols.push(Symbol::new(name, SymbolKind::Key, row, indent + 1));
            }
            seen_table = true;
            continue;
        }
        if !seen_table && let Some(eq) = trimmed.find('=') {
            let key = trimmed[..eq].trim();
            if !key.is_empty() {
                symbols.push(Symbol::new(key, SymbolKind::Key, row, indent));
            }
        }
    }
    symbols
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(line: &str) -> Vec<(TokenKind, usize, usize)> {
        let (spans, _) = TomlProvider.highlight(line, HighlightState::default());
        spans
            .into_iter()
            .map(|s| (s.kind, s.range.start, s.range.end))
            .collect()
    }

    #[test]
    fn highlights_table_header_keys_and_values() {
        let kinds: Vec<TokenKind> = spans("[package]").iter().map(|s| s.0).collect();
        assert_eq!(kinds, vec![TokenKind::Attribute]);

        let s = spans("name = \"koda\"");
        assert_eq!(s[0].0, TokenKind::Function);
        assert!(s.iter().any(|(kind, _, _)| *kind == TokenKind::String));
        assert!(s.iter().any(|(kind, _, _)| *kind == TokenKind::Operator));
    }

    #[test]
    fn comments_are_ignored() {
        let s = spans("key = 1 # trailing");
        assert_eq!(s.last().map(|e| e.0), Some(TokenKind::Comment));
    }

    #[test]
    fn multiline_strings_carry_state() {
        let (_, state) = TomlProvider.highlight("text = \"\"\"start", HighlightState::default());
        assert!(state.in_block_comment);
        let (s, state) = TomlProvider.highlight("finish\"\"\" # done", state);
        assert!(!state.in_block_comment);
        assert_eq!(s[0].kind, TokenKind::String);
        assert_eq!(s.last().map(|e| e.kind), Some(TokenKind::Comment));
    }

    #[test]
    fn symbols_list_tables_and_root_keys() {
        let text = "title = \"x\"\n\n[package]\nname = \"koda\"\n\n[dependencies]\nserde = \"1\"\n";
        let symbols = toml_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["title", "package", "dependencies"]);
    }

    #[test]
    fn descriptor_does_not_claim_cargo_as_project_marker() {
        assert!(TomlProvider.descriptor().project_markers.is_empty());
        assert!(TomlProvider.descriptor().file_names.contains(&"Cargo.toml"));
    }
}
