//! The tab strip: a quiet row of pills.
//!
//! The active tab is a solid bossanova pill with a lilac bar; inactive tabs
//! fade into sirocco. Unsaved work is a small honey star.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::editor::Editor;
use crate::ui::theme;

pub fn render(frame: &mut Frame, area: Rect, editor: &Editor) {
    if editor.documents.is_empty() || area.height == 0 {
        return;
    }
    let active = editor.active_index();
    let mut spans: Vec<Span> = vec![Span::raw(" ")];

    for (index, doc) in editor.documents.iter().enumerate() {
        let is_active = index == active;
        let name = doc.file_name();

        if is_active {
            spans.push(Span::styled("▏", theme::accent()));
            spans.push(Span::styled(
                format!(" {name} "),
                theme::pill(theme::CURSORLINE_BG, theme::TEXT_BRIGHT),
            ));
            if doc.is_dirty() {
                spans.push(Span::styled("●", theme::star().bg(theme::CURSORLINE_BG)));
            }
        } else {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(format!(" {name} "), theme::muted()));
            if doc.is_dirty() {
                spans.push(Span::styled("●", theme::star()));
            }
        }

        if index + 1 < editor.documents.len() {
            spans.push(Span::styled(" │ ", theme::dim()));
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
