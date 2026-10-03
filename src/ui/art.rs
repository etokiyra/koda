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

/// The Koda familiar, drowsy.
pub const CAT_SLEEPY: &[&str] = &["/\\___/\\", "( ˘ω˘ )", " > ω < ", " ~~~~~ "];

/// The Koda familiar, wide-eyed and curious.
pub const CAT_CURIOUS: &[&str] = &["/\\___/\\", "( ⊙ω⊙ )", " > ω < ", "/|   |\\"];

/// The Koda familiar, concentrating on the code.
pub const CAT_FOCUS: &[&str] = &["/\\___/\\", "( ◉ω◉ )", " > ω < ", "/|   |\\"];

/// The Koda familiar, giving a wink.
pub const CAT_WINK: &[&str] = &["/\\___/\\", "( ^ω- )", " > ω < ", "/|   |\\"];

/// The Koda familiar, purring.
pub const CAT_PURR: &[&str] = &["/\\___/\\", "( ≧ω≦ )", " > ω < ", "/|   |\\"];

/// Every pose, for the animation cycle.
pub const POSES: &[&[&str]] = &[
    CAT,
    CAT_BLINK,
    CAT_HAPPY,
    CAT_AWAKE,
    CAT_CURIOUS,
    CAT_WINK,
    CAT_PURR,
];

/// A small drifting star-spirit that keeps the familiar company.
pub const STAR_SPIRIT: &str = "✧";

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

/// A single-line empty state: the familiar alongside a short message.
pub fn familiar_line(message: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("( -ω- )  ", theme::soft()),
        Span::styled(message.to_string(), theme::muted()),
    ])
}

/// A one-line loading state with the familiar and a busy sparkle.
pub fn busy_line(phase: usize, message: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{} ", spinner(phase)), theme::star()),
        Span::styled("( ･ω･ )  ", theme::soft()),
        Span::styled(message.to_string(), theme::muted()),
    ])
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

// ---------------------------------------------------------------------------
// Welcome scenes
//
// Each scene is drawn onto a small character canvas — placed by coordinate
// rather than hand-aligned — so the compositions stay symmetric and can animate
// individual elements without the rest of the art drifting.
// ---------------------------------------------------------------------------

/// A rotating collection of atmospheric welcome scenes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WelcomeScene {
    /// A moonlit hillside with the familiar star-gazing.
    #[default]
    Starry,
    /// A cozy desk with a hanging lamp and a little terminal.
    Cozy,
    /// A rain-streaked window and a drowsy familiar.
    Rainy,
    /// Drifting blossom petals around a happy familiar.
    Sakura,
}

impl WelcomeScene {
    pub const ALL: [WelcomeScene; 4] = [
        WelcomeScene::Starry,
        WelcomeScene::Cozy,
        WelcomeScene::Rainy,
        WelcomeScene::Sakura,
    ];

    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|scene| *scene == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            WelcomeScene::Starry => "starry night",
            WelcomeScene::Cozy => "cozy desk",
            WelcomeScene::Rainy => "rainy window",
            WelcomeScene::Sakura => "sakura drift",
        }
    }
}

const SCENE_WIDTH: usize = 46;
const SCENE_HEIGHT: usize = 8;

/// A small character grid used to compose scenes by coordinate.
struct Canvas {
    cells: Vec<Vec<(char, Style)>>,
    width: usize,
    height: usize,
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Canvas {
            cells: vec![vec![(' ', Style::default()); width]; height],
            width,
            height,
        }
    }

    /// Place `text` starting at `(x, y)`, clipping at the canvas edge.
    fn put(&mut self, x: usize, y: usize, text: &str, style: Style) {
        if y >= self.height {
            return;
        }
        for (offset, ch) in text.chars().enumerate() {
            let column = x + offset;
            if column >= self.width {
                break;
            }
            self.cells[y][column] = (ch, style);
        }
    }

    /// Turn the canvas into equal-width lines (trailing blanks included, so
    /// per-line centering keeps every row aligned).
    fn lines(&self) -> Vec<Line<'static>> {
        self.cells
            .iter()
            .map(|row| {
                let mut spans: Vec<Span> = Vec::new();
                let mut run = String::new();
                let mut run_style: Option<Style> = None;
                for (ch, style) in row {
                    if run_style == Some(*style) {
                        run.push(*ch);
                    } else {
                        if let Some(previous) = run_style.take() {
                            spans.push(Span::styled(std::mem::take(&mut run), previous));
                        }
                        run.push(*ch);
                        run_style = Some(*style);
                    }
                }
                if let Some(style) = run_style {
                    spans.push(Span::styled(run, style));
                }
                Line::from(spans)
            })
            .collect()
    }
}

