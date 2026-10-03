//! HTML provider.
//!
//! Built-in and offline: tag, attribute and entity highlighting, a tag-balance
//! diagnostic, `id` symbols, completion and hover. `vscode-html-language-server`
//! (from `vscode-langservers-extracted`) is provisioned for formatting and
//! richer, schema-aware intelligence.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::detection::LanguageDescriptor;
use crate::language::diagnostics::{Diagnostic, Severity, TextPos};
use crate::language::format::FormatOutcome;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Symbol, SymbolKind};

use std::path::Path;

const TAGS: &[&str] = &[
    "a", "article", "aside", "body", "button", "canvas", "code", "div", "em", "fieldset", "footer",
    "form", "h1", "h2", "h3", "h4", "h5", "h6", "head", "header", "html", "img", "input", "label",
    "li", "link", "main", "meta", "nav", "ol", "option", "p", "pre", "script", "section", "select",
    "small", "span", "strong", "style", "table", "tbody", "td", "textarea", "tfoot", "th", "thead",
    "title", "tr", "ul", "video",
];

const ATTRIBUTES: &[&str] = &[
    "alt",
    "aria-label",
    "class",
    "content",
    "data-",
    "disabled",
    "for",
    "height",
    "href",
    "id",
    "lang",
    "method",
    "name",
    "placeholder",
    "rel",
    "role",
    "src",
    "style",
    "target",
    "type",
    "value",
    "width",
];

/// Elements that never need a closing tag.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

pub struct HtmlProvider;

