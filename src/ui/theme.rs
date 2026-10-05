//! Koda's visual identity: bundled themes and the semantic roles built on them.
//!
//! **Mellow** is the default, and its palette is the separately named colorscheme
//! shipped with Helix — *not* Helix's default theme. Koda reads Mellow's palette
//! and semantic mappings and translates them into its own UI roles; the layout,
//! components, ASCII art and overall design remain Koda's own. Two further
//! bundled themes (a high-contrast dark and a light theme) share the same roles.
//!
//! Transparency is a first-class concern: we never paint a full-screen
//! background. Plain surfaces use [`Color::Reset`] so the terminal's own
//! background (and any blur/wallpaper) shows through. Only small, intentional
//! surfaces — the statusline, popups, the current line and selections — use a
//! near-background panel colour.
//!
//! The active theme is a small thread-local so every widget can keep calling the
//! role accessors without threading a `Theme` through the whole render tree.
//! Rendering happens on one thread; tests are isolated per thread.

use std::cell::Cell;

use ratatui::style::{Color, Modifier, Style};

use crate::language::provider::TokenKind;
use crate::settings::ThemeId;

/// The raw Mellow palette, named exactly as the upstream theme names it.
///
/// Values are copied verbatim from
/// `runtime/themes/mellow.toml` (`[palette]`) in the Helix repository.
mod mellow {
    use ratatui::style::Color;

    pub const FG: Color = Color::Rgb(0xc9, 0xc7, 0xcd);
    pub const BG_DARK: Color = Color::Rgb(0x13, 0x13, 0x14);

    pub const BRIGHT_RED: Color = Color::Rgb(0xff, 0xae, 0x9f);

    pub const GREEN: Color = Color::Rgb(0x90, 0xb9, 0x9f);
    pub const BRIGHT_GREEN: Color = Color::Rgb(0x9d, 0xc6, 0xac);

    pub const YELLOW: Color = Color::Rgb(0xe6, 0xb9, 0x9d);
    pub const BRIGHT_YELLOW: Color = Color::Rgb(0xf0, 0xc5, 0xa9);

    pub const BLUE: Color = Color::Rgb(0xac, 0xa1, 0xcf);
    pub const BRIGHT_BLUE: Color = Color::Rgb(0xb9, 0xae, 0xda);

    pub const MAGENTA: Color = Color::Rgb(0xe2, 0x9e, 0xca);

    pub const CYAN: Color = Color::Rgb(0xea, 0x83, 0xa5);
    pub const BRIGHT_CYAN: Color = Color::Rgb(0xf5, 0x91, 0xb2);

    pub const WHITE: Color = Color::Rgb(0xc1, 0xc0, 0xd4);
    pub const BRIGHT_WHITE: Color = Color::Rgb(0xca, 0xc9, 0xdd);

    pub const GRAY01: Color = Color::Rgb(0x1b, 0x1b, 0x1d);
    pub const GRAY02: Color = Color::Rgb(0x2a, 0x2a, 0x2d);
    pub const GRAY03: Color = Color::Rgb(0x3e, 0x3e, 0x43);
    pub const GRAY04: Color = Color::Rgb(0x57, 0x57, 0x5f);
    pub const GRAY05: Color = Color::Rgb(0x75, 0x75, 0x81);
}

/// The syntax token colours a theme provides.
#[derive(Clone, Copy)]
pub struct Syntax {
    pub plain: Color,
    pub keyword: Color,
    pub type_: Color,
    pub function: Color,
    pub string: Color,
    pub number: Color,
    pub comment: Color,
    pub macro_: Color,
    pub constant: Color,
    pub operator: Color,
    pub attribute: Color,
}

