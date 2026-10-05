//! The tab strip: a quiet row of pills.
//!
//! The active tab is a solid panel-grey pill (`ui.menu.selected`) with a blue
//! accent bar; inactive tabs fade into the secondary grey. Unsaved work is a
//! small star. When there are more tabs than fit, the strip scrolls around the
//! active tab and shows overflow chevrons.

use std::collections::HashMap;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::editor::Editor;
use crate::ui::theme;

/// Longest a tab title may grow before it is clipped with an ellipsis.
const MAX_TITLE: usize = 20;

pub fn render(frame: &mut Frame, area: Rect, editor: &Editor) {
    if editor.documents.is_empty() || area.height == 0 {
        return;
    }
    let active = editor.active_index();
    let labels = tab_labels(editor);
    let count = labels.len();
    let widths: Vec<usize> = labels
        .iter()
        .zip(editor.documents.iter())
        .enumerate()
        .map(|(index, (label, doc))| tab_width(label, doc.is_dirty(), index + 1 < count))
        .collect();
    let total: usize = widths.iter().sum::<usize>() + 1;
    let width = area.width as usize;

    let (first, last, more_left, more_right) = if total <= width {
        (0, count - 1, false, false)
    } else {
        let available = width.saturating_sub(4);
        let mut first = active;
        let mut last = active;
        let mut used = widths[active];
        loop {
            let mut progressed = false;
            if first > 0 && used + widths[first - 1] <= available {
                first -= 1;
                used += widths[first];
                progressed = true;
            }
            if last + 1 < count && used + widths[last + 1] <= available {
                last += 1;
                used += widths[last];
                progressed = true;
            }
            if !progressed {
                break;
            }
        }
        (first, last, first > 0, last + 1 < count)
    };

    let mut spans: Vec<Span> = Vec::new();
    if more_left {
        spans.push(Span::styled("‹ ", theme::dim()));
    } else {
        spans.push(Span::raw(" "));
    }

    for (offset, label) in labels[first..=last].iter().enumerate() {
        let index = first + offset;
        let is_active = index == active;
        spans.push(if is_active {
            Span::styled("▏", theme::accent())
        } else {
            Span::raw(" ")
        });

        let style = if is_active {
            theme::pill(theme::menu_selected_bg(), theme::text_bright())
        } else {
            theme::muted()
        };
        spans.push(Span::styled(format!(" {label} "), style));

        if editor.documents[index].is_dirty() {
            let dot = if is_active {
                theme::star().bg(theme::menu_selected_bg())
            } else {
                theme::star()
            };
            spans.push(Span::styled("●", dot));
        }

        if index < last {
            spans.push(Span::styled(" │ ", theme::dim()));
        }
    }

    if more_right {
        spans.push(Span::styled(" ›", theme::dim()));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn tab_width(label: &str, dirty: bool, has_separator: bool) -> usize {
    let mut width = 1 + label.chars().count() + 2 + dirty as usize;
    if has_separator {
        width += 3;
    }
    width
}

/// Titles for every tab, disambiguating duplicate file names by parent folder.
fn tab_labels(editor: &Editor) -> Vec<String> {
    let names: Vec<String> = editor.documents.iter().map(|doc| doc.file_name()).collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for name in &names {
        *counts.entry(name.as_str()).or_insert(0) += 1;
    }

    editor
        .documents
        .iter()
        .enumerate()
        .map(|(index, doc)| {
            let name = &names[index];
            let title = if counts[name.as_str()] > 1 {
                doc.buffer
                    .path
                    .as_ref()
                    .and_then(|path| path.parent())
                    .and_then(|parent| parent.file_name())
                    .and_then(|parent| parent.to_str())
                    .map(|parent| format!("{parent}/{name}"))
                    .unwrap_or_else(|| name.clone())
            } else {
                name.clone()
            };
            truncate(&title, MAX_TITLE)
        })
        .collect()
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}
