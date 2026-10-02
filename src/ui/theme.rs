//! Koda's visual identity.
//!
//! The colour language is **Mellow** (the default Helix theme), used semantically
//! exactly as Helix maps it. Koda keeps its own *design* — its own header, tabs,
//! statusline, file tree and mascot — but speaks Mellow's palette.
//!
//! Transparency is a first-class concern: we never paint a full-screen
//! background. Plain surfaces use [`Color::Reset`] so the terminal's own
//! background (and any blur/wallpaper) shows through. Small, intentional
//! surfaces (statusline, popups) use Mellow's dark purples.

use ratatui::style::{Color, Modifier, Style};

use crate::language::provider::TokenKind;

/// The raw Mellow palette, named exactly as Helix names it.
///
/// Source: Helix `theme.toml` (the Mellow default theme).
pub mod palette {
    use ratatui::style::Color;

    pub const WHITE: Color = Color::Rgb(0xff, 0xff, 0xff);
    pub const LILAC: Color = Color::Rgb(0xdb, 0xbf, 0xef);
    pub const LAVENDER: Color = Color::Rgb(0xa4, 0xa0, 0xe8);
    pub const COMET: Color = Color::Rgb(0x5a, 0x59, 0x77);
    pub const BOSSANOVA: Color = Color::Rgb(0x45, 0x28, 0x59);
    pub const MIDNIGHT: Color = Color::Rgb(0x3b, 0x22, 0x4c);
    pub const REVOLVER: Color = Color::Rgb(0x28, 0x17, 0x33);

    pub const SILVER: Color = Color::Rgb(0xcc, 0xcc, 0xcc);
    pub const SIROCCO: Color = Color::Rgb(0x69, 0x7c, 0x81);
    pub const MINT: Color = Color::Rgb(0x9f, 0xf2, 0x8f);
    pub const ALMOND: Color = Color::Rgb(0xec, 0xcd, 0xba);
    pub const CHAMOIS: Color = Color::Rgb(0xe8, 0xdc, 0xa0);
    pub const HONEY: Color = Color::Rgb(0xef, 0xba, 0x5d);

    pub const APRICOT: Color = Color::Rgb(0xf4, 0x78, 0x68);
    pub const LIGHTNING: Color = Color::Rgb(0xff, 0xcd, 0x1c);
    pub const DELTA: Color = Color::Rgb(0x6f, 0x44, 0xf0);

    /// Mellow's `ui.selection` / `ui.selection.primary`.
    pub const SELECTION: Color = Color::Rgb(0x54, 0x00, 0x99);
    /// Mellow's `ui.cursor.match` background.
    pub const CURSOR_MATCH: Color = Color::Rgb(0x6c, 0x69, 0x99);
}

// ---------------------------------------------------------------------------
// Semantic roles
// ---------------------------------------------------------------------------

/// Koda's signature accent (Mellow `lilac`).
pub const ACCENT: Color = palette::LILAC;
/// A softer companion accent (Mellow `lavender`).
pub const ACCENT_SOFT: Color = palette::LAVENDER;
/// Decorative sparkle (Mellow `lightning`).
pub const STAR: Color = palette::LIGHTNING;

/// Primary reading colour (Mellow `ui.text`).
pub const TEXT: Color = palette::LAVENDER;
/// Emphasised text (Mellow `ui.text.focus`).
pub const TEXT_BRIGHT: Color = palette::WHITE;
/// Secondary text (Mellow `ui.text.inactive`).
pub const MUTED: Color = palette::SIROCCO;
/// Faint structural text: borders, guides, rules.
pub const FAINT: Color = palette::COMET;

pub const SUCCESS: Color = palette::MINT;
pub const WARN: Color = palette::LIGHTNING;
pub const ERROR: Color = palette::APRICOT;
pub const INFO: Color = palette::DELTA;

/// Selected text background (Mellow `ui.selection`).
pub const SELECTION_BG: Color = palette::SELECTION;
/// The current line's soft band (Mellow `ui.cursorline.primary`).
pub const CURSORLINE_BG: Color = palette::BOSSANOVA;
/// Popup / statusline / menu surface (Mellow `ui.popup`).
pub const PANEL_BG: Color = palette::REVOLVER;
/// Structural border for panels.
pub const BORDER: Color = palette::BOSSANOVA;
/// Border that indicates focus.
pub const BORDER_FOCUS: Color = palette::COMET;

pub const SEARCH_BG: Color = palette::MIDNIGHT;
pub const SEARCH_CURRENT_BG: Color = palette::LIGHTNING;
/// Matching-bracket highlight (Mellow `ui.cursor.match`).
pub const BRACKET_BG: Color = palette::CURSOR_MATCH;
pub const SIDEBAR_SELECTED_BG: Color = palette::BOSSANOVA;

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

pub fn accent() -> Style {
    Style::default().fg(ACCENT)
}

pub fn accent_bold() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn soft() -> Style {
    Style::default().fg(ACCENT_SOFT)
}

pub fn star() -> Style {
    Style::default().fg(STAR)
}

pub fn text() -> Style {
    Style::default().fg(TEXT)
}

pub fn bright() -> Style {
    Style::default().fg(TEXT_BRIGHT)
}

pub fn bright_bold() -> Style {
    Style::default()
        .fg(TEXT_BRIGHT)
        .add_modifier(Modifier::BOLD)
}

pub fn muted() -> Style {
    Style::default().fg(MUTED)
}

pub fn dim() -> Style {
    Style::default().fg(FAINT)
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

pub fn info() -> Style {
    Style::default().fg(INFO)
}

/// A solid panel surface with optional foreground.
pub fn panel(styled_fg: Style) -> Style {
    styled_fg.bg(PANEL_BG)
}

/// Border style that subtly indicates focus.
pub fn border(focused: bool) -> Style {
    Style::default().fg(if focused { BORDER_FOCUS } else { BORDER })
}

/// A pill: solid background with a contrasting foreground.
pub fn pill(bg: Color, fg: Color) -> Style {
    Style::default().bg(bg).fg(fg).add_modifier(Modifier::BOLD)
}

/// Foreground style for a highlighted token, following Mellow semantics.
pub fn token_style(kind: TokenKind) -> Style {
    match kind {
        // Mellow `variable` / `punctuation` are lavender.
        TokenKind::Plain => Style::default().fg(palette::LAVENDER),
        TokenKind::Keyword => Style::default().fg(palette::ALMOND),
        TokenKind::Type => Style::default().fg(palette::WHITE),
        TokenKind::Function => Style::default().fg(palette::WHITE),
        TokenKind::String => Style::default().fg(palette::SILVER),
        TokenKind::Number => Style::default().fg(palette::CHAMOIS),
        TokenKind::Comment => Style::default()
            .fg(palette::SIROCCO)
            .add_modifier(Modifier::ITALIC),
        TokenKind::Macro => Style::default().fg(palette::LILAC),
        TokenKind::Constant => Style::default().fg(palette::WHITE),
        TokenKind::Operator => Style::default().fg(palette::LILAC),
        TokenKind::Attribute => Style::default().fg(palette::HONEY),
    }
}
