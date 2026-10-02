//! Visual identity.
//!
//! Koda stays terminal-native: default backgrounds are left untouched so
//! transparent terminals keep working. Colour is used deliberately for syntax,
//! status and overlays.

use ratatui::style::{Color, Modifier, Style};

use crate::language::provider::TokenKind;

// A calm, modern palette (Tokyo Night inspired). Reserving `Color::Reset` for
// plain text means the terminal's own foreground/background shine through.
pub const ACCENT: Color = Color::Rgb(122, 162, 247);
pub const ACCENT_ALT: Color = Color::Rgb(187, 154, 247);
pub const SUCCESS: Color = Color::Rgb(158, 206, 106);
pub const WARN: Color = Color::Rgb(224, 175, 104);
pub const ERROR: Color = Color::Rgb(247, 118, 142);
pub const DIM: Color = Color::Rgb(86, 95, 137);
pub const MUTED: Color = Color::Rgb(120, 130, 165);
pub const TEXT: Color = Color::Reset;

pub const SELECTION_BG: Color = Color::Rgb(45, 62, 100);
pub const SEARCH_BG: Color = Color::Rgb(90, 78, 45);
pub const SEARCH_CURRENT_BG: Color = Color::Rgb(150, 110, 40);
pub const OVERLAY_BG: Color = Color::Rgb(22, 24, 36);
pub const SIDEBAR_SELECTED_BG: Color = Color::Rgb(34, 39, 58);
pub const CURSOR_LINE_BG: Color = Color::Rgb(28, 31, 45);

pub fn accent() -> Style {
    Style::default().fg(ACCENT)
}

pub fn accent_bold() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn dim() -> Style {
    Style::default().fg(DIM)
}

pub fn muted() -> Style {
    Style::default().fg(MUTED)
}

pub fn text() -> Style {
    Style::default().fg(TEXT)
}

pub fn success() -> Style {
    Style::default().fg(SUCCESS)
}

pub fn warn() -> Style {
    Style::default().fg(WARN)
}

pub fn error() -> Style {
    Style::default().fg(ERROR)
}

/// Border style that subtly indicates focus.
pub fn border(focused: bool) -> Style {
    if focused {
        Style::default().fg(DIM)
    } else {
        Style::default().fg(Color::Rgb(50, 55, 78))
    }
}

/// Foreground colour for a highlighted token.
pub fn token_style(kind: TokenKind) -> Style {
    let color = match kind {
        TokenKind::Plain => TEXT,
        TokenKind::Keyword => Color::Rgb(187, 154, 247),
        TokenKind::Type => Color::Rgb(42, 195, 222),
        TokenKind::Function => Color::Rgb(122, 162, 247),
        TokenKind::String => Color::Rgb(158, 206, 106),
        TokenKind::Number => Color::Rgb(255, 158, 100),
        TokenKind::Comment => Color::Rgb(86, 95, 137),
        TokenKind::Macro => Color::Rgb(125, 207, 255),
        TokenKind::Constant => Color::Rgb(255, 199, 119),
        TokenKind::Operator => Color::Rgb(137, 221, 255),
        TokenKind::Attribute => Color::Rgb(224, 175, 104),
    };
    Style::default().fg(color)
}