/// Every semantic colour a theme provides. Widgets use the role accessors, never
/// the fields directly, so a theme swap reaches every surface.
#[derive(Clone, Copy)]
pub struct Theme {
    pub bg_dark: Color,
    pub accent: Color,
    pub accent_soft: Color,
    pub star: Color,
    pub text: Color,
    pub text_bright: Color,
    pub muted: Color,
    pub faint: Color,
    pub success: Color,
    pub warn: Color,
    pub error: Color,
    pub info: Color,
    pub hint: Color,
    pub selection_bg: Color,
    pub multi_cursor: Color,
    pub menu_selected_bg: Color,
    pub sidebar_selected_bg: Color,
    pub cursorline_bg: Color,
    pub panel_bg: Color,
    pub border: Color,
    pub border_focus: Color,
    pub search_bg: Color,
    pub search_current_bg: Color,
    pub bracket_match: Color,
    pub syntax: Syntax,
}

/// Mellow — Koda's default. The semantics are the existing Mellow mappings.
const MELLOW: Theme = Theme {
    bg_dark: mellow::BG_DARK,
    accent: mellow::BLUE,
    accent_soft: mellow::BRIGHT_BLUE,
    star: mellow::BRIGHT_YELLOW,
    text: mellow::FG,
    text_bright: mellow::BRIGHT_WHITE,
    muted: mellow::GRAY05,
    faint: mellow::GRAY04,
    success: mellow::BRIGHT_GREEN,
    warn: mellow::BRIGHT_YELLOW,
    error: mellow::BRIGHT_RED,
    info: mellow::BRIGHT_BLUE,
    hint: mellow::BRIGHT_CYAN,
    selection_bg: mellow::GRAY03,
    multi_cursor: mellow::BLUE,
    menu_selected_bg: mellow::GRAY03,
    sidebar_selected_bg: mellow::GRAY03,
    cursorline_bg: mellow::GRAY01,
    panel_bg: mellow::GRAY01,
    border: mellow::GRAY03,
    border_focus: mellow::BLUE,
    search_bg: mellow::GRAY02,
    search_current_bg: mellow::GRAY03,
    bracket_match: mellow::YELLOW,
    syntax: Syntax {
        plain: mellow::FG,
        keyword: mellow::BLUE,
        type_: mellow::BRIGHT_BLUE,
        function: mellow::WHITE,
        string: mellow::GREEN,
        number: mellow::MAGENTA,
        comment: mellow::GRAY05,
        macro_: mellow::BRIGHT_CYAN,
        constant: mellow::CYAN,
        operator: mellow::YELLOW,
        attribute: mellow::BLUE,
    },
};

/// Midnight — a high-contrast cool dark theme.
const MIDNIGHT: Theme = Theme {
    bg_dark: Color::Rgb(0x0a, 0x0c, 0x12),
    accent: Color::Rgb(0x6e, 0xa8, 0xfe),
    accent_soft: Color::Rgb(0x9d, 0xc1, 0xff),
    star: Color::Rgb(0xf0, 0xc6, 0x74),
    text: Color::Rgb(0xd7, 0xdc, 0xe6),
    text_bright: Color::Rgb(0xff, 0xff, 0xff),
    muted: Color::Rgb(0x8b, 0x93, 0xa7),
    faint: Color::Rgb(0x5c, 0x64, 0x78),
    success: Color::Rgb(0x9e, 0xce, 0x6a),
    warn: Color::Rgb(0xe0, 0xaf, 0x68),
    error: Color::Rgb(0xf7, 0x76, 0x8e),
    info: Color::Rgb(0x7d, 0xcf, 0xff),
    hint: Color::Rgb(0xbb, 0x9a, 0xf7),
    selection_bg: Color::Rgb(0x28, 0x34, 0x4a),
    multi_cursor: Color::Rgb(0x6e, 0xa8, 0xfe),
    menu_selected_bg: Color::Rgb(0x28, 0x34, 0x4a),
    sidebar_selected_bg: Color::Rgb(0x28, 0x34, 0x4a),
    cursorline_bg: Color::Rgb(0x16, 0x1a, 0x22),
    panel_bg: Color::Rgb(0x12, 0x15, 0x1c),
    border: Color::Rgb(0x2a, 0x30, 0x40),
    border_focus: Color::Rgb(0x6e, 0xa8, 0xfe),
    search_bg: Color::Rgb(0x2f, 0x3a, 0x4f),
    search_current_bg: Color::Rgb(0x3d, 0x4a, 0x63),
    bracket_match: Color::Rgb(0xe0, 0xaf, 0x68),
    syntax: Syntax {
        plain: Color::Rgb(0xd7, 0xdc, 0xe6),
        keyword: Color::Rgb(0x6e, 0xa8, 0xfe),
        type_: Color::Rgb(0x9d, 0xc1, 0xff),
        function: Color::Rgb(0xe6, 0xeb, 0xf5),
        string: Color::Rgb(0x9e, 0xce, 0x6a),
        number: Color::Rgb(0xbb, 0x9a, 0xf7),
        comment: Color::Rgb(0x56, 0x5f, 0x89),
        macro_: Color::Rgb(0x7d, 0xcf, 0xff),
        constant: Color::Rgb(0x2a, 0xc3, 0xde),
        operator: Color::Rgb(0xe0, 0xaf, 0x68),
        attribute: Color::Rgb(0x6e, 0xa8, 0xfe),
    },
};

