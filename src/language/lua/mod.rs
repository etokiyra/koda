//! Lua provider.
//!
//! Built-in and offline: `--` line comments, `--[[ … ]]` block comments (which
//! carry across lines), single- and double-quoted strings, `[[ … ]]` long
//! strings, numbers, keywords, builtins and operators. Structural diagnostics
//! reuse the shared delimiter checker. `lua-language-server`, which is
//! self-contained and needs no runtime, is provisioned for rich intelligence.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_quoted};
use crate::language::detection::LanguageDescriptor;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Location, Symbol, SymbolKind, word_at};

const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

const CONSTANTS: &[&str] = &["false", "nil", "true", "_G", "_VERSION", "self"];

const BUILTINS: &[&str] = &[
    "assert",
    "collectgarbage",
    "coroutine",
    "error",
    "getmetatable",
    "io",
    "ipairs",
    "math",
    "next",
    "os",
    "pairs",
    "pcall",
    "print",
    "rawequal",
    "rawget",
    "rawlen",
    "rawset",
    "require",
    "select",
    "setmetatable",
    "string",
    "table",
    "tonumber",
    "tostring",
    "type",
    "unpack",
    "xpcall",
];

pub struct LuaProvider;

impl LanguageProvider for LuaProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Lua
    }

    fn display_name(&self) -> &'static str {
        "Lua"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Lua,
            extensions: &["lua"],
            project_markers: &[".luarc.json"],
            file_names: &[],
            shebangs: &["lua"],
            content_hints: &["local ", "function ", "end\n"],
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
        lua_symbols(text)
    }

    fn definition(&self, text: &str, line: usize, col: usize) -> Option<Symbol> {
        let word = word_at(text, line, col)?;
        lua_symbols(text)
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
        completions.extend(
            CONSTANTS
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Constant)),
        );
        completions
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = lua_symbols(text);
        crate::language::hover::describe(text, line, col, &symbols)
    }

    fn line_comment(&self) -> &'static str {
        "--"
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

            // Comments: `--` to end of line, or a `--[[ … ]]` block.
            if c == '-' && next == Some('-') {
                if let Some(open) = long_bracket_open(&chars, i + 2) {
                    match find_long_close(&chars, open) {
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
                } else {
                    push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                    break;
                }
                continue;
            }

            // Long strings: `[[ … ]]` and `[=[ … ]=]`.
            if c == '['
                && let Some(open) = long_bracket_open(&chars, i)
            {
                match find_long_close(&chars, open) {
                    Some(end) => {
                        push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                        i = end;
                    }
                    None => {
                        push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::String));
                        break;
                    }
                }
                continue;
            }

            if c == '"' || c == '\'' {
                let end = scan_quoted(&chars, i, c);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
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
                } else if CONSTANTS.contains(&word.as_str()) {
                    TokenKind::Constant
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
                    | '^'
                    | '#'
                    | '<'
                    | '>'
                    | '~'
                    | '&'
                    | '|'
                    | ';'
                    | ':'
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

/// If a long bracket (`[[`, `[=[`, …) opens at `start`, the number of `=`.
fn long_bracket_open(chars: &[char], start: usize) -> Option<usize> {
    if chars.get(start) != Some(&'[') {
        return None;
    }
    let mut i = start + 1;
    let mut level = 0;
    while chars.get(i) == Some(&'=') {
        level += 1;
        i += 1;
    }
    (chars.get(i) == Some(&'[')).then_some(level)
}

/// The index just past the closing bracket for a long bracket of `level`.
fn find_long_close(chars: &[char], level: usize) -> Option<usize> {
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ']' {
            let mut j = i + 1;
            let mut seen = 0;
            while chars.get(j) == Some(&'=') {
                seen += 1;
                j += 1;
            }
            if seen == level && chars.get(j) == Some(&']') {
                return Some(j + 1);
            }
        }
        i += 1;
    }
    None
}

/// A `--[[` block comment opener, used when resuming a carried block comment.
fn find_block_end(chars: &[char], from: usize) -> Option<usize> {
    // A block comment closes at `]]` (level 0) or `]=]` etc.; level 0 is by far
    // the common case and the only one a plain `--[[` can open.
    let mut i = from;
    while i + 1 < chars.len() {
        if chars[i] == ']' && chars[i + 1] == ']' {
            return Some(i + 2);
        }
        i += 1;
    }
    None
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic() || (c as u32) > 0x7f
}

