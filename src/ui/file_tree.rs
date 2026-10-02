//! The project sidebar.
//!
//! A single hairline rule separates it from the editor. Directories are lilac,
//! files lavender, selection is a warm bossanova row, and git state lives in a
//! quiet right-aligned column.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::git::{GitFileStatus, GitInfo};
use crate::project::file_tree::{FileTree, VisibleEntry};
use crate::ui::{art, theme};

pub fn render(frame: &mut Frame, area: Rect, tree: &FileTree, git: &GitInfo, focused: bool) {
    if area.width < 8 || area.height == 0 {
        return;
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

    // The hairline rule between sidebar and editor.
    let rules: Vec<Line> = (0..area.height)
        .map(|_| Line::from(Span::styled("│", theme::border(focused))))
        .collect();
    frame.render_widget(Paragraph::new(Text::from(rules)), separator);

    // Header: `✦ files ────────`. The star brightens when the tree has focus.
    let title = "✦ files";
    let used = 2 + title.chars().count();
    let rule = "─".repeat((content_width as usize).saturating_sub(used).max(1));
    let (star_style, title_style) = if focused {
        (theme::star(), theme::accent_bold())
    } else {
        (theme::dim(), theme::muted())
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ", theme::text()),
            Span::styled("✦", star_style),
            Span::styled(" files ", title_style),
            Span::styled(rule, theme::dim()),
        ])),
        Rect {
            height: 1,
            ..content
        },
    );

    if area.height < 2 {
        return;
    }
    let list = Rect {
        y: content.y + 1,
        height: content.height - 1,
        ..content
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
        lines.push(entry_line(
            index == tree.selected,
            entry,
            content_width,
            git,
        ));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), list);
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