/// Daylight — a light theme for light terminal backgrounds.
const DAYLIGHT: Theme = Theme {
    bg_dark: Color::Rgb(0xe9, 0xe7, 0xe0),
    accent: Color::Rgb(0x3b, 0x5b, 0xdb),
    accent_soft: Color::Rgb(0x42, 0x63, 0xeb),
    star: Color::Rgb(0x9a, 0x6b, 0x00),
    text: Color::Rgb(0x2b, 0x2b, 0x33),
    text_bright: Color::Rgb(0x11, 0x11, 0x18),
    muted: Color::Rgb(0x5f, 0x5f, 0x6b),
    faint: Color::Rgb(0x7a, 0x7a, 0x86),
    success: Color::Rgb(0x2f, 0x8a, 0x4c),
    warn: Color::Rgb(0x9a, 0x6b, 0x00),
    error: Color::Rgb(0xc0, 0x39, 0x2b),
    info: Color::Rgb(0x1c, 0x6f, 0xb8),
    hint: Color::Rgb(0x7b, 0x4b, 0xb7),
    selection_bg: Color::Rgb(0xd6, 0xe0, 0xff),
    multi_cursor: Color::Rgb(0x3b, 0x5b, 0xdb),
    menu_selected_bg: Color::Rgb(0xdd, 0xe4, 0xf5),
    sidebar_selected_bg: Color::Rgb(0xdd, 0xe4, 0xf5),
    cursorline_bg: Color::Rgb(0xf0, 0xef, 0xe9),
    panel_bg: Color::Rgb(0xf1, 0xf0, 0xea),
    border: Color::Rgb(0xc9, 0xc8, 0xc0),
    border_focus: Color::Rgb(0x3b, 0x5b, 0xdb),
    search_bg: Color::Rgb(0xf5, 0xe6, 0xa8),
    search_current_bg: Color::Rgb(0xf0, 0xd9, 0x8a),
    bracket_match: Color::Rgb(0x9a, 0x6b, 0x00),
    syntax: Syntax {
        plain: Color::Rgb(0x2b, 0x2b, 0x33),
        keyword: Color::Rgb(0x3b, 0x5b, 0xdb),
        type_: Color::Rgb(0x1c, 0x4f, 0xd8),
        function: Color::Rgb(0x2b, 0x2b, 0x33),
        string: Color::Rgb(0x2f, 0x8a, 0x4c),
        number: Color::Rgb(0xb0, 0x2a, 0x8a),
        comment: Color::Rgb(0x7a, 0x7a, 0x86),
        macro_: Color::Rgb(0x9a, 0x6b, 0x00),
        constant: Color::Rgb(0x0b, 0x72, 0x85),
        operator: Color::Rgb(0x9a, 0x6b, 0x00),
        attribute: Color::Rgb(0x3b, 0x5b, 0xdb),
    },
};

const THEMES: [Theme; 3] = [MELLOW, MIDNIGHT, DAYLIGHT];

thread_local! {
    static CURRENT: Cell<ThemeId> = const { Cell::new(ThemeId::Mellow) };
}

