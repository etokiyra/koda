//! Shared lexical scanning helpers for the data-format providers.
//!
//! JSON, TOML and YAML differ in their surface grammar but share the same small
//! building blocks: quoted strings, numbers, comments and simple identifiers.
//! Keeping those scanners here means each provider can stay focused on its own
//! structure without three near-identical copies drifting apart.
//!
//! Everything operates on `&[char]` and measures positions in **characters**, so
//! the spans providers hand back are always valid for rendering.

use crate::language::provider::HighlightSpan;

/// Advance past a quoted string that starts at `start`, honouring `\` escapes.
///
/// The returned index is one past the closing quote, or the end of input when
/// the string is unterminated.
pub fn scan_quoted(chars: &[char], start: usize, quote: char) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            c if c == quote => return (i + 1).min(chars.len()),
            _ => i += 1,
        }
    }
    chars.len()
}

/// Advance past a single-quoted, non-escaping string (TOML/YAML literal form).
pub fn scan_single_quoted(chars: &[char], start: usize) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == '\'' {
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

/// Advance past a numeric literal (digits, separators, decimal point and
/// exponent), including a leading sign.
pub fn scan_number(chars: &[char], start: usize) -> usize {
    let mut i = start;
    if matches!(chars.get(i), Some('+') | Some('-')) {
        i += 1;
    }
    while i < chars.len() {
        let c = chars[i];
        let scalar = c.is_ascii_alphanumeric() || c == '.' || c == '_';
        // A sign only belongs to the number when it follows an exponent marker.
        let exponent_sign =
            (c == '+' || c == '-') && i > start && matches!(chars[i - 1], 'e' | 'E');
        if scalar || exponent_sign {
            i += 1;
        } else {
            break;
        }
    }
    i
}

/// Append a span, merging it into the previous one when they are contiguous and
/// the same kind. Keeps highlight runs tidy for the renderer.
pub fn push_merged(spans: &mut Vec<HighlightSpan>, span: HighlightSpan) {
    if let Some(last) = spans.last_mut()
        && last.kind == span.kind
        && last.range.end == span.range.start
    {
        last.range.end = span.range.end;
        return;
    }
    spans.push(span);
}
