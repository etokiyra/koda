//! The welcome screen — Koda's little terminal art scene.
//!
//! Shown when no file is open. The composition adapts to the terminal: a full
//! scene when there is room, a compact wordmark when there is not.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::ui::art;
use crate::ui::theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let lines = compose(area, app);
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn compose(area: Rect, app: &App) -> Vec<Line<'static>> {
    let width = area.width as usize;
    let height = area.height as usize;

    let project_empty = app.workspace.tree.is_empty();
    let open_hint = if project_empty {
        "Ctrl+O to open"
    } else {
        "Ctrl+P to open"
    };

    // Tiny terminals: just the mark and one hint.
    if width < 30 || height < 6 {
        return center_vertically(
            vec![
                art::wordmark(),
                Line::from(""),
                Line::from(Span::styled(open_hint, theme::muted())).centered(),
            ],
            height,
        );
    }

    let pose = if project_empty && app.workspace.project.markers.is_empty() {
        art::CAT_ASLEEP
    } else {
        art::CAT
    };

    let roomy = width >= 46;
    let show_context = width >= 40;

    // Budget the optional blocks so the whole composition always fits. Priority:
    // shortcuts (discovery) first, then the mascot, then the scene, then stars.
    let core = if show_context { 5 } else { 4 };
    let available = height.saturating_sub(core);
    let show_shortcuts = roomy && available >= 7;
    let mut remaining = available - if show_shortcuts { 7 } else { 0 };
    let show_cat = roomy && remaining >= 5;
    if show_cat {
        remaining -= 5;
    }
    let show_window = roomy && remaining >= 7;
    if show_window {
        remaining -= 7;
    }
    let show_stars = roomy && remaining >= 2;

    let mut lines: Vec<Line<'static>> = Vec::new();

    if show_stars {
        lines.push(art::star_scatter());
        lines.push(Line::from(""));
    }
    if show_window {
        lines.extend(art::code_window().into_iter().map(Line::centered));
        lines.push(Line::from(""));
    }
    if show_cat {
        lines.extend(art::art_lines(pose, theme::soft()));
        lines.push(Line::from(""));
    }

    lines.push(art::wordmark());
    let tagline = if width >= 34 {
        "your cozy little coding space"
    } else {
        "cozy coding space"
    };
    lines.push(
        Line::from(Span::styled(
            tagline,
            theme::muted().add_modifier(Modifier::ITALIC),
        ))
        .centered(),
    );
    lines.push(Line::from(""));

    if show_shortcuts {
        lines.extend(shortcuts());
        lines.push(Line::from(""));
    }

    let hint = if project_empty && width >= 40 {
        "no files yet · press Ctrl+O to open one"
    } else if width >= 34 {
        "press Ctrl+P to open a file"
    } else {
        open_hint
    };
    lines.push(Line::from(Span::styled(hint, theme::muted())).centered());
    if show_context {
        lines.push(context_line(app));
    }

    // Center vertically for a deliberate, balanced composition.
    center_vertically(lines, height)
}

fn center_vertically(mut lines: Vec<Line<'static>>, height: usize) -> Vec<Line<'static>> {
    if height > lines.len() {
        let top = (height - lines.len()) / 2;
        let mut padded: Vec<Line> = (0..top).map(|_| Line::from("")).collect();
        padded.append(&mut lines);
        lines = padded;
    }
    lines
}

fn shortcuts() -> Vec<Line<'static>> {
    let rows = [
        ("Ctrl+P", "quick open"),
        ("Ctrl+Shift+P", "command palette"),
        ("Ctrl+O", "open file"),
        ("Ctrl+B", "toggle files"),
        ("Ctrl+F", "find"),
        ("Ctrl+S", "save"),
    ];
    // Pad both columns so every row has an identical width and centering keeps
    // the key/description columns aligned.
    let key_width = rows
        .iter()
        .map(|(key, _)| key.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    let desc_width = rows
        .iter()
        .map(|(_, description)| description.chars().count())
        .max()
        .unwrap_or(0);

    rows.iter()
        .map(|(key, description)| {
            Line::from(vec![
                Span::styled("  · ", theme::dim()),
                Span::styled(format!("{key:<key_width$}"), theme::accent()),
                Span::styled(format!("{description:<desc_width$}"), theme::muted()),
            ])
            .centered()
        })
        .collect()
}

fn context_line(app: &App) -> Line<'static> {
    Line::from(vec![
        Span::styled("☾  ", theme::accent()),
        Span::styled(app.workspace.project.label().to_string(), theme::soft()),
        Span::styled("  ·  ", theme::dim()),
        Span::styled(app.workspace.root().display().to_string(), theme::muted()),
    ])
    .centered()
}
