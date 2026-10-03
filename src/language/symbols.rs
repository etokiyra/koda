//! Lightweight document-symbol extraction.
//!
//! Providers scan the current document for named definitions. This is a
//! heuristic, not a semantic index: it powers a symbol outline and within-file
//! navigation and will be superseded by language-server symbols where available.
//! It never attempts to resolve types or scopes.
//!
//! Extraction is line-oriented on purpose: definitions in Rust and Go begin at
//! the start of a statement, so requiring the keyword to be preceded only by
//! modifiers keeps false positives (a `fn` inside a string, say) out.

/// What kind of thing a symbol is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Interface,
    Module,
    Type,
    Constant,
    Variable,
    Macro,
}

impl SymbolKind {
    /// A short lowercase name used in lists.
    pub fn label(self) -> &'static str {
        match self {
            SymbolKind::Function => "fn",
            SymbolKind::Method => "method",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Interface => "interface",
            SymbolKind::Module => "mod",
            SymbolKind::Type => "type",
            SymbolKind::Constant => "const",
            SymbolKind::Variable => "var",
            SymbolKind::Macro => "macro",
        }
    }
}

/// A named definition in a document, positioned at its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// Zero-based line of the definition.
    pub line: usize,
    /// Zero-based character column of the name.
    pub col: usize,
}

impl Symbol {
    pub fn new(name: impl Into<String>, kind: SymbolKind, line: usize, col: usize) -> Self {
        Symbol {
            name: name.into(),
            kind,
            line,
            col,
        }
    }
}

/// Symbols in a Rust document.
pub fn rust_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        if let Some(symbol) = rust_line_symbol(line, row) {
            symbols.push(symbol);
        }
    }
    symbols
}

/// Words that may precede a Rust definition without changing what it is.
const RUST_MODIFIERS: &[&str] = &[
    "pub", "async", "unsafe", "const", "extern", "default", "auto", "crate", "super", "self", "in",
];

fn rust_item_kind(word: &str) -> Option<SymbolKind> {
    Some(match word {
        "fn" => SymbolKind::Function,
        "struct" | "union" => SymbolKind::Struct,
        "enum" => SymbolKind::Enum,
        "trait" => SymbolKind::Trait,
        "mod" => SymbolKind::Module,
        "type" => SymbolKind::Type,
        "const" | "static" => SymbolKind::Constant,
        "macro_rules" => SymbolKind::Macro,
        _ => return None,
    })
}

fn rust_line_symbol(line: &str, row: usize) -> Option<Symbol> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return None;
    }
    let indent = line.chars().take_while(|c| c.is_whitespace()).count();
    let words = words_with_offsets(trimmed);

    let mut index = 0;
    while index < words.len() {
        let (word, _) = words[index];
        // `const fn` is a function, not a constant.
        if word == "const" && words.get(index + 1).is_some_and(|(next, _)| *next == "fn") {
            index += 1;
            continue;
        }
        if let Some(kind) = rust_item_kind(word) {
            // Everything before the keyword must be a modifier, otherwise this
            // is an expression that merely mentions the keyword.
            if !words[..index]
                .iter()
                .all(|(w, _)| RUST_MODIFIERS.contains(w))
            {
                return None;
            }
            let (name, offset) = words.get(index + 1)?;
            return Some(Symbol::new((*name).to_string(), kind, row, indent + offset));
        }
        if !RUST_MODIFIERS.contains(&word) {
            return None;
        }
        index += 1;
    }
    None
}

/// Symbols in a Go document.
pub fn go_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        if let Some(symbol) = go_line_symbol(line, row) {
            symbols.push(symbol);
        }
    }
    symbols
}

