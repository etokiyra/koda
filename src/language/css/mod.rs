//! CSS provider.
//!
//! Built-in and offline: selector, property, value, colour, at-rule and comment
//! highlighting, brace-balance diagnostics, selector symbols, completion and
//! hover. `vscode-css-language-server` (from `vscode-langservers-extracted`) is
//! provisioned for formatting and richer intelligence.

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

const PROPERTIES: &[&str] = &[
    "align-items",
    "background",
    "background-color",
    "border",
    "border-radius",
    "bottom",
    "box-shadow",
    "color",
    "column-gap",
    "content",
    "cursor",
    "display",
    "flex",
    "flex-direction",
    "font",
    "font-family",
    "font-size",
    "font-weight",
    "gap",
    "grid",
    "grid-template-columns",
    "height",
    "justify-content",
    "left",
    "letter-spacing",
    "line-height",
    "margin",
    "max-width",
    "min-height",
    "opacity",
    "outline",
    "overflow",
    "padding",
    "position",
    "right",
    "row-gap",
    "text-align",
    "text-decoration",
    "text-transform",
    "top",
    "transform",
    "transition",
    "vertical-align",
    "visibility",
    "white-space",
    "width",
    "z-index",
];

const VALUES: &[&str] = &[
    "absolute",
    "auto",
    "block",
    "bold",
    "border-box",
    "center",
    "column",
    "dashed",
    "fixed",
    "flex",
    "grid",
    "hidden",
    "inherit",
    "inline",
    "inline-block",
    "italic",
    "none",
    "normal",
    "pointer",
    "relative",
    "solid",
    "sticky",
    "transparent",
    "underline",
];

const AT_RULES: &[&str] = &[
    "@media",
    "@import",
    "@font-face",
    "@keyframes",
    "@supports",
    "@layer",
];

pub struct CssProvider;

impl LanguageProvider for CssProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Css
    }

    fn display_name(&self) -> &'static str {
        "CSS"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Css,
            extensions: &["css"],
            project_markers: &[],
            file_names: &[],
            shebangs: &[],
            content_hints: &["@media", "color:", "background", "display:"],
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
        crate::language::diagnostics::check_delimiters(self, text)
    }

    fn symbols(&self, text: &str) -> Vec<Symbol> {
        css_symbols(text)
    }

    fn completions(&self, _text: &str, _line: usize, _col: usize) -> Vec<Completion> {
        let mut completions: Vec<Completion> = PROPERTIES
            .iter()
            .map(|property| Completion::new(*property, CompletionKind::Variable))
            .collect();
        completions.extend(
            VALUES
                .iter()
                .map(|value| Completion::new(*value, CompletionKind::Keyword)),
        );
        completions.extend(
            AT_RULES
                .iter()
                .map(|rule| Completion::new(*rule, CompletionKind::Type)),
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
        let symbols = css_symbols(text);
        crate::language::hover::describe(text, line, col, &symbols)
    }

    fn line_comment(&self) -> &'static str {
        "/*"
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

        let brace = chars.iter().position(|c| *c == '{');

        while i < len {
            let c = chars[i];

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

            // At-rules.
            if c == '@' {
                let mut j = i + 1;
                while j < len && (chars[j].is_alphanumeric() || chars[j] == '-') {
                    j += 1;
                }
                push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Attribute));
                i = j;
                continue;
            }

            // `#id` before `{` is a selector; `#ff0` inside a block is a colour.
            if c == '#' && chars.get(i + 1).is_some_and(|c| is_ident_continue(*c)) {
                let selector = brace.is_some_and(|b| i < b);
                let mut j = i + 1;
                while j < len
                    && if selector {
                        is_ident_continue(chars[j])
                    } else {
                        chars[j].is_ascii_hexdigit()
                    }
                {
                    j += 1;
                }
                let kind = if selector {
                    TokenKind::Type
                } else {
                    TokenKind::Number
                };
                push_merged(&mut spans, HighlightSpan::new(i, j, kind));
                i = j;
                continue;
            }

            if c.is_ascii_digit()
                || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
            {
                let end = scan_number(&chars, i);
                // Include a unit suffix (`px`, `rem`, `%`, `s`).
                let mut j = end;
                while j < len && (chars[j].is_alphanumeric() || chars[j] == '%') {
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
                // Before `{` identifiers are selectors; inside a declaration
                // block a name before `:` is a property.
                let kind = if brace.is_some_and(|b| i < b) {
                    TokenKind::Type
                } else if word.eq_ignore_ascii_case("important") {
                    TokenKind::Keyword
                } else if next_nonspace(&chars, j) == Some(':')
                    || PROPERTIES.contains(&word.as_str())
                {
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
                ':' | ';' | '{' | '}' | ',' | '>' | '+' | '~' | '!' | '*' | '.'
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

fn next_nonspace(chars: &[char], from: usize) -> Option<char> {
    chars[from..].iter().copied().find(|c| !c.is_whitespace())
}

/// Selectors: the text before `{`, split on commas.
fn css_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let Some(brace) = line.find('{') else {
            continue;
        };
        let selector = &line[..brace];
        let mut column = 0usize;
        for part in selector.split(',') {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                let offset = part.len() - part.trim_start().len();
                symbols.push(Symbol::new(
                    trimmed.to_string(),
                    SymbolKind::Type,
                    row,
                    column + part[..offset].chars().count(),
                ));
            }
            column += part.chars().count() + 1;
        }
    }
    symbols
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c == '-' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c == '-' || c.is_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_claims_css() {
        let descriptor = CssProvider.descriptor();
        assert_eq!(descriptor.id, LanguageId::Css);
        assert!(descriptor.extensions.contains(&"css"));
    }

    #[test]
    fn highlights_selectors_properties_and_colours() {
        let kind_at = |spans: &[HighlightSpan], col: usize| {
            spans
                .iter()
                .find(|span| span.range.contains(&col))
                .map(|span| span.kind)
        };
        let (spans, _) = CssProvider.highlight(
            ".card { color: #ff0; padding: 4px; }",
            HighlightState::default(),
        );
        assert_eq!(kind_at(&spans, 1), Some(TokenKind::Type)); // card selector
        assert_eq!(kind_at(&spans, 9), Some(TokenKind::Function)); // color
        assert_eq!(kind_at(&spans, 16), Some(TokenKind::Number)); // #ff0
    }

    #[test]
    fn extracts_selector_symbols() {
        let symbols = css_symbols(".a, .b {\n  color: red;\n}\n#id {\n}\n");
        let names: Vec<_> = symbols.into_iter().map(|symbol| symbol.name).collect();
        assert!(names.contains(&".a".to_string()));
        assert!(names.contains(&".b".to_string()));
        assert!(names.contains(&"#id".to_string()));
    }
}
