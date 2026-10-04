//! PHP provider.
//!
//! Built-in and offline: `<?php … ?>` tags, comments (`//`, `#`, `/* */`),
//! single- and double-quoted strings, variables, keywords, builtins, numbers
//! and operators. Structural diagnostics reuse the shared delimiter checker,
//! and `phpactor` is used for richer intelligence when it happens to be
//! installed (there is no portable user-local installer, so Koda does not
//! pretend to provision it).

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_quoted, scan_single_quoted};
use crate::language::detection::LanguageDescriptor;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Location, Symbol, SymbolKind, word_at};

const KEYWORDS: &[&str] = &[
    "abstract",
    "and",
    "array",
    "as",
    "break",
    "callable",
    "case",
    "catch",
    "class",
    "clone",
    "const",
    "continue",
    "declare",
    "default",
    "do",
    "echo",
    "else",
    "elseif",
    "empty",
    "enddeclare",
    "endfor",
    "endforeach",
    "endif",
    "endswitch",
    "endwhile",
    "enum",
    "extends",
    "final",
    "finally",
    "fn",
    "for",
    "foreach",
    "function",
    "global",
    "goto",
    "if",
    "implements",
    "include",
    "include_once",
    "instanceof",
    "insteadof",
    "interface",
    "isset",
    "list",
    "match",
    "namespace",
    "new",
    "or",
    "print",
    "private",
    "protected",
    "public",
    "readonly",
    "require",
    "require_once",
    "return",
    "static",
    "switch",
    "throw",
    "trait",
    "try",
    "unset",
    "use",
    "var",
    "while",
    "xor",
    "yield",
];

const CONSTANTS: &[&str] = &[
    "false",
    "null",
    "true",
    "PHP_EOL",
    "PHP_INT_MAX",
    "PHP_INT_MIN",
    "PHP_VERSION",
    "__DIR__",
    "__FILE__",
    "__LINE__",
    "__NAMESPACE__",
    "__CLASS__",
    "__FUNCTION__",
    "__METHOD__",
];

const BUILTINS: &[&str] = &[
    "count",
    "define",
    "explode",
    "file_get_contents",
    "fopen",
    "fwrite",
    "header",
    "implode",
    "in_array",
    "intval",
    "is_array",
    "is_null",
    "json_decode",
    "json_encode",
    "preg_match",
    "preg_replace",
    "printf",
    "sprintf",
    "str_replace",
    "strlen",
    "strtolower",
    "strpos",
    "strtoupper",
    "substr",
    "trim",
    "var_dump",
];

pub struct PhpProvider;

impl LanguageProvider for PhpProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Php
    }

    fn display_name(&self) -> &'static str {
        "PHP"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Php,
            extensions: &["php", "phtml", "php3", "php4", "php5", "phps"],
            project_markers: &["composer.json"],
            file_names: &[],
            shebangs: &["php"],
            content_hints: &["<?php", "<?=", "$this", "->", "::"],
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
        php_symbols(text)
    }

    fn definition(&self, text: &str, line: usize, col: usize) -> Option<Symbol> {
        let word = word_at(text, line, col)?;
        php_symbols(text)
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
        let symbols = php_symbols(text);
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

        if state.in_block_comment
            && let Some(end) = find_block_end(&chars, 0)
        {
            push_merged(&mut spans, HighlightSpan::new(0, end, TokenKind::Comment));
            i = end;
        } else if state.in_block_comment {
            if len > 0 {
                push_merged(&mut spans, HighlightSpan::new(0, len, TokenKind::Comment));
            }
            return (
                spans,
                HighlightState {
                    in_block_comment: true,
                    ..Default::default()
                },
            );
        }

        while i < len {
            let c = chars[i];
            let next = chars.get(i + 1).copied();

            // Block comment.
            if c == '/' && next == Some('*') {
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
                                ..Default::default()
                            },
                        );
                    }
                }
                continue;
            }

            // Line comments (`//` and `#`). `#[` is an attribute, not a comment.
            if (c == '/' && next == Some('/')) || (c == '#' && next != Some('[')) {
                push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                break;
            }

            // `<?php`, `<?=`, `<?` and the closing `?>`.
            if c == '<' && next == Some('?') {
                let end = if chars[i..].starts_with(&['<', '?', 'p', 'h', 'p']) {
                    i + 5
                } else if chars[i..].starts_with(&['<', '?', '=']) {
                    i + 3
                } else {
                    i + 2
                };
                push_merged(
                    &mut spans,
                    HighlightSpan::new(i, end.min(len), TokenKind::Keyword),
                );
                i = end;
                continue;
            }
            if c == '?' && next == Some('>') {
                push_merged(&mut spans, HighlightSpan::new(i, i + 2, TokenKind::Keyword));
                i += 2;
                continue;
            }

            if c == '\'' {
                let end = scan_single_quoted(&chars, i);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }
            if c == '"' {
                let end = scan_quoted(&chars, i, '"');
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }
            if c == '`' {
                let end = scan_single_quoted(&chars, i).max(i + 1);
                let end = if end > i + 1 && chars.get(end - 1) == Some(&'`') {
                    end
                } else {
                    chars[i + 1..]
                        .iter()
                        .position(|c| *c == '`')
                        .map(|off| i + off + 2)
                        .unwrap_or(len)
                };
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Function));
                i = end;
                continue;
            }

            // Variables: `$name`.
            if c == '$' && chars.get(i + 1).is_some_and(|c| is_ident_continue(*c)) {
                let mut j = i + 1;
                while j < len && is_ident_continue(chars[j]) {
                    j += 1;
                }
                push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Type));
                i = j;
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

            // Two-character operators first.
            if matches!(
                (c, next),
                ('-', Some('>')) | ('=', Some('>')) | (':', Some(':'))
            ) {
                push_merged(
                    &mut spans,
                    HighlightSpan::new(i, i + 2, TokenKind::Operator),
                );
                i += 2;
                continue;
            }
            if matches!(
                c,
                '=' | '+'
                    | '-'
                    | '*'
                    | '/'
                    | '%'
                    | '.'
                    | '!'
                    | '<'
                    | '>'
                    | '|'
                    | '&'
                    | '?'
                    | ':'
                    | ';'
                    | ','
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

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic() || (c as u32) > 0x7f
}

