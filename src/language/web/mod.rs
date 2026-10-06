//! TypeScript and JavaScript provider.
//!
//! One scanner serves both languages because their surface syntax overlaps
//! almost entirely; the two differ in a handful of keywords and types and in
//! which files they claim. Like the other providers this is entirely built-in
//! and offline: highlighting, structural diagnostics, symbols, completion,
//! hover and within-file navigation. `typescript-language-server` can be
//! provisioned for rename, code actions and richer, type-aware analysis.
//!
//! Multi-line block comments, template literals and JSX tags carry over between
//! lines; a `${ … }` interpolation resumes code highlighting and returns to
//! template text once its braces close.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_number, scan_quoted};
use crate::language::detection::LanguageDescriptor;
use crate::language::diagnostics::Diagnostic;
use crate::language::format::FormatOutcome;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, LexMode, TokenKind,
};
use crate::language::symbols::{Symbol, SymbolKind};

use std::path::Path;

/// Keywords common to JavaScript and TypeScript.
const KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "asserts",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "declare",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "finally",
    "for",
    "from",
    "function",
    "get",
    "if",
    "implements",
    "import",
    "in",
    "infer",
    "instanceof",
    "interface",
    "is",
    "keyof",
    "let",
    "namespace",
    "new",
    "of",
    "override",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "satisfies",
    "set",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "try",
    "type",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// TypeScript primitive and utility types.
const TYPES: &[&str] = &[
    "any",
    "bigint",
    "boolean",
    "never",
    "number",
    "object",
    "string",
    "symbol",
    "undefined",
    "unknown",
    "Array",
    "ReadonlyArray",
    "Promise",
    "Record",
    "Partial",
    "Required",
    "Readonly",
    "Pick",
    "Omit",
    "Exclude",
    "Extract",
    "ReturnType",
    "Parameters",
];

const BUILTINS: &[&str] = &[
    "Array",
    "BigInt",
    "Boolean",
    "Date",
    "Error",
    "Function",
    "JSON",
    "Map",
    "Math",
    "Number",
    "Object",
    "Promise",
    "RegExp",
    "Set",
    "String",
    "Symbol",
    "WeakMap",
    "clearInterval",
    "clearTimeout",
    "console",
    "decodeURIComponent",
    "document",
    "encodeURIComponent",
    "exports",
    "fetch",
    "globalThis",
    "isFinite",
    "isNaN",
    "module",
    "parseFloat",
    "parseInt",
    "process",
    "queueMicrotask",
    "require",
    "setInterval",
    "setTimeout",
    "structuredClone",
    "window",
];

const CONSTANTS: &[&str] = &["true", "false", "null", "undefined", "NaN", "Infinity"];

/// A provider instance for one of the two web languages.
pub struct WebProvider {
    id: LanguageId,
}

impl WebProvider {
    pub fn typescript() -> Self {
        WebProvider {
            id: LanguageId::TypeScript,
        }
    }

    pub fn javascript() -> Self {
        WebProvider {
            id: LanguageId::JavaScript,
        }
    }
}

impl LanguageProvider for WebProvider {
    fn id(&self) -> LanguageId {
        self.id
    }