/// The theme definition for a given id.
pub fn theme_for(id: ThemeId) -> &'static Theme {
    match id {
        ThemeId::Mellow => &THEMES[0],
        ThemeId::Midnight => &THEMES[1],
        ThemeId::Daylight => &THEMES[2],
    }
}

/// The active theme's definition.
pub fn current() -> &'static Theme {
    theme_for(theme_id())
}

/// The active theme.
pub fn theme_id() -> ThemeId {
    CURRENT.with(Cell::get)
}

/// Select the active theme. Rendering picks it up on the next frame.
pub fn set_theme(id: ThemeId) {
    CURRENT.with(|current| current.set(id));
}

// ---------------------------------------------------------------------------
// Role accessors
// ---------------------------------------------------------------------------

pub fn bg_dark() -> Color {
    current().bg_dark
}
pub fn accent_color() -> Color {
    current().accent
}
pub fn accent_soft() -> Color {
    current().accent_soft
}
pub fn star_color() -> Color {
    current().star
}
pub fn text_color() -> Color {
    current().text
}
pub fn text_bright() -> Color {
    current().text_bright
}
pub fn muted_color() -> Color {
    current().muted
}
pub fn faint() -> Color {
    current().faint
}
pub fn success_color() -> Color {
    current().success
}
pub fn warn_color() -> Color {
    current().warn
}
pub fn error_color() -> Color {
    current().error
}
pub fn info_color() -> Color {
    current().info
}
pub fn hint_color() -> Color {
    current().hint
}
pub fn selection_bg() -> Color {
    current().selection_bg
}
pub fn multi_cursor() -> Color {
    current().multi_cursor
}
pub fn menu_selected_bg() -> Color {
    current().menu_selected_bg
}
pub fn sidebar_selected_bg() -> Color {
    current().sidebar_selected_bg
}
pub fn cursorline_bg() -> Color {
    current().cursorline_bg
}
pub fn panel_bg() -> Color {
    current().panel_bg
}
pub fn border_color() -> Color {
    current().border
}
pub fn border_focus() -> Color {
    current().border_focus
}
pub fn search_bg() -> Color {
    current().search_bg
}
pub fn search_current_bg() -> Color {
    current().search_current_bg
}
pub fn bracket_match_color() -> Color {
    current().bracket_match
}

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

pub fn accent() -> Style {
    Style::default().fg(current().accent)
}

pub fn accent_bold() -> Style {
    Style::default()
        .fg(current().accent)
        .add_modifier(Modifier::BOLD)
}

pub fn soft() -> Style {
    Style::default().fg(current().accent_soft)
}

pub fn star() -> Style {
    Style::default().fg(current().star)
}

pub fn text() -> Style {
    Style::default().fg(current().text)
}

pub fn bright() -> Style {
    Style::default().fg(current().text_bright)
}

pub fn bright_bold() -> Style {
    Style::default()
        .fg(current().text_bright)
        .add_modifier(Modifier::BOLD)
}

pub fn muted() -> Style {
    Style::default().fg(current().muted)
}

pub fn dim() -> Style {
    Style::default().fg(current().faint)
}

pub fn success() -> Style {
    Style::default().fg(current().success)
}

pub fn warn() -> Style {
    Style::default().fg(current().warn)
}

pub fn error() -> Style {
    Style::default().fg(current().error)
}

pub fn info() -> Style {
    Style::default().fg(current().info)
}

pub fn hint() -> Style {
    Style::default().fg(current().hint)
}

/// A solid panel surface with optional foreground.
pub fn panel(styled_fg: Style) -> Style {
    styled_fg.bg(current().panel_bg)
}

/// Border style that subtly indicates focus.
pub fn border(focused: bool) -> Style {
    let color = if focused {
        current().border_focus
    } else {
        current().border
    };
    Style::default().fg(color)
}

/// A pill: solid background with a contrasting foreground.
pub fn pill(bg: Color, fg: Color) -> Style {
    Style::default().bg(bg).fg(fg).add_modifier(Modifier::BOLD)
}

