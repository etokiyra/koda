//! The project sidebar.
//!
//! A single hairline rule separates it from the editor. Directories take the
//! accent, files the primary text, selection is a `ui.selection` row, and git
//! state lives in a quiet right-aligned column. Pressing `/` turns the sidebar
//! into an inline fuzzy file filter.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::app::overlay::TreeFilter;
use crate::git::{GitFileStatus, GitInfo};
use crate::project::file_tree::{FileTree, VisibleEntry};
use crate::ui::{art, theme};

pub fn render(frame: &mut Frame, area: Rect, tree: &FileTree, git: &GitInfo, focused: bool) {
    let Some(list) = draw_chrome(frame, area, focused, files_header(focused, area.width)) else {
        return;
    };

    if tree.is_empty() {
        render_empty(frame, list);
        return;
    }

    let entries = tree.entries();
    let height = list.height as usize;
    let start = if tree.selected >= height {
        tree.selected + 1 - height
    } else {
        0
    };

    let mut lines: Vec<Line> = Vec::new();
    for (index, entry) in entries.iter().enumerate().skip(start).take(height) {
        lines.push(entry_line(index == tree.selected, entry, list.width, git));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), list);
}

/// Render the inline file filter.
pub fn render_filter(frame: &mut Frame, area: Rect, filter: &TreeFilter, focused: bool) {
    let Some(list) = draw_chrome(frame, area, focused, filter_header(filter, area.width)) else {
        return;
    };

    if filter.matches.is_empty() {
        frame.render_widget(
            Paragraph::new(art::familiar_line("no matching files")),
            list,
        );
        return;
    }

    let height = list.height as usize;
    let start = if filter.selected >= height {
        filter.selected + 1 - height
    } else {
        0
    };

    let mut lines: Vec<Line> = Vec::new();
    for (index, path) in filter.matches.iter().enumerate().skip(start).take(height) {
        let selected = index == filter.selected;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let detail = path
            .strip_prefix(&filter.root)
            .unwrap_or(path)
            .display()
            .to_string();

        let name_style = if selected {
            theme::bright_bold()
        } else {
            theme::text()
        };
        let prefix = vec![
            if selected {
                Span::styled("▏ ", theme::accent())
            } else {
                Span::raw("  ")
            },
            Span::styled("· ", theme::dim()),
        ];
        let prefix_used: usize = prefix.iter().map(|s| s.content.chars().count()).sum();

        let name_budget = (list.width as usize).saturating_sub(prefix_used + 1);
        let name = truncate(&name, name_budget);
        let used = prefix_used + name.chars().count();

        let detail_budget = (list.width as usize)
            .saturating_sub(used + 2)
            .saturating_sub(1);
        let detail = truncate(&detail, detail_budget);

        let mut spans = prefix;
        spans.push(Span::styled(name, name_style));
        if !detail.is_empty() {
            spans.push(Span::styled("  ", theme::dim()));
            spans.push(Span::styled(detail, theme::dim()));
        }

        let mut line = Line::from(spans);
        if selected {
            line = line.style(Style::default().bg(theme::SIDEBAR_SELECTED_BG));
        }
        lines.push(line);
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), list);
}

/// Draw the separator rule and header, returning the list rectangle.
fn draw_chrome(
    frame: &mut Frame,
    area: Rect,
    focused: bool,
    header: Line<'static>,
) -> Option<Rect> {
    if area.width < 8 || area.height == 0 {
        return None;
    }
    let content_width = area.width - 1;
    let content = Rect {
        width: content_width,
        ..area
    };
    let separator = Rect {
        x: area.x + content_width,
        y: area.y,
        width: 1,
        height: area.height,
    };

    let rules: Vec<Line> = (0..area.height)
        .map(|_| Line::from(Span::styled("│", theme::border(focused))))
        .collect();
    frame.render_widget(Paragraph::new(Text::from(rules)), separator);
    frame.render_widget(
        Paragraph::new(header),
        Rect {
            height: 1,
            ..content
        },
    );

    if area.height < 2 {
        return None;
    }
    Some(Rect {
        y: content.y + 1,
        height: content.height - 1,
        ..content
    })
}