    fn display_name(&self) -> &'static str {
        self.id.name()
    }

    fn descriptor(&self) -> LanguageDescriptor {
        match self.id {
            LanguageId::TypeScript => LanguageDescriptor {
                id: LanguageId::TypeScript,
                extensions: &["ts", "tsx", "mts", "cts"],
                project_markers: &["tsconfig.json", "package.json"],
                file_names: &[],
                shebangs: &[],
                // TypeScript is a superset of JavaScript, so it shares the
                // generic hints and adds its own; otherwise a `.tsx` file with
                // only JS-shaped content would lose to the JavaScript provider.
                content_hints: &[
                    "interface ",
                    "type ",
                    ": string",
                    ": number",
                    "=>",
                    "const ",
                ],
            },
            _ => LanguageDescriptor {
                id: LanguageId::JavaScript,
                extensions: &["js", "jsx", "mjs", "cjs"],
                project_markers: &["package.json"],
                file_names: &[],
                shebangs: &["node"],
                content_hints: &["const ", "=>", "console.log", "function "],
            },
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
            Capability::Formatting,
        ]
    }

    fn diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        crate::language::diagnostics::check_delimiters(self, text)
    }

    fn symbols(&self, text: &str) -> Vec<Symbol> {
        web_symbols(text)
    }

    fn definition(
        &self,
        text: &str,
        line: usize,
        col: usize,
    ) -> Option<crate::language::symbols::Symbol> {
        let word = crate::language::symbols::word_at(text, line, col)?;
        web_symbols(text)
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

    fn format(&self, path: &Path, text: &str) -> FormatOutcome {
        crate::language::format::prettier(path, text)
    }

    fn formatter(&self) -> Option<&'static str> {
        Some("prettier")
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = web_symbols(text);
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
        let mut mode = state.mode;
        let mut in_block_comment = state.in_block_comment;

        // Finish a block comment carried over from the previous line.
        if in_block_comment {
            match find_block_end(&chars, 0) {
                Some(end) => {
                    push_merged(&mut spans, HighlightSpan::new(0, end, TokenKind::Comment));
                    i = end;
                    in_block_comment = false;
                }
                None => {
                    if len > 0 {
                        push_merged(&mut spans, HighlightSpan::new(0, len, TokenKind::Comment));
                    }
                    return (
                        spans,
                        HighlightState {
                            in_block_comment: true,
                            block_comment_depth: 0,
                            mode,
                            embed: state.embed,
                        },
                    );
                }
            }
        }

        // Resume template text carried over from the previous line.
        if mode == LexMode::Template {
            let (next, next_mode) = scan_template_text(&chars, i, &mut spans);
            i = next;
            mode = next_mode;
        }

        // Resume the attributes of a JSX tag that spanned lines.
        if mode == LexMode::JsxTag {
            let (next, next_mode) = scan_jsx_attributes(&chars, i, &mut spans);
            i = next;
            mode = next_mode;
        }

        let first_nonspace = chars.iter().position(|c| !c.is_whitespace());

        while i < len {
            let c = chars[i];

            // Inside `${ ... }` the braces must be balanced so template text can
            // resume once the interpolation closes.
            if let LexMode::TemplateInterpolation(depth) = mode {
                if c == '{' {
                    push_merged(
                        &mut spans,
                        HighlightSpan::new(i, i + 1, TokenKind::Operator),
                    );
                    mode = LexMode::TemplateInterpolation(depth.saturating_add(1));
                    i += 1;
                    continue;
                }
                if c == '}' {
                    push_merged(
                        &mut spans,
                        HighlightSpan::new(i, i + 1, TokenKind::Operator),
                    );
                    i += 1;
                    if depth <= 1 {
                        let (next, next_mode) = scan_template_text(&chars, i, &mut spans);
                        i = next;
                        mode = next_mode;
                    } else {
                        mode = LexMode::TemplateInterpolation(depth - 1);
                    }
                    continue;
                }
            }

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
                                block_comment_depth: 0,
                                mode,
                                embed: state.embed,
                            },
                        );
                    }
                }
                continue;
            }

            // A `/` in expression position starts a regex literal; otherwise it
            // stays a division operator. This is the usual lookback heuristic.
            if c == '/'
                && regex_can_start(&chars, i)
                && let Some(end) = scan_regex(&chars, i)
            {
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            if c == '\'' || c == '"' {
                let end = scan_quoted(&chars, i, c);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }

            if c == '`' {
                let saved = mode;
                let (end, returned) = scan_template(&chars, i, &mut spans);
                i = end;
                // A nested template that closes on this line restores the
                // enclosing interpolation; otherwise the returned mode carries.
                mode = match (saved, returned) {
                    (LexMode::TemplateInterpolation(depth), LexMode::None) => {
                        LexMode::TemplateInterpolation(depth)
                    }
                    _ => returned,
                };
                continue;
            }

            if c.is_ascii_digit() {
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

                // `function name` / `class Name`: highlight the name too.
                let name_kind = match word.as_str() {
                    "function" => Some(TokenKind::Function),
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

            // A `<` in expression position opens a JSX element; a comparison or
            // generic angle bracket stays an operator.
            if c == '<' && jsx_can_start(&chars, i) {
                let (end, jsx_mode) = highlight_jsx(&chars, i, &mut spans);
                if end > i {
                    i = end;
                    if jsx_mode != LexMode::None {
                        mode = jsx_mode;
                    }
                    continue;
                }
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
                in_block_comment,
                block_comment_depth: 0,
                mode,
                embed: state.embed,
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

/// The identifier following a `function`/`class`, skipping spaces and a
/// generator `*`.
fn name_after(chars: &[char], from: usize) -> Option<(usize, usize)> {
    let mut j = from;
    while j < chars.len() && chars[j] == ' ' {
        j += 1;
    }
    if chars.get(j) == Some(&'*') {
        j += 1;
        while j < chars.len() && chars[j] == ' ' {
            j += 1;
        }
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

/// End of a `*/` run starting at `from` (past the opening `/*`).
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

/// Scan a template literal starting at `start` (the opening backtick).
///
/// Emits the template text and `${` boundaries, and returns the index reached
/// with the carry-over mode: `Template` (or an interpolation) when the literal is
/// still open at the end of the line, otherwise `None`.
fn scan_template(chars: &[char], start: usize, spans: &mut Vec<HighlightSpan>) -> (usize, LexMode) {
    push_merged(
        spans,
        HighlightSpan::new(start, start + 1, TokenKind::String),
    );
    scan_template_text(chars, start + 1, spans)
}

/// Scan template text from `i` until the closing backtick or a `${`.
fn scan_template_text(
    chars: &[char],
    mut i: usize,
    spans: &mut Vec<HighlightSpan>,
) -> (usize, LexMode) {
    let len = chars.len();
    let start = i;
    while i < len {
        match chars[i] {
            '\\' => i = (i + 2).min(len),
            '`' => {
                push_merged(spans, HighlightSpan::new(start, i + 1, TokenKind::String));
                return (i + 1, LexMode::None);
            }
            '$' if chars.get(i + 1) == Some(&'{') => {
                push_merged(spans, HighlightSpan::new(start, i, TokenKind::String));
                push_merged(spans, HighlightSpan::new(i, i + 2, TokenKind::Operator));
                return (i + 2, LexMode::TemplateInterpolation(1));
            }
            _ => i += 1,
        }
    }
    push_merged(spans, HighlightSpan::new(start, len, TokenKind::String));
    (len, LexMode::Template)
}

/// The word ending immediately before `end` (skipping whitespace), if any.
fn word_before(chars: &[char], end: usize) -> String {
    let mut j = end;
    while j > 0 && chars[j - 1].is_whitespace() {
        j -= 1;
    }
    let mut k = j;
    while k > 0 && is_ident_continue(chars[k - 1]) {
        k -= 1;
    }
    chars[k..j].iter().collect()
}

/// Whether a `/` at `i` begins a regex literal rather than a division.
///
/// A regex may follow an expression-start operator or a keyword that takes an
/// expression; after a value (identifier, number, `)`, `]`) it is division.
fn regex_can_start(chars: &[char], i: usize) -> bool {
    let mut j = i;
    while j > 0 && chars[j - 1].is_whitespace() {
        j -= 1;
    }
    if j == 0 {
        return true;
    }
    let prev = chars[j - 1];
    if matches!(
        prev,
        '(' | ','
            | '='
            | ':'
            | '['
            | '!'
            | '&'
            | '|'
            | '?'
            | '{'
            | '}'
            | ';'
            | '+'
            | '-'
            | '*'
            | '%'
            | '<'
            | '>'
            | '~'
            | '^'
    ) {
        return true;
    }
    if is_ident_continue(prev) {
        return matches!(
            word_before(chars, j).as_str(),
            "return"
                | "typeof"
                | "instanceof"
                | "in"
                | "of"
                | "new"
                | "delete"
                | "void"
                | "case"
                | "do"
                | "else"
                | "yield"
                | "await"
                | "default"
        );
    }
    false
}

/// The end of a regex literal starting at `/`, honouring escapes and classes.
fn scan_regex(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start + 1;
    let mut in_class = false;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '[' => {
                in_class = true;
                i += 1;
            }
            ']' => {
                in_class = false;
                i += 1;
            }
            '/' if !in_class => {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_alphabetic() {
                    j += 1;
                }
                return Some(j);
            }
            _ => i += 1,
        }
    }
    None
}

/// Whether a `<` at `i` opens a JSX element rather than a comparison/generic.
fn jsx_can_start(chars: &[char], i: usize) -> bool {
    let Some(next) = chars.get(i + 1) else {
        return false;
    };
    // A closing tag `</…>` can follow text content, so it is accepted wherever
    // it appears (`< /` is not valid JavaScript).
    if *next == '/' {
        return chars
            .get(i + 2)
            .is_some_and(|c| c.is_alphabetic() || *c == '_' || *c == '$' || *c == '>');
    }
    if !(next.is_alphabetic() || *next == '_' || *next == '$' || *next == '>') {
        return false;
    }
    let mut j = i;
    while j > 0 && chars[j - 1].is_whitespace() {
        j -= 1;
    }
    if j == 0 {
        return true;
    }
    let prev = chars[j - 1];
    if matches!(
        prev,
        '(' | ',' | '=' | '>' | ':' | '[' | '!' | '&' | '|' | '?' | '{' | '}' | ';' | '+'
    ) {
        return true;
    }
    if is_ident_continue(prev) {
        return matches!(
            word_before(chars, j).as_str(),
            "return" | "default" | "else" | "yield" | "await" | "case" | "in" | "of"
        );
    }
    false
}

/// Highlight a JSX tag starting at `<`, returning the index past it.
fn highlight_jsx(chars: &[char], start: usize, spans: &mut Vec<HighlightSpan>) -> (usize, LexMode) {
    let len = chars.len();
    let mut i = start + 1;
    let closing = chars.get(i) == Some(&'/');
    if closing {
        i += 1;
    }
    // A fragment `<>` or closing fragment `</>`.
    if chars.get(i) == Some(&'>') {
        push_merged(spans, HighlightSpan::new(start, i + 1, TokenKind::Operator));
        return (i + 1, LexMode::None);
    }
    let name_start = i;
    while i < len && (is_ident_continue(chars[i]) || matches!(chars[i], '.' | ':' | '-')) {
        i += 1;
    }
    if i == name_start {
        return (start, LexMode::None); // Not a tag; let the caller treat `<` as an operator.
    }
    push_merged(spans, HighlightSpan::new(name_start, i, TokenKind::Type));
    scan_jsx_attributes(chars, i, spans)
}

/// Highlight JSX tag attributes from `i` until the closing `>`.
///
/// Returns the index reached and a carry-over mode: `JsxTag` when the tag is
/// still open at the end of the line (its attributes continue on the next one).
fn scan_jsx_attributes(
    chars: &[char],
    mut i: usize,
    spans: &mut Vec<HighlightSpan>,
) -> (usize, LexMode) {
    let len = chars.len();
    while i < len && chars[i] != '>' {
        let c = chars[i];
        if c.is_whitespace() || c == '/' {
            i += 1;
            continue;
        }
        if c == '{' {
            i = scan_braces(chars, i);
            continue;
        }
        if c == '"' || c == '\'' {
            let end = scan_quoted(chars, i, c);
            push_merged(spans, HighlightSpan::new(i, end, TokenKind::String));
            i = end;
            continue;
        }
        if c == '=' {
            push_merged(spans, HighlightSpan::new(i, i + 1, TokenKind::Operator));
            i += 1;
            continue;
        }
        let attr_start = i;
        while i < len
            && !chars[i].is_whitespace()
            && !matches!(chars[i], '=' | '>' | '/' | '"' | '\'' | '{')
        {
            i += 1;
        }
        push_merged(
            spans,
            HighlightSpan::new(attr_start, i, TokenKind::Attribute),
        );
    }
    if i < len {
        push_merged(spans, HighlightSpan::new(i, i + 1, TokenKind::Operator));
        return (i + 1, LexMode::None);
    }
    (len, LexMode::JsxTag)
}

/// The index just past the `}` matching the `{` at `start`.
fn scan_braces(chars: &[char], start: usize) -> usize {
    let mut depth = 0usize;
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    chars.len()
}

/// Words that may precede a declaration keyword.
const MODIFIERS: &[&str] = &[
    "export",
    "default",
    "async",
    "declare",
    "abstract",
    "public",
    "private",
    "protected",
    "static",
    "readonly",
    "override",
];

/// Named top-level declarations in a document.
fn web_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = skip_ws(&chars, 0);
        // Skip leading modifiers so `export default class` still registers.
        loop {
            let (word, next) = word_at(&chars, i);
            if MODIFIERS.contains(&word.as_str()) {
                i = skip_ws(&chars, next);
            } else {
                break;
            }
        }
        let (keyword, next) = word_at(&chars, i);
        let kind = match keyword.as_str() {
            "function" => SymbolKind::Function,
            "class" => SymbolKind::Type,
            "interface" => SymbolKind::Interface,
            "enum" => SymbolKind::Enum,
            "type" => SymbolKind::Type,
            "namespace" | "module" => SymbolKind::Module,
            "const" => SymbolKind::Constant,
            "let" | "var" => SymbolKind::Variable,
            _ => continue,
        };
        let mut j = skip_ws(&chars, next);
        if keyword == "function" && chars.get(j) == Some(&'*') {
            j = skip_ws(&chars, j + 1);
        }
        let (name, _) = word_at(&chars, j);
        if name.is_empty() {
            continue;
        }
        symbols.push(Symbol::new(name, kind, row, j));
    }
    symbols
}

fn skip_ws(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

/// The identifier starting at `i` and the index just past it.
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

fn is_ident_start(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphanumeric()
}

fn is_operator(c: char) -> bool {
    matches!(
        c,
        '+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | '~' | '?' | ':'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_claim_the_right_extensions() {
        let ts = WebProvider::typescript().descriptor();
        assert_eq!(ts.id, LanguageId::TypeScript);
        assert!(ts.extensions.contains(&"ts") && ts.extensions.contains(&"tsx"));
        let js = WebProvider::javascript().descriptor();
        assert_eq!(js.id, LanguageId::JavaScript);
        assert!(js.extensions.contains(&"js") && js.extensions.contains(&"jsx"));
    }

    #[test]
    fn highlights_comments_strings_and_keywords() {
        let provider = WebProvider::typescript();
        let (spans, state) =
            provider.highlight("const name = \"koda\"; // hi", HighlightState::default());
        assert!(!state.in_block_comment);
        let kind_at = |col: usize| {
            spans
                .iter()
                .find(|span| span.range.contains(&col))
                .map(|span| span.kind)
        };
        assert_eq!(kind_at(0), Some(TokenKind::Keyword)); // const
        assert_eq!(kind_at(13), Some(TokenKind::String)); // "koda"
        assert_eq!(kind_at(21), Some(TokenKind::Comment)); // // hi
    }

    #[test]
    fn carries_block_comments_across_lines() {
        let provider = WebProvider::typescript();
        let (_, state) = provider.highlight("/* open", HighlightState::default());
        assert!(state.in_block_comment);
        let (spans, state) = provider.highlight(" still open */ let x = 1;", state);
        assert!(!state.in_block_comment);
        assert!(
            spans
                .iter()
                .any(|span| span.kind == TokenKind::Comment && span.range.contains(&0))
        );
    }

    #[test]
    fn carries_template_literals_and_interpolation_across_lines() {
        let provider = WebProvider::typescript();
        // A template opened on one line stays a string on the next.
        let (spans, state) = provider.highlight("const s = `hello", HighlightState::default());
        assert_eq!(state.mode, LexMode::Template);
        assert_eq!(kind_at(&spans, 10), Some(TokenKind::String));

        // `${name}` resumes code, and the text after `}` is a string again.
        let (spans, state) = provider.highlight("world ${name} tail`", state);
        assert_eq!(state.mode, LexMode::None);
        assert_eq!(kind_at(&spans, 1), Some(TokenKind::String)); // "world "
        assert_eq!(kind_at(&spans, 9), Some(TokenKind::Plain)); // name
        assert_eq!(kind_at(&spans, 16), Some(TokenKind::String)); // " tail`"
    }

    #[test]
    fn carries_jsx_tags_across_lines() {
        let provider = WebProvider::typescript();
        let (spans, state) = provider.highlight("const el = <Panel", HighlightState::default());
        assert_eq!(state.mode, LexMode::JsxTag);
        assert_eq!(kind_at(&spans, 12), Some(TokenKind::Type)); // Panel

        let (spans, state) = provider.highlight("  title=\"hi\"", state);
        assert_eq!(state.mode, LexMode::JsxTag);
        assert_eq!(kind_at(&spans, 2), Some(TokenKind::Attribute)); // title
        assert_eq!(kind_at(&spans, 8), Some(TokenKind::String)); // "hi"

        let (spans, state) = provider.highlight("  visible>", state);
        assert_eq!(state.mode, LexMode::None);
        assert_eq!(kind_at(&spans, 2), Some(TokenKind::Attribute)); // visible
        assert_eq!(kind_at(&spans, 9), Some(TokenKind::Operator)); // >
    }

    #[test]
    fn an_open_interpolation_carries_its_depth() {
        let provider = WebProvider::typescript();
        let (_, state) = provider.highlight("`a ${b", HighlightState::default());
        assert_eq!(state.mode, LexMode::TemplateInterpolation(1));
        // The closing brace returns to template text, which needs a backtick.
        let (spans, state) = provider.highlight("} c`", state);
        assert_eq!(state.mode, LexMode::None);
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Operator)); // }
        assert_eq!(kind_at(&spans, 2), Some(TokenKind::String)); // c`
    }

    #[test]
    fn interpolation_braces_are_balanced() {
        let provider = WebProvider::typescript();
        // A nested `{}` inside the interpolation must not close it early.
        let (_, state) = provider.highlight("`a ${ ({x: 1}) } b`", HighlightState::default());
        assert_eq!(state.mode, LexMode::None);
    }

    #[test]
    fn extracts_declarations() {
        let text = "import x from \"y\";\n\
                    export async function run() {}\n\
                    export default class Server {}\n\
                    interface Options {}\n\
                    type Handler = () => void;\n\
                    const PORT = 8080;\n";
        let symbols = web_symbols(text);
        let names: Vec<_> = symbols
            .iter()
            .map(|symbol| (symbol.name.as_str(), symbol.kind))
            .collect();
        assert!(names.contains(&("run", SymbolKind::Function)));
        assert!(names.contains(&("Server", SymbolKind::Type)));
        assert!(names.contains(&("Options", SymbolKind::Interface)));
        assert!(names.contains(&("Handler", SymbolKind::Type)));
        assert!(names.contains(&("PORT", SymbolKind::Constant)));
    }

    fn kind_at(spans: &[HighlightSpan], col: usize) -> Option<TokenKind> {
        spans
            .iter()
            .find(|span| span.range.contains(&col))
            .map(|span| span.kind)
    }

    #[test]
    fn regex_literals_are_strings_but_division_is_an_operator() {
        let provider = WebProvider::javascript();
        let (spans, _) = provider.highlight("let re = /ab+c/gi;", HighlightState::default());
        assert_eq!(kind_at(&spans, 9), Some(TokenKind::String)); // /ab+c/gi
        assert_eq!(kind_at(&spans, 16), Some(TokenKind::String)); // flags included

        let (spans, _) = provider.highlight("let x = a / b / c;", HighlightState::default());
        assert_eq!(kind_at(&spans, 10), Some(TokenKind::Operator)); // first /
        assert_eq!(kind_at(&spans, 14), Some(TokenKind::Operator)); // second /

        // A regex after `return` is not division.
        let (spans, _) = provider.highlight("return /x/.test(s)", HighlightState::default());
        assert_eq!(kind_at(&spans, 7), Some(TokenKind::String));
    }

    #[test]
    fn jsx_tags_and_attributes_highlight() {
        let provider = WebProvider::typescript();
        let (spans, _) = provider.highlight(
            "return <div className=\"card\">Hi</div>;",
            HighlightState::default(),
        );
        let div = "return <div className=\"card\">Hi</div>;";
        let div_col = div.find("div").unwrap();
        assert_eq!(kind_at(&spans, div_col), Some(TokenKind::Type));
        assert_eq!(kind_at(&spans, 12), Some(TokenKind::Attribute)); // className
        assert_eq!(kind_at(&spans, 22), Some(TokenKind::String)); // "card"
        // The closing tag's name is highlighted too.
        let close_col = div.rfind("div").unwrap();
        assert_eq!(kind_at(&spans, close_col), Some(TokenKind::Type));
    }

    #[test]
    fn generics_and_comparisons_are_not_jsx() {
        let provider = WebProvider::typescript();
        let (spans, _) = provider.highlight("let xs: Array<Foo> = [];", HighlightState::default());
        assert_eq!(kind_at(&spans, 13), Some(TokenKind::Operator)); // <
        let (spans, _) = provider.highlight("if (a < b) {}", HighlightState::default());
        assert_eq!(kind_at(&spans, 6), Some(TokenKind::Operator)); // <
    }

    #[test]
    fn jsx_fragments_and_self_closing_tags() {
        let provider = WebProvider::javascript();
        let (spans, _) = provider.highlight("return <></>;", HighlightState::default());
        assert_eq!(kind_at(&spans, 7), Some(TokenKind::Operator)); // <
        let (spans, _) =
            provider.highlight("const el = <Input value={x} />;", HighlightState::default());
        let input_col = "const el = <Input value={x} />;".find("Input").unwrap();
        assert_eq!(kind_at(&spans, input_col), Some(TokenKind::Type));
    }

    #[test]
    fn web_providers_offer_prettier_formatting() {
        for provider in [WebProvider::typescript(), WebProvider::javascript()] {
            assert!(
                provider.capabilities().contains(&Capability::Formatting),
                "web providers expose Formatting"
            );
            assert_eq!(provider.formatter(), Some("prettier"));
        }
    }
}
