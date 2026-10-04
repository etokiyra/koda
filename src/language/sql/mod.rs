//! SQL provider.
//!
//! Built-in and offline, dialect-neutral: `--` and `/* … */` comments,
//! single-quoted strings and quoted identifiers, numbers, case-insensitive
//! keywords, common functions and operators. Structural diagnostics reuse the
//! shared delimiter checker and `sqls` supplies richer intelligence when it is
//! installed. Dialect-specific behaviour is intentionally not hard-coded here.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_number};
use crate::language::detection::LanguageDescriptor;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Location, Symbol, SymbolKind, word_at};

/// Keywords common to most SQL dialects.
const KEYWORDS: &[&str] = &[
    "add",
    "all",
    "alter",
    "and",
    "any",
    "as",
    "asc",
    "begin",
    "between",
    "by",
    "case",
    "cast",
    "check",
    "column",
    "commit",
    "constraint",
    "create",
    "cross",
    "database",
    "default",
    "delete",
    "desc",
    "distinct",
    "drop",
    "else",
    "end",
    "except",
    "exists",
    "false",
    "foreign",
    "from",
    "full",
    "group",
    "having",
    "if",
    "in",
    "index",
    "inner",
    "insert",
    "intersect",
    "into",
    "is",
    "join",
    "key",
    "left",
    "like",
    "limit",
    "not",
    "null",
    "on",
    "or",
    "order",
    "outer",
    "primary",
    "references",
    "returning",
    "right",
    "rollback",
    "select",
    "set",
    "table",
    "then",
    "true",
    "truncate",
    "union",
    "unique",
    "update",
    "using",
    "values",
    "view",
    "when",
    "where",
    "with",
];

const FUNCTIONS: &[&str] = &[
    "abs",
    "avg",
    "cast",
    "ceil",
    "coalesce",
    "concat",
    "count",
    "current_date",
    "current_time",
    "current_timestamp",
    "date_trunc",
    "floor",
    "greatest",
    "json_agg",
    "least",
    "length",
    "lower",
    "max",
    "min",
    "now",
    "nullif",
    "position",
    "round",
    "row_number",
    "substring",
    "sum",
    "trim",
    "upper",
];

const TYPES: &[&str] = &[
    "bigint",
    "blob",
    "boolean",
    "char",
    "date",
    "decimal",
    "double",
    "float",
    "int",
    "integer",
    "json",
    "numeric",
    "real",
    "serial",
    "smallint",
    "text",
    "time",
    "timestamp",
    "uuid",
    "varchar",
];

pub struct SqlProvider;

