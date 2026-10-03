//! Overlays: command palette, quick open, prompts and the search bar.
//!
//! Mellow's `ui.menu` language: a panel background, a blue accent border, stars
//! in the title, and a `ui.menu.selected` highlight row.

use std::collections::HashMap;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::app::overlay::{CompletionState, Help, HoverState, Picker, Prompt, Search, SearchField};
use crate::commands::CommandRegistry;
use crate::ui::{centered, theme};

/// Render a filterable list (command palette / quick open).
pub fn render_picker(frame: &mut Frame, area: Rect, picker: &Picker) {
    let width = ((area.width as u32 * 3 / 5) as u16).clamp(36, area.width.max(1));
    let rows = picker.filtered.len().min(12) as u16 + 4;
    // Keep enough room for the query row, the divider and a result area even
    // when the filter matches nothing.
    let height = rows.max(6).min(area.height);
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
    if picker.filtered.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("  no matches", theme::muted()))
                .style(Style::default().bg(theme::PANEL_BG)),
            list_area,
        );
        return;
    }
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
        let label_style = if !item.enabled {
            theme::muted()
        } else if selected {
            theme::bright_bold()
        } else {
            theme::text()
        };

        let marker = if selected {
            Span::styled(" ❯ ", theme::star())
        } else {
            Span::raw("   ")
        };

        // A disabled command shows why; an available one shows what it does.
        let description = if item.enabled {
            item.detail.as_str()
        } else {
            item.hint.as_deref().unwrap_or("")
        };
        let reserved = 3 + item.label.chars().count() + item.shortcut.chars().count() + 2;
        let description = truncate(description, width.saturating_sub(reserved));

        let mut spans = vec![marker, Span::styled(item.label.clone(), label_style)];
        if !description.is_empty() {
            spans.push(Span::styled("  ", theme::dim()));
            let style = if item.enabled {
                theme::dim()
            } else {
                theme::muted()
            };
            spans.push(Span::styled(description, style));
        }

        if !item.shortcut.is_empty() {
            let used: usize = spans.iter().map(|span| span.content.chars().count()).sum();
            let pad = width.saturating_sub(used + item.shortcut.chars().count() + 1);
            spans.push(Span::raw(" ".repeat(pad)));
            spans.push(Span::styled(item.shortcut.clone(), theme::accent()));
        }

        let mut line = Line::from(spans).style(Style::default().bg(theme::PANEL_BG));
        if selected {
            line = line.style(Style::default().bg(theme::MENU_SELECTED_BG));
        }
        lines.push(line);
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(Style::default().bg(theme::PANEL_BG)),
        list_area,
    );
}

/// Render the completion popup, anchored just below the cursor.
pub fn render_completion(
    frame: &mut Frame,
    area: Rect,
    state: &CompletionState,
    anchor: Option<(u16, u16)>,
) {
    if state.items.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }
    const MAX_ROWS: usize = 8;

    let label_width = state
        .items
        .iter()
        .map(|item| item.label.chars().count())
        .max()
        .unwrap_or(0);
    let width = ((label_width + 7).clamp(14, 48) as u16).min(area.width);
    let rows = state.items.len().min(MAX_ROWS);
    let height = (rows as u16 + 2).min(area.height);

    let (anchor_x, anchor_y) = anchor.unwrap_or((area.x, area.y));
    let x = anchor_x.min(area.x + area.width.saturating_sub(width));
    let below = anchor_y.saturating_add(1);
    let y = if below + height <= area.y + area.height {
        below
    } else {
        anchor_y.saturating_sub(height).max(area.y)
    };
    let rect = Rect {
        x,
        y,
        width,
        height,
    };

    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .style(Style::default().bg(theme::PANEL_BG))
        .title(Line::from(vec![
            Span::styled("✦ ", theme::star()),
            Span::styled("complete", theme::accent_bold()),
        ]));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height == 0 {
        return;
    }

    let visible = inner.height as usize;
    let start = if state.selected >= visible {
        state.selected + 1 - visible
    } else {
        0
    };
    let mut lines: Vec<Line> = Vec::new();
    for index in start..(start + visible).min(state.items.len()) {
        let item = &state.items[index];
        let selected = index == state.selected;
        let label_style = if selected {
            theme::bright_bold()
        } else {
            theme::text()
        };
        let kind_style = if selected {
            theme::accent()
        } else {
            theme::dim()
        };
        let marker = if selected {
            Span::styled(" ❯ ", theme::star())
        } else {
            Span::raw("   ")
        };
        let mut line = Line::from(vec![
            marker,
            Span::styled(format!("{} ", item.kind.glyph()), kind_style),
            Span::styled(item.label.clone(), label_style),
        ])
        .style(Style::default().bg(theme::PANEL_BG));
        if selected {
            line = line.style(Style::default().bg(theme::MENU_SELECTED_BG));
        }
        lines.push(line);
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(Style::default().bg(theme::PANEL_BG)),
        inner,
    );
}

