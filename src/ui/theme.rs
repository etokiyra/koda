//! Koda's visual identity.
//!
//! The colour language is **Mellow**, the separately named colorscheme shipped
//! with Helix — *not* Helix's default theme. Koda reads Mellow's palette and
//! semantic mappings and translates them into its own UI roles; the layout,
//! components, ASCII art and overall design remain Koda's own.
//!
//! Upstream reference (source of truth):
//! <https://github.com/helix-editor/helix/blob/master/runtime/themes/mellow.toml>
//!
//! Transparency is a first-class concern: we never paint a full-screen
//! background. Plain surfaces use [`Color::Reset`] so the terminal's own
//! background (and any blur/wallpaper) shows through. Only small, intentional
//! surfaces — the statusline, popups, the current line and selections — use
//! Mellow's near-black panel greys.

use ratatui::style::{Color, Modifier, Style};

use crate::language::provider::TokenKind;

/// The raw Mellow palette, named exactly as the upstream theme names it.
///
/// Values are copied verbatim from
/// `runtime/themes/mellow.toml` (`[palette]`) in the Helix repository.
pub mod palette {
    use ratatui::style::Color;

    pub const BG: Color = Color::Rgb(0x16, 0x16, 0x17);
    pub const FG: Color = Color::Rgb(0xc9, 0xc7, 0xcd);
    pub const BG_DARK: Color = Color::Rgb(0x13, 0x13, 0x14);

    pub const BLACK: Color = Color::Rgb(0x27, 0x27, 0x2a);
    pub const BRIGHT_BLACK: Color = Color::Rgb(0x35, 0x35, 0x39);

    pub const RED: Color = Color::Rgb(0xf5, 0xa1, 0x91);
    pub const BRIGHT_RED: Color = Color::Rgb(0xff, 0xae, 0x9f);

    pub const GREEN: Color = Color::Rgb(0x90, 0xb9, 0x9f);
    pub const BRIGHT_GREEN: Color = Color::Rgb(0x9d, 0xc6, 0xac);

    pub const YELLOW: Color = Color::Rgb(0xe6, 0xb9, 0x9d);
    pub const BRIGHT_YELLOW: Color = Color::Rgb(0xf0, 0xc5, 0xa9);

    pub const BLUE: Color = Color::Rgb(0xac, 0xa1, 0xcf);
    pub const BRIGHT_BLUE: Color = Color::Rgb(0xb9, 0xae, 0xda);

    pub const MAGENTA: Color = Color::Rgb(0xe2, 0x9e, 0xca);
    pub const BRIGHT_MAGENTA: Color = Color::Rgb(0xec, 0xaa, 0xd6);

    pub const CYAN: Color = Color::Rgb(0xea, 0x83, 0xa5);
    pub const BRIGHT_CYAN: Color = Color::Rgb(0xf5, 0x91, 0xb2);

    pub const WHITE: Color = Color::Rgb(0xc1, 0xc0, 0xd4);
    pub const BRIGHT_WHITE: Color = Color::Rgb(0xca, 0xc9, 0xdd);

    pub const GRAY01: Color = Color::Rgb(0x1b, 0x1b, 0x1d);
    pub const GRAY02: Color = Color::Rgb(0x2a, 0x2a, 0x2d);
    pub const GRAY03: Color = Color::Rgb(0x3e, 0x3e, 0x43);
    pub const GRAY04: Color = Color::Rgb(0x57, 0x57, 0x5f);
    pub const GRAY05: Color = Color::Rgb(0x75, 0x75, 0x81);
    pub const GRAY06: Color = Color::Rgb(0x99, 0x98, 0xa8);
    pub const GRAY07: Color = Color::Rgb(0xc1, 0xc0, 0xd4);
}

// ---------------------------------------------------------------------------
// Semantic roles
//
// Each role cites the Mellow scope it is derived from so the mapping stays
// auditable against the upstream theme.
// ---------------------------------------------------------------------------

/// Koda's signature accent — Mellow `keyword` / `blue`.
pub const ACCENT: Color = palette::BLUE;
/// A softer companion accent — Mellow `type` / `bright_blue`.
pub const ACCENT_SOFT: Color = palette::BRIGHT_BLUE;
/// Decorative sparkle — Mellow `bright_yellow`.
pub const STAR: Color = palette::BRIGHT_YELLOW;

