//! Nix language provider.
//!
//! Nix is a small functional language used for packages and system
//! configuration. The offline provider recognises its comments (`#` and nested
//! `/* … */`), double-quoted and indented (`''…''`) strings (which may span
//! lines), search paths (`<nixpkgs>`), paths, numbers, keywords, literals and
//! the builtins that appear in almost every expression. Attribute bindings
//! (`services.nginx.enable = true;`) drive the symbol outline, navigation and
//! hover.
//!
//! `nil` is the language server Koda uses when it is available, and `nixfmt`
//! formats a buffer from stdin. Koda installs both from a pinned nixpkgs
//! revision through a Koda-private Nix profile.

use std::path::Path;

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_number};
use crate::language::detection::LanguageDescriptor;
use crate::language::diagnostics::Diagnostic;
use crate::language::format::FormatOutcome;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, LexMode, TokenKind,
};
use crate::language::symbols::{Location, Symbol, SymbolKind, word_at};

const KEYWORDS: &[&str] = &[
    "assert", "else", "if", "in", "inherit", "let", "or", "rec", "then", "with",
];

const CONSTANTS: &[&str] = &["false", "null", "true"];

/// Builtins and names that are common enough to be worth completing and
/// highlighting. This is a lexical assist, not a `builtins` introspection.
const BUILTINS: &[&str] = &[
    "abort",
    "baseNameOf",
    "builtins",
    "config",
    "derivation",
    "dirOf",
    "fetchGit",
    "fetchTarball",
    "fetchurl",
    "import",
    "isNull",
    "lib",
    "map",
    "options",
    "pkgs",
    "removeAttrs",
    "self",
    "super",
    "throw",
    "toString",
];

pub struct NixProvider;

impl LanguageProvider for NixProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Nix
    }

    fn display_name(&self) -> &'static str {
        "Nix"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Nix,
            extensions: &["nix"],
            project_markers: &["flake.nix"],
            file_names: &["flake.nix", "default.nix", "shell.nix"],
            shebangs: &["nix-shell"],
            content_hints: &["{ pkgs", "with lib;", " mkDerivation", "builtins.", " in {"],
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
            Capability::Formatting,
        ]
    }

    fn diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        crate::language::diagnostics::check_delimiters(self, text)
    }

    fn symbols(&self, text: &str) -> Vec<Symbol> {
        nix_symbols(text)
    }

    fn definition(&self, text: &str, line: usize, col: usize) -> Option<Symbol> {
        let word = word_at(text, line, col)?;
        nix_symbols(text)
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
            CONSTANTS
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Constant)),
        );
        completions.extend(
            BUILTINS
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Function)),
        );
        completions
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = nix_symbols(text);
        crate::language::hover::describe(text, line, col, &symbols)
    }

    fn format(&self, _path: &Path, text: &str) -> FormatOutcome {
        crate::language::format::nixfmt(text)
    }

    fn formatter(&self) -> Option<&'static str> {
        Some("nixfmt")
    }

    fn line_comment(&self) -> &'static str {
        "#"
    }

    fn highlight(&self, line: &str, state: HighlightState) -> (Vec<HighlightSpan>, HighlightState) {
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let mut spans = Vec::new();
        let mut i = 0;

        // Resume a block comment that was still open at the end of the previous
        // line. Nix block comments nest, so the carried depth matters.
        if state.in_block_comment {
            let (end, depth) = scan_block_comment(&chars, 0, state.block_comment_depth.max(1));
            if end > 0 {
                push_merged(&mut spans, HighlightSpan::new(0, end, TokenKind::Comment));
            }
            if depth > 0 {
                return (
                    spans,
                    HighlightState {
                        in_block_comment: true,
                        block_comment_depth: depth,
                        ..Default::default()
                    },
                );
            }
            i = end;
        }

        // Resume a multi-line string. Everything up to its closing delimiter is
        // still string content.
        if let LexMode::MultiLineString { indented } = state.mode {
            let (end, closed) = if indented {
                scan_indented_string(&chars, 0)
            } else {
                scan_double_quoted(&chars, 0)
            };
            if end > 0 {
                push_merged(&mut spans, HighlightSpan::new(0, end, TokenKind::String));
            }
            if !closed {
                return (
                    spans,
                    HighlightState {
                        mode: LexMode::MultiLineString { indented },
                        ..Default::default()
                    },
                );
            }
            i = end;
        }

        while i < len {
            let c = chars[i];
            let next = chars.get(i + 1).copied();

            if c == '#' {
                push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                break;
            }

            if c == '/' && next == Some('*') {
                let (end, depth) = scan_block_comment(&chars, i, 0);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Comment));
                if depth > 0 {
                    return (
                        spans,
                        HighlightState {
                            in_block_comment: true,
                            block_comment_depth: depth,
                            ..Default::default()
                        },
                    );
                }
                i = end;
                continue;
            }

            if c == '"' {
                let (end, closed) = scan_double_quoted(&chars, i);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                if !closed {
                    return (
                        spans,
                        HighlightState {
                            mode: LexMode::MultiLineString { indented: false },
                            ..Default::default()
                        },
                    );
                }
                i = end;
                continue;
            }

            if c == '\'' && next == Some('\'') {
                let (end, closed) = scan_indented_string(&chars, i);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                if !closed {
                    return (
                        spans,
                        HighlightState {
                            mode: LexMode::MultiLineString { indented: true },
                            ..Default::default()
                        },
                    );
                }
                i = end;
                continue;
            }

            // Search paths such as `<nixpkgs>` are strings in Nix.
            if c == '<'
                && let Some(end) = scan_search_path(&chars, i)
            {
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            // Paths and URIs are string-like values.
            if let Some(end) = scan_path_or_uri(&chars, i) {
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
                } else if BUILTINS.contains(&word.as_str()) || is_binding_name(&chars, j) {
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

            if is_operator(c) {
                let end = scan_operator(&chars, i);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Operator));
                i = end;
                continue;
            }

            i += 1;
        }

        (spans, HighlightState::default())
    }
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic()
}

