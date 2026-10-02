//! The project sidebar.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::git::{GitFileStatus, GitInfo};
use crate::project::file_tree::FileTree;
use crate::ui::theme;

pub fn render(frame: &mut Frame, area: Rect, tree: &FileTree, git: &GitInfo, focused: bool) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::border(focused))
        .title(" PROJECT ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let entries = tree.entries();
    let height = inner.height as usize;
    let start = if tree.selected >= height {
        tree.selected + 1 - height
    } else {
        0
    };

    let mut lines: Vec<Line> = Vec::new();
    for (index, entry) in entries.iter().enumerate().skip(start).take(height) {
        let selected = index == tree.selected;
        let indent = "  ".repeat(entry.depth);
        let glyph = if entry.is_dir {
            if entry.expanded { "▾ " } else { "▸ " }
        } else {
            "  "
        };

        let name_style = if entry.is_dir {
            theme::accent()
        } else if selected {
            theme::text()
        } else {
            theme::muted()
        };

        let mut spans = vec![
            Span::raw(indent),
            Span::styled(glyph, name_style),
            Span::styled(entry.name.clone(), name_style),
        ];

        if let Some(status) = git.status_for(&entry.path) {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                status.indicator().to_string(),
                status_style(status),
            ));
        }

        let mut line = Line::from(spans);
        if selected {
            line = line.style(Style::default().bg(theme::SIDEBAR_SELECTED_BG));
        }
        lines.push(line);
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn status_style(status: GitFileStatus) -> Style {
    match status {
        GitFileStatus::Added => theme::success(),
        GitFileStatus::Deleted | GitFileStatus::Conflicted => theme::error(),
        GitFileStatus::Modified | GitFileStatus::TypeChanged => theme::warn(),
        GitFileStatus::Renamed => theme::accent(),
        GitFileStatus::Untracked => theme::dim(),
    }
}
