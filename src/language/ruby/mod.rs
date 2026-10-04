//! Ruby provider.
//!
//! Built-in and offline: `#` line comments, `=begin`/`=end` block comments,
//! single- and double-quoted strings, backticks, percent literals, symbols,
//! instance/class/global variables, constants, keywords, builtins numbers and
//! operators. Structural diagnostics reuse the shared delimiter checker and a
//! `solargraph` server can be provisioned for richer intelligence.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_quoted, scan_single_quoted};
use crate::language::detection::LanguageDescriptor;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Location, Symbol, SymbolKind, word_at};

const KEYWORDS: &[&str] = &[
    "alias", "and", "begin", "break", "case", "class", "def", "defined?", "do", "else", "elsif",
    "end", "ensure", "false", "for", "if", "in", "module", "next", "nil", "not", "or", "redo",
    "rescue", "retry", "return", "self", "super", "then", "true", "undef", "unless", "until",
    "when", "while", "yield",
];

const BUILTINS: &[&str] = &[
    "Array",
    "Hash",
    "Integer",
    "String",
    "Symbol",
    "puts",
    "print",
    "p",
    "require",
    "require_relative",
    "attr_accessor",
    "attr_reader",
    "attr_writer",
    "include",
    "extend",
    "raise",
    "loop",
    "lambda",
    "proc",
    "gets",
    "format",
    "sprintf",
    "rand",
    "sleep",
    "catch",
    "throw",
    "freeze",
    "new",
];

pub struct RubyProvider;

impl LanguageProvider for RubyProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Ruby
    }

    fn display_name(&self) -> &'static str {
        "Ruby"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Ruby,
            extensions: &["rb", "rake", "gemspec", "ru", "builder"],
            project_markers: &["Gemfile", "Rakefile", ".ruby-version"],
            file_names: &["Gemfile", "Rakefile", "Guardfile", "Vagrantfile", "Capfile"],
            shebangs: &["ruby"],
            content_hints: &["def ", "end\n", "require ", "class "],
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
        ruby_symbols(text)
    }

    fn definition(&self, text: &str, line: usize, col: usize) -> Option<Symbol> {
        let word = word_at(text, line, col)?;
        ruby_symbols(text)
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
        let symbols = ruby_symbols(text);
        crate::language::hover::describe(text, line, col, &symbols)
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
            if line.trim_start().starts_with("=end") {
                push_merged(&mut spans, HighlightSpan::new(0, len, TokenKind::Comment));
                return (spans, HighlightState::default());
            }
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

        // `=begin` must start the line.
        if line.starts_with("=begin") {
            push_merged(&mut spans, HighlightSpan::new(0, len, TokenKind::Comment));
            return (
                spans,
                HighlightState {
                    in_block_comment: true,
                    ..Default::default()
                },
            );
        }

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

            // Symbols: `:name`, `:"..."`.
            if c == ':'
                && chars
                    .get(i + 1)
                    .is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '@' || *c == '$')
            {
                let mut j = i + 1;
                while j < len
                    && (chars[j].is_alphanumeric()
                        || matches!(chars[j], '_' | '@' | '$' | '?' | '!'))
                {
                    j += 1;
                }
                push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Constant));
                i = j;
                continue;
            }

            // Variables: `@x`, `@@x`, `$x`.
            if (c == '@' || c == '$') && chars.get(i + 1).is_some_and(|c| is_ident_start(*c)) {
                let mut j = i + 1;
                if c == '@' && chars.get(j) == Some(&'@') {
                    j += 1;
                }
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
                } else if word.chars().next().is_some_and(char::is_uppercase) {
                    TokenKind::Constant
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

/// Classes, modules and methods.
fn ruby_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        for (keyword, kind) in [
            ("def ", SymbolKind::Method),
            ("class ", SymbolKind::Struct),
            ("module ", SymbolKind::Module),
        ] {
            if let Some(rest) = trimmed.strip_prefix(keyword) {
                // `def self.name`, `def a.b` — keep the whole qualified name.
                let name: String = rest
                    .trim_start()
                    .chars()
                    .take_while(|c| is_ident_continue(*c) || matches!(c, '.' | '?' | '!' | '='))
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
    fn descriptor_claims_ruby() {
        let descriptor = RubyProvider.descriptor();
        assert_eq!(descriptor.id, LanguageId::Ruby);
        assert!(descriptor.project_markers.contains(&"Gemfile"));
    }

    #[test]
    fn highlights_symbols_and_variables() {
        let (spans, _) = RubyProvider.highlight(
            "def greet(name); puts :ok; @count += 1; end",
            HighlightState::default(),
        );
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Keyword)); // def
        assert_eq!(kind_at(&spans, 24), Some(TokenKind::Constant)); // :ok
        assert_eq!(kind_at(&spans, 29), Some(TokenKind::Type)); // @count
    }

    #[test]
    fn block_comments_are_carried() {
        let (_, state) = RubyProvider.highlight("=begin", HighlightState::default());
        assert!(state.in_block_comment);
        let (line, state) = RubyProvider.highlight("still a comment", state);
        assert!(state.in_block_comment);
        assert_eq!(kind_at(&line, 0), Some(TokenKind::Comment));
        let (_, state) = RubyProvider.highlight("=end", state);
        assert!(!state.in_block_comment);
    }

    #[test]
    fn symbols_cover_modules_and_methods() {
        let text = "\
module Greeter
  class Speaker
    def greet(name)
    end
    def self.build
    end
  end
end
";
        let symbols = ruby_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Greeter", "Speaker", "greet", "self.build"]);
    }

    #[test]
    fn diagnostics_report_unbalanced_parens() {
        let diagnostics = RubyProvider.diagnostics("def f(a\n");
        assert!(diagnostics.iter().any(|d| d.message.contains("unclosed")));
    }
}