fn files_header(focused: bool, width: u16) -> Line<'static> {
    let (star_style, title_style) = if focused {
        (theme::star(), theme::accent_bold())
    } else {
        (theme::dim(), theme::muted())
    };
    let used = 2 + "✦ files".chars().count();
    let rule = "─".repeat((width as usize).saturating_sub(1 + used).max(1));
    Line::from(vec![
        Span::styled(" ", theme::text()),
        Span::styled("✦", star_style),
        Span::styled(" files ", title_style),
        Span::styled(rule, theme::dim()),
    ])
}

fn filter_header(filter: &TreeFilter, width: u16) -> Line<'static> {
    let (query_style, text) = if filter.query.is_empty() {
        (theme::muted(), "filter…".to_string())
    } else {
        (theme::bright(), filter.query.clone())
    };
    let used = 2 + 1 + text.chars().count();
    let rule = "─".repeat((width as usize).saturating_sub(1 + used).max(1));
    Line::from(vec![
        Span::styled(" ", theme::text()),
        Span::styled("✦", theme::star()),
        Span::styled("/", theme::accent()),
        Span::styled(text, query_style),
        Span::styled(" ", theme::dim()),
        Span::styled(rule, theme::dim()),
    ])
}

fn entry_line(selected: bool, entry: &VisibleEntry, width: u16, git: &GitInfo) -> Line<'static> {
    let is_dir = entry.is_dir;
    let glyph = if is_dir {
        if entry.expanded { "▾" } else { "▸" }
    } else {
        "·"
    };
    let glyph_style = if is_dir {
        theme::accent()
    } else {
        theme::dim()
    };
    let name_style = if selected {
        theme::bright_bold()
    } else if is_dir {
        theme::accent()
    } else {
        theme::text()
    };

    let status = git
        .status_for(&entry.path)
        .map(|status| Span::styled(status.indicator().to_string(), status_style(status)));
    let status_len = status
        .as_ref()
        .map_or(0, |span| span.content.chars().count());

    let prefix = vec![
        if selected {
            Span::styled("▏ ", theme::accent())
        } else {
            Span::raw("  ")
        },
        Span::raw("  ".repeat(entry.depth)),
        Span::styled(format!("{glyph} "), glyph_style),
    ];
    let prefix_used: usize = prefix.iter().map(|span| span.content.chars().count()).sum();

    // Keep at least one column between the name and the git indicator.
    let name_budget = (width as usize).saturating_sub(prefix_used + status_len + 1);
    let name = truncate(&entry.name, name_budget);

    let mut spans = prefix;
    let used = prefix_used + name.chars().count();
    spans.push(Span::styled(name, name_style));

    let pad = (width as usize).saturating_sub(used + status_len);
    spans.push(Span::raw(" ".repeat(pad)));
    if let Some(status) = status {
        spans.push(status);
    }

    let mut line = Line::from(spans);
    if selected {
        line = line.style(Style::default().bg(theme::SIDEBAR_SELECTED_BG));
    }
    line
}

fn render_empty(frame: &mut Frame, area: Rect) {
    let mut lines = art::art_lines(art::CAT_ASLEEP, theme::dim());
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("no files", theme::muted())).centered());

    let block_height = lines.len() as u16;
    let mut padded: Vec<Line> = Vec::new();
    if area.height > block_height {
        let top = (area.height - block_height) / 2;
        padded.extend((0..top).map(|_| Line::from("")));
    }
    padded.extend(lines);
    frame.render_widget(Paragraph::new(Text::from(padded)), area);
}

fn status_style(status: GitFileStatus) -> Style {
    match status {
        GitFileStatus::Added => theme::success(),
        GitFileStatus::Deleted | GitFileStatus::Conflicted => theme::error(),
        GitFileStatus::Modified | GitFileStatus::TypeChanged => theme::warn(),
        GitFileStatus::Renamed => theme::accent(),
        GitFileStatus::Untracked => theme::muted(),
    }
}

/// Shorten a name to `max` columns, appending an ellipsis when clipped.
fn truncate(text: &str, max: usize) -> String {
    let max = max.max(1);
    if text.chars().count() <= max {
        return text.to_string();
    }
    if max == 1 {
        return "…".to_string();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}