/// The matching bracket is yellow in Mellow: bold and underlined.
pub fn bracket_match() -> Style {
    Style::default()
        .fg(current().bracket_match)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

/// Foreground style for a highlighted token.
pub fn token_style(kind: TokenKind) -> Style {
    let syntax = current().syntax;
    match kind {
        TokenKind::Plain => Style::default().fg(syntax.plain),
        TokenKind::Keyword => Style::default().fg(syntax.keyword),
        TokenKind::Type => Style::default().fg(syntax.type_),
        TokenKind::Function => Style::default().fg(syntax.function),
        TokenKind::String => Style::default().fg(syntax.string),
        TokenKind::Number => Style::default().fg(syntax.number),
        TokenKind::Comment => Style::default()
            .fg(syntax.comment)
            .add_modifier(Modifier::ITALIC),
        TokenKind::Macro => Style::default().fg(syntax.macro_),
        TokenKind::Constant => Style::default().fg(syntax.constant),
        TokenKind::Operator => Style::default().fg(syntax.operator),
        TokenKind::Attribute => Style::default()
            .fg(syntax.attribute)
            .add_modifier(Modifier::ITALIC),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every role a theme must define; a `Reset` here means one was missed.
    fn roles(theme: &Theme) -> Vec<Color> {
        vec![
            theme.bg_dark,
            theme.accent,
            theme.accent_soft,
            theme.star,
            theme.text,
            theme.text_bright,
            theme.muted,
            theme.faint,
            theme.success,
            theme.warn,
            theme.error,
            theme.info,
            theme.hint,
            theme.selection_bg,
            theme.multi_cursor,
            theme.menu_selected_bg,
            theme.sidebar_selected_bg,
            theme.cursorline_bg,
            theme.panel_bg,
            theme.border,
            theme.border_focus,
            theme.search_bg,
            theme.search_current_bg,
            theme.bracket_match,
        ]
    }

    #[test]
    fn every_bundled_theme_is_registered_and_complete() {
        assert_eq!(ThemeId::ALL.len(), 3);
        for id in ThemeId::ALL {
            let theme = theme_for(id);
            for color in roles(theme) {
                assert_ne!(color, Color::Reset, "{} has an unset role", id.name());
            }
            let syntax = theme.syntax;
            for color in [
                syntax.plain,
                syntax.keyword,
                syntax.type_,
                syntax.function,
                syntax.string,
                syntax.number,
                syntax.comment,
                syntax.macro_,
                syntax.constant,
                syntax.operator,
                syntax.attribute,
            ] {
                assert_ne!(
                    color,
                    Color::Reset,
                    "{} has an unset syntax colour",
                    id.name()
                );
            }
        }
    }

    #[test]
    fn mellow_is_the_default_and_selection_resolves() {
        set_theme(ThemeId::Mellow);
        assert_eq!(theme_id(), ThemeId::Mellow);
        assert_eq!(current().accent, MELLOW.accent);
        for id in ThemeId::ALL {
            set_theme(id);
            assert_eq!(theme_id(), id);
            assert_eq!(current().accent, theme_for(id).accent);
        }
        set_theme(ThemeId::Mellow);
    }

    #[test]
    fn switching_themes_does_not_panic() {
        for id in ThemeId::ALL {
            set_theme(id);
            let _ = accent();
            let _ = accent_bold();
            let _ = soft();
            let _ = star();
            let _ = text();
            let _ = bright();
            let _ = bright_bold();
            let _ = muted();
            let _ = dim();
            let _ = success();
            let _ = warn();
            let _ = error();
            let _ = info();
            let _ = hint();
            let _ = panel(text());
            let _ = border(true);
            let _ = border(false);
            let _ = pill(panel_bg(), text_bright());
            let _ = bracket_match();
            for kind in [
                TokenKind::Plain,
                TokenKind::Keyword,
                TokenKind::Type,
                TokenKind::Function,
                TokenKind::String,
                TokenKind::Number,
                TokenKind::Comment,
                TokenKind::Macro,
                TokenKind::Constant,
                TokenKind::Operator,
                TokenKind::Attribute,
            ] {
                let _ = token_style(kind);
            }
        }
        set_theme(ThemeId::Mellow);
    }
}
