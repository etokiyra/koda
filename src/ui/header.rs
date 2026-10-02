//! The Koda header: identity plus a breadcrumb, no boring rule.
//!
//! `  ✦ koda  ·  Rust  ·  src/main.rs ●`

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::ui::{art, theme};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let mut spans = vec![
        Span::styled(" ✦ ", theme::star()),
        Span::styled("koda", theme::accent_bold()),
        Span::styled("  ", theme::dim()),
        Span::styled("·", theme::dim()),
        Span::styled("  ", theme::dim()),
        Span::styled(app.workspace.project.label().to_string(), theme::soft()),
    ];

    if let Some(doc) = app.editor.active_document() {
        spans.push(Span::styled("  ·  ", theme::dim()));
        spans.push(Span::styled(
            relative_path(app, doc.buffer.path.as_deref()),
            theme::bright(),
        ));
        if doc.is_dirty() {
            spans.push(Span::styled(" ●", theme::star()));
        }
    }

    let left_width: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    // A quiet mark on the right for balance, only when it will not collide.
    let right = if app.editor.len() > 1 {
        format!("{} open  {}", app.editor.len(), art::SPARK)
    } else {
        format!("{}  {}  {}", art::SPARK, art::DOT, art::SPARK)
    };
    let right_width = right.chars().count();
    if (area.width as usize) > left_width + right_width + 3 {
        let rect = Rect {
            x: area.x + area.width - right_width as u16 - 1,
            y: area.y,
            width: right_width as u16 + 1,
            height: 1,
        };
        frame.render_widget(Paragraph::new(Span::styled(right, theme::dim())), rect);
    }
}

fn relative_path(app: &App, path: Option<&std::path::Path>) -> String {
    let Some(path) = path else {
        return "Untitled".to_string();
    };
    path.strip_prefix(app.workspace.root())
        .unwrap_or(path)
        .display()
        .to_string()
}
