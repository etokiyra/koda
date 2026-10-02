//! The top bar: identity, project and active file.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::ui::theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let left = Line::from(vec![
        Span::styled("koda", theme::accent_bold()),
        Span::styled("  ", theme::dim()),
        Span::styled(app.workspace.project.label(), theme::muted()),
    ]);
    frame.render_widget(Paragraph::new(left), area);

    if let Some(doc) = app.editor.active_document() {
        let dirty = if doc.is_dirty() { " ●" } else { "" };
        let text = format!("{}{dirty} ", doc.file_name());
        let width = text.chars().count() as u16;
        if area.width > width + 4 {
            let rect = Rect {
                x: area.x + area.width - width,
                y: area.y,
                width,
                height: 1,
            };
            let spans = if doc.is_dirty() {
                vec![
                    Span::styled(doc.file_name(), theme::text()),
                    Span::styled(" ●", theme::accent()),
                ]
            } else {
                vec![Span::styled(doc.file_name(), theme::muted())]
            };
            frame.render_widget(Paragraph::new(Line::from(spans)), rect);
        }
    }
}