impl LanguageProvider for SqlProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Sql
    }

    fn display_name(&self) -> &'static str {
        "SQL"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Sql,
            extensions: &["sql", "psql", "ddl", "dml"],
            project_markers: &[],
            file_names: &[],
            shebangs: &[],
            content_hints: &["select ", "SELECT ", "create table", "CREATE TABLE"],
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
        sql_symbols(text)
    }

    fn definition(&self, text: &str, line: usize, col: usize) -> Option<Symbol> {
        let word = word_at(text, line, col)?;
        sql_symbols(text)
            .into_iter()
            .find(|symbol| symbol.name.eq_ignore_ascii_case(&word))
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
            .map(|word| Completion::new(word.to_ascii_uppercase(), CompletionKind::Keyword))
            .collect();
        completions.extend(
            FUNCTIONS
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Function)),
        );
        completions.extend(
            TYPES
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Type)),
        );
        completions
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = sql_symbols(text);
        crate::language::hover::describe(text, line, col, &symbols)
    }

    fn line_comment(&self) -> &'static str {
        "--"
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
                            ..Default::default()
                        },
                    );
                }
            }
        }

        while i < len {
            let c = chars[i];
            let next = chars.get(i + 1).copied();

            if c == '-' && next == Some('-') {
                push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                break;
            }
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

            if c == '\'' || c == '"' {
                // SQL doubles a quote to escape it.
                let mut j = i + 1;
                while j < len {
                    if chars[j] == c {
                        if chars.get(j + 1) == Some(&c) {
                            j += 2;
                            continue;
                        }
                        j += 1;
                        break;
                    }
                    j += 1;
                }
                let kind = if c == '\'' {
                    TokenKind::String
                } else {
                    TokenKind::Type
                };
                push_merged(&mut spans, HighlightSpan::new(i, j.min(len), kind));
                i = j.min(len);
                continue;
            }

            if c.is_ascii_digit() {
                let end = scan_number(&chars, i);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Number));
                i = end;
                continue;
            }

            if c.is_ascii_alphabetic() || c == '_' {
                let mut j = i;
                while j < len && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let word: String = chars[i..j].iter().collect();
                let lower = word.to_ascii_lowercase();
                let kind = if KEYWORDS.contains(&lower.as_str()) {
                    TokenKind::Keyword
                } else if TYPES.contains(&lower.as_str()) {
                    TokenKind::Type
                } else if FUNCTIONS.contains(&lower.as_str()) || chars.get(j) == Some(&'(') {
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
                '=' | '<' | '>' | '!' | '+' | '-' | '*' | '/' | '%' | '|' | ';' | ','
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

/// Tables, views, indexes, schemas, functions and procedures.
fn sql_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("--") {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        let (kind, prefix) = if lower.starts_with("create table") {
            (SymbolKind::Struct, "create table")
        } else if lower.starts_with("create view") {
            (SymbolKind::Type, "create view")
        } else if lower.starts_with("create index") {
            (SymbolKind::Key, "create index")
        } else if lower.starts_with("create schema") {
            (SymbolKind::Module, "create schema")
        } else if lower.starts_with("create function") {
            (SymbolKind::Function, "create function")
        } else if lower.starts_with("create procedure") {
            (SymbolKind::Function, "create procedure")
        } else {
            continue;
        };
        let rest = &trimmed[prefix.len()..];
        // Skip `if not exists` and qualifiers.
        let rest = rest
            .trim_start()
            .trim_start_matches("if not exists")
            .trim_start();
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
            .collect();
        if !name.is_empty() {
            let col = line
                .find(&name)
                .map(|byte| line[..byte].chars().count())
                .unwrap_or(0);
            symbols.push(Symbol::new(name, kind, row, col));
        }
    }
    symbols
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
    fn descriptor_claims_sql() {
        let descriptor = SqlProvider.descriptor();
        assert_eq!(descriptor.id, LanguageId::Sql);
        assert!(descriptor.extensions.contains(&"sql"));
    }

    #[test]
    fn highlights_keywords_case_insensitively() {
        let (spans, _) = SqlProvider.highlight(
            "SELECT id FROM users WHERE name = 'x';",
            HighlightState::default(),
        );
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Keyword)); // SELECT
        assert_eq!(kind_at(&spans, 7), Some(TokenKind::Plain)); // id
        assert_eq!(kind_at(&spans, 10), Some(TokenKind::Keyword)); // FROM
        assert_eq!(kind_at(&spans, 34), Some(TokenKind::String)); // 'x'
    }

    #[test]
    fn block_comments_carry_across_lines() {
        let (first, state) = SqlProvider.highlight("/* a comment", HighlightState::default());
        assert!(state.in_block_comment);
        assert_eq!(kind_at(&first, 0), Some(TokenKind::Comment));
        let (second, state) = SqlProvider.highlight("still */ SELECT 1", state);
        assert!(!state.in_block_comment);
        assert_eq!(kind_at(&second, 9), Some(TokenKind::Keyword));
    }

    #[test]
    fn symbols_cover_tables_and_views() {
        let text = "\
CREATE TABLE users (id INT);
create view active_users as select * from users;
CREATE INDEX idx_users ON users(id);
";
        let symbols = sql_symbols(text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["users", "active_users", "idx_users"]);
    }

    #[test]
    fn diagnostics_report_unbalanced_parentheses() {
        let diagnostics = SqlProvider.diagnostics("SELECT count(id FROM users;\n");
        assert!(diagnostics.iter().any(|d| d.message.contains("unclosed")));
    }
}
