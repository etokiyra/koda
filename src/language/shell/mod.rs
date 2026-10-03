//! Shell language provider (bash, zsh and POSIX sh).
//!
//! A focused scanner for scripts: comments, single- and double-quoted strings,
//! variables and expansions, keywords, builtins, numbers and function
//! definitions. Like the other providers it is entirely built-in and offline;
//! `bash-language-server` can be provisioned separately for richer features.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_quoted, scan_single_quoted};
use crate::language::detection::LanguageDescriptor;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Symbol, SymbolKind};

const KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "function", "in", "select", "return", "local", "export", "declare", "readonly", "source",
    "alias", "unset", "shift", "trap", "set", "time", "coproc", "continue", "break",
];

const BUILTINS: &[&str] = &[
    "echo", "printf", "read", "cd", "pwd", "ls", "cat", "grep", "sed", "awk", "mkdir", "rmdir",
    "rm", "cp", "mv", "touch", "chmod", "chown", "ln", "find", "test", "exit", "eval", "exec",
    "command", "type", "hash", "help", "jobs", "bg", "fg", "kill", "wait", "sleep", "true",
    "false", "pushd", "popd", "dirname", "basename",
];

pub struct ShellProvider;

impl LanguageProvider for ShellProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Shell
    }

    fn display_name(&self) -> &'static str {
        "Shell"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Shell,
            extensions: &["sh", "bash", "zsh", "ksh"],
            project_markers: &[],
            file_names: &[
                ".bashrc",
                ".bash_profile",
                ".bash_aliases",
                ".zshrc",
                ".zprofile",
                ".profile",
                "PKGBUILD",
            ],
            shebangs: &["bash", "zsh", "ksh", "/sh"],
            content_hints: &["fi\n", "done\n", "esac\n", "then\n", "#!/"],
        }
    }

    fn capabilities(&self) -> &'static [Capability] {
        &[
            Capability::SyntaxHighlighting,
            Capability::DocumentSymbols,
            Capability::GotoDefinition,
            Capability::GotoReference,
            Capability::Completion,
            Capability::Hover,
        ]
    }

    fn symbols(&self, text: &str) -> Vec<Symbol> {
        shell_symbols(text)
    }

    fn definition(
        &self,
        text: &str,
        line: usize,
        col: usize,
    ) -> Option<crate::language::symbols::Symbol> {
        let word = crate::language::symbols::word_at(text, line, col)?;
        shell_symbols(text)
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
            BUILTINS
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Function)),
        );
        completions
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = shell_symbols(text);
        crate::language::hover::describe(text, line, col, &symbols)
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
        let mut i = 0;

        while i < len {
            let c = chars[i];

            // `#` starts a comment at the start of a line or after space, so
            // `${x#prefix}` and `foo#bar` stay intact.
            if c == '#' && (i == 0 || chars[i - 1].is_whitespace()) {
                push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                break;
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
                let mut j = i + 1;
                while j < len && chars[j] != '`' {
                    if chars[j] == '\\' {
                        j += 1;
                    }
                    j += 1;
                }
                let end = (j + 1).min(len);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            // Variables and expansions: `$name`, `${...}`, `$1`, `$?`, `$@`.
            if c == '$' {
                let end = scan_variable(&chars, i);
                if end > i + 1 {
                    push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Type));
                    i = end;
                    continue;
                }
            }

            if c.is_ascii_digit() {
                let mut j = i;
                while j < len && (chars[j].is_ascii_digit() || chars[j] == '.') {
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
                } else if BUILTINS.contains(&word.as_str()) {
                    TokenKind::Function
                } else if chars.get(j) == Some(&'(') {
                    // `name()` introduces a function.
                    TokenKind::Function
                } else {
                    TokenKind::Plain
                };
                push_merged(&mut spans, HighlightSpan::new(i, j, kind));

                // `function name` highlights the name too.
                if word == "function"
                    && let Some((start, end)) = name_after(&chars, j)
                {
                    push_merged(
                        &mut spans,
                        HighlightSpan::new(start, end, TokenKind::Function),
                    );
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

/// End of a `$…` variable reference starting at `$`.
fn scan_variable(chars: &[char], start: usize) -> usize {
    let mut i = start + 1;
    if chars.get(i) == Some(&'{') {
        i += 1;
        while i < chars.len() && chars[i] != '}' {
            i += 1;
        }
        return (i + 1).min(chars.len());
    }
    if chars.get(i).is_some_and(|c| is_ident_continue(*c)) {
        while i < chars.len() && is_ident_continue(chars[i]) {
            i += 1;
        }
        return i;
    }
    // Special one-character parameters such as `$?`, `$@`, `$#`, `$$`, `$1`.
    if i < chars.len() {
        return i + 1;
    }
    start + 1
}

/// The identifier after `function`.
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

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

fn is_operator(c: char) -> bool {
    matches!(
        c,
        '|' | '&'
            | ';'
            | '<'
            | '>'
            | '('
            | ')'
            | '{'
            | '}'
            | '['
            | ']'
            | '='
            | '!'
            | '+'
            | '-'
            | '*'
            | '/'
            | '%'
            | '~'
            | '?'
            | ':'
    )
}

/// Function definitions, for the symbol outline.
pub fn shell_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.chars().count() - trimmed.chars().count();

        if let Some(rest) = trimmed.strip_prefix("function ") {
            let lead = rest.chars().take_while(|c| *c == ' ').count();
            if let Some(name) = first_ident(rest) {
                symbols.push(Symbol::new(
                    name,
                    SymbolKind::Function,
                    row,
                    indent + "function ".chars().count() + lead,
                ));
            }
            continue;
        }

        // `name()` (with optional arguments left empty).
        if let Some(open) = trimmed.find('(') {
            let name = trimmed[..open].trim();
            let valid = !name.is_empty()
                && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
                && trimmed[open + 1..].starts_with(')');
            if valid {
                symbols.push(Symbol::new(name, SymbolKind::Function, row, indent));
            }
        }
    }
    symbols
}

