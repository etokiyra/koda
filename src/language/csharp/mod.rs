//! C# provider.
//!
//! Built-in and offline: highlighting (types, keywords, attributes, regular,
//! verbatim, interpolated and raw strings, line/block/XML-doc comments),
//! structural diagnostics, symbols, completion, hover and within-file
//! navigation. `OmniSharp` is provisioned separately for rename, code actions
//! and richer, project-aware analysis.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_number};
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
    "abstract",
    "as",
    "async",
    "await",
    "base",
    "break",
    "case",
    "catch",
    "checked",
    "class",
    "const",
    "continue",
    "default",
    "delegate",
    "do",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "for",
    "foreach",
    "get",
    "goto",
    "if",
    "implicit",
    "in",
    "init",
    "interface",
    "internal",
    "is",
    "lock",
    "namespace",
    "new",
    "null",
    "operator",
    "out",
    "override",
    "params",
    "partial",
    "private",
    "protected",
    "public",
    "readonly",
    "record",
    "ref",
    "return",
    "sealed",
    "set",
    "sizeof",
    "stackalloc",
    "static",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "unchecked",
    "unsafe",
    "using",
    "value",
    "virtual",
    "volatile",
    "when",
    "where",
    "while",
    "with",
    "yield",
];

const TYPES: &[&str] = &[
    "bool", "byte", "char", "decimal", "double", "dynamic", "float", "int", "long", "nint",
    "nuint", "object", "sbyte", "short", "string", "uint", "ulong", "ushort", "var", "void",
];

const BUILTINS: &[&str] = &[
    "Array",
    "Boolean",
    "Byte",
    "Char",
    "Console",
    "Convert",
    "DateTime",
    "Debug",
    "Decimal",
    "Dictionary",
    "Double",
    "Enum",
    "Enumerable",
    "Environment",
    "Exception",
    "Guid",
    "HashSet",
    "Int32",
    "Int64",
    "IEnumerable",
    "IList",
    "List",
    "Math",
    "Object",
    "Path",
    "Queue",
    "Random",
    "String",
    "StringBuilder",
    "Task",
    "TimeSpan",
    "Tuple",
    "Uri",
    "ValueTask",
];

const CONSTANTS: &[&str] = &["true", "false", "null", "default"];

/// Modifiers that may precede a declaration.
const MODIFIERS: &[&str] = &[
    "public",
    "private",
    "protected",
    "internal",
    "static",
    "sealed",
    "abstract",
    "partial",
    "readonly",
    "unsafe",
    "virtual",
    "override",
    "async",
    "extern",
    "new",
];

/// Type keywords that name a declaration.
const DECLARATIONS: &[(&str, SymbolKind)] = &[
    ("class", SymbolKind::Type),
    ("struct", SymbolKind::Struct),
    ("interface", SymbolKind::Interface),
    ("enum", SymbolKind::Enum),
    ("record", SymbolKind::Type),
    ("delegate", SymbolKind::Type),
    ("namespace", SymbolKind::Module),
];

pub struct CSharpProvider;

impl LanguageProvider for CSharpProvider {
    fn id(&self) -> LanguageId {
        LanguageId::CSharp
    }

    fn display_name(&self) -> &'static str {
        "C#"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::CSharp,
            extensions: &["cs", "csx"],
            project_markers: &["global.json"],
            file_names: &[],
            shebangs: &[],
            content_hints: &[
                "using System",
                "namespace ",
                "public class ",
                "Console.WriteLine",
            ],
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

