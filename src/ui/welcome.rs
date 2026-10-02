//! The welcome screen shown when no file is open.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::ui::theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled("k o d a", theme::accent_bold())).centered(),
        Line::from(Span::styled(
            "a modern, terminal-native IDE",
            theme::muted(),
        ))
        .centered(),
        Line::from(""),
        Line::from(""),
        shortcut("Ctrl+P", "Quick open"),
        shortcut("Ctrl+Shift+P", "Command palette"),
        shortcut("Ctrl+O", "Open file"),
        shortcut("Ctrl+B", "Toggle file tree"),
        shortcut("Ctrl+E", "Focus file tree"),
        shortcut("Ctrl+F", "Find"),
        Line::from(""),
        Line::from(Span::styled(
            app.workspace.root().display().to_string(),
            theme::dim(),
        ))
        .centered(),
    ];

    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn shortcut(key: &str, description: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {key:<14}", key = key), theme::accent()),
        Span::styled(description.to_string(), theme::muted()),
    ])
    .centered()
}