fn first_ident(text: &str) -> Option<String> {
    let word: String = text
        .chars()
        .skip_while(|c| *c == ' ')
        .take_while(|c| is_ident_continue(*c))
        .collect();
    if word.is_empty() { None } else { Some(word) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<TokenKind> {
        let (spans, _) = ShellProvider.highlight(line, HighlightState::default());
        spans.into_iter().map(|span| span.kind).collect()
    }

    #[test]
    fn highlights_keywords_variables_and_builtins() {
        let s = kinds("if [ \"$HOME\" = \"x\" ]; then echo $1; fi");
        assert!(s.contains(&TokenKind::Keyword), "if/then/fi");
        assert!(s.contains(&TokenKind::String));
        assert!(s.contains(&TokenKind::Type), "variable");
        assert!(s.contains(&TokenKind::Function), "echo");
    }

    #[test]
    fn comments_need_a_boundary() {
        assert_eq!(kinds("# note"), vec![TokenKind::Comment]);
        let s = kinds("echo foo#bar");
        assert!(!s.contains(&TokenKind::Comment));
        let s = kinds("${value#prefix}");
        assert!(!s.contains(&TokenKind::Comment));
    }

    #[test]
    fn single_quotes_do_not_escape() {
        // In shell, a backslash does not escape a quote inside single quotes,
        // so `'a\'` closes at the backslash-adjacent quote.
        let (spans, _) = ShellProvider.highlight("x='a\\'b'", HighlightState::default());
        let strings: Vec<&HighlightSpan> = spans
            .iter()
            .filter(|span| span.kind == TokenKind::String)
            .collect();
        assert_eq!(strings.len(), 2);
        assert_eq!(strings[0].range, 2..6); // 'a\'
    }

    #[test]
    fn symbols_find_functions() {
        let text = "\
greet() {
  echo hi
}

function serve {
  echo served
}
";
        let symbols = shell_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["greet", "serve"]);
        assert!(symbols.iter().all(|s| s.kind == SymbolKind::Function));
    }
}