    fn diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        crate::language::diagnostics::check_delimiters(self, text)
    }

    fn symbols(&self, text: &str) -> Vec<Symbol> {
        csharp_symbols(text)
    }

    fn definition(
        &self,
        text: &str,
        line: usize,
        col: usize,
    ) -> Option<crate::language::symbols::Symbol> {
        let word = crate::language::symbols::word_at(text, line, col)?;
        csharp_symbols(text)
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

    fn format(&self, _path: &Path, _text: &str) -> FormatOutcome {
        FormatOutcome::Unsupported
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = csharp_symbols(text);
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

        // Resume a block comment.
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

        // Preprocessor / region directives.
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

            // `///` documentation comments read like comments (kept distinct by
            // being italic, like every comment).
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

            // Ordinary and verbatim-character strings, plus their `@`/`$`
            // prefixes.
            if c == '"' || c == '\'' || is_string_prefix(&chars, i) {
                let end = scan_string(&chars, i);
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

            // `[Attribute]` at the start of a line (or after another attribute).
            if c == '['
                && attribute_at(&chars, i)
                && let Some(end) = attribute_end(&chars, i)
            {
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Attribute));
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

                // `class Name`, `interface Name`, …: highlight the name too.
                if DECLARATIONS.iter().any(|(keyword, _)| *keyword == word)
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

/// Whether `i` begins a `@`/`$` string prefix.
fn is_string_prefix(chars: &[char], i: usize) -> bool {
    let mut j = i;
    while j < chars.len() && matches!(chars[j], '@' | '$') {
        j += 1;
    }
    j > i && chars.get(j) == Some(&'"')
}

/// Scan a string starting at `i`, which may be a `"` or an `@`/`$` prefix.
fn scan_string(chars: &[char], i: usize) -> usize {
    let mut quote = i;
    let mut verbatim = false;
    while quote < chars.len() && matches!(chars[quote], '@' | '$') {
        if chars[quote] == '@' {
            verbatim = true;
        }
        quote += 1;
    }
    if chars.get(quote) != Some(&'"') {
        // A stray prefix; treat as an operator.
        return i + 1;
    }

    // Raw string literal: `"""` … `"""`.
    if chars.get(quote + 1) == Some(&'"') && chars.get(quote + 2) == Some(&'"') {
        let mut j = quote + 3;
        while j + 2 < chars.len() {
            if chars[j] == '"' && chars[j + 1] == '"' && chars[j + 2] == '"' {
                return j + 3;
            }
            j += 1;
        }
        return chars.len();
    }

    let mut j = quote + 1;
    while j < chars.len() {
        let c = chars[j];
        if verbatim {
            if c == '"' {
                if chars.get(j + 1) == Some(&'"') {
                    j += 2;
                    continue;
                }
                return j + 1;
            }
            j += 1;
        } else {
            match c {
                '\\' => j += 2,
                '"' => return j + 1,
                _ => j += 1,
            }
        }
    }
    chars.len()
}

/// Whether a `[` at `i` looks like the start of an attribute list.
fn attribute_at(chars: &[char], i: usize) -> bool {
    let before = (0..i).rev().find(|index| !chars[*index].is_whitespace());
    match before {
        None => true,
        Some(index) => chars[index] == ']',
    }
}

/// The index just past a `[...]` attribute at `i`, when it closes on this line.
fn attribute_end(chars: &[char], i: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut j = i;
    while j < chars.len() {
        match chars[j] {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(j + 1);
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
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

fn name_after(chars: &[char], from: usize) -> Option<(usize, usize)> {
    let start = skip_ws(chars, from);
    let (name, end) = word_at(chars, start);
    if name.is_empty() {
        return None;
    }
    Some((start, end))
}

fn skip_ws(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

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

/// Named declarations in a document.
fn csharp_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        // Skip attributes and modifiers.
        loop {
            i = skip_ws(&chars, i);
            if chars.get(i) == Some(&'[')
                && let Some(end) = attribute_end(&chars, i)
            {
                i = end;
                continue;
            }
            let (word, end) = word_at(&chars, i);
            if MODIFIERS.contains(&word.as_str()) {
                i = end;
                continue;
            }
            break;
        }
        let (keyword, after) = word_at(&chars, i);
        let Some((_, kind)) = DECLARATIONS.iter().find(|(name, _)| *name == keyword) else {
            continue;
        };
        if let Some((name, col)) = name_at(&chars, after) {
            symbols.push(Symbol::new(name, *kind, row, col));
        }
    }
    symbols
}

fn name_at(chars: &[char], from: usize) -> Option<(String, usize)> {
    let start = skip_ws(chars, from);
    let (name, _) = word_at(chars, start);
    (!name.is_empty()).then_some((name, start))
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
        '+' | '-'
            | '*'
            | '/'
            | '%'
            | '='
            | '<'
            | '>'
            | '!'
            | '&'
            | '|'
            | '^'
            | '~'
            | '?'
            | ':'
            | '.'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_claims_csharp_files() {
        let descriptor = CSharpProvider.descriptor();
        assert_eq!(descriptor.id, LanguageId::CSharp);
        assert!(descriptor.extensions.contains(&"cs"));
    }

    #[test]
    fn highlights_strings_attributes_and_keywords() {
        let kind_at = |spans: &[HighlightSpan], col: usize| {
            spans
                .iter()
                .find(|span| span.range.contains(&col))
                .map(|span| span.kind)
        };
        let (spans, _) = CSharpProvider.highlight(
            "[Obsolete]\npublic class Widget { }",
            HighlightState::default(),
        );
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Attribute));

        let (spans, _) =
            CSharpProvider.highlight("var s = $\"hi {name}\";", HighlightState::default());
        assert_eq!(kind_at(&spans, 8), Some(TokenKind::String));
    }

    #[test]
    fn scans_verbatim_and_raw_strings() {
        let verbatim: Vec<char> = r#"var p = @"C:\temp"; var q = 1;"#.chars().collect();
        let end = scan_string(&verbatim, 8);
        let text: String = verbatim[8..end].iter().collect();
        assert_eq!(text, r#"@"C:\temp""#);

        let raw: Vec<char> = "x = \"\"\"a\"b\"\"\" + 1".chars().collect();
        let end = scan_string(&raw, 4);
        assert_eq!(raw[end..].iter().collect::<String>(), " + 1");
    }

    #[test]
    fn extracts_declarations() {
        let text = "namespace Demo;\npublic class Widget { }\ninterface IThing { }\nrecord Point(int X, int Y);\nenum Color { Red }\n";
        let names: Vec<_> = csharp_symbols(text)
            .into_iter()
            .map(|symbol| (symbol.name, symbol.kind))
            .collect();
        assert!(names.contains(&("Demo".to_string(), SymbolKind::Module)));
        assert!(names.contains(&("Widget".to_string(), SymbolKind::Type)));
        assert!(names.contains(&("IThing".to_string(), SymbolKind::Interface)));
        assert!(names.contains(&("Point".to_string(), SymbolKind::Type)));
        assert!(names.contains(&("Color".to_string(), SymbolKind::Enum)));
    }
}
