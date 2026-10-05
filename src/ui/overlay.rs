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

use crate::app::overlay::{
    CompletionState, DiffLineKind, DiffState, DirEntryKind, DirPicker, Help, HoverState,
    NewProject, NewProjectStep, Picker, Prompt, Search, SearchField,
};
use crate::app::{Toast, ToastKind};
use crate::commands::CommandRegistry;
use crate::project::create;
use crate::ui::{art, centered, theme};

/// Render a filterable list (command palette / quick open).
pub fn render_picker(frame: &mut Frame, area: Rect, picker: &Picker) {
    let width = ((area.width as u32 * 3 / 5) as u16).clamp(36, area.width.max(1));
    // An empty result gets a little extra room so the familiar and its hint can
    // breathe instead of being clipped to a single row.
    let rows = if picker.filtered.is_empty() {
        12
    } else {
        picker.filtered.len().min(12) as u16 + 4
    };
    // Keep enough room for the query row, the divider and a result area even
    // when the filter matches nothing.
    let height = rows.max(6).min(area.height);
    let rect = centered(area, width, height);
    let on_panel = Style::default().bg(theme::panel_bg());

    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .style(on_panel)
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

    // A hint row is a small luxury: only spend the line when there is room.
    let show_footer = inner.height >= 6;
    let constraints = if show_footer {
        vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ]
    } else {
        vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    // Query input, with a position counter on the right when there are results.
    let input = if picker.query.is_empty() {
        Span::styled(picker.placeholder.clone(), theme::muted())
    } else {
        Span::styled(picker.query.clone(), theme::bright())
    };
    let mut query_spans = vec![Span::styled(" ❯ ", theme::star()), input];
    if !picker.filtered.is_empty() {
        let counter = format!("{}/{}", picker.selected + 1, picker.filtered.len());
        let used: usize = query_spans
            .iter()
            .map(|span| span.content.chars().count())
            .sum();
        let pad = (chunks[0].width as usize).saturating_sub(used + counter.chars().count() + 1);
        query_spans.push(Span::raw(" ".repeat(pad)));
        query_spans.push(Span::styled(counter, theme::dim()));
    }
    frame.render_widget(
        Paragraph::new(Line::from(query_spans)).style(on_panel),
        chunks[0],
    );
    let cursor_x = chunks[0].x + 3 + picker.query.chars().count() as u16;
    if cursor_x < chunks[0].x + chunks[0].width {
        frame.set_cursor_position((cursor_x, chunks[0].y));
    }
    frame.render_widget(
        Paragraph::new(Span::styled("─".repeat(inner.width as usize), theme::dim()))
            .style(on_panel),
        chunks[1],
    );

    // Results.
    let list_area = chunks[2];
    if picker.filtered.is_empty() {
        let empty = picker_empty(picker, list_area.height as usize);
        frame.render_widget(Paragraph::new(Text::from(empty)).style(on_panel), list_area);
        render_picker_footer(
            frame,
            show_footer.then(|| chunks[3]),
            picker_footer_hint(picker),
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

        let mut line = Line::from(spans).style(on_panel);
        if selected {
            line = line.style(Style::default().bg(theme::menu_selected_bg()));
        }
        lines.push(line);
    }

    frame.render_widget(Paragraph::new(Text::from(lines)).style(on_panel), list_area);
    render_picker_footer(
        frame,
        show_footer.then(|| chunks[3]),
        picker_footer_hint(picker),
    );
}

/// Context-specific extra hints for a picker's footer.
fn picker_footer_hint(picker: &Picker) -> Option<&'static str> {
    match picker.title.as_str() {
        "Changed Files" => Some("Space stage · d diff"),
        _ => None,
    }
}