/// Draw one of the familiar's poses onto the canvas.
fn place_cat(canvas: &mut Canvas, x: usize, y: usize, phase: usize) {
    let pose = POSES[(phase / 5) % POSES.len()];
    for (row, line) in pose.iter().enumerate() {
        canvas.put(x, y + row, line, theme::soft());
    }
}

fn starry(phase: usize) -> Canvas {
    let mut canvas = Canvas::new(SCENE_WIDTH, SCENE_HEIGHT);
    canvas.put(40, 0, MOON, theme::accent());

    const STARS: &[(usize, usize)] = &[
        (3, 0),
        (12, 1),
        (22, 0),
        (30, 2),
        (37, 1),
        (44, 0),
        (7, 2),
        (27, 1),
        (34, 0),
        (42, 3),
        (1, 3),
        (17, 3),
    ];
    for (index, (x, y)) in STARS.iter().enumerate() {
        let (glyph, style) = match (phase + index) % 3 {
            0 => (STAR, theme::star()),
            1 => (SPARK, theme::soft()),
            _ => (DOT, theme::dim()),
        };
        canvas.put(*x, *y, glyph, style);
    }

    // A shooting star crosses the sky now and then.
    let streak = phase % 24;
    if streak < 4 {
        canvas.put(2 + streak * 6, 1, "·", theme::star());
        if streak >= 1 {
            canvas.put(3 + (streak - 1) * 6, 2, "·", theme::star());
        }
    }

    canvas.put(6, 7, "·  ·  ·  ~~~~~~~~~~~~~~  ·  ·", theme::dim());
    place_cat(&mut canvas, 19, 3, phase);
    canvas
}

fn cozy(phase: usize) -> Canvas {
    let mut canvas = Canvas::new(SCENE_WIDTH, SCENE_HEIGHT);
    canvas.put(41, 0, MOON, theme::accent());
    // A hanging lamp that blinks.
    let lamp = if phase.is_multiple_of(4) { "✦" } else { "·" };
    canvas.put(29, 0, "╭─╮", theme::dim());
    canvas.put(29, 1, &format!("│{lamp}│"), theme::dim());
    canvas.put(29, 2, "╰─╯", theme::dim());

    place_cat(&mut canvas, 16, 1, phase);
    canvas.put(23, 2, SPARK, theme::star());

    // A little terminal on the desk.
    canvas.put(6, 5, "╭──────────────────────────╮", theme::border(false));
    canvas.put(6, 6, "│", theme::border(false));
    canvas.put(8, 6, ">_", theme::star());
    canvas.put(10, 6, " koda", theme::accent_bold());
    canvas.put(32, 6, "│", theme::border(false));
    canvas.put(6, 7, "╰──────────────────────────╯", theme::border(false));
    canvas
}

fn rainy(phase: usize) -> Canvas {
    let mut canvas = Canvas::new(SCENE_WIDTH, SCENE_HEIGHT);
    canvas.put(
        4,
        0,
        "╭──────────────────────────────────╮",
        theme::border(false),
    );
    for y in 1..4 {
        canvas.put(4, y, "│", theme::border(false));
        canvas.put(39, y, "│", theme::border(false));
    }
    canvas.put(
        4,
        4,
        "╰──────────────────────────────────╯",
        theme::border(false),
    );

    canvas.put(20, 2, MOON, theme::accent());
    canvas.put(12, 1, SPARK, theme::soft());
    canvas.put(30, 1, STAR, theme::star());

    // Rain falling inside the window; streaks shift with the phase.
    const RAIN_X: &[usize] = &[8, 14, 22, 27, 33, 36];
    for (index, x) in RAIN_X.iter().enumerate() {
        let y = (phase + index * 2) % 3 + 1;
        canvas.put(*x, y, "╵", theme::soft());
        if y > 1 {
            canvas.put(*x, y - 1, "╷", theme::dim());
        }
    }

    place_cat(&mut canvas, 24, 5, phase);
    canvas.put(31, 6, "z", theme::dim());
    canvas.put(33, 6, "Z", theme::dim());
    canvas
}