fn is_ident_continue(c: char) -> bool {
    is_ident_start(c) || c.is_ascii_digit()
}

/// Functions, classes, interfaces, traits, enums, constants and namespaces.
fn php_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        // Skip line comments; block comments and strings are close enough to
        // ignore for an outline.
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with('*') {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if !is_ident_start(chars[i]) {
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && is_ident_continue(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let kind = match word.as_str() {
                "function" => Some(SymbolKind::Function),
                "class" => Some(SymbolKind::Struct),
                "interface" => Some(SymbolKind::Interface),
                "trait" => Some(SymbolKind::Trait),
                "enum" => Some(SymbolKind::Enum),
                "namespace" => Some(SymbolKind::Module),
                "const" => Some(SymbolKind::Constant),
                _ => None,
            };
            if let Some(kind) = kind
                && let Some((name, col)) = name_after(&chars, i)
            {
                symbols.push(Symbol::new(name, kind, row, col));
                i = col + 1;
                continue;
            }
        }
    }
    symbols
}

/// The identifier after position `from`, with its character column.
fn name_after(chars: &[char], from: usize) -> Option<(String, usize)> {
    let mut j = from;
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }
    if j >= chars.len() || !is_ident_start(chars[j]) {
        return None;
    }
    let start = j;
    while j < chars.len() && is_ident_continue(chars[j]) {
        j += 1;
    }
    Some((chars[start..j].iter().collect(), start))
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
    fn descriptor_claims_php() {
        let descriptor = PhpProvider.descriptor();
        assert_eq!(descriptor.id, LanguageId::Php);
        assert!(descriptor.extensions.contains(&"php"));
        assert!(descriptor.project_markers.contains(&"composer.json"));
    }

    #[test]
    fn highlights_tags_variables_and_keywords() {
        let (spans, state) = PhpProvider.highlight(
            "<?php $name = 'koda'; echo $name;",
            HighlightState::default(),
        );
        assert!(!state.in_block_comment);
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Keyword)); // <?php
        assert_eq!(kind_at(&spans, 6), Some(TokenKind::Type)); // $name
        assert_eq!(kind_at(&spans, 14), Some(TokenKind::String)); // 'koda'
        assert_eq!(kind_at(&spans, 22), Some(TokenKind::Keyword)); // echo
    }

    #[test]
    fn block_comments_carry_across_lines() {
        let (first, state) = PhpProvider.highlight("/* comment", HighlightState::default());
        assert!(state.in_block_comment);
        assert_eq!(kind_at(&first, 0), Some(TokenKind::Comment));
        let (second, state) = PhpProvider.highlight("still */ echo 1;", state);
        assert!(!state.in_block_comment);
        assert_eq!(kind_at(&second, 6), Some(TokenKind::Comment)); // `*`
        assert_eq!(kind_at(&second, 9), Some(TokenKind::Keyword)); // echo
    }

    #[test]
    fn attributes_are_not_hash_comments() {
        let (spans, _) = PhpProvider.highlight("#[Attribute]", HighlightState::default());
        assert_ne!(kind_at(&spans, 0), Some(TokenKind::Comment));
    }

    #[test]
    fn symbols_cover_classes_functions_and_namespaces() {
        let text = "\
<?php
namespace App\\Core;
class Router {}
interface Handler {}
trait Loggable {}
function boot() {}
const VERSION = '1';
";
        let symbols = php_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["App", "Router", "Handler", "Loggable", "boot", "VERSION"]
        );
        assert_eq!(symbols[1].kind, SymbolKind::Struct);
        assert_eq!(symbols[2].kind, SymbolKind::Interface);
        assert_eq!(symbols[4].kind, SymbolKind::Function);
    }

    #[test]
    fn diagnostics_report_unbalanced_braces() {
        let diagnostics = PhpProvider.diagnostics("<?php function f() {\n");
        assert!(
            diagnostics.iter().any(|d| d.message.contains("unclosed")),
            "expected an unclosed brace: {diagnostics:?}"
        );
        assert!(
            PhpProvider
                .diagnostics("<?php function f() {\n    return 1;\n}\n")
                .is_empty()
        );
    }

    #[test]
    fn definition_resolves_within_the_file() {
        let text = "<?php\nfunction greet() {}\ngreet();\n";
        let definition = PhpProvider.definition(text, 2, 1).expect("definition");
        assert_eq!(definition.name, "greet");
        assert_eq!(definition.line, 1);
    }
}