/// A warm, contextual empty state for a picker with nothing to show.
///
/// A missing search result should never feel like a dead end, so the familiar
/// keeps the user company and the hint suggests what to do next. Short panels
/// get the single-line familiar instead of the full composition.
fn picker_empty(picker: &Picker, height: usize) -> Vec<Line<'static>> {
    let (pose, title, hint): (&[&str], &str, &str) = match picker.title.as_str() {
        "Changed Files" => (
            art::CAT_ASLEEP,
            "the working tree is clean",
            "edit a file and it will appear here",
        ),
        "Diagnostics" => (
            art::CAT_HAPPY,
            "no problems found",
            "a tidy little codebase",
        ),
        "Search Results" => (
            art::CAT_CURIOUS,
            "nothing matched",
            "try a different word or pattern",
        ),
        "Workspace Symbols" => (
            art::CAT_CURIOUS,
            "no symbols found",
            "this project may still be waking up",
        ),
        "Symbols" => (
            art::CAT_CURIOUS,
            "no symbols in this file",
            "nothing to outline yet",
        ),
        "References" => (
            art::CAT_CURIOUS,
            "no references found",
            "the symbol may be unused here",
        ),
        "Code Actions" => (
            art::CAT_SLEEPY,
            "no quick fixes here",
            "the code under the cursor looks settled",
        ),
        "Language Setup" => (
            art::CAT_HAPPY,
            "every language tool is ready",
            "nothing left to install",
        ),
        _ => (
            art::CAT_CURIOUS,
            "no matches",
            "keep typing to narrow it down",
        ),
    };
    if height >= 6 {
        art::empty_state(pose, title, hint)
    } else if height >= 1 {
        vec![art::familiar_line(title)]
    } else {
        Vec::new()
    }
}

/// Draw the picker's footer hint row, when there is room for one.
fn render_picker_footer(frame: &mut Frame, area: Option<Rect>, extra: Option<&str>) {
    let Some(area) = area else {
        return;
    };
    let mut spans = vec![
        Span::styled(" ↑↓ move ", theme::dim()),
        Span::styled("·", theme::dim()),
        Span::styled(" Enter select ", theme::dim()),
    ];
    if let Some(extra) = extra {
        spans.push(Span::styled("·", theme::dim()));
        spans.push(Span::styled(format!(" {extra} "), theme::dim()));
    }
    spans.push(Span::styled("·", theme::dim()));
    spans.push(Span::styled(" Esc close ", theme::dim()));
    let line = Line::from(spans);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme::panel_bg())),
        area,
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
        .style(Style::default().bg(theme::panel_bg()))
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
        .style(Style::default().bg(theme::panel_bg()));
        if selected {
            line = line.style(Style::default().bg(theme::menu_selected_bg()));
        }
        lines.push(line);
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(Style::default().bg(theme::panel_bg())),
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
        .style(Style::default().bg(theme::panel_bg()))
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
        Paragraph::new(Text::from(lines)).style(Style::default().bg(theme::panel_bg())),
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
        .style(Style::default().bg(theme::panel_bg()))
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
        Paragraph::new(Text::from(slice)).style(Style::default().bg(theme::panel_bg())),
        inner,
    );
}

/// Render the signature-help popup, anchored just below the cursor.
pub fn render_signature(
    frame: &mut Frame,
    area: Rect,
    help: &crate::language::lsp::convert::SignatureHelp,
    anchor: Option<(u16, u16)>,
) {
    let Some(signature) = help.signatures.get(help.active) else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let on_panel = Style::default().bg(theme::panel_bg());

    // The label with the active parameter emphasised.
    let (before, parameter, after) =
        split_parameter(&signature.label, &signature.parameters, help.parameter);
    let mut spans = vec![Span::styled("✦ ", theme::star())];
    match parameter {
        Some(parameter) => {
            spans.push(Span::styled(before.to_string(), theme::text()));
            spans.push(Span::styled(parameter.to_string(), theme::accent_bold()));
            spans.push(Span::styled(after.to_string(), theme::text()));
        }
        None => spans.push(Span::styled(signature.label.clone(), theme::text())),
    }

    let documentation = signature
        .documentation
        .as_deref()
        .and_then(|text| text.lines().find(|line| !line.trim().is_empty()));
    let label_width = signature.label.chars().count();
    let doc_width = documentation.map_or(0, |line| line.trim().chars().count());
    let width = ((label_width.max(doc_width) + 4).clamp(24, 84) as u16).min(area.width);
    let inner_width = width.saturating_sub(2) as usize;

    let mut rows: Vec<Line> = vec![Line::from(spans)];
    if let Some(documentation) = documentation {
        rows.push(Line::from(Span::styled(
            truncate(documentation.trim(), inner_width),
            theme::muted(),
        )));
    }
    if help.signatures.len() > 1 {
        rows.push(Line::from(Span::styled(
            format!("{}/{} signatures", help.active + 1, help.signatures.len()),
            theme::dim(),
        )));
    }
    let height = (rows.len() as u16 + 2).min(area.height);

    let (ax, ay) = anchor.unwrap_or((area.x, area.y));
    let x = ax.min(area.x + area.width.saturating_sub(width));
    let below = ay.saturating_add(1);
    let y = if below + height <= area.y + area.height {
        below
    } else {
        ay.saturating_sub(height).max(area.y)
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
        .style(on_panel)
        .title(Line::from(vec![
            Span::styled("✦ ", theme::star()),
            Span::styled("signature", theme::accent_bold()),
        ]));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height == 0 {
        return;
    }

    // The paragraph clips horizontally at the panel edge, keeping the styled
    // signature spans intact.
    frame.render_widget(Paragraph::new(Text::from(rows)).style(on_panel), inner);
}

/// Split a signature label around its active parameter, if it can be found.
fn split_parameter<'a>(
    signature: &'a str,
    parameters: &'a [String],
    active: usize,
) -> (&'a str, Option<&'a str>, &'a str) {
    let Some(parameter) = parameters.get(active) else {
        return (signature, None, "");
    };
    if parameter.is_empty() {
        return (signature, None, "");
    }
    match signature.find(parameter.as_str()) {
        Some(index) => (
            &signature[..index],
            Some(&signature[index..index + parameter.len()]),
            &signature[index + parameter.len()..],
        ),
        None => (signature, None, ""),
    }
}