/// Render the hover popup, anchored just below the cursor.
pub fn render_hover(frame: &mut Frame, area: Rect, hover: &HoverState, anchor: Option<(u16, u16)>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let body_width = hover
        .body
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    let width =
        ((hover.title.chars().count().max(body_width) + 4).clamp(18, 64) as u16).min(area.width);
    let rows = hover.body.len().max(1);
    let height = ((rows + 2) as u16).min(area.height);

    let (anchor_x, anchor_y) = anchor.unwrap_or((area.x, area.y));
    let x = anchor_x.min(area.x + area.width.saturating_sub(width));
    let below = anchor_y.saturating_add(1);
    let y = if below + height <= area.y + area.height {
        below
    } else {
        anchor_y.saturating_sub(height).max(area.y)
    };
    let rect = Rect {
        x,
        y,
        width,
        height,
    };

    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .style(Style::default().bg(theme::PANEL_BG))
        .title(Line::from(vec![
            Span::styled("✦ ", theme::star()),
            Span::styled(hover.title.clone(), theme::accent_bold()),
        ]));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height == 0 {
        return;
    }

    let lines: Vec<Line> = hover
        .body
        .iter()
        .take(inner.height as usize)
        .map(|line| Line::from(Span::styled(line.clone(), theme::text())))
        .collect();
    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(Style::default().bg(theme::PANEL_BG)),
        inner,
    );
}

/// Render the keyboard-shortcuts cheatsheet.
pub fn render_help(
    frame: &mut Frame,
    area: Rect,
    help: &Help,
    commands: &CommandRegistry,
    phase: usize,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let content = help_lines(commands, phase);
    let width = ((area.width as u32 * 3 / 5) as u16)
        .clamp(40, 76)
        .min(area.width.max(1));
    let height = (content.len() as u16 + 2).min(area.height);
    let rect = centered(area, width, height);

    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .style(Style::default().bg(theme::PANEL_BG))
        .title(Line::from(vec![
            Span::styled("✦ ", theme::star()),
            Span::styled("keyboard shortcuts", theme::accent_bold()),
            Span::styled(" ✦", theme::star()),
        ]));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height == 0 {
        return;
    }

    let visible = inner.height as usize;
    let max_scroll = content.len().saturating_sub(visible);
    let scroll = help.scroll.min(max_scroll);
    let slice: Vec<Line> = content.into_iter().skip(scroll).take(visible).collect();
    frame.render_widget(
        Paragraph::new(Text::from(slice)).style(Style::default().bg(theme::PANEL_BG)),
        inner,
    );
}

/// Build the cheatsheet from the command registry (so it never goes stale) plus
/// a few editor-movement keys that are not commands.
fn help_lines(commands: &CommandRegistry, phase: usize) -> Vec<Line<'static>> {
    const EDITOR: &[(&str, &str)] = &[
        ("Arrows", "move the cursor"),
        ("Ctrl+←/→", "move by word"),
        ("Shift+Arrows", "select"),
        ("Home / End", "line start / end"),
        ("Ctrl+Home / End", "document start / end"),
        ("PageUp / PageDown", "scroll a page"),
    ];

    let mut order: Vec<&'static str> = Vec::new();
    let mut groups: HashMap<&'static str, Vec<(&'static str, &'static str)>> = HashMap::new();
    for command in commands.all() {
        let Some(shortcut) = command.shortcut else {
            continue;
        };
        if !groups.contains_key(command.category) {
            order.push(command.category);
        }
        groups
            .entry(command.category)
            .or_default()
            .push((shortcut, command.title));
    }

    // Align every shortcut column to the widest entry.
    let key_width = commands
        .all()
        .iter()
        .filter_map(|command| command.shortcut)
        .chain(EDITOR.iter().map(|(key, _)| *key))
        .map(|key| key.chars().count())
        .max()
        .unwrap_or(0);

    let mut lines: Vec<Line<'static>> = Vec::new();
    for category in order {
        lines.push(Line::from(Span::styled(
            category.to_string(),
            theme::accent_bold(),
        )));
        for (shortcut, title) in &groups[category] {
            lines.push(help_row(shortcut, title, key_width));
        }
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled("Editor", theme::accent_bold())));
    for (shortcut, title) in EDITOR {
        lines.push(help_row(shortcut, title, key_width));
    }
    lines.push(Line::from(""));

    // A little familiar to keep it warm, blinking on the blink frame.
    let face = if phase % 12 == 7 {
        "( -ω- )"
    } else {
        "( ･ω･ )"
    };
    lines.push(Line::from(vec![
        Span::styled(format!("  {face}  "), theme::soft()),
        Span::styled("Esc to close", theme::muted()),
    ]));
    lines
}

fn help_row(shortcut: &str, title: &str, key_width: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled("  ", theme::dim()),
        Span::styled(format!("{shortcut:<key_width$}"), theme::accent()),
        Span::styled("  ", theme::dim()),
        Span::styled(title.to_string(), theme::text()),
    ])
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
    if search.case_sensitive {
        query_spans.push(Span::styled("  Aa", theme::accent_bold()));
    }
    if search.whole_word {
        query_spans.push(Span::styled("  |ab|", theme::accent_bold()));
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

/// Shorten text to `max` columns, appending an ellipsis when clipped.
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
