//! YAML language provider.
//!
//! YAML is not line-oriented in general, but the parts that matter for reading
//! configuration — mapping keys, quoted and plain scalars, `# comments`,
//! document markers, numbers and boolean-ish literals — can be recognised
//! line by line. Keys are found by locating the first unquoted `: ` separator,
//! which keeps URLs and other colons in values from being mistaken for keys.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_number, scan_quoted, scan_single_quoted};
use crate::language::detection::LanguageDescriptor;
use crate::language::diagnostics::Diagnostic;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Symbol, SymbolKind};

/// Words that YAML treats as booleans or null.
const LITERALS: &[&str] = &["true", "false", "yes", "no", "on", "off", "null"];

pub struct YamlProvider;

impl LanguageProvider for YamlProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Yaml
    }

    fn display_name(&self) -> &'static str {
        "YAML"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Yaml,
            extensions: &["yaml", "yml"],
            project_markers: &[],
            file_names: &[],
            shebangs: &[],
            content_hints: &["---\n", "apiVersion:", "kind: ", "spec:", "name: "],
        }
    }

    fn capabilities(&self) -> &'static [Capability] {
        &[
            Capability::SyntaxHighlighting,
            Capability::Diagnostics,
            Capability::DocumentSymbols,
            Capability::Completion,
        ]
    }

    fn diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        crate::language::diagnostics::check_delimiters(self, text)
    }

    fn symbols(&self, text: &str) -> Vec<Symbol> {
        yaml_symbols(text)
    }

    fn completions(&self, _text: &str, _line: usize, _col: usize) -> Vec<Completion> {
        LITERALS
            .iter()
            .map(|word| Completion::new(*word, CompletionKind::Constant))
            .collect()
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
        let key = yaml_key(&chars);
        let first_nonspace = chars.iter().position(|c| !c.is_whitespace());
        let mut i = 0;

        while i < len {
            let c = chars[i];

            // A `#` starts a comment only at the start of a line or after space,
            // so `url#fragment` stays intact. Quoted strings are consumed whole
            // before we reach here, so a `#` inside quotes is never seen.
            if c == '#' && (i == 0 || chars[i - 1].is_whitespace()) {
                push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                break;
            }

            // Document markers.
            if Some(i) == first_nonspace
                && (chars[i..].starts_with(&['-', '-', '-'])
                    || chars[i..].starts_with(&['.', '.', '.']))
            {
                push_merged(
                    &mut spans,
                    HighlightSpan::new(i, i + 3, TokenKind::Attribute),
                );
                i += 3;
                continue;
            }

            // A lone `-` that opens a list item is punctuation, not a number.
            if c == '-'
                && Some(i) == first_nonspace
                && chars.get(i + 1).is_none_or(|n| n.is_whitespace())
            {
                push_merged(
                    &mut spans,
                    HighlightSpan::new(i, i + 1, TokenKind::Operator),
                );
                i += 1;
                continue;
            }

            if c == '"' {
                let end = scan_quoted(&chars, i, '"');
                let kind = if key.is_some_and(|(start, _, _)| start == i) {
                    TokenKind::Attribute
                } else {
                    TokenKind::String
                };
                push_merged(&mut spans, HighlightSpan::new(i, end, kind));
                i = end;
                continue;
            }
            if c == '\'' {
                let end = scan_single_quoted(&chars, i);
                let kind = if key.is_some_and(|(start, _, _)| start == i) {
                    TokenKind::Attribute
                } else {
                    TokenKind::String
                };
                push_merged(&mut spans, HighlightSpan::new(i, end, kind));
                i = end;
                continue;
            }

            if let Some((start, end, _)) = key
                && i == start
            {
                push_merged(
                    &mut spans,
                    HighlightSpan::new(start, end, TokenKind::Function),
                );
                i = end;
                continue;
            }

            if c == '~' {
                push_merged(
                    &mut spans,
                    HighlightSpan::new(i, i + 1, TokenKind::Constant),
                );
                i += 1;
                continue;
            }

            if c.is_ascii_digit()
                || (matches!(c, '+' | '-') && chars.get(i + 1).is_some_and(char::is_ascii_digit))
            {
                let end = scan_number(&chars, i);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Number));
                i = end;
                continue;
            }

            if c.is_alphanumeric() || c == '_' {
                let mut j = i;
                while j < len && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let word: String = chars[i..j].iter().collect();
                if LITERALS.contains(&word.as_str()) {
                    push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Constant));
                }
                i = j;
                continue;
            }

            if matches!(
                c,
                ':' | ',' | '{' | '}' | '[' | ']' | '?' | '|' | '>' | '&' | '*'
            ) {
                push_merged(
                    &mut spans,
                    HighlightSpan::new(i, i + 1, TokenKind::Operator),
                );
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

/// A block mapping key at the start of the line, if this line has one.
///
/// Returns `(start, end, quoted)`. The leading list dash (`- key: …`) is skipped.
fn yaml_key(chars: &[char]) -> Option<(usize, usize, bool)> {
    let len = chars.len();
    let mut start = chars.iter().position(|c| !c.is_whitespace())?;
    if chars[start] == '-' && chars.get(start + 1).is_none_or(|c| c.is_whitespace()) {
        start = chars[start + 1..]
            .iter()
            .position(|c| !c.is_whitespace())
            .map(|p| start + 1 + p)?;
    }

    let (end, quoted) = if chars[start] == '"' || chars[start] == '\'' {
        let quote = chars[start];
        let end = if quote == '"' {
            scan_quoted(chars, start, quote)
        } else {
            scan_single_quoted(chars, start)
        };
        (end, true)
    } else {
        let mut end = start;
        while end < len && (chars[end].is_alphanumeric() || is_key_punct(chars[end])) {
            end += 1;
        }
        if end == start {
            return None;
        }
        (end, false)
    };

    // Require a `:` separator followed by whitespace or end of line, so URLs and
    // other colon-bearing scalars are not mistaken for keys.
    let mut j = end;
    while j < len && chars[j] == ' ' {
        j += 1;
    }
    if chars.get(j) == Some(&':') && chars.get(j + 1).is_none_or(|c| c.is_whitespace()) {
        Some((start, end, quoted))
    } else {
        None
    }
}

/// Characters allowed in a bare (unquoted) YAML key.
fn is_key_punct(c: char) -> bool {
    matches!(c, '_' | '-' | '.' | '/' | '@')
}

/// Top-level keys, for the symbol outline and workspace search.
pub fn yaml_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let indent = line.chars().take_while(|c| c.is_whitespace()).count();
        if indent != 0 {
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("---") {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        if let Some((start, end, _)) = yaml_key(&chars) {
            let name: String = chars[start..end].iter().collect();
            let name = name.trim_matches(|c| c == '"' || c == '\'').to_string();
            if !name.is_empty() {
                symbols.push(Symbol::new(name, SymbolKind::Key, row, start));
            }
        }
    }
    symbols
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<TokenKind> {
        let (spans, _) = YamlProvider.highlight(line, HighlightState::default());
        spans.into_iter().map(|s| s.kind).collect()
    }

    #[test]
    fn highlights_keys_and_scalar_values() {
        let s = kinds("name: koda");
        assert_eq!(s[0], TokenKind::Function);

        let s = kinds("enabled: true");
        assert!(s.contains(&TokenKind::Constant));

        let s = kinds("count: 42");
        assert!(s.contains(&TokenKind::Number));
    }

    #[test]
    fn does_not_treat_url_colons_as_keys() {
        let (spans, _) =
            YamlProvider.highlight("  url: http://example.com", HighlightState::default());
        // The only `Function` span is the real key, `url`.
        let keys: Vec<&HighlightSpan> = spans
            .iter()
            .filter(|s| s.kind == TokenKind::Function)
            .collect();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].range, 2..5);
    }

    #[test]
    fn comments_require_a_boundary() {
        assert_eq!(kinds("# note"), vec![TokenKind::Comment]);
        let s = kinds("name: value");
        assert!(!s.contains(&TokenKind::Comment));
    }

    #[test]
    fn list_dash_is_punctuation() {
        let s = kinds("- item");
        assert_eq!(s[0], TokenKind::Operator);
    }

    #[test]
    fn document_marker_is_an_attribute() {
        assert_eq!(kinds("---"), vec![TokenKind::Attribute]);
    }

    #[test]
    fn symbols_lists_top_level_keys() {
        let text = "apiVersion: v1\nkind: Service\nmetadata:\n  name: web\nspec:\n  ports: []\n";
        let symbols = yaml_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["apiVersion", "kind", "metadata", "spec"]);
        assert_eq!(symbols[0].kind, SymbolKind::Key);
    }
}