/// Render a unified diff in a scrollable panel.
pub fn render_diff(frame: &mut Frame, area: Rect, diff: &DiffState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = ((area.width as u32 * 4 / 5) as u16)
        .clamp(40, 140)
        .min(area.width.max(1));
    let height = area.height.saturating_sub(2).max(3).min(area.height);
    let rect = centered(area, width, height);

    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .style(Style::default().bg(theme::panel_bg()))
        .title(Line::from(vec![
            Span::styled("✦ ", theme::star()),
            Span::styled(diff.title.clone(), theme::accent_bold()),
            Span::styled(" ✦", theme::star()),
        ]));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height == 0 {
        return;
    }

    // Reserve the last row for a footer hint.
    let visible = (inner.height as usize).saturating_sub(1).max(1);
    let max_scroll = diff.lines.len().saturating_sub(visible);
    let scroll = diff.scroll.min(max_scroll);
    let lines: Vec<Line> = diff
        .lines
        .iter()
        .skip(scroll)
        .take(visible)
        .map(|line| {
            let style = match line.kind {
                DiffLineKind::Add => Style::default().fg(theme::success_color()),
                DiffLineKind::Remove => Style::default().fg(theme::error_color()),
                DiffLineKind::Hunk => Style::default().fg(theme::accent_color()),
                DiffLineKind::Header => Style::default().fg(theme::muted_color()),
                DiffLineKind::Context => Style::default().fg(theme::text_color()),
            };
            Line::from(Span::styled(format!(" {} ", line.text), style))
        })
        .collect();
    let content_area = Rect {
        height: inner.height.saturating_sub(1),
        ..inner
    };
    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(Style::default().bg(theme::panel_bg())),
        content_area,
    );

    let last = diff.lines.len();
    let footer = Line::from(vec![
        Span::styled(" ↑↓ scroll ", theme::muted()),
        Span::styled("·", theme::dim()),
        Span::styled(format!(" {last} line(s) "), theme::muted()),
        Span::styled("·", theme::dim()),
        Span::styled(" Esc close ", theme::muted()),
    ]);
    let footer_area = Rect {
        y: inner.y + inner.height.saturating_sub(1),
        height: 1,
        ..inner
    };
    frame.render_widget(
        Paragraph::new(footer).style(Style::default().bg(theme::panel_bg())),
        footer_area,
    );
}