fn go_line_symbol(line: &str, row: usize) -> Option<Symbol> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return None;
    }

    let first = first_word(trimmed)?;
    match first.as_str() {
        "func" => {
            let rest = trimmed[first.len()..].trim_start();
            // Methods carry a receiver: `func (r T) Name(...)`.
            if rest.starts_with('(') {
                let close = rest.find(')')?;
                let name = first_word(&rest[close + 1..])?;
                Some(Symbol::new(
                    name.clone(),
                    SymbolKind::Method,
                    row,
                    column_of(line, &name),
                ))
            } else {
                let name = first_word(rest)?;
                Some(Symbol::new(
                    name.clone(),
                    SymbolKind::Function,
                    row,
                    column_of(line, &name),
                ))
            }
        }
        "type" => {
            let name = first_word(&trimmed[first.len()..])?;
            let kind = if trimmed.contains(" struct") {
                SymbolKind::Struct
            } else if trimmed.contains(" interface") {
                SymbolKind::Interface
            } else {
                SymbolKind::Type
            };
            Some(Symbol::new(name.clone(), kind, row, column_of(line, &name)))
        }
        "var" | "const" => {
            let name = first_word(&trimmed[first.len()..])?;
            let kind = if first == "const" {
                SymbolKind::Constant
            } else {
                SymbolKind::Variable
            };
            Some(Symbol::new(name.clone(), kind, row, column_of(line, &name)))
        }
        "package" => {
            let name = first_word(&trimmed[first.len()..])?;
            Some(Symbol::new(
                name.clone(),
                SymbolKind::Module,
                row,
                column_of(line, &name),
            ))
        }
        _ => None,
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn first_word(text: &str) -> Option<String> {
    text.split(|c: char| !is_ident_char(c))
        .find(|word| !word.is_empty())
        .map(str::to_string)
}

/// The character column where `needle` first appears on the line.
fn column_of(line: &str, needle: &str) -> usize {
    match line.find(needle) {
        Some(byte) => line[..byte].chars().count(),
        None => line.chars().take_while(|c| c.is_whitespace()).count(),
    }
}

/// Identifier words on a line, paired with their byte offset from the start.
fn words_with_offsets(text: &str) -> Vec<(&str, usize)> {
    let mut words = Vec::new();
    let mut start: Option<usize> = None;
    for (index, c) in text.char_indices() {
        if is_ident_char(c) {
            if start.is_none() {
                start = Some(index);
            }
        } else if let Some(begin) = start.take() {
            words.push((&text[begin..index], begin));
        }
    }
    if let Some(begin) = start {
        words.push((&text[begin..], begin));
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_finds_items() {
        let text = "\
use std::fmt;

pub struct Point {
    x: i32,
}

pub(crate) enum Kind { A, B }

trait Draw {}

impl Draw for Point {
    fn draw(&self) {}
}

pub const MAX: u32 = 10;

pub async unsafe fn run() {}

macro_rules! make { () => {} }

mod inner;
";
        let symbols = rust_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Point", "Kind", "Draw", "draw", "MAX", "run", "make", "inner"
            ]
        );
        assert_eq!(symbols[0].kind, SymbolKind::Struct);
        assert_eq!(symbols[3].kind, SymbolKind::Function);
        assert_eq!(symbols[3].line, 11);
        assert_eq!(symbols[4].kind, SymbolKind::Constant);
        assert_eq!(symbols[7].kind, SymbolKind::Module);
    }

    #[test]
    fn rust_ignores_non_definitions() {
        assert!(rust_symbols("let x = foo();\n// fn fake() {}\n").is_empty());
        assert!(rust_symbols("let f: fn(i32) -> i32 = id;\n").is_empty());
    }

    #[test]
    fn rust_const_fn_is_a_function() {
        let symbols = rust_symbols("pub const fn value() -> u32 { 1 }\n");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "value");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn go_finds_items() {
        let text = "\
package main

import \"fmt\"

func main() {}

func (s *Server) Serve() {}

type Server struct{}

type Handler interface{}

const Limit = 3

var counter int
";
        let symbols = go_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "main", "main", "Serve", "Server", "Handler", "Limit", "counter"
            ]
        );
        assert_eq!(symbols[2].kind, SymbolKind::Method);
        assert_eq!(symbols[3].kind, SymbolKind::Struct);
        assert_eq!(symbols[4].kind, SymbolKind::Interface);
        assert_eq!(symbols[5].kind, SymbolKind::Constant);
        assert_eq!(symbols[6].kind, SymbolKind::Variable);
    }

    #[test]
    fn column_points_at_the_name() {
        let symbols = rust_symbols("    pub fn helper() {}\n");
        assert_eq!(symbols[0].col, 11);
    }
}
