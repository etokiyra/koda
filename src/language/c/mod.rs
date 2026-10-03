//! C and C++ provider.
//!
//! One scanner serves both languages: they share the preprocessor, the brace
//! syntax and the comment forms, and differ mainly in keywords, types and the
//! files they claim. Built-in and offline: highlighting, structural
//! diagnostics, symbols, completion, hover and within-file navigation.
//!
//! `clangd` is not provisioned — there is no portable, user-local installer —
//! but Koda uses it when it is already on the system, so a compiler installation
//! brings richer, type-aware intelligence with no setup.

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
    "alignas",
    "alignof",
    "asm",
    "auto",
    "break",
    "case",
    "catch",
    "class",
    "concept",
    "const",
    "consteval",
    "constexpr",
    "constinit",
    "const_cast",
    "continue",
    "co_await",
    "co_return",
    "co_yield",
    "decltype",
    "default",
    "delete",
    "do",
    "dynamic_cast",
    "else",
    "enum",
    "explicit",
    "export",
    "extern",
    "final",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "operator",
    "override",
    "private",
    "protected",
    "public",
    "register",
    "reinterpret_cast",
    "requires",
    "return",
    "sizeof",
    "static",
    "static_assert",
    "static_cast",
    "struct",
    "switch",
    "template",
    "this",
    "thread_local",
    "throw",
    "try",
    "typedef",
    "typeid",
    "typename",
    "union",
    "using",
    "virtual",
    "volatile",
    "while",
];

const TYPES: &[&str] = &[
    "bool",
    "char",
    "char8_t",
    "char16_t",
    "char32_t",
    "double",
    "float",
    "int",
    "int8_t",
    "int16_t",
    "int32_t",
    "int64_t",
    "long",
    "short",
    "signed",
    "size_t",
    "ssize_t",
    "ptrdiff_t",
    "uint8_t",
    "uint16_t",
    "uint32_t",
    "uint64_t",
    "unsigned",
    "void",
    "wchar_t",
    "auto",
];

const BUILTINS: &[&str] = &[
    "abort",
    "assert",
    "atof",
    "atoi",
    "bsearch",
    "calloc",
    "cerr",
    "cin",
    "cout",
    "endl",
    "exit",
    "fclose",
    "fflush",
    "fgets",
    "fopen",
    "fprintf",
    "fputs",
    "fread",
    "free",
    "fwrite",
    "getchar",
    "malloc",
    "make_shared",
    "make_unique",
    "memcpy",
    "memmove",
    "memset",
    "move",
    "perror",
    "printf",
    "putchar",
    "puts",
    "qsort",
    "realloc",
    "scanf",
    "snprintf",
    "sprintf",
    "strcat",
    "strchr",
    "strcmp",
    "strcpy",
    "strlen",
    "strncpy",
    "strstr",
    "strtod",
    "strtol",
    "swap",
];

const CONSTANTS: &[&str] = &[
    "NULL", "true", "false", "nullptr", "EOF", "stdin", "stdout", "stderr",
];

/// Control-flow words never name a function.
const CONTROL: &[&str] = &[
    "if", "for", "while", "switch", "return", "sizeof", "catch", "do", "else", "case", "throw",
    "new", "delete", "assert", "defined",
];

/// A provider instance for C or C++.
pub struct CProvider {
    id: LanguageId,
}

impl CProvider {
    pub fn c() -> Self {
        CProvider { id: LanguageId::C }
    }

    pub fn cpp() -> Self {
        CProvider {
            id: LanguageId::Cpp,
        }
    }
}

impl LanguageProvider for CProvider {
    fn id(&self) -> LanguageId {
        self.id
    }