/// Build the cheatsheet from the command registry (so it never goes stale) plus
/// a few editor-movement keys that are not commands.
fn help_lines(commands: &CommandRegistry, phase: usize) -> Vec<Line<'static>> {
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
        .chain(crate::commands::EDITOR_KEYS.iter().map(|(key, _)| *key))
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
    for (shortcut, title) in crate::commands::EDITOR_KEYS {
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
    // A little extra height buys a hint row; small terminals keep it minimal.
    let height = if area.height >= 6 { 4 } else { 3 };
    let rect = centered(area, width, height);
    let on_panel = Style::default().bg(theme::panel_bg());

    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .style(on_panel)
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
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    let input = if prompt.input.is_empty() {
        Span::styled(prompt.placeholder.clone(), theme::muted())
    } else {
        Span::styled(prompt.input.clone(), theme::bright())
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(" ❯ ", theme::star()), input])).style(on_panel),
        rows[0],
    );

    if rows.len() > 1 && rows[1].height > 0 {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "   Enter confirm  ·  Esc cancel",
                theme::dim(),
            ))
            .style(on_panel),
            rows[1],
        );
    }

    let cursor_x = rows[0].x + 3 + prompt.input.chars().count() as u16;
    if cursor_x < rows[0].x + rows[0].width {
        frame.set_cursor_position((cursor_x, rows[0].y));
    }
}

/// Render the find / replace bar as a solid strip above the statusline.
pub fn render_search(frame: &mut Frame, area: Rect, search: &Search) {
    let on_panel = Style::default().bg(theme::panel_bg());
    let query_focused = search.field == SearchField::Query;
    let mut lines: Vec<Line> = Vec::new();

    let query_pill = if query_focused {
        theme::pill(theme::accent_soft(), theme::panel_bg())
    } else {
        theme::pill(theme::muted_color(), theme::panel_bg())
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
    if search.regex {
        query_spans.push(Span::styled("  .*", theme::accent_bold()));
    }
    if let Some(error) = &search.regex_error {
        query_spans.push(Span::styled(format!("  {error}"), theme::error()));
    }
    pad_line(&mut query_spans, area.width);
    lines.push(Line::from(query_spans));

    if search.replace_mode {
        let replace_pill = if query_focused {
            theme::pill(theme::muted_color(), theme::panel_bg())
        } else {
            theme::pill(theme::accent_soft(), theme::panel_bg())
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

// ---------------------------------------------------------------------------
// Directory picker and new-project flow
// ---------------------------------------------------------------------------

/// Draw a rounded panel with a title, returning its inner rect.
fn draw_panel(
    frame: &mut Frame,
    area: Rect,
    title: Line<'static>,
    width: u16,
    height: u16,
) -> Rect {
    let rect = centered(area, width, height);
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .style(Style::default().bg(theme::panel_bg()))
        .title(title);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    inner
}

fn panel_title(text: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("✦ ", theme::star()),
        Span::styled(text.to_string(), theme::accent_bold()),
        Span::styled(" ✦", theme::star()),
    ])
}

fn panel_subtitle(text: &str, width: u16) -> Line<'static> {
    Line::from(vec![
        Span::styled("  ", theme::dim()),
        Span::styled(
            truncate(text, width.saturating_sub(3) as usize),
            theme::bright(),
        ),
    ])
}

fn render_divider(frame: &mut Frame, rect: Rect) {
    frame.render_widget(
        Paragraph::new(Span::styled("─".repeat(rect.width as usize), theme::dim()))
            .style(Style::default().bg(theme::panel_bg())),
        rect,
    );
}

fn render_footer(frame: &mut Frame, rect: Rect, text: &str, error: bool) {
    let style = if error { theme::error() } else { theme::dim() };
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(
                "  {}",
                truncate(text, rect.width.saturating_sub(3) as usize)
            ),
            style,
        ))
        .style(Style::default().bg(theme::panel_bg())),
        rect,
    );
}

/// Render selectable `(label, detail)` rows with the selected row highlighted.
fn render_panel_rows(frame: &mut Frame, list: Rect, rows: &[(String, String)], selected: usize) {
    let on_panel = Style::default().bg(theme::panel_bg());
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(art::empty_state(
                art::CAT_CURIOUS,
                "nothing here",
                "try another folder",
            ))
            .style(on_panel),
            list,
        );
        return;
    }
    let visible = list.height as usize;
    let start = if selected >= visible {
        selected + 1 - visible
    } else {
        0
    };
    let width = list.width as usize;
    // Align the detail column to the widest label so the list reads as a table.
    let label_width = rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0);
    let mut lines = Vec::new();
    for (index, (label, detail)) in rows.iter().enumerate().skip(start).take(visible) {
        let highlighted = index == selected;
        let marker = if highlighted {
            Span::styled(" ❯ ", theme::star())
        } else {
            Span::raw("   ")
        };
        let label_style = if highlighted {
            theme::bright_bold()
        } else {
            theme::text()
        };
        let mut spans = vec![
            marker,
            Span::styled(format!("{label:<label_width$}"), label_style),
        ];
        if !detail.is_empty() {
            spans.push(Span::styled("  ", theme::dim()));
            let used = 3 + label_width + 2;
            let budget = width.saturating_sub(used + 1);
            spans.push(Span::styled(truncate(detail, budget), theme::dim()));
        }
        let mut line = Line::from(spans).style(on_panel);
        if highlighted {
            line = line.style(Style::default().bg(theme::menu_selected_bg()));
        }
        lines.push(line);
    }
    frame.render_widget(Paragraph::new(Text::from(lines)).style(on_panel), list);
}

