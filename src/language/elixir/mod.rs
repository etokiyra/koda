//! Elixir provider.
//!
//! Built-in and offline: `#` comments, single- and double-quoted strings,
//! heredocs, sigils, atoms, module attributes, aliases, keywords, numbers and
//! operators. Structural diagnostics reuse the shared delimiter checker. A
//! language server (ElixirLS or Lexical, via Erlang/Elixir) is discovered when
//! installed; Koda does not replace a system Erlang/Elixir installation.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_number, scan_quoted};
use crate::language::detection::LanguageDescriptor;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Location, Symbol, SymbolKind, word_at};

const KEYWORDS: &[&str] = &[
    "after",
    "alias",
    "and",
    "case",
    "catch",
    "cond",
    "def",
    "defdelegate",
    "defimpl",
    "defmacro",
    "defmodule",
    "defp",
    "defprotocol",
    "defstruct",
    "do",
    "else",
    "end",
    "fn",
    "for",
    "if",
    "import",
    "in",
    "not",
    "or",
    "quote",
    "raise",
    "receive",
    "require",
    "rescue",
    "throw",
    "try",
    "unless",
    "unquote",
    "use",
    "when",
    "with",
];

const BUILTINS: &[&str] = &[
    "IO",
    "Enum",
    "Map",
    "List",
    "String",
    "Kernel",
    "Process",
    "Agent",
    "GenServer",
    "Supervisor",
    "spawn",
    "send",
    "apply",
    "inspect",
    "length",
    "hd",
    "tl",
    "is_nil",
    "is_atom",
    "is_integer",
];

pub struct ElixirProvider;

impl LanguageProvider for ElixirProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Elixir
    }

    fn display_name(&self) -> &'static str {
        "Elixir"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Elixir,
            extensions: &["ex", "exs"],
            project_markers: &["mix.exs"],
            file_names: &[],
            shebangs: &["elixir"],
            content_hints: &["defmodule ", "def ", "defp ", "use Mix"],
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
        elixir_symbols(text)
    }

    fn definition(&self, text: &str, line: usize, col: usize) -> Option<Symbol> {
        let word = word_at(text, line, col)?;
        elixir_symbols(text)
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
        let symbols = elixir_symbols(text);
        crate::language::hover::describe(text, line, col, &symbols)
    }

    fn line_comment(&self) -> &'static str {
        "#"
    }

    fn highlight(
        &self,
        line: &str,
        _state: HighlightState,
    ) -> (Vec<HighlightSpan>, HighlightState) {
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let mut spans = Vec::new();
        let mut i = 0;

        while i < len {
            let c = chars[i];
            let next = chars.get(i + 1).copied();

            if c == '#' {
                push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                break;
            }

            // Sigils: `~r/.../`, `~w[...]`, `~s(...)`.
            if c == '~' && next.is_some_and(|c| c.is_ascii_alphabetic()) {
                let mut j = i + 1;
                while j < len && chars[j].is_ascii_alphabetic() {
                    j += 1;
                }
                if let Some(delim) = chars.get(j).copied()
                    && let Some(close) = matching_delimiter(delim)
                {
                    let mut k = j + 1;
                    while k < len && chars[k] != close {
                        k += 1;
                    }
                    let end = (k + 1).min(len);
                    push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                    i = end;
                    continue;
                }
            }

            if c == '"' || c == '\'' {
                let end = scan_quoted(&chars, i, c);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            // Module attributes: `@name`.
            if c == '@' && chars.get(i + 1).is_some_and(|c| is_ident_start(*c)) {
                let mut j = i + 1;
                while j < len && is_ident_continue(chars[j]) {
                    j += 1;
                }
                push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Attribute));
                i = j;
                continue;
            }

            // Atoms: `:name`.
            if c == ':'
                && chars
                    .get(i + 1)
                    .is_some_and(|c| is_ident_start(*c) || *c == '"')
            {
                let mut j = i + 1;
                if chars[j] == '"' {
                    j = scan_quoted(&chars, j, '"');
                } else {
                    while j < len && is_ident_continue(chars[j]) {
                        j += 1;
                    }
                }
                push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Constant));
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

fn matching_delimiter(open: char) -> Option<char> {
    Some(match open {
        '/' => '/',
        '|' => '|',
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        '"' => '"',
        '\'' => '\'',
        _ => return None,
    })
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric() || c == '?' || c == '!'
}

/// Modules, functions and macros.
fn elixir_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        for (keyword, kind) in [
            ("defmodule ", SymbolKind::Module),
            ("defprotocol ", SymbolKind::Interface),
            ("defimpl ", SymbolKind::Struct),
            ("defmacro ", SymbolKind::Macro),
            ("defp ", SymbolKind::Function),
            ("def ", SymbolKind::Function),
        ] {
            if let Some(rest) = trimmed.strip_prefix(keyword) {
                let name: String = rest
                    .trim_start()
                    .chars()
                    .take_while(|c| is_ident_continue(*c) || *c == '.')
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
    fn descriptor_claims_elixir() {
        let descriptor = ElixirProvider.descriptor();
        assert_eq!(descriptor.id, LanguageId::Elixir);
        assert!(descriptor.extensions.contains(&"ex"));
        assert!(descriptor.project_markers.contains(&"mix.exs"));
    }

    #[test]
    fn highlights_atoms_attributes_and_keywords() {
        let (spans, _) = ElixirProvider.highlight(
            "defmodule Greeter do\n  @moduledoc false\n  def hi, do: :ok\n",
            HighlightState::default(),
        );
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Keyword)); // defmodule
        assert_eq!(kind_at(&spans, 10), Some(TokenKind::Type)); // Greeter
    }

    #[test]
    fn atoms_and_attributes() {
        let (spans, _) = ElixirProvider.highlight("x = :ok", HighlightState::default());
        assert_eq!(kind_at(&spans, 4), Some(TokenKind::Constant)); // :ok
        let (spans, _) = ElixirProvider.highlight("@value 1", HighlightState::default());
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Attribute));
    }

    #[test]
    fn sigils_are_strings() {
        let (spans, _) =
            ElixirProvider.highlight("~r/^#not-a-comment$/", HighlightState::default());
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::String));
        assert!(!spans.iter().any(|s| s.kind == TokenKind::Comment));
    }

    #[test]
    fn symbols_cover_modules_and_functions() {
        let text = "\
defmodule Greeter do
  def hello(name), do: name
  defp secret, do: :ok
end
";
        let symbols = elixir_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Greeter", "hello", "secret"]);
        assert_eq!(symbols[0].kind, SymbolKind::Module);
        assert_eq!(symbols[1].kind, SymbolKind::Function);
    }
}