/// Nix identifiers may contain letters, digits, `_`, `-` and `'`.
fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '\'')
}

/// End of a `/* … */` block comment opened at `start`.
///
/// Returns the index just past the closing `*/` (or the end of the line) and the
/// number of comment levels still open. Nix block comments nest.
fn scan_block_comment(chars: &[char], start: usize, mut depth: u8) -> (usize, u8) {
    let mut i = start;
    if depth == 0 {
        depth = 1;
        i += 2;
    }
    while i + 1 < chars.len() {
        if chars[i] == '/' && chars[i + 1] == '*' {
            depth = depth.saturating_add(1);
            i += 2;
        } else if chars[i] == '*' && chars[i + 1] == '/' {
            i += 2;
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return (i, 0);
            }
        } else {
            i += 1;
        }
    }
    (chars.len(), depth)
}

/// End of a `"…"` string opened at `start`, honouring `\` escapes.
fn scan_double_quoted(chars: &[char], start: usize) -> (usize, bool) {
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '"' => return (i + 1, true),
            _ => i += 1,
        }
    }
    (chars.len(), false)
}

/// End of an indented `''…''` string opened at `start`.
///
/// The three-character escapes inside the string (`'''`, `''$` and `''\`) do
/// not close it.
fn scan_indented_string(chars: &[char], start: usize) -> (usize, bool) {
    let mut i = start + 2;
    while i < chars.len() {
        if chars[i] == '\'' && chars.get(i + 1) == Some(&'\'') {
            if matches!(chars.get(i + 2), Some('\'' | '$' | '\\')) {
                i += 3;
                continue;
            }
            return (i + 2, true);
        }
        i += 1;
    }
    (chars.len(), false)
}

