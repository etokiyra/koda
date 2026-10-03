//! Assembly provider.
//!
//! Assembly is not one language, so this is a deliberately pragmatic baseline:
//! it highlights labels, directives, registers and common x86/x86-64 and
//! AArch64 mnemonics, and recognises both `;` (NASM/GAS) and `#` (GAS) comments.
//! The architecture is inferred only for completion purposes; the provider
//! never claims semantic understanding of a specific assembler.

use crate::language::completion::{Completion, CompletionKind};
use crate::language::data::{push_merged, scan_number, scan_quoted};
use crate::language::detection::LanguageDescriptor;
use crate::language::id::LanguageId;
use crate::language::provider::{
    Capability, HighlightSpan, HighlightState, LanguageProvider, TokenKind,
};
use crate::language::symbols::{Location, Symbol, SymbolKind, word_at};

const REGISTERS: &[&str] = &[
    // x86 / x86-64
    "rax", "rbx", "rcx", "rdx", "rsi", "rdi", "rbp", "rsp", "rip", "eax", "ebx", "ecx", "edx",
    "esi", "edi", "ebp", "esp", "ax", "bx", "cx", "dx", "al", "ah", "bl", "bh", "cl", "ch", "dl",
    "dh", "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15", "xmm0", "xmm1", "ymm0", "zmm0",
    // AArch64
    "sp", "xzr", "wzr", "lr", "pc", "x0", "x1", "x2", "x3", "x4", "x5", "x6", "x7", "w0", "w1",
    "w2", "w3", "v0", "v1", "d0", "s0", "q0",
];

const INSTRUCTIONS: &[&str] = &[
    // x86 / x86-64
    "mov",
    "movq",
    "movl",
    "movzbl",
    "push",
    "pop",
    "call",
    "ret",
    "jmp",
    "je",
    "jne",
    "jz",
    "jnz",
    "jg",
    "jl",
    "jge",
    "jle",
    "cmp",
    "test",
    "add",
    "sub",
    "mul",
    "imul",
    "div",
    "idiv",
    "inc",
    "dec",
    "and",
    "or",
    "xor",
    "not",
    "shl",
    "shr",
    "sar",
    "lea",
    "nop",
    "int",
    "syscall",
    "leave",
    "enter",
    "xchg",
    "cqo",
    "cdq",
    "bts",
    "btr",
    "bsf",
    "bsr",
    "sete",
    "setne",
    "loop",
    "rep",
    "movsb",
    "stosb",
    "lodsb",
    "addss",
    "addsd",
    "subsd",
    "mulsd",
    "divsd",
    "pxor",
    "movaps",
    "movups",
    "cvtsi2sd",
    "cvttsd2si",
    // AArch64
    "ldr",
    "str",
    "b",
    "bl",
    "blr",
    "cbz",
    "cbnz",
    "movz",
    "movk",
    "adr",
    "adrp",
    "tst",
    "orr",
    "eor",
    "lsl",
    "lsr",
    "asr",
    "ldp",
    "stp",
    "br",
    "ldar",
    "stlr",
    "svc",
];

const DIRECTIVES: &[&str] = &[
    ".section",
    ".text",
    ".data",
    ".bss",
    ".globl",
    ".global",
    ".type",
    ".size",
    ".align",
    ".byte",
    ".word",
    ".long",
    ".quad",
    ".asciz",
    ".ascii",
    ".equ",
    ".set",
    ".file",
    ".ident",
    "section",
    "global",
    "extern",
    "bits",
    "org",
    "db",
    "dw",
    "dd",
    "dq",
    "equ",
    "resb",
    "resw",
    "resd",
    "resq",
    "%define",
    "%macro",
    "%endmacro",
    "%include",
    "%ifdef",
    "%endif",
    ".cfi_startproc",
    ".cfi_endproc",
];

pub struct AsmProvider;

impl LanguageProvider for AsmProvider {
    fn id(&self) -> LanguageId {
        LanguageId::Assembly
    }