fn is_ident_continue(c: char) -> bool {
    is_ident_start(c) || c.is_ascii_digit()
}

/// Functions and methods, for the symbol outline.
fn lua_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("--") {
            continue;
        }
        let indent = line.chars().count() - trimmed.chars().count();

        // `function name`, `local function name`.
        let (prefix, kind_source) = if let Some(rest) = trimmed.strip_prefix("local function ") {
            (Some(rest), false)
        } else if let Some(rest) = trimmed.strip_prefix("function ") {
            (Some(rest), true)
        } else {
            (None, false)
        };
        if let Some(rest) = prefix {
            if let Some((name, offset)) = leading_name(rest) {
                let kind = if kind_source && name.contains([':', '.']) {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                symbols.push(Symbol::new(
                    name,
                    kind,
                    row,
                    indent + trimmed.len() - rest.len() + offset,
                ));
            }
            continue;
        }

        // `name = function(`, including `M.foo = function(` and `M:foo = function(`.
        if let Some(eq) = trimmed.find('=') {
            let after = trimmed[eq + 1..].trim_start();
            if after.starts_with("function") {
                let lhs = trimmed[..eq].trim_end();
                if let Some((name, _)) = leading_name(lhs) {
                    let kind = if name.contains([':', '.']) {
                        SymbolKind::Method
                    } else {
                        SymbolKind::Function
                    };
                    let offset =
                        indent + trimmed[..eq].len() - lhs.len() + (lhs.len() - name.len());
                    symbols.push(Symbol::new(name, kind, row, offset));
                }
            }
        }
    }
    symbols
}

/// The identifier at the start of `text` (trimmed), with its byte offset.
fn leading_name(text: &str) -> Option<(String, usize)> {
    let start = text.len() - text.trim_start().len();
    let text = &text[start..];
    let name: String = text
        .chars()
        .take_while(|c| is_ident_continue(*c) || *c == ':' || *c == '.')
        .collect();
    (!name.is_empty()).then_some((name, start))
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
    fn descriptor_claims_lua() {
        let descriptor = LuaProvider.descriptor();
        assert_eq!(descriptor.id, LanguageId::Lua);
        assert!(descriptor.extensions.contains(&"lua"));
    }

    #[test]
    fn highlights_keywords_strings_and_builtins() {
        let (spans, state) =
            LuaProvider.highlight("local x = \"hi\" print(x)", HighlightState::default());
        assert!(!state.in_block_comment);
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Keyword)); // local
        assert_eq!(kind_at(&spans, 8), Some(TokenKind::Operator)); // =
        assert_eq!(kind_at(&spans, 10), Some(TokenKind::String)); // "hi"
        assert_eq!(kind_at(&spans, 15), Some(TokenKind::Function)); // print
    }

    #[test]
    fn block_comments_carry_across_lines() {
        let (first, state) = LuaProvider.highlight("--[[ block", HighlightState::default());
        assert!(state.in_block_comment);
        assert_eq!(kind_at(&first, 0), Some(TokenKind::Comment));
        let (second, state) = LuaProvider.highlight("still ]] print(1)", state);
        assert!(!state.in_block_comment);
        assert_eq!(kind_at(&second, 9), Some(TokenKind::Function)); // print
    }

    #[test]
    fn long_strings_and_comments_use_matching_levels() {
        let (spans, _) = LuaProvider.highlight("x = [==[ a ]] b ]==]", HighlightState::default());
        assert!(spans.iter().any(|span| span.kind == TokenKind::String));
        // `--` inside a long string is not a comment.
        let (spans, _) =
            LuaProvider.highlight("local s = [[-- not a comment]]", HighlightState::default());
        assert_eq!(kind_at(&spans, 12), Some(TokenKind::String));
    }

    #[test]
    fn symbols_find_functions_and_methods() {
        let text = "\
local function helper() end
function greet(name) end
function M:run() end
M.stop = function() end
";
        let symbols = lua_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["helper", "greet", "M:run", "M.stop"]);
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[2].kind, SymbolKind::Method);
    }

    #[test]
    fn diagnostics_report_unbalanced_blocks() {
        let diagnostics = LuaProvider.diagnostics("local function f(\n");
        assert!(
            diagnostics.iter().any(|d| d.message.contains("unclosed")),
            "expected an unclosed paren: {diagnostics:?}"
        );
        assert!(
            LuaProvider
                .diagnostics("local function f()\n  return 1\nend\n")
                .is_empty()
        );
    }
}