/// End of a `<nixpkgs>` search path, if one starts at `start`.
fn scan_search_path(chars: &[char], start: usize) -> Option<usize> {
    let close = chars[start + 1..].iter().position(|c| *c == '>')?;
    let content = &chars[start + 1..start + 1 + close];
    if content.is_empty() || content.iter().any(|c| c.is_whitespace()) {
        return None;
    }
    Some(start + 1 + close + 1)
}

/// End of a path or URI literal at `start`, or `None` when it is punctuation.
///
/// Paths start with `.`, `..`, `~` or `/` at an expression boundary. Requiring
/// the boundary keeps `a/b` as the division operator rather than a path.
fn scan_path_or_uri(chars: &[char], start: usize) -> Option<usize> {
    let c = chars.get(start).copied()?;
    let previous = start.checked_sub(1).and_then(|i| chars.get(i)).copied();
    let boundary = previous.is_none_or(|p| {
        p.is_whitespace() || matches!(p, '(' | '[' | '{' | '=' | ':' | ';' | ',' | '+' | '-')
    });

    let uri = (c.is_ascii_alphabetic() && boundary)
        .then(|| {
            let mut i = start;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || matches!(chars[i], '+' | '-' | '.'))
            {
                i += 1;
            }
            chars
                .get(i..i + 3)
                .is_some_and(|scheme| scheme == [':', '/', '/'])
                .then_some(i + 3)
        })
        .flatten();

    let path = if boundary && matches!(c, '.' | '~' | '/') {
        let rest: String = chars[start..].iter().take(3).collect();
        let looks_like_path = rest.starts_with("./")
            || rest.starts_with("../")
            || rest.starts_with("~/")
            || (c == '/'
                && !matches!(chars.get(start + 1), Some('/') | Some('*'))
                && chars.get(start + 1).is_some_and(|n| !is_path_stop(*n)));
        looks_like_path.then(|| {
            let mut i = start;
            while i < chars.len() && !is_path_stop(chars[i]) {
                i += 1;
            }
            i
        })
    } else {
        None
    };

    let end = uri.or(path)?;
    let mut i = end;
    // A URI path continues through characters that a bare path stops at.
    if uri.is_some() {
        while i < chars.len()
            && !chars[i].is_whitespace()
            && !matches!(chars[i], ';' | ',' | ')' | ']' | '}')
        {
            i += 1;
        }
    }
    (i > start).then_some(i)
}

fn is_path_stop(c: char) -> bool {
    c.is_whitespace() || matches!(c, ';' | ',' | ')' | ']' | '}' | ':' | '=' | '?' | '@')
}

/// Whether the identifier ending at `end` is an attribute name (`foo.bar =`).
fn is_binding_name(chars: &[char], mut end: usize) -> bool {
    loop {
        while end < chars.len() && chars[end].is_whitespace() {
            end += 1;
        }
        match chars.get(end) {
            Some('=') if chars.get(end + 1) != Some(&'=') => return true,
            Some('.') => {
                end += 1;
                while end < chars.len() && chars[end].is_whitespace() {
                    end += 1;
                }
                while end < chars.len() && is_ident_continue(chars[end]) {
                    end += 1;
                }
            }
            _ => return false,
        }
    }
}

fn is_operator(c: char) -> bool {
    matches!(
        c,
        '=' | ':'
            | ';'
            | ','
            | '.'
            | '?'
            | '@'
            | '+'
            | '-'
            | '*'
            | '/'
            | '!'
            | '<'
            | '>'
            | '&'
            | '|'
    )
}

/// The end of the operator run starting at `start`, capped to the longest
/// meaningful Nix operator so `=>` and `->` do not swallow following code.
fn scan_operator(chars: &[char], start: usize) -> usize {
    const TWO: &[&str] = &["->", "==", "!=", "<=", ">=", "&&", "||", "//", "++"];
    const THREE: &[&str] = &["..."];
    for length in [3, 2] {
        if start + length <= chars.len() {
            let candidate: String = chars[start..start + length].iter().collect();
            let table: &[&str] = if length == 3 { THREE } else { TWO };
            if table.contains(&candidate.as_str()) {
                return start + length;
            }
        }
    }
    start + 1
}

