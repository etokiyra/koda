//! Markdown language provider.
//!
//! The scanner understands the pieces that make prose readable and navigable:
//! ATX headings, fenced code blocks (carried across lines), blockquotes and list
//! markers, links and images, inline code, emphasis and raw HTML tags. Headings
//! become document symbols, so `Ctrl+Shift+O` outlines a document.

use crate::language::data::push_merged;
use crate::language::detection::LanguageDescriptor;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Symbol, SymbolKind};

pub struct MarkdownProvider;

impl LanguageProvider for MarkdownProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Markdown
    }

    fn display_name(&self) -> &'static str {
        "Markdown"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Markdown,
            extensions: &["md", "markdown", "mdx"],
            project_markers: &[],
            file_names: &[],
            shebangs: &[],
            content_hints: &["```", "](", "## ", "- [ ]", "![", "**"],
        }
    }

    fn capabilities(&self) -> &'static [Capability] {
        &[Capability::SyntaxHighlighting, Capability::DocumentSymbols]
    }

    fn symbols(&self, text: &str) -> Vec<Symbol> {
        markdown_symbols(text)
    }

    fn highlight(&self, line: &str, state: HighlightState) -> (Vec<HighlightSpan>, HighlightState) {
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let mut spans = Vec::new();
        let first = chars.iter().position(|c| !c.is_whitespace());

        // Inside a fenced code block: only a closing fence is special.
        if state.in_block_comment {
            if let Some(start) = first
                && is_fence(&chars, start)
            {
                push_merged(
                    &mut spans,
                    HighlightSpan::new(start, len, TokenKind::Comment),
                );
                return (
                    spans,
                    HighlightState {
                        in_block_comment: false,
                    },
                );
            }
            return (
                spans,
                HighlightState {
                    in_block_comment: true,
                },
            );
        }

        // Opening fence.
        if let Some(start) = first
            && is_fence(&chars, start)
        {
            push_merged(
                &mut spans,
                HighlightSpan::new(start, len, TokenKind::Comment),
            );
            return (
                spans,
                HighlightState {
                    in_block_comment: true,
                },
            );
        }

        // ATX heading: 1–6 `#`, then space (or end of line).
        if let Some(end) = heading_marker(&chars) {
            let start = first.unwrap_or(0);
            push_merged(
                &mut spans,
                HighlightSpan::new(start, end, TokenKind::Keyword),
            );
            if end < len {
                push_merged(&mut spans, HighlightSpan::new(end, len, TokenKind::Type));
            }
            return (spans, HighlightState::default());
        }

        let mut i = first.unwrap_or(0);

        // Blockquote marker.
        if i < len && chars[i] == '>' {
            push_merged(
                &mut spans,
                HighlightSpan::new(i, i + 1, TokenKind::Operator),
            );
            i += 1;
        }

        // List marker.
        if i < len {
            if matches!(chars[i], '-' | '*' | '+') && chars.get(i + 1) == Some(&' ') {
                push_merged(
                    &mut spans,
                    HighlightSpan::new(i, i + 1, TokenKind::Operator),
                );
                i += 1;
            } else if chars[i].is_ascii_digit() {
                let mut d = i;
                while d < len && chars[d].is_ascii_digit() {
                    d += 1;
                }
                if d < len && matches!(chars[d], '.' | ')') && chars.get(d + 1) == Some(&' ') {
                    push_merged(
                        &mut spans,
                        HighlightSpan::new(i, d + 1, TokenKind::Operator),
                    );
                    i = d + 1;
                }
            }
        }

        while i < len {
            let c = chars[i];

            if c == '`' {
                let mut run = 0;
                while i + run < len && chars[i + run] == '`' {
                    run += 1;
                }
                let end = find_code_end(&chars, i + run, run).unwrap_or(len);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            if c == '*' || c == '_' {
                let marker = if chars.get(i + 1) == Some(&c) { 2 } else { 1 };
                if let Some(end) = find_emphasis_end(&chars, i + marker, c, marker) {
                    push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Type));
                    i = end;
                    continue;
                }
                i += marker;
                continue;
            }

            if c == '~' && chars.get(i + 1) == Some(&'~') {
                let end = find_emphasis_end(&chars, i + 2, '~', 2).unwrap_or(i + 2);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Type));
                i = end;
                continue;
            }

            if c == '[' || (c == '!' && chars.get(i + 1) == Some(&'[')) {
                let bracket = if c == '!' { i + 1 } else { i };
                if let Some(close) = find_char(&chars, bracket + 1, ']') {
                    push_merged(
                        &mut spans,
                        HighlightSpan::new(bracket + 1, close, TokenKind::Function),
                    );
                    if chars.get(close + 1) == Some(&'(')
                        && let Some(url_end) = find_char(&chars, close + 2, ')')
                    {
                        push_merged(
                            &mut spans,
                            HighlightSpan::new(close + 2, url_end, TokenKind::String),
                        );
                        i = url_end + 1;
                        continue;
                    }
                    i = close + 1;
                    continue;
                }
            }

            if c == '<'
                && let Some(end) = find_char(&chars, i + 1, '>')
            {
                push_merged(
                    &mut spans,
                    HighlightSpan::new(i, end + 1, TokenKind::Attribute),
                );
                i = end + 1;
                continue;
            }

            if c == '\\' {
                i += 2;
                continue;
            }

            i += 1;
        }

        (spans, HighlightState::default())
    }
}