impl LanguageProvider for HtmlProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Html
    }

    fn display_name(&self) -> &'static str {
        "HTML"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Html,
            extensions: &["html", "htm", "xhtml"],
            project_markers: &[],
            file_names: &[],
            shebangs: &[],
            content_hints: &["<!doctype", "<html", "</", "<div"],
        }
    }

    fn capabilities(&self) -> &'static [Capability] {
        &[
            Capability::SyntaxHighlighting,
            Capability::Diagnostics,
            Capability::DocumentSymbols,
            Capability::Completion,
            Capability::Hover,
        ]
    }

    fn diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        unbalanced_tags(text)
    }

    fn symbols(&self, text: &str) -> Vec<Symbol> {
        html_symbols(text)
    }

    fn completions(&self, _text: &str, _line: usize, _col: usize) -> Vec<Completion> {
        let mut completions: Vec<Completion> = TAGS
            .iter()
            .map(|tag| Completion::new(*tag, CompletionKind::Keyword))
            .collect();
        completions.extend(
            ATTRIBUTES
                .iter()
                .map(|attribute| Completion::new(*attribute, CompletionKind::Variable)),
        );
        completions
    }

    fn format(&self, _path: &Path, _text: &str) -> FormatOutcome {
        FormatOutcome::Unsupported
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = html_symbols(text);
        crate::language::hover::describe(text, line, col, &symbols)
    }

    fn line_comment(&self) -> &'static str {
        "<!--"
    }

    fn highlight(&self, line: &str, state: HighlightState) -> (Vec<HighlightSpan>, HighlightState) {
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let mut spans = Vec::new();
        let mut i = 0;

        // Resume an HTML comment.
        if state.in_block_comment {
            match find_comment_end(&chars, 0) {
                Some(end) => {
                    push(&mut spans, 0, end, TokenKind::Comment);
                    i = end;
                }
                None => {
                    if len > 0 {
                        push(&mut spans, 0, len, TokenKind::Comment);
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
            if chars[i] == '<' && chars.get(i + 1) == Some(&'!') && chars.get(i + 2) == Some(&'-') {
                match find_comment_end(&chars, i + 4) {
                    Some(end) => {
                        push(&mut spans, i, end, TokenKind::Comment);
                        i = end;
                    }
                    None => {
                        push(&mut spans, i, len, TokenKind::Comment);
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
            if chars[i] == '<' {
                i = highlight_tag(&chars, i, &mut spans);
                continue;
            }
            if chars[i] == '&'
                && let Some(end) = chars[i..]
                    .iter()
                    .position(|c| *c == ';')
                    .map(|off| i + off + 1)
                && end - i <= 10
            {
                push(&mut spans, i, end, TokenKind::Constant);
                i = end;
                continue;
            }
            i += 1;
        }

        (spans, HighlightState::default())
    }
}

fn push(spans: &mut Vec<HighlightSpan>, start: usize, end: usize, kind: TokenKind) {
    if end > start {
        spans.push(HighlightSpan::new(start, end, kind));
    }
}

fn find_comment_end(chars: &[char], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 2 < chars.len() {
        if chars[i] == '-' && chars[i + 1] == '-' && chars[i + 2] == '>' {
            return Some(i + 3);
        }
        i += 1;
    }
    None
}

/// Highlight a `<...>` tag starting at `start`, returning the index past it.
fn highlight_tag(chars: &[char], start: usize, spans: &mut Vec<HighlightSpan>) -> usize {
    let len = chars.len();
    let mut i = start + 1;
    let closing = chars.get(i) == Some(&'/');
    if closing {
        i += 1;
    }

    // Doctype / declaration.
    if chars.get(i) == Some(&'!') {
        let end = chars[i..]
            .iter()
            .position(|c| *c == '>')
            .map(|off| i + off + 1)
            .unwrap_or(len);
        push(spans, start, end, TokenKind::Attribute);
        return end;
    }

    push(spans, start, i, TokenKind::Keyword);
    let name_start = i;
    while i < len && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == ':') {
        i += 1;
    }
    push(spans, name_start, i, TokenKind::Type);

    // Attributes and their values.
    while i < len && chars[i] != '>' {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        if chars[i] == '/' {
            push(spans, i, i + 1, TokenKind::Keyword);
            i += 1;
            continue;
        }
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            let mut j = i + 1;
            while j < len && chars[j] != quote {
                j += 1;
            }
            let end = (j + 1).min(len);
            push(spans, i, end, TokenKind::String);
            i = end;
            continue;
        }
        if chars[i] == '=' {
            push(spans, i, i + 1, TokenKind::Operator);
            i += 1;
            continue;
        }
        let attr_start = i;
        while i < len
            && !chars[i].is_whitespace()
            && !matches!(chars[i], '=' | '>' | '/' | '"' | '\'')
        {
            i += 1;
        }
        push(spans, attr_start, i, TokenKind::Attribute);
    }
    if i < len {
        push(spans, i, i + 1, TokenKind::Keyword);
        i += 1;
    }
    i
}

/// `id="…"` values, so the outline and navigation can reach anchors.
fn html_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let lower = line.to_ascii_lowercase();
        let mut from = 0;
        while let Some(offset) = lower[from..].find("id=") {
            let at = from + offset;
            let rest = &line[at + 3..];
            let mut chars = rest.chars();
            let quote = chars.next().unwrap_or(' ');
            if quote != '"' && quote != '\'' {
                from = at + 3;
                continue;
            }
            let value: String = chars.take_while(|c| *c != quote).collect();
            // The column of the value, measured in characters.
            let value_col = line[..at + 3].chars().count() + 1;
            if !value.is_empty() {
                symbols.push(Symbol::new(value, SymbolKind::Key, row, value_col));
            }
            from = at + 3;
        }
    }
    symbols
}

/// Report tags that are opened but never closed (void elements excepted).
fn unbalanced_tags(text: &str) -> Vec<Diagnostic> {
    let mut stack: Vec<(String, TextPos)> = Vec::new();
    let mut diagnostics = Vec::new();
    let mut in_comment = false;

    for (row, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if in_comment {
                if let Some(off) = line[i..].find("-->") {
                    i += off + 3;
                    in_comment = false;
                } else {
                    break;
                }
                continue;
            }
            if line[i..].starts_with("<!--") {
                in_comment = true;
                i += 4;
                continue;
            }
            if chars[i] != '<' {
                i += 1;
                continue;
            }
            // Read the tag name and whether it closes / self-closes.
            let mut j = i + 1;
            let closing = chars.get(j) == Some(&'/');
            if closing {
                j += 1;
            }
            let name_start = j;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '-') {
                j += 1;
            }
            if j == name_start {
                i += 1;
                continue;
            }
            let name: String = chars[name_start..j]
                .iter()
                .collect::<String>()
                .to_ascii_lowercase();
            let end = line[j..]
                .find('>')
                .map(|off| j + off)
                .unwrap_or(chars.len() - 1);
            let self_closing = end > 0 && chars.get(end.wrapping_sub(1)) == Some(&'/');
            let attrs: String = chars[j..end].iter().collect();

            if VOID.contains(&name.as_str()) || self_closing || attrs.trim_start().starts_with('!')
            {
                i = end + 1;
                continue;
            }
            if closing {
                match stack.pop() {
                    Some((open, _)) if open == name => {}
                    Some((open, pos)) => {
                        // A mismatched close; report it without losing the opener.
                        diagnostics.push(Diagnostic::new(
                            TextPos::new(row, i),
                            TextPos::new(row, (i + 1).max(1)),
                            Severity::Error,
                            format!("</{name}> closes <{open}>"),
                        ));
                        stack.push((open, pos));
                    }
                    None => diagnostics.push(Diagnostic::new(
                        TextPos::new(row, i),
                        TextPos::new(row, (i + 1).max(1)),
                        Severity::Error,
                        format!("stray closing </{name}>"),
                    )),
                }
            } else {
                stack.push((name, TextPos::new(row, i)));
            }
            i = end + 1;
        }
    }

    for (name, pos) in stack {
        diagnostics.push(Diagnostic::new(
            pos,
            TextPos::new(pos.line, pos.col + 1),
            Severity::Warning,
            format!("<{name}> is never closed"),
        ));
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_claims_html() {
        let descriptor = HtmlProvider.descriptor();
        assert_eq!(descriptor.id, LanguageId::Html);
        assert!(descriptor.extensions.contains(&"html"));
    }

    #[test]
    fn highlights_tags_and_attributes() {
        let kind_at = |spans: &[HighlightSpan], col: usize| {
            spans
                .iter()
                .find(|span| span.range.contains(&col))
                .map(|span| span.kind)
        };
        let (spans, _) = HtmlProvider.highlight(
            "<a href=\"/x\" class=\"y\">hi</a>",
            HighlightState::default(),
        );
        assert_eq!(kind_at(&spans, 1), Some(TokenKind::Type)); // a
        assert_eq!(kind_at(&spans, 3), Some(TokenKind::Attribute)); // href
        assert_eq!(kind_at(&spans, 8), Some(TokenKind::String)); // "/x"
    }

    #[test]
    fn reports_unclosed_and_mismatched_tags() {
        let diagnostics = unbalanced_tags("<div>\n<span></div>\n");
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("never closed")),
            "expected an unclosed tag: {diagnostics:?}"
        );
    }

    #[test]
    fn extracts_ids() {
        let symbols = html_symbols("<section id=\"intro\">\n<div id='outro'></div>\n");
        let names: Vec<_> = symbols.into_iter().map(|symbol| symbol.name).collect();
        assert!(names.contains(&"intro".to_string()));
        assert!(names.contains(&"outro".to_string()));
    }
}
