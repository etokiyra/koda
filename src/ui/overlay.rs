//! Overlays: command palette, quick open, prompts and the search bar.
//!
//! Mellow's `ui.menu` language: a revolver panel, a lilac border, stars in the
//! title, and a warm bossanova highlight.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::app::overlay::{Picker, Prompt, Search, SearchField};
use crate::ui::{centered, theme};

/// Render a filterable list (command palette / quick open).
pub fn render_picker(frame: &mut Frame, area: Rect, picker: &Picker) {
    let width = ((area.width as u32 * 3 / 5) as u16).clamp(36, area.width.max(1));
    let rows = picker.filtered.len().min(12) as u16 + 4;
    let height = rows.min(area.height);
    let rect = centered(area, width, height);

    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .style(Style::default().bg(theme::PANEL_BG))
        .title(Line::from(vec![
            Span::styled("✦ ", theme::star()),
            Span::styled(picker.title.clone(), theme::accent_bold()),
            Span::styled(" ✦", theme::star()),
        ]));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    if inner.height < 3 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner);

    // Query input.
    let input = if picker.query.is_empty() {
        Span::styled(picker.placeholder.clone(), theme::muted())
    } else {
        Span::styled(picker.query.clone(), theme::bright())
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(" ❯ ", theme::star()), input]))
            .style(Style::default().bg(theme::PANEL_BG)),
        chunks[0],
    );
    let cursor_x = chunks[0].x + 3 + picker.query.chars().count() as u16;
    if cursor_x < chunks[0].x + chunks[0].width {
        frame.set_cursor_position((cursor_x, chunks[0].y));
    }
    frame.render_widget(
        Paragraph::new(Span::styled("─".repeat(inner.width as usize), theme::dim()))
            .style(Style::default().bg(theme::PANEL_BG)),
        chunks[1],
    );

    // Results.
    let list_area = chunks[2];
    let visible = list_area.height as usize;
    let start = if picker.selected >= visible {
        picker.selected + 1 - visible
    } else {
        0
    };
    let width = list_area.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    for index in start..(start + visible).min(picker.filtered.len()) {
        let Some(item) = picker.item(index) else {
            continue;
        };
        let selected = index == picker.selected;
        let label_style = if selected {
            theme::bright_bold()
        } else {
            theme::text()
        };

        let marker = if selected {
            Span::styled(" ❯ ", theme::star())
        } else {
            Span::raw("   ")
        };
        let mut spans = vec![marker, Span::styled(item.label.clone(), label_style)];
        if !item.detail.is_empty() {
            spans.push(Span::styled("  ", theme::dim()));
            spans.push(Span::styled(item.detail.clone(), theme::dim()));
        }

        if !item.shortcut.is_empty() {
            let used: usize = spans.iter().map(|span| span.content.chars().count()).sum();
            let pad = width.saturating_sub(used + item.shortcut.chars().count() + 1);
            spans.push(Span::raw(" ".repeat(pad)));
            spans.push(Span::styled(item.shortcut.clone(), theme::accent()));
        }

        let mut line = Line::from(spans).style(Style::default().bg(theme::PANEL_BG));
        if selected {
            line = line.style(Style::default().bg(theme::CURSORLINE_BG));
        }
        lines.push(line);
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(Style::default().bg(theme::PANEL_BG)),
        list_area,
    );
}

/// Render a single-line text prompt.
pub fn render_prompt(frame: &mut Frame, area: Rect, prompt: &Prompt) {
    let width = (area.width.saturating_sub(8)).clamp(28, 72);
    let rect = centered(area, width, 3);

    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .style(Style::default().bg(theme::PANEL_BG))
        .title(Line::from(vec![
            Span::styled("✦ ", theme::star()),
            Span::styled(prompt.label.clone(), theme::accent_bold()),
            Span::styled(" ✦", theme::star()),
        ]));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    if inner.height == 0 {
        return;
    }

    let input = if prompt.input.is_empty() {
        Span::styled(prompt.placeholder.clone(), theme::muted())
    } else {
        Span::styled(prompt.input.clone(), theme::bright())
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(" ❯ ", theme::star()), input]))
            .style(Style::default().bg(theme::PANEL_BG)),
        inner,
    );

    let cursor_x = inner.x + 3 + prompt.input.chars().count() as u16;
    if cursor_x < inner.x + inner.width {
        frame.set_cursor_position((cursor_x, inner.y));
    }
}

/// Render the find / replace bar as a solid strip above the statusline.
pub fn render_search(frame: &mut Frame, area: Rect, search: &Search) {
    let on_panel = Style::default().bg(theme::PANEL_BG);
    let query_focused = search.field == SearchField::Query;
    let mut lines: Vec<Line> = Vec::new();

    let query_pill = if query_focused {
        theme::pill(theme::ACCENT_SOFT, theme::PANEL_BG)
    } else {
        theme::pill(theme::MUTED, theme::PANEL_BG)
    };

    let counter = if search.query.is_empty() {
        String::new()
    } else if search.matches.is_empty() {
        "  no matches".to_string()
    } else {
        let current = search.current.map(|c| c + 1).unwrap_or(0);
        format!("  {current}/{}", search.matches.len())
    };

    let mut query_spans = vec![
        Span::styled(" find ", query_pill),
        Span::styled(" ", on_panel),
        Span::styled(search.query.clone(), theme::bright()),
    ];
    if !counter.is_empty() {
        query_spans.push(Span::styled(counter, theme::muted()));
    }
    pad_line(&mut query_spans, area.width);
    lines.push(Line::from(query_spans));

    if search.replace_mode {
        let replace_pill = if query_focused {
            theme::pill(theme::MUTED, theme::PANEL_BG)
        } else {
            theme::pill(theme::ACCENT_SOFT, theme::PANEL_BG)
        };
        let mut spans = vec![
            Span::styled(" replace ", replace_pill),
            Span::styled(" ", on_panel),
            Span::styled(search.replacement.clone(), theme::bright()),
        ];
        pad_line(&mut spans, area.width);
        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), area);

    let cursor_col = if query_focused {
        search.query.chars().count()
    } else {
        search.replacement.chars().count()
    };
    let cursor_y = area.y + if query_focused { 0 } else { 1 };
    let cursor_x = area.x + 7 + cursor_col as u16;
    if cursor_x < area.x + area.width {
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

/// Fill the remaining width of a search row with the panel background.
fn pad_line(spans: &mut Vec<Span<'static>>, width: u16) {
    let used: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    let pad = (width as usize).saturating_sub(used);
    spans.push(Span::styled(
        " ".repeat(pad),
        Style::default().bg(theme::PANEL_BG),
    ));
}
