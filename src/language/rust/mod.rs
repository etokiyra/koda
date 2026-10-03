//! Rust language provider.
//!
//! The highlighter is a small, purpose-built scanner rather than a full parser.
//! It is intentionally conservative: it recognises the constructs that matter for
//! readable code and stays fast and maintainable.

use crate::language::detection::LanguageDescriptor;
use crate::language::diagnostics::Diagnostic;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};

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
        &[Capability::SyntaxHighlighting, Capability::Diagnostics]
    }

    fn diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        crate::language::diagnostics::check_delimiters(self, text)
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

            // Block comment.
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

            // Char literal (but not a lifetime such as `'a`).
            if c == '\'' {
                if let Some(end) = scan_char_literal(&chars, i) {
                    spans.push(HighlightSpan::new(i, end, TokenKind::String));
                    i = end;
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
}
