//! The tab strip above the editor.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::editor::Editor;
use crate::ui::theme;

pub fn render(frame: &mut Frame, area: Rect, editor: &Editor) {
    if editor.documents.is_empty() {
        return;
    }
    let active = editor.active_index();
    let mut spans: Vec<Span> = Vec::new();
    for (index, doc) in editor.documents.iter().enumerate() {
        let is_active = index == active;
        let style = if is_active {
            theme::accent_bold()
        } else {
            theme::dim()
        };
        spans.push(Span::styled(" ", style));
        spans.push(Span::styled(doc.file_name(), style));
        if doc.is_dirty() {
            spans.push(Span::styled(" ●", theme::accent()));
        }
        spans.push(Span::styled(" ", style));
        if index + 1 < editor.documents.len() {
            spans.push(Span::styled("│", theme::dim()));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
