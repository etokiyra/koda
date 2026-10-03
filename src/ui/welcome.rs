//! The welcome screen — Koda's home screen and little terminal art scene.
//!
//! Shown whenever no file is open. Besides the animated Koda familiar and the
//! wordmark, it presents a keyboard-navigable menu: open a file, open a
//! project, create a new project, resume the workspace's last session, and
//! reopen recent projects or files. The composition budgets its space so the
//! menu always fits, and degrades to a compact wordmark on small terminals.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::app::{App, WelcomeItem};
use crate::ui::art;
use crate::ui::theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let lines = compose(area, app);
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn compose(area: Rect, app: &App) -> Vec<Line<'static>> {
    let width = area.width as usize;
    let height = area.height as usize;

    // Tiny terminals: just the mark and one hint.
    if width < 30 || height < 6 {
        return center_vertically(
            vec![
                art::wordmark(),
                Line::from(""),
                Line::from(Span::styled(
                    "Ctrl+O open  ·  Ctrl+Shift+P commands",
                    theme::muted(),
                ))
                .centered(),
            ],
            height,
        );
    }

    let items = app.welcome_items();
    let roomy = width >= 44;
    let show_context = width >= 40;

    // Header: wordmark and tagline.
    let tagline = if width >= 34 {
        "your cozy little coding space"
    } else {
        "cozy coding space"
    };
    let header: Vec<Line<'static>> = vec![
        art::wordmark(),
        Line::from(Span::styled(
            tagline,
            theme::muted().add_modifier(Modifier::ITALIC),
        ))
        .centered(),
        Line::from(""),
    ];

    // Footer: navigation hint and (when there is room) the project context.
    let hint = if width >= 52 {
        "↑↓ choose  ·  Enter open  ·  v scene  ·  Ctrl+Shift+P"
    } else if width >= 44 {
        "↑↓ choose  ·  Enter open  ·  Ctrl+Shift+P commands"
    } else if width >= 32 {
        "↑↓ choose  ·  Enter open"
    } else {
        "↑↓ · Enter"
    };
    let mut footer: Vec<Line<'static>> =
        vec![Line::from(Span::styled(hint, theme::dim())).centered()];
    if show_context {
        footer.push(context_line(app));
    }

    // Optional art blocks, added in priority order while the menu still fits.
    let min_list = items.len().clamp(3, 6);
    let available_art = height.saturating_sub(header.len() + footer.len() + min_list + 1);
    let mut art_block: Vec<Line<'static>> = Vec::new();
    // The atmospheric scene is the centrepiece; fall back to just the familiar
    // when there is room for one but not the whole scene.
    let scene = art::scene(app.welcome_scene, app.anim_phase, app.motion);
    if roomy && available_art > scene.len() {
        art_block.extend(scene.into_iter().map(Line::centered));
        art_block.push(Line::from(""));
    } else if roomy && available_art >= 5 {
        art_block.extend(art::art_lines(cat_pose(app), theme::soft()));
        art_block.push(Line::from(""));
    }

    let list_rows = height
        .saturating_sub(header.len() + footer.len() + art_block.len() + 1)
        .max(3);
    let selected = app.welcome_selected.min(items.len().saturating_sub(1));
    let list = action_list(&items, selected, width, list_rows);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.extend(art_block);
    lines.extend(header);
    lines.extend(list);
    lines.push(Line::from(""));
    lines.extend(footer);

    center_vertically(lines, height)
}

/// The familiar's pose: asleep on an empty project, otherwise alive.
fn cat_pose(app: &App) -> &'static [&'static str] {
    if app.workspace.tree.is_empty() && app.workspace.project.markers.is_empty() {
        art::CAT_ASLEEP
    } else if app.anim_phase % 12 == 7 {
        art::CAT_BLINK
    } else {
        art::CAT
    }
}

/// The keyboard-navigable welcome menu, centred as a block.
fn action_list(
    items: &[WelcomeItem],
    selected: usize,
    width: usize,
    rows: usize,
) -> Vec<Line<'static>> {
    if items.is_empty() || rows == 0 {
        return Vec::new();
    }
    let panel = width.saturating_sub(6).clamp(24, 58);
    let start = if selected >= rows {
        selected + 1 - rows
    } else {
        0
    };
    let mut lines = Vec::new();
    for (index, item) in items.iter().enumerate().skip(start).take(rows) {
        let highlighted = index == selected;
        // Only spend width on the detail when the label still has room.
        let detail_len = item.detail.chars().count();
        let label_with_detail =
            panel.saturating_sub(2 + detail_len + usize::from(detail_len > 0) * 2);
        let show_detail = detail_len > 0 && label_with_detail >= 10;
        let detail_len = if show_detail { detail_len } else { 0 };
        let label_budget = panel.saturating_sub(2 + detail_len + usize::from(detail_len > 0) * 2);
        let label = truncate(&item.label, label_budget.max(6));
        let used = 2 + label.chars().count();
        let pad = panel.saturating_sub(used + detail_len);
        let label_style = if highlighted {
            theme::bright_bold()
        } else {
            theme::text()
        };
        let mut spans = vec![
            Span::styled(if highlighted { "❯ " } else { "  " }, theme::star()),
            Span::styled(label, label_style),
            Span::raw(" ".repeat(pad)),
        ];
        if show_detail {
            spans.push(Span::styled(item.detail.clone(), theme::dim()));
        }
        let mut line = Line::from(spans);
        if highlighted {
            line = line.style(Style::default().bg(theme::MENU_SELECTED_BG));
        }
        lines.push(line.centered());
    }
    lines
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

fn context_line(app: &App) -> Line<'static> {
    Line::from(vec![
        Span::styled("☾  ", theme::accent()),
        Span::styled(app.workspace.project.label().to_string(), theme::soft()),
        Span::styled("  ·  ", theme::dim()),
        Span::styled(app.workspace.root().display().to_string(), theme::muted()),
    ])
    .centered()
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    if max == 1 {
        return "…".to_string();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}