    fn display_name(&self) -> &'static str {
        "Assembly"
    }

    fn descriptor(&self) -> LanguageDescriptor {
        LanguageDescriptor {
            id: LanguageId::Assembly,
            extensions: &["asm", "s", "nasm", "inc"],
            project_markers: &[],
            file_names: &[],
            shebangs: &[],
            content_hints: &[".section", "section .text", "global ", "%define"],
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
        asm_symbols(text)
    }

    fn definition(&self, text: &str, line: usize, col: usize) -> Option<Symbol> {
        let word = word_at(text, line, col)?;
        asm_symbols(text)
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
        let mut completions: Vec<Completion> = INSTRUCTIONS
            .iter()
            .map(|word| Completion::new(*word, CompletionKind::Function))
            .collect();
        completions.extend(
            REGISTERS
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Variable)),
        );
        completions.extend(
            DIRECTIVES
                .iter()
                .map(|word| Completion::new(*word, CompletionKind::Keyword)),
        );
        completions
    }

    fn hover(&self, text: &str, line: usize, col: usize) -> Option<crate::language::hover::Hover> {
        let symbols = asm_symbols(text);
        crate::language::hover::describe(text, line, col, &symbols)
    }

    fn line_comment(&self) -> &'static str {
        ";"
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

            if c == ';' || c == '#' {
                push_merged(&mut spans, HighlightSpan::new(i, len, TokenKind::Comment));
                break;
            }
            if c == '"' || c == '\'' {
                let end = scan_quoted(&chars, i, c);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::String));
                i = end;
                continue;
            }
            if c == '%' && chars.get(i + 1) == Some(&'%') {
                let mut j = i + 2;
                while j < len && is_ident_continue(chars[j]) {
                    j += 1;
                }
                push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Type));
                i = j;
                continue;
            }
            if c == '$' && chars.get(i + 1).is_some_and(|c| c.is_ascii_hexdigit()) {
                let mut j = i + 1;
                while j < len && chars[j].is_ascii_hexdigit() {
                    j += 1;
                }
                push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Number));
                i = j;
                continue;
            }
            if c.is_ascii_digit() {
                let end = scan_number(&chars, i);
                push_merged(&mut spans, HighlightSpan::new(i, end, TokenKind::Number));
                i = end;
                continue;
            }
            if c == '.' && chars.get(i + 1).is_some_and(|c| c.is_alphabetic()) {
                let mut j = i + 1;
                while j < len && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                push_merged(&mut spans, HighlightSpan::new(i, j, TokenKind::Keyword));
                i = j;
                continue;
            }
            if is_ident_start(c) {
                let mut j = i;
                while j < len && is_ident_continue(chars[j]) {
                    j += 1;
                }
                let word: String = chars[i..j].iter().collect();
                let lower = word.to_ascii_lowercase();
                // A label is an identifier at the start of a statement followed
                // by `:`, or a directive/mnemonic/register/plain identifier.
                let kind = if chars.get(j) == Some(&':') && i == first_nonspace(&chars) {
                    TokenKind::Type
                } else if DIRECTIVES.contains(&lower.as_str())
                    || DIRECTIVES.contains(&word.as_str())
                {
                    TokenKind::Keyword
                } else if REGISTERS.contains(&lower.as_str()) {
                    TokenKind::Type
                } else if INSTRUCTIONS.contains(&lower.as_str()) {
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
                ',' | '+' | '-' | '*' | ':' | '=' | '<' | '>' | '&' | '|' | '^' | '~'
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

fn first_nonspace(chars: &[char]) -> usize {
    chars
        .iter()
        .position(|c| !c.is_whitespace())
        .unwrap_or(chars.len())
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c == '.' || c == '$' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c == '.' || c == '$' || c.is_alphanumeric()
}

/// Labels (`name:`) and `global`/`globl` exports.
fn asm_symbols(text: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(rest) = lower
            .strip_prefix("global ")
            .or_else(|| lower.strip_prefix(".global "))
            .or_else(|| lower.strip_prefix(".globl "))
        {
            let name: String = rest
                .trim_start()
                .chars()
                .take_while(|c| is_ident_continue(*c))
                .collect();
            if !name.is_empty() {
                let col = line
                    .find(&name)
                    .map(|b| line[..b].chars().count())
                    .unwrap_or(0);
                symbols.push(Symbol::new(name, SymbolKind::Function, row, col));
            }
            continue;
        }
        if let Some(colon) = trimmed.find(':') {
            let name = trimmed[..colon].trim();
            let valid = !name.is_empty() && name.chars().all(|c| is_ident_continue(c) || c == '@');
            if valid {
                let col = line
                    .find(name)
                    .map(|b| line[..b].chars().count())
                    .unwrap_or(0);
                symbols.push(Symbol::new(name, SymbolKind::Function, row, col));
            }
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
    fn descriptor_claims_assembly() {
        let descriptor = AsmProvider.descriptor();
        assert_eq!(descriptor.id, LanguageId::Assembly);
        assert!(descriptor.extensions.contains(&"asm"));
    }

    #[test]
    fn highlights_instructions_registers_and_directives() {
        let (spans, _) =
            AsmProvider.highlight("    mov rax, 1    ; load 1", HighlightState::default());
        assert_eq!(kind_at(&spans, 4), Some(TokenKind::Function)); // mov
        assert_eq!(kind_at(&spans, 8), Some(TokenKind::Type)); // rax
        assert_eq!(kind_at(&spans, 20), Some(TokenKind::Comment));
    }

    #[test]
    fn labels_are_types_and_symbols() {
        let (spans, _) = AsmProvider.highlight("_start:", HighlightState::default());
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Type));
        let symbols = asm_symbols("_start:\n    nop\n");
        assert_eq!(symbols[0].name, "_start");
    }

    #[test]
    fn hash_and_semicolon_are_comments() {
        let (spans, _) = AsmProvider.highlight("# gas comment", HighlightState::default());
        assert_eq!(kind_at(&spans, 0), Some(TokenKind::Comment));
        let (spans, _) = AsmProvider.highlight("mov eax, 1 ; nasm", HighlightState::default());
        assert_eq!(kind_at(&spans, 11), Some(TokenKind::Comment));
    }

    #[test]
    fn symbols_include_global_exports() {
        let symbols = asm_symbols("global main\nmain:\n    ret\n");
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["main", "main"]);
    }
}