/// Whether a fence (```` ``` ```` or `~~~`) begins at `start`.
fn is_fence(chars: &[char], start: usize) -> bool {
    let Some(&c) = chars.get(start) else {
        return false;
    };
    if c != '`' && c != '~' {
        return false;
    }
    chars[start..].iter().take_while(|&&ch| ch == c).count() >= 3
}

/// The end of an ATX heading marker at the start of the line, if this is one.
fn heading_marker(chars: &[char]) -> Option<usize> {
    let start = chars.iter().position(|c| !c.is_whitespace())?;
    let mut end = start;
    while end < chars.len() && chars[end] == '#' {
        end += 1;
    }
    let hashes = end - start;
    if (1..=6).contains(&hashes) && chars.get(end).is_none_or(|&c| c == ' ') {
        Some(end)
    } else {
        None
    }
}

fn find_code_end(chars: &[char], from: usize, run: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '`' {
            let mut j = i;
            while j < chars.len() && chars[j] == '`' {
                j += 1;
            }
            if j - i == run {
                return Some(j);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    None
}

fn find_emphasis_end(chars: &[char], from: usize, marker: char, run: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == marker {
            let mut j = i;
            while j < chars.len() && chars[j] == marker {
                j += 1;
            }
            if j - i >= run {
                return Some(i + run);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    None
}

fn find_char(chars: &[char], from: usize, target: char) -> Option<usize> {
    chars[from..]
        .iter()
        .position(|&c| c == target)
        .map(|p| from + p)
}

/// Headings, for the symbol outline and workspace search.
pub fn markdown_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let Some(end) = heading_marker(&chars) else {
            continue;
        };
        // `end` is a character index; the heading prefix is ASCII (spaces and
        // `#`), so the byte offset is safe to derive directly.
        let byte = line
            .char_indices()
            .nth(end)
            .map(|(offset, _)| offset)
            .unwrap_or(line.len());
        let rest = &line[byte..];
        let spaces = rest.chars().take_while(|c| *c == ' ').count();
        let title = rest.trim_start().trim_end_matches('#').trim_end();
        if title.is_empty() {
            continue;
        }
        symbols.push(Symbol::new(title, SymbolKind::Heading, row, end + spaces));
    }
    symbols
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<TokenKind> {
        let (spans, _) = MarkdownProvider.highlight(line, HighlightState::default());
        spans.into_iter().map(|span| span.kind).collect()
    }

    #[test]
    fn headings_highlight_marker_and_text() {
        let (spans, _) = MarkdownProvider.highlight("## Title", HighlightState::default());
        assert_eq!(spans[0].kind, TokenKind::Keyword);
        assert_eq!(spans[1].kind, TokenKind::Type);
        assert_eq!(spans[0].range, 0..2);
    }

    #[test]
    fn fenced_code_blocks_carry_state() {
        let (_, state) = MarkdownProvider.highlight("```rust", HighlightState::default());
        assert!(state.in_block_comment);
        let (spans, state) = MarkdownProvider.highlight("let x = 1;", state);
        assert!(state.in_block_comment);
        assert!(spans.is_empty(), "code inside a fence stays plain");
        let (_, state) = MarkdownProvider.highlight("```", state);
        assert!(!state.in_block_comment);
    }

    #[test]
    fn inline_code_links_and_emphasis() {
        let (spans, _) = MarkdownProvider.highlight(
            "Use `cargo` and [Koda](https://koda.dev) *today*.",
            HighlightState::default(),
        );
        let kinds: Vec<TokenKind> = spans.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&TokenKind::String));
        assert!(kinds.contains(&TokenKind::Function));
        assert!(kinds.contains(&TokenKind::Type));
    }

    #[test]
    fn list_marker_is_punctuation() {
        assert_eq!(kinds("- item"), vec![TokenKind::Operator]);
        assert_eq!(kinds("1. item"), vec![TokenKind::Operator]);
    }

    #[test]
    fn hashtag_without_space_is_not_a_heading() {
        let (spans, _) = MarkdownProvider.highlight("#hashtag", HighlightState::default());
        assert!(spans.is_empty());
    }

    #[test]
    fn symbols_collect_headings() {
        let text = "# Koda\n\ntext\n\n## Install\n\n### Notes\n";
        let symbols = markdown_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Koda", "Install", "Notes"]);
        assert_eq!(symbols[0].kind, SymbolKind::Heading);
        assert_eq!(symbols[1].line, 4);
    }
}
