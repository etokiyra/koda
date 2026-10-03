//! Hover information for the symbol under the cursor.
//!
//! Built-in providers have no type checker, so hover is a heuristic: it reports
//! whether the word is defined in this file, shows the definition line and
//! counts its usages. A language server can replace this with real type and
//! documentation info later without changing the popup.

use crate::language::symbols::{Symbol, SymbolKind, locations_of_word, word_at};

/// What to show for the symbol under the cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    /// The headline, e.g. `fn main` or a bare identifier.
    pub title: String,
    /// The kind of definition, when the word names one.
    pub kind: Option<SymbolKind>,
    /// Supporting lines: the definition, usage count, and so on.
    pub body: Vec<String>,
}

impl Hover {
    pub fn new(title: impl Into<String>, kind: Option<SymbolKind>, body: Vec<String>) -> Self {
        Hover {
            title: title.into(),
            kind,
            body,
        }
    }
}
/// Build hover info for the word at `(line, col)` from a document's symbols.
///
/// Returns `None` when the cursor is not on an identifier.
pub fn describe(text: &str, line: usize, col: usize, symbols: &[Symbol]) -> Option<Hover> {
    let word = word_at(text, line, col)?;
    let references = locations_of_word(text, &word).len();
    match symbols.iter().find(|symbol| symbol.name == word) {
        Some(symbol) => {
            let mut body = Vec::new();
            if let Some(source) = text.lines().nth(symbol.line) {
                let source = source.trim();
                if !source.is_empty() {
                    body.push(source.to_string());
                }
            }
            body.push(format!("{references} occurrence(s) in this file"));
            Some(Hover::new(
                format!("{} {word}", symbol.kind.label()),
                Some(symbol.kind),
                body,
            ))
        }
        None => Some(Hover::new(
            word,
            None,
            vec![format!("{references} occurrence(s) in this file")],
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::symbols::rust_symbols;

    #[test]
    fn describe_reports_definitions_and_usages() {
        let text = "fn main() {\n    helper();\n    helper();\n}\n\nfn helper() {}\n";
        let symbols = rust_symbols(text);
        let hover = describe(text, 1, 4, &symbols).expect("hover");
        assert_eq!(hover.title, "fn helper");
        assert_eq!(hover.kind, Some(SymbolKind::Function));
        assert!(
            hover.body.iter().any(|line| line.contains("3 occurrence")),
            "body was {:?}",
            hover.body
        );
        assert!(
            hover
                .body
                .iter()
                .any(|line| line.contains("fn helper() {}"))
        );
    }

    #[test]
    fn describe_reports_plain_words() {
        let text = "let value = 1;\n";
        let hover = describe(text, 0, 4, &rust_symbols(text)).expect("hover");
        assert_eq!(hover.title, "value");
        assert_eq!(hover.kind, None);
    }
}