/// A compact path, using `~` for the home directory when possible.
fn compact_path(path: &std::path::Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from)
        && let Ok(relative) = path.strip_prefix(&home)
    {
        return format!("~/{}", relative.display());
    }
    path.display().to_string()
}

fn directory_rows(entries: &[crate::app::overlay::DirEntry]) -> Vec<(String, String)> {
    entries
        .iter()
        .map(|entry| {
            let label = match entry.kind {
                DirEntryKind::ChooseCurrent => "✓  use this folder".to_string(),
                DirEntryKind::Parent => "..".to_string(),
                DirEntryKind::Directory => format!("{}/", entry.name),
            };
            (label, String::new())
        })
        .collect()
}

/// Render the "Open Project…" directory browser.
pub fn render_dir_picker(frame: &mut Frame, area: Rect, picker: &DirPicker) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let browser = &picker.browser;
    let rows = directory_rows(&browser.entries);
    let width = ((area.width as u32 * 3 / 5) as u16)
        .clamp(40, 84)
        .min(area.width.max(1));
    let visible = rows.len().min(14);
    let height = (visible as u16 + 4).min(area.height);
    let inner = draw_panel(frame, area, panel_title("Open Project"), width, height);
    if inner.height < 4 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);
    frame.render_widget(
        Paragraph::new(panel_subtitle(
            &compact_path(&browser.current),
            chunks[0].width,
        ))
        .style(Style::default().bg(theme::panel_bg())),
        chunks[0],
    );
    render_divider(frame, chunks[1]);
    render_panel_rows(frame, chunks[2], &rows, browser.selected);
    match &browser.error {
        Some(error) => render_footer(frame, chunks[3], error, true),
        None => render_footer(
            frame,
            chunks[3],
            "↑↓ move  ·  Enter open  ·  ← up  ·  Esc cancel",
            false,
        ),
    }
}

