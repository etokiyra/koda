//! Perl provider.
//!
//! Built-in and offline: `#` comments, POD blocks, single- and double-quoted
//! strings, backticks, sigiled variables, keywords, built-ins, numbers and
//! operators. Perl is famously context-sensitive, so this is a lexical scan
//! rather than a parser: it never claims to understand context. Structural
//! diagnostics reuse the shared delimiter checker, and a Perl language server
//! is used when one is installed.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_quoted, scan_single_quoted};
use crate::language::detection::LanguageDescriptor;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Location, Symbol, SymbolKind, word_at};

const KEYWORDS: &[&str] = &[
    "and", "chomp", "chop", "continue", "do", "else", "elsif", "eq", "for", "foreach", "ge",
    "goto", "gt", "if", "last", "le", "local", "lt", "my", "ne", "next", "no", "not", "or", "our",
    "package", "redo", "require", "return", "state", "sub", "unless", "until", "use", "while",
    "xor",
];

const BUILTINS: &[&str] = &[
    "abs",
    "chdir",
    "close",
    "defined",
    "delete",
    "die",
    "each",
    "eval",
    "exists",
    "exit",
    "grep",
    "hex",
    "index",
    "int",
    "join",
    "keys",
    "lc",
    "length",
    "map",
    "open",
    "pop",
    "print",
    "printf",
    "push",
    "quotemeta",
    "rand",
    "read",
    "reverse",
    "rindex",
    "scalar",
    "shift",
    "sort",
    "splice",
    "split",
    "sprintf",
    "sqrt",
    "sprintf",
    "substr",
    "sprintf",
    "uc",
    "undef",
    "unlink",
    "unshift",
    "values",
    "warn",
];

pub struct PerlProvider;

impl LanguageProvider for PerlProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Perl
    }

    fn display_name(&self) -> &'static str {
        "Perl"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Perl,
            extensions: &["pl", "pm", "t", "pod", "psgi", "cgi"],
            project_markers: &["Makefile.PL", "cpanfile", "dist.ini"],
            file_names: &[],
            shebangs: &["perl"],
            content_hints: &["use strict", "my $", "use warnings", "sub "],
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
        perl_symbols(text)
    }

    fn definition(&self, text: &str, line: usize, col: usize) -> Option<Symbol> {
        let word = word_at(text, line, col)?;
        perl_symbols(text)
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
                .map(|word| Completion::new(*word, CompletionKind::Function)),
        );
        completions
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = perl_symbols(text);
        crate::language::hover::describe(text, line, col, &symbols)
    }

    fn line_comment(&self) -> &'static str {
        "#"
    }

    fn highlight(&self, line: &str, state: HighlightState) -> (Vec<HighlightSpan>, HighlightState) {
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let mut spans = Vec::new();

        // POD starts with `=` at the beginning of a line and ends at `=cut`.
        if line.starts_with('=') && !line.starts_with("==") {
            let pod = !line.starts_with("=cut");
            push_merged(&mut spans, HighlightSpan::new(0, len, TokenKind::Comment));
            return (
                spans,
                HighlightState {
                    in_block_comment: pod,
                },
            );
        }
        if state.in_block_comment {
            let ends = line.starts_with("=cut");
            if len > 0 {
                push_merged(&mut spans, HighlightSpan::new(0, len, TokenKind::Comment));
            }
            return (
                spans,
                HighlightState {
                    in_block_comment: !ends,
                },
            );
        }

        let mut i = 0;
        while i < len {
            let c = chars[i];

            if c == '#' {
                push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                break;
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
            if c == '`' {
                let end = scan_quoted(&chars, i, '`');
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            // Sigiled variables: `$x`, `@x`, `%x`, `&x`.
            if matches!(c, '$' | '@' | '%' | '&')
                && chars.get(i + 1).is_some_and(|c| is_ident_start(*c))
            {
                let mut j = i + 1;
                while j < len && is_ident_continue(chars[j]) {
                    j += 1;
                }
                push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Type));
                i = j;
                continue;
            }

            if c.is_ascii_digit() {
                let mut j = i;
                while j < len
                    && (chars[j].is_ascii_alphanumeric() || chars[j] == '.' || chars[j] == '_')
                {
                    j += 1;
                }
                push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Number));
                i = j;
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
                } else if BUILTINS.contains(&word.as_str()) || chars.get(j) == Some(&'(') {
                    TokenKind::Function
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

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// Packages and subroutines.
fn perl_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        for (keyword, kind) in [
            ("sub ", SymbolKind::Function),
            ("package ", SymbolKind::Module),
        ] {
            if let Some(rest) = trimmed.strip_prefix(keyword) {
                let name: String = rest
                    .trim_start()
                    .chars()
                    .take_while(|c| is_ident_continue(*c) || *c == ':')
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
    fn descriptor_claims_perl() {
        let descriptor = PerlProvider.descriptor();
        assert_eq!(descriptor.id, LanguageId::Perl);
        assert!(descriptor.extensions.contains(&"pm"));
        assert!(descriptor.project_markers.contains(&"cpanfile"));
    }

    #[test]
    fn highlights_variables_strings_and_keywords() {
        let (spans, _) = PerlProvider.highlight(
            "my $name = \"world\"; print $name;",
            HighlightState::default(),
        );
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Keyword)); // my
        assert_eq!(kind_at(&spans, 3), Some(TokenKind::Type)); // $name
        assert_eq!(kind_at(&spans, 11), Some(TokenKind::String)); // "world"
        assert_eq!(kind_at(&spans, 20), Some(TokenKind::Function)); // print
    }

    #[test]
    fn pod_blocks_are_comments() {
        let (_, state) = PerlProvider.highlight("=pod", HighlightState::default());
        assert!(state.in_block_comment);
        let (line, state) = PerlProvider.highlight("Some documentation", state);
        assert_eq!(kind_at(&line, 0), Some(TokenKind::Comment));
        let (_, state) = PerlProvider.highlight("=cut", state);
        assert!(!state.in_block_comment);
    }

    #[test]
    fn symbols_cover_subs_and_packages() {
        let symbols = perl_symbols("package Greeter;\nsub greet { }\n");
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Greeter", "greet"]);
    }

    #[test]
    fn diagnostics_report_unbalanced_braces() {
        let diagnostics = PerlProvider.diagnostics("sub f {\n");
        assert!(diagnostics.iter().any(|d| d.message.contains("unclosed")));
    }
}
