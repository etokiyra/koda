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
    Capability, Embed, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Symbol, SymbolKind};

use crate::language::css::CssProvider;
use crate::language::web::WebProvider;

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
            Capability::Formatting,
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

    fn format(&self, path: &Path, text: &str) -> FormatOutcome {
        crate::language::format::prettier(path, text)
    }

    fn formatter(&self) -> Option<&'static str> {
        Some("prettier")
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = html_symbols(text);
        crate::language::hover::describe(text, line, col, &symbols)
    }

    fn line_comment(&self) -> &'static str {
        "<!--"
    }

    fn highlight(&self, line: &str, state: HighlightState) -> (Vec<HighlightSpan>, HighlightState) {
        // Inside an open `<script>`/`<style>` the line belongs to the embedded
        // language, not HTML.
        if state.embed != Embed::None {
            return self.highlight_embedded(line, state);
        }

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
                            ..Default::default()
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
                                ..Default::default()
                            },
                        );
                    }
                }
                continue;
            }
            if chars[i] == '<' {
                let embed = open_embed(&chars, i);
                let end = highlight_tag(&chars, i, &mut spans);
                if let Some(kind) = embed {
                    // Hand the rest of the line to the embedded tokenizer.
                    let rest: String = chars[end..].iter().collect();
                    let (embedded, next) = self.highlight_embedded(
                        &rest,
                        HighlightState {
                            embed: kind,
                            ..Default::default()
                        },
                    );
                    for span in embedded {
                        push(
                            &mut spans,
                            end + span.range.start,
                            end + span.range.end,
                            span.kind,
                        );
                    }
                    return (spans, next);
                }
                i = end;
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

impl HtmlProvider {
    /// Tokenize a line inside an open `<script>`/`<style>` element.
    ///
    /// The embedded body is handed to the JavaScript or CSS tokenizer, which
    /// carries its own state between lines. The closing tag ends the region and
    /// the rest of the line is parsed as HTML again.
    fn highlight_embedded(
        &self,
        line: &str,
        state: HighlightState,
    ) -> (Vec<HighlightSpan>, HighlightState) {
        let (kind, close) = match state.embed {
            Embed::Script => (Embed::Script, "</script"),
            Embed::Style => (Embed::Style, "</style"),
            Embed::None => return (Vec::new(), state),
        };
        let lower = line.to_ascii_lowercase();
        let split = lower.find(close);
        let body = match split {
            Some(at) => &line[..at],
            None => line,
        };
        let sub_state = HighlightState {
            in_block_comment: state.in_block_comment,
            block_comment_depth: state.block_comment_depth,
            mode: state.mode,
            embed: Embed::None,
        };
        let (mut spans, embedded) = match kind {
            Embed::Script => WebProvider::javascript().highlight(body, sub_state),
            Embed::Style => CssProvider.highlight(body, sub_state),
            Embed::None => unreachable!(),
        };
        match split {
            None => (
                spans,
                HighlightState {
                    in_block_comment: embedded.in_block_comment,
                    block_comment_depth: embedded.block_comment_depth,
                    mode: embedded.mode,
                    embed: kind,
                },
            ),
            Some(at) => {
                // Re-parse the closing tag and anything after it as HTML. The
                // byte offset is converted to characters for the span shift.
                let offset = line[..at].chars().count();
                let (tail, tail_state) = self.highlight(&line[at..], HighlightState::default());
                for span in tail {
                    push(
                        &mut spans,
                        offset + span.range.start,
                        offset + span.range.end,
                        span.kind,
                    );
                }
                (spans, tail_state)
            }
        }
    }
}

/// Whether `<` at `start` opens a `<script>` or `<style>` element, unless it is
/// a closing tag or self-closing.
fn open_embed(chars: &[char], start: usize) -> Option<Embed> {
    let mut i = start + 1;
    if chars.get(i) == Some(&'/') {
        return None;
    }
    let name_start = i;
    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == ':') {
        i += 1;
    }
    let name: String = chars[name_start..i]
        .iter()
        .collect::<String>()
        .to_ascii_lowercase();
    let embed = match name.as_str() {
        "script" => Embed::Script,
        "style" => Embed::Style,
        _ => return None,
    };
    // A self-closing `<script .../>` has no body.
    let mut j = i;
    while j < chars.len() && chars[j] != '>' {
        j += 1;
    }
    if j > i && chars[j - 1] == '/' {
        return None;
    }
    Some(embed)
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
///
/// Everything is measured in **characters** into a local `chars` buffer: mixing
/// a character index with a byte slice panics on any line containing non-ASCII
/// text (`é`, CJK, emoji), which is ordinary HTML content.
fn unbalanced_tags(text: &str) -> Vec<Diagnostic> {
    let mut stack: Vec<(String, TextPos)> = Vec::new();
    let mut diagnostics = Vec::new();
    let mut in_comment = false;
    // While inside a `<script>`/`<style>` element its body is raw text: `<` and
    // `>` there are code, not tags.
    let mut raw_tag: Option<String> = None;

    for (row, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if let Some(tag) = raw_tag.clone() {
                let needle: Vec<char> = format!("</{tag}").chars().collect();
                match find_char_seq(&chars, i, &needle) {
                    Some(end) => {
                        // Continue at the closing tag so it is parsed and popped.
                        i = end - needle.len();
                        raw_tag = None;
                        continue;
                    }
                    None => break,
                }
            }
            if in_comment {
                match find_char_seq(&chars, i, &['-', '-', '>']) {
                    Some(end) => {
                        i = end;
                        in_comment = false;
                    }
                    None => break,
                }
                continue;
            }
            if chars[i..].starts_with(&['<', '!', '-', '-']) {
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
            let end = chars[j..]
                .iter()
                .position(|c| *c == '>')
                .map(|off| j + off)
                .unwrap_or(chars.len().saturating_sub(1));
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
                            TextPos::new(row, i + 1),
                            Severity::Error,
                            format!("</{name}> closes <{open}>"),
                        ));
                        stack.push((open, pos));
                    }
                    None => diagnostics.push(Diagnostic::new(
                        TextPos::new(row, i),
                        TextPos::new(row, i + 1),
                        Severity::Error,
                        format!("stray closing </{name}>"),
                    )),
                }
            } else {
                if name == "script" || name == "style" {
                    raw_tag = Some(name.clone());
                }
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

/// The index just past `needle` in `chars` at or after `from`, all in
/// characters. Used so the tag scanner never slices a `&str` by a character
/// offset.
fn find_char_seq(chars: &[char], from: usize, needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > chars.len() || from > chars.len() - needle.len() {
        return None;
    }
    (from..=chars.len() - needle.len())
        .find(|&i| chars[i..].starts_with(needle))
        .map(|i| i + needle.len())
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
    fn embedded_script_is_tokenized_as_javascript() {
        let provider = HtmlProvider;
        // A `<script>` body opens on one line and continues on the next.
        let (spans, state) = provider.highlight("<script>const x = 1;", HighlightState::default());
        assert_eq!(state.embed, Embed::Script);
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Keyword)); // <
        assert_eq!(kind_at(&spans, 8), Some(TokenKind::Keyword)); // const
        assert_eq!(kind_at(&spans, 18), Some(TokenKind::Number)); // 1

        // The next line is JavaScript until `</script>`.
        let (spans, state) = provider.highlight("let y = `a", state);
        assert_eq!(state.embed, Embed::Script);
        assert_eq!(state.mode, crate::language::provider::LexMode::Template);
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Keyword)); // let
    }

    #[test]
    fn embedded_style_closes_and_html_resumes() {
        let provider = HtmlProvider;
        let (spans, state) = provider.highlight(
            "<style>.a { color: red; }</style>",
            HighlightState::default(),
        );
        assert_eq!(state.embed, Embed::None);
        assert_eq!(kind_at(&spans, 1), Some(TokenKind::Type)); // style
        assert!(
            spans
                .iter()
                .any(|span| span.range.start >= 25 && span.range.start < 33),
            "the closing tag must be highlighted as HTML: {spans:?}"
        );
    }

    #[test]
    fn self_closing_script_has_no_body() {
        let provider = HtmlProvider;
        let (_, state) = provider.highlight("<script src=\"a.js\" />", HighlightState::default());
        assert_eq!(state.embed, Embed::None);
    }

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
    fn script_bodies_do_not_confuse_tag_balance() {
        // `<` inside a script is code, not a tag, and must not be reported.
        let diagnostics = unbalanced_tags("<script>\nif (a < b) { x(); }\n</script>\n");
        assert!(diagnostics.is_empty(), "unexpected: {diagnostics:?}");
        let diagnostics = unbalanced_tags("<style>\na > b { color: red }\n</style>\n");
        assert!(diagnostics.is_empty(), "unexpected: {diagnostics:?}");
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

    #[test]
    fn non_ascii_content_does_not_panic() {
        // Regression: the tag scanner measured positions in characters but
        // sliced the line by bytes, so any non-ASCII character could land the
        // index mid-codepoint and panic. These are ordinary documents.
        let documents = [
            "<é></é>",
            "<h1>Café ☕</h1>",
            "<p>こんにちは</p><p>world</p>",
            "<div title=\"café\">emoji 🎉</div>",
            "<!-- commenté -->\n<span>ünïcödé</span>",
        ];
        for document in documents {
            let diagnostics = unbalanced_tags(document);
            // The well-formed documents above have nothing to report.
            assert!(
                diagnostics.is_empty(),
                "unexpected diagnostics for {document:?}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn finds_comment_terminators_in_characters() {
        assert_eq!(
            find_char_seq(&['a', '-', '-', '>'], 0, &['-', '-', '>']),
            Some(4)
        );
        assert_eq!(
            find_char_seq(&['→', '-', '-', '>'], 0, &['-', '-', '>']),
            Some(4)
        );
        assert_eq!(find_char_seq(&['-', '-'], 0, &['-', '-', '>']), None);
    }
}