/// Render the guided "Create a new project" flow.
pub fn render_new_project(frame: &mut Frame, area: Rect, flow: &NewProject) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = ((area.width as u32 * 3 / 5) as u16)
        .clamp(42, 86)
        .min(area.width.max(1));
    let on_panel = Style::default().bg(theme::panel_bg());

    match flow.step {
        NewProjectStep::Parent => {
            let rows = directory_rows(&flow.browser.entries);
            let visible = rows.len().min(14);
            let height = (visible as u16 + 4).min(area.height);
            let title = panel_title("New Project  ·  1 of 3  choose a folder");
            let inner = draw_panel(frame, area, title, width, height);
            if inner.height < 4 {
                return;
            }
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(inner);
            frame.render_widget(
                Paragraph::new(panel_subtitle(
                    &compact_path(&flow.browser.current),
                    chunks[0].width,
                ))
                .style(on_panel),
                chunks[0],
            );
            render_divider(frame, chunks[1]);
            render_panel_rows(frame, chunks[2], &rows, flow.browser.selected);
            match &flow.error {
                Some(error) => render_footer(frame, chunks[3], error, true),
                None => render_footer(
                    frame,
                    chunks[3],
                    "↑↓ move  ·  Enter choose / open  ·  ← up  ·  Esc cancel",
                    false,
                ),
            }
        }
        NewProjectStep::Name => {
            let height = 7.min(area.height);
            let title = panel_title("New Project  ·  2 of 3  name it");
            let inner = draw_panel(frame, area, title, width, height);
            if inner.height == 0 {
                return;
            }
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(0),
                ])
                .split(inner);
            frame.render_widget(
                Paragraph::new(panel_subtitle(
                    &format!("in {}", compact_path(&flow.parent)),
                    chunks[0].width,
                ))
                .style(on_panel),
                chunks[0],
            );
            render_divider(frame, chunks[1]);
            let input = if flow.name.is_empty() {
                Span::styled("project-name", theme::muted())
            } else {
                Span::styled(flow.name.clone(), theme::bright())
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(" ❯ ", theme::star()), input]))
                    .style(on_panel),
                chunks[2],
            );
            let target = if flow.name.trim().is_empty() {
                "will be created here".to_string()
            } else {
                format!(
                    "creates {}",
                    compact_path(&flow.parent.join(flow.name.trim()))
                )
            };
            frame.render_widget(
                Paragraph::new(panel_subtitle(&target, chunks[3].width)).style(on_panel),
                chunks[3],
            );
            if chunks[4].height > 0 {
                match &flow.error {
                    Some(error) => render_footer(frame, chunks[4], error, true),
                    None => render_footer(frame, chunks[4], "Enter continue  ·  Esc back", false),
                }
            }
            // Place the terminal cursor in the input row.
            let cursor_x = chunks[2].x + 3 + flow.name.chars().count() as u16;
            if cursor_x < chunks[2].x + chunks[2].width {
                frame.set_cursor_position((cursor_x, chunks[2].y));
            }
        }
        NewProjectStep::Language => {
            let rows: Vec<(String, String)> = create::CREATABLE
                .iter()
                .map(|language| {
                    (
                        language.name().to_string(),
                        create::describe(*language).to_string(),
                    )
                })
                .collect();
            let visible = rows.len();
            let height = (visible as u16 + 5).min(area.height);
            let title = panel_title("New Project  ·  3 of 3  language");
            let inner = draw_panel(frame, area, title, width, height);
            if inner.height < 4 {
                return;
            }
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(inner);
            frame.render_widget(
                Paragraph::new(panel_subtitle(
                    &format!("{} in {}", flow.name, compact_path(&flow.parent)),
                    chunks[0].width,
                ))
                .style(on_panel),
                chunks[0],
            );
            render_divider(frame, chunks[1]);
            render_panel_rows(frame, chunks[2], &rows, flow.language);
            match &flow.error {
                Some(error) => render_footer(frame, chunks[3], error, true),
                None => render_footer(
                    frame,
                    chunks[3],
                    "↑↓ choose  ·  Enter create  ·  Esc back",
                    false,
                ),
            }
        }
    }
}

/// Render notifications stacked above the statusline.
pub fn render_toasts(frame: &mut Frame, area: Rect, toasts: &[Toast]) {
    if toasts.is_empty() || area.height < 3 || area.width < 16 {
        return;
    }
    let width = ((area.width as usize) * 2 / 3).clamp(16, 64) as u16;
    let x = area.x + area.width.saturating_sub(width + 2);
    let on_panel = Style::default().bg(theme::panel_bg());

    for (index, toast) in toasts.iter().rev().take(3).enumerate() {
        let Some(offset) = area.height.checked_sub(2 + index as u16) else {
            break;
        };
        if offset == 0 {
            break;
        }
        let rect = Rect {
            x,
            y: area.y + offset,
            width,
            height: 1,
        };
        let (glyph, style) = match toast.kind {
            ToastKind::Success => ("✦", theme::success()),
            ToastKind::Error => ("●", theme::error()),
            ToastKind::Info => ("·", theme::info()),
        };
        let message = truncate(&toast.message, width.saturating_sub(4) as usize);
        let pad = (width as usize).saturating_sub(3 + message.chars().count());
        let line = Line::from(vec![
            Span::styled(format!(" {glyph} "), style.bg(theme::panel_bg())),
            Span::styled(message, on_panel),
            Span::styled(" ".repeat(pad), on_panel),
        ]);
        frame.render_widget(Clear, rect);
        frame.render_widget(Paragraph::new(line).style(on_panel), rect);
    }
}

/// Fill the remaining width of a search row with the panel background.
fn pad_line(spans: &mut Vec<Span<'static>>, width: u16) {
    let used: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    let pad = (width as usize).saturating_sub(used);
    spans.push(Span::styled(
        " ".repeat(pad),
        Style::default().bg(theme::panel_bg()),
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