fn sakura(phase: usize) -> Canvas {
    let mut canvas = Canvas::new(SCENE_WIDTH, SCENE_HEIGHT);

    // Drifting petals fall gently, wrapping around the scene.
    const PETALS: &[(usize, usize, bool)] = &[
        (3, 0, true),
        (12, 1, false),
        (24, 0, true),
        (33, 2, false),
        (41, 1, true),
        (7, 3, false),
        (28, 3, false),
        (38, 4, true),
        (10, 6, true),
        (35, 6, true),
        (18, 6, false),
    ];
    for (index, (x, y, big)) in PETALS.iter().enumerate() {
        let drift = (phase / 2 + index) % 4;
        let yy = (y + drift) % SCENE_HEIGHT;
        let (glyph, style) = if *big {
            ("✿", theme::accent())
        } else {
            ("❀", theme::soft())
        };
        canvas.put(*x, yy, glyph, style);
    }

    place_cat(&mut canvas, 19, 3, phase);
    canvas
}

/// Render a welcome scene, frozen at its first frame when motion is off.
pub fn scene(kind: WelcomeScene, phase: usize, motion: bool) -> Vec<Line<'static>> {
    let phase = if motion { phase } else { 0 };
    let canvas = match kind {
        WelcomeScene::Starry => starry(phase),
        WelcomeScene::Cozy => cozy(phase),
        WelcomeScene::Rainy => rainy(phase),
        WelcomeScene::Sakura => sakura(phase),
    };
    canvas.lines()
}

/// A section rule, optionally labelled, used across panels and headers.
pub fn divider(width: usize, label: Option<&str>) -> Line<'static> {
    match label {
        Some(label) => {
            let used = label.chars().count() + 4;
            let rule = "─".repeat(width.saturating_sub(used).max(1));
            Line::from(vec![
                Span::styled(format!(" {label} "), theme::accent()),
                Span::styled(rule, theme::dim()),
            ])
        }
        None => Line::from(Span::styled("─".repeat(width), theme::dim())),
    }
}

/// A centered empty state: a small familiar, a title and a hint.
pub fn empty_state(pose: &[&str], title: &str, hint: &str) -> Vec<Line<'static>> {
    let mut lines = art_lines(pose, theme::dim());
    lines.push(Line::from(""));
    if !title.is_empty() {
        lines.push(Line::from(Span::styled(title.to_string(), theme::muted())).centered());
    }
    if !hint.is_empty() {
        lines.push(Line::from(Span::styled(hint.to_string(), theme::dim())).centered());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_are_equal_width_and_bounded() {
        for kind in WelcomeScene::ALL {
            let lines = scene(kind, 5, true);
            assert_eq!(lines.len(), SCENE_HEIGHT, "{:?} height", kind);
            for line in &lines {
                assert_eq!(line.width(), SCENE_WIDTH, "{:?} width", kind);
            }
        }
    }

    #[test]
    fn scenes_freeze_when_motion_is_off() {
        for kind in WelcomeScene::ALL {
            let still = scene(kind, 0, false);
            let later = scene(kind, 7, false);
            assert_eq!(still, later, "{:?} should not animate", kind);
        }
    }

    #[test]
    fn scenes_cycle_and_label() {
        let mut scene = WelcomeScene::Starry;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..WelcomeScene::ALL.len() {
            assert!(seen.insert(scene));
            assert!(!scene.label().is_empty());
            scene = scene.next();
        }
        assert_eq!(scene, WelcomeScene::Starry, "cycle wraps around");
    }

    #[test]
    fn empty_state_centers_the_title() {
        let lines = empty_state(CAT_ASLEEP, "nothing here", "press / to search");
        assert!(lines.iter().any(|line| line.width() > 0));
        assert!(lines.len() >= 6);
    }
}