/// Primary text — Mellow `fg` (`ui.text`).
pub const TEXT: Color = palette::FG;
/// Emphasised text — Mellow `bright_white` (`ui.menu.selected`).
pub const TEXT_BRIGHT: Color = palette::BRIGHT_WHITE;
/// Secondary text — Mellow `gray05` (its `comment` grey).
pub const MUTED: Color = palette::GRAY05;
/// Faint structural text — Mellow `gray04` (`ui.linenr`).
pub const FAINT: Color = palette::GRAY04;

/// Mellow `diff.plus` / `bright_green`.
pub const SUCCESS: Color = palette::BRIGHT_GREEN;
/// Mellow `warning` / `bright_yellow`.
pub const WARN: Color = palette::BRIGHT_YELLOW;
/// Mellow `error` / `bright_red`.
pub const ERROR: Color = palette::BRIGHT_RED;
/// Mellow `info` / `bright_blue`.
pub const INFO: Color = palette::BRIGHT_BLUE;
/// Mellow `hint` / `bright_cyan`.
pub const HINT: Color = palette::BRIGHT_CYAN;

/// Text selection background — Mellow `ui.selection` (`gray03`).
pub const SELECTION_BG: Color = palette::GRAY03;
/// Selected list rows and the active tab — Mellow `ui.menu.selected` (`gray03`).
pub const MENU_SELECTED_BG: Color = palette::GRAY03;
/// Selected file-tree row — Mellow `ui.selection` (`gray03`).
pub const SIDEBAR_SELECTED_BG: Color = palette::GRAY03;
/// The current line's soft band — Mellow `ui.cursorline.primary` (`gray01`).
pub const CURSORLINE_BG: Color = palette::GRAY01;
/// Popup / statusline / menu surface — Mellow `ui.popup` / `ui.statusline`
/// (`gray01`).
pub const PANEL_BG: Color = palette::GRAY01;
/// Structural border — between Mellow `ui.window` (`gray02`) and `gray03`, kept
/// a touch brighter so the hairline stays legible.
pub const BORDER: Color = palette::GRAY03;
/// Border that indicates focus — the accent.
pub const BORDER_FOCUS: Color = palette::BLUE;

/// Ordinary search matches — Mellow `ui.highlight` (`gray02`).
pub const SEARCH_BG: Color = palette::GRAY02;
/// The current search match — Mellow `ui.selection` (`gray03`).
pub const SEARCH_CURRENT_BG: Color = palette::GRAY03;
/// Matching-bracket foreground — Mellow `ui.cursor.match` (`yellow`).
pub const BRACKET_MATCH: Color = palette::YELLOW;

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

pub fn hint() -> Style {
    Style::default().fg(HINT)
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

/// Mellow `ui.cursor.match`: the matching bracket is yellow, bold, underlined.
pub fn bracket_match() -> Style {
    Style::default()
        .fg(BRACKET_MATCH)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

/// Foreground style for a highlighted token, following Mellow's syntax scopes.
pub fn token_style(kind: TokenKind) -> Style {
    match kind {
        // Mellow `variable` / `punctuation` resolve to the base foreground.
        TokenKind::Plain => Style::default().fg(palette::FG),
        // `keyword` = blue.
        TokenKind::Keyword => Style::default().fg(palette::BLUE),
        // `type` = bright_blue.
        TokenKind::Type => Style::default().fg(palette::BRIGHT_BLUE),
        // `function` = white.
        TokenKind::Function => Style::default().fg(palette::WHITE),
        // `string` = green.
        TokenKind::String => Style::default().fg(palette::GREEN),
        // `constant.numeric` = magenta.
        TokenKind::Number => Style::default().fg(palette::MAGENTA),
        // `comment` = gray05, italic.
        TokenKind::Comment => Style::default()
            .fg(palette::GRAY05)
            .add_modifier(Modifier::ITALIC),
        // `function.macro` = bright_cyan.
        TokenKind::Macro => Style::default().fg(palette::BRIGHT_CYAN),
        // `constant` = cyan.
        TokenKind::Constant => Style::default().fg(palette::CYAN),
        // `operator` = yellow.
        TokenKind::Operator => Style::default().fg(palette::YELLOW),
        // `attribute` = blue, italic.
        TokenKind::Attribute => Style::default()
            .fg(palette::BLUE)
            .add_modifier(Modifier::ITALIC),
    }
}
