//! Python language provider.
//!
//! Koda's Python support is entirely built-in and works offline: syntax
//! highlighting, structural diagnostics, symbols, completion, hover and
//! within-file navigation. A Python language server is not provisioned yet, so
//! the palette honestly reports that rename and code actions need one.

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
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
    "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
];

const CONSTANTS: &[&str] = &["True", "False", "None"];

const TYPES: &[&str] = &[
    "int",
    "float",
    "complex",
    "str",
    "bool",
    "bytes",
    "bytearray",
    "list",
    "dict",
    "set",
    "frozenset",
    "tuple",
    "object",
    "type",
];

const BUILTINS: &[&str] = &[
    "abs",
    "all",
    "any",
    "ascii",
    "bin",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "delattr",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "format",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "locals",
    "map",
    "max",
    "min",
    "next",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "setattr",
    "sorted",
    "staticmethod",
    "sum",
    "super",
    "vars",
    "zip",
];

pub struct PythonProvider;

impl LanguageProvider for PythonProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Python
    }

    fn display_name(&self) -> &'static str {
        "Python"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Python,
            extensions: &["py", "pyi", "pyw"],
            project_markers: &["pyproject.toml", "setup.py", "requirements.txt", "Pipfile"],
            file_names: &[],
            shebangs: &["python", "pypy"],
            content_hints: &["def ", "import ", "self.", "class ", "if __name__"],
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
        python_symbols(text)
    }

    fn definition(
        &self,
        text: &str,
        line: usize,
        col: usize,
    ) -> Option<crate::language::symbols::Symbol> {
        let word = crate::language::symbols::word_at(text, line, col)?;
        python_symbols(text)
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
        let symbols = python_symbols(text);
        crate::language::hover::describe(text, line, col, &symbols)
    }

    fn line_comment(&self) -> &'static str {
        "#"
    }

    fn highlight(&self, line: &str, state: HighlightState) -> (Vec<HighlightSpan>, HighlightState) {
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let mut spans = Vec::new();
        let mut i = 0;

        // Finish a triple-quoted string carried over from the previous line.
        if state.in_block_comment {
            match find_triple_end(&chars, 0) {
                Some(end) => {
                    push_merged(&mut spans, HighlightSpan::new(0, end, TokenKind::String));
                    i = end;
                }
                None => {
                    if len > 0 {
                        push_merged(&mut spans, HighlightSpan::new(0, len, TokenKind::String));
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

        let first_nonspace = chars.iter().position(|c| !c.is_whitespace());

        while i < len {
            let c = chars[i];

            if c == '#' {
                push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                break;
            }

            // Triple-quoted strings (may span lines).
            if (c == '"' || c == '\'')
                && chars.get(i + 1) == Some(&c)
                && chars.get(i + 2) == Some(&c)
            {
                match find_triple_end(&chars, i + 3) {
                    Some(end) => {
                        push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                        i = end;
                    }
                    None => {
                        push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::String));
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

            if c == '"' || c == '\'' {
                let end = scan_quoted(&chars, i, c);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            if c.is_ascii_digit()
                || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
            {
                let end = scan_number(&chars, i);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Number));
                i = end;
                continue;
            }

            // A decorator only appears at the start of a line.
            if c == '@' && Some(i) == first_nonspace {
                let mut j = i + 1;
                while j < len && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '.')
                {
                    j += 1;
                }
                push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Attribute));
                i = j;
                continue;
            }

            if is_ident_start(c) {
                let mut j = i;
                while j < len && is_ident_continue(chars[j]) {
                    j += 1;
                }
                let word: String = chars[i..j].iter().collect();
                push_merged(&mut spans, HighlightSpan::new(i, j, classify_word(&word)));

                // `def name` / `class Name`: highlight the name too.
                let name_kind = match word.as_str() {
                    "def" => Some(TokenKind::Function),
                    "class" => Some(TokenKind::Type),
                    _ => None,
                };
                if let Some(kind) = name_kind
                    && let Some((start, end)) = name_after(&chars, j)
                {
                    push_merged(&mut spans, HighlightSpan::new(start, end, kind));
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

        (
            spans,
            HighlightState {
                in_block_comment: false,
                ..Default::default()
            },
        )
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

/// The identifier following a `def`/`class`, skipping spaces.
fn name_after(chars: &[char], from: usize) -> Option<(usize, usize)> {
    let mut j = from;
    while j < chars.len() && chars[j] == ' ' {
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

/// End of a `"""` or `'''` run starting at `from` (past the opening quotes).
fn find_triple_end(chars: &[char], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 2 < chars.len() {
        let c = chars[i];
        if (c == '"' || c == '\'') && chars[i + 1] == c && chars[i + 2] == c {
            return Some(i + 3);
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
            | '@'
            | ':'
            | ','
    )
}

/// Functions, classes and module-level constants, for the symbol outline.
pub fn python_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        if let Some(symbol) = python_line_symbol(line, row) {
            symbols.push(symbol);
        }
    }
    symbols
}

fn python_line_symbol(line: &str, row: usize) -> Option<Symbol> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('@') {
        return None;
    }
    let indent = line.chars().count() - trimmed.chars().count();

    for prefix in ["async def ", "def ", "class "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let kind = if prefix.contains("class") {
                SymbolKind::Type
            } else {
                SymbolKind::Function
            };
            let lead = rest.chars().take_while(|c| *c == ' ').count();
            let name = first_ident(rest)?;
            let col = indent + prefix.chars().count() + lead;
            return Some(Symbol::new(name, kind, row, col));
        }
    }

    // A module-level `NAME = ...` constant.
    if indent == 0
        && let Some((lhs, _)) = trimmed.split_once('=')
    {
        let name = lhs.trim();
        let is_const = !name.is_empty()
            && name.chars().any(char::is_uppercase)
            && name
                .chars()
                .all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit());
        if is_const {
            return Some(Symbol::new(name, SymbolKind::Constant, row, 0));
        }
    }
    None
}

fn first_ident(text: &str) -> Option<String> {
    let mut chars = text.chars().peekable();
    while chars.peek() == Some(&' ') {
        chars.next();
    }
    let mut word = String::new();
    while let Some(&c) = chars.peek() {
        if is_ident_continue(c) {
            word.push(c);
            chars.next();
        } else {
            break;
        }
    }
    if word.is_empty() { None } else { Some(word) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<TokenKind> {
        let (spans, _) = PythonProvider.highlight(line, HighlightState::default());
        spans.into_iter().map(|span| span.kind).collect()
    }

    #[test]
    fn highlights_keywords_strings_and_builtins() {
        let s = kinds("def greet(name):");
        assert!(s.contains(&TokenKind::Keyword), "def");
        assert!(s.contains(&TokenKind::Function), "greet");
        let s = kinds("print(\"hello\")");
        assert!(s.contains(&TokenKind::Function), "print");
        assert!(s.contains(&TokenKind::String));
    }

    #[test]
    fn comments_and_decorators() {
        assert_eq!(kinds("    # note"), vec![TokenKind::Comment]);
        let s = kinds("@app.route");
        assert_eq!(s, vec![TokenKind::Attribute]);
    }

    #[test]
    fn triple_quotes_carry_state() {
        let (_, state) = PythonProvider.highlight("\"\"\"start", HighlightState::default());
        assert!(state.in_block_comment);
        let (spans, state) = PythonProvider.highlight("still\"\"\" # done", state);
        assert!(!state.in_block_comment);
        assert_eq!(spans[0].kind, TokenKind::String);
        assert_eq!(spans.last().map(|s| s.kind), Some(TokenKind::Comment));
    }

    #[test]
    fn symbols_find_functions_classes_and_constants() {
        let text = "\
LIMIT = 10

class Server:
    def __init__(self):
        pass

    async def serve(self):
        pass

def main():
    pass
";
        let symbols = python_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["LIMIT", "Server", "__init__", "serve", "main"]);
        assert_eq!(symbols[0].kind, SymbolKind::Constant);
        assert_eq!(symbols[1].kind, SymbolKind::Type);
        assert_eq!(symbols[2].kind, SymbolKind::Function);
    }
}
