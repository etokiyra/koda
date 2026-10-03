//! Terminal lifecycle and terminal-native services.
//!
//! Koda cooperates with the terminal rather than hiding it. In particular it never
//! paints an opaque background, so transparent terminals keep working.

use std::io::{self, Write};

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange,
};
use crossterm::execute;
use ratatui::DefaultTerminal;

/// Initialize the terminal: raw mode, alternate screen, bracketed paste and
/// focus reporting.
pub fn init() -> io::Result<DefaultTerminal> {
    let terminal = ratatui::try_init()?;
    // Bracketed paste lets the terminal deliver a paste as one event; focus
    // reporting lets Koda refresh the tree when the user returns to it.
    let _ = execute!(io::stdout(), EnableBracketedPaste, EnableFocusChange);
    Ok(terminal)
}

/// Restore the terminal to its previous state.
pub fn restore() {
    let _ = execute!(io::stdout(), DisableBracketedPaste, DisableFocusChange);
    ratatui::restore();
}

/// Copy text to the system clipboard using the OSC 52 escape sequence.
///
/// This works over SSH and in most modern terminals. Terminals that do not
/// support it simply ignore the sequence.
pub fn set_clipboard(text: &str) {
    let encoded = base64(text.as_bytes());
    let mut stdout = io::stdout();
    let _ = write!(stdout, "\x1b]52;c;{encoded}\x07");
    let _ = stdout.flush();
}

/// A tiny standard-alphabet base64 encoder, kept local to avoid a dependency.
fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn encodes_base64() {
        assert_eq!(base64(b"hello"), "aGVsbG8=");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
    }
}
