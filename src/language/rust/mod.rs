//! Rust language provider.
//!
//! The highlighter is a small, purpose-built scanner rather than a full parser.
//! It is intentionally conservative: it recognises the constructs that matter for
//! readable code and stays fast and maintainable.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::detection::LanguageDescriptor;
use crate::language::diagnostics::Diagnostic;
use crate::language::format::{FormatOutcome, rustfmt};
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};

use std::path::Path;

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "static", "struct", "super", "trait", "type", "unsafe", "use", "where", "while",
    "yield", "union", "default", "macro",
];

const TYPES: &[&str] = &[
    "bool", "char", "str", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64",
    "u128", "usize", "f32", "f64", "String", "Vec", "Option", "Result", "Box", "Rc", "Arc", "Cell",
    "RefCell", "HashMap", "HashSet", "BTreeMap", "BTreeSet", "Cow", "Self",
];

const CONSTANTS: &[&str] = &["true", "false", "None", "Some", "Ok", "Err"];

pub struct RustProvider;

impl LanguageProvider for RustProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Rust
    }

    fn display_name(&self) -> &'static str {
        "Rust"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Rust,
            extensions: &["rs"],
            project_markers: &["Cargo.toml", "Cargo.lock"],
            file_names: &[],
            shebangs: &[],
            content_hints: &["fn ", "let mut ", "impl ", "use std", "pub struct", "-> "],
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
            Capability::CodeActions,
        ]
    }

    fn diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        crate::language::diagnostics::check_delimiters(self, text)
    }

    fn symbols(&self, text: &str) -> Vec<crate::language::symbols::Symbol> {
        crate::language::symbols::rust_symbols(text)
    }

    fn definition(
        &self,
        text: &str,
        line: usize,
        col: usize,
    ) -> Option<crate::language::symbols::Symbol> {
        let word = crate::language::symbols::word_at(text, line, col)?;
        crate::language::symbols::rust_symbols(text)
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
            CONSTANTS
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Constant)),
        );
        completions
    }

    fn format(&self, path: &Path, text: &str) -> FormatOutcome {
        rustfmt(path, text)
    }

    fn formatter(&self) -> Option<&'static str> {
        Some("rustfmt")
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = crate::language::symbols::rust_symbols(text);
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

        // Finish a block comment carried over from the previous line. Rust block
        // comments nest, so the carried depth decides where the comment really ends.
        if state.in_block_comment {
            let (end, depth) = scan_block_comment(&chars, 0, state.block_comment_depth);
            if end > 0 {
                spans.push(HighlightSpan::new(0, end, TokenKind::Comment));
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

        while i < len {
            let c = chars[i];

            // Block comment; Rust nests `/* */`, so the depth must balance.
            if c == '/' && i + 1 < len && chars[i + 1] == '*' {
                let (end, depth) = scan_block_comment(&chars, i, 0);
                spans.push(HighlightSpan::new(i, end, TokenKind::Comment));
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

            // Line comment.
            if c == '/' && i + 1 < len && chars[i + 1] == '/' {
                spans.push(HighlightSpan::new(i, len, TokenKind::Comment));
                break;
            }

            // Attribute: #[...] or #![...]
            if c == '#'
                && i + 1 < len
                && (chars[i + 1] == '['
                    || (chars[i + 1] == '!' && i + 2 < len && chars[i + 2] == '['))
            {
                let mut j = i + 1;
                let mut depth = 0usize;
                while j < len {
                    match chars[j] {
                        '[' => depth += 1,
                        ']' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                j += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                spans.push(HighlightSpan::new(i, j.min(len), TokenKind::Attribute));
                i = j.min(len);
                continue;
            }

            // Byte string, byte literal and raw byte string: b"…", b'…', br"…",
            // br#"…"#. Include the prefix in the string span.
            if c == 'b' && i + 1 < len {
                match chars[i + 1] {
                    '"' => {
                        let end = scan_quoted(&chars, i + 1, '"');
                        spans.push(HighlightSpan::new(i, end, TokenKind::String));
                        i = end;
                        continue;
                    }
                    '\'' => {
                        if let Some(end) = scan_char_literal(&chars, i + 1) {
                            spans.push(HighlightSpan::new(i, end, TokenKind::String));
                            i = end;
                            continue;
                        }
                    }
                    'r' if i + 2 < len && (chars[i + 2] == '"' || chars[i + 2] == '#') => {
                        if let Some(end) = scan_raw_string(&chars, i + 1) {
                            spans.push(HighlightSpan::new(i, end, TokenKind::String));
                            i = end;
                            continue;
                        }
                    }
                    _ => {}
                }
            }

            // Raw string: r"..." or r#"..."# / r##"..."##
            if c == 'r'
                && i + 1 < len
                && (chars[i + 1] == '"' || chars[i + 1] == '#')
                && let Some(end) = scan_raw_string(&chars, i)
            {
                spans.push(HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            // String literal.
            if c == '"' {
                let end = scan_quoted(&chars, i, '"');
                spans.push(HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            // Char literal, or a lifetime/label such as `'a`, `'static`, `'_`.
            if c == '\'' {
                if let Some(end) = scan_char_literal(&chars, i) {
                    spans.push(HighlightSpan::new(i, end, TokenKind::String));
                    i = end;
                    continue;
                }
                if i + 1 < len && is_ident_start(chars[i + 1]) {
                    let mut j = i + 1;
                    while j < len && is_ident_continue(chars[j]) {
                        j += 1;
                    }
                    push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Type));
                    i = j;
                    continue;
                }
                spans.push(HighlightSpan::new(i, i + 1, TokenKind::Operator));
                i += 1;
                continue;
            }

            // Number literal.
            if c.is_ascii_digit() {
                let end = scan_number(&chars, i);
                spans.push(HighlightSpan::new(i, end, TokenKind::Number));
                i = end;
                continue;
            }

            // Identifier or keyword.
            if is_ident_start(c) {
                let mut j = i;
                while j < len && is_ident_continue(chars[j]) {
                    j += 1;
                }
                let word: String = chars[i..j].iter().collect();
                let kind = classify_word(&word, chars.get(j) == Some(&'!'));
                push_merged(&mut spans, HighlightSpan::new(i, j, kind));
                i = j;
                continue;
            }

            // Operators and punctuation.
            if is_operator(c) {
                spans.push(HighlightSpan::new(i, i + 1, TokenKind::Operator));
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

fn classify_word(word: &str, followed_by_bang: bool) -> TokenKind {
    if KEYWORDS.contains(&word) {
        return TokenKind::Keyword;
    }
    if TYPES.contains(&word) {
        return TokenKind::Type;
    }
    if CONSTANTS.contains(&word) {
        return TokenKind::Constant;
    }
    if followed_by_bang {
        return TokenKind::Macro;
    }
    if word.chars().next().is_some_and(char::is_uppercase)
        && word.chars().all(|c| !c.is_lowercase())
    {
        return TokenKind::Constant;
    }
    TokenKind::Plain
}

/// Scan a block comment from `start` with `depth` levels already open, returning
/// the index just past the character that closed the outermost comment (or the
/// line length if it stays open) and the depth still open.
///
/// Rust block comments nest, so `/* a /* b */ c */` closes only at the last `*/`;
/// the same depth is carried across lines through
/// [`HighlightState::block_comment_depth`].
fn scan_block_comment(chars: &[char], start: usize, depth: u8) -> (usize, u8) {
    let mut i = start;
    let mut depth = depth;
    while i < chars.len() {
        if i + 1 < chars.len() {
            match (chars[i], chars[i + 1]) {
                ('/', '*') => {
                    depth = depth.saturating_add(1);
                    i += 2;
                    continue;
                }
                ('*', '/') => {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        return (i, 0);
                    }
                    continue;
                }
                _ => {}
            }
        }
        i += 1;
    }
    (chars.len(), depth)
}

fn scan_quoted(chars: &[char], start: usize, quote: char) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            c if c == quote => return (i + 1).min(chars.len()),
            _ => i += 1,
        }
    }
    chars.len()
}

fn scan_char_literal(chars: &[char], start: usize) -> Option<usize> {
    // Forms: 'a'  '\n'  '\u{1F600}'
    if start + 2 < chars.len() && chars[start + 1] == '\\' {
        let mut i = start + 2;
        while i < chars.len() && chars[i] != '\'' {
            i += 1;
        }
        if i < chars.len() {
            return Some(i + 1);
        }
        return None;
    }
    if start + 2 < chars.len() && chars[start + 2] == '\'' {
        return Some(start + 3);
    }
    None
}

fn scan_raw_string(chars: &[char], start: usize) -> Option<usize> {
    let mut hashes = 0;
    let mut i = start + 1;
    while i < chars.len() && chars[i] == '#' {
        hashes += 1;
        i += 1;
    }
    if i >= chars.len() || chars[i] != '"' {
        return None;
    }
    i += 1;
    while i < chars.len() {
        if chars[i] == '"' {
            // Check for the matching hash terminator.
            let mut ok = true;
            for k in 0..hashes {
                if chars.get(i + 1 + k) != Some(&'#') {
                    ok = false;
                    break;
                }
            }
            if ok {
                return Some(i + 1 + hashes);
            }
        }
        i += 1;
    }
    Some(chars.len())
}

fn scan_number(chars: &[char], start: usize) -> usize {
    let mut i = start;
    if chars[i] == '0'
        && i + 1 < chars.len()
        && matches!(chars[i + 1], 'x' | 'X' | 'b' | 'B' | 'o' | 'O')
    {
        i += 2;
        while i < chars.len()
            && (chars[i].is_ascii_hexdigit() || chars[i] == '_' || chars[i] == '.')
        {
            i += 1;
        }
        if i < chars.len() && matches!(chars[i], 'p' | 'P') {
            i = scan_exponent(chars, i);
        }
        while i < chars.len() && chars[i].is_ascii_alphanumeric() {
            i += 1;
        }
        return i;
    }
    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
        i += 1;
    }
    // A single `.` makes a float; a second one starts a range such as `1..=2`.
    if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1] != '.' {
        i += 1;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
            i += 1;
        }
    }
    if i < chars.len() && matches!(chars[i], 'e' | 'E') {
        i = scan_exponent(chars, i);
    }
    // A type suffix such as `u32`, `f64` or `i8`.
    while i < chars.len() && chars[i].is_ascii_alphanumeric() {
        i += 1;
    }
    i
}

/// Consume an exponent marker (`e`/`E`/`p`/`P`), an optional sign and its digits.
fn scan_exponent(chars: &[char], marker: usize) -> usize {
    let mut i = marker + 1;
    if i < chars.len() && matches!(chars[i], '+' | '-') {
        i += 1;
    }
    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
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
    fn highlights_keywords_and_strings() {
        let (spans, _) = RustProvider.highlight("let x = \"hi\";", HighlightState::default());
        assert!(spans.iter().any(|s| s.kind == TokenKind::Keyword));
        assert!(spans.iter().any(|s| s.kind == TokenKind::String));
    }

    #[test]
    fn block_comment_carries_state() {
        let (_, state) = RustProvider.highlight("/* start", HighlightState::default());
        assert!(state.in_block_comment);
        let (spans, state) = RustProvider.highlight("still here */ code", state);
        assert!(!state.in_block_comment);
        assert_eq!(spans[0].kind, TokenKind::Comment);
    }

    #[test]
    fn nested_block_comments_close_only_at_the_outer_end() {
        let line = "/* a /* b */ c */ let x";
        let (spans, state) = RustProvider.highlight(line, HighlightState::default());
        assert!(!state.in_block_comment);
        let outer_end = line.rfind("*/").expect("the line ends a comment") + 2;
        assert_eq!(spans[0].kind, TokenKind::Comment);
        assert_eq!(spans[0].range.end, outer_end);
        assert!(spans.iter().any(|span| span.kind == TokenKind::Keyword));
    }

    #[test]
    fn nested_block_comment_depth_carries_across_lines() {
        let (spans, state) = RustProvider.highlight("/* outer /* inner", HighlightState::default());
        assert!(state.in_block_comment);
        assert_eq!(state.block_comment_depth, 2);
        assert_eq!(spans[0].kind, TokenKind::Comment);

        let (spans, state) = RustProvider.highlight("still */ outer */ let x", state);
        assert!(!state.in_block_comment);
        assert_eq!(state.block_comment_depth, 0);
        assert_eq!(spans[0].kind, TokenKind::Comment);
        // The `let` after the comment is code, not swallowed by the comment.
        assert!(spans.iter().any(|span| span.kind == TokenKind::Keyword));
    }

    #[test]
    fn byte_strings_and_byte_literals_are_single_string_spans() {
        for source in ["b\"bytes\"", "b'x'", "br\"raw\"", "br#\"raw # hash\"#"] {
            let (spans, _) = RustProvider.highlight(source, HighlightState::default());
            let len = source.chars().count();
            assert!(
                spans
                    .iter()
                    .any(|span| span.kind == TokenKind::String && span.range == (0..len)),
                "{source} was not highlighted as one string span: {spans:?}"
            );
        }
    }

    #[test]
    fn lifetimes_are_not_read_as_char_literals() {
        let (spans, _) = RustProvider.highlight("fn f<'a>(x: &'a str)", HighlightState::default());
        assert!(
            spans
                .iter()
                .any(|span| span.kind == TokenKind::Type && span.range == (5..7)),
            "the lifetime 'a should be a type span: {spans:?}"
        );

        // A real char literal stays a string, and adds no lifetime span.
        let (spans, _) = RustProvider.highlight("let c = 'x';", HighlightState::default());
        assert!(spans.iter().any(|span| span.kind == TokenKind::String));
        assert!(!spans.iter().any(|span| span.kind == TokenKind::Type));
    }

    #[test]
    fn numeric_exponents_and_ranges_are_scanned() {
        for (source, expected) in [
            ("1e10", "1e10"),
            ("1e-5", "1e-5"),
            ("1.5e+3f64", "1.5e+3f64"),
            ("0x1p-2", "0x1p-2"),
            ("0xFFu8", "0xFFu8"),
        ] {
            let (spans, _) = RustProvider.highlight(source, HighlightState::default());
            let number = spans.iter().find(|span| span.kind == TokenKind::Number);
            assert_eq!(
                number.map(|span| &source[span.range.clone()]),
                Some(expected),
                "{source} was not one number: {spans:?}"
            );
        }

        // `1..=2` is a range, so the number stops before the dots.
        let (spans, _) = RustProvider.highlight("1..=2", HighlightState::default());
        assert_eq!(spans[0].range, 0..1);
    }
}