    fn display_name(&self) -> &'static str {
        self.id.name()
    }

    fn descriptor(&self) -> LanguageDescriptor {
        match self.id {
            LanguageId::C => LanguageDescriptor {
                id: LanguageId::C,
                extensions: &["c", "h"],
                project_markers: &["CMakeLists.txt", "Makefile", "meson.build"],
                file_names: &[],
                shebangs: &[],
                content_hints: &["#include", "#define", "int main", "printf"],
            },
            _ => LanguageDescriptor {
                id: LanguageId::Cpp,
                extensions: &["cc", "cpp", "cxx", "c++", "hh", "hpp", "hxx", "ipp"],
                project_markers: &["CMakeLists.txt", "Makefile", "meson.build"],
                file_names: &[],
                shebangs: &[],
                content_hints: &["#include", "std::", "namespace ", "template<"],
            },
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
        c_symbols(text)
    }

    fn definition(
        &self,
        text: &str,
        line: usize,
        col: usize,
    ) -> Option<crate::language::symbols::Symbol> {
        let word = crate::language::symbols::word_at(text, line, col)?;
        c_symbols(text)
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

    fn format(&self, path: &Path, text: &str) -> FormatOutcome {
        crate::language::format::clang_format(path, text)
    }

    fn formatter(&self) -> Option<&'static str> {
        Some("clang-format")
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = c_symbols(text);
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

        // Finish a block comment carried over from the previous line.
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

        // A preprocessor line reads as one distinct colour.
        if let Some(first) = chars.iter().position(|c| !c.is_whitespace())
            && chars[first] == '#'
        {
            push_merged(
                &mut spans,
                HighlightSpan::new(first, len, TokenKind::Attribute),
            );
            return (spans, HighlightState::default());
        }

        while i < len {
            let c = chars[i];

            if c == '/' && chars.get(i + 1) == Some(&'/') {
                push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                break;
            }

            if c == '/' && chars.get(i + 1) == Some(&'*') {
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

            if is_ident_start(c) {
                let mut j = i;
                while j < len && is_ident_continue(chars[j]) {
                    j += 1;
                }
                let word: String = chars[i..j].iter().collect();
                push_merged(&mut spans, HighlightSpan::new(i, j, classify_word(&word)));

                // `struct Name`, `class Name`, `enum Name`, `union Name`.
                if matches!(word.as_str(), "struct" | "class" | "enum" | "union")
                    && let Some((start, end)) = name_after(&chars, j)
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

/// The identifier following a `struct`/`class`/`enum`/`union`, skipping spaces.
fn name_after(chars: &[char], from: usize) -> Option<(usize, usize)> {
    let mut j = from;
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }
    let start = j;
    if start >= chars.len() || !is_ident_start(chars[start]) {
        return None;
    }
    let mut end = start;
    while end < chars.len() && is_ident_continue(chars[end]) {
        end += 1;
    }
    Some((start, end))
}

/// End of a `*/` run starting at `from` (past the opening `/*`).
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

/// Named declarations in a document.
fn c_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let start = skip_ws(&chars, 0);
        if start >= chars.len() {
            continue;
        }

        // `#define NAME`.
        if chars[start] == '#' {
            let after_hash = skip_ws(&chars, start + 1);
            let (directive, end) = word_at(&chars, after_hash);
            if directive == "define" {
                let name_start = skip_ws(&chars, end);
                let (name, _) = word_at(&chars, name_start);
                if !name.is_empty() {
                    symbols.push(Symbol::new(name, SymbolKind::Constant, row, name_start));
                }
            }
            continue;
        }

        let (first, after_first) = word_at(&chars, start);
        match first.as_str() {
            "struct" | "union" | "class" => {
                if let Some((name, col)) = name_at(&chars, after_first) {
                    symbols.push(Symbol::new(name, SymbolKind::Struct, row, col));
                }
            }
            "enum" => {
                if let Some((name, col)) = name_at(&chars, after_first) {
                    symbols.push(Symbol::new(name, SymbolKind::Enum, row, col));
                }
            }
            "typedef" => {
                if let Some((name, col)) = last_identifier(&chars, ';') {
                    symbols.push(Symbol::new(name, SymbolKind::Type, row, col));
                }
            }
            _ => {
                if let Some((name, col)) = function_name(&chars) {
                    symbols.push(Symbol::new(name, SymbolKind::Function, row, col));
                }
            }
        }
    }
    symbols
}

/// The identifier starting at `i` and the index just past it.
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

fn name_at(chars: &[char], from: usize) -> Option<(String, usize)> {
    let start = skip_ws(chars, from);
    let (name, _) = word_at(chars, start);
    (!name.is_empty()).then_some((name, start))
}

fn skip_ws(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

/// The last identifier before `terminator`, used for `typedef` names.
fn last_identifier(chars: &[char], terminator: char) -> Option<(String, usize)> {
    let end = chars
        .iter()
        .position(|c| *c == terminator)
        .unwrap_or(chars.len());
    let mut best = None;
    let mut i = 0;
    while i < end {
        if is_ident_start(chars[i]) {
            let mut j = i;
            while j < end && is_ident_continue(chars[j]) {
                j += 1;
            }
            best = Some((chars[i..j].iter().collect::<String>(), i));
            i = j;
        } else {
            i += 1;
        }
    }
    best
}

/// A plausible function name: an identifier immediately before `(` with a
/// type-like token before it, and not a control keyword or a member access.
fn function_name(chars: &[char]) -> Option<(String, usize)> {
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '(' {
            i += 1;
            continue;
        }
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
            let plausible_type = before
                .is_some_and(|b| is_ident_continue(chars[b]) || matches!(chars[b], '*' | '&'));
            let member = before.is_some_and(|b| matches!(chars[b], '.'));
            if plausible_type && !member && !CONTROL.contains(&name.as_str()) {
                return Some((name, start));
            }
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

fn is_operator(c: char) -> bool {
    matches!(
        c,
        '+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | '~' | '?' | ':'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_claim_the_right_extensions() {
        let c = CProvider::c().descriptor();
        assert_eq!(c.id, LanguageId::C);
        assert!(c.extensions.contains(&"c") && c.extensions.contains(&"h"));
        let cpp = CProvider::cpp().descriptor();
        assert_eq!(cpp.id, LanguageId::Cpp);
        assert!(cpp.extensions.contains(&"cpp") && cpp.extensions.contains(&"hpp"));
    }

    #[test]
    fn highlights_preprocessor_strings_and_keywords() {
        let provider = CProvider::c();
        let kind_at = |spans: &[HighlightSpan], col: usize| {
            spans
                .iter()
                .find(|span| span.range.contains(&col))
                .map(|span| span.kind)
        };

        let (spans, _) = provider.highlight("#include <stdio.h>", HighlightState::default());
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Attribute));

        let (spans, _) = provider.highlight("int main() { return 0; }", HighlightState::default());
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Type)); // int
        assert_eq!(kind_at(&spans, 14), Some(TokenKind::Keyword)); // return
    }

    #[test]
    fn carries_block_comments_across_lines() {
        let provider = CProvider::cpp();
        let (_, state) = provider.highlight("/* open", HighlightState::default());
        assert!(state.in_block_comment);
        let (spans, state) = provider.highlight(" still */ int x = 1;", state);
        assert!(!state.in_block_comment);
        assert!(
            spans
                .iter()
                .any(|span| span.kind == TokenKind::Comment && span.range.contains(&0))
        );
    }

    #[test]
    fn extracts_declarations() {
        let text = "#define MAX 10\n\
                    struct Point { int x; };\n\
                    enum Color { RED };\n\
                    typedef unsigned long ulong;\n\
                    int add(int a, int b) {\n\
                        return a + b;\n\
                    }\n";
        let symbols = c_symbols(text);
        let names: Vec<_> = symbols
            .iter()
            .map(|symbol| (symbol.name.as_str(), symbol.kind))
            .collect();
        assert!(names.contains(&("MAX", SymbolKind::Constant)));
        assert!(names.contains(&("Point", SymbolKind::Struct)));
        assert!(names.contains(&("Color", SymbolKind::Enum)));
        assert!(names.contains(&("ulong", SymbolKind::Type)));
        assert!(names.contains(&("add", SymbolKind::Function)));
    }
}
