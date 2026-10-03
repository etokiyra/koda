//! Lightweight, provider-driven diagnostics.
//!
//! Koda's built-in providers are lexical scanners, not full compilers, so they
//! cannot type-check. They *can* reliably catch structural mistakes — an
//! unbalanced bracket — without a language server. Those diagnostics flow
//! through the same background channel a future LSP backend will use, so the UI
//! never needs to know where they came from.
//!
//! Diagnostics are deliberately neutral data: they carry line/column positions
//! in characters, never terminal colours or editor types. Rendering decides how
//! they look.

use crate::language::provider::{HighlightState, LanguageProvider, TokenKind};

/// How serious a diagnostic is. Ordering runs from least to most severe so a
/// caller can pick the worst diagnostic on a line with `max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Hint,
    Info,
    Warning,
    Error,
}

impl Severity {
    /// A single glyph for the gutter. Info and hint share the quietest mark.
    pub fn gutter(self) -> char {
        match self {
            Severity::Error => '●',
            Severity::Warning => '▲',
            Severity::Info | Severity::Hint => '·',
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::Hint => "hint",
        }
    }
}

/// A position inside a diagnostic, measured in characters from the start of the
/// document. `line` and `col` are zero-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextPos {
    pub line: usize,
    pub col: usize,
}

impl TextPos {
    pub const fn new(line: usize, col: usize) -> Self {
        TextPos { line, col }
    }
}

/// A single problem found in a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Inclusive start of the affected range.
    pub start: TextPos,
    /// Exclusive end of the affected range.
    pub end: TextPos,
    pub severity: Severity,
    pub message: String,
}

impl Diagnostic {
    pub fn new(
        start: TextPos,
        end: TextPos,
        severity: Severity,
        message: impl Into<String>,
    ) -> Self {
        Diagnostic {
            start,
            end,
            severity,
            message: message.into(),
        }
    }

    /// Whether `position` falls inside the diagnostic's range.
    pub fn covers(&self, position: TextPos) -> bool {
        if position.line < self.start.line || position.line > self.end.line {
            return false;
        }
        if position.line == self.start.line && position.col < self.start.col {
            return false;
        }
        if position.line == self.end.line && position.col >= self.end.col {
            return false;
        }
        true
    }
}

/// Find unbalanced brackets, ignoring anything inside strings and comments.
///
/// The provider's own highlighter supplies the string/comment mask, so this
/// works for any language that tokenises those constructs — Koda never needs a
/// per-language copy of the scan.
pub fn check_delimiters(provider: &dyn LanguageProvider, text: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut stack: Vec<(char, TextPos)> = Vec::new();
    let mut state = HighlightState::default();

    for (row, line) in text.lines().enumerate() {
        let (spans, next) = provider.highlight(line, state);
        state = next;

        let chars: Vec<char> = line.chars().collect();
        let mut protected = vec![false; chars.len()];
        for span in &spans {
            if matches!(span.kind, TokenKind::String | TokenKind::Comment) {
                for index in span.range.clone() {
                    if let Some(flag) = protected.get_mut(index) {
                        *flag = true;
                    }
                }
            }
        }

        for (col, &c) in chars.iter().enumerate() {
            if protected[col] {
                continue;
            }
            match c {
                '(' | '[' | '{' => stack.push((c, TextPos::new(row, col))),
                ')' | ']' | '}' => report_closer(c, row, col, &mut stack, &mut diagnostics),
                _ => {}
            }
        }
    }

    // Anything still open at the end of the file was never closed.
    for (open, pos) in stack {
        diagnostics.push(unclosed(open, pos));
    }
    diagnostics
}

fn report_closer(
    closer: char,
    row: usize,
    col: usize,
    stack: &mut Vec<(char, TextPos)>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let expected = match closer {
        ')' => '(',
        ']' => '[',
        '}' => '{',
        _ => return,
    };
    // Only a closer that matches the innermost opener closes it. A mismatched
    // closer is reported on its own without consuming the opener, so the opener
    // can still be closed further down the file.
    if stack.last().is_some_and(|(open, _)| *open == expected) {
        stack.pop();
        return;
    }
    diagnostics.push(Diagnostic::new(
        TextPos::new(row, col),
        TextPos::new(row, col + 1),
        Severity::Error,
        format!("unmatched `{closer}`"),
    ));
}

fn unclosed(open: char, pos: TextPos) -> Diagnostic {
    Diagnostic::new(
        pos,
        TextPos::new(pos.line, pos.col + 1),
        Severity::Error,
        format!("unclosed `{open}`; expected `{}`", matching_close(open)),
    )
}

fn matching_close(open: char) -> char {
    match open {
        '(' => ')',
        '[' => ']',
        _ => '}',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::LanguageService;
    use crate::language::id::LanguageId;

    fn rust(text: &str) -> Vec<Diagnostic> {
        let service = LanguageService::builtin();
        service.provider(LanguageId::Rust).diagnostics(text)
    }

    #[test]
    fn balanced_code_has_no_diagnostics() {
        assert!(rust("fn main() {\n    let v = vec![1, 2];\n}\n").is_empty());
    }

    #[test]
    fn unclosed_brace_is_reported_at_the_opener() {
        let diags = rust("fn main() {\n    let x = 1;\n");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].start, TextPos::new(0, 10));
        assert!(diags[0].message.contains("unclosed `{`"));
    }

    #[test]
    fn unmatched_closer_is_reported() {
        let diags = rust("let x = 1;\n}\n");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].start, TextPos::new(1, 0));
        assert!(diags[0].message.contains("unmatched `}`"));
    }

    #[test]
    fn mismatched_pair_keeps_the_opener_open() {
        // A stray `]` must not silently consume the `{`; the brace still closes
        // on the next line, so only the `]` is reported.
        let diags = rust("fn main() {\n]\n}\n");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].start, TextPos::new(1, 0));
        assert!(diags[0].message.contains("unmatched `]`"));
    }

    #[test]
    fn swapped_pair_reports_both_ends() {
        let diags = rust("foo(]\n");
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].start, TextPos::new(0, 4));
        assert_eq!(diags[1].start, TextPos::new(0, 3));
    }

    #[test]
    fn brackets_inside_strings_and_comments_are_ignored() {
        assert!(rust("let s = \"{(\";\n// ) \n/* ] */\n").is_empty());
    }

    #[test]
    fn brackets_in_char_literals_are_ignored() {
        assert!(rust("let c = '(';\nlet d = ')';\n").is_empty());
    }

    #[test]
    fn block_comments_span_lines() {
        assert!(rust("let x = 1;\n/* {\n   } */\n").is_empty());
    }

    #[test]
    fn covers_checks_ranges() {
        let d = Diagnostic::new(
            TextPos::new(1, 2),
            TextPos::new(1, 5),
            Severity::Warning,
            "x",
        );
        assert!(d.covers(TextPos::new(1, 2)));
        assert!(d.covers(TextPos::new(1, 4)));
        assert!(!d.covers(TextPos::new(1, 5)));
        assert!(!d.covers(TextPos::new(0, 2)));
    }
}
