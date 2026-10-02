//! Overlays: command palette, quick open, prompts and the search bar.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::app::overlay::{Picker, Prompt, Search, SearchField};
use crate::ui::centered;
use crate::ui::theme;

/// Render a filterable list (command palette / quick open).
pub fn render_picker(frame: &mut Frame, area: Rect, picker: &Picker) {
    let width = ((area.width as u32 * 3 / 5) as u16).clamp(32, area.width.max(1));
    let rows = picker.filtered.len().min(12) as u16 + 4;
    let height = rows.min(area.height);
    let rect = centered(area, width, height);

    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .style(Style::default().bg(theme::OVERLAY_BG))
        .title(format!(" {} ", picker.title));
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
        Span::styled(picker.placeholder.clone(), theme::dim())
    } else {
        Span::styled(picker.query.clone(), theme::text())
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled("❯ ", theme::accent()), input])),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled("─".repeat(inner.width as usize), theme::dim())),
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
            theme::accent_bold()
        } else {
            theme::text()
        };

        let mut spans = vec![
            Span::styled(" ", label_style),
            Span::styled(item.label.clone(), label_style),
        ];
        if !item.detail.is_empty() {
            spans.push(Span::styled("  ", theme::dim()));
            spans.push(Span::styled(item.detail.clone(), theme::dim()));
        }

        if !item.shortcut.is_empty() {
            let used: usize = spans.iter().map(|span| span.content.chars().count()).sum();
            let pad = width.saturating_sub(used + item.shortcut.chars().count() + 1);
            spans.push(Span::raw(" ".repeat(pad)));
            spans.push(Span::styled(item.shortcut.clone(), theme::dim()));
        }

        let mut line = Line::from(spans);
        if selected {
            line = line.style(Style::default().bg(theme::SIDEBAR_SELECTED_BG));
        }
        lines.push(line);
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), list_area);
}

/// Render a single-line text prompt.
pub fn render_prompt(frame: &mut Frame, area: Rect, prompt: &Prompt) {
    let width = (area.width.saturating_sub(8)).clamp(24, 72);
    let rect = centered(area, width, 3);

    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .style(Style::default().bg(theme::OVERLAY_BG))
        .title(format!(" {} ", prompt.label));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    if inner.height == 0 {
        return;
    }

    let input = if prompt.input.is_empty() {
        Span::styled(prompt.placeholder.clone(), theme::dim())
    } else {
        Span::styled(prompt.input.clone(), theme::text())
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled("❯ ", theme::accent()), input])),
        inner,
    );

    let cursor_x = inner.x + 2 + prompt.input.chars().count() as u16;
    if cursor_x < inner.x + inner.width {
        frame.set_cursor_position((cursor_x, inner.y));
    }
}

/// Render the find / replace bar.
pub fn render_search(frame: &mut Frame, area: Rect, search: &Search) {
    let mut lines: Vec<Line> = Vec::new();
    let query_focused = search.field == SearchField::Query;

    let counter = if search.query.is_empty() {
        String::new()
    } else if search.matches.is_empty() {
        "  no matches".to_string()
    } else {
        let current = search.current.map(|c| c + 1).unwrap_or(0);
        format!("  {current}/{}", search.matches.len())
    };

    let query_style = if query_focused {
        theme::accent_bold()
    } else {
        theme::dim()
    };
    let mut query_spans = vec![
        Span::styled(" Find    ", query_style),
        Span::styled(search.query.clone(), theme::text()),
    ];
    if !counter.is_empty() {
        query_spans.push(Span::styled(counter, theme::dim()));
    }
    lines.push(Line::from(query_spans));

    if search.replace_mode {
        let replace_style = if !query_focused {
            theme::accent_bold()
        } else {
            theme::dim()
        };
        lines.push(Line::from(vec![
            Span::styled(" Replace ", replace_style),
            Span::styled(search.replacement.clone(), theme::text()),
        ]));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), area);

    let cursor_col = if query_focused {
        search.query.chars().count()
    } else {
        search.replacement.chars().count()
    };
    let cursor_y = area.y + if query_focused { 0 } else { 1 };
    let cursor_x = area.x + 9 + cursor_col as u16;
    if cursor_x < area.x + area.width {
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}
