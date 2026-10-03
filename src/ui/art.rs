//! Original ASCII art for Koda.
//!
//! Art is deliberate, not decorative spam. Pieces are small, symmetric and
//! reused consistently so they become part of Koda's visual vocabulary:
//!
//! * the four-pointed star `✦` — Koda's mark
//! * a crescent moon `☾`
//! * the Koda familiar, a little star-cat
//!
//! Art is used where there is room for personality — welcome screens and empty
//! states — never behind someone's code.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::language::provider::TokenKind;
use crate::ui::theme;

/// Koda's mark.
pub const STAR: &str = "✦";
/// A softer sparkle.
pub const SPARK: &str = "✧";
/// The mascot's moon.
pub const MOON: &str = "☾";
/// A quiet separator / bullet.
pub const DOT: &str = "·";
/// Selection pointer.
pub const POINTER: &str = "❯";
/// Selection bar.
pub const BAR: &str = "▏";

/// The base Koda familiar, sitting and content. Every line is 7 columns wide.
pub const CAT: &[&str] = &["/\\___/\\", "( ･ω･ )", " > ω < ", "/|   |\\"];

/// The Koda familiar, fast asleep.
pub const CAT_ASLEEP: &[&str] = &["/\\___/\\", "( -ω- )", " > ω < ", " ~~~~~ "];

/// The Koda familiar, a little surprised.
pub const CAT_AWAKE: &[&str] = &["/\\___/\\", "( o.o )", " > ω < ", "/|   |\\"];

/// The Koda familiar, mid-blink.
pub const CAT_BLINK: &[&str] = &["/\\___/\\", "( -ω- )", " > ω < ", "/|   |\\"];

/// The Koda familiar, celebrating.
pub const CAT_HAPPY: &[&str] = &["/\\___/\\", "( ^ω^ )", "\\|   |/"];

/// Frames for the busy sparkle, cycled while background work runs.
pub const SPINNER: &[&str] = &["✶", "✸", "✹", "✷"];

/// The busy sparkle for a given animation phase.
pub fn spinner(phase: usize) -> &'static str {
    SPINNER[phase % SPINNER.len()]
}

/// Pad lines to equal width so per-line centering keeps the art aligned.
pub fn equalize(lines: &[&str]) -> Vec<String> {
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    lines
        .iter()
        .map(|l| {
            let pad = width - l.chars().count();
            format!("{l}{}", " ".repeat(pad))
        })
        .collect()
}

/// Turn a set of raw art lines into centered, uniformly styled lines.
pub fn art_lines(raw: &[&str], style: Style) -> Vec<Line<'static>> {
    equalize(raw)
        .into_iter()
        .map(|line| Line::from(Span::styled(line, style)).centered())
        .collect()
}

/// Wrap content in a rounded frame, padding every row to `inner_width`.
pub fn frame(
    content: Vec<Line<'static>>,
    inner_width: usize,
    border: Style,
    corner: Style,
) -> Vec<Line<'static>> {
    let mut out = Vec::with_capacity(content.len() + 2);
    out.push(Line::from(vec![
        Span::styled("╭", corner),
        Span::styled("─".repeat(inner_width), border),
        Span::styled("╮", corner),
    ]));
    for line in content {
        let width = line.width();
        let pad = inner_width.saturating_sub(width);
        let mut spans = vec![Span::styled("│", border)];
        spans.extend(line.spans);
        if pad > 0 {
            spans.push(Span::raw(" ".repeat(pad)));
        }
        spans.push(Span::styled("│", border));
        out.push(Line::from(spans));
    }
    out.push(Line::from(vec![
        Span::styled("╰", corner),
        Span::styled("─".repeat(inner_width), border),
        Span::styled("╯", corner),
    ]));
    out
}

/// A tiny code window for the welcome scene, already equal width.
pub fn code_window() -> Vec<Line<'static>> {
    let keyword = theme::token_style(TokenKind::Keyword);
    let function = theme::token_style(TokenKind::Function);
    let plain = theme::token_style(TokenKind::Plain);

    let content = vec![
        Line::from(vec![
            Span::styled(">_ ", theme::star()),
            Span::styled("koda", theme::accent_bold()),
        ]),
        Line::from(vec![
            Span::styled("fn", keyword),
            Span::styled(" ", plain),
            Span::styled("main", function),
            Span::styled("() {", plain),
        ]),
        Line::from(vec![
            Span::styled("    meow", function),
            Span::styled("();", plain),
            Span::styled("   ", plain),
            Span::styled(STAR, theme::star()),
        ]),
        Line::from(Span::styled("}", plain)),
    ];
    frame(content, 24, theme::border(false), theme::accent())
}

/// A single line of scattered stars for the top of the welcome scene.
pub fn star_scatter() -> Line<'static> {
    Line::from(vec![
        Span::styled("  ✦", theme::star()),
        Span::styled("      ·      ", theme::dim()),
        Span::styled("☾", theme::accent()),
        Span::styled("      ·      ", theme::dim()),
        Span::styled("✦  ", theme::star()),
    ])
    .centered()
}

/// The `K O D A` wordmark.
pub fn wordmark() -> Line<'static> {
    Line::from(vec![
        Span::styled("✦  ", theme::star()),
        Span::styled("K", theme::accent_bold()),
        Span::raw(" "),
        Span::styled("O", theme::accent_bold()),
        Span::raw(" "),
        Span::styled("D", theme::accent_bold()),
        Span::raw(" "),
        Span::styled("A", theme::accent_bold()),
        Span::styled("  ✦", theme::star()),
    ])
    .centered()
}