/// Attribute bindings, for the symbol outline.
///
/// Nix has no classes or functions distinct from values, so every binding is a
/// key. A dotted path (`services.nginx.enable = true;`) is reported under its
/// full name, which makes the outline and workspace search useful in modules.
pub fn nix_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
            || trimmed.starts_with("*/")
        {
            continue;
        }
        if let Some((name, offset)) = binding_at(trimmed) {
            let indent = line.chars().count() - trimmed.chars().count();
            symbols.push(Symbol::new(name, SymbolKind::Key, row, indent + offset));
        }
    }
    symbols
}

/// The dotted attribute name at the start of a trimmed line, with its offset.
fn binding_at(text: &str) -> Option<(String, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut start = chars.iter().position(|c| !c.is_whitespace())?;
    // A `let` binding is written `let name = …;`, often on the same line.
    if chars[start..].starts_with(&['l', 'e', 't']) {
        let mut after = start + 3;
        if chars.get(after).is_some_and(|c| c.is_whitespace()) {
            while after < chars.len() && chars[after].is_whitespace() {
                after += 1;
            }
            start = after;
        }
    }
    if start >= chars.len() || (!is_ident_start(chars[start]) && chars[start] != '"') {
        return None;
    }
    let mut i = start;
    loop {
        if chars[i] == '"' {
            let (end, closed) = scan_double_quoted(&chars, i);
            if !closed {
                return None;
            }
            i = end;
        } else {
            while i < chars.len() && is_ident_continue(chars[i]) {
                i += 1;
            }
        }
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if chars.get(i) == Some(&'.') {
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i >= chars.len() || (!is_ident_start(chars[i]) && chars[i] != '"') {
                return None;
            }
            continue;
        }
        break;
    }
    if chars.get(i) != Some(&'=') || chars.get(i + 1) == Some(&'=') {
        return None;
    }
    let name: String = chars[start..i].iter().collect();
    let name = name.trim().to_string();
    (!name.is_empty()).then_some((name, start))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(line: &str) -> Vec<(TokenKind, usize, usize)> {
        let (spans, _) = NixProvider.highlight(line, HighlightState::default());
        spans
            .into_iter()
            .map(|span| (span.kind, span.range.start, span.range.end))
            .collect()
    }

    fn kind_at(spans: &[HighlightSpan], col: usize) -> Option<TokenKind> {
        spans
            .iter()
            .find(|span| span.range.contains(&col))
            .map(|span| span.kind)
    }

    #[test]
    fn descriptor_claims_nix() {
        let descriptor = NixProvider.descriptor();
        assert_eq!(descriptor.id, LanguageId::Nix);
        assert!(descriptor.extensions.contains(&"nix"));
        assert!(descriptor.project_markers.contains(&"flake.nix"));
    }

    #[test]
    fn highlights_keywords_literals_and_bindings() {
        let (spans, state) = NixProvider.highlight(
            "{ pkgs, ... }: let enabled = true; in builtins.map (x: x) [ ]",
            HighlightState::default(),
        );
        assert!(!state.in_block_comment);
        assert_eq!(kind_at(&spans, 2), Some(TokenKind::Function)); // pkgs
        assert_eq!(kind_at(&spans, 15), Some(TokenKind::Keyword)); // let
        assert_eq!(kind_at(&spans, 19), Some(TokenKind::Function)); // enabled
        assert_eq!(kind_at(&spans, 29), Some(TokenKind::Constant)); // true
        assert_eq!(kind_at(&spans, 35), Some(TokenKind::Keyword)); // in
        assert_eq!(kind_at(&spans, 38), Some(TokenKind::Function)); // builtins
    }

    #[test]
    fn comments_and_nested_blocks_carry_state() {
        let (spans, state) = NixProvider.highlight("x = 1; # note", HighlightState::default());
        assert_eq!(spans.last().map(|span| span.kind), Some(TokenKind::Comment));
        assert!(!state.in_block_comment);

        let (spans, state) = NixProvider.highlight("/* outer /* inner", HighlightState::default());
        assert!(state.in_block_comment);
        assert_eq!(state.block_comment_depth, 2);
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Comment));

        let (spans, state) = NixProvider.highlight("still */ done */ x = 1;", state);
        assert!(!state.in_block_comment);
        assert_eq!(spans[0].kind, TokenKind::Comment);
        assert!(spans.iter().any(|span| span.kind == TokenKind::Function));
    }

    #[test]
    fn strings_carry_across_lines() {
        let (_, state) = NixProvider.highlight("script = \"echo hi", HighlightState::default());
        assert_eq!(state.mode, LexMode::MultiLineString { indented: false });
        let (spans, state) = NixProvider.highlight("and bye\";", state);
        assert_eq!(state.mode, LexMode::None);
        assert_eq!(spans[0].kind, TokenKind::String);

        let (_, state) = NixProvider.highlight("text = ''hello", HighlightState::default());
        assert_eq!(state.mode, LexMode::MultiLineString { indented: true });
        let (spans, state) = NixProvider.highlight("world'';", state);
        assert_eq!(state.mode, LexMode::None);
        assert_eq!(spans[0].kind, TokenKind::String);
    }

    #[test]
    fn escapes_do_not_close_strings_early() {
        // `''${` is a literal `${` inside an indented string, not an escape out.
        let (_, state) = NixProvider.highlight("x = ''a ''${b} c'';", HighlightState::default());
        assert_eq!(state.mode, LexMode::None);
        let (spans, _) = NixProvider.highlight("x = \"a\\\"b\";", HighlightState::default());
        assert_eq!(kind_at(&spans, 4), Some(TokenKind::String));
    }

    #[test]
    fn paths_and_search_paths_are_strings() {
        let s = spans("src = ./modules;");
        assert!(
            s.iter()
                .any(|(kind, start, _)| *kind == TokenKind::String && *start == 6),
            "{s:?}"
        );
        assert_eq!(
            spans("pkgs = import <nixpkgs> {};")[0].0,
            TokenKind::Function
        );
        let s = spans("url = \"https://example.com/x\";");
        assert!(s.iter().any(|(kind, _, _)| *kind == TokenKind::String));
        let s = spans("home = https://example.com/x;");
        assert!(s.iter().any(|(kind, _, _)| *kind == TokenKind::String));
        // Division stays an operator.
        let s = spans("half = a/b;");
        assert!(s.iter().any(|(kind, _, _)| *kind == TokenKind::Operator));
    }

    #[test]
    fn symbols_find_attribute_bindings() {
        let text = "\
{ pkgs, ... }:
{
  services.nginx.enable = true;
  environment.systemPackages = with pkgs; [ git ];
  # commented = false;
  let local = 1; in local
}
";
        let symbols = nix_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "services.nginx.enable",
                "environment.systemPackages",
                "local"
            ]
        );
        assert!(symbols.iter().all(|symbol| symbol.kind == SymbolKind::Key));
    }

    #[test]
    fn diagnostics_ignore_brackets_in_strings_and_comments() {
        let text = "x = \"{(\";\n# )\n/* ] */\n";
        assert!(NixProvider.diagnostics(text).is_empty());
        assert!(
            NixProvider
                .diagnostics("x = { y = 1;\n")
                .iter()
                .any(|d| d.message.contains("unclosed"))
        );
    }

    #[test]
    fn definition_and_references_stay_within_the_file() {
        let text = "let helper = 1; in helper + helper\n";
        let symbol = NixProvider.definition(text, 0, 5).expect("definition");
        assert_eq!(symbol.name, "helper");
        assert_eq!(NixProvider.references(text, 0, 5).len(), 3);
    }
}
