//! The Koda statusline.
//!
//! A solid panel strip with a language pill, git state and cursor position —
//! Koda's most recognisable band of colour.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::language::LanguageId;
use crate::language::diagnostics::{Severity, TextPos};
use crate::ui::{art, theme};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let on_panel = Style::default().bg(theme::PANEL_BG);
    let mut spans: Vec<Span> = Vec::new();

    let document = app.editor.active_document();

    if let Some(doc) = document {
        let language = doc.buffer.language;
        let pill_style = if language == LanguageId::Unknown {
            theme::pill(theme::MUTED, theme::PANEL_BG)
        } else {
            theme::pill(theme::ACCENT_SOFT, theme::PANEL_BG)
        };
        spans.push(Span::styled(format!(" {} ", language.name()), pill_style));

        if doc.is_dirty() {
            spans.push(Span::styled(
                " ●",
                Style::default().fg(theme::STAR).bg(theme::PANEL_BG),
            ));
        }

        if let Some(selected) = doc.selected_text() {
            let lines = selected.matches('\n').count() + 1;
            let label = if lines > 1 {
                format!("  {lines} lines")
            } else {
                format!("  {} sel", selected.chars().count())
            };
            spans.push(Span::styled(
                label,
                Style::default().fg(theme::ACCENT).bg(theme::PANEL_BG),
            ));
        }

        // Diagnostics: keep errors loud, warnings present, both compact.
        let (errors, warnings) = doc.diagnostic_counts();
        if errors > 0 {
            spans.push(Span::styled(
                format!("  {errors}✖"),
                Style::default().fg(theme::ERROR).bg(theme::PANEL_BG),
            ));
        }
        if warnings > 0 {
            spans.push(Span::styled(
                format!("  {warnings}⚠"),
                Style::default().fg(theme::WARN).bg(theme::PANEL_BG),
            ));
        }

        if app.workspace.git.available
            && let Some(branch) = app.workspace.git.branch_label()
        {
            spans.push(Span::styled(" ", on_panel));
            spans.push(Span::styled(
                format!("{} {branch}", art::MOON),
                Style::default().fg(theme::ACCENT).bg(theme::PANEL_BG),
            ));
            let changed = app.workspace.git.files.len();
            if changed > 0 {
                spans.push(Span::styled(
                    format!(" {changed}±"),
                    Style::default().fg(theme::WARN).bg(theme::PANEL_BG),
                ));
            }
        }
    } else {
        spans.push(Span::styled(
            " ✦ koda ready ",
            theme::pill(theme::MUTED, theme::PANEL_BG),
        ));
    }

    // Right side: cursor position and scroll percentage.
    let right = document.map(|doc| {
        let position = doc.clamped_cursor();
        let total = doc.buffer.len_lines().max(1);
        let percent = ((position.row + 1) * 100 / total).min(100);
        format!(
            "Ln {}, Col {}   {} {}% ",
            position.row + 1,
            position.col + 1,
            art::MOON,
            percent
        )
    });
    let right_width = right.as_ref().map_or(0, |text| text.chars().count());

    // A transient status message wins; otherwise surface the diagnostic under
    // the cursor so problems explain themselves as you move through the file.
    let message = match app.status_message() {
        Some(message) => Some((
            message.to_string(),
            if app.status.error {
                theme::ERROR
            } else {
                theme::ACCENT
            },
        )),
        None => document
            .and_then(|doc| {
                let cursor = doc.clamped_cursor();
                doc.diagnostics()
                    .iter()
                    .find(|diagnostic| diagnostic.covers(TextPos::new(cursor.row, cursor.col)))
            })
            .map(|diagnostic| {
                (
                    format!("{} {}", diagnostic.severity.gutter(), diagnostic.message),
                    severity_color(diagnostic.severity),
                )
            }),
    };

    if let Some((text, color)) = message {
        let base_width: usize = spans.iter().map(|span| span.content.chars().count()).sum();
        let budget = (area.width as usize).saturating_sub(base_width + right_width + 6);
        if budget > 1 {
            spans.push(Span::styled(
                "  ·  ",
                Style::default().fg(theme::FAINT).bg(theme::PANEL_BG),
            ));
            spans.push(Span::styled(
                truncate(&text, budget),
                Style::default().fg(color).bg(theme::PANEL_BG),
            ));
        }
    }

    let left_width: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    let pad = (area.width as usize).saturating_sub(left_width + right_width);
    spans.push(Span::styled(" ".repeat(pad), on_panel));
    if let Some(right) = right {
        spans.push(Span::styled(
            right,
            Style::default().fg(theme::ACCENT_SOFT).bg(theme::PANEL_BG),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn severity_color(severity: Severity) -> Color {
    match severity {
        Severity::Error => theme::ERROR,
        Severity::Warning => theme::WARN,
        Severity::Info => theme::INFO,
        Severity::Hint => theme::HINT,
    }
}
