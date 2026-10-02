//! The status bar.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::language::LanguageId;
use crate::ui::theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans: Vec<Span> = Vec::new();

    if let Some(message) = app.status_message() {
        let style = if app.status.error {
            theme::error()
        } else {
            theme::muted()
        };
        spans.push(Span::styled(" ", style));
        spans.push(Span::styled(message.to_string(), style));
    } else if let Some(doc) = app.editor.active_document() {
        let language = doc.buffer.language;
        let dot = if language == LanguageId::Unknown {
            theme::dim()
        } else {
            theme::success()
        };
        spans.push(Span::styled(" ● ", dot));
        spans.push(Span::styled(language.name(), theme::text()));

        if app.workspace.git.available
            && let Some(branch) = app.workspace.git.branch_label()
        {
            spans.push(Span::styled("  │  ", theme::dim()));
            spans.push(Span::styled(format!("⎇ {branch}"), theme::accent()));
            let changed = app.workspace.git.files.len();
            if changed > 0 {
                spans.push(Span::styled(format!(" {changed}±"), theme::warn()));
            }
        }
    } else {
        spans.push(Span::styled(
            " Koda — press Ctrl+P to open a file",
            theme::muted(),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    if let Some(doc) = app.editor.active_document() {
        let position = doc.clamped_cursor();
        let text = format!("Ln {}, Col {} ", position.row + 1, position.col + 1);
        let width = text.chars().count() as u16;
        if area.width > width {
            let rect = Rect {
                x: area.x + area.width - width,
                y: area.y,
                width,
                height: 1,
            };
            frame.render_widget(Paragraph::new(Span::styled(text, theme::muted())), rect);
        }
    }
}
